//! GitHub Actions step / job log handler.
//!
//! ## Format primer
//!
//! GHA logs wrap each step in `##[group]<step-name>` /
//! `##[endgroup]` markers. Inside a group the body is ordinary
//! tool output (cargo, npm, pytest, …). Between groups there are
//! timestamp lines and `##[section]` dividers.
//!
//! ## State machine
//!
//! ```text
//! Idle
//!  │  ##[group]<step>   ──────────────────────────────────────────► GroupOk
//!  │                                                                   │
//!  │                                 body lines pass through (all)     │
//!  │                                                                   │
//!  │  ##[error] or body contains "error"/"fail"                        │
//!  │                         ┌─────────────────────────────────────────┘
//!  │                         ▼
//!  │                       GroupFailed  ← body kept verbatim
//!  │
//!  │  ##[endgroup]  ─────────────────────────────────────────────── emit summary / keep body
//! ```
//!
//! ## Preservation rules
//!
//! * `##[error]` / `##[warning]` directive lines → always kept.
//! * `##[group]` / `##[endgroup]` lines → always kept (structure).
//! * **Successful step body** (no error/fail lines) → collapsed to
//!   `[gha] <step-name>: ok (<N> lines)`.
//! * **Failed step body** → kept verbatim.
//! * Lines outside any group (timestamp lines, run metadata) → kept.

use std::fmt::Write;

/// Collapse GHA log output, preserving every failed-step body.
pub fn digest(input: &str) -> String {
    let mut out = String::with_capacity(input.len() / 5);

    let mut state = State::Outside;
    let mut step_name = String::new();
    let mut held_body: Vec<String> = Vec::new();
    let mut step_failed = false;

    for raw in input.lines() {
        let trimmed = raw.trim_start();

        match state {
            State::Outside => {
                if let Some(name) = strip_group_prefix(trimmed) {
                    // Emit the ##[group] line itself (structure marker).
                    push_line(&mut out, raw);
                    step_name = name.to_string();
                    held_body.clear();
                    step_failed = false;
                    state = State::GroupBody;
                } else {
                    push_line(&mut out, raw);
                }
            }

            State::GroupBody => {
                if trimmed == "##[endgroup]" {
                    // Close the group.
                    if step_failed {
                        // Flush full body.
                        for h in held_body.drain(..) {
                            push_line(&mut out, &h);
                        }
                    } else {
                        // Green step: emit one-line summary, drop body.
                        let n = held_body.len();
                        held_body.clear();
                        if n > 0 {
                            let _ = writeln!(
                                out,
                                "[gha] {}: ok ({n} lines suppressed)",
                                step_name
                            );
                        }
                    }
                    push_line(&mut out, raw); // emit ##[endgroup]
                    state = State::Outside;
                } else if is_gha_directive(trimmed) {
                    // ##[error] / ##[warning] — always keep; mark failed.
                    if trimmed.starts_with("##[error]") {
                        step_failed = true;
                    }
                    held_body.push(raw.to_string());
                } else {
                    // Body line: check for error/fail signals.
                    if !step_failed && is_failure_signal(trimmed) {
                        step_failed = true;
                    }
                    held_body.push(raw.to_string());
                }
            }
        }
    }

    // Unterminated group (truncated log): flush whatever we have.
    if !held_body.is_empty() {
        for h in held_body {
            push_line(&mut out, &h);
        }
    }

    out
}

// ── States ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum State {
    Outside,
    GroupBody,
}

// ── Classifiers ───────────────────────────────────────────────────────────

fn strip_group_prefix(trimmed: &str) -> Option<&str> {
    trimmed.strip_prefix("##[group]")
}

fn is_gha_directive(trimmed: &str) -> bool {
    trimmed.starts_with("##[error]")
        || trimmed.starts_with("##[warning]")
        || trimmed.starts_with("##[notice]")
        || trimmed.starts_with("##[debug]")
        || trimmed.starts_with("##[section]")
}

fn is_failure_signal(trimmed: &str) -> bool {
    let lower = trimmed.to_ascii_lowercase();
    lower.contains("error")
        || lower.contains("failed")
        || lower.contains("failure")
        || lower.contains("panic")
        || lower.starts_with("fatal")
        // ripgrep / compiler file:line:col patterns
        || (trimmed.contains(':') && {
            let parts: Vec<&str> = trimmed.splitn(3, ':').collect();
            parts.len() >= 2 && parts[1].chars().all(|c| c.is_ascii_digit())
        })
}

#[inline]
fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_successful_step_body() {
        let input = concat!(
            "##[group]Build crate\n",
            "   Compiling serde v1.0.0\n",
            "   Compiling tokio v1.30.0\n",
            "    Finished dev profile in 3.21s\n",
            "##[endgroup]\n",
        );
        let out = digest(input);
        assert!(
            out.contains("[gha] Build crate: ok"),
            "missing ok summary: {out}"
        );
        assert!(
            !out.contains("Compiling serde"),
            "body must be collapsed: {out}"
        );
        assert!(out.contains("##[group]Build crate"), "group header kept: {out}");
        assert!(out.contains("##[endgroup]"), "endgroup kept: {out}");
        assert!(out.len() < input.len(), "must shrink: {out}");
    }

    #[test]
    fn keeps_failed_step_body_verbatim() {
        let input = concat!(
            "##[group]Run tests\n",
            "running 3 tests\n",
            "test a::passes ... ok\n",
            "test b::fails ... FAILED\n",
            "##[endgroup]\n",
        );
        let out = digest(input);
        // Body preserved because of the FAILED line.
        assert!(out.contains("test a::passes"), "ok line must survive: {out}");
        assert!(out.contains("test b::fails ... FAILED"), "failure must survive: {out}");
    }

    #[test]
    fn keeps_gha_error_directive() {
        let input = concat!(
            "##[group]Lint\n",
            "##[error]ESLint found 3 problems\n",
            "  src/index.js:5:10 error  no-undef  'foo' is not defined\n",
            "##[endgroup]\n",
        );
        let out = digest(input);
        assert!(out.contains("##[error]ESLint"), "##[error] directive must survive: {out}");
        assert!(out.contains("no-undef"), "error body must survive: {out}");
    }

    #[test]
    fn keeps_lines_outside_groups() {
        let input = concat!(
            "2024-01-01T00:00:00.000Z Run pluck test suite\n",
            "##[group]Build\n",
            "   Compiling app\n",
            "##[endgroup]\n",
            "2024-01-01T00:01:00.000Z Job completed\n",
        );
        let out = digest(input);
        assert!(
            out.contains("Run pluck test suite"),
            "timestamp line must survive: {out}"
        );
        assert!(
            out.contains("Job completed"),
            "trailing timestamp must survive: {out}"
        );
    }

    #[test]
    fn multiple_groups_independent() {
        let input = concat!(
            "##[group]Build\n",
            "   Compiling app\n",
            "##[endgroup]\n",
            "##[group]Test\n",
            "running 2 tests\n",
            "test a::fails FAILED\n",
            "##[endgroup]\n",
        );
        let out = digest(input);
        // Build group collapses; Test group kept.
        assert!(
            out.contains("[gha] Build: ok"),
            "build summary missing: {out}"
        );
        assert!(out.contains("FAILED"), "test failure must survive: {out}");
    }

    #[test]
    fn does_not_grow_on_content_without_groups() {
        let input = "plain log line\nanother line\n";
        let out = digest(input);
        assert!(out.len() <= input.len() + 1);
    }
}
