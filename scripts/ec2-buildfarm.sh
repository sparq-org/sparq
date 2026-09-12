#!/usr/bin/env bash
# [OPUS-4.8] sparq EC2 BUILD FARM launcher. Authored by Opus 4.8 (1M context); Fable
# unavailable — flag for re-review when Fable returns.
#
#   scripts/ec2-buildfarm.sh <branch|#PR> [region]      # launch + gate + self-terminate
#   scripts/ec2-buildfarm.sh --dry-run <branch|#PR>      # print plan, launch NOTHING
#   scripts/ec2-buildfarm.sh --self-test                 # hermetic unit test (no aws)
#   scripts/ec2-buildfarm.sh --orphan-check [--apply]    # find/terminate buildfarm orphans
#
# PURPOSE. The local 8-core work box is the throughput ceiling for `cargo` gating: it can
# sustain only ~6 parallel cargo-heavy agents before it thrashes. This launcher offloads the
# *merge gate* of a single branch to an EPHEMERAL EC2 instance so many branches can be gated
# in parallel beyond that ceiling. It is a BUILD farm — build + clippy(-D warnings) + test of
# ONE branch in BOTH feature states — and is COMPLETELY SEPARATE from:
#   * the per-commit perf CI on EC2 (purpose=sparq-ci-bench, scripts/ec2-bench.sh), and
#   * the competitor gather / same-box bench farm (purpose=sparq-bench, scripts/gather-ec2.sh,
#     scripts/qlever-same-box.sh — the sq-sxso same-box competitor lane).
# Distinct purpose tag (sparq-buildfarm) keeps the three farms from ever colliding: each
# launcher and orphan-checker only ever sees its own tag (allow-list, not deny-list).
#
# THE GATE it reproduces (faithful to AGENTS.md "Merge discipline" + ci.yml/feature-matrix.yml):
#   1. cargo build --workspace
#   2. cargo clippy --workspace --exclude sparq-py --all-targets -- -D warnings    (GATING)
#   3. cargo test  --workspace                                                     (default features)
#   4. cargo test --workspace --features "${ARCHIVE_FEATURES}" (CI archive opt-in set)
#      + cargo clippy ... --features <archive set> -- -D warnings
# (3) and (4) are the BOTH feature states the contract requires: default-OFF, then the canonical
# CI opt-in feature set (the same set ci.yml's nextest archive + doctests carry). fmt is run
# --check but kept ADVISORY (matches ci.yml: rustfmt is informational until the deferred
# `cargo fmt --all` reformat lands). A non-zero gate => the instance writes FAIL into the result.
#
# ORPHAN-PROOF (agent-death-independent; >=2 independent hard caps + shutdown=terminate; §safety):
#   --instance-initiated-shutdown-behavior terminate   any in-box shutdown -> the box is DESTROYED
#   + ( sleep $MAX_LIFETIME_SEC; shutdown -h now ) &    detached, no deps, arms immediately
#   + systemd-run --on-active=${MAX_LIFETIME_SEC}s ...  transient timer, survives the shell exit
# The user-data writes the gate result + a /root/BUILD_DONE sentinel and then STOPS — it does NOT
# shut down. Only the watchdog auto-terminates (orphan backstop). The orchestrator SSH-pulls the
# result while the box is ALIVE, then terminates EXPLICITLY (cleanup trap). So: a result is never
# lost to a shutdown race, AND an instance can never silently outlive its job even if THIS process
# (the controller) is killed mid-run.
#
# COST-CAP (hard, ENFORCED before launch): MAX_FLEET concurrent buildfarm instances. The launcher
# counts running+pending purpose=sparq-buildfarm instances and REFUSES to launch a (count+1)-th.
# Spot by default (much cheaper, safe because the box is disposable and idempotently re-launchable).
# See the cost model in research/ec2-build-farm-design.md.
#
# SAFETY: profile pss, region eu-west-2, tag purpose=sparq-buildfarm. NEVER touches prod
# (i-090531b4ede8f2d3f) or dev (i-00f76802f345b6b77) — the orphan-check hard-excludes both, and
# this launcher only ever terminates the ONE instance id it itself launched. Orphan-checks BEFORE
# launch (to enforce the cap honestly) and AFTER (cleanup trap + a final sweep hint).
set -euo pipefail

