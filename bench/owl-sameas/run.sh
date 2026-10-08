#!/usr/bin/env bash
# [OPUS-4.8] OWL sameAs equality micro-suite per-commit runner (sq-msl6, design A3 + A5) — a
# self-contained, self-asserting entry point CI calls (mirrors bench/deep-taxonomy/run.sh and
# bench/lubm/run.sh). For EACH requested TIER (per-class size N) it:
#   1. ensures the corpus exists (gen.sh -> gen_sameas.py): K owl:sameAs classes of N members + M
#      anchor data triples (K=4, M=3 fixed in gen.sh).
#   2. materialises the OWL-RL closure with `sparq-cli reason <f> ntriples owl <out.nt>` (sparq-reason
#      owl:sameAs union-find rewriting + expand-back) and records the closure time + closure triple-count.
#   3. runs query.rq over the closure (count mode, $ITERS iters) and records the query latency.
#   4. ASSERTS the closure triple-count AND the query row-count against expected.tsv (exit 1 on any
#      mismatch — a sparq-reason owl:sameAs correctness regression fails the run LOUDLY).
#
# The self-asserting closure-size gate is the POINT of the suite (design A5: closure_triples is the
# single most valuable reasoner gate — it catches silent UNDER-DERIVATION the query alone misses).
# A correct membership query (K*N rows) does NOT prove the full N^2 sameAs relation was materialised;
# the closure_triples = K*N*(N+M) assertion does. The closed form is exact + verified — see
# expected.tsv + README.md.
#
# Usage: bench/owl-sameas/run.sh   (run from anywhere; honours these env knobs)
#   SAMEAS_TIERS="8 32"   space-separated per-class-size tiers (default: the per-commit pair)
#   CLI=<path>            sparq-cli binary (default: target/release/sparq-cli)
#   ITERS=<n>             query bench iterations (default 3)
# stdout: one TSV line per metric:  <metric_name>\t<value>\t<unit>  — harvested by the
#         scripts/ci-bench.sh sameas hook into github-action-benchmark JSON. Metric names:
#   sameas_size<TIER>_closure_s        OWL-RL closure wall seconds (engine-internal "in Xs")
#   sameas_size<TIER>_query_us         query.rq min-of-ITERS latency (µs), count mode
#   sameas_size<TIER>_closure_triples  materialised closure triple-count (DETERMINISTIC structural)
# The `size<TIER>` token is the dashboard's existing `_size<digits>` size axis (sizeAxisOf), so the
# scaling chart works with NO dashboard.js change; the `sameas` prefix routes it to the featured
# "OWL sameAs" suite (featuredSuiteOf). See README.md.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
CLI="${CLI:-$ROOT/target/release/sparq-cli}"
ITERS="${ITERS:-3}"
GEN="$ROOT/bench/owl-sameas/gen.sh"
QUERY="$ROOT/bench/owl-sameas/query.rq"
EXP="$ROOT/bench/owl-sameas/expected.tsv"
CACHE="${SAMEAS_CACHE:-/tmp/owl-sameas}"
# Default per-commit tiers: N=8 + N=32 (cheap — closures 352 / 4,480 triples, sub-second total).
# N=256 (closure 265,216) is available via SAMEAS_TIERS for the EC2/nightly tier.
TIERS="${SAMEAS_TIERS:-8 32}"

if [ ! -x "$CLI" ]; then
  echo "[sameas] ERROR: sparq-cli not found at $CLI (build: cargo build --release -p sparq-cli)" >&2
  exit 1
fi

# query.rq lives in a file but `sparq-cli bench` takes a queries DIRECTORY; stage a dir with just
# the probe query so the bench runner's TSV reports it under a stable `q` name.
# CACHE persists across runs (default /tmp/owl-sameas), so CLEAR any stale `.rq` first: `bench` runs
# EVERY *.rq in the dir, and a leftover probe (e.g. a renamed query) would emit an extra TSV line
# that could sort ahead of q_membership and false-fail the row-count gate. [OPUS-4.8] sq-f4ps
QDIR="$CACHE/qdir"
rm -rf "$QDIR"
mkdir -p "$QDIR"
cp "$QUERY" "$QDIR/q_membership.rq"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail=0
expected_for() {  # $1 = tier, $2 = column index (2=closure_triples, 3=query_rows)
  awk -F'\t' -v t="$1" -v c="$2" '$1==t{print $c}' "$EXP"
}

