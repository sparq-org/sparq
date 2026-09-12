#!/usr/bin/env bash
# [OPUS-4.8] sparq EC2 CODEX WORKER — runs GPT-5.6 (codex-cli, ChatGPT-OAuth) on an EPHEMERAL
# EC2 instance to implement ONE bead, then pulls the committed branch back as a git bundle and
# opens the PR from THIS (trusted, gh-authed) box. Fable unavailable — flag for re-review when
# Fable returns. Companion to ec2-buildfarm.sh; reuses its orphan-proof machinery.
#
#   scripts/ec2-codex-worker.sh <bead-id>        # launch + run codex + pull bundle + open PR
#   scripts/ec2-codex-worker.sh --dry-run <bead> # print the plan, launch NOTHING
#   scripts/ec2-codex-worker.sh --self-test      # hermetic unit test (no aws, no network)
#   scripts/ec2-codex-worker.sh --orphan-check [--apply]   # find/terminate codex-worker orphans
#
# WHY. The local box is a t4g.large (2 vCPU / 8G) that CANNOT run heavy cargo builds. The
# maintainer directive is "spin up EC2 for any intensive work" while staying cost-conscious.
# This offloads a bead's IMPLEMENTATION + full crate gate to a large ephemeral arm64 instance,
# where GPT-5.6 (codex) writes + gates + commits the branch. There is no gh auth on the instance,
# so the instance only COMMITS; the controller SSH-pulls a `git bundle` of the branch, fetches it
# into a local worktree, pushes from THIS box, and opens the PR (codex is the author). It does
# NOT arm the PR — the orchestrator arms after review.
#
# ORPHAN-PROOF (agent-death-independent; identical discipline to ec2-buildfarm.sh):
#   --instance-initiated-shutdown-behavior terminate + ( sleep MAX; shutdown -h now ) & +
#   systemd-run --on-active=MAXs /sbin/shutdown -h now  +  trap cleanup EXIT (ALWAYS terminates
#   the one launched instance, deletes the ephemeral keypair/SG) + a pre-launch fleet cost cap
#   (MAX_FLEET) + a final orphan sweep. Distinct tag purpose=sparq-codex-worker (NOT
#   sparq-buildfarm / sparq-bench / sparq-ci-bench). NEVER touches prod/dev (hard-excluded).
#
# COST. On-demand only: the pss profile CANNOT create the EC2-Spot service-linked role, so spot
# launches are rejected (AuthFailure.ServiceLinkedRoleCreationNotPermitted). c7g.4xlarge on-demand
# in eu-west-2 is ~$0.58/hr; the 2h watchdog caps a worst-case orphan at ~$1.16. MAX_FLEET=1.
set -euo pipefail

PROG="ec2-codex-worker"
log() { printf '[%s] %s\n' "$PROG" "$*" >&2; }
die() { printf '[%s] ERROR: %s\n' "$PROG" "$*" >&2; exit 1; }

