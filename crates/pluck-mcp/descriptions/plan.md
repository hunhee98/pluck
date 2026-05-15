Exploration recommender: given a task, suggest the next pluck calls.

## WHEN

Use `pluck.plan` at the start of an unfamiliar task, vague bug report,
or new repo area. Pass the task text; it returns 3-5 recommended calls
plus confidence.

## WHY

It collapses speculative `search -> read -> search` loops into one
probe. Recommendations choose `read`, `peek`, `symbol`, or `impact`
based on ranked chunks, size, and kind, so the agent starts with the
cheapest useful context.

## FALLBACK

Use `pluck.grep` directly for exact error strings, constants, or known
identifiers. Use bash only outside the repo or when the daemon is
unreachable.
