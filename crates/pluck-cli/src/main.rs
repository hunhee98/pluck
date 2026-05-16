//! `pluck` — CLI front-end.
//!
//! Phase 0: index / read / search / grep wired against `pluck-core`.
//! peek / symbol / expand land in Phase 1 with the MCP server.

use std::path::{Path, PathBuf};
use std::process::{Command as Shell, Stdio};
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use pluck_core::chunker::Language;
use pluck_core::digest::{self, Format};
use pluck_core::index::{PluckIndex, SearchHit};
use pluck_core::indexer::{index_repo, IndexStats};
use pluck_core::outliner::{outline_source, render as render_outline};
use pluck_core::store::tantivy_dir;

#[derive(Parser, Debug)]
#[command(version, about = "pluck — token-efficient code reading")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Build / refresh the index for a repository.
    Index {
        /// Repo root. Defaults to current directory.
        path: Option<PathBuf>,
    },

    /// Read a code file. Smart outline by default; `--raw` for cat parity.
    Read {
        path: PathBuf,
        /// Bypass the outliner and dump the file verbatim (matches `cat`).
        #[arg(long)]
        raw: bool,
        /// Return only the given inclusive line range, e.g. "100-200".
        #[arg(long)]
        lines: Option<String>,
    },

    /// Keyword search. Without `--raw`, wraps the index with the same shape
    /// as `rg`; with `--raw`, shells out to `rg` for byte-identical output.
    Grep {
        pattern: String,
        /// Optional paths / extra ripgrep flags.
        #[arg(trailing_var_arg = true)]
        rest: Vec<String>,
        /// Shell out to ripgrep for cat/grep parity.
        #[arg(long)]
        raw: bool,
    },

    /// Hybrid (BM25 today, BM25+semantic in Phase 2) chunk search.
    Search {
        query: String,
        /// Repo root. Defaults to current directory.
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Maximum number of hits.
        #[arg(short = 'k', long, default_value_t = 10)]
        top_k: usize,
        /// Render only score + path:range + matching lines (lossy; useful
        /// for pure discovery). Default keeps the chunk body (lossless).
        #[arg(long)]
        compact: bool,
        /// Drop hits scoring below `cutoff × top_score`. Default 0.12.
        #[arg(long, default_value_t = 0.12)]
        cutoff: f32,
    },

    /// Register the pluck MCP server with an agent's config so the
    /// daemon auto-starts on the next agent launch. Idempotent.
    Init {
        /// Target agent. Default: claude.
        #[arg(long, value_enum, default_value_t = InitTarget::Claude)]
        target: InitTarget,
        /// Adoption strength. `passive` only registers MCP; `strong` also
        /// installs pluck-first rules / permissions; `aggressive` adds a
        /// Claude Code hook that blocks Bash cat/grep/rg retrieval.
        #[arg(long, value_enum, default_value_t = InitMode::Strong)]
        mode: InitMode,
        /// Path to the pluckd binary. Default: resolved via `which pluckd`.
        #[arg(long)]
        pluckd: Option<PathBuf>,
        /// Repo root to register. Default: current directory.
        #[arg(long)]
        repo: Option<PathBuf>,
    },

    /// Compress verbose tool output (cargo, npm, pytest, GitHub Actions).
    ///
    /// Reads from stdin by default. Pass a file path to read from a file
    /// instead. Output is written to stdout; the compressed text is always
    /// <= the original byte count by construction.
    Digest {
        /// Path to the log file. Omit to read from stdin.
        path: Option<PathBuf>,
        /// Force a specific format instead of auto-detecting.
        /// One of: cargo, npm, pnpm, yarn, pytest, ci, gha, actions.
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Print the detected format and byte savings to stderr instead
        /// of compressing (useful for debugging detection).
        #[arg(long)]
        show_format: bool,
    },

    /// Print version.
    Version,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum InitTarget {
    /// Claude Code project `.mcp.json` in the current directory.
    Claude,
    /// Codex global `~/.codex/config.toml` (`[mcp_servers.pluck]`).
    Codex,
    /// Cursor project `.cursor/mcp.json` in the current directory.
    Cursor,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum InitMode {
    /// Only register the MCP server.
    Passive,
    /// Register MCP plus pluck-first agent instructions / allow rules.
    Strong,
    /// Strong mode plus a Claude Code hook that blocks Bash retrieval.
    Aggressive,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Version => {
            println!("pluck {}", pluck_core::version());
        }
        Command::Index { path } => cmd_index(path)?,
        Command::Read { path, raw, lines } => cmd_read(&path, raw, lines.as_deref())?,
        Command::Grep { pattern, rest, raw } => cmd_grep(&pattern, &rest, raw)?,
        Command::Search {
            query,
            repo,
            top_k,
            compact,
            cutoff,
        } => cmd_search(&query, repo, top_k, compact, cutoff)?,
        Command::Init {
            target,
            mode,
            pluckd,
            repo,
        } => cmd_init(target, mode, pluckd, repo)?,
        Command::Digest {
            path,
            format,
            show_format,
        } => cmd_digest(path, format.as_deref(), show_format)?,
    }
    Ok(())
}

fn resolve_repo(path: Option<PathBuf>) -> Result<PathBuf> {
    let p = path.unwrap_or_else(|| PathBuf::from("."));
    std::fs::canonicalize(&p).with_context(|| format!("canonicalize repo {p:?}"))
}

