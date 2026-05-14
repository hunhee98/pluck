//! `pluckd` server handler.
//!
//! Holds a per-repo BM25 index and a session-scoped state map, and exposes
//! the pluck.* tools over MCP. Tool descriptions are shipped via
//! [`crate::descriptions`] (compiled in from `docs/mcp-descriptions/`).
//!
//! Phase 1 ships three tools wired through `pluck-core`:
//!
//!   pluck.read    file outline (or raw cat parity)
//!   pluck.search  BM25 chunk search with 12% noise floor
//!   pluck.grep    keyword search via ripgrep passthrough
//!
//! pluck.symbol, pluck.peek, pluck.expand are registered with placeholder
//! handlers so the agent sees the full tool set during the MCP handshake
//! and can plan accordingly — they're filled in in subsequent phases.

use std::path::PathBuf;
use std::process::{Command as Shell, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use pluck_core::callees::extract_callees;
use pluck_core::chunker::Language;
use pluck_core::index::{PluckIndex, SearchHit};
use pluck_core::indexer::index_repo;
use pluck_core::outliner::{outline_source, render as render_outline};
use pluck_core::watcher::{spawn_watcher, WatcherHandle, DEFAULT_DEBOUNCE};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    ErrorData as McpError, ServerHandler,
};
use serde::Deserialize;

use crate::session::SessionState;

#[derive(Clone)]
pub struct PluckServer {
    inner: Arc<ServerInner>,
    tool_router: ToolRouter<Self>,
}

struct ServerInner {
    repo_root: PathBuf,
    index: Arc<PluckIndex>,
    session: Mutex<SessionState>,
    // Held for the server's lifetime; drop stops the watcher task.
    _watcher: Option<WatcherHandle>,
}

impl PluckServer {
    /// Build a server, indexing `repo_root` on startup. Use this from
    /// non-async contexts (tests, CLI) — the watcher is *not* attached
    /// because spawning the tokio task requires a runtime. Use
    /// [`PluckServer::new_with_watcher`] from within a tokio runtime to
    /// get incremental reindex.
    pub fn new(repo_root: PathBuf) -> Result<Self> {
        Self::build(repo_root, None)
    }

    /// Build a server and attach a notify-based watcher so file changes
    /// reindex automatically. Must be called from a tokio runtime.
    pub fn new_with_watcher(repo_root: PathBuf) -> Result<Self> {
        Self::build(repo_root, Some(DEFAULT_DEBOUNCE))
    }

    fn build(repo_root: PathBuf, debounce: Option<std::time::Duration>) -> Result<Self> {
        let index = Arc::new(PluckIndex::in_ram()?);
        let stats = index_repo(&index, &repo_root)?;
        tracing::info!(
            files = stats.files_indexed,
            chunks = stats.chunks_indexed,
            repo = ?repo_root,
            "indexed repo on startup"
        );

        let watcher = match debounce {
            Some(d) => match spawn_watcher(repo_root.clone(), Arc::clone(&index), d) {
                Ok(w) => Some(w),
                Err(e) => {
                    tracing::warn!("watcher failed to start: {e}; running without auto-reindex");
                    None
                }
            },
            None => None,
        };

        Ok(Self {
            inner: Arc::new(ServerInner {
                repo_root,
                index,
                session: Mutex::new(SessionState::default()),
                _watcher: watcher,
            }),
            tool_router: Self::tool_router(),
        })
    }

    fn resolve_in_repo(&self, path: &str) -> PathBuf {
        let p = PathBuf::from(path);
        if p.is_absolute() {
            p
        } else {
            self.inner.repo_root.join(p)
        }
    }
}

// ── Tool parameter schemas ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadParams {
    /// Path to the file, relative to the repo root (absolute paths also work).
    pub path: String,
    /// Return byte-equivalent cat output instead of the outline.
    #[serde(default)]
    pub raw: bool,
    /// Inclusive line range, e.g. "100-200".
    #[serde(default)]
    pub lines: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Natural-language or keyword query.
    pub query: String,
    /// Maximum number of hits to return (after the 12% noise floor).
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Compact mode: only score, path, range, and matching lines. Lossy —
    /// useful for pure discovery. Default is the lossless full-body
    /// rendering.
    #[serde(default)]
    pub compact: bool,
}

