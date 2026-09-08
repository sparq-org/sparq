"""[GPT-6 Astra] Summarize the fixed local matrix without dropping outliers."""
from pathlib import Path
import json
import statistics as stats

out = Path(__file__).parent
records = json.loads((out / "measurements.json").read_text())
controls = json.loads((out / "allocation-control.json").read_text())
cases = [("base", 1), ("base", 512), ("overlay", 1), ("overlay", 512), ("drained", 1), ("drained-overlay", 1), ("ties", 1), ("ties-overlay", 1)]
assert len(records) == 96 and all(r["exit_code"] == 0 for r in records)
summary = []
for case, k in cases:
    variants = {}
    for variant in ["new", "old", "disabled", "main"]:
        group = {r["mode"]: r for r in records if (r["case"], r["k"], r["variant"]) == (case, k, variant)}
        timing = [r for r in group["timing"]["rows"] if r["kind"] == "sample"]
        counts = [r for r in group["count"]["rows"] if r["kind"] == "sample"]
        assert len(timing) == 7 and len(counts) == 3
        assert all(r["oracle"] == "pass" for r in timing + counts)
        values = [r["whole_query_ns"] / 1e6 for r in timing]
        metrics = ["allocs", "reallocs", "requested_bytes", "peak_live_delta"]
        assert len({tuple(r[m] for m in metrics) for r in counts}) == 1
        fixture = next(r for r in group["timing"]["rows"] if r["kind"] == "fixture")
        verify = next(r for r in group["verify"]["rows"] if r["kind"] == "verify")
        variants[variant] = dict(
            whole_query_ms=dict(median=stats.median(values), minimum=min(values), maximum=max(values), mean=stats.mean(values), sample_stddev=stats.stdev(values), repetitions=7, all_samples=values),
            query_allocations={m: counts[0][m] for m in metrics},
            allocation_repetitions=3,
            fixture={m: fixture[m] for m in ["n", "seed_rows", "pending_rows", "tied_rows", "probe_predicates", "tombstones"]},
            rss_bytes=dict(setup_max=fixture["setup_max_rss"], after_warmup_max=timing[0]["pre_query_max_rss"], final_cumulative_max=timing[-1]["post_query_max_rss"], cumulative_increase_after_warmup=timing[-1]["post_query_max_rss"]-timing[0]["pre_query_max_rss"]),
            verification=verify,
        )
    ratios = {name: variants[a]["whole_query_ms"]["median"] / variants[b]["whole_query_ms"]["median"] for name, a, b in [("new_over_old", "new", "old"), ("new_over_disabled", "new", "disabled"), ("new_over_main", "new", "main"), ("disabled_over_main", "disabled", "main")]}
    disjoint = {f"new_vs_{v}": variants["new"]["whole_query_ms"]["maximum"] < variants[v]["whole_query_ms"]["minimum"] or variants["new"]["whole_query_ms"]["minimum"] > variants[v]["whole_query_ms"]["maximum"] for v in ["old", "disabled", "main"]}
    summary.append(dict(case=case, k=k, variants=variants, median_ratios=ratios, repetition_ranges_disjoint=disjoint))

