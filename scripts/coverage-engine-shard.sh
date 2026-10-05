#!/usr/bin/env bash
# [FABLE-5] sq-piapk: cross-runner SPLIT of the sparq-engine per-commit coverage shard.
#
# WHY THIS EXISTS
# ---------------
# After the 4-way per-commit coverage matrix (sq-p0hcd / #1394), the ONE remaining
# wall-clock floor is the sparq-engine shard: it measured ALONE because it could not be
# split without cross-runner .profraw merging. A compile-vs-test profile of that shard
# (bead sq-piapk) found it is ~97% TEST-EXECUTION-bound (instrumented compile ~16-26s;
# serial instrumented test-exec ~13min / parallel ~7-8min on an 8-core box — the CI
# runner numbers differ but the ~1:30 compile:test RATIO transfers). So the lever is
# splitting the TEST EXECUTION across N runners, each emitting .profraw, then MERGING the
# .profraw into ONE `cargo llvm-cov report -p sparq-engine` before the ratchet check.
#
# WHY NO SHARED BUILD ARCHIVE (each runner compiles its own instrumented objects):
# the instrumented sparq-engine test binaries are BIT-FOR-BIT reproducible across runners
# (same pinned toolchain + same `--features parallel,regex,digest` + same source +
# reproducible-build hygiene => identical per-function llvm coverage-counter hashes). So a
# .profraw emitted by runner-1's compile merges CLEANLY (byte-identical coverage, zero
# hash-mismatch warnings) against runner-2's — or the merge job's — independently-compiled
# objects. Measured on cargo-llvm-cov 0.8.7: same-compile == cross-compile == 1716/12962
# on a filtered arm, md5-identical binaries, clean llvm-profdata merge (validation recorded
# in the PR that introduced this file). The instrumented COMPILE is cheap (~16-26s, plus a
# warm dep cache) — a fraction of the test-exec pole — so paying it once per runner is far
# cheaper than the fragile cargo-llvm-cov `--archive-file` transport (whose 0.8.7 object
# layout does NOT match where `report` reads objects). The native path uses the STANDARD
# llvm-cov target layout `report` reads with no workarounds.
#
# The split is coverage-IDENTICAL to the single-shard number: `report` merges every
# accumulated .profraw against the FULL instrumented object set, so the merged covered-line
# set == the union of what each partition executed == the un-split run, and the denominator
# (total instrumented lines) is the SAME full object set. Same number, not an approximation.
# Proven at full fidelity: a 3-way nextest-partition split merged to 11648/12962 EXACTLY
# equal to the un-split run (per-file identical), recorded in the introducing PR.
#
# WHY test-GRANULARITY partitioning (nextest --partition), not per-binary/per-file:
# the sparq-engine LIB target holds two tests (`group_aggregate_above_par_threshold_complete`,
# `function_registry_reaches_parallel_workers`) each a large fraction of the whole shard's
# wall-clock; a per-binary split would leave one runner carrying the lib target ~alone.
# `nextest --partition count:N/k` hashes INDIVIDUAL tests across the k partitions, so the two
# monster lib tests land on DIFFERENT runners. (sq-ysx2l tracks making those two cheaper — a
# larger lever still.)
#
# The .profraw-accumulate-then-report flow mirrors coverage.sh's measure_merged() (the
# nightly conformance-binary merge): `--no-report` runs LEAVE their .profraw in the profraw
# dir (it only resets on `clean`), and a later `report` merges ALL of them. The one NEW
# thing here is transporting each partition's .profraw BETWEEN runners (artifact up/down).
#
# SUBCOMMANDS (one CI job each; see .github/workflows/ci.yml coverage-engine-*):
#   run   <k> <N>          compile the instrumented engine test binaries AND run partition k
#                          of N (nextest --partition), --no-report, leaving this partition's
#                          .profraw in the profraw dir. No archive.
#   build-objects          compile the instrumented engine test binaries WITHOUT running any
#                          test (no profraw) — the merge job calls this to reconstruct the
#                          object files `report` needs on a fresh runner.
#   report <out.json>      merge ALL .profraw currently in the profraw dir into one
#                          `cargo llvm-cov report -p sparq-engine`, and write a
#                          coverage-summary.json in coverage.sh's EXACT shape (so
#                          scripts/coverage-gate.py --check gates it unchanged).
#   --self-test            fast, NO-compile checks of the pure logic + the shape of the
#                          emitted summary row (run in a cheap CI step).
#
# CRATE + FEATURES: sparq-engine is measured with its DEFAULT features (parallel,regex,digest)
# — EXACTLY as scripts/coverage.sh measure() does for engine (engine has no special `case`
# arm there). Keep these two in lock-step: the coverage number the ratchet floor gates is the
# default-feature number, so this helper must not add/drop engine features. The instrumented
# COMPILE inputs MUST be pinned identical across all runners (same toolchain, same features,
# same RUSTFLAGS) so the binaries stay bit-reproducible and the cross-runner profraw merge is
# valid — the CI jobs use one pinned dtolnay/rust-toolchain and one IDENTICAL job-level
# RUSTFLAGS across every run partition and the merge job.
# [FABLE-5]
#
# [SONNET-4.6] sq-6vshe.11: that RUSTFLAGS is no longer empty. The coverage-engine-{run,merge}
# jobs set `RUSTFLAGS: -C link-arg=-fuse-ld=lld` at JOB level, because cargo-llvm-cov's own
# RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS shadows ci.yml's workflow-level
# CARGO_TARGET_<triple>_RUSTFLAGS (cargo picks ONE rustflags source, first match wins — it
# never merges them), so the lld flag reached every other native job but not this lane. The
# value is the SAME string in both jobs, so the compile inputs stay identical and the merge
# stays valid; scripts/check-coverage-rustflags.py fails CI if they ever diverge.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

