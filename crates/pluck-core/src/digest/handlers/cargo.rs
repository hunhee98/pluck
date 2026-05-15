//! Cargo build / test / check output handler.
//!
//! Three rolling states drive the line walker:
//!
//!   * **Progress** — `Compiling X v1.0.0`, `Checking X v1.0.0`,
//!     `Downloading`, `Updating crates.io index` rows. Collapsed
//!     into a single `Compiled N crates` summary.
//!   * **Diagnostic block** — opened by `error[E…]:` / `error:` /
//!     `warning:`. Held verbatim until we hit a blank line that is
//!     not followed by an indented continuation. Every byte of a
//!     diagnostic must survive (file:line:col, the squiggle line,
//!     `help:` / `note:` follow-ups).
//!   * **Test stream** — `running N tests`, `test foo … ok`,
//!     `test result: …`. When every line in the run is `ok`, we
//!     keep only the `running …` / `test result:` summary. As soon
//!     as a `FAILED` row appears we flush every kept test line so
//!     the agent sees the full failure context, and we preserve
//!     the `failures:` block + indented panic traces below.
//!
//! Anything we don't recognize is passed through verbatim. The
//! handler must never lose bytes silently — only collapse progress.

use std::fmt::Write;

/// Collapse cargo output, preserving every diagnostic. See module
/// docs for the rules.
pub fn digest(input: &str) -> String {
    // Heuristic capacity: most digested outputs land between 5–20 %
    // of the input. Pre-allocating 20 % avoids the typical reallocs
    // without overspending when the handler keeps less.
    let mut out = String::with_capacity(input.len() / 5);

    let mut state = State::Idle;
    // Running counters that flush as a single summary line when we
    // leave Progress.
    let mut progress = ProgressCounts::default();
    // Lines held inside a non-cargo "running tests" block whose
    // outcome we haven't decided yet. Released verbatim on FAILED,
    // discarded on all-ok.
    let mut held_tests: Vec<String> = Vec::new();
    let mut any_test_failed = false;

    for raw in input.lines() {
        let line = raw;
        let trimmed = line.trim_start();

        // 1) Top-level "Compiling / Checking / Downloading / Updating"
        //    rows are pure progress. Count + skip.
        if is_progress_line(trimmed) {
            if !matches!(state, State::Progress) {
                state = State::Progress;
            }
            progress.tally(trimmed);
            continue;
        }

        // Leaving progress: emit the summary before whatever comes next.
        if matches!(state, State::Progress) {
            progress.flush(&mut out);
            state = State::Idle;
        }

        // 2) Diagnostic blocks — preserve completely. We re-enter
        //    Diagnostic on every `error` / `warning` start; only a
        //    blank line that isn't followed by an indented line ends
        //    the block (handled below).
        if is_diagnostic_start(trimmed) {
            state = State::Diagnostic;
            push_line(&mut out, line);
            continue;
        }
        if matches!(state, State::Diagnostic) {
            // Hold blank lines until we see whether the diagnostic
            // continues with indentation. Cargo / rustc separate
            // entries with a single blank line.
            if line.is_empty() {
                push_line(&mut out, line);
                state = State::DiagnosticAfterBlank;
                continue;
            }
            push_line(&mut out, line);
            continue;
        }
        if matches!(state, State::DiagnosticAfterBlank) {
            // An indented continuation keeps us in the diagnostic
            // block. Anything else closes it.
            if line.starts_with(' ') || line.starts_with('\t') {
                push_line(&mut out, line);
                state = State::Diagnostic;
                continue;
            }
            state = State::Idle;
            // fall through to the rest of the matchers for this line
        }

        // 3) Test stream.
        if trimmed.starts_with("running ") && trimmed.ends_with(" tests")
            || trimmed.starts_with("running 0 tests")
            || trimmed == "running 1 test"
        {
            // Flush any prior held-tests as a precaution (shouldn't
            // happen in normal cargo output, but defensive).
            if !held_tests.is_empty() {
                for h in held_tests.drain(..) {
                    push_line(&mut out, &h);
                }
            }
            any_test_failed = false;
            push_line(&mut out, line);
            state = State::TestRun;
            continue;
        }
        if matches!(state, State::TestRun) {
            // Order matters: "test result: ok. N passed; …" also
            // begins with "test " and contains " ok", so the
            // summary-line check has to run before is_test_line.
            if trimmed.starts_with("test result:") {
                // End-of-run summary. If everything was ok we drop
                // the held ok-lines and emit only the summary; on
                // failure we already flushed.
                if !any_test_failed {
                    held_tests.clear();
                }
                push_line(&mut out, line);
                state = State::Idle;
                continue;
            }
            if is_test_line(trimmed) {
                if trimmed.contains(" FAILED") {
                    any_test_failed = true;
                    // Once we've seen a failure, flush every held
                    // ok-line so the order matches what cargo wrote.
                    for h in held_tests.drain(..) {
                        push_line(&mut out, &h);
                    }
                    push_line(&mut out, line);
                } else if any_test_failed {
                    push_line(&mut out, line);
                } else {
                    held_tests.push(line.to_string());
                }
                continue;
            }
            if trimmed.starts_with("failures:") || trimmed.starts_with("---- ") {
                // We're entering the panic / failure detail block.
                // Flush whatever's pending so the surrounding context
                // sits with the bodies that follow.
                for h in held_tests.drain(..) {
                    push_line(&mut out, &h);
                }
                push_line(&mut out, line);
                state = State::TestFailureBody;
                continue;
            }
            // Anything else mid-run: pass through (keeps stderr
            // interleaving readable).
            push_line(&mut out, line);
            continue;
        }
        if matches!(state, State::TestFailureBody) {
            push_line(&mut out, line);
            // Empty line ends the failure body unless the next
            // line is also indented — we don't peek; cargo's
            // "test result:" reliably re-opens the test stream.
            if trimmed.starts_with("test result:") {
                state = State::Idle;
            }
            continue;
        }

        // 4) Final-summary lines we always keep.
        if is_keep_always(trimmed) {
            push_line(&mut out, line);
            continue;
        }

        // 5) Default: keep verbatim. Cargo emits user-facing prints
        //    (`println!` from build.rs, `cargo:warning=…`, etc.) we
        //    don't want to lose.
        push_line(&mut out, line);
    }

    // Flush trailing progress if the input ends mid-build.
    if matches!(state, State::Progress) {
        progress.flush(&mut out);
    }
    // Test-run that ended without a result line: emit any held lines
    // so we never silently drop them.
    for h in held_tests {
        push_line(&mut out, &h);
    }

    out
}

