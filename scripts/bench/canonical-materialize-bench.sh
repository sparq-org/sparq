#!/usr/bin/env bash
# [FABLE-5] sq-hmd7l.32 — CANONICAL materialization-benchmark EC2 launcher (committed).
#
# 🤖 SPARQ agent. Sibling of scripts/bench/canonical-competitor-bench.sh (same
# orphan-proof pattern — that header documents the rails); this one launches ONE
# dedicated QUIET c6i.4xlarge in eu-west-2, clones the requested branch ON the box, and
# runs the committed instance-side gather
# scripts/bench/canonical-materialize-gather-instance.sh — the D6 materialization panel
# (sparq reason OWL-RL/RDFS vs Jena vs VLog vs Nemo on LUBM univ>=100, engines strictly
# sequential) plus the HDT decode-only comparison (sq-hmd7l.33).
#
# ORPHAN-PROOF (the launcher can die at ANY point without leaking a box):
#   * --instance-initiated-shutdown-behavior terminate
#   * user-data watchdog: (sleep $WATCHDOG_S; shutdown -h now) + systemd-run backup — the
#     box self-terminates at the hard cap even if the gather AND this launcher both hang.
#   * launcher poll deadline sits BELOW the watchdog, teardown is sentinel-gated.
#   * EXIT trap: terminate instance (waits), delete ephemeral keypair + security group.
#   * REFUSES to touch the protected prod/dev instances.
#   * results are ALSO cat'd to the console log by the instance script, so
#     `aws ec2 get-console-output` recovers the envelopes even with no SSH pull.
#   * [OPUS-5] sq-ffaa9: with BENCH_IAM_PROFILE + BENCH_RESULTS_S3 exported the box also
#     uploads every envelope to a run-scoped S3 prefix (durable even when the AMI's serial
#     console yields nothing, the AL2023/Nitro failure mode). Opt-in; inert without them.
#
# Usage:  AWS_PROFILE=pss scripts/bench/canonical-materialize-bench.sh [<branch>]
#   env: REGION ITYPE LUBM_UNIVS MAT_ITERS TIMEOUT_S JAVA_XMX HDT WATCHDOG_S EBS_GB
#        RESULTS_LOCAL BENCH_IAM_PROFILE BENCH_RESULTS_S3
# Results (materialize-lubm*/hdt-* envelopes) are SSH-pulled incrementally into
# RESULTS_LOCAL (default ~/sparq-bench-results/canonical-<UTC-date>-materialize/);
# vendor reviewed envelopes into bench/canonical-competitor-results/<date>/ + run
# scripts/bench/ingest-canonical-competitors.mjs deliberately (never auto-committed).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${AWS_PROFILE:=pss}"; export AWS_PROFILE   # this box's default creds are the dev-instance role — MUST use pss
REGION="${REGION:-eu-west-2}"
ITYPE="${ITYPE:-c6i.4xlarge}"
BRANCH="${1:-${BRANCH:-main}}"
REPO="https://github.com/sparq-org/sparq.git"
LUBM_UNIVS="${LUBM_UNIVS:-1 100}"
MAT_ITERS="${MAT_ITERS:-5 3}"
TIMEOUT_S="${TIMEOUT_S:-3600}"
JAVA_XMX="${JAVA_XMX:-24g}"
HDT="${HDT:-1}"
# builds (~40m) + univ=100 corpus gen (~25m) + 4 engines x 2 profiles under a 1h
# per-engine cap: ~4h expected worst case -> 6h hard watchdog (workload +50%).
WATCHDOG_S="${WATCHDOG_S:-21600}"                 # 6h hard cap (self-terminate backstop)
POLL_DEADLINE_S="${POLL_DEADLINE_S:-20400}"       # 5h40m < watchdog, so watchdog stays backstop
POLL_INTERVAL_S="${POLL_INTERVAL_S:-60}"
EBS_GB="${EBS_GB:-100}"
RESULTS_LOCAL="${RESULTS_LOCAL:-$HOME/sparq-bench-results/canonical-$(date -u +%Y-%m-%d)-materialize}"

PROD_INSTANCE="i-090531b4ede8f2d3f"; DEV_INSTANCE="i-00f76802f345b6b77"

log() { printf '[canonical-materialize %s] %s\n' "$(date -u +%H:%M:%S)" "$*" >&2; }
die() { printf '[canonical-materialize] ERROR: %s\n' "$*" >&2; exit 1; }

command -v aws >/dev/null || die "aws CLI not found"
mkdir -p "$RESULTS_LOCAL"

