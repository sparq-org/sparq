#!/usr/bin/env bash
# [FABLE-5] sq-7d3dj.34 — CANONICAL competitor benchmark EC2 launcher (committed harness).
#
# The 2026-07-07 canonical 5-engine CLI matrix (bench/canonical-competitor-results/2026-07-07/)
# was produced by an ad-hoc launcher that never landed in the repo; this is its committed,
# extended successor. It launches ONE dedicated QUIET c6i.4xlarge (16 vCPU / 32 GiB, x86_64)
# in eu-west-2, clones the requested branch ON the box, and runs the committed instance-side
# gather scripts/bench/canonical-http-gather-instance.sh — the HTTP/TTFB panel (D9/D10):
# sparq in HTTP server mode (sparq-server) alongside Oxigraph-serve / Fuseki (offline
# tdb2.tdbloader, the D10 fix) / Virtuoso / QLever, full-request latency + TTFB, in BOTH
# keep-alive and fresh-connect regimes, over SP2Bench + WatDiv, GATHERS=2 back-to-back.
#
# ORPHAN-PROOF (the launcher can die at ANY point without leaking a box):
#   * --instance-initiated-shutdown-behavior terminate
#   * user-data watchdog: (sleep $WATCHDOG_S; shutdown -h now) + systemd-run backup — the
#     box self-terminates at the hard cap even if the benchmark AND this launcher both hang.
#   * launcher poll deadline sits BELOW the watchdog, teardown is sentinel-gated.
#   * EXIT trap: terminate instance (waits), delete ephemeral keypair + security group.
#   * REFUSES to touch the protected prod/dev instances.
#   * [OPUS-5] sq-ffaa9: with BENCH_IAM_PROFILE + BENCH_RESULTS_S3 exported the box also
#     uploads every envelope to a run-scoped S3 prefix — the channel that survives an AMI
#     whose serial console returns nothing usable. Opt-in; inert without them.
#
# Usage:  AWS_PROFILE=pss scripts/bench/canonical-competitor-bench.sh [<branch>]
#   env: REGION ITYPE SP2B_TRIPLES WATDIV_SF ITERS QTO GATHERS WATCHDOG_S EBS_GB RESULTS_LOCAL
#        BENCH_IAM_PROFILE BENCH_RESULTS_S3 (opt-in durable S3 egress, sq-ffaa9)
# Results (canonical-*-http-*.json envelopes) are SSH-pulled incrementally into
# RESULTS_LOCAL (default ~/sparq-bench-results/canonical-<UTC-date>-http/); vendor reviewed
# envelopes into bench/canonical-competitor-results/<date>-http/ + run
# scripts/bench/ingest-canonical-competitors.mjs deliberately (never auto-committed).
set -euo pipefail

: "${AWS_PROFILE:=pss}"; export AWS_PROFILE   # this box's default creds are the dev-instance role — MUST use pss
REGION="${REGION:-eu-west-2}"
ITYPE="${ITYPE:-c6i.4xlarge}"
BRANCH="${1:-${BRANCH:-main}}"
REPO="https://github.com/sparq-org/sparq.git"
SP2B_TRIPLES="${SP2B_TRIPLES:-250000}"
WATDIV_SF="${WATDIV_SF:-1}"
ITERS="${ITERS:-3}"
QTO="${QTO:-300}"
GATHERS="${GATHERS:-2}"
SUITES="${SUITES:-sp2b watdiv}"   # space-separated suite filter (partial re-runs)
WATCHDOG_S="${WATCHDOG_S:-10800}"                 # 3h hard cap (self-terminate backstop)
POLL_DEADLINE_S="${POLL_DEADLINE_S:-9900}"        # 2h45m < watchdog, so watchdog stays backstop
POLL_INTERVAL_S="${POLL_INTERVAL_S:-60}"
EBS_GB="${EBS_GB:-100}"
RESULTS_LOCAL="${RESULTS_LOCAL:-$HOME/sparq-bench-results/canonical-$(date -u +%Y-%m-%d)-http}"

PROD_INSTANCE="i-090531b4ede8f2d3f"; DEV_INSTANCE="i-00f76802f345b6b77"

log() { printf '[canonical-bench %s] %s\n' "$(date -u +%H:%M:%S)" "$*" >&2; }
die() { printf '[canonical-bench] ERROR: %s\n' "$*" >&2; exit 1; }

command -v aws >/dev/null || die "aws CLI not found"
mkdir -p "$RESULTS_LOCAL"

# [OPUS-5] sq-ffaa9 — optional durable S3 egress (BENCH_IAM_PROFILE + BENCH_RESULTS_S3).
# Inert unless BOTH are exported; half-configured fails fast here rather than after a
# multi-hour gather. See scripts/bench/bootstrap-bench-iam.sh (one-time maintainer setup).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$SCRIPT_DIR/bench-result-egress.sh"
bench_egress_preflight || die "S3 result egress is misconfigured (see the message above)"

WORK="$(mktemp -d)"
KEYFILE="$WORK/key"
KEY_NAME="sparq-bench-canonical-http-$$-${RANDOM}"
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
log "AMI=$AMI VPC=$VPC SUBNET=$SUBNET MYIP=$MYIP ITYPE=$ITYPE BRANCH=$BRANCH SP2B=$SP2B_TRIPLES WATDIV_SF=$WATDIV_SF ITERS=$ITERS GATHERS=$GATHERS"

