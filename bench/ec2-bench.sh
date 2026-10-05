#!/usr/bin/env bash
# [OPUS-4.8] ec2-bench.sh — orphan-proof, SSH-less EC2 benchmark launcher for sparq.
# Authored by Opus 4.8 (Fable unavailable; flag for re-review when Fable returns).
#
# WHY this exists (vs scripts/ec2-bench.sh, the CI variant): the CI script needs an
# inbound SSH port + an ephemeral keypair to SCP a results JSON back. On the dev box we
# have NO sparq keypair private key and the `pss` SSO policy has NO S3. So this harness
# uses the SERIAL-CONSOLE result channel instead: the benchmark prints its result lines
# between `=== SPARQ_BENCH_RESULT === ... === END SPARQ_BENCH_RESULT ===` markers to
# /dev/console, and we pull them with `aws ec2 get-console-output`. NO inbound rules.
#
# 64KB-BUFFER DISCIPLINE (sq-6tley): get-console-output retains only the LAST ~64KB of the
# serial buffer. An invoke that streams a lot of output (notably `cargo run`, which emits a
# "Compiling <dep>" line for every dependency before the program even starts) will push its
# OWN result lines out of that window and the results are LOST even though the run succeeded.
# So a chatty benchmark MUST: (1) BUILD first with a tiny tail (`cargo build ... | tail -3`),
# (2) run the PREBUILT binary to a FILE, and (3) echo back ONLY the compact final table. Use
# per-dataset sub-markers `=== SPARQ_BENCH_RESULT <ds> ===` / `=== END SPARQ_BENCH_RESULT <ds> ===`
# (NOTE the trailing `<ds> ===`, not a bare `===`, so they do NOT close the outer harness range
# below). See the `kge-ablation` entry in benchmarks.toml for the canonical pattern.
#
# ORPHAN-SAFETY (paramount — a prior session orphaned a billing instance), BOTH ways:
#   (a) every instance launches with --instance-initiated-shutdown-behavior terminate;
#   (b) user-data's FIRST action backgrounds a hard watchdog `( sleep WATCHDOG; shutdown -h now ) &`
#       so the box self-terminates within WATCHDOG seconds no matter what (boot failure,
#       hung build, hung benchmark, or lost AWS access). The script body also ends in
#       `shutdown -h now`. Default cap 45 min — override with WATCHDOG_SECS for a long bench.
# Tag: purpose=sparq-bench (so a fresh session can orphan-check + sweep). ~$5/day cap:
# default c7g.large (eu-west-2 ~ $0.08/hr). NEVER touches prod/dev-box (we only ever act
# on instance ids we launched and on the purpose=sparq-bench tag).
#
# USAGE
#   bench/ec2-bench.sh launch <benchmark-id> <repo-sha> [instance-type]   # -> prints instance id
#   bench/ec2-bench.sh result <instance-id>                               # pull console result lines
#   bench/ec2-bench.sh wait-result <instance-id> [max-min=12]             # poll until marker or timeout
#   bench/ec2-bench.sh terminate <instance-id>                            # explicit terminate + confirm
#   bench/ec2-bench.sh orphan-check                                       # list any live purpose=sparq-bench
#   bench/ec2-bench.sh sweep                                              # terminate ALL purpose=sparq-bench
#
# <benchmark-id> is an `id` from bench/benchmarks.toml; its `invoke` line is run verbatim
# (trailing `# ...` comment stripped, literal \n expanded to real newlines).
#
# PARALLEL USE — launch one box per quiet-box-sensitive benchmark, all at once:
#   SHA=$(git rev-parse HEAD)
#   for b in sparq-bench-compare selective-bindjoin parse-baseline; do
#     bench/ec2-bench.sh launch "$b" "$SHA" >> /tmp/sparq-bench-ids.txt
#   done
#   # later: for i in $(cat /tmp/sparq-bench-ids.txt); do bench/ec2-bench.sh wait-result "$i"; done
#   bench/ec2-bench.sh orphan-check   # must be empty when done
#
# ENV overrides: AWS_PROFILE(=pss) AWS_REGION(=eu-west-2) WATCHDOG_SECS(=2700)
#   INSTANCE_TYPE(default arg3 or c7g.large) VOLUME_GB(=40) AMI_OWNER(=099720109477)
set -euo pipefail

