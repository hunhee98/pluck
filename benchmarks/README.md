# Benchmarks

Reproducible. Public. Run nightly in CI; runnable locally in one command.

```bash
./scripts/benchmark-local.sh fix/auth-token-expiry bash,pluck
```

## Layout

```
benchmarks/
├── scenarios/        one YAML per benchmark scenario
│   ├── search/
│   ├── fix/
│   ├── refactor/
│   ├── explore/
│   └── review/
├── runners/          one YAML per agent setup
│   ├── bash.yaml         (Claude Code + Bash only — baseline)
│   ├── ripgrep.yaml      (Claude Code + ripgrep)
│   └── pluck.yaml        (Claude Code + pluck MCP)
├── quality/          labeled retrieval datasets for recall / NDCG benches
├── repos/            git submodules pointing at fixed revisions of test repos
└── results/          per-run JSON output (gitignored after Phase 0)
```

## Scenario format

See `scenarios/fix/auth-token-expiry.yaml` for the canonical example. Fields:

| Field | Purpose |
|-------|---------|
| `id` | Unique slug |
| `category` | search / fix / refactor / explore / review |
| `repo` | Repo slug under `benchmarks/repos/` |
| `repo_revision` | Commit hash for reproducibility |
| `prompt` | Exact prompt sent to the agent |
| `success_criteria` | Deterministic checks + LLM-judge rubric |
| `budget` | Caps on tool calls, tokens, wall time |
| `repetitions` | N runs per (scenario, runner) cell |

## Runner format

| Field | Purpose |
|-------|---------|
| `name` | Slug shown in reports |
| `agent` | `claude-code` for now |
| `model` | Anthropic model ID |
| `mcp_servers` | MCP entries to register before the session |
| `tools` | Allowed tool list |
| `prompt_suffix` | Optional snippet appended to the prompt (e.g. "prefer pluck.*") |

## Retrieval quality format

`quality/recall.json` is the v0.3 labeled retrieval suite. Each case has a
natural-language query and one or more relevant `(path, symbol)` labels with a
graded relevance score:

| Relevance | Meaning |
|-----------|---------|
| `3` | Primary target for the query |
| `2` | Acceptable alternate target |
| `1` | Weakly relevant supporting target |

Run it locally with:

```bash
PLUCK_RUN_RECALL_BENCH=1 cargo bench -p pluck-core --bench recall
```

The bench prints Recall@5, Recall@10, MRR, and NDCG@10, then writes
`benchmarks/results/recall-quality.json`.

Repo-backed datasets set `kind: "repo-backed"` plus `root_env` and
`root_default`. The bench indexes all supported source extensions under that
root, skipping common build/dependency directories. This lets the same format
cover tokio, django, next.js, and future fixture repos without adding new Rust
branches per project.