CRATE="sparq-engine"
# DEFAULT features only — mirror coverage.sh measure() (no engine `case`). Do NOT change
# without changing coverage.sh in lock-step (the floor gates the default-feature number).
ENGINE_FEATURES="parallel,regex,digest"

# [FABLE-5] sq-piapk: count every .profraw the native llvm-cov path emits. `cargo llvm-cov
# nextest --no-report` (no archive) writes .profraw under CARGO_LLVM_COV_TARGET_DIR/llvm-cov-
# target per LLVM_PROFILE_FILE, which `cargo llvm-cov report` reads natively. Search
# recursively under target/ so the count is robust to the exact subdir.
count_profraw() {
  find "$ROOT/target" -name '*.profraw' 2>/dev/null | wc -l
}

cmd="${1:-}"

case "$cmd" in
  --self-test)
    # No compile. Verify (a) the partition-arg validator rejects bad k/N, (b) the summary
    # row the report step emits has coverage.sh's EXACT shape + keys the gate reads.
    python3 - <<'PY'
import json,sys

# (a) validate_partition: 1<=k<=N, both positive ints.
def validate_partition(k, N):
    for name,v in (("k",k),("N",N)):
        if not isinstance(v,int) or v < 1:
            raise ValueError(f"{name} must be a positive integer, got {v!r}")
    if k > N:
        raise ValueError(f"partition k={k} exceeds N={N}")
    return True

assert validate_partition(1,3)
assert validate_partition(3,3)
for bad in [(0,3),(4,3),(-1,3),(1,0)]:
    try:
        validate_partition(*bad); raise SystemExit(f"validate_partition accepted {bad}")
    except ValueError:
        pass

# (b) summary_row: given an llvm-cov `report` totals block, emit coverage.sh's row shape.
def summary_row(crate, totals, seconds, features):
    t = totals["lines"]
    return {"crate":crate,"lines_pct":round(t["percent"],2),
            "lines_covered":t["covered"],"lines_total":t["count"],
            "seconds":seconds,"features":features,"skipped_tests":[],"measured":True}

row = summary_row("sparq-engine",
                  {"lines":{"percent":90.9509,"covered":11789,"count":12962}},
                  123, ["parallel","regex","digest"])
# The gate (coverage-gate.py check()) reads exactly these keys:
for key in ("lines_pct","lines_covered","lines_total","measured","features"):
    assert key in row, f"summary row missing {key}"
