mod config;
mod dockerfile;
mod lang;
mod shell;
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
    pub parse_errors: bool,
}

pub fn chunk_file(path: &Path) -> Result<Vec<Chunk>> {
    let lang = Language::from_path(path).with_context(|| format!("unsupported path: {path:?}"))?;
    let src = std::fs::read_to_string(path).with_context(|| format!("failed to read {path:?}"))?;
    chunk_source_with_meta_for_path(&src, lang, path).map(|r| r.chunks)
}

pub fn chunk_source(src: &str, lang: Language) -> Result<Vec<Chunk>> {
    chunk_source_with_meta(src, lang).map(|r| r.chunks)
}

pub fn chunk_source_with_meta(src: &str, lang: Language) -> Result<ChunkResult> {
    chunk_source_with_meta_labeled(src, lang, None)
}

pub fn chunk_source_with_meta_for_path(
    src: &str,
    lang: Language,
    path: &Path,
) -> Result<ChunkResult> {
    let label = path.to_string_lossy();
    chunk_source_with_meta_labeled(src, lang, Some(label.as_ref()))
}

fn chunk_source_with_meta_labeled(
    src: &str,
    lang: Language,
    source_path: Option<&str>,
) -> Result<ChunkResult> {
    if lang.is_config_format() {
        return Ok(config::chunk_config_source(src, lang));
    }
    if lang == Language::Dockerfile {
        return Ok(dockerfile::chunk_dockerfile_source(src));
    }
    if lang == Language::Shell {
        return Ok(shell::chunk_shell_source(src));
    }

    let Some(query) = lang.compiled_query() else {
        return Ok(ChunkResult::default());
    };

    let ts_lang = lang.ts_language();

    let mut parser = Parser::new();
    parser.set_language(&ts_lang).context("set language")?;

    let tree = parser.parse(src, None).context("parse failed")?;

    let parse_errors = tree.root_node().has_error();
    if parse_errors {
        if let Some(path) = source_path {
            tracing::warn!(
                path,
                "parse tree contains errors; extracting available chunks"
            );
        } else {
            tracing::warn!("parse tree contains errors; extracting available chunks");
        }
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
        let raw_symbol = src[nr].to_string();
        let symbol = normalize_symbol(lang, &raw_symbol, &content);
        if !should_emit_chunk(lang, &kind, &symbol, &content) {
            continue;
        }
        let doc_comment = leading_doc_comment(&lines, lang, node.start_position().row, &content);

        // signature = node text up to the `body` field's start (if present),
        // so multi-line parameter lists are captured intact.
        let signature = if lang == Language::Html {
            html_signature(&content)
        } else if matches!(lang, Language::Css | Language::Scss) {
            css_signature(&content)
        } else if matches!(lang, Language::Markdown | Language::Mdx) {
            markdown_signature(&content)
        } else {
            match node.child_by_field_name("body") {
                Some(body) => src[start_byte..body.start_byte()].trim_end().to_string(),
                None => content.lines().next().unwrap_or("").trim_end().to_string(),
            }
        };

        partials.push(PartialChunk {
            symbol,
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

    Ok(ChunkResult {
        chunks,
        imports,
        parse_errors,
    })
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
        // Swift doc comments are `///` (Rust-style); plain `//` is an ordinary
        // comment, so it must not be treated as a doc line.
        Language::Swift => line.strip_prefix("///").map(|s| s.trim().to_string()),
        Language::TypeScript
        | Language::Tsx
        | Language::JavaScript
        | Language::Go
        | Language::Java
        | Language::Kotlin
        | Language::C
        | Language::Cpp
        | Language::Scss => line.strip_prefix("//").map(|s| s.trim().to_string()),
        Language::Python | Language::Ruby => {
            line.strip_prefix('#').map(|s| s.trim().to_string())
        }
        Language::Sql => line.strip_prefix("--").map(|s| s.trim().to_string()),
        Language::Hcl => line
            .strip_prefix("//")
            .or_else(|| line.strip_prefix('#'))
            .map(|s| s.trim().to_string()),
        Language::Html => line
            .strip_prefix("<!--")
            .and_then(|s| s.strip_suffix("-->"))
            .map(|s| s.trim().to_string()),
        Language::Css
        | Language::Markdown
        | Language::Mdx
        | Language::Json
        | Language::Yaml
        | Language::Toml
        | Language::Dockerfile
        | Language::Shell => None,
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

fn normalize_symbol(lang: Language, raw_symbol: &str, content: &str) -> String {
    if matches!(lang, Language::Css | Language::Scss) {
        return normalize_css_symbol(raw_symbol, content);
    }

    if matches!(lang, Language::Markdown | Language::Mdx) {
        return normalize_markdown_symbol(raw_symbol, content);
    }

    if lang == Language::Hcl {
        return normalize_hcl_symbol(content);
    }

    if lang != Language::Html {
        return raw_symbol.to_string();
    }

    let tag = raw_symbol.trim().to_ascii_lowercase();
    if let Some(id) = html_non_empty_attr_value(content, "id") {
        return format!("{tag}#{id}");
    }
    if let Some(name) = html_non_empty_attr_value(content, "data-component")
        .or_else(|| html_non_empty_attr_value(content, "data-controller"))
        .or_else(|| html_non_empty_attr_value(content, "data-testid"))
        .or_else(|| html_non_empty_attr_value(content, "data-test"))
    {
        return format!("{tag}[{name}]");
    }
    if tag == "script" {
        if let Some(src) = html_non_empty_attr_value(content, "src") {
            return format!("script[src={src}]");
        }
        if let Some(typ) = html_non_empty_attr_value(content, "type") {
            return format!("script[type={typ}]");
        }
    }
    if tag == "style" && html_attr_value(content, "scoped").is_some() {
        return "style[scoped]".to_string();
    }

    tag
}

fn should_emit_chunk(lang: Language, kind: &ChunkKind, symbol: &str, content: &str) -> bool {
    if lang != Language::Html {
        return true;
    }
    if kind != &ChunkKind::Module {
        return true;
    }

    let tag = html_tag_from_symbol(symbol);
    is_semantic_html_tag(tag)
        || tag.contains('-')
        || html_non_empty_attr_value(content, "id").is_some()
        || html_non_empty_attr_value(content, "data-component").is_some()
        || html_non_empty_attr_value(content, "data-controller").is_some()
        || html_non_empty_attr_value(content, "data-testid").is_some()
        || html_non_empty_attr_value(content, "data-test").is_some()
        || html_non_empty_attr_value(content, "role").is_some()
}

fn html_tag_from_symbol(symbol: &str) -> &str {
    symbol
        .split(['#', '.', '['])
        .next()
        .unwrap_or(symbol)
        .trim()
}

fn is_semantic_html_tag(tag: &str) -> bool {
    matches!(
        tag,
        "main"
            | "article"
            | "section"
            | "nav"
            | "header"
            | "footer"
            | "aside"
            | "form"
            | "dialog"
            | "details"
            | "summary"
            | "template"
            | "slot"
            | "figure"
            | "figcaption"
            | "table"
            | "thead"
            | "tbody"
            | "tfoot"
            | "script"
            | "style"
    )
}

fn html_signature(content: &str) -> String {
    content
        .find('>')
        .map(|idx| content[..=idx].trim_end().to_string())
        .unwrap_or_else(|| content.lines().next().unwrap_or("").trim_end().to_string())
}

fn html_attr_value(content: &str, name: &str) -> Option<String> {
    let open_end = content.find('>').unwrap_or(content.len());
    let open = &content[..open_end];
    let lower = open.to_ascii_lowercase();
    let name_lower = name.to_ascii_lowercase();

    let mut search_from = 0;
    while let Some(rel) = lower[search_from..].find(&name_lower) {
        let start = search_from + rel;
        let before = lower[..start].chars().next_back();
        let after = lower[start + name_lower.len()..].chars().next();
        let before_ok = before
            .map(|c| c.is_whitespace() || c == '<' || c == '/')
            .unwrap_or(true);
        let after_ok = after
            .map(|c| c.is_whitespace() || c == '=' || c == '>' || c == '/')
            .unwrap_or(true);
        if before_ok && after_ok {
            let rest = open[start + name.len()..].trim_start();
            if !rest.starts_with('=') {
                return Some(String::new());
            }
            let value = rest[1..].trim_start();
            if let Some(quoted) = value.strip_prefix('"') {
                return quoted.split('"').next().map(|s| s.to_string());
            }
            if let Some(quoted) = value.strip_prefix('\'') {
                return quoted.split('\'').next().map(|s| s.to_string());
            }
            return value
                .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
                .next()
                .map(|s| s.to_string());
        }
        search_from = start + name_lower.len();
    }

    None
}

fn html_non_empty_attr_value(content: &str, name: &str) -> Option<String> {
    html_attr_value(content, name).filter(|value| !value.is_empty())
}

fn css_signature(content: &str) -> String {
    css_rule_header(content)
        .unwrap_or_else(|| content.lines().next().unwrap_or("").trim_end().to_string())
}

fn normalize_css_symbol(raw_symbol: &str, content: &str) -> String {
    let raw = collapse_ascii_ws(raw_symbol.trim());
    if content.trim_start().starts_with('@') {
        return css_rule_header(content).unwrap_or(raw);
    }
    raw
}

fn css_rule_header(content: &str) -> Option<String> {
    let trimmed = content.trim_start();
    let end = trimmed
        .find('{')
        .or_else(|| trimmed.find(';'))
        .unwrap_or(trimmed.len());
    let header = collapse_ascii_ws(trimmed[..end].trim());
    (!header.is_empty()).then_some(header)
}

fn normalize_markdown_symbol(raw_symbol: &str, content: &str) -> String {
    markdown_fence_symbol(content)
        .or_else(|| markdown_heading_text(raw_symbol))
        .or_else(|| markdown_heading_text(content))
        .unwrap_or_else(|| collapse_ascii_ws(raw_symbol.trim()))
}

/// HCL chunks become dotted symbols composed from the block header
/// line: `resource "aws_s3_bucket" "main" { ... }` → `resource.aws_s3_bucket.main`;
/// `variable "region" { ... }` → `variable.region`; bare-block forms
/// like `terraform { ... }` or `locals { ... }` collapse to just the
/// block type. This matches how HCL itself references the same objects
/// (e.g., `aws_s3_bucket.main.arn`, `var.region`).
fn normalize_hcl_symbol(content: &str) -> String {
    let header = content
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim())
        .unwrap_or("");
    // Strip both braces and label-quotes per token so the multi-line
    // form `resource "foo" "bar" {` and the one-liner `data "x" "y" {}`
    // both collapse cleanly to `<type>.<label>.<label>`.
    let parts: Vec<String> = header
        .split_whitespace()
        .map(|p| {
            p.trim_matches(|c: char| c == '"' || c == '{' || c == '}')
                .to_string()
        })
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return content.lines().next().unwrap_or("").to_string();
    }
    parts.join(".")
}

fn markdown_signature(content: &str) -> String {
    content
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| collapse_ascii_ws(line.trim_end()))
        .unwrap_or_default()
}

fn markdown_heading_text(input: &str) -> Option<String> {
    let first = input.lines().find(|line| !line.trim().is_empty())?.trim();
    let text = if first.starts_with('#') {
        let without_marker = first.trim_start_matches('#').trim_start();
        let trimmed = without_marker.trim_end();
        let without_hashes = trimmed.trim_end_matches('#');
        if without_hashes.len() != trimmed.len()
            && without_hashes
                .chars()
                .next_back()
                .map(|c| c.is_whitespace())
                .unwrap_or(false)
        {
            collapse_ascii_ws(without_hashes.trim_end())
        } else {
            collapse_ascii_ws(trimmed)
        }
    } else {
        collapse_ascii_ws(first)
    };
    (!text.is_empty()).then_some(text)
}

fn markdown_fence_symbol(content: &str) -> Option<String> {
    let first = content.lines().find(|line| !line.trim().is_empty())?.trim();
    let rest = first
        .strip_prefix("```")
        .or_else(|| first.strip_prefix("~~~"))?
        .trim();
    if rest.is_empty() {
        return Some("fenced code".to_string());
    }

    let language = rest
        .split_whitespace()
        .next()
        .unwrap_or(rest)
        .trim_matches(['{', '}']);
    if language.is_empty() {
        Some("fenced code".to_string())
    } else {
        Some(format!("fenced code: {language}"))
    }
}

fn collapse_ascii_ws(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
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
    fn test_tsx_uses_tsx_grammar_for_jsx_components() {
        let src = r#"
import React from "react";

export function Component({ label }: { label: string }) {
  return <div className="card">{label}</div>;
}
"#;
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::Tsx));

        let mut parser = tree_sitter::Parser::new();
        let tsx = Language::Tsx.ts_language();
        parser.set_language(&tsx).unwrap();
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "TSX grammar should parse JSX without syntax errors"
        );

        let result = chunk_source_with_meta(src, Language::Tsx).unwrap();
        assert!(!result.parse_errors);
        assert!(result.imports.contains(&"react".to_string()));
        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(
            names.contains(&"Component"),
            "missing Component: {result:?}"
        );
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

    // ── Java ──────────────────────────────────────────────────────────────

    #[test]
    fn test_java_class_constructor_method_enum_interface_record() {
        let src = r#"
package com.example.auth;

import java.util.List;

/**
 * Token lifecycle operations.
 */
public final class AuthService {
    private final TokenStore store;

    public AuthService(TokenStore store) {
        this.store = store;
    }

    public boolean verify(String token) {
        return store.lookup(token).isPresent();
    }
}

interface TokenStore {
    java.util.Optional<String> lookup(String token);
}

record LoginRequest(String token) {}

enum AuthStatus {
    VALID,
    EXPIRED
}
"#;
        let chunks = chunk_source(src, Language::Java).unwrap();
        let names: Vec<&str> = chunks.iter().map(|c| c.symbol.as_str()).collect();

        assert!(names.contains(&"AuthService"), "missing class: {chunks:?}");
        assert!(names.contains(&"AuthService"), "missing constructor");
        assert!(names.contains(&"verify"), "missing method");
        assert!(names.contains(&"TokenStore"), "missing interface");
        assert!(names.contains(&"LoginRequest"), "missing record");
        assert!(names.contains(&"AuthStatus"), "missing enum");

        let class = chunks
            .iter()
            .find(|c| c.symbol == "AuthService" && c.kind == ChunkKind::Class)
            .expect("AuthService class missing");
        assert_eq!(class.kind, ChunkKind::Class);
        assert!(class.doc_comment.contains("Token lifecycle operations"));

        let method = chunks.iter().find(|c| c.symbol == "verify").unwrap();
        assert_eq!(method.kind, ChunkKind::Method);
        assert!(method.signature.contains("public boolean verify"));
        assert!(method.callees.contains(&"lookup".to_string()));

        let status = chunks.iter().find(|c| c.symbol == "AuthStatus").unwrap();
        assert_eq!(status.kind, ChunkKind::Enum);
    }

    #[test]
    fn test_java_annotation_type_and_constructor_callees() {
        let src = r#"
public @interface Route {
    String value();
}

class Handler {
    Handler() {
        new java.util.ArrayList<String>();
    }
}
"#;
        let chunks = chunk_source(src, Language::Java).unwrap();
        let route = chunks.iter().find(|c| c.symbol == "Route").unwrap();
        assert_eq!(route.kind, ChunkKind::Class);

        let handler_ctor = chunks
            .iter()
            .find(|c| c.symbol == "Handler" && c.kind == ChunkKind::Method)
            .unwrap();
        assert!(
            handler_ctor
                .callees
                .contains(&"java.util.ArrayList".to_string())
                || handler_ctor.callees.contains(&"ArrayList".to_string()),
            "constructor callee missing: {:?}",
            handler_ctor.callees
        );
    }

    // ── Kotlin ────────────────────────────────────────────────────────────

    #[test]
    fn test_kotlin_class_object_function_and_imports() {
        assert_eq!(Language::from_extension("kt"), Some(Language::Kotlin));
        assert_eq!(Language::from_extension("kts"), Some(Language::Kotlin));

        let src = r#"
package com.example.auth

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

/**
 * Token lifecycle operations.
 */
class AuthService(private val store: TokenStore) {
    fun verify(token: String): Boolean {
        return store.lookup(token) != null
    }
}

interface TokenStore {
    fun lookup(token: String): String?
}

data class LoginRequest(val token: String)

object SessionRegistry {
    fun current(): String? = null
}

enum class AuthStatus {
    VALID,
    EXPIRED,
}

// Top-level extension function.
fun String.normalizeToken(): String = trim().lowercase()
"#;
        let result = chunk_source_with_meta(src, Language::Kotlin).unwrap();
        assert!(!result.parse_errors, "Kotlin parse errors: {result:?}");

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();

        assert!(names.contains(&"AuthService"), "missing class: {result:?}");
        assert!(names.contains(&"verify"), "missing member fun: {result:?}");
        assert!(
            names.contains(&"TokenStore"),
            "missing interface: {result:?}"
        );
        assert!(
            names.contains(&"LoginRequest"),
            "missing data class: {result:?}"
        );
        assert!(
            names.contains(&"SessionRegistry"),
            "missing object: {result:?}"
        );
        assert!(names.contains(&"AuthStatus"), "missing enum: {result:?}");
        assert!(
            names.contains(&"current"),
            "missing object member fun: {result:?}"
        );
        assert!(
            names.contains(&"normalizeToken"),
            "missing top-level extension fun: {result:?}"
        );

        let class = result
            .chunks
            .iter()
            .find(|c| c.symbol == "AuthService" && c.kind == ChunkKind::Class)
            .expect("AuthService class missing");
        assert!(class.doc_comment.contains("Token lifecycle operations"));

        let verify = result.chunks.iter().find(|c| c.symbol == "verify").unwrap();
        assert_eq!(verify.kind, ChunkKind::Method);
        assert!(verify.signature.contains("fun verify"));
        assert!(
            verify.callees.contains(&"lookup".to_string()),
            "verify callees missing lookup: {:?}",
            verify.callees
        );

        assert!(
            result.imports.contains(&"kotlinx.coroutines.flow.Flow".to_string()),
            "missing import: {:?}",
            result.imports
        );
        assert!(
            result.imports.contains(&"kotlinx.coroutines.flow.map".to_string()),
            "missing import: {:?}",
            result.imports
        );
    }

    // ── SQL ───────────────────────────────────────────────────────────────

    #[test]
    fn test_sql_create_statements_and_alter() {
        assert_eq!(Language::from_extension("sql"), Some(Language::Sql));
        assert_eq!(Language::from_extension("ddl"), Some(Language::Sql));

        // CREATE PROCEDURE is intentionally omitted — tree-sitter-sequel
        // does not model it, so including it would dirty `parse_errors`.
        let src = r#"
-- Users table
CREATE TABLE users (
    id BIGSERIAL PRIMARY KEY,
    email TEXT NOT NULL UNIQUE
);

CREATE VIEW active_users AS
SELECT * FROM users WHERE deleted_at IS NULL;

CREATE INDEX idx_users_email ON users(email);

-- Normalize email addresses before lookup.
CREATE OR REPLACE FUNCTION normalize_email(addr TEXT)
RETURNS TEXT AS $$
BEGIN
    RETURN lower(trim(addr));
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER users_lowercase_email
BEFORE INSERT ON users
FOR EACH ROW
EXECUTE FUNCTION normalize_email_trigger();

ALTER TABLE users ADD COLUMN created_at TIMESTAMPTZ NOT NULL DEFAULT now();
"#;
        let result = chunk_source_with_meta(src, Language::Sql).unwrap();
        assert!(!result.parse_errors, "SQL parse errors: {result:?}");

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();

        assert!(names.contains(&"users"), "missing CREATE TABLE: {result:?}");
        assert!(
            names.contains(&"active_users"),
            "missing CREATE VIEW: {result:?}"
        );
        assert!(
            names.contains(&"idx_users_email"),
            "missing CREATE INDEX: {result:?}"
        );
        assert!(
            names.contains(&"normalize_email"),
            "missing CREATE FUNCTION: {result:?}"
        );
        assert!(
            names.contains(&"users_lowercase_email"),
            "missing CREATE TRIGGER: {result:?}"
        );

        // ALTER TABLE re-emits the targeted table as a module chunk so
        // migration files surface the touched object even when the
        // original CREATE TABLE lives in a different file.
        let alter = result
            .chunks
            .iter()
            .find(|c| c.symbol == "users" && c.kind == ChunkKind::Module)
            .expect("missing ALTER TABLE module chunk");
        assert!(
            alter.content.contains("ADD COLUMN created_at"),
            "ALTER TABLE chunk content unexpected: {alter:?}"
        );

        let table = result
            .chunks
            .iter()
            .find(|c| c.symbol == "users" && c.kind == ChunkKind::Class)
            .expect("users CREATE TABLE chunk missing");
        assert!(
            table.doc_comment.contains("Users table"),
            "missing -- doc comment on CREATE TABLE: {table:?}"
        );
        assert!(table.content.contains("BIGSERIAL PRIMARY KEY"));

        let func = result
            .chunks
            .iter()
            .find(|c| c.symbol == "normalize_email")
            .expect("normalize_email function missing");
        assert_eq!(func.kind, ChunkKind::Function);
        assert!(
            func.doc_comment.contains("Normalize email"),
            "missing -- doc comment on CREATE FUNCTION: {func:?}"
        );
        assert!(
            func.signature
                .contains("CREATE OR REPLACE FUNCTION normalize_email"),
            "function signature unexpected: {:?}",
            func.signature
        );
        assert!(
            func.callees.contains(&"lower".to_string()),
            "missing PL/pgSQL callee `lower`: {:?}",
            func.callees
        );
        assert!(
            func.callees.contains(&"trim".to_string()),
            "missing PL/pgSQL callee `trim`: {:?}",
            func.callees
        );

        let trigger = result
            .chunks
            .iter()
            .find(|c| c.symbol == "users_lowercase_email")
            .expect("trigger chunk missing");
        assert_eq!(trigger.kind, ChunkKind::Function);
        assert!(
            trigger
                .content
                .contains("EXECUTE FUNCTION normalize_email_trigger"),
            "trigger content unexpected: {trigger:?}"
        );

        let view = result
            .chunks
            .iter()
            .find(|c| c.symbol == "active_users")
            .unwrap();
        assert_eq!(view.kind, ChunkKind::Class);
        assert!(view.content.contains("SELECT * FROM users"));

        let index = result
            .chunks
            .iter()
            .find(|c| c.symbol == "idx_users_email")
            .unwrap();
        assert_eq!(index.kind, ChunkKind::Module);
    }

    #[test]
    fn test_sql_migration_file_fixture() {
        // Real-world migration file shape: file-header block comment,
        // multiple CREATE TABLE / CREATE INDEX statements paired in the
        // same file, FK REFERENCES, multiple ALTER TABLE statements.
        // Pays one slice of the v0.4 chunker-fixtures debt by giving SQL
        // a fixture-style assertion set, not just a minimal smoke test.
        let src = r#"/*
 * 0042_user_schema.sql
 * Migration: add user table and supporting indexes.
 */

-- Core user account record.
CREATE TABLE accounts (
    id BIGSERIAL PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_accounts_email ON accounts(email);
CREATE INDEX idx_accounts_created_at ON accounts(created_at);

-- Audit trail keyed by account.
CREATE TABLE account_events (
    id BIGSERIAL PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES accounts(id),
    event_type TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_account_events_account_id ON account_events(account_id);

ALTER TABLE accounts ADD COLUMN deleted_at TIMESTAMPTZ;
ALTER TABLE accounts ADD COLUMN locked_at TIMESTAMPTZ;
"#;
        let result = chunk_source_with_meta(src, Language::Sql).unwrap();
        assert!(!result.parse_errors, "SQL parse errors: {result:?}");

        let names: Vec<String> = result.chunks.iter().map(|c| c.symbol.clone()).collect();

        // Every CREATE statement gets a chunk.
        assert!(names.iter().any(|n| n == "accounts"));
        assert!(names.iter().any(|n| n == "account_events"));
        assert!(names.iter().any(|n| n == "idx_accounts_email"));
        assert!(names.iter().any(|n| n == "idx_accounts_created_at"));
        assert!(names.iter().any(|n| n == "idx_account_events_account_id"));

        // Both ALTER TABLE statements emit module chunks on the same
        // target table — neither swallows the other via dedup.
        let alter_count = result
            .chunks
            .iter()
            .filter(|c| c.symbol == "accounts" && c.kind == ChunkKind::Module)
            .count();
        assert_eq!(
            alter_count, 2,
            "expected 2 ALTER TABLE chunks, got chunks: {result:?}"
        );

        // -- doc comment lifted onto the directly-following CREATE TABLE,
        // even with a file-header /* */ block earlier in the file.
        let accounts_table = result
            .chunks
            .iter()
            .find(|c| c.symbol == "accounts" && c.kind == ChunkKind::Class)
            .expect("accounts table chunk missing");
        assert!(
            accounts_table.doc_comment.contains("Core user account"),
            "missing -- doc comment on accounts: {accounts_table:?}"
        );

        let events_table = result
            .chunks
            .iter()
            .find(|c| c.symbol == "account_events")
            .expect("account_events table chunk missing");
        assert!(
            events_table.doc_comment.contains("Audit trail"),
            "missing -- doc comment on account_events: {events_table:?}"
        );

        // FK REFERENCES syntax parses cleanly (otherwise parse_errors
        // would already have caught it above).
        assert!(events_table.content.contains("REFERENCES accounts(id)"));
    }

    // ── C++ ───────────────────────────────────────────────────────────────

    #[test]
    fn test_cpp_namespace_class_template_and_qualified_impl() {
        assert_eq!(Language::from_extension("cpp"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("cc"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("cxx"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("hpp"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("hxx"), Some(Language::Cpp));

        let src = r#"
#include <vector>
#include "session.h"

namespace pluck::auth {

enum class Status {
    Ok,
    Expired,
};

// Verifies issued tokens.
class TokenStore {
public:
    TokenStore(std::size_t cap);
    ~TokenStore();

    bool verify(const std::string &token) const;

    Status operator()(const std::string &token) const;

private:
    std::vector<std::string> tokens_;
};

TokenStore::TokenStore(std::size_t cap) : tokens_(cap) {}
TokenStore::~TokenStore() = default;

bool TokenStore::verify(const std::string &token) const {
    return std::find(tokens_.begin(), tokens_.end(), token) != tokens_.end();
}

template <typename T>
T clamp(T v, T lo, T hi) {
    return v < lo ? lo : (v > hi ? hi : v);
}

template <typename T>
class Cache {
public:
    void put(const std::string &key, T value);
    T get(const std::string &key) const;
};

}  // namespace pluck::auth
"#;
        let result = chunk_source_with_meta(src, Language::Cpp).unwrap();
        assert!(!result.parse_errors, "C++ parse errors: {result:?}");

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();

        // Namespace (nested form).
        assert!(
            names.contains(&"pluck::auth"),
            "missing nested namespace: {result:?}"
        );

        // enum class + class.
        assert!(names.contains(&"Status"), "missing scoped enum: {result:?}");
        assert!(names.contains(&"TokenStore"), "missing class: {result:?}");
        assert!(names.contains(&"Cache"), "missing templated class: {result:?}");

        // In-class member declarations.
        assert!(names.contains(&"verify"), "missing member fn decl: {result:?}");
        assert!(
            names.contains(&"operator()"),
            "missing operator overload decl: {result:?}"
        );

        // Out-of-class method definitions (qualified). Constructor +
        // destructor + regular method all reuse the same simple name
        // ("TokenStore", "~TokenStore", "verify") — the surface that
        // matters is the qualified scope, which lives in chunk
        // content / signature, not the symbol.
        let verify_chunks: Vec<_> = result
            .chunks
            .iter()
            .filter(|c| c.symbol == "verify")
            .collect();
        // Declaration in class body + qualified out-of-class definition.
        assert!(
            verify_chunks.len() >= 2,
            "expected verify declaration + definition, got: {verify_chunks:?}"
        );

        // Destructor name captured with leading tilde.
        assert!(
            names.contains(&"~TokenStore"),
            "missing destructor: {result:?}"
        );

        // Free templated function.
        assert!(names.contains(&"clamp"), "missing template free fn: {result:?}");

        // Kinds.
        let token_store_class = result
            .chunks
            .iter()
            .find(|c| c.symbol == "TokenStore" && c.kind == ChunkKind::Class)
            .expect("TokenStore class chunk missing");
        assert!(
            token_store_class.doc_comment.contains("Verifies issued tokens"),
            "missing // doc on class: {token_store_class:?}"
        );

        let status = result.chunks.iter().find(|c| c.symbol == "Status").unwrap();
        assert_eq!(status.kind, ChunkKind::Enum);

        let ns = result
            .chunks
            .iter()
            .find(|c| c.symbol == "pluck::auth")
            .unwrap();
        assert_eq!(ns.kind, ChunkKind::Module);

        // Qualified method body callee: std::find captured as `find`.
        let verify_def = verify_chunks
            .iter()
            .find(|c| c.content.contains("std::find"))
            .expect("qualified verify definition missing");
        assert!(
            verify_def.callees.contains(&"find".to_string()),
            "missing std::find callee: {:?}",
            verify_def.callees
        );

        // Imports.
        assert!(
            result.imports.iter().any(|i| i.contains("vector")),
            "missing <vector> include: {:?}",
            result.imports
        );
        assert!(
            result.imports.iter().any(|i| i == "session.h"),
            "missing session.h include: {:?}",
            result.imports
        );
    }

    #[test]
    fn test_cpp_realistic_header_fixture() {
        // Realistic C++ header: nested namespace, multiple classes with
        // ctor/dtor/method/operator, free template, scoped enum,
        // forward decl. Pays one slice of v0.5 fixtures debt inline.
        let src = r#"
#pragma once

#include <memory>
#include <string>
#include "result.h"

namespace pluck::store {

class Session;

enum class Tier {
    Free,
    Pro,
    Enterprise,
};

class SessionRegistry {
public:
    SessionRegistry();
    ~SessionRegistry();

    std::shared_ptr<Session> create(const std::string &id, Tier tier);
    bool release(const std::string &id);

    std::size_t size() const noexcept;

    SessionRegistry(const SessionRegistry &) = delete;
    SessionRegistry &operator=(const SessionRegistry &) = delete;

private:
    struct Impl;
    std::unique_ptr<Impl> impl_;
};

template <typename T>
class Slot {
public:
    explicit Slot(T value) : value_(std::move(value)) {}
    const T &get() const noexcept { return value_; }
private:
    T value_;
};

template <typename Fn>
auto guard(Fn &&fn) -> decltype(fn()) {
    return fn();
}

}  // namespace pluck::store
"#;
        let result = chunk_source_with_meta(src, Language::Cpp).unwrap();
        assert!(!result.parse_errors, "C++ parse errors: {result:?}");

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();

        // Nested namespace.
        assert!(names.contains(&"pluck::store"));

        // Forward class decl + full class decl.
        assert!(names.contains(&"Session"), "forward class decl missing: {result:?}");
        assert!(names.contains(&"SessionRegistry"));

        // Scoped enum.
        assert!(names.contains(&"Tier"));

        // Member decls: ctor / dtor / methods.
        assert!(names.contains(&"create"), "missing member fn create: {result:?}");
        assert!(names.contains(&"release"));
        assert!(names.contains(&"size"));
        assert!(
            names.contains(&"~SessionRegistry"),
            "missing destructor: {result:?}"
        );
        // operator= overload.
        assert!(
            names.contains(&"operator="),
            "missing assignment operator: {result:?}"
        );

        // Templated class + templated free function.
        assert!(names.contains(&"Slot"));
        assert!(names.contains(&"guard"));

        // Doc-class flag: SessionRegistry chunk content should include
        // the deleted-copy-ctor lines (the chunker captures the whole
        // class body).
        let registry = result
            .chunks
            .iter()
            .find(|c| c.symbol == "SessionRegistry" && c.kind == ChunkKind::Class)
            .expect("SessionRegistry class chunk missing");
        assert!(
            registry.content.contains("= delete"),
            "registry chunk should include deleted ctor lines: {registry:?}"
        );

        // Imports.
        assert!(result.imports.iter().any(|i| i.contains("memory")));
        assert!(result.imports.iter().any(|i| i.contains("string")));
        assert!(result.imports.iter().any(|i| i == "result.h"));
    }

    // ── C ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_c_function_struct_typedef_macro_include() {
        assert_eq!(Language::from_extension("c"), Some(Language::C));
        assert_eq!(Language::from_extension("h"), Some(Language::C));

        let src = r#"
#include <stdio.h>
#include "config.h"

// Maximum number of cached tokens.
#define MAX_TOKENS 1024
#define CLAMP(x, lo, hi) ((x) < (lo) ? (lo) : (x) > (hi) ? (hi) : (x))

// Anonymous struct typedef'd to a name.
typedef struct {
    int id;
    char *email;
} user_t;

typedef enum auth_status {
    AUTH_OK,
    AUTH_EXPIRED,
} auth_status_t;

struct token_store {
    user_t users[MAX_TOKENS];
    int (*lookup)(const char *token);
};

union value {
    int i;
    char *s;
};

// Forward decl.
static int normalize_email(char *addr);

// Normalize email by lowercasing.
int normalize_email(char *addr) {
    return strlen(addr);
}

static inline int clamp_int(int v) {
    return CLAMP(v, 0, 100);
}
"#;
        let result = chunk_source_with_meta(src, Language::C).unwrap();
        assert!(!result.parse_errors, "C parse errors: {result:?}");

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();

        // Macros.
        assert!(names.contains(&"MAX_TOKENS"), "missing object-like macro: {result:?}");
        assert!(names.contains(&"CLAMP"), "missing function-like macro: {result:?}");

        // Types.
        assert!(names.contains(&"user_t"), "missing typedef'd anonymous struct: {result:?}");
        assert!(
            names.contains(&"auth_status_t"),
            "missing typedef'd enum (typedef name): {result:?}"
        );
        assert!(
            names.contains(&"auth_status"),
            "missing typedef'd enum (inner enum name): {result:?}"
        );
        assert!(names.contains(&"token_store"), "missing standalone struct: {result:?}");
        assert!(names.contains(&"value"), "missing union: {result:?}");

        // Functions (forward decl + definition share the same symbol;
        // both chunks are kept since they live at different start bytes
        // and document different surfaces — header decl vs body).
        let normalize_chunks: Vec<_> = result
            .chunks
            .iter()
            .filter(|c| c.symbol == "normalize_email")
            .collect();
        assert_eq!(
            normalize_chunks.len(),
            2,
            "expected forward decl + definition chunks, got: {result:?}"
        );

        assert!(names.contains(&"clamp_int"), "missing static inline function: {result:?}");

        // Kinds match the chunker's prefix mapping.
        let max_tokens = result.chunks.iter().find(|c| c.symbol == "MAX_TOKENS").unwrap();
        assert_eq!(max_tokens.kind, ChunkKind::Module);
        let clamp = result.chunks.iter().find(|c| c.symbol == "CLAMP").unwrap();
        assert_eq!(clamp.kind, ChunkKind::Function);
        let token_store = result.chunks.iter().find(|c| c.symbol == "token_store").unwrap();
        assert_eq!(token_store.kind, ChunkKind::Struct);
        let union_v = result.chunks.iter().find(|c| c.symbol == "value").unwrap();
        assert_eq!(union_v.kind, ChunkKind::Struct);
        let auth_inner = result.chunks.iter().find(|c| c.symbol == "auth_status").unwrap();
        assert_eq!(auth_inner.kind, ChunkKind::Enum);

        // Doc comments lifted via `//`.
        assert!(
            max_tokens.doc_comment.contains("Maximum number of cached tokens"),
            "missing // doc on macro: {max_tokens:?}"
        );
        let user_t = result.chunks.iter().find(|c| c.symbol == "user_t").unwrap();
        assert!(
            user_t.doc_comment.contains("Anonymous struct"),
            "missing // doc on typedef: {user_t:?}"
        );

        let normalize_def = result
            .chunks
            .iter()
            .find(|c| c.symbol == "normalize_email" && c.content.contains('{'))
            .expect("definition chunk missing");
        assert!(
            normalize_def.doc_comment.contains("Normalize email"),
            "missing // doc on function definition: {normalize_def:?}"
        );

        // Callee captured.
        assert!(
            normalize_def.callees.contains(&"strlen".to_string()),
            "missing strlen callee: {:?}",
            normalize_def.callees
        );

        // Imports.
        assert!(
            result.imports.iter().any(|i| i.contains("stdio.h")),
            "missing system include: {:?}",
            result.imports
        );
        assert!(
            result.imports.iter().any(|i| i == "config.h"),
            "missing project include: {:?}",
            result.imports
        );
    }

    #[test]
    fn test_c_realistic_header_fixture() {
        // Header file shape: include guards, typedef function-pointer,
        // multiple struct typedefs, extern function declarations, an
        // inline static helper. Pays one slice of the v0.5
        // "Per-language real-world fixtures" debt.
        let src = r#"
#ifndef PLUCK_AUTH_H
#define PLUCK_AUTH_H

#include <stddef.h>
#include "common.h"

typedef int (*auth_callback)(const char *token, void *ctx);

typedef struct auth_session {
    char *id;
    size_t refcount;
    auth_callback on_expire;
} auth_session_t;

typedef enum {
    AUTH_OK = 0,
    AUTH_DENIED = 1,
    AUTH_EXPIRED = 2,
} auth_result_t;

extern auth_session_t *auth_session_create(const char *id);
extern void auth_session_release(auth_session_t *session);

static inline int auth_result_ok(auth_result_t r) {
    return r == AUTH_OK;
}

#endif /* PLUCK_AUTH_H */
"#;
        let result = chunk_source_with_meta(src, Language::C).unwrap();
        assert!(!result.parse_errors, "C parse errors: {result:?}");

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();

        // Include-guard macro.
        assert!(names.contains(&"PLUCK_AUTH_H"), "missing guard macro: {result:?}");

        // Typedefs.
        assert!(names.contains(&"auth_callback"), "missing fn-pointer typedef: {result:?}");
        assert!(
            names.contains(&"auth_session_t"),
            "missing struct typedef name: {result:?}"
        );
        assert!(
            names.contains(&"auth_session"),
            "missing inner struct name: {result:?}"
        );
        assert!(
            names.contains(&"auth_result_t"),
            "missing anonymous enum typedef name: {result:?}"
        );

        // Extern function declarations.
        assert!(
            names.contains(&"auth_session_create"),
            "missing extern decl: {result:?}"
        );
        assert!(
            names.contains(&"auth_session_release"),
            "missing extern decl: {result:?}"
        );

        // Inline function.
        let helper = result
            .chunks
            .iter()
            .find(|c| c.symbol == "auth_result_ok")
            .expect("inline helper missing");
        assert_eq!(helper.kind, ChunkKind::Function);
        assert!(helper.signature.contains("static inline"));

        // Imports.
        assert!(
            result.imports.iter().any(|i| i.contains("stddef.h")),
            "missing stddef include: {:?}",
            result.imports
        );
        assert!(
            result.imports.iter().any(|i| i == "common.h"),
            "missing common.h include: {:?}",
            result.imports
        );
    }

    // ── HCL ───────────────────────────────────────────────────────────────

    #[test]
    fn test_hcl_terraform_block_types() {
        assert_eq!(Language::from_extension("tf"), Some(Language::Hcl));
        assert_eq!(Language::from_extension("tfvars"), Some(Language::Hcl));
        assert_eq!(Language::from_extension("hcl"), Some(Language::Hcl));

        let src = r#"
# main.tf - example
terraform {
  required_version = ">= 1.5"
}

provider "aws" {
  region = var.region
}

# Region to deploy into.
variable "region" {
  type    = string
  default = "us-east-1"
}

locals {
  tags = { Env = "prod" }
}

data "aws_caller_identity" "current" {}

// S3 bucket for app state.
resource "aws_s3_bucket" "main" {
  bucket = "my-bucket"
  tags   = merge(local.tags, { Owner = data.aws_caller_identity.current.arn })
}

module "vpc" {
  source = "terraform-aws-modules/vpc/aws"
}

output "bucket_arn" {
  value = aws_s3_bucket.main.arn
}
"#;
        let result = chunk_source_with_meta(src, Language::Hcl).unwrap();
        assert!(!result.parse_errors, "HCL parse errors: {result:?}");

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();

        assert!(names.contains(&"terraform"), "missing terraform: {result:?}");
        assert!(names.contains(&"provider.aws"), "missing provider: {result:?}");
        assert!(
            names.contains(&"variable.region"),
            "missing variable: {result:?}"
        );
        assert!(names.contains(&"locals"), "missing locals: {result:?}");
        assert!(
            names.contains(&"data.aws_caller_identity.current"),
            "missing data source: {result:?}"
        );
        assert!(
            names.contains(&"resource.aws_s3_bucket.main"),
            "missing resource: {result:?}"
        );
        assert!(names.contains(&"module.vpc"), "missing module: {result:?}");
        assert!(
            names.contains(&"output.bucket_arn"),
            "missing output: {result:?}"
        );

        // All HCL chunks are Module kind; discrimination lives in the
        // dotted symbol prefix.
        for chunk in &result.chunks {
            assert_eq!(
                chunk.kind,
                ChunkKind::Module,
                "expected Module kind for {chunk:?}"
            );
        }

        // -- and // doc comments both lift onto the directly-following block.
        let resource = result
            .chunks
            .iter()
            .find(|c| c.symbol == "resource.aws_s3_bucket.main")
            .unwrap();
        assert!(
            resource.doc_comment.contains("S3 bucket for app state"),
            "missing // doc comment: {resource:?}"
        );

        let var_region = result
            .chunks
            .iter()
            .find(|c| c.symbol == "variable.region")
            .unwrap();
        assert!(
            var_region.doc_comment.contains("Region to deploy"),
            "missing # doc comment: {var_region:?}"
        );

        // Function callees captured from inline expressions inside
        // attribute values.
        assert!(
            resource.callees.contains(&"merge".to_string()),
            "missing merge callee: {:?}",
            resource.callees
        );
    }

    #[test]
    fn test_hcl_terraform_realistic_fixture() {
        // Realistic Terraform shape: required_providers + backend nested
        // inside terraform, lifecycle nested inside a resource,
        // jsonencode + interpolation in attributes. Pays one slice of
        // the v0.5 "Per-language real-world fixtures" debt inline.
        let src = r#"
terraform {
  required_version = ">= 1.5"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
  }
  backend "s3" {
    bucket = "tfstate-app"
    key    = "envs/prod/terraform.tfstate"
    region = "us-east-1"
  }
}

# Shared tag map.
locals {
  common_tags = {
    Project   = "app"
    ManagedBy = "terraform"
  }
}

# Persistent log storage.
resource "aws_s3_bucket" "logs" {
  bucket = "${var.env}-app-logs"
  tags   = local.common_tags

  lifecycle {
    prevent_destroy = true
  }
}

resource "aws_iam_role" "app" {
  name = "${var.env}-app-role"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Action    = "sts:AssumeRole"
      Effect    = "Allow"
      Principal = { Service = "ec2.amazonaws.com" }
    }]
  })

  tags = local.common_tags
}
"#;
        let result = chunk_source_with_meta(src, Language::Hcl).unwrap();
        assert!(!result.parse_errors, "HCL parse errors: {result:?}");

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();

        // Top-level blocks captured.
        assert!(names.contains(&"terraform"));
        assert!(names.contains(&"locals"));
        assert!(names.contains(&"resource.aws_s3_bucket.logs"));
        assert!(names.contains(&"resource.aws_iam_role.app"));

        // Nested blocks ALSO captured so agents can search them
        // independently (e.g., grep for "backend" finds the s3 backend).
        assert!(
            names.iter().any(|n| n == &"backend.s3"),
            "expected nested backend.s3 chunk: {result:?}"
        );
        assert!(
            names.iter().any(|n| n == &"lifecycle"),
            "expected nested lifecycle chunk: {result:?}"
        );
        assert!(
            names.iter().any(|n| n == &"required_providers"),
            "expected nested required_providers chunk: {result:?}"
        );

        // Doc comments lift onto directly-following blocks.
        let logs = result
            .chunks
            .iter()
            .find(|c| c.symbol == "resource.aws_s3_bucket.logs")
            .unwrap();
        assert!(
            logs.doc_comment.contains("Persistent log storage"),
            "missing # doc on logs bucket: {logs:?}"
        );

        let local_tags = result
            .chunks
            .iter()
            .find(|c| c.symbol == "locals")
            .unwrap();
        assert!(
            local_tags.doc_comment.contains("Shared tag map"),
            "missing # doc on locals: {local_tags:?}"
        );

        // jsonencode pulled out as a callee on the IAM role chunk.
        let iam = result
            .chunks
            .iter()
            .find(|c| c.symbol == "resource.aws_iam_role.app")
            .unwrap();
        assert!(
            iam.callees.contains(&"jsonencode".to_string()),
            "missing jsonencode callee: {:?}",
            iam.callees
        );
    }

    // ── HTML ──────────────────────────────────────────────────────────────

    #[test]
    fn test_html_semantic_component_script_and_style_chunks() {
        let src = r#"
<!doctype html>
<html>
  <body>
    <!-- Product shell -->
    <main id="app">
      <section id="hero">
        <h1>Launch faster</h1>
        <p>Not its own chunk.</p>
      </section>

      <article data-component="PricingCard">
        <header><h2>Pro</h2></header>
      </article>

      <app-shell>
        <app-card title="Usage"></app-card>
      </app-shell>

      <script type="module">
        initDashboard();
      </script>

      <style scoped>
        main { display: grid; }
      </style>
    </main>
  </body>
</html>
"#;
        let chunks = chunk_source(src, Language::Html).unwrap();
        let names: Vec<&str> = chunks.iter().map(|c| c.symbol.as_str()).collect();

        assert!(names.contains(&"main#app"), "missing main: {chunks:?}");
        assert!(
            names.contains(&"section#hero"),
            "missing section: {chunks:?}"
        );
        assert!(
            names.contains(&"article[PricingCard]"),
            "missing data-component article: {chunks:?}"
        );
        assert!(
            names.contains(&"app-shell"),
            "missing custom element: {chunks:?}"
        );
        assert!(
            names.contains(&"app-card"),
            "missing nested custom element: {chunks:?}"
        );
        assert!(
            names.contains(&"script[type=module]"),
            "missing module script: {chunks:?}"
        );
        assert!(
            names.contains(&"style[scoped]"),
            "missing scoped style: {chunks:?}"
        );
        assert!(
            !names.contains(&"p"),
            "inline paragraph should not become a chunk: {chunks:?}"
        );

        let hero = chunks.iter().find(|c| c.symbol == "section#hero").unwrap();
        assert_eq!(hero.kind, ChunkKind::Module);
        assert!(hero.signature.contains("<section id=\"hero\">"));
    }

    #[test]
    fn test_html_extension_and_role_chunks() {
        assert_eq!(Language::from_extension("html"), Some(Language::Html));
        assert_eq!(Language::from_extension("htm"), Some(Language::Html));

        let src = r#"
<div class="layout">
  <span>ignored</span>
</div>
<div role="alert">Session expired</div>
<my-widget id="profile"></my-widget>
<app-icon name="search" />
"#;
        let chunks = chunk_source(src, Language::Html).unwrap();
        let names: Vec<&str> = chunks.iter().map(|c| c.symbol.as_str()).collect();

        assert_eq!(
            names.iter().filter(|name| **name == "div").count(),
            1,
            "only the role-bearing div should be indexed: {chunks:?}"
        );
        assert!(
            names.contains(&"my-widget#profile"),
            "custom element with id missing: {chunks:?}"
        );
        assert!(
            names.contains(&"app-icon"),
            "self-closing custom element missing: {chunks:?}"
        );
    }

    // ── CSS / SCSS ───────────────────────────────────────────────────────

    #[test]
    fn test_css_selector_and_at_rule_chunks() {
        assert_eq!(Language::from_extension("css"), Some(Language::Css));

        let src = r#"
/* Theme root */
:root {
  --brand: #0f766e;
}

.button,
.link:hover {
  color: var(--brand);
}

@media (min-width: 720px) {
  .button {
    display: inline-flex;
  }
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}
"#;
        let chunks = chunk_source(src, Language::Css).unwrap();
        let names: Vec<&str> = chunks.iter().map(|c| c.symbol.as_str()).collect();

        assert!(names.contains(&":root"), "missing :root: {chunks:?}");
        assert!(
            names.contains(&".button, .link:hover"),
            "missing multi-selector rule: {chunks:?}"
        );
        assert!(
            names.contains(&"@media (min-width: 720px)"),
            "missing @media chunk: {chunks:?}"
        );
        assert!(
            names.contains(&"@keyframes spin"),
            "missing @keyframes chunk: {chunks:?}"
        );

        let media = chunks
            .iter()
            .find(|c| c.symbol == "@media (min-width: 720px)")
            .unwrap();
        assert_eq!(media.kind, ChunkKind::Module);
        assert_eq!(media.signature, "@media (min-width: 720px)");
    }

    #[test]
    fn test_scss_selector_mixin_function_and_nested_chunks() {
        assert_eq!(Language::from_extension("scss"), Some(Language::Scss));

        let src = r#"
// Card theme helpers.
@mixin card-tone($tone) {
  border-color: $tone;
}

@function spacing($step) {
  @return $step * 0.25rem;
}

.dashboard {
  @include card-tone(#0f766e);

  &__item {
    padding: spacing(4);
  }
}
"#;
        let chunks = chunk_source(src, Language::Scss).unwrap();
        let names: Vec<&str> = chunks.iter().map(|c| c.symbol.as_str()).collect();

        assert!(
            names.contains(&"@mixin card-tone($tone)"),
            "missing mixin: {chunks:?}"
        );
        assert!(
            names.contains(&"@function spacing($step)"),
            "missing function: {chunks:?}"
        );
        assert!(
            names.contains(&".dashboard"),
            "missing parent selector: {chunks:?}"
        );
        assert!(
            names.contains(&"&__item"),
            "missing nested selector: {chunks:?}"
        );

        let mixin = chunks
            .iter()
            .find(|c| c.symbol == "@mixin card-tone($tone)")
            .unwrap();
        assert_eq!(mixin.kind, ChunkKind::Module);
        assert!(mixin.doc_comment.contains("Card theme helpers."));
    }

    // ── Markdown / MDX ───────────────────────────────────────────────────

    #[test]
    fn test_markdown_heading_sections_and_fenced_code_chunks() {
        assert_eq!(Language::from_extension("md"), Some(Language::Markdown));
        assert_eq!(
            Language::from_extension("markdown"),
            Some(Language::Markdown)
        );

        let src = r#"
# Pluck Docs

Use pluck before raw file reads.

```rust
fn main() {
    println!("fast");
}
```

Install
-------

Run the MCP server from your agent.
"#;
        let result = chunk_source_with_meta(src, Language::Markdown).unwrap();
        assert!(!result.parse_errors);

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(
            names.contains(&"Pluck Docs"),
            "missing ATX heading section: {result:?}"
        );
        assert!(
            names.contains(&"Install"),
            "missing setext heading section: {result:?}"
        );
        assert!(
            names.contains(&"fenced code: rust"),
            "missing fenced code chunk: {result:?}"
        );

        let fence = result
            .chunks
            .iter()
            .find(|c| c.symbol == "fenced code: rust")
            .unwrap();
        assert_eq!(fence.kind, ChunkKind::Module);
        assert_eq!(fence.signature, "```rust");
        assert!(fence.content.contains("println!"));
    }

    #[test]
    fn test_mdx_heading_sections_and_fenced_code_chunks() {
        assert_eq!(Language::from_extension("mdx"), Some(Language::Mdx));

        let src = r#"
---
title: Dashboard
---

import Widget from "./Widget"

# Dashboard MDX

<Widget status="ok" />

~~~tsx
<Widget status="ok" />
~~~

## Props

<PropTable />
"#;
        let result = chunk_source_with_meta(src, Language::Mdx).unwrap();
        assert!(!result.parse_errors);

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(
            names.contains(&"Dashboard MDX"),
            "missing MDX top heading: {result:?}"
        );
        assert!(
            names.contains(&"Props"),
            "missing MDX nested heading: {result:?}"
        );
        assert!(
            names.contains(&"fenced code: tsx"),
            "missing MDX fenced code chunk: {result:?}"
        );
    }

    #[test]
    fn test_markdown_heading_keeps_hash_in_title() {
        let src = "# C# notes\n\nUse the parser safely.\n";
        let chunks = chunk_source(src, Language::Markdown).unwrap();
        assert_eq!(chunks[0].symbol, "C# notes");
    }

    // ── YAML / JSON / TOML ───────────────────────────────────────────────

    #[test]
    fn test_json_path_key_chunks() {
        assert_eq!(Language::from_extension("json"), Some(Language::Json));

        let src = r#"
{
  "scripts": {
    "build": "cargo build",
    "test": "cargo test"
  },
  "dependencies": {
    "serde": { "version": "1", "features": ["derive"] }
  },
  "plugins": [
    { "name": "auth", "enabled": true }
  ]
}
"#;
        let result = chunk_source_with_meta(src, Language::Json).unwrap();
        assert!(!result.parse_errors);

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(
            names.contains(&"scripts.build"),
            "missing nested script key: {result:?}"
        );
        assert!(
            names.contains(&"dependencies.serde.version"),
            "missing nested dependency key: {result:?}"
        );
        assert!(
            names.contains(&"plugins[0].name"),
            "missing array object key: {result:?}"
        );

        let scripts = result
            .chunks
            .iter()
            .find(|c| c.symbol == "scripts")
            .unwrap();
        assert_eq!(scripts.kind, ChunkKind::Module);
        assert!(scripts.content.contains("\"build\""));
    }

    #[test]
    fn test_yaml_path_key_chunks() {
        assert_eq!(Language::from_extension("yaml"), Some(Language::Yaml));
        assert_eq!(Language::from_extension("yml"), Some(Language::Yaml));

        let src = r#"
services:
  web:
    image: nginx:latest
    env:
      - name: RUST_LOG
        value: debug
  worker:
    command: cargo run
"#;
        let result = chunk_source_with_meta(src, Language::Yaml).unwrap();
        assert!(!result.parse_errors);

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(
            names.contains(&"services.web.image"),
            "missing YAML nested key: {result:?}"
        );
        assert!(
            names.contains(&"services.web.env.name"),
            "missing YAML list item key: {result:?}"
        );
        assert!(
            names.contains(&"services.worker.command"),
            "missing YAML sibling key: {result:?}"
        );

        let web = result
            .chunks
            .iter()
            .find(|c| c.symbol == "services.web")
            .unwrap();
        assert!(web.content.contains("image: nginx"));
        assert!(web.content.contains("env:"));
    }

    #[test]
    fn test_toml_path_key_chunks() {
        assert_eq!(Language::from_extension("toml"), Some(Language::Toml));

        let src = r#"
[package]
name = "pluck"
version = "0.4.0"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }

[[bin]]
name = "pluck"
path = "src/main.rs"
"#;
        let result = chunk_source_with_meta(src, Language::Toml).unwrap();
        assert!(!result.parse_errors);

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(
            names.contains(&"package.name"),
            "missing TOML package key: {result:?}"
        );
        assert!(
            names.contains(&"workspace.dependencies.serde"),
            "missing TOML dependency key: {result:?}"
        );
        assert!(
            names.contains(&"bin[].path"),
            "missing TOML array table key: {result:?}"
        );

        let deps = result
            .chunks
            .iter()
            .find(|c| c.symbol == "workspace.dependencies")
            .unwrap();
        assert!(deps.content.contains("serde"));
    }

    // ── Dockerfile ───────────────────────────────────────────────────────

    #[test]
    fn test_dockerfile_stages_instructions_and_install_blocks() {
        assert_eq!(
            Language::from_path(std::path::Path::new("Dockerfile")),
            Some(Language::Dockerfile)
        );
        assert_eq!(
            Language::from_path(std::path::Path::new("docker/app.Dockerfile")),
            Some(Language::Dockerfile)
        );
        assert_eq!(
            Language::from_path(std::path::Path::new("Containerfile.prod")),
            Some(Language::Dockerfile)
        );

        let src = r#"
# syntax=docker/dockerfile:1.7
FROM rust:1.78 AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN apt-get update && apt-get install -y pkg-config \
    libssl-dev && cargo fetch
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN <<EOF
apt-get update
apt-get install -y ca-certificates
EOF
COPY --from=builder /app/target/release/pluck /usr/local/bin/pluck
ENTRYPOINT ["pluck"]
"#;
        let result = chunk_source_with_meta(src, Language::Dockerfile).unwrap();
        assert!(!result.parse_errors);

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(
            names.contains(&"stage: builder"),
            "missing named stage: {result:?}"
        );
        assert!(
            names.contains(&"stage 2: debian:bookworm-slim"),
            "missing final image stage: {result:?}"
        );
        assert!(
            names.contains(&"deps: Cargo.toml Cargo.lock ./"),
            "missing dependency manifest copy: {result:?}"
        );
        assert!(
            names.contains(&"install: apt-get install"),
            "missing apt install block: {result:?}"
        );
        assert!(
            names.contains(&"RUN cargo build --release"),
            "missing generic RUN instruction: {result:?}"
        );

        let install = result
            .chunks
            .iter()
            .find(|c| c.symbol == "install: apt-get install")
            .unwrap();
        assert!(install.content.contains("libssl-dev"));
        assert!(install.content.contains("cargo fetch"));

        let heredoc = result
            .chunks
            .iter()
            .find(|c| {
                c.symbol == "install: apt-get install"
                    && c.content.contains("apt-get install -y ca-certificates")
            })
            .unwrap();
        assert_eq!(heredoc.symbol, "install: apt-get install");
        assert!(heredoc.content.contains("EOF"));
    }

    // ── Shell ────────────────────────────────────────────────────────────

    #[test]
    fn test_shell_functions_case_arms_and_sections() {
        assert_eq!(Language::from_extension("sh"), Some(Language::Shell));
        assert_eq!(Language::from_extension("bash"), Some(Language::Shell));
        assert_eq!(
            Language::from_path(std::path::Path::new(".zshrc")),
            Some(Language::Shell)
        );

        let src = r#"
#!/usr/bin/env bash
set -euo pipefail

# === Build helpers ===
# Compile the release binary.
build_release() {
  cargo build --release
}

function deploy_app {
  local target="${1:-staging}"
  case "$target" in
    prod|production)
      ./scripts/deploy-prod.sh
      ;;
    staging)
      ./scripts/deploy-staging.sh
      ;;
    *)
      echo "unknown target"
      return 1
      ;;
  esac
}

