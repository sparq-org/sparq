#!/usr/bin/env bash
# [SONNET-4.6] sq-hmd7l.13 — LDBC Graphalytics validation-gated comparison entry point.
#
# THE PATTERN: validate, THEN time. For every (engine, algorithm) pair the harness first
# gates the engine's output against a Graphalytics reference output; a timing is only
# printed once that gate is green. A red gate is reported as red — never dropped, never
# softened into a footnote next to a number.
#
# FOUR HONEST OUTCOMES per algorithm, and no fifth:
#   OK           — the engine implements the Graphalytics semantics and its output validated.
#   GAP          — sparq-algos does not implement the algorithm. Printed as a FEATURE GAP row
#                  with no timing. Never faked, never silently skipped.
#   MISMATCH     — the engine has a related algorithm whose SEMANTICS differ from the spec
#                  (sparq's CDLP; igraph's randomised label propagation). It was gated
#                  against the spec reference and the divergence is printed. It does not
#                  fail the run, because it is a known, documented semantic gap rather than a
#                  regression — see research/gap-graphalytics-2026-07.md.
#   SEMANTIC-GAP — the engine's answer is not even ADDRESSED by the reference we hold, so it
#                  is not gated at all: igraph's PageRank solves for the stationary
#                  distribution and the reference is a fixed ten-sweep vector. No converged
#                  oracle is generated here, so the row is neither validated nor timed.
#
# NO TIMING WITHOUT A GREEN GATE. Every runner's output is CAPTURED, not streamed; its
# `metric_us` lines reach the transcript only after that engine's gate comes back green. On
# any other path the log is still shown for diagnosis, with the timing lines stripped out.
#
# THE ORACLE IS INDEPENDENT. The committed reference outputs under data/*/ are produced by
# `gx.py reference`, a from-the-spec Python implementation — a different language and a
# different author-path from the Rust under test. Every run re-derives them and diffs, so
# a committed reference cannot silently drift into "whatever sparq printed" (which would
# validate nothing). Set GX_SKIP_ORACLE_CROSSCHECK=1 to bypass on a dataset that ships its
# own authoritative reference and where our oracle is the thing under suspicion.
#
# USAGE
#   bash bench/graphalytics/run.sh --smoke   # acceptance: sparq only, committed fixture,
#                                            # exit 0 = the PageRank + WCC gates are green
#   bash bench/graphalytics/run.sh           # full panel; competitor columns if installed
#
# ENV KNOBS (all optional)
#   GX_DATA_DIR     directory holding dataset directories  (default bench/graphalytics/data)
#   GX_DATASET      dataset directory name                 (default smoke-directed)
#   GX_ALGOS        comma-separated algorithm subset       (default: all six)
#   GX_ITERS        timed repetitions per engine           (default 3)
#   GX_ALGOS_BIN    prebuilt graphalytics example binary   (default: cargo build --release)
#   CARGO           cargo command                          (default cargo)
#   GX_SKIP_ORACLE_CROSSCHECK=1   skip the reference re-derivation diff
#
# A REAL LDBC RUN. The committed fixture is a 10-vertex graph in the Graphalytics FORMAT,
# not an LDBC-distributed dataset — it exists so the format parser and the validation gate
# are exercised without a multi-gigabyte download. For a real run, fetch a dataset from the
# LDBC Graphalytics data repository, unpack it so that <dir>/<name>.properties sits beside
# its .v/.e and <name>-PR/<name>-WCC reference files, and point the harness at it:
#   GX_DATA_DIR=/data/graphalytics GX_DATASET=example-directed bash bench/graphalytics/run.sh
#
# Wall-clock numbers from a shared or CI box are NOT canonical (bench/CATALOG.md).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

SMOKE=0
for arg in "$@"; do
  case "$arg" in
    --smoke) SMOKE=1 ;;
    *) echo "[gx] unknown arg: $arg (usage: bench/graphalytics/run.sh [--smoke])" >&2; exit 2 ;;
  esac
done

GX_DIR="$ROOT/bench/graphalytics"
GX_PY="$GX_DIR/gx.py"
ADAPTER="$ROOT/scripts/bench-adapters/graph_analytics_adapter.py"
DATA_DIR="${GX_DATA_DIR:-$GX_DIR/data}"
DATASET="${GX_DATASET:-smoke-directed}"
DATASET_DIR="$DATA_DIR/$DATASET"
ITERS="${GX_ITERS:-3}"
CARGO="${CARGO:-cargo}"

log()  { printf '[gx] %s\n' "$*" >&2; }
die()  { printf '[gx] ERROR: %s\n' "$*" >&2; exit 1; }

