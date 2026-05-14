//! pluck-bench — driver for reproducible agent token benchmarks.

use clap::{Parser, Subcommand};

mod driver;
mod runners;
mod scoring;
mod report;

#[derive(Parser, Debug)]
#[command(version, about = "pluck benchmark harness")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run one scenario with one runner.
    Run {
        #[arg(long)]
        scenario: String,
        #[arg(long)]
        runner: String,
        #[arg(long, default_value_t = 5)]
        repetitions: u32,
        #[arg(long, default_value = "benchmarks/results/")]
        output: String,
    },

    /// Aggregate results into a report (markdown + JSON).
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
        .init();
    let cli = Cli::parse();
    match cli.command {
        Cmd::Run { scenario, runner, repetitions, output } => {
            driver::run(&scenario, &runner, repetitions, &output).await
        }
        Cmd::Report { input, markdown } => {
            report::generate(&input, markdown)
        }
    }
}
