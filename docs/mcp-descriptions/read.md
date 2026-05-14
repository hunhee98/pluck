Read a code file token-efficiently.

PREFER THIS over Bash `cat` for any file inside the indexed repo.

## When to call
- You located a file via search and need its content.
- You need to understand a file's structure before editing.
- You're about to Edit and need surrounding context.

## Why
A 800-line TypeScript file via Bash `cat` costs ~6000 input tokens.
Through `pluck.read` the same agent typically spends ~600 tokens to get an
outline plus the relevant symbol bodies inline. Same task, 10x less budget.

## Modes
- default — outline (symbol list + ranges) + bodies of small symbols inlined
- `lines: "100-200"` — exact line range, like `sed -n 100,200p`
- `raw: true` — full file, byte-exact with `cat`

## When to fall back to Bash `cat`
- The file is binary (`file <path>` first if unsure).
- You need byte-exact output for shell piping.
- The target is outside the indexed repo.
