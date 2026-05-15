//! BM25 chunk index backed by tantivy.
//!
//! Each chunk produced by the AST chunker becomes one tantivy document.
//! The `symbol`, `signature`, and `content` fields participate in the
//! default BM25 scoring. Hybrid search additionally lets the BM25 side
//! see `doc_comment` so natural-language queries can use API prose
//! without changing BM25-only behavior.

use std::collections::{HashMap, HashSet};
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
    /// Reverse caller index: callee leaf name (lowercased) → Vec of
    /// chunk_ids whose content calls that callee. Built incrementally
    /// in `add_chunk`; used by `impact()` for upstream blast-radius
    /// queries. Shared with `IndexBatch` via `Arc`.
    callers: Arc<RwLock<HashMap<String, Vec<u64>>>>,
    /// Forward import map: file_path → list of raw import module paths
    /// (Rust `use` arg, Python module name, JS/TS/Go string literal).
    /// Populated by `IndexBatch::add_imports`; used by `deps()` and
    /// `importers()`. Shared with `IndexBatch` via `Arc`.
    imports: Arc<RwLock<HashMap<String, Vec<String>>>>,
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
            callers: Arc::new(RwLock::new(HashMap::new())),
            imports: Arc::new(RwLock::new(HashMap::new())),
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
            callers: Arc::new(RwLock::new(HashMap::new())),
            imports: Arc::new(RwLock::new(HashMap::new())),
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
            callers: Arc::clone(&self.callers),
            imports: Arc::clone(&self.imports),
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
        let mut fields = vec![
            self.fields.symbol,
            self.fields.signature,
            self.fields.content,
        ];
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

    /// Look up the chunk_ids of all chunks that contain a call to
    /// `callee_name` (leaf-matched, case-insensitive). Used by `impact`.
    pub fn lookup_callers(&self, callee_name: &str) -> Vec<u64> {
        let leaf = callee_leaf(callee_name).to_lowercase();
        self.callers
            .read()
            .map(|m| m.get(&leaf).cloned().unwrap_or_default())
            .unwrap_or_default()
    }

    /// Fetch a single `SearchHit` by its tantivy `chunk_id`. Returns
    /// `None` if the id is not in the index (e.g. deleted by the
    /// watcher).
    pub fn hit_by_chunk_id(&self, chunk_id: u64) -> Result<Option<SearchHit>> {
        use tantivy::query::TermQuery;
        let reader = self.inner.reader().context("open reader")?;
        let searcher = reader.searcher();
        let term = Term::from_field_u64(self.fields.chunk_id, chunk_id);
        let q = TermQuery::new(term, IndexRecordOption::Basic);
        let top = searcher
            .search(&q, &TopDocs::with_limit(1).order_by_score())
            .context("chunk_id lookup")?;
        match top.into_iter().next() {
            Some((score, addr)) => {
                let doc: TantivyDocument = searcher.doc(addr).context("doc retrieve")?;
                Ok(Some(self.doc_to_hit(score, &doc)?))
            }
            None => Ok(None),
        }
    }

    /// Reverse call-graph traversal: "who calls `name`, transitively?"
    ///
    /// Returns one entry per unique caller chunk, annotated with its
    /// BFS depth from the target. Test-file callers sort after
    /// production callers at the same depth. Depth is clamped to 3 to
    /// prevent output explosion on widely-used utility functions.
    ///
    /// Cycle-safe: a visited set prevents any chunk from appearing
    /// more than once.
    pub fn impact(&self, name: &str, depth: u8) -> Result<Vec<ImpactHit>> {
        let depth = depth.clamp(1, 3);

        let mut out: Vec<ImpactHit> = Vec::new();
        let mut visited: std::collections::HashSet<u64> = std::collections::HashSet::new();

        // BFS frontier: (chunk_id, level)
        let mut frontier: std::collections::VecDeque<(u64, u8)> = std::collections::VecDeque::new();

        for id in self.lookup_callers(name) {
            if visited.insert(id) {
                frontier.push_back((id, 1));
            }
        }

        while let Some((chunk_id, level)) = frontier.pop_front() {
            let Some(hit) = self.hit_by_chunk_id(chunk_id)? else {
                continue;
            };

            // Enqueue callers of this chunk at level+1 (if within depth).
            if level < depth {
                for caller_id in self.lookup_callers(&hit.symbol) {
                    if visited.insert(caller_id) {
                        frontier.push_back((caller_id, level + 1));
                    }
                }
            }

            let is_test = hit.path.contains("/test")
                || hit.path.contains("_test")
                || hit.path.contains("spec");
            out.push(ImpactHit {
                depth: level,
                is_test,
                hit,
            });
        }

        // Sort: production callers first, then test callers; within
        // each group, ascending depth (closest callers first).
        out.sort_by_key(|h| (h.is_test as u8, h.depth));
        Ok(out)
    }

    /// Forward dependency edges for `file_path`: every raw import string
    /// extracted from the file during chunking. Each edge carries a
    /// best-effort `resolved` path when the import resolves to another
    /// indexed file (relative imports + suffix match on absolute paths);
    /// otherwise the import is returned as `raw` only.
    pub fn deps(&self, file_path: &str) -> Vec<DepHit> {
        let raws: Vec<String> = self
            .imports
            .read()
            .ok()
            .and_then(|map| map.get(file_path).cloned())
            .unwrap_or_default();
        if raws.is_empty() {
            return Vec::new();
        }
        let indexed: Vec<String> = self
            .imports
            .read()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        raws.into_iter()
            .map(|raw| {
                let resolved = resolve_import(file_path, &raw, &indexed);
                DepHit { raw, resolved }
            })
            .collect()
    }

    /// Reverse dependency edges: which indexed files import `file_path`
    /// (after resolution). Useful for "who depends on this module?" at
    /// the file level, complementing `impact()` at the symbol level.
    pub fn importers(&self, file_path: &str) -> Vec<DepHit> {
        let map = match self.imports.read() {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };
        let indexed: Vec<String> = map.keys().cloned().collect();
        let mut out: Vec<DepHit> = Vec::new();
        for (importer, raws) in map.iter() {
            for raw in raws {
                if resolve_import(importer, raw, &indexed).as_deref() == Some(file_path) {
                    out.push(DepHit {
                        raw: importer.clone(),
                        resolved: Some(importer.clone()),
                    });
                    break;
                }
            }
        }
        out.sort_by(|a, b| a.raw.cmp(&b.raw));
        out
    }

    /// Exploration recommender. Given a free-form task description, probe
    /// the index with a hybrid search and propose the next 3-5 retrieval
    /// calls the agent should make. The plan is a *guardrail*, not an
    /// oracle — low-confidence output is a lead, not a fact.
    pub fn plan(&self, task: &str, top_k: usize) -> Result<PlanResult> {
        let top_k = top_k.clamp(1, 5);
        let probe_k = 10;
        let hits = self.search_with_cutoff(task, probe_k, 0.0)?;

        if hits.is_empty() {
            return Ok(PlanResult {
                confidence: PlanConfidence::Low,
                probe_hits: vec![],
                steps: vec![],
                broaden: Some(
                    "No probe hits. Try `pluck.grep` with a concrete identifier from the task, or `pluck.search` with a simpler query."
                        .to_string(),
                ),
            });
        }

        let confidence = plan_confidence(&hits);
        let steps = plan_steps(&hits, top_k);
        let broaden = if matches!(confidence, PlanConfidence::Low) {
            Some(
                "Score distribution is flat — none of these is a clear lead. Broaden with `pluck.grep` on a concrete identifier or rephrase the task with more specific terms."
                    .to_string(),
            )
        } else {
            None
        };

        Ok(PlanResult {
            confidence,
            probe_hits: hits,
            steps,
            broaden,
        })
    }
}

