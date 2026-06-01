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
| v0.4 release train | 🟡 in progress — Java, HTML, CSS/SCSS, Markdown/MDX, YAML/JSON/TOML, Dockerfile, Shell, official agent setup prompt, TSX fixes landed on main |
| v0.5 systems + JVM | 🟡 chunker wave complete — Kotlin, SQL, HCL, C, and C++ landed on main; fixtures consolidation + NDCG breakdown remain |
| v0.6 app-framework tier | 🟡 in progress — Ruby, PHP, Swift, Svelte landed on main; OpenAPI/GraphQL + "20+ formats" claim remain. Vue/Astro deferred to v0.9 (grammar ABI) |

---

## Versioning policy

The roadmap is a milestone map, not an automatic version bump rule. When
planned roadmap work lands on `main`, it should be assigned to the milestone
that owns it. When unrelated issues, regressions, or release repairs come in,
choose the SemVer lane independently.

- Full versioning and release-lane rules live in
  [`docs/VERSIONING.md`](docs/VERSIONING.md).
- `v0.x.0` minor releases are user-visible capabilities: new languages,
  formats, tools, integrations, ranking behavior, storage layers, or benchmark
  surfaces.
- `v0.x.y` patch releases are only for bug fixes, security/dependency updates,
  CI/release repairs, and documentation fixes that clarify shipped behavior.
- New language or format support is never a patch release. It rides the next
  planned minor milestone. If the milestone mapping is wrong, update this
  roadmap first instead of forcing the change into the wrong version.

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
- [x] Universal all-in-one agent setup prompt: unknown MCP agents can install
      pluck, register the MCP server, and apply the strongest available
      pluck-first retrieval layer.
- [x] Official Claude Code / Codex / Cursor setup guidance: project-scoped MCP
      registration, permission / hook / rule / `AGENTS.md` layers, and
      verification steps documented in README and `docs/AGENT_INSTALL.md`.
- [x] HTML chunker: semantic elements, component-ish blocks, script/style
      sections.
- [x] TSX parser correctness: `.tsx` uses the TSX grammar, parse warnings name
      the path, and index summaries count parse-error files.
- [x] CSS / SCSS chunker: selector and at-rule chunks.
- [x] Markdown / MDX chunker: heading sections and fenced code blocks.
- [x] YAML / JSON / TOML chunker: path/key chunks for config-heavy repos.
- [x] Dockerfile chunker: stages, instructions, and dependency/install blocks.
- [x] Shell chunker: functions, case arms, and major script sections.
- [ ] Chunker accuracy fixtures for all v0.4 formats.
- [ ] Regression-gate metric for "format chunk recovery" so coverage does not
      silently shrink.
- [ ] Outline emits top-level side-effect chunks (HTTP interceptors,
      route / middleware registration, module-level initializers) so common
      library setup is visible without reading the full file.
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

- [x] C chunker: function definitions and forward declarations (incl.
      pointer-return), struct / enum / union (standalone + typedef'd,
      anonymous and named), function-pointer typedefs, object-like
      and function-like macros (`#define`), and `#include` directives.
      Inner names of typedef'd named enums / structs index alongside
      the typedef name so grep by either surface lands.
- [x] C++ chunker: namespace (single + nested), class / struct /
      enum (incl. `enum class`) / union, templated class and free
      function, out-of-class member impls via `qualified_identifier`,
      in-class member declarations (regular method, destructor,
      operator overload), `= delete` / `= default` special members,
      value / pointer / reference return variants. C-style typedefs,
      macros, and `#include` directives share patterns with C.
- [x] Kotlin chunker (Android + JVM).
- [x] SQL chunker: CREATE TABLE / VIEW / INDEX / FUNCTION / TRIGGER
      and ALTER TABLE migrations. CREATE PROCEDURE not supported —
      tree-sitter-sequel grammar limitation; waits on upstream fix or
      parser swap.
- [x] Terraform / HCL chunker: resource / data / module / variable /
      output / provider / locals / terraform blocks, plus nested
      blocks (backend / lifecycle / required_providers / dynamic).
      Uniform Module kind; dotted symbols matching HCL reference
      syntax (`resource.aws_s3_bucket.main`, `variable.region`,
      `data.aws_caller_identity.current`).
- [ ] Per-language real-world fixtures for C, C++, Kotlin, SQL, and HCL.
- [ ] Recall / NDCG breakdown includes every v0.5 language where labeled data
      exists.

---

## v0.6.0 — app-framework tier

Round out the long-tail repos agents still touch every day. Scope is
limited to work we can land ourselves; component frameworks whose
tree-sitter grammars are not yet compatible with our parser version
(Vue, Astro) are tracked under v0.9.0 — ecosystem, gated on grammar
availability, so they do not hold this milestone hostage.

