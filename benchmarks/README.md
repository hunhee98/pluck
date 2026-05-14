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
│   ├── a prior-art code search tool.yaml    (Claude Code + a prior-art code search tool MCP)
│   ├── .yaml       (Claude Code +  MCP)
│   └── pluck.yaml        (Claude Code + pluck MCP)
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
