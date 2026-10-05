#!/usr/bin/env bash
# [SONNET-4.6] sq-7d3dj.30.22 — dedicated EC2 launcher for the q08/q12b Virtuoso same-box
# re-measure.  QLever is DISQUALIFIED on these two queries (wrong count 0 vs 358/1 — the
# bnode!=IRI strict-type-error divergence adjudicated in sq-ai2wa).  This script measures
# sparq (current main, with predicate-range id-filter-fastpath from PRs #1785+#1795) vs
# Virtuoso OSS (the only prior CORRECT comparator on q08/q12b) on the SAME quiet box at
# the SAME 250k SP2Bench scale as the 2026-07-07 canonical gather.
#
# HONESTY INVARIANT (per bead description):
#   Count cross-check against bench/sp2b/expected-rows.tsv is MANDATORY.
#   If Virtuoso returns a wrong count it is recorded as WRONG, not as a timing comparator.
#   If Virtuoso fails to run it is recorded as NOT-COMPARABLE, NEVER as a sparq win.
#   The envelope is the sole source of truth; no numbers are hard-coded anywhere else.
#
# EC2 RAILS (matching the bead's mandatory requirements):
#   --profile pss | dedicated quiet c6i.4xlarge | tag Name=sparq-bench
#   --instance-initiated-shutdown-behavior terminate
#   user-data self-terminate watchdog sized to runtime + 50%
#   results via SSH pull; NEVER touches i-090531b4ede8f2d3f / i-00f76802f345b6b77
#   orphan-check after (describe-instances tag sparq-bench)
#
# Usage:
#   AWS_PROFILE=pss bash scripts/gather-ec2-q08-virtuoso.sh [<branch>] [<region>]
#     <branch>  git branch/commit to build  (default: main)
#     <region>  AWS region                  (default: eu-west-2)
#
#   GATHER_ITERS=K   min-of-K per query (default 5, per canonical protocol)
#   SP2B_TRIPLES=N   corpus scale (default 250000, matching 2026-07-07 canonical run)
#
# Output: bench/canonical-competitor-results/<date>/canonical-sp2b-q08-virtuoso-<ts>.json
set -euo pipefail

BRANCH="${1:-main}"
REGION="${2:-${AWS_REGION:-eu-west-2}}"
ITYPE="c6i.4xlarge"   # MANDATORY: same arch+class as 2026-07-07 canonical baseline
ITYPE_FB="c6i.2xlarge"  # fallback (same arch, half the vCPUs)
# x86_64 Ubuntu Noble AMI (matches the canonical c6i.4xlarge box)
AMI_NAME="${GATHER_AMI_NAME:-ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-amd64-server-*}"
REPO="https://github.com/sparq-org/sparq.git"
SP2B_TRIPLES="${SP2B_TRIPLES:-250000}"   # canonical scale
ITERS="${GATHER_ITERS:-5}"              # canonical min-of-5

# [SONNET-4.6] per-step timeout caps — same class as gather-ec2-sparql.sh
STEP_APT_TIMEOUT="${STEP_APT_TIMEOUT:-900}"
STEP_RUSTUP_TIMEOUT="${STEP_RUSTUP_TIMEOUT:-600}"
STEP_BUILD_TIMEOUT="${STEP_BUILD_TIMEOUT:-2400}"
STEP_GEN_TIMEOUT="${STEP_GEN_TIMEOUT:-600}"
STEP_SPARQ_RUN_TIMEOUT="${STEP_SPARQ_RUN_TIMEOUT:-1200}"
STEP_VIRT_TIMEOUT="${STEP_VIRT_TIMEOUT:-3600}"  # Virtuoso: pull+load+query+teardown

# Watchdog budget: apt(900)+rustup(600)+build(2400)+gen(600)+sparq(1200)+virtuoso(3600)
# = ~9300s raw + 50% headroom = ~14000s. Round up to 4h=14400s.
WATCHDOG_S=14400

