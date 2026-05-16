#!/usr/bin/env python3
"""Run the cheap-and-deterministic benches, parse their stdout, compare
each measured number against `benchmarks/baseline.json`, exit non-zero
on any regression beyond the per-metric tolerance.

Stdlib-only — no `pip install` step required in CI.

Usage:
    scripts/regression-gate.py                  # run all gated benches
    scripts/regression-gate.py --update         # rewrite baseline with
                                                # the new measurements
                                                # (use only after an
                                                # intentional improvement)
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASELINE_PATH = ROOT / "benchmarks" / "baseline.json"


def run_bench(crate: str, name: str) -> str:
    """Invoke `cargo bench -p <crate> --bench <name>` and return stdout +
    stderr merged. Failures surface as a non-zero exit + the bench
    output, which we print before re-raising."""
    cmd = ["cargo", "bench", "-p", crate, "--bench", name]
    print(f"\n--- running: {' '.join(cmd)} ---", flush=True)
    proc = subprocess.run(cmd, cwd=ROOT, text=True, capture_output=True)
    if proc.returncode != 0:
        print(proc.stdout)
        print(proc.stderr, file=sys.stderr)
        raise SystemExit(f"bench {crate}/{name} failed with exit code {proc.returncode}")
    return proc.stdout


def extract(pattern: str, text: str, group: int = 1) -> float:
    """Apply a regex to text and return float(match.group(group)). Raises
    `ValueError` if the pattern doesn't match — the caller will surface
    that as a "bench output shape changed" failure, which is itself a
    regression we want to know about."""
    m = re.search(pattern, text, flags=re.DOTALL)
    if not m:
        raise ValueError(f"pattern not found in bench output: {pattern!r}")
    return float(m.group(group).replace(",", ""))


def measure_chunker() -> dict[str, float]:
    """Criterion output: 'chunk_medium (500 lines) ... time: [low median high]'"""
    out = run_bench("pluck-core", "chunker")
    # Median is the second number inside [...].
    median = extract(
        r"chunk_medium \(500 lines\).*?time:\s*\[\s*[\d.]+\s*ms\s+([\d.]+)\s*ms",
        out,
    )
    return {"chunker_medium_ms_p50": median}


def measure_indexer() -> dict[str, float]:
    """Custom bench, markdown table row (8 columns, no bold markers):
    | medium (500 files) | 500 | 3500 | 1287 ms | 388 | 2719 | 0.06 ms | 0.43 ms |
    Columns: label | files | chunks | index_time | files/s | chunks/s | warm ms | cold ms
    """
    out = run_bench("pluck-core", "indexer")
    # Capture every numeric field on the medium row.
    row = re.search(
        r"medium \(500 files\)\s*\|\s*(\d+)\s*\|\s*(\d+)\s*\|\s*(\d+)\s*ms\s*\|\s*(\d+)\s*\|\s*(\d+)\s*\|\s*([\d.]+)\s*ms\s*\|\s*([\d.]+)\s*ms",
        out,
    )
    if not row:
        raise ValueError("indexer medium row pattern did not match")
    files_per_sec = float(row.group(4))
    warm_ms = float(row.group(6))
    return {
        "indexer_files_per_sec_medium": files_per_sec,
        "warm_search_p50_ms_medium": warm_ms,
    }


def measure_freshness() -> dict[str, float]:
    """| medium (500 files) | 10 | **183 ms** | 193 ms |"""
    out = run_bench("pluck-core", "freshness")
    p50 = extract(
        r"medium \(500 files\)\s*\|\s*\d+\s*\|\s*\*\*(\d+)\s*ms\*\*",
        out,
    )
    return {"freshness_p50_ms_medium": p50}


def measure_session_dedup() -> dict[str, float]:
    """| Σ | total | **9073** | **5090** | **44%** |"""
    out = run_bench("pluck-mcp", "session_dedup")
    savings = extract(
        r"Σ\s*\|\s*total\s*\|\s*\*\*\d+\*\*\s*\|\s*\*\*\d+\*\*\s*\|\s*\*\*(\d+)%\*\*",
        out,
    )
    return {"session_dedup_session_savings_pct": savings}


def measure_digest() -> dict[str, float]:
    """Median savings: **71%**  (gated metric: digest_savings_pct)"""
    out = run_bench("pluck-core", "digest")
    savings = extract(
        r"Median savings:\s*\*\*(\d+)%\*\*",
        out,
    )
    return {"digest_savings_pct": savings}


COLLECTORS = [
    measure_chunker,
    measure_indexer,
    measure_freshness,
    measure_session_dedup,
    measure_digest,
]


def collect_all() -> dict[str, float]:
    measurements: dict[str, float] = {}
    for fn in COLLECTORS:
        measurements.update(fn())
    return measurements


def compare(baseline: dict, measured: dict[str, float]) -> int:
    """Print a per-metric comparison and return the count of regressions."""
    print()
    print(f"{'metric':45} {'baseline':>12} {'measured':>12} {'delta':>10}  status")
    print("-" * 95)
    failures = 0
    for name, spec in baseline["metrics"].items():
        if name not in measured:
            print(f"{name:45}  (not measured this run)")
            continue
        baseline_v = float(spec["value"])
        measured_v = measured[name]
        direction = spec["regression_direction"]
        tol_pct = float(spec["tolerance_pct"])
        if baseline_v == 0:
            continue
        delta_pct = (measured_v - baseline_v) / baseline_v * 100.0
        regressed = False
        max_value = spec.get("max_value")
        min_value = spec.get("min_value")
        if direction == "higher":
            if max_value is not None:
                regressed = measured_v > float(max_value)
            else:
                regressed = delta_pct > tol_pct
        elif direction == "lower":
            if min_value is not None:
                regressed = measured_v < float(min_value)
            else:
                regressed = delta_pct < -tol_pct
        status = "FAIL" if regressed else "ok"
        unit = spec.get("unit", "")
        print(
            f"{name:45} {baseline_v:>10.2f}{unit:>2} {measured_v:>10.2f}{unit:>2} {delta_pct:>+8.1f}%  {status}"
        )
        if regressed:
            failures += 1
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--update",
        action="store_true",
        help="rewrite the baseline with the freshly measured values",
    )
    args = parser.parse_args()

    if not BASELINE_PATH.exists():
        raise SystemExit(f"baseline file missing: {BASELINE_PATH}")
    baseline = json.loads(BASELINE_PATH.read_text())

    measured = collect_all()

    failures = compare(baseline, measured)

    if args.update:
        for name, value in measured.items():
            if name in baseline["metrics"]:
                baseline["metrics"][name]["value"] = round(value, 2)
        BASELINE_PATH.write_text(json.dumps(baseline, indent=2) + "\n")
        print(f"\nbaseline updated in place: {BASELINE_PATH}")
        return 0

    if failures:
        print(f"\n✗ {failures} regression(s). Investigate before merging.")
        return 1
    print("\n✓ all gated metrics within tolerance.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