PROG="ec2-buildfarm"
log()  { printf '[%s] %s\n' "$PROG" "$*" >&2; }
die()  { printf '[%s] ERROR: %s\n' "$PROG" "$*" >&2; exit 1; }

# ---- tunables (overridable by env) ----------------------------------------------------
REGION_DEFAULT="${AWS_REGION:-eu-west-2}"
# c7g.4xlarge: 16 vCPU arm64 (Graviton3). cargo build of the workspace is heavily
# parallel, so a 16-vCPU box gates a branch in a few minutes. arm64 spot ~ $0.25/hr in
# eu-west-2 (see cost model). Override with BUILDFARM_INSTANCE_TYPE.
ITYPE="${BUILDFARM_INSTANCE_TYPE:-c7g.4xlarge}"
# Canonical CI opt-in feature set (the second "feature state"). Matches ci.yml's archive +
# doctests + the feature-matrix legs that ride these three. Override with BUILDFARM_FEATURES.
ARCHIVE_FEATURES="${BUILDFARM_FEATURES:-approx-ann,filtered-ann,vec-predicate}"
# Hard max lifetime: the watchdog terminates the box after this many seconds NO MATTER WHAT.
# 2h is generous headroom over a from-scratch build+clippy+test-x2 (~20-40 min) while bounding
# a worst-case orphan to <=2h of spend. Override with BUILDFARM_MAX_LIFETIME_SEC (clamped 600..10800).
MAX_LIFETIME_SEC="${BUILDFARM_MAX_LIFETIME_SEC:-7200}"
# Hard concurrency ceiling across the whole buildfarm fleet (the cost cap). Refuse to launch
# beyond this many running/pending purpose=sparq-buildfarm instances. Override with BUILDFARM_MAX_FLEET
# (clamped 1..16 — 16 is the documented ceiling; do not raise without re-doing the cost model).
MAX_FLEET="${BUILDFARM_MAX_FLEET:-12}"
USE_SPOT="${BUILDFARM_SPOT:-1}"            # 1=spot (default, cheap), 0=on-demand
DISK_GB="${BUILDFARM_DISK_GB:-40}"
REPO="https://github.com/sparq-org/sparq.git"

# ---- tag + protected ids (mirror orphan-check-bench.sh's hard-exclusion discipline) ----
readonly TAG_KEY="purpose"
readonly TAG_VALUE="sparq-buildfarm"          # DISTINCT from sparq-bench / sparq-ci-bench
readonly PROD_INSTANCE="i-090531b4ede8f2d3f"
readonly DEV_INSTANCE="i-00f76802f345b6b77"
readonly -a NEVER_TOUCH=("$PROD_INSTANCE" "$DEV_INSTANCE")

# Clamp an integer into [lo,hi]; non-integer -> lo. Pure-text, testable.
clamp() { # <val> <lo> <hi>
  local v="$1" lo="$2" hi="$3"
  case "$v" in (''|*[!0-9]*) v="$lo" ;; esac
  [ "$v" -lt "$lo" ] && v="$lo"
  [ "$v" -gt "$hi" ] && v="$hi"
  printf '%s' "$v"
}
MAX_LIFETIME_SEC="$(clamp "$MAX_LIFETIME_SEC" 600 10800)"
MAX_FLEET="$(clamp "$MAX_FLEET" 1 16)"

# Resolve a #PR / 123 / PR-123 token to its head ref; pass a plain branch through unchanged.
# Needs `gh` only for the PR form. Pure shell except the gh lookup; testable on the branch path.
resolve_ref() { # <token> -> prints a git ref (branch name or pull/<n>/head)
  local t="$1" n=""
  case "$t" in
    '#'[0-9]*)  n="${t#\#}" ;;
    PR-[0-9]*)  n="${t#PR-}" ;;
    pr-[0-9]*)  n="${t#pr-}" ;;
    [0-9]*)     # a bare number is ambiguous; treat as a PR number (branches are named, not numeric)
                case "$t" in (*[!0-9]*) printf '%s' "$t"; return 0 ;; esac
                n="$t" ;;
    *)          printf '%s' "$t"; return 0 ;;     # a normal branch name
  esac
  # PR form: ask gh for the head ref of PR #n in sparq-org/sparq.
  if command -v gh >/dev/null 2>&1; then
    local head
    head="$(gh pr view "$n" --repo sparq-org/sparq --json headRefName --jq .headRefName 2>/dev/null || true)"
    [ -n "$head" ] && { printf '%s' "$head"; return 0; }
  fi
  # Fallback: fetch the immutable pull/<n>/head ref directly (works without gh).
  printf 'pull/%s/head' "$n"
}

