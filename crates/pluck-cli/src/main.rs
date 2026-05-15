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
        /// Path to the pluckd binary. Default: resolved via `which pluckd`.
        #[arg(long)]
        pluckd: Option<PathBuf>,
        /// Repo root to register. Default: current directory.
        #[arg(long)]
        repo: Option<PathBuf>,
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
            pluckd,
            repo,
        } => cmd_init(target, pluckd, repo)?,
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

fn cmd_init(target: InitTarget, pluckd: Option<PathBuf>, repo: Option<PathBuf>) -> Result<()> {
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
        InitTarget::Claude => write_claude_mcp_json(&pluckd_path, &repo_path),
        InitTarget::Codex => {
            let config_path = dirs::home_dir()
                .context("could not resolve home directory")?
                .join(".codex")
                .join("config.toml");
            write_codex_config_toml(&config_path, &pluckd_path, &repo_path)
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

#[cfg(test)]
mod tests {
    use super::{cmd_read, parse_line_range, write_claude_mcp_json, write_codex_config_toml};
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
}
