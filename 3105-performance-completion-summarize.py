"""Recompute every displayed result from preserved successful sample records."""
from pathlib import Path
import datetime
import hashlib
import json
import shutil
import statistics
import subprocess

P = Path(__file__).resolve().parent
assert shutil.disk_usage(P).free > 6509559808
original = json.loads((P / "samples.json").read_text())
quiescent = json.loads((P / "allocation-quiescent/samples.json").read_text())


def stats(values):
    mean = statistics.mean(values)
    return {"n": len(values), "min": min(values), "max": max(values),
            "median": statistics.median(values), "mean": mean,
            "sample_stddev": statistics.stdev(values) if len(values) > 1 else None,
            "cv_percent": 100 * statistics.stdev(values) / mean if len(values) > 1 and mean else 0}


def select(rows, case, view, source, mode):
    return [r for r in rows if (r["case"], r["view"], r["source"], r["mode"])
            == (case, view, source, mode)]


metrics = ["allocs", "reallocs", "requested_bytes", "peak_live_growth", "live_after",
           "setup_peak_rss", "before_peak_rss", "after_peak_rss"]
summary = []
for case in ["first", "second", "miss", "late", "multi"]:
    for view in ["base", "overlay"]:
        point = {"case": case, "view": view, "sources": {}}
        for source in ["baseline", "candidate"]:
            timing = select(original, case, view, source, "timing")
            count = select(quiescent, case, view, source, "count")
            old_count = select(original, case, view, source, "count")
            assert len(timing) == 5 and len(count) == 3
            point["sources"][source] = {
                "nanoseconds_per_query": stats([r["nanos"] / r["iterations"] for r in timing]),
                "timing_rss": {k: stats([r[k] for r in timing]) for k in metrics if "rss" in k},
                "allocations_quiescent": {k: stats([r[k] for r in count]) for k in metrics},
                "allocations_original": {k: stats([r[k] for r in old_count]) for k in metrics},
            }
        b, c = [point["sources"][s] for s in ["baseline", "candidate"]]
        bt, ct = b["nanoseconds_per_query"], c["nanoseconds_per_query"]
        bp, cp = [s["allocations_quiescent"]["peak_live_growth"]["median"] for s in [b, c]]
        point["candidate_baseline_median_latency_ratio"] = ct["median"] / bt["median"]
        point["candidate_faster_disjoint_ranges"] = ct["max"] < bt["min"]
        point["candidate_slower_disjoint_ranges"] = ct["min"] > bt["max"]
        point["peak_live_growth_delta_bytes"] = cp - bp
        point["peak_live_growth_delta_percent"] = 100 * (cp - bp) / bp
        point["experiment_memory_reject"] = cp > bp * 1.25 and cp - bp > 8 * 1024**2
        summary.append(point)
(P / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")

lines = ["Local advisory screening; timing uses original unchanged System binaries (five samples, three queries each). Allocation columns use the amended complete matrix (three samples, one query each).",
         "", "| Case / view | Baseline ms/query range | Candidate ms/query range | C/B median | Timing CV B/C % | Requested bytes B/C | Peak requested live growth B/C | Allocations B/C | Reallocations B/C |",
         "|---|---:|---:|---:|---:|---:|---:|---:|---:|"]
for point in summary:
    b, c = [point["sources"][s] for s in ["baseline", "candidate"]]
    bt, ct = b["nanoseconds_per_query"], c["nanoseconds_per_query"]
    ba, ca = b["allocations_quiescent"], c["allocations_quiescent"]
    pair = lambda k: f'{ba[k]["median"]:,.0f} / {ca[k]["median"]:,.0f}'
    lines.append(f'| {point["case"]} / {point["view"]} | {bt["min"]/1e6:.4f}–{bt["max"]/1e6:.4f} | {ct["min"]/1e6:.4f}–{ct["max"]/1e6:.4f} | {point["candidate_baseline_median_latency_ratio"]:.4f} | {bt["cv_percent"]:.2f} / {ct["cv_percent"]:.2f} | {pair("requested_bytes")} | {pair("peak_live_growth")} | {pair("allocs")} | {pair("reallocs")} |')
lines += ["", "Full min/max/median/mean/sample-standard-deviation/CV and cumulative setup/before/after RSS ranges are in summary.json. Count metrics are identical across the three completed amended repetitions of every case. live_after is total process requested live bytes after the query, not query-retained bytes: the harness does not emit the begin baseline. Peak live growth subtracts that actual baseline internally. RSS is cumulative process high-water; setup includes the full oracle, and cannot isolate query physical memory."]
(P / "summary.md").write_text("\n".join(lines) + "\n")

sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
provenance = {"utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "author": "GPT-6 Astra, xhigh (actual inherited implementation runtime)",
              "baseline": "e53464c73f31f7aca800f3867ac054c36408e346",
              "candidate": "6e86f1d0ba447aa78a50800337a7a379702fa999",
              "harness_and_binaries": {}, "runtime_sources": {},
              "free_bytes": shutil.disk_usage(P).free}
for root in [P, P / "allocation-quiescent"]:
    for path in sorted(root.glob("binary/*")):
        provenance["harness_and_binaries"][str(path.relative_to(P))] = {"sha256": sha(path), "bytes": path.stat().st_size}
    for source in ["baseline", "candidate"]:
        for part in ["Cargo.toml", "Cargo.lock", "src/main.rs", "src/counting.rs"]:
            path = root / source / part
            provenance["harness_and_binaries"][str(path.relative_to(P))] = {"sha256": sha(path), "bytes": path.stat().st_size}
for name, wt in [("baseline", "issue6475"), ("candidate", "issue3105")]:
    root = P.parents[1] / "worktrees" / wt
    for part in ["crates/sparq-engine/src/exec.rs", "crates/sparq-engine/src/budget.rs", "crates/sparq-core/src/store.rs", "crates/sparq-core/src/overlay.rs"]:
        path = root / part
        if path.exists():
            provenance["runtime_sources"][name + "/" + part] = {"sha256": sha(path), "bytes": path.stat().st_size}
    provenance[name + "_git_tree"] = subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], cwd=root, text=True).strip()
    assert subprocess.check_output(["git", "status", "--porcelain=v1"], cwd=root, text=True) == ""
assert (P / "baseline/src/main.rs").read_bytes() == (P / "candidate/src/main.rs").read_bytes()
assert (P / "allocation-quiescent/baseline/src/main.rs").read_bytes() == (P / "allocation-quiescent/candidate/src/main.rs").read_bytes()
assert len({sha(p) for p in [P / "baseline/src/counting.rs", P / "candidate/src/counting.rs", P / "allocation-quiescent/baseline/src/counting.rs", P / "allocation-quiescent/candidate/src/counting.rs"]}) == 1
old = (P / "baseline/src/main.rs").read_text()
new = (P / "allocation-quiescent/baseline/src/main.rs").read_text()
assert old[old.index("    for _ in 0..reps {"):] == new[new.index("    for _ in 0..reps {"):]
provenance["checks"] = {"paired_harness_byte_equal": True, "counting_all_four_byte_equal": True,
                        "old_new_measured_block_byte_equal": True, "both_worktrees_clean": True}
(P / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
print("summary points", len(summary), "original samples", len(original), "amended samples", len(quiescent))