# Drop hard-excluded ids from a newline-separated id list on stdin (belt-and-braces for orphan-check).
drop_excluded() {
  local ids id keep ex; ids="$(cat)"
  while IFS= read -r id; do
    [ -n "$id" ] || continue
    keep=1
    for ex in "${NEVER_TOUCH[@]}"; do [ "$id" = "$ex" ] && { keep=0; break; }; done
    [ "$keep" -eq 1 ] && printf '%s\n' "$id"
  done <<< "$ids"
  return 0
}

# Count running/pending buildfarm instances (the live fleet size) for the cost cap.
fleet_count() { # <region> -> integer
  local region="$1" out
  out="$(aws ec2 describe-instances --region "$region" \
    --filters "Name=tag:${TAG_KEY},Values=${TAG_VALUE}" \
              "Name=instance-state-name,Values=running,pending" \
    --query 'Reservations[].Instances[].InstanceId' --output text 2>/dev/null \
    | tr '\t' '\n' | sed '/^$/d' | wc -l | tr -d '[:space:]')"
  case "$out" in (''|*[!0-9]*) out=0 ;; esac
  printf '%s' "$out"
}

# Render the orphan-proof user-data for a resolved ref. Pure-text so the self-test can assert it.
render_userdata() { # <ref> -> the cloud-init script on stdout
  local ref="$1"
  cat <<UD
#!/bin/bash
set -x
exec > >(tee /var/log/buildfarm.log) 2>&1
# [OPUS-4.8] ORPHAN-PROOF: TWO independent hard caps, each from a DIFFERENT subsystem, BOTH
# armed before any apt-get so a single failure cannot leave the box running past ${MAX_LIFETIME_SEC}s:
#   (1) detached sleep subshell — works immediately, no package deps.
#   (2) systemd-run transient timer — survives the user-data shell exiting; pre-installed on the AMI.
# Combined with --instance-initiated-shutdown-behavior=terminate, any shutdown DESTROYS the box.
( sleep ${MAX_LIFETIME_SEC}; shutdown -h now ) &
systemd-run --on-active=${MAX_LIFETIME_SEC}s /sbin/shutdown -h now || true

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq build-essential pkg-config git curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --component clippy rustfmt
export PATH="/root/.cargo/bin:\$PATH"
. /root/.cargo/env || true
for _ in \$(seq 1 30); do command -v cargo >/dev/null 2>&1 && break; sleep 2; done
command -v cargo || echo "WARN: cargo still not on PATH"

cd /root
git clone -q "$REPO" sparq
cd sparq
# Fetch the requested ref. A PR head comes in as pull/<n>/head -> a local FETCH_HEAD checkout;
# a branch name fetches origin/<branch>. Either way we end up on a detached HEAD at the tip.
git fetch -q origin "$ref" || git fetch -q origin "$ref:_buildfarm_ref" || true
git checkout -q FETCH_HEAD 2>/dev/null || git checkout -q "$ref" 2>/dev/null || git checkout -q _buildfarm_ref 2>/dev/null || true
SHA=\$(git rev-parse --short HEAD 2>/dev/null || echo UNKNOWN)
echo "BUILDFARM_REF=$ref SHA=\$SHA"

# ---- THE GATE (faithful to AGENTS.md merge discipline) -------------------------------
RC=0
run() { echo "::: \$*"; "\$@" || { echo "::: ^ FAILED (\$?)"; RC=1; }; }

# 1. build (default features)
run cargo build --workspace
# 2. clippy -D warnings (GATING) — default features
run cargo clippy --workspace --exclude sparq-py --all-targets -- -D warnings
# 3. test — FEATURE STATE A: default features
run cargo test --workspace
# 4. FEATURE STATE B: the canonical CI opt-in feature set (build+clippy+test)
run cargo build  --workspace --features ${ARCHIVE_FEATURES}
run cargo clippy --workspace --exclude sparq-py --all-targets --features ${ARCHIVE_FEATURES} -- -D warnings
run cargo test   --workspace --features ${ARCHIVE_FEATURES}
# fmt: ADVISORY (matches ci.yml — rustfmt informational until the deferred reformat lands)
echo "::: cargo fmt --all --check (ADVISORY, non-gating)"
cargo fmt --all --check || echo "::: ^ fmt drift (advisory only — does NOT fail the gate)"

if [ "\$RC" -eq 0 ]; then VERDICT=PASS; else VERDICT=FAIL; fi
echo "BUILDFARM_VERDICT=\$VERDICT (ref=$ref sha=\$SHA)"
# Machine-readable result file the orchestrator pulls.
printf '{"ref":"%s","sha":"%s","verdict":"%s","features_b":"%s"}\n' "$ref" "\$SHA" "\$VERDICT" "${ARCHIVE_FEATURES}" > /root/buildfarm-result.json
# Also emit to the serial console so a result survives even if the SSH pull fails.
echo "===BUILDFARM-RESULT-BEGIN==="; cat /root/buildfarm-result.json; echo "===BUILDFARM-RESULT-END==="
sync
# sentinel LAST — the orchestrator waits for this, pulls, then terminates. We do NOT shut
# down here; the watchdog is the only auto-terminate (orphan backstop).
echo "\$VERDICT" > /root/BUILD_DONE
sync
UD
}