TAGSPEC='ResourceType=instance,Tags=[{Key=Name,Value=sparq-bench},{Key=purpose,Value=sparq-bench}]'
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RESULTS_DIR="$ROOT/bench/canonical-competitor-results/$(date -u +%Y-%m-%d)"
POLL_DEADLINE_S="${GATHER_POLL_DEADLINE_S:-13200}"   # 220 min; < 14400s watchdog

log() { printf '[gather-ec2-q08-virtuoso] %s\n' "$*" >&2; }
die() { printf '[gather-ec2-q08-virtuoso] ERROR: %s\n' "$*" >&2; exit 1; }

# ---- preflight checks -----------------------------------------------------------------
[ -n "${AWS_PROFILE:-}" ] || export AWS_PROFILE=pss
aws sts get-caller-identity --query Account --output text >/dev/null \
  || die "AWS auth failed (profile=${AWS_PROFILE}); run 'aws --profile pss sts get-caller-identity' first"

# SAFETY: never touch the prod/dev boxes
for FORBIDDEN in i-090531b4ede8f2d3f i-00f76802f345b6b77; do
  if aws ec2 describe-instances --region "$REGION" --instance-ids "$FORBIDDEN" \
       --query 'Reservations[0].Instances[0].State.Name' --output text 2>/dev/null | grep -q "running\|stopped\|pending"; then
    log "WARNING: $FORBIDDEN exists — we will NEVER touch it (safety check passed)"
  fi
done

WORK="$(mktemp -d)"
KEYFILE="$WORK/key"
KEY_NAME="sparq-bench-q08-$$-${RANDOM}"
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

log "resolving AMI / network in $REGION"
AMI=$(aws ec2 describe-images --region "$REGION" --owners 099720109477 \
  --filters "Name=name,Values=$AMI_NAME" "Name=state,Values=available" \
  --query 'sort_by(Images,&CreationDate)[-1].ImageId' --output text)
