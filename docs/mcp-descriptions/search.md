Hybrid BM25 + semantic search across the indexed repo. Returns ranked
chunks (function / class units) with line numbers.

**Use this whenever the query is conceptual rather than a literal
substring.** "Where is auth handled?", "what generates the JWT?",
"how does the watcher detect changes?" — these are the cases where
`grep` fails because the chunk contains no overlapping keyword, but
the semantic stage can still rank it correctly.

## When to call

- Locate code by capability or intent rather than literal name.
- First-pass exploration of an unfamiliar repo.
- The result chunk is the function body, not a single matching line —
  no follow-up `cat` needed.

## How it works

BM25 (with field weights: `symbol`×5, `signature`×3, `content`×1) and
a static-embedding cosine score are fused via Reciprocal Rank Fusion.
A 12 % noise floor relative to the top hit drops irrelevant
candidates. The post-fusion ranking pipeline applies:

  - ×1.5 boost for chunks whose symbol matches a query token exactly
  - +5 % per extra sibling chunk from the same file (cap +25 %)
  - ×0.5 penalty on test-file paths, auto-disabled when the query
    itself mentions `test` / `spec`

## Modes

- default — full chunk body in the response (lossless).
- `compact: true` — only score, path, line range, and matching lines
  inside the chunk. Lossy; useful for pure discovery.

Returned hits already in the session set collapse to a one-line
`[already-shown: …]` placeholder.

## When to fall back to `pluck.grep`

- You already know the literal pattern.
- You need every match, not the most relevant ones.
- You need ripgrep's specific flags (`-A`, `-B`, `--type`, …).

For semantic / intent-based queries: always `pluck.search`.