for TIER in $TIERS; do
  echo "[sameas] === tier N=$TIER ===" >&2
  # ---- 1. ensure corpus -----------------------------------------------------------------
  CORPUS="$("$GEN" "$TIER")"
  CLOSURE="$CACHE/sameas-${TIER}-closure.nt"

  # ---- 2. materialise the OWL-RL closure; capture engine time + triple-count ------------
  # `reason <f> ntriples owl <out.nt>`: because an output file is given, stderr carries both
  # `<N> triples after owl reasoning` (the deduplicated closure size, #6466) and the
  # engine-internal `... in <X>s` timer (robust to machine load). We read both from stderr.
  "$CLI" reason "$CORPUS" ntriples owl "$CLOSURE" >"$TMP/r.out" 2>"$TMP/r.err"
  triples="$(grep -oE '[0-9]+ triples after owl reasoning' "$TMP/r.err" | grep -oE '^[0-9]+' | head -1)"
  closure_s="$(grep -oE 'in [0-9.]+s' "$TMP/r.err" | head -1 | grep -oE '[0-9.]+' | head -1)"

  # ---- 3. run query.rq over the closure (count mode, min-of-ITERS) ----------------------
  # `bench` prints one `<name>\t<rows>\t<us>` line per *.rq (sorted by name). Pick the
  # q_membership line BY NAME rather than reading line 1 — robust to any extra/stale line that
  # would otherwise sort ahead of it and false-fail the gate. [OPUS-4.8] sq-f4ps
  "$CLI" bench "$CLOSURE" ntriples "$QDIR" "$ITERS" count 2>/dev/null > "$TMP/q.tsv" || true
  qrows=""; qus=""
  IFS=$'\t' read -r qrows qus < <(awk -F'\t' '$1=="q_membership"{print $2"\t"$3; exit}' "$TMP/q.tsv")

  # ---- emit metric TSV lines (harvested by the ci-bench sameas hook) --------------------
  [ -n "${closure_s:-}" ] && printf 'sameas_size%s_closure_s\t%s\ts\n'            "$TIER" "$closure_s"
  [ -n "${qus:-}" ]       && printf 'sameas_size%s_query_us\t%s\tus\n'            "$TIER" "$qus"
  [ -n "${triples:-}" ]   && printf 'sameas_size%s_closure_triples\t%s\ttriples\n' "$TIER" "$triples"

  # ---- 4. SELF-ASSERTING closure-size + query-row gate (deterministic) ------------------
  exp_tri="$(expected_for "$TIER" 2)"
  exp_rows="$(expected_for "$TIER" 3)"
  if [ -z "$exp_tri" ] || [ -z "$exp_rows" ]; then
    echo "[sameas] ERROR: tier N=$TIER has no expected.tsv entry (add closure_triples + query_rows)" >&2
    fail=1; continue
  fi
  if [ "${triples:-MISSING}" != "$exp_tri" ]; then
    echo "[sameas] ERROR: tier N=$TIER closure=${triples:-MISSING} expected=$exp_tri (owl:sameAs regression)" >&2
    fail=1
  fi
  if [ "${qrows:-MISSING}" != "$exp_rows" ]; then
    echo "[sameas] ERROR: tier N=$TIER query rows=${qrows:-MISSING} expected=$exp_rows (owl:sameAs regression)" >&2
    fail=1
  fi
done

if [ "$fail" = 0 ]; then
  echo "[sameas] OK: all closure sizes + query rows match expected.tsv ($TIERS)" >&2
else
  echo "[sameas] FAILED: closure size or query rows diverged from expected.tsv" >&2
fi
exit "$fail"
