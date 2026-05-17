# Contributing to pluck

Thanks for taking the time. pluck is a performance-first OSS retrieval
engine for AI coding agents; the rules below exist so that performance
guarantees and the agent-facing contract stay intact across contributions.

## Quick start

```bash
./scripts/bootstrap.sh      # toolchain + embedding model + submodules
cargo build --release
cargo test --workspace
```

For an end-to-end install verification:

```bash
./scripts/smoke.sh
```

## Workspace layout

| Crate | Responsibility | Forbidden |
|-------|----------------|-----------|
| `pluck-core` | Chunker, indexer, store, watcher, BM25F + semantic, ranking | Depend on `pluck-mcp`. Hold session state. |
| `pluck-mcp` | `pluckd` binary, MCP handlers, session dedup map | Business logic in handlers. Mutate index directly. |
| `pluck-cli` | `pluck` binary, CLI args, output formatting, `pluck init` | Re-implement retrieval — delegate to `pluck-core`. |
| `pluck-bench` | Benchmark scenarios, scoring, runners | Live in production code paths. |

The `pluck-core` isolation (no MCP dependency) is what keeps Aider / OpenHands /
Cursor harnesses pluggable later. Don't break it.

## Style

- `cargo fmt` before commit.
- `cargo clippy --workspace -- -D warnings` must pass.
- Public APIs documented with rustdoc; behavior-rich functions get a
  brief WHY paragraph (constraints, invariants), not a WHAT paragraph
  (the signature already says that).
- Comments only when the WHY is non-obvious. Don't narrate the code.

## Commit conventions

Subject line: `<type>(<scope>): <description>` (≤ 72 chars).

- `type`: `feat` / `fix` / `bench` / `docs` / `chore` / `refactor` / `ci`.
- `scope`: the crate (`core`, `mcp`, `cli`, `bench`) or a section name for top-level docs (`readme`, `ci`, …).

Body cites the metric that improved or the invariant preserved. Example:

```
feat(core): post-fusion ranking pipeline (symbol / sibling / test-file)

warm_search_p50 unchanged (0.06 ms).
session_dedup savings: 44% → 47%.
```

One change per commit. No bundled "while I'm at it" refactors.

## Version impact

Every PR must choose a release lane before it merges. See
[`docs/VERSIONING.md`](docs/VERSIONING.md) for the full policy.

- `patch` — bug fixes, security updates, CI/release repairs, and docs that
  clarify already-shipped behavior.
- `minor-train` — new languages, formats, tools, integrations, ranking
  behavior, storage layers, benchmark surfaces, or install/adoption behavior.
- `release-now` — the PR that cuts a GitHub/crates.io release.
- `no-release` — internal-only or test-only work with no user-visible behavior.

If a bug fix must ship before the next minor train, create a patch-only
maintenance branch (for example `release/v0.3.x`) and cherry-pick it there.
Once new minor features have landed on `main`, that same fix ships with the
next minor unless it is backported.

Use `python3 scripts/bump-version.py patch` for patch release branches and
`python3 scripts/bump-version.py minor` when `main` starts the next minor train.
The script updates the workspace version, internal dependency pins, and
`Cargo.lock` together.

## Engine-core changes — the regression gate

If your PR touches anything under
`crates/pluck-core/src/{index,store,chunker,watcher,tokenizer,ranking}.rs`
or the MCP session-dedup map, you **must** run the regression gate before
committing:

```bash
python3 scripts/regression-gate.py
```

The gate compares against [`benchmarks/baseline.json`](benchmarks/baseline.json),
five frozen metrics:

| Metric | Direction | Tolerance |
|---|---|---|
| `chunker_medium_ms_p50` | lower | 100 % |
| `indexer_files_per_sec_medium` | higher | 60 % |
| `warm_search_p50_ms_medium` | lower | 200 % |
| `freshness_p50_ms_medium` | lower | 50 % |
| `session_dedup_session_savings_pct` | higher | 25 % |

A failing gate blocks the commit. Intentional improvements rewrite the
baseline:

```bash
python3 scripts/regression-gate.py --update
```

Commit the updated `baseline.json` in the same PR as the change, with the
% delta in the commit body.

## Performance claims

Every number that appears in user-facing copy (README, `docs/*.md`, MCP
tool descriptions, release notes) must cite a row in `benchmarks/baseline.json`
or a file under `benchmarks/results/`. **No source = no number.** Hand-waved
claims block the PR.

## MCP tool descriptions

Tool descriptions ship at every MCP handshake. The full tool set currently
costs ≈ 1.5 K tokens — every byte is paid for on every conversation start
in every agent that uses pluck.

If you edit anything under `docs/mcp-descriptions/`:

1. The contract is **WHEN / WHY / FALLBACK**. Every description has all
   three sections. Prose around the contract is cuttable; the contract
   block is not.
2. The "FALLBACK to bash when:" clause must list concrete triggers
   (binary file, path outside repo, daemon down). Vague fallbacks let
   agents disengage at will.
3. Run the description audit to see the token-count delta:

   ```bash
   python3 scripts/desc-audit.py   # if available, else summarize manually
   ```

## Pull request checklist

- [ ] `cargo build --release` passes.
- [ ] `cargo test --workspace` passes.
- [ ] `cargo clippy --workspace -- -D warnings` clean.
- [ ] Engine-core touched? `python3 scripts/regression-gate.py` output
      pasted into the PR body.
- [ ] New MCP tool? Description carries `WHEN / WHY / FALLBACK` sections.
- [ ] User-facing perf claim? Cited row in `benchmarks/baseline.json`.
- [ ] Version lane selected in the PR body.
- [ ] Patch backport decision selected in the PR body.
- [ ] ROADMAP updated if the PR changes user-visible behavior.
- [ ] CHANGELOG entry under `[Unreleased]`.

## Filing issues

- **Bug** — use the bug template. Include `pluck --version`, OS, repro steps,
  and `tracing` output if relevant.
- **Feature request** — use the feature template. State the problem, then
  the proposed solution, then alternatives you considered.
- **Quality question** ("why didn't pluck find X?") — open a discussion or
  a feature request. We treat recall complaints as roadmap input.

## License

By contributing you agree your contributions are licensed under the project's
MIT license — see [LICENSE](LICENSE).

## Code of conduct

Participation in this project is governed by the [Contributor Covenant Code
of Conduct](CODE_OF_CONDUCT.md). Report unacceptable behavior to the
maintainers via private channels.
