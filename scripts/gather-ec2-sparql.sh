#!/usr/bin/env bash
# [OPUS-4.8] sq-ays7 / sq-t0c3 — orphan-proof EC2 orchestrator for the same-box SPARQL
# competitor gather (PHASE 2). Sibling of scripts/gather-ec2.sh (the SHACL gather): it
# reuses the SAME orphan-proofing + SSH-pull-while-alive + explicit-terminate design, but
# runs a SPARQL same-box comparison on a featured corpus instead of the SHACL suite.
#
# WHAT IT MEASURES (HONEST same-box, single box, ONE generated corpus):
#   1. sparq    : `sparq-cli bench <corpus> turtle bench/sp2b/queries <iters> count`
#                 (the EXACT runner CI uses for the SP2Bench featured suite).
#   2. oxigraph : by DEFAULT the PREBUILT, SHA-PINNED Oxigraph CLI (sq-sxso) — `oxigraph
#                 load` into an on-disk store, then `oxigraph query` per `.rq` (min-of-N,
#                 JSON-results count) — NO from-source compile. Set OXI_EMBEDDED=1 to use
#                 the in-process embedded path instead: `sparq-bench bench-corpus <corpus>
#                 turtle bench/sp2b/queries <iters>` (embedded dev-dep 0.5.x from Cargo.lock).
#   Both engines load the SAME generated SP2Bench corpus and run the SAME query files, so
#   the two TSVs (<name>\t<rows>\t<best_us>) are a self-consistent same-box comparison.
#   Solution counts are cross-checked against bench/sp2b/expected-rows.tsv (correctness).
#   The envelope records oxigraph_mode = prebuilt-cli (on-disk RocksDB store) | embedded
#   (in-RAM oxigraph::store::Store) so the load regime is never silently conflated.
#   3. qlever   : BEST-EFFORT (GATHER_QLEVER=1). Delegates to scripts/qlever-same-box.sh
#                 (sq-52fo): builds a QLever index over the same corpus in Docker
#                 (IndexBuilderMain), starts ServerMain on a port, polls readiness, runs the
#                 same queries over HTTP, then ALWAYS tears the server + temp index down via
#                 an EXIT trap. EVERY step is timeout-bounded (pull/index/ready/query) so it
#                 can never hang the gather (the prior ~53min stall). If Docker/index/server
#                 is too heavy/flaky in the bounded window it is SKIPPED and stays honest-n/a.
#
# [OPUS-4.8] sq-sxso — DIAGNOSE-AND-AVOID-THE-STALL HARDENING. The same-box Oxigraph half
# previously had NO per-step timeout and NO timestamped sentinel, so a hung step (a heavy
# from-source `cargo build`, or a pathological SP2Bench query in Oxigraph) burned the whole
# window with NO sentinel — the observed "73 min, no GATHER_DONE" failure, the same class of
# stall the QLever recipe already fixed. THREE measured-rooted fixes (LOCALLY REPRODUCED — at
# only 50k triples the prebuilt Oxigraph CLI ran q07 in ~38 s and q08 PAST 60 s, so at the old
# 250k default those queries are minutes-long → the stall):
#   (a) PER-STEP TIMESTAMPED LOGGING — every phase (apt/rustup/build/gen/sparq-run/oxi-run)
#       emits `step "..."` -> a UTC-stamped line to /var/log/gather.log AND appends to
#       /root/GATHER_STEP. The orchestrator pulls /root/GATHER_STEP when no GATHER_DONE
#       appears, so a stalled run names EXACTLY the hung step instead of being invisible.
#   (b) PREBUILT, SHA-PINNED OXIGRAPH CLI by default — removes the from-source `cargo build
#       -p sparq-bench` Oxigraph compile from the critical path (a likely build-time hang on
#       the small fallback box). The binary is pinned by version + sha256 (OXI_* below).
#   (c) HARD PER-STEP `timeout` + SMALLER configurable dataset — each phase is wrapped in a
#       `timeout` (caps below) and the per-query Oxigraph loop is per-query timeout-bounded,
#       so a hang fails FAST with the logged step instead of stalling for an hour. The default
#       corpus is SMALLER (SP2B_TRIPLES default 100000, was 250000) to keep the smoke/gather
#       cheap; raise SP2B_TRIPLES=250000 for the full per-commit scale.
#
# HONESTY (sq-sxso): this is the SCRIPT-side diagnostic + avoidance hardening. It is authored
# + locally reproduced on the aarch64 work box (the prebuilt-CLI load/query lane was run end
# to end), but the actual confirmation that a real gather now reaches GATHER_DONE without
# stalling REQUIRES one EC2 gather run — that green-gather confirmation is a follow-up. The
# instance-side stall itself cannot be reproduced in a worktree. Competitor numbers from this
# gather are NON-CANONICAL (work-box / ephemeral EC2), per bench/CATALOG.md QUIET-BOX.
#
# Corpus scale is BOUNDED + documented: SP2Bench is the cheapest deterministic featured
# corpus (real Freiburg sp2b_gen, g++ -O2, sha256-pinned, byte-identical output; ~250k
# triples ≈ 26 MB Turtle). SP2B_TRIPLES caps it (default 100000 smoke scale; 250000 = the
# full CI per-commit scale).
#
# ORPHAN-PROOFING (agent-death-independent, 3h hard cap; identical to gather-ec2.sh):
#   --instance-initiated-shutdown-behavior terminate + detached (sleep 10800; shutdown)
#   + systemd-run --on-active=10800 shutdown. The gather writes results + /root/GATHER_DONE
#   (the DONE sentinel, written ONLY after BOTH the sparq AND oxigraph TSVs + the combined
#   envelope are on disk), then STOPS (never shuts down) — only the watchdog auto-terminates
#   if the orchestrator dies. The orchestrator SSH-polls for the sentinel while the box is
#   ALIVE, pulls results, then terminates it. Teardown is SENTINEL-GATED, never fired on a
#   fixed iteration bound that could race a still-running Oxigraph run (see sq-ays7 fix below);
#   the launcher poll deadline sits BELOW the 3h watchdog so the watchdog stays the backstop.
#
# SAFETY: operates ONLY on the ONE instance it launches; NEVER touches prod
# (i-090531b4ede8f2d3f) or the work box. Tag purpose=sparq-bench.
#
# Usage:  AWS_PROFILE=pss scripts/gather-ec2-sparql.sh <branch> [<region>]
#   GATHER_QLEVER=1   also attempt the QLever best-effort path (default off)
#   SP2B_TRIPLES=N    corpus scale (default 100000 smoke; 250000 = full CI scale)
#   GATHER_ITERS=K    min-of-K per query (default 5)
#   OXI_EMBEDDED=1    use the in-process embedded `sparq-bench bench-corpus` Oxigraph path
#                     (in-RAM store) instead of the default prebuilt SHA-pinned CLI (on-disk)
#   OXI_VERSION=...   prebuilt Oxigraph CLI release tag (default below); pinned by sha256
#   STEP_*_TIMEOUT    per-phase hard caps (seconds) — see the user-data step() wrappers below
# Writes decoded envelopes under bench/competitor-results/ and prints each on stdout.
# ALWAYS terminates the instance + deletes the keypair/SG (cleanup trap).
set -euo pipefail