# ---- tunables -------------------------------------------------------------------------
export AWS_PROFILE="${AWS_PROFILE:-pss}"
REGION="${AWS_REGION:-eu-west-2}"
ITYPE="${CODEX_INSTANCE_TYPE:-c7g.4xlarge}"        # 16 vCPU arm64; c7g.2xlarge for lighter beads
MAX_LIFETIME_SEC="${CODEX_MAX_LIFETIME_SEC:-7200}"
MAX_FLEET="${CODEX_MAX_FLEET:-1}"
USE_SPOT="${CODEX_SPOT:-0}"                         # 0=on-demand (spot role not permitted on pss)
DISK_GB="${CODEX_DISK_GB:-40}"
CODEX_VERSION="${CODEX_VERSION:-0.144.1}"
CODEX_HOME_DIR="${CODEX_HOME_DIR:-$HOME/.codex}"
# [OPUS-4.8] sq-uhqah: briefs are now vendored in-repo next to this script. The default is the
# cargo brief; JS/wasm beads pass CODEX_BRIEF_TEMPLATE=<dir>/codex-ec2-briefs/js-brief.txt.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BRIEF_TEMPLATE="${CODEX_BRIEF_TEMPLATE:-$SCRIPT_DIR/codex-ec2-briefs/cargo-brief.txt}"
# [OPUS-4.8] sq-uhqah: optional wasm toolchain in the EC2 user-data for JS/wasm npm beads
# (sq-6xasp.8/.9). Gated on CODEX_NEEDS_WASM so cargo dispatches stay fast (wasm-pack is a heavy
# ~2-4min build). Absolute cargo paths (no $ so the unquoted user-data heredoc won't expand it here).
WASM_TOOLCHAIN=""
if [ -n "${CODEX_NEEDS_WASM:-}" ]; then
  WASM_TOOLCHAIN="su - ubuntu -c '/home/ubuntu/.cargo/bin/rustup target add wasm32-unknown-unknown' || true
su - ubuntu -c '/home/ubuntu/.cargo/bin/cargo install wasm-pack --locked' || true"
fi
BUNDLE_DIR="${CODEX_BUNDLE_DIR:-/home/ubuntu/agent-worker/bundles}"
REPO="https://github.com/sparq-org/sparq.git"

# ---- tag + protected ids --------------------------------------------------------------
readonly TAG_KEY="purpose"
readonly TAG_VALUE="sparq-codex-worker"            # DISTINCT tag
readonly PROD_INSTANCE="i-090531b4ede8f2d3f"
readonly DEV_INSTANCE="i-00f76802f345b6b77"
readonly -a NEVER_TOUCH=("$PROD_INSTANCE" "$DEV_INSTANCE")

clamp() { local v="$1" lo="$2" hi="$3"; case "$v" in (''|*[!0-9]*) v="$lo";; esac; [ "$v" -lt "$lo" ] && v="$lo"; [ "$v" -gt "$hi" ] && v="$hi"; printf '%s' "$v"; }
MAX_LIFETIME_SEC="$(clamp "$MAX_LIFETIME_SEC" 600 10800)"
MAX_FLEET="$(clamp "$MAX_FLEET" 1 4)"

# Derive a filesystem/branch-safe slug from a bead id.
slugify() { printf '%s' "$1" | tr -c 'A-Za-z0-9._-' '-' | sed 's/-\{2,\}/-/g; s/^-//; s/-$//'; }

# Drop hard-excluded ids from a newline id list on stdin.
drop_excluded() {
  local ids id ex keep; ids="$(cat)"
  while IFS= read -r id; do
    [ -n "$id" ] || continue; keep=1
    for ex in "${NEVER_TOUCH[@]}"; do [ "$id" = "$ex" ] && { keep=0; break; }; done
    [ "$keep" = 1 ] && printf '%s\n' "$id"
  done <<< "$ids"; return 0
}

fleet_count() {
  local out; out="$(aws ec2 describe-instances --region "$REGION" \
    --filters "Name=tag:${TAG_KEY},Values=${TAG_VALUE}" "Name=instance-state-name,Values=running,pending" \
    --query 'Reservations[].Instances[].InstanceId' --output text 2>/dev/null | tr '\t' '\n' | sed '/^$/d' | wc -l | tr -d '[:space:]')"
  case "$out" in (''|*[!0-9]*) out=0;; esac; printf '%s' "$out"
}

