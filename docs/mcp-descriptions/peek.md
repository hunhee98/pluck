Return a symbol's signature plus direct callees, without the body.
Call from the main conversation; sub-agent delegation skips this tool
and falls back to a full file read, losing the body-skip savings.

## WHEN

Use `pluck.peek` when you need the interface: parameters, return type,
and one-hop dependencies. It is ideal for API mapping, refactor
planning, and deciding whether a body read is worth the tokens.

## WHY

Many questions need shape, not implementation. `peek` gives the useful
contract and callee list at a fraction of the cost of `symbol`.

## FALLBACK

Use `pluck.symbol` when you need the body, `pluck.expand` when you need
callee bodies across hops, and bash only outside the repo or when the
daemon is unreachable.