BRANCH="${1:?usage: gather-ec2-sparql.sh <branch> [region]}"
REGION="${2:-${AWS_REGION:-eu-west-2}}"
ITYPE="${GATHER_ITYPE:-c7g.2xlarge}"     # 8 vCPU arm64 (more RAM for the QLever index; falls back below)
# [FABLE-5] sq-7d3dj.30.6 — arch-parametric AMI + fallback so the CANONICAL c6i.4xlarge (x86_64)
# re-measure can pin the SAME architecture as the 2026-07-07 §0 baseline (mixing arm64 vs x86_64
# would make the per-query rows non-comparable). Defaults are the historical arm64 values, so an
# unset environment reproduces the prior behaviour byte-for-byte. For the canonical x86_64 run:
#   GATHER_ITYPE=c6i.4xlarge GATHER_ITYPE_FB=c6i.2xlarge \
#   GATHER_AMI_NAME='ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-amd64-server-*'
ITYPE_FB="${GATHER_ITYPE_FB:-c7g.xlarge}"
AMI_NAME="${GATHER_AMI_NAME:-ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-arm64-server-*}"
REPO="https://github.com/sparq-org/sparq.git"
SP2B_TRIPLES="${SP2B_TRIPLES:-100000}"   # [OPUS-4.8] sq-sxso: smaller smoke default (was 250000)
ITERS="${GATHER_ITERS:-5}"
GATHER_QLEVER="${GATHER_QLEVER:-0}"
OXI_EMBEDDED="${OXI_EMBEDDED:-0}"        # [OPUS-4.8] sq-sxso: 0 = prebuilt CLI (default); 1 = embedded sparq-bench

# [OPUS-4.8] sq-sxso — PREBUILT, SHA-PINNED Oxigraph CLI (avoids the from-source compile).
# Pin: oxigraph CLI v0.5.9 standalone release binaries (github.com/oxigraph/oxigraph/releases).
# The gather box is arm64 (Graviton / ubuntu-noble-24.04-arm64); the x86_64 sha is pinned too
# so the lane is portable + locally reproducible. Re-pin only deliberately: bump OXI_VERSION
# AND both shas together. Verified 2026-06-19.
OXI_VERSION="${OXI_VERSION:-v0.5.9}"
OXI_SHA256_AARCH64="${OXI_SHA256_AARCH64:-dc17e58cf65d74f89853bb8b49180099399a53b9df7f1671a4a9ca188208d794}"
OXI_SHA256_X86_64="${OXI_SHA256_X86_64:-4be355715ba3945e8fb8c94a06662a29808683bf2ec355894dba9e82762e7cc7}"

