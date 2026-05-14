mod lang;
mod types;

pub use lang::Lang as Language;
pub use types::{Chunk, ChunkKind};

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::{Parser, Query, QueryCursor};

pub fn chunk_file(path: &Path) -> Result<Vec<Chunk>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = Language::from_extension(ext)
        .with_context(|| format!("unsupported extension: {ext:?}"))?;
    let src = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {path:?}"))?;
    chunk_source(&src, lang)
}

pub fn chunk_source(src: &str, lang: Language) -> Result<Vec<Chunk>> {
    let query_src = lang.query_str();
    if query_src.is_empty() {
        return Ok(Vec::new());
    }

    let ts_lang = lang.ts_language();

    let mut parser = Parser::new();
    parser.set_language(&ts_lang).context("set language")?;

    let tree = parser.parse(src, None).context("parse failed")?;

    if tree.root_node().has_error() {
        tracing::warn!("parse tree contains errors; extracting available chunks");
    }

    let query = Query::new(&ts_lang, query_src).context("compile query")?;
    let capture_names = query.capture_names();

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&query, tree.root_node(), src.as_bytes());

    let mut chunks: Vec<Chunk> = Vec::new();
    // deduplicate: same start byte can appear when a node matches multiple patterns
    let mut seen: HashSet<usize> = HashSet::new();

    for m in matches {
        let mut def_node: Option<tree_sitter::Node> = None;
        let mut name_range: Option<std::ops::Range<usize>> = None;
        let mut chunk_kind: Option<ChunkKind> = None;

        for cap in m.captures {
            let cap_name = capture_names[cap.index as usize];
            if let Some(prefix) = cap_name.strip_suffix(".definition") {
                def_node = Some(cap.node);
                chunk_kind = Some(kind_from_prefix(prefix));
            } else if cap_name.ends_with(".name") {
                name_range = Some(cap.node.byte_range());
            }
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

        // 1 copy of the source slice into owned content
        let content = src[start_byte..end_byte].to_string();

        // signature = node text up to the `body` field's start (if present),
        // so multi-line parameter lists are captured intact. Falls back to
        // first line for nodes without a body field (e.g. type aliases).
        let signature = match node.child_by_field_name("body") {
            Some(body) => src[start_byte..body.start_byte()].trim_end().to_string(),
            None => content.lines().next().unwrap_or("").trim_end().to_string(),
        };

        let symbol = src[nr].to_string();

        chunks.push(Chunk {
            symbol,
            kind,
            start_line: node.start_position().row as u32 + 1,
            end_line: node.end_position().row as u32 + 1,
            start_byte: start_byte as u32,
            end_byte: end_byte as u32,
            content,
            signature,
        });
    }

    Ok(chunks)
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
        let class_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Class).collect();
        let method_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Method).collect();

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
        let arrow_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Function).collect();
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
        let methods: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Method).collect();
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

        assert!(kinds.contains(&&ChunkKind::Struct), "missing Struct: {chunks:?}");
        assert!(kinds.contains(&&ChunkKind::Impl), "missing Impl: {chunks:?}");
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
        let r = chunks.iter().find(|c| c.symbol == "Reader").expect("Reader missing");
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
}
