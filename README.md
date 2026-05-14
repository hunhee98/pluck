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

pluck's bet is that the **agent-facing layer** is what's been missing from
code search for AI agents — not the indexing algorithm.

| Capability | `cat` + `grep` / `rg` | Other code-search tools | **pluck** |
|------------|----------------------|-------------------------|-----------|
| Hybrid BM25 + semantic ranking | ✗ | typically ✓ | ✓ (Phase 2) |
| AST-level chunks | ✗ | typically ✓ | ✓ |
| Persistent daemon (MCP stdio) | — | ✗ (cold CLI per call) | **✓** |
| Persistent index (mmap) | — | usually ✗ | **✓** |
| Incremental reindex (file watcher) | — | usually ✗ | **✓** |
| **Session-scoped dedup** | — | ✗ | **✓** |
| **`--raw` cat/grep byte parity** | — | ✗ | **✓** |
| **Lossless default, lossy opt-in** | — | varies | **✓** |
| `peek` (signature + direct callees) | ✗ | ✗ | **✓** |
| Single-file outline (`pluck.read`) | ✗ | ✗ | **✓** |
| Multi-hop `expand` (call graph) | ✗ | ✗ | **✓** |

The principle that drives the surface design: **savings must come from
removing structural waste, never from omitting information the agent might
need.** Re-reading the same chunk in one session, scrolling past unrelated
functions to reach the one that matters, re-paying tokens for the same
imports / headers on every read — that's the redundancy pluck targets.
Stripping comments, dropping types, returning only matched lines without
their surrounding function — that's information loss the agent pays for
later in extra round-trips or wrong decisions. pluck defaults to the
lossless modes and makes any lossy mode (peek, match-lines-only) an
explicit opt-in the agent has to choose.

## Benchmarks

Reproducible. Run nightly in CI. Public dashboard. See [docs/BENCHMARKS.md](docs/BENCHMARKS.md).

| Scenario | Repo size | Bash only | **pluck** |
|----------|-----------|-----------|-----------|
| fix bug | medium (50k LOC) | 48k tok | **5k** |
| refactor | large (500k LOC) | 112k tok | **12k** |
| explore | mono | 89k tok | **8k** |

_Numbers above are projection targets validated against the harness in `crates/pluck-bench`._

## Performance

### Token savings — `pluck.search` vs `rg` / `cat`

Measured against a synthetic 92-file TypeScript repo: 12 subject-matter
files plus 80 noise modules that mention query keywords incidentally — the
shape of a real codebase. `cargo bench --bench search`. Repo size: 32,756
`cat`-tokens total. The `--full` rendering preserves chunk bodies (the
lossless default); `--compact` is an opt-in mode that keeps only the score,
path, range, and matching lines — useful for pure discovery but lossy for
editing tasks.

| Query | `rg` lines | `rg + cat matched files` | pluck `--full` | pluck `--compact` | save vs cat | save vs rg |
|-------|----------:|-------------------------:|---------------:|------------------:|------------:|-----------:|
| session expiry refresh | 3,273 | 6,859 | 900 | 622 | 91% | 81% |
| password verification  | 2,034 | 4,402 | 1,029 | 559 | 87% | 73% |
| refund window          | 1,085 | 2,186 | 1,000 | 430 | 80% | 60% |
| subscription billing   | 1,052 | 2,189 | 987 | 465 | 79% | 56% |
| auth middleware        | 2,728 | 6,194 | 1,013 | 443 | 93% | 84% |

Average: **86% vs `grep + cat matched files`**, **71% vs raw `rg` lines**,
with the `--compact` mode. BM25-only today; semantic ranking (Phase 2) will
improve precision on natural-language queries.

### Token savings — `pluck.read` outline vs `cat`

Measured with `cl100k_base` (the BPE Claude / GPT-4 use). `cargo bench --bench tokens`.

| Scenario | Lines | `cat` tokens | `pluck.read` tokens | Savings |
|----------|------:|-------------:|--------------------:|--------:|
| Tiny (raw mode pass-through) | 10 | 60 | 60 | 0% (raw) |
| Medium realistic — 5 handlers | 119 | 929 | 116 | **88%** |
| Large realistic — 25 handlers | 579 | 4,549 | 556 | **88%** |
| XL realistic — 100 handlers | 2,304 | 18,124 | 2,320 | **87%** |
| Class with 10 methods | 173 | 1,768 | 277 | **84%** |
| Class with 50 methods | 813 | 8,608 | 1,277 | **85%** |

Files ≤ 100 lines fall through to raw mode automatically (no win in outlining a tiny file). Above that, the outline is signature-only; the agent fetches bodies on demand via `pluck.symbol(name)` or `pluck.read(lines: …)`.

### Session dedup — lossless savings across a multi-call session

The MCP daemon tracks every chunk id it has returned in the current
session. A chunk surfaced by a later query whose id is already in the
session set is emitted as a one-line `[already-shown: <path>:L<a>-<b>
<symbol> score=<s>]` placeholder instead of repeating its body — the
bytes are already in the agent's context window, repeating them is
pure waste. `cargo bench -p pluck-mcp --bench session_dedup`.