# [OPUS-4.8] sq-sxso — per-phase hard-timeout caps (seconds). A hung step now fails FAST with
# the step logged, instead of burning the whole window with no sentinel (the 73-min stall).
STEP_APT_TIMEOUT="${STEP_APT_TIMEOUT:-900}"        # apt-get update+install
STEP_RUSTUP_TIMEOUT="${STEP_RUSTUP_TIMEOUT:-600}"  # rustup install
STEP_BUILD_TIMEOUT="${STEP_BUILD_TIMEOUT:-2400}"   # cargo build (sparq-cli [+ sparq-bench if embedded])
STEP_GEN_TIMEOUT="${STEP_GEN_TIMEOUT:-600}"        # sp2b corpus gen (fetch+build gen + emit)
STEP_OXI_FETCH_TIMEOUT="${STEP_OXI_FETCH_TIMEOUT:-300}"  # prebuilt Oxigraph CLI download
STEP_SPARQ_RUN_TIMEOUT="${STEP_SPARQ_RUN_TIMEOUT:-1200}" # sparq-cli bench (all queries)
STEP_OXI_LOAD_TIMEOUT="${STEP_OXI_LOAD_TIMEOUT:-1200}"   # oxigraph load (whole corpus)
STEP_OXI_RUN_TIMEOUT="${STEP_OXI_RUN_TIMEOUT:-1800}"     # oxigraph query loop (all queries, outer cap)
STEP_OXI_QUERY_TIMEOUT="${STEP_OXI_QUERY_TIMEOUT:-120}"  # per-query inner cap (a slow query -> ERROR, not a hang)

TAGSPEC='ResourceType=instance,Tags=[{Key=purpose,Value=sparq-bench}]'
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RESULTS_DIR="$ROOT/bench/competitor-results"

log() { printf '[gather-ec2-sparql] %s\n' "$*" >&2; }
die() { printf '[gather-ec2-sparql] ERROR: %s\n' "$*" >&2; exit 1; }

WORK="$(mktemp -d)"
KEYFILE="$WORK/key"
KEY_NAME="sparq-bench-sparql-$$-${RANDOM}"
INSTANCE_ID=""; SG_ID=""
ssh-keygen -t ed25519 -N '' -f "$KEYFILE" -q
SSHO="-i $KEYFILE -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=15"

cleanup() {
  set +e
  if [ -n "$INSTANCE_ID" ]; then
    log "cleanup: terminating $INSTANCE_ID"
    aws ec2 terminate-instances --region "$REGION" --instance-ids "$INSTANCE_ID" >/dev/null 2>&1
    aws ec2 wait instance-terminated --region "$REGION" --instance-ids "$INSTANCE_ID" >/dev/null 2>&1
    log "cleanup: $INSTANCE_ID terminated"
  fi
  [ -n "$SG_ID" ] && aws ec2 delete-security-group --region "$REGION" --group-id "$SG_ID" >/dev/null 2>&1
  aws ec2 delete-key-pair --region "$REGION" --key-name "$KEY_NAME" >/dev/null 2>&1
  rm -rf "$WORK"
}
trap cleanup EXIT

log "resolve AMI / network in $REGION"
AMI=$(aws ec2 describe-images --region "$REGION" --owners 099720109477 \
  --filters "Name=name,Values=$AMI_NAME" "Name=state,Values=available" \
  --query 'sort_by(Images,&CreationDate)[-1].ImageId' --output text)
