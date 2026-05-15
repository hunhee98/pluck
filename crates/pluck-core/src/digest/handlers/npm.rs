//! npm / pnpm / yarn / bun install + script output handler.
//!
//! The dominant noise in npm-family output is install-phase progress:
//!
//!   * **yarn** — `[1/4] Resolving packages…`, `[2/4] Fetching…`, …
//!     Collapsed to `[npm] resolved N packages` once `Done` or the
//!     "added N packages" summary appears.
//!   * **pnpm** — repeated `Progress: resolved N, reused M, downloaded X,
//!     added Y` lines. Collapsed to a single `[npm] …` summary when the
//!     phase ends (blank line, or the `done` suffix on the final Progress
//!     line).
//!   * **generic** — spinner residue, bare counts. Dropped if they match
//!     a progress-line pattern.
//!
//! Three categories of lines are always kept verbatim:
//!
//!   1. **Errors** — `npm ERR!`, `error:`, `ENOENT`, lines containing
//!      `failed` / `FAILED`.
//!   2. **Summaries** — `added N packages`, `changed N packages`,
//!      `removed N packages`, `audited N packages`, `Done in X.XXs`,
//!      `Lockfile is up to date`.
//!   3. **Script output** — every line after a `> package scriptname`
//!      header is preserved verbatim (build / test script output must
//!      survive). Script mode exits on the next install summary or
//!      script header.

use std::fmt::Write;

/// Collapse npm-family output, preserving every error and script output.
pub fn digest(input: &str) -> String {
    let mut out = String::with_capacity(input.len() / 5);

    let mut state = State::Idle;
    let mut progress = NpmProgress::default();

    for raw in input.lines() {
        let trimmed = raw.trim_start();

        // ── Always-keep lines ──────────────────────────────────────────
        if is_keep_always(trimmed) {
            flush_progress(&mut progress, &mut out);
            state = State::Idle;
            push_line(&mut out, raw);
            continue;
        }

        // ── Error lines: keep, enter error pass-through ────────────────
        if is_error_line(trimmed) {
            flush_progress(&mut progress, &mut out);
            state = State::Idle;
            push_line(&mut out, raw);
            continue;
        }

        // ── Script header: `> package@ver scriptname` ─────────────────
        if is_script_header(trimmed) {
            flush_progress(&mut progress, &mut out);
            state = State::ScriptRun;
            push_line(&mut out, raw);
            continue;
        }

        // ── Inside a script run: pass through verbatim ─────────────────
        if matches!(state, State::ScriptRun) {
            // An install-summary line (e.g. "added N packages") or the
            // next script header exits script mode. Both branches above
            // handle those cases and reset state, so by the time we
            // reach here we're still in a script body.
            push_line(&mut out, raw);
            continue;
        }

        // ── Progress lines ─────────────────────────────────────────────
        if let Some(kind) = classify_progress(trimmed) {
            state = State::InstallProgress;
            progress.tally(kind, trimmed);
            continue;
        }

        // Leaving install progress on a non-progress, non-keep line.
        if matches!(state, State::InstallProgress) {
            flush_progress(&mut progress, &mut out);
            state = State::Idle;
        }

        // ── Default: pass through ──────────────────────────────────────
        push_line(&mut out, raw);
    }

    // Flush any trailing progress block.
    flush_progress(&mut progress, &mut out);

    out
}

// ── States ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum State {
    Idle,
    InstallProgress,
    ScriptRun,
}

// ── Progress accounting ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum ProgressKind {
    YarnPhase,
    PnpmProgress,
    GenericSpinner,
}

#[derive(Default)]
struct NpmProgress {
    yarn_phases: usize,
    pnpm_resolved: usize,
    pnpm_reused: usize,
    pnpm_downloaded: usize,
    pnpm_added: usize,
    generic_lines: usize,
}

