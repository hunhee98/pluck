mod lang;
mod types;

pub use lang::Lang as Language;
pub use types::{Chunk, ChunkKind};

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::{Parser, QueryCursor, StreamingIterator};

/// File-level chunker output: chunks plus the raw import strings extracted
/// from the single tree walk. `imports` is what `pluck.deps` consumes.
#[derive(Debug, Default)]
pub struct ChunkResult {
    pub chunks: Vec<Chunk>,
    pub imports: Vec<String>,
}

pub fn chunk_file(path: &Path) -> Result<Vec<Chunk>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang =
        Language::from_extension(ext).with_context(|| format!("unsupported extension: {ext:?}"))?;
    let src = std::fs::read_to_string(path).with_context(|| format!("failed to read {path:?}"))?;
    chunk_source(&src, lang)
}

pub fn chunk_source(src: &str, lang: Language) -> Result<Vec<Chunk>> {
    chunk_source_with_meta(src, lang).map(|r| r.chunks)
}

pub fn chunk_source_with_meta(src: &str, lang: Language) -> Result<ChunkResult> {
    let Some(query) = lang.compiled_query() else {
        return Ok(ChunkResult::default());
    };

    let ts_lang = lang.ts_language();

    let mut parser = Parser::new();
    parser.set_language(&ts_lang).context("set language")?;

    let tree = parser.parse(src, None).context("parse failed")?;

    if tree.root_node().has_error() {
        tracing::warn!("parse tree contains errors; extracting available chunks");
    }

    let capture_names = query.capture_names();

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), src.as_bytes());

    let lines: Vec<&str> = src.lines().collect();

    // Partial chunk record — callees bucketed in after the single tree walk.
    struct PartialChunk {
        symbol: String,
        kind: ChunkKind,
        start_line: u32,
        end_line: u32,
        start_byte: usize,
        end_byte: usize,
        doc_comment: String,
        content: String,
        signature: String,
    }

    let mut partials: Vec<PartialChunk> = Vec::new();
    let mut all_callees: Vec<(usize, String)> = Vec::new();
    let mut imports: Vec<String> = Vec::new();
    let mut imports_seen: HashSet<String> = HashSet::new();
    // deduplicate: same start byte can appear when a node matches multiple patterns
    let mut seen: HashSet<usize> = HashSet::new();

    while let Some(m) = matches.next() {
        let mut def_node: Option<tree_sitter::Node> = None;
        let mut name_range: Option<std::ops::Range<usize>> = None;
        let mut chunk_kind: Option<ChunkKind> = None;
        let mut callee_node: Option<tree_sitter::Node> = None;
        let mut import_node: Option<tree_sitter::Node> = None;

        for cap in m.captures {
            let cap_name = capture_names[cap.index as usize];
            if let Some(prefix) = cap_name.strip_suffix(".definition") {
                def_node = Some(cap.node);
                chunk_kind = Some(kind_from_prefix(prefix));
            } else if cap_name.ends_with(".name") {
                name_range = Some(cap.node.byte_range());
            } else if cap_name == "callee" {
                callee_node = Some(cap.node);
            } else if cap_name == "import" {
                import_node = Some(cap.node);
            }
        }

        if let Some(node) = import_node {
            // JS/TS/Go imports are string literals like `"./foo"` — strip
            // the surrounding quotes. Rust/Python imports are bare paths
            // (`foo::bar`, `foo.bar`) and trim is a no-op for them.
            let raw = src[node.byte_range()].trim();
            let trimmed = raw
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                .or_else(|| raw.strip_prefix('`').and_then(|s| s.strip_suffix('`')))
                .unwrap_or(raw);
            let normalized: String = trimmed.split_whitespace().collect::<Vec<_>>().join("");
            if !normalized.is_empty() && imports_seen.insert(normalized.clone()) {
                imports.push(normalized);
            }
            continue;
        }

        if let Some(node) = callee_node {
            let text = src[node.byte_range()].trim();
            let normalized: String = text.split_whitespace().collect::<Vec<_>>().join("");
            if !normalized.is_empty() {
                all_callees.push((node.start_byte(), normalized));
            }
            continue;
        }

        let (Some(node), Some(nr), Some(kind)) = (def_node, name_range, chunk_kind) else {
            continue;
        };

        // Python: a function/class_definition nested directly under a
        // decorated_definition is already covered by the outer match.
        if let Some(parent) = node.parent() {
            if parent.kind() == "decorated_definition" {
                continue;
            }
        }

        let start_byte = node.start_byte();
        if !seen.insert(start_byte) {
            continue;
        }
        let end_byte = node.end_byte();

        let content = src[start_byte..end_byte].to_string();
        let doc_comment = leading_doc_comment(&lines, lang, node.start_position().row, &content);

        // signature = node text up to the `body` field's start (if present),
        // so multi-line parameter lists are captured intact.
        let signature = match node.child_by_field_name("body") {
            Some(body) => src[start_byte..body.start_byte()].trim_end().to_string(),
            None => content.lines().next().unwrap_or("").trim_end().to_string(),
        };

        partials.push(PartialChunk {
            symbol: src[nr].to_string(),
            kind,
            start_line: node.start_position().row as u32 + 1,
            end_line: node.end_position().row as u32 + 1,
            start_byte,
            end_byte,
            doc_comment,
            content,
            signature,
        });
    }

    let chunks = partials
        .into_iter()
        .map(|p| {
            let mut seen: HashSet<&str> = HashSet::new();
            let mut callees: Vec<String> = Vec::new();
            for (start, name) in &all_callees {
                if *start >= p.start_byte && *start < p.end_byte && seen.insert(name.as_str()) {
                    callees.push(name.clone());
                }
            }
            Chunk {
                symbol: p.symbol,
                kind: p.kind,
                start_line: p.start_line,
                end_line: p.end_line,
                start_byte: p.start_byte as u32,
                end_byte: p.end_byte as u32,
                doc_comment: p.doc_comment,
                content: p.content,
                signature: p.signature,
                callees,
            }
        })
        .collect();

    Ok(ChunkResult { chunks, imports })
}

