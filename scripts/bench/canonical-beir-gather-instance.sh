#!/usr/bin/env bash
# [FABLE-5] sq-tvzyi — INSTANCE-side canonical BEIR IR-quality gather (FTS quality axis).
#
# 🤖 SPARQ agent. Runs ON the dedicated EC2 gather box (launched by
# scripts/bench/canonical-beir-bench.sh) from a cloned sparq checkout, as root.
# Produces the BEIR quality-axis results `research/gap-fts-2026-07.md` §4 needs:
# sparq-text vs Lucene/Anserini (the kernel BM25 IR ORACLE, sq-1fz0), BOTH scored by
# the ONE shared scorer (scripts/bench-adapters/beir_ir_adapter.py) against the SAME
# BEIR cut + qrels — Recall@100 / nDCG@10 emitted as DEFICITS (smaller is better).
# GATHER-ONLY: the BEIR corpus is not redistributable in-repo, so this is a download
# step; a maintainer vendors reviewed deficits + provenance into §4 deliberately —
# results are never auto-committed.
#
# This script carries the heavy pip `pyserini`+`beir` provisioning that was NOT part
# of the wave-1 bin-packed box (scripts/bench/multi-axis-box.sh ran the latency axis
# only): JDK 21 for Anserini's Lucene, a dedicated venv, torch-CPU pulled by pip. A
# failed provision degrades to an honest NOT-RUN step log — a RE-RUN action, NEVER a
# sparq win by omission (a quality claim cannot be made either way without the run).
#
# Results: bench/competitor-results/*.json (one envelope per engine/cut, written by
# scripts/gather-competitors.sh) + $RESULTS_DIR/gather-meta.json (box provenance).
# Every envelope is ALSO cat'd into the gather log between ===ENVELOPE-BEGIN/END===
# markers so `aws ec2 get-console-output` can recover results even if the SSH pull
# path dies (envelopes are a few KB; recovered by scripts/bench/extract-console-envelopes.sh)
# and — when the launcher passed BENCH_RESULTS_S3_URI (sq-ffaa9) — uploaded to the
# run-scoped S3 prefix as each CUT finishes, so a box that dies mid-gather has already
# made its completed cuts durable; that is the one channel that survives an unusable
# serial console.
#
# SENTINEL PROTOCOL (machine-enforced honesty): /root/GATHER_DONE is written ONLY
# after every requested cut has a VALID-JSON envelope for BOTH engines (the
# lucene-anserini oracle AND sparq-text) AND gather-meta.json has been durably
# written + independently re-parsed with canonical=true — provenance is a
# canonical-success ARTIFACT, not a best-effort side channel. GATHER_DONE is
# touched BEFORE the 'gather complete' console marker, so the console text can
# never appear without the sentinel on disk (the launcher accepts that text as
# success when SSH is dead). A failed provision / build / cut / meta write instead
# writes /root/GATHER_FAILED (+ gather-meta.json canonical=false, status
# partial|failed) and exits nonzero — so an empty or partial gather can never look
# like a completed canonical run at the launcher protocol level. NEVER shuts the
# box down — the launcher terminates it (its user-data watchdog is the orphan-proof
# backstop).
#
# Env knobs (all defaulted):
#   BEIR_CUTS="scifact"   space-separated BEIR cuts (scifact | trec-covid)
#   BEIR_K=100            top-k for Recall@k retrieval
#   BEIR_DATA_DIR=/root/beir-data   download cache for the BEIR cuts
#   PYSERINI_PIN= / BEIR_PIN=       optional pip version pins (e.g. "==1.2.0");
#                                   resolved versions are ALWAYS logged for provenance
#   BEIR_VENV_DIR=/root/beir-venv / SENTINEL_DIR=/root   overridden ONLY by the
#                                   hermetic self-test (test_beir_gather_sentinel.sh)
set -uo pipefail   # NOT -e: one failed cut/provision step must never kill the gather

