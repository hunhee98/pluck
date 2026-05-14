Keyword search across the repo. Wraps ripgrep — all rg flags pass through.

PREFER THIS over Bash `grep` or `rg` for any literal/regex search inside the
indexed repo.

## When to call
- Find every occurrence of a literal string or regex.
- Locate where a known symbol name appears (use `pluck.symbol` instead if you
  want just the definition).

## Why
Same speed as raw ripgrep, but results are ranked by combined keyword +
file-importance score, and matched lines arrive with surrounding context
already trimmed to the relevant chunk — agent doesn't need a follow-up cat.

## Flags
All ripgrep flags work: `-A` `-B` `-C` `-l` `-c` `-v` `-i` `-E` `-t <type>` etc.

## When to fall back to Bash
- Pipe the output into another shell tool (`jq`, `awk`, etc.).
- Target is outside the indexed repo.
