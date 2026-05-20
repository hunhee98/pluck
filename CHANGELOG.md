# Changelog

All notable changes to this project will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [SemVer](https://semver.org).

## [Unreleased]

### Fixed

- Release workflow no longer blocks GitHub Release creation when the
  `aarch64-unknown-linux-gnu` cross build fails. The target is marked
  `continue-on-error: true` because Azure-hosted Ubuntu runners cannot
  fetch arm64 apt indexes through the default mirror, breaking
  `libssl-dev:arm64` install. Proper fix (ports.ubuntu.com sources or
  `cross-rs/cross` Docker container) tracked for v0.5.x. Until then a
  release ships with the three binaries that DO build (x86_64-darwin,
  aarch64-darwin, x86_64-linux).

## [0.5.0] — 2026-05-19

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
- Kotlin chunker support for class, interface, enum class, data class, and
  object declarations, and top-level / member / extension `fun`, with KDoc
  and `//` line-doc extraction. Receiver-call (`bar.foo`) and chained
  (`baz.bar.foo`) callees come from `navigation_expression` so call-graph
  retrieval (`pluck.impact`, `pluck.expand`) follows Kotlin method chains.
- SQL chunker support for `CREATE TABLE` / `VIEW` / `INDEX` / `FUNCTION` /
  `TRIGGER` and `ALTER TABLE` migration statements via tree-sitter-sequel,
  with PL/pgSQL body callees and `--` / `/* */` doc comments. Each `ALTER
  TABLE` emits a module chunk on the targeted object so migration files
  surface the touched table even when the original `CREATE TABLE` lives
  elsewhere. `CREATE PROCEDURE` is not yet supported (grammar limitation;
  waits on upstream fix or parser swap).
- Terraform / HCL chunker support for `resource` / `data` / `module` /
  `variable` / `output` / `provider` / `locals` / `terraform` blocks plus
  nested blocks (`backend` / `lifecycle` / `required_providers` / `dynamic`).
  Dotted symbols match HCL's own reference syntax —
  `resource.aws_s3_bucket.main`, `variable.region`,
  `data.aws_caller_identity.current` — so a search by either the block
  type or the addressable form resolves.
- C chunker support for function definitions and forward declarations
  (including pointer return), struct / enum / union (standalone and
  typedef'd, anonymous and named), function-pointer typedefs, object-like
  and function-like `#define` macros, and `#include` directives (system
  and project). Inner names of typedef'd named enums and structs index
  alongside the typedef name so `grep` by either surface lands.
- C++ chunker support for namespaces (single and nested `a::b`), classes
  / structs / `enum class` / unions, templated classes and free functions,
  out-of-class member implementations via `qualified_identifier`, in-class
  member declarations (regular methods, destructors, operator overloads),
  `= delete` / `= default` special members, and value / pointer / reference
  return variants. Macros, typedefs, and `#include`s share the C patterns.
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

- `pluckd` can pin its advertised MCP protocol with
  `PLUCK_MCP_PROTOCOL_VERSION` for clients that have not caught up to the
  current `2025-11-25` protocol, while keeping the default protocol unchanged.
- TSX files now parse with tree-sitter's TSX grammar instead of the plain
  TypeScript grammar. Parse warnings include the repo-relative path, and index
  summaries count files with parse errors.
- `pluck.grep` now treats `pattern` as a literal string by default, matching
  the tool's advertised "literal by default" semantics. ripgrep is invoked
  with `--fixed-strings` unless the caller passes a mode flag (`-e`,
  `--regexp`, `-P`, `--pcre2`, `-F`, `--fixed-strings`, `-f`, `--file`).
  Identifiers containing regex metacharacters such as `Foo(` previously
  failed with an unclosed-group parse error.
- `pluck.grep` now pre-validates `cwd` and surfaces a clear "cwd does not
  exist" diagnostic when the directory is missing, instead of propagating a
  misleading "is `rg` on PATH?" spawn error (Unix OS error 2 / Windows OS
  error 267).

- Release workflow now owns GitHub Release creation and binary asset
  upload only; crates.io publishing stays in `scripts/release.sh` to
  avoid duplicate publish attempts after a tag push.
- Release workflow now configures target OpenSSL for
  `aarch64-unknown-linux-gnu` cross builds so the ARM Linux tarball
  remains part of the release asset set.

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

[Unreleased]: https://github.com/hunhee98/pluck/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/hunhee98/pluck/releases/tag/v0.5.0
[0.3.0]: https://github.com/hunhee98/pluck/releases/tag/v0.3.0
[0.2.0]: https://github.com/hunhee98/pluck/releases/tag/v0.2.0
[0.1.0]: https://github.com/hunhee98/pluck/releases/tag/v0.1.0
