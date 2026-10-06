#!/usr/bin/env bash
# CI benchmark emitter (roadmap G2). Runs the engine over a freshly-generated synthetic dataset and
# emits results as github-action-benchmark `customSmallerIsBetter` JSON (all metrics are
# times/latencies — smaller is better), so perf is tracked + graphed across commits and a large
# regression raises an alert. Covers the main subsystems: load (parse+build), query (count +
# materialize = join compute), serialisation (json), and RDFS inference.
#
#   scripts/ci-bench.sh [scale_entities=200000] [out.json=bench-results.json]
#
# (sq-dzfu) CHEAP TIMING RE-MEASURE MODE for the perf-gate's best-of-N flake fix:
#   scripts/ci-bench.sh --parse-only [out.json=bench-results.json]
# emits ONLY the TIMING metric (parse_ns_per_byte) over the fixed corpus — NO wasm build, NO big-scale
# corpus, NO well-known suites — so the perf-gate (scripts/perf-gate.py --remeasure-cmd) can re-measure
# just the noisy wall-clock metric quickly when it trips the band, take the BEST reading, and only fail
# on a real (every-reading) regression. Reuses the exact same min-of-N parse measurement as the full run.
#
# GitHub-hosted runners are small + noisy, so absolute numbers drift; the value is the cross-commit
# TREND and large-regression alerting (the workflow sets a generous threshold + fail-on-alert=false).
#
# The load-bearing REGRESSION GATES are the DETERMINISTIC (runner-noise-immune) metrics —
# store/dict bytes-per-{triple,term} (memory layout), wasm_bundle_bytes, and two added on a FIXED
# corpus: store_bytes_per_triple_small (a SECOND scale, catches per-triple-overhead regressions the
# primary scale hides) and parse_ns_per_byte (parse cost on a fixed byte count; emitted as ns/byte =
# 1000/(MB/s) so it reads smaller-is-better under customSmallerIsBetter). Wall-clock latencies stay
# trend-only on free runners.
set -euo pipefail
cd "$(dirname "$0")/.."
# (sq-dzfu) --parse-only: cheap TIMING re-measure for the perf-gate best-of-N flake fix.
# When present as the first arg, ONLY the parse_ns_per_byte metric is (re-)measured (see measure_parse
# below) and written out — no big corpus, no wasm build, no well-known suites.
#
# (sq-6vshe.6 heavy-lane placement, MAINTAINER-DIRECTED extension) --deterministic-only:
#   scripts/ci-bench.sh --deterministic-only [scale=200000] [out.json=bench-results.json]
# emits ONLY the DETERMINISTIC (runner-noise-immune) hard-gated byte-count / memory-layout metrics —
# store_bytes_per_triple, comp_store_bytes_per_triple, store_bytes_per_triple_small, dict_bytes_per_term,
# wasm_bundle_bytes — and NOTHING else. It deliberately SKIPS every wall-clock timing measurement (the
# min-of-3 load loop, the min-of-5 parse loop, the query-latency loops, RDFS-inference timing, wasm-opt
# trend) AND every well-known / crate-example suite (sp2b/dbpsb/watdiv/bsbm/lubm/deeptax/sameas/shacl/
# geo/fts/vector/rsp/hdt/solid/nlq/zk). This is the FAST, DETERMINISTIC form of the per-PR / merge_group
# bench gate (bench.yml): the deterministic byte-count RATCHET (scripts/perf-gate.py) still hard-gates
# every PR against bench/perf-baseline.json — a pure function of the code, seconds not minutes, immune to
# shared-runner noise — while the noisy latency suites are RELOCATED to the EC2 full-suite lane
# (bench-ec2.yml; #3784: MANUAL DISPATCH ONLY, cron retired with the descoped AWS OIDC role)
# so they no longer drag the merge queue or flap the gate. The stanzas below are the EXACT same load/wasm
# commands the full run uses (single source of truth), just without the timing/suite blocks.
PARSE_ONLY=0
DET_ONLY=0
if [ "${1:-}" = "--parse-only" ]; then
  PARSE_ONLY=1
  shift
  OUT="${1:-bench-results.json}"
  SCALE=0
elif [ "${1:-}" = "--deterministic-only" ]; then
  DET_ONLY=1
  shift
  SCALE="${1:-200000}"
  OUT="${2:-bench-results.json}"
else
  SCALE="${1:-200000}"
  OUT="${2:-bench-results.json}"
