//! Build / test / CI output digestion.
//!
//! Stateless transformer that takes verbose tool output (`cargo build`,
//! `pnpm install`, `pytest`, GitHub Actions step log) and collapses
//! progress noise to one-line counts while keeping every signal an
//! agent actually needs: file:line:col positions, panic stacks,
//! tracebacks, failed-step bodies.
//!
//! Used by `pluck.digest` MCP tool and `pluck digest` CLI. Has no
//! dependency on the indexer or the daemon — pure text transformer,
//! safe to call from any context.
//!
//! ## Contract
//!
//! 1. **Preserve** every line that names a file:line:col, contains a
//!    panic stack frame / traceback, or sits inside a failed-step
//!    block.
//! 2. **Collapse** progress lines, "Compiling …" spam, "ok / passed"
//!    rows for green builds. Replace with a one-line summary like
//!    `Compiled 218 crates`.
//! 3. **Pass-through** for formats we don't recognize — never lose
//!    bytes silently.
//!
//! ## Format detection
//!
//! Detection looks at the first ~50 lines for leading markers
//! ("Compiling " + crate-name pattern → cargo; "[ /] " spinner +
//! "added N packages" → npm-family; "============ test session
//! starts ============" → pytest; "##[group]" / "##[endgroup]" →
//! GitHub Actions). If nothing matches, output is returned verbatim
//! with `Format::Unknown`.

mod format;
mod handlers;

pub use format::{detect, Format};

/// Compress one chunk of tool output into a digested form.
///
/// `format` is optional: when `None`, the format is auto-detected
/// from `input`'s leading lines. When detection fails, the input
/// is returned verbatim (lossless fallback).
///
/// Returns the compressed string + the detected/used format so the
/// caller can log which handler ran. Total byte cost of the response
/// is always `<= input.len()` by construction.
pub fn digest(input: &str, format: Option<Format>) -> DigestOutput {
    let fmt = format.unwrap_or_else(|| detect(input));
    let text = match fmt {
        Format::Cargo => handlers::cargo::digest(input),
        Format::NpmFamily => handlers::npm::digest(input),
        // pytest / GHA handlers land in subsequent commits.
        // Until then, those known formats pass through verbatim so
        // CLI/MCP wiring already accepts them as valid format names.
        Format::Pytest | Format::GitHubActions | Format::Unknown => input.to_string(),
    };
    DigestOutput {
        format: fmt,
        text,
        input_bytes: input.len(),
    }
}

/// Result of [`digest`] — compressed text + provenance.
#[derive(Debug, Clone)]
pub struct DigestOutput {
    pub format: Format,
    pub text: String,
    pub input_bytes: usize,
}

impl DigestOutput {
    /// Byte savings as a fraction in `0.0 ..= 1.0`. `0.0` = no
    /// reduction, `1.0` = empty output.
    pub fn savings_fraction(&self) -> f32 {
        if self.input_bytes == 0 {
            return 0.0;
        }
        let saved = self.input_bytes.saturating_sub(self.text.len()) as f32;
        saved / self.input_bytes as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_format_passes_through_verbatim() {
        let input = "some random text\nthat doesn't match any format\n";
        let out = digest(input, None);
        assert_eq!(out.format, Format::Unknown);
        assert_eq!(out.text, input);
        assert!((out.savings_fraction() - 0.0).abs() < 1e-6);
    }

    #[test]
    fn empty_input_zero_savings() {
        let out = digest("", None);
        assert_eq!(out.input_bytes, 0);
        assert_eq!(out.savings_fraction(), 0.0);
    }

    #[test]
    fn savings_fraction_is_one_when_text_is_empty() {
        let mut out = digest("anything", Some(Format::Unknown));
        out.text = String::new();
        assert!((out.savings_fraction() - 1.0).abs() < 1e-6);
    }
}
