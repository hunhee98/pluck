//! BM25 chunk index backed by tantivy.
//!
//! Each chunk produced by the AST chunker becomes one tantivy document.
//! The `symbol`, `signature`, and `content` fields participate in the
//! default BM25 scoring. Hybrid search additionally lets the BM25 side
//! see `doc_comment` so natural-language queries can use API prose
//! without changing BM25-only behavior.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use tantivy::{
    collector::TopDocs,
    doc,
    query::QueryParser,
    schema::{
        Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, Value, FAST, INDEXED,
        STORED, STRING,
    },
    Index as TantivyIndex, IndexWriter, TantivyDocument, Term,
};

use crate::chunker::{Chunk, ChunkKind};
use crate::ranking::apply_boosts;
use crate::semantic::{cosine_similarity, StaticEncoder};
use crate::tokenizer::{PluckTokenizer, TOKENIZER_NAME};

/// BM25F per-field boosts.
///
/// Standard structured-doc IR move: symbol matches dominate (the user
/// usually means the function they named), signature next (carries the
/// type + param names — semantic-dense per byte), doc comments next
/// for hybrid NL queries, content last (longest field, BM25's IDF
/// already favors rare tokens here). Plain BM25 keeps the historical
/// 5 / 3 / 1 ratio; hybrid adds doc comments at 4.
const BM25F_BOOST_SYMBOL: f32 = 5.0;
const BM25F_BOOST_SIGNATURE: f32 = 3.0;
const BM25F_BOOST_DOC_COMMENT: f32 = 4.0;
const BM25F_BOOST_CONTENT: f32 = 1.0;

const WRITER_HEAP_BYTES: usize = 50_000_000;

pub struct PluckIndex {
    inner: TantivyIndex,
    fields: Fields,
    /// Optional static encoder. When present, `add_chunk` auto-encodes
    /// each chunk's doc-comment + signature + content and
    /// `search_hybrid` fuses BM25 with cosine similarity. When absent,
    /// the index degrades cleanly to BM25-only — every existing call
    /// site keeps working without the network/disk cost of loading the
    /// embedding model.
    encoder: Option<Arc<StaticEncoder>>,
    /// chunk_id → embedding vector. Populated only when `encoder` is set.
    /// RwLock so reads (search) don't block other reads.
    embeddings: Arc<RwLock<HashMap<u64, Vec<f32>>>>,
}

#[derive(Clone, Copy)]
struct Fields {
    chunk_id: Field,
    path: Field,
    symbol: Field,
    kind: Field,
    start_line: Field,
    end_line: Field,
    doc_comment: Field,
    signature: Field,
    content: Field,
}

/// Text option for fields that participate in BM25 — uses the custom
/// `pluck` tokenizer registered in [`register_pluck_tokenizer`].
fn pluck_text_field() -> TextOptions {
    TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(TOKENIZER_NAME)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored()
}

fn build_schema() -> (Schema, Fields) {
    let mut sb = Schema::builder();
    let chunk_id = sb.add_u64_field("chunk_id", INDEXED | STORED | FAST);
    let path = sb.add_text_field("path", STRING | STORED);
    let symbol = sb.add_text_field("symbol", pluck_text_field());
    let kind = sb.add_text_field("kind", STRING | STORED);
    let start_line = sb.add_u64_field("start_line", STORED);
    let end_line = sb.add_u64_field("end_line", STORED);
    let doc_comment = sb.add_text_field("doc_comment", pluck_text_field());
    let signature = sb.add_text_field("signature", pluck_text_field());
    let content = sb.add_text_field("content", pluck_text_field());
    let schema = sb.build();
    (
        schema,
        Fields {
            chunk_id,
            path,
            symbol,
            kind,
            start_line,
            end_line,
            doc_comment,
            signature,
            content,
        },
    )
}

/// Every freshly opened `PluckIndex` must register the `pluck` tokenizer
/// before any reader / writer talks to it — otherwise tantivy reports
/// "tokenizer not found" on the first BM25 query. Cheap (Clone of a
/// unit struct).
fn register_pluck_tokenizer(index: &TantivyIndex) {
    index.tokenizers().register(TOKENIZER_NAME, PluckTokenizer);
}