fn default_top_k() -> usize {
    10
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GrepParams {
    /// Pattern (literal by default; pass `-e <regex>` via `args` for regex).
    pub pattern: String,
    /// Extra `rg` flags. E.g. `["-A", "5", "--type", "ts"]`.
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolParams {
    /// Symbol name (optionally path-qualified: `auth/handleLogin`).
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PeekParams {
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExpandParams {
    pub name: String,
    #[serde(default = "default_hop")]
    pub hop: u8,
}

fn default_hop() -> u8 {
    1
}

// ── Tool router ─────────────────────────────────────────────────────────────

#[tool_router(router = tool_router)]
impl PluckServer {
    #[doc = include_str!("../../../docs/mcp-descriptions/read.md")]
    #[tool(name = "pluck.read")]
    pub async fn read(&self, Parameters(p): Parameters<ReadParams>) -> Result<String, McpError> {
        let path = self.resolve_in_repo(&p.path);
        let src = std::fs::read_to_string(&path).map_err(|e| {
            McpError::invalid_params(format!("failed to read {:?}: {e}", p.path), None)
        })?;

        if p.raw {
            return Ok(src);
        }
        if let Some(range) = p.lines {
            let (s, e) = parse_line_range(&range)?;
            let mut out = String::new();
            for (i, line) in src.lines().enumerate() {
                let n = (i + 1) as u32;
                if n >= s && n <= e {
                    out.push_str(line);
                    out.push('\n');
                }
            }
            return Ok(out);
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let lang = Language::from_extension(ext);
        let display = path.to_string_lossy();
        let outline = outline_source(&src, lang, &display);
        Ok(render_outline(&outline))
    }

    #[doc = include_str!("../../../docs/mcp-descriptions/search.md")]
    #[tool(name = "pluck.search")]
    pub async fn search(&self, Parameters(p): Parameters<SearchParams>) -> Result<String, McpError> {
        let hits = self
            .inner
            .index
            .search_with_cutoff(&p.query, p.top_k, 0.12)
            .map_err(|e| McpError::internal_error(format!("search failed: {e}"), None))?;
        if hits.is_empty() {
            return Ok("(no hits)\n".to_string());
        }

        // Session dedup — split hits into chunks the agent has already
        // received this session and chunks it has not. The first set is
        // emitted as a one-line `[already-shown]` placeholder; the bytes
        // are already in the agent's context window, repeating them is
        // pure waste. The second set goes out as a normal full or
        // compact rendering.
        let (already_shown, fresh): (Vec<_>, Vec<_>) = {
            let session = self.inner.session.lock().expect("session mutex");
            hits.into_iter()
                .partition(|h| session.was_seen(h.chunk_id))
        };

        // Mark the fresh chunks before we lose the borrow scope.
        {
            let mut s = self.inner.session.lock().expect("session mutex");
            for h in &fresh {
                s.mark_seen(h.chunk_id);
            }
        }

        let mut out = String::new();
        if p.compact {
            out.push_str(&render_compact(&fresh, &p.query));
        } else {
            out.push_str(&render_full(&fresh));
        }
        for h in &already_shown {
            out.push_str(&format!(
                "[already-shown: {}:L{}-{} {} score={:.4}]\n",
                h.path, h.start_line, h.end_line, h.symbol, h.score
            ));
        }
        Ok(out)
    }

    #[doc = include_str!("../../../docs/mcp-descriptions/grep.md")]
    #[tool(name = "pluck.grep")]
    pub async fn grep(&self, Parameters(p): Parameters<GrepParams>) -> Result<String, McpError> {
        let mut cmd = Shell::new("rg");
        cmd.arg(&p.pattern);
        for a in &p.args {
            cmd.arg(a);
        }
        cmd.current_dir(&self.inner.repo_root);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let out = cmd.output().map_err(|e| {
            McpError::internal_error(format!("failed to invoke ripgrep: {e}"), None)
        })?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // ripgrep exits 1 when there are no matches — that's not an MCP
        // error; just return the (empty) stdout so the agent gets a
        // truthful answer.
        Ok(stdout)
    }

    #[doc = include_str!("../../../docs/mcp-descriptions/symbol.md")]
    #[tool(name = "pluck.symbol")]
    pub async fn symbol(
        &self,
        Parameters(p): Parameters<SymbolParams>,
    ) -> Result<String, McpError> {
        // Path-qualified form: `auth/handleLogin` → look up `handleLogin`
        // and require the chunk's path to contain `auth`.
        let (path_filter, name) = match p.name.rsplit_once('/') {
            Some((path, sym)) => (Some(path), sym),
            None => (None, p.name.as_str()),
        };

        let hits = self
            .inner
            .index
            .lookup_symbol(name, path_filter)
            .map_err(|e| McpError::internal_error(format!("symbol lookup failed: {e}"), None))?;

        if hits.is_empty() {
            return Ok(format!(
                "no symbol named `{}` found{}.\n\nTry pluck.search with the same name as a free-text query — BM25 picks up partial / fuzzy matches.\n",
                p.name,
                path_filter.map(|p| format!(" under path `{p}`")).unwrap_or_default()
            ));
        }

        // Apply session dedup, like pluck.search. New chunks get full
        // bodies; chunks the agent already saw this session collapse to
        // a placeholder.
        let (already_shown, fresh): (Vec<_>, Vec<_>) = {
            let session = self.inner.session.lock().expect("session mutex");
            hits.into_iter()
                .partition(|h| session.was_seen(h.chunk_id))
        };
        {
            let mut s = self.inner.session.lock().expect("session mutex");
            for h in &fresh {
                s.mark_seen(h.chunk_id);
            }
        }

        // Ambiguous case (more than one fresh hit): emit a one-line list
        // instead of dumping every body. The agent can then re-call with
        // a path-qualified name. The unit tests pin this behavior.
        if fresh.len() > 1 {
            let mut out = format!(
                "`{}` is ambiguous — {} candidates. Disambiguate with `<path>/<name>`:\n",
                p.name,
                fresh.len()
            );
            for h in &fresh {
                out.push_str(&format!(
                    "  {}:L{}-{}  {} ({:?})\n",
                    h.path, h.start_line, h.end_line, h.symbol, h.kind
                ));
            }
            for h in &already_shown {
                out.push_str(&format!(
                    "  [already-shown: {}:L{}-{} {}]\n",
                    h.path, h.start_line, h.end_line, h.symbol
                ));
            }
            return Ok(out);
        }

        let mut out = String::new();
        for h in &fresh {
            out.push_str(&format!(
                "{}:L{}-{}  {} ({:?})\n",
                h.path, h.start_line, h.end_line, h.symbol, h.kind
            ));
            out.push_str(&h.content);
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        for h in &already_shown {
            out.push_str(&format!(
                "[already-shown: {}:L{}-{} {}]\n",
                h.path, h.start_line, h.end_line, h.symbol
            ));
        }
        Ok(out)
    }

    #[doc = include_str!("../../../docs/mcp-descriptions/peek.md")]
    #[tool(name = "pluck.peek")]
    pub async fn peek(
        &self,
        Parameters(p): Parameters<PeekParams>,
    ) -> Result<String, McpError> {
        let (path_filter, name) = match p.name.rsplit_once('/') {
            Some((path, sym)) => (Some(path), sym),
            None => (None, p.name.as_str()),
        };

        let hits = self
            .inner
            .index
            .lookup_symbol(name, path_filter)
            .map_err(|e| McpError::internal_error(format!("symbol lookup failed: {e}"), None))?;

        if hits.is_empty() {
            return Ok(format!(
                "no symbol named `{}` found{}.\n",
                p.name,
                path_filter.map(|p| format!(" under path `{p}`")).unwrap_or_default(),
            ));
        }

        // Ambiguous — show candidate list (peek wants surgical answers,
        // dumping every signature defeats the purpose).
        if hits.len() > 1 {
            let mut out = format!(
                "`{}` matches {} symbols — disambiguate with `<path>/<name>`:\n",
                p.name,
                hits.len()
            );
            for h in &hits {
                out.push_str(&format!(
                    "  {}:L{}-{}  {} ({:?})\n",
                    h.path, h.start_line, h.end_line, h.symbol, h.kind
                ));
            }
            return Ok(out);
        }

        // Single match: signature + direct callees. No body — peek's value
        // proposition is "10x cheaper than pluck.symbol when you only need
        // the interface".
        let h = &hits[0];
        let lang = std::path::Path::new(&h.path)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Language::from_extension)
            .unwrap_or(Language::TypeScript);
        let callees = extract_callees(&h.content, lang);

        let mut out = format!(
            "{}:L{}-{}  {} ({:?})\n",
            h.path, h.start_line, h.end_line, h.symbol, h.kind
        );
        out.push_str(&h.signature);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        if callees.is_empty() {
            out.push_str("  (no direct callees)\n");
        } else {
            out.push_str("  calls: ");
            out.push_str(&callees.join(", "));
            out.push('\n');
        }
        Ok(out)
    }

    #[doc = include_str!("../../../docs/mcp-descriptions/expand.md")]
    #[tool(name = "pluck.expand")]
    async fn expand(&self, Parameters(p): Parameters<ExpandParams>) -> Result<String, McpError> {
        let _ = p;
        Ok("pluck.expand — not yet implemented (Phase 4). Use pluck.search for the symbol, then follow callees by hand.".into())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PluckServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "pluck — token-efficient code reading. Prefer pluck.read/search/grep \
                 over Bash cat/grep/rg whenever the target is inside the indexed repo. \
                 All pluck tools have a --raw or equivalent fallback that matches \
                 cat/grep byte-for-byte if you need exact parity.",
            )
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn parse_line_range(s: &str) -> Result<(u32, u32), McpError> {
    let (a, b) = s
        .split_once('-')
        .ok_or_else(|| McpError::invalid_params(format!("expected 'start-end', got {s:?}"), None))?;
    let a: u32 = a
        .trim()
        .parse()
        .map_err(|_| McpError::invalid_params("bad start line", None))?;
    let b: u32 = b
        .trim()
        .parse()
        .map_err(|_| McpError::invalid_params("bad end line", None))?;
    if a == 0 || b < a {
        return Err(McpError::invalid_params(
            format!("invalid line range {s}"),
            None,
        ));
    }
    Ok((a, b))
}

fn render_full(hits: &[SearchHit]) -> String {
    let mut out = String::new();
    for h in hits {
        out.push_str(&format!(
            "{:.4}  {}:L{}-{}  {} ({:?})\n",
            h.score, h.path, h.start_line, h.end_line, h.symbol, h.kind
        ));
        out.push_str(&h.content);
        out.push_str("\n\n");
    }
    out
}

fn render_compact(hits: &[SearchHit], query: &str) -> String {
    let words: Vec<String> = query
        .split_whitespace()
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_lowercase())
        .collect();
    let mut out = String::new();
    for h in hits {
        out.push_str(&format!(
            "{:.4}\t{}:{}-{}\n",
            h.score, h.path, h.start_line, h.end_line
        ));
        for (i, line) in h.content.lines().enumerate() {
            let lower = line.to_lowercase();
            if words.iter().any(|w| lower.contains(w)) {
                let ln = h.start_line as usize + i;
                out.push_str(&format!("  L{ln}: {}\n", line.trim()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_range_ok() {
        assert_eq!(parse_line_range("10-20").unwrap(), (10, 20));
    }
    #[test]
    fn parse_line_range_rejects_inverted() {
        assert!(parse_line_range("20-10").is_err());
    }
    #[test]
    fn parse_line_range_rejects_missing_dash() {
        assert!(parse_line_range("10").is_err());
    }

    #[tokio::test]
    async fn server_serves_read_outline_for_pluck_repo() {
        // Index this very crate's directory and ask for the outline of
        // pluck-core/src/lib.rs — a sanity check that the wire-up works
        // end-to-end against real code.
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo.clone()).expect("server new");
        let res = server
            .read(Parameters(ReadParams {
                path: "crates/pluck-core/src/lib.rs".to_string(),
                raw: false,
                lines: None,
            }))
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn pluck_peek_returns_signature_plus_callees() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");
        let out = server
            .peek(Parameters(PeekParams {
                name: "chunk_source".into(),
            }))
            .await
            .expect("peek");

        // Signature line is present, no full body.
        assert!(out.contains("pub fn chunk_source"), "got: {out}");
        assert!(out.contains("calls:"), "got: {out}");
        // peek must be dramatically smaller than pluck.symbol for the
        // same name — that's the entire reason peek exists.
        let server2 = PluckServer::new(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf(),
        )
        .unwrap();
        let symbol_out = server2
            .symbol(Parameters(SymbolParams {
                name: "chunk_source".into(),
            }))
            .await
            .expect("symbol");
        assert!(
            out.len() * 2 < symbol_out.len(),
            "peek should be at least 2x smaller than symbol; peek={} symbol={}",
            out.len(),
            symbol_out.len()
        );
    }

    #[tokio::test]
    async fn pluck_peek_unknown_name() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");
        let out = server
            .peek(Parameters(PeekParams {
                name: "definitely_not_a_real_function_xyzzy".into(),
            }))
            .await
            .expect("peek");
        assert!(out.contains("no symbol"), "got: {out}");
    }

    #[tokio::test]
    async fn pluck_symbol_returns_named_function_body() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");

        // chunk_source is a public function defined in pluck-core/src/chunker/mod.rs.
        let out = server
            .symbol(Parameters(SymbolParams {
                name: "chunk_source".into(),
            }))
            .await
            .expect("symbol lookup");

        assert!(
            out.contains("pub fn chunk_source"),
            "expected symbol body, got: {out}"
        );
        assert!(out.contains("chunker/mod.rs"), "missing path: {out}");
    }

    #[tokio::test]
    async fn pluck_symbol_unknown_name_returns_no_match_message() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");
        let out = server
            .symbol(Parameters(SymbolParams {
                name: "definitely_not_a_real_function_xyzzy".into(),
            }))
            .await
            .expect("symbol lookup");
        assert!(out.contains("no symbol"), "got: {out}");
    }

    #[tokio::test]
    async fn pluck_symbol_repeat_call_uses_placeholder() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");
        let first = server
            .symbol(Parameters(SymbolParams {
                name: "chunk_source".into(),
            }))
            .await
            .expect("first lookup");
        let second = server
            .symbol(Parameters(SymbolParams {
                name: "chunk_source".into(),
            }))
            .await
            .expect("second lookup");
        // Second call should be much shorter (placeholder line only).
        assert!(
            second.len() < first.len() / 4,
            "second call should collapse to placeholder; first={} second={}",
            first.len(),
            second.len()
        );
        assert!(
            second.contains("[already-shown:"),
            "expected placeholder in repeat: {second}"
        );
    }

    #[tokio::test]
    async fn session_dedup_replaces_repeat_results_with_placeholder() {
        // Same query twice. First call returns chunk bodies; second call
        // returns the same chunks as `[already-shown]` placeholders — same
        // metadata, none of the body bytes.
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");

        let first = server
            .search(Parameters(SearchParams {
                query: "chunk source".into(),
                top_k: 5,
                compact: false,
            }))
            .await
            .expect("first search");
        let second = server
            .search(Parameters(SearchParams {
                query: "chunk source".into(),
                top_k: 5,
                compact: false,
            }))
            .await
            .expect("second search");

        // Second call must shrink dramatically. We don't pin a hard ratio
        // because content/scoring can shift, but the second response must
        // contain only placeholder lines and zero body content.
        assert!(
            second.len() < first.len() / 4,
            "second call should be < 25% of first; got first={} second={}",
            first.len(),
            second.len()
        );
        // Every line in `second` must be either blank or a placeholder.
        for line in second.lines() {
            if line.trim().is_empty() {
                continue;
            }
            assert!(
                line.starts_with("[already-shown:"),
                "non-placeholder line on repeat: {line:?}"
            );
        }
        // Same chunks → recall preserved (placeholder must reference each
        // file path the first call returned).
        for line in first.lines() {
            // First-call lines look like "<score>  <path>:L<a>-<b>  ..."
            if let Some(path_seg) = line.split("  ").nth(1) {
                if let Some(path) = path_seg.split(':').next() {
                    if !path.is_empty() && path.contains('/') {
                        assert!(
                            second.contains(path),
                            "path {path:?} from first call missing on repeat:\n{second}"
                        );
                    }
                }
            }
        }
    }
}
