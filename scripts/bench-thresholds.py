#!/usr/bin/env python3
# Empirical benchmark-noise threshold deriver (noise-reduction-bench-selfmonitor).
#
# WHY — the benchmark self-monitoring two-zone design (maintainer-directed). The per-PR /
# merge github-action-benchmark step compares each metric's current value against the
# published `benchmark-data` series and, in the maintainer's design, gates in TWO zones:
#   * SOFT (alert-threshold): a regression that COULD be shared-runner noise — it does NOT
#     fail CI and does NOT @-mention anyone; it auto-files/updates a deduped bench-flake
#     issue (see scripts/bench-triage.py).
#   * HARD (fail-threshold + fail-on-alert): a regression clearly BEYOND any observed runner
#     noise — the benchmark step FAILS CI so an agent must fix it before merge.
#
# The two thresholds MUST be derived empirically from the historical run-to-run variance of
# the timing metrics on largely-unchanged code, NOT invented. This script recomputes them
# from the `benchmark-data` history branch so the values baked into the workflow YAML can be
# re-derived + justified at any time. It is NOT run in the gating path — it is the audit /
# refresh tool the workflow comments cite. (No hard-coded perf numbers land in markdown; the
# thresholds live in the workflow YAML only, and this script reproduces them on demand.)
#
# github-action-benchmark semantics: for a `customSmallerIsBetter` metric it computes the
# ratio (current / previous). A threshold of `150%` means "alert when current >= 1.5x
# previous" i.e. a +50% regression. So the natural noise quantity is the run-to-run RELATIVE
# CHANGE abs(v2 - v1) / v1 between adjacent commits' values for the SAME metric.
#
# DERIVATION POLICY (documented so the numbers are reproducible, not magical):
#   soft alert-threshold  = 100% + round_up(p95 timing swing + ~30pp headroom)   [>= 125%]
#   hard fail-threshold   = 100% + clamp(worst observed timing swing, ...)         [>= 150%]
#                           capped at 200% so a genuine 2x regression is still caught
#                           (2x-worst-swing is uselessly loose once the worst swing is a
#                           microsecond-scale flake); floored at 150% per the directive.
#
# USAGE
#   scripts/bench-thresholds.py                      # derive from origin/benchmark-data
#   scripts/bench-thresholds.py --data-js path/to/data.js [--data-js ...]   # from files
#   scripts/bench-thresholds.py --json               # machine-readable output
#   scripts/bench-thresholds.py --self-test          # hermetic; no git, no network
from __future__ import annotations

import argparse
import json
import math
import re
import subprocess
import sys
from statistics import median

PROG = "bench-thresholds"

# The dashboards on the benchmark-data branch. dev/bench is the PER-COMMIT GitHub-hosted-
# runner series the PR/merge github-action-benchmark step compares against, so it is the
# noise source that governs the gating thresholds. The EC2 / nightly quiet-box series are
# far less noisy and reported separately (informational) when present.
_DEFAULT_DATA_PATHS = [
    "dev/bench/data.js",
    "dev/bench-ec2/data.js",
    "dev/bench-nightly/data.js",
]
_GATING_DASHBOARD = "dev/bench/data.js"

# Metric-name category heuristics (name-substring based; case-insensitive).
_BYTE_PAT = re.compile(r"(_bytes|bytes_per|bytes_per_triple|bytes_per_term|bundle_bytes)", re.I)
_TIMING_PAT = re.compile(r"(_ns|_ms|_us|ns_per|per_byte|latency|_secs|_s\b|throughput|qps)", re.I)
_COUNT_PAT = re.compile(r"(_rows|_triples|_terms|_chars|_count|_gates|recall)", re.I)


def log(msg: str) -> None:
    print(f"[{PROG}] {msg}", file=sys.stderr)


def parse_data_js(text: str) -> dict:
    """Parse a github-action-benchmark data.js (`window.BENCHMARK_DATA = {...};`) payload."""
    # Strip the assignment prefix + any trailing semicolon/whitespace.
    m = re.search(r"window\.BENCHMARK_DATA\s*=\s*", text)
    if m:
        text = text[m.end():]
    text = text.strip()
    if text.endswith(";"):
        text = text[:-1].strip()
    return json.loads(text)


def categorize(name: str) -> str:
    if _BYTE_PAT.search(name):
        return "byte"
    if _TIMING_PAT.search(name):
        return "timing"
    if _COUNT_PAT.search(name):
        return "count"
    return "other"


