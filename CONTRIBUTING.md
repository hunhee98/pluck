# Contributing to pluck

## Quick start

```bash
./scripts/bootstrap.sh      # toolchain + ONNX model + submodules
cargo build --release
cargo test
```

## Workspace layout

| Crate | Responsibility |
|-------|----------------|
| `pluck-core` | Indexing, AST chunking, BM25, semantic, SQLite, file watcher |
| `pluck-mcp` | MCP server (rmcp) — tool implementations, session dedup |
| `pluck-cli` | Standalone CLI (`pluck`, `pluckd`) |
| `pluck-bench` | Benchmark harness — driver, scoring, report |

## Style

- `cargo fmt` before commit
- `cargo clippy -- -D warnings` must pass
- Public APIs documented with rustdoc
- New MCP tools require an entry in [docs/MCP_TOOLS.md](docs/MCP_TOOLS.md)
- New tool descriptions are evaluated in `pluck-bench` — capability loss = revert

## Pull request checklist

- [ ] `cargo build --release` passes
- [ ] `cargo test` passes
- [ ] `cargo clippy` clean
- [ ] If a new tool: description carries WHEN / WHY / COMPARISON sections
- [ ] If touching ranking / chunking: benchmark numbers attached
- [ ] CHANGELOG entry under `[Unreleased]`

## Benchmarks

Anything that affects token output must show a benchmark delta:

```bash
./scripts/benchmark-local.sh fix/auth-token-expiry bash,pluck
```

Negative regressions on any scenario block the PR unless justified in the description.
