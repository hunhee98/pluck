//! pytest test runner output handler.
//!
//! ## State machine
//!
//! ```text
//!  Idle
//!   │  "====== test session starts ======" banner
//!   ▼
//!  Header     — platform / rootdir / configfile lines pass through
//!   │  first "collecting …" or blank line after header
//!   ▼
//!  Collection — "collecting …", "collected N items" collapsed
//!   │  first test node-id or "PASSED/FAILED/ERROR/SKIPPED"
//!   ▼
//!  TestRun    — one-liner pass ("PASSED") held; failure/error kept
//!   │  "====== … passed … ======" short-test-summary or errors section
//!   ▼
//!  Summary    — everything verbatim (short summary, failures block)
//! ```
//!
//! ## Preservation rules
//!
//! * **Always keep**: banner lines (`====`), `FAILED`/`ERROR` test rows,
//!   traceback lines (`E `, `>`, source context with `_`), `assert`
//!   failure lines, short-test-summary, warnings summary.
//! * **Collapse**: `PASSED` one-liners when the run is all-green (held
//!   until we see either a failure or the final summary). `collecting …`
//!   lines replaced with a single `[pytest] collected N items` summary.
//! * **Pass-through**: anything unrecognised, so unknown pytest plugins
//!   whose output we haven't modelled don't get silently dropped.

use std::fmt::Write;

/// Collapse pytest output, preserving every failure diagnostic.
pub fn digest(input: &str) -> String {
    let mut out = String::with_capacity(input.len() / 5);

    let mut state = State::Idle;
    let mut collected_items: Option<usize> = None;
    // PASSED lines held until we know the run outcome.
    let mut held_passed: Vec<String> = Vec::new();
    let mut any_failure = false;

    for raw in input.lines() {
        let trimmed = raw.trim_start();

        match state {
            State::Idle => {
                if is_session_banner(trimmed) {
                    state = State::Header;
                    push_line(&mut out, raw);
                } else {
                    push_line(&mut out, raw);
                }
            }

            State::Header => {
                // Keep platform / rootdir / configfile / plugins lines.
                // Exit to Collection on first "collecting" or blank line.
                if is_collection_line(trimmed) {
                    state = State::Collection;
                    collect_tally(trimmed, &mut collected_items);
                    // don't emit — we'll summarize later
                } else if raw.trim().is_empty() {
                    state = State::Collection;
                    push_line(&mut out, raw);
                } else {
                    push_line(&mut out, raw);
                }
            }

            State::Collection => {
                if is_collection_line(trimmed) {
                    collect_tally(trimmed, &mut collected_items);
                    // absorb — summary emitted when we leave collection
                } else {
                    // Leaving collection: emit summary, then process line.
                    if let Some(n) = collected_items.take() {
                        let _ = writeln!(out, "[pytest] collected {n} items");
                    }
                    state = State::TestRun;
                    // Fall through — process this line as TestRun.
                    process_test_run_line(
                        raw,
                        trimmed,
                        &mut state,
                        &mut held_passed,
                        &mut any_failure,
                        &mut out,
                    );
                }
            }

            State::TestRun => {
                process_test_run_line(
                    raw,
                    trimmed,
                    &mut state,
                    &mut held_passed,
                    &mut any_failure,
                    &mut out,
                );
            }

            State::Summary => {
                push_line(&mut out, raw);
            }
        }
    }

    // Flush held PASSED lines if we never saw a failure or summary.
    if !held_passed.is_empty() && !any_failure {
        // All-green: drop held PASSED lines (they're noise). The caller
        // already has the "collected N items" summary and will see the
        // final result line when it arrives. If the input was truncated
        // before the result, we don't re-emit the PASSED lines — they
        // provide no signal an agent would act on.
        held_passed.clear();
    }

    out
}

fn process_test_run_line(
    raw: &str,
    trimmed: &str,
    state: &mut State,
    held_passed: &mut Vec<String>,
    any_failure: &mut bool,
    out: &mut String,
) {
    // ── Summary / error section openers ──────────────────────────────
    if is_summary_banner(trimmed) || is_errors_section(trimmed) {
        // Flush held PASSED lines on failure; drop on all-green.
        if *any_failure {
            for h in held_passed.drain(..) {
                push_line(out, &h);
            }
        } else {
            held_passed.clear();
        }
        *state = State::Summary;
        push_line(out, raw);
        return;
    }

    // ── Test result rows ──────────────────────────────────────────────
    // Compact one-liner: `tests/test_foo.py::test_bar PASSED`
    // Verbose one-liner: `PASSED tests/test_foo.py::test_bar (0.01s)`
    if is_passed_line(trimmed) {
        // Hold until we know the outcome.
        held_passed.push(raw.to_string());
        return;
    }
    if is_failure_line(trimmed) {
        *any_failure = true;
        // Flush all held PASSED so the order matches pytest's output.
        for h in held_passed.drain(..) {
            push_line(out, &h);
        }
        push_line(out, raw);
        return;
    }

    // ── Traceback / failure body ──────────────────────────────────────
    if is_failure_body(trimmed) {
        push_line(out, raw);
        return;
    }

    // ── Default: pass through ─────────────────────────────────────────
    push_line(out, raw);
}

// ── States ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum State {
    Idle,
    Header,
    Collection,
    TestRun,
    Summary,
}

// ── Classifiers ───────────────────────────────────────────────────────────

fn is_session_banner(trimmed: &str) -> bool {
    trimmed.starts_with("====") && trimmed.contains("test session starts")
}