VPC=$(aws ec2 describe-vpcs --region "$REGION" --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)
SUBNET=$(aws ec2 describe-subnets --region "$REGION" --filters Name=vpc-id,Values="$VPC" "Name=default-for-az,Values=true" --query 'Subnets[0].SubnetId' --output text)
MYIP=$(curl -s https://checkip.amazonaws.com | tr -d '[:space:]')
[ -n "$MYIP" ] || die "could not determine public IP"
log "AMI=$AMI VPC=$VPC SUBNET=$SUBNET MYIP=$MYIP ITYPE=$ITYPE SP2B_TRIPLES=$SP2B_TRIPLES ITERS=$ITERS"

log "keypair + locked-down security group (ssh from $MYIP/32 only)"
aws ec2 import-key-pair --region "$REGION" --key-name "$KEY_NAME" --public-key-material "fileb://${KEYFILE}.pub" >/dev/null
SG_ID=$(aws ec2 create-security-group --region "$REGION" --group-name "$KEY_NAME" \
  --description "sparq q08-virtuoso bench (ephemeral)" --vpc-id "$VPC" --query 'GroupId' --output text)
aws ec2 authorize-security-group-ingress --region "$REGION" --group-id "$SG_ID" \
  --protocol tcp --port 22 --cidr "${MYIP}/32" >/dev/null

# ---- user-data: build sparq, gen corpus, run sparq + virtuoso, write sentinel ---------
# NOTE: heredoc is UNQUOTED (<<UD) so ${...} expansions from the LAUNCHER are injected at
# render time. ALL instance-side shell vars must use \$ to survive the render.
# NO backticks in comment text — they are command-substituted at render time (PR #1827 fix).
USERDATA=$(cat <<UD
#!/bin/bash
set -x
exec > >(tee /var/log/gather.log) 2>&1

# [SONNET-4.6] sq-7d3dj.30.22: instance-side self-terminate watchdog.
# Two mechanisms: background sleep + systemd-run (belt-and-braces).
# --instance-initiated-shutdown-behavior=terminate means shutdown -h = permanent termination.
( sleep ${WATCHDOG_S}; echo "[WATCHDOG] ${WATCHDOG_S}s reached, self-terminating" >> /root/GATHER_STEP; shutdown -h now ) &
systemd-run --on-active=${WATCHDOG_S} /sbin/shutdown -h now || true

step() { echo "[STEP \$(date -u +%Y-%m-%dT%H:%M:%SZ)] \$*" | tee -a /root/GATHER_STEP >&2; }
run_step() {
  local name="\$1" to="\$2"; shift 3
  step "BEGIN \$name (timeout \${to}s): \$*"
  if timeout "\$to" "\$@"; then
    step "OK    \$name"
  else
    local rc=\$?
    [ "\$rc" = 124 ] && step "TIMEOUT \$name hit \${to}s cap" || step "FAIL \$name (rc=\$rc)"
    return "\$rc"
  fi
}

export DEBIAN_FRONTEND=noninteractive
step "apt update+install (including docker.io for Virtuoso)"
run_step apt ${STEP_APT_TIMEOUT} -- bash -c 'apt-get update -qq && apt-get install -y -qq build-essential g++ pkg-config git curl jq python3 python3-venv python3-pip unzip docker.io' || true

step "start docker daemon (required for Virtuoso OSS)"
systemctl enable --now docker || systemctl start docker || true
for _ in \$(seq 1 30); do docker info >/dev/null 2>&1 && { step "docker daemon up"; break; }; sleep 2; done
docker info >/dev/null 2>&1 || step "WARN: docker daemon still not reachable — Virtuoso will stay NOT-COMPARABLE"

step "rustup install"
run_step rustup ${STEP_RUSTUP_TIMEOUT} -- bash -c "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal" || true
export PATH="/root/.cargo/bin:\$PATH"
. /root/.cargo/env || true
for _ in \$(seq 1 30); do command -v cargo >/dev/null 2>&1 && break; sleep 2; done
command -v cargo || step "WARN: cargo not on PATH after rustup"

step "git clone + checkout ${BRANCH}"
cd /root
git clone -q "$REPO" sparq
cd sparq
git fetch -q origin "${BRANCH}"
git checkout -q "${BRANCH}"
SHA=\$(git rev-parse --short HEAD)
SHA_FULL=\$(git rev-parse HEAD)
step "checked out \$SHA"

step "cargo build sparq-cli (release)"
run_step build-sparq-cli ${STEP_BUILD_TIMEOUT} -- cargo build --release -q -p sparq-cli \
  || step "WARN: sparq-cli build failed"

step "sp2b gen (${SP2B_TRIPLES} triples)"
run_step sp2b-gen ${STEP_GEN_TIMEOUT} -- bash -c "bash bench/sp2b/gen.sh ${SP2B_TRIPLES} > /tmp/corpus_path.txt 2>/tmp/gen.err" \
  || step "WARN: sp2b gen failed (see /tmp/gen.err)"
CORPUS=\$(tail -n1 /tmp/corpus_path.txt 2>/dev/null || true)
step "corpus=\$CORPUS"
[ -n "\$CORPUS" ] && ls -la "\$CORPUS" || step "WARN: no corpus path"

export QUIET_BOX=true
mkdir -p bench/competitor-results bench/canonical-competitor-results

# ---- 1. sparq-cli bench (count mode, all queries) ------------------------------------
step "sparq-cli bench (count, iters=${ITERS})"
run_step sparq-run ${STEP_SPARQ_RUN_TIMEOUT} -- bash -c \
  "./target/release/sparq-cli bench \"\$CORPUS\" turtle bench/sp2b/queries ${ITERS} count > /tmp/sparq.tsv 2>/tmp/sparq.err" \
  || step "WARN: sparq run failed (see /tmp/sparq.err)"
echo "=== sparq.tsv ==="; cat /tmp/sparq.tsv 2>/dev/null || true

# ---- 2. Virtuoso OSS (docker pull, ld_dir bulk load, HTTP query, EXIT-trap teardown) -
step "Virtuoso same-box run (scripts/virtuoso-same-box.sh, iters=${ITERS})"
: > /tmp/virtuoso.tsv
VIRT_STATUS="not-run"
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  if VIRTUOSO_PULL_TIMEOUT=600 VIRTUOSO_READY_TIMEOUT=300 VIRTUOSO_LOAD_TIMEOUT=1200 VIRTUOSO_QUERY_TIMEOUT=120 \
       run_step virtuoso ${STEP_VIRT_TIMEOUT} -- bash scripts/virtuoso-same-box.sh \
         "\$CORPUS" ttl bench/sp2b/queries ${ITERS} /tmp/virtuoso.tsv; then
    VIRT_STATUS="ok"
    step "Virtuoso OK"
  else
    VIRT_STATUS="failed"
    step "WARN: Virtuoso run failed/timed out — keeping cell NOT-COMPARABLE per honesty invariant"
  fi
else
  VIRT_STATUS="docker-unavailable"
  step "WARN: Docker not available — Virtuoso NOT-COMPARABLE"
fi
echo "=== virtuoso.tsv ==="; cat /tmp/virtuoso.tsv 2>/dev/null || true

# ---- 3. count cross-check (expected-rows.tsv) ----------------------------------------
step "count cross-check"
python3 - <<'PYCHECK'
import sys

expected_path = "bench/sp2b/expected-rows.tsv"
exp = {}
with open(expected_path) as f:
    for line in f:
        line = line.strip()
        if not line or line.startswith('#'): continue
        parts = line.split('\t')
        if len(parts) >= 2:
            exp[parts[0]] = parts[1]

def parse_tsv(path):
    rows = {}
    try:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if not line: continue
                parts = line.split('\t')
                if len(parts) >= 2:
                    rows[parts[0]] = parts[1]
    except FileNotFoundError:
        pass
    return rows

sparq = parse_tsv("/tmp/sparq.tsv")
virt  = parse_tsv("/tmp/virtuoso.tsv")

print("=== Count cross-check ===")
print(f"{'query':<8} {'expected':<12} {'sparq':<12} {'virtuoso':<12} {'sparq_ok':<10} {'virt_ok':<10}")
for q in sorted(exp.keys()):
    e = exp[q]
    s = sparq.get(q, "n/a")
    v = virt.get(q, "n/a")
    s_ok = (s == e)
    v_ok = (v == e)
    flag = ""
    if not s_ok: flag += " SPARQ-WRONG"
    if not v_ok and v not in ("n/a", "ERROR"): flag += " VIRT-WRONG"
    print(f"{q:<8} {e:<12} {s:<12} {v:<12} {str(s_ok):<10} {str(v_ok):<10}{flag}")

# Special focus: q08/q12b
print()
print("=== q08/q12b focus ===")
for q in ["q08", "q12b"]:
    e = exp.get(q, "?")
    s = sparq.get(q, "n/a")
    v = virt.get(q, "n/a")
    print(f"{q}: expected={e} sparq={s} virtuoso={v}")
PYCHECK

# ---- 4. timing comparison for q08/q12b -----------------------------------------------
step "q08/q12b timing comparison"
python3 - <<'PYTIME'
import sys

def parse_tsv(path):
    rows = {}
    try:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if not line: continue
                parts = line.split('\t')
                if len(parts) >= 3:
                    rows[parts[0]] = (parts[1], parts[2])
                elif len(parts) >= 2:
                    rows[parts[0]] = (parts[1], "n/a")
    except FileNotFoundError:
        pass
    return rows

sparq = parse_tsv("/tmp/sparq.tsv")
virt  = parse_tsv("/tmp/virtuoso.tsv")

print("=== Same-box timing: sparq vs Virtuoso (best_us = min-of-5) ===")
print(f"{'query':<8} {'sparq_rows':<12} {'sparq_us':<14} {'virt_rows':<12} {'virt_us':<14} {'verdict'}")

expected = {"q08": "358", "q12b": "1"}
for q in ["q08", "q12b"]:
    s_rows, s_us = sparq.get(q, ("n/a", "n/a"))
    v_rows, v_us = virt.get(q, ("n/a", "n/a"))
    exp_rows = expected[q]
    s_ok = (s_rows == exp_rows)
    v_ok = (v_rows == exp_rows)
    if not s_ok:
        verdict = f"SPARQ-WRONG(got={s_rows},exp={exp_rows})"
    elif not v_ok:
        verdict = f"NOT-COMPARABLE(virt wrong/na/error: got={v_rows})"
    else:
        try:
            s_f = float(s_us)
            v_f = float(v_us)
            ratio = s_f / v_f
            if ratio < 1.0:
                verdict = f"AHEAD {1/ratio:.1f}x"
            else:
                verdict = f"BEHIND {ratio:.1f}x"
        except (ValueError, ZeroDivisionError):
            verdict = "timing-unavailable"
    print(f"{q:<8} {s_rows:<12} {s_us:<14} {v_rows:<12} {v_us:<14} {verdict}")
PYTIME

# ---- 5. assemble canonical envelope --------------------------------------------------
CPU=\$(LC_ALL=C grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2- | sed 's/^ *//' || true)
[ -n "\$CPU" ] || CPU=\$(uname -p)
NPROC=\$(nproc)
KERNEL=\$(uname -r)
NOW=\$(date -u +%Y-%m-%dT%H:%M:%SZ)
TODAY=\$(date -u +%Y-%m-%d)

SPARQ_TSV=\$(python3 -c 'import json,os; print(json.dumps(open("/tmp/sparq.tsv").read() if os.path.exists("/tmp/sparq.tsv") else ""))')
VIRT_TSV=\$(python3  -c 'import json,os; print(json.dumps(open("/tmp/virtuoso.tsv").read() if os.path.exists("/tmp/virtuoso.tsv") else ""))')
GATHER_STEPS=\$(python3 -c 'import json,os; print(json.dumps(open("/root/GATHER_STEP").read() if os.path.exists("/root/GATHER_STEP") else ""))')

mkdir -p bench/canonical-competitor-results/\$TODAY
ENVELOPE_FILE="bench/canonical-competitor-results/\$TODAY/canonical-sp2b-q08-virtuoso-\${NOW//:/}.json"
cat > "\$ENVELOPE_FILE" <<EOF2
{
  "gather": "sparql-same-box-comparison-q08-q12b-virtuoso",
  "bead": "sq-7d3dj.30.22",
  "canonical": true,
  "canonical_note": "CANONICAL: dedicated quiet EC2 c6i.4xlarge (16 vCPU / 32 GiB, x86_64) in ${REGION}. min-of-${ITERS} on loaded stores; count cross-checked vs bench/sp2b/expected-rows.tsv. Virtuoso OSS (Docker) is the only prior CORRECT comparator on q08/q12b (QLever disqualified: wrong count 0 on both). sparq built from ${BRANCH}@\$SHA.",
  "git_commit": "\$SHA",
  "git_commit_full": "\$SHA_FULL",
  "git_branch": "${BRANCH}",
  "suite": "sp2b",
  "scale": "SP2Bench ${SP2B_TRIPLES} triples (real Freiburg sp2b_gen, deterministic)",
  "iters": ${ITERS},
  "tsv_format": "<query>\\t<rows|ERROR>\\t<best_us|engine>",
  "disqualified_note": "QLever returns count 0 on q08 (expected 358) and q12b (expected 1); bnode!=IRI strict-type-error divergence (sq-ai2wa). QLever timing on these two rows is NOT a valid comparator.",
  "engines": {
    "sparq": {
      "version": "\$SHA",
      "features": "id-filter-fastpath (PRs #1785+#1795)",
      "mode": "sparq-cli bench (count mode)"
    },
    "virtuoso": {
      "version": "openlink/virtuoso-opensource-7",
      "mode": "http-sparql via scripts/virtuoso-same-box.sh (isql ld_dir bulk-load) + http_sparql_adapter.py"
    }
  },
  "statuses": {
    "sparq": "ok",
    "virtuoso": "\$VIRT_STATUS"
  },
  "env": {
    "host_class": "dedicated-quiet-ec2-c6i.4xlarge (CANONICAL 16 vCPU / 32 GiB x86_64)",
    "cpu_model": "\$CPU",
    "nproc": \$NPROC,
    "os": "Linux",
    "kernel": "\$KERNEL",
    "quiet_box": true,
    "gathered_at_utc": "\$NOW",
    "region": "${REGION}"
  },
  "sparq_tsv": \$SPARQ_TSV,
  "virtuoso_tsv": \$VIRT_TSV,
  "step_log": \$GATHER_STEPS
}
EOF2

step "validating envelope JSON"
jq . "\$ENVELOPE_FILE" >/dev/null && step "envelope valid JSON" || step "WARN: envelope invalid JSON"
ls -la bench/canonical-competitor-results/\$TODAY/
cat "\$ENVELOPE_FILE"
step "GATHER_DONE"
echo "\$SHA" > /root/GATHER_DONE
sync
UD
)

log "launching $ITYPE (orphan-proof: --instance-initiated-shutdown-behavior terminate + ${WATCHDOG_S}s watchdog)"
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
  i-*)  ;;
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
IP=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" \
  --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)
