Symbol body + signatures of callees up to N hops deep.

**Use this to grasp a call chain in one tool call instead of N.** It
is the default substitute for `pluck.symbol(X)` followed by
`pluck.peek(callee1)`, `pluck.peek(callee2)`, … — same information
arrives in a single response with a structural rendering.

## When to call

- Need to understand the immediate context around a function: what it
  does and what one or two hops of its call graph look like.
- Refactor / impact analysis on a single entry point.
- Avoiding the pattern: `pluck.symbol(X)` → `pluck.peek(callee1)` →
  `pluck.peek(callee2)` → … the exact case `expand` was built for.

## Output shape

```
src/auth/login.ts:L45-89  handleLogin (Function)
async function handleLogin(req: LoginRequest): Promise<AuthResult> { … full body … }

=== hop 1 ===
  → src/auth/token.ts:L12-30   validateToken (Function)
      async function validateToken(token: string): Promise<TokenClaims>
        calls: jwt.verify, db.tokens.findOne
  → src/db/users.ts:L100-120   findOne (Method)
      async findOne(id: string): Promise<User | null>
        calls: this.query
  · audit.log  (external / not indexed)

[expanded 2 callees across 1 hop(s)]
```

## Arguments

- `name` — symbol to expand (required).
- `hop` — call-graph depth (default 1, max 3).

## Limits and safety

- Per-hop cap of 30 callees; oversize hops emit a "+ N more
  suppressed" footer with a hint to drill into specific branches.
- Cycle-safe: a callee already expanded earlier in the response is
  emitted as a one-line "already expanded above" pointer.
- External callees (not indexed) appear as `· name  (external / not
  indexed)` — the agent knows we saw them and chose not to follow.
- Session dedup applies to every chunk surfaced in the response, so a
  subsequent `pluck.search` returning any of the same chunks
  collapses to placeholders.

## When to fall back

- Need only the signature, not the body — `pluck.peek`.
- Need only the body, no callees — `pluck.symbol`.
- The chain is multi-hop and N > 3 — split into multiple `pluck.expand`
  calls at different roots.