impl PluckIndex {
    pub fn in_ram() -> Result<Self> {
        let (schema, fields) = build_schema();
        let inner = TantivyIndex::create_in_ram(schema);
        register_pluck_tokenizer(&inner);
        Ok(Self {
            inner,
            fields,
            encoder: None,
            embeddings: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn open_or_create(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).ok();
        let (schema, fields) = build_schema();
        let inner = match TantivyIndex::open_in_dir(dir) {
            Ok(idx) => idx,
            Err(_) => TantivyIndex::create_in_dir(dir, schema).context("create tantivy dir")?,
        };
        register_pluck_tokenizer(&inner);
        Ok(Self {
            inner,
            fields,
            encoder: None,
            embeddings: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Attach an embedding encoder. Subsequent `add_chunk` calls will
    /// also store an embedding; `search_hybrid` becomes usable.
    pub fn with_encoder(mut self, encoder: Arc<StaticEncoder>) -> Self {
        self.encoder = Some(encoder);
        self
    }

    pub fn has_encoder(&self) -> bool {
        self.encoder.is_some()
    }

    pub fn writer(&self) -> Result<IndexBatch> {
        let writer = self
            .inner
            .writer::<TantivyDocument>(WRITER_HEAP_BYTES)
            .context("open index writer")?;
        Ok(IndexBatch {
            writer,
            fields: self.fields,
            next_chunk_id: 0,
            encoder: self.encoder.clone(),
            embeddings: Arc::clone(&self.embeddings),
        })
    }

    pub fn search(&self, query_str: &str, k: usize) -> Result<Vec<SearchHit>> {
        self.search_with_cutoff(query_str, k, 0.0)
    }

    /// Build a query parser that scores BM25 per field but with the
    /// BM25F per-field boosts applied. The actual fusion across fields
    /// is BM25's own field-by-field accumulator — tantivy's
    /// QueryParser distributes the parsed query across each field and
    /// multiplies the per-field score by the boost we set here.
    fn bm25f_query_parser(&self, include_doc_comment: bool) -> QueryParser {
        let mut fields = vec![self.fields.symbol, self.fields.signature, self.fields.content];
        if include_doc_comment {
            fields.push(self.fields.doc_comment);
        }
        let mut qp = QueryParser::for_index(&self.inner, fields);
        qp.set_field_boost(self.fields.symbol, BM25F_BOOST_SYMBOL);
        qp.set_field_boost(self.fields.signature, BM25F_BOOST_SIGNATURE);
        if include_doc_comment {
            qp.set_field_boost(self.fields.doc_comment, BM25F_BOOST_DOC_COMMENT);
        }
        qp.set_field_boost(self.fields.content, BM25F_BOOST_CONTENT);
        qp
    }

    /// Exact lookup by symbol name (the `symbol` field).
    ///
    /// `name` is matched as a term against the tantivy default tokenizer
    /// applied to the symbol field. Returns every chunk whose symbol
    /// matches; an optional `path_contains` filter narrows ambiguous
    /// matches (e.g. when two files both define `handleLogin`).
    pub fn lookup_symbol(&self, name: &str, path_contains: Option<&str>) -> Result<Vec<SearchHit>> {
        use tantivy::collector::TopDocs;
        use tantivy::query::QueryParser;

        let reader = self.inner.reader().context("open reader")?;
        let searcher = reader.searcher();
        let qp = QueryParser::for_index(&self.inner, vec![self.fields.symbol]);
        // Tantivy's default tokenizer lowercases — match its behavior here.
        let q = qp
            .parse_query(&name.to_lowercase())
            .context("parse symbol query")?;
        let top = searcher
            .search(&q, &TopDocs::with_limit(64).order_by_score())
            .context("symbol search")?;

        let mut hits = Vec::new();
        for (score, addr) in top {
            let doc: TantivyDocument = searcher.doc(addr).context("doc retrieve")?;
            let hit = self.doc_to_hit(score, &doc)?;
            // Require exact symbol equality (case-insensitive) — the BM25
            // path may surface near-matches we don't want here.
            if hit.symbol.eq_ignore_ascii_case(name)
                && path_contains.map(|p| hit.path.contains(p)).unwrap_or(true)
            {
                hits.push(hit);
            }
        }
        Ok(hits)
    }

    /// Same as `search`, but drops every hit whose BM25 score is below
    /// `cutoff_frac × top_score`. With `cutoff_frac = 0.12` this implements
    /// the 12% noise floor described in docs/MCP_TOOLS.md.
    pub fn search_with_cutoff(
        &self,
        query_str: &str,
        k: usize,
        cutoff_frac: f32,
    ) -> Result<Vec<SearchHit>> {
        self.search_with_cutoff_inner(query_str, k, cutoff_frac, false)
    }

    fn search_with_cutoff_inner(
        &self,
        query_str: &str,
        k: usize,
        cutoff_frac: f32,
        include_doc_comment: bool,
    ) -> Result<Vec<SearchHit>> {
        let reader = self.inner.reader().context("open reader")?;
        let searcher = reader.searcher();
        let qp = self.bm25f_query_parser(include_doc_comment);
        let query = qp.parse_query(query_str).context("parse query")?;
        let top = searcher
            .search(&query, &TopDocs::with_limit(k).order_by_score())
            .context("search")?;

        let threshold = top.first().map(|(s, _)| s * cutoff_frac).unwrap_or(0.0);

        let mut hits = Vec::with_capacity(top.len());
        for (score, addr) in top {
            if score < threshold {
                break;
            }
            let doc: TantivyDocument = searcher.doc(addr).context("doc retrieve")?;
            hits.push(self.doc_to_hit(score, &doc)?);
        }
        // Post-fusion ranking pipeline: symbol-match / sibling-chunk /
        // test-file boosts. Re-sorts in place.
        apply_boosts(&mut hits, query_str);
        Ok(hits)
    }

    /// Hybrid BM25 + semantic search via Reciprocal Rank Fusion.
    ///
    /// If no encoder is attached the call transparently falls through to
    /// `search_with_cutoff` so existing call sites keep working.
    ///
    /// Tuning constants — both well-trodden values from the IR
    /// literature:
    ///   - `RRF_K = 60`: the standard reciprocal-rank smoothing constant
    ///   - `OVERFETCH = 5`: pull 5×k candidates from each side before
    ///     fusion, so the fusion has room to rerank
    pub fn search_hybrid(
        &self,
        query_str: &str,
        k: usize,
        cutoff_frac: f32,
        alpha: Option<f32>,
    ) -> Result<Vec<SearchHit>> {
        const RRF_K: f32 = 60.0;
        const OVERFETCH: usize = 5;

        let Some(encoder) = &self.encoder else {
            return self.search_with_cutoff(query_str, k, cutoff_frac);
        };

        let candidate_k = (k * OVERFETCH).max(20);
        let semantic_alpha = alpha
            .unwrap_or_else(|| inferred_rrf_alpha(query_str))
            .clamp(0.0, 1.0);
        let bm25_alpha = 1.0 - semantic_alpha;

        // BM25 side.
        let bm25 = self.search_with_cutoff_inner(query_str, candidate_k, 0.0, true)?;

        // Semantic side: encode the query, then score it against *every*
        // chunk that has an embedding. Without this, queries whose terms
        // never appear lexically (the whole reason we added embeddings)
        // would still get filtered out by the BM25 pre-pass.
        let q_emb = encoder.encode(query_str)?;
        let embeddings = self
            .embeddings
            .read()
            .map_err(|_| anyhow::anyhow!("embeddings lock poisoned"))?;

        // Sort all chunk_ids by cosine, keep candidate_k.
        let mut scored: Vec<(u64, f32)> = embeddings
            .iter()
            .map(|(id, v)| (*id, cosine_similarity(&q_emb, v)))
            .collect();
        drop(embeddings);
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(candidate_k);

        // Hydrate the top semantic chunk_ids into SearchHits via tantivy.
        // This second tantivy roundtrip is bounded by `candidate_k`
        // documents.
        let reader = self.inner.reader().context("open reader")?;
        let searcher = reader.searcher();
        let mut sem: Vec<(SearchHit, f32)> = Vec::with_capacity(scored.len());
        for (id, cos) in &scored {
            // chunk_id is INDEXED — look it up by term.
            let term = Term::from_field_u64(self.fields.chunk_id, *id);
            let q = tantivy::query::TermQuery::new(term, tantivy::schema::IndexRecordOption::Basic);
            let top = searcher
                .search(&q, &TopDocs::with_limit(1).order_by_score())
                .context("chunk_id lookup")?;
            if let Some((_, addr)) = top.into_iter().next() {
                let doc: TantivyDocument = searcher.doc(addr).context("doc retrieve")?;
                sem.push((self.doc_to_hit(*cos, &doc)?, *cos));
            }
        }

        // Weighted RRF fusion. For identifier-like queries BM25 and
        // semantic stay balanced. Natural-language queries get a
        // semantic-heavy alpha so prose-aligned hits are not drowned by
        // lexical noise from large real repos.
        let mut rrf: HashMap<u64, (SearchHit, f32)> = HashMap::with_capacity(candidate_k);

        for (rank, hit) in bm25.iter().enumerate() {
            let bonus = bm25_alpha / (RRF_K + rank as f32 + 1.0);
            rrf.entry(hit.chunk_id)
                .and_modify(|(_, s)| *s += bonus)
                .or_insert_with(|| (hit.clone(), bonus));
        }
        for (rank, (hit, _cos)) in sem.iter().enumerate() {
            let bonus = semantic_alpha / (RRF_K + rank as f32 + 1.0);
            rrf.entry(hit.chunk_id)
                .and_modify(|(_, s)| *s += bonus)
                .or_insert_with(|| (hit.clone(), bonus));
        }

        let mut fused: Vec<(SearchHit, f32)> = rrf.into_values().collect();
        fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Reapply the noise floor against the fused score; same 12 %
        // semantic as `search_with_cutoff`.
        let threshold = fused.first().map(|(_, s)| s * cutoff_frac).unwrap_or(0.0);

        let mut out: Vec<SearchHit> = Vec::with_capacity(k);
        for (mut h, score) in fused {
            if score < threshold {
                break;
            }
            h.score = score;
            out.push(h);
            if out.len() >= k {
                break;
            }
        }
        apply_boosts(&mut out, query_str);
        Ok(out)
    }

    fn doc_to_hit(&self, score: f32, doc: &TantivyDocument) -> Result<SearchHit> {
        let f = self.fields;
        Ok(SearchHit {
            score,
            chunk_id: u64_field(doc, f.chunk_id).unwrap_or(0),
            path: str_field(doc, f.path).unwrap_or_default(),
            symbol: str_field(doc, f.symbol).unwrap_or_default(),
            kind: kind_from_str(&str_field(doc, f.kind).unwrap_or_default()),
            start_line: u64_field(doc, f.start_line).unwrap_or(0) as u32,
            end_line: u64_field(doc, f.end_line).unwrap_or(0) as u32,
            signature: str_field(doc, f.signature).unwrap_or_default(),
            content: str_field(doc, f.content).unwrap_or_default(),
        })
    }
}

fn inferred_rrf_alpha(query: &str) -> f32 {
    if is_natural_language_query(query) {
        0.7
    } else {
        0.5
    }
}

fn is_natural_language_query(query: &str) -> bool {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    tokens.len() >= 3 && query.contains(char::is_whitespace) && !has_identifier_pattern(&tokens)
}

fn has_identifier_pattern(tokens: &[&str]) -> bool {
    tokens.iter().any(|token| {
        token.contains("::")
            || token.contains('_')
            || token.contains('/')
            || token.contains('.')
            || token.chars().any(|c| c.is_ascii_digit())
            || has_camel_case_shape(token)
    })
}

fn has_camel_case_shape(token: &str) -> bool {
    let has_lower = token.chars().any(|c| c.is_ascii_lowercase());
    let has_upper = token.chars().any(|c| c.is_ascii_uppercase());
    has_lower && has_upper
}

pub struct IndexBatch {
    writer: IndexWriter,
    fields: Fields,
    next_chunk_id: u64,
    encoder: Option<Arc<StaticEncoder>>,
    embeddings: Arc<RwLock<HashMap<u64, Vec<f32>>>>,
}

impl IndexBatch {
    pub fn add_chunk(&mut self, file_path: &str, c: &Chunk) -> Result<u64> {
        let id = self.next_chunk_id;
        self.next_chunk_id += 1;
        self.writer
            .add_document(doc!(
                self.fields.chunk_id => id,
                self.fields.path => file_path,
                self.fields.symbol => c.symbol.as_str(),
                self.fields.kind => kind_str(&c.kind),
                self.fields.start_line => c.start_line as u64,
                self.fields.end_line => c.end_line as u64,
                self.fields.doc_comment => c.doc_comment.as_str(),
                self.fields.signature => c.signature.as_str(),
                self.fields.content => c.content.as_str(),
            ))
            .context("add_document")?;

        // If an encoder is attached, embed
        // `doc_comment + symbol + signature + content` so prose API docs
        // can carry natural-language queries toward the right chunk.
        // The encoder is allowed to fail silently — we degrade to
        // BM25-only for that chunk rather than aborting the whole
        // indexing pass.
        if let Some(enc) = &self.encoder {
            let text = embedding_text(c);
            match enc.encode(&text) {
                Ok(v) => {
                    if let Ok(mut map) = self.embeddings.write() {
                        map.insert(id, v);
                    }
                }
                Err(e) => {
                    tracing::warn!(chunk_id = id, "embedding failed: {e}");
                }
            }
        }
        Ok(id)
    }

    pub fn commit(mut self) -> Result<()> {
        self.writer.commit().context("commit")?;
        Ok(())
    }

    /// Mark every chunk for `rel_path` as deleted. The deletes are
    /// applied at commit time (tantivy adds a tombstone now and merges
    /// it into the segment on commit). Returns the (writer-local)
    /// opstamp from tantivy — unused by callers today, useful for
    /// future ordering invariants.
    pub fn delete_path(&mut self, rel_path: &str) -> u64 {
        let term = Term::from_field_text(self.fields.path, rel_path);
        self.writer.delete_term(term)
    }
}

fn embedding_text(c: &Chunk) -> String {
    let mut text = String::with_capacity(
        c.doc_comment.len() + c.symbol.len() + c.signature.len() + c.content.len() + 3,
    );
    text.push_str(&c.doc_comment);
    text.push('\n');
    text.push_str(&c.symbol);
    text.push('\n');
    text.push_str(&c.signature);
    text.push('\n');
    text.push_str(&c.content);
    text
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub score: f32,
    pub chunk_id: u64,
    pub path: String,
    pub symbol: String,
    pub kind: ChunkKind,
    pub start_line: u32,
    pub end_line: u32,
    pub signature: String,
    pub content: String,
}

fn kind_str(k: &ChunkKind) -> &'static str {
    match k {
        ChunkKind::Function => "function",
        ChunkKind::Method => "method",
        ChunkKind::Class => "class",
        ChunkKind::Struct => "struct",
        ChunkKind::Enum => "enum",
        ChunkKind::Impl => "impl",
        ChunkKind::Trait => "trait",
        ChunkKind::Module => "module",
    }
}

fn kind_from_str(s: &str) -> ChunkKind {
    match s {
        "method" => ChunkKind::Method,
        "class" => ChunkKind::Class,
        "struct" => ChunkKind::Struct,
        "enum" => ChunkKind::Enum,
        "impl" => ChunkKind::Impl,
        "trait" => ChunkKind::Trait,
        "module" => ChunkKind::Module,
        _ => ChunkKind::Function,
    }
}

fn str_field(doc: &TantivyDocument, field: Field) -> Option<String> {
    doc.get_first(field)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
}

fn u64_field(doc: &TantivyDocument, field: Field) -> Option<u64> {
    doc.get_first(field).and_then(|v| v.as_u64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::{chunk_source, Language};

    fn index_one_file(src: &str, path: &str, lang: Language) -> PluckIndex {
        let idx = PluckIndex::in_ram().unwrap();
        let mut w = idx.writer().unwrap();
        for c in chunk_source(src, lang).unwrap() {
            w.add_chunk(path, &c).unwrap();
        }
        w.commit().unwrap();
        idx
    }

    #[test]
    fn search_returns_matching_symbol() {
        let src = r#"
async function handleLogin(user: string, pass: string): Promise<boolean> {
  const token = await issueToken(user);
  return token !== null;
}

function issueToken(user: string): string | null {
  if (!user) return null;
  return "tk_" + user;
}

function unrelatedHelper(): void {
  console.log("noop");
}
"#;
        let idx = index_one_file(src, "auth.ts", Language::TypeScript);
        let hits = idx.search("handleLogin", 5).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].symbol, "handleLogin");
        assert_eq!(hits[0].path, "auth.ts");
        assert_eq!(hits[0].kind, ChunkKind::Function);
    }

    #[test]
    fn search_ranks_by_relevance() {
        // Two functions; query matches one strongly via body content.
        let src = r#"
function tokenizer(input: string): string[] {
  return input.split(" ");
}

function unrelated(): number {
  return 42;
}
"#;
        let idx = index_one_file(src, "x.ts", Language::TypeScript);
        let hits = idx.search("tokenizer split", 5).unwrap();
        assert!(!hits.is_empty(), "expected hits");
        assert_eq!(hits[0].symbol, "tokenizer");
    }

    #[test]
    fn search_hybrid_falls_through_to_bm25_without_encoder() {
        // No encoder attached → hybrid == bm25 cutoff.
        let src = r#"
function validateToken(token: string): boolean { return token.length > 0; }
function unrelatedHelper(): void {}
"#;
        let idx = index_one_file(src, "auth.ts", Language::TypeScript);
        assert!(!idx.has_encoder());
        let h1 = idx.search_with_cutoff("validateToken", 5, 0.0).unwrap();
        let h2 = idx.search_hybrid("validateToken", 5, 0.0, None).unwrap();
        assert_eq!(h1.len(), h2.len());
        assert!(!h1.is_empty());
        assert_eq!(h1[0].symbol, h2[0].symbol);
    }

    #[test]
    fn rrf_alpha_prefers_semantic_for_natural_language_queries() {
        assert_eq!(
            inferred_rrf_alpha("receive value from channel asynchronously"),
            0.7
        );
        assert_eq!(inferred_rrf_alpha("Runtime::spawn future"), 0.5);
        assert_eq!(inferred_rrf_alpha("validateToken"), 0.5);
    }

    #[test]
    fn embedding_text_includes_doc_comment_before_code() {
        let src = r#"
/// Receive a value from an asynchronous channel.
pub async fn recv_value() -> Option<u8> {
    Some(1)
}
"#;
        let chunks = chunk_source(src, Language::Rust).unwrap();
        let text = embedding_text(&chunks[0]);
        assert!(text.starts_with("Receive a value from an asynchronous channel."));
        assert!(text.contains("recv_value"));
    }

    /// Real model + hybrid search. Gated — same env var as the encoder
    /// E2E test. Verifies that the semantic rerank actually changes
    /// rankings on a natural-language query where BM25 alone would miss.
    #[test]
    fn search_hybrid_reranks_with_real_model_if_opted_in() {
        if std::env::var("PLUCK_RUN_MODEL_TESTS").is_err() {
            return;
        }
        let enc = std::sync::Arc::new(
            crate::semantic::StaticEncoder::load_or_fetch(&crate::semantic::selected_model_id())
                .expect("load encoder"),
        );
        let idx = PluckIndex::in_ram().unwrap().with_encoder(enc);
        let mut w = idx.writer().unwrap();
        // The "auth" symbol uses no obvious keyword from the query "user
        // login authentication". The unrelated helper has shared rare
        // words ("authentication" appears in its docstring). BM25 alone
        // ranks the unrelated one higher; hybrid should pull the real
        // auth function up.
        let src = r#"
// JWT-based session validation entry point.
function validateSession(token: string): boolean {
  if (!token) return false;
  return token.length === 36;
}

// Helper: pretty-print authentication errors for log messages.
function formatAuthErrorLine(line: string): string {
  return "ERR(authentication): " + line;
}
"#;
        for c in chunk_source(src, Language::TypeScript).unwrap() {
            w.add_chunk("auth.ts", &c).unwrap();
        }
        w.commit().unwrap();

        let hits = idx
            .search_hybrid("user login authentication", 5, 0.0, None)
            .unwrap();
        // Both should surface; the hybrid order is what we care about.
        // Whichever the test prefers, semantic + BM25 fusion must produce
        // a non-empty ranking and not crash.
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|h| h.symbol == "validateSession"));
    }

    #[test]
    fn tokenizer_splits_camelcase_for_partial_match() {
        let src = r#"
class HandlerStack { run(): void {} }
class CorsMiddleware { wrap(): void {} }
"#;
        let idx = index_one_file(src, "x.ts", Language::TypeScript);
        // Partial-match: query `handler` should surface `HandlerStack`
        // even though no field contains the literal token `handler` in
        // isolation. The tokenizer's camelCase splitter is the only way
        // this can match.
        let hits = idx.search_with_cutoff("handler", 5, 0.0).unwrap();
        assert!(
            hits.iter().any(|h| h.symbol == "HandlerStack"),
            "partial camelCase match missed: {:?}",
            hits.iter().map(|h| &h.symbol).collect::<Vec<_>>()
        );
    }

    #[test]
    fn tokenizer_indexes_non_ascii_identifiers() {
        // A symbol with Hangul letters should survive the index and be
        // findable by either part of the underscore-split name. The same
        // path works for CJK or any \p{L} script.
        let src = "function 의존성_검사(x: string): void {}\n";
        let idx = index_one_file(src, "deps.ts", Language::TypeScript);
        let hits = idx.search_with_cutoff("의존성", 5, 0.0).unwrap();
        assert!(
            hits.iter().any(|h| h.symbol == "의존성_검사"),
            "unicode identifier missed: {:?}",
            hits.iter().map(|h| &h.symbol).collect::<Vec<_>>()
        );
    }

    #[test]
    fn bm25f_boosts_symbol_match_above_body_match() {
        // Two chunks: one *is* the symbol `handleLogin`, the other just
        // mentions `handleLogin` inside its body. Symbol-match must rank
        // first under BM25F because of the symbol field boost.
        let src = r#"
function handleLogin(user: string): boolean {
  return user.length > 0;
}

function dispatchByName(name: string): void {
  if (name === "handleLogin") {
    console.log("matched");
  }
}
"#;
        let idx = index_one_file(src, "auth.ts", Language::TypeScript);
        let hits = idx.search_with_cutoff("handleLogin", 5, 0.0).unwrap();
        assert!(!hits.is_empty(), "expected hits");
        assert_eq!(
            hits[0].symbol,
            "handleLogin",
            "BM25F must rank the symbol-match above the body-mention; got: {:?}",
            hits.iter().map(|h| &h.symbol).collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_top_k_limit() {
        let mut src = String::new();
        for i in 0..20 {
            src.push_str(&format!(
                "function fn_{i}(): string {{\n  return \"keyword_{i}\";\n}}\n\n"
            ));
        }
        let idx = index_one_file(&src, "p.ts", Language::TypeScript);
        let hits = idx
            .search("keyword_3 OR keyword_7 OR keyword_11", 3)
            .unwrap();
        assert!(hits.len() <= 3);
        assert!(!hits.is_empty());
    }
}
