//! Format detection for [`super::digest`].
//!
//! Looks at the first ~50 lines of the input for leading markers
//! that uniquely identify a tool's output stream. Detection is
//! deliberately conservative: ambiguous input returns
//! [`Format::Unknown`] so the handler pass-through preserves bytes
//! verbatim rather than mis-compressing.

/// Recognized output formats. Add a variant + a detect_*() helper
/// when wiring a new handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `cargo build` / `cargo test` / `cargo check` output.
    Cargo,
    /// `npm` / `pnpm` / `yarn` / `bun` install + script output.
    NpmFamily,
    /// `pytest` test runner output.
    Pytest,
    /// GitHub Actions step / job log (with `##[group]` / `##[error]`
    /// directives).
    GitHubActions,
    /// Fell through every detector — handler will pass through
    /// the input verbatim.
    Unknown,
}

impl Format {
    /// Stable string name, used for `--format <name>` CLI override
    /// and `--show-format` debug output. Lowercase, no spaces.
    pub fn name(self) -> &'static str {
        match self {
            Format::Cargo => "cargo",
            Format::NpmFamily => "npm",
            Format::Pytest => "pytest",
            Format::GitHubActions => "ci",
            Format::Unknown => "unknown",
        }
    }

    /// Parse the user-facing `--format <name>` argument. Unknown
    /// names map to `None` so the CLI can emit a clear diagnostic.
    pub fn parse_name(s: &str) -> Option<Format> {
        match s.to_ascii_lowercase().as_str() {
            "cargo" => Some(Format::Cargo),
            "npm" | "pnpm" | "yarn" | "bun" => Some(Format::NpmFamily),
            "pytest" => Some(Format::Pytest),
            "ci" | "gha" | "actions" => Some(Format::GitHubActions),
            _ => None,
        }
    }
}

/// Auto-detect the format of `input`. Looks at the first ~50 lines
/// (or 8 KB, whichever comes first) for leading markers.
///
/// Returns [`Format::Unknown`] if no marker matches; the digest
/// handler will then pass the input through verbatim.
pub fn detect(input: &str) -> Format {
    // Cheap header scan: stop after a small prefix so giant logs
    // don't iterate every line just to decide format.
    const MAX_BYTES: usize = 8 * 1024;
    const MAX_LINES: usize = 50;

    let header_bytes = input.len().min(MAX_BYTES);
    let header = &input[..header_bytes];

    let mut compiling_count = 0usize;
    let mut npm_marker = false;
    let mut pytest_marker = false;
    let mut gha_marker = false;

    for (i, line) in header.lines().enumerate() {
        if i >= MAX_LINES {
            break;
        }

        // GHA wins early — its directives are unambiguous and may
        // wrap any of the other formats inside group blocks.
        if line.starts_with("##[group]")
            || line.starts_with("##[endgroup]")
            || line.starts_with("##[error]")
            || line.starts_with("##[warning]")
        {
            gha_marker = true;
        }

        // Cargo prints `   Compiling <crate> v<ver>` rows. One line
        // could be coincidence; ≥ 2 is a strong signal.
        let trimmed = line.trim_start();
        if trimmed.starts_with("Compiling ") || trimmed.starts_with("Checking ") {
            compiling_count += 1;
        }

        // npm-family install spinner / "added N packages in …"
        // header. Either alone is reasonably specific.
        if line.contains("added ") && line.contains(" packages") {
            npm_marker = true;
        }
        if line.starts_with("npm warn ") || line.starts_with("npm notice ") {
            npm_marker = true;
        }
        if line.starts_with("Progress: resolved") || line.starts_with("Lockfile is up to date") {
            // pnpm / yarn family
            npm_marker = true;
        }

        // pytest session banner.
        if line.contains("test session starts") && line.contains("==") {
            pytest_marker = true;
        }
        if line.starts_with("platform ") && line.contains("python") && line.contains("pytest") {
            pytest_marker = true;
        }
    }

    if gha_marker {
        return Format::GitHubActions;
    }
    if compiling_count >= 2 {
        return Format::Cargo;
    }
    if pytest_marker {
        return Format::Pytest;
    }
    if npm_marker {
        return Format::NpmFamily;
    }
    Format::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_for_arbitrary_text() {
        assert_eq!(
            detect("hello world\nnothing to see here\n"),
            Format::Unknown
        );
    }

    #[test]
    fn cargo_detected_from_two_compiling_lines() {
        let input =
            "   Compiling serde v1.0.0\n   Compiling tokio v1.30.0\n    Finished `dev` profile\n";
        assert_eq!(detect(input), Format::Cargo);
    }

    #[test]
    fn cargo_not_detected_from_one_lookalike_line() {
        // A single "Compiling foo" mention in unrelated text shouldn't
        // tip the scale.
        let input = "build started\n   Compiling app\nrunning tests\n";
        assert_eq!(detect(input), Format::Unknown);
    }

    #[test]
    fn npm_detected_from_added_packages_line() {
        let input =
            "yarn install v1.22.0\n[1/4] Resolving packages...\nadded 248 packages in 5.2s\n";
        assert_eq!(detect(input), Format::NpmFamily);
    }

    #[test]
    fn npm_detected_from_pnpm_progress_line() {
        let input = "Progress: resolved 134, reused 134, downloaded 0, added 0\n";
        assert_eq!(detect(input), Format::NpmFamily);
    }

    #[test]
    fn pytest_detected_from_session_banner() {
        let input = "============ test session starts ============\nplatform darwin -- Python 3.11.0, pytest-7.4.0\n";
        assert_eq!(detect(input), Format::Pytest);
    }

    #[test]
    fn github_actions_beats_other_markers() {
        // Even if the inner content looks like cargo output, the
        // ##[group] directive means this is a GHA log wrapping cargo.
        // The GHA handler can recurse into inner content later.
        let input = "##[group]Build crate\n   Compiling serde\n   Compiling tokio\n##[endgroup]\n";
        assert_eq!(detect(input), Format::GitHubActions);
    }

    #[test]
    fn parse_name_round_trip() {
        for fmt in [
            Format::Cargo,
            Format::NpmFamily,
            Format::Pytest,
            Format::GitHubActions,
        ] {
            assert_eq!(Format::parse_name(fmt.name()), Some(fmt));
        }
    }

    #[test]
    fn parse_name_accepts_aliases() {
        assert_eq!(Format::parse_name("yarn"), Some(Format::NpmFamily));
        assert_eq!(Format::parse_name("bun"), Some(Format::NpmFamily));
        assert_eq!(Format::parse_name("gha"), Some(Format::GitHubActions));
        assert_eq!(Format::parse_name("actions"), Some(Format::GitHubActions));
    }

    #[test]
    fn parse_name_rejects_unknown() {
        assert_eq!(Format::parse_name("rustc"), None);
        assert_eq!(Format::parse_name(""), None);
    }
}
