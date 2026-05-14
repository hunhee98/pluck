//! Orchestrate scenarios end-to-end: load fixture, run every registered
//! workflow, score each, emit the markdown report (and a JSON copy for
//! later aggregation).

use std::io::{self, Write};
use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::report::{print_markdown, save_json, ScenarioReport};
use crate::runners::{BashRunner, PluckRunner, Runner};
use crate::scenarios::{load as load_scenario, Scenario};
use crate::scoring::{bpe, score};

pub async fn run(
    scenario_name: &str,
    runner_filter: &str,
    _repetitions: u32,
    output_dir: &str,
) -> Result<()> {
    let scenario = load_scenario(scenario_name)
        .ok_or_else(|| anyhow!("unknown scenario: {scenario_name}"))?;

    let runners = select_runners(runner_filter)?;
    let bpe = bpe()?;

    let mut workflows = Vec::with_capacity(runners.len());
    for r in &runners {
        let run = r.run(&scenario).context("runner failed")?;
        let s = score(&scenario, &run, &bpe).context("scoring failed")?;
        workflows.push(s);
    }

    let report = ScenarioReport {
        scenario: scenario.name.to_string(),
        bug_marker: scenario.bug_marker.to_string(),
        bug_file: scenario.bug_file.to_string(),
        bug_line: scenario.bug_line,
        workflows,
    };

    let stdout = io::stdout();
    let mut lock = stdout.lock();
    print_markdown(&mut lock, &report)?;
    lock.flush()?;

    let out_path = save_json(&report, Path::new(output_dir))?;
    eprintln!("\nreport saved → {}", out_path.display());

    if let Some(failed) = report.workflows.iter().find(|w| !w.found_bug) {
        eprintln!(
            "WARNING: workflow `{}` did not surface bug marker `{}`",
            failed.runner, report.bug_marker
        );
    }

    Ok(())
}

fn select_runners(filter: &str) -> Result<Vec<Box<dyn Runner>>> {
    let lower = filter.to_lowercase();
    let mut runners: Vec<Box<dyn Runner>> = Vec::new();
    if lower == "all" || lower.is_empty() {
        runners.push(Box::new(BashRunner));
        runners.push(Box::new(PluckRunner));
        return Ok(runners);
    }
    for name in lower.split(',') {
        match name.trim() {
            "bash" | "grep" => runners.push(Box::new(BashRunner)),
            "pluck" => runners.push(Box::new(PluckRunner)),
            other => anyhow::bail!("unknown runner: {other}"),
        }
    }
    if runners.is_empty() {
        anyhow::bail!("no runners selected");
    }
    Ok(runners)
}

/// Convenience for tests + dogfood: synchronous variant that scores both
/// runners and returns the report struct instead of printing.
#[cfg(test)]
pub fn run_report(scenario: &Scenario) -> Result<ScenarioReport> {
    let bpe = bpe()?;
    let runners: Vec<Box<dyn Runner>> = vec![Box::new(BashRunner), Box::new(PluckRunner)];
    let mut workflows = Vec::new();
    for r in &runners {
        let run = r.run(scenario)?;
        let s = score(scenario, &run, &bpe)?;
        workflows.push(s);
    }
    Ok(ScenarioReport {
        scenario: scenario.name.to_string(),
        bug_marker: scenario.bug_marker.to_string(),
        bug_file: scenario.bug_file.to_string(),
        bug_line: scenario.bug_line,
        workflows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_token_expiry_scenario_runs_end_to_end() {
        let s = load_scenario("fix-auth-token-expiry").expect("scenario exists");
        let report = run_report(&s).expect("run report");
        assert_eq!(report.workflows.len(), 2);
        // Both workflows must surface the seeded bug.
        for w in &report.workflows {
            assert!(
                w.found_bug,
                "{} failed to surface bug marker",
                w.runner
            );
        }
        // pluck should beat bash on tokens — that's the whole point of the
        // scenario. If this regresses we want a loud failure.
        let bash = report.workflows.iter().find(|w| w.runner.starts_with("bash")).unwrap();
        let pluck = report.workflows.iter().find(|w| w.runner.starts_with("pluck")).unwrap();
        assert!(
            pluck.total_tokens < bash.total_tokens,
            "pluck must use fewer tokens than bash; got pluck={} bash={}",
            pluck.total_tokens, bash.total_tokens
        );
    }
}