# [FABLE-5] DEFINE HOME/USER/LOGNAME BEFORE ANYTHING ELSE — cloud-init's root
# scripts_user context exports none of them, and under `set -u` the first bare $HOME
# aborts the gather (the sq-hmd7l.32 wave-2 stall). cargo/rustup/java/pip also read
# $HOME at runtime for their caches.
: "${HOME:=$(getent passwd "$(id -u)" 2>/dev/null | cut -d: -f6)}"
: "${HOME:=/root}"
export HOME
: "${USER:=$(id -un 2>/dev/null || echo root)}"; export USER
: "${LOGNAME:=$USER}"; export LOGNAME
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"

# Mirror all output to the serial console — the only zero-dependency telemetry channel
# on a self-terminating box (`aws ec2 get-console-output`). Best-effort: fall through
# untee'd when /dev/console is not writable (a local test run).
if [ -w /dev/console ] 2>/dev/null; then
  exec > >(tee -a /dev/console) 2>&1
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

BEIR_CUTS="${BEIR_CUTS:-scifact}"
BEIR_K="${BEIR_K:-100}"
BEIR_DATA_DIR="${BEIR_DATA_DIR:-/root/beir-data}"
PYSERINI_PIN="${PYSERINI_PIN:-}"
BEIR_PIN="${BEIR_PIN:-}"
RESULTS_DIR="$ROOT/bench/competitor-results"   # fixed by scripts/gather-competitors.sh
VENV="${BEIR_VENV_DIR:-/root/beir-venv}"
# Sentinels (GATHER_STEP/GATHER_DONE/GATHER_FAILED) live in /root on the real box —
# that path is what the launcher polls over SSH. Overridable ONLY so the hermetic
# self-test (scripts/tests/test_beir_gather_sentinel.sh) can run unprivileged.
SENTINEL_DIR="${SENTINEL_DIR:-/root}"

step() { echo "[STEP $(date -u +%Y-%m-%dT%H:%M:%SZ)] $*" | tee -a "$SENTINEL_DIR/GATHER_STEP" >&2; }

# [OPUS-5] sq-ffaa9 — durable result egress. bench_egress_push is a successful no-op
# unless the launcher passed BENCH_RESULTS_S3_URI in, so a run without the instance
# profile attached behaves exactly as before (console + SSH pull only).
. "$HERE/bench-result-egress.sh"

mkdir -p "$RESULTS_DIR" "$BEIR_DATA_DIR"
step "canonical BEIR gather start: cuts='$BEIR_CUTS' k=$BEIR_K commit=$(git rev-parse --short HEAD 2>/dev/null || echo '?')"
df -h / >&2

# ---- 0. apt deps (self-contained; idempotent) ----------------------------------------
step "apt deps"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq || true
# openjdk-21: pyserini drives Anserini's Lucene through the JVM; python3-venv: Ubuntu
# 24.04 pip is PEP-668 externally-managed, so the heavy libs go in a dedicated venv.
apt-get install -y -qq build-essential pkg-config git curl jq unzip bc \
  python3 python3-pip python3-venv openjdk-21-jdk-headless || step "WARN: apt install returned non-zero"
export JAVA_HOME="${JAVA_HOME:-/usr/lib/jvm/java-21-openjdk-amd64}"
export PATH="$JAVA_HOME/bin:$PATH"
java -version 2>&1 | head -2 >&2 || step "WARN: java not runnable — Anserini column will degrade honestly"

# ---- 1. the heavy pip provisioning (venv: pyserini + beir) ---------------------------
step "pip provision: venv + pyserini${PYSERINI_PIN} + beir${BEIR_PIN}"
PROVISIONED=0
if python3 -m venv "$VENV" 2>/dev/null || [ -x "$VENV/bin/python3" ]; then
  export PATH="$VENV/bin:$PATH"   # gather-competitors.sh calls bare `python3`
  python3 -m pip install -q --upgrade pip || true
  python3 -m pip install -q "pyserini${PYSERINI_PIN}" "beir${BEIR_PIN}" \
    || step "WARN: pip install pyserini/beir returned non-zero — trying import anyway"
  # Some pyserini releases import faiss at module load; pull faiss-cpu iff the plain
  # import fails, then re-check (kept out of the default install to stay minimal).
  if ! python3 -c 'import pyserini, beir' 2>/dev/null; then
    step "import failed — retrying with faiss-cpu"
    python3 -m pip install -q faiss-cpu || true
  fi
  if python3 -c 'import pyserini, beir' 2>/dev/null; then
    PROVISIONED=1
    step "provision ok: $(python3 -m pip show pyserini beir 2>/dev/null | grep -E '^(Name|Version):' | tr '\n' ' ')"
  fi