orphan_check() { # <apply:0|1>
  local apply="$1" raw
  command -v aws >/dev/null 2>&1 || { log "aws not found — orphan check skipped."; return 0; }
  raw="$(aws ec2 describe-instances --region "$REGION" \
    --filters "Name=tag:${TAG_KEY},Values=${TAG_VALUE}" "Name=instance-state-name,Values=running,pending" \
    --query 'Reservations[].Instances[].InstanceId' --output text 2>/dev/null | tr '\t' '\n' | sed '/^$/d')" \
    || { log "describe-instances failed — no action."; return 0; }
  [ -n "$raw" ] || { log "orphan-check: no running/pending sparq-codex-worker instances — CLEAN."; return 0; }
  local -a ids kill_set; mapfile -t ids <<< "$raw"
  log "tag-matched running/pending:"; printf '  %s\n' "${ids[@]}" >&2
  mapfile -t kill_set < <(printf '%s\n' "${ids[@]}" | drop_excluded)
  local ex
  for ex in "${NEVER_TOUCH[@]}"; do
    printf '%s\n' "${ids[@]}" | grep -qxF "$ex" && log "WARNING: protected $ex carries the tag — HARD-EXCLUDED, NOT terminating."
  done
  [ "${#kill_set[@]}" -gt 0 ] || { log "after hard-exclusion, nothing to terminate."; return 0; }
  log "ORPHAN candidates: ${kill_set[*]}"
  [ "$apply" = 1 ] || { log "dry-run: re-run --orphan-check --apply to terminate."; return 0; }
  aws ec2 terminate-instances --region "$REGION" --instance-ids "${kill_set[@]}" >/dev/null 2>&1 \
    && log "terminate submitted: ${kill_set[*]}" || die "terminate FAILED: ${kill_set[*]}"
}

# Render the orphan-proof user-data (rust + node + codex install; no early shutdown). Pure-text.
render_userdata() { # <bead> <branch>
  local branch="$2"  # $1 (bead) is not referenced in the user-data; kept for the call contract
  cat <<UD
#!/bin/bash
set -x
exec > >(tee /var/log/codex-worker.log) 2>&1
# [OPUS-4.8] ORPHAN-PROOF: two independent hard caps, armed before any package install.
( sleep ${MAX_LIFETIME_SEC}; shutdown -h now ) &
systemd-run --on-active=${MAX_LIFETIME_SEC}s /sbin/shutdown -h now || true

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq build-essential pkg-config git curl ca-certificates
# Node 20 + codex-cli (parity with the box's npm-global @openai/codex install)
curl -fsSL https://deb.nodesource.com/setup_20.x | bash - >/dev/null 2>&1
apt-get install -y -qq nodejs
npm i -g @openai/codex@${CODEX_VERSION} >/dev/null 2>&1 || npm i -g @openai/codex >/dev/null 2>&1
# Rust (minimal + clippy + rustfmt), for the ubuntu user (codex runs as ubuntu)
su - ubuntu -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --component clippy rustfmt' || true
# [OPUS-4.8] optional wasm toolchain (empty unless CODEX_NEEDS_WASM was set at dispatch)
${WASM_TOOLCHAIN}
# 'bd' (beads) — a static release binary the worker uses for 'bd show'. Best-effort; the brief
# also inlines nothing so if bd is missing the worker still has the bead spec via the file.
su - ubuntu -c 'cargo install --locked beads 2>/dev/null' || true
# Clone the repo + create the worktree branch off origin/main, owned by ubuntu.
su - ubuntu -c "git clone -q ${REPO} /home/ubuntu/sparq" || echo "WARN clone failed"
su - ubuntu -c "cd /home/ubuntu/sparq && git config user.name 'GPT-5.6' && git config user.email 'noreply@openai.com' && git worktree add /home/ubuntu/wt -b ${branch} origin/main" || echo "WARN worktree failed"
mkdir -p /home/ubuntu/.codex && chown -R ubuntu:ubuntu /home/ubuntu/.codex
echo done > /root/CODEX_SETUP_DONE
sync
UD
}

