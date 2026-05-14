#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

SCENARIO="${1:-fix/auth-token-expiry}"
RUNNERS="${2:-bash,pluck}"
REPS="${REPS:-3}"

if [[ -z "${ANTHROPIC_API_KEY:-}" ]]; then
  echo "ANTHROPIC_API_KEY not set; aborting." >&2
  exit 1
fi

echo "[pluck-bench] Building..."
cargo build --release --bin pluck-bench --bin pluckd

IFS=',' read -ra RUNNER_ARR <<<"$RUNNERS"
for r in "${RUNNER_ARR[@]}"; do
  echo
  echo "=== scenario=$SCENARIO runner=$r reps=$REPS ==="
  cargo run --release --quiet --bin pluck-bench -- run \
    --scenario "benchmarks/scenarios/$SCENARIO.yaml" \
    --runner   "benchmarks/runners/$r.yaml" \
    --repetitions "$REPS" \
    --output   benchmarks/results/
done

echo
echo "[pluck-bench] Report:"
cargo run --release --quiet --bin pluck-bench -- report --markdown