PROFILE="${AWS_PROFILE:-pss}"
REGION="${AWS_REGION:-eu-west-2}"
WATCHDOG_SECS="${WATCHDOG_SECS:-2700}"            # 45 min absolute self-terminate cap
VOLUME_GB="${VOLUME_GB:-40}"
REPO="https://github.com/sparq-org/sparq.git"
TOML="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/benchmarks.toml"
TAGSPEC='ResourceType=instance,Tags=[{Key=purpose,Value=sparq-bench},{Key=Name,Value=sparq-bench-run}]'
AWS="aws --profile $PROFILE --region $REGION"

die() { echo "ERROR: $*" >&2; exit 1; }

# Extract the `invoke` command for a benchmark id from benchmarks.toml.
# Finds the [[benchmark]] block whose id == $1, returns its invoke string with the
# trailing "  # ..." comment stripped and literal \n turned into real newlines.
extract_invoke() {
  local id="$1"
  [ -f "$TOML" ] || die "benchmarks.toml not found at $TOML"
  awk -v want="$id" '
    /^\[\[benchmark\]\]/ { inblk=1; cur=""; inv=""; next }
    inblk && /^id[[:space:]]*=/ {
      line=$0; sub(/^id[[:space:]]*=[[:space:]]*"/,"",line); sub(/".*/,"",line); cur=line
    }
    inblk && /^invoke[[:space:]]*=/ {
      # Parse the TOML basic string EXPLICITLY: walk to the first UNESCAPED closing
      # quote, preserving escaped chars (\" \\ \n) verbatim, then ignore anything after
      # (a trailing inline comment). A regex cannot distinguish \" from a real close or
      # an in-string # from a comment, which kept truncating valid values
      # (roborev #2264/#2277/#2278), so a regex is not used here.
      line=$0; sub(/^invoke[[:space:]]*=[[:space:]]*"/,"",line)
      out=""; n=length(line); i=1
      while (i<=n) {
        c=substr(line,i,1)
        if (c=="\\" && i<n) { out=out c substr(line,i+1,1); i+=2; continue }
        if (c=="\"") break
        out=out c; i++
      }
      inv=out
    }
    inblk && cur==want && inv!="" && /^invoke[[:space:]]*=/ {
      print inv; found=1; exit
    }
    END { if(!found) exit 3 }
  ' "$TOML" \
    | sed -E 's/\\n/\n/g'
}

cmd_launch() {
  local id="${1:-}" sha="${2:-}" itype="${3:-${INSTANCE_TYPE:-c7g.large}}"
  [ -n "$id" ] && [ -n "$sha" ] || die "usage: launch <benchmark-id> <repo-sha> [instance-type]"

  local invoke
  invoke="$(extract_invoke "$id")" || die "benchmark id '$id' not found in benchmarks.toml"
  [ -n "$invoke" ] || die "benchmark id '$id' has an empty invoke"

  echo "[ec2-bench] id=$id sha=$sha type=$itype watchdog=${WATCHDOG_SECS}s" >&2
  echo "[ec2-bench] invoke: $invoke" >&2

  # Resolve AMI + network fresh (so this works in any account state).
  local ami vpc subnet sg
  ami=$($AWS ec2 describe-images --owners "${AMI_OWNER:-099720109477}" \
    --filters "Name=name,Values=ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-arm64-server-*" "Name=state,Values=available" \
    --query 'sort_by(Images,&CreationDate)[-1].ImageId' --output text)
  [ -n "$ami" ] && [ "$ami" != "None" ] || die "could not resolve Ubuntu 24.04 arm64 AMI"
  vpc=$($AWS ec2 describe-vpcs --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)
  subnet=$($AWS ec2 describe-subnets --filters Name=vpc-id,Values="$vpc" "Name=default-for-az,Values=true" \
    --query 'Subnets[0].SubnetId' --output text)
  sg=$($AWS ec2 describe-security-groups --filters Name=vpc-id,Values="$vpc" Name=group-name,Values=default \
    --query 'SecurityGroups[0].GroupId' --output text)

  # --- user-data: watchdog FIRST, then bootstrap+build+run, results to /dev/console, shutdown ---
  # The benchmark's invoke runs under `set +e` so a non-zero exit still reports + shuts down.
  local ud
  ud="$(cat <<UDEOF
#!/bin/bash
( sleep ${WATCHDOG_SECS}; shutdown -h now ) &     # (a) hard self-terminate cap, FIRST line
exec > /var/log/sparq-bench.log 2>&1
set -x
# MEASURED (Opus 4.8, 2026-06-13): on Nitro/Graviton the EC2 get-console-output API only
# snapshots the serial buffer in bulk after ~8 min of uptime, and writes to BOTH /dev/console
# and /dev/ttyS0 reach it. So: (i) emit results to both devices, (ii) keep the box alive
# >=9 min AFTER writing results so the snapshot captures them before we self-terminate.
con() { echo "\$1" > /dev/console 2>/dev/null; echo "\$1" > /dev/ttyS0 2>/dev/null; }
con "=== SPARQ_BENCH_BOOT $(date -u +%FT%TZ) id=${id} sha=${sha} type=${itype} ==="

export DEBIAN_FRONTEND=noninteractive
export HOME=/root
apt-get update -y && apt-get install -y git build-essential pkg-config libssl-dev curl python3 clang
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
source \$HOME/.cargo/env
export PATH="\$HOME/.cargo/bin:\$PATH"

cd /root
git clone --filter=blob:none "${REPO}" sparq && cd sparq
git checkout "${sha}" || { con "=== SPARQ_BENCH_RESULT ==="; con "FAIL: checkout ${sha}"; con "=== END SPARQ_BENCH_RESULT ==="; sleep 600; shutdown -h now; }

# Many invokes assume release artifacts; build the workspace release once up front so
# bare \`./target/release/<bin>\` invocations resolve. (cargo run/build invokes self-build.)
cargo build --release 2>&1 | tail -5 | while IFS= read -r ln; do con "build: \$ln"; done \
  || con "WARN: workspace build had errors (per-bench cargo may still build)"

set +e
con "=== SPARQ_BENCH_RESULT id=${id} sha=${sha} ==="
{
${invoke}
} 2>&1 | tee /tmp/bench.out | while IFS= read -r ln; do con "\$ln"; done
RC=\${PIPESTATUS[0]}
con "=== SPARQ_BENCH_EXIT rc=\$RC ==="
con "=== END SPARQ_BENCH_RESULT ==="
sync
# Dwell so the Nitro console snapshot captures the results before self-terminate (see note above).
# Bounded well under the watchdog. Poll with: ec2-bench.sh wait-result <id> 18
sleep 600
shutdown -h now
UDEOF
)"

  local tmpud; tmpud="$(mktemp)"; printf '%s\n' "$ud" > "$tmpud"

  local iid
  iid=$($AWS ec2 run-instances \
    --image-id "$ami" --instance-type "$itype" \
    --instance-initiated-shutdown-behavior terminate \
    --security-group-ids "$sg" --subnet-id "$subnet" \
    --block-device-mappings "[{\"DeviceName\":\"/dev/sda1\",\"Ebs\":{\"VolumeSize\":${VOLUME_GB},\"VolumeType\":\"gp3\",\"DeleteOnTermination\":true}}]" \
    --user-data "file://${tmpud}" \
    --tag-specifications "$TAGSPEC" \
    --query 'Instances[0].InstanceId' --output text)
  rm -f "$tmpud"
  [ -n "$iid" ] && [ "$iid" != "None" ] || die "run-instances did not return an instance id"
  echo "$iid"            # stdout = instance id ONLY (so callers can capture it)
}

