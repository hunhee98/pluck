---
name: Bug report
about: pluck did something wrong, crashed, or returned the wrong thing
title: "fix: <one-line summary>"
labels: bug
---

## What happened

<!-- One or two sentences. Include the actual symptom — error message,
empty output, wrong hit, panic, hang, etc. -->

## Expected

<!-- What you thought would happen instead. -->

## How to reproduce

<!-- Smallest steps that trigger the bug. Paste the exact command +
the input that goes with it. -->

```bash
# example
pluck index .
pluck search "auth flow"
```

## Environment

- `pluck --version`:
- `pluckd --version`:
- OS + arch (e.g. macOS 14 / arm64, Ubuntu 22.04 / x86_64):
- Rust toolchain (`rustc --version`) if you built from source:
- Agent harness, if relevant (Claude Code / Codex / etc.):

## Logs

<!-- If pluck or pluckd produced log output, paste it here. For
verbose tracing: `RUST_LOG=pluck=debug pluckd …`. Trim to the
relevant lines. -->

<details>
<summary>tracing output</summary>

```
(paste here)
```
</details>

## Suspicion (optional)

<!-- If you have a guess where the bug is — a file, a function, a
specific code path — say so. Helps narrow the search. -->
