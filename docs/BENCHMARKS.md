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
| `a prior-art code search tool` | Claude Code + a prior-art code search tool MCP |
| `` | Claude Code +  MCP |
| `pluck` | Claude Code + pluck MCP (this project) |

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

Input tokens per session, lower is better.

```
Scenario × Repo:    fix-bug × medium

  Bash only       ████████████████████████████████  48,000
  ripgrep         █████████████████████████████      44,000
  a prior-art code search tool       ████████████████                   24,000
            █████████████                      19,000
  pluck           ████                                5,000   (-90%)
```

Full numbers under `benchmarks/results/` after every nightly run.

## CI

`.github/workflows/benchmark.yml` runs weekly. Output is uploaded as a build
artifact and published to the dashboard. Regressions of more than 10% on any
scenario open an issue automatically.

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