assert row["lines_pct"] == 90.95 and row["lines_covered"] == 11789 and row["lines_total"] == 12962
assert row["measured"] is True
# And the doc it lands in is the coverage.sh summary shape { generated, tier, crates{...} }.
doc = {"tier":"per-commit","crates":{row["crate"]:{k:v for k,v in row.items() if k!="crate"}}}
assert doc["crates"]["sparq-engine"]["lines_pct"] == 90.95
print("coverage-engine-shard self-test: ALL ASSERTIONS PASSED")
PY
    exit $?
    ;;

  run)
    K="${2:?usage: coverage-engine-shard.sh run <k> <N>}"
    N="${3:?partition total N}"
    cd "$ROOT"
    case "$K" in ''|*[!0-9]*) echo "ERROR: k='$K' not a positive int" >&2; exit 2;; esac
    case "$N" in ''|*[!0-9]*) echo "ERROR: N='$N' not a positive int" >&2; exit 2;; esac
    [ "$K" -ge 1 ] && [ "$N" -ge 1 ] && [ "$K" -le "$N" ] \
      || { echo "ERROR: need 1<=k<=N, got k=$K N=$N" >&2; exit 2; }
    echo "==> [engine-cov] compiling + running partition $K/$N (--no-report, leaving .profraw)"
    # Native (no-archive) path: `cargo llvm-cov nextest --no-report` compiles the instrumented
    # engine test binaries AND runs the requested nextest test-level partition, leaving the
    # .profraw in the standard llvm-cov target dir that `report` reads. --partition count:K/N
    # hashes INDIVIDUAL tests across the N partitions (splits the two monster lib tests apart).
    # `--test-threads 1` is a NEXTEST top-level flag (one test process at a time) for
    # profraw determinism — NOT a libtest passthrough after `--` (nextest rejects that).
    cargo llvm-cov nextest --no-report \
      --package "$CRATE" --features "$ENGINE_FEATURES" \
      --partition "count:$K/$N" \
      --test-threads 1 \
      || {
        # nextest exit 4 = "no tests matched" is NOT expected here (every partition of a
        # non-empty suite has tests); surface it loudly rather than silently emitting no
        # profraw (which would drop covered lines in the merge -> a spurious floor failure).
        rc=$?; echo "::error::[engine-cov] partition $K/$N run failed (rc=$rc)"; exit "$rc"; }
    echo "==> [engine-cov] partition $K/$N done; .profraw under target/: $(count_profraw)"
    ;;

  build-objects)
    # [FABLE-5] sq-piapk: the merge job runs on a FRESH runner with no build. `cargo llvm-cov
    # report` needs the instrumented object files (it maps counters -> source lines) in the SAME
    # llvm-cov target dir a `run` partition compiled into. Compile them here WITHOUT running any
    # test, producing the SAME bit-reproducible instrumented binaries the run partitions compiled
    # (same toolchain+features+source), so the downloaded partition .profraw merge cleanly against
    # them. Native-layout analogue of the run compile — no archive, no extraction.
    #
    # WHY NOT `cargo llvm-cov nextest --no-report --no-run`: on cargo-llvm-cov 0.8.7 `--no-run` is
    # a DEPRECATED llvm-cov flag meaning "report WITHOUT running tests" (the old `report` alias),
    # so llvm-cov's own parser owns it and REJECTS it alongside --no-report:
    #   "error: --no-report may not be used together with --no-run"
    # (it never reaches nextest, whose own `--no-run` = "compile, don't run"). So we instead
    # export llvm-cov's instrumentation env with `show-env` and drive `cargo nextest run --no-run`
    # DIRECTLY, forcing the SAME `<target>/llvm-cov-target` dir that `cargo llvm-cov nextest` uses
    # for the run partitions (and that `cargo llvm-cov report` reads objects from). This compiles
    # the instrumented engine + test binaries and runs NO test, so it emits no test-execution
    # profraw under llvm-cov-target (only benign build-script/proc-macro profraw at the target
    # ROOT, which `report`, reading llvm-cov-target, ignores — validated: merged number == the
    # un-split single-shard number, byte-identical).
    cd "$ROOT"
    echo "==> [engine-cov] compiling instrumented engine objects for the merge (no test run)"
    # `<target>/llvm-cov-target` — resolve <target> via show-env so it tracks the tool's layout
    # rather than hard-coding; this is exactly where `cargo llvm-cov nextest`/`report` operate.
    cov_target_dir="$(cargo llvm-cov show-env 2>/dev/null \
      | sed -n 's/^CARGO_LLVM_COV_TARGET_DIR=\(.*\)$/\1/p')"
    [ -n "$cov_target_dir" ] || cov_target_dir="$ROOT/target"
    # Source llvm-cov's instrumentation env (RUSTC_WRAPPER + coverage RUSTFLAGS) into THIS shell,
    # then compile-without-running via nextest's OWN --no-run into the llvm-cov target dir.
    set -a
    eval "$(cargo llvm-cov show-env --export-prefix 2>/dev/null)"
    set +a
    cargo nextest run --no-run \
      --package "$CRATE" --features "$ENGINE_FEATURES" \
      --target-dir "$cov_target_dir/llvm-cov-target"
    echo "==> [engine-cov] instrumented objects built under $cov_target_dir/llvm-cov-target"
    ;;

  report)
    OUT="${2:?usage: coverage-engine-shard.sh report <out-summary.json>}"
    cd "$ROOT"
    mkdir -p "$(dirname "$OUT")"
    n="$(count_profraw)"
    echo "==> [engine-cov] merging $n .profraw file(s) from all partitions -> report"
    [ "$n" -ge 1 ] || { echo "::error::[engine-cov] NO .profraw to merge — every partition's"\
      " profraw is missing; refusing to emit a coverage number (would be a false 0)"; exit 1; }
    tmpjson="$(mktemp)"; start=$(date +%s)
    # `report -p sparq-engine` merges ALL accumulated .profraw against the FULL instrumented
    # object set, so covered == union of every partition's executed lines and total == the
    # full-suite denominator: IDENTICAL to the un-split single-shard number. `report` takes NO
    # --features (that is build-time, already baked into the objects) — same as coverage.sh
    # measure_merged step 4.
    cargo llvm-cov report --package "$CRATE" \
      --summary-only --json --output-path "$tmpjson"
    end=$(date +%s)
    # Assemble a coverage-summary.json in coverage.sh's EXACT shape so coverage-gate.py
    # --check validates it with zero special-casing (same keys: lines_pct/covered/total/
    # measured/features). tier=per-commit so the base `floor` (not nightly_floor) applies.
    python3 - "$OUT" "$tmpjson" "$((end-start))" "$ENGINE_FEATURES" "$(rustc --version 2>/dev/null || echo unknown)" <<'PY'