impl NpmProgress {
    fn tally(&mut self, kind: ProgressKind, line: &str) {
        match kind {
            ProgressKind::YarnPhase => {
                self.yarn_phases += 1;
            }
            ProgressKind::PnpmProgress => {
                // Parse the last seen counts from the line.
                // `Progress: resolved N, reused M, downloaded X, added Y`
                self.pnpm_resolved = parse_after(line, "resolved ").unwrap_or(self.pnpm_resolved);
                self.pnpm_reused = parse_after(line, "reused ").unwrap_or(self.pnpm_reused);
                self.pnpm_downloaded =
                    parse_after(line, "downloaded ").unwrap_or(self.pnpm_downloaded);
                self.pnpm_added = parse_after(line, "added ").unwrap_or(self.pnpm_added);
            }
            ProgressKind::GenericSpinner => {
                self.generic_lines += 1;
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.yarn_phases == 0 && self.pnpm_resolved == 0 && self.generic_lines == 0
    }
}

fn flush_progress(p: &mut NpmProgress, out: &mut String) {
    if p.is_empty() {
        return;
    }
    if p.pnpm_resolved > 0 {
        let _ = writeln!(
            out,
            "[npm] resolved {}, reused {}, downloaded {}, added {}",
            p.pnpm_resolved, p.pnpm_reused, p.pnpm_downloaded, p.pnpm_added
        );
    }
    if p.yarn_phases > 0 {
        let _ = writeln!(out, "[npm] {} install phase(s) complete", p.yarn_phases);
    }
    *p = NpmProgress::default();
}

// ── Classifiers ───────────────────────────────────────────────────────────

fn classify_progress(trimmed: &str) -> Option<ProgressKind> {
    // pnpm: `Progress: resolved N, reused M, downloaded X, added Y`
    if trimmed.starts_with("Progress: resolved ") {
        return Some(ProgressKind::PnpmProgress);
    }
    // yarn: `[1/4] Resolving packages...`
    if is_yarn_phase(trimmed) {
        return Some(ProgressKind::YarnPhase);
    }
    // Bare spinner/download lines: start with known spinner chars or
    // Unicode braille spinners. These appear in npm v7+ interactive mode.
    let first = trimmed.chars().next().unwrap_or(' ');
    if matches!(first, '⠙' | '⠹' | '⠸' | '⠼' | '⠴' | '⠦' | '⠧' | '⠇' | '⠏') {
        return Some(ProgressKind::GenericSpinner);
    }
    // bun install progress lines: `[N/N] ...`
    if trimmed.starts_with('[') {
        if let Some(rest) = trimmed.strip_prefix('[') {
            if rest.contains('/') && rest.contains(']') {
                let digits_only = rest
                    .split('/')
                    .next()
                    .map(|s| s.chars().all(|c| c.is_ascii_digit()))
                    .unwrap_or(false);
                if digits_only && !trimmed.contains("ERR") && !trimmed.contains("error") {
                    return Some(ProgressKind::GenericSpinner);
                }
            }
        }
    }
    None
}

fn is_yarn_phase(trimmed: &str) -> bool {
    // `[1/4] Resolving packages...`
    if !trimmed.starts_with('[') {
        return false;
    }
    let inner = &trimmed[1..];
    let Some(slash) = inner.find('/') else {
        return false;
    };
    let num = &inner[..slash];
    let rest = &inner[slash..];
    if !num.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    // After `N/M]` there should be a space and text.
    rest.starts_with('/') && rest.contains(']')
}

fn is_script_header(trimmed: &str) -> bool {
    // npm/yarn/pnpm script header: `> package@version scriptname`
    // or just `> scriptname` (yarn workspaces).
    if !trimmed.starts_with("> ") {
        return false;
    }
    let rest = &trimmed[2..];
    // Must have at least one non-whitespace token that looks like a
    // package or script name (not a shell redirect).
    !rest.is_empty() && !rest.starts_with('/') && !rest.starts_with("./")
}

fn is_error_line(trimmed: &str) -> bool {
    trimmed.starts_with("npm ERR!")
        || trimmed.starts_with("npm error")
        || trimmed.starts_with("error:")
        || trimmed.starts_with("Error:")
        || trimmed.starts_with("ENOENT")
        || trimmed.starts_with("EACCES")
        || trimmed.to_ascii_lowercase().contains("failed")
        || trimmed.starts_with("warning:")
        || trimmed.starts_with("npm warn ")
        || trimmed.starts_with("WARN ")
}

fn is_keep_always(trimmed: &str) -> bool {
    // Install summary lines.
    (trimmed.starts_with("added ") && trimmed.contains(" packages"))
        || (trimmed.starts_with("changed ") && trimmed.contains(" packages"))
        || (trimmed.starts_with("removed ") && trimmed.contains(" packages"))
        || (trimmed.starts_with("audited ") && trimmed.contains(" packages"))
        || trimmed.starts_with("Done in ")
        || trimmed.starts_with("Lockfile is up to date")
        || trimmed.starts_with("Already up to date")
        || trimmed.starts_with("Packages: ")
        || (trimmed.contains("packages") && trimmed.contains("looking for funding"))
        // pnpm/yarn version headers.
        || trimmed.starts_with("pnpm ")
        || trimmed.starts_with("yarn ")
        || trimmed.starts_with("npm ")
        // `run` keyword from `npm fund`.
        || trimmed.starts_with("run `npm fund`")
}

fn parse_after(s: &str, keyword: &str) -> Option<usize> {
    let pos = s.find(keyword)?;
    let rest = &s[pos + keyword.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
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
    fn collapses_pnpm_progress_to_one_line() {
        let input = concat!(
            "Progress: resolved 134, reused 130, downloaded 0, added 0\n",
            "Progress: resolved 134, reused 130, downloaded 4, added 4\n",
            "Progress: resolved 134, reused 130, downloaded 4, added 4, done\n",
            "Lockfile is up to date\n",
        );
        let out = digest(input);
        // Progress block collapses to one summary.
        assert!(
            out.contains("[npm] resolved 134"),
            "missing pnpm summary: {out}"
        );
        assert!(
            out.lines().filter(|l| l.starts_with("[npm]")).count() == 1,
            "should emit exactly one [npm] line: {out}"
        );
        assert!(out.contains("Lockfile is up to date"), "keep: {out}");
        assert!(out.len() < input.len(), "must shrink: {out}");
    }

    #[test]
    fn collapses_yarn_phases() {
        let input = concat!(
            "yarn install v1.22.0\n",
            "[1/4] Resolving packages...\n",
            "[2/4] Fetching packages...\n",
            "[3/4] Linking dependencies...\n",
            "[4/4] Building fresh packages...\n",
            "Done in 5.21s.\n",
        );
        let out = digest(input);
        assert!(
            out.contains("Done in 5.21s"),
            "Done line must survive: {out}"
        );
        assert!(
            !out.contains("[1/4]"),
            "yarn phase lines must be collapsed: {out}"
        );
        assert!(out.len() < input.len(), "must shrink: {out}");
    }

    #[test]
    fn keeps_added_packages_summary() {
        let input = concat!(
            "Progress: resolved 248, reused 248, downloaded 0, added 0\n",
            "added 248 packages in 5.2s\n",
        );
        let out = digest(input);
        assert!(
            out.contains("added 248 packages in 5.2s"),
            "summary must survive: {out}"
        );
    }

    #[test]
    fn keeps_npm_errors_verbatim() {
        let input = concat!(
            "Progress: resolved 134, reused 130, downloaded 0, added 0\n",
            "npm ERR! code ENOENT\n",
            "npm ERR! syscall open\n",
            "npm ERR! path /tmp/pkg/package.json\n",
        );
        let out = digest(input);
        assert!(
            out.contains("npm ERR! code ENOENT"),
            "error must survive: {out}"
        );
        assert!(
            out.contains("npm ERR! syscall open"),
            "error must survive: {out}"
        );
        assert!(out.contains("npm ERR! path"), "error must survive: {out}");
    }

    #[test]
    fn keeps_script_output_verbatim() {
        let input = concat!(
            "Progress: resolved 50, reused 50, downloaded 0, added 0\n",
            "> app@1.0.0 build\n",
            "> vite build\n",
            "\n",
            "vite v5.0.0 building for production...\n",
            "dist/index.html  0.46 kB\n",
            "added 50 packages in 2s\n",
        );
        let out = digest(input);
        assert!(
            out.contains("> app@1.0.0 build"),
            "script header must survive: {out}"
        );
        assert!(
            out.contains("vite v5.0.0"),
            "script output must survive: {out}"
        );
        assert!(
            out.contains("dist/index.html"),
            "script output must survive: {out}"
        );
        assert!(
            out.contains("added 50 packages"),
            "summary must survive: {out}"
        );
        assert!(out.len() < input.len(), "must shrink: {out}");
    }

    #[test]
    fn does_not_grow_input_on_unknown_content() {
        let input = "random build output\nno npm-specific patterns\n";
        let out = digest(input);
        assert!(out.len() <= input.len() + 1);
    }
}