(out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
provenance = json.loads((out / "provenance.json").read_text())
report = dict(
    task="PR5983 / issue6465 oversized-leading-tie early rejection only",
    decision="Repair has measured allocation benefit; full candidate remains NO_GO for admission",
    decision_basis="The new guard removes six probe preparations from the two known oversized-leading-tie cases without changing positive-path allocations. Drained-prefix fallback regressions remain, with disjoint candidate/disabled timing ranges in both base and overlay cases. Timing noise and baseline drift prohibit treating the tie medians as a precise speedup claim. No independent review or admission was performed in this stage.",
    author="OpenAI GPT-6 Astra, actual xhigh implementation and measurement runtime; no additional agents or model calls",
    head=provenance["head"],
    base=provenance["base"],
    branch=provenance["branch"],
    tracked_scope=dict(files=["crates/sparq-engine/src/exec.rs", "crates/sparq-engine/tests/topk_orderby_indexed_differential.rs"], additions=78, deletions=7, new_tests=3),
    change="Compute the unchanged max(n/2, 256, 2*row_budget) cap immediately after the seed's sort and all-inline checks. If first and cap-th seed keys are equal in the requested direction, return the existing fallback before preparing other scans or retained subject vectors. Keep the original late block guard, fallback, shape, budget and fanout rules intact. Historical Luke Dary / Claude Sonnet5 authorship remains attributed; this commit carries the actual Astra trailer.",
    source_argument=[
        "The existing actual_sort == Some(2) guard and all-inline proof precede this new check; scan_with preserves permutation order when merging overlays.",
        "For sorted rows, equality between logical positions 0 and cap means the leading group has at least cap+1 rows. A group exactly cap long remains allowed.",
        "n > cap establishes n is nonzero, cap < n and (n-1)-cap is representable; both ASC and DESC indices are within the scan.",
        "No probe scan, subject vector or collected result has been built at the new return. Seed scan materialization and the O(seed_rows) all-inline scan still happen first.",
        "The same cap is used later, including its floor and saturating 2*row_budget component. A rejected leading group was already guaranteed to exceed the old first-block cap; the change moves rejection, not result semantics or thresholds.",
    ],
    validation=dict(
        default="46 passed: indexed differential 29, existing topk_orderby 14, orderby_limit_zero 3; one test thread, two build jobs, offline/locked task-local target",
        compact_index="29 differential tests passed, including fallback where PSO is absent; conditional trace expectation does not claim indexed engagement on unsupported permutations",
        new_test_coverage="ASC/DESC, OFFSET with total-order oracle, floor cap256 and proportional cap300, exact cap and cap+1, large row-budget cap, overlay tombstone-adjusted n/group",
        semantic_control_limit="The old late placement is semantically correct for these cases; semantic tests alone cannot pin avoided preparation. The allocation contract is the headline performance guard control.",
        allocation_control=controls["checks"],
        mutation_contract="One real compiled mutation disables only the new early return and retains the late cap. Base positive metrics match; both tied fixtures match old binary metrics exactly. Saved executed-binary data passes allocation contracts for new and causes exit1 AssertionError for mutated bytes on each tied fixture. No wall-clock assertions.",
        allocator_calibration="Both newly compiled counting binaries passed the unchanged harness calibration. Prior standalone counter negative controls remain preserved in the original diagnostic packet.",
        preflight="Not fully green: only privacy-claims failed because macOS Bash3.2 lacks mapfile at scripts/check-privacy-claims.sh:92. G1/G2/G6 and guard-untested ran; no skip, permission change or bypass was introduced. git diff --check passed and source worktree is clean.",
        not_run="No full workspace, new broad conformance, additional no-default variant, clippy, network install or full benchmark sweep in this stage. Earlier semantic evidence is preserved, not recounted as newly executed.",
    ),
    build=dict(results=json.loads((out / "build-results.json").read_text()), jobs=2, warm_target="Task-local diagnostic/target reused; original stored binaries and evidence unchanged", profile="release opt-level3, lto=false, codegen-units16, debug=false", runtime_threads=1, features="same default engine/core features and exact harness Cargo.lock; count-alloc affects only separate instrumentation binary", deadline_seconds=900),
    protocol=dict(
        fixture_points=8, n=50000, k=[1,512], probe_predicates=6,
        overlay="Fork retains original snapshot; delete every fifth seven-triple star: 70,000 tombstones, 40,000 live seed rows",
        adverse="Drained prefix leaves pending*2 >= seed and difference <=4, immediately below the existing upfront selectivity decline. Leading ties exceed max_group by one:25,001/50,000 base and20,001/40,000 overlay.",
        matrix="Eight points × new/old/disabled/main × verify/timing/count =96 process runs,320 query samples (224 uninstrumented timing +96 allocation)",
        additional_control="Three points × verify/count =6 process runs,9 allocation samples. Total102 process runs /329 query samples. No additional timing confirmation or expanded sweep.",
        warmup=2, timing_repetitions=7, allocation_repetitions=3,
        oracle="Same independent generated Rust sort on priority and unique sequence; all results compared outside measurement. Separate verify processes use the existing BGP trace to infer indexed engagement for favorable new/old cases and fallback elsewhere.",
        query_window="Whole query call includes parsing, evaluation and result construction; checking, formatting and result destruction excluded. Count-allocator timing is not reported.",
        source_order="Round-robin source order by fixture, sequential local process runs. No concurrent local build. No CPU affinity, thermal control or host-wide quietness claim; per-process load averages retained. No outliers removed.",
    ),
    results=dict(paired_matrix=summary, all_oracles_and_path_verifications_passed=True, deterministic_allocations_across_repetitions=True),
    memory_limits=[
        "requested_bytes is allocation traffic, counting successful realloc's full new size; it is not simultaneously live heap. peak_live_delta tracks instrumented requested live bytes above pre-query baseline.",
        "Tie-case query heap peaks are unchanged: earlier preparation was already freed before fallback reached its high-water mark. The improvement is avoided allocation work/traffic, not a lower measured peak.",
        "The overlay seed scan remains: new tie-overlay requested bytes exceed disabled by601,838 (base excess1,838). No claim that early rejection is free.",
        "RSS is macOS getrusage process cumulative high-water, recorded after setup, after warmup and after queries. A flat RSS after warmup does not imply zero query memory. It cannot isolate a query-only RSS peak; use heap delta only within its instrumentation limits.",
        "Heap instrumentation excludes allocator metadata, stack, mappings and transient allocator-internal realloc peaks; calibration is bounded, not an external profiler or Miri proof.",
    ],
    timing_limits="This run has substantial noise: base-k1 new includes first-fixture warm process/host variability; ties old has a112.173ms sample. Main/disabled medians drift up to about32% despite identical allocation metrics. Preserve all samples and report ratios as local diagnostic observations, not causal precision or canonical speedups. Drained new/disabled ranges remain disjoint despite noise.",
    next_smallest_step="After independent review of this isolated repair, investigate one read-only first-block feasibility check for drained prefixes using borrowed/lazy probe ranges before retaining all subject vectors; if its semantics/cost cannot be bounded, decline those unsupported shapes. Do not select a new heuristic threshold from these eight points or broaden fanout. Re-measure the same controls only after a concrete reviewed candidate; this stage does not implement that design.",
    unresolved=["Drained-prefix construction/probing before fallback (unchanged)", "F1 seed/overlay setup cost and wider resource scaling", "F3 public documentation for unspecified exact-tie ordering", "F4 explicit indexed-path trace", "F5 remaining path-pinned semantic coverage and Reduced/constant-probe/source-context review questions", "Actual independent Opus review of this repair and future admission decision"],
    provenance="Exact hashes and commit identities in provenance.json; byte-identical harness verification in harness-equality.json; all39 semantic and139 diagnostic manifest files reverified in preserved-evidence.json",
)
(out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
lines = ["# Early oversized-leading-tie repair: focused review and measurement", "", f"Frozen local HEAD `{provenance['head']}`, parent `{provenance['base']}`. Actual GPT-6 Astra xhigh; historical attribution preserved. Two files, +78/-7, three new tests. No remote actions or independent model review in this stage.", "", "**Decision: the focused repair removes measured allocation work; the full candidate remains NO-GO for admission.** Drained-prefix regressions and F3–F5 remain. The new check reuses the existing cap and fallback after seed sort/inline verification, before any probe materialization. Equality at logical index cap proves group length > cap; n > cap protects both directional indices. Exact-cap and larger-row-budget groups remain eligible.", "", "46 default focused tests and29 compact-index tests pass. The only author preflight failure is the known Bash3.2 `mapfile` limitation at `scripts/check-privacy-claims.sh:92`; this was not bypassed. Diff whitespace passes and the worktree is clean.", "", "The real early-return-disabled binary preserves semantic results and exactly reproduces the old allocation metrics. The fixed-work contract passes on new and exits1 for disabled-early bytes on each tied fixture: base avoids1,200,846 requested bytes /37 allocations; overlay avoids4,560,846 bytes /43 allocations. Favorable base metrics are identical. Tie query-heap peaks are unchanged because preparation was already freed before fallback's peak. The original late block guard remains.", "", "The unchanged harness compares the new binary with all three original stored controls. 96 matrix process runs /320 query samples, plus6 allocation-control process runs /9 samples; all oracles/path checks pass. One sequential lane, two build jobs, one runtime Rayon thread; three optimized binaries built in56.6seconds. Same fixtures/features/lockfile, warmup2, timing7, allocation3. No outliers dropped.", "", "| Fixture | New ms | Old ms | Disabled ms | Main ms | New/old | New/disabled |", "|---|---:|---:|---:|---:|---:|---:|"]
for s in summary:
    vals = [s["variants"][v]["whole_query_ms"]["median"] for v in ["new", "old", "disabled", "main"]]
    lines.append(f"| {s['case']} k{s['k']} | " + " | ".join(f"{v:.3f}" for v in vals) + f" | {s['median_ratios']['new_over_old']:.3f} | {s['median_ratios']['new_over_disabled']:.3f} |")
lines += ["", "These medians are diagnostic observations. Base ties overlap old timing ranges; overlay ties improve with disjoint old/new ranges, but baseline drift is material. Main/disabled have identical allocation metrics and up to about32% timing drift. All raw samples, standard deviation, range, load and cumulative RSS are in summary.json/measurements.json. No precise speedup claim follows. Drained base and overlay remain1.39×/1.57× disabled with disjoint ranges.", "", "The benchmark times the whole query call; result checks/drop are outside. Counting is in a separate allocator binary. Requested bytes measure traffic, and tracked peak-live delta excludes allocator metadata/stack/mappings. RSS is cumulative process high-water after setup/warmup/query, not an isolated query peak. The repaired overlay tie still allocates its seed scan and exceeds disabled requested traffic by601,838bytes; this is not a cost-free decline.", "", "Next smallest useful step: separately assess whether the initial drained block can be rejected using borrowed/lazy probe ranges before materializing every subject vector. This is an unimplemented design candidate, not permission to add heuristics or fanout. Prefer declining an unproved shape. F3 public tie semantics, F4 explicit trace, F5 path-pinned coverage/source questions remain before admission.", "", "The full semantic prior packet remains immutable (SHA256 ef7b0dc4d5bd22642aa673516bdb40a33b83e8049f2447364d617bf96de18458), as does the original NO-GO diagnostic packet (SHA256 9aba74e161159a4fbfd98401cc1a4ceeab9cafd402a7df87883df9c388053af5). All39 and139 respective manifest entries were reverified. Source/binary/harness hashes, build commands, full whole-runtime diff and raw evidence accompany this packet; benchmark sources are copied byte-identically under source/bench/indexed-topk. The focused complete caller/callee and scan consumer context follows.", "", "## Exact delta from reviewed semantic head", "", "```diff", (out / "focused.diff").read_text(), "```", "", "## Complete affected functions and relevant consumers", "", (out / "source-context.md").read_text()]
(out / "review-packet.md").write_text("\n".join(lines) + "\n")
print(json.dumps({"summary_points": len(summary), "report_bytes": (out / "report.json").stat().st_size, "packet_bytes": (out / "review-packet.md").stat().st_size}, indent=2))