# =======================================================================================
# Hermetic self-test (no aws / no network).
# =======================================================================================
self_test() {
  local fails=0
  _check() { if [ "$2" = "$3" ]; then printf '  ok   %s\n' "$1"; else printf '  FAIL %s\n       got:  %q\n       want: %q\n' "$1" "$2" "$3"; fails=$((fails+1)); fi; }
  log "running --self-test (hermetic)"

  _check "clamp below lo"       "$(clamp 5 600 10800)"     "600"
  _check "clamp above hi"       "$(clamp 99999 600 10800)" "10800"
  _check "clamp non-int -> lo"  "$(clamp xyz 1 4)"         "1"
  _check "clamp fleet hi"       "$(clamp 9 1 4)"           "4"

  _check "slug simple"          "$(slugify sq-g3pji)"      "sq-g3pji"
  _check "slug keeps dots"      "$(slugify sq-6xasp.3)"    "sq-6xasp.3"
  _check "slug strips spaces"   "$(slugify 'sq/ab cd ef')" "sq-ab-cd-ef"

  local got
  got="$(printf '%s\n%s\n%s\n' "i-realbead0000000ab" "$PROD_INSTANCE" "$DEV_INSTANCE" | drop_excluded)"
  _check "prod+dev dropped, real kept" "$got" "i-realbead0000000ab"
  got="$(printf '%s\n%s\n' "$PROD_INSTANCE" "$DEV_INSTANCE" | drop_excluded)"
  _check "only prod+dev => empty" "$got" ""

  _check "tag distinct"         "$TAG_VALUE" "sparq-codex-worker"
  _check "never-touch count"    "${#NEVER_TOUCH[@]}" "2"
  _check "spot off by default"  "$USE_SPOT" "0"

  local ud; ud="$(render_userdata sq-test feat/sq-test-codex)"
  _grep() { if printf '%s' "$ud" | grep -qF -- "$2"; then _check "$1" y y; else _check "$1" n y; fi; }
  _grep "ud: detached sleep watchdog" "( sleep ${MAX_LIFETIME_SEC}; shutdown -h now ) &"
  _grep "ud: systemd-run watchdog"    "systemd-run --on-active=${MAX_LIFETIME_SEC}s"
  _grep "ud: installs codex"          "npm i -g @openai/codex"
  _grep "ud: installs rust"           "sh.rustup.rs"
  _grep "ud: worktree branch"         "git worktree add /home/ubuntu/wt -b feat/sq-test-codex"
  _grep "ud: setup sentinel"          "/root/CODEX_SETUP_DONE"
  _check "ud: exactly 2 shutdown calls" "$(printf '%s' "$ud" | grep -c 'shutdown -h now')" "2"

  echo
  if [ "$fails" -eq 0 ]; then log "self-test PASSED"; return 0; fi
  die "self-test FAILED ($fails check(s))"
}

# =======================================================================================
# Arg parse
# =======================================================================================
MODE="run"; DRY=0; APPLY=0; BEAD=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --self-test)    self_test; exit 0 ;;
    --orphan-check) MODE="orphan" ;;
    --apply)        APPLY=1 ;;
    --dry-run)      DRY=1 ;;
    -h|--help)      sed -n '2,40p' "$0"; exit 0 ;;
    -*)             die "unknown flag: $1" ;;
    *)              BEAD="$1" ;;
  esac
  shift
done

[ "$MODE" = "orphan" ] && { orphan_check "$APPLY"; exit 0; }
[ -n "$BEAD" ] || die "usage: $PROG <bead-id>  (or --dry-run / --self-test / --orphan-check)"
command -v aws >/dev/null 2>&1 || die "aws CLI not found"
[ -f "$CODEX_HOME_DIR/auth.json" ]  || die "missing $CODEX_HOME_DIR/auth.json (codex not authed on this box)"
[ -f "$CODEX_HOME_DIR/config.toml" ] || die "missing $CODEX_HOME_DIR/config.toml"
[ -f "$BRIEF_TEMPLATE" ] || die "missing brief template $BRIEF_TEMPLATE"

