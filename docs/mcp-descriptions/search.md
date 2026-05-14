Hybrid semantic + keyword search across the repo.

USE THIS when you're looking for code by intent rather than by literal name.
Example: "where is the JWT refresh handled?" rather than "find `refreshToken`".

## When to call
- Locate code by capability or purpose ("auth", "rate limiting", "image upload").
- You don't know the exact symbol name yet.
- First step of an exploration task — gives you ranked entry points.

## Why
BM25 + semantic embedding fused with RRF. Returns ranked AST chunks
(function/class units) with line numbers, so the agent doesn't need a
follow-up `cat`. Sibling chunks in the same file get a boost, so long files
don't get missed. Top results past the noise threshold (12% of top score)
are dropped.

## When to fall back to `pluck.grep`
- You already know the literal string or regex.
- You need every match, not the most relevant ones.
