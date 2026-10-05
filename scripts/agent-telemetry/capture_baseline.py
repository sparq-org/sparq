#!/usr/bin/env python3
# Baseline capture for the metrics harness (bead sq-lhwo.1, epic sq-lhwo).
#
# Runs metrics_row.py over the last N merged PRs to establish the current-state
# distribution for each metric -- the "before" the §5 A/B compares against. Token
# columns are deliberately UNATTRIBUTED in the baseline (per-PR token attribution is
# not derivable post-hoc -- see metrics_row.py honesty notes); the baseline captures
# the GitHub/git/roborev-derived columns, which ARE the gate signals (first-shot,
# rework, CI-first-pass, pushback). The A/B fills token columns per-task.
#
# Output: metrics/baseline.jsonl (committed DATA -- not a perf claim; kept out of the
# no-perf-numbers scan scope via .gitignore-style exemption documented in the README).
#
# USAGE
#   capture_baseline.py --count 25 --out metrics/baseline.jsonl [--repo owner/name]
#   capture_baseline.py --prs 1057 1056 ... --out metrics/baseline.jsonl

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import metrics_row as mr  # noqa: E402


def merged_prs(count: int, repo: str | None) -> list[int]:
    cmd = ["gh", "pr", "list", "--state", "merged", "--limit", str(count), "--json",
           "number"]
    if repo:
        cmd += ["--repo", repo]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
    return [int(p["number"]) for p in json.loads(out)]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Capture a metrics baseline over merged PRs.")
    ap.add_argument("--count", type=int, default=25, help="number of recent merged PRs")
    ap.add_argument("--prs", type=int, nargs="*", help="explicit PR numbers (overrides --count)")
    ap.add_argument("--repo", help="owner/name (default: current repo)")
    ap.add_argument("--out", default="metrics/baseline.jsonl")
    args = ap.parse_args(argv)

    prs = args.prs or merged_prs(args.count, args.repo)
    os.makedirs(os.path.dirname(os.path.abspath(args.out)) or ".", exist_ok=True)
    rows = []
    for pr in prs:
        try:
            ns = argparse.Namespace(
                pr=pr, bead=None, arm=None, agent_type=None, repo=args.repo,
                telemetry_json=None, transcript=None,
                price_input=None, price_output=None,
                price_cache_read=None, price_cache_write=None,
            )
            row = mr.collect_row(ns)
            # Strip the per-PR timestamp so the committed baseline is reproducible
            # data, not a time-varying artifact.
            row.pop("collected_at", None)
            rows.append(row)
            print(f"PR {pr}: first_shot={row['first_shot']} "
                  f"no_rework={row['no_rework']} ci={row['ci_first_green']} "
                  f"roborev={row['roborev_findings']}", file=sys.stderr)
        except mr.CollectorError as exc:
            print(f"PR {pr}: SKIP ({exc})", file=sys.stderr)

    # Sort by PR number for a stable, diff-friendly committed file.
    rows.sort(key=lambda r: r["pr"])
    with open(args.out, "w", encoding="utf-8") as fh:
        for r in rows:
            fh.write(json.dumps(r, sort_keys=True) + "\n")
    print(f"wrote {len(rows)} baseline rows to {args.out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