VPC=$(aws ec2 describe-vpcs --region "$REGION" --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)
SUBNET=$(aws ec2 describe-subnets --region "$REGION" --filters Name=vpc-id,Values="$VPC" "Name=default-for-az,Values=true" --query 'Subnets[0].SubnetId' --output text)
MYIP=$(curl -s https://checkip.amazonaws.com | tr -d '[:space:]')
[ -n "$MYIP" ] || die "could not determine public IP (checkip returned empty)"
log "AMI=$AMI VPC=$VPC SUBNET=$SUBNET MYIP=$MYIP ITYPE=$ITYPE QLEVER=$GATHER_QLEVER SP2B_TRIPLES=$SP2B_TRIPLES"

log "keypair + locked-down security group (ssh from $MYIP/32 only)"
aws ec2 import-key-pair --region "$REGION" --key-name "$KEY_NAME" --public-key-material "fileb://${KEYFILE}.pub" >/dev/null
SG_ID=$(aws ec2 create-security-group --region "$REGION" --group-name "$KEY_NAME" --description "sparq sparql bench gather (ephemeral)" --vpc-id "$VPC" --query 'GroupId' --output text)
aws ec2 authorize-security-group-ingress --region "$REGION" --group-id "$SG_ID" --protocol tcp --port 22 --cidr "${MYIP}/32" >/dev/null

# ---- user-data: build, gen corpus, run sparq + oxigraph (+ qlever best-effort), write
# results + sentinel, then STOP (watchdog is the only auto-terminate). ---------------------
USERDATA=$(cat <<UD
#!/bin/bash
set -x
exec > >(tee /var/log/gather.log) 2>&1
( sleep 10800; shutdown -h now ) &
systemd-run --on-active=10800 /sbin/shutdown -h now || true

# [OPUS-4.8] sq-sxso — PER-STEP TIMESTAMPED SENTINEL. Every phase calls step() FIRST, which
# UTC-stamps a line into the tee'd /var/log/gather.log AND appends it to /root/GATHER_STEP. A
# stalled run no longer hangs invisibly — the orchestrator pulls /root/GATHER_STEP and the
# LAST line is EXACTLY the step that hung (the "73 min, no sentinel" failure was just an
# absence of this). run_step() wraps a command in 'timeout' so a hung step fails FAST (exit
# 124) with the step name already logged, instead of burning the whole window.
#   step() writes to STDERR (the top-level 'exec ... 2>&1' still folds it into the gather log)
#   AND appends to /root/GATHER_STEP. Writing to stderr — not stdout — is load-bearing: several
#   run_step callers redirect the wrapped command's STDOUT to a TSV, and a step line on stdout
#   would corrupt that TSV. 'tee ... >&2' sends tee's own copy to fd 2.
step() { echo "[STEP \$(date -u +%Y-%m-%dT%H:%M:%SZ)] \$*" | tee -a /root/GATHER_STEP >&2; }
run_step() { # run_step <name> <timeout-s> -- <cmd...>
  local name="\$1" to="\$2"; shift 3   # drop name, timeout, and the literal '--'
  step "BEGIN \$name (timeout \${to}s): \$*"
  if timeout "\$to" "\$@"; then
    step "OK    \$name"
  else
    local rc=\$?
    [ "\$rc" = 124 ] && step "TIMEOUT \$name hit \${to}s cap (rc=124)" || step "FAIL  \$name (rc=\$rc)"
    return "\$rc"
  fi
}

export DEBIAN_FRONTEND=noninteractive
step "apt update+install"
run_step apt $STEP_APT_TIMEOUT -- bash -c 'apt-get update -qq && apt-get install -y -qq build-essential g++ pkg-config git curl jq python3 python3-venv python3-pip unzip' || true
if [ "$GATHER_QLEVER" = "1" ]; then
  step "apt docker.io (qlever opt-in)"
  apt-get install -y -qq docker.io || true
  # [OPUS-4.8] sq-vw3ax.12.1 — WAIT for the daemon (fixes the Wave-0 qlever fast-fail). Wave 0
  # did 'systemctl start docker || true' and moved on, so when the socket was not yet ready the
  # qlever recipe hit an instant "Cannot connect to the Docker daemon" cascade (~9s, recorded
  # only as qlever_status:"failed"). enable --now + a bounded 'docker info' poll makes the daemon
  # actually be up before scripts/qlever-same-box.sh runs (which itself now preflights the daemon).
  # [FABLE-5] sq-7d3dj.30.6 — the backticks in these two comment lines were UNESCAPED inside the
  # UNQUOTED '<<UD' heredoc, so on any launch host that has a 'docker' binary they were command-
  # substituted AT RENDER TIME: the local 'docker info' printed docker's help text with an
  # unbalanced paren straight into the user-data, syntax-erroring the instance-side script at boot
  # (load=0, apt done, nothing else runs — the exact silent-death this bead's predecessor hit).
  # Fixed by using plain quotes in the prose; keep backticks out of unquoted-heredoc comment text.
  systemctl enable --now docker || systemctl start docker || true
  for _ in \$(seq 1 30); do docker info >/dev/null 2>&1 && { step "docker daemon up"; break; }; sleep 2; done
  docker info >/dev/null 2>&1 || step "WARN: docker daemon still not reachable — qlever will stay honest-n/a"
fi
step "rustup install"
run_step rustup $STEP_RUSTUP_TIMEOUT -- bash -c "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal" || true
export PATH="/root/.cargo/bin:\$PATH"
. /root/.cargo/env || true
for _ in \$(seq 1 30); do command -v cargo >/dev/null 2>&1 && break; sleep 2; done
command -v cargo || step "WARN: cargo still not on PATH"

step "git clone + checkout $BRANCH"
cd /root
git clone -q "$REPO" sparq
cd sparq
git fetch -q origin "$BRANCH"
git checkout -q "$BRANCH"
SHA=\$(git rev-parse --short HEAD)
step "checked out \$SHA"

# [OPUS-4.8] sq-sxso: ALWAYS build sparq-cli (the sparq engine under test). Only build the
# embedded sparq-bench (the heavy from-source Oxigraph compile) when OXI_EMBEDDED=1 — the
# default prebuilt-CLI path does NOT need it, which removes the most likely build-time hang
# from the critical path.
run_step build-sparq-cli $STEP_BUILD_TIMEOUT -- cargo build --release -q -p sparq-cli \
  || step "WARN: sparq-cli build failed/timed out"
if [ "$OXI_EMBEDDED" = "1" ]; then
  run_step build-sparq-bench $STEP_BUILD_TIMEOUT -- cargo build --release -q -p sparq-bench \
    || step "WARN: sparq-bench (embedded oxigraph) build failed/timed out"
fi

# ---- generate the bounded SP2Bench corpus (real Freiburg sp2b_gen) -----------------------
step "sp2b gen ($SP2B_TRIPLES triples)"
# Redirect lives INSIDE the wrapped command (bash -c '... > file'), so run_step's own step
# markers still reach the log instead of the redirected file. $SP2B_TRIPLES is baked by the
# launcher; the inner stdout (the corpus path gen.sh prints) goes to /tmp/corpus_path.txt.
run_step sp2b-gen $STEP_GEN_TIMEOUT -- bash -c "bash bench/sp2b/gen.sh $SP2B_TRIPLES > /tmp/corpus_path.txt 2>/tmp/gen.err" \
  || step "WARN: sp2b gen failed/timed out (see /tmp/gen.err)"
CORPUS=\$(tail -n1 /tmp/corpus_path.txt 2>/dev/null || true)
step "corpus=\$CORPUS"
[ -n "\$CORPUS" ] && ls -la "\$CORPUS" || step "WARN: no corpus path"

OXI_V=\$(awk '/^name = "oxigraph"\$/{f=1} f&&/^version = /{gsub(/[",]/,"",\$3);print \$3;exit}' Cargo.lock 2>/dev/null || echo unknown)
step "oxigraph (cargo.lock dev-dep) version=\$OXI_V"

export QUIET_BOX=true
mkdir -p bench/competitor-results

# ---- sparq (the CI runner, count mode) ---------------------------------------------------
step "sparq-cli bench (count, iters=$ITERS)"
run_step sparq-run $STEP_SPARQ_RUN_TIMEOUT -- bash -c "./target/release/sparq-cli bench \"\$CORPUS\" turtle bench/sp2b/queries $ITERS count > /tmp/sparq.tsv 2>/tmp/sparq.err" \
  || step "WARN: sparq run failed/timed out (see /tmp/sparq.err)"
echo "=== sparq.tsv ==="; cat /tmp/sparq.tsv 2>/dev/null || true

# ---- oxigraph (same corpus, same queries, same count semantics) --------------------------
# [OPUS-4.8] sq-sxso: DEFAULT = prebuilt SHA-pinned Oxigraph CLI (on-disk store); no compile.
# OXI_EMBEDDED=1 = the in-process embedded path (in-RAM store). Either way the per-query loop
# is timeout-bounded so a pathological SP2Bench query (q07/q08 are minutes-long in Oxigraph at
# scale — measured) records ERROR and the gather moves on, instead of hanging the whole run.
OXI_MODE="embedded"
if [ "$OXI_EMBEDDED" = "1" ]; then
  step "oxigraph EMBEDDED (sparq-bench bench-corpus, in-RAM, iters=$ITERS)"
  run_step oxi-run $STEP_OXI_RUN_TIMEOUT -- bash -c "./target/release/sparq-bench bench-corpus \"\$CORPUS\" turtle bench/sp2b/queries $ITERS > /tmp/oxi.tsv 2>/tmp/oxi.err" \
    || step "WARN: embedded oxigraph run failed/timed out (see /tmp/oxi.err)"
else
  OXI_MODE="prebuilt-cli"
  # ---- (1) fetch + sha256-verify the prebuilt Oxigraph CLI (arch-selected) ----
  ARCH=\$(uname -m)
  case "\$ARCH" in
    aarch64|arm64) OXI_ASSET="oxigraph_${OXI_VERSION}_aarch64_linux_gnu"; OXI_SHA="$OXI_SHA256_AARCH64" ;;
    x86_64|amd64)  OXI_ASSET="oxigraph_${OXI_VERSION}_x86_64_linux_gnu";  OXI_SHA="$OXI_SHA256_X86_64" ;;
    *) step "FATAL: unsupported arch \$ARCH for prebuilt Oxigraph CLI"; OXI_ASSET=""; OXI_SHA="" ;;
  esac
  OXI_URL="https://github.com/oxigraph/oxigraph/releases/download/${OXI_VERSION}/\$OXI_ASSET"
  OXI_BIN=/usr/local/bin/oxigraph
  if [ -n "\$OXI_ASSET" ]; then
    step "oxigraph PREBUILT CLI fetch ${OXI_VERSION} (\$OXI_ASSET)"
    if run_step oxi-fetch $STEP_OXI_FETCH_TIMEOUT -- curl -fsSL -o "\$OXI_BIN" "\$OXI_URL"; then
      ACT=\$(sha256sum "\$OXI_BIN" | cut -d' ' -f1)
      if [ "\$ACT" = "\$OXI_SHA" ]; then
        chmod +x "\$OXI_BIN"
        step "oxigraph CLI sha256 OK (\$ACT); version: \$("\$OXI_BIN" --version 2>/dev/null || echo unknown)"
      else
        step "FATAL: oxigraph CLI sha256 MISMATCH expected=\$OXI_SHA actual=\$ACT — refusing to run unpinned binary"
        rm -f "\$OXI_BIN"; OXI_BIN=""
      fi
    else
      step "WARN: oxigraph CLI download failed/timed out"; OXI_BIN=""
    fi
  else
    OXI_BIN=""
  fi
  # ---- (2) load the corpus into an on-disk store (bounded) ----
  : > /tmp/oxi.tsv
  if [ -n "\$OXI_BIN" ] && [ -n "\$CORPUS" ]; then
    OXI_STORE=\$(mktemp -d /tmp/oxi-store.XXXXXX)
    OXI_V="\$("\$OXI_BIN" --version 2>/dev/null | awk '{print \$2}' || echo "$OXI_VERSION")"
    step "oxigraph PREBUILT load (on-disk, \$OXI_STORE)"
    if run_step oxi-load $STEP_OXI_LOAD_TIMEOUT -- "\$OXI_BIN" load --location "\$OXI_STORE" --file "\$CORPUS" --format ttl; then
      # ---- (3) per-query loop, EACH query inner-timeout-bounded; min-of-N JSON-results count ----
      step "oxigraph PREBUILT query loop (iters=$ITERS, per-query cap \${STEP_OXI_QUERY_TIMEOUT}s)"
      OXI_BIN="\$OXI_BIN" OXI_STORE="\$OXI_STORE" ITERS="$ITERS" QTO="$STEP_OXI_QUERY_TIMEOUT" QDIR="bench/sp2b/queries" \
        timeout $STEP_OXI_RUN_TIMEOUT python3 - <<'PYOXI' > /tmp/oxi.tsv 2>/tmp/oxi.err || step "WARN: prebuilt oxigraph query loop failed/timed out"