log "public IP=$IP; waiting for sshd"
SSH_UP=0
for i in $(seq 1 40); do
  ssh $SSHO "ubuntu@$IP" true 2>/dev/null && { log "ssh up (attempt $i)"; SSH_UP=1; break; }
  sleep 10
done
[ "$SSH_UP" = 1 ] || die "sshd never became reachable on $IP — cleanup trap terminates $INSTANCE_ID"

# ---- poll for GATHER_DONE sentinel (sentinel-gated teardown, same pattern as gather-ec2-sparql.sh)
mkdir -p "$RESULTS_DIR"
log "polling for /root/GATHER_DONE (apt+rustup+build+gen+sparq+virtuoso ~45-90min; deadline ${POLL_DEADLINE_S}s < ${WATCHDOG_S}s watchdog)"
DONE=0
POLL_START=$(date +%s)
poll_i=0
while :; do
  ELAPSED=$(( $(date +%s) - POLL_START ))
  if [ "$ELAPSED" -ge "$POLL_DEADLINE_S" ]; then
    log "  poll deadline ${POLL_DEADLINE_S}s reached without sentinel — pulling step log + giving up"
    break
  fi
  sleep 30
  poll_i=$(( poll_i + 1 ))
  if ssh $SSHO "ubuntu@$IP" "sudo test -f /root/GATHER_DONE" 2>/dev/null; then
    log "  [$poll_i / ${ELAPSED}s] sentinel present — pulling results"
    DONE=1; break
  fi
  STATE=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" \
    --query 'Reservations[0].Instances[0].State.Name' --output text 2>/dev/null || echo unknown)
  CUR_STEP=$(ssh $SSHO "ubuntu@$IP" "sudo tail -n1 /root/GATHER_STEP 2>/dev/null" 2>/dev/null || true)
  log "  [$poll_i / ${ELAPSED}s] state=$STATE; step: ${CUR_STEP:-<not started>}"
  [ "$STATE" = "terminated" ] && { log "  instance terminated before sentinel — check watchdog"; break; }
