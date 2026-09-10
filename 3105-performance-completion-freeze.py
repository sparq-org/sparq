"""Freeze this completed bounded diagnostic without changing earlier bundles."""
from pathlib import Path
import datetime
import hashlib
import json
import shutil

P = Path(__file__).resolve().parent
assert shutil.disk_usage(P).free > 6509559808
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
prior = []
for root, expected in [(P.parent, "188a6cffeb664756aec40923d18da3ea905ca217b76f654fcb42e7a863676f48"),
                       (P.parent / "performance", "9cd9164058fa95e142633787c30cd9d00a65463e975de28f3b46166503f87bed")]:
    manifest = root / "manifest.json"
    assert sha(manifest) == expected
    entries = json.loads(manifest.read_text())["files"]
    for name, item in entries.items():
        path = root / name
        assert path.stat().st_size == item["bytes"] and sha(path) == item["sha256"], name
    prior.append({"manifest": str(manifest), "sha256": expected, "verified_files": len(entries)})
(P / "prior-bundles-verified.json").write_text(json.dumps(prior, indent=2) + "\n")
summary = json.loads((P / "summary.json").read_text())
original = json.loads((P / "samples.json").read_text())
amended = json.loads((P / "allocation-quiescent/samples.json").read_text())
commands = json.loads((P / "measurement-commands.json").read_text())
new_commands = json.loads((P / "allocation-quiescent/measurement-commands.json").read_text())
assert len(original) == 159 and len(amended) == 60 and len(commands) == 160 and len(new_commands) == 60
failures = [c for c in commands if c["exit"] != 0]
assert len(failures) == 1 and all(c["exit"] == 0 for c in new_commands)
assert all(not s["experiment_memory_reject"] for s in summary)
assert all(not s["candidate_slower_disjoint_ranges"] for s in summary)
for point in summary:
    for source in ["baseline", "candidate"]:
        data = point["sources"][source]
        for metric in ["allocs", "reallocs", "requested_bytes", "peak_live_growth"]:
            old, new = data["allocations_original"][metric], data["allocations_quiescent"][metric]
            assert old["min"] == old["max"] == new["min"] == new["max"]