import json,sys,datetime
out,path,secs,feat_csv,toolchain = sys.argv[1:6]
d=json.load(open(path))
t=d["data"][0]["totals"]["lines"]   # llvm-cov shape: {count,covered,percent}
feats=[f for f in feat_csv.split(",") if f]
row={"lines_pct":round(t["percent"],2),"lines_covered":t["covered"],
     "lines_total":t["count"],"seconds":int(secs),"features":feats,
     "skipped_tests":[],"measured":True}
doc={"generated":datetime.datetime.now(datetime.timezone.utc).isoformat(),
     "tier":"per-commit","toolchain":toolchain,"total_seconds":int(secs),
     "note":"[FABLE-5] sq-piapk: sparq-engine measured via cross-runner nextest-partition "
            "split; this summary is the MERGED .profraw report over the full object set "
            "(coverage-identical to the un-split shard).",
     "crates":{"sparq-engine":row}}
json.dump(doc,open(out,"w"),indent=2,sort_keys=True); open(out,"a").write("\n")
print(f"==> [engine-cov] wrote {out}: sparq-engine lines={row['lines_pct']}% "
      f"({row['lines_covered']}/{row['lines_total']})")
PY
    rm -f "$tmpjson"
    ;;

  *)
    echo "usage: coverage-engine-shard.sh {run <k> <N> | build-objects | report <out.json> | --self-test}" >&2
    exit 2
    ;;
esac
