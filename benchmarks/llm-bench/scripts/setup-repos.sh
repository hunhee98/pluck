#!/usr/bin/env bash
# setup-repos.sh — clone every bench repo + build a pluck index for it.
#
# Indexing happens at setup time, not during the bench, so the timed
# arm only measures task completion (matches CodeGraph methodology).
#
# Defaults:
#   $PLUCK_BENCH_REPOS_DIR  ../repos     (where this script clones into)
#   $PLUCK_BENCH_DEPTH       1           (shallow clone; rev tracked by ref)

set -euo pipefail

BENCH_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPOS_DIR="${PLUCK_BENCH_REPOS_DIR:-$BENCH_ROOT/repos}"
DEPTH="${PLUCK_BENCH_DEPTH:-1}"

mkdir -p "$REPOS_DIR"

python3 - "$REPOS_DIR" "$DEPTH" <<'PY'
import os
import shlex
import subprocess
import sys
import yaml

repos_dir, depth = sys.argv[1], sys.argv[2]
bench_root = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
# When invoked via the heredoc the __file__ trick above is unreliable;
# fall back to the path passed by the shell wrapper above.
bench_root = os.environ.get("BENCH_ROOT_OVERRIDE", bench_root)
with open(os.path.join(repos_dir, "..", "repos.yaml")) as f:
    cfg = yaml.safe_load(f)

for r in cfg["repos"]:
    rid = r["id"]
    target = os.path.join(repos_dir, rid)
    if os.path.exists(target):
        print(f"[setup] {rid:25s} already cloned at {target}")
    else:
        print(f"[setup] {rid:25s} cloning {r['url']} (--depth {depth}, branch {r['rev']})")
        subprocess.run(
            ["git", "clone", "--depth", str(depth),
             "--branch", r["rev"], r["url"], target],
            check=True,
        )

    # Build index for the with-pluck arm. The empty arm doesn't use it.
    print(f"[setup] {rid:25s} pluck index ...")
    subprocess.run(["pluck", "index", target], check=True)

print("[setup] all repos cloned and indexed.")
PY
