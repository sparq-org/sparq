#!/usr/bin/env bash
# [OPUS-4.8] Deep Taxonomy per-commit runner (sq-1hgz; parent sq-i0nm "design D") — the
# self-contained, self-asserting entry point CI calls (mirrors bench/lubm/run.sh). For EACH
# requested DEPTH tier it:
#   1. ensures the DT N3 corpus exists (gen.sh, which reuses bench/inference/gen_deeptaxonomy.py).
#   2. materializes the N3 forward closure with `sparq-cli reason <f> n3 n3 <out.nt>` (sparq-reason)
#      and records the engine-internal closure time + the materialized closure triple-count.
#   3. runs query.rq over the closure (count mode, $ITERS iters) and records the query latency.
#   4. ASSERTS the closure triple-count AND the query row-count against expected.tsv (exit 1 on
#      any mismatch — a sparq-reason / engine correctness regression fails the run LOUDLY).
#
# The self-asserting closure-size gate is the point of the suite: DeepTaxonomy is the canonical
# rule-heavy inference workload where a reasoner regression (missing/extra entailments, a broken
# fixpoint) changes the closure size, and that is caught deterministically here.
#
# Usage: bench/deep-taxonomy/run.sh   (run from anywhere; honours these env knobs)
#   DEEPTAX_DEPTHS="1000 10000"   space-separated depth tiers (default: the per-commit pair)
#   CLI=<path>                    sparq-cli binary (default: target/release/sparq-cli)
#   ITERS=<n>                     query bench iterations (default 3)
# stdout: one TSV line per metric:  <metric_name>\t<value>\t<unit>  — harvested by the
#         scripts/ci-bench.sh deeptax hook into github-action-benchmark JSON. Metric names:
#   deeptax_d<DEPTH>_closure_s        N3 forward-closure wall seconds (engine-internal "in Xs")
#   deeptax_d<DEPTH>_query_us         query.rq min-of-ITERS latency (µs), count mode
#   deeptax_d<DEPTH>_closure_triples  materialized closure triple-count (DETERMINISTIC structural)
# The `d<DEPTH>` token is numeric so the dashboard derives the DEPTH axis for its scaling chart
# (bench/dashboard/dashboard.js sizeAxisOf `_d<digits>`); the `deeptax` prefix routes it to the
# featured "Deep Taxonomy" suite (featuredSuiteOf). See README.md.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
CLI="${CLI:-$ROOT/target/release/sparq-cli}"
ITERS="${ITERS:-3}"
GEN="$ROOT/bench/deep-taxonomy/gen.sh"
QUERY="$ROOT/bench/deep-taxonomy/query.rq"
EXP="$ROOT/bench/deep-taxonomy/expected.tsv"
CACHE="${DEEPTAX_CACHE:-/tmp/deep-taxonomy}"
# Default per-commit tiers: dt1k + dt10k (cheap — closures 2,001 / 20,001 triples, ~0.1 s total).
# dt100k (depth 100000) is available via DEEPTAX_DEPTHS for the EC2/nightly tier.
DEPTHS="${DEEPTAX_DEPTHS:-1000 10000}"

if [ ! -x "$CLI" ]; then
  echo "[deeptax] ERROR: sparq-cli not found at $CLI (build: cargo build --release -p sparq-cli)" >&2
  exit 1
fi

# query.rq lives in a file but `sparq-cli bench` takes a queries DIRECTORY; stage a dir with just
# the probe query so the bench runner's TSV reports it under a stable `q` name.
QDIR="$CACHE/qdir"
mkdir -p "$QDIR"
cp "$QUERY" "$QDIR/q_membership.rq"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail=0
expected_for() {  # $1 = depth, $2 = column index (2=closure_triples, 3=query_rows)
  awk -F'\t' -v d="$1" -v c="$2" '$1==d{print $c}' "$EXP"
}

for DEPTH in $DEPTHS; do
  echo "[deeptax] === depth $DEPTH ===" >&2
  # ---- 1. ensure corpus -----------------------------------------------------------------
  CORPUS="$("$GEN" "$DEPTH")"
  CLOSURE="$CACHE/dt-${DEPTH}-closure.nt"

  # ---- 2. materialize the N3 forward closure; capture engine time + triple-count --------
  # `reason <f> n3 n3 <out.nt>`: stderr carries `reasoned [N3]: <N> ground triples in closure in
  # <X>s` (engine-internal timer, robust to machine load) and, because an output file is given,
  # `<N> triples after n3 reasoning` (#6466). We read both from stderr.
  "$CLI" reason "$CORPUS" n3 n3 "$CLOSURE" >"$TMP/r.out" 2>"$TMP/r.err"
  triples="$(grep -oE '[0-9]+ triples after n3 reasoning' "$TMP/r.err" | grep -oE '^[0-9]+' | head -1)"
  closure_s="$(grep -oE 'in [0-9.]+s' "$TMP/r.err" | head -1 | grep -oE '[0-9.]+' | head -1)"

  # ---- 3. run query.rq over the closure (count mode, min-of-ITERS) ----------------------
  "$CLI" bench "$CLOSURE" ntriples "$QDIR" "$ITERS" count 2>/dev/null > "$TMP/q.tsv" || true
  IFS=$'\t' read -r _qname qrows qus < "$TMP/q.tsv"

  # ---- emit metric TSV lines (harvested by the ci-bench deeptax hook) -------------------
  [ -n "${closure_s:-}" ] && printf 'deeptax_d%s_closure_s\t%s\ts\n'        "$DEPTH" "$closure_s"
  [ -n "${qus:-}" ]       && printf 'deeptax_d%s_query_us\t%s\tus\n'         "$DEPTH" "$qus"
  [ -n "${triples:-}" ]   && printf 'deeptax_d%s_closure_triples\t%s\ttriples\n' "$DEPTH" "$triples"

  # ---- 4. SELF-ASSERTING closure-size + query-row gate (deterministic) ------------------
  exp_tri="$(expected_for "$DEPTH" 2)"
  exp_rows="$(expected_for "$DEPTH" 3)"
  if [ -z "$exp_tri" ] || [ -z "$exp_rows" ]; then
    echo "[deeptax] ERROR: depth $DEPTH has no expected.tsv entry (add closure_triples + query_rows)" >&2
    fail=1; continue
  fi
  if [ "${triples:-MISSING}" != "$exp_tri" ]; then
    echo "[deeptax] ERROR: depth $DEPTH closure=${triples:-MISSING} expected=$exp_tri (reasoner regression)" >&2
    fail=1
  fi
  if [ "${qrows:-MISSING}" != "$exp_rows" ]; then
    echo "[deeptax] ERROR: depth $DEPTH query rows=${qrows:-MISSING} expected=$exp_rows (reasoner regression)" >&2
    fail=1
  fi
done

if [ "$fail" = 0 ]; then
  echo "[deeptax] OK: all closure sizes + query rows match expected.tsv ($DEPTHS)" >&2
else
  echo "[deeptax] FAILED: closure size or query rows diverged from expected.tsv" >&2
fi
exit "$fail"