fn cmd_index(path: Option<PathBuf>) -> Result<()> {
    let repo = resolve_repo(path)?;
    let dir = tantivy_dir(&repo)?;
    // Wipe stale index so a fresh `index` is idempotent. Once incremental
    // reindex lands (Phase 2), this becomes additive.
    if dir.exists() {
        std::fs::remove_dir_all(&dir).ok();
    }
    std::fs::create_dir_all(&dir)?;
    let idx = PluckIndex::open_or_create(&dir).context("open or create tantivy dir")?;
    let t0 = Instant::now();
    let stats: IndexStats = index_repo(&idx, &repo)?;
    let elapsed = t0.elapsed();
    eprintln!(
        "indexed {} files ({} chunks) in {:.2}s — skipped: {} lang, {} size, {} unreadable",
        stats.files_indexed,
        stats.chunks_indexed,
        elapsed.as_secs_f64(),
        stats.files_skipped_lang,
        stats.files_skipped_size,
        stats.files_skipped_read,
    );
    eprintln!("index → {}", dir.display());
    Ok(())
}

fn cmd_init(
    target: InitTarget,
    mode: InitMode,
    pluckd: Option<PathBuf>,
    repo: Option<PathBuf>,
) -> Result<()> {
    let pluckd_path = match pluckd {
        Some(p) => p,
        None => resolve_pluckd_binary().context(
            "could not locate `pluckd` on PATH; install it with \
             `cargo install pluck-mcp` or pass --pluckd <path>",
        )?,
    };
    let repo_path = match repo {
        Some(p) => std::fs::canonicalize(&p).with_context(|| format!("canonicalize repo {p:?}"))?,
        None => std::fs::canonicalize(".").context("canonicalize current directory")?,
    };

    match target {
        InitTarget::Claude => {
            write_claude_mcp_json(&pluckd_path, &repo_path)?;
            if mode != InitMode::Passive {
                write_claude_adoption_layer(&repo_path, mode)?;
            }
            Ok(())
        }
        InitTarget::Codex => {
            let config_path = dirs::home_dir()
                .context("could not resolve home directory")?
                .join(".codex")
                .join("config.toml");
            write_codex_config_toml(&config_path, &pluckd_path, &repo_path)?;
            if mode != InitMode::Passive {
                write_agents_md_policy(&repo_path)?;
            }
            Ok(())
        }
        InitTarget::Cursor => {
            write_cursor_mcp_json(&pluckd_path, &repo_path)?;
            if mode != InitMode::Passive {
                write_cursor_adoption_layer(&repo_path)?;
                write_agents_md_policy(&repo_path)?;
            }
            Ok(())
        }
    }
}

fn resolve_pluckd_binary() -> Result<PathBuf> {
    let out = Shell::new("which")
        .arg("pluckd")
        .stderr(Stdio::null())
        .output()
        .context("invoke `which pluckd`")?;
    if !out.status.success() {
        anyhow::bail!("pluckd not found on PATH");
    }
    let path = String::from_utf8(out.stdout)
        .context("non-UTF-8 path from `which pluckd`")?
        .trim()
        .to_string();
    if path.is_empty() {
        anyhow::bail!("`which pluckd` returned empty output");
    }
    Ok(PathBuf::from(path))
}

const PLUCK_FIRST_POLICY: &str = r#"
Use pluck before Bash for repository code retrieval.

- Use `mcp__pluck__read` instead of `cat`, built-in Read, `head`, `tail`, or `sed -n` for files inside the indexed repo. Outline mode is the default; use `raw: true` only when byte-exact output is required.
- Use `mcp__pluck__search` for conceptual lookup when you do not know the exact identifier or path.
- Use `mcp__pluck__grep` instead of `grep` / `rg` when you need exact strings, regexes, TODOs, or all textual matches inside the repo.
- Use `mcp__pluck__peek` when you need a symbol's interface and direct callees without paying for the body.
- Use `mcp__pluck__symbol` when you know the symbol and need the body.
- Use `mcp__pluck__expand` for local call chains, `mcp__pluck__impact` before refactors, and `mcp__pluck__deps` for import relationships.
- Use `mcp__pluck__digest` for long cargo, npm, pytest, or GitHub Actions logs before pasting raw output into context.

Fallback to Bash only for binary files, paths outside the indexed repo, byte-exact shell pipelines, unsupported formats, or when the pluck daemon is unreachable.
"#;

const CURSOR_PLUCK_FIRST_RULE: &str = r#"
---
description: Prefer pluck MCP for repository code retrieval
alwaysApply: true
---

Use pluck MCP tools before built-in code search/read tools or terminal `cat`,
`grep`, and `rg` for files inside this repository.

- `mcp__pluck__read`: file reads; outline-first by default.
- `mcp__pluck__search`: conceptual code lookup.
- `mcp__pluck__grep`: exact strings, regexes, TODOs, and all textual matches.
- `mcp__pluck__peek`: signature plus direct callees.
- `mcp__pluck__symbol`: one symbol body.
- `mcp__pluck__expand`: root body plus bounded callee chain.
- `mcp__pluck__impact`: reverse call graph before refactors.
- `mcp__pluck__deps`: file import graph.
- `mcp__pluck__digest`: compress long build, test, install, and CI logs.

