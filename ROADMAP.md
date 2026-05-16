# Roadmap

Public arc of pluck. Internal planning notes — including IR / CS
justifications, deferred work, and decision gates — live in
[`docs/ROADMAP.md`](docs/ROADMAP.md) (not tracked).

## Mission

**Be the agent's default retrieval tool.** Every retrieval call an
AI coding agent makes inside an indexed repo should default to
pluck instead of `cat` / `grep` / built-in Read / built-in Grep.
pluck is byte-equivalent in `--raw` mode and 10–100× cheaper in
tokens for the non-trivial cases (outline, hybrid search, peek,
expand, session dedup).

---

## Status snapshot

| Phase | State |
|-------|-------|
| Foundation (workspace, chunker, BM25 index, CLI, first scenario) | ✅ shipped |
| Core MCP (6 base tools wired, descriptions at handshake) | ✅ shipped |
| Semantic + incremental (RRF, watcher, session dedup) | ✅ shipped |
| Regression gate (6 frozen metrics, baseline.json) | ✅ shipped |
| CI + release gates | ✅ shipped — PR test/bench artifacts, release regression gate |
| Plugin + release infra | ✅ shipped — Claude/Codex/Cursor pluck-first init + release gates |
| Retrieval quality (peek / expand / BM25F / ranking) | ✅ shipped — v0.3.0 |
| v0.4 release train | 🟡 in progress — Java chunker landed on main |

---

## Versioning policy

The roadmap is the release train. When work lands on `main`, it should be
assigned to the next unreleased version here.

- `v0.x.0` minor releases are user-visible capabilities: new languages,
  formats, tools, integrations, ranking behavior, storage layers, or benchmark
  surfaces.
- `v0.x.y` patch releases are only for bug fixes, security/dependency updates,
  CI/release repairs, and documentation fixes that clarify shipped behavior.
- New language or format support is never a patch release. It rides the next
  minor train.

---

## v0.1.0 — ship cutline (shipped)

Sharp, not broad. Required to put the binary in users' hands without
embarrassment.

### Ship infrastructure
- [x] First GitHub push of the repo to the public.
- [x] `cargo publish` `pluck-core` → `pluck-mcp` → `pluck-cli` in
      dependency order. Driver script (`scripts/release.sh`) handles
      the 20 s sleeps between hops.
- [x] Tag `v0.1.0`. Release workflow builds binaries and attaches
      them to the GitHub Release.

### One-command install
- [x] `pluck init --target claude` writes `.mcp.json`.
- [x] `pluck init --target codex` writes the Codex MCP block.
- [x] `pluck init --target cursor` writes `.cursor/mcp.json` and a
      pluck-first Cursor rule.

### Safety
- [x] `pluck.read --raw` on a binary returns a cat-style diagnostic
      (no panic).
- [x] Absolute paths outside the repo are rejected.
- [x] Encoder load failure / `PLUCK_DISABLE_EMBEDDINGS` falls back
      to BM25-only without panicking.

### OSS hygiene
- [x] CONTRIBUTING.md, CODE_OF_CONDUCT.md.
- [x] GitHub issue templates + PR template.
- [x] `scripts/smoke.sh` install verification.

---

## v0.2.0 — surface area wave (shipped May 2026)

**semble** ships 2 MCP tools (`search`, `find_related`). After v0.2.0
pluck ships 10. None of these are catch-up — semble has none of them.

- [x] **`pluck.digest`** — build / CI / test output compression
      (cargo / npm / pytest / GHA). 71 % median savings.
- [x] **`pluck.impact`** — reverse call-graph blast radius. BFS
      depth-capped, test-file callers segregated.
- [x] **`pluck.deps`** — file-level import graph (forward and
      reverse). 5 languages, relative + suffix-match resolution.
- [x] **`pluck.plan`** — exploration recommender. Probe-search + 3–5
      next-call recommendations + confidence indicator.

---

## v0.3.0 — benchmark credibility + retrieval quality

Before adding more surface area, make the public claims hard to argue
with: reproducible CI, stable release gates, and labeled retrieval
quality.

### Release and benchmark infrastructure
- [x] PR CI on `ubuntu-latest`: `cargo test --all`, bench artifacts,
      and conditional regression gate comments on PRs.
- [x] Release gate: tags must pass tests and `scripts/regression-gate.py`
      before binaries or crates are published.
- [x] Nightly/manual full scenario benchmark workflow with artifact
      upload and a secret slot for Claude-backed runners.
- [x] Branch protection: require the CI check on `main`.
- [x] Publish benchmark artifacts to a public dashboard or release
      attachment, not only ephemeral Actions artifacts.

### Match and beat semble's NDCG@10

**Target:** NDCG@10 ≥ 0.854 (semble's published number) on a
comparable multi-repo benchmark. Until this ships, semble wins the
recall argument.

- [x] Two-stage cascade — widen BM25 candidate pool, embed-rerank.
- [x] Query expansion via embedding-nearest BM25 vocab terms.
- [x] Labeled retrieval suite format with Recall@K / MRR / NDCG@10
      reporting.
- [x] Expand labeled NL recall@K test set to 100 queries across tokio,
      django, next.js).
- [x] Per-language NL recall breakdown.
- [x] Hangul / CJK retrieval-accuracy bench.
- [x] NDCG@10 measurement infrastructure.
- [x] Continuous α from query embedding (replace
      `is_natural_language_query` heuristic with two centroid
      dot-products).