fn leading_doc_comment(lines: &[&str], lang: Language, start_row: usize, content: &str) -> String {
    let mut row = start_row;
    let mut collected: Vec<String> = Vec::new();

    while row > 0 {
        let line = lines[row - 1].trim();
        if line.is_empty() {
            break;
        }

        if lang == Language::Rust && line.starts_with("#[") {
            row -= 1;
            continue;
        }

        if let Some(cleaned) = clean_line_doc(lang, line) {
            collected.push(cleaned);
            row -= 1;
            continue;
        }

        if line.ends_with("*/") {
            let (block, next_row) = collect_block_doc(lines, row - 1);
            if !block.is_empty() {
                collected.extend(block.into_iter().rev());
                row = next_row;
                continue;
            }
        }

        break;
    }

    collected.reverse();
    let mut doc = collected
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    if doc.is_empty() {
        doc = leading_doc_in_content(content, lang).unwrap_or_default();
    }

    if doc.is_empty() && lang == Language::Python {
        doc = python_docstring(content).unwrap_or_default();
    }

    doc
}

fn leading_doc_in_content(content: &str, lang: Language) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut row = 0;
    let mut out = Vec::new();

    while row < lines.len() {
        let line = lines[row].trim();
        if line.is_empty() {
            row += 1;
            continue;
        }
        if let Some(cleaned) = clean_line_doc(lang, line) {
            out.push(cleaned);
            row += 1;
            continue;
        }
        if line.starts_with("/**") || line.starts_with("/*") {
            loop {
                let cleaned = lines[row]
                    .trim()
                    .trim_start_matches("/**")
                    .trim_start_matches("/*")
                    .trim_end_matches("*/")
                    .trim_start_matches('*')
                    .trim()
                    .to_string();
                out.push(cleaned);
                if lines[row].trim().ends_with("*/") {
                    break;
                }
                row += 1;
                if row >= lines.len() {
                    break;
                }
            }
        }
        break;
    }

    let doc = out
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!doc.is_empty()).then_some(doc)
}