report = {
    "completed_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "scope": "Single fixed local diagnostic of unchanged candidate; no production changes, tuning, supplementary coverage, remote actions or admission.",
    "candidate_head": "6e86f1d0ba447aa78a50800337a7a379702fa999",
    "baseline_head": "e53464c73f31f7aca800f3867ac054c36408e346",
    "author": "GPT-6 Astra xhigh (actual implementation runtime)",
    "decision": "Positive local timing/heap screen supports proceeding to bounded coverage and full validation. Performance admission remains INCONCLUSIVE pending physical-path gaps and authoritative gates; no merge/issue-closure claim.",
    "counts": {"timing_successes": 100, "original_allocation_successes": 59,
               "original_allocation_startup_failures": 1, "amended_allocation_successes": 60,
               "amended_allocation_failures": 0, "measurement_commands": 220},
    "failure": {"command": failures[0], "phase": "Startup calibration immediately after build_global, before fixture/query or measured workload.",
                "observed": [2, 1, 448, 320, 12804], "expected": [2, 1, 448, 320, 10500],
                "interpretation": "LIVE differed by 2304 bytes while alloc/realloc/request/peak counts matched. Consistent with worker startup outside ACTIVE; precise cause not proven. Not an optimizer failure or established allocator bug.",
                "response": "Root-authorized predeclared harness correction moved unchanged calibration after fixture/oracle/two warmups and explicit global broadcast barrier; reran the entire allocation matrix once, not the failed point alone. All original evidence retained."},
    "findings": [
        "Full miss candidate/base median latency ratios 0.8549 base and 0.8435 overlay; two-RHS residual-false ratios 0.7888 and 0.7713. All four timing ranges disjoint.",
        "First-hit ratios 0.9948 and 1.0019 with overlapping ranges; second-hit and late-LIMIT improved in this fixture. No observed predeclared favorable-case regression.",
        "Two-RHS peak requested live growth increased 143160 bytes (1.3243%), far below this experiment's 25% AND 8MiB screen. Sum-of-RHS retention with more/larger steps remains unbounded by this fixture.",
        "Completed original and amended allocations/reallocations/requested bytes/peak live measurements agree exactly at every point; amended three repetitions agree exactly.",
        "All successful processes passed generated oracle and query result assertions. All100 timing processes and all60 amended allocation processes succeeded. This is local advisory screening, not statistical or canonical published performance proof."],
    "measurement_limits": [
        "Original timing binaries and measured block unchanged; allocation amendment has its own source and binary hashes. No timing claims from count-instrumented durations.",
        "Whole-query public ASK/query calls are timed, including parsing/planning. Graph/overlay construction, uncapped oracle and two warmups are excluded. Query-local RHS reuse cache is cold on every query; persistent graph and code paths are warm.",
        "Only two maximum reached RHS relations, 70000 seed rows, one thread, one macOS arm64 host and one default feature configuration were measured; no cold-process, concurrent query, many-RHS or variable-cardinality sweep.",
        "Allocator metrics count requested bytes, excluding allocator metadata and transient realloc internals. Query peak is growth over actual begin baseline. live_after is total process live bytes, not retained-query growth because begin baseline is not emitted.",
        "RSS is cumulative process high-water and setup includes oracle; it cannot isolate physical query memory. In amended multi-overlay samples candidate after-RSS 49266688–49299456 exceeds baseline47054848–47104000 despite modest requested peak growth. No claim that RSS equals heap.",
        "Broadcast plus complete warmups and successful calibration reduce demonstrated startup noise; they do not formally prove absence of all external/background activity. Original failure retained.",
        "Five paired timing repetitions with min/max, sample standard deviation and CV are advisory; alternatingAB/BA reduces order bias but does not remove host noise. No retry/noise hunting beyond one expressly authorized complete allocation rerun.",
        "Generic process listing and hardware sysctl were sandbox-denied, without retry; exact Cargo-lock lsof had no owners before build. CPU model/RAM metadata not independently collected; uname and rustc target are recorded."],
    "path_and_semantic_limits": [
        "Measured queries retain two shared variables, arithmetic residual filter and actual subject-ordered seed scan; source route is ASK/SLICE -> try_capped -> eval_bgp_binary_capped. Prior phase1 compiled counters established this shape on a related 70000-row fixture, not a new physical trace for each performance point.",
        "Three-pattern multi uses duplicate {s,o} variable edges, which GYO reduction classifies acyclic/binary. Every generated subject/object matches both RHS relations; only final arithmetic residual is false. It reaches both RHS relations by source/data construction; no first-join miss makes retention vacuous.",
        "LIMIT65537 checks count and every row's generated identity; oracle uses uncapped query and generated per-row identity. Limited-row uniqueness/multiset equality is not separately checked in this harness.",
        "Actual changing scan_sort across blocks, mixed bind/non-bind kernels, disconnected cross, restricted permutations, named graphs and residual EXISTS remain pending explicit path-observed coverage. Ordinary base/fork with unrelated tombstones are measured here.",
        "Phase1 budget/row/cancellation controls remain unchanged. Root identified separate baseline nested public-query budget::Guard issue6476 by source review, without claimed runtime reproduction; no fix included here."],
    "next_smallest_step": "Root-authorized later phase: add actual changing-sort/mixed-kernel/disconnected-cross path-observed tests, then relevant feature checks; evaluate any slot-release-before-replacement change separately on exact delta. Obtain full authoritative workspace/wasm/perf ratchet gates before admission. No further work started in this phase.",
    "resources": {"phase_start_utc": "2026-09-10T01:10:55.552996Z", "phase_deadline_utc": "2026-09-10T01:30:55Z", "free_bytes_at_freeze": shutil.disk_usage(P).free,
                  "floor_bytes": 6509559808, "build_jobs": 2, "incremental": False, "offline_locked": True,
                  "commands_pending": False, "cache": "direct-5983/implementation/target is regenerable private cache only; no cache cleanup occurred"},
    "prior_frozen_verification": prior,
}
(P / "report.json").write_text(json.dumps(report, indent=2) + "\n")
prov = json.loads((P / "provenance.json").read_text())
packet = """# Issue3105 exact-candidate local performance completion

Actual GPT-6 Astra xhigh. Candidate6e86f1d0ba447aa78a50800337a7a379702fa999 against main e53464c73f31f7aca800f3867ac054c36408e346; both worktrees clean. No production source changed. Actual Opus source verdict already APPROVE_FOR_VALIDATION; this packet does not claim admission.

The fixed timing and requested-allocation screen is positive. Admission remains incomplete until path-observed coverage and authoritative full gates. The table uses all100 original timing samples and all60 amended allocation samples. Original59 successful allocation samples plus one startup calibration failure are preserved separately; no replacement/discarding of that historical evidence.

"""
packet += (P / "summary.md").read_text()
packet += "\n## Exact interpretation and remaining work\n\n" + json.dumps(report, indent=2) + "\n"
packet += "\n## Predeclared protocol and one authorized amendment\n\n" + (P / "protocol.json").read_text() + "\n" + (P / "allocation-quiescent/amendment.json").read_text()
packet += "\n## Harness delta (both comparison sources identical)\n\n```diff\n" + (P / "allocation-quiescent/baseline-harness.diff").read_text() + "```\n"
packet += "\n## Full final allocation harness; timing harness differs only by the shown delta\n\n```rust\n" + (P / "allocation-quiescent/candidate/src/main.rs").read_text() + "```\n"
packet += "\n## Source, binary and toolchain provenance\n\n" + json.dumps(prov, indent=2) + "\n" + (P / "rustc-version.stdout").read_text() + "\n" + (P / "features.json").read_text()
packet += "\n## Evidence map and scope\n\nOriginal raw/ contains160 exact stdout/stderr pairs and measurement-commands.json records every argv/exit/binary hash. allocation-quiescent/raw/ contains60 pairs and its own receipts. samples.json retains159 successful original records; allocation-quiescent/samples.json retains60. summary.py reproduces all displayed summaries from those files; summary.json contains full statistics including cumulative RSS. builds.json, quiescent-build-*.json and allocation-quiescent/builds.json record actual sequential optimized builds and reserve monitoring. All six binaries are frozen; all four counting.rs copies are byte-identical. The complete production source/test review context remains in prior59-file phase1 packet, not duplicated here; new physical-path coverage was not executed. No source repair, next-phase test, model call, publication, workflow or remote change occurred.\n"
(P / "review-packet.md").write_text(packet)
files = {}
for path in sorted(P.rglob("*")):
    if path.is_file() and path.name != "manifest.json":
        files[str(path.relative_to(P))] = {"bytes": path.stat().st_size, "sha256": sha(path)}
manifest = {"files": files, "count": len(files), "total_bytes": sum(x["bytes"] for x in files.values()),
            "candidate_head": report["candidate_head"], "baseline_head": report["baseline_head"]}
(P / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(json.dumps({"files": len(files), "bytes": manifest["total_bytes"], "manifest_sha256": sha(P / "manifest.json"),
                  "packet_bytes": (P / "review-packet.md").stat().st_size, "packet_sha256": sha(P / "review-packet.md"),
                  "report_sha256": sha(P / "report.json"), "free_bytes": shutil.disk_usage(P).free}, indent=2))
