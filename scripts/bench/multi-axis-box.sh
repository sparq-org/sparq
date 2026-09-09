#!/usr/bin/env bash
# [FABLE-5] sq-hmd7l.25 — bin-packed MULTI-AXIS canonical EC2 bench runner.
#
# WHY: the workload-triage cost discipline says small benchmark axes do NOT get one
# EC2 box each (research/comparative-benchmarking-everything.md §4, point 5). This
# runner provisions ONE dedicated quiet box (tag purpose=sparq-bench), runs an
# ordered list of per-axis same-box gathers SERIALLY — one axis, and therefore one
# engine, active at a time (the canonical protocol makes serial execution mandatory
# anyway) — collects the envelopes, and the box then self-terminates.
#
# LINEAGE: launch/teardown idioms from scripts/bench/canonical-competitor-bench.sh
# (ephemeral keypair/SG, sentinel-gated poll BELOW the watchdog, incremental SSH
# pull); console-result idioms from bench/ec2-bench.sh (SPARQ_BENCH_RESULT markers,
# the 64KB serial-buffer discipline, the ~8min Nitro console-snapshot dwell).
#
# ORPHAN-PROOF (the launcher can die at ANY point without leaking a box):
#   * --instance-initiated-shutdown-behavior terminate
#   * user-data watchdog FIRST LINE: ( sleep WATCHDOG_S; shutdown -h now ) &
#     plus a systemd-run backup — hard wall-clock cap (default 3h) no matter what.
#   * the box self-terminates after the axis list finishes (dwell, then shutdown);
#     the launcher poll deadline sits BELOW the watchdog and its EXIT trap
#     terminates the instance + deletes the ephemeral keypair/SG regardless.
#   * REFUSES to touch the protected prod/dev instance ids, ever.
#   * scripts/orphan-check-bench.sh is run (advisory dry-run) after a real launch;
#     it must come back clean after every run.
#
# RESULT CHANNELS (both):
#   1. console: each axis emits a `=== SPARQ_BENCH_RESULT <axis> ===` block
#      (provenance line + status + compact envelope JSON, per-envelope byte cap so
#      five axes fit the ~64KB serial buffer) inside one outer
#      `=== SPARQ_BENCH_RESULT id=multi-axis ... === / === END SPARQ_BENCH_RESULT ===`
#      range; the launcher saves the extracted range to RESULTS_LOCAL.
#   2. SSH: full (uncapped) envelopes are pulled incrementally from
#      /root/axis-results/<axis>/ into RESULTS_LOCAL while the run progresses.
#
# MODES
#   --dry-run    print the packed execution plan and exit; NO AWS call is made
#                (default when no mode flag is given, with a hint).
#   --launch     really provision the box and run the packed plan (costs money).
#   --instance   ON-BOX entrypoint (run by user-data as root from the cloned repo).
#                Contains NO shutdown/terminate call — self-termination lives in
#                user-data only, so a local smoke of --instance is harmless.
#
# AXES (ordered; wave-1 tier-1 set, bead sq-hmd7l.26): fts geo hdt update parse.
# fts/geo/hdt/update run scripts/bench/<axis>-same-box.sh (the shacl-same-box.sh
# template: TIMEOUT_S / CANONICAL / OUT_DIR / ONLY knobs); parse runs the
# bench/parse harness directly (its envelope wrapper is bead sq-hmd7l.6). An axis
# whose harness is not on the checked-out branch yet is SKIPPED with an honest
# `absent` status — never a fabricated row.
#
# ENV overrides (all defaulted):
#   AWS_PROFILE(=pss) REGION(=eu-west-2) ITYPE(=c6i.4xlarge, the canonical
#   quiet-box class) EBS_GB(=80) BRANCH(=main, or $1)
#   AXES(="fts geo hdt update parse")   ordered axis list / subset
#   WATCHDOG_S(=10800 3h hard cap)  POLL_DEADLINE_S(=9900)  POLL_INTERVAL_S(=60)
#   DWELL_S(=600 Nitro console-snapshot lag)  AXIS_TIMEOUT_S(=1500 per-axis wall
#   cap)  TIMEOUT_S(=900 per-workload cap handed to every template axis script)
#   DISK_FLOOR_GB(=10 df floor before each axis)  CONSOLE_CAP_B(=6000 per-envelope
#   console cap)  PARSE_GEN_N(=320000 synthetic-triples dataset cap, the registered
#   bench/parse size)  RESULTS_LOCAL(=~/sparq-bench-results/multi-axis-<UTC>)
#   EXTRA_INSTANCE_ENV(="" verbatim KEY=VALUE tokens prepended to the on-box
#   invocation, e.g. 'AXIS_ENV_hdt=HDT_ARCHIVE=/root/big.hdt')
#   AXIS_ENV_<axis>(="" instance-mode extra KEY=VALUE tokens for one axis)
#
# USAGE
#   bash scripts/bench/multi-axis-box.sh --dry-run              # plan only, no AWS
#   AWS_PROFILE=pss bash scripts/bench/multi-axis-box.sh --launch [<branch>]
#   AXES="hdt parse" bash scripts/bench/multi-axis-box.sh --launch mybranch
set -euo pipefail

