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
| Plugin + release infra | 🟡 partial — pending broader agent installers |
| Retrieval quality (peek / expand / BM25F / ranking) | 🟡 partial — NDCG@10 lands in v0.3.0 |

---

## v0.1.0 — ship cutline (next, blocks the first public tag)

Sharp, not broad. Required to put the binary in users' hands without
embarrassment.

### Ship infrastructure
- [ ] First GitHub push of the repo to the public.
- [ ] `cargo publish` `pluck-core` → `pluck-mcp` → `pluck-cli` in
      dependency order. Driver script (`scripts/release.sh`) handles
      the 20 s sleeps between hops.
- [ ] Tag `v0.1.0`. Release workflow auto-builds four binaries
      (x86_64 / aarch64 × darwin / linux) and attaches them to the
      GitHub Release.

### One-command install
- [x] `pluck init --target claude` writes `.mcp.json`.
- [x] `pluck init --target codex` writes the Codex MCP block.
- [ ] `pluck init --target cursor` once Cursor's MCP config path
      stabilizes (otherwise defer to v0.5.0).

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

### Quality from-deferred work
- [ ] Path-qualifier in `pluck.peek` / `pluck.symbol`
      (`tokio/runtime/spawn` as a path filter).
- [ ] Display path normalization (relative to canonicalized repo
      root, not `/private/tmp/…` macOS resolution).
- [ ] Token-budget packing for `pluck.search` and `pluck.expand`
      (`max_tokens` param; greedy packing so the agent never gets a
      half-truncated chunk).

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
- [ ] Branch protection: require the CI check on `main`.
- [ ] Publish benchmark artifacts to a public dashboard or release
      attachment, not only ephemeral Actions artifacts.

### Match and beat semble's NDCG@10

**Target:** NDCG@10 ≥ 0.854 (semble's published number) on a
comparable multi-repo benchmark. Until this ships, semble wins the
recall argument.

- [ ] Two-stage cascade — widen BM25 candidate pool, embed-rerank.
- [ ] Query expansion via embedding-nearest BM25 vocab terms.
- [ ] Labeled NL recall@K test set (100 queries across tokio,
      django, next.js).
- [ ] Per-language NL recall breakdown.
- [ ] Hangul / CJK retrieval-accuracy bench.
- [ ] NDCG@10 measurement infrastructure.
- [ ] Continuous α from query embedding (replace
      `is_natural_language_query` heuristic with two centroid
      dot-products).
- [ ] BM25 stopword filter.

---

## v0.4.0 — language coverage

Five-language baseline (Rust / Py / TS / Go / JS) covers ~80 % of
agentic coding traffic. The next tier closes credibility gaps.

- [ ] Java chunker (largest gap — enterprise users).
- [ ] C / C++ chunker.
- [ ] Kotlin chunker (Android).
- [ ] Ruby, PHP, Swift chunkers (round-out the tier).
- [ ] Per-language chunker accuracy on real-world fixtures, gated by
      the regression gate.

---

## v0.5.0 — adoption + observability

By here we have install data and real query distributions.

- [ ] `pluck install` — install-time replacement of retrieval channel
      across detected agents (Claude Code / Codex / Cursor): MCP +
      hooks + permissions + rule files + shell wrappers (opt-in
      `--aggressive`).
- [ ] Adoption-rate counter (pluck call vs. bash fallback per session).
- [ ] Tool-description A/B harness.
- [ ] Real LLM-in-loop bench (Claude / Codex / Gemini on a fixed
      task, with and without pluck active).
- [ ] Latency p99 (current baseline only quotes p50).
- [ ] Memory / disk usage caps.
- [ ] Aider hook / loader.
- [ ] OpenHands tool registration.
- [ ] Cursor extension thin wrapper.
- [ ] Cline / Continue integration.
- [ ] Korean / Japanese / Chinese tool descriptions.

---

## SOON (not version-gated)

Engineering work that improves the inner loop without being
version-blocking.

- mmap-persistent on-disk index (warm-start cost: ~5 s → ~10 ms).
- Index schema versioning + automatic rebuild signal.
- Incremental embedding re-encode (only changed chunks).
- BM25 `b` / `k1` tuner.
- `.pluckignore`, symlink-loop guard, huge-file policy.
- JSON output mode for every tool.
- `find-pattern` (thin `ast-grep` wrapper).

## LATER

- Session-graph moat: opt-in personalized PageRank with `acted_on`
  seed.
- Content-type expansion (logs, markdown, JSON/YAML, OpenAPI, IaC,
  Jupyter notebooks).
- `pluck.diff`, `pluck.history`, `pluck.profile`.