# [OPUS-5] sq-ffaa9 — optional durable S3 egress (BENCH_IAM_PROFILE + BENCH_RESULTS_S3).
# Inert unless BOTH are exported; half-configured fails fast here rather than after a
# multi-hour gather. See scripts/bench/bootstrap-bench-iam.sh (one-time maintainer setup).
. "$SCRIPT_DIR/bench-result-egress.sh"
bench_egress_preflight || die "S3 result egress is misconfigured (see the message above)"

WORK="$(mktemp -d)"
KEYFILE="$WORK/key"
KEY_NAME="sparq-bench-canonical-mat-$$-${RANDOM}"
INSTANCE_ID=""; SG_ID=""
ssh-keygen -t ed25519 -N '' -f "$KEYFILE" -q
SSHO="-i $KEYFILE -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=15"
# Run-scoped S3 prefix (empty when egress is off). KEY_NAME is already unique per run.
EGRESS_URI="$(bench_egress_run_uri "$KEY_NAME")" || die "could not derive the S3 run prefix"

cleanup() {
  set +e
  if [ -n "$INSTANCE_ID" ]; then
    case "$INSTANCE_ID" in
      "$PROD_INSTANCE"|"$DEV_INSTANCE") log "REFUSING to terminate protected $INSTANCE_ID" ;;
      i-*)
        log "cleanup: terminating $INSTANCE_ID"
        aws ec2 terminate-instances --region "$REGION" --instance-ids "$INSTANCE_ID" >/dev/null 2>&1
        aws ec2 wait instance-terminated --region "$REGION" --instance-ids "$INSTANCE_ID" >/dev/null 2>&1
        log "cleanup: $INSTANCE_ID terminated"
        ;;
    esac
  fi
  [ -n "$SG_ID" ] && aws ec2 delete-security-group --region "$REGION" --group-id "$SG_ID" >/dev/null 2>&1
  aws ec2 delete-key-pair --region "$REGION" --key-name "$KEY_NAME" >/dev/null 2>&1
  rm -rf "$WORK"
}
trap cleanup EXIT

log "resolve x86_64 Ubuntu 24.04 AMI / network in $REGION"
AMI=$(aws ec2 describe-images --region "$REGION" --owners 099720109477 \
  --filters "Name=name,Values=ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-amd64-server-*" "Name=state,Values=available" \
  --query 'sort_by(Images,&CreationDate)[-1].ImageId' --output text)
