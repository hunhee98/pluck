//! pluck-bench — driver for reproducible agent token benchmarks.

use clap::{Parser, Subcommand};

mod driver;
mod report;
mod runners;
mod scenarios;
mod scoring;

#[derive(Parser, Debug)]
#[command(version, about = "pluck benchmark harness")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run a scenario across one or more workflow runners.
    Run {
        #[arg(long, default_value = "fix-auth-token-expiry")]
        scenario: String,
        /// `all`, or a comma-separated list (`bash,pluck`).
        #[arg(long, default_value = "all")]
        runner: String,
        #[arg(long, default_value_t = 1)]
        repetitions: u32,
        #[arg(long, default_value = "benchmarks/results/")]
        output: String,
    },

    /// List every registered scenario.
    List,

    /// Aggregate results into a report (markdown + JSON). Phase 4.
    Report {
        #[arg(long, default_value = "benchmarks/results/")]
        input: String,
        #[arg(long)]
        markdown: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    let cli = Cli::parse();
    match cli.command {
        Cmd::Run {
            scenario,
            runner,
            repetitions,
            output,
        } => driver::run(&scenario, &runner, repetitions, &output).await,
        Cmd::List => {
            for name in scenarios::all_names() {
                println!("{name}");
            }
            Ok(())
        }
        Cmd::Report { input, markdown } => report::generate(&input, markdown),
    }
}
