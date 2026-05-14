//! Token counting and recall scoring for a workflow run.
//!
//! Token count uses cl100k_base — the BPE Claude / GPT-4 use. Recall is
//! binary at this phase: did the workflow's combined tool outputs surface
//! the scenario's bug marker substring? Phase 4 will swap this for
//! LLM-as-judge correctness + structured patch-diff validation.

use anyhow::{Context, Result};
use serde::Serialize;
use tiktoken_rs::CoreBPE;

use crate::runners::{ToolCall, WorkflowRun};
use crate::scenarios::Scenario;

#[derive(Debug, Clone, Serialize)]
pub struct StepScore {
    pub tool: String,
    pub query: String,
    pub tokens: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowScore {
    pub runner: String,
    pub steps: Vec<StepScore>,
    pub total_tokens: usize,
    pub call_count: usize,
    pub found_bug: bool,
}

pub fn score(scenario: &Scenario, run: &WorkflowRun, bpe: &CoreBPE) -> Result<WorkflowScore> {
    let mut steps = Vec::with_capacity(run.calls.len());
    let mut total = 0usize;
    for c in &run.calls {
        let t = count_tokens(bpe, &c.output)?;
        total += t;
        steps.push(StepScore {
            tool: c.tool.to_string(),
            query: c.query.clone(),
            tokens: t,
        });
    }
    Ok(WorkflowScore {
        runner: run.runner.to_string(),
        steps,
        total_tokens: total,
        call_count: run.calls.len(),
        found_bug: run.surfaces(scenario.bug_marker),
    })
}

fn count_tokens(bpe: &CoreBPE, s: &str) -> Result<usize> {
    Ok(bpe.encode_with_special_tokens(s).len())
}

pub fn bpe() -> Result<CoreBPE> {
    tiktoken_rs::cl100k_base().context("load cl100k_base")
}

// Tiny accessor used by report rendering.
pub fn step_summary(c: &ToolCall, tokens: usize) -> String {
    format!("{:<14} {:<40} {} tok", c.tool, truncate(&c.query, 38), tokens)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}
