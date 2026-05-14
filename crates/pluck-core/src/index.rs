//! BM25 chunk index backed by tantivy.
//!
//! Each chunk produced by the AST chunker becomes one tantivy document.
//! The `symbol`, `signature`, and `content` fields participate in the
//! default BM25 scoring; everything else is stored only for retrieval.

use std::path::Path;

use anyhow::{Context, Result};
use tantivy::{
    collector::TopDocs,
    doc,
    query::QueryParser,
    schema::{Field, Schema, Value, FAST, INDEXED, STORED, STRING, TEXT},
    Index as TantivyIndex, IndexWriter, TantivyDocument, Term,
};

use crate::chunker::{Chunk, ChunkKind};

const WRITER_HEAP_BYTES: usize = 50_000_000;

pub struct PluckIndex {
    inner: TantivyIndex,
    fields: Fields,
}

#[derive(Clone, Copy)]
struct Fields {
    chunk_id: Field,
    path: Field,
    symbol: Field,
    kind: Field,
    start_line: Field,
    end_line: Field,
    signature: Field,
    content: Field,
}

fn build_schema() -> (Schema, Fields) {
    let mut sb = Schema::builder();
    let chunk_id = sb.add_u64_field("chunk_id", INDEXED | STORED | FAST);
    let path = sb.add_text_field("path", STRING | STORED);
    let symbol = sb.add_text_field("symbol", TEXT | STORED);
    let kind = sb.add_text_field("kind", STRING | STORED);
    let start_line = sb.add_u64_field("start_line", STORED);
    let end_line = sb.add_u64_field("end_line", STORED);
    let signature = sb.add_text_field("signature", TEXT | STORED);
    let content = sb.add_text_field("content", TEXT | STORED);
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
            signature,
            content,
        },
    )
}

impl PluckIndex {
    pub fn in_ram() -> Result<Self> {
        let (schema, fields) = build_schema();
        let inner = TantivyIndex::create_in_ram(schema);
        Ok(Self { inner, fields })
    }

    pub fn open_or_create(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).ok();
        let (schema, fields) = build_schema();
        let inner = match TantivyIndex::open_in_dir(dir) {
            Ok(idx) => idx,
            Err(_) => TantivyIndex::create_in_dir(dir, schema).context("create tantivy dir")?,
        };
        Ok(Self { inner, fields })
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
        })
    }

    pub fn search(&self, query_str: &str, k: usize) -> Result<Vec<SearchHit>> {
        self.search_with_cutoff(query_str, k, 0.0)
    }

    /// Exact lookup by symbol name (the `symbol` field).
    ///
    /// `name` is matched as a term against the tantivy default tokenizer
    /// applied to the symbol field. Returns every chunk whose symbol
    /// matches; an optional `path_contains` filter narrows ambiguous
    /// matches (e.g. when two files both define `handleLogin`).
    pub fn lookup_symbol(
        &self,
        name: &str,
        path_contains: Option<&str>,
    ) -> Result<Vec<SearchHit>> {
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
                && path_contains
                    .map(|p| hit.path.contains(p))
                    .unwrap_or(true)
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
        let reader = self.inner.reader().context("open reader")?;
        let searcher = reader.searcher();
        let qp = QueryParser::for_index(
            &self.inner,
            vec![self.fields.symbol, self.fields.signature, self.fields.content],
        );
        let query = qp.parse_query(query_str).context("parse query")?;
        let top = searcher
            .search(&query, &TopDocs::with_limit(k).order_by_score())
            .context("search")?;

        let threshold = top
            .first()
            .map(|(s, _)| s * cutoff_frac)
            .unwrap_or(0.0);

        let mut hits = Vec::with_capacity(top.len());
        for (score, addr) in top {
            if score < threshold {
                break;
            }
            let doc: TantivyDocument = searcher.doc(addr).context("doc retrieve")?;
            hits.push(self.doc_to_hit(score, &doc)?);
        }
        Ok(hits)
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

pub struct IndexBatch {
    writer: IndexWriter,
    fields: Fields,
    next_chunk_id: u64,
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
                self.fields.signature => c.signature.as_str(),
                self.fields.content => c.content.as_str(),
            ))
            .context("add_document")?;
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
    fn search_top_k_limit() {
        let mut src = String::new();
        for i in 0..20 {
            src.push_str(&format!(
                "function fn_{i}(): string {{\n  return \"keyword_{i}\";\n}}\n\n"
            ));
        }
        let idx = index_one_file(&src, "p.ts", Language::TypeScript);
        let hits = idx.search("keyword_3 OR keyword_7 OR keyword_11", 3).unwrap();
        assert!(hits.len() <= 3);
        assert!(!hits.is_empty());
    }
}
