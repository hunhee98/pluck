# Benchmarks

Reproducible. CI-driven. Public dashboard. No cherry-picked numbers.

## Methodology

Each measurement = `(scenario, runner, repo_size, repetition)`.

- **Scenario** — a YAML manifest declaring a prompt + success criteria
- **Runner** — an agent setup (which model, which MCP servers, which tools allowed)
- **Repo size** — small (1k LOC), medium (50k), large (500k), monorepo (100k+)
- **Repetitions** — 5 per cell; report median + IQR

Total per full run: `5 scenarios × 5 runners × 4 sizes × 5 reps = 500 sessions`.

## Runners compared

| Runner | Setup |
|--------|-------|
| `bash` | Claude Code + Bash tool only (cat / grep / ls) |
| `ripgrep` | Claude Code + Bash + ripgrep on PATH |
| `pluck` | Claude Code + pluck MCP (this project) |

Additional third-party MCP code-search tools may be benchmarked separately
under `benchmarks/external/`; the public dashboard reports the bash and
ripgrep baselines.

Same model (`claude-sonnet-4-6`), same prompt, same starting commit.

## Scenarios

| ID | Category | Intent |
|----|----------|--------|
| `search/find-auth` | search | Locate auth code from a high-level description |
| `fix/auth-token-expiry` | fix | Off-by-one token expiry bug |
| `refactor/add-user-field` | refactor | Add a field to a type and propagate across the codebase |
| `explore/payment-flow` | explore | Summarize a multi-file domain flow |
| `review/pr-diff` | review | Review a diff for issues |

Each scenario carries:

- `prompt` — the user message to the agent
- `success_criteria` — files changed, diff patterns, tests pass, LLM-judge rubric
- `budget` — max tool calls, max tokens, max wall time

## Metrics

| Metric | Source |
|--------|--------|
| Input tokens | `tiktoken-rs` over the actual API payloads |
| Output tokens | Same |
| Tool call count | Captured from Claude Code session logs |
| Wall clock | Driver stopwatch |
| Task success | Deterministic checks + LLM-as-judge (≥ 0.8) |
| Cost (USD) | tokens × Sonnet 4.6 price |

## Headline chart (target)

Input tokens per session, lower is better. Headline savings use eligible
retrieval workloads; control cases such as tiny files, first dedup calls, and
byte-exact raw reads are published separately and expected to show little or no
savings.

```
Scenario × Repo:    fix-bug × medium

  Bash only       ████████████████████████████████  48,000
  ripgrep         █████████████████████████████      44,000
  pluck           ████                                5,000   (-90%)
```

Full numbers under `benchmarks/results/` after every nightly run.

## Headline claims

| Claim | Scope |
|-------|-------|
| 84-88% fewer tokens | `pluck.read` outline mode on eligible medium-to-XL code reads |
| 71% shorter output | Median `pluck.digest` compression across 6 build / test / CI fixtures |
| 23% session savings | 5-query session-dedup bench, including first-call control cases |

## Retrieval Quality

`crates/pluck-core/benches/recall.rs` reads
`benchmarks/quality/recall.json` and reports Recall@5, Recall@10, MRR,
NDCG@10, per-query ranks, and per-language breakdowns. Repo-backed datasets
are skipped when their checkout is unavailable, so CI can compile the bench
without requiring large external repos.

## CI

`.github/workflows/ci.yml` runs on PRs and `main` pushes. It runs tests,
benchmark harnesses, uploads `benchmarks/results/`, and runs
`scripts/regression-gate.py` when engine-core or benchmark files change.

`.github/workflows/nightly-benchmark.yml` runs the full scenario suite on a
schedule or via manual dispatch. Output is uploaded as a build artifact and
also published to the stable `benchmark-results` GitHub Release as:

- `nightly-benchmark-results.tar.gz` — complete `benchmarks/results/` bundle
- `nightly-report.md` — markdown report from `pluck-bench report`
- `nightly-summary.md` — run inputs and scenario list

## Local run

```bash
./scripts/benchmark-local.sh fix/auth-token-expiry bash,pluck
```

Runs the chosen runners against one scenario and prints a markdown table.

## Honesty policy

- Scenarios where pluck does **not** win are published with the same prominence
  as wins.
- "Token savings" headline always references a specific scenario + repo size.
- The harness, repos (as submodules), and runner configs are open — anyone can
  re-run the suite.
