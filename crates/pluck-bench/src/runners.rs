//! Deterministic agent-workflow runners.
//!
//! Each runner simulates a plausible sequence of tool calls an agent would
//! make to solve the scenario task. Outputs are exactly what the tool
//! would have returned to the agent — those bytes are what ends up in
//! the model's context window, so that's what we measure.
//!
//! Phase 0: hand-written. Phase 4 will replace these with real LLM tool
//! selection over the same scenario fixtures; the bug-marker recall
//! check stays apples-to-apples.

use anyhow::Result;
use pluck_core::chunker::Language;
use pluck_core::index::PluckIndex;
use pluck_core::indexer::index_files_in_memory;
use pluck_core::outliner::{outline_source, render as render_outline};

use crate::scenarios::Scenario;

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub tool: &'static str,
    pub query: String,
    pub output: String,
}

#[derive(Debug, Clone)]
pub struct WorkflowRun {
    pub runner: &'static str,
    pub calls: Vec<ToolCall>,
}

impl WorkflowRun {
    pub fn surfaces(&self, marker: &str) -> bool {
        self.calls.iter().any(|c| c.output.contains(marker))
    }
}

pub trait Runner {
    /// Display name. Kept for future report rendering and runner-listing
    /// commands; the current driver renders runners via their
    /// `WorkflowRun.runner` field.
    #[allow(dead_code)]
    fn name(&self) -> &'static str;

    fn run(&self, scenario: &Scenario) -> Result<WorkflowRun>;
}

// ── Bash baseline: `rg -l` / `cat` / `rg -n` ────────────────────────────────

pub struct BashRunner;

impl Runner for BashRunner {
    fn name(&self) -> &'static str {
        "bash (rg + cat)"
    }

    fn run(&self, scenario: &Scenario) -> Result<WorkflowRun> {
        let mut calls = Vec::new();

        // Step 1: rg -l on a broad keyword from the task description.
        let q1 = "token";
        let files1 = rg_files_with_match(&scenario.repo, q1);
        calls.push(ToolCall {
            tool: "rg -l",
            query: q1.into(),
            output: files1.join("\n"),
        });

        // Step 2: cat the first 3 matched files (a realistic agent reads
        // the top matches before narrowing).
        for path in files1.iter().take(3) {
            let content = scenario
                .repo
                .iter()
                .find(|(p, _)| p == path)
                .map(|(_, c)| c.clone())
                .unwrap_or_default();
            calls.push(ToolCall {
                tool: "cat",
                query: path.clone(),
                output: format!("=== {path} ===\n{content}"),
            });
        }

        // Step 3: rg -n for "expir" to narrow.
        let q2 = "expir";
        let hits = rg_lines_with_match(&scenario.repo, q2);
        calls.push(ToolCall {
            tool: "rg -n",
            query: q2.into(),
            output: hits.join("\n"),
        });

        // Step 4: cat any file that surfaced "expir" we haven't already cat'd.
        let already_cat: std::collections::HashSet<String> = calls
            .iter()
            .filter(|c| c.tool == "cat")
            .map(|c| c.query.clone())
            .collect();
        let q2_files = rg_files_with_match(&scenario.repo, q2);
        for path in q2_files.iter().take(3) {
            if already_cat.contains(path) {
                continue;
            }
            let content = scenario
                .repo
                .iter()
                .find(|(p, _)| p == path)
                .map(|(_, c)| c.clone())
                .unwrap_or_default();
            calls.push(ToolCall {
                tool: "cat",
                query: path.clone(),
                output: format!("=== {path} ===\n{content}"),
            });
        }

        Ok(WorkflowRun {
            runner: "bash (rg + cat)",
            calls,
        })
    }
}

fn rg_files_with_match(repo: &[(String, String)], needle: &str) -> Vec<String> {
    let needle = needle.to_lowercase();
    let mut out = Vec::new();
    for (path, src) in repo {
        if src.to_lowercase().contains(&needle) {
            out.push(path.clone());
        }
    }
    out
}

fn rg_lines_with_match(repo: &[(String, String)], needle: &str) -> Vec<String> {
    let needle = needle.to_lowercase();
    let mut out = Vec::new();
    for (path, src) in repo {
        for (i, line) in src.lines().enumerate() {
            if line.to_lowercase().contains(&needle) {
                out.push(format!("{path}:{}:{}", i + 1, line));
            }
        }
    }
    out
}

// ── Pluck workflow: search → read outline → narrow search ───────────────────

pub struct PluckRunner;

impl Runner for PluckRunner {
    fn name(&self) -> &'static str {
        "pluck (search + read + symbol)"
    }

    fn run(&self, scenario: &Scenario) -> Result<WorkflowRun> {
        // Build the index once. Indexing cost is not charged to the
        // workflow — the daemon amortizes it across every query in a
        // session.
        let idx = PluckIndex::in_ram()?;
        index_files_in_memory(&idx, &scenario.repo)?;

        let mut calls = Vec::new();

        // Step 1: hybrid search on the task description (BM25 today;
        // semantic stage lands in Phase 2).
        let q1 = "auth session token expiry";
        let hits = idx.search_with_cutoff(q1, 5, 0.12)?;
        calls.push(ToolCall {
            tool: "pluck.search",
            query: q1.into(),
            output: render_search_full(&hits),
        });

        // Step 2: pluck.read on the top hit's file. Outline mode by
        // default — lossless map of the file's symbols, agent fetches
        // bodies on demand.
        let top_path = hits
            .first()
            .map(|h| h.path.clone())
            .unwrap_or_else(|| scenario.bug_file.to_string());
        let src = scenario
            .repo
            .iter()
            .find(|(p, _)| p == &top_path)
            .map(|(_, c)| c.clone())
            .unwrap_or_default();
        let outline = outline_source(&src, Some(Language::TypeScript), &top_path);
        calls.push(ToolCall {
            tool: "pluck.read",
            query: top_path.clone(),
            output: render_outline(&outline),
        });

        // Step 3: targeted search that pulls the buggy function body.
        let q3 = "isSessionExpired";
        let hits3 = idx.search_with_cutoff(q3, 3, 0.12)?;
        calls.push(ToolCall {
            tool: "pluck.search",
            query: q3.into(),
            output: render_search_full(&hits3),
        });

        Ok(WorkflowRun {
            runner: "pluck (search + read + symbol)",
            calls,
        })
    }
}

fn render_search_full(hits: &[pluck_core::index::SearchHit]) -> String {
    let mut out = String::new();
    for h in hits {
        out.push_str(&format!(
            "{:.4}  {}:L{}-{}  {} ({:?})\n",
            h.score, h.path, h.start_line, h.end_line, h.symbol, h.kind
        ));
        out.push_str(&h.content);
        out.push_str("\n\n");
    }
    out
}