def adjacent_rel_changes(entries: list) -> dict:
    """metric-name -> list of abs relative changes between adjacent distinct-commit values.

    entries is one suite's list of run records (each with 'commit' and 'benches'). We order
    by date (github-action-benchmark appends chronologically), then for every adjacent pair of
    DISTINCT commits compute abs(v2 - v1) / v1 per metric. On largely-unchanged code this
    approximates the run-to-run noise; genuine perf-changing commits inflate it (conservative).
    """
    ordered = sorted(entries, key=lambda e: e.get("date", 0))
    per_metric_series: dict[str, list[tuple[str, float]]] = {}
    for e in ordered:
        commit_id = (e.get("commit") or {}).get("id") or str(e.get("date", ""))
        for b in e.get("benches", []):
            name = b.get("name")
            val = b.get("value")
            if name is None or not isinstance(val, (int, float)):
                continue
            per_metric_series.setdefault(name, []).append((commit_id, float(val)))
    changes: dict[str, list[float]] = {}
    for name, series in per_metric_series.items():
        out = []
        for (c1, v1), (c2, v2) in zip(series, series[1:]):
            if c1 == c2 or v1 == 0:
                continue
            out.append(abs(v2 - v1) / abs(v1))
        if out:
            changes[name] = out
    return changes


def _pct(x: float) -> float:
    return round(x * 100.0, 2)


def _quantile(sorted_vals: list[float], q: float) -> float:
    if not sorted_vals:
        return 0.0
    if len(sorted_vals) == 1:
        return sorted_vals[0]
    pos = q * (len(sorted_vals) - 1)
    lo = math.floor(pos)
    hi = math.ceil(pos)
    if lo == hi:
        return sorted_vals[lo]
    frac = pos - lo
    return sorted_vals[lo] * (1 - frac) + sorted_vals[hi] * frac


def summarize(changes: dict, category: str) -> dict:
    """Pool the relative-change fractions across all metrics in a category."""
    pool: list[float] = []
    for name, vals in changes.items():
        if categorize(name) == category:
            pool.extend(vals)
    pool.sort()
    if not pool:
        return {"category": category, "metrics": 0, "pairs": 0}
    return {
        "category": category,
        "metrics": sum(1 for n in changes if categorize(n) == category),
        "pairs": len(pool),
        "median_pct": _pct(median(pool)),
        "p90_pct": _pct(_quantile(pool, 0.90)),
        "p95_pct": _pct(_quantile(pool, 0.95)),
        "p99_pct": _pct(_quantile(pool, 0.99)),
        "max_pct": _pct(max(pool)),
    }


def derive_thresholds(timing_summary: dict, n_runs: int) -> dict:
    """Turn a timing-noise summary into github-action-benchmark soft/hard percentages.

    Returns {soft, hard, basis} where soft/hard are integer percentages (150 == 1.5x).
    Conservative fallback when the sample is too small (<5 runs) to characterise the tail.
    """
    if not timing_summary or timing_summary.get("pairs", 0) == 0 or n_runs < 5:
        return {
            "soft": 130,
            "hard": 200,
            "basis": (
                f"CONSERVATIVE FALLBACK — too little history "
                f"(runs={n_runs}, timing-pairs={timing_summary.get('pairs', 0)}) to derive "
                f"robustly; re-run bench-thresholds.py once more history accrues."
            ),
        }
    p95 = timing_summary["p95_pct"]
    worst = timing_summary["max_pct"]
    # Soft: p95 swing + ~30pp headroom, expressed as a github-action-benchmark ratio %.
    soft = int(math.ceil((100.0 + p95 + 30.0) / 5.0) * 5)  # round UP to nearest 5%
    soft = max(soft, 125)
    # Hard: above the worst observed noise swing, floored at 150%, capped at 200% (a genuine
    # 2x regression is still caught; 2x-worst is uselessly loose once worst is a µs flake).
    hard = int(math.ceil((100.0 + worst) / 5.0) * 5)
    hard = max(hard, 150)
    hard = min(hard, 200)
    if hard <= soft:
        hard = min(max(soft + 25, 150), 200)
    return {
        "soft": soft,
        "hard": hard,
        "basis": (
            f"soft={soft}% = 100 + p95 timing swing ({p95}%) + ~30pp headroom, rounded up to 5%; "
            f"hard={hard}% = above worst observed timing noise swing ({worst}%), floored 150% / "
            f"capped 200%; over {n_runs} runs."
        ),
    }


