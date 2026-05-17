# Changelog

All notable changes to this project will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [SemVer](https://semver.org).

## [Unreleased]

### Added

- Java chunker support for classes, interfaces, records, annotation types,
  enums, methods, constructors, imports, and direct callees.
- HTML chunker support for semantic elements, component-like blocks, and
  script/style sections.
- CSS / SCSS chunker support for selector rules, nested SCSS selectors, and
  at-rule blocks such as `@media`, `@keyframes`, `@mixin`, and `@function`.
- Markdown / MDX chunker support for heading sections and fenced code blocks.
- YAML / JSON / TOML chunker support for config key-path chunks such as
  `scripts.build`, `services.web.image`, and `workspace.dependencies.serde`.
- Dockerfile / Containerfile chunker support for build stages, individual
  instructions, dependency-manifest copies, and install blocks.
- Shell chunker support for functions, `case` arms, and major comment-marked
  script sections.
- Prompt-first agent install flow so unknown MCP-capable agents can
  self-configure pluck with their strongest available rule, hook, allowlist,
  or permission layer.
- Version-management guide and release metadata check. PRs now declare their
  release lane, and tag releases fail if Cargo versions, internal dependency
  versions, Cargo.lock, and CHANGELOG are out of sync.
- Patch release lane and `scripts/bump-version.py`, so `v0.x.y` maintenance
  releases are a first-class path instead of an ad hoc manual edit.
- Roadmap-vs-release decision rules, separating planned minor milestones from
  independent patch decisions for shipped bugs and support issues.
- Maintainer orchestration loop and `scripts/maintainer-status.py`, so merge
  completion always surfaces version, tag, changelog, patch-backport, and
  release-cut decisions.

### Changed

- Workspace version advanced to `0.4.0` for the active v0.4 release train.
- README / ROADMAP install guidance now makes the agent prompt the primary
  adoption path instead of a hidden fallback.
- Roadmap image removed so version progress is tracked in text instead of a
  frequently stale generated asset.

### Fixed

- TSX files now parse with tree-sitter's TSX grammar instead of the plain
  TypeScript grammar. Parse warnings include the repo-relative path, and index
  summaries count files with parse errors.

## [0.3.0] — 2026-05-17

### Added

- PR CI on `ubuntu-latest`: `cargo test --all`, benchmark artifacts, and
  conditional regression-gate comments on PRs.
- Release gate for tags: tests and `scripts/regression-gate.py` must pass
  before binaries or crates are published.
- Nightly/manual full-scenario benchmark workflow with artifact upload and a
  secret slot for Claude-backed runners.
- Labeled retrieval suite with Recall@K / MRR / NDCG@10 reporting, expanded to
  100 natural-language queries across tokio, django, and next.js.
- Retrieval-quality improvements: two-stage BM25 + embedding cascade, query
  expansion, continuous hybrid weighting from query embeddings, BM25 stopword
  filtering, and per-language / CJK retrieval breakdowns.

### Changed

- Benchmark artifacts are published beyond ephemeral local runs, so release
  claims can cite committed or attached result files.
- README roadmap and performance claims were aligned to the public benchmark
  evidence shipped with the repo.

## [0.2.0] — 2026-05-15

### Added

#### `pluck.plan` — exploration recommender (v0.2.0 surface item)
- Probe-search with a free-form task description; recommends the next
  3–5 retrieval calls plus a confidence indicator (high/medium/low).
- Heuristics pick tools by chunk kind and size: large functions →
  `peek`, small ones → `symbol`, struct/class/impl → `symbol`, module →
  `read`, top-hit function → an `impact` follow-up.
- 2+ chunks from the same file collapse into one `read` step (shared
  context).
- Low-confidence output emits a broaden hint instead of bad leads.

#### `pluck.deps` — file-level import graph (v0.2.0 surface item)
- Forward edges (`deps`) and reverse edges (`importers`) over the
  indexed files.
- Per-language import queries (Rust `use` / `extern crate`, Python
  `import` / `from`, JS/TS `import` / `require` / dynamic `import()` /
  re-exports, Go `import_spec`). Captures merged into the single
  compiled tree-sitter query used for chunks + callees.
- Best-effort resolution: relative imports (`./foo`, `../bar`), Python
  dotted relatives (`from . import x`), Rust `use crate::...` suffix
  match against indexed files; externals returned with `resolved: None`.

#### `pluck.impact` — reverse caller index (v0.2.0 surface item)
- BFS depth-capped (1..3) traversal of the reverse callee → caller
  index. Test-file callers sorted to the bottom so production impact
  surfaces first.
- Reverse index (`HashMap<callee_leaf, Vec<chunk_id>>`) shared between
  `PluckIndex` and `IndexBatch` via `Arc`. Populated from the
  pre-extracted `Chunk.callees` field — no re-parse per chunk.

#### `pluck.digest` — build / CI / test output compression (v0.2.0 surface item)
- Per-tool handler registry: `cargo` (build / test / check), npm / pnpm /
  yarn, pytest, GitHub Actions log. Errors / panics / tracebacks /
  failed-step bodies kept verbatim; progress lines collapse to counts.
- Auto-detect from format markers; `--format <name>` override;
  `--show-format` for debugging.
- 6-fixture bench suite; median savings 71 % (gated metric
  `digest_savings_pct`).
- MCP tool + CLI subcommand (`pluck digest [path] [--format] [--show-format]`).

### Changed

- **Chunker optimization (single-pass merged tree-sitter query).**
  Chunker query + callee query + import query merged into one
  compiled `Query` per language, cached in `OnceLock`. One tree walk
  per file instead of three; zero per-call query compilation. Improved
  `chunker_medium_ms_p50` from 4.24 ms → 1.05 ms (−75 %); improved
  `indexer_files_per_sec_medium` from 386 → 2 747 files/s (+612 %).
- **Baseline refrozen** (`benchmarks/baseline.json`):
  - `chunker_medium_ms_p50`: 4.24 → 1.05 ms
  - `indexer_files_per_sec_medium`: 386 → 2 747 files/s
  - `warm_search_p50_ms_medium`: 0.06 → 0.07 ms
  - `freshness_p50_ms_medium`: 183 → 171 ms
  - `session_dedup_session_savings_pct`: 44 → 23 % (side-effect of
    richer v0.2.0 codebase — more chunks, sharper top-K, less query
    overlap on the 5 fixture queries)
  - `digest_savings_pct`: new metric, 71 %
- **README rewrite (English + Korean).** Centered logo header,
  Quickstart / Why pluck / MCP Tools / Performance / Feature comparison
  / Roadmap sections; every number cites `benchmarks/baseline.json` or
  `benchmarks/results/`; mmap claim corrected to "roadmapped (SOON)";
  ONNX claim corrected to static `model2vec` lookup.

### Fixed

- Watcher / indexer no longer re-parse source per chunk for callee
  extraction; the cost moved to chunking and was eliminated by the
  single-pass query merge.

## [0.1.0] — internal pre-release

Internal development cutline before the first crates.io publish.

### Added

- Workspace scaffolding (`pluck-core`, `pluck-mcp`, `pluck-cli`,
  `pluck-bench`).
- 6 base MCP tools: `read`, `grep`, `search`, `symbol`, `peek`, `expand`.
- Tree-sitter AST chunker for Rust / Python / TypeScript / Go /
  JavaScript.
- BM25F tantivy index with per-field boosts (symbol / signature /
  content); hybrid BM25 + semantic via reciprocal-rank fusion using
  `potion-code-16M` static embedding.
- File-watcher with 150 ms debounce + incremental reindex.
- Session-scoped chunk dedup at the MCP layer.
- `pluck init --target {claude,codex}` for one-command MCP registration.
- Regression gate (`scripts/regression-gate.py`) with 5 frozen metrics
  in `benchmarks/baseline.json`.
- Safety guards: `pluck.read --raw` on binaries returns cat-style
  diagnostic; absolute paths outside the repo are rejected;
  `PLUCK_DISABLE_EMBEDDINGS` and encoder-load failure degrade cleanly
  to BM25-only.
- `scripts/smoke.sh` one-liner install verification.
- CI workflows, Claude Code plugin manifest, CONTRIBUTING,
  CODE_OF_CONDUCT, GitHub issue + PR templates.

[Unreleased]: https://github.com/hunhee98/pluck/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/hunhee98/pluck/releases/tag/v0.3.0
[0.2.0]: https://github.com/hunhee98/pluck/releases/tag/v0.2.0
[0.1.0]: https://github.com/hunhee98/pluck/releases/tag/v0.1.0