fi
[ "$PROVISIONED" = 1 ] || step "WARN: pyserini/beir provisioning FAILED — the gather below will die per-cut with the adapter's own error (honest NOT-RUN, a re-run action)"

# ---- 2. rust toolchain + the sparq-text beir_text retriever --------------------------
if ! command -v cargo >/dev/null 2>&1; then
  step "rustup install"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal || true
fi
export PATH="$CARGO_HOME/bin:/root/.cargo/bin:$HOME/.cargo/bin:$PATH"
. /root/.cargo/env 2>/dev/null || . "$HOME/.cargo/env" 2>/dev/null || true

step "build sparq-text beir_text example (release)"
SPARQ_TEXT_BEIR=""
if cargo build --release -p sparq-text --example beir_text; then
  SPARQ_TEXT_BEIR="$ROOT/target/release/examples/beir_text"
  step "beir_text built: $SPARQ_TEXT_BEIR"
else
  step "WARN: beir_text build FAILED — the sparq-text side of the comparison cannot run; the oracle-only gather still records Lucene/Anserini (a re-run action for the sparq column)"
fi

# ---- 2b. isolate THIS invocation's output (reject stale prefix-matching envelopes) ---
# RESULTS_DIR is shared, git-ignored, and is NEVER cleared between runs, so a PRIOR
# run's <engine>-beir-ir-<cut>-*.json would otherwise satisfy the outcome validation
# below even when THIS gather produced nothing — a re-run whose gather died would be
# certified canonical on stale evidence. Snapshot the pre-existing envelope paths and
# treat them as NOT-produced-by-this-run: valid_envelope only accepts a file absent
# from this list. Non-destructive (other axes may share the box) — we reject stale
# paths rather than clearing the directory.
PRE_EXISTING="$(mktemp)"
trap 'rm -f "$PRE_EXISTING"' EXIT
for f in "$RESULTS_DIR"/*.json; do [ -f "$f" ] && printf '%s\n' "$f"; done > "$PRE_EXISTING"
step "pre-existing envelopes recorded as stale (rejected): $(wc -l < "$PRE_EXISTING" | tr -d ' ')"

# ---- 3. the gather, per cut (recipe sq-1fz0: one shared scorer, same cut + qrels) ----
for cut in $BEIR_CUTS; do
  step "gather-competitors.sh --run --only lucene-anserini (cut=$cut k=$BEIR_K)"
  if env BEIR_CUT="$cut" BEIR_K="$BEIR_K" BEIR_DATA_DIR="$BEIR_DATA_DIR" \
      ${SPARQ_TEXT_BEIR:+SPARQ_TEXT_BEIR="$SPARQ_TEXT_BEIR"} \
      bash scripts/gather-competitors.sh --run --only lucene-anserini; then
    step "cut $cut done"
  else
    step "WARN: gather exited non-zero for cut $cut — that cut stays NOT-MEASURED (a re-run action, never a sparq win)"
  fi
  # Durable egress AS SOON AS this cut's envelopes exist — the box self-terminates, so a
  # completed cut that waited for the remaining cuts (and for the validation/meta stages)
  # would be LOST if the box died in between. No-op unless the launcher configured it.
  bench_egress_sweep "$RESULTS_DIR"
done

# ---- 4. machine-enforced outcome validation (per cut, BOTH engines) ------------------
# The §4 comparison needs, for EVERY requested cut, a valid-JSON envelope from BOTH
# engines: lucene-anserini (the oracle) AND sparq-text. The sentinel protocol below
# is gated on this validation, so a run where pyserini cannot import, sparq-text
# cannot build, or a cut dies is DISTINGUISHABLE from a completed canonical gather
# — the WARN-and-continue steps above only keep the box alive to collect whatever
# partial evidence exists; they never launder it into success.
# valid_envelope <engine> <cut> → 0 iff THIS run produced ≥1 well-formed, correctly
# identified envelope for <engine> on <cut>. A file that merely parses as JSON is NOT
# enough (that let `{}` or a stale prefix-matching file certify success): it must be
# ABSENT from the pre-run snapshot (produced by this invocation, not a prior one),
# carry the exact engine + `beir-ir-<cut>` suite identity, and expose FINITE
# Recall@100 / nDCG@10 deficit values under result.deficits_milli — the scored
# evidence the §4 comparison actually needs. This is what makes GATHER_DONE certify
# the claimed measurements rather than any JSON that happens to sit in RESULTS_DIR.
valid_envelope() {
  local engine="$1" cut="$2" f
  for f in "$RESULTS_DIR/${engine}-beir-ir-${cut}"-*.json; do
    [ -f "$f" ] || continue
    grep -qxF "$f" "$PRE_EXISTING" 2>/dev/null && continue   # stale: predates this run
    python3 - "$f" "$engine" "beir-ir-${cut}" <<'PY' 2>/dev/null && return 0
import json, math, sys
path, want_engine, want_suite = sys.argv[1], sys.argv[2], sys.argv[3]
try:
    d = json.load(open(path))
except Exception:
    sys.exit(1)
if not isinstance(d, dict) or d.get("engine") != want_engine or d.get("suite") != want_suite:
    sys.exit(1)
r = d.get("result")
if not isinstance(r, dict):
    sys.exit(1)
dm = r.get("deficits_milli")
if not isinstance(dm, dict):
    sys.exit(1)
for key in ("recall_at_100", "ndcg_at_10"):
    v = dm.get(key)
    if isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v):
        sys.exit(1)
sys.exit(0)
PY
  done
  return 1
}
FAILURES=""
VALID_ENVELOPES=0
[ "$PROVISIONED" = 1 ] || FAILURES="$FAILURES provision:pyserini-beir"
[ -n "$SPARQ_TEXT_BEIR" ] || FAILURES="$FAILURES build:beir_text"
for cut in $BEIR_CUTS; do
  for engine in lucene-anserini sparq-text; do
    if valid_envelope "$engine" "$cut"; then
      VALID_ENVELOPES=$((VALID_ENVELOPES + 1))
    else
      FAILURES="$FAILURES cut:${cut}:${engine}:envelope-missing-or-invalid"
    fi
  done
done
if [ -z "$FAILURES" ]; then
  GATHER_STATUS="complete"
elif [ "$VALID_ENVELOPES" -gt 0 ]; then
  GATHER_STATUS="partial"
else
  GATHER_STATUS="failed"
fi
CANONICAL_PY=False; [ "$GATHER_STATUS" = "complete" ] && CANONICAL_PY=True
step "outcome validation: status=$GATHER_STATUS valid_envelopes=$VALID_ENVELOPES${FAILURES:+ missing:$FAILURES}"

# ---- 5. box provenance for the §4 transcription (a CANONICAL-SUCCESS artifact) -------
step "write gather-meta.json"
IID="$(TOK=$(curl -s -m 3 -X PUT http://169.254.169.254/latest/api/token -H 'X-aws-ec2-metadata-token-ttl-seconds: 60' 2>/dev/null); \
  curl -s -m 3 -H "X-aws-ec2-metadata-token: $TOK" http://169.254.169.254/latest/meta-data/instance-id 2>/dev/null || true)"
META_OK=0
python3 - "$RESULTS_DIR/gather-meta.json" <<META && META_OK=1
import json, subprocess, sys, datetime
def sh(c):
    try: return subprocess.run(c, shell=True, capture_output=True, text=True, timeout=30).stdout.strip()
    except Exception: return ""
json.dump({
    "canonical": ${CANONICAL_PY}, "status": "${GATHER_STATUS}",
    "missing": "${FAILURES}".split(),
    "axis": "fts-beir-ir-quality", "bead": "sq-tvzyi",
    "utc": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "instance_id": "${IID}" or "unknown",
    "commit": sh("git rev-parse HEAD"),
    "cuts": "${BEIR_CUTS}".split(), "k": int("${BEIR_K}"),
    "java": sh("java -version 2>&1 | head -1"),
    "pyserini": sh("python3 -m pip show pyserini 2>/dev/null | grep ^Version:"),
    "beir": sh("python3 -m pip show beir 2>/dev/null | grep ^Version:"),
}, open(sys.argv[1], "w"), indent=1)
META
# Durable-provenance gate: independently re-parse what actually landed on disk and
# demand it carries the validated status. A run whose provenance record is missing,
# truncated, or stale can never be canonical — demote it (complete -> partial) so
# the sentinel branch below writes GATHER_FAILED and exits nonzero instead of
# laundering the write failure into success.
if [ "$META_OK" = 1 ]; then
  python3 -c 'import json, sys
m = json.load(open(sys.argv[1]))
assert m["status"] == sys.argv[2]
assert m["canonical"] is (sys.argv[2] == "complete")' \
    "$RESULTS_DIR/gather-meta.json" "$GATHER_STATUS" 2>/dev/null || META_OK=0
fi
if [ "$META_OK" != 1 ]; then
  FAILURES="$FAILURES meta:gather-meta.json-write-or-reparse-failed"
  [ "$GATHER_STATUS" = "complete" ] && GATHER_STATUS="partial"
  step "WARN: gather-meta.json write/re-parse FAILED — no durable provenance, demoted to status=$GATHER_STATUS (NOT canonical)"
fi
bench_egress_sweep "$RESULTS_DIR"   # provenance out too, before the sentinel branch

# ---- 6. egress RETRY sweep + console-output backstop + sentinel protocol --------------
# The envelope dump runs in EVERY outcome — partial evidence must stay recoverable
# from the serial console (scripts/bench/extract-console-envelopes.sh parses it).
# The sweep here is the RETRY pass for uploads that failed at their per-cut attempt
# (bench_egress_sweep skips what already landed), never the first attempt.
step "envelopes in $RESULTS_DIR:"
ls -la "$RESULTS_DIR" >&2 || true
bench_egress_sweep "$RESULTS_DIR"
for f in "$RESULTS_DIR"/*.json; do
  [ -f "$f" ] || continue
  echo "===ENVELOPE-BEGIN $(basename "$f")==="
  cat "$f"
  echo "===ENVELOPE-END==="
done
df -h / >&2
if [ "$GATHER_STATUS" = "complete" ] && touch "$SENTINEL_DIR/GATHER_DONE"; then
  # Sentinel BEFORE the console marker: the launcher accepts the console text
  # 'gather complete' as success when SSH is dead, so that text must be impossible
  # to emit unless GATHER_DONE is already durably on disk — a crash (or a failed
  # touch) between the two must read as failure, never success.
  step "gather complete"
else
  if [ "$GATHER_STATUS" = "complete" ]; then
    # Validation passed but the success sentinel could not be written — without it
    # the launcher protocol cannot certify the run, so it is NOT canonical.
    GATHER_STATUS="partial"; FAILURES="$FAILURES sentinel:GATHER_DONE-touch-failed"
  fi
  # Distinct FAILURE sentinel + nonzero exit: the launcher surfaces partial/failed
  # instead of treating the run as canonical. Honest NOT-RUN, a re-run action —
  # never a sparq win by omission.
  {
    echo "status=$GATHER_STATUS"
    for miss in $FAILURES; do echo "missing $miss"; done
  } > "$SENTINEL_DIR/GATHER_FAILED"
  step "gather FAILED (status=$GATHER_STATUS) — NOT canonical; missing:$FAILURES"
  exit 1
fi