#[derive(Debug, Clone, Copy)]
enum State {
    Idle,
    Progress,
    Diagnostic,
    DiagnosticAfterBlank,
    TestRun,
    TestFailureBody,
}

#[derive(Default)]
struct ProgressCounts {
    compiling: usize,
    checking: usize,
    downloading: usize,
    updating: usize,
}

impl ProgressCounts {
    fn tally(&mut self, trimmed: &str) {
        if trimmed.starts_with("Compiling ") {
            self.compiling += 1;
        } else if trimmed.starts_with("Checking ") {
            self.checking += 1;
        } else if trimmed.starts_with("Downloading ") {
            self.downloading += 1;
        } else if trimmed.starts_with("Updating ") {
            self.updating += 1;
        }
    }

    fn flush(&mut self, out: &mut String) {
        let mut parts: Vec<String> = Vec::new();
        if self.compiling > 0 {
            parts.push(format!("compiled {}", self.compiling));
        }
        if self.checking > 0 {
            parts.push(format!("checked {}", self.checking));
        }
        if self.downloading > 0 {
            parts.push(format!("downloaded {}", self.downloading));
        }
        if self.updating > 0 {
            parts.push(format!("updated {} registries", self.updating));
        }
        if !parts.is_empty() {
            let _ = writeln!(out, "[cargo] {}", parts.join(", "));
        }
        *self = ProgressCounts::default();
    }
}

fn is_progress_line(trimmed: &str) -> bool {
    trimmed.starts_with("Compiling ")
        || trimmed.starts_with("Checking ")
        || trimmed.starts_with("Downloading ")
        || trimmed.starts_with("Updating ")
        || trimmed.starts_with("Documenting ")
}

fn is_diagnostic_start(trimmed: &str) -> bool {
    if trimmed.starts_with("error[") || trimmed.starts_with("error: ") || trimmed == "error" {
        return true;
    }
    if trimmed.starts_with("warning[") || trimmed.starts_with("warning: ") {
        return true;
    }
    // `error: aborting due to N previous errors` is a summary; keep
    // it as a diagnostic line so the agent always sees terminal
    // status.
    if trimmed.starts_with("error: aborting due to") {
        return true;
    }
    false
}

fn is_test_line(trimmed: &str) -> bool {
    // Matches `test foo::bar ... ok`, `... FAILED`, `... ignored`,
    // `... bench`. The trailing `...` is `... `.
    trimmed.starts_with("test ")
        && (trimmed.contains(" ok") || trimmed.contains(" FAILED") || trimmed.contains(" ignored")
            || trimmed.contains(" bench:"))
}

