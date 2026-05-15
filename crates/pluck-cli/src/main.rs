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

    /// Print version.
    Version,
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
    use super::{cmd_read, parse_line_range};

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
        assert!(result.is_ok(), "UTF-8 text must read clean, got: {result:?}");
    }

}
