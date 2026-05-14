//! File outlining: turn a source file into a token-efficient symbol map.
//!
//! Default behavior of `pluck.read`: emit a compact outline (symbol list +
//! signatures, plus inline bodies for short symbols) instead of the full file.
//! Designed to beat `cat` by ~10x in tokens for files above the inline
//! threshold while staying byte-equivalent for short files.

use std::fmt::Write;
use std::path::Path;

use anyhow::Result;

use crate::chunker::{chunk_source, Chunk, ChunkKind, Language};

/// Files at or under this many lines are emitted raw (cat-equivalent).
/// Below this, outlining hurts more than it helps.
pub const RAW_BELOW_LINES: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutlineMode {
    /// Raw file content (file too small or unsupported language).
    Raw,
    /// Symbol outline.
    Symbols,
}

#[derive(Debug, Clone)]
pub struct Outline {
    pub path: String,
    pub total_lines: u32,
    pub mode: OutlineMode,
    pub entries: Vec<OutlineEntry>,
    /// Raw source kept around for Raw mode rendering, and for inline-body fetches.
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct OutlineEntry {
    pub kind: ChunkKind,
    pub symbol: String,
    pub start_line: u32,
    pub end_line: u32,
    pub signature: String,
}

pub fn outline_file(path: &Path) -> Result<Outline> {
    let src = std::fs::read_to_string(path)?;
    let path_str = path.to_string_lossy().into_owned();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = Language::from_extension(ext);
    Ok(outline_source(&src, lang, &path_str))
}

pub fn outline_source(src: &str, lang: Option<Language>, path: &str) -> Outline {
    let total_lines = count_lines(src);

    // Raw mode: too small, or no language support.
    let Some(lang) = lang else {
        return raw(path, src, total_lines);
    };
    if total_lines <= RAW_BELOW_LINES {
        return raw(path, src, total_lines);
    }

    let chunks = match chunk_source(src, lang) {
        Ok(c) if !c.is_empty() => c,
        // No chunks recovered → fall back to raw, agent still gets full file.
        _ => return raw(path, src, total_lines),
    };

    let entries = chunks
        .into_iter()
        .map(|c: Chunk| OutlineEntry {
            kind: c.kind,
            symbol: c.symbol,
            start_line: c.start_line,
            end_line: c.end_line,
            signature: c.signature,
        })
        .collect();

    Outline {
        path: path.to_string(),
        total_lines,
        mode: OutlineMode::Symbols,
        entries,
        source: src.to_string(),
    }
}

fn raw(path: &str, src: &str, total_lines: u32) -> Outline {
    Outline {
        path: path.to_string(),
        total_lines,
        mode: OutlineMode::Raw,
        entries: Vec::new(),
        source: src.to_string(),
    }
}

fn count_lines(src: &str) -> u32 {
    if src.is_empty() {
        return 0;
    }
    let nl = src.bytes().filter(|&b| b == b'\n').count() as u32;
    if src.ends_with('\n') {
        nl
    } else {
        nl + 1
    }
}

/// Render an outline as the byte-stream returned to the agent.
///
/// Format (symbols mode):
/// ```text
/// path/to/file.ts (245 lines)
///
/// L1-42 class AuthService extends Base implements UserStore
/// L8-19   method login(user: string, pass: string): Promise<AuthResult>
/// L21-30  method logout(): void
/// L45-52 fn validateToken(token: string): boolean
///   function validateToken(token: string): boolean {
///     return token.length === 36;
///   }
/// L54-120 async fn handleRequest(req: Request): Promise<Response>
/// ```
pub fn render(outline: &Outline) -> String {
    match outline.mode {
        OutlineMode::Raw => outline.source.clone(),
        OutlineMode::Symbols => render_symbols(outline),
    }
}

fn render_symbols(o: &Outline) -> String {
    let mut out = String::with_capacity(256 + o.entries.len() * 80);
    writeln!(out, "{} ({} lines)", o.path, o.total_lines).unwrap();
    writeln!(out).unwrap();

    // Signature-only outline. Methods indent under their class. Body lookup is
    // a separate call (pluck.symbol); inlining bodies here re-prints what the
    // signature already describes and inflates token count.
    for e in &o.entries {
        let prefix = match e.kind {
            ChunkKind::Method => "  ",
            _ => "",
        };
        let sig = compact_signature(&e.signature);
        writeln!(out, "{prefix}L{}-{} {sig}", e.start_line, e.end_line).unwrap();
    }

    out
}

/// Strip leading visibility/modifier noise so the signature reads compactly.
fn compact_signature(sig: &str) -> String {
    // Collapse internal whitespace runs (tree-sitter signatures preserve
    // newlines in multi-line parameter lists).
    let mut buf = String::with_capacity(sig.len());
    let mut prev_space = false;
    for c in sig.chars() {
        if c.is_whitespace() {
            if !prev_space {
                buf.push(' ');
            }
            prev_space = true;
        } else {
            buf.push(c);
            prev_space = false;
        }
    }
    buf.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_file_uses_raw_mode() {
        let src = "function a() { return 1; }\n";
        let o = outline_source(src, Some(Language::TypeScript), "tiny.ts");
        assert_eq!(o.mode, OutlineMode::Raw);
        assert_eq!(render(&o), src);
    }

    #[test]
    fn unsupported_language_uses_raw_mode() {
        let src = "anything here\n";
        let o = outline_source(src, None, "x.unknown");
        assert_eq!(o.mode, OutlineMode::Raw);
    }

    #[test]
    fn large_file_uses_symbol_outline() {
        let mut src = String::new();
        for i in 0..30 {
            src.push_str(&format!(
                "async function fn_{i}(arg: string): Promise<void> {{\n  console.log(arg);\n  return;\n}}\n\n"
            ));
        }
        let o = outline_source(&src, Some(Language::TypeScript), "big.ts");
        assert_eq!(o.mode, OutlineMode::Symbols);
        assert_eq!(o.entries.len(), 30);
        let rendered = render(&o);
        assert!(rendered.starts_with("big.ts ("));
        assert!(rendered.contains("L1-4 "));
        assert!(rendered.contains("fn_0"));
        // Signature only — no body content duplicated.
        assert!(!rendered.contains("console.log"));
    }

    #[test]
    fn rendered_outline_strictly_smaller_than_source_for_large_files() {
        let mut src = String::new();
        for i in 0..50 {
            src.push_str(&format!("function fn_{i}(x: number): number {{\n"));
            for _ in 0..15 {
                src.push_str("  x = x * 2 + 1;\n");
            }
            src.push_str("  return x;\n}\n\n");
        }
        let o = outline_source(&src, Some(Language::TypeScript), "p.ts");
        let rendered = render(&o);
        assert!(
            rendered.len() < src.len() / 2,
            "outline must shrink by at least half on body-heavy file; src={} rendered={}",
            src.len(),
            rendered.len()
        );
    }
}
