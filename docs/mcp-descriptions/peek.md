Signature + direct-callee names for a symbol. No body.

**Use this when you need the interface, not the implementation.** It
is the cheapest way to learn what a function takes / returns and what
it depends on — strictly fewer tokens than `pluck.symbol` for the same
question.

## When to call

- "What does this function take and return?"
- "Who does this function call?" (one-hop callees only)
- Building a mental model of an API surface without reading bodies.
- Refactor planning: who calls whom inside this module.

## Output shape

```
src/auth/login.ts:L45-89  handleLogin (Function)
async function handleLogin(req: LoginRequest): Promise<AuthResult>
  calls: validateToken, db.user.findOne, audit.log
```

## Token math

A 40-line function body costs ~200 tokens via `pluck.symbol`.
Signature + callee list costs ~15. **~10× cheaper** for "I just need
the interface" tasks.

## When to escalate

- Need the body itself — `pluck.symbol`.
- Need callees-of-callees — `pluck.expand` with `hop >= 2`.

## Name resolution

Same `<path>/<name>` disambiguation as `pluck.symbol`. Ambiguous bare
names return a candidate list.