import os, sys, glob, json, time, subprocess
oxi   = os.environ["OXI_BIN"]; store = os.environ["OXI_STORE"]
iters = int(os.environ["ITERS"]); qto = float(os.environ["QTO"]); qdir = os.environ["QDIR"]
def count_json(text):
    # SELECT -> len(results.bindings); ASK -> 1 if true else 0 (matches the embedded oxi_count:
    # Boolean -> 1; here we count the boolean's solution as 1 only when true, 0 when false, so
    # the row count equals the number of solutions, like sparq count mode on these ASK queries).
    obj = json.loads(text)
    if "boolean" in obj: return 1 if obj["boolean"] else 0
    res = obj.get("results", {})
    return len(res.get("bindings", []))
for qf in sorted(glob.glob(os.path.join(qdir, "*.rq"))):
    name = os.path.splitext(os.path.basename(qf))[0]
    best = None; rows = "ERROR"
    for _ in range(max(1, iters)):
        t = time.perf_counter()
        try:
            out = subprocess.run([oxi, "query", "--location", store, "--query-file", qf,
                                  "--results-format", "json"],
                                 capture_output=True, text=True, timeout=qto)
        except subprocess.TimeoutExpired:
            rows = "ERROR"; best = None; break
        if out.returncode != 0:
            rows = "ERROR"; best = None; break
        try:
            rows = count_json(out.stdout)
        except Exception:
            rows = "ERROR"; best = None; break
        us = (time.perf_counter() - t) * 1e6
        best = us if best is None else min(best, us)
    if rows == "ERROR" or best is None:
        print(f"{name}\tERROR\toxigraph")
    else:
        print(f"{name}\t{rows}\t{best:.1f}")
    sys.stdout.flush()
