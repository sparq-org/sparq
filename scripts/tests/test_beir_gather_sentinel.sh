#!/usr/bin/env bash
# [FABLE-5] Hermetic self-tests for the SENTINEL PROTOCOL of
# scripts/bench/canonical-beir-gather-instance.sh (PR #3488 review round 1).
#
# WHY: the gather deliberately WARN-and-continues past provisioning/build/per-cut
# failures (to keep the box alive and collect partial evidence), but it must NEVER
# launder those failures into success — GATHER_DONE + gather-meta.json canonical=true
# are allowed ONLY when every requested cut has a valid-JSON envelope for BOTH
# engines (lucene-anserini oracle AND sparq-text). Anything less must write the
# DISTINCT /root/GATHER_FAILED sentinel, canonical=false + status partial|failed,
# and exit nonzero, so the launcher can machine-distinguish an honest NOT-RUN from
# a completed canonical gather. A "valid" envelope is NOT merely one that parses as
# JSON: it must be produced by THIS run (not a stale prefix-matching file from a
# prior gather), carry the exact engine + beir-ir-<cut> identity, and expose finite
# Recall@100/nDCG@10 deficits — else empty/wrong/stale JSON could certify a run
# without the claimed scored evidence (round 3). This harness pins exactly that,
# covering TOTAL failure, PARTIAL output, invalid-JSON envelopes, the success path
# (including console-recovery of the emitted envelopes via extract-console-envelopes.sh),
# a FORCED gather-meta.json write failure (provenance is a canonical-success
# prerequisite — round 2), a failed GATHER_DONE touch (the console 'gather complete'
# marker the launcher accepts as success must be impossible to emit unless the
# sentinel is already on disk — round 2), and — round 3 — empty `{}`, wrong-engine,
# wrong-cut, and stale prefix-matching envelopes all correctly rejected while
# realistic scored envelopes pass.
#
# HERMETIC: the script under test is copied into a scratch repo tree whose
# scripts/gather-competitors.sh is a scenario stub; apt-get/java/cargo/curl and the
# venv python3 are PATH-shadowed (python3 -m venv/-m pip never touch the network;
# everything else execs the real python3); SENTINEL_DIR/BEIR_VENV_DIR/BEIR_DATA_DIR
# point into the sandbox. No AWS, no network, no /root writes.
# Run: bash scripts/tests/test_beir_gather_sentinel.sh   (exit 0 = all pass)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/scripts/bench/canonical-beir-gather-instance.sh"
EXTRACT="$ROOT/scripts/bench/extract-console-envelopes.sh"
[ -f "$SCRIPT" ] || { echo "FATAL: $SCRIPT not found"; exit 2; }
REAL_PY="$(command -v python3)" || { echo "FATAL: python3 required"; exit 2; }
REAL_TOUCH="$(command -v touch)" || { echo "FATAL: touch required"; exit 2; }

pass=0; fail=0
ok()  { pass=$((pass + 1)); }
bad() { fail=$((fail + 1)); printf 'CASE FAILED: %s\n' "$1"; }

SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT
REPO="$SANDBOX/repo"; BIN="$SANDBOX/bin"; SBROOT="$SANDBOX/root"
VENVDIR="$SANDBOX/beir-venv"; CARGOHOME="$SANDBOX/cargo-home"
RESULTS="$REPO/bench/competitor-results"
mkdir -p "$REPO/scripts/bench" "$BIN" "$SANDBOX/home" "$CARGOHOME/bin"
cp "$SCRIPT" "$REPO/scripts/bench/"
# The gather sources the durable-egress library from its own directory (sq-ffaa9); it is
# inert here (no BENCH_RESULTS_S3_URI), but it must be present for the source to succeed.
cp "$ROOT/scripts/bench/bench-result-egress.sh" "$REPO/scripts/bench/"