# =======================================================================================
# Hermetic self-test (no aws, no network) — asserts the load-bearing invariants.
# =======================================================================================
self_test() {
  local fails=0 got
  _check() { if [ "$2" = "$3" ]; then printf '  ok   %s\n' "$1"; else printf '  FAIL %s\n       got:  %q\n       want: %q\n' "$1" "$2" "$3"; fails=$((fails+1)); fi; }
  log "running --self-test (hermetic; no aws calls)"

  # clamp() bounds.
  _check "clamp below lo"           "$(clamp 5 600 10800)"     "600"
  _check "clamp above hi"           "$(clamp 99999 600 10800)" "10800"
  _check "clamp in band"            "$(clamp 7200 600 10800)"  "7200"
  _check "clamp non-int -> lo"      "$(clamp xyz 1 16)"        "1"
  _check "clamp fleet hi"           "$(clamp 20 1 16)"         "16"

  # resolve_ref() on the forms that do not need gh (the branch path + PR-token shapes
  # fall back to pull/<n>/head when gh is absent — assert that deterministic fallback).
  _check "branch passthrough"       "$(resolve_ref my-feature-branch)" "my-feature-branch"
  _check "branch with slash"        "$(resolve_ref feat/x-y)"          "feat/x-y"
  if ! command -v gh >/dev/null 2>&1; then
    _check "#123 -> pull/123/head"  "$(resolve_ref '#123')"  "pull/123/head"
    _check "PR-7 -> pull/7/head"    "$(resolve_ref 'PR-7')"  "pull/7/head"
  else
    printf '  skip resolve_ref PR forms (gh present; would hit network)\n'
  fi

  # drop_excluded(): prod/dev are NEVER in the kill set.
  got="$(printf '%s\n%s\n%s\n' "i-bbbb222233334444b" "$PROD_INSTANCE" "$DEV_INSTANCE" | drop_excluded)"
  _check "prod+dev dropped, real kept" "$got" "i-bbbb222233334444b"
  got="$(printf '%s\n%s\n' "$PROD_INSTANCE" "$DEV_INSTANCE" | drop_excluded)"
  _check "only prod+dev => empty"      "$got" ""

  # tag is the DISTINCT buildfarm tag (must not collide with the bench farms).
  _check "tag value distinct"       "$TAG_VALUE" "sparq-buildfarm"
  _check "never-touch count"        "${#NEVER_TOUCH[@]}" "2"

  # user-data contains BOTH orphan-proof mechanisms, the shutdown=terminate companion is set
  # at run-instances (asserted separately), and BOTH feature states + the GATING clippy flag.
  local ud; ud="$(render_userdata pull/1/head)"
  _grep() { if printf '%s' "$ud" | grep -q -e "$2"; then _check "$1" y y; else _check "$1" n y; fi; }
  _grep "ud: detached sleep watchdog"   "( sleep ${MAX_LIFETIME_SEC}; shutdown -h now ) &"
  _grep "ud: systemd-run watchdog"      "systemd-run --on-active=${MAX_LIFETIME_SEC}s"
  _grep "ud: clippy -D warnings gating" "-D warnings"
  _grep "ud: feature state A (default)" 'cargo test --workspace$'
  _grep "ud: feature state B (opt-in)"  "cargo test   --workspace --features ${ARCHIVE_FEATURES}"
  _grep "ud: sentinel written"          "BUILD_DONE"
  # exactly the 2 watchdogs, no 3rd eager shutdown after the gate (no race that loses the result)
  _check "ud: exactly 2 shutdown calls (no eager 3rd)" "$(printf '%s' "$ud" | grep -c 'shutdown -h now')" "2"

  echo
  if [ "$fails" -eq 0 ]; then log "self-test PASSED"; return 0; fi
  die "self-test FAILED ($fails check(s))"
}

