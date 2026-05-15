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

use std::path::{Component, Path, PathBuf};
use std::process::{Command as Shell, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use pluck_core::callees::extract_callees;
use pluck_core::chunker::Language;
use pluck_core::digest::{self, Format};
use pluck_core::index::{ImpactHit, PluckIndex, SearchHit};
use pluck_core::indexer::index_repo;
use pluck_core::outliner::{outline_source, render as render_outline};
use pluck_core::semantic::{selected_model_id, StaticEncoder};
use pluck_core::watcher::{spawn_watcher, WatcherHandle, DEFAULT_DEBOUNCE};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
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
    /// Test / CLI constructor. No watcher, no embedding encoder — pure
    /// BM25, no network or tokio runtime needed. `search_hybrid` still
    /// works (it degrades to BM25-only when no encoder is attached).
    pub fn new(repo_root: PathBuf) -> Result<Self> {
        Self::build(repo_root, None, false, None)
    }

    /// Production daemon constructor. Loads the embedding encoder (with
    /// PLUCK_DISABLE_EMBEDDINGS=1 escape hatch) and starts the notify
    /// watcher. Must be called inside a tokio runtime.
    pub fn new_with_watcher(repo_root: PathBuf) -> Result<Self> {
        Self::build(repo_root, Some(DEFAULT_DEBOUNCE), true, None)
    }

    fn build(
        repo_root: PathBuf,
        debounce: Option<std::time::Duration>,
        load_encoder: bool,
        // Test hook: when `Some`, overrides the env-resolved model id so
        // an integration test can simulate `load_or_fetch` failure
        // without racing on `PLUCK_EMBED_MODEL`. Production callers
        // pass `None` and the model is resolved via `selected_model_id`.
        model_id_override: Option<String>,
    ) -> Result<Self> {
        // Embedding encoder. Off entirely in test/CLI mode; in daemon
        // mode the load is opt-out via PLUCK_DISABLE_EMBEDDINGS=1. Load
        // failure is non-fatal — we log and degrade to BM25-only so
        // first-run network problems never break pluckd.
        let encoder: Option<Arc<StaticEncoder>> = if !load_encoder {
            None
        } else if std::env::var("PLUCK_DISABLE_EMBEDDINGS").is_ok() {
            tracing::info!("PLUCK_DISABLE_EMBEDDINGS set; running BM25-only");
            None
        } else {
            let model_id = model_id_override.unwrap_or_else(selected_model_id);
            match StaticEncoder::load_or_fetch(&model_id) {
                Ok(enc) => {
                    tracing::info!(
                        model = model_id,
                        dim = enc.dim(),
                        "embedding encoder loaded; hybrid search active"
                    );
                    Some(Arc::new(enc))
                }
                Err(e) => {
                    tracing::warn!("failed to load embedding model ({e}); running BM25-only");
                    None
                }
            }
        };

        let mut idx = PluckIndex::in_ram()?;
        if let Some(enc) = encoder.as_ref() {
            idx = idx.with_encoder(Arc::clone(enc));
        }
        let index = Arc::new(idx);
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

    /// Resolve a caller-supplied path against `repo_root` and enforce
    /// that the result stays inside the indexed repo.
    ///
    /// Logical (component-level) normalization only — `..` segments are
    /// folded so escape attempts like `subdir/../../etc/passwd` are
    /// caught. Symlink-following is not checked here; that boundary
    /// case is tracked for v0.1.x. For paths legitimately outside the
    /// repo, agents are expected to fall back to bash.
    fn resolve_in_repo(&self, path: &str) -> Result<PathBuf, McpError> {
        let raw = PathBuf::from(path);
        let joined = if raw.is_absolute() {
            raw
        } else {
            self.inner.repo_root.join(raw)
        };
        let normalized = normalize_path(&joined);
        let root = normalize_path(&self.inner.repo_root);
        if !normalized.starts_with(&root) {
            return Err(McpError::invalid_params(
                format!(
                    "pluck: {path}: path is outside the indexed repo ({}). \
                     Use bash for byte-level work outside the indexed root.",
                    root.display()
                ),
                None,
            ));
        }
        Ok(normalized)
    }
}

/// Component-level path normalization. Removes `.` segments and
/// resolves `..` segments by popping the preceding component. Does
/// not touch the filesystem (so it doesn't follow symlinks).
fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

// ── Tool parameter schemas ──────────────────────────────────────────────────

/// Largest file we'll buffer fully in memory for a single read.
/// Bigger files force `lines: "<a>-<b>"` or fall back to bash. Keeps
/// the daemon from being OOMed by one bad request.
pub const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;

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
    /// Working directory to grep in, relative to repo root or absolute.
    /// Defaults to the indexed repo root. Use this when the agent needs
    /// to grep a subtree (`src/auth/`) or somewhere outside the repo.
    #[serde(default)]
    pub cwd: Option<String>,
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DigestParams {
    /// The raw tool output to compress (cargo, npm, pytest, or GitHub Actions log).
    pub input: String,
    /// Force a specific format instead of auto-detecting.
    /// One of: cargo, npm, pnpm, yarn, bun, pytest, ci, gha, actions.
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ImpactParams {
    /// Symbol whose upstream callers you want. Leaf-matched and
    /// case-insensitive: `validate_token`, `Logger::new`, `db.insert`.
    pub name: String,
    /// BFS depth cap (1 = direct callers, 2 = callers-of-callers).
    /// Clamped to 3. Default 1.
    #[serde(default = "default_impact_depth")]
    pub depth: u8,
}

fn default_impact_depth() -> u8 {
    1
}

// ── Tool router ─────────────────────────────────────────────────────────────

#[tool_router(router = tool_router)]
impl PluckServer {
    #[doc = include_str!("../../../docs/mcp-descriptions/read.md")]
    #[tool(name = "read")]
    pub async fn read(&self, Parameters(p): Parameters<ReadParams>) -> Result<String, McpError> {
        let path = self.resolve_in_repo(&p.path)?;

        // Stat first — we want a cheap, cat-shaped failure before
        // trying to buffer the whole file in memory.
        let meta = std::fs::metadata(&path)
            .map_err(|e| McpError::invalid_params(read_err(&p.path, &e), None))?;
        if meta.is_dir() {
            return Err(McpError::invalid_params(
                format!("pluck.read: {}: is a directory", p.path),
                None,
            ));
        }
        if meta.len() > MAX_READ_BYTES {
            return Err(McpError::invalid_params(
                format!(
                    "pluck.read: {} is {} bytes (cap {}). Use `lines: \"A-B\"` to slice, or fall back to bash for the full file.",
                    p.path,
                    meta.len(),
                    MAX_READ_BYTES
                ),
                None,
            ));
        }

        // Read bytes once. UTF-8 is required for `raw` text output (MCP
        // responses are JSON strings) and for the outliner. Binary
        // files trip a clean diagnostic; bash remains the right tool
        // for byte-level work.
        let bytes = std::fs::read(&path)
            .map_err(|e| McpError::invalid_params(read_err(&p.path, &e), None))?;
        let src = match std::str::from_utf8(&bytes) {
            Ok(s) => s.to_string(),
            Err(_) => {
                return Err(McpError::invalid_params(
                    format!(
                        "pluck.read: {} is not valid UTF-8 (likely binary). Use bash `cat` for byte-level reads.",
                        p.path
                    ),
                    None,
                ));
            }
        };

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

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = Language::from_extension(ext);
        let display = path.to_string_lossy();
        let outline = outline_source(&src, lang, &display);
        Ok(render_outline(&outline))
    }

    #[doc = include_str!("../../../docs/mcp-descriptions/search.md")]
    #[tool(name = "search")]
    pub async fn search(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<String, McpError> {
        // search_hybrid auto-degrades to BM25 when no encoder is
        // attached, so callers don't need to branch on capability.
        let hits = self
            .inner
            .index
            .search_hybrid(&p.query, p.top_k, 0.12, None)
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
            hits.into_iter().partition(|h| session.was_seen(h.chunk_id))
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
    #[tool(name = "grep")]
    pub async fn grep(&self, Parameters(p): Parameters<GrepParams>) -> Result<String, McpError> {
        let cwd = match p.cwd.as_deref() {
            Some(c) => self.resolve_in_repo(c)?,
            None => self.inner.repo_root.clone(),
        };

        let mut cmd = Shell::new("rg");
        cmd.arg(&p.pattern);
        for a in &p.args {
            cmd.arg(a);
        }
        cmd.current_dir(&cwd);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let out = cmd.output().map_err(|e| {
            McpError::internal_error(
                format!("pluck.grep: failed to invoke ripgrep: {e} (is `rg` on PATH?)"),
                None,
            )
        })?;

        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let code = out.status.code().unwrap_or(-1);

        // ripgrep exit codes:
        //   0 = matches found
        //   1 = no matches (not an error; return empty stdout)
        //   2 = real error (bad pattern, unreadable file, etc.)
        // We propagate stderr on code 2 so the agent sees the same
        // diagnostic it would see from running `rg` in a shell.
        if code == 2 {
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            return Err(McpError::invalid_params(
                format!("pluck.grep: ripgrep exited 2:\n{stderr}"),
                None,
            ));
        }

        Ok(stdout)
    }

    #[doc = include_str!("../../../docs/mcp-descriptions/symbol.md")]
    #[tool(name = "symbol")]
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
            hits.into_iter().partition(|h| session.was_seen(h.chunk_id))
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
    #[tool(name = "peek")]
    pub async fn peek(&self, Parameters(p): Parameters<PeekParams>) -> Result<String, McpError> {
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
                path_filter
                    .map(|p| format!(" under path `{p}`"))
                    .unwrap_or_default(),
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
    #[tool(name = "expand")]
    pub async fn expand(
        &self,
        Parameters(p): Parameters<ExpandParams>,
    ) -> Result<String, McpError> {
        let hop = p.hop.clamp(1, 3); // beyond 3 hops the output explodes
        let (path_filter, name) = match p.name.rsplit_once('/') {
            Some((path, sym)) => (Some(path), sym),
            None => (None, p.name.as_str()),
        };

        let root_hits = self
            .inner
            .index
            .lookup_symbol(name, path_filter)
            .map_err(|e| McpError::internal_error(format!("symbol lookup failed: {e}"), None))?;

        if root_hits.is_empty() {
            return Ok(format!("no symbol named `{}` found.\n", p.name));
        }
        if root_hits.len() > 1 {
            // Same disambiguation contract as peek/symbol — don't dump
            // every candidate's body and call-graph.
            let mut out = format!(
                "`{}` is ambiguous — {} candidates. Disambiguate with `<path>/<name>`:\n",
                p.name,
                root_hits.len()
            );
            for h in &root_hits {
                out.push_str(&format!(
                    "  {}:L{}-{}  {} ({:?})\n",
                    h.path, h.start_line, h.end_line, h.symbol, h.kind
                ));
            }
            return Ok(out);
        }

        let root = &root_hits[0];

        // BFS over the call graph. Mark the root + each chunk we render in
        // the local visited set, so the same symbol never expands twice in
        // one response (otherwise mutually-recursive functions loop). Also
        // pipe each rendered chunk through the session dedup tracker so
        // pluck.search / pluck.read are aware of what expand revealed.
        let mut visited: std::collections::HashSet<u64> = std::collections::HashSet::new();
        visited.insert(root.chunk_id);
        {
            let mut s = self.inner.session.lock().expect("session mutex");
            s.mark_seen(root.chunk_id);
        }

        let mut out = format!(
            "{}:L{}-{}  {} ({:?})\n",
            root.path, root.start_line, root.end_line, root.symbol, root.kind
        );
        out.push_str(&root.content);
        if !out.ends_with('\n') {
            out.push('\n');
        }

        // Hop 1: direct callees of the root.
        let mut frontier: Vec<String> = callees_of(root);
        let mut total_rendered = 0usize;
        let max_per_level = 30;

        for level in 1..=hop {
            if frontier.is_empty() {
                break;
            }
            out.push_str(&format!("\n=== hop {level} ===\n"));
            let mut next_frontier: Vec<String> = Vec::new();

            for callee_name in frontier.iter().take(max_per_level) {
                let leaf = callee_leaf(callee_name);
                let hits = self
                    .inner
                    .index
                    .lookup_symbol(leaf, None)
                    .unwrap_or_default();
                let Some(hit) = hits.first() else {
                    // External / unindexed callee — emit a one-liner so
                    // the agent knows we saw it but didn't follow.
                    out.push_str(&format!("  · {callee_name}  (external / not indexed)\n"));
                    continue;
                };
                if !visited.insert(hit.chunk_id) {
                    out.push_str(&format!("  · {} — already expanded above\n", hit.symbol));
                    continue;
                }
                {
                    let mut s = self.inner.session.lock().expect("session mutex");
                    s.mark_seen(hit.chunk_id);
                }
                let nested =
                    pluck_core::callees::extract_callees(&hit.content, lang_for_path(&hit.path));
                out.push_str(&format!(
                    "  → {}:L{}-{}  {} ({:?})\n    {}\n",
                    hit.path,
                    hit.start_line,
                    hit.end_line,
                    hit.symbol,
                    hit.kind,
                    hit.signature.replace('\n', "\n    ")
                ));
                if !nested.is_empty() {
                    out.push_str("    calls: ");
                    out.push_str(&nested.join(", "));
                    out.push('\n');
                }
                next_frontier.extend(nested);
                total_rendered += 1;
            }

            if frontier.len() > max_per_level {
                out.push_str(&format!(
                    "  (+ {} more callees at this hop suppressed — re-call with a path-qualified name to drill into specific branches)\n",
                    frontier.len() - max_per_level
                ));
            }

            frontier = dedup_keep_order(next_frontier);
        }

        out.push_str(&format!(
            "\n[expanded {} callees across {} hop(s)]\n",
            total_rendered, hop
        ));
        Ok(out)
    }

    #[doc = include_str!("../../../docs/mcp-descriptions/digest.md")]
    #[tool(name = "digest")]
    pub async fn digest_tool(
        &self,
        Parameters(p): Parameters<DigestParams>,
    ) -> Result<String, McpError> {
        let fmt: Option<Format> = match p.format.as_deref() {
            Some(name) => Some(Format::parse_name(name).ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "unknown format {:?}; valid names: cargo, npm, pnpm, yarn, bun, pytest, ci, gha, actions",
                        name
                    ),
                    None,
                )
            })?),
            None => None,
        };

        let result = digest::digest(&p.input, fmt);
        let saved = result.input_bytes.saturating_sub(result.text.len());
        let pct = (result.savings_fraction() * 100.0).round() as u32;

        let mut out = result.text;
        out.push_str(&format!(
            "\n[digest: format={} saved={saved}B ({pct}%)]\n",
            result.format.name()
        ));
        Ok(out)
    }

    #[doc = include_str!("../../../docs/mcp-descriptions/impact.md")]
    #[tool(name = "impact")]
    pub async fn impact(
        &self,
        Parameters(p): Parameters<ImpactParams>,
    ) -> Result<String, McpError> {
        let results = self
            .inner
            .index
            .impact(&p.name, p.depth)
            .map_err(|e| McpError::internal_error(format!("impact failed: {e}"), None))?;

        if results.is_empty() {
            return Ok(format!(
                "no callers found for `{}`.\n\n\
                 Try `pluck.grep` with the symbol name for callers in unindexed files, \
                 or check the spelling (impact is leaf-matched and case-insensitive).\n",
                p.name
            ));
        }

        let prod: Vec<&ImpactHit> = results.iter().filter(|h| !h.is_test).collect();
        let test: Vec<&ImpactHit> = results.iter().filter(|h| h.is_test).collect();

        let mut out = format!(
            "=== impact: {} (depth {}) — {} caller(s) ===\n\n",
            p.name,
            p.depth,
            results.len()
        );

        for h in &prod {
            out.push_str(&format!(
                "[depth {}]  {}:L{}-{}  {} ({:?})\n",
                h.depth,
                h.hit.path,
                h.hit.start_line,
                h.hit.end_line,
                h.hit.symbol,
                h.hit.kind
            ));
            out.push_str(&h.hit.content);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }

        if !test.is_empty() {
            out.push_str(&format!("[test callers — {}]\n", test.len()));
            for h in &test {
                out.push_str(&format!(
                    "[depth {}]  {}:L{}-{}  {} ({:?})\n",
                    h.depth,
                    h.hit.path,
                    h.hit.start_line,
                    h.hit.end_line,
                    h.hit.symbol,
                    h.hit.kind
                ));
                out.push_str(&h.hit.content);
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
            }
        }

        // Session dedup: mark every rendered chunk seen.
        {
            let mut s = self.inner.session.lock().expect("session mutex");
            for h in &results {
                s.mark_seen(h.hit.chunk_id);
            }
        }

        Ok(out)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PluckServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "pluck — token-efficient code reading. Prefer pluck.read/search/grep \
                 over Bash cat/grep/rg whenever the target is inside the indexed repo. \
                 All pluck tools have a --raw or equivalent fallback that matches \
                 cat/grep byte-for-byte if you need exact parity.",
        )
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Format a filesystem error the way `cat` does it, so agents trained
/// on bash transcripts recognize the shape immediately.
///
/// Note: directories are detected separately via `metadata().is_dir()`
/// before this function is ever called, so we don't need to match
/// `ErrorKind::IsADirectory` here — that variant is also gated behind
/// a newer rust-stable than the project's MSRV.
fn read_err(path: &str, e: &std::io::Error) -> String {
    use std::io::ErrorKind;
    match e.kind() {
        ErrorKind::NotFound => format!("pluck.read: {path}: No such file or directory"),
        ErrorKind::PermissionDenied => format!("pluck.read: {path}: Permission denied"),
        _ => format!("pluck.read: {path}: {e}"),
    }
}

fn parse_line_range(s: &str) -> Result<(u32, u32), McpError> {
    let (a, b) = s.split_once('-').ok_or_else(|| {
        McpError::invalid_params(format!("expected 'start-end', got {s:?}"), None)
    })?;
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

fn callees_of(hit: &SearchHit) -> Vec<String> {
    extract_callees(&hit.content, lang_for_path(&hit.path))
}

fn lang_for_path(path: &str) -> Language {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(Language::from_extension)
        .unwrap_or(Language::TypeScript)
}

/// `db.user.findOne` → `findOne` ; `Logger::new` → `new` ; bare names pass through.
fn callee_leaf(name: &str) -> &str {
    if let Some(after) = name.rsplit_once("::") {
        return after.1;
    }
    if let Some(after) = name.rsplit_once('.') {
        return after.1;
    }
    name
}

fn dedup_keep_order(xs: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(xs.len());
    for x in xs {
        if seen.insert(x.clone()) {
            out.push(x);
        }
    }
    out
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
    async fn pluck_expand_includes_root_body_and_hop_one_callees() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");

        let out = server
            .expand(Parameters(ExpandParams {
                name: "chunk_source".into(),
                hop: 1,
            }))
            .await
            .expect("expand");

        // Root body is included.
        assert!(
            out.contains("pub fn chunk_source"),
            "missing root sig: {out}"
        );
        // Hop 1 header is present.
        assert!(out.contains("=== hop 1 ==="), "missing hop header: {out}");
        // Footer prints summary.
        assert!(out.contains("[expanded"), "missing footer: {out}");
    }

    #[tokio::test]
    async fn pluck_expand_unknown_name() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");
        let out = server
            .expand(Parameters(ExpandParams {
                name: "definitely_not_a_real_function_xyzzy".into(),
                hop: 1,
            }))
            .await
            .expect("expand");
        assert!(out.contains("no symbol"), "got: {out}");
    }

    #[test]
    fn callee_leaf_strips_namespace_prefixes() {
        assert_eq!(callee_leaf("foo"), "foo");
        assert_eq!(callee_leaf("db.user.findOne"), "findOne");
        assert_eq!(callee_leaf("Logger::new"), "new");
        assert_eq!(callee_leaf("std::collections::HashMap::new"), "new");
    }

    #[tokio::test]
    async fn read_rejects_directory() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");
        let res = server
            .read(Parameters(ReadParams {
                path: "crates".to_string(),
                raw: true,
                lines: None,
            }))
            .await;
        assert!(res.is_err(), "directory must be rejected");
        let msg = format!("{:?}", res.err().unwrap());
        assert!(
            msg.contains("is a directory"),
            "expected cat-style 'is a directory' diagnostic, got: {msg}"
        );
    }

    #[tokio::test]
    async fn read_rejects_missing_with_cat_style_message() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");
        let res = server
            .read(Parameters(ReadParams {
                path: "does/not/exist.rs".to_string(),
                raw: true,
                lines: None,
            }))
            .await;
        assert!(res.is_err());
        let msg = format!("{:?}", res.err().unwrap());
        assert!(
            msg.contains("No such file or directory"),
            "expected cat-style 'No such file or directory', got: {msg}"
        );
    }

    #[tokio::test]
    async fn read_rejects_binary_file() {
        // Write a binary file in a temp dir, point the server at the
        // tempdir, ask for the file. Must error with the binary
        // diagnostic.
        let nano = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp = std::env::temp_dir().join(format!("pluck-bin-{nano}"));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("bin.dat"), [0xFFu8, 0xFE, 0x00, 0x01, 0x02]).unwrap();
        let server = PluckServer::new(tmp.clone()).expect("server new");
        let res = server
            .read(Parameters(ReadParams {
                path: "bin.dat".to_string(),
                raw: true,
                lines: None,
            }))
            .await;
        let _ = std::fs::remove_dir_all(&tmp);
        let msg = format!("{:?}", res.expect_err("binary must error"));
        assert!(
            msg.contains("not valid UTF-8") || msg.contains("binary"),
            "expected binary diagnostic, got: {msg}"
        );
    }

    #[tokio::test]
    async fn read_rejects_absolute_path_outside_repo() {
        // Repo is the workspace root. Pluck must refuse `/etc/hosts` with
        // a boundary diagnostic, not silently read it.
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");
        let res = server
            .read(Parameters(ReadParams {
                path: "/etc/hosts".to_string(),
                raw: true,
                lines: None,
            }))
            .await;
        let msg = format!(
            "{:?}",
            res.expect_err("outside-repo absolute path must error")
        );
        assert!(
            msg.contains("outside the indexed repo"),
            "expected boundary diagnostic, got: {msg}"
        );
    }

    #[tokio::test]
    async fn read_rejects_parent_traversal_outside_repo() {
        // Relative `..` traversal that escapes the repo root must be
        // caught by component-level normalization.
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");
        let res = server
            .read(Parameters(ReadParams {
                path: "../../../../etc/hosts".to_string(),
                raw: true,
                lines: None,
            }))
            .await;
        let msg = format!("{:?}", res.expect_err(".. escape must error"));
        assert!(
            msg.contains("outside the indexed repo"),
            "expected boundary diagnostic, got: {msg}"
        );
    }

    #[tokio::test]
    async fn build_falls_back_to_bm25_when_encoder_load_fails() {
        // Simulate the production "model fetch failed" path: pass a
        // bogus model id to `build` and confirm the daemon comes up
        // anyway, with a working BM25 index. Search must still return
        // hits — proving the Err arm of the encoder match degrades
        // gracefully instead of propagating.
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::build(
            repo,
            None,
            true,
            Some("pluck-test/this-model-does-not-exist".to_string()),
        )
        .expect("daemon must come up even when encoder load fails");

        let out = server
            .search(Parameters(SearchParams {
                query: "chunk_source".to_string(),
                top_k: 3,
                compact: false,
            }))
            .await
            .expect("BM25 search must work without encoder");
        assert!(!out.is_empty(), "BM25-only search should still return hits");
    }

    #[tokio::test]
    async fn read_allows_internal_parent_traversal() {
        // `subdir/../README.md` normalizes to `README.md`, which stays
        // inside the repo. Must succeed.
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");
        let res = server
            .read(Parameters(ReadParams {
                path: "crates/../README.md".to_string(),
                raw: true,
                lines: None,
            }))
            .await;
        assert!(
            res.is_ok(),
            "internal `..` traversal must be allowed, got: {res:?}"
        );
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

    #[tokio::test]
    async fn digest_tool_compresses_cargo_output() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");

        let cargo_log = "   Compiling serde v1.0.0\n   Compiling tokio v1.30.0\n   Compiling app v0.1.0\n    Finished `dev` profile in 3.21s\n";
        let out = server
            .digest_tool(Parameters(DigestParams {
                input: cargo_log.to_string(),
                format: None,
            }))
            .await
            .expect("digest_tool");

        assert!(out.contains("[cargo] compiled 3"), "missing summary: {out}");
        assert!(out.contains("Finished"), "Finished must survive: {out}");
        assert!(!out.contains("Compiling serde"), "progress must be collapsed: {out}");
        assert!(out.contains("[digest: format=cargo"), "missing metadata footer: {out}");
    }

    #[tokio::test]
    async fn impact_returns_callers_of_pluck_symbol() {
        // Index this repo and ask for callers of `chunk_source` — a
        // function called in multiple places — to verify the reverse
        // index is populated end-to-end.
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");

        let out = server
            .impact(Parameters(ImpactParams {
                name: "chunk_source".into(),
                depth: 1,
            }))
            .await
            .expect("impact");

        // Must find at least one caller — indexer.rs calls chunk_source.
        assert!(
            out.contains("impact: chunk_source"),
            "missing header: {out}"
        );
        assert!(
            !out.contains("no callers found"),
            "chunk_source must have callers: {out}"
        );
    }

    #[tokio::test]
    async fn impact_unknown_symbol_returns_helpful_message() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");

        let out = server
            .impact(Parameters(ImpactParams {
                name: "definitely_not_a_real_fn_xyzzy".into(),
                depth: 1,
            }))
            .await
            .expect("impact");

        assert!(out.contains("no callers found"), "got: {out}");
    }

    #[tokio::test]
    async fn digest_tool_rejects_unknown_format() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let server = PluckServer::new(repo).expect("server new");
        let res = server
            .digest_tool(Parameters(DigestParams {
                input: "anything".to_string(),
                format: Some("not-a-format".to_string()),
            }))
            .await;
        assert!(res.is_err(), "unknown format must error");
        let msg = format!("{:?}", res.err().unwrap());
        assert!(msg.contains("unknown format"), "got: {msg}");
    }
}