PYOXI
    else
      step "WARN: oxigraph load failed/timed out — leaving oxi.tsv empty (honest-n/a)"
    fi
    rm -rf "\$OXI_STORE"
  else
    step "WARN: no prebuilt oxigraph binary or no corpus — oxi.tsv stays empty (honest-n/a)"
  fi
fi
echo "=== oxi.tsv (mode=\$OXI_MODE) ==="; cat /tmp/oxi.tsv 2>/dev/null || true; echo "--- oxi.err ---"; cat /tmp/oxi.err 2>/dev/null || true

# ---- env block + combine into ONE same-box-comparison envelope ---------------------------
CPU=\$(LC_ALL=C grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2- | sed 's/^ *//' || true)
[ -n "\$CPU" ] || CPU=\$(uname -p)
NPROC=\$(nproc)
KERNEL=\$(uname -r)
NOW=\$(date -u +%Y-%m-%dT%H:%M:%SZ)

# qlever best-effort  [OPUS-4.8] sq-52fo
# Delegates the heavy index->server->query->teardown dance to the dedicated, BOUNDED,
# orphan-safe recipe scripts/qlever-same-box.sh (every step timeout-capped, an EXIT trap
# always tears down the server container + temp index — fixes the prior ~53min hang / leaked
# server). This block stays a no-op unless GATHER_QLEVER=1 AND docker is present, so the
# Oxigraph-only path is byte-for-byte unchanged.
QLEVER_STATUS="skipped"
QLEVER_TSV=""
if [ "$GATHER_QLEVER" = "1" ] && command -v docker >/dev/null 2>&1; then
  step "qlever best-effort (scripts/qlever-same-box.sh)"
  # sp2b_gen emits Turtle, so format=ttl; the recipe also accepts nt for N-Triples corpora.
  # The recipe is itself bounded + trap-cleaned, but we ALSO wrap it in an outer 'timeout'
  # as a belt-and-braces ceiling so it can never extend the gather window indefinitely.
  if QLEVER_PORT=7001 QLEVER_INDEX_TIMEOUT=1200 QLEVER_READY_TIMEOUT=120 QLEVER_QUERY_TIMEOUT=60 \
       timeout 1800 bash scripts/qlever-same-box.sh "\$CORPUS" ttl bench/sp2b/queries "$ITERS" /tmp/qlever.tsv; then
    QLEVER_STATUS="ok"
    QLEVER_TSV=\$(python3 -c 'import json,sys; print(json.dumps(open("/tmp/qlever.tsv").read()))')
    step "qlever OK"
  else
    step "qlever recipe failed/timed out — keeping dashboard cell honest-n/a"
    QLEVER_STATUS="failed"
    [ -s /tmp/qlever.tsv ] && QLEVER_TSV=\$(python3 -c 'import json,sys; print(json.dumps(open("/tmp/qlever.tsv").read()))')
  fi