command -v python3 >/dev/null 2>&1 || die "python3 is required (stdlib only)"
[ -d "$DATASET_DIR" ] || die "dataset directory not found: $DATASET_DIR"

# The dataset name is the stem of its .properties file, which need not equal the directory
# name (an unpacked LDBC archive routinely differs).
NAME="$(python3 "$GX_PY" info --dataset "$DATASET_DIR" | sed -n 's/^name=//p')"
[ -n "$NAME" ] || die "could not determine the dataset name in $DATASET_DIR"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ---- 0. the sparq leg's binary ----------------------------------------------------------
ALGOS_BIN="${GX_ALGOS_BIN:-$ROOT/target/release/examples/graphalytics}"
if [ ! -x "$ALGOS_BIN" ]; then
  log "building the sparq-algos Graphalytics runner ..."
  "$CARGO" build -p sparq-algos --release --example graphalytics >&2
  ALGOS_BIN="$ROOT/target/release/examples/graphalytics"
fi
[ -x "$ALGOS_BIN" ] || die "graphalytics example binary not found at $ALGOS_BIN"

# ---- the algorithm manifest -------------------------------------------------------------
# The six Graphalytics algorithms and what sparq-algos actually offers for each. This table
# is the single place the honest status is declared; the harness derives every row from it,
# so an algorithm cannot fall out of the report by being forgotten.
#   conformant    — sparq implements the Graphalytics semantics; a red gate FAILS the run.
#   nonconformant — sparq has a related algorithm with different semantics; the gate result
#                   is reported, and a red gate does NOT fail the run.
#   missing       — not implemented; a FEATURE GAP row, no timing, no fabricated value.
ALL_ALGOS="pr wcc cdlp bfs lcc sssp"
algo_status() {
  case "$1" in
    pr|wcc)          echo conformant ;;
    cdlp)            echo nonconformant ;;
    bfs|lcc|sssp)    echo missing ;;
    *)               echo unknown ;;
  esac
}
algo_note() {
  case "$1" in
    pr)   echo "pagerank() with tolerance=0 runs exactly pr.num-iterations sweeps" ;;
    wcc)  echo "weakly_connected_components(), compared by partition equivalence" ;;
    cdlp) echo "label_propagation() is ASYNCHRONOUS + run-to-fixed-point; CDLP is SYNCHRONOUS + fixed-count" ;;
    bfs)  echo "no traversal/hop-distance algorithm in sparq-algos" ;;
    lcc)  echo "no local clustering coefficient in sparq-algos" ;;
    sssp) echo "no weighted shortest paths; NodeGraph is an unweighted view" ;;
  esac
}

if [ -n "${GX_ALGOS:-}" ]; then
  ALGOS="$(printf '%s' "$GX_ALGOS" | tr ',' ' ')"
else
  ALGOS="$ALL_ALGOS"
fi

echo ""
echo "================================================================================"
echo " LDBC Graphalytics validation-gated comparison — sparq-algos vs igraph / NetworKit"
echo " [SONNET-4.6] sq-hmd7l.13   dataset=$NAME   dir=$DATASET_DIR   iters=$ITERS"
echo "================================================================================"
python3 "$GX_PY" info --dataset "$DATASET_DIR" | sed 's/^/  /'
echo ""

# engine \t algo \t verdict \t detail. Deliberately a FILE, not shell variables: the
# reference resolution below runs inside a command substitution (a subshell), so a verdict
# held in a variable would be silently discarded on the way out — which is exactly how a
# gate becomes vacuous. The final exit status is derived from this file, once, at the end.
RESULTS="$TMP/results.tsv"
: > "$RESULTS"

record() { printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" >> "$RESULTS"; }

# ---- 0b. prove the gate is NON-VACUOUS before trusting anything it says -------------------
# A validation gate that cannot go red is decoration. Every run therefore first feeds the
# validator a deliberately WRONG answer of each kind and asserts it rejects it. If any of
# these "failures" passes, the harness aborts rather than reporting green results gated by
# a broken gate.
gate_self_test() {
  local d="$TMP/selftest"
  mkdir -p "$d"
  printf '1 1.000000000000e-01\n2 9.000000000000e-01\n' > "$d/ref-pr"
  # (a) a float outside the relative epsilon must be rejected ...
  printf '1 2.000000000000e-01\n2 9.000000000000e-01\n' > "$d/bad-pr"
  # (b) ... while one inside it must still be accepted (a gate that rejects everything is
  #     equally useless, and would make every green row above meaningless).
  printf '1 1.000000100000e-01\n2 9.000000000000e-01\n' > "$d/ok-pr"
  # (c) a missing vertex must be rejected.
  printf '1 1.000000000000e-01\n' > "$d/short-pr"
  # (c2) a NON-FINITE value must be rejected. NaN is the important one: every comparison
  #      against it is false, so a tolerance check alone silently ACCEPTS it, and a diverged
  #      power method is exactly what produces one. The infinities are pinned here too so
  #      they are rejected by rule rather than by arithmetic accident.
  printf '1 nan\n2 9.000000000000e-01\n' > "$d/nan-pr"
  printf '1 inf\n2 9.000000000000e-01\n' > "$d/posinf-pr"
  printf '1 -inf\n2 9.000000000000e-01\n' > "$d/neginf-pr"
  printf '1 1\n2 1\n3 3\n' > "$d/ref-wcc"
  # (d) merging two reference groups must be rejected ...
  printf '1 7\n2 7\n3 7\n' > "$d/merged-wcc"
  # (e) ... as must splitting one ...
  printf '1 7\n2 8\n3 9\n' > "$d/split-wcc"
  # (f) ... but a pure RELABELLING of the same partition must pass, since partition
  #     equivalence — not label equality — is the Graphalytics rule.
  printf '1 42\n2 42\n3 99\n' > "$d/relabelled-wcc"

  local case_name expect algo file
  # "<file> <algo> <expected-exit>"
  while read -r file algo expect; do
    [ -n "$file" ] || continue
    case_name="$file"
    local ref="$d/ref-${algo}"
    set +e
    python3 "$GX_PY" validate --reference "$ref" --actual "$d/$file" --algo "$algo" >/dev/null 2>&1
    local got=$?
    set -e
    if [ "$got" != "$expect" ]; then
      die "GATE SELF-TEST FAILED: validating '$case_name' ($algo) exited $got, expected \
$expect. The validation gate is not trustworthy, so no result below would mean anything."
    fi
  done <<'CASES'
bad-pr pr 1
ok-pr pr 0
short-pr pr 1
nan-pr pr 1
posinf-pr pr 1
neginf-pr pr 1
merged-wcc wcc 1
split-wcc wcc 1
relabelled-wcc wcc 0
CASES
  log "gate self-test: the validator rejects wrong answers (including non-finite ones) and \
accepts equivalent ones"
}
gate_self_test

# ---- 1. resolve (and cross-check) the reference output for one algorithm -----------------
# Echoes the reference path on stdout. Exit: 0 = usable reference, 3 = the cross-check
# failed (already recorded), 1 = no reference obtainable.
resolve_reference() {
  local algo="$1" upper ref
  upper="$(printf '%s' "$algo" | tr '[:lower:]' '[:upper:]')"
  ref="$DATASET_DIR/$NAME-$upper"
  if [ -f "$ref" ]; then
    if [ "${GX_SKIP_ORACLE_CROSSCHECK:-0}" != "1" ]; then
      # Re-derive from the spec and diff. A committed reference that no longer matches an
      # independent implementation of the specification is a finding, not a nuisance.
      if python3 "$GX_PY" reference --dataset "$DATASET_DIR" --algo "$algo" \
           --out "$TMP/oracle-$algo" >/dev/null 2>&1; then
        if ! python3 "$GX_PY" validate --reference "$ref" --actual "$TMP/oracle-$algo" \
               --algo "$algo" >"$TMP/xcheck-$algo" 2>&1; then
          log "ORACLE CROSS-CHECK FAILED for $upper — the committed reference and the"
          log "  from-the-spec oracle DISAGREE. Neither may be trusted until resolved:"
          sed 's/^/[gx]   /' "$TMP/xcheck-$algo" >&2
          record oracle "$algo" XCHECK-FAILED "committed reference disagrees with the spec oracle"
          return 3
        fi
      fi
    fi
    printf '%s' "$ref"
    return 0
  fi
  # No shipped reference: derive one from the spec oracle, where the oracle supports it.
  if python3 "$GX_PY" reference --dataset "$DATASET_DIR" --algo "$algo" \
       --out "$TMP/oracle-$algo" >/dev/null 2>&1; then
    log "$upper: no committed reference in the dataset — using the from-the-spec oracle"
    printf '%s' "$TMP/oracle-$algo"
    return 0
  fi
  return 1
}

# ---- 2. gate one engine's output, then report its timing ---------------------------------
# Reproduces a captured runner log with every `metric_us` line REMOVED. Used on every path
# that has not passed the validation gate — a runner error, a semantic gap, a red gate. The
# invariant is "validate, THEN time": an engine whose output did not validate must not leave
# a number behind in the transcript, where it would be quoted without its verdict.
strip_timings() { grep -v '^metric_us ' "$1" || true; }

# $1 engine  $2 algo  $3 output file  $4 reference  $5 captured runner log  $6 status
# The log is CAPTURED, not streamed, precisely so this function can decide whether its
# timing lines are allowed to be printed at all — validation runs first, and the log is
# echoed verbatim only after a green gate.
gate_and_record() {
  local engine="$1" algo="$2" out="$3" ref="$4" log="$5" status="$6" us
  us="$(sed -n 's/^metric_us algo=//p' "$log")"
  if python3 "$GX_PY" validate --reference "$ref" --actual "$out" --algo "$algo" \
       > "$TMP/v-$engine-$algo" 2>&1; then
    sed 's/^/  '"$engine"' /' "$log"
    sed 's/^/  /' "$TMP/v-$engine-$algo"
    record "$engine" "$algo" OK "algo_us=${us:-n/a}"
  else
    strip_timings "$log" | sed 's/^/  '"$engine"' /'
    sed 's/^/  /' "$TMP/v-$engine-$algo"
    if [ "$status" = "conformant" ]; then
      log "$engine/$algo: gate RED on an algorithm declared CONFORMANT — this is a failure"
      record "$engine" "$algo" FAILED "validation gate red (timing withheld)"
    else
      record "$engine" "$algo" MISMATCH "known semantic divergence (timing withheld)"
    fi
  fi
}

for algo in $ALGOS; do
  upper="$(printf '%s' "$algo" | tr '[:lower:]' '[:upper:]')"
  status="$(algo_status "$algo")"
  echo "-- $upper ($status) — $(algo_note "$algo") --"

