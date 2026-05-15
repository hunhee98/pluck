File-level import graph: "what does this file depend on?" or the
reverse — "who imports this file?".

## WHEN to call

Call `pluck.deps` whenever you need the **edges around a file**, not
the symbols inside it. `pluck.impact` answers it at the symbol level
(callers of a function); `deps` answers it at the file level (modules
imported by `auth/login.ts`, or every file that imports
`crypto/jwt.ts`).

- Before editing a module's public surface — see every file that
  imports it (`reverse: true`).
- Mapping an unfamiliar repo — start at the entrypoint and walk
  forward edges to learn the layering without reading bodies.
- Cleaning up dead code — `importers` returns empty → the file is
  a candidate for deletion.

## WHY it saves tokens

Without `pluck.deps`, the agent runs `grep -r 'import.*auth/login'`,
opens every matching file, scans imports by eye, repeats per direction.
`deps` returns the resolved graph in one call: every edge with a
best-effort path resolution, externals bucketed at the bottom.

## Parameters

- `path` — repo-relative file path (absolute paths also work).
- `reverse` — when true, return importers of `path` instead of deps
  out of it. Default false.

## Output shape

```
=== deps of: src/auth/login.ts — 3 edge(s) ===

../crypto/jwt -> src/crypto/jwt.ts
../db/users -> src/db/users.ts
./schema -> src/auth/schema.ts

[external — 2 edge(s), no in-repo match]
zod
jsonwebtoken
```

Reverse mode lists importer paths directly:

```
=== importers of: src/crypto/jwt.ts — 2 edge(s) ===

src/admin/panel.ts
src/auth/login.ts
```

## Resolution rules

- Relative imports (`./foo`, `../bar`) resolve by joining with the
  importer's directory and trying `.ts/.tsx/.js/.jsx/.py/.go/.rs` plus
  `/index.{ts,tsx,js,jsx}` and `/__init__.py`.
- Rust `use crate::a::b::c` resolves via suffix match on `/a/b/c.rs`.
- Python dotted imports (`from foo.bar import baz`) resolve to
  `**/foo/bar.py` or `**/foo/bar/__init__.py`.
- Anything that doesn't match is returned in the `external` bucket —
  stdlib, third-party packages, or cross-repo imports.

## FALLBACK to bash when

- The file is outside the indexed repo — `deps` returns empty; use
  `rg "import .* path/to/file"`.
- You need symbol-level granularity (which *function* uses this
  import) — switch to `pluck.impact` or `pluck.expand`.
- The daemon is unreachable — `rg -l "from foo"` finds importers
  textually; cheaper than booting a Python AST tool.