- [x] Ruby chunker.
- [x] PHP chunker.
- [x] Swift chunker.
- [x] Svelte single-file component chunker.
- [ ] OpenAPI / GraphQL schema chunker.
- [ ] "20+ code and project formats" README claim backed by tests.

---

## v0.7.0 — scale + persistence

Make large repos feel instant after the first index.

- [ ] mmap-persistent on-disk index (warm-start cost: ~5 s → ~10 ms).
- [ ] Index schema versioning + automatic rebuild signal.
- [ ] Staleness signal on `pluck.search` / `peek` / `symbol` / `expand`:
      per-chunk `stale: bool` + `index_age_ms` derived from
      mtime-vs-`indexed_at` comparison, so the agent knows when to
      fall back to `pluck.read` for fresh content. Flag-only — no
      synchronous re-index on the read path, to preserve the
      "fast indexed search" semantic. `--raw` mode suppresses both
      fields to keep byte-equivalent output. Trust precondition for
      being the default retrieval layer; pairs with the
      schema-versioning item above.
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
- [x] Agent setup docs distinguish installation from pluck-first behavior
      while keeping the user-facing prompt copy-pastable as one block.
- [ ] Adoption-rate counter: pluck calls vs. bash/read/grep fallback per
      session.
- [ ] Tool-description A/B harness.
- [ ] Real LLM-in-loop bench: Claude / Codex / Gemini on fixed tasks, with and
      without pluck active. Gold metric: total conversation tokens to task
      completion; per-call token counts secondary. Prerequisite for changing
      tool defaults (e.g., compact-first `pluck.search`).
- [ ] Per-call cost preview in tool responses: outline / search / grep return
      a "this view = N tokens; expanded = M tokens" line so the agent sees
      the price of each shape and gravitates to the cheap one.
- [ ] Korean / Japanese / Chinese tool descriptions.
- [ ] Public benchmark dashboard fed by nightly runs.

---

## v0.9.0 — workflow intelligence + ecosystem

Turn retrieval into workflow memory and meet agents where users already work.

- [ ] JSON output mode for every tool.
- [ ] `pluck.diff`: change-aware retrieval for current branch / PR.
- [ ] `pluck.history`: search relevant prior changes.
- [ ] `pluck.profile`: explain token, latency, and retrieval behavior.
- [ ] `pluck.plan` v2 — cheapest-path orchestrator: returns an ordered call
      sequence (e.g., grep → outline → symbol for body X) with per-step
      token estimates, not just a list of candidate files.
- [ ] `pluck.grep` v2 — enclosing-context responses: each hit returns
      the enclosing chunk (`symbol` / `kind` / `signature` + a small
      snippet window around the match) and dedups hits inside the same
      chunk into one entry with a `match_lines` array. Includes a
      string-literal-vs-identifier flag from the parse tree so log /
      regex / format-string matches are distinguishable from real code
      matches. Default sort is by enclosing-chunk `kind` (code-first),
      lossless — never filters. `--raw` mode suppresses enclosing /
      category / sort to preserve rg-byte-equivalent output. Direct
      attack on the grep→read roundtrip that is the highest-frequency
      token-waste pattern.
- [ ] Session-graph ranking: opt-in personalized PageRank with `acted_on`
      seeds.
- [ ] Aider hook / loader.
- [ ] OpenHands tool registration.
- [ ] Cursor extension thin wrapper.
- [ ] Cline / Continue integration.
- [ ] `find-pattern`: thin `ast-grep` wrapper.
- [ ] Vue single-file component chunker: template, script, style, and nested
      JS/TS/CSS chunks. **Gated on grammar availability** — `tree-sitter-vue`
      ships only 0.0.x pinned to tree-sitter ^0.20, incompatible with our
      0.25 parser. Promote to a near-term milestone once a compatible
      grammar (or a maintained fork) lands on crates.io.
- [ ] Astro single-file component chunker. **Gated on grammar availability**
      — no `tree-sitter-astro` crate exists on crates.io yet.

---

## v1.0.0 — default retrieval layer

The first stability line: pluck can credibly ask users to make it the default
read/search layer for coding agents.

- [ ] Stable MCP tool contract and compatibility notes.
- [ ] Stable CLI output contracts for scripted use.
- [ ] Install docs beyond the first supported agents: Aider, OpenHands, Cline,
      Continue, and any other MCP-capable agent with stable official config
      surfaces.
- [ ] Reproducible benchmark dashboard with release-pinned artifacts.
- [ ] Release checklist covers crates.io, GitHub Release, Homebrew, README,
      roadmap image, and benchmark baseline updates.
- [ ] Backward-compatible config migration path.
- [ ] Security and supply-chain review of GitHub Actions, Rust dependencies,
      and generated artifacts.