fi
CLI=target/release/sparq-cli
GEN=target/release/sparq-bench
Q=bench/qlever-synthetic/queries
# Operator-coverage suite (one query per SPARQL operator family). Wall-clock /
# trend-only like the query latencies above — NOT a hard perf-gate (only the byte/parse
# metrics in scripts/perf-gate.py are gated). Registry: bench/benchmarks.toml (operator-coverage).
OPQ=bench/operators/queries
# SP2Bench (well-known suite) — per-commit subset + fixed-corpus generator.
# Trend-only latency like the operator suite, PLUS a hard expected-rows correctness diff.
# Registry: bench/benchmarks.toml (sp2b); details: bench/sp2b/README.md. (sq-0jp)
SP2B_GEN=bench/sp2b/gen.sh
SP2B_Q=bench/sp2b/queries
SP2B_EXP=bench/sp2b/expected-rows.tsv
SP2B_TRIPLES="${SP2B_TRIPLES:-250000}"
# DBPSB/FEASIBLE (well-known suite) — per-commit subset over a pinned, fetched +
# cached DBpedia Databus slice (NO generator; fetch.sh verifies sha256 + emits a deterministic
# 750k-triple N-Triples cut). Trend-only latency like sp2b, PLUS a hard expected-rows
# correctness diff. Registry: bench/benchmarks.toml (dbpsb); details: bench/dbpsb/README.md.
DBPSB_FETCH=bench/dbpsb/fetch.sh
DBPSB_Q=bench/dbpsb/queries
DBPSB_EXP=bench/dbpsb/expected-rows.tsv
DBPSB_TRIPLES="${DBPSB_TRIPLES:-750000}"
# WatDiv (Waterloo SPARQL Diversity, sq-13i) — per-commit subset over the FIXED SF=1
# corpus from the real Waterloo generator (gen.sh; g++ + Boost). Trend-only latency like sp2b,
# PLUS a hard expected-rows correctness diff (count mode). Registry: bench/benchmarks.toml (watdiv);
# details: bench/watdiv/README.md. Mirrors the sp2b inline pattern.
WATDIV_GEN=bench/watdiv/gen.sh
WATDIV_Q=bench/watdiv/queries
WATDIV_EXP=bench/watdiv/expected-rows.tsv
WATDIV_SF="${WATDIV_SF:-1}"
# BSBM (Berlin SPARQL Benchmark, Explore mix) — per-commit subset over the FIXED -pc 300
# corpus from the prebuilt bsbmtools distribution (gen.sh; JRE + unzip). The Explore mix includes a
# CONSTRUCT (query12) + a DESCRIBE (query09), so the expected-rows diff runs against MATERIALIZE
# mode (graph-valued forms report produced-triple counts). Trend-only latency + hard correctness
# diff. Registry: bench/benchmarks.toml (bsbm); details: bench/bsbm/README.md.
BSBM_GEN=bench/bsbm/gen.sh
BSBM_Q=bench/bsbm/queries
BSBM_EXP=bench/bsbm/expected-rows.tsv
BSBM_PC="${BSBM_PC:-300}"
# LUBM (Lehigh University Benchmark) — the REASONING suite. Its run.sh is fully
# self-contained: it builds the LUBM(1) corpus (gen.sh; javac + rapper), materializes the OWL-RL
# closure with `sparq-cli reason`, runs BOTH the extensional + entailed tiers in count mode, and
# asserts every solution count vs expected-rows.tsv internally (exit 1 on mismatch). So the hook
# below just invokes run.sh (the self-asserting path) and harvests its count-mode TSV lines into
# `lubm_<query>_count_us`. Registry: bench/benchmarks.toml (lubm); details: bench/lubm/README.md.
LUBM_RUN=bench/lubm/run.sh
# Deep Taxonomy (DeepTaxonomy, sq-1hgz; parent sq-i0nm "design D") — the rule-heavy N3
# REASONING suite. Like LUBM, its run.sh is the self-asserting entry point: it reuses the existing
# generator (bench/inference/gen_deeptaxonomy.py via bench/deep-taxonomy/gen.sh), materializes the
# N3 forward closure per depth tier with `sparq-cli reason … n3`, runs query.rq over the closure,
# and asserts BOTH the closure triple-count AND the query row-count vs expected.tsv internally
# (exit 1 on any mismatch). So the hook below just invokes run.sh and harvests its metric TSV into
# `deeptax_d<DEPTH>_{closure_s,query_us,closure_triples}`. Registry: bench/benchmarks.toml
# (deep-taxonomy); details: bench/deep-taxonomy/README.md. (sq-1hgz)
DEEPTAX_RUN=bench/deep-taxonomy/run.sh
DEEPTAX_DEPTHS="${DEEPTAX_DEPTHS:-1000 10000}"
# OWL sameAs equality micro-suite (sq-msl6, design A3 + A5) — the EQUALITY reasoning
# suite (the owl:sameAs analogue of Deep Taxonomy's subclass transitivity). Like LUBM/deep-taxonomy
# its run.sh is the self-asserting entry point: it builds K owl:sameAs classes of N members per tier
# (bench/owl-sameas/gen_sameas.py via bench/owl-sameas/gen.sh — pure Python, NO javac/rapper),
# materializes the OWL-RL closure with `sparq-cli reason … owl`, runs query.rq over the closure, and
# asserts BOTH the closure triple-count (K·N·(N+M)) AND the query row-count (K·N) vs expected.tsv
# internally (exit 1 on any mismatch). The hook below invokes run.sh and harvests its metric TSV into
# `sameas_size<TIER>_{closure_s,query_us,closure_triples}`. Registry: bench/benchmarks.toml (owl-sameas);
# details: bench/owl-sameas/README.md. (sq-msl6)
SAMEAS_RUN=bench/owl-sameas/run.sh
SAMEAS_TIERS="${SAMEAS_TIERS:-8 32}"
# SHACL VALIDATION suite (sq-7iai, design §3.1) — the cleanest competitor surface.
# Reuses the LUBM(1) ABox as its data substrate (so it shares the javac/rapper guard) x the 5
# committed shape graphs in bench/shacl/shapes/. Like LUBM, run.sh is the self-asserting entry
# point: it validates with examples/bench_shacl, asserts each workload's violations/conforms/
# focus_nodes vs expected.tsv internally (exit 1 on drift), and prints the
# `<workload>\t<violations>\t<validate_us>` contract on stdout, harvested below into the ADVISORY
# `shacl_<workload>_validate_us` (trend-only, NOT in scripts/perf-gate.py). Registry:
# bench/benchmarks.toml (shacl-validate-bench); details: bench/shacl/README.md.
SHACL_RUN=bench/shacl/run.sh
# FULL-TEXT-SEARCH suite (sq-ustq, design §3.4). Exercises sparq-text (BM25 inverted
# index + text: magic predicates). Unlike LUBM/SHACL the corpus is SYNTHETIC + generated
# in-process by examples/bench_text from a deterministic seed — NO external generator (no
# rapper/javac). run.sh is the self-asserting entry point: it runs bench_text on the pinned
# corpus, asserts each workload's hit count + the integer index bytes-per-doc vs expected.tsv
# (exit 1 on drift), and prints the `<workload>\t<count>\t<us>` contract on stdout. The hit
# counts/bytes_per_doc are the HARD gate (in run.sh); the timings harvested below are ADVISORY
# (text_*_us/text_build_s, trend-only, NOT in scripts/perf-gate.py). The integer bytes-per-doc
# additionally gets a mode:auto ratchet in bench/perf-baseline.json (fts_bytes_per_doc).
FTS_RUN=bench/fts/run.sh
# VECTOR / ANN suite (sq-v02y, design §3.3). Exercises sparq-vectors (mmap'd .spqv
# vector store + HNSW / Vamana / PQ ANN). Like FTS the corpus is SYNTHETIC + generated in-process
# by examples/bench_vectors from a deterministic seed — NO external generator. run.sh is the
# self-asserting entry point: it runs bench_vectors on the pinned corpus, asserts each workload's
# recall DEFICIT (recall@10 vs nearest_exact, emitted as round((1-recall)*1000) so it slots into
# the smaller-is-better mode:auto ratchet — design G4) vs expected.tsv (exit 1 on drift; diskann/pq
# EXACT-gated, hnsw FLOOR-gated as its build is rayon-parallel), and prints the
# `<workload>\t<deficit>\t<us>` contract on stdout. The deficits are the HARD gate (in run.sh);
# the timings harvested below are ADVISORY (vectors_*_query_us/vectors_build_s, trend-only, NOT in
# scripts/perf-gate.py). The deterministic diskann/pq deficits additionally get a mode:auto ratchet
# in bench/perf-baseline.json (vectors_diskann_recall_at10 / vectors_pq_recall_at10).
VECTOR_RUN=bench/vector/run.sh
# GeoSPARQL suite (sq-tf8n, design §3.5). A FIXED ~100k-point CRS84 corpus
# (bench/geo/gen.sh -> bench_geo gen; pure Rust, no javac/rapper/Docker) x within/nearest/
# geof: workloads. Like LUBM/SHACL, run.sh is the self-asserting entry point: it asserts each
# workload's result-set SIZE / compliance pass COUNT vs expected.tsv (counts-not-coords, since
# float geometry is not bit-stable; exit 1 on drift), and prints `<name>\t<count>\t<us>`. The
# hook harvests `geo_<name>_us` (ADVISORY) and routes geo_compliance_pass to the HARD-gated
# DEFICIT geo_compliance_deficit (mode:auto in bench/perf-baseline.json). Registry:
# bench/benchmarks.toml (geo-bench); details: bench/geo/README.md.
GEO_RUN=bench/geo/run.sh
# RSP-QL streaming suite (sq-b1hn, design §3.6). sparq-rsp is a CLOCK-FREE library, so a
# fixed (triple,ts) replay is a pure function — the gate is a DETERMINISTIC per-window result-row
# count (single-window tumbling/sliding/GROUP-BY x three EvalModes, encoding the 3-EvalMode
# equivalence) + an SRBench correctness ORACLE (multi-window observation⋈metadata join). Like FTS
# the runner is a crate EXAMPLE (sparq-rsp is isolated, not a sparq-cli dependency). run.sh is the
# self-asserting entry point: it asserts every `rsp_*_rows` metric vs bench/rsp/expected.tsv (exit 1
# on drift) and prints the `<metric>\t<value>\t<unit>` contract; the rows are the HARD gate (in
# run.sh), the single rsp_persistentdict_triples_per_s is ADVISORY (trend-only, NOT in
# scripts/perf-gate.py). NO competitor perf column — the RSP peers are wall-clock service engines
# (apples-to-oranges). Registry: bench/benchmarks.toml (rsp-ql); details: bench/rsp/README.md.
RSP_RUN=bench/rsp/run.sh
# HDT load-and-decode suite (sq-lrp9, design §3.7). Like FTS/RSP the runner is a crate
# EXAMPLE (bench_oracle) that only needs `cargo` (sparq-hdt is isolated, not a sparq-cli dependency,
# and is behind the `hdt` cargo feature). run.sh is the self-asserting entry point: it loads the
# VENDORED snikmeta.hdt, decodes it to a native sparq Graph, resolves triple patterns, and asserts
# every DETERMINISTIC `count`-unit metric vs bench/hdt/expected.tsv (exit 1 on drift) — the
# load-and-decode-to-native + triple-pattern-resolution gate (the ONLY like-for-like axis vs
# hdt-cpp/java; query-over-HDT is OUT OF SCOPE). The advisory hdt_load_s / hdt_vs_ntgz_load_s
# (a contention-robust RATIO) are trend-only (NOT in scripts/perf-gate.py). Registry:
# bench/benchmarks.toml (hdt-suite); details: bench/hdt/README.md.
HDT_RUN=bench/hdt/run.sh
# Solid WAC/ACP per-commit hook (sq-k0km). Like RSP/HDT the runner is a crate
# EXAMPLE (sparq-solid's `bench`) that only needs `cargo`. run.sh is the self-asserting entry
# point: it materialises the in-crate WAC + ACP fixtures and asserts every DETERMINISTIC
# STRUCTURAL count (named-graph/quad/auth-triple/authorized-row counts — a pure function of the
# FIXED fixture, byte-stable across machines) vs bench/solid/expected.tsv (exit 1 on drift), then
# prints the `<metric>\t<value>\tcount` contract. These light up the dashboard's Solid/access-
# control family row (familyOf: `solid_*` -> solid). Registry: bench/benchmarks.toml
# (solid-wac-bench); details: bench/solid/README.md.
SOLID_RUN=bench/solid/run.sh
# sparq-nlq OFFLINE NL->SPARQL per-commit hook (sq-k0km). Crate-example runner
# (sparq-nlq's `bench`), cargo-only, FULLY OFFLINE (no network, no `live` feature). run.sh pins
# N and asserts the DETERMINISTIC counts (synthetic triples / grounded-prompt chars / ask rows /
# repair rounds — a pure function of N + the synthetic schema) vs bench/nlq/expected.tsv (exit 1
# on drift), then prints the `<metric>\t<value>\t<unit>` contract. These light up the dashboard's
# GenAI family row (familyOf: `nlq_*` -> genai). HONESTY: offline deterministic core only; the
# sibling GenAI suites (sim-olympics-eval / introspect-olympics) need the gitignored 1.78M-triple
# olympics.nt dataset (NOT in CI) and the GPU suite needs a GPU backend (NOT on gh runners) — both
# are EC2/dataset-gathered, NOT wired here. Registry: bench/benchmarks.toml (nlq-offline-bench).
NLQ_RUN=bench/nlq/run.sh
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
RES="$TMP/res.tsv"; : > "$RES"
add() { printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$RES"; }

emit_json() {
  python3 - "$RES" <<'PY'
import sys, json
rows = [l.rstrip("\n").split("\t") for l in open(sys.argv[1]) if l.strip()]
print(json.dumps([{"name": n, "unit": u, "value": float(v)} for n, u, v in rows], indent=2))
PY
}

# (sq-3ul2n.2) assert_wasm_simd128 — prove the root-invoked wasm32 gate build actually
# carries +simd128. `+simd128` lives in the WORKSPACE .cargo/config.toml (not a crate-level file
# cargo discovers by CWD), so every wasm32 build — this gate build AND the shipped wasm-pack build —
# gets it identically. The `release-wasm` profile's `strip = "symbols"` REMOVES the tiny
# `target_features` custom section that records the feature, so we cannot read it off the
# byte-ratchet artifact directly. Instead we build ONE probe with stripping disabled
# (CARGO_PROFILE_RELEASE_WASM_STRIP=none) — identical profile / config / target otherwise — and
# assert its `target_features` section contains simd128. It shares the dependency compile cache with
# the ratchet build (same target dir; only the final crate re-links), and it CLOBBERS the ratchet
# `.wasm` at that path, so this must be called ONLY AFTER wasm_bundle_bytes (and wasm_opt) have read
# the stripped artifact. HARD failure (exit 1) if simd128 is absent: a silent drop would ship the
# gate build without SIMD while the crate-dir build had it (the exact parity gap this closes).
# Skipped gracefully when the wasm target or python3 is unavailable — same posture as the byte metric.
assert_wasm_simd128() {
  rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown || return 0
  command -v python3 >/dev/null 2>&1 || return 0
  [ -f scripts/check-wasm-features.py ] || return 0
  local probe="target/wasm32-unknown-unknown/release-wasm/sparq_wasm.wasm"
  if CARGO_PROFILE_RELEASE_WASM_STRIP=none cargo build --profile release-wasm -q -p sparq-wasm --target wasm32-unknown-unknown 2>/dev/null; then
    if [ -f "$probe" ] && ! python3 scripts/check-wasm-features.py "$probe" --require simd128; then
      echo "ci-bench: FATAL — wasm gate build is missing +simd128 (workspace .cargo/config.toml regressed?)" >&2
      exit 1
    fi
  fi
}

# (sq-dzfu) measure_parse — the TIMING metric (parse_ns_per_byte) over the FIXED corpus.
# Factored out so BOTH the full run and the cheap `--parse-only` re-measure path use the IDENTICAL
# measurement (min-of-5 parse timing on the deterministic byte count). Leaves $TMP/fe populated so the
# full run can still extract store_bytes_per_triple_small from the same load summary afterwards.
# PARSE_FIX is intentionally small so the min-of-N parse timing is the least-preempted (= most
# reproducible) wall-clock signal we can get without pulling the heavy bench/parse crate into CI.
measure_parse() {
  local PARSE_FIX="${PARSE_FIX:-50000}"
  "$GEN" dump "$PARSE_FIX" "$TMP/fix.nt" >/dev/null 2>&1
  local FIX_BYTES; FIX_BYTES=$(wc -c < "$TMP/fix.nt" | tr -d ' ')
  # parse throughput on the fixed corpus. github-action-benchmark's customSmallerIsBetter treats ALL
  # metrics as smaller-is-better, so we emit parse COST as nanoseconds-per-byte (= 1000 / MB/s): a
  # parse SLOWDOWN raises it and correctly trips the alert, whereas a raw MB/s metric would alert on
  # improvements and miss regressions. min-of-5 over the deterministic byte count.
  : > "$TMP/fixloads"
  for _ in 1 2 3 4 5; do
    "$CLI" bench "$TMP/fix.nt" ntriples "$Q" 1 count >/dev/null 2>"$TMP/fe" || true
    grep -oE 'in [0-9.]+s' "$TMP/fe" | head -1 | grep -oE '[0-9.]+' | head -1 >> "$TMP/fixloads" || true
  done
  local fixbest; fixbest=$(sort -n "$TMP/fixloads" | head -1)
  if [ -n "${fixbest:-}" ] && [ "${FIX_BYTES:-0}" -gt 0 ]; then
    local nspb; nspb=$(python3 -c "import sys;print(round(float(sys.argv[1])*1e9/float(sys.argv[2]),4))" "$fixbest" "$FIX_BYTES")
    add parse_ns_per_byte ns/byte "$nspb"
  fi
}

# (sq-dzfu) --parse-only: measure ONLY the TIMING metric and emit it, then exit. This is the
# cheap re-measure path the perf-gate shells out to (best-of-N) — it deliberately skips the big corpus,
# the wasm build, and every well-known suite so a re-measure of the noisy parse metric is fast.
if [ "$PARSE_ONLY" = "1" ]; then
  measure_parse
  emit_json > "$OUT"
  echo "wrote $OUT (--parse-only):"; cat "$OUT"
  exit 0
fi

# (sq-6vshe.6, MAINTAINER-DIRECTED) --deterministic-only: emit ONLY the DETERMINISTIC
# byte-count / memory-layout metrics the perf-gate HARD-gates, then exit. This is the FAST per-PR /
# merge_group form (bench.yml): NO timing loops, NO well-known / crate-example suites. Every command
# here is byte-for-byte the same one the full run below uses (one load summary for store/dict bytes,
# one compressed-profile load, one fixed-scale small load, one wasm build) — so the ratchet compares
# the identical values, just measured without the noisy latency work. `add` is idempotent-per-name;
# the full run measures these too when it runs on the nightly EC2 lane.
if [ "$DET_ONLY" = "1" ]; then
  "$GEN" dump "$SCALE" "$TMP/data.nt" >/dev/null 2>&1
  # store/dict bytes-per-{triple,term} — DETERMINISTIC memory-layout from the load summary line.
  "$CLI" bench "$TMP/data.nt" ntriples "$Q" 1 count >/dev/null 2>"$TMP/e" || true
  dbtriple=$(grep -oE '\([0-9]+ B/triple\)' "$TMP/e" | head -1 | grep -oE '[0-9]+' | head -1)
  [ -n "${dbtriple:-}" ] && add store_bytes_per_triple bytes "$dbtriple"
  dbterm=$(grep -oE '[0-9]+ B/term' "$TMP/e" | head -1 | grep -oE '[0-9]+' | head -1)
  [ -n "${dbterm:-}" ] && add dict_bytes_per_term bytes "$dbterm"
  # compressed-profile B/triple (SPARQ_STORE_PROFILE=compressed => into_compressed()) — DETERMINISTIC.
  SPARQ_STORE_PROFILE=compressed "$CLI" bench "$TMP/data.nt" ntriples "$Q" 1 count >/dev/null 2>"$TMP/e_comp" || true
  dcbtriple=$(grep -oE '\([0-9]+ B/triple\)' "$TMP/e_comp" | head -1 | grep -oE '[0-9]+' | head -1)
  [ -n "${dcbtriple:-}" ] && add comp_store_bytes_per_triple bytes "$dcbtriple"
  # store bytes-per-triple at the SECOND (fixed 50k) scale — catches per-triple-overhead regressions.
  DET_FIX="${PARSE_FIX:-50000}"
  "$GEN" dump "$DET_FIX" "$TMP/fix.nt" >/dev/null 2>&1
  "$CLI" bench "$TMP/fix.nt" ntriples "$Q" 1 count >/dev/null 2>"$TMP/fe" || true
  dbtriple2=$(grep -oE '\([0-9]+ B/triple\)' "$TMP/fe" | head -1 | grep -oE '[0-9]+' | head -1)
  [ -n "${dbtriple2:-}" ] && add store_bytes_per_triple_small bytes "$dbtriple2"
  # wasm bundle size (bytes, DETERMINISTIC) — same release-wasm profile the shipped bundle uses.
  if rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown; then
    if cargo build --profile release-wasm -q -p sparq-wasm --target wasm32-unknown-unknown 2>/dev/null; then
      DET_WASM=$(ls target/wasm32-unknown-unknown/release-wasm/*.wasm 2>/dev/null | head -1)
      [ -n "${DET_WASM:-}" ] && add wasm_bundle_bytes bytes "$(wc -c < "$DET_WASM" | tr -d ' ')"
    fi
  fi
  # (sq-3ul2n.2) assert the gate build carries +simd128 — AFTER wasm_bundle_bytes read the
  # stripped artifact (this clobbers it with an unstripped probe; nothing below reuses it).
  assert_wasm_simd128
  emit_json > "$OUT"
  echo "wrote $OUT (--deterministic-only):"; cat "$OUT"
  exit 0
fi

"$GEN" dump "$SCALE" "$TMP/data.nt" >/dev/null 2>&1

# load (seconds, smaller better) — the "loaded … in Xs" line goes to stderr; min over 3 runs.
: > "$TMP/loads"
for _ in 1 2 3; do
  "$CLI" bench "$TMP/data.nt" ntriples "$Q" 1 count >/dev/null 2>"$TMP/e" || true
  grep -oE 'in [0-9.]+s' "$TMP/e" | head -1 | grep -oE '[0-9.]+' | head -1 >> "$TMP/loads" || true
done
loadbest=$(sort -n "$TMP/loads" | head -1)
[ -n "${loadbest:-}" ] && add load_s s "$loadbest"

# Memory regression metrics (DETERMINISTIC — not runner-noise) from the load summary line
# "store ~X GB (W B/triple), dict ~Y GB (… Z B/term)". These directly track memory efficiency
# across commits, satisfying the "no regressions in memory usage" requirement.
btriple=$(grep -oE '\([0-9]+ B/triple\)' "$TMP/e" | head -1 | grep -oE '[0-9]+' | head -1)
[ -n "${btriple:-}" ] && add store_bytes_per_triple bytes "$btriple"
bterm=$(grep -oE '[0-9]+ B/term' "$TMP/e" | head -1 | grep -oE '[0-9]+' | head -1)
[ -n "${bterm:-}" ] && add dict_bytes_per_term bytes "$bterm"

# (sq-7d3dj.32.2.5) comp_store_bytes_per_triple — DETERMINISTIC compressed-profile
# B/triple ratchet. Mirrors the raw store_bytes_per_triple stanza exactly, differing only in that
# SPARQ_STORE_PROFILE=compressed instructs load_quiet to call into_compressed() after the raw
# index-build, so the stderr load summary line reports the compressed store's heap_bytes()/len().
# The {:.0} format in load() is an INTEGER (rounded) — runner-noise-immune, mode:auto gated.
# Mode=auto, threshold=2%: floor seeded at 56 (SPQCPRM2 V2 post-#1824, 200k scale, 3 consecutive
# runs, all identical). The floor auto-ratchets DOWN on a genuine improvement, never auto-raises.
SPARQ_STORE_PROFILE=compressed "$CLI" bench "$TMP/data.nt" ntriples "$Q" 1 count >/dev/null 2>"$TMP/e_comp" || true
cbtriple=$(grep -oE '\([0-9]+ B/triple\)' "$TMP/e_comp" | head -1 | grep -oE '[0-9]+' | head -1)
[ -n "${cbtriple:-}" ] && add comp_store_bytes_per_triple bytes "$cbtriple"

# Two more DETERMINISTIC (runner-noise-immune) regression GATES on a FIXED corpus.
# The bytes-per-triple figures vary with scale (fixed per-graph overhead amortises differently),
# so a SECOND, fixed scale catches per-triple-overhead regressions that the primary scale hides;
# and a parse-cost metric on a FIXED byte count tracks raw ingest throughput. The corpus comes
# from the DETERMINISTIC synthetic generator (same bytes every run, independent of $SCALE), so
# its size + memory layout are reproducible — these are the load-bearing gates on noisy runners
# (the wall-clock query latencies above are trend-only).
#
# The parse_ns_per_byte (TIMING) measurement is factored into measure_parse() above so the perf-gate's
# best-of-N re-measure path (--parse-only) uses the IDENTICAL min-of-5 loop. It leaves $TMP/fe populated
# with the load summary so the deterministic store_bytes_per_triple_small can be extracted right after.
measure_parse

# store bytes-per-triple at the SECOND (fixed) scale — fully deterministic memory layout.
btriple2=$(grep -oE '\([0-9]+ B/triple\)' "$TMP/fe" | head -1 | grep -oE '[0-9]+' | head -1)
[ -n "${btriple2:-}" ] && add store_bytes_per_triple_small bytes "$btriple2"

# query latencies — bench reports min-of-iters micros per query (TSV: name<TAB>rows<TAB>micros).
for mode in count materialize json; do
  "$CLI" bench "$TMP/data.nt" ntriples "$Q" 3 "$mode" 2>/dev/null > "$TMP/o" || true
  while IFS=$'\t' read -r name _rows us; do
    [ -n "${us:-}" ] && add "${name}_${mode}_us" us "$us"
  done < "$TMP/o"
done

# operator-coverage latencies — one *.rq per SPARQL operator family
# (BGP/star/chain/triangle, UNION, OPTIONAL/!bound, MINUS, FILTER {num,string,IN,EXISTS},
# BIND, VALUES, aggregates+GROUP BY+HAVING, DISTINCT, ORDER BY+LIMIT/OFFSET, property paths
# {+,*,?,seq,alt,inverse,negated-set}, subquery, ASK/CONSTRUCT/DESCRIBE). Same TSV runner +
# naming as the query latencies, namespaced `op_<query>_<mode>_us`. Graph forms (CONSTRUCT/
# DESCRIBE) are routed by `bench` to the construct/describe executor (rows = produced triples).
# Wall-clock / trend-only: these are NOT in the deterministic perf-gate (scripts/perf-gate.py).
if [ -d "$OPQ" ]; then
  for mode in count materialize json; do
    "$CLI" bench "$TMP/data.nt" ntriples "$OPQ" 3 "$mode" 2>/dev/null > "$TMP/op" || true
    while IFS=$'\t' read -r name rows us; do
      # skip any query that errored (rows == ERROR) so the JSON stays valid + numeric.
      [ "${rows:-}" = "ERROR" ] && continue
      [ -n "${us:-}" ] && add "op_${name}_${mode}_us" us "$us"
    done < "$TMP/op"
  done
fi

# SP2Bench per-commit subset (sq-0jp). Builds+caches the real Freiburg sp2b_gen
# and generates a FIXED, deterministic 250k-triple DBLP-in-RDF corpus (Turtle), then runs the
# 14 sub-second canonical queries (the 3 pathological ones — q05a/q06/q12a — are in
# queries-heavy/ for the EC2/nightly tier). Emits `sp2b_<query>_<mode>_us` (trend-only, NOT in
# scripts/perf-gate.py) AND a HARD expected-rows equality check on count mode: a solution-count
# drift on the fixed corpus is a correctness regression and fails the run. The whole block is
# guarded — if the generator can't be fetched/built (no network/g++), it is skipped gracefully
# so ci-bench still emits valid JSON for the rest of the metrics.
if [ -x "$SP2B_GEN" ] && [ -d "$SP2B_Q" ]; then
  if SP2B_CORPUS="$("$SP2B_GEN" "$SP2B_TRIPLES" 2>/dev/null)" && [ -s "${SP2B_CORPUS:-}" ]; then
    for mode in count materialize json; do
      "$CLI" bench "$SP2B_CORPUS" turtle "$SP2B_Q" 3 "$mode" 2>/dev/null > "$TMP/sp2b.$mode" || true
      while IFS=$'\t' read -r name rows us; do
        [ "${rows:-}" = "ERROR" ] && continue
        [ -n "${us:-}" ] && add "sp2b_${name}_${mode}_us" us "$us"
      done < "$TMP/sp2b.$mode"
    done
    # HARD differential: count-mode solution counts must match the committed expected sizes.
    if [ -f "$SP2B_EXP" ]; then
      sp2b_fail=0
      while IFS=$'\t' read -r q exp; do
        case "$q" in \#*|"") continue;; esac
        got=$(awk -F'\t' -v k="$q" '$1==k{print $2}' "$TMP/sp2b.count")
        if [ "${got:-MISSING}" != "$exp" ]; then
          echo "ERROR: sp2b $q rows=${got:-MISSING} expected=$exp (correctness regression)" >&2
          sp2b_fail=1
        fi
      done < "$SP2B_EXP"
      [ "$sp2b_fail" = 0 ] || exit 1
    fi
  else
    echo "note: sp2b skipped (generator unavailable — no network/g++?)" >&2
  fi
fi

# EC2/nightly-only SP2Bench heavy queries and full reference scales.
# The runner applies a timeout to each query and fails closed on result-size drift.
if [ "${SP2B_EC2:-0}" = 1 ]; then
  bench/sp2b/run-ec2.sh > "$TMP/sp2b-ec2.tsv"
  while IFS=$'\t' read -r name _rows us; do
    [ -n "${us:-}" ] && add "$name" us "$us"
  done < "$TMP/sp2b-ec2.tsv"
fi

# DBPSB/FEASIBLE per-commit subset. fetch.sh downloads ONE sha256-pinned DBpedia
# Databus slice (mappingbased-objects_lang=en 2019.09.01, CC-BY-SA — see bench/dbpsb/README.md)
# and emits a FIXED, deterministic 750k-triple N-Triples cut, then runs the 13 sub-second
# curated FEASIBLE/DBPSB queries (BGP/star/chain joins, OPTIONAL, UNION, FILTER, DISTINCT,
# ORDER+LIMIT, aggregates, subquery, negation-by-OPTIONAL+!bound). Emits `dbpsb_<query>_<mode>_us`
# (trend-only, NOT in scripts/perf-gate.py) AND a HARD expected-rows equality check on count
# mode (a solution-count drift on the fixed cut is a correctness regression and fails the run).
# Guarded — if the slice can't be fetched (no network), the whole block is skipped gracefully so
# ci-bench still emits valid JSON for everything else. (CI keys actions/cache on the pinned
# URL+sha so the steady state does NO network — see bench/dbpsb/README.md "CI wiring".)
if [ -x "$DBPSB_FETCH" ] && [ -d "$DBPSB_Q" ]; then
  if DBPSB_CUT="$("$DBPSB_FETCH" "$DBPSB_TRIPLES" 2>/dev/null)" && [ -s "${DBPSB_CUT:-}" ]; then
    for mode in count materialize json; do
      "$CLI" bench "$DBPSB_CUT" ntriples "$DBPSB_Q" 3 "$mode" 2>/dev/null > "$TMP/dbpsb.$mode" || true
      while IFS=$'\t' read -r name rows us; do
        [ "${rows:-}" = "ERROR" ] && continue
        [ -n "${us:-}" ] && add "dbpsb_${name}_${mode}_us" us "$us"
      done < "$TMP/dbpsb.$mode"
    done
    # HARD differential: count-mode solution counts must match the committed expected sizes.
    if [ -f "$DBPSB_EXP" ]; then
      dbpsb_fail=0
      while IFS=$'\t' read -r q exp; do
        case "$q" in \#*|"") continue;; esac
        got=$(awk -F'\t' -v k="$q" '$1==k{print $2}' "$TMP/dbpsb.count")
        if [ "${got:-MISSING}" != "$exp" ]; then
          echo "ERROR: dbpsb $q rows=${got:-MISSING} expected=$exp (correctness regression)" >&2
          dbpsb_fail=1
        fi
      done < "$DBPSB_EXP"
      [ "$dbpsb_fail" = 0 ] || exit 1
    fi
  else
    echo "note: dbpsb skipped (pinned slice unavailable — no network?)" >&2
  fi
fi

# WatDiv per-commit subset (sq-13i). Builds+caches the real Waterloo generator and
# emits a FIXED, deterministic SF=1 corpus (N-Triples), then runs the 16 sub-ms Basic-Testing
# queries (Linear/Star/Snowflake/Complex; the 4 SF=1-empty ones live in queries-heavy/ for the
# EC2/nightly tier). Emits `watdiv_sf<SF>_<query>_<mode>_us` (trend-only, NOT in
# scripts/perf-gate.py) AND a HARD expected-rows equality check on count mode (a solution-count
# drift on the fixed corpus is a correctness regression and fails the run). Guarded on g++ (the
# generator needs g++ + Boost; gen.sh handles Boost) — if g++/the generator is unavailable, the
# whole block is skipped gracefully so ci-bench still emits valid JSON for everything else. (CI
# keys actions/cache on /tmp/watdiv so the steady state does NO rebuild — see bench/watdiv/README.md.)
#
# (sq-1wrw) The metric stem carries the scale factor as an `_sf<SF>` token so the per-
# commit tier (`watdiv_sf1_*`) and the EC2/nightly full-scale tier (e.g. `watdiv_sf1000_*`) form
# DISTINCT github-action-benchmark series instead of colliding into one (gh-action-benchmark keys
# series by metric NAME, so an SF-less stem makes the two tiers overwrite each other and a scaling
# axis can never form). The token is exactly the `_sf<digits>` form the dashboard's scaling axis
# parses (bench/dashboard/dashboard.js sizeAxisOf), so the engine-vs-scale-factor scaling chart
# (sq-viby) lights up automatically. SF is digit-sanitised so the token stays a clean metric name
# regardless of how WATDIV_SF was passed (e.g. a stray "SF=1" string).
WATDIV_SFTOK="$(printf '%s' "$WATDIV_SF" | tr -cd '0-9')"; WATDIV_SFTOK="${WATDIV_SFTOK:-1}"
if command -v g++ >/dev/null 2>&1 && [ -x "$WATDIV_GEN" ] && [ -d "$WATDIV_Q" ]; then
  if WATDIV_CORPUS="$(bash "$WATDIV_GEN" "$WATDIV_SF" 2>/dev/null)" && [ -s "${WATDIV_CORPUS:-}" ]; then
    for mode in count materialize json; do
      "$CLI" bench "$WATDIV_CORPUS" ntriples "$WATDIV_Q" 3 "$mode" 2>/dev/null > "$TMP/watdiv.$mode" || true
      while IFS=$'\t' read -r name rows us; do
        [ "${rows:-}" = "ERROR" ] && continue
        [ -n "${us:-}" ] && add "watdiv_sf${WATDIV_SFTOK}_${name}_${mode}_us" us "$us"
      done < "$TMP/watdiv.$mode"
    done
    # HARD differential: count-mode solution counts must match the committed expected sizes.
    if [ -f "$WATDIV_EXP" ]; then
      watdiv_fail=0
      while IFS=$'\t' read -r q exp; do
        case "$q" in \#*|"") continue;; esac
        got=$(awk -F'\t' -v k="$q" '$1==k{print $2}' "$TMP/watdiv.count")
        if [ "${got:-MISSING}" != "$exp" ]; then
          echo "ERROR: watdiv $q rows=${got:-MISSING} expected=$exp (correctness regression)" >&2
          watdiv_fail=1
        fi
      done < "$WATDIV_EXP"
      [ "$watdiv_fail" = 0 ] || exit 1
    fi
  else
    echo "note: watdiv skipped (generator unavailable — no network/g++/boost?)" >&2
  fi
else
  echo "note: watdiv skipped (g++ not on PATH)" >&2
fi

# BSBM Explore-mix per-commit subset. gen.sh fetches+caches the prebuilt bsbmtools
# distribution (JRE-only; sha256-pinned zip) and emits a FIXED, deterministic -pc 300 corpus
# (~116k N-Triples), then runs the 11 sub-ms Explore queries. The Explore mix includes a CONSTRUCT
# (query12) + a DESCRIBE (query09), so the HARD expected-rows diff runs against MATERIALIZE mode
# (the graph-valued forms report produced-triple counts through the bench runner; counts are
# mode-independent for this mix). Emits `bsbm_<query>_<mode>_us` (trend-only, NOT in
# scripts/perf-gate.py). Guarded on java + unzip (gen.sh needs a JRE to run the generator and
# unzip to unpack the distribution) — if either is absent, the whole block is skipped gracefully
# so ci-bench still emits valid JSON. (CI keys actions/cache on /tmp/bsbm — see bench/bsbm/README.md.)
if command -v java >/dev/null 2>&1 && command -v unzip >/dev/null 2>&1 && [ -x "$BSBM_GEN" ] && [ -d "$BSBM_Q" ]; then
  if BSBM_CORPUS="$(bash "$BSBM_GEN" "$BSBM_PC" 2>/dev/null)" && [ -s "${BSBM_CORPUS:-}" ]; then
    for mode in count materialize json; do
      "$CLI" bench "$BSBM_CORPUS" ntriples "$BSBM_Q" 3 "$mode" 2>/dev/null > "$TMP/bsbm.$mode" || true
      while IFS=$'\t' read -r name rows us; do
        [ "${rows:-}" = "ERROR" ] && continue
        [ -n "${us:-}" ] && add "bsbm_${name}_${mode}_us" us "$us"
      done < "$TMP/bsbm.$mode"
    done
    # HARD differential: result sizes must match the committed expected sizes. The mix includes a
    # CONSTRUCT + a DESCRIBE, so we diff MATERIALIZE mode (produced-triple counts for graph forms).
    if [ -f "$BSBM_EXP" ]; then
      bsbm_fail=0
      while IFS=$'\t' read -r q exp; do
        case "$q" in \#*|"") continue;; esac
        got=$(awk -F'\t' -v k="$q" '$1==k{print $2}' "$TMP/bsbm.materialize")
        if [ "${got:-MISSING}" != "$exp" ]; then
          echo "ERROR: bsbm $q rows=${got:-MISSING} expected=$exp (correctness regression)" >&2
          bsbm_fail=1
        fi
      done < "$BSBM_EXP"
      [ "$bsbm_fail" = 0 ] || exit 1
    fi
  else
    echo "note: bsbm skipped (generator unavailable — no network/java?)" >&2
  fi
else
  echo "note: bsbm skipped (java/unzip not on PATH)" >&2
fi

# LUBM reasoning suite per-commit. Unlike watdiv/bsbm (inline sp2b-style), LUBM's
# run.sh is the self-asserting entry point: it ensures the LUBM(1) corpus + Univ-Bench TBox
# (gen.sh), materializes the OWL-RL closure with `sparq-cli reason`, runs the extensional tier on
# the raw ABox + the entailed tier on the closure (count mode), and asserts EVERY count vs
# expected-rows.tsv internally (exit 1 on any mismatch). So we invoke run.sh and harvest its
# count-mode TSV (it prints `<query>\t<rows>\t<min_us>` lines for both tiers on stdout) into
# `lubm_<query>_count_us` (trend-only, NOT in scripts/perf-gate.py). Guarded on javac + rapper
# (gen.sh compiles the UBA generator with javac and converts RDF/XML via rapper) — if either is
# absent, skipped gracefully so ci-bench still emits valid JSON. run.sh failing (a reasoner OR
# engine correctness regression) fails the whole ci-bench run. (CI keys actions/cache on /tmp/lubm.)
if command -v javac >/dev/null 2>&1 && command -v rapper >/dev/null 2>&1 && [ -x "$LUBM_RUN" ]; then
  if CLI="$CLI" ITERS=3 bash "$LUBM_RUN" > "$TMP/lubm.tsv" 2>"$TMP/lubm.err"; then
    while IFS=$'\t' read -r name rows us; do
      [ "${rows:-}" = "ERROR" ] && continue
      [ -n "${us:-}" ] && add "lubm_${name}_count_us" us "$us"
    done < "$TMP/lubm.tsv"
  else
    # run.sh exits non-zero ONLY on a hard correctness mismatch (the gen/tool guards above already
    # ensure the toolchain is present), so surface its diagnostics and fail the run.
    echo "ERROR: lubm run.sh failed (correctness regression or reasoner/engine error)" >&2
    cat "$TMP/lubm.err" >&2 || true
    exit 1
  fi
else
  echo "note: lubm skipped (javac/rapper not on PATH)" >&2
fi

# Deep Taxonomy reasoning suite per-commit (sq-1hgz). Like the LUBM hook above, this
# suite's run.sh is the self-asserting entry point: it reuses bench/inference/gen_deeptaxonomy.py
# (via gen.sh) to build the DT N3 corpus at each depth tier, materializes the N3 forward closure
# with `sparq-cli reason … n3`, runs query.rq over the closure, and asserts EVERY closure
# triple-count + query row-count vs expected.tsv internally (exit 1 on any mismatch). So we invoke
# run.sh and harvest its metric TSV (it prints `<metric>\t<value>\t<unit>` lines on stdout) into
# `deeptax_d<DEPTH>_{closure_s,query_us,closure_triples}` (trend-only, NOT in scripts/perf-gate.py;
# closure_triples is a deterministic structural metric, never an alert source). UNLIKE
# watdiv/bsbm/lubm this suite needs NO heavyweight toolchain — only python3 (always present) + the
# built sparq-cli — so it runs on the per-commit tier by default at a SMALL depth pair (dt1k+dt10k;
# closures 2,001 / 20,001 triples, ~0.1 s total). dt100k is opt-in via DEEPTAX_DEPTHS for the
# EC2/nightly tier. Guarded on python3 + run.sh present, mirroring the LUBM guard shape: if python3
# is somehow absent it is skipped gracefully so ci-bench still emits valid JSON. run.sh failing (a
# reasoner OR engine correctness regression) fails the whole ci-bench run.
if command -v python3 >/dev/null 2>&1 && [ -x "$DEEPTAX_RUN" ]; then
  if CLI="$CLI" ITERS=3 DEEPTAX_DEPTHS="$DEEPTAX_DEPTHS" bash "$DEEPTAX_RUN" > "$TMP/deeptax.tsv" 2>"$TMP/deeptax.err"; then
    while IFS=$'\t' read -r name value unit; do
      [ -n "${name:-}" ] && [ -n "${value:-}" ] && add "$name" "${unit:-}" "$value"
    done < "$TMP/deeptax.tsv"
  else
    # run.sh exits non-zero ONLY on a hard correctness mismatch (the python3 guard above ensures
    # the toolchain is present), so surface its diagnostics and fail the run.
    echo "ERROR: deep-taxonomy run.sh failed (closure-size regression or reasoner/engine error)" >&2
    cat "$TMP/deeptax.err" >&2 || true
    exit 1
  fi
else
  echo "note: deep-taxonomy skipped (python3 not on PATH)" >&2
fi

# OWL sameAs equality micro-suite per-commit (sq-msl6, design A3 + A5). Exactly the
# deep-taxonomy hook shape: run.sh is the self-asserting entry point — it reuses
# bench/owl-sameas/gen_sameas.py (via gen.sh) to build K owl:sameAs equivalence classes of N members
# per tier, materializes the OWL-RL closure with `sparq-cli reason … owl`, runs query.rq over the
# closure, and asserts BOTH the closure triple-count (K·N·(N+M)) AND the query row-count (K·N) vs
# expected.tsv internally (exit 1 on any mismatch). So we invoke run.sh and harvest its metric TSV
# (it prints `<metric>\t<value>\t<unit>` lines on stdout) into
# `sameas_size<TIER>_{closure_s,query_us,closure_triples}` (trend-only, NOT in scripts/perf-gate.py;
# closure_triples is a deterministic structural metric, never an alert source). Like deep-taxonomy
# this suite needs NO heavyweight toolchain — only python3 (always present) + the built sparq-cli —
# so it runs on the per-commit tier by default at a SMALL tier pair (N=8 + N=32; closures 352 / 4,480
# triples, sub-second total). N=256 is opt-in via SAMEAS_TIERS for the EC2/nightly tier. Guarded on
# python3 + run.sh present (same shape as the deep-taxonomy guard): if python3 is somehow absent it
# is skipped gracefully so ci-bench still emits valid JSON. run.sh failing (an owl:sameAs reasoner
# regression) fails the whole ci-bench run.
if command -v python3 >/dev/null 2>&1 && [ -x "$SAMEAS_RUN" ]; then
  if CLI="$CLI" ITERS=3 SAMEAS_TIERS="$SAMEAS_TIERS" bash "$SAMEAS_RUN" > "$TMP/sameas.tsv" 2>"$TMP/sameas.err"; then
    while IFS=$'\t' read -r name value unit; do
      [ -n "${name:-}" ] && [ -n "${value:-}" ] && add "$name" "${unit:-}" "$value"
    done < "$TMP/sameas.tsv"
  else
    # run.sh exits non-zero ONLY on a hard correctness mismatch (the python3 guard above ensures
    # the toolchain is present), so surface its diagnostics and fail the run.
    echo "ERROR: owl-sameas run.sh failed (closure-size regression or owl:sameAs reasoner error)" >&2
    cat "$TMP/sameas.err" >&2 || true
    exit 1
  fi
else
  echo "note: owl-sameas skipped (python3 not on PATH)" >&2
fi

# SHACL validation suite per-commit (sq-7iai). Reuses the LUBM(1) ABox, so it shares
# the javac + rapper guard (gen.sh delegates to bench/lubm/gen.sh). run.sh is the self-asserting
# entry point: it validates the ABox against the 5 committed shape graphs with examples/bench_shacl
# and asserts each workload's violations/conforms/focus_nodes vs expected.tsv internally (exit 1 on
# any drift). The example is built on demand (sparq-shacl is isolated — not a sparq-cli dependency —
# so the bench runner is its own --example binary). We harvest run.sh's `<workload>\t<violations>\t
# <validate_us>` stdout into the ADVISORY `shacl_<workload>_validate_us` (trend-only, NOT in
# scripts/perf-gate.py). run.sh failing (a SHACL correctness regression) fails the whole ci-bench
# run, exactly like LUBM. Skipped gracefully when javac/rapper are absent (PRs install no toolchain).
SHACL_BIN=target/release/examples/bench_shacl
if command -v javac >/dev/null 2>&1 && command -v rapper >/dev/null 2>&1 && [ -x "$SHACL_RUN" ]; then
  # Always (re)build: rust-cache restores target/, so a file-exists check can run a STALE
  # bench_shacl. Let cargo's own staleness detection decide (a no-op rebuild is cheap), and do
  # NOT swallow the build error (`|| true`) — a real compile failure must fail the gate, not
  # silently skip the SHACL correctness check on main. Mirrors how the other suites assume a
  # freshly-built binary.
  if ! cargo build --release -q -p sparq-shacl --example bench_shacl; then
    echo "ERROR: bench_shacl example failed to build" >&2
    exit 1
  fi
  if BENCH_SHACL="$SHACL_BIN" ITERS=3 bash "$SHACL_RUN" > "$TMP/shacl.tsv" 2>"$TMP/shacl.err"; then
    while IFS=$'\t' read -r name violations us; do
      [ "${violations:-}" = "ERROR" ] && continue
      [ -n "${us:-}" ] && add "shacl_${name}_validate_us" us "$us"
    done < "$TMP/shacl.tsv"
  else
    echo "ERROR: shacl run.sh failed (correctness regression)" >&2
    cat "$TMP/shacl.err" >&2 || true
    exit 1
  fi
else
  echo "note: shacl skipped (javac/rapper not on PATH)" >&2
fi

# GeoSPARQL suite per-commit (sq-tf8n). Unlike SHACL/LUBM this suite needs NO
# heavyweight toolchain — only cargo (the corpus is pure Rust via bench_geo gen). run.sh is the
# self-asserting entry point: it ensures the fixed ~100k-point corpus (bench/geo/gen.sh), runs
# bench_geo in `bench` mode, and asserts each workload's result-set SIZE / compliance pass COUNT
# vs expected.tsv (counts-not-coords; exit 1 on drift). The example is built on demand (sparq-geo
# is isolated — not a sparq-cli dependency — so the runner is its own --example). We harvest the
# `<name>\t<count>\t<us>` stdout into the ADVISORY `geo_<name>_us` (trend-only, NOT in
# scripts/perf-gate.py) and ALSO route the geo_compliance_pass row to the HARD-gated DEFICIT
# `geo_compliance_deficit` (= GEO_COMPLIANCE_MAX - passed; mode:auto in bench/perf-baseline.json,
# so geo compliance can only TIGHTEN). run.sh failing (a geo correctness regression) fails the
# whole ci-bench run, exactly like LUBM/SHACL. Skipped gracefully when cargo is absent.
GEO_BIN=target/release/examples/bench_geo
GEO_COMPLIANCE_MAX=25
# PR-tier skip: run only on main (or locally, where GITHUB_REF is unset) — mirrors the
# main-only-toolchain skip the LUBM/SHACL hooks get for free (javac/rapper are installed only on
# main), keeping the sparq-geo example build + ~100k-point corpus gen/load OFF the per-PR path.
# Same pattern as the FTS block below (sq-ustq, #186). On a LOCAL run GITHUB_REF is unset so
# devs / bench/geo/run.sh still work; on a PR build GITHUB_REF is the PR ref so we skip.
GEO_REF="${GITHUB_REF:-}"
if [ -n "$GEO_REF" ] && [ "$GEO_REF" != "refs/heads/main" ]; then
  echo "note: geo skipped (PR tier — sparq-geo example build/run is main-only, like the javac/rapper suites)" >&2
elif command -v cargo >/dev/null 2>&1 && [ -x "$GEO_RUN" ]; then
  # Always (re)build: rust-cache restores target/, so a file-exists check can run a STALE
  # bench_geo. Let cargo's staleness detection decide (a no-op rebuild is cheap); do NOT
  # swallow the build error — a real compile failure must fail the gate, not silently skip.
  if ! cargo build --release -q -p sparq-geo --example bench_geo; then
    echo "ERROR: bench_geo example failed to build" >&2
    exit 1
  fi
  if BENCH_GEO="$GEO_BIN" ITERS=3 bash "$GEO_RUN" > "$TMP/geo.tsv" 2>"$TMP/geo.err"; then
    while IFS=$'\t' read -r name count us; do
      [ "${count:-}" = "ERROR" ] && continue
      [ -n "${us:-}" ] && add "geo_${name}_us" us "$us"   # TREND-ONLY (advisory)
      # Route the compliance pass count to the HARD-gated deficit (max - passed): a
      # smaller-is-better integer that mode:auto ratchets DOWN, so coverage only tightens (G4).
      if [ "$name" = "geo_compliance_pass" ]; then
        add geo_compliance_deficit fixtures "$((GEO_COMPLIANCE_MAX - count))"
      fi
    done < "$TMP/geo.tsv"
  else
    echo "ERROR: geo run.sh failed (correctness regression)" >&2
    cat "$TMP/geo.err" >&2 || true
    exit 1
  fi
else
  # Reached only on the MAIN/local tier (the PR tier is handled by the skip branch above) when
  # cargo or the run.sh entry point is missing.
  echo "note: geo skipped (cargo not on PATH or $GEO_RUN missing)" >&2
fi

# FULL-TEXT-SEARCH suite per-commit (sq-ustq). The corpus is SYNTHETIC (generated
# in-process by examples/bench_text from a seed), so there is NO external-toolchain guard like
# the LUBM/SHACL javac/rapper — bench_text only needs `cargo`, which is ALWAYS present (incl. on
# PRs). The other suites avoid per-PR cost because their toolchains (javac/rapper/g++/JRE) are
# main-only (bench.yml installs them only on `github.ref == refs/heads/main`), so their
# `command -v` guard skips them on PRs. To match that discipline — and keep the FTS example
# build+run (release compile of bench_text + N=100000 corpus) OFF the per-PR path — we gate this
# hook to the MAIN tier: it runs on push-to-main (GITHUB_REF=refs/heads/main) and on a LOCAL run
# (GITHUB_REF unset, so devs/`bench/fts/run.sh` still work), but is SKIPPED on the PR tier. The
# deterministic correctness gate therefore runs once per merge to main (alongside the other
# example-building suites), not on every PR build. run.sh is the self-asserting entry point: it
# runs bench_text on the pinned N=100000/seed=0 corpus and asserts each workload's hit count +
# the integer index bytes-per-doc vs expected.tsv internally (exit 1 on any drift), failing the
# whole ci-bench run on a regression exactly like LUBM/SHACL. We harvest run.sh's
# `<workload>\t<count>\t<us>` stdout: the `bytes_per_doc` row feeds the DETERMINISTIC
# fts_bytes_per_doc ratchet (mode:auto, scripts/perf-gate.py), and the query rows feed ADVISORY
# timings (text_<workload>_us, trend-only, NOT in perf-gate).
FTS_BIN=target/release/examples/bench_text
# PR-tier skip: run only on main (or locally, where GITHUB_REF is unset) — mirrors the
# main-only-toolchain skip the LUBM/SHACL hooks get for free, keeping the FTS example build off PRs.
FTS_REF="${GITHUB_REF:-}"
if [ -n "$FTS_REF" ] && [ "$FTS_REF" != "refs/heads/main" ]; then
  echo "note: fts skipped (PR tier — FTS example build/run is main-only, like the javac/rapper suites)" >&2
elif command -v cargo >/dev/null 2>&1 && [ -x "$FTS_RUN" ]; then
  # Always (re)build: rust-cache restores target/, so a file-exists check can run a STALE
  # bench_text. Let cargo's staleness detection decide (a no-op rebuild is cheap); do NOT
  # swallow a real compile failure (it must fail the gate, not silently skip the FTS check).
  if ! cargo build --release -q -p sparq-text --example bench_text; then
    echo "ERROR: bench_text example failed to build" >&2
    exit 1
  fi
  if BENCH_TEXT="$FTS_BIN" ITERS=3 bash "$FTS_RUN" > "$TMP/fts.tsv" 2>"$TMP/fts.err"; then
    while IFS=$'\t' read -r name count us; do
      [ "${count:-}" = "ERROR" ] && continue
      if [ "$name" = "bytes_per_doc" ]; then
        # DETERMINISTIC (integer, runner-noise-immune) — the mode:auto ratchet metric.
        [ -n "${count:-}" ] && add fts_bytes_per_doc bytes "$count"
        # us here is the advisory index BUILD time (microseconds -> seconds for the trend).
        [ -n "${us:-}" ] && add text_build_s s "$(awk -v u="$us" 'BEGIN{printf "%.6f", u/1e6}')"
      else
        # ADVISORY per-query latency (microseconds): text_<workload>_us, trend-only.
        [ -n "${us:-}" ] && add "text_${name}_us" us "$us"
      fi
    done < "$TMP/fts.tsv"
  else
    echo "ERROR: fts run.sh failed (correctness/structure regression)" >&2
    cat "$TMP/fts.err" >&2 || true
    exit 1
  fi
else
  # Reached only on the MAIN/local tier (the PR tier is handled by the skip branch above) when
  # cargo or the run.sh entry point is missing.
  echo "note: fts skipped (cargo not on PATH or $FTS_RUN missing)" >&2
fi

# VECTOR / ANN per-commit hook (sq-v02y). Like FTS, bench_vectors only needs `cargo`
# (sparq-vectors is the ISOLATED ANN crate, not a sparq-cli dependency), which is ALWAYS present —
# so to match the main-only-toolchain skip the LUBM/SHACL hooks get for free (and keep the vector
# index BUILD + corpus off per-PR CI), we PIN this hook to the MAIN tier: it runs on push-to-main
# (GITHUB_REF=refs/heads/main) and on a LOCAL run (GITHUB_REF unset, so devs/bench/vector/run.sh
# still work), but is SKIPPED on the PR tier — copying the FTS GITHUB_REF guard verbatim.
VECTOR_BIN=target/release/examples/bench_vectors
VECTOR_REF="${GITHUB_REF:-}"
if [ -n "$VECTOR_REF" ] && [ "$VECTOR_REF" != "refs/heads/main" ]; then
  echo "note: vector skipped (PR tier — ANN example build/run is main-only, like the javac/rapper suites)" >&2
elif command -v cargo >/dev/null 2>&1 && [ -x "$VECTOR_RUN" ]; then
  # Always (re)build: rust-cache restores target/, so a file-exists check can run a STALE binary.
  # Let cargo's staleness detection decide (a no-op rebuild is cheap); do NOT swallow a real
  # compile failure (it must fail the gate, not silently skip the ANN recall check).
  # sq-k9o2: --features approx-ann is REQUIRED here. The example links `VectorIndex`
  # (the in-RAM HNSW backend, `#[cfg(feature = "approx-ann")]` in src/lib.rs) and its Cargo.toml
  # `[[example]]` block declares `required-features = ["approx-ann"]`. Naming the target WITHOUT
  # the feature is a hard `error: target bench_vectors requires the features: approx-ann` (exit
  # 101) — the failure that RED-ed main after #807 added that `required-features` line. The flag
  # is crate-scoped (`-p sparq-vectors`), so it widens NO other crate's feature set.
  if ! cargo build --release -q -p sparq-vectors --example bench_vectors --features approx-ann; then
    echo "ERROR: bench_vectors example failed to build" >&2
    exit 1
  fi
  if BENCH_VECTORS="$VECTOR_BIN" ITERS=3 bash "$VECTOR_RUN" > "$TMP/vector.tsv" 2>"$TMP/vector.err"; then
    while IFS=$'\t' read -r name count us; do
      [ "${count:-}" = "ERROR" ] && continue
      if [ "$name" = "build_s" ]; then
        # us here is the advisory HNSW index BUILD time (microseconds -> seconds for the trend);
        # the sentinel-0 count carries no gate.
        [ -n "${us:-}" ] && add vectors_build_s s "$(awk -v u="$us" 'BEGIN{printf "%.6f", u/1e6}')"
      else
        # DETERMINISTIC recall DEFICIT: vectors_<workload>_recall_at10. The value is a MILLI-scaled
        # deficit — round((1-recall)*1000) — so the emitted unit is `milli` (matching metric-labels.json),
        # NOT a bare `deficit` (which would mis-state the value's scale on the dashboard). The diskann/pq
        # deficits are mode:auto ratcheted in bench/perf-baseline.json; hnsw is floor-gated in run.sh
        # only (rayon-parallel build jitter), so its deficit is harvested as TREND-only here.
        [ -n "${count:-}" ] && add "vectors_${name}" milli "$count"
        # ADVISORY per-query latency (microseconds): vectors_<searcher>_query_us, trend-only.
        # name is `<searcher>_recall_at10`; strip the `_recall_at10` suffix for the timing stem.
        [ -n "${us:-}" ] && add "vectors_${name%_recall_at10}_query_us" us "$us"
      fi
    done < "$TMP/vector.tsv"
  else
    echo "ERROR: vector run.sh failed (recall regression)" >&2
    cat "$TMP/vector.err" >&2 || true
    exit 1
  fi
else
  echo "note: vector skipped (cargo not on PATH or $VECTOR_RUN missing)" >&2
fi

# RSP-QL streaming per-commit hook (sq-b1hn). Like FTS/vector the runner is a crate
# EXAMPLE (rsp_oracle) that only needs `cargo`, so to match the main-only-toolchain skip the
# LUBM/SHACL hooks get for free — and keep the example build off per-PR CI — we PIN this hook to the
# MAIN/local tier: it runs on push-to-main (GITHUB_REF=refs/heads/main) and on a LOCAL run
# (GITHUB_REF unset, so devs/bench/rsp/run.sh still work), but is SKIPPED on the PR tier. The
# DETERMINISTIC per-window row-count gate therefore runs once per merge to main. run.sh asserts every
# `rsp_*_rows` vs expected.tsv internally (exit 1 on drift), failing the whole ci-bench run on a
# regression exactly like LUBM/FTS. We harvest run.sh's `<metric>\t<value>\t<unit>` stdout:
# `*_rows` are DETERMINISTIC per-window counts (emitted for the dashboard's RSP-QL card; correctness
# is the run.sh internal gate, so these are trend rows not perf-gate ratchets), and the single
# `rsp_persistentdict_triples_per_s` is ADVISORY throughput (trend-only, NOT in scripts/perf-gate.py).
RSP_BIN=target/release/examples/rsp_oracle
RSP_REF="${GITHUB_REF:-}"
if [ -n "$RSP_REF" ] && [ "$RSP_REF" != "refs/heads/main" ]; then
  echo "note: rsp skipped (PR tier — RSP-QL example build/run is main-only, like the javac/rapper suites)" >&2
elif command -v cargo >/dev/null 2>&1 && [ -x "$RSP_RUN" ]; then
  # Always (re)build: rust-cache restores target/, so a file-exists check can run a STALE binary.
  # `replay_runner` (sq-3f5ay) is built alongside so run.sh's replay-FILE equivalence guard
  # actually runs here (it self-skips when the example is missing): it re-derives the same
  # per-window counts through the pinned replay FILES, catching an export/example drift the
  # in-code oracle path cannot see.
  if ! cargo build --release -q -p sparq-rsp --example rsp_oracle --example replay_runner; then
    echo "ERROR: rsp_oracle / replay_runner examples failed to build" >&2
    exit 1
  fi
  if RSP_BIN="$RSP_BIN" bash "$RSP_RUN" > "$TMP/rsp.tsv" 2>"$TMP/rsp.err"; then
    while IFS=$'\t' read -r metric value unit; do
      [ -n "${metric:-}" ] && [ -n "${value:-}" ] || continue
      # DETERMINISTIC per-window counts + the advisory throughput: forward the metric verbatim
      # (run.sh has already asserted the *_rows correctness gate, so these are trend rows).
      add "$metric" "${unit:-rows}" "$value"
    done < "$TMP/rsp.tsv"
  else
    echo "ERROR: rsp run.sh failed (per-window row-count regression)" >&2
    cat "$TMP/rsp.err" >&2 || true
    exit 1
  fi
else
  echo "note: rsp skipped (cargo not on PATH or $RSP_RUN missing)" >&2
fi

# HDT load-and-decode per-commit hook (sq-lrp9). Like RSP/FTS the runner is a crate
# EXAMPLE (bench_oracle) that only needs `cargo`, so — to match the main-only-toolchain skip the
# LUBM/SHACL hooks get for free and keep the example build off per-PR CI — we PIN this hook to the
# MAIN/local tier: it runs on push-to-main (GITHUB_REF=refs/heads/main) and on a LOCAL run
# (GITHUB_REF unset, so dev `bench/hdt/run.sh` still works), but is SKIPPED on the PR tier. The
# DETERMINISTIC load-and-decode count gate therefore runs once per merge to main. run.sh asserts
# every `count`-unit metric vs expected.tsv internally (exit 1 on drift), failing the whole ci-bench
# run on a regression exactly like LUBM/FTS/RSP. We harvest run.sh's `<metric>\t<value>\t<unit>`
# stdout: the `snikmeta_*` counts are DETERMINISTIC (emitted for the dashboard's HDT card; correctness
# is the run.sh internal gate, so trend rows not perf-gate ratchets), and hdt_load_s /
# hdt_vs_ntgz_load_s are ADVISORY (trend-only, NOT in scripts/perf-gate.py). A small HDT_BENCH_N keeps
# the advisory synthetic-archive build quick; the deterministic gate is on the fixture, not N.
HDT_BIN=target/release/examples/bench_oracle
HDT_REF="${GITHUB_REF:-}"
if [ -n "$HDT_REF" ] && [ "$HDT_REF" != "refs/heads/main" ]; then
  echo "note: hdt skipped (PR tier — HDT example build/run is main-only, like the javac/rapper suites)" >&2
elif command -v cargo >/dev/null 2>&1 && [ -x "$HDT_RUN" ]; then
  # Always (re)build: rust-cache restores target/, so a file-exists check can run a STALE binary.
  if ! cargo build --release -q -p sparq-hdt --example bench_oracle; then
    echo "ERROR: bench_oracle example failed to build" >&2
    exit 1
  fi
  if HDT_BIN="$HDT_BIN" HDT_BENCH_N="${HDT_BENCH_N:-100000}" bash "$HDT_RUN" > "$TMP/hdt.tsv" 2>"$TMP/hdt.err"; then
    while IFS=$'\t' read -r metric value unit; do
      [ -n "${metric:-}" ] && [ -n "${value:-}" ] || continue
      # DETERMINISTIC load-and-decode counts + the advisory load/ratio rows: forward verbatim
      # (run.sh has already asserted the count gate, so these are trend rows).
      add "$metric" "${unit:-count}" "$value"
    done < "$TMP/hdt.tsv"
  else
    echo "ERROR: hdt run.sh failed (load-and-decode count regression)" >&2
    cat "$TMP/hdt.err" >&2 || true
    exit 1
  fi
else
  echo "note: hdt skipped (cargo not on PATH or $HDT_RUN missing)" >&2
fi

# ZERO-KNOWLEDGE family (sq dashboard-data-feed) — feeds the dashboard's
# Zero-knowledge family (FAMILIES key `zk`, #253) so its rows show real data instead of
# "not yet reported". EVERY ZK metric name leads with the token `zk` so the dashboard's
# prefix routing (familyOf: name.split('_')[0]) buckets it into the ZK family. Three feeds,
# each grounded in an EXISTING bench (bench/benchmarks.toml zk-* registrations) — nothing
# invented:
#
#  1. zk_compose_<member>_gates  — DETERMINISTIC ultra_honk circuit gate counts for the
#     zk/compose circuit family. The headline ZK measurement. `bb gates` is the ground
#     truth (noir-optimisation SKILL §) but needs nargo+bb (NOT installed in CI), so we
#     harvest the COMMITTED, reproducible bench/zk-compose/gate_counts_latest.json (a fixed
#     toolchain pins it; re-measure with bench/zk-compose/scripts/gate_counts.sh). Gate
#     counts are toolchain-deterministic, so the committed JSON is a faithful per-commit
#     value — emitted on EVERY tier (no toolchain guard). Trend-only/advisory here (NOT in
#     scripts/perf-gate.py); a tightening is tracked on the dashboard.
#  2. zk_commit_*  — sparq-zk commitment-pipeline criterion means (RDFC10 canon, end-to-end
#     commit, leaves+fold, Poseidon2) in µs. Standalone cargo project (bench/zk), cargo-only.
#  3. zk_trace_*  — zk-trace per-operator capture overhead criterion means (traced vs
#     untraced, per plan shape) in µs. Standalone cargo project (bench/zk-trace), cargo-only.
#
# (2)+(3) are WALL-CLOCK criterion runs, so — exactly like the FTS/vector/geo hooks — they
# are PINNED to the MAIN/local tier (run on push-to-main or a LOCAL run where GITHUB_REF is
# unset; SKIPPED on the PR tier) to keep the criterion build+run off per-PR CI. A short
# measurement window (criterion --measurement-time) bounds the cost. NON-CANONICAL timing:
# the values are cross-commit TREND signals, never hard-coded into docs/tests.
# The zk-compose prove+verify suite (bench/zk-compose/scripts/prove_verify.sh) is wall-clock
# and needs nargo+bb, so it has NO CI-runnable bench — intentionally NOT emitted here.
ZK_GATES_JSON=bench/zk-compose/gate_counts_latest.json
if [ -f "$ZK_GATES_JSON" ]; then
  # Deterministic gate counts from the committed JSON -> zk_compose_<member>_gates.
  while IFS=$'\t' read -r member gates; do
    [ -n "${member:-}" ] && [ -n "${gates:-}" ] && add "zk_compose_${member}_gates" gates "$gates"
  done < <(python3 -c "import json,sys
d=json.load(open(sys.argv[1])).get('benchmarks',{})
for k in sorted(d):
    print('%s\t%s' % (k, d[k]['circuit_size']))" "$ZK_GATES_JSON")
fi

ZK_HARVEST=bench/zk/criterion_harvest.py
ZK_REF="${GITHUB_REF:-}"
if [ -n "$ZK_REF" ] && [ "$ZK_REF" != "refs/heads/main" ]; then
  echo "note: zk criterion suites skipped (PR tier — criterion build/run is main-only, like the fts/vector hooks)" >&2
elif command -v cargo >/dev/null 2>&1 && [ -f "$ZK_HARVEST" ]; then
  # sparq-zk commitment-pipeline throughput (bench/zk; standalone cargo project).
  if (cd bench/zk && cargo bench -q -- --warm-up-time 0.5 --measurement-time 1 --sample-size 10) >/dev/null 2>"$TMP/zk.err"; then
    while IFS=$'\t' read -r name unit value; do
      [ -n "${name:-}" ] && [ -n "${value:-}" ] && add "$name" "${unit:-us}" "$value"
    done < <(python3 "$ZK_HARVEST" bench/zk/target/criterion zk)
  else
    echo "ERROR: zk commit bench failed" >&2; cat "$TMP/zk.err" >&2 || true; exit 1
  fi
  # zk-trace per-operator capture overhead (bench/zk-trace; standalone cargo project,
  # sparq-engine built WITH the non-default `zk` feature).
  if (cd bench/zk-trace && cargo bench -q -- --warm-up-time 0.5 --measurement-time 1 --sample-size 10) >/dev/null 2>"$TMP/zkt.err"; then
    while IFS=$'\t' read -r name unit value; do
      [ -n "${name:-}" ] && [ -n "${value:-}" ] && add "$name" "${unit:-us}" "$value"
    done < <(python3 "$ZK_HARVEST" bench/zk-trace/target/criterion zk)
  else
    echo "ERROR: zk-trace bench failed" >&2; cat "$TMP/zkt.err" >&2 || true; exit 1
  fi
else
  echo "note: zk criterion suites skipped (cargo not on PATH or $ZK_HARVEST missing)" >&2
fi

# Solid WAC/ACP per-commit hook (sq-k0km). Crate-example runner (sparq-solid's
# `bench`), cargo-only — so, exactly like the RSP/HDT/ZK-criterion hooks, PINNED to the
# MAIN/local tier (run on push-to-main or a LOCAL run where GITHUB_REF is unset; SKIPPED on
# the PR tier) to keep the example build off per-PR CI. run.sh asserts every DETERMINISTIC
# count vs expected.tsv internally (exit 1 on drift), failing the whole ci-bench run on a
# regression exactly like RSP/HDT. We harvest its `<metric>\t<value>\tcount` stdout; every row
# is a DETERMINISTIC structural count emitted for the dashboard's Solid family card (correctness
# is the run.sh internal gate, so these are trend rows, NOT perf-gate ratchets).
SOLID_BIN=target/release/examples/bench
SOLID_REF="${GITHUB_REF:-}"
if [ -n "$SOLID_REF" ] && [ "$SOLID_REF" != "refs/heads/main" ]; then
  echo "note: solid skipped (PR tier — sparq-solid example build/run is main-only, like the rsp/hdt hooks)" >&2
elif command -v cargo >/dev/null 2>&1 && [ -x "$SOLID_RUN" ]; then
  # Always (re)build: rust-cache restores target/, AND the example target name `bench` is shared
  # by several crates, so build sparq-solid's IMMEDIATELY before invoking run.sh to be sure the
  # binary on disk is this crate's.
  if ! cargo build --release -q -p sparq-solid --example bench; then
    echo "ERROR: sparq-solid bench example failed to build" >&2
    exit 1
  fi
  if SOLID_BIN="$SOLID_BIN" bash "$SOLID_RUN" > "$TMP/solid.tsv" 2>"$TMP/solid.err"; then
    while IFS=$'\t' read -r metric value unit; do
      [ -n "${metric:-}" ] && [ -n "${value:-}" ] || continue
      add "$metric" "${unit:-count}" "$value"
    done < "$TMP/solid.tsv"
  else
    echo "ERROR: solid run.sh failed (auth-view count regression)" >&2
    cat "$TMP/solid.err" >&2 || true
    exit 1
  fi
else
  echo "note: solid skipped (cargo not on PATH or $SOLID_RUN missing)" >&2
fi

# sparq-nlq OFFLINE NL->SPARQL per-commit hook (sq-k0km). Crate-example runner
# (sparq-nlq's `bench`), cargo-only + FULLY OFFLINE — so, like the solid/rsp/hdt hooks, PINNED
# to the MAIN/local tier (SKIPPED on the PR tier). run.sh pins N and asserts every DETERMINISTIC
# count vs expected.tsv internally (exit 1 on drift), failing the whole ci-bench run on a
# regression. We harvest its `<metric>\t<value>\t<unit>` stdout for the dashboard's GenAI family
# card (correctness is the run.sh internal gate, so these are trend rows). The sibling GenAI
# suites (sim/introspect) need the gitignored olympics.nt (NOT in CI) and the GPU suite needs a
# GPU backend (NOT on gh runners): both are EC2/dataset-gathered and intentionally NOT emitted.
NLQ_BIN=target/release/examples/bench
NLQ_REF="${GITHUB_REF:-}"
if [ -n "$NLQ_REF" ] && [ "$NLQ_REF" != "refs/heads/main" ]; then
  echo "note: nlq skipped (PR tier — sparq-nlq example build/run is main-only, like the solid/rsp/hdt hooks)" >&2
elif command -v cargo >/dev/null 2>&1 && [ -x "$NLQ_RUN" ]; then
  # Always (re)build IMMEDIATELY before run.sh: the example target name `bench` is shared by
  # several crates (e.g. sparq-solid's, built just above), so this build resets the on-disk
  # binary to sparq-nlq's before the runner invokes it.
  if ! cargo build --release -q -p sparq-nlq --example bench; then
    echo "ERROR: sparq-nlq bench example failed to build" >&2
    exit 1
  fi
  if NLQ_BIN="$NLQ_BIN" bash "$NLQ_RUN" > "$TMP/nlq.tsv" 2>"$TMP/nlq.err"; then
    while IFS=$'\t' read -r metric value unit; do
      [ -n "${metric:-}" ] && [ -n "${value:-}" ] || continue
      add "$metric" "${unit:-count}" "$value"
    done < "$TMP/nlq.tsv"
  else
    echo "ERROR: nlq run.sh failed (offline-loop count regression)" >&2
    cat "$TMP/nlq.err" >&2 || true
    exit 1
  fi
else
  echo "note: nlq skipped (cargo not on PATH or $NLQ_RUN missing)" >&2
fi

# RDFS inference (seconds) — instance-heavy: SCALE individuals under a depth-20 class hierarchy.
{ echo '@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .'; echo '@prefix : <http://ex/> .'
  for k in $(seq 0 19); do echo ":c$k rdfs:subClassOf :c$((k+1)) ."; done
  for j in $(seq 1 "$SCALE"); do echo ":x$j a :c0 ."; done; } > "$TMP/inf.ttl"
infs=$("$CLI" reason "$TMP/inf.ttl" turtle rdfs 2>&1 | grep -oE 'in [0-9.]+s' | grep -oE '[0-9.]+' | head -1)
[ -n "${infs:-}" ] && add rdfs_infer_s s "$infs"

# WASM bundle size (bytes, deterministic) — enforces the "zero wasm bundle impact" rule for
# opt-in features: any feature that leaks into the browser bundle shows up here per commit.
# sq-7d3dj.1: built with the `release-wasm` profile (NOT plain `release`) so the
# GATE BUILDS UNDER THE SAME CARGO PROFILE AS THE SHIPPED BUNDLE — js `build:wasm{,:lean}`
# builds the same profile via `wasm-pack --profile release-wasm` (size-optimised cold parse
# crates + strip=symbols; hot engine crates stay opt-level 3). This ratchets the raw `cargo
# build` `.wasm`, not the post-wasm-bindgen/wasm-opt npm artifact, so it tracks the profile
# the shipped bundle uses. Skipped gracefully when the wasm target isn't installed.
WASM_BIN=""
if rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown; then
  if cargo build --profile release-wasm -q -p sparq-wasm --target wasm32-unknown-unknown 2>/dev/null; then
    WASM_BIN=$(ls target/wasm32-unknown-unknown/release-wasm/*.wasm 2>/dev/null | head -1)
    if [ -n "${WASM_BIN:-}" ]; then
      add wasm_bundle_bytes bytes "$(wc -c < "$WASM_BIN" | tr -d ' ')"
    fi
  fi
fi

# (sq-7d3dj.14) wasm_opt_bundle_bytes — TREND-ONLY shipped size after wasm-opt -Oz.
# The raw wasm_bundle_bytes above is the HARD-GATED deterministic ratchet (scripts/perf-gate.py,
# 2% band). This companion series emits the post-wasm-opt -Oz size (the "shipped" artifact after
# the same wasm-bindgen/wasm-opt pass that `wasm-pack build` runs for the published npm bundle).
# The ~10% gap between the two is name-section and producer metadata that browsers strip at load
# time — real bundle-size wins from the optimisation program show up here while the raw gate stays
# bit-identical. TREND-ONLY: scripts/perf-gate.py is intentionally UNTOUCHED. The wasm-opt version
# is logged to stderr so per-run comparisons can account for tool-version changes. Skipped gracefully
# when wasm-opt (binaryen) is absent or the wasm binary was not built above.
if command -v wasm-opt >/dev/null 2>&1 && [ -n "${WASM_BIN:-}" ] && [ -f "${WASM_BIN:-}" ]; then
  WASM_OPT_VER="$(wasm-opt --version 2>&1 | head -1 || echo 'unknown')"
  echo "note: wasm-opt version for wasm_opt_bundle_bytes: $WASM_OPT_VER" >&2
  WASM_OPT_OUT="$TMP/opt.wasm"
  if wasm-opt -Oz --strip-debug --strip-producers "$WASM_BIN" -o "$WASM_OPT_OUT" 2>/dev/null; then
    add wasm_opt_bundle_bytes bytes "$(wc -c < "$WASM_OPT_OUT" | tr -d ' ')"
  fi
fi

# (sq-3ul2n.2) assert the gate build carries +simd128 — LAST, after wasm_bundle_bytes and
# wasm_opt have read the stripped artifact ($WASM_BIN); this clobbers it with an unstripped probe.
assert_wasm_simd128

emit_json > "$OUT"
echo "wrote $OUT:"; cat "$OUT"