  if [ "$status" = "unknown" ]; then
    die "unknown algorithm '$algo' (expected one of: $ALL_ALGOS)"
  fi
  if [ "$status" = "missing" ]; then
    echo "  FEATURE GAP: sparq-algos does not implement Graphalytics $upper. No row emitted."
    record sparq "$algo" GAP "$(algo_note "$algo")"
    echo ""
    continue
  fi

  set +e
  REF="$(resolve_reference "$algo")"
  rr=$?
  set -e
  if [ "$rr" -eq 3 ]; then
    echo "  Reference cross-check failed — nothing is run or timed for $upper."
    echo ""
    continue
  fi
  if [ "$rr" -ne 0 ] || [ -z "${REF:-}" ]; then
    log "$upper: no reference output available and the oracle cannot derive one — SKIPPED"
    record sparq "$algo" NO-REFERENCE "cannot validate, so nothing is timed"
    echo ""
    continue
  fi

  # -- the sparq leg --
  OUT="$TMP/sparq-$algo"
  set +e
  "$ALGOS_BIN" --dataset "$DATASET_DIR" --name "$NAME" --algo "$algo" \
      --out "$OUT" --iters "$ITERS" > "$TMP/sparq-$algo.log" 2>"$TMP/sparq-$algo.err"
  rc=$?
  set -e
  sed 's/^/  /' "$TMP/sparq-$algo.err" >&2 || true
  if [ "$rc" -eq 3 ]; then
    echo "  FEATURE GAP (reported by the runner): $upper"
    record sparq "$algo" GAP "$(algo_note "$algo")"
    echo ""
    continue
  elif [ "$rc" -ne 0 ]; then
    log "$upper: the sparq runner exited $rc"
    strip_timings "$TMP/sparq-$algo.log" | sed 's/^/[gx]   /' >&2
    record sparq "$algo" ERROR "runner exit $rc"
    echo ""
    continue
  fi
  # The runner's log — timings included — is printed by gate_and_record, and only once the
  # gate is green.
  gate_and_record sparq "$algo" "$OUT" "$REF" "$TMP/sparq-$algo.log" "$status"

