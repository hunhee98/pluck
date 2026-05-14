Read exactly one symbol (function, class, struct, method) by name.

USE THIS instead of `cat`-ing the whole file when you only need that one
symbol's body.

## When to call
- You found a function name via `pluck.search` or `pluck.grep` and want its
  body without reading the surrounding file.
- You know the symbol name and want a precise, minimal read.

## Why
For a 800-line file containing a 40-line function, `cat` costs ~6000 tokens
and `pluck.symbol` costs ~300. Same information for the task, 20x less budget.

## Disambiguation
- `handleLogin` — returns body if unambiguous.
- `auth/handleLogin` — path-qualified form for collisions.
- Ambiguous name returns a candidate list (`kind`, `path`) — pick one and
  re-call with the path-qualified form.

## Fallback
If the symbol isn't in the index (e.g. it's a string literal, not a defined
symbol), `pluck.symbol` automatically falls back to a `pluck.grep` for the
name.