fn is_summary_banner(trimmed: &str) -> bool {
    trimmed.starts_with("====")
        && (trimmed.contains(" passed")
            || trimmed.contains(" failed")
            || trimmed.contains(" error")
            || trimmed.contains(" warning")
            || trimmed.contains("short test summary")
            || trimmed.contains("no tests ran"))
}

fn is_errors_section(trimmed: &str) -> bool {
    trimmed.starts_with("====") && trimmed.contains("ERRORS")
        || trimmed.starts_with("____") && trimmed.contains("ERROR")
}

fn is_collection_line(trimmed: &str) -> bool {
    trimmed.starts_with("collecting ") || trimmed.starts_with("collected ")
}

fn collect_tally(trimmed: &str, count: &mut Option<usize>) {
    // `collected N items` or `collected N item (M deselected)`
    if let Some(rest) = trimmed.strip_prefix("collected ") {
        if let Some(n) = rest
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<usize>().ok())
        {
            *count = Some(n);
        }
    }
}

fn is_passed_line(trimmed: &str) -> bool {
    // `tests/test_foo.py::test_bar PASSED`
    // `PASSED tests/test_foo.py::test_bar (0.01s)`
    (trimmed.ends_with("PASSED") || trimmed.ends_with("PASSED)") || trimmed.starts_with("PASSED "))
        || (trimmed.ends_with("SKIPPED") || trimmed.contains("SKIPPED["))
        || (trimmed.ends_with("xfail") || trimmed.ends_with("xpassed"))
}

fn is_failure_line(trimmed: &str) -> bool {
    trimmed.ends_with("FAILED")
        || trimmed.ends_with("FAILED)")
        || trimmed.starts_with("FAILED ")
        || trimmed.ends_with("ERROR")
        || trimmed.ends_with("ERROR)")
        || trimmed.starts_with("ERROR ")
}

fn is_failure_body(trimmed: &str) -> bool {
    // pytest failure body lines start with `E ` (assertion / exception),
    // `>` (the line that triggered the error), or `_` repeated (section separator).
    trimmed.starts_with("E ")
        || trimmed.starts_with("E\t")
        || trimmed == "E"
        || trimmed.starts_with("____")
        || (trimmed.starts_with('>') && !trimmed.starts_with("> "))
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

    fn banner() -> &'static str {
        "==================== test session starts ====================\n"
    }

    fn platform() -> &'static str {
        "platform darwin -- Python 3.11.0, pytest-7.4.0, pluggy-1.3.0\n"
    }

    fn collecting(n: usize) -> String {
        format!("collected {n} items\n")
    }

    #[test]
    fn collapses_collection_to_one_line() {
        let input = format!(
            "{}{}{}{}tests/test_foo.py::test_bar PASSED\ntest result: ok\n",
            banner(),
            platform(),
            "collecting ...\n",
            collecting(3),
        );
        let out = digest(&input);
        assert!(
            out.contains("[pytest] collected 3 items"),
            "missing collected summary: {out}"
        );
        assert!(
            !out.contains("collecting ..."),
            "raw collecting line leaked: {out}"
        );
    }

    #[test]
    fn collapses_all_green_passed_lines() {
        let input = format!(
            "{}{}{}tests/t.py::a PASSED\ntests/t.py::b PASSED\ntests/t.py::c PASSED\n{}\n",
            banner(),
            platform(),
            collecting(3),
            "==================== 3 passed in 0.12s ====================",
        );
        let out = digest(&input);
        assert!(
            out.contains("3 passed"),
            "final summary must survive: {out}"
        );
        assert!(
            !out.contains("PASSED"),
            "per-test PASSED lines must be collapsed on all-green: {out}"
        );
        assert!(out.len() < input.len(), "must shrink: {out}");
    }

    #[test]
    fn keeps_failing_run_in_full() {
        let input = format!(
            "{}{}{}tests/t.py::a PASSED\ntests/t.py::b FAILED\ntests/t.py::c PASSED\n{}",
            banner(),
            platform(),
            collecting(3),
            "==================== 1 failed, 2 passed in 0.14s ====================\n",
        );
        let out = digest(&input);
        assert!(
            out.contains("PASSED"),
            "PASSED lines must survive on failure: {out}"
        );
        assert!(out.contains("FAILED"), "FAILED line must survive: {out}");
        assert!(
            out.contains("1 failed, 2 passed"),
            "final summary must survive: {out}"
        );
    }

    #[test]
    fn keeps_traceback_body() {
        let input = format!(
            "{}{}{}tests/t.py::bad FAILED\n____test_bad____\nE   AssertionError: assert 1 == 2\nE   where 1 = foo()\n{}",
            banner(),
            platform(),
            collecting(1),
            "==================== 1 failed in 0.05s ====================\n",
        );
        let out = digest(&input);
        assert!(
            out.contains("E   AssertionError"),
            "traceback must survive: {out}"
        );
        assert!(
            out.contains("assert 1 == 2"),
            "assertion must survive: {out}"
        );
    }

    #[test]
    fn does_not_grow_input_on_unknown_content() {
        let input = "random text\nno pytest markers\n";
        let out = digest(input);
        assert!(out.len() <= input.len() + 1);
    }

    #[test]
    fn keeps_short_test_summary_section() {
        let input = format!(
            "{}{}{}tests/t.py::bad FAILED\n==================== short test summary info ====================\nFAILED tests/t.py::bad\n==================== 1 failed in 0.05s ====================\n",
            banner(),
            platform(),
            collecting(1),
        );
        let out = digest(&input);
        assert!(
            out.contains("short test summary"),
            "short-summary header must survive: {out}"
        );
        assert!(
            out.contains("FAILED tests/t.py::bad"),
            "summary content must survive: {out}"
        );
    }
}