# =======================================================================================
# Orphan check — find/terminate buildfarm orphans (same allow-list + hard-exclude as bench).
# =======================================================================================
orphan_check() { # <region> <apply:0|1>
  local region="$1" apply="$2"
  command -v aws >/dev/null 2>&1 || { log "aws CLI not found — orphan check skipped (no-op)."; return 0; }
  log "orphan-check (region=$region): state in {running,pending} AND tag:${TAG_KEY}=${TAG_VALUE}"
  local raw
  raw="$(aws ec2 describe-instances --region "$region" \
    --filters "Name=tag:${TAG_KEY},Values=${TAG_VALUE}" \
              "Name=instance-state-name,Values=running,pending" \
    --query 'Reservations[].Instances[].InstanceId' --output text 2>/dev/null | tr '\t' '\n' | sed '/^$/d')" || {
    log "describe-instances failed (creds/region?) — no action."; return 0; }
  [ -n "$raw" ] || { log "no running/pending sparq-buildfarm instances — clean."; return 0; }
  local -a ids; mapfile -t ids <<< "$raw"
  log "tag-matched running/pending:"; printf '  %s\n' "${ids[@]}" >&2
  local -a kill_set; mapfile -t kill_set < <(printf '%s\n' "${ids[@]}" | drop_excluded)
  local ex
  for ex in "${NEVER_TOUCH[@]}"; do
    if printf '%s\n' "${ids[@]}" | grep -qxF "$ex"; then
      log "WARNING: protected $ex carries the buildfarm tag — HARD-EXCLUDED, NOT terminating."
    fi
  done
  [ "${#kill_set[@]}" -gt 0 ] || { log "after hard-exclusion, nothing to terminate."; return 0; }
  log "ORPHAN candidates: ${kill_set[*]}"
  if [ "$apply" -ne 1 ]; then log "dry-run (default): nothing terminated. Re-run --orphan-check --apply to terminate."; return 0; fi
  log "--apply: terminating: ${kill_set[*]}"
  if aws ec2 terminate-instances --region "$region" --instance-ids "${kill_set[@]}" >/dev/null 2>&1; then
    log "terminate submitted."
  else
    die "terminate FAILED for: ${kill_set[*]}"
  fi
}

# =======================================================================================
# Arg parse
# =======================================================================================
MODE="launch"; DRY=0; APPLY=0; REGION="$REGION_DEFAULT"; TOKEN=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --self-test)    self_test; exit 0 ;;
    --orphan-check) MODE="orphan" ;;
    --apply)        APPLY=1 ;;
    --dry-run)      DRY=1 ;;
    --region)       shift; [ "$#" -gt 0 ] || die "--region needs a value"; REGION="$1" ;;
    --region=*)     REGION="${1#--region=}" ;;
    -h|--help)      sed -n '2,55p' "$0"; exit 0 ;;
    -*)             die "unknown flag: $1 (try --dry-run, --self-test, --orphan-check, --apply, --region, --help)" ;;
    *)              TOKEN="$1" ;;
  esac
  shift
done

[ "$MODE" = "orphan" ] && { orphan_check "$REGION" "$APPLY"; exit 0; }