done

if [ "$DONE" = 1 ]; then
  # Pull all canonical-competitor-results from the instance
  ssh $SSHO "ubuntu@$IP" "sudo tar -C /root/sparq/bench -cf - canonical-competitor-results 2>/dev/null" \
    | tar -C "$ROOT/bench" -xf - 2>/dev/null \
    && log "pulled canonical-competitor-results" \
    || log "tar pull failed — trying direct scp"
  # Print the envelopes
  for f in "$RESULTS_DIR"/*.json; do
    [ -f "$f" ] || continue
    log "envelope: $f"
    cat "$f"; echo
  done
else
  log "NO sentinel — pulling diagnostic step log"
  STEP_LOG=$(ssh $SSHO "ubuntu@$IP" "sudo cat /root/GATHER_STEP 2>/dev/null" 2>/dev/null || true)
  if [ -n "$STEP_LOG" ]; then
    log "  --- /root/GATHER_STEP ---"
    printf '%s\n' "$STEP_LOG" >&2
    log "  >>> LAST STEP: $(printf '%s\n' "$STEP_LOG" | tail -n1)"
  fi
  ssh $SSHO "ubuntu@$IP" "sudo tail -200 /var/log/gather.log 2>/dev/null" >&2 || true
fi

log "done; cleanup trap terminates $INSTANCE_ID + deletes keypair/SG"