def git_show(ref: str, path: str) -> str | None:
    try:
        return subprocess.run(
            ["git", "show", f"{ref}:{path}"],
            check=True, capture_output=True, text=True,
        ).stdout
    except subprocess.CalledProcessError:
        return None


def load_from_git(ref: str) -> dict:
    """dashboard-path -> parsed data.js dict, for every dashboard present at `ref`."""
    # Best-effort fetch (no checkout) so origin/<ref> resolves in CI + locally.
    subprocess.run(
        ["git", "fetch", "origin", "benchmark-data:refs/remotes/origin/benchmark-data"],
        capture_output=True, text=True,
    )
    out = {}
    for p in _DEFAULT_DATA_PATHS:
        raw = git_show(ref, p)
        if raw is None:
            log(f"{p}: absent at {ref} (skipped)")
            continue
        try:
            out[p] = parse_data_js(raw)
        except (json.JSONDecodeError, ValueError) as e:
            log(f"{p}: parse failed ({e})")
    return out


def report(datasets: dict) -> dict:
    """Build the full report + the gating-dashboard thresholds."""
    result = {"dashboards": {}, "gating": None}
    for path, data in datasets.items():
        entries = data.get("entries", {})
        # A dashboard has ONE suite normally, but pool across all suites in the file.
        pooled_changes: dict[str, list[float]] = {}
        n_runs = 0
        for suite, suite_entries in entries.items():
            n_runs = max(n_runs, len(suite_entries))
            for name, vals in adjacent_rel_changes(suite_entries).items():
                pooled_changes.setdefault(f"{suite}::{name}", []).extend(vals)
        cats = {c: summarize(pooled_changes, c) for c in ("byte", "count", "timing", "other")}
        result["dashboards"][path] = {"runs": n_runs, "categories": cats}
        if path == _GATING_DASHBOARD:
            result["gating"] = {
                "dashboard": path,
                "runs": n_runs,
                "timing": cats["timing"],
                "thresholds": derive_thresholds(cats["timing"], n_runs),
            }
    if result["gating"] is None:
        # No dev/bench history present — conservative fallback.
        result["gating"] = {
            "dashboard": _GATING_DASHBOARD,
            "runs": 0,
            "timing": {},
            "thresholds": derive_thresholds({}, 0),
        }
    return result


def render_human(rep: dict) -> str:
    lines = []
    for path, d in rep["dashboards"].items():
        lines.append(f"== {path} ({d['runs']} runs) ==")
        for cat in ("byte", "count", "timing", "other"):
            c = d["categories"][cat]
            if c.get("pairs", 0) == 0:
                lines.append(f"  {cat:7s}: (none)")
                continue
            lines.append(
                f"  {cat:7s}: metrics={c['metrics']} pairs={c['pairs']} "
                f"median={c['median_pct']}% p95={c['p95_pct']}% max={c['max_pct']}%"
            )
    g = rep["gating"]
    t = g["thresholds"]
    lines.append("")
    lines.append(f"GATING dashboard: {g['dashboard']} ({g['runs']} runs)")
    lines.append(f"  soft alert-threshold : {t['soft']}%")
    lines.append(f"  hard fail-threshold  : {t['hard']}%")
    lines.append(f"  basis: {t['basis']}")
    return "\n".join(lines)


