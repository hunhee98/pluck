Keyword / regex search across files. Wraps ripgrep — every `rg` flag
passes through verbatim.

**Use this for every keyword grep inside the repo.** It is the default
substitute for `grep -rn`, `rg`, and built-in Grep tools. The
underlying engine is the same ripgrep your shell uses; the win is
that you skip the bash fork/exec round-trip, and the daemon keeps
its file cache warm across calls.

## When to call

- Any time you would run `rg <pattern>` or `grep -rn <pattern>`.
- Find every occurrence of a literal string or regex.
- Find every location a known symbol name appears (use `pluck.symbol`
  instead if you want only the definition; this tool gives every
  match including call sites and comments).

## Modes

- default `pattern` — literal substring, like `rg <pattern>`.
- `args: [...]` — every ripgrep flag works. Examples:
  `args: ["-A", "5"]`  — 5 lines of trailing context.
  `args: ["--type", "ts"]`  — restrict to TypeScript.
  `args: ["-e", "<regex>"]` — regex form.
- `cwd: "<path>"` — grep in a subdirectory or absolute path instead
  of the repo root.

## When to fall back to bash

- The target is outside the indexed repo and you cannot pass `cwd`.
- You need to pipe the output into another shell tool inline.
- ripgrep is not installed (you'll see a clear "is `rg` on PATH?"
  diagnostic).

Otherwise: default to `pluck.grep`. It is strictly at least as
efficient as `rg` from bash, and saves the agent the fork/exec
overhead on every call.