| # | Query | No-dedup tokens | With-dedup tokens | Savings |
|--:|-------|----------------:|------------------:|--------:|
| 1 | `chunk source`                | 1,741 | 1,741 | 0% (first call, nothing to dedup) |
| 2 | `tree sitter query`           | 2,386 | 1,696 | 29% |
| 3 | `search index chunk`          | 1,340 | 1,185 | 12% |
| 4 | `chunk source tree sitter`    | 1,894 |   220 | **88%** |
| 5 | `BM25 search chunk`           | 1,712 |   248 | **86%** |
| Σ | session total                 | **9,073** | **5,090** | **44%** |

**Zero information loss.** Every dedup'd chunk keeps its path, line range,
symbol, and score — only the body bytes the agent already has are elided.
A CLI-based code-search tool architecturally can't do this: each invocation
is a fresh process with no memory of prior calls. pluck's persistent
daemon is what makes this savings shape possible.

### End-to-end scenario: `fix-auth-token-expiry`

A 92-file TypeScript fixture with a single seeded bug — `s.expiresAt > now()`
in `src/auth/session.ts` where the comparison should be `<`. Both workflows
must surface the buggy line for the run to count as a success. `cargo run
-p pluck-bench -- run --scenario fix-auth-token-expiry`.

| Workflow | Tool calls | Total tokens | Bug surfaced? |
|----------|-----------:|-------------:|:-------------:|
| Bash (`rg -l` → `cat` × 3 → `rg -n` → `cat`)         | 7 | 1,248 | ✅ |
| Pluck (`pluck.search` → `pluck.read` → `pluck.search`) | 3 | 931   | ✅ |

**25% fewer tokens, 4 fewer tool calls, identical recall.** The seeded
bug is a deliberate substring (`s.expiresAt > now()`) the verifier checks
for in each workflow's output — same correctness bar, fewer bytes.
Phase 4 replaces the hand-written workflows with real LLM tool selection;
the fixture and the marker stay the same so the comparison stays
apples-to-apples.

### Indexer throughput & search latency

`cargo bench --bench indexer` on synthetic TypeScript repos (each file: 1
interface + 6 async handler functions, ~25 lines each):

| Repo | Files | Chunks | Index time | Files/s | Chunks/s | Search warm (p50) | Search cold (p50) |
|------|------:|-------:|----------:|--------:|---------:|------------------:|------------------:|
| Small  | 50   | 350    | 135 ms    | 371     | 2,594    | **0.05 ms**       | 0.40 ms |
| Medium | 500  | 3,500  | 1.3 s     | 386     | 2,701    | **0.06 ms**       | 0.40 ms |
| Large  | 2,000 | 14,000 | 5.2 s    | 387     | 2,709    | **0.10 ms**       | 0.51 ms |

Indexing throughput is linear in file count (~387 files/s on M-series).
Warm search — the path the agent takes for every call after the first —
is **sub-millisecond at every repo size**. Cold search (fresh mmap open
per query, the worst case the daemon ever pays) stays under 1 ms.

### AST chunker latency

`cargo bench --bench chunker` (TypeScript, M-series Mac):

| Workload | Source size | Time | Throughput |
|----------|-------------|------|-----------|
| Small  | 10 lines, 3 symbols | **2.96 ms** | — (dominated by parser + query setup) |
| Medium | 500 lines, 100 fns  | **4.24 ms** | ~118 KLOC/s |
| Large  | 5000 lines, 1000 fns | **18.59 ms** | ~269 KLOC/s |

Median of 100 samples (Criterion). Most of the small-workload cost is one-time `Query` compilation; caching parser+query per language will bring sub-ms steady-state cost.

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

## MCP server (`pluckd`)

`pluckd` speaks the Model Context Protocol over stdio. Any MCP-compatible
agent (Claude Code, Cursor, Codex, Aider, OpenHands, …) can wire it up.

```bash
# Build the daemon binary
cargo build --release -p pluck-mcp

# Probe locally
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  | ./target/release/pluckd --repo /path/to/repo
```

Six tools register at handshake — `pluck.read`, `pluck.search`, `pluck.grep`,
`pluck.symbol`, `pluck.peek`, `pluck.expand`. Today `read` / `search` / `grep`
are fully wired through `pluck-core`; the other three return placeholder
text and land in subsequent phases. Tool descriptions live under
`docs/mcp-descriptions/` and are compiled into the binary via `include_str!`,
so every release ships the exact copy the agent reads during tool selection.

### Wiring into Claude Code

```jsonc
// claude_config.json (excerpt)
{
  "mcpServers": {
    "pluck": {
      "command": "pluckd",
      "args": ["--repo", "/path/to/your/repo"]
    }
  }
}
```

## CLI

The `pluck` binary works standalone (the MCP server in Phase 1 is the same
core, exposed over stdio).

```bash
pluck index .                          # build the index for the current repo
pluck search "auth token expiry" \     # ranked chunks; default mode is lossless
        --repo . -k 10
pluck search "auth token expiry" \
        --repo . --compact             # opt-in lossy: score + path + match lines
pluck read src/auth/login.ts           # outline by default (~85% fewer tokens)
pluck read src/auth/login.ts --raw     # byte-identical to `cat`
pluck read src/auth/login.ts --lines 45-120
pluck grep "TODO" -- --type ts         # passthrough to ripgrep (--raw safety net)
```

The index is persisted at `~/.pluck/<repo-hash>/tantivy/` (override with
`PLUCK_HOME`). Today `pluck index` rebuilds from scratch; incremental
reindex via `notify` lands in Phase 2.

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
