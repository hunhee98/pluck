Reverse call-graph traversal: "who calls this function, transitively?"

## WHEN to call

Call `pluck.impact` when you need the **upstream blast radius** of a
change: which callers will break or need updating if `validate_token`,
`parseConfig`, or `UserModel` changes its contract.

- Before refactoring a public function — see every direct caller at
  `depth=1` (default), then widen to `depth=2` for transitive callers.
- Before changing a struct field — pass the field's parent struct name
  to find every method that reads or writes it.
- Before removing a symbol — confirm no production caller exists before
  deleting.

## WHY it saves tokens

Without `pluck.impact`, agents grep for the symbol name, open each
caller file, read it, and repeat transitively. A widely-used utility
function can touch 20+ files. `pluck.impact` returns the full upstream
tree in one call, with test-file callers sorted to the bottom so the
agent sees production impact first.

## Parameters

- `name` — the symbol whose callers you want. Leaf-matched and
  case-insensitive: `validate_token`, `Logger::new`, `db.insert` all
  work.
- `depth` — BFS depth cap (default 1, max 3). `depth=1` = direct
  callers; `depth=2` = callers-of-callers. Beyond 3 the output
  explodes; narrow with a path-qualified name instead.

## Output shape

```
=== impact: validate_token (depth 1) — 3 caller(s) ===

[depth 1]  src/handler.rs:L12-25  handle_request (Function)
pub fn handle_request(token: &str) -> bool {
    validate_token(token)
}

[depth 1]  src/middleware.rs:L8-19  auth_middleware (Function)
...

[test callers — 1]
[depth 1]  tests/auth_test.rs:L5-18  test_validate_token (Function)
...
```

## FALLBACK to bash when

- The symbol is not indexed (external crate, stdlib, generated code) —
  `pluck.impact` returns an empty result; fall back to `rg <name>`.
- You need callers across repositories — the index is per-repo.
- The daemon is unreachable — fall back to `rg -l <name>`.