SLUG="$(slugify "$BEAD")"
BRANCH="feat/${SLUG}-codex-gpt56"
log "bead=$BEAD slug=$SLUG branch=$BRANCH region=$REGION type=$ITYPE spot=$USE_SPOT max-life=${MAX_LIFETIME_SEC}s cap=$MAX_FLEET"

# ---- COST-CAP ------------------------------------------------------------------------
CURRENT="$(fleet_count)"; log "live sparq-codex-worker fleet: $CURRENT / $MAX_FLEET"
[ "$CURRENT" -ge "$MAX_FLEET" ] && die "COST-CAP: $CURRENT >= $MAX_FLEET — refusing to launch."
orphan_check 0 || true

USERDATA="$(render_userdata "$BEAD" "$BRANCH")"
BRIEF="$(sed "s/__BEAD__/${BEAD}/g" "$BRIEF_TEMPLATE")"
# Inline the bead spec (and its parent epic, if any) from THIS box's bd — the instance may not
# have bd, and the spec is load-bearing. Appended as a verbatim block so codex never needs bd.
if command -v bd >/dev/null 2>&1; then
  BEAD_SPEC="$(bd show "$BEAD" 2>/dev/null || true)"
  if [ -n "$BEAD_SPEC" ]; then
    BRIEF="${BRIEF}

## BEAD SPEC (inlined from the controller's bd — this is your authoritative spec):
${BEAD_SPEC}"
  fi
fi

if [ "$DRY" -eq 1 ]; then
  log "DRY-RUN: would launch 1x $ITYPE (spot=$USE_SPOT) tagged ${TAG_KEY}=${TAG_VALUE} in $REGION for bead $BEAD."
  log "DRY-RUN: --instance-initiated-shutdown-behavior=terminate; watchdog ${MAX_LIFETIME_SEC}s; branch $BRANCH."
  log "DRY-RUN: ----- user-data -----"; printf '%s\n' "$USERDATA" >&2
  log "DRY-RUN: ----- brief (first 20 lines) -----"; printf '%s\n' "$BRIEF" | head -20 >&2
  log "DRY-RUN: nothing launched."; exit 0
fi

mkdir -p "$BUNDLE_DIR"
WORK="$(mktemp -d)"; KEYFILE="$WORK/key"; KEY_NAME="sparq-codex-worker-$$-${RANDOM}"
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
  # [OPUS-4.8] final belt-and-braces sweep ONLY in solo mode. In concurrent mode (MAX_FLEET>1)
  # this indiscriminate tag-wide sweep would terminate LIVE SIBLING instances (it killed
  # sq-1ijoc/sq-5ruwm when sq-250si's launcher declined and exited) — the cleanup above already
  # terminated THIS launcher's own instance; real leaks are caught by tick-level manual sweeps.
  [ "${MAX_FLEET:-1}" = 1 ] && orphan_check 1 || true
}
trap cleanup EXIT

log "resolve AMI / network in $REGION"
AMI=$(aws ec2 describe-images --region "$REGION" --owners 099720109477 \
  --filters "Name=name,Values=ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-arm64-server-*" "Name=state,Values=available" \
  --query 'sort_by(Images,&CreationDate)[-1].ImageId' --output text)
