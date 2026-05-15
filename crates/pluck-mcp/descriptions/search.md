Hybrid BM25 + semantic search over indexed code chunks.

## WHEN

Use `pluck.search` for conceptual lookup: "where is auth handled?",
"what generates the token?", or any query where you do not already know
the exact string. Use `compact: true` for discovery-only results.

## WHY

Search returns ranked function/class chunks with paths and line ranges,
so the agent can inspect the right code directly instead of guessing
identifiers and opening whole files. Session dedup collapses repeated
chunks to placeholders.

## FALLBACK

Use `pluck.grep` when you already know the literal pattern, need every
match, or need ripgrep flags. Use bash only for paths outside the repo
or when the daemon is unreachable.