# ---- scenario stub for scripts/gather-competitors.sh (behavior via STUB_GC_MODE) ----
cat > "$REPO/scripts/gather-competitors.sh" <<'STUB'
#!/usr/bin/env bash
out="bench/competitor-results"; mkdir -p "$out"; stamp="20260101T000000Z"
lf="$out/lucene-anserini-beir-ir-${BEIR_CUT}-${stamp}.json"
sf="$out/sparq-text-beir-ir-${BEIR_CUT}-${stamp}.json"
# A REALISTIC scored envelope: the same shape write_result + beir_deficits_json emit —
# top-level engine/suite identity + result.deficits_milli{recall_at_100,ndcg_at_10}.
envfile() {  # envfile <engine> <suite> <recall_milli> <ndcg_milli> <path>
  printf '{"engine":"%s","suite":"%s","version":"stub","env":{},"result":{"engine":"%s","deficits_milli":{"recall_at_100":%s,"ndcg_at_10":%s}}}\n' \
    "$1" "$2" "$1" "$3" "$4" > "$5"
}
lucene() { envfile "lucene-anserini" "beir-ir-${BEIR_CUT}" 8 15 "$lf"; }
sparq()  { envfile "sparq-text"      "beir-ir-${BEIR_CUT}" 21 40 "$sf"; }
case "${STUB_GC_MODE:-fail}" in
  fail)           exit 1 ;;                    # total failure: nothing written
  lucene-only)    lucene; exit 1 ;;            # sparq-text half died mid-gather
  # The BOX DIES after the first cut: cut 1 writes both envelopes normally, then the
  # next cut's invocation SIGKILLs the gather (its parent) — the self-terminating-box
  # shape. $out is wiped between runs, so the marker resets with it.
  die-after-first)
    if [ -e "$out/.stub-cut-done" ]; then
      kill -KILL "$PPID" 2>/dev/null
      sleep 5   # give the signal time to land; never hang if the kill did not work
      exit 1
    fi
    lucene; sparq; : > "$out/.stub-cut-done"; exit 0 ;;
  invalid-lucene) printf '{truncated' > "$lf"; sparq; exit 0 ;;
  both)           lucene; sparq; exit 0 ;;
  empty)          printf '{}\n' > "$lf"; printf '{}\n' > "$sf"; exit 0 ;;                    # parses, no scored evidence
  wrong-engine)   envfile "sparq-text" "beir-ir-${BEIR_CUT}" 8 15 "$lf"; sparq; exit 0 ;;    # lucene slot, wrong engine id
  wrong-cut)      envfile "lucene-anserini" "beir-ir-nfcorpus" 8 15 "$lf"; sparq; exit 0 ;;  # lucene slot, wrong cut/suite
esac
STUB

# ---- PATH shadows: no apt/network/toolchain touches anything real -------------------
printf '#!/usr/bin/env bash\nexit 0\n' > "$BIN/apt-get"
printf '#!/usr/bin/env bash\necho "stub openjdk 21"\nexit 0\n' > "$BIN/java"
printf '#!/usr/bin/env bash\nexit 1\n' > "$BIN/curl"
printf '#!/usr/bin/env bash\nexit "${STUB_CARGO_RC:-0}"\n' > "$BIN/cargo"
# aws shadow: records argv so a case can assert WHICH envelopes were uploaded and WHEN.
# No account, no network — the durable-egress library only ever shells out to this.
printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$*" >> "${AWS_STUB_LOG:-/dev/null}"\nexit 0\n' > "$BIN/aws"
cp "$BIN/cargo" "$CARGOHOME/bin/cargo"   # the script prepends $CARGO_HOME/bin later
# touch shadow: fail on demand (case 6 forces the GATHER_DONE touch to fail);
# execs the real touch otherwise so the success path still creates the sentinel.
cat > "$BIN/touch" <<TSTUB
#!/usr/bin/env bash
[ "\${STUB_TOUCH_RC:-0}" != 0 ] && exit "\$STUB_TOUCH_RC"
exec "$REAL_TOUCH" "\$@"
TSTUB
# python3 shadow: block `-m venv` (the script then falls back to the pre-made fake
# venv, or honestly fails provisioning when there is none); exec the real python3
# for everything else (JSON validation + gather-meta.json need it).
cat > "$BIN/python3" <<PYSTUB
#!/usr/bin/env bash
if [ "\${1:-}" = "-m" ] && [ "\${2:-}" = "venv" ]; then exit 1; fi
exec "$REAL_PY" "\$@"
PYSTUB
# fake VENV python3 (installed per-scenario): pip install/show are no-op stubs and
# `import pyserini, beir` succeeds; everything else execs the real python3.
cat > "$SANDBOX/venv-python3" <<PYSTUB
#!/usr/bin/env bash
if [ "\${1:-}" = "-m" ] && [ "\${2:-}" = "pip" ]; then
  if [ "\${3:-}" = "show" ]; then printf 'Name: stub\nVersion: 0.0-stub\n'; fi
  exit 0
