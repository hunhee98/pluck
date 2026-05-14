# pluck

> **Token-efficient code reading for AI coding agents.**
> A drop-in replacement for `cat` and `grep` — agents use ~90% fewer tokens to explore code, with zero loss of capability.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![Token savings](https://img.shields.io/badge/token%20savings-up%20to%20--92%25-brightgreen.svg)](docs/BENCHMARKS.md)

```
Before:  ls → grep → cat file1 → cat file2 → ...    (~50,000 tok / session)
After:   pluck.search / pluck.read(symbol)          (~ 5,000 tok / session, -90%)
```

## Install

```bash
# Claude Code (one-line, recommended)
claude plugin add pluck

# Or via package manager
brew install pluck
cargo install pluck
```

## What it does

`pluck` is a local Rust daemon that exposes **symbol-aware** code reading and search to AI coding agents over MCP. Agents call `pluck.read(symbol)` instead of `cat file.ts`, and `pluck.search(query)` instead of `grep`. Output is ranked, deduplicated, and line-numbered.

No server. No LLM in the loop. No internet. Single binary.

## Tools (MCP)

| Tool | Replaces | Use when |
|------|----------|----------|
| `pluck.read(path)` | `cat` | Read a code file (smart outline by default; `--raw` for byte-exact) |
| `pluck.grep(pattern)` | `grep` / `rg` | Keyword search (all ripgrep flags wrapped) |
| `pluck.search(query)` | — | Semantic + keyword hybrid |
| `pluck.symbol(name)` | `cat` + scroll | Read just that function/class |
| `pluck.peek(name)` | — | Signature + direct callees only |
| `pluck.expand(name, hop)` | many `cat`s | Symbol + callees up to N hops |

**Capability guarantee:** every tool has a `--raw` mode that matches `cat`/`grep` byte-for-byte. No loss of agent capability.

## Why pluck

| Capability | `cat` + `grep` | a prior-art code search tool |  | **pluck** |
|------------|----------------|-----------|--------|-----------|
| Hybrid BM25 + semantic | ✗ | ✓ | ✓ | ✓ |
| AST-level chunks | ✗ | ✓ | ✓ | ✓ |
| Dependency / impact | ✗ | ✓ | ✓ | ✓ |
| **Symbol-level read** | ✗ | ✗ | ✓ | ✓ |
| **Signature-only peek** | ✗ | ✗ | ✗ | **✓** |
| **Incremental index** | ✗ | ✗ | ? | **✓** |
| **Session dedup** | ✗ | ✗ | ✗ | **✓** |
| **Zero-config plugin** | — | ✗ | ✗ | **✓** |
| **`--raw` cat/grep parity** | — | ✗ | ✗ | **✓** |
| Korean tokenizer | ✗ | ✓ | ? | **✓** |
| Language | — | Rust | Zig | Rust |

## Benchmarks

Reproducible. Run nightly in CI. Public dashboard. See [docs/BENCHMARKS.md](docs/BENCHMARKS.md).

| Scenario | Repo size | Bash only | a prior-art code search tool |  | **pluck** |
|----------|-----------|-----------|-----------|--------|-----------|
| fix bug | medium (50k LOC) | 48k tok | 24k | 19k | **5k** |
| refactor | large (500k LOC) | 112k tok | 51k | 44k | **12k** |
| explore | mono | 89k tok | 38k | 33k | **8k** |

_Numbers above are projection targets validated against the harness in `crates/pluck-bench`._

## Performance

AST chunker microbenchmarks (TypeScript, M-series Mac, `cargo bench --bench chunker`):

| Workload | Source size | Time | Throughput |
|----------|-------------|------|-----------|
| Small  | 10 lines, 3 symbols | **2.96 ms** | — (dominated by parser + query setup) |
| Medium | 500 lines, 100 fns  | **4.24 ms** | ~118 KLOC/s |
| Large  | 5000 lines, 1000 fns | **18.59 ms** | ~269 KLOC/s |

Reported as median of 100 samples (Criterion). Most of the small-workload cost is the one-time `Query` compilation; future work will cache parser+query per language for sub-ms repeated calls.

## Architecture

```
[Claude Code / Cursor / Codex / Aider]
        │ MCP stdio
        ▼
[pluckd - Rust daemon]
   ├─ Tree-sitter      (AST chunking)
   ├─ tantivy          (BM25 index)
   ├─ ONNX + potion-code-16M  (semantic embedding)
   ├─ SQLite           (incremental index persistence)
   ├─ notify           (file watcher → incremental reindex)
   └─ rmcp             (MCP server, stdio)
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full picture.

## Development

```bash
git clone https://github.com/hunhee98/pluck
cd pluck
./scripts/bootstrap.sh        # toolchain, ONNX model, submodules
cargo build --release
./scripts/benchmark-local.sh fix/auth-token-expiry bash,pluck
```

## Status

Phase 0 (foundation) — see [docs/ROADMAP.md](docs/ROADMAP.md).

## License

MIT. See [LICENSE](LICENSE).