# ---------------------------------------------------------------------------------
# Shared constants + knobs
# ---------------------------------------------------------------------------------
: "${AWS_PROFILE:=pss}"; export AWS_PROFILE  # dev-box default creds lack EC2 — must use pss
REGION="${REGION:-eu-west-2}"
ITYPE="${ITYPE:-c6i.4xlarge}"                # canonical quiet-box class (16 vCPU / 32 GiB)
EBS_GB="${EBS_GB:-80}"
REPO="https://github.com/sparq-org/sparq.git"
AXES="${AXES:-fts geo hdt update parse}"
WATCHDOG_S="${WATCHDOG_S:-10800}"            # 3h hard self-terminate cap (backstop)
POLL_DEADLINE_S="${POLL_DEADLINE_S:-9900}"   # 2h45m < watchdog, so watchdog stays backstop
POLL_INTERVAL_S="${POLL_INTERVAL_S:-60}"
DWELL_S="${DWELL_S:-600}"                    # Nitro console snapshot lags ~8min (measured, bench/ec2-bench.sh)
AXIS_TIMEOUT_S="${AXIS_TIMEOUT_S:-1500}"     # per-axis wall cap; 5 axes must fit under the poll deadline
TIMEOUT_S="${TIMEOUT_S:-900}"                # per-workload cap inside each template axis script
DISK_FLOOR_GB="${DISK_FLOOR_GB:-10}"
CONSOLE_CAP_B="${CONSOLE_CAP_B:-6000}"
PARSE_GEN_N="${PARSE_GEN_N:-320000}"         # registered bench/parse synthetic dataset size (~162MB NT)
PREBUILD_ALLOW_S="${PREBUILD_ALLOW_S:-900}"  # budget line item for the one-off workspace release build
EXTRA_INSTANCE_ENV="${EXTRA_INSTANCE_ENV:-}"
# ${HOME:-/root}: cloud-init user-data runs with HOME unset — a bare $HOME under
# `set -u` killed --instance at this line on the first real launch (rc=1).
RESULTS_LOCAL="${RESULTS_LOCAL:-${HOME:-/root}/sparq-bench-results/multi-axis-$(date -u +%Y%m%dT%H%M%SZ)}"

# HARD never-touch instance ids (prod + dev) — same list as scripts/orphan-check-bench.sh.
readonly PROD_INSTANCE="i-090531b4ede8f2d3f"
readonly DEV_INSTANCE="i-00f76802f345b6b77"
TAGSPEC='ResourceType=instance,Tags=[{Key=Name,Value=sparq-bench},{Key=Project,Value=sparq-bench},{Key=purpose,Value=sparq-bench}]'

log() { printf '[multi-axis-box %s] %s\n' "$(date -u +%H:%M:%S)" "$*" >&2; }
die() { printf '[multi-axis-box] ERROR: %s\n' "$*" >&2; exit 1; }

