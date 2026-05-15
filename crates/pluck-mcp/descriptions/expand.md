Expand one symbol into its body plus callee signatures up to a bounded
hop depth.

## WHEN

Use `pluck.expand` to understand a call chain from one entry point:
what the root does, what it calls, and what one to three hops look like.
Set `hop` from 1 to 3.

## WHY

It replaces a loop of `symbol` and repeated `peek` calls with one
structured response. Cycles and large hops are capped, and external
callees are marked instead of silently ignored.

## FALLBACK

Use `pluck.peek` for signature-only questions, `pluck.symbol` for only
the root body, split into multiple expands when depth exceeds 3, and
bash only outside the repo or when the daemon is unreachable.