fn is_keep_always(trimmed: &str) -> bool {
    // The terminal "Finished … in 5.99s" / "Running …" / "error: build
    // failed" / "warning: …" headers and any "thread 'foo' panicked".
    trimmed.starts_with("Finished ")
        || trimmed.starts_with("Running ")
        || trimmed.starts_with("Executable ")
        || trimmed.starts_with("Doc-tests")
        || trimmed.starts_with("thread '")
        || trimmed.starts_with("note:")
        || trimmed.starts_with("help:")
        || trimmed.starts_with("stack backtrace")
}

#[inline]
fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_progress_into_summary() {
        let input = "   Compiling serde v1.0.0\n   Compiling tokio v1.30.0\n   Compiling foo v0.1.0 (/tmp/foo)\n    Finished `dev` profile [optimized + debuginfo] target(s) in 3.21s\n";
        let out = digest(input);
        // 3 Compiling rows → 1 summary line + Finished row.
        assert!(
            out.contains("[cargo] compiled 3"),
            "missing compiled summary, got: {out}"
        );
        assert!(out.contains("Finished"), "Finished line must survive: {out}");
        // No raw Compiling rows.
        assert!(
            !out.contains("Compiling serde"),
            "raw Compiling row leaked: {out}"
        );
        assert!(out.len() < input.len(), "digest must not grow input");
    }

    #[test]
    fn keeps_error_block_verbatim() {
        let input = "   Compiling foo v0.1.0\nerror[E0382]: borrow of moved value: `s`\n  --> src/lib.rs:3:5\n   |\n 2 |     let s = String::new();\n 3 |     drop(s);\n   |          - value moved here\n   |\nerror: aborting due to previous error\n";
        let out = digest(input);
        assert!(
            out.contains("error[E0382]"),
            "error opener must survive: {out}"
        );
        assert!(
            out.contains("src/lib.rs:3:5"),
            "file:line:col must survive: {out}"
        );
        assert!(
            out.contains("value moved here"),
            "diagnostic body must survive: {out}"
        );
        assert!(
            out.contains("error: aborting due to"),
            "terminal summary must survive: {out}"
        );
    }

    #[test]
    fn collapses_all_green_test_run() {
        let input = "   Compiling foo v0.1.0\n    Finished test [optimized + debuginfo] target(s)\n     Running unittests src/lib.rs\n\nrunning 5 tests\ntest a::passes ... ok\ntest b::passes ... ok\ntest c::passes ... ok\ntest d::passes ... ok\ntest e::passes ... ok\n\ntest result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
        let out = digest(input);
        assert!(out.contains("running 5 tests"));
        assert!(out.contains("test result: ok. 5 passed"));
        // Per-test ok lines should be dropped.
        assert!(
            !out.contains("test a::passes"),
            "per-test ok line leaked: {out}"
        );
        // But "Running unittests …" should stay.
        assert!(out.contains("Running unittests"));
    }

    #[test]
    fn keeps_failing_test_run_in_full() {
        let input = "running 3 tests\ntest a::passes ... ok\ntest b::fails ... FAILED\ntest c::passes ... ok\n\nfailures:\n\n---- b::fails stdout ----\nthread 'b::fails' panicked at 'assertion failed: false', src/lib.rs:42:5\n\nfailures:\n    b::fails\n\ntest result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured\n";
        let out = digest(input);
        // Every test row must survive when a failure exists.
        assert!(out.contains("test a::passes ... ok"), "got: {out}");
        assert!(out.contains("test b::fails ... FAILED"));
        assert!(out.contains("test c::passes ... ok"));
        // Panic + traceback must survive.
        assert!(out.contains("thread 'b::fails' panicked"));
        assert!(out.contains("src/lib.rs:42:5"));
        // Final summary intact.
        assert!(out.contains("test result: FAILED. 2 passed; 1 failed"));
    }

    #[test]
    fn keeps_warning_block() {
        let input = "   Compiling foo v0.1.0\nwarning: unused variable: `x`\n  --> src/lib.rs:5:9\n   |\n 5 |     let x = 1;\n   |         ^ help: prefix with underscore\n   |\n    Finished\n";
        let out = digest(input);
        assert!(out.contains("warning: unused variable"));
        assert!(out.contains("src/lib.rs:5:9"));
        assert!(out.contains("help: prefix with underscore"));
    }

    #[test]
    fn does_not_grow_input() {
        // Pathological case — nothing collapsible, just text. The
        // handler should pass through, not balloon the output.
        let input = "some build-script println output\ncargo:warning=hello\ncargo:rerun-if-changed=build.rs\n";
        let out = digest(input);
        assert!(out.len() <= input.len() + 1);
    }
}
