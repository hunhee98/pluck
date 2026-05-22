#!/usr/bin/env python3
"""parse.py — aggregate raw claude -p JSON outputs into a comparison CSV.

Reads every `results/<query>.<arm>.runN.json`, extracts the four
metrics CodeGraph reported (cost / tokens / time / tool_calls), takes
the median per (query, arm), and prints both the raw per-run and
aggregated CSVs to stdout.

Usage:
  python3 parse.py                    # full report
  python3 parse.py --query <qid>      # one query only
  python3 parse.py --csv aggregated   # only the aggregated CSV
  python3 parse.py --csv raw          # only the per-run CSV
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import statistics
import sys
from pathlib import Path

BENCH_ROOT = Path(__file__).resolve().parent.parent
RESULTS_DIR = BENCH_ROOT / "results"


def parse_one(path: Path) -> dict | None:
    """Pull cost/tokens/duration/tool_calls from one claude -p json."""
    try:
        with path.open() as f:
            data = json.load(f)
    except (json.JSONDecodeError, OSError) as exc:
        print(f"[parse] WARN unreadable {path.name}: {exc}", file=sys.stderr)
        return None

    # claude -p --output-format json shape (Claude Code v2.x):
    #   total_cost_usd, total_tokens (or usage.{input_tokens, output_tokens, …}),
    #   duration_ms (or duration_api_ms / duration_wall_ms), tool_calls (count or list).
    # We accept a few aliases so a slight schema drift doesn't break the
    # whole bench.
    def first(*keys):
        for k in keys:
            if k in data and data[k] is not None:
                return data[k]
        return None

    usage = data.get("usage") or {}

    cost = first("total_cost_usd", "cost_usd")
    tokens = first("total_tokens", "tokens_total") or (
        (usage.get("input_tokens") or 0) + (usage.get("output_tokens") or 0)
    )
    duration_ms = first("duration_ms", "duration_wall_ms", "duration_api_ms")
    tool_calls = first("tool_calls", "num_tool_calls")
    if isinstance(tool_calls, list):
        tool_calls = len(tool_calls)

    stem = path.stem  # e.g. vscode_extension_host.with-pluck.run3
    parts = stem.rsplit(".", 2)
    if len(parts) != 3:
        print(f"[parse] WARN unparseable filename {path.name}", file=sys.stderr)
        return None
    query_id, arm, run_part = parts
    if not run_part.startswith("run"):
        print(f"[parse] WARN unparseable run index in {path.name}", file=sys.stderr)
        return None
    run_idx = int(run_part[3:])

    return {
        "query": query_id,
        "arm": arm,
        "run": run_idx,
        "cost_usd": cost,
        "tokens": tokens,
        "duration_ms": duration_ms,
        "tool_calls": tool_calls,
    }


def median_of(values):
    clean = [v for v in values if v is not None]
    return statistics.median(clean) if clean else None


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--query", help="restrict to a single query id")
    ap.add_argument(
        "--csv",
        choices=("raw", "aggregated", "both"),
        default="both",
        help="which CSV to emit",
    )
    args = ap.parse_args()

    raw_rows = []
    for path in sorted(RESULTS_DIR.glob("*.json")):
        row = parse_one(path)
        if row is None:
            continue
        if args.query and row["query"] != args.query:
            continue
        raw_rows.append(row)

    if not raw_rows:
        print("[parse] no results/*.json found", file=sys.stderr)
        sys.exit(1)

    if args.csv in ("raw", "both"):
        print("# Per-run raw metrics")
        writer = csv.DictWriter(
            sys.stdout,
            fieldnames=["query", "arm", "run", "cost_usd", "tokens", "duration_ms", "tool_calls"],
        )
        writer.writeheader()
        for row in raw_rows:
            writer.writerow(row)
        print()

    if args.csv in ("aggregated", "both"):
        by_pair: dict[tuple[str, str], list[dict]] = {}
        for row in raw_rows:
            by_pair.setdefault((row["query"], row["arm"]), []).append(row)

        print("# Median per (query, arm)")
        writer = csv.DictWriter(
            sys.stdout,
            fieldnames=[
                "query",
                "arm",
                "n_runs",
                "median_cost_usd",
                "median_tokens",
                "median_duration_ms",
                "median_tool_calls",
            ],
        )
        writer.writeheader()
        for (query, arm), rows in sorted(by_pair.items()):
            writer.writerow(
                {
                    "query": query,
                    "arm": arm,
                    "n_runs": len(rows),
                    "median_cost_usd": median_of(r["cost_usd"] for r in rows),
                    "median_tokens": median_of(r["tokens"] for r in rows),
                    "median_duration_ms": median_of(r["duration_ms"] for r in rows),
                    "median_tool_calls": median_of(r["tool_calls"] for r in rows),
                }
            )
        print()

        # Print head-to-head delta table so the report is readable at
        # a glance without dropping into a spreadsheet.
        print("# (with-pluck) vs (empty) deltas — per query")
        print("query,delta_cost_pct,delta_tokens_pct,delta_duration_pct,delta_tool_calls_pct")
        queries = sorted({row["query"] for row in raw_rows})
        for q in queries:
            pluck = by_pair.get((q, "with-pluck"))
            empty = by_pair.get((q, "empty"))
            if not pluck or not empty:
                continue

            def pct(metric: str) -> str:
                p = median_of(r[metric] for r in pluck)
                e = median_of(r[metric] for r in empty)
                if p is None or e is None or e == 0:
                    return ""
                return f"{(p - e) / e * 100:+.1f}"

            print(
                ",".join(
                    [
                        q,
                        pct("cost_usd"),
                        pct("tokens"),
                        pct("duration_ms"),
                        pct("tool_calls"),
                    ]
                )
            )


if __name__ == "__main__":
    main()