[ -n "$AMI" ] && [ "$AMI" != None ] || die "could not resolve AMI"
VPC=$(aws ec2 describe-vpcs --region "$REGION" --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)
SUBNET=$(aws ec2 describe-subnets --region "$REGION" --filters Name=vpc-id,Values="$VPC" "Name=default-for-az,Values=true" --query 'Subnets[0].SubnetId' --output text)
MYIP=$(curl -s https://checkip.amazonaws.com | tr -d '[:space:]')
[ -n "$MYIP" ] || die "could not determine public IP"
log "AMI=$AMI VPC=$VPC SUBNET=$SUBNET MYIP=$MYIP ITYPE=$ITYPE BRANCH=$BRANCH UNIVS='$LUBM_UNIVS' ITERS='$MAT_ITERS'"

log "ephemeral keypair + locked-down SG (ssh from $MYIP/32 only)"
aws ec2 import-key-pair --region "$REGION" --key-name "$KEY_NAME" --public-key-material "fileb://${KEYFILE}.pub" >/dev/null
SG_ID=$(aws ec2 create-security-group --region "$REGION" --group-name "$KEY_NAME" --description "sparq canonical materialization bench (ephemeral)" --vpc-id "$VPC" --query 'GroupId' --output text)
aws ec2 authorize-security-group-ingress --region "$REGION" --group-id "$SG_ID" --protocol tcp --port 22 --cidr "${MYIP}/32" >/dev/null

TAGSPEC='ResourceType=instance,Tags=[{Key=Name,Value=sparq-bench},{Key=Project,Value=sparq-bench},{Key=purpose,Value=sparq-bench}]'

# Thin user-data: watchdog + clone the branch + run the COMMITTED instance script
# (the heavy gather logic — deps, pinned VLog/Nemo builds — lives in-repo, reviewable).
USERDATA=$(cat <<UD
#!/bin/bash
set -x
exec > >(tee /var/log/gather.log) 2>&1
( sleep $WATCHDOG_S; shutdown -h now ) &
systemd-run --on-active=$WATCHDOG_S /sbin/shutdown -h now || true

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq || true
apt-get install -y -qq git curl || true

cd /root
git clone -q "$REPO" sparq
cd sparq
git fetch -q origin "$BRANCH" && git checkout -q "$BRANCH"
echo "[user-data] checked out \$(git rev-parse --short HEAD)" | tee /dev/console

# BACKGROUND the multi-hour gather (nohup + setsid) so cloud-final RETURNS promptly and
# scripts_user succeeds — running the gather in the foreground of scripts_user wedged
# cloud-init and gave zero telemetry (sq-hmd7l.32 wave-1/2). The gather streams to
# /dev/console itself (recoverable via get-console-output even if SSH breaks); the
# watchdog above is the orphan-proof backstop independent of this shell.
# HOME/USER/LOGNAME are exported into the clean env below so the gather never trips on a
# bare HOME under set -u and cargo/rustup/java find their ~/.cargo caches. sq-hmd7l.32
# wave-2 stalled on "HOME: unbound variable" under cloud-init's HOME-less root context.
# NOTE: this is an UNQUOTED heredoc, so keep comments free of backticks and dollar signs
# or the launcher shell expands them into the rendered user-data.
setsid nohup env HOME=/root USER=root LOGNAME=root \
  BENCH_RESULTS_S3_URI="$EGRESS_URI" BENCH_EGRESS_REGION="$REGION" \
  LUBM_UNIVS="$LUBM_UNIVS" MAT_ITERS="$MAT_ITERS" TIMEOUT_S=$TIMEOUT_S JAVA_XMX=$JAVA_XMX HDT=$HDT \
  bash scripts/bench/canonical-materialize-gather-instance.sh >/var/log/gather.log 2>&1 < /dev/null &
echo "[user-data] gather backgrounded (pid \$!); cloud-final returning" | tee /dev/console
UD
)

log "launching $ITYPE (${EBS_GB}GB gp3, $((WATCHDOG_S / 3600))h watchdog)"
printf '%s\n' "$USERDATA" > "$WORK/userdata.sh"
UD_RAW=$(wc -c < "$WORK/userdata.sh")
log "user-data raw=${UD_RAW}B (limit 16384B)"
[ "$UD_RAW" -le 16384 ] || die "user-data ${UD_RAW}B exceeds 16384B — trim it"
LAUNCH_ERR="$WORK/launch.err"
# shellcheck disable=SC2046  # intentional word-split: bench_egress_launch_args emits either
# nothing (egress off — the launch command is then byte-identical to before) or the two
# words `--iam-instance-profile Name=<profile>`.
INSTANCE_ID=$(aws ec2 run-instances --region "$REGION" --image-id "$AMI" --instance-type "$ITYPE" \
  --instance-initiated-shutdown-behavior terminate \
  --key-name "$KEY_NAME" --security-group-ids "$SG_ID" \
  --subnet-id "$SUBNET" --associate-public-ip-address \
  $(bench_egress_launch_args) \
  --block-device-mappings "[{\"DeviceName\":\"/dev/sda1\",\"Ebs\":{\"VolumeSize\":${EBS_GB},\"VolumeType\":\"gp3\",\"DeleteOnTermination\":true}}]" \
  --tag-specifications "$TAGSPEC" \
  --user-data "file://$WORK/userdata.sh" \
  --query 'Instances[0].InstanceId' --output text 2>"$LAUNCH_ERR") || true
case "$INSTANCE_ID" in
  i-*) ;;
  *) INSTANCE_ID=""; die "run-instances failed: $(cat "$LAUNCH_ERR" 2>/dev/null)" ;;
esac
log "launched INSTANCE_ID=$INSTANCE_ID"
echo "$INSTANCE_ID" > "$RESULTS_LOCAL/INSTANCE_ID.txt"

aws ec2 wait instance-running --region "$REGION" --instance-ids "$INSTANCE_ID"
IP=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)
log "public IP=$IP; waiting for sshd"
SSH_UP=0
for i in $(seq 1 40); do ssh $SSHO "ubuntu@$IP" true 2>/dev/null && { log "ssh up"; SSH_UP=1; break; }; sleep 10; done
[ "$SSH_UP" = 1 ] || die "sshd never reachable on $IP — aborting (cleanup terminates)"

