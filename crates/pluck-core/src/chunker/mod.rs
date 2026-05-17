mod config;
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
    pub parse_errors: bool,
}

pub fn chunk_file(path: &Path) -> Result<Vec<Chunk>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang =
        Language::from_extension(ext).with_context(|| format!("unsupported extension: {ext:?}"))?;
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
        Language::TypeScript
        | Language::Tsx
        | Language::JavaScript
        | Language::Go
        | Language::Java
        | Language::Scss => line.strip_prefix("//").map(|s| s.trim().to_string()),
        Language::Python => line.strip_prefix('#').map(|s| s.trim().to_string()),
        Language::Html => line
            .strip_prefix("<!--")
            .and_then(|s| s.strip_suffix("-->"))
            .map(|s| s.trim().to_string()),
        Language::Css
        | Language::Markdown
        | Language::Mdx
        | Language::Json
        | Language::Yaml
        | Language::Toml => None,
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