cmd_result() {
  local iid="${1:-}"; [ -n "$iid" ] || die "usage: result <instance-id>"
  $AWS ec2 get-console-output --instance-id "$iid" --output text 2>&1 \
    | sed -n '/=== SPARQ_BENCH_RESULT/,/=== END SPARQ_BENCH_RESULT ===/p'
}

cmd_wait_result() {
  # default 18 min: MEASURED Nitro/Graviton console-snapshot lag is ~8 min AFTER uptime,
  # plus boot+build+bench+the 600s result-dwell. Bump for long builds/benchmarks.
  local iid="${1:-}" maxmin="${2:-18}"; [ -n "$iid" ] || die "usage: wait-result <instance-id> [max-min]"
  local deadline=$(( $(date +%s) + maxmin*60 )) out
  while [ "$(date +%s)" -lt "$deadline" ]; do
    out="$($AWS ec2 get-console-output --instance-id "$iid" --output text 2>&1 || true)"
    if echo "$out" | grep -q 'END SPARQ_BENCH_RESULT'; then
      echo "$out" | sed -n '/=== SPARQ_BENCH_RESULT/,/=== END SPARQ_BENCH_RESULT ===/p'
      return 0
    fi
    local st; st="$($AWS ec2 describe-instances --instance-ids "$iid" \
      --query 'Reservations[0].Instances[0].State.Name' --output text 2>/dev/null || echo unknown)"
    echo "[ec2-bench] $iid state=$st, no result marker yet; sleeping 30s..." >&2
    # roborev #2264: if the box terminated before emitting the END marker, the benchmark
    # did NOT complete — surface whatever console captured but return FAILURE, not success.
    [ "$st" = "terminated" ] && { echo "$out" | sed -n '/=== SPARQ_BENCH_RESULT/,/=== END SPARQ_BENCH_RESULT ===/p'; echo "[ec2-bench] $iid terminated WITHOUT a result marker — benchmark did not complete" >&2; return 1; }
    sleep 30
  done
  echo "[ec2-bench] TIMEOUT after ${maxmin}m waiting for result on $iid" >&2; return 1
}