fi

# [OPUS-4.8] sq-sxso: read the TSVs DEFENSIVELY — a timed-out step leaves an empty/missing
# file, and the envelope must still be written (so /root/GATHER_DONE is reached and the
# orchestrator does not itself hang waiting for a sentinel that never comes). A missing file
# becomes "" rather than aborting the heredoc under set -euo pipefail.
SPARQ_TSV=\$(python3 -c 'import json; print(json.dumps(open("/tmp/sparq.tsv").read() if __import__("os").path.exists("/tmp/sparq.tsv") else ""))')
OXI_TSV=\$(python3 -c 'import json; print(json.dumps(open("/tmp/oxi.tsv").read() if __import__("os").path.exists("/tmp/oxi.tsv") else ""))')
[ -n "\$QLEVER_TSV" ] || QLEVER_TSV='""'
GATHER_STEPS=\$(python3 -c 'import json; print(json.dumps(open("/root/GATHER_STEP").read() if __import__("os").path.exists("/root/GATHER_STEP") else ""))')

step "writing same-box envelope (oxigraph_mode=\$OXI_MODE)"
cat > bench/competitor-results/sparql-same-box-\$(date -u +%Y%m%dT%H%M%SZ).json <<EOF2
{
  "gather": "sparql-same-box-comparison",
  "git_commit": "\$SHA",
  "suite": "sp2b",
  "scale": "SP2Bench $SP2B_TRIPLES triples (real Freiburg sp2b_gen, deterministic)",
  "iters": $ITERS,
  "oxigraph_mode": "\$OXI_MODE",
  "oxigraph_version": "\$OXI_V",
  "env": {"host_class":"ephemeral-ec2-$ITYPE","cpu_model":"\$CPU","nproc":\$NPROC,"os":"Linux","kernel":"\$KERNEL","quiet_box":true,"gathered_at_utc":"\$NOW","git_commit":"\$SHA"},
  "qlever_status": "\$QLEVER_STATUS",
  "sparq_tsv": \$SPARQ_TSV,
  "oxigraph_tsv": \$OXI_TSV,
  "qlever_tsv": \$QLEVER_TSV,
  "step_log": \$GATHER_STEPS
}
EOF2

jq . bench/competitor-results/sparql-same-box-*.json || step "WARN: envelope did not validate"
ls -la bench/competitor-results/
step "GATHER_DONE"
echo "\$SHA" > /root/GATHER_DONE
sync
UD
)

log "launching $ITYPE"
LAUNCH_ERR="$WORK/launch.err"
INSTANCE_ID=$(aws ec2 run-instances --region "$REGION" --image-id "$AMI" --instance-type "$ITYPE" \
  --instance-initiated-shutdown-behavior terminate \
  --key-name "$KEY_NAME" --security-group-ids "$SG_ID" \
  --subnet-id "$SUBNET" --associate-public-ip-address \
  --block-device-mappings '[{"DeviceName":"/dev/sda1","Ebs":{"VolumeSize":40,"VolumeType":"gp3","DeleteOnTermination":true}}]' \
  --tag-specifications "$TAGSPEC" \
  --user-data "$USERDATA" \
  --query 'Instances[0].InstanceId' --output text 2>"$LAUNCH_ERR") || true
case "$INSTANCE_ID" in
  i-*) ;;
  *)
    log "primary itype $ITYPE failed: $(cat "$LAUNCH_ERR" 2>/dev/null)"
    log "retrying with fallback $ITYPE_FB"
    INSTANCE_ID=$(aws ec2 run-instances --region "$REGION" --image-id "$AMI" --instance-type "$ITYPE_FB" \
      --instance-initiated-shutdown-behavior terminate \
      --key-name "$KEY_NAME" --security-group-ids "$SG_ID" \
      --subnet-id "$SUBNET" --associate-public-ip-address \
      --block-device-mappings '[{"DeviceName":"/dev/sda1","Ebs":{"VolumeSize":40,"VolumeType":"gp3","DeleteOnTermination":true}}]' \
      --tag-specifications "$TAGSPEC" \
      --user-data "$USERDATA" \
      --query 'Instances[0].InstanceId' --output text 2>"$LAUNCH_ERR") || true
    case "$INSTANCE_ID" in
      i-*) ITYPE="$ITYPE_FB" ;;
      *) INSTANCE_ID=""; die "run-instances failed on both itypes: $(cat "$LAUNCH_ERR" 2>/dev/null)" ;;
    esac
    ;;
esac
log "launched INSTANCE_ID=$INSTANCE_ID ($ITYPE)"
aws ec2 wait instance-running --region "$REGION" --instance-ids "$INSTANCE_ID"
IP=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)
log "public IP=$IP; waiting for sshd"
SSH_UP=0
for i in $(seq 1 40); do ssh $SSHO "ubuntu@$IP" true 2>/dev/null && { log "ssh up"; SSH_UP=1; break; }; sleep 10; done
[ "$SSH_UP" = 1 ] || die "sshd never became reachable on $IP after 40 attempts — aborting (cleanup trap terminates $INSTANCE_ID)"