[ -n "$TOKEN" ] || die "usage: $PROG <branch|#PR> [region]  (or --dry-run / --self-test / --orphan-check)"
command -v aws >/dev/null 2>&1 || die "aws CLI not found"

REF="$(resolve_ref "$TOKEN")"
log "token=$TOKEN -> ref=$REF | region=$REGION | type=$ITYPE | spot=$USE_SPOT | max-life=${MAX_LIFETIME_SEC}s | cap=$MAX_FLEET"

# ---- COST-CAP: enforce the concurrency ceiling BEFORE launching ----------------------
CURRENT="$(fleet_count "$REGION")"
log "live buildfarm fleet: $CURRENT / $MAX_FLEET"
if [ "$CURRENT" -ge "$MAX_FLEET" ]; then
  die "COST-CAP: $CURRENT running/pending sparq-buildfarm instances >= cap $MAX_FLEET — refusing to launch. \
Wait for one to finish, or raise BUILDFARM_MAX_FLEET (<=16) after re-checking the cost model."
fi
# Orphan-check (advisory) BEFORE launch so the operator sees any leak; never auto-terminates here.
orphan_check "$REGION" 0 || true

USERDATA="$(render_userdata "$REF")"

if [ "$DRY" -eq 1 ]; then
  log "DRY-RUN: would launch 1x $ITYPE (spot=$USE_SPOT) tagged ${TAG_KEY}=${TAG_VALUE} in $REGION."
  log "DRY-RUN: --instance-initiated-shutdown-behavior=terminate; watchdog hard cap ${MAX_LIFETIME_SEC}s."
  log "DRY-RUN: gate = build + clippy(-D warnings) + test[default] + build/clippy/test[${ARCHIVE_FEATURES}]."
  log "DRY-RUN: ----- user-data that WOULD be sent -----"
  printf '%s\n' "$USERDATA" >&2
  log "DRY-RUN: nothing launched."
  exit 0
fi

# ---- real launch (mirrors gather-ec2.sh's proven, reviewed sequence) ------------------
WORK="$(mktemp -d)"; KEYFILE="$WORK/key"; KEY_NAME="sparq-buildfarm-$$-${RANDOM}"
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
trap cleanup EXIT   # ALWAYS terminate the one box we launched — even on error/cancel.

log "resolve AMI / network in $REGION"
AMI=$(aws ec2 describe-images --region "$REGION" --owners 099720109477 \
  --filters "Name=name,Values=ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-arm64-server-*" "Name=state,Values=available" \
  --query 'sort_by(Images,&CreationDate)[-1].ImageId' --output text)
VPC=$(aws ec2 describe-vpcs --region "$REGION" --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)
SUBNET=$(aws ec2 describe-subnets --region "$REGION" --filters Name=vpc-id,Values="$VPC" "Name=default-for-az,Values=true" --query 'Subnets[0].SubnetId' --output text)
MYIP=$(curl -s https://checkip.amazonaws.com | tr -d '[:space:]')
[ -n "$MYIP" ] || die "could not determine public IP (checkip returned empty)"
log "AMI=$AMI VPC=$VPC SUBNET=$SUBNET MYIP=$MYIP"

log "ephemeral keypair + locked-down SG (ssh from $MYIP/32 only)"
aws ec2 import-key-pair --region "$REGION" --key-name "$KEY_NAME" --public-key-material "fileb://${KEYFILE}.pub" >/dev/null
SG_ID=$(aws ec2 create-security-group --region "$REGION" --group-name "$KEY_NAME" --description "sparq buildfarm (ephemeral)" --vpc-id "$VPC" --query 'GroupId' --output text)
aws ec2 authorize-security-group-ingress --region "$REGION" --group-id "$SG_ID" --protocol tcp --port 22 --cidr "${MYIP}/32" >/dev/null

MARKET=()
[ "$USE_SPOT" = "1" ] && MARKET=(--instance-market-options 'MarketType=spot')

log "launching 1x $ITYPE (spot=$USE_SPOT) tagged ${TAG_KEY}=${TAG_VALUE}"
INSTANCE_ID=$(aws ec2 run-instances --region "$REGION" --image-id "$AMI" --instance-type "$ITYPE" \
  --instance-initiated-shutdown-behavior terminate \
  --key-name "$KEY_NAME" --security-group-ids "$SG_ID" --subnet-id "$SUBNET" --associate-public-ip-address \
  "${MARKET[@]}" \
  --block-device-mappings "[{\"DeviceName\":\"/dev/sda1\",\"Ebs\":{\"VolumeSize\":${DISK_GB},\"VolumeType\":\"gp3\",\"DeleteOnTermination\":true}}]" \
  --tag-specifications "ResourceType=instance,Tags=[{Key=${TAG_KEY},Value=${TAG_VALUE}},{Key=ref,Value=$(printf '%s' "$REF" | tr -c 'A-Za-z0-9._/-' '_')}]" \
  --user-data "$USERDATA" \
  --query 'Instances[0].InstanceId' --output text)
# run-instances can return ""/"None" on a failed launch (VcpuLimitExceeded, no spot capacity).
case "$INSTANCE_ID" in
  i-*) ;;
  *) INSTANCE_ID=""; die "run-instances did not return a valid instance id (launch failed — VcpuLimit / no capacity?) — aborting" ;;