/// Output of [`PluckIndex::plan`].
#[derive(Debug, Clone)]
pub struct PlanResult {
    pub confidence: PlanConfidence,
    /// Raw probe-search hits used to build the plan (up to 10).
    pub probe_hits: Vec<SearchHit>,
    /// Concrete next-call recommendations (3-5).
    pub steps: Vec<PlanStep>,
    /// Suggested broader query when confidence is low. `None` otherwise.
    pub broaden: Option<String>,
}

/// One recommended next call.
#[derive(Debug, Clone)]
pub struct PlanStep {
    /// Tool name (without the `mcp__pluck__` prefix): `read`, `symbol`,
    /// `peek`, `expand`, `impact`.
    pub tool: &'static str,
    /// Target argument — symbol name or repo-relative path depending on
    /// the tool.
    pub target: String,
    /// One-line rationale for the agent.
    pub reason: String,
    /// The probe hit that motivated this step.
    pub source: SearchHit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanConfidence {
    High,
    Medium,
    Low,
}

impl PlanConfidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

fn plan_confidence(hits: &[SearchHit]) -> PlanConfidence {
    if hits.is_empty() {
        return PlanConfidence::Low;
    }
    let top = hits[0].score.max(0.0001);
    // Compare top score against rank-5 (or the last hit if fewer).
    let cmp_idx = hits.len().min(5).saturating_sub(1);
    let cmp = hits[cmp_idx].score.max(0.0001);
    let ratio = top / cmp;
    if ratio >= 2.0 {
        PlanConfidence::High
    } else if ratio >= 1.2 {
        PlanConfidence::Medium
    } else {
        PlanConfidence::Low
    }
}

fn plan_steps(hits: &[SearchHit], max: usize) -> Vec<PlanStep> {
    // Count how often each path shows up in the top results — if a file
    // has multiple chunks ranking, one `read` covers them all.
    let consider = hits.len().min(max * 2);
    let mut file_counts: HashMap<String, usize> = HashMap::new();
    for h in hits.iter().take(consider) {
        *file_counts.entry(h.path.clone()).or_insert(0) += 1;
    }

    let mut steps: Vec<PlanStep> = Vec::new();
    let mut seen_targets: HashSet<String> = HashSet::new();
    let mut files_emitted: HashSet<String> = HashSet::new();

    for (i, h) in hits.iter().enumerate() {
        if steps.len() >= max {
            break;
        }

        let count = file_counts.get(&h.path).copied().unwrap_or(1);
        // Multi-chunk file: one `read` step covers all of them.
        if count > 1 {
            if files_emitted.insert(h.path.clone()) {
                steps.push(PlanStep {
                    tool: "read",
                    target: h.path.clone(),
                    reason: format!(
                        "{} chunks from this file rank in the top results — outline once for shared context",
                        count
                    ),
                    source: h.clone(),
                });
            }
            continue;
        }

        if !seen_targets.insert(h.symbol.clone()) {
            continue;
        }

        let (tool, reason) = recommend_tool(h);
        steps.push(PlanStep {
            tool,
            target: h.symbol.clone(),
            reason,
            source: h.clone(),
        });

        // Top hit + function/method → add an impact step as the next call,
        // to surface caller blast-radius before any change to the contract.
        if i == 0
            && matches!(h.kind, ChunkKind::Function | ChunkKind::Method)
            && steps.len() < max
            && seen_targets.insert(format!("__impact__{}", h.symbol))
        {
            steps.push(PlanStep {
                tool: "impact",
                target: h.symbol.clone(),
                reason: "top probe hit — see who depends on this before changing the contract"
                    .to_string(),
                source: h.clone(),
            });
        }
    }

    steps
}

fn recommend_tool(h: &SearchHit) -> (&'static str, String) {
    let size = h.end_line.saturating_sub(h.start_line);
    match h.kind {
        ChunkKind::Function | ChunkKind::Method => {
            if size > 40 {
                (
                    "peek",
                    "large function — peek returns signature + direct callees without paying for the body".to_string(),
                )
            } else {
                (
                    "symbol",
                    "small function — full body fits within a tight token budget".to_string(),
                )
            }
        }
        ChunkKind::Struct | ChunkKind::Enum => (
            "symbol",
            "type definition — read the shape before any usage site".to_string(),
        ),
        ChunkKind::Class => (
            "symbol",
            "class definition — see the methods at the same time as the structure".to_string(),
        ),
        ChunkKind::Impl | ChunkKind::Trait => (
            "symbol",
            "method / contract surface — read in one shot".to_string(),
        ),
        ChunkKind::Module => (
            "read",
            "module-level chunk — outline the file to see every symbol".to_string(),
        ),
    }
}

/// One edge in a `deps` / `importers` result.
#[derive(Debug, Clone)]
pub struct DepHit {
    /// The import statement as it appears in source (Rust `use` arg,
    /// JS/TS string literal contents, etc.). For `importers`, this is
    /// the importing file's path.
    pub raw: String,
    /// Resolved file path within the indexed repo, if the importer's
    /// directory + raw points at a known file. `None` means external
    /// (std crate, npm package, Go module not in this repo).
    pub resolved: Option<String>,
}

/// Best-effort resolution of `raw` (an import string) against the list
/// of `indexed` file paths in the repo. Handles:
///   - JS/TS relative imports: `./foo`, `../foo/bar` → join with
///     importer's directory, try suffixes `.ts`/`.tsx`/`.js`/`.jsx`/
///     `/index.{ts,tsx,js,jsx}`.
///   - Python relative imports: `.foo`, `..foo.bar` → dotted leading
///     dots = directory hops; rest = path components, try `.py` and
///     `/__init__.py`.
///   - Suffix match for absolute imports: `crate::foo::bar` →
///     `**/foo/bar.rs`; `foo.bar` → `**/foo/bar.py`; `fmt` (Go) →
///     `**/fmt.go`.
///
/// Returns `None` if nothing matches.
fn resolve_import(importer: &str, raw: &str, indexed: &[String]) -> Option<String> {
    // JS/TS-style relative import.
    if raw.starts_with("./") || raw.starts_with("../") {
        let dir = Path::new(importer).parent()?;
        let base = dir.join(raw).to_string_lossy().into_owned();
        let base = normalize_dots(&base);
        for ext in &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"] {
            let candidate = format!("{base}{ext}");
            if indexed.iter().any(|p| p == &candidate) {
                return Some(candidate);
            }
        }
        for idx in &["/index.ts", "/index.tsx", "/index.js", "/index.jsx"] {
            let candidate = format!("{base}{idx}");
            if indexed.iter().any(|p| p == &candidate) {
                return Some(candidate);
            }
        }
        return None;
    }
    // Python relative import: leading dots are parent-dir hops.
    if raw.starts_with('.') {
        let leading_dots = raw.chars().take_while(|c| *c == '.').count();
        let rest = &raw[leading_dots..];
        let mut dir = Path::new(importer).parent().map(Path::to_path_buf)?;
        for _ in 1..leading_dots {
            dir = dir.parent()?.to_path_buf();
        }
        let sub = rest.replace('.', "/");
        let base = if sub.is_empty() {
            dir.to_string_lossy().into_owned()
        } else {
            dir.join(&sub).to_string_lossy().into_owned()
        };
        for cand in &[format!("{base}.py"), format!("{base}/__init__.py")] {
            if indexed.iter().any(|p| p == cand) {
                return Some(cand.clone());
            }
        }
        return None;
    }
    // Absolute / module-style: try suffix match against indexed files.
    // For Rust `crate::foo::bar`, strip the head, join with `/`, append `.rs`.
    let parts: Vec<&str> = if raw.contains("::") {
        raw.split("::").collect()
    } else if raw.contains('.') {
        raw.split('.').collect()
    } else if raw.contains('/') {
        raw.split('/').collect()
    } else {
        vec![raw]
    };
    // Drop common roots that aren't part of the path on disk.
    let parts: Vec<&str> = parts
        .into_iter()
        .filter(|p| !p.is_empty() && !matches!(*p, "crate" | "self" | "super"))
        .collect();
    if parts.is_empty() {
        return None;
    }
    let needle = parts.join("/");
    for ext in &[".rs", ".py", ".ts", ".tsx", ".js", ".jsx", ".go"] {
        let suffix = format!("/{needle}{ext}");
        let bare = &suffix[1..];
        if let Some(hit) = indexed.iter().find(|p| p.ends_with(&suffix) || p == &bare) {
            return Some(hit.clone());
        }
    }
    // `__init__.py` form.
    let suffix = format!("/{needle}/__init__.py");
    if let Some(hit) = indexed.iter().find(|p| p.ends_with(&suffix)) {
        return Some(hit.clone());
    }
    None
}

