#!/usr/bin/env bash
# run-one.sh — single bench arm execution.
#
# Usage:
#   run-one.sh <query-id> <arm> <run-index>
#   query-id   matches an entry's `id` in queries.yaml (e.g. vscode_extension_host)
#   arm        with-pluck | empty
#   run-index  1..N (used to disambiguate repeat runs for median)
#
# Output: results/<query-id>.<arm>.run<N>.json (raw `claude -p` json output)
#
# Honored env vars:
#   PLUCK_BENCH_REPOS_DIR   ../repos       (where setup-repos.sh cloned)
#   PLUCK_BENCH_MODEL       (optional; passed to `claude --model`)

set -euo pipefail

if [ $# -ne 3 ]; then
    echo "usage: $0 <query-id> <with-pluck|empty> <run-index>" >&2
    exit 2
fi

QUERY_ID="$1"
ARM="$2"
RUN_IDX="$3"

BENCH_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPOS_DIR="${PLUCK_BENCH_REPOS_DIR:-$BENCH_ROOT/repos}"
RESULTS_DIR="$BENCH_ROOT/results"
mkdir -p "$RESULTS_DIR"

# Read repo id + question text from queries.yaml.
read -r REPO_ID QUESTION_FILE <<<"$(python3 - "$BENCH_ROOT" "$QUERY_ID" <<'PY'
import os, sys, tempfile, yaml
bench_root, qid = sys.argv[1], sys.argv[2]
with open(os.path.join(bench_root, "queries.yaml")) as f:
    q = yaml.safe_load(f)
for entry in q["queries"]:
    if entry["id"] == qid:
        # Write question to a temp file so multi-line / quoted content
        # survives the shell hand-off unchanged.
        tmp = tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False)
        tmp.write(entry["question"].strip())
        tmp.close()
        print(entry["repo"], tmp.name)
        sys.exit(0)
print(f"no such query: {qid}", file=sys.stderr)
sys.exit(1)
PY
)"

REPO_PATH="$REPOS_DIR/$REPO_ID"
if [ ! -d "$REPO_PATH" ]; then
    echo "repo not cloned: $REPO_PATH (run setup-repos.sh first)" >&2
    exit 1
fi

# Resolve MCP config.
case "$ARM" in
    empty)
        MCP_CONFIG="$BENCH_ROOT/configs/empty.mcp.json"
        ;;
    with-pluck)
        MCP_CONFIG=$(mktemp -t with-pluck-XXXXXX.mcp.json)
        sed "s|__REPO_PATH__|$REPO_PATH|g" \
            "$BENCH_ROOT/configs/with-pluck.mcp.json.template" > "$MCP_CONFIG"
        ;;
    *)
        echo "unknown arm: $ARM (expected with-pluck | empty)" >&2
        exit 2
        ;;
esac

OUTPUT="$RESULTS_DIR/$QUERY_ID.$ARM.run$RUN_IDX.json"
echo "[run] $QUERY_ID / $ARM / run$RUN_IDX -> $OUTPUT"

MODEL_ARG=()
if [ -n "${PLUCK_BENCH_MODEL:-}" ]; then
    MODEL_ARG=(--model "$PLUCK_BENCH_MODEL")
fi

# Run claude -p inside the repo dir so the agent's default cwd matches
# the codebase under question. `${MODEL_ARG[@]+"${MODEL_ARG[@]}"}` is the
# set -u-safe expansion of an array that may be empty.
START_TS=$(date +%s)
(
    cd "$REPO_PATH"
    claude -p "$(cat "$QUESTION_FILE")" \
        --strict-mcp-config \
        --mcp-config "$MCP_CONFIG" \
        --output-format json \
        ${MODEL_ARG[@]+"${MODEL_ARG[@]}"} \
        > "$OUTPUT"
)
END_TS=$(date +%s)
echo "[run] $QUERY_ID / $ARM / run$RUN_IDX done in $((END_TS - START_TS))s"

# Tidy temp files.
rm -f "$QUESTION_FILE"
[ "$ARM" = "with-pluck" ] && rm -f "$MCP_CONFIG"