VPC=$(aws ec2 describe-vpcs --region "$REGION" --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)
SUBNET=$(aws ec2 describe-subnets --region "$REGION" --filters Name=vpc-id,Values="$VPC" "Name=default-for-az,Values=true" --query 'Subnets[0].SubnetId' --output text)
MYIP=$(curl -s https://checkip.amazonaws.com | tr -d '[:space:]')
[ -n "$MYIP" ] || die "could not determine public IP"
log "AMI=$AMI VPC=$VPC SUBNET=$SUBNET MYIP=$MYIP"

aws ec2 import-key-pair --region "$REGION" --key-name "$KEY_NAME" --public-key-material "fileb://${KEYFILE}.pub" >/dev/null
SG_ID=$(aws ec2 create-security-group --region "$REGION" --group-name "$KEY_NAME" --description "sparq codex worker (ephemeral)" --vpc-id "$VPC" --query 'GroupId' --output text)
aws ec2 authorize-security-group-ingress --region "$REGION" --group-id "$SG_ID" --protocol tcp --port 22 --cidr "${MYIP}/32" >/dev/null

MARKET=(); [ "$USE_SPOT" = "1" ] && MARKET=(--instance-market-options 'MarketType=spot')

log "launching 1x $ITYPE (spot=$USE_SPOT) tagged ${TAG_KEY}=${TAG_VALUE}"
INSTANCE_ID=$(aws ec2 run-instances --region "$REGION" --image-id "$AMI" --instance-type "$ITYPE" \
  --instance-initiated-shutdown-behavior terminate \
  --key-name "$KEY_NAME" --security-group-ids "$SG_ID" --subnet-id "$SUBNET" --associate-public-ip-address \
  "${MARKET[@]}" \
  --block-device-mappings "[{\"DeviceName\":\"/dev/sda1\",\"Ebs\":{\"VolumeSize\":${DISK_GB},\"VolumeType\":\"gp3\",\"DeleteOnTermination\":true}}]" \
  --tag-specifications "ResourceType=instance,Tags=[{Key=${TAG_KEY},Value=${TAG_VALUE}},{Key=bead,Value=$(printf '%s' "$BEAD" | tr -c 'A-Za-z0-9._-' '_')}]" \
  --user-data "$USERDATA" \
  --query 'Instances[0].InstanceId' --output text)
case "$INSTANCE_ID" in i-*) ;; *) INSTANCE_ID=""; die "run-instances failed (VcpuLimit / no capacity?)";; esac
log "launched INSTANCE_ID=$INSTANCE_ID"
aws ec2 wait instance-running --region "$REGION" --instance-ids "$INSTANCE_ID"
IP=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)
log "public IP=$IP; waiting for sshd"
SSH_UP=0
# shellcheck disable=SC2086
for i in $(seq 1 40); do ssh $SSHO "ubuntu@$IP" true 2>/dev/null && { log "ssh up (attempt $i)"; SSH_UP=1; break; }; sleep 10; done
[ "$SSH_UP" = 1 ] || die "sshd never reachable"

log "waiting for CODEX_SETUP_DONE (node+codex+rust+clone+worktree)…"
SETUP=0
for i in $(seq 1 60); do
  # shellcheck disable=SC2086
  ssh $SSHO "ubuntu@$IP" "sudo test -f /root/CODEX_SETUP_DONE" 2>/dev/null && { SETUP=1; log "  [$i] setup done"; break; }
  sleep 15
done
[ "$SETUP" = 1 ] || { ssh $SSHO "ubuntu@$IP" "sudo tail -80 /var/log/codex-worker.log" 2>/dev/null >&2 || true; die "setup did not finish"; }

# ---- copy codex auth (contents never logged) + the brief -----------------------------
log "scp codex auth + brief to instance"
# shellcheck disable=SC2086
scp $SSHO "$CODEX_HOME_DIR/auth.json" "$CODEX_HOME_DIR/config.toml" "ubuntu@$IP:/home/ubuntu/.codex/" >/dev/null 2>&1 || die "scp of codex auth failed"
# shellcheck disable=SC2086
ssh $SSHO "ubuntu@$IP" "chmod 600 /home/ubuntu/.codex/auth.json" 2>/dev/null || true
printf '%s\n' "$BRIEF" > "$WORK/brief.txt"
# shellcheck disable=SC2086
scp $SSHO "$WORK/brief.txt" "ubuntu@$IP:/home/ubuntu/brief.txt" >/dev/null 2>&1 || die "scp of brief failed"

