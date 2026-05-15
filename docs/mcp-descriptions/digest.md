Compress verbose tool output so the agent only sees signals, not noise.

## WHEN to call

Call `pluck.digest` whenever you receive raw output from:
- `cargo build` / `cargo test` / `cargo check` — hundreds of "Compiling …" lines
- `npm install` / `pnpm install` / `yarn install` — progress spinners and phase lines
- `pytest` — per-test PASSED lines on an all-green run
- GitHub Actions step logs — successful step bodies wrapping any of the above

Pass the raw output as `input`. The tool returns the same content with progress
noise collapsed to one-line summaries, leaving every error, failure, traceback,
file:line:col position, and test failure intact.

## WHY it saves tokens

A typical `cargo build` of a mid-size workspace emits 200–400 "Compiling …" lines
before the real signal (error or "Finished"). Digest collapses those to a single
`[cargo] compiled N crates` line. The compressed output contains exactly the
bytes an agent needs to decide what to do next.

## Formats

Auto-detected from the first 50 lines. Override with `format` if detection is
wrong (e.g. cargo output piped through a wrapper that strips the leading lines):

- `cargo` — cargo build / test / check
- `npm` / `pnpm` / `yarn` / `bun` — install + script output
- `pytest` — pytest test runner
- `ci` / `gha` / `actions` — GitHub Actions step log

## FALLBACK to bash when

- The input is already short (< 50 lines) — pass-through saves no tokens.
- The format is not one of the four above — the tool returns the input verbatim,
  so calling it costs a round-trip for zero gain.
- You need byte-exact output (e.g. piping into another tool) — the digested text
  is lossless on signal but drops noise lines.
