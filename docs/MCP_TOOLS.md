# MCP Tools

Six tools. All have a `--raw` mode that drops to byte-exact cat/grep behavior
so the agent never loses capability.

## `pluck.read`

Read a code file.

| Mode | Behavior |
|------|----------|
| default | Returns file outline (symbol list + line ranges) + bodies of small symbols inline. ~10x fewer tokens than cat for >300-line files. |
| `lines: "100-200"` | Returns just that line range. |
| `raw: true` | Returns full file, byte-exact with cat. |

Falls back to raw mode automatically for files <100 lines (no win in
outlining a tiny file).

## `pluck.grep`

Keyword search. Wraps ripgrep — every flag passes through.

| Addition over plain rg | |
|------------------------|---|
| Semantic boost | Hits ranked by combined BM25 + semantic similarity (only when called via `pluck.search`; `grep` is pure rg semantics). |
| Output shape | Defaults to one match-line + ±2 context lines per hit, with `file:line:` prefix (`grep -n`). |

## `pluck.search`

Hybrid semantic + keyword search. Natural-language query.

Returns ranked chunks (function/class units), each with:

- `path`
- `symbol`
- `score`
- `start_line` / `end_line`
- `body_preview` (first ~20 lines or full body if small)

Sibling chunks in the same file get a boost. Test files get a penalty unless
the query references tests. Score cutoff at 12% of the top hit drops noise.

## `pluck.symbol`

Read exactly one symbol by name. No file scrolling.

| Input | Resolution |
|-------|------------|
| `handleLogin` | Returns the function body if unambiguous |
| `auth/handleLogin` | Path-qualified for disambiguation |
| `LoginForm` (ambiguous) | Returns a list of candidates with `kind` + `path` |

## `pluck.peek`

Signature + direct callee names only. ~10x cheaper than `pluck.symbol` when
the agent just needs the interface.

Output:

```
async function handleLogin(req: LoginReq): Promise<AuthResult>
  throws: AuthError, ValidationError
  calls:  validateToken, db.user.findOne, audit.log
```

## `pluck.expand`

Symbol + callees up to N hops. Returns the full body for the requested
symbol and signatures for each callee.

| Argument | Default |
|----------|---------|
| `name` | required |
| `hop` | 1 |

Saves round-trips when the agent needs to understand a call chain at a glance.

## Tool description rule

Every tool description text (the field the MCP client shows to the agent
during handshake) must contain four sections:

1. One-line summary
2. **WHEN** to call (concrete triggers)
3. **WHY** (token math, comparison to cat/grep/rg)
4. **WHEN to fall back** to Bash

The verbatim copy lives under `docs/mcp-descriptions/<tool>.md` and is
included into the binary at compile time via `include_str!`.
