#!/usr/bin/env bash
# [FABLE-5] sq-hmd7l.32 — INSTANCE-side canonical MATERIALIZATION (+ HDT decode) gather.
#
# 🤖 SPARQ agent. Runs ON the dedicated quiet EC2 box (launched by
# scripts/bench/canonical-materialize-bench.sh) from a cloned sparq checkout, as root.
# Produces the D6 (materialization-throughput) canonical envelopes the gap table needs:
# sparq `reason` OWL-RL/RDFS vs Apache Jena's rule reasoners vs VLog vs Nemo closing the
# SAME LUBM (ABox+TBox) input at univ>=100, one engine active at a time (the harness runs
# engines strictly sequentially), plus — same box, afterwards — the HDT decode-only
# comparison (sparq-hdt vs hdt-cpp) with the numeric in-container decode_us (sq-hmd7l.33).
#
# VLog / Nemo run the VALIDATED bench/reason-encodings/ programs (closure set-identical
# to sparq's, sq-hmd7l.30/.31); both are built FROM SOURCE at a PINNED version below —
# a build failure degrades to the adapters' honest NOT-RUN-LOCALLY row (a RE-RUN action,
# NEVER a sparq win by omission). Timing basis is each engine's own loaded-graph
# materialization report (see materialize-same-box.sh METHODOLOGY).
#
# Envelopes: $OUT/materialize-lubm<univ>-<UTC>.json + $OUT/hdt-lubm<univ>-<UTC>.json,
# CANONICAL=1. Every envelope is ALSO cat'd into the gather log between
# ===ENVELOPE-BEGIN/END=== markers so `aws ec2 get-console-output` can recover results
# even if the SSH pull path dies (envelopes are a few KB), and — when the launcher passed
# BENCH_RESULTS_S3_URI (sq-ffaa9) — uploaded to the run-scoped S3 prefix as each STAGE
# finishes (per LUBM scale, then the HDT decode), so a box that dies mid-gather has
# already made its completed scales durable; that is the one channel that survives an AMI
# whose console output is unusable. Writes /root/GATHER_DONE when
# everything is on disk. NEVER shuts the box down — the launcher terminates it (its
# user-data watchdog is the orphan-proof backstop).
#
# Env knobs (all defaulted):
#   LUBM_UNIVS="1 100"  scales (univ=1 first: the pinned-closure ORACLE run that guards
#                        the whole toolchain before the big univ>=100 gather)
#   MAT_ITERS="5 3"     best-of-N per scale   TIMEOUT_S=3600   per-engine cap, s
#   JAVA_XMX=24g        Jena heap (the JVM default ~25%-of-RAM OOMs at univ=100)
#   HDT=1               also run the HDT decode gather        OUT=<repo>/bench/gather-out
#   HDT_LUBM_UNIV=1     LUBM scale converted to HDT for the decode comparison
set -uo pipefail   # NOT -e: one failed engine/build must never kill the gather

# [FABLE-5] sq-hmd7l.32 — DEFINE HOME/USER/LOGNAME BEFORE ANYTHING ELSE. This script is
# started by the launcher's user-data as `setsid nohup env LUBM_UNIVS=... bash <this>` —
# cloud-init's root scripts_user context has NO HOME/USER/LOGNAME exported, and under
# `set -u` the FIRST bare `$HOME` (the rustup PATH line) aborted the whole gather with
# "line NN: HOME: unbound variable" (sq-hmd7l.32 wave-2 stall, diagnosed from console).
# Beyond the set -u trip, cargo/rustup/java read $HOME at RUNTIME for their caches
# (~/.cargo registry/config), so an unset HOME would also break the builds. This runs as
# root on the box; compute the real home dir, default to /root, and export to children.
: "${HOME:=$(getent passwd "$(id -u)" 2>/dev/null | cut -d: -f6)}"
: "${HOME:=/root}"
export HOME
: "${USER:=$(id -un 2>/dev/null || echo root)}"; export USER
: "${LOGNAME:=$USER}"; export LOGNAME
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"

