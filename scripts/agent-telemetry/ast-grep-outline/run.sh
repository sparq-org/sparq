#!/usr/bin/env bash
# One-shot driver for the ast-grep+outline read-payload A/B (bead
# sq-lhwo.4, epic sq-lhwo). 🤖 SPARQ agent.
#
# RUNS the firm A/B per research/dogfooding-sparq-knowledge-graph.md §5.1-§5.6 over
# the FROZEN stratified task set, emitting the §5.6 verdict object. Every number it
# prints is work-box / runtime-only and NON-CANONICAL — NEVER paste one into
# committed markdown (check-no-perf-numbers.py would flag it). The committed
# artifacts are the harness CODE, the PREREG, the frozen tasks, and the schema.
#
# Usage:  scripts/agent-telemetry/ast-grep-outline/run.sh [OUTDIR]
# Output (default OUTDIR=/tmp/astgrep-outline-ab):
#   rows.jsonl      per-task paired effective-token rows (measure_ab.py)
#   quality.json    deterministic §5.2 quality_delta (grade.py)
#   verdict.json    the §5.6 verdict object (ab_stats.py)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTDIR="${1:-/tmp/astgrep-outline-ab}"
mkdir -p "$OUTDIR"

# ast-grep may be installed but off PATH (see .claude/skills/ast-grep/SKILL.md §0).
export PATH="$PATH:$HOME/.cargo/bin"

echo "==> [1/4] verify tools + frozen-corpus integrity"
python3 "$HERE/measure_ab.py" --check

echo "==> [2/4] measure read-payload A/B over the frozen tasks (NON-CANONICAL)"
python3 "$HERE/measure_ab.py" --out "$OUTDIR/rows.jsonl"

echo "==> [3/4] deterministic arm-blinded quality grade (list/negative strata)"
python3 "$HERE/grade.py" --out "$OUTDIR/quality.json" >/dev/null

# Derive the break-even cost model from the MEASURED rows (never a frozen number):
#   c_docread = median arm-A effective tokens per use,
#   c_query   = median arm-B effective tokens per use,
#   c_ingest  = the one-time amortisable cost. The CLI route builds NO index, so the
#               only one-time cost is the ast-grep SKILL definition that must sit in
#               the prompt. We set it to one arm-A median as a conservative upper
#               bound (the real skill-def is far smaller), so break-even is not
#               under-stated. realistic_horizon = how many times a "list/locate/shape"
#               question recurs before the file changes (a representative window).
python3 - "$OUTDIR/rows.jsonl" "$OUTDIR/cost-model.json" <<'PY'
import json, sys
from statistics import median
rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
a = median(r["arm_a_eff"] for r in rows)
b = median(r["arm_b_eff"] for r in rows)
json.dump({
    "c_docread": a, "c_query": b, "c_ingest": a, "c_maintain_per_use": 0.0,
    "realistic_horizon": 1000,
    "_note": "MEASURED runtime cost model (NON-CANONICAL); c_ingest is a conservative "
             "upper bound = one arm-A median (real ast-grep skill-def is smaller).",
}, open(sys.argv[2], "w"), indent=2)
PY

echo "==> [4/4] emit the §5.6 verdict object (kill criteria applied mechanically)"
python3 "$HERE/ab_stats.py" "$OUTDIR/rows.jsonl" \
    --quality-json "$OUTDIR/quality.json" \
    --cost-model "$OUTDIR/cost-model.json" \
    --out "$OUTDIR/verdict.json"

echo
echo "verdict -> $OUTDIR/verdict.json (runtime-only, NON-CANONICAL — do not commit numbers)"