fn clean_line_doc(lang: Language, line: &str) -> Option<String> {
    match lang {
        Language::Rust => line
            .strip_prefix("///")
            .or_else(|| line.strip_prefix("//!"))
            .map(|s| s.trim().to_string()),
        Language::TypeScript | Language::JavaScript | Language::Go => {
            line.strip_prefix("//").map(|s| s.trim().to_string())
        }
        Language::Python => line.strip_prefix('#').map(|s| s.trim().to_string()),
    }
}

fn collect_block_doc(lines: &[&str], mut row: usize) -> (Vec<String>, usize) {
    let mut out = Vec::new();
    loop {
        let line = lines[row].trim();
        let cleaned = line
            .trim_start_matches("/**")
            .trim_start_matches("/*")
            .trim_end_matches("*/")
            .trim_start_matches('*')
            .trim()
            .to_string();
        out.push(cleaned);

        if line.starts_with("/**") || line.starts_with("/*") || row == 0 {
            return (out, row);
        }
        row -= 1;
    }
}

fn python_docstring(content: &str) -> Option<String> {
    let mut lines = content.lines().skip(1);
    let first = lines.find(|line| !line.trim().is_empty())?.trim();
    let quote = if first.starts_with("\"\"\"") {
        "\"\"\""
    } else if first.starts_with("'''") {
        "'''"
    } else {
        return None;
    };

    let mut out = Vec::new();
    let rest = first.trim_start_matches(quote);
    if let Some((body, _)) = rest.split_once(quote) {
        out.push(body.trim().to_string());
        return Some(out.join("\n"));
    }
    out.push(rest.trim().to_string());

    for line in lines {
        let trimmed = line.trim();
        if let Some((body, _)) = trimmed.split_once(quote) {
            out.push(body.trim().to_string());
            break;
        }
        out.push(trimmed.to_string());
    }

    Some(
        out.into_iter()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn kind_from_prefix(prefix: &str) -> ChunkKind {
    match prefix {
        "function" => ChunkKind::Function,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks_of(src: &str) -> Vec<Chunk> {
        chunk_source(src, Language::TypeScript).expect("chunk_source failed")
    }

    #[test]
    fn test_single_function() {
        let src = r#"
function greet(name: string): string {
  return `Hello, ${name}`;
}
"#;
        let chunks = chunks_of(src);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].symbol, "greet");
        assert_eq!(chunks[0].kind, ChunkKind::Function);
        assert_eq!(chunks[0].start_line, 2);
        assert_eq!(chunks[0].end_line, 4);
    }

    #[test]
    fn test_class_with_methods() {
        let src = r#"
class AuthService {
  private secret: string;

  constructor(secret: string) {
    this.secret = secret;
  }

  async login(user: string): Promise<boolean> {
    return user.length > 0;
  }

  logout(): void {
    this.secret = "";
  }
}
"#;
        let chunks = chunks_of(src);
        // expect: AuthService (class) + constructor (method) + login (method) + logout (method)
        let class_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Class)
            .collect();
        let method_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Method)
            .collect();

        assert_eq!(class_chunks.len(), 1, "expected 1 class chunk");
        assert_eq!(class_chunks[0].symbol, "AuthService");

        assert_eq!(method_chunks.len(), 3, "expected 3 method chunks");
        let names: Vec<&str> = method_chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(names.contains(&"login"), "missing login method");
        assert!(names.contains(&"logout"), "missing logout method");
        assert!(names.contains(&"constructor"), "missing constructor");
    }

    #[test]
    fn test_export_const_arrow_function() {
        let src = r#"
export const handleRequest = async (req: Request): Promise<Response> => {
  return new Response("ok");
};
"#;
        let chunks = chunks_of(src);
        let arrow_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Function)
            .collect();
        assert_eq!(arrow_chunks.len(), 1);
        assert_eq!(arrow_chunks[0].symbol, "handleRequest");
        assert_eq!(arrow_chunks[0].start_line, 2);
    }

    #[test]
    fn test_async_method() {
        let src = r#"
class TokenService {
  async verify(token: string): Promise<boolean> {
    return token !== "";
  }

  async refresh(token: string): Promise<string> {
    return token + "_new";
  }
}
"#;
        let chunks = chunks_of(src);
        let methods: Vec<_> = chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Method)
            .collect();
        assert_eq!(methods.len(), 2);
        let names: Vec<&str> = methods.iter().map(|c| c.symbol.as_str()).collect();
        assert!(names.contains(&"verify"));
        assert!(names.contains(&"refresh"));
        // verify line ranges are non-overlapping
        let verify = methods.iter().find(|c| c.symbol == "verify").unwrap();
        let refresh = methods.iter().find(|c| c.symbol == "refresh").unwrap();
        assert!(verify.end_line < refresh.start_line);
    }

    #[test]
    fn test_generator_function() {
        let src = r#"
function* counter(start: number) {
  let i = start;
  while (true) {
    yield i++;
  }
}
"#;
        let chunks = chunks_of(src);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].symbol, "counter");
        assert_eq!(chunks[0].kind, ChunkKind::Function);
    }

    #[test]
    fn test_interface_captured_as_class() {
        let src = r#"
interface UserRepository {
  findById(id: string): Promise<User | null>;
  save(user: User): Promise<void>;
}
"#;
        let chunks = chunks_of(src);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].symbol, "UserRepository");
        assert_eq!(chunks[0].kind, ChunkKind::Class);
    }

    #[test]
    fn test_anonymous_callbacks_skipped() {
        let src = r#"
const items = [1, 2, 3].map((x) => x * 2);
const filtered = [1, 2, 3].filter(function(x) { return x > 1; });
"#;
        // anonymous arrow and function expression in callbacks → no named chunk
        let chunks = chunks_of(src);
        assert!(
            chunks.is_empty(),
            "expected no chunks for anonymous callbacks, got: {chunks:?}"
        );
    }

    #[test]
    fn test_enum() {
        let src = r#"
enum Direction {
  Up,
  Down,
  Left,
  Right,
}
"#;
        let chunks = chunks_of(src);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].symbol, "Direction");
        assert_eq!(chunks[0].kind, ChunkKind::Enum);
    }

    #[test]
    fn test_line_range_1based() {
        let src = "function a() {\n  return 1;\n}\n";
        let chunks = chunks_of(src);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 3);
    }

    #[test]
    fn test_byte_range_matches_content() {
        let src = "function hello() {}\n";
        let chunks = chunks_of(src);
        assert_eq!(chunks.len(), 1);
        let c = &chunks[0];
        assert_eq!(&src[c.start_byte as usize..c.end_byte as usize], c.content);
    }

    #[test]
    fn test_doc_comment_extracted_for_rust_function() {
        let src = r#"
/// Receives a queued value without blocking the executor.
#[track_caller]
pub async fn recv_value() -> Option<u8> {
    Some(1)
}
"#;
        let chunks = chunk_source(src, Language::Rust).unwrap();
        let c = chunks.iter().find(|c| c.symbol == "recv_value").unwrap();
        assert_eq!(
            c.doc_comment,
            "Receives a queued value without blocking the executor."
        );
    }

    #[test]
    fn test_doc_comment_extracted_for_js_block() {
        let src = r#"
/**
 * Parse a webhook payload into an event.
 */
function decodeWebhook(body) {
  return body;
}
"#;
        let chunks = chunk_source(src, Language::JavaScript).unwrap();
        let c = chunks.iter().find(|c| c.symbol == "decodeWebhook").unwrap();
        assert_eq!(c.doc_comment, "Parse a webhook payload into an event.");
    }

    #[test]
    fn test_python_docstring_extracted() {
        let src = r#"
def normalize_email(value: str) -> str:
    """Normalize an email address before lookup."""
    return value.strip().lower()
"#;
        let chunks = chunk_source(src, Language::Python).unwrap();
        let c = chunks
            .iter()
            .find(|c| c.symbol == "normalize_email")
            .unwrap();
        assert_eq!(c.doc_comment, "Normalize an email address before lookup.");
    }

    // ── Rust ──────────────────────────────────────────────────────────────

    #[test]
    fn test_rust_function_struct_impl() {
        let src = r#"
pub struct Config {
    pub name: String,
}

fn main() {
    println!("hello");
}

impl Config {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}
"#;
        let chunks = chunk_source(src, Language::Rust).unwrap();
        let kinds: Vec<&ChunkKind> = chunks.iter().map(|c| &c.kind).collect();
        let names: Vec<&str> = chunks.iter().map(|c| c.symbol.as_str()).collect();

        assert!(
            kinds.contains(&&ChunkKind::Struct),
            "missing Struct: {chunks:?}"
        );
        assert!(
            kinds.contains(&&ChunkKind::Impl),
            "missing Impl: {chunks:?}"
        );
        assert!(names.contains(&"Config"));
        assert!(names.contains(&"main"));
        assert!(names.contains(&"new"));
    }

    #[test]
    fn test_rust_trait_and_enum() {
        let src = r#"
pub trait Greeter {
    fn greet(&self) -> String;
}

pub enum Status {
    Ok,
    Err(String),
}
"#;
        let chunks = chunk_source(src, Language::Rust).unwrap();
        let by_kind = |k: ChunkKind| {
            chunks
                .iter()
                .find(|c| c.kind == k)
                .cloned()
                .unwrap_or_else(|| panic!("no chunk of kind {k:?}"))
        };
        assert_eq!(by_kind(ChunkKind::Trait).symbol, "Greeter");
        assert_eq!(by_kind(ChunkKind::Enum).symbol, "Status");
    }

    // ── Python ────────────────────────────────────────────────────────────

    #[test]
    fn test_python_function_and_class() {
        let src = r#"
def greet(name: str) -> str:
    return f"Hello, {name}"

class AuthService:
    def __init__(self, secret: str):
        self.secret = secret

    async def login(self, user: str) -> bool:
        return len(user) > 0
"#;
        let chunks = chunk_source(src, Language::Python).unwrap();
        let names: Vec<&str> = chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"AuthService"));
        assert!(names.contains(&"__init__"));
        assert!(names.contains(&"login"));
    }

    #[test]
    fn test_python_decorated_function() {
        let src = r#"
@app.route("/")
def index():
    return "hi"
"#;
        let chunks = chunk_source(src, Language::Python).unwrap();
        // expect exactly one chunk for `index` (decorated, deduped by start_byte)
        let fns: Vec<_> = chunks.iter().filter(|c| c.symbol == "index").collect();
        assert_eq!(fns.len(), 1, "expected one index chunk, got: {chunks:?}");
        // chunk should start at the decorator line (line 2)
        assert_eq!(fns[0].start_line, 2);
    }

    // ── Go ────────────────────────────────────────────────────────────────

    #[test]
    fn test_go_function_method_struct() {
        let src = r#"
package main

type Server struct {
    addr string
}

func NewServer(addr string) *Server {
    return &Server{addr: addr}
}

func (s *Server) Run() error {
    return nil
}
"#;
        let chunks = chunk_source(src, Language::Go).unwrap();
        let names: Vec<&str> = chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(names.contains(&"Server"), "missing Server struct");
        assert!(names.contains(&"NewServer"));
        assert!(names.contains(&"Run"));
        let server = chunks.iter().find(|c| c.symbol == "Server").unwrap();
        assert_eq!(server.kind, ChunkKind::Struct);
        let run = chunks.iter().find(|c| c.symbol == "Run").unwrap();
        assert_eq!(run.kind, ChunkKind::Method);
    }

    #[test]
    fn test_go_interface() {
        let src = r#"
package main

type Reader interface {
    Read(p []byte) (n int, err error)
}
"#;
        let chunks = chunk_source(src, Language::Go).unwrap();
        let r = chunks
            .iter()
            .find(|c| c.symbol == "Reader")
            .expect("Reader missing");
        assert_eq!(r.kind, ChunkKind::Class);
    }

    // ── JavaScript ────────────────────────────────────────────────────────

    #[test]
    fn test_js_function_class_arrow() {
        let src = r#"
function add(a, b) {
  return a + b;
}

class Counter {
  constructor() { this.n = 0; }
  inc() { this.n++; }
}

const square = (x) => x * x;
"#;
        let chunks = chunk_source(src, Language::JavaScript).unwrap();
        let names: Vec<&str> = chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"Counter"));
        assert!(names.contains(&"constructor"));
        assert!(names.contains(&"inc"));
        assert!(names.contains(&"square"));
    }

    #[test]
    fn imports_rust() {
        let src = r#"
use std::collections::HashMap;
use crate::index::PluckIndex;
use super::types::Chunk;
extern crate serde;

fn main() {}
"#;
        let r = chunk_source_with_meta(src, Language::Rust).unwrap();
        assert!(r.imports.iter().any(|i| i.contains("std::collections::HashMap")));
        assert!(r.imports.iter().any(|i| i.contains("crate::index::PluckIndex")));
        assert!(r.imports.iter().any(|i| i.contains("super::types::Chunk")));
        assert!(r.imports.iter().any(|i| i == "serde"));
    }

    #[test]
    fn imports_python() {
        let src = r#"
import os
import json as j
from pathlib import Path
from .local import helper
from ..parent import thing

def main():
    pass
"#;
        let r = chunk_source_with_meta(src, Language::Python).unwrap();
        assert!(r.imports.iter().any(|i| i == "os"));
        assert!(r.imports.iter().any(|i| i.contains("json")));
        assert!(r.imports.iter().any(|i| i == "pathlib"));
        assert!(r.imports.iter().any(|i| i.contains(".local") || i == ".local"));
    }

    #[test]
    fn imports_typescript() {
        let src = r#"
import foo from "./bar";
import { a, b } from "./baz";
import * as ns from "./ns";
import "./side";
const q = require("./req");
const dyn = import("./dyn");
export { x } from "./re-export";
"#;
        let r = chunk_source_with_meta(src, Language::TypeScript).unwrap();
        assert!(r.imports.contains(&"./bar".to_string()));
        assert!(r.imports.contains(&"./baz".to_string()));
        assert!(r.imports.contains(&"./ns".to_string()));
        assert!(r.imports.contains(&"./side".to_string()));
        assert!(r.imports.contains(&"./req".to_string()));
        assert!(r.imports.contains(&"./dyn".to_string()));
        assert!(r.imports.contains(&"./re-export".to_string()));
    }

    #[test]
    fn imports_go() {
        let src = r#"
package main

import "fmt"
import (
    "os"
    "github.com/foo/bar"
)

func main() {}
"#;
        let r = chunk_source_with_meta(src, Language::Go).unwrap();
        assert!(r.imports.contains(&"fmt".to_string()));
        assert!(r.imports.contains(&"os".to_string()));
        assert!(r.imports.contains(&"github.com/foo/bar".to_string()));
    }
}