# Axis catalog: harness path (repo-relative) per axis. Template axes share the
# shacl-same-box.sh knob contract; parse is a direct-harness special case.
axis_harness() {
  case "$1" in
    fts)    echo "scripts/bench/fts-same-box.sh" ;;      # bead sq-hmd7l.2
    geo)    echo "scripts/bench/geo-same-box.sh" ;;      # bead sq-hmd7l.3
    hdt)    echo "scripts/bench/hdt-same-box.sh" ;;      # bead sq-hmd7l.4
    update) echo "scripts/bench/update-same-box.sh" ;;   # bead sq-hmd7l.5
    parse)  echo "bench/parse/Cargo.toml" ;;             # direct harness (bead sq-hmd7l.6 owns bench/parse)
    *)      echo "" ;;
  esac
}
axis_bead() {
  case "$1" in
    fts) echo "sq-hmd7l.2" ;; geo) echo "sq-hmd7l.3" ;; hdt) echo "sq-hmd7l.4" ;;
    update) echo "sq-hmd7l.5" ;; parse) echo "sq-hmd7l.6" ;; *) echo "?" ;;
  esac
}

# ---------------------------------------------------------------------------------
# --dry-run: the packed execution plan. Reads ONLY the local checkout; no AWS call.
# ---------------------------------------------------------------------------------
print_plan() {
  local root n_axes=0 sum_caps
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  echo "multi-axis-box packed execution plan (sq-hmd7l.25) — DRY RUN, no AWS call"
  echo
  echo "  box:        ONE ${ITYPE} in ${REGION}, ${EBS_GB}GB gp3, Ubuntu 24.04 x86_64"
  echo "  tags:       Name=sparq-bench Project=sparq-bench purpose=sparq-bench"
  echo "  branch:     ${BRANCH}"
  echo "  never-touch: ${PROD_INSTANCE} (prod), ${DEV_INSTANCE} (dev) — refused by id"
  echo
  echo "  orphan-proofing:"
  echo "    shutdown-behavior=terminate; user-data watchdog (sleep ${WATCHDOG_S}s; shutdown) FIRST"
  echo "    + systemd-run backup; instance self-terminates after the axis list (+${DWELL_S}s dwell);"
  echo "    launcher poll deadline ${POLL_DEADLINE_S}s < ${WATCHDOG_S}s watchdog; EXIT-trap terminate;"
  echo "    post-run: scripts/orphan-check-bench.sh (dry-run) must come back clean."
  echo
  echo "  serialization: axes run strictly one after another; each same-box axis script"
  echo "  itself runs one engine at a time — never two engines co-tenant on the box."
  echo
  echo "  packed axes (ordered; per-axis wall cap ${AXIS_TIMEOUT_S}s; per-workload cap ${TIMEOUT_S}s;"
  echo "  df floor ${DISK_FLOOR_GB}GB + scratch cleanup between axes; CANONICAL=1 provenance):"
  local axis harness present bead
  for axis in $AXES; do
    harness="$(axis_harness "$axis")"
    [ -n "$harness" ] || { echo "    !! unknown axis '$axis' — would be refused"; continue; }
    n_axes=$((n_axes + 1))
    if [ -e "$root/$harness" ]; then present="present"; else present="ABSENT on this checkout -> honest skip"; fi
    bead="$(axis_bead "$axis")"
    if [ "$axis" = parse ]; then
      echo "    $n_axes. $axis    bench/parse harness (gen ${PARSE_GEN_N} synthetic triples -> bench-nt/-ttl/-zip/-ext)"
      echo "         [$present; envelope wrapper pending $bead — raw rows to console until then]"
    else
      echo "    $n_axes. $axis    $harness  (TIMEOUT_S=$TIMEOUT_S CANONICAL=1 OUT_DIR=/root/axis-results/$axis)"
      echo "         [$present; harness bead $bead]"
    fi
  done
  sum_caps=$((n_axes * AXIS_TIMEOUT_S + PREBUILD_ALLOW_S))
  echo
  echo "  budget check: ${n_axes} axes x ${AXIS_TIMEOUT_S}s + ${PREBUILD_ALLOW_S}s prebuild = ${sum_caps}s"
  if [ "$sum_caps" -gt "$POLL_DEADLINE_S" ]; then
    echo "  WARN: ${sum_caps}s exceeds the ${POLL_DEADLINE_S}s poll deadline — trim AXES or AXIS_TIMEOUT_S"
  else
    echo "  fits under the ${POLL_DEADLINE_S}s poll deadline (watchdog ${WATCHDOG_S}s stays the backstop)"
  fi
  echo
  echo "  result channels:"
  echo "    console: SPARQ_BENCH_RESULT envelope blocks, one '=== SPARQ_BENCH_RESULT <axis> ===' sub-"
  echo "             range per axis (compact JSON, ${CONSOLE_CAP_B}B/envelope cap for the 64KB buffer)"
  echo "    ssh:     incremental pull of /root/axis-results/ -> ${RESULTS_LOCAL}"
  echo
  echo "  to run for real:  AWS_PROFILE=pss bash scripts/bench/multi-axis-box.sh --launch ${BRANCH}"
  echo "  (no AWS call was made by this dry run)"
}

