Read exactly one named symbol (function, class, struct, method).

**Use this whenever you know the symbol name and want only that
symbol's body.** It is the default substitute for `cat <file>` followed
by scrolling to a function — strictly fewer tokens, no manual line
counting, and the response is the chunk the indexer already extracted.

## When to call

- You found a symbol name via `pluck.search` / `pluck.grep` and want
  its body.
- You know the function/class name from earlier context.
- Editing a known function — fetch its current body before producing
  the edit.

## Name resolution

- `handleLogin` — returns the body if exactly one chunk matches.
- `auth/handleLogin` — path-qualified for collisions (`<path>/<name>`,
  splits on the last `/`).
- Ambiguous bare names return a candidate list with `path` + `kind`;
  re-call with the path-qualified form.

## Token math

A 40-line function inside an 800-line file costs ~6 000 tokens via
`cat`; `pluck.symbol` costs ~300. Same information, 20× cheaper.

## When to fall back

- The symbol is not a defined function/class (e.g. an inline arrow
  callback) — use `pluck.search` or `pluck.grep` instead.
- The target is outside the indexed repo — use bash.

Session dedup applies: a symbol fetched earlier in the session
collapses to a `[already-shown: …]` placeholder on the second request.