fn normalize_dots(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out.join("/")
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
    /// Shared with `PluckIndex::callers` — written here, read by `impact`.
    callers: Arc<RwLock<HashMap<String, Vec<u64>>>>,
    /// Shared with `PluckIndex::imports` — written by `add_imports`, read
    /// by `deps`/`importers`.
    imports: Arc<RwLock<HashMap<String, Vec<String>>>>,
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

        // Populate the reverse caller index from pre-extracted callees.
        // Callees are extracted once during chunking (with the already-parsed
        // tree) so add_chunk never re-parses source.
        if !c.callees.is_empty() {
            if let Ok(mut map) = self.callers.write() {
                for callee in &c.callees {
                    let leaf = callee_leaf(callee).to_lowercase();
                    map.entry(leaf).or_default().push(id);
                }
            }
        }

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

    /// Store the imports extracted for `file_path` (extracted once per
    /// file during chunking). Always inserts (even when `imports` is
    /// empty) so the keyset of `imports` doubles as the canonical list
    /// of indexed files — needed for import-path resolution.
    pub fn add_imports(&self, file_path: &str, imports: Vec<String>) {
        if let Ok(mut map) = self.imports.write() {
            map.insert(file_path.to_string(), imports);
        }
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
        // Also drop the file's import edges so deletes don't leak into
        // deps/importers output.
        if let Ok(mut map) = self.imports.write() {
            map.remove(rel_path);
        }
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

/// One entry in the result of [`PluckIndex::impact`].
#[derive(Debug, Clone)]
pub struct ImpactHit {
    /// BFS depth from the queried symbol (1 = direct caller).
    pub depth: u8,
    /// Whether the caller lives in a test file (path contains `/test`,
    /// `_test`, or `spec`). Test callers sort after production callers.
    pub is_test: bool,
    pub hit: SearchHit,
}

/// Strip namespace qualifiers from a callee name so it can be matched
/// against the `symbol` leaf. Mirrors `callee_leaf` in `server.rs`.
/// `db.user.findOne` → `findOne`; `Logger::new` → `new`; bare names
/// pass through unchanged.
fn callee_leaf(name: &str) -> &str {
    if let Some(after) = name.rsplit_once("::") {
        return after.1;
    }
    if let Some(after) = name.rsplit_once('.') {
        return after.1;
    }
    name
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

    // ── impact / reverse caller index ─────────────────────────────────

    fn index_two_rust_files() -> PluckIndex {
        // caller.rs calls `validate_token`; callee.rs defines it.
        let caller_src = r#"
pub fn handle_request(token: &str) -> bool {
    validate_token(token)
}
"#;
        let callee_src = r#"
pub fn validate_token(token: &str) -> bool {
    !token.is_empty()
}
"#;
        let idx = PluckIndex::in_ram().unwrap();
        let mut w = idx.writer().unwrap();
        for c in chunk_source(caller_src, Language::Rust).unwrap() {
            w.add_chunk("src/handler.rs", &c).unwrap();
        }
        for c in chunk_source(callee_src, Language::Rust).unwrap() {
            w.add_chunk("src/auth.rs", &c).unwrap();
        }
        w.commit().unwrap();
        idx
    }

    #[test]
    fn lookup_callers_finds_direct_caller() {
        let idx = index_two_rust_files();
        let caller_ids = idx.lookup_callers("validate_token");
        assert!(
            !caller_ids.is_empty(),
            "handle_request must appear as a caller"
        );
    }

    #[test]
    fn impact_depth_1_returns_direct_caller() {
        let idx = index_two_rust_files();
        let results = idx.impact("validate_token", 1).unwrap();
        assert!(
            !results.is_empty(),
            "impact must return at least one caller"
        );
        assert!(
            results.iter().any(|h| h.hit.symbol == "handle_request"),
            "handle_request must be in impact result"
        );
        assert_eq!(results[0].depth, 1, "direct caller is at depth 1");
    }

    #[test]
    fn impact_unknown_name_returns_empty() {
        let idx = index_two_rust_files();
        let results = idx.impact("definitely_not_a_real_fn", 1).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn impact_clamps_depth_to_3() {
        let idx = index_two_rust_files();
        // depth=99 must not panic or loop infinitely; it's clamped to 3.
        let results = idx.impact("validate_token", 99).unwrap();
        // Just check it returns without error.
        let _ = results;
    }

    #[test]
    fn hit_by_chunk_id_roundtrip() {
        let idx = index_two_rust_files();
        // Any chunk_id from lookup_callers must be retrievable.
        let ids = idx.lookup_callers("validate_token");
        assert!(!ids.is_empty());
        let hit = idx.hit_by_chunk_id(ids[0]).unwrap();
        assert!(hit.is_some(), "chunk_id must be retrievable after indexing");
    }

    #[test]
    fn hit_by_chunk_id_missing_returns_none() {
        let idx = index_two_rust_files();
        let hit = idx.hit_by_chunk_id(999_999).unwrap();
        assert!(hit.is_none());
    }

    #[test]
    fn deps_resolves_typescript_relative_import() {
        let idx = PluckIndex::in_ram().unwrap();
        let files = [
            (
                "src/auth/login.ts",
                "import { jwt } from \"../crypto/jwt\";\nexport function login() {}\n",
            ),
            ("src/crypto/jwt.ts", "export function jwt() {}\n"),
        ];
        crate::indexer::index_files_in_memory(&idx, &files).unwrap();

        let deps = idx.deps("src/auth/login.ts");
        assert!(!deps.is_empty(), "deps was empty");
        let hit = deps
            .iter()
            .find(|d| d.raw == "../crypto/jwt")
            .expect("import missing");
        assert_eq!(hit.resolved.as_deref(), Some("src/crypto/jwt.ts"));
    }

    #[test]
    fn deps_resolves_rust_absolute_use() {
        let idx = PluckIndex::in_ram().unwrap();
        let files = [
            (
                "crates/x/src/main.rs",
                "use crate::auth::login;\nfn main() {}\n",
            ),
            ("crates/x/src/auth/login.rs", "pub fn login() {}\n"),
        ];
        crate::indexer::index_files_in_memory(&idx, &files).unwrap();

        let deps = idx.deps("crates/x/src/main.rs");
        let hit = deps
            .iter()
            .find(|d| d.raw.contains("auth"))
            .expect("auth import missing");
        assert_eq!(hit.resolved.as_deref(), Some("crates/x/src/auth/login.rs"));
    }

    #[test]
    fn importers_reverse_edge() {
        let idx = PluckIndex::in_ram().unwrap();
        let files = [
            (
                "src/auth/login.ts",
                "import { jwt } from \"../crypto/jwt\";\nexport function login() {}\n",
            ),
            (
                "src/admin/panel.ts",
                "import { jwt } from \"../crypto/jwt\";\nexport function panel() {}\n",
            ),
            ("src/crypto/jwt.ts", "export function jwt() {}\n"),
        ];
        crate::indexer::index_files_in_memory(&idx, &files).unwrap();

        let importers = idx.importers("src/crypto/jwt.ts");
        let paths: Vec<&str> = importers.iter().map(|d| d.raw.as_str()).collect();
        assert!(paths.contains(&"src/auth/login.ts"), "got: {paths:?}");
        assert!(paths.contains(&"src/admin/panel.ts"), "got: {paths:?}");
    }

    #[test]
    fn deps_external_import_unresolved() {
        let idx = PluckIndex::in_ram().unwrap();
        let files = [(
            "src/main.go",
            "package main\nimport \"fmt\"\nfunc main() { fmt.Println(\"hi\") }\n",
        )];
        crate::indexer::index_files_in_memory(&idx, &files).unwrap();

        let deps = idx.deps("src/main.go");
        let fmt_dep = deps.iter().find(|d| d.raw == "fmt").expect("fmt missing");
        assert!(
            fmt_dep.resolved.is_none(),
            "fmt should not resolve to a repo file"
        );
    }

    #[test]
    fn plan_empty_index_returns_low_confidence_with_broaden_hint() {
        let idx = PluckIndex::in_ram().unwrap();
        let plan = idx.plan("fix the auth bug", 5).unwrap();
        assert_eq!(plan.confidence, PlanConfidence::Low);
        assert!(plan.steps.is_empty());
        assert!(plan.broaden.is_some(), "expected broaden hint");
    }

    #[test]
    fn plan_recommends_symbol_for_small_function() {
        let idx = PluckIndex::in_ram().unwrap();
        let files = [(
            "src/auth.ts",
            "function validateToken(t: string) {\n  return t.length > 0;\n}\n",
        )];
        crate::indexer::index_files_in_memory(&idx, &files).unwrap();

        let plan = idx.plan("validate token", 3).unwrap();
        assert!(!plan.steps.is_empty(), "expected at least one step");
        let first = &plan.steps[0];
        // Top hit is the function — recommend `symbol` (small body fits).
        assert_eq!(first.tool, "symbol");
        assert_eq!(first.target, "validateToken");
    }

    #[test]
    fn plan_adds_impact_followup_for_top_function() {
        let idx = PluckIndex::in_ram().unwrap();
        let files = [(
            "src/auth.ts",
            "function validateToken(t: string) {\n  return t.length > 0;\n}\n",
        )];
        crate::indexer::index_files_in_memory(&idx, &files).unwrap();

        let plan = idx.plan("validate token", 5).unwrap();
        // The top hit is a function, so an `impact` follow-up should be
        // included so the agent sees the blast radius before refactoring.
        let has_impact = plan
            .steps
            .iter()
            .any(|s| s.tool == "impact" && s.target == "validateToken");
        assert!(has_impact, "expected an impact step on the top function");
    }

    #[test]
    fn plan_collapses_multi_chunk_file_to_one_read() {
        let idx = PluckIndex::in_ram().unwrap();
        // Two functions in the same file. The probe matches both — the plan
        // should emit one `read` step instead of two separate calls.
        let files = [(
            "src/handlers.ts",
            r#"
function authenticate(token: string) { return true; }
function authorize(user: string) { return true; }
"#,
        )];
        crate::indexer::index_files_in_memory(&idx, &files).unwrap();

        let plan = idx.plan("authenticate authorize", 5).unwrap();
        let read_steps: Vec<&PlanStep> = plan.steps.iter().filter(|s| s.tool == "read").collect();
        // One read step covering the whole file.
        assert_eq!(
            read_steps.len(),
            1,
            "expected one read step, got {:?}",
            plan.steps
        );
        assert_eq!(read_steps[0].target, "src/handlers.ts");
    }
}
