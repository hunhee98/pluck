//! Scenario registry.
//!
//! A scenario bundles a fixture repo + an agent task description + a
//! verifiable success criterion (a "bug marker" — the exact substring the
//! agent's tool-call outputs must surface to be credited with discovery).
//!
//! Phase 0: deterministic. Runners are hand-written workflows that
//! approximate what an agent would do. Phase 4 will replace these with
//! real LLM-driven tool selection.

pub mod auth_token_expiry;

pub struct Scenario {
    pub name: &'static str,
    /// Stored for Phase 4 — once the runners are LLM-driven this is the
    /// prompt sent to the agent. The Phase 0 deterministic runners ignore
    /// it.
    #[allow(dead_code)]
    pub task_prompt: &'static str,
    pub repo: Vec<(String, String)>,
    pub bug_marker: &'static str,
    pub bug_file: &'static str,
    pub bug_line: u32,
}

pub fn load(name: &str) -> Option<Scenario> {
    match name {
        "fix-auth-token-expiry" | "fix/auth-token-expiry" => Some(auth_token_expiry::scenario()),
        _ => None,
    }
}

pub fn all_names() -> &'static [&'static str] {
    &["fix-auth-token-expiry"]
}