log "ephemeral keypair + locked-down SG (ssh from $MYIP/32 only)"
aws ec2 import-key-pair --region "$REGION" --key-name "$KEY_NAME" --public-key-material "fileb://${KEYFILE}.pub" >/dev/null
SG_ID=$(aws ec2 create-security-group --region "$REGION" --group-name "$KEY_NAME" --description "sparq canonical HTTP/TTFB competitor bench (ephemeral)" --vpc-id "$VPC" --query 'GroupId' --output text)
aws ec2 authorize-security-group-ingress --region "$REGION" --group-id "$SG_ID" --protocol tcp --port 22 --cidr "${MYIP}/32" >/dev/null

TAGSPEC='ResourceType=instance,Tags=[{Key=Name,Value=sparq-bench},{Key=Project,Value=sparq-bench},{Key=purpose,Value=sparq-bench}]'

# Thin user-data: watchdog + deps + clone the branch + run the COMMITTED instance script.
# (The heavy gather logic lives in-repo, reviewable — not in this heredoc.)
USERDATA=$(cat <<UD
#!/bin/bash
set -x
exec > >(tee /var/log/gather.log) 2>&1
( sleep $WATCHDOG_S; shutdown -h now ) &
systemd-run --on-active=$WATCHDOG_S /sbin/shutdown -h now || true

step() { echo "[STEP \$(date -u +%Y-%m-%dT%H:%M:%SZ)] \$*" | tee -a /root/GATHER_STEP >&2; }
export DEBIAN_FRONTEND=noninteractive
step "apt update+install"
apt-get update -qq
# libboost-all-dev: the WatDiv generator (bench/watdiv/gen.sh) compiles against Boost
# headers — omitting it silently skipped the watdiv suite ("NO CORPUS", 2026-07-07 run 1).
apt-get install -y -qq build-essential g++ pkg-config git curl jq python3 unzip docker.io openjdk-21-jre-headless libboost-all-dev bc || true
step "start docker"
systemctl enable --now docker || systemctl start docker || true
for _ in \$(seq 1 60); do docker info >/dev/null 2>&1 && { step "docker up"; break; }; sleep 2; done
docker info >/dev/null 2>&1 || step "WARN: docker daemon not reachable"

step "rustup install"
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal || true
export PATH="/root/.cargo/bin:\$PATH"; . /root/.cargo/env || true

step "git clone + checkout $BRANCH"
cd /root
git clone -q "$REPO" sparq
cd sparq
git fetch -q origin "$BRANCH" && git checkout -q "$BRANCH"
step "checked out \$(git rev-parse --short HEAD)"

SP2B_TRIPLES=$SP2B_TRIPLES WATDIV_SF=$WATDIV_SF ITERS=$ITERS QTO=$QTO GATHERS=$GATHERS SUITES="$SUITES" \
  BENCH_RESULTS_S3_URI="$EGRESS_URI" BENCH_EGRESS_REGION="$REGION" \
  bash scripts/bench/canonical-http-gather-instance.sh
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
  # incremental pull of any canonical envelopes already written
  ssh $SSHO "ubuntu@$IP" "sudo tar -C /root/sparq/bench -cf - competitor-results 2>/dev/null" 2>/dev/null | tar -C "$RESULTS_LOCAL" -xf - 2>/dev/null && \
    cp -f "$RESULTS_LOCAL"/competitor-results/canonical-*.json "$RESULTS_LOCAL/" 2>/dev/null || true
  if ssh $SSHO "ubuntu@$IP" "sudo test -f /root/GATHER_DONE" 2>/dev/null; then
    log "[$i / ${ELAPSED}s] sentinel present — final pull"; DONE=1; break
  fi
  STATE=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" --query 'Reservations[0].Instances[0].State.Name' --output text 2>/dev/null || echo unknown)
  CUR_STEP=$(ssh $SSHO "ubuntu@$IP" "sudo tail -n1 /root/GATHER_STEP 2>/dev/null" 2>/dev/null || true)
  log "[$i / ${ELAPSED}s] state=$STATE; step: ${CUR_STEP:-<not started>}"
  [ "$STATE" = "terminated" ] && { log "instance terminated before sentinel — results may be partial"; break; }
done

# [OPUS-5] sq-ffaa9 — durable channel first when configured: the box uploaded each
# envelope as it was produced, so this survives a dead SSH path AND an AMI whose serial
# console returns nothing usable. No-op when egress is off.
bench_egress_pull "$EGRESS_URI" "$RESULTS_LOCAL"

log "final result pull"
ssh $SSHO "ubuntu@$IP" "sudo tar -C /root/sparq/bench -cf - competitor-results 2>/dev/null" 2>/dev/null | tar -C "$RESULTS_LOCAL" -xf - 2>/dev/null || true
cp -f "$RESULTS_LOCAL"/competitor-results/canonical-*.json "$RESULTS_LOCAL/" 2>/dev/null || true
ssh $SSHO "ubuntu@$IP" "sudo cat /root/GATHER_STEP 2>/dev/null" > "$RESULTS_LOCAL/GATHER_STEP.txt" 2>/dev/null || true
ssh $SSHO "ubuntu@$IP" "sudo tail -400 /var/log/gather.log 2>/dev/null" > "$RESULTS_LOCAL/gather.log.tail" 2>/dev/null || true

log "canonical envelopes in $RESULTS_LOCAL:"
ls -la "$RESULTS_LOCAL"/canonical-*.json 2>/dev/null >&2 || log "  (none yet)"
[ "$DONE" = 1 ] || log "NOTE: sentinel not observed — see $RESULTS_LOCAL/GATHER_STEP.txt for the last step"
log "done; cleanup trap terminates $INSTANCE_ID + deletes keypair/SG"
