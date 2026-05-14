//! Pretty-print scenario results for the terminal and persist a JSON copy
//! for the dashboard.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::scoring::{StepScore, WorkflowScore};

#[derive(Debug, Serialize)]
pub struct ScenarioReport {
    pub scenario: String,
    pub bug_marker: String,
    pub bug_file: String,
    pub bug_line: u32,
    pub workflows: Vec<WorkflowScore>,
}

pub fn print_markdown(out: &mut impl Write, report: &ScenarioReport) -> Result<()> {
    writeln!(out, "# Scenario: `{}`", report.scenario)?;
    writeln!(out)?;
    writeln!(
        out,
        "Bug seeded at `{}:{}` — marker substring `{}`.",
        report.bug_file, report.bug_line, report.bug_marker
    )?;
    writeln!(out)?;

    // Summary table
    writeln!(out, "| Runner | Calls | Total tokens | Bug surfaced? |")?;
    writeln!(out, "|--------|------:|-------------:|:-------------:|")?;
    for w in &report.workflows {
        writeln!(
            out,
            "| {} | {} | {} | {} |",
            w.runner,
            w.call_count,
            w.total_tokens,
            if w.found_bug { "✅" } else { "❌" }
        )?;
    }
    writeln!(out)?;

    // Savings vs first workflow (treated as baseline).
    if let Some(base) = report.workflows.first() {
        for w in report.workflows.iter().skip(1) {
            let pct = if base.total_tokens > 0 {
                100.0 * (base.total_tokens as f64 - w.total_tokens as f64)
                    / base.total_tokens as f64
            } else {
                0.0
            };
            writeln!(
                out,
                "`{}` vs `{}`: **{:.1}%** fewer tokens.",
                w.runner, base.runner, pct
            )?;
        }
        writeln!(out)?;
    }

    // Per-step breakdown.
    for w in &report.workflows {
        writeln!(out, "## {}", w.runner)?;
        writeln!(out)?;
        writeln!(out, "| # | Tool | Query | Tokens |")?;
        writeln!(out, "|--:|------|-------|------:|")?;
        for (i, s) in w.steps.iter().enumerate() {
            writeln!(
                out,
                "| {} | `{}` | `{}` | {} |",
                i + 1,
                s.tool,
                trim_one_line(&s.query, 50),
                s.tokens
            )?;
        }
        writeln!(out)?;
    }

    Ok(())
}

pub fn save_json(report: &ScenarioReport, out_dir: &Path) -> Result<std::path::PathBuf> {
    fs::create_dir_all(out_dir).context("create output dir")?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let safe = report.scenario.replace('/', "-");
    let path = out_dir.join(format!("{safe}-{ts}.json"));
    let body = serde_json::to_string_pretty(report)?;
    fs::write(&path, body).context("write report JSON")?;
    Ok(path)
}

fn trim_one_line(s: &str, max: usize) -> String {
    let one = s.replace('\n', " ");
    if one.chars().count() <= max {
        return one;
    }
    let mut out: String = one.chars().take(max - 1).collect();
    out.push('…');
    out
}

#[allow(dead_code)]
pub fn step_log(s: &StepScore) -> String {
    format!("[{}] {} — {} tok", s.tool, s.query, s.tokens)
}

/// Legacy entry from the original scaffold — preserved so the existing CLI
/// subcommand still type-checks. The driver does the real work today.
pub fn generate(_input: &str, _markdown: bool) -> Result<()> {
    eprintln!("pluck-bench report aggregation: not implemented yet");
    Ok(())
}
