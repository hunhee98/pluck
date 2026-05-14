//! Drives a single scenario × runner combination.
//!
//! 1. Prepare the target repo (clone / checkout revision)
//! 2. Spin up Claude Code session via Agent SDK (or subprocess)
//! 3. Send scenario prompt
//! 4. Capture tool calls + tokens + diff
//! 5. Score against success criteria
//! 6. Write results JSON
//!
//! TODO: implement.

pub async fn run(
    _scenario: &str,
    _runner: &str,
    _repetitions: u32,
    _output: &str,
) -> anyhow::Result<()> {
    eprintln!("pluck-bench: run — not implemented yet");
    Ok(())
}