  # -- the competitor legs (gather-time only; never part of the smoke acceptance) --
  if [ "$SMOKE" = "0" ]; then
    for engine in igraph networkit; do
      COUT="$TMP/$engine-$algo"
      set +e
      python3 "$ADAPTER" --engine "$engine" --dataset "$DATASET_DIR" --name "$NAME" \
          --algo "$algo" --out "$COUT" --iters "$ITERS" > "$TMP/$engine-$algo.log" 2>&1
      crc=$?
      set -e
      if [ "$crc" -eq 4 ]; then
        echo "  $engine: SKIPPED (not installed) — no row is invented for a missing engine"
        record "$engine" "$algo" SKIPPED "engine not installed"
        continue
      elif [ "$crc" -ne 0 ]; then
        log "$engine/$algo: adapter exited $crc"
        strip_timings "$TMP/$engine-$algo.log" | sed 's/^/[gx]   /' >&2
        record "$engine" "$algo" ERROR "adapter exit $crc"
        continue
      fi
      SEM="$(sed -n 's/^semantics=//p' "$TMP/$engine-$algo.log")"
      # A PageRank whose TERMINATION SEMANTICS are not the spec's fixed sweep count is NOT
      # gated against the fixed-sweep reference. $REF is the ten-sweep vector; an engine
      # that solves for the stationary distribution instead (igraph/PRPACK) does not
      # approximate it, so running that gate would print a large per-vertex delta that reads
      # as an igraph correctness failure when it is nothing of the kind. The honest verdict
      # is that the two are incomparable on this axis. This harness generates NO converged
      # oracle, so there is no reference this row could legitimately be validated against —
      # it is therefore neither validated nor timed, exactly like a feature gap.
      if [ "$algo" = "pr" ] && [ -n "$SEM" ] && [ "$SEM" != "fixed-iterations" ]; then
        strip_timings "$TMP/$engine-$algo.log" | sed 's/^/  '"$engine"' /'
        echo "  $engine: SEMANTIC GAP — PageRank termination semantics '$SEM' vs the"
        echo "  Graphalytics fixed-sweep definition. The fixed-sweep reference is not a valid"
        echo "  oracle for it and no converged oracle exists here, so this row is neither"
        echo "  validated nor timed. See research/gap-graphalytics-2026-07.md."
        record "$engine" "$algo" SEMANTIC-GAP "termination semantics '$SEM'; not gated, no timing"
        continue
      fi
      if [ -n "$SEM" ] && [ "$SEM" != "fixed-iterations" ]; then
        log "$engine/$algo: semantics '$SEM' differ from the Graphalytics definition —"
        log "  this row is NOT semantics-matched with sparq."
      fi
      # A competitor is never declared conformant by this harness: its verdict is recorded
      # from the same gate, and a red gate marks the row MISMATCH rather than failing a run
      # whose subject is sparq.
      gate_and_record "$engine" "$algo" "$COUT" "$REF" "$TMP/$engine-$algo.log" nonconformant
    done
  fi
  echo ""
done

# ---- 3. summary --------------------------------------------------------------------------
echo "-- Summary (engine / algorithm / verdict) --"
printf '  %-10s %-6s %-14s %s\n' ENGINE ALGO VERDICT DETAIL
while IFS=$'\t' read -r engine algo verdict detail; do
  [ -n "$engine" ] || continue
  printf '  %-10s %-6s %-14s %s\n' "$engine" "$algo" "$verdict" "$detail"
done < "$RESULTS"
echo ""

GAPS="$(awk -F'\t' '$3=="GAP"' "$RESULTS" | wc -l | tr -d ' ')"
if [ "$GAPS" != "0" ]; then
  echo "  $GAPS FEATURE GAP row(s): sparq-algos is missing a Graphalytics algorithm."
  echo "  Each is a tracked P2 feature request, NOT a silent omission — see"
  echo "  research/gap-graphalytics-2026-07.md for the per-algorithm record."
  echo ""
fi

if [ "$SMOKE" = "1" ]; then
  echo "  --smoke: sparq leg only. Competitor columns require a gather run with"
  echo "  python-igraph / networkit installed on the SAME box."
  echo ""
fi
echo "  Differentiator + cost, both measured above per algorithm: sparq runs the analytics"
echo "  directly over the dict-encoded RDF store's node projection (metric 'project', no"
echo "  export), while 'export_edgelist' is what handing the same graph to an embedded"
echo "  library would cost. Report both or neither."
echo "  Timings from a shared/CI box are NOT canonical (bench/CATALOG.md)."
echo ""

# The exit status is derived from the recorded verdicts — the one place every outcome,
# including those produced inside a subshell, is durably written.
HARD="$(awk -F'\t' '$3=="FAILED" || $3=="ERROR" || $3=="XCHECK-FAILED"' "$RESULTS" | wc -l | tr -d ' ')"
if [ "$HARD" = "0" ]; then
  echo "[gx] OK: every gate on a CONFORMANT algorithm is green."
  exit 0
fi
echo "[gx] FAILED: $HARD blocking row(s) — a conformant gate is red, a runner errored," >&2
echo "[gx]   or a committed reference disagrees with the spec oracle (see above)." >&2
exit 1
