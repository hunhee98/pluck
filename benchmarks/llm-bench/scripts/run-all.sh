#!/usr/bin/env bash
# run-all.sh — orchestrate the full matrix with rate-limit-aware pacing.
#
# Default: 4 runs per (query, arm). 10 queries × 2 arms × 4 runs = 80 runs.
# A short sleep separates calls so a burst doesn't trip Max-plan rate
# limits. Results land in `results/<query>.<arm>.runN.json`; existing
# files are skipped so the script is resumable.
#
# Honored env vars:
#   PLUCK_BENCH_RUNS_PER_ARM   default 4
#   PLUCK_BENCH_SLEEP_SECONDS  default 15 (between runs)
#   PLUCK_BENCH_ONLY_QUERY     limit to one query id (for smoke runs)
#   PLUCK_BENCH_ONLY_ARM       limit to one arm (with-pluck | empty)

set -euo pipefail

BENCH_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUNS="${PLUCK_BENCH_RUNS_PER_ARM:-4}"
SLEEP_S="${PLUCK_BENCH_SLEEP_SECONDS:-15}"
ONLY_QUERY="${PLUCK_BENCH_ONLY_QUERY:-}"
ONLY_ARM="${PLUCK_BENCH_ONLY_ARM:-}"

QUERY_IDS=$(python3 - <<PY
import yaml
with open("$BENCH_ROOT/queries.yaml") as f:
    print("\n".join(q["id"] for q in yaml.safe_load(f)["queries"]))
PY
)

total=0
skipped=0
ran=0

for qid in $QUERY_IDS; do
    if [ -n "$ONLY_QUERY" ] && [ "$qid" != "$ONLY_QUERY" ]; then
        continue
    fi
    for arm in with-pluck empty; do
        if [ -n "$ONLY_ARM" ] && [ "$arm" != "$ONLY_ARM" ]; then
            continue
        fi
        for i in $(seq 1 "$RUNS"); do
            total=$((total + 1))
            out="$BENCH_ROOT/results/$qid.$arm.run$i.json"
            if [ -s "$out" ]; then
                skipped=$((skipped + 1))
                echo "[skip] $qid / $arm / run$i (already exists)"
                continue
            fi
            "$BENCH_ROOT/scripts/run-one.sh" "$qid" "$arm" "$i"
            ran=$((ran + 1))
            echo "[pace] sleeping ${SLEEP_S}s..."
            sleep "$SLEEP_S"
        done
    done
done

echo
echo "[run-all] total slots: $total, ran: $ran, skipped (already done): $skipped"