log "polling for /root/GATHER_DONE (deadline ${POLL_DEADLINE_S}s, < ${WATCHDOG_S}s watchdog)…"
DONE=0; i=0; POLL_START=$(date +%s)
while :; do
  ELAPSED=$(( $(date +%s) - POLL_START ))
  [ "$ELAPSED" -ge "$POLL_DEADLINE_S" ] && { log "poll deadline reached without sentinel — giving up (cleanup terminates)"; break; }
  sleep "$POLL_INTERVAL_S"; i=$(( i + 1 ))
  # incremental pull of any envelopes already written (SSH — best effort; the console
  # dump is the AUTHORITATIVE recovery channel since SSH broke on a saturated box).
  ssh $SSHO "ubuntu@$IP" "sudo tar -C /root/sparq/bench -cf - gather-out 2>/dev/null" 2>/dev/null | tar -C "$RESULTS_LOCAL" -xf - 2>/dev/null || true
  if ssh $SSHO "ubuntu@$IP" "sudo test -f /root/GATHER_DONE" 2>/dev/null; then
    log "[$i / ${ELAPSED}s] sentinel present — final pull"; DONE=1; break
  fi
  STATE=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" --query 'Reservations[0].Instances[0].State.Name' --output text 2>/dev/null || echo unknown)
  # PRIMARY telemetry: the serial console (the gather tees every STEP there). Snapshot it
  # every tick so progress is visible even with SSH down, and the final envelopes (dumped
  # between ===ENVELOPE-BEGIN/END=== markers) are recoverable from it.
  aws ec2 get-console-output --region "$REGION" --instance-id "$INSTANCE_ID" --output text > "$RESULTS_LOCAL/console.txt" 2>/dev/null || true
  # `|| true`: on early ticks the console is still empty so grep finds no STEP line and
  # exits non-zero; under this launcher's `set -euo pipefail` a naked `CON_STEP=$(grep …)`
  # would then trip set -e on iteration 1 and the EXIT trap would terminate the box ~65s in
  # with zero telemetry (sq-hmd7l.32 wave-3 early-cleanup). Same guard on the ssh step read.
  CON_STEP=$(grep -aoE '\[STEP [0-9TZ:-]+\] .*' "$RESULTS_LOCAL/console.txt" 2>/dev/null | tail -1 || true)
  CUR_STEP=$(ssh $SSHO "ubuntu@$IP" "sudo tail -n1 /root/GATHER_STEP 2>/dev/null" 2>/dev/null || true)
  log "[$i / ${ELAPSED}s] state=$STATE; step(ssh): ${CUR_STEP:-<none>}; step(console): ${CON_STEP:-<none>}"
  if grep -qa 'STEP.*gather complete' "$RESULTS_LOCAL/console.txt" 2>/dev/null; then
    log "[$i / ${ELAPSED}s] 'gather complete' on console — recovering envelopes from console"; DONE=1; break
  fi
  [ "$STATE" = "terminated" ] && { log "instance terminated before sentinel — results may be partial"; break; }
done

# [OPUS-5] sq-ffaa9 — durable channel FIRST when configured: the box uploaded each
# envelope to the run-scoped S3 prefix as it was produced, so this survives both a dead
# SSH path AND an AMI whose serial console returns nothing usable (the AL2023/Nitro case
# that left the x86_64 DiskANN gather unrecoverable). No-op when egress is off.
bench_egress_pull "$EGRESS_URI" "$RESULTS_LOCAL/gather-out"

# Recover the envelopes from the serial console (authoritative when SSH failed): each
# envelope was cat'd between ===ENVELOPE-BEGIN <name>=== / ===ENVELOPE-END=== markers.
aws ec2 get-console-output --region "$REGION" --instance-id "$INSTANCE_ID" --output text > "$RESULTS_LOCAL/console.txt" 2>/dev/null || true
if grep -qa 'ENVELOPE-BEGIN' "$RESULTS_LOCAL/console.txt" 2>/dev/null; then
  log "extracting envelopes from serial console → $RESULTS_LOCAL/gather-out/"
  "$SCRIPT_DIR/extract-console-envelopes.sh" \
    "$RESULTS_LOCAL/console.txt" "$RESULTS_LOCAL/gather-out" 2>/dev/null || true
fi

log "final result pull"
ssh $SSHO "ubuntu@$IP" "sudo tar -C /root/sparq/bench -cf - gather-out 2>/dev/null" 2>/dev/null | tar -C "$RESULTS_LOCAL" -xf - 2>/dev/null || true
ssh $SSHO "ubuntu@$IP" "sudo cat /root/GATHER_STEP 2>/dev/null" > "$RESULTS_LOCAL/GATHER_STEP.txt" 2>/dev/null || true
ssh $SSHO "ubuntu@$IP" "sudo tail -800 /var/log/gather.log 2>/dev/null" > "$RESULTS_LOCAL/gather.log.tail" 2>/dev/null || true

log "envelopes in $RESULTS_LOCAL/gather-out:"
ls -la "$RESULTS_LOCAL"/gather-out/*.json 2>/dev/null >&2 || log "  (none yet)"
[ "$DONE" = 1 ] || log "NOTE: sentinel not observed — see $RESULTS_LOCAL/GATHER_STEP.txt for the last step"
log "done; cleanup trap terminates $INSTANCE_ID + deletes keypair/SG"