Use terminal fallback only for binary files, paths outside the indexed repo,
byte-exact shell pipelines, unsupported formats, or when pluck is unavailable.
"#;

const PLUCK_FIRST_BASH_HOOK: &str = r#"
#!/usr/bin/env python3
import json
import re
import sys

try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)

command = ((data.get("tool_input") or {}).get("command") or "").strip()
if not command:
    sys.exit(0)

retrieval = re.compile(
    r"(^|[;&|\n]\s*)(cat|grep|rg|head|tail)\b|(^|[;&|\n]\s*)sed\s+-n\b"
)

if retrieval.search(command):
    reason = (
        "Use pluck first for repo code retrieval: mcp__pluck__read for "
        "cat/head/tail/sed -n, mcp__pluck__grep for exact search, and "
        "mcp__pluck__search for conceptual lookup. Fall back to Bash only "
        "for binary files, paths outside the indexed repo, byte-exact shell "
        "pipelines, unsupported formats, or when pluck is unavailable."
    )
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    }))
"#;

/// Write or update `./.mcp.json` so that Claude Code launches `pluckd`
/// on the next agent start. Preserves any other `mcpServers` entries
/// the user already has.
fn write_claude_mcp_json(pluckd_path: &Path, repo_path: &Path) -> Result<()> {
    let target = repo_path.join(".mcp.json");
    let mut doc: serde_json::Value = if target.exists() {
        let raw = std::fs::read_to_string(&target)
            .with_context(|| format!("read {}", target.display()))?;
        serde_json::from_str(&raw).with_context(|| {
            format!(
                "{} exists but is not valid JSON; fix or remove it before re-running `pluck init`",
                target.display()
            )
        })?
    } else {
        serde_json::json!({})
    };

    if !doc.is_object() {
        anyhow::bail!(
            "{} top-level is not a JSON object; refusing to overwrite",
            target.display()
        );
    }

    let servers = doc
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    if !servers.is_object() {
        anyhow::bail!(
            "`mcpServers` in {} is not an object; refusing to overwrite",
            target.display()
        );
    }

    let prev = servers.as_object().unwrap().get("pluck").cloned();
    let entry = serde_json::json!({
        "command": pluckd_path.display().to_string(),
        "args": ["--repo", repo_path.display().to_string()],
    });
    let already_correct = prev.as_ref() == Some(&entry);

    servers
        .as_object_mut()
        .unwrap()
        .insert("pluck".to_string(), entry);

    let body = serde_json::to_string_pretty(&doc).context("serialize .mcp.json")?;
    std::fs::write(&target, body + "\n").with_context(|| format!("write {}", target.display()))?;

    if already_correct {
        println!(
            "pluck init: {} already registered the same pluck entry (no change)",
            target.display()
        );
    } else if prev.is_some() {
        println!("pluck init: updated `pluck` entry in {}", target.display());
    } else {
        println!(
            "pluck init: registered `pluck` MCP server in {}",
            target.display()
        );
    }
    println!("  command: {}", pluckd_path.display());
    println!("  repo:    {}", repo_path.display());
    Ok(())
}

fn write_claude_adoption_layer(repo_path: &Path, mode: InitMode) -> Result<()> {
    let claude_dir = repo_path.join(".claude");
    std::fs::create_dir_all(&claude_dir)
        .with_context(|| format!("create {}", claude_dir.display()))?;

    write_claude_settings_json(repo_path, mode)?;

    let rules_dir = claude_dir.join("rules");
    std::fs::create_dir_all(&rules_dir)
        .with_context(|| format!("create {}", rules_dir.display()))?;
    write_text_if_changed(
        &rules_dir.join("pluck-first.md"),
        PLUCK_FIRST_POLICY.trim_start(),
    )?;

    if mode == InitMode::Aggressive {
        let hooks_dir = claude_dir.join("hooks");
        std::fs::create_dir_all(&hooks_dir)
            .with_context(|| format!("create {}", hooks_dir.display()))?;
        let hook_path = hooks_dir.join("pluck-first-bash.py");
        write_text_if_changed(&hook_path, PLUCK_FIRST_BASH_HOOK.trim_start())?;
        make_executable(&hook_path)?;
    }

    println!(
        "pluck init: installed Claude Code pluck-first adoption layer in {}",
        claude_dir.display()
    );
    if mode == InitMode::Aggressive {
        println!("  mode:    aggressive (Bash cat/grep/rg retrieval is blocked by hook)");
    } else {
        println!("  mode:    strong (rules + mcp__pluck__* permission allow)");
    }
    Ok(())
}

fn write_claude_settings_json(repo_path: &Path, mode: InitMode) -> Result<()> {
    let target = repo_path.join(".claude").join("settings.json");
    let mut doc = read_json_object_or_empty(&target)?;

    let permissions = doc
        .as_object_mut()
        .unwrap()
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}));
    if !permissions.is_object() {
        anyhow::bail!(
            "`permissions` in {} is not an object; refusing to overwrite",
            target.display()
        );
    }
    json_array_insert_unique(permissions, "allow", "mcp__pluck__*")?;

    if mode == InitMode::Aggressive {
        let hooks = doc
            .as_object_mut()
            .unwrap()
            .entry("hooks")
            .or_insert_with(|| serde_json::json!({}));
        if !hooks.is_object() {
            anyhow::bail!(
                "`hooks` in {} is not an object; refusing to overwrite",
                target.display()
            );
        }
        upsert_claude_pretool_hook(hooks)?;
    }

    write_json_pretty(&target, &doc)
}