fi
if [ "\${1:-}" = "-c" ]; then
  case "\${2:-}" in *"import pyserini"*) exit 0 ;; esac
fi
exec "$REAL_PY" "\$@"
PYSTUB
chmod +x "$REPO/scripts/gather-competitors.sh" "$BIN"/* "$CARGOHOME/bin/cargo" "$SANDBOX/venv-python3"

run_gather() {  # run_gather <provision:yes|no> <cargo_rc> <gc_mode> <logfile> [pre-cmd] -> echoes rc
  local provision="$1" cargo_rc="$2" mode="$3" logf="$4" rc
  rm -rf "$SBROOT" "$VENVDIR" "$RESULTS" "$SANDBOX/beir-data"
  mkdir -p "$SBROOT" "$SANDBOX/beir-data"
  if [ "$provision" = "yes" ]; then
    mkdir -p "$VENVDIR/bin"
    cp "$SANDBOX/venv-python3" "$VENVDIR/bin/python3"
  fi
  if [ -n "${5:-}" ]; then eval "$5"; fi   # per-case sabotage hook (runs after the reset)
  set +e
  env HOME="$SANDBOX/home" PATH="$BIN:$PATH" CARGO_HOME="$CARGOHOME" \
      SENTINEL_DIR="$SBROOT" BEIR_VENV_DIR="$VENVDIR" BEIR_DATA_DIR="$SANDBOX/beir-data" \
      BEIR_CUTS="${STUB_BEIR_CUTS:-scifact}" BEIR_K=100 \
      BENCH_RESULTS_S3_URI="${STUB_S3_URI:-}" AWS_STUB_LOG="${AWS_STUB_LOG:-/dev/null}" \
      STUB_GC_MODE="$mode" STUB_CARGO_RC="$cargo_rc" STUB_TOUCH_RC="${STUB_TOUCH_RC:-0}" \
      bash "$REPO/scripts/bench/canonical-beir-gather-instance.sh" > "$logf" 2>&1
  rc=$?
  set -e
  echo "$rc"
}

check_meta() {  # check_meta <case> <expected-status> <expected-canonical:true|false>
  if "$REAL_PY" -c '
import json, sys
m = json.load(open(sys.argv[1]))
assert m["status"] == sys.argv[2], m
assert m["canonical"] is (sys.argv[3] == "true"), m
' "$RESULTS/gather-meta.json" "$2" "$3" 2>/dev/null; then ok; else bad "$1: gather-meta.json lacks status=$2 canonical=$3"; fi
}

# ---- 1. TOTAL FAILURE: provision + build + cut all fail → failed, no GATHER_DONE ----
rc="$(run_gather no 1 fail "$SANDBOX/s1.log")"
[ "$rc" != 0 ] && ok || bad "total failure must exit nonzero (got rc=$rc)"
[ ! -e "$SBROOT/GATHER_DONE" ] && ok || bad "total failure must NOT emit the GATHER_DONE success sentinel"
[ -f "$SBROOT/GATHER_FAILED" ] && ok || bad "total failure must emit the GATHER_FAILED sentinel"
grep -q "status=failed" "$SBROOT/GATHER_FAILED" && ok || bad "GATHER_FAILED must carry status=failed"
grep -qa "gather FAILED" "$SANDBOX/s1.log" && ok || bad "total-failure console log must say 'gather FAILED'"
grep -qa "gather complete" "$SANDBOX/s1.log" && bad "total-failure log must not say 'gather complete' (launcher greps for it)" || ok
check_meta "total failure" "failed" "false"

# ---- 2. PARTIAL: lucene envelope lands, sparq-text missing → partial, nonzero -------
rc="$(run_gather yes 0 lucene-only "$SANDBOX/s2.log")"
[ "$rc" != 0 ] && ok || bad "partial gather must exit nonzero (got rc=$rc)"
[ ! -e "$SBROOT/GATHER_DONE" ] && ok || bad "partial gather must NOT emit GATHER_DONE"
grep -q "cut:scifact:sparq-text" "$SBROOT/GATHER_FAILED" 2>/dev/null && ok || bad "GATHER_FAILED must name the missing sparq-text envelope"
check_meta "partial" "partial" "false"
grep -qa "===ENVELOPE-BEGIN lucene-anserini-beir-ir-scifact" "$SANDBOX/s2.log" && ok || bad "partial evidence must still be dumped to the console for recovery"

# ---- 3. INVALID JSON: an envelope exists but does not parse → partial, nonzero ------
rc="$(run_gather yes 0 invalid-lucene "$SANDBOX/s3.log")"
[ "$rc" != 0 ] && ok || bad "invalid-JSON envelope must exit nonzero (got rc=$rc)"
[ ! -e "$SBROOT/GATHER_DONE" ] && ok || bad "invalid-JSON envelope must NOT emit GATHER_DONE"
grep -q "cut:scifact:lucene-anserini" "$SBROOT/GATHER_FAILED" 2>/dev/null && ok || bad "GATHER_FAILED must name the invalid lucene-anserini envelope"
check_meta "invalid json" "partial" "false"

# ---- 4. SUCCESS: both engines valid on every cut → GATHER_DONE, canonical=true ------
rc="$(run_gather yes 0 both "$SANDBOX/s4.log")"
[ "$rc" = 0 ] && ok || bad "validated success must exit 0 (got rc=$rc)"
[ -f "$SBROOT/GATHER_DONE" ] && ok || bad "validated success must emit GATHER_DONE"
[ ! -e "$SBROOT/GATHER_FAILED" ] && ok || bad "validated success must not emit GATHER_FAILED"
grep -qa "gather complete" "$SANDBOX/s4.log" && ok || bad "success log must say 'gather complete' (launcher greps for it)"
check_meta "success" "complete" "true"
# end-to-end recovery: the success console dump round-trips through the extractor
if [ -f "$EXTRACT" ]; then
  rm -rf "$SANDBOX/recovered"
  bash "$EXTRACT" "$SANDBOX/s4.log" "$SANDBOX/recovered" 2>/dev/null
  for n in lucene-anserini-beir-ir-scifact-20260101T000000Z.json sparq-text-beir-ir-scifact-20260101T000000Z.json; do
    if "$REAL_PY" -c 'import json, sys; json.load(open(sys.argv[1]))' "$SANDBOX/recovered/$n" 2>/dev/null; then
      ok
    else
      bad "console round-trip: $n not recovered as valid JSON"
    fi
  done
else
  bad "extractor $EXTRACT missing"
fi

# ---- 5. META WRITE FAILURE: envelopes all valid but gather-meta.json cannot be ------
# written (pre-made as a directory) → provenance is a canonical-success prerequisite,
# so the run must demote to partial: GATHER_FAILED, no GATHER_DONE, nonzero, and the
# console must NOT carry the 'gather complete' marker the launcher accepts as DONE.
rc="$(run_gather yes 0 both "$SANDBOX/s5.log" 'mkdir -p "$RESULTS/gather-meta.json"')"
[ "$rc" != 0 ] && ok || bad "meta-write failure must exit nonzero (got rc=$rc)"
[ ! -e "$SBROOT/GATHER_DONE" ] && ok || bad "meta-write failure must NOT emit GATHER_DONE"
grep -q "meta:gather-meta.json" "$SBROOT/GATHER_FAILED" 2>/dev/null && ok || bad "GATHER_FAILED must name the failed gather-meta.json write"
grep -q "status=partial" "$SBROOT/GATHER_FAILED" 2>/dev/null && ok || bad "meta-write failure must demote the run to status=partial"
grep -qa "STEP.*gather complete" "$SANDBOX/s5.log" && bad "meta-write-failure log must not say 'gather complete' (launcher greps for it)" || ok
grep -qa "gather FAILED" "$SANDBOX/s5.log" && ok || bad "meta-write-failure log must say 'gather FAILED'"

# ---- 6. GATHER_DONE TOUCH FAILURE: validation passes but the success sentinel ------
# cannot be written → the console 'gather complete' marker must never appear (it is
# emitted only AFTER a successful touch), so a launcher trusting console text can
# never report success without the sentinel durably on disk.
rc="$(STUB_TOUCH_RC=1 run_gather yes 0 both "$SANDBOX/s6.log")"
[ "$rc" != 0 ] && ok || bad "GATHER_DONE touch failure must exit nonzero (got rc=$rc)"
[ ! -e "$SBROOT/GATHER_DONE" ] && ok || bad "failed touch must not leave a GATHER_DONE sentinel"
grep -qa "STEP.*gather complete" "$SANDBOX/s6.log" && bad "touch-failure log must not say 'gather complete' without GATHER_DONE on disk" || ok
grep -q "sentinel:GATHER_DONE-touch-failed" "$SBROOT/GATHER_FAILED" 2>/dev/null && ok || bad "GATHER_FAILED must name the failed GATHER_DONE touch"
grep -qa "gather FAILED" "$SANDBOX/s6.log" && ok || bad "touch-failure log must say 'gather FAILED'"

# ---- 7. EMPTY-BUT-PARSING envelopes: `{}` for both engines → no scored evidence, so
# neither validates → failed, nonzero, no GATHER_DONE (an envelope must carry the
# claimed Recall@100/nDCG@10 deficits, not merely parse as JSON).
rc="$(run_gather yes 0 empty "$SANDBOX/s7.log")"
[ "$rc" != 0 ] && ok || bad "empty {} envelopes must exit nonzero (got rc=$rc)"
[ ! -e "$SBROOT/GATHER_DONE" ] && ok || bad "empty {} envelopes must NOT emit GATHER_DONE"
check_meta "empty envelopes" "failed" "false"

# ---- 8. WRONG-IDENTITY envelopes: the lucene slot carries the wrong engine id (8a)
# or the wrong beir-ir cut/suite (8b) → that engine fails identity, so the run is
# partial (only sparq-text valid), never canonical.
rc="$(run_gather yes 0 wrong-engine "$SANDBOX/s8a.log")"
[ "$rc" != 0 ] && ok || bad "wrong-engine envelope must exit nonzero (got rc=$rc)"
grep -q "cut:scifact:lucene-anserini" "$SBROOT/GATHER_FAILED" 2>/dev/null && ok || bad "GATHER_FAILED must name the wrong-engine lucene envelope"
check_meta "wrong engine" "partial" "false"
rc="$(run_gather yes 0 wrong-cut "$SANDBOX/s8b.log")"
[ "$rc" != 0 ] && ok || bad "wrong-cut envelope must exit nonzero (got rc=$rc)"
grep -q "cut:scifact:lucene-anserini" "$SBROOT/GATHER_FAILED" 2>/dev/null && ok || bad "GATHER_FAILED must name the wrong-cut lucene envelope"
check_meta "wrong cut" "partial" "false"

# ---- 9. STALE prefix-matching envelopes: valid-looking envelopes from a PRIOR run are
# already on disk, but THIS gather produces nothing (fail mode). The pre-run snapshot
# must reject them, so the run is failed — a re-run whose gather died can never be
# certified canonical by reusing stale evidence.
stale='mkdir -p "$RESULTS"
printf "%s\n" "{\"engine\":\"lucene-anserini\",\"suite\":\"beir-ir-scifact\",\"result\":{\"engine\":\"lucene-anserini\",\"deficits_milli\":{\"recall_at_100\":8,\"ndcg_at_10\":15}}}" > "$RESULTS/lucene-anserini-beir-ir-scifact-20259901T000000Z.json"
printf "%s\n" "{\"engine\":\"sparq-text\",\"suite\":\"beir-ir-scifact\",\"result\":{\"engine\":\"sparq-text\",\"deficits_milli\":{\"recall_at_100\":21,\"ndcg_at_10\":40}}}" > "$RESULTS/sparq-text-beir-ir-scifact-20259901T000000Z.json"'
rc="$(run_gather yes 0 fail "$SANDBOX/s9.log" "$stale")"
[ "$rc" != 0 ] && ok || bad "stale pre-existing envelopes must not certify a failed gather (got rc=$rc)"
[ ! -e "$SBROOT/GATHER_DONE" ] && ok || bad "stale envelopes must NOT emit GATHER_DONE"
check_meta "stale reuse" "failed" "false"

# ---- 10. DURABLE EGRESS IS INCREMENTAL (sq-ffaa9, PR #4681 review round 2) ----------
# The box DIES partway through: cut 1 lands both envelopes, then cut 2's invocation
# SIGKILLs the gather. The completed cut must ALREADY be in S3 — the whole point of the
# channel on a self-terminating box. Before this, the only bench_egress_push sat in the
# end-of-gather console-dump loop, which a killed box never reaches, so every completed
# envelope was lost; a grep for the function name could not tell the two apart, which is
# why this case asserts the recorded UPLOAD instead.
AWS_STUB_LOG="$SANDBOX/aws.log"; : > "$AWS_STUB_LOG"
rc="$(STUB_BEIR_CUTS="scifact trec-covid" STUB_S3_URI="s3://bkt/gathers/run1" \
      AWS_STUB_LOG="$AWS_STUB_LOG" run_gather yes 0 die-after-first "$SANDBOX/s10.log")"
[ "$rc" != 0 ] && ok || bad "a gather killed mid-run must not exit 0 (got rc=$rc)"
[ ! -e "$SBROOT/GATHER_DONE" ] && ok || bad "a gather killed mid-run must not leave GATHER_DONE"
grep -qa 'ENVELOPE-BEGIN' "$SANDBOX/s10.log" \
  && bad "the killed gather somehow reached its end-of-run dump loop — the case proves nothing" || ok
for e in lucene-anserini sparq-text; do
  if grep -q "^s3 cp .*${e}-beir-ir-scifact-.*\.json s3://bkt/gathers/run1/${e}-beir-ir-scifact-.*\.json$" "$AWS_STUB_LOG"; then
    ok
  else
    bad "completed cut's $e envelope was never uploaded before the box died; aws log: $(cat "$AWS_STUB_LOG")"
  fi
done
grep -q 'trec-covid' "$AWS_STUB_LOG" \
  && bad "uploaded an envelope for a cut that never completed" || ok

# ---- 11. the incremental sweep does not RE-upload on the success path ---------------
# Every cut pushes as it finishes and the end-of-gather sweep is a RETRY pass, so a
# clean run must upload each envelope EXACTLY once (a per-cut push plus an unconditional
# final push would double every upload).
: > "$AWS_STUB_LOG"
rc="$(STUB_S3_URI="s3://bkt/gathers/run2" AWS_STUB_LOG="$AWS_STUB_LOG" \
      run_gather yes 0 both "$SANDBOX/s11.log")"
[ "$rc" = 0 ] && ok || bad "egress-enabled success path must still exit 0 (got rc=$rc)"
for e in lucene-anserini sparq-text gather-meta; do
  n=$(grep -c "^s3 cp .*/${e}[^ ]*\.json s3://bkt/gathers/run2/" "$AWS_STUB_LOG" || true)
  [ "$n" = 1 ] && ok || bad "success path uploaded the $e envelope $n times, want exactly 1"
done

echo ""
echo "test_beir_gather_sentinel: ${pass} passed, ${fail} failed."
[ "$fail" -eq 0 ] || exit 1
echo "test_beir_gather_sentinel: OK — failed/partial/invalid/provenance-less gathers never emit GATHER_DONE or the console success marker; success validates + round-trips."
