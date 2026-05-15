Read a file from the indexed repo.

**Use this for every file read inside the repo.** It is the default
substitute for `cat`, `Read`, and similar built-in file-read tools.
For small files the response is byte-identical to `cat`; for larger
files (>100 lines) it returns a symbol outline that uses ~10× fewer
tokens for the same recall. Either way, the agent has zero capability
loss compared to bash — the underlying bytes are reachable via
`raw: true` or `lines: "A-B"`.

## When to call

- Any time you would have called `cat` / `Read` on a file in the repo.
- After locating a candidate file via `pluck.search` or `pluck.grep`.
- Before editing a file, to learn the surrounding structure.

## Modes

- default — outline (one line per symbol with its signature and line
  range). Best for files >100 lines. Bodies are not duplicated; fetch
  them with `pluck.symbol` or `pluck.read --lines`.
- `raw: true` — full file contents, byte-equivalent to `cat`.
- `lines: "A-B"` — inclusive line range, like `sed -n A,Bp`.

## When to fall back to bash

- The file is binary (you'll see a clear "not valid UTF-8" diagnostic).
- The file is larger than 4 MB (you'll see a size-cap diagnostic with
  a suggested `lines:` range).
- The target is outside the indexed repo and you need raw bytes.

Otherwise: default to `pluck.read`. It is strictly cheaper than `cat`
on any file the indexer can see, and its outline mode is the cheapest
way for an agent to learn what a file contains.