# ── self-test (hermetic: no git, no network) ─────────────────────────────────────
def self_test() -> int:
    # parse_data_js strips the assignment + trailing semicolon.
    js = 'window.BENCHMARK_DATA = {"entries": {"s": []}};\n'
    assert parse_data_js(js) == {"entries": {"s": []}}
    assert parse_data_js('window.BENCHMARK_DATA = {"a":1}') == {"a": 1}

    # categorization.
    assert categorize("store_bytes_per_triple") == "byte"
    assert categorize("wasm_bundle_bytes") == "byte"
    assert categorize("watdiv_q1_us") == "timing"
    assert categorize("parse_ns_per_byte") == "timing"
    assert categorize("result_rows") == "count"
    assert categorize("some_gauge") == "other"

    # adjacent_rel_changes: deterministic byte metric = 0 noise; timing metric picks up swing.
    entries = [
        {"commit": {"id": "c1"}, "date": 1, "benches": [
            {"name": "store_bytes_per_triple", "value": 100.0},
            {"name": "q_us", "value": 10.0},
        ]},
        {"commit": {"id": "c2"}, "date": 2, "benches": [
            {"name": "store_bytes_per_triple", "value": 100.0},   # unchanged
            {"name": "q_us", "value": 12.0},                       # +20%
        ]},
        {"commit": {"id": "c3"}, "date": 3, "benches": [
            {"name": "store_bytes_per_triple", "value": 100.0},
            {"name": "q_us", "value": 6.0},                        # -50%
        ]},
    ]
    ch = adjacent_rel_changes(entries)
    assert ch["store_bytes_per_triple"] == [0.0, 0.0], ch["store_bytes_per_triple"]
    assert abs(ch["q_us"][0] - 0.20) < 1e-9 and abs(ch["q_us"][1] - 0.50) < 1e-9, ch["q_us"]

    # Same commit id adjacent => the pair is dropped (a scheduled re-run of the same SHA).
    dup = [
        {"commit": {"id": "c1"}, "date": 1, "benches": [{"name": "q_us", "value": 10.0}]},
        {"commit": {"id": "c1"}, "date": 2, "benches": [{"name": "q_us", "value": 99.0}]},
    ]
    assert "q_us" not in adjacent_rel_changes(dup)

    # summarize byte category => all zero; timing => nonzero p95/max.
    byte_sum = summarize(ch, "byte")
    assert byte_sum["max_pct"] == 0.0 and byte_sum["p95_pct"] == 0.0
    timing_sum = summarize(ch, "timing")
    assert timing_sum["max_pct"] == 50.0 and timing_sum["pairs"] == 2

    # quantile sanity.
    assert _quantile([0, 100], 0.0) == 0 and _quantile([0, 100], 1.0) == 100
    assert _quantile([0, 100], 0.5) == 50

    # derive_thresholds: with enough runs, soft > p95, hard capped at 200 and > soft.
    ts = derive_thresholds({"pairs": 100, "p95_pct": 43.3, "max_pct": 185.0}, 20)
    assert ts["soft"] == 175, ts          # ceil((100+43.3+30)/5)*5 = 175
    assert ts["hard"] == 200, ts          # ceil((100+185)/5)*5 = 285 -> capped 200
    assert ts["hard"] > ts["soft"]
    # Small worst swing => hard floored at 150, still > soft.
    ts2 = derive_thresholds({"pairs": 100, "p95_pct": 10.0, "max_pct": 20.0}, 20)
    assert ts2["soft"] == 140 and ts2["hard"] == 150, ts2
    # Too little history => conservative fallback.
    ts3 = derive_thresholds({"pairs": 3, "p95_pct": 5.0, "max_pct": 5.0}, 3)
    assert ts3["soft"] == 130 and ts3["hard"] == 200 and "CONSERVATIVE" in ts3["basis"]
    ts4 = derive_thresholds({}, 0)
    assert ts4["soft"] == 130 and ts4["hard"] == 200

    # end-to-end report over a tiny in-memory dataset.
    datasets = {
        _GATING_DASHBOARD: {"entries": {"sparq engine": entries}},
    }
    rep = report(datasets)
    assert rep["gating"]["dashboard"] == _GATING_DASHBOARD
    assert rep["gating"]["runs"] == 3
    assert "soft" in rep["gating"]["thresholds"] and "hard" in rep["gating"]["thresholds"]
    txt = render_human(rep)
    assert "GATING dashboard" in txt and "soft alert-threshold" in txt

    log("self-test OK")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(prog=PROG)
    ap.add_argument("--ref", default="origin/benchmark-data",
                    help="git ref holding the dashboards (default origin/benchmark-data)")
    ap.add_argument("--data-js", action="append", default=None,
                    help="explicit data.js file(s) instead of reading from git")
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    if args.data_js:
        datasets = {}
        for p in args.data_js:
            try:
                with open(p, encoding="utf-8") as fh:
                    datasets[p] = parse_data_js(fh.read())
            except (OSError, json.JSONDecodeError, ValueError) as e:
                log(f"{p}: {e}")
    else:
        datasets = load_from_git(args.ref)

    if not datasets:
        log("no benchmark-data dashboards found — nothing to derive from.")
        rep = report({})
    else:
        rep = report(datasets)

    if args.json:
        print(json.dumps(rep, indent=2))
    else:
        print(render_human(rep))
    return 0


if __name__ == "__main__":
    sys.exit(main())
