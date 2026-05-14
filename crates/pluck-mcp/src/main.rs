//! `pluckd` — pluck MCP server.
//!
//! Speaks the Model Context Protocol over stdio. Exposes pluck.read,
//! pluck.grep, pluck.search today; pluck.symbol/peek/expand land in the
//! follow-up phases with full call-graph machinery.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use rmcp::{transport::io::stdio, ServiceExt};

use pluck_mcp::server::PluckServer;

#[derive(Parser, Debug)]
#[command(version, about = "pluck MCP server (stdio)")]
struct Args {
    /// Repository root to index on startup. Defaults to the current
    /// working directory.
    #[arg(long)]
    repo: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Logs go to stderr so they never mix with the MCP protocol on stdout.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args = Args::parse();
    let repo = args
        .repo
        .unwrap_or_else(|| std::env::current_dir().expect("current dir"));
    let repo =
        std::fs::canonicalize(&repo).with_context(|| format!("canonicalize repo {repo:?}"))?;

    tracing::info!(version = pluck_core::version(), repo = ?repo, "pluckd starting");

    let server = PluckServer::new_with_watcher(repo).context("build server")?;
    let service = server
        .serve(stdio())
        .await
        .context("rmcp serve over stdio")?;
    service.waiting().await.context("serve loop")?;
    Ok(())
}