# [FABLE-5] sq-hmd7l.32 — MIRROR ALL OUTPUT TO THE SERIAL CONSOLE. On a self-terminating
# gather box the ONLY zero-dependency telemetry channel is `aws ec2 get-console-output`
# (SSH can break under a saturated build box; cloud-init's own console buffer stops when
# scripts_user returns). Tee every step + child-process line to /dev/console so progress
# and the final envelopes are recoverable even if SSH never works. Best-effort: if
# /dev/console is not writable (e.g. a local test run) fall through untee'd.
if [ -w /dev/console ] 2>/dev/null; then
  exec > >(tee -a /dev/console) 2>&1
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

LUBM_UNIVS="${LUBM_UNIVS:-1 100}"
MAT_ITERS="${MAT_ITERS:-5 3}"
TIMEOUT_S="${TIMEOUT_S:-3600}"
JAVA_XMX="${JAVA_XMX:-24g}"
HDT="${HDT:-1}"
HDT_LUBM_UNIV="${HDT_LUBM_UNIV:-1}"
OUT="${OUT:-$ROOT/bench/gather-out}"

# Pinned competitor sources (recorded into the envelopes as VLOG_VERSION/NEMO_VERSION).
VLOG_COMMIT="${VLOG_COMMIT:-10e23c7453b93876b93c69a51c030aef962b9348}"   # karmaresearch/vlog master 2026-07
NEMO_TAG="${NEMO_TAG:-v0.9.1}"                                           # knowsys/nemo release

step() { echo "[STEP $(date -u +%Y-%m-%dT%H:%M:%SZ)] $*" | tee -a /root/GATHER_STEP >&2; }

# [OPUS-5] sq-ffaa9 — durable result egress. bench_egress_push is a successful no-op
# unless the launcher passed BENCH_RESULTS_S3_URI in, so this changes nothing on a run
# without the instance profile attached.
. "$HERE/bench-result-egress.sh"

mkdir -p "$OUT"
step "canonical materialize gather start: univs='$LUBM_UNIVS' iters='$MAT_ITERS' timeout=${TIMEOUT_S}s xmx=$JAVA_XMX hdt=$HDT commit=$(git rev-parse --short HEAD 2>/dev/null || echo '?')"
df -h / >&2

# ---- 0. deps (self-contained; idempotent) -------------------------------------------
step "apt deps"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq || true
# openjdk JDK (javac: the LUBM UBA generator build + the Jena driver compile),
# raptor2-utils (rapper: RDF/XML -> N-Triples), cmake+zlib (VLog), docker (hdt-cpp),
# libssl-dev (Nemo's cargo dep chain pulls openssl-sys, which needs libssl.pc +
# pkg-config or the Nemo build fails immediately — sq-hmd7l.32 wave-3).
apt-get install -y -qq build-essential g++ cmake pkg-config git curl jq python3 unzip \
  docker.io openjdk-21-jdk-headless raptor2-utils zlib1g-dev libssl-dev bc || step "WARN: apt install returned non-zero"

step "start docker (hdt-cpp column)"
systemctl enable --now docker >/dev/null 2>&1 || systemctl start docker >/dev/null 2>&1 || true
for _ in $(seq 1 30); do docker info >/dev/null 2>&1 && break; sleep 2; done
docker info >/dev/null 2>&1 || step "WARN: docker daemon not reachable (hdt-cpp will degrade honestly)"

if ! command -v cargo >/dev/null 2>&1; then
  step "rustup install"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal || true
fi
export PATH="/root/.cargo/bin:$HOME/.cargo/bin:$PATH"
. /root/.cargo/env 2>/dev/null || . "$HOME/.cargo/env" 2>/dev/null || true

# ---- 1. build sparq-cli --------------------------------------------------------------
step "build sparq-cli (release)"
if cargo build --release -p sparq-cli; then
  step "sparq-cli built"
else
  step "FATAL: sparq-cli build failed — no reference column possible"; touch /root/GATHER_DONE; exit 1
fi

