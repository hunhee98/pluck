File-level import graph: dependencies or importers for one file.

## WHEN

Use `pluck.deps` when the question is about module edges, not symbol
bodies. Default mode lists imports from `path`; `reverse: true` lists
files that import `path`.

## WHY

It resolves common JS/TS, Python, Rust, and Go imports against indexed
repo files and buckets unresolved externals. The agent sees layering
and deletion/refactor blast radius without scanning import lines by
hand.

## FALLBACK

Use `pluck.impact` for symbol-level callers. Use `pluck.grep` or bash
when the file is outside the repo, the import syntax is unsupported, or
the daemon is unreachable.