# Cleanup
cleanup () {
  rm -rf target/tmp
}
"#;
        let result = chunk_source_with_meta(src, Language::Shell).unwrap();
        assert!(!result.parse_errors);

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(
            names.contains(&"section: Build helpers"),
            "missing shell section: {result:?}"
        );
        assert!(
            names.contains(&"build_release"),
            "missing name() function: {result:?}"
        );
        assert!(
            names.contains(&"deploy_app"),
            "missing function keyword form: {result:?}"
        );
        assert!(
            names.contains(&"cleanup"),
            "missing spaced function form: {result:?}"
        );
        assert!(
            names.contains(&"case: prod|production"),
            "missing prod case arm: {result:?}"
        );
        assert!(
            names.contains(&"case: staging"),
            "missing staging case arm: {result:?}"
        );
        assert!(
            names.contains(&"case: *"),
            "missing fallback case arm: {result:?}"
        );

        let build = result
            .chunks
            .iter()
            .find(|c| c.symbol == "build_release")
            .unwrap();
        assert_eq!(build.kind, ChunkKind::Function);
        assert!(build.doc_comment.contains("Compile the release binary."));
        assert!(build.content.contains("cargo build --release"));

        let prod = result
            .chunks
            .iter()
            .find(|c| c.symbol == "case: prod|production")
            .unwrap();
        assert_eq!(prod.kind, ChunkKind::Module);
        assert!(prod.content.contains("deploy-prod"));
    }

    // ── Swift ─────────────────────────────────────────────────────────────

    #[test]
    fn test_swift_class_struct_protocol_extension_and_imports() {
        assert_eq!(Language::from_extension("swift"), Some(Language::Swift));

        let src = r#"
import Foundation
import os.log

/// Token lifecycle operations.
class AuthService {
    let store: TokenStore

    init(store: TokenStore) {
        self.store = store
    }

    func verify(_ token: String) -> Bool {
        return store.lookup(token) != nil
    }
}

struct LoginRequest {
    let token: String
}

protocol TokenStore {
    func lookup(_ token: String) -> String?
}

enum AuthStatus {
    case valid
    case expired
}

extension String {
    func normalizeToken() -> String {
        return trimmingCharacters(in: .whitespaces).lowercased()
    }
}
"#;
        let result = chunk_source_with_meta(src, Language::Swift).unwrap();
        assert!(!result.parse_errors, "Swift parse errors: {result:?}");

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(names.contains(&"AuthService"), "missing class: {result:?}");
        assert!(names.contains(&"verify"), "missing member func: {result:?}");
        assert!(
            names.contains(&"LoginRequest"),
            "missing struct: {result:?}"
        );
        assert!(
            names.contains(&"TokenStore"),
            "missing protocol: {result:?}"
        );
        assert!(names.contains(&"AuthStatus"), "missing enum: {result:?}");
        assert!(
            names.contains(&"normalizeToken"),
            "missing extension method: {result:?}"
        );

        let class = result
            .chunks
            .iter()
            .find(|c| c.symbol == "AuthService" && c.kind == ChunkKind::Class)
            .expect("AuthService class missing");
        assert!(
            class.doc_comment.contains("Token lifecycle operations"),
            "missing /// doc: {:?}",
            class.doc_comment
        );

        let verify = result.chunks.iter().find(|c| c.symbol == "verify").unwrap();
        assert_eq!(verify.kind, ChunkKind::Method);
        assert!(
            verify.callees.contains(&"lookup".to_string()),
            "verify callees missing lookup: {:?}",
            verify.callees
        );

        assert!(
            result.imports.contains(&"Foundation".to_string()),
            "missing import: {:?}",
            result.imports
        );
        assert!(
            result.imports.contains(&"os.log".to_string()),
            "missing dotted import: {:?}",
            result.imports
        );
    }

    // ── Ruby ──────────────────────────────────────────────────────────────

    #[test]
    fn test_ruby_class_module_method_and_singleton() {
        assert_eq!(Language::from_extension("rb"), Some(Language::Ruby));

        let src = r#"
require "json"
require_relative "store"

module Auth
  class Service
    def initialize(store)
      @store = store
    end

    def verify(token)
      @store.lookup(token)
    end

    def self.build
      new(TokenStore.new)
    end
  end

  module Helpers
    def normalize(token)
      token.strip.downcase
    end
  end
end
"#;
        let result = chunk_source_with_meta(src, Language::Ruby).unwrap();
        assert!(!result.parse_errors, "Ruby parse errors: {result:?}");

        let names: Vec<&str> = result.chunks.iter().map(|c| c.symbol.as_str()).collect();
        assert!(names.contains(&"Auth"), "missing module: {result:?}");
        assert!(names.contains(&"Service"), "missing class: {result:?}");
        assert!(
            names.contains(&"verify"),
            "missing instance method: {result:?}"
        );
        assert!(
            names.contains(&"build"),
            "missing singleton method: {result:?}"
        );
        assert!(
            names.contains(&"Helpers"),
            "missing nested module: {result:?}"
        );
        assert!(
            names.contains(&"normalize"),
            "missing module method: {result:?}"
        );

        assert!(
            result
                .chunks
                .iter()
                .any(|c| c.symbol == "Auth" && c.kind == ChunkKind::Module),
            "Auth should be Module kind: {result:?}"
        );
        assert!(
            result
                .chunks
                .iter()
                .any(|c| c.symbol == "Service" && c.kind == ChunkKind::Class),
            "Service should be Class kind: {result:?}"
        );

        let verify = result.chunks.iter().find(|c| c.symbol == "verify").unwrap();
        assert_eq!(verify.kind, ChunkKind::Method);
        assert!(
            verify.callees.contains(&"lookup".to_string()),
            "verify callees missing lookup: {:?}",
            verify.callees
        );
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
        assert!(r
            .imports
            .iter()
            .any(|i| i.contains("std::collections::HashMap")));
        assert!(r
            .imports
            .iter()
            .any(|i| i.contains("crate::index::PluckIndex")));
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
        assert!(r
            .imports
            .iter()
            .any(|i| i.contains(".local") || i == ".local"));
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

    #[test]
    fn imports_java() {
        let src = r#"
package com.example;

import java.util.List;
import static java.util.Collections.emptyList;

class Example {}
"#;
        let r = chunk_source_with_meta(src, Language::Java).unwrap();
        assert!(r.imports.contains(&"java.util.List".to_string()));
        assert!(r
            .imports
            .contains(&"java.util.Collections.emptyList".to_string()));
    }
}
