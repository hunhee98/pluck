Symbol body + signatures of callees up to N hops away.

USE THIS to grasp a call chain in one tool call instead of many.

## When to call
- Need to understand the immediate context around a function: what it does,
  what it depends on, one or two hops out.
- Avoiding the pattern: `pluck.symbol(X)` → `pluck.peek(callee1)` → ...

## Why
A typical 3-call exploration ≈ 1000 input tokens once you include MCP
boilerplate. `pluck.expand(X, hop=1)` returns the same shape in one call
≈ 400 tokens.

## Output shape
```
handleLogin (body, 40 lines)
  └─ validateToken(token: string): TokenClaims
  └─ db.user.findOne(filter): Promise<User | null>
  └─ audit.log(event: AuditEvent): void
```

## Arguments
- `name` — symbol to expand (required)
- `hop` — call-graph depth (default 1, max 3)

## Limits
At `hop >= 2` the output grows fast. The server caps total tokens; truncated
nodes appear as `[...]` and can be expanded with a follow-up call.
