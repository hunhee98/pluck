# Architecture

## Data flow

```
Source files
    │
    ▼
[Indexer]                                (background daemon)
  ├─ Tree-sitter parse → AST chunks (symbol-level)
  ├─ BM25 index (tantivy)
  ├─ Embedding (ONNX, potion-code-16M)
  └─ Persist (SQLite)
    │
    ▼                                    (file watcher reindexes on change)
[Index store: ~/.pluck/<repo-hash>/]

Agent query
    │
    ▼
[MCP server (pluckd)]
  ├─ Parse tool call
  ├─ Hit index
  ├─ Fuse BM25 + semantic (RRF)
  ├─ Rank + filter (12% noise cutoff)
  ├─ Session dedup (skip chunks already returned this session)
  └─ Return snippet (with line numbers)
```

## Crates

| Crate | Role |
|-------|------|
| `pluck-core` | Indexing, search, AST chunking, persistence, watcher |
| `pluck-mcp` | MCP server (`pluckd` binary), tool handlers, session state |
| `pluck-cli` | Standalone CLI (`pluck` binary) for non-MCP envs |
| `pluck-bench` | Reproducible benchmark harness |

## Index layout (SQLite)

```sql
CREATE TABLE chunks (
  id           INTEGER PRIMARY KEY,
  path         TEXT NOT NULL,
  symbol       TEXT,
  kind         TEXT,        -- function / class / struct / method / ...
  start_line   INTEGER,
  end_line     INTEGER,
  content      TEXT,
  signature    TEXT,
  embedding    BLOB,         -- f32[256]
  file_mtime   INTEGER,
  file_hash    TEXT
);
CREATE INDEX idx_chunks_path   ON chunks(path);
CREATE INDEX idx_chunks_symbol ON chunks(symbol);

CREATE TABLE files (
  path       TEXT PRIMARY KEY,
  mtime      INTEGER,
  hash       TEXT,
  lang       TEXT
);
```

BM25 lives separately in a memory-mapped `tantivy/` directory inside the same
repo-hash folder.

## Incremental update

1. `notify` reports a file change.
2. Hash new content; skip if unchanged.
3. Re-parse with Tree-sitter → diff chunks vs DB.
4. Delete removed chunks, insert added, update changed.
5. Schedule BM25 segment merge on threshold (size or count).
6. Re-embed only changed chunks.

## Session state (MCP)

The MCP server keeps a per-connection map of `chunk_id → returned_at`.
When a tool would return a chunk already shown this session, it elides the
body and emits a one-line placeholder:

```
[already shown: src/auth/login.ts handleLogin L45-L89]
```

The agent already has the content in its context, so this is pure token win.

## Languages (Phase 1)

| Language | Tree-sitter grammar | Test repo |
|----------|---------------------|-----------|
| Rust | `tree-sitter-rust` | `tokio-rs/tokio` (subset) |
| TypeScript / JavaScript | `tree-sitter-typescript` | `vercel/next.js` (subset) |
| Python | `tree-sitter-python` | `pallets/flask` |
| Go | `tree-sitter-go` | `gin-gonic/gin` |

More languages added behind the same chunker interface.

## Why no language server

LSP would give us perfect call graphs, but:

- Heavy install (each language needs its server)
- Slow startup
- Memory hungry
- Cross-language graphs require N×N integration

Tree-sitter gives 85% of the value at 5% of the cost. We can layer LSP later
as an opt-in for users who already run a language server.