# ---- run codex on the bead (in the worktree) -----------------------------------------
log "running codex on bead $BEAD (this is the long step; watchdog cap ${MAX_LIFETIME_SEC}s)…"
# codex reads the brief from stdin; runs inside the worktree; danger-full-access (already sandboxed by EC2).
REMOTE_RUN='cd /home/ubuntu/wt && export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH" && \
  timeout 6000 codex exec --dangerously-bypass-approvals-and-sandbox - < /home/ubuntu/brief.txt > /home/ubuntu/codex.log 2>&1; \
  echo "CODEX_EXIT=$?" >> /home/ubuntu/codex.log; \
  git -C /home/ubuntu/wt rev-parse --abbrev-ref HEAD > /home/ubuntu/branch.txt 2>/dev/null || true; \
  git -C /home/ubuntu/wt log --oneline -1 > /home/ubuntu/head.txt 2>/dev/null || true; \
  git -C /home/ubuntu/wt bundle create /home/ubuntu/result.bundle origin/main..HEAD 2>/home/ubuntu/bundle.err || \
    git -C /home/ubuntu/wt bundle create /home/ubuntu/result.bundle HEAD 2>>/home/ubuntu/bundle.err; \
  echo BUNDLE_DONE > /home/ubuntu/CODEX_DONE; sync'
# shellcheck disable=SC2086
ssh $SSHO "ubuntu@$IP" "$REMOTE_RUN" 2>/dev/null || log "remote codex run returned non-zero (will still pull logs)"

# ---- pull results: the bundle + codex log + branch/head ------------------------------
log "pulling codex.log tail (for the record):"
# shellcheck disable=SC2086
ssh $SSHO "ubuntu@$IP" "tail -80 /home/ubuntu/codex.log" 2>/dev/null >&2 || true
CODEX_RESULT_LINE="$(ssh $SSHO "ubuntu@$IP" "grep -a '^CODEX_RESULT=' /home/ubuntu/codex.log | tail -1" 2>/dev/null || true)"
REMOTE_BRANCH="$(ssh $SSHO "ubuntu@$IP" "cat /home/ubuntu/branch.txt 2>/dev/null" 2>/dev/null | tr -d '[:space:]' || true)"
BUNDLE_LOCAL="$BUNDLE_DIR/${SLUG}.bundle"
LOG_LOCAL="$BUNDLE_DIR/${SLUG}.codex.log"
# shellcheck disable=SC2086
scp $SSHO "ubuntu@$IP:/home/ubuntu/codex.log" "$LOG_LOCAL" >/dev/null 2>&1 || true
# shellcheck disable=SC2086
if scp $SSHO "ubuntu@$IP:/home/ubuntu/result.bundle" "$BUNDLE_LOCAL" >/dev/null 2>&1 && [ -s "$BUNDLE_LOCAL" ]; then
  log "bundle pulled -> $BUNDLE_LOCAL ($(wc -c < "$BUNDLE_LOCAL") bytes)"
else
  log "NO bundle came back (codex may not have committed) — see $LOG_LOCAL"
  BUNDLE_LOCAL=""
fi

log "CODEX_RESULT: ${CODEX_RESULT_LINE:-<none printed>}"
log "remote branch: ${REMOTE_BRANCH:-<unknown>}"
log "done on the instance; cleanup trap terminates $INSTANCE_ID + deletes keypair/SG + final sweep."

# The cleanup trap terminates the instance HERE (EXIT). The PR is opened from the local box below,
# after the instance is gone — everything we need (bundle + logs + branch) is already local.
trap - EXIT
cleanup

# =======================================================================================
# LOCAL side: fetch the bundle into a worktree, push from THIS box, open the PR. No arming.
# =======================================================================================
[ -n "$BUNDLE_LOCAL" ] && [ -s "$BUNDLE_LOCAL" ] || die "no committed branch to publish (empty bundle). See $LOG_LOCAL"