# ---------------------------------------------------------------------------------
# --instance: ON-BOX serial axis loop (root, cwd = cloned repo). NO shutdown here —
# self-termination lives in the launcher-generated user-data only.
# ---------------------------------------------------------------------------------
CONSOLE_LOG="${CONSOLE_LOG:-/root/multi-axis-console.log}"
STEP_LOG="${STEP_LOG:-/root/GATHER_STEP}"
SENTINEL="${SENTINEL:-/root/GATHER_DONE}"
# CANONICAL defaults to 1 because --instance is designed to run ON the dedicated
# quiet box; set CANONICAL=0 for any local smoke so envelopes stay honestly flagged.
CANONICAL="${CANONICAL:-1}"

con() {  # console-result line: serial console (both devices) + a pull-able log + stderr
  local ln="$1"
  echo "$ln" > /dev/console 2>/dev/null || true
  echo "$ln" > /dev/ttyS0   2>/dev/null || true
  echo "$ln" >> "$CONSOLE_LOG" 2>/dev/null || true
  printf '%s\n' "$ln" >&2
}
step() { echo "[STEP $(date -u +%Y-%m-%dT%H:%M:%SZ)] $*" | tee -a "$STEP_LOG" >&2 || true; }

imds() {  # IMDSv2 metadata (best-effort; "unknown" off-box)
  local tok
  tok=$(curl -sS -m 2 -X PUT "http://169.254.169.254/latest/api/token" \
        -H "X-aws-ec2-metadata-token-ttl-seconds: 300" 2>/dev/null) || { echo unknown; return; }
  curl -sS -m 2 -H "X-aws-ec2-metadata-token: $tok" \
        "http://169.254.169.254/latest/meta-data/$1" 2>/dev/null || echo unknown
}

avail_gb() { df -BG --output=avail / 2>/dev/null | tail -1 | tr -dc '0-9' || echo 0; }