fn upsert_claude_pretool_hook(hooks: &mut serde_json::Value) -> Result<()> {
    let groups = hooks
        .as_object_mut()
        .unwrap()
        .entry("PreToolUse")
        .or_insert_with(|| serde_json::json!([]));
    if !groups.is_array() {
        anyhow::bail!("`hooks.PreToolUse` is not an array; refusing to overwrite");
    }

    let hook = serde_json::json!({
        "type": "command",
        "command": "${CLAUDE_PROJECT_DIR}/.claude/hooks/pluck-first-bash.py"
    });
    let group = serde_json::json!({
        "matcher": "Bash",
        "hooks": [hook]
    });

    let groups = groups.as_array_mut().unwrap();
    let already_present = groups.iter().any(|g| {
        g.get("matcher") == Some(&serde_json::Value::String("Bash".into()))
            && g.get("hooks")
                .and_then(|h| h.as_array())
                .map(|hs| {
                    hs.iter().any(|h| {
                        h.get("command").and_then(|c| c.as_str())
                            == Some("${CLAUDE_PROJECT_DIR}/.claude/hooks/pluck-first-bash.py")
                    })
                })
                .unwrap_or(false)
    });
    if !already_present {
        groups.push(group);
    }
    Ok(())
}

/// Write or update `~/.codex/config.toml` so that Codex launches
/// `pluckd` on the next session. Codex's config is global, so a
/// re-run from a different repo replaces the previous entry's
/// `--repo` arg in place. Format and comments outside the
/// `mcp_servers.pluck` table are preserved via `toml_edit`.
fn write_codex_config_toml(config_path: &Path, pluckd_path: &Path, repo_path: &Path) -> Result<()> {
    if !config_path.exists() {
        anyhow::bail!(
            "Codex config not found at {}. Install Codex first, then re-run `pluck init --target codex`.",
            config_path.display()
        );
    }

    let raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let mut doc: toml_edit::DocumentMut = raw.parse().with_context(|| {
        format!(
            "{} is not valid TOML; fix or remove it before re-running `pluck init`",
            config_path.display()
        )
    })?;

    let prev = doc
        .get("mcp_servers")
        .and_then(|s| s.get("pluck"))
        .map(|t| t.to_string());

    let servers = doc
        .entry("mcp_servers")
        .or_insert_with(toml_edit::table)
        .as_table_mut()
        .ok_or_else(|| {
            anyhow::anyhow!("`mcp_servers` in {} is not a table", config_path.display())
        })?;
    // Keep nested-table style ([mcp_servers.pluck]) rather than inline.
    servers.set_implicit(true);

    let mut entry = toml_edit::Table::new();
    entry.insert(
        "command",
        toml_edit::Item::Value(pluckd_path.display().to_string().into()),
    );
    let mut args = toml_edit::Array::new();
    args.push("--repo");
    args.push(repo_path.display().to_string());
    entry.insert("args", toml_edit::Item::Value(args.into()));

    servers.insert("pluck", toml_edit::Item::Table(entry));

    let new_pluck_str = doc
        .get("mcp_servers")
        .and_then(|s| s.get("pluck"))
        .map(|t| t.to_string());
    let already_correct = prev.is_some() && prev == new_pluck_str;

    std::fs::write(config_path, doc.to_string())
        .with_context(|| format!("write {}", config_path.display()))?;

    if already_correct {
        println!(
            "pluck init: {} already registered the same pluck entry (no change)",
            config_path.display()
        );
    } else if prev.is_some() {
        println!(
            "pluck init: updated `pluck` entry in {}",
            config_path.display()
        );
    } else {
        println!(
            "pluck init: registered `pluck` MCP server in {}",
            config_path.display()
        );
    }
    println!("  command: {}", pluckd_path.display());
    println!("  repo:    {}", repo_path.display());
    println!(
        "  note:    Codex's `mcp_servers` table is global; re-run from a different repo to switch."
    );
    Ok(())
}

fn write_cursor_mcp_json(pluckd_path: &Path, repo_path: &Path) -> Result<()> {
    let cursor_dir = repo_path.join(".cursor");
    std::fs::create_dir_all(&cursor_dir)
        .with_context(|| format!("create {}", cursor_dir.display()))?;
    let target = cursor_dir.join("mcp.json");
    let mut doc = read_json_object_or_empty(&target)?;

    let servers = doc
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    if !servers.is_object() {
        anyhow::bail!(
            "`mcpServers` in {} is not an object; refusing to overwrite",
            target.display()
        );
    }

    let prev = servers.as_object().unwrap().get("pluck").cloned();
    let entry = serde_json::json!({
        "command": pluckd_path.display().to_string(),
        "args": ["--repo", repo_path.display().to_string()],
    });
    let already_correct = prev.as_ref() == Some(&entry);
    servers
        .as_object_mut()
        .unwrap()
        .insert("pluck".to_string(), entry);

    write_json_pretty(&target, &doc)?;

    if already_correct {
        println!(
            "pluck init: {} already registered the same pluck entry (no change)",
            target.display()
        );
    } else if prev.is_some() {
        println!("pluck init: updated `pluck` entry in {}", target.display());
    } else {
        println!(
            "pluck init: registered `pluck` MCP server in {}",
            target.display()
        );
    }
    println!("  command: {}", pluckd_path.display());
    println!("  repo:    {}", repo_path.display());
    Ok(())
}