# ---- 2. build VLog @ pinned commit (gather-only dep; stays out of git) ----------------
VLOG_BIN=""
step "build VLog @ ${VLOG_COMMIT:0:12}"
if git clone -q https://github.com/karmaresearch/vlog.git /root/vlog-src \
   && git -C /root/vlog-src checkout -q "$VLOG_COMMIT" \
   && mkdir -p /root/vlog-src/build \
   && (cd /root/vlog-src/build && cmake -DCMAKE_CXX_FLAGS="-include cstdint" .. >/dev/null); then
  # First make pass fetches + builds the kognac/trident ExternalProjects; the pinned
  # trident source has a GCC-13 missing-<cstdint> include the TOP-LEVEL flag cannot
  # reach (ExternalProject flags don't propagate) — patch the fetched header, resume.
  (cd /root/vlog-src/build && make -j"$(nproc)" >/root/vlog-make1.log 2>&1) || {
    step "VLog make pass 1 failed (expected on GCC-13: trident <cstdint>) — patching + resuming"
    sed -i '1i #include <cstdint>' /root/vlog-src/build/trident/include/trident/utils/fft.h 2>/dev/null || true
    (cd /root/vlog-src/build && make -j"$(nproc)" >/root/vlog-make2.log 2>&1) || true
  }
  [ -x /root/vlog-src/build/vlog ] && VLOG_BIN=/root/vlog-src/build/vlog
fi
if [ -n "$VLOG_BIN" ]; then step "VLog built: $VLOG_BIN"; else step "WARN: VLog build FAILED — column degrades to honest NOT-RUN-LOCALLY (re-run action)"; fi

# ---- 3. build Nemo @ pinned tag (gather-only dep; stays out of git) -------------------
NEMO_BIN=""
step "build Nemo @ $NEMO_TAG"
if git clone -q --depth 1 --branch "$NEMO_TAG" https://github.com/knowsys/nemo.git /root/nemo-src \
   && (cd /root/nemo-src && cargo build -r -p nemo-cli >/root/nemo-build.log 2>&1); then
  [ -x /root/nemo-src/target/release/nmo ] && NEMO_BIN=/root/nemo-src/target/release/nmo
fi
if [ -n "$NEMO_BIN" ]; then step "Nemo built: $NEMO_BIN"; else step "WARN: Nemo build FAILED — column degrades to honest NOT-RUN-LOCALLY (re-run action)"; fi

# ---- 4. the canonical materialization gather -----------------------------------------
# Pass the competitor bins through EXPORTED env, not an inline `${VLOG_BIN:+VLOG=…}`
# assignment prefix: a NAME=value produced by parameter expansion lands in COMMAND-WORD
# position (bash only treats LITERAL NAME=value tokens as an assignment prefix), so the
# old form tried to exec a file literally named "VLOG=/root/…/vlog" → "No such file or
# directory" and the materialize run never happened whenever a competitor bin was present
# (sq-hmd7l.32 wave-3). Export conditionally instead — absent bins stay unset so the
# adapters emit their honest NOT-RUN-LOCALLY row.
step "materialize-same-box.sh CANONICAL=1 (univs: $LUBM_UNIVS)"
if [ -n "$VLOG_BIN" ]; then
  export VLOG="$VLOG_BIN" VLOG_VERSION="karmaresearch/vlog@${VLOG_COMMIT:0:12} (source build)"
fi
if [ -n "$NEMO_BIN" ]; then
  export NEMO="$NEMO_BIN" NEMO_VERSION="knowsys/nemo $NEMO_TAG (source build)"
fi
# ONE SCALE PER INVOCATION, egressed the moment its envelope lands. The box
# self-terminates, so a univ=1 envelope that has to wait for the (far longer) univ=100
# run — and then for the whole HDT stage — is LOST if the box dies in between; that is
# the exact silent-loss shape sq-ffaa9 exists to close, and a single whole-list
# invocation could not push anything until every scale was done. Otherwise equivalent:
# MAT_ITERS is index-aligned with LUBM_UNIVS in materialize-same-box.sh too, and its
# Jena tarball + LUBM corpus are cached across invocations. It also stops a `set -e`
# abort inside an early scale from taking the later scales down with it.
read -r -a MAT_UNIVS_ARR <<< "$LUBM_UNIVS"
read -r -a MAT_ITERS_ARR <<< "$MAT_ITERS"
MAT_FAILED=0
for mi in "${!MAT_UNIVS_ARR[@]}"; do
  mu="${MAT_UNIVS_ARR[$mi]}"
  mit="${MAT_ITERS_ARR[$mi]:-1}"
  step "materialize-same-box.sh CANONICAL=1 univ=$mu iters=$mit"
  CANONICAL=1 LUBM_UNIVS="$mu" MAT_ITERS="$mit" TIMEOUT_S="$TIMEOUT_S" \
    JAVA_XMX="$JAVA_XMX" OUT_DIR="$OUT" \
    bash scripts/bench/materialize-same-box.sh \
    && step "materialize univ=$mu done (oracle held)" \
    || { MAT_FAILED=$((MAT_FAILED + 1)); step "WARN: materialize-same-box.sh exited non-zero for univ=$mu (a pinned univ=1 closure oracle diverged, or the scale failed) — envelopes on disk are still honest per-row records; inspect before citing"; }
  # Durable egress AS SOON AS this scale's envelope exists (no-op unless configured).
  bench_egress_sweep "$OUT"