- [x] BM25 stopword filter.

---

## v0.4.0 — Java + repo-format coverage

Make pluck useful across the files agents read constantly, not only
programming-language source files. This is the first v0.4 train and should be
released as `0.4.0`, not `0.3.1`.

- [x] Java chunker: class, interface, record, annotation type, enum, method,
      constructor, imports, and direct callees.
- [x] Universal agent install prompt: unknown MCP agents can self-configure
      pluck with the strongest available allowlist / hook / rule layer.
- [ ] HTML chunker: semantic elements, component-ish blocks, script/style
      sections.
- [ ] CSS / SCSS chunker: selector and at-rule chunks.
- [ ] Markdown / MDX chunker: heading sections and fenced code blocks.
- [ ] YAML / JSON / TOML chunker: path/key chunks for config-heavy repos.
- [ ] Dockerfile chunker: stages, instructions, and dependency/install blocks.
- [ ] Shell chunker: functions, case arms, and major script sections.
- [ ] Chunker accuracy fixtures for all v0.4 formats.
- [ ] Regression-gate metric for "format chunk recovery" so coverage does not
      silently shrink.
- [ ] Path-qualifier in `pluck.peek` / `pluck.symbol`
      (`tokio/runtime/spawn` as a path filter).
- [ ] Display path normalization (relative to canonicalized repo root, not
      `/private/tmp/…` macOS resolution).
- [ ] Token-budget packing for `pluck.search` and `pluck.expand`
      (`max_tokens` param; greedy packing so the agent never gets a
      half-truncated chunk).

---

## v0.5.0 — systems + JVM tier

Close the next high-signal language gaps after Java and repo formats.

- [ ] C chunker.
- [ ] C++ chunker.
- [ ] Kotlin chunker (Android + JVM).
- [ ] SQL chunker: statements, views, functions/procedures, migrations.
- [ ] Terraform / HCL chunker: resources, data sources, modules, variables.
- [ ] Per-language real-world fixtures for C, C++, Kotlin, SQL, and HCL.
- [ ] Recall / NDCG breakdown includes every v0.5 language where labeled data
      exists.

---

## v0.6.0 — app-framework tier

Round out the long-tail repos agents still touch every day.

- [ ] Ruby chunker.
- [ ] PHP chunker.
- [ ] Swift chunker.
- [ ] Vue single-file component chunker: template, script, style, and nested
      JS/TS/CSS chunks.
- [ ] Svelte single-file component chunker.
- [ ] Astro single-file component chunker.
- [ ] OpenAPI / GraphQL schema chunker.
- [ ] "20+ code and project formats" README claim backed by tests.

---

## v0.7.0 — scale + persistence

Make large repos feel instant after the first index.

- [ ] mmap-persistent on-disk index (warm-start cost: ~5 s → ~10 ms).
- [ ] Index schema versioning + automatic rebuild signal.
- [ ] Incremental embedding re-encode for changed chunks only.
- [ ] Memory and disk usage caps.
- [ ] `.pluckignore`, symlink-loop guard, and huge-file policy.
- [ ] Latency p99 benchmark lane, not only p50.

---

## v0.8.0 — adoption + observability

Measure whether agents actually choose pluck over fallback tools.

- [x] `pluck init` — install-time replacement of retrieval channel across
      Claude Code / Codex / Cursor: MCP + hooks + permissions + rule files
      (opt-in `--mode aggressive` for Claude).
- [ ] Adoption-rate counter: pluck calls vs. bash/read/grep fallback per
      session.
- [ ] Tool-description A/B harness.
- [ ] Real LLM-in-loop bench: Claude / Codex / Gemini on fixed tasks, with and
      without pluck active.
- [ ] Korean / Japanese / Chinese tool descriptions.
- [ ] Public benchmark dashboard fed by nightly runs.

---

## v0.9.0 — workflow intelligence + ecosystem

Turn retrieval into workflow memory and meet agents where users already work.

- [ ] JSON output mode for every tool.
- [ ] `pluck.diff`: change-aware retrieval for current branch / PR.
- [ ] `pluck.history`: search relevant prior changes.
- [ ] `pluck.profile`: explain token, latency, and retrieval behavior.
- [ ] Session-graph ranking: opt-in personalized PageRank with `acted_on`
      seeds.
- [ ] Aider hook / loader.
- [ ] OpenHands tool registration.
- [ ] Cursor extension thin wrapper.
- [ ] Cline / Continue integration.
- [ ] `find-pattern`: thin `ast-grep` wrapper.

---

## v1.0.0 — default retrieval layer

The first stability line: pluck can credibly ask users to make it the default
read/search layer for coding agents.

- [ ] Stable MCP tool contract and compatibility notes.
- [ ] Stable CLI output contracts for scripted use.
- [ ] Install docs for Claude Code, Codex, Cursor, Aider, OpenHands, Cline, and
      Continue.
- [ ] Reproducible benchmark dashboard with release-pinned artifacts.
- [ ] Release checklist covers crates.io, GitHub Release, Homebrew, README,
      roadmap image, and benchmark baseline updates.
- [ ] Backward-compatible config migration path.
- [ ] Security and supply-chain review of GitHub Actions, Rust dependencies,
      and generated artifacts.