fn write_cursor_adoption_layer(repo_path: &Path) -> Result<()> {
    let rules_dir = repo_path.join(".cursor").join("rules");
    std::fs::create_dir_all(&rules_dir)
        .with_context(|| format!("create {}", rules_dir.display()))?;
    write_text_if_changed(
        &rules_dir.join("pluck-first.mdc"),
        CURSOR_PLUCK_FIRST_RULE.trim_start(),
    )?;
    println!(
        "pluck init: installed Cursor pluck-first rule in {}",
        rules_dir.display()
    );
    Ok(())
}

fn write_agents_md_policy(repo_path: &Path) -> Result<()> {
    let target = repo_path.join("AGENTS.md");
    upsert_markdown_block(&target, "Pluck-First Retrieval", PLUCK_FIRST_POLICY.trim())
        .with_context(|| format!("write {}", target.display()))?;
    println!(
        "pluck init: installed pluck-first retrieval policy in {}",
        target.display()
    );
    Ok(())
}

fn upsert_markdown_block(path: &Path, title: &str, body: &str) -> Result<()> {
    const START: &str = "<!-- pluck:first:start -->";
    const END: &str = "<!-- pluck:first:end -->";
    let block = format!("{START}\n## {title}\n\n{body}\n{END}\n");
    let existing = if path.exists() {
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?
    } else {
        String::new()
    };

    let next = match (existing.find(START), existing.find(END)) {
        (Some(s), Some(e)) if s < e => {
            let end = e + END.len();
            format!("{}{}{}", &existing[..s], block, &existing[end..])
        }
        _ if existing.trim().is_empty() => format!("# Agent Instructions\n\n{block}"),
        _ => {
            let sep = if existing.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            format!("{existing}{sep}{block}")
        }
    };

    write_text_if_changed(path, &next)
}

fn read_json_object_or_empty(path: &Path) -> Result<serde_json::Value> {
    let doc = if path.exists() {
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| {
            format!(
                "{} exists but is not valid JSON; fix or remove it before re-running `pluck init`",
                path.display()
            )
        })?
    } else {
        serde_json::json!({})
    };
    if !doc.is_object() {
        anyhow::bail!(
            "{} top-level is not a JSON object; refusing to overwrite",
            path.display()
        );
    }
    Ok(doc)
}

fn json_array_insert_unique(parent: &mut serde_json::Value, key: &str, value: &str) -> Result<()> {
    let arr = parent
        .as_object_mut()
        .unwrap()
        .entry(key)
        .or_insert_with(|| serde_json::json!([]));
    if !arr.is_array() {
        anyhow::bail!("`{key}` is not an array; refusing to overwrite");
    }
    let arr = arr.as_array_mut().unwrap();
    if !arr.iter().any(|v| v.as_str() == Some(value)) {
        arr.push(serde_json::Value::String(value.to_string()));
    }
    Ok(())
}

fn write_json_pretty(path: &Path, doc: &serde_json::Value) -> Result<()> {
    let body = serde_json::to_string_pretty(doc).context("serialize JSON")?;
    write_text_if_changed(path, &(body + "\n"))
}

fn write_text_if_changed(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    if path.exists()
        && std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?
            == body
    {
        return Ok(());
    }
    std::fs::write(path, body).with_context(|| format!("write {}", path.display()))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).with_context(|| format!("chmod +x {}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn cmd_read(path: &Path, raw: bool, lines: Option<&str>) -> Result<()> {
    // Binary files trip a `read_to_string` UTF-8 error with a noisy
    // anyhow trace. Match the MCP `read` shape: read bytes once, then
    // emit a one-line `cat`-style diagnostic and exit with status 1
    // so agents can route to bash for byte-level work.
    let bytes = std::fs::read(path).with_context(|| format!("read {path:?}"))?;
    let src = match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            anyhow::bail!(
                "pluck: {}: not valid UTF-8 (likely binary). Use bash `cat` for byte-level reads.",
                path.display()
            );
        }
    };

    if raw {
        print!("{src}");
        return Ok(());
    }

    if let Some(range) = lines {
        let (s, e) = parse_line_range(range)?;
        for (i, line) in src.lines().enumerate() {
            let n = (i + 1) as u32;
            if n >= s && n <= e {
                println!("{line}");
            }
        }
        return Ok(());
    }

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = Language::from_extension(ext);
    let display = path.to_string_lossy();
    let outline = outline_source(&src, lang, &display);
    print!("{}", render_outline(&outline));
    Ok(())
}

fn parse_line_range(s: &str) -> Result<(u32, u32)> {
    let (a, b) = s
        .split_once('-')
        .with_context(|| format!("expected 'start-end', got {s:?}"))?;
    let a: u32 = a.trim().parse().context("parse start line")?;
    let b: u32 = b.trim().parse().context("parse end line")?;
    if a == 0 || b < a {
        anyhow::bail!("invalid line range {s}");
    }
    Ok((a, b))
}