done
[ "$MAT_FAILED" -eq 0 ] \
  && step "materialize gather done (${#MAT_UNIVS_ARR[@]} scales, oracle held)" \
  || step "WARN: $MAT_FAILED of ${#MAT_UNIVS_ARR[@]} materialize scales exited non-zero"

# ---- 5. the HDT decode gather (same box, afterwards — sq-hmd7l.33) --------------------
if [ "$HDT" = "1" ]; then
  # [SONNET-4.6] sq-45zbg — build a scale-representative archive with the same
  # hdt-cpp image used for decode. gen.sh is deterministic for seed 0; combining
  # ABox + TBox gives the gather a self-contained LUBM corpus.
  step "generate LUBM($HDT_LUBM_UNIV) HDT archive"
  HDT_INPUT_DIR="/tmp/hdt-lubm${HDT_LUBM_UNIV}"
  mkdir -p "$HDT_INPUT_DIR"
  mapfile -t HDT_ARTS < <("$ROOT/bench/lubm/gen.sh" "$HDT_LUBM_UNIV" 0)
  HDT_NT="$HDT_INPUT_DIR/lubm${HDT_LUBM_UNIV}.nt"
  HDT_ARCHIVE="$HDT_INPUT_DIR/lubm${HDT_LUBM_UNIV}.hdt"
  if [ "${#HDT_ARTS[@]}" -ge 2 ] \
     && cat "${HDT_ARTS[0]}" "${HDT_ARTS[1]}" > "$HDT_NT" \
     && docker run --rm -v "$HDT_INPUT_DIR:/data" rdfhdt/hdt-cpp:latest \
          rdf2hdt "/data/$(basename "$HDT_NT")" "/data/$(basename "$HDT_ARCHIVE")"; then
    step "build sparq-hdt bench_oracle"
    cargo build --release -p sparq-hdt --example bench_oracle || true
  else
    step "WARN: LUBM-to-HDT generation failed — hdt gather skipped"
  fi
  if [ -x "$ROOT/target/release/examples/bench_oracle" ] && [ -f "$HDT_ARCHIVE" ]; then
    step "hdt-same-box.sh CANONICAL=1 archive=$HDT_ARCHIVE"
    CANONICAL=1 OUT_DIR="$OUT" HDT_ARCHIVE="$HDT_ARCHIVE" \
      bash scripts/bench/hdt-same-box.sh \
      && step "hdt gather done" || step "WARN: hdt-same-box.sh exited non-zero"
    bench_egress_sweep "$OUT"   # the hdt envelope, durable before the sentinel
  else
    step "WARN: bench_oracle or generated HDT archive absent — hdt gather skipped"
  fi
fi

# ---- 6. egress RETRY sweep + console-output backstop + sentinel -----------------------
# Every envelope was already pushed as its stage finished; this is the retry pass for any
# upload that failed then (bench_egress_sweep skips what already landed), never the first
# attempt — a box that dies before reaching here has its completed scales in S3 already.
step "envelopes in $OUT:"
ls -la "$OUT" >&2 || true
bench_egress_sweep "$OUT"
for f in "$OUT"/*.json; do
  [ -f "$f" ] || continue
  echo "===ENVELOPE-BEGIN $(basename "$f")==="
  cat "$f"
  echo "===ENVELOPE-END==="
done
df -h / >&2
step "gather complete"
touch /root/GATHER_DONE
