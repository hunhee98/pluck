//! pluckd — pluck MCP server.
//!
//! Speaks the Model Context Protocol over stdio. Exposes pluck.read,
//! pluck.grep, pluck.search, pluck.symbol, pluck.peek, pluck.expand.

use clap::Parser;

mod session;
mod descriptions;
mod tools;

#[derive(Parser, Debug)]
#[command(version, about = "pluck MCP server")]
struct Args {
    /// Run as MCP server over stdio.
    #[arg(long, default_value_t = true)]
    stdio: bool,

    /// Repository root to index.
    #[arg(long)]
    repo: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    tracing::info!(version = pluck_core::version(), "pluckd starting");

    // TODO: wire up rmcp server + register tools/*.
    let _ = args;
    Ok(())
}