esac
log "launched INSTANCE_ID=$INSTANCE_ID"
aws ec2 wait instance-running --region "$REGION" --instance-ids "$INSTANCE_ID"
IP=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)
log "public IP=$IP; waiting for sshd"
SSH_UP=0
# shellcheck disable=SC2086  # $SSHO is deliberately word-split into ssh options.
for i in $(seq 1 40); do ssh $SSHO "ubuntu@$IP" true 2>/dev/null && { log "ssh up (attempt $i)"; SSH_UP=1; break; }; sleep 10; done
[ "$SSH_UP" = 1 ] || die "sshd never reachable on $IP after 40 attempts — aborting (cleanup trap terminates $INSTANCE_ID)"

# ---- poll for the BUILD_DONE sentinel; pull the result while the box is ALIVE ---------
log "polling for /root/BUILD_DONE (build+clippy+test x2 ~ 20-40 min; watchdog cap ${MAX_LIFETIME_SEC}s)…"
DONE=0
for i in $(seq 1 120); do
  sleep 30
  # shellcheck disable=SC2086  # $SSHO is deliberately word-split into ssh options.
  if ssh $SSHO "ubuntu@$IP" "sudo test -f /root/BUILD_DONE" 2>/dev/null; then
    log "  [$i] sentinel present — pulling result"; DONE=1; break
  fi
  STATE=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" --query 'Reservations[0].Instances[0].State.Name' --output text 2>/dev/null || echo unknown)
  log "  [$i] state=$STATE; waiting for sentinel"
  [ "$STATE" = "terminated" ] && { log "  instance terminated before sentinel (spot reclaim / watchdog) — result lost; re-run."; break; }
done

VERDICT="UNKNOWN"
if [ "$DONE" = 1 ]; then
  # shellcheck disable=SC2086  # $SSHO is deliberately word-split into ssh options.
  RESULT_JSON="$(ssh $SSHO "ubuntu@$IP" "sudo cat /root/buildfarm-result.json" 2>/dev/null || true)"
  if [ -n "$RESULT_JSON" ]; then echo "$RESULT_JSON"; VERDICT="$(printf '%s' "$RESULT_JSON" | sed -n 's/.*"verdict":"\([A-Z]*\)".*/\1/p')"; fi
  log "fetching gate log tail for the record:"
  # shellcheck disable=SC2086  # $SSHO is deliberately word-split into ssh options.
  ssh $SSHO "ubuntu@$IP" "sudo tail -60 /var/log/buildfarm.log" 2>/dev/null >&2 || true
else
  log "NO sentinel — pulling /var/log/buildfarm.log for diagnosis"
  # shellcheck disable=SC2086  # $SSHO is deliberately word-split into ssh options.
  ssh $SSHO "ubuntu@$IP" "sudo tail -120 /var/log/buildfarm.log 2>/dev/null" >&2 || true
fi

log "GATE VERDICT for $REF: $VERDICT"
log "done; cleanup trap will terminate $INSTANCE_ID + delete keypair/SG"
# Exit non-zero on a failed/unknown gate so a caller (or a CI step) can branch on it.
[ "$VERDICT" = "PASS" ]
