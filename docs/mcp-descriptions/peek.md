Show only the signature and direct callee names for a symbol. No body.

USE THIS when you need the interface, not the implementation.

## When to call
- "What does this function take and return?"
- "Who does this function call?" (one-hop callees only)
- Building a mental model of an API surface without reading bodies.

## Why
A 40-line function body costs ~200 tokens to read. Its signature line + a
callee list costs ~15 tokens. 10–15x savings for the common "I just need
the type signature" case.

## Output shape
```
async function handleLogin(req: LoginReq): Promise<AuthResult>
  throws: AuthError, ValidationError
  calls:  validateToken, db.user.findOne, audit.log
```

## When to escalate
- Need the body — use `pluck.symbol`.
- Need callees-of-callees — use `pluck.expand` with `hop` ≥ 2.
