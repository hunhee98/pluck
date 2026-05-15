Reverse call graph: find who calls a symbol.

## WHEN

Use `pluck.impact` before changing, removing, or refactoring a public
function, method, or type. `depth=1` shows direct callers; `depth=2..3`
widens to callers of callers.

## WHY

It returns the upstream blast radius in one call, with production
callers before test callers. The agent avoids grepping a symbol name,
opening each file, and repeating that loop transitively.

## FALLBACK

Use `pluck.grep` when the symbol is not indexed or you need textual
mentions instead of callers. Use bash only across repositories, outside
the repo, or when the daemon is unreachable.