fn cmd_grep(pattern: &str, rest: &[String], _raw: bool) -> Result<()> {
    // Today both `grep` and `grep --raw` shell out to ripgrep — the result
    // is byte-equivalent. Once the BM25 index can be invoked here without a
    // repo arg, the non-raw form will switch to the index. The `--raw`
    // contract stays the same forever.
    let mut cmd = Shell::new("rg");
    cmd.arg(pattern).args(rest);
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = cmd
        .status()
        .context("failed to invoke `rg`; is ripgrep installed?")?;
    if !status.success() {
        // rg exits 1 when no matches found — propagate so shells see it,
        // but don't treat as an internal error.
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

fn cmd_search(
    query: &str,
    repo: Option<PathBuf>,
    top_k: usize,
    compact: bool,
    cutoff: f32,
) -> Result<()> {
    let repo = resolve_repo(repo)?;
    let dir = tantivy_dir(&repo)?;
    if !dir.exists() {
        anyhow::bail!(
            "no index at {} — run `pluck index {}` first",
            dir.display(),
            repo.display()
        );
    }
    let idx = PluckIndex::open_or_create(&dir)?;
    let hits = idx.search_with_cutoff(query, top_k, cutoff)?;
    if hits.is_empty() {
        eprintln!("no hits.");
        return Ok(());
    }
    if compact {
        print_compact(&hits, query);
    } else {
        print_full(&hits);
    }
    Ok(())
}

fn print_full(hits: &[SearchHit]) {
    for h in hits {
        println!(
            "{:.4}  {}:L{}-{}  {} ({:?})",
            h.score, h.path, h.start_line, h.end_line, h.symbol, h.kind
        );
        for line in h.content.lines() {
            println!("  {line}");
        }
        println!();
    }
}

fn print_compact(hits: &[SearchHit], query: &str) {
    let words: Vec<String> = query
        .split_whitespace()
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_lowercase())
        .collect();
    for h in hits {
        println!("{:.4}\t{}:{}-{}", h.score, h.path, h.start_line, h.end_line);
        for (i, line) in h.content.lines().enumerate() {
            let lower = line.to_lowercase();
            if words.iter().any(|w| lower.contains(w)) {
                let ln = h.start_line as usize + i;
                println!("  L{ln}: {}", line.trim());
            }
        }
    }
}

fn cmd_digest(path: Option<PathBuf>, format_name: Option<&str>, show_format: bool) -> Result<()> {
    use std::io::Read as _;

    let input = match path {
        Some(p) => std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?,
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("read stdin")?;
            buf
        }
    };

    let fmt: Option<Format> = match format_name {
        Some(name) => Some(Format::parse_name(name).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown format {name:?}; valid names: cargo, npm, pnpm, yarn, bun, pytest, ci, gha, actions"
            )
        })?),
        None => None,
    };

    let result = digest::digest(&input, fmt);

    if show_format {
        let saved = result.input_bytes.saturating_sub(result.text.len());
        let pct = (result.savings_fraction() * 100.0).round() as u32;
        eprintln!(
            "format: {}  input: {} bytes  output: {} bytes  saved: {} bytes ({pct}%)",
            result.format.name(),
            result.input_bytes,
            result.text.len(),
            saved,
        );
    }

    print!("{}", result.text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        cmd_digest, cmd_read, parse_line_range, write_agents_md_policy,
        write_claude_adoption_layer, write_claude_mcp_json, write_codex_config_toml,
        write_cursor_adoption_layer, write_cursor_mcp_json, InitMode,
    };
    use std::path::PathBuf;

    #[test]
    fn line_range_parses() {
        assert_eq!(parse_line_range("10-20").unwrap(), (10, 20));
    }
    #[test]
    fn line_range_rejects_inverted() {
        assert!(parse_line_range("20-10").is_err());
    }
    #[test]
    fn line_range_rejects_zero() {
        assert!(parse_line_range("0-5").is_err());
    }
    #[test]
    fn line_range_rejects_missing_dash() {
        assert!(parse_line_range("10").is_err());
    }

    #[test]
    fn cmd_read_emits_binary_diagnostic_for_non_utf8() {
        let nano = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp = std::env::temp_dir().join(format!("pluck-cli-bin-{nano}"));
        std::fs::create_dir_all(&tmp).unwrap();
        let bin = tmp.join("bin.dat");
        std::fs::write(&bin, [0xFFu8, 0xFE, 0x00, 0x01, 0x02]).unwrap();

        let err = cmd_read(&bin, true, None).expect_err("binary must error");
        let msg = format!("{err}");
        let _ = std::fs::remove_dir_all(&tmp);

        assert!(
            msg.contains("not valid UTF-8") || msg.contains("binary"),
            "expected cat-style binary diagnostic, got: {msg}"
        );
    }

    fn tmp_dir(label: &str) -> PathBuf {
        let nano = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("pluck-init-{label}-{nano}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn write_claude_mcp_json_creates_fresh_file() {
        let tmp = tmp_dir("fresh");
        let pluckd = PathBuf::from("/opt/pluck/bin/pluckd");
        write_claude_mcp_json(&pluckd, &tmp).unwrap();

        let body = std::fs::read_to_string(tmp.join(".mcp.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            doc["mcpServers"]["pluck"]["command"],
            "/opt/pluck/bin/pluckd"
        );
        let args = doc["mcpServers"]["pluck"]["args"].as_array().unwrap();
        assert_eq!(args[0], "--repo");
        assert_eq!(args[1], tmp.display().to_string());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_claude_mcp_json_preserves_other_servers() {
        let tmp = tmp_dir("preserve");
        let pre = serde_json::json!({
            "mcpServers": {
                "other": { "command": "/usr/local/bin/other", "args": ["serve"] }
            }
        });
        std::fs::write(
            tmp.join(".mcp.json"),
            serde_json::to_string_pretty(&pre).unwrap(),
        )
        .unwrap();

        let pluckd = PathBuf::from("/opt/pluck/bin/pluckd");
        write_claude_mcp_json(&pluckd, &tmp).unwrap();

        let body = std::fs::read_to_string(tmp.join(".mcp.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            doc["mcpServers"]["other"]["command"], "/usr/local/bin/other",
            "existing `other` server must survive"
        );
        assert_eq!(
            doc["mcpServers"]["pluck"]["command"],
            "/opt/pluck/bin/pluckd"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_claude_mcp_json_is_idempotent() {
        let tmp = tmp_dir("idempotent");
        let pluckd = PathBuf::from("/opt/pluck/bin/pluckd");
        write_claude_mcp_json(&pluckd, &tmp).unwrap();
        let first = std::fs::read_to_string(tmp.join(".mcp.json")).unwrap();
        write_claude_mcp_json(&pluckd, &tmp).unwrap();
        let second = std::fs::read_to_string(tmp.join(".mcp.json")).unwrap();
        assert_eq!(first, second, "re-running init must produce identical file");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_claude_mcp_json_updates_stale_pluckd_path() {
        let tmp = tmp_dir("update");
        write_claude_mcp_json(&PathBuf::from("/old/pluckd"), &tmp).unwrap();
        write_claude_mcp_json(&PathBuf::from("/new/pluckd"), &tmp).unwrap();

        let body = std::fs::read_to_string(tmp.join(".mcp.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(doc["mcpServers"]["pluck"]["command"], "/new/pluckd");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_claude_mcp_json_rejects_corrupt_existing_file() {
        let tmp = tmp_dir("corrupt");
        std::fs::write(tmp.join(".mcp.json"), "{ this is not valid json").unwrap();
        let err = write_claude_mcp_json(&PathBuf::from("/opt/pluckd"), &tmp)
            .expect_err("corrupt existing .mcp.json must error, not silently overwrite");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("not valid JSON"),
            "expected helpful diagnostic, got: {msg}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_claude_adoption_layer_adds_permissions_rules_and_hook() {
        let tmp = tmp_dir("claude-adoption");
        std::fs::create_dir_all(tmp.join(".claude")).unwrap();
        std::fs::write(
            tmp.join(".claude").join("settings.json"),
            r#"{"permissions":{"allow":["Bash(cargo test *)"]}}"#,
        )
        .unwrap();

        write_claude_adoption_layer(&tmp, InitMode::Aggressive).unwrap();
        write_claude_adoption_layer(&tmp, InitMode::Aggressive).unwrap();

        let settings = std::fs::read_to_string(tmp.join(".claude").join("settings.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&settings).unwrap();
        let allow = doc["permissions"]["allow"].as_array().unwrap();
        assert!(allow.iter().any(|v| v == "Bash(cargo test *)"));
        assert!(allow.iter().any(|v| v == "mcp__pluck__*"));
        let hooks = doc["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(
            hooks.iter().filter(|h| h["matcher"] == "Bash").count(),
            1,
            "same hook must not duplicate on re-run: {settings}"
        );

        let rule =
            std::fs::read_to_string(tmp.join(".claude").join("rules").join("pluck-first.md"))
                .unwrap();
        assert!(rule.contains("Use pluck before Bash"));
        let hook = std::fs::read_to_string(
            tmp.join(".claude")
                .join("hooks")
                .join("pluck-first-bash.py"),
        )
        .unwrap();
        assert!(hook.contains("mcp__pluck__read"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_agents_md_policy_upserts_marked_block() {
        let tmp = tmp_dir("agents-policy");
        let agents = tmp.join("AGENTS.md");
        std::fs::write(&agents, "# Existing\n\nKeep this.\n").unwrap();

        write_agents_md_policy(&tmp).unwrap();
        write_agents_md_policy(&tmp).unwrap();

        let body = std::fs::read_to_string(&agents).unwrap();
        assert!(body.contains("Keep this."));
        assert!(body.contains("<!-- pluck:first:start -->"));
        assert_eq!(body.matches("Pluck-First Retrieval").count(), 1);
        assert!(body.contains("mcp__pluck__search"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_cursor_mcp_json_and_rule_preserve_existing_config() {
        let tmp = tmp_dir("cursor");
        std::fs::create_dir_all(tmp.join(".cursor")).unwrap();
        std::fs::write(
            tmp.join(".cursor").join("mcp.json"),
            r#"{"mcpServers":{"other":{"command":"node","args":["server.js"]}}}"#,
        )
        .unwrap();

        write_cursor_mcp_json(
            &PathBuf::from("/opt/pluck/bin/pluckd"),
            &PathBuf::from(&tmp),
        )
        .unwrap();
        write_cursor_adoption_layer(&tmp).unwrap();

        let body = std::fs::read_to_string(tmp.join(".cursor").join("mcp.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(doc["mcpServers"]["other"]["command"], "node");
        assert_eq!(
            doc["mcpServers"]["pluck"]["command"],
            "/opt/pluck/bin/pluckd"
        );
        let rule =
            std::fs::read_to_string(tmp.join(".cursor").join("rules").join("pluck-first.mdc"))
                .unwrap();
        assert!(rule.contains("alwaysApply: true"));
        assert!(rule.contains("mcp__pluck__grep"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_codex_config_toml_creates_pluck_entry_preserving_others() {
        let tmp = tmp_dir("codex-preserve");
        let cfg = tmp.join("config.toml");
        // Mimic a real Codex config with an unrelated server + comments.
        std::fs::write(
            &cfg,
            r#"# user config
model = "gpt-5"

[mcp_servers.pencil]
command = "/Applications/Pencil.app/run"
args = ["--app", "desktop"]

[projects."/Users/me/x"]
trust_level = "trusted"
"#,
        )
        .unwrap();

        write_codex_config_toml(
            &cfg,
            &PathBuf::from("/opt/pluck/bin/pluckd"),
            &PathBuf::from("/Users/me/x"),
        )
        .unwrap();

        let body = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            body.contains("[mcp_servers.pencil]"),
            "pre-existing pencil entry must survive: {body}"
        );
        assert!(
            body.contains("[mcp_servers.pluck]"),
            "new pluck entry must be present: {body}"
        );
        assert!(body.contains("/opt/pluck/bin/pluckd"));
        assert!(body.contains("/Users/me/x"));
        assert!(
            body.contains("model = \"gpt-5\""),
            "top-level keys must survive"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_codex_config_toml_rejects_missing_file() {
        let tmp = tmp_dir("codex-missing");
        let cfg = tmp.join("does-not-exist.toml");
        let err =
            write_codex_config_toml(&cfg, &PathBuf::from("/opt/pluckd"), &PathBuf::from("/repo"))
                .expect_err("missing Codex config must error, not silently create");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("Codex config not found"),
            "expected install-Codex-first diagnostic, got: {msg}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_codex_config_toml_is_idempotent_and_updates_path() {
        let tmp = tmp_dir("codex-update");
        let cfg = tmp.join("config.toml");
        std::fs::write(&cfg, "# starter\n").unwrap();

        write_codex_config_toml(
            &cfg,
            &PathBuf::from("/old/pluckd"),
            &PathBuf::from("/repo/old"),
        )
        .unwrap();
        let first = std::fs::read_to_string(&cfg).unwrap();
        // Idempotent same-input re-run.
        write_codex_config_toml(
            &cfg,
            &PathBuf::from("/old/pluckd"),
            &PathBuf::from("/repo/old"),
        )
        .unwrap();
        let second = std::fs::read_to_string(&cfg).unwrap();
        assert_eq!(first, second, "same-input re-run must be byte-identical");

        // Different path → entry updated, other tables untouched.
        write_codex_config_toml(
            &cfg,
            &PathBuf::from("/new/pluckd"),
            &PathBuf::from("/repo/new"),
        )
        .unwrap();
        let third = std::fs::read_to_string(&cfg).unwrap();
        assert!(third.contains("/new/pluckd"));
        assert!(third.contains("/repo/new"));
        assert!(!third.contains("/old/pluckd"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_codex_config_toml_rejects_corrupt_toml() {
        let tmp = tmp_dir("codex-corrupt");
        let cfg = tmp.join("config.toml");
        std::fs::write(&cfg, "not = valid = toml === bad").unwrap();
        let err =
            write_codex_config_toml(&cfg, &PathBuf::from("/opt/pluckd"), &PathBuf::from("/repo"))
                .expect_err("corrupt TOML must error, not silently overwrite");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("not valid TOML"),
            "expected helpful diagnostic, got: {msg}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cmd_read_succeeds_on_utf8_text() {
        let nano = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp = std::env::temp_dir().join(format!("pluck-cli-txt-{nano}"));
        std::fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("hello.txt");
        std::fs::write(&f, "hello\nworld\n").unwrap();
        let result = cmd_read(&f, true, None);
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(
            result.is_ok(),
            "UTF-8 text must read clean, got: {result:?}"
        );
    }

    #[test]
    fn digest_reads_file_and_compresses_cargo_output() {
        let nano = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp = std::env::temp_dir().join(format!("pluck-digest-{nano}"));
        std::fs::create_dir_all(&tmp).unwrap();
        let log = tmp.join("cargo.log");
        let content = "   Compiling serde v1.0.0\n   Compiling tokio v1.30.0\n    Finished `dev` profile in 3.21s\n";
        std::fs::write(&log, content).unwrap();
        let result = cmd_digest(Some(log), None, false);
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(result.is_ok(), "digest must succeed: {result:?}");
    }

    #[test]
    fn digest_rejects_unknown_format_name() {
        let result = cmd_digest(None, Some("not-a-format"), false);
        let msg = format!("{:?}", result.expect_err("unknown format must error"));
        assert!(msg.contains("unknown format"), "got: {msg}");
    }
}
