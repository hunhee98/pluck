# LLM-in-loop bench

Head-to-head: an AI coding agent (Claude Code, headless) answering the
same question against the same repo, **with** vs **without** pluck's
MCP server enabled.

Methodology mirrors [CodeGraph's published bench][cg] so the seven
overlapping queries are directly comparable. Three additional queries
are pluck's own, chosen to exercise axes where pluck's design — hybrid
BM25 + static-embedding ranking, callgraph traversal, config-format
coverage — is supposed to win and CodeGraph's text-only FTS5 graph is
not.

[cg]: https://github.com/colbymchenry/codegraph#benchmark-results

## Corpus

| Source | Repos | Queries |
|--------|-------|---------|
| CodeGraph verbatim | 7 (VS Code, Excalidraw, Django, Tokio, OkHttp, Gin, Alamofire) | 7 architecture-style "How does X do Y" |
| Pluck's own | + kubernetes/website | 3 (semantic intent, refactor impact, config-format coverage) |
| **Total** | 8 | 10 |

Each (query, arm) is run **N=4** times; the median across runs is the
reported number. Total runs: **10 × 2 × 4 = 80**.

## What gets compared

Per CodeGraph's published metric set:

- **cost** — `total_cost_usd` from `claude -p --output-format json`
- **tokens** — sum of input + output tokens
- **duration** — wall-clock ms
- **tool_calls** — count of every tool invocation across the session,
  including those inside sub-agents the model spawns

(`total_cost_usd` is informational for Max subscribers — billing is
covered by the subscription. The dollar figure is computed locally
from token counts.)

## Run

```bash
# 1. Clone repos and build the pluck index for each.
./scripts/setup-repos.sh

# 2. Smoke run (single query, both arms, one run each) to validate
#    plumbing before firing the full batch.
PLUCK_BENCH_ONLY_QUERY=gin_middleware \
PLUCK_BENCH_RUNS_PER_ARM=1 \
    ./scripts/run-all.sh

# 3. Full run (~80 runs, paced so a Max-plan burst doesn't rate-limit).
./scripts/run-all.sh

# 4. Aggregate raw JSON into CSV and a per-query delta table.
python3 scripts/parse.py
```

## Cost

`claude -p` on Max 20x ($200/mo) is covered by subscription — no
separate billing. The 80 runs do consume quota against the shared
weekly/monthly limit. CodeGraph reported $0.36–$1.04 per run; a full
80-run pass is roughly $30–80 equivalent value. Pace via
`PLUCK_BENCH_SLEEP_SECONDS=30` (or higher) if a burst trips the
rate-limiter.

## Honest reporting policy

- Every raw run's JSON is committed under `results/`. Anyone can
  re-parse without re-running.
- If pluck's median is **worse** than the empty arm on a query, that
  row is reported as-is. We do not cherry-pick.
- The "pluck's own three" queries are flagged as such (the queries.yaml
  `source` field is `pluck_bench_v1` vs `codegraph_bench_v1`). If we
  publish a summary that omits the head-to-head rows, the file path
  makes the omission visible.

## Re-running CodeGraph for direct comparison

This bench only measures pluck (with vs without). CodeGraph numbers
come from their published table; we have not re-run their bench
ourselves. To make a direct pluck-vs-CodeGraph claim, both arms would
need to run on the same machine on the same day — a separate task
not in scope for v0.6.
