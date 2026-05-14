//! pluck — CLI front-end.

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(version, about = "pluck — token-efficient code reading")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Build / refresh the index for a repository.
    Index { path: Option<String> },

    /// Read a code file (smart outline by default; --raw for cat parity).
    Read {
        path: String,
        #[arg(long)]
        raw: bool,
        #[arg(long)]
        lines: Option<String>,
    },

    /// Keyword search (wraps ripgrep; all flags pass through).
    Grep {
        pattern: String,
        #[arg(trailing_var_arg = true)]
        rest: Vec<String>,
    },

    /// Hybrid semantic + keyword search.
    Search { query: String },

    /// Read a symbol by name.
    Symbol { name: String },

    /// Show only the signature + direct callees for a symbol.
    Peek { name: String },

    /// Symbol + callees up to N hops.
    Expand {
        name: String,
        #[arg(long, default_value_t = 1)]
        hop: u8,
    },

    /// Print version.
    Version,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Version => println!("pluck {}", pluck_core::version()),
        _ => eprintln!("not implemented yet — Phase 0 scaffold"),
    }
    Ok(())
}