clean_scratch() {  # between-axis disk discipline (gather-only scratch; bench data is regenerable)
  # Root-gated: a non-root LOCAL smoke of --instance must never delete the shared
  # work box's /tmp scratch or prune its docker images.
  if [ "$(id -u)" -ne 0 ]; then step "scratch cleanup skipped (not root — local smoke)"; return 0; fi
  step "scratch cleanup (df before: $(avail_gb)GB free)"
  rm -rf /tmp/*same-box* /tmp/*bench* /tmp/jena* /tmp/sparq-* 2>/dev/null || true
  if command -v docker >/dev/null 2>&1; then
    docker system prune -af --volumes >/dev/null 2>&1 || true
  fi
  step "scratch cleanup done (df after: $(avail_gb)GB free)"
}

emit_envelopes() {  # $1=axis $2=out_dir — compact envelopes to console under the byte cap
  local axis="$1" out_dir="$2" f compact n=0
  for f in "$out_dir"/*.json; do
    [ -e "$f" ] || continue
    n=$((n + 1))
    if ! compact=$(jq -c . "$f" 2>/dev/null); then
      con "envelope: $f INVALID JSON"; continue
    fi
    con "envelope: $f ($(printf '%s' "$compact" | wc -c)B)"
    if [ "$(printf '%s' "$compact" | wc -c)" -le "$CONSOLE_CAP_B" ]; then
      con "$compact"
    else
      # Reduced form: keep identity/provenance/statuses, drop the bulky per-query
      # tables; the FULL envelope still reaches RESULTS_LOCAL via the SSH pull.
      con "$(jq -c '{suite,scale,canonical,git_commit,statuses,truncated:true,
                     note:"exceeds console cap; full envelope via SSH pull"}' "$f" 2>/dev/null \
             || printf '{"truncated":true,"file":"%s"}' "$f")"
    fi
  done
  [ "$n" -gt 0 ] || con "envelope: none produced under $out_dir"
}

run_instance() {
  cd "$(dirname "${BASH_SOURCE[0]}")/../.."   # repo root
  local sha iid mtype now
  sha=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
  iid=$(imds instance-id); mtype=$(imds instance-type)
  export QUIET_BOX=true

  con "=== SPARQ_BENCH_RESULT id=multi-axis sha=${sha} ==="
  con "plan: axes=[${AXES}] axis_cap=${AXIS_TIMEOUT_S}s workload_cap=${TIMEOUT_S}s df_floor=${DISK_FLOOR_GB}GB"

  if [ "${PREBUILD:-1}" = 1 ]; then
    step "one-off workspace release prebuild"
    cargo build --release 2>&1 | tail -3 | while IFS= read -r ln; do step "build: $ln"; done \
      || step "WARN: workspace prebuild had errors (per-axis cargo may still build)"
  else
    step "prebuild skipped (PREBUILD=0 — local smoke)"
  fi

  local axis harness rc t0 t1 wall out_dir envvar extra
  for axis in $AXES; do
    harness="$(axis_harness "$axis")"
    now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    con "=== SPARQ_BENCH_RESULT ${axis} ==="
    con "provenance: instance=${iid} type=${mtype} commit=${sha} utc=${now} canonical=${CANONICAL}"
    if [ -z "$harness" ]; then
      con "status: refused (unknown axis '${axis}')"
      con "=== END SPARQ_BENCH_RESULT ${axis} ==="
      continue
    fi
    if [ ! -e "$harness" ]; then
      con "status: absent (harness ${harness} not on this branch yet — bead $(axis_bead "$axis"))"
      con "=== END SPARQ_BENCH_RESULT ${axis} ==="
      continue
    fi

    clean_scratch
    if [ "$(avail_gb)" -lt "$DISK_FLOOR_GB" ]; then
      con "status: skipped-disk-floor ($(avail_gb)GB free < ${DISK_FLOOR_GB}GB floor after cleanup)"
      con "=== END SPARQ_BENCH_RESULT ${axis} ==="
      continue
    fi

    out_dir="/root/axis-results/${axis}"; mkdir -p "$out_dir"
    envvar="AXIS_ENV_${axis}"; extra="${!envvar:-}"
    step "[${axis}] start (cap ${AXIS_TIMEOUT_S}s; extra env: ${extra:-<none>})"
    t0=$(date +%s); rc=0
    if [ "$axis" = parse ]; then
      # Direct harness: deterministic synthetic dataset (size-capped), NT+TTL+compressed
      # rows straight to the console block (envelope wrapper is bead sq-hmd7l.6).
      # shellcheck disable=SC2086,SC2016  # $extra word-split KEY=VALUE tokens; $0 expands inside bash -c
      timeout "$AXIS_TIMEOUT_S" env $extra bash -c '
        set -euo pipefail
        cd bench/parse
        cargo build --release 2>&1 | tail -2
        mkdir -p data
        ./target/release/parse-baseline gen "$0" data/synthetic.nt data/synthetic.ttl
        ./target/release/parse-baseline compress data/synthetic.nt
        ./target/release/parse-baseline bench-nt  data/synthetic.nt
        ./target/release/parse-baseline bench-ttl data/synthetic.ttl
        ./target/release/parse-baseline bench-zip data/synthetic.nt
        # External competitor columns (sq-hmd7l.6): serdi / rapper / riot
        # subprocess rows over the SAME corpus files; absent tool => absent column.
        ./target/release/parse-baseline bench-ext data/synthetic.nt
        ./target/release/parse-baseline bench-ext data/synthetic.ttl
      ' "$PARSE_GEN_N" > "$out_dir/parse-rows.txt" 2>&1 || rc=$?
      t1=$(date +%s); wall=$((t1 - t0))
      con "status: $([ "$rc" -eq 0 ] && echo ok || echo failed) rc=${rc} wall_s=${wall}"
      con "rows: (tail; full output pulled via SSH from ${out_dir}/parse-rows.txt)"
      tail -40 "$out_dir/parse-rows.txt" 2>/dev/null | while IFS= read -r ln; do con "$ln"; done
      con "envelope: none (bench/parse raw rows; envelope wrapper pending sq-hmd7l.6)"
    else
      # shellcheck disable=SC2086  # $extra is intentionally word-split KEY=VALUE tokens
      timeout "$AXIS_TIMEOUT_S" env CANONICAL="$CANONICAL" TIMEOUT_S="$TIMEOUT_S" OUT_DIR="$out_dir" $extra \
        bash "$harness" > "$out_dir/run.log" 2>&1 || rc=$?
      t1=$(date +%s); wall=$((t1 - t0))
      con "status: $([ "$rc" -eq 0 ] && echo ok || echo failed) rc=${rc} wall_s=${wall}"
      [ "$rc" -eq 0 ] || tail -5 "$out_dir/run.log" 2>/dev/null | while IFS= read -r ln; do con "log: $ln"; done
      emit_envelopes "$axis" "$out_dir"
    fi
    step "[${axis}] done rc=${rc} wall_s=${wall}"
    con "=== END SPARQ_BENCH_RESULT ${axis} ==="
  done

  clean_scratch
  con "=== SPARQ_BENCH_EXIT rc=0 ==="
  con "=== END SPARQ_BENCH_RESULT ==="
  sync
  git rev-parse --short HEAD > "$SENTINEL" 2>/dev/null || echo "done" > "$SENTINEL"
  step "GATHER_DONE"
}

# ---------------------------------------------------------------------------------
# --launch: provision the box, poll, pull, terminate. (canonical-competitor-bench.sh
# lineage; the heavy per-axis logic stays in-repo via --instance, not in user-data.)
# ---------------------------------------------------------------------------------
run_launch() {
  command -v aws >/dev/null || die "aws CLI not found"
  mkdir -p "$RESULTS_LOCAL"

  local WORK KEYFILE KEY_NAME INSTANCE_ID SG_ID SSHO
  WORK="$(mktemp -d)"
  KEYFILE="$WORK/key"
  KEY_NAME="sparq-bench-multi-axis-$$-${RANDOM}"
  INSTANCE_ID=""; SG_ID=""
  ssh-keygen -t ed25519 -N '' -f "$KEYFILE" -q
  SSHO="-i $KEYFILE -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=15"

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
  local AMI VPC SUBNET MYIP
  AMI=$(aws ec2 describe-images --region "$REGION" --owners 099720109477 \
    --filters "Name=name,Values=ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-amd64-server-*" "Name=state,Values=available" \
    --query 'sort_by(Images,&CreationDate)[-1].ImageId' --output text)
  if [ -z "$AMI" ] || [ "$AMI" = None ]; then die "could not resolve AMI"; fi
  VPC=$(aws ec2 describe-vpcs --region "$REGION" --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)
  SUBNET=$(aws ec2 describe-subnets --region "$REGION" --filters Name=vpc-id,Values="$VPC" "Name=default-for-az,Values=true" --query 'Subnets[0].SubnetId' --output text)
  MYIP=$(curl -s https://checkip.amazonaws.com | tr -d '[:space:]')
  [ -n "$MYIP" ] || die "could not determine public IP"
  log "AMI=$AMI VPC=$VPC SUBNET=$SUBNET MYIP=$MYIP ITYPE=$ITYPE BRANCH=$BRANCH AXES='$AXES'"

  log "ephemeral keypair + locked-down SG (ssh from $MYIP/32 only)"
  aws ec2 import-key-pair --region "$REGION" --key-name "$KEY_NAME" --public-key-material "fileb://${KEYFILE}.pub" >/dev/null
  SG_ID=$(aws ec2 create-security-group --region "$REGION" --group-name "$KEY_NAME" \
    --description "sparq multi-axis bin-packed bench box (ephemeral)" --vpc-id "$VPC" --query 'GroupId' --output text)
  aws ec2 authorize-security-group-ingress --region "$REGION" --group-id "$SG_ID" --protocol tcp --port 22 --cidr "${MYIP}/32" >/dev/null

  # Thin user-data: watchdog FIRST, deps, clone, then the COMMITTED --instance mode
  # of THIS script; self-termination (dwell + shutdown) lives HERE, not in --instance.
  local USERDATA
  USERDATA=$(cat <<UD
#!/bin/bash
( sleep $WATCHDOG_S; shutdown -h now ) &
systemd-run --on-active=$WATCHDOG_S /sbin/shutdown -h now || true
set -x
exec > >(tee /var/log/gather.log) 2>&1

step() { echo "[STEP \$(date -u +%Y-%m-%dT%H:%M:%SZ)] \$*" | tee -a /root/GATHER_STEP >&2; }
# cloud-init runs user-data WITHOUT HOME: rustup/cargo/git and every \$HOME expansion
# (e.g. .cargo/env's PATH prepend, which produced "/.cargo/bin") misroute without this.
export HOME=/root
export DEBIAN_FRONTEND=noninteractive
step "apt update+install"
apt-get update -qq
# libboost-all-dev: corpus generators; jre: Fuseki-backed axes; raptor2-utils +
# serdi: the parse axis rapper/serd columns (absent tool => absent column, never a
# fake number — but the canonical box provisions all three registered columns).
apt-get install -y -qq build-essential g++ pkg-config git curl jq python3 python3-venv python3-pip unzip docker.io openjdk-21-jre-headless libboost-all-dev bc raptor2-utils serdi || true
# Jena riot: the parse axis third competitor column (gap-parse-2026-07 requires it
# on the canonical box; apt has no package — pinned apache-jena dist, best-effort).
step "jena riot install"
curl -fsSL "https://archive.apache.org/dist/jena/binaries/apache-jena-${JENA_VERSION:-5.4.0}.tar.gz" | tar -xz -C /opt \
  && ln -sf "/opt/apache-jena-${JENA_VERSION:-5.4.0}/bin/riot" /usr/local/bin/riot || step "WARN: riot install failed — parse riot column will be honestly absent"
step "start docker"
systemctl enable --now docker || systemctl start docker || true
for _ in \$(seq 1 60); do docker info >/dev/null 2>&1 && { step "docker up"; break; }; sleep 2; done

step "rustup install"
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal || true
export PATH="/root/.cargo/bin:\$PATH"; . /root/.cargo/env || true

step "git clone + checkout $BRANCH"
cd /root
git clone -q "$REPO" sparq
cd sparq
git fetch -q origin "$BRANCH" && git checkout -q "$BRANCH"
step "checked out \$(git rev-parse --short HEAD)"

AXES="$AXES" AXIS_TIMEOUT_S=$AXIS_TIMEOUT_S TIMEOUT_S=$TIMEOUT_S DISK_FLOOR_GB=$DISK_FLOOR_GB \
CONSOLE_CAP_B=$CONSOLE_CAP_B PARSE_GEN_N=$PARSE_GEN_N CANONICAL=1 $EXTRA_INSTANCE_ENV \
  bash scripts/bench/multi-axis-box.sh --instance || step "WARN: instance run rc=\$?"

# Dwell so the Nitro console snapshot captures the result blocks, then SELF-TERMINATE
# (shutdown-behavior=terminate). The watchdog above remains the backstop.
sleep $DWELL_S
shutdown -h now
UD
)

  printf '%s\n' "$USERDATA" > "$WORK/userdata.sh"
  local UD_RAW
  UD_RAW=$(wc -c < "$WORK/userdata.sh")
  log "user-data raw=${UD_RAW}B (limit 16384B)"
  [ "$UD_RAW" -le 16384 ] || die "user-data ${UD_RAW}B exceeds 16384B — trim it"

  log "launching $ITYPE (${EBS_GB}GB gp3, $((WATCHDOG_S / 3600))h watchdog)"
  local LAUNCH_ERR="$WORK/launch.err"
  INSTANCE_ID=$(aws ec2 run-instances --region "$REGION" --image-id "$AMI" --instance-type "$ITYPE" \
    --instance-initiated-shutdown-behavior terminate \
    --key-name "$KEY_NAME" --security-group-ids "$SG_ID" \
    --subnet-id "$SUBNET" --associate-public-ip-address \
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
  local IP
  IP=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)
  log "public IP=$IP; waiting for sshd"
  local SSH_UP=0 i
  for i in $(seq 1 40); do
    # shellcheck disable=SC2086  # $SSHO is intentionally word-split ssh options
    ssh $SSHO "ubuntu@$IP" true 2>/dev/null && { log "ssh up"; SSH_UP=1; break; }
    sleep 10
  done
  [ "$SSH_UP" = 1 ] || die "sshd never reachable on $IP — aborting (cleanup terminates)"

  log "polling for /root/GATHER_DONE (deadline ${POLL_DEADLINE_S}s, < ${WATCHDOG_S}s watchdog)…"
  local DONE=0 POLL_START ELAPSED STATE CUR_STEP
  i=0; POLL_START=$(date +%s)
  while :; do
    ELAPSED=$(( $(date +%s) - POLL_START ))
    [ "$ELAPSED" -ge "$POLL_DEADLINE_S" ] && { log "poll deadline reached without sentinel — giving up (cleanup terminates)"; break; }
    sleep "$POLL_INTERVAL_S"; i=$(( i + 1 ))
    # incremental pull of any axis results already written
    # shellcheck disable=SC2086
    ssh $SSHO "ubuntu@$IP" "sudo tar -C /root -cf - axis-results 2>/dev/null" 2>/dev/null | tar -C "$RESULTS_LOCAL" -xf - 2>/dev/null || true
    # shellcheck disable=SC2086
    if ssh $SSHO "ubuntu@$IP" "sudo test -f /root/GATHER_DONE" 2>/dev/null; then
      log "[$i / ${ELAPSED}s] sentinel present — final pull"; DONE=1; break
    fi
    STATE=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" --query 'Reservations[0].Instances[0].State.Name' --output text 2>/dev/null || echo unknown)
    # shellcheck disable=SC2086
    CUR_STEP=$(ssh $SSHO "ubuntu@$IP" "sudo tail -n1 /root/GATHER_STEP 2>/dev/null" 2>/dev/null || true)
    log "[$i / ${ELAPSED}s] state=$STATE; step: ${CUR_STEP:-<not started>}"
    [ "$STATE" = "terminated" ] && { log "instance terminated before sentinel — results may be partial"; break; }
  done

  log "final result pull"
  # shellcheck disable=SC2086
  ssh $SSHO "ubuntu@$IP" "sudo tar -C /root -cf - axis-results 2>/dev/null" 2>/dev/null | tar -C "$RESULTS_LOCAL" -xf - 2>/dev/null || true
  # shellcheck disable=SC2086
  ssh $SSHO "ubuntu@$IP" "sudo cat /root/multi-axis-console.log 2>/dev/null" > "$RESULTS_LOCAL/console-envelopes.txt" 2>/dev/null || true
  # shellcheck disable=SC2086
  ssh $SSHO "ubuntu@$IP" "sudo cat /root/GATHER_STEP 2>/dev/null" > "$RESULTS_LOCAL/GATHER_STEP.txt" 2>/dev/null || true
  # Console-output backstop (works even if SSH died): extract the marker range.
  aws ec2 get-console-output --region "$REGION" --instance-id "$INSTANCE_ID" --output text 2>/dev/null \
    | sed -n '/=== SPARQ_BENCH_RESULT/,/^=== END SPARQ_BENCH_RESULT ===$/p' \
    >> "$RESULTS_LOCAL/console-envelopes.txt" 2>/dev/null || true

  log "results in $RESULTS_LOCAL:"
  ls -la "$RESULTS_LOCAL" >&2 || true
  [ "$DONE" = 1 ] || log "NOTE: sentinel not observed — see $RESULTS_LOCAL/GATHER_STEP.txt for the last step"
  log "cleanup trap terminates $INSTANCE_ID + deletes keypair/SG"
  trap - EXIT; cleanup

  # The bead invariant: orphan-check must come back clean after every run (advisory here).
  if [ -x "scripts/orphan-check-bench.sh" ]; then
    log "post-run orphan check (dry-run):"
    bash scripts/orphan-check-bench.sh || log "WARN: orphan-check reported orphans or failed — investigate NOW"
  fi
}

# ---------------------------------------------------------------------------------
# Mode dispatch
# ---------------------------------------------------------------------------------
MODE="hint"
BRANCH="${BRANCH:-main}"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run)  MODE="dry-run" ;;
    --launch)   MODE="launch" ;;
    --instance) MODE="instance" ;;
    -h|--help)  sed -n '2,70p' "$0"; exit 0 ;;
    -*)         die "unknown flag: $1 (try --dry-run, --launch, --instance, --help)" ;;
    *)          BRANCH="$1" ;;
  esac
  shift
done

case "$MODE" in
  dry-run)  print_plan ;;
  instance) run_instance ;;
  launch)   run_launch ;;
  hint)     print_plan; echo; echo "(no mode flag given — printed the dry-run plan; use --launch to provision)" ;;
esac