cmd_terminate() {
  local iid="${1:-}"; [ -n "$iid" ] || die "usage: terminate <instance-id>"
  # SAFETY GUARD (roborev #2264, HIGH): refuse to terminate anything that is not one of
  # OUR bench boxes. This is the last line of defence against fat-fingering prod
  # (i-090531b4ede8f2d3f) or the dev box — terminate must NEVER touch an untagged instance.
  local purpose; purpose="$($AWS ec2 describe-instances --instance-ids "$iid" \
    --query 'Reservations[0].Instances[0].Tags[?Key==`purpose`]|[0].Value' --output text 2>/dev/null || echo '')"
  [ "$purpose" = "sparq-bench" ] || die "refusing to terminate $iid: tag purpose='$purpose' (expected 'sparq-bench'). Guard against touching prod / the dev box."
  $AWS ec2 terminate-instances --instance-ids "$iid" --query 'TerminatingInstances[0].[InstanceId,CurrentState.Name]' --output text
  $AWS ec2 wait instance-terminated --instance-ids "$iid" && echo "[ec2-bench] $iid terminated"
}

cmd_orphan_check() {
  $AWS ec2 describe-instances \
    --filters Name=tag:purpose,Values=sparq-bench Name=instance-state-name,Values=running,pending,stopped,stopping \
    --query 'Reservations[].Instances[].[InstanceId,State.Name,InstanceType]' --output text
}

cmd_sweep() {
  local ids; ids=$($AWS ec2 describe-instances \
    --filters Name=tag:purpose,Values=sparq-bench Name=instance-state-name,Values=running,pending,stopped,stopping \
    --query 'Reservations[].Instances[].InstanceId' --output text)
  [ -z "$ids" ] && { echo "[ec2-bench] no sparq-bench instances to sweep"; return 0; }
  echo "[ec2-bench] terminating: $ids" >&2
  $AWS ec2 terminate-instances --instance-ids $ids --query 'TerminatingInstances[].[InstanceId,CurrentState.Name]' --output text
}

case "${1:-}" in
  launch)       shift; cmd_launch "$@";;
  result)       shift; cmd_result "$@";;
  wait-result)  shift; cmd_wait_result "$@";;
  terminate)    shift; cmd_terminate "$@";;
  orphan-check) shift; cmd_orphan_check "$@";;
  sweep)        shift; cmd_sweep "$@";;
  *) echo "usage: $0 {launch|result|wait-result|terminate|orphan-check|sweep} ..." >&2; exit 2;;
esac