mkdir -p "$RESULTS_DIR"
# [OPUS-4.8] sq-ays7 — SENTINEL-GATED teardown (poll-bound bug fix).
# PREVIOUS BUG: the poll loop was a FIXED `for i in $(seq 1 120)` @ sleep 30 = 60 min hard
# bound. On the c7g.xlarge fallback, apt + docker + rustup + `cargo build --release` (two
# crates) + sp2b corpus-gen + the sparq run consumed almost the whole 60 min, so the bound
# expired while `sparq-bench bench-corpus` (Oxigraph) was STILL RUNNING — the cleanup trap
# then terminated the box AFTER sparq.tsv was captured but BEFORE oxi.tsv was written and
# BEFORE /root/GATHER_DONE was created. Result: dashboard showed sparq numbers, Oxigraph n/a.
# FIX: the instance-side gather already writes /root/GATHER_DONE only AFTER BOTH sparq AND
# oxigraph TSVs (+ the combined envelope) are on disk (see user-data, near "GATHER_DONE").
# So here we WAIT for that sentinel and only THEN pull + let the trap terminate. The poll
# budget is no longer a guess at the gather duration: it is a launcher-side ceiling
# (POLL_DEADLINE_S) set safely BELOW the instance's 3h orphan-proof watchdog (10800 s), so a
# ~10-15 min Oxigraph corpus-load+query run finishes with wide margin. The watchdog stays the
# orphan-proof backstop ONLY — normal teardown is sentinel-gated, never bound-gated mid-gather.
POLL_DEADLINE_S="${GATHER_POLL_DEADLINE_S:-9900}"   # 165 min; < 10800 s watchdog (keeps it the backstop)
POLL_INTERVAL_S=30
log "polling for /root/GATHER_DONE (build+gen+gather ~25-50 min; +QLever if enabled; deadline ${POLL_DEADLINE_S}s, below 3h watchdog)…"
DONE=0
i=0
POLL_START=$(date +%s)
while :; do
  ELAPSED=$(( $(date +%s) - POLL_START ))
  if [ "$ELAPSED" -ge "$POLL_DEADLINE_S" ]; then
    log "  poll deadline ${POLL_DEADLINE_S}s reached without sentinel — giving up (cleanup trap terminates)"
    break
  fi
  sleep "$POLL_INTERVAL_S"
  i=$(( i + 1 ))
  if ssh $SSHO "ubuntu@$IP" "sudo test -f /root/GATHER_DONE" 2>/dev/null; then
    log "  [$i / ${ELAPSED}s] sentinel present (both engines' TSVs written) — pulling results"
    DONE=1; break
  fi
  STATE=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" --query 'Reservations[0].Instances[0].State.Name' --output text 2>/dev/null || echo unknown)
  # [OPUS-4.8] sq-sxso: surface the CURRENT step LIVE while polling — the last /root/GATHER_STEP
  # line tells us which phase the box is on, so a stall is visible AS IT HAPPENS (not only after
  # the deadline). This is the diagnostic the prior "73 min, no sentinel" run lacked entirely.
  CUR_STEP=$(ssh $SSHO "ubuntu@$IP" "sudo tail -n1 /root/GATHER_STEP 2>/dev/null" 2>/dev/null || true)
  log "  [$i / ${ELAPSED}s] state=$STATE; step: ${CUR_STEP:-<not started>}"
  [ "$STATE" = "terminated" ] && { log "  instance terminated before sentinel — results may be lost"; break; }
done

if [ "$DONE" = 1 ]; then
  ssh $SSHO "ubuntu@$IP" "sudo tar -C /root/sparq/bench -cf - competitor-results 2>/dev/null" | tar -C "$ROOT/bench" -xf - 2>/dev/null \
    && log "pulled result envelopes:" || log "tar pull failed"
  for f in "$RESULTS_DIR"/sparql-same-box-*.json; do [ -f "$f" ] || continue; cat "$f"; echo; done
else
  # [OPUS-4.8] sq-sxso: on NO sentinel, pull the per-step log FIRST and call out the LAST step
  # (the one that hung) explicitly, THEN the gather.log tail. This is the core diagnosability
  # fix: a future stalled run names the hung phase instead of being invisible.
  log "NO sentinel — pulling /root/GATHER_STEP (the hung step) + /var/log/gather.log for diagnosis"
  STEP_LOG=$(ssh $SSHO "ubuntu@$IP" "sudo cat /root/GATHER_STEP 2>/dev/null" 2>/dev/null || true)
  if [ -n "$STEP_LOG" ]; then
    log "  --- per-step log (/root/GATHER_STEP) ---"
    printf '%s\n' "$STEP_LOG" >&2
    log "  >>> LAST STEP (likely the hung one): $(printf '%s\n' "$STEP_LOG" | tail -n1)"
  else
    log "  (no /root/GATHER_STEP — the box may have stalled before user-data ran)"
  fi
  ssh $SSHO "ubuntu@$IP" "sudo tail -160 /var/log/gather.log 2>/dev/null" >&2 || true
fi

log "done; cleanup trap will terminate $INSTANCE_ID + delete keypair/SG"