# HONEST-DECLINE GUARD: if codex escalated (committed=false / needs_architect) do NOT push a
# commit-less branch. `git bundle create origin/main..HEAD` FALLS BACK to bundling all of HEAD
# when the range is empty, so a fat bundle does NOT imply a real change — check the RESULT line.
case "$CODEX_RESULT_LINE" in
  *'"committed":false'*|*'"needs_architect":true'*|*'"gates_green":false'*)
    log "codex did NOT produce a committable change (escalated/declined). NO PR opened."
    log "reason (from codex): $CODEX_RESULT_LINE"
    printf 'PR_URL=null (codex declined/escalated — see %s)\n' "$LOG_LOCAL"
    exit 0 ;;
esac

REPO_ROOT="${SPARQ_REPO_ROOT:-/home/ubuntu/sparq}"
cd "$REPO_ROOT"
git fetch -q origin main
LOCAL_BRANCH="${REMOTE_BRANCH:-$BRANCH}"
log "fetching bundle branch '$LOCAL_BRANCH' from the bundle"
git fetch -q "$BUNDLE_LOCAL" "HEAD:refs/heads/${LOCAL_BRANCH}" 2>/dev/null \
  || git fetch -q "$BUNDLE_LOCAL" "${LOCAL_BRANCH}:refs/heads/${LOCAL_BRANCH}" 2>/dev/null \
  || die "could not fetch branch from bundle $BUNDLE_LOCAL"
# Belt-and-braces: refuse to push/open a PR when the branch has NO commits ahead of origin/main.
AHEAD="$(git rev-list --count origin/main.."refs/heads/${LOCAL_BRANCH}" 2>/dev/null || echo 0)"
if [ "${AHEAD:-0}" -lt 1 ]; then
  git branch -D "${LOCAL_BRANCH}" >/dev/null 2>&1 || git update-ref -d "refs/heads/${LOCAL_BRANCH}" 2>/dev/null || true
  log "bundle branch has 0 commits ahead of origin/main — nothing to publish. NO PR opened."
  printf 'PR_URL=null (no commits ahead of main — see %s)\n' "$LOG_LOCAL"
  exit 0
fi
log "branch has $AHEAD commit(s) ahead of main — pushing from this box"
git push -q -u origin "${LOCAL_BRANCH}" 2>&1 | tail -3 >&2 || die "push failed for ${LOCAL_BRANCH}"

# Compose the PR body: SPARQ-GPT-5.6 self-id blockquote + bead cite + a codex-result note.
BEAD_TITLE="$(command -v bd >/dev/null 2>&1 && bd show "$BEAD" 2>/dev/null | head -1 | sed 's/^[^·]*· //; s/  *\[.*$//' || echo "$BEAD")"
PR_TITLE="$(printf '%s' "$BEAD_TITLE" | cut -c1-70) [GPT-5.6]"
PR_BODY="$(cat <<PB
> 🤖 SPARQ agent — written by **GPT-5.6** (codex-cli, run on an ephemeral EC2 worker).

Implements bead **${BEAD}** — ${BEAD_TITLE}.

GPT-5.6 (codex) authored the change on an ephemeral EC2 arm64 instance: it implemented the
bead, ran the crate-scoped gates (clippy -D warnings + tests in both feature states + rustdoc),
and committed the branch. The controller pulled the commit back as a git bundle and pushed +
opened this PR from the trusted box (no gh auth on the worker).

codex final result line:
\`\`\`
${CODEX_RESULT_LINE:-<not printed — see run log>}
\`\`\`

Not armed — the orchestrator arms after review.
PB
)"
log "opening PR (title: $PR_TITLE)"
PR_URL="$(gh pr create --repo sparq-org/sparq --base main --head "${LOCAL_BRANCH}" --title "$PR_TITLE" --body "$PR_BODY" 2>&1 | tail -1 || true)"
log "PR: $PR_URL"
printf 'PR_URL=%s\n' "$PR_URL"
