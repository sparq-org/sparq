#!/usr/bin/env bash
# [OPUS-4.8] sq-hoaj — orphan-proof EC2 harness for the MPC heavy/ceiling matrix
# (PLAN.md M6 "Network tiers", the sq-hoaj OPEN slice).
#
# WHAT IT RUNS. The Tier-3 (netem-shaped) sweep of the ceiling matrix on a single
# disposable EC2 box that HAS the CAP_NET_ADMIN the dev box lacks (so `tc qdisc`
# for LAN shaping actually applies — that is the ONLY reason this needs EC2). The
# cells:
#   N (parties) ∈ {7, 9, 11}           — honest-majority Shamir, t = ⌊(N-1)/2⌋
#   rows/party  ∈ {100, 1000, 10000}   — the hidden-value all-pairs join
#   profile     = lan                  — 1 Gbit/s, 2 ms RTT (netprofiles.rs::lan())
# The all-pairs hidden-value join is `rows²` `secure_equal` opens, so 10^4 rows is
# 10^8 opens — a DELIBERATE ceiling probe (design record §5.3, ORQ SOSP'25 anchor:
# even SOTA O(n log n) MPC joins are minutes-to-tens-of-minutes on LAN). Each cell
# runs the REAL multi-process driver (examples/mpc_net_bench.rs) under a netem LAN
# qdisc via scripts/mpc-netem.sh; the driver asserts the networked result equals
# the in-process reference, so a faster-but-wrong run can never be reported.
#
# HONESTY (design record §5, PLAN.md M6 — this is load-bearing):
#   * Report the REAL minutes-to-tens-of-minutes envelope. NEVER extrapolate past
#     the ceiling: rows are HARD-CAPPED at ROWS_MAX (10^4). A larger scale is
#     refused, not silently run.
#   * The dishonest-majority and WAN-at-scale regimes have ZERO published data
#     points, so this harness does NOT run them and does NOT emit a number for
#     them — it records them as explicit `"status":"no-data-research-risk"` cells.
#     A blank is honest; a fabricated WAN/malicious latency is not.
#   * Results are git-ignored (bench/mpc-ceiling-results/) and never hard-coded
#     into any doc (repo no-hard-coded-perf rule).
#
# ORPHAN-PROOFING (agent-death-independent, hard cap; ≥2 independent mechanisms —
# mirrors the proven scripts/gather-ec2.sh recipe):
#   --instance-initiated-shutdown-behavior terminate  (any in-box shutdown → terminate)
#   + detached  (sleep WATCHDOG_SECS; shutdown -h now)  (no deps, arms immediately)
#   + systemd-run --on-active=WATCHDOG_SECS shutdown    (survives the shell exiting)
#   + a df watchdog                                     (shutdown if disk fills)
# The user-data is FAIL-CLOSED (set -euo pipefail): a setup/clone/build failure —
# or a sweep where EVERY cell failed — writes /root/MPC_CEILING_FAILED and never
# the DONE sentinel, so a broken run can never read as complete. On success it
# writes results + a /root/MPC_CEILING_DONE sentinel then STOPS (it does NOT
# eagerly shut down — the orchestrator pulls results off the DeleteOnTermination
# volume while the box is still alive, then terminates it explicitly, so a
# shutdown race can never lose results). Tag purpose=sparq-bench.
#
# SAFETY. Operates ONLY on the ONE instance it launches; NEVER touches prod
# (i-090531b4ede8f2d3f) or the dev/work box. On exit — success, error, or Ctrl-C —
# the cleanup trap terminates the instance and deletes the ephemeral keypair/SG,
# then runs scripts/orphan-check-bench.sh so a leaked sibling box is surfaced.
#
# Usage:
#   AWS_PROFILE=pss scripts/mpc-ec2-ceiling.sh <branch> [<region>]
#   scripts/mpc-ec2-ceiling.sh --self-test        # hermetic; asserts the rails, no aws
#
# Env overrides (all bounded): MPC_PARTIES="7 9 11", MPC_ROWS="100 1000 10000",
#   MPC_ITYPE, WATCHDOG_SECS, DF_MIN_MB, POLL_GRACE_SECS. Rows above ROWS_MAX are refused. The
#   branch and every override interpolated into the root-run user-data are
#   charset/bounds-validated up front (injection-shaped values are refused before
#   any aws call — see validate_inputs / validate_branch).
set -euo pipefail

PROG="mpc-ec2-ceiling"
log() { printf '[%s] %s\n' "$PROG" "$*" >&2; }
die() { printf '[%s] ERROR: %s\n' "$PROG" "$*" >&2; exit 1; }

# --- the ceiling matrix (bounded; never extrapolated) ----------------------------------
readonly ROWS_MAX=10000                      # HARD ceiling — the design's viable-regime edge
PARTIES="${MPC_PARTIES:-7 9 11}"             # honest-majority N
ROWS="${MPC_ROWS:-100 1000 10000}"           # rows/party for the hidden-value join
PROFILE="lan"                                # honest-majority, LAN — the ONLY viable regime
WATCHDOG_SECS="${WATCHDOG_SECS:-14400}"      # 4h hard cap (10^8 opens can take tens of minutes)
DF_MIN_MB="${DF_MIN_MB:-1024}"               # abort+terminate if free space drops below this
readonly POLL_INTERVAL_SECS=30               # launcher sentinel-poll cadence
POLL_GRACE_SECS="${POLL_GRACE_SECS:-1800}"   # launcher headroom beyond the in-box watchdog (boot/apt/rustup)

# The launcher's wait budget is DERIVED from the in-box watchdog: it must never be
# the shorter deadline, or a healthy multi-hour sweep gets killed by the launcher's
# EXIT trap before the box can write DONE. Kept as a function so the self-test can
# assert the lifetime relationship (budget >= watchdog + grace) hermetically.
poll_attempts() {
  echo $(( (WATCHDOG_SECS + POLL_GRACE_SECS + POLL_INTERVAL_SECS - 1) / POLL_INTERVAL_SECS ))
}

# Final launcher outcome — success ONLY when the DONE sentinel was observed AND the
# result archive was transferred+extracted. Everything else (FAILED sentinel, no
# sentinel, early termination, tar failure) fails closed with a non-zero exit, so
# automation can never mistake a missing/broken artifact for a completed run.
# Factored so the self-test can assert the truth table without aws/ssh.
launcher_outcome() { # <done 0|1> <pulled 0|1>
  [ "$1" = 1 ] && [ "$2" = 1 ]
}

# Validate the row cap up front (in BOTH real-run and self-test paths) so a runaway
# scale can never reach the instance. Never extrapolate past the ceiling.
validate_rows() {
  local r
  for r in $ROWS; do
    case "$r" in
      ''|*[!0-9]*) die "row count '$r' is not a positive integer" ;;
    esac
    [ "$r" -ge 1 ] || die "row count must be >= 1"
    [ "$r" -le "$ROWS_MAX" ] || die "row count $r exceeds ROWS_MAX=$ROWS_MAX — refusing to extrapolate past the ceiling (design record §5.3)"
  done
}

# EVERY value interpolated into the root-run user-data is validated below against a
# strict charset + bounds, and refused BEFORE any aws call. The user-data is a
# root-run cloud-init script, so an unvalidated branch or env value would be a
# root command injection on the instance (quote breakout / heredoc termination).
readonly PARTIES_MAX=99                      # driver spawns N party procs; bound it
readonly WATCHDOG_SECS_MAX=86400             # the box may never outlive 24h
readonly DF_MIN_MB_MAX=1048576               # 1 TiB floor already exceeds the 30 GB volume

validate_pos_int() { # <name> <value> <min> <max>
  case "$2" in
    ''|*[!0-9]*) die "$1 '$2' is not a positive integer" ;;
  esac
  { [ "$2" -ge "$3" ] && [ "$2" -le "$4" ]; } || die "$1 $2 out of bounds [$3, $4]"
}

validate_parties() {
  local n
  for n in $PARTIES; do
    validate_pos_int "party count" "$n" 2 "$PARTIES_MAX"
  done
}

# Conservative git ref charset — [A-Za-z0-9._/-] only, no leading '-' or '.', no
# '..'. Deliberately stricter than git itself: the value must be inert inside the
# double-quoted context of the generated user-data, so quotes, \$, backticks,
# backslashes, whitespace, and newlines can never pass.
validate_branch() {
  case "$1" in
    ''|-*|.*|*..*|*[!A-Za-z0-9._/-]*) die "branch '$1' is not a conservative git ref ([A-Za-z0-9._/-] only, no leading '-'/'.',  no '..')" ;;
  esac
}

validate_inputs() {
  validate_rows
  validate_parties
  validate_pos_int "WATCHDOG_SECS" "$WATCHDOG_SECS" 60 "$WATCHDOG_SECS_MAX"
  validate_pos_int "DF_MIN_MB" "$DF_MIN_MB" 1 "$DF_MIN_MB_MAX"
  validate_pos_int "POLL_GRACE_SECS" "$POLL_GRACE_SECS" 60 21600
}

# Emit the runnable (viable-regime) cell list "N ROWS" — the cartesian product,
# guarded by the cap. Kept as a function so the self-test can assert it directly.
ceiling_cells() {
  local n r
  for n in $PARTIES; do
    for r in $ROWS; do
      printf '%s %s\n' "$n" "$r"
    done
  done
}

# --- the on-instance run script (the netem-shaped sweep) --------------------------------
# Built as a string so the hermetic self-test can assert its safety rails are present
# without launching anything. `\$` escapes defer expansion to the instance shell;
# $PROFILE/$WATCHDOG_SECS/$DF_MIN_MB/the cell list are expanded HERE (this host) —
# every host-expanded value is charset/bounds-validated by validate_inputs /
# validate_branch first, so nothing interpolated can break out of the script.
build_userdata() {
  local repo="$1" branch="$2" cells="$3"
  cat <<UD
#!/bin/bash
# Fail CLOSED: any setup/clone/build failure aborts the run and surfaces as a
# distinct MPC_CEILING_FAILED sentinel — never as an apparently-complete run.
# Per-cell benchmark failures are the ONE narrowly-handled exception (each recorded
# honestly as a cell-failed JSON); if EVERY cell fails, the run also fails closed.
set -euxo pipefail
RUN_ROOT="\${MPC_CEILING_ROOT:-/root}"                       # test seam (hermetic self-test
LOG_FILE="\${MPC_CEILING_LOG:-/var/log/mpc-ceiling.log}"     #  redirects these; prod = /root)
exec > >(tee "\$LOG_FILE") 2>&1
trap 'rc=\$?; if [ "\$rc" -ne 0 ] && [ ! -f "\$RUN_ROOT/MPC_CEILING_DONE" ] && [ ! -f "\$RUN_ROOT/MPC_CEILING_FAILED" ]; then echo "setup/build failed rc=\$rc" > "\$RUN_ROOT/MPC_CEILING_FAILED"; sync; fi' EXIT
if [ -z "\${MPC_CEILING_TEST:-}" ]; then
  # Orphan-proof self-terminate — TWO independent hard caps from DIFFERENT subsystems
  # so a single failure can't leave the box running (mirrors gather-ec2.sh):
  ( sleep $WATCHDOG_SECS; shutdown -h now ) &
  systemd-run --on-active=$WATCHDOG_SECS /sbin/shutdown -h now || true
  # df watchdog — the third mechanism: if the root volume free space drops below the
  # floor at any point, self-terminate rather than wedge on a full disk.
  ( while true; do
      FREE_MB=\$(df -Pm / | awk 'NR==2{print \$4}')
      if [ "\${FREE_MB:-0}" -lt $DF_MIN_MB ]; then
        echo "DF-WATCHDOG: free \${FREE_MB}MB < ${DF_MIN_MB}MB floor — self-terminating"; shutdown -h now; break
      fi
      sleep 60
    done ) &
fi

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq build-essential pkg-config git curl iproute2
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
export PATH="/root/.cargo/bin:\$PATH"
. /root/.cargo/env || true
for _ in \$(seq 1 30); do command -v cargo >/dev/null 2>&1 && break; sleep 2; done
command -v cargo >/dev/null 2>&1 || { echo "FATAL: cargo not on PATH after rustup install"; exit 1; }

cd "\$RUN_ROOT"
git clone -q "$repo" sparq
cd sparq
git fetch -q origin "$branch"
git checkout -q "$branch"
SHA=\$(git rev-parse --short HEAD)
echo "MPC_CEILING_SHA=\$SHA"

# Build the driver + party binary once (mpc-netem.sh reuses them per cell).
cargo build -q -p sparq-mpc --features insecure-test-rng --examples

RESULTS="\$RUN_ROOT/mpc-ceiling-results"
mkdir -p "\$RESULTS"
META="\$RESULTS/run-meta.json"
# Honest run metadata: what regime this IS, and the regimes it deliberately did NOT
# touch (recorded as no-data, never as a fabricated number).
cat > "\$META" <<META_EOF
{
  "sha": "\$SHA",
  "tier": "tier-3-netem-shaped",
  "profile": "$PROFILE",
  "trust_model": "honest-majority",
  "adversary_model": "semi-honest",
  "rows_max": $ROWS_MAX,
  "query_class": "hidden-value-all-pairs-join",
  "no_data_research_risk": [
    {"regime": "dishonest-majority", "reason": "zero published data points; not run — never extrapolate"},
    {"regime": "wan-at-scale",       "reason": "zero published data points; not run — never extrapolate"},
    {"regime": "rows > rows_max",    "reason": "beyond the viable-regime ceiling; refused, not run"}
  ]
}
META_EOF

# Sweep the viable-regime cells under the LAN netem profile. mpc-netem.sh applies
# the qdisc (needs CAP_NET_ADMIN — present here as root), runs the driver, clears.
# Each cell's JSON is captured verbatim (measured wall-clock + bytes + rounds).
TOTAL_CELLS=0
FAILED_CELLS=0
while read -r N ROWS_N; do
  [ -n "\$N" ] || continue
  TOTAL_CELLS=\$((TOTAL_CELLS + 1))
  echo "=== cell N=\$N rows=\$ROWS_N profile=$PROFILE (\$((ROWS_N * ROWS_N)) secure_equal opens) ==="
  CELL="\$RESULTS/join-N\${N}-rows\${ROWS_N}-$PROFILE.json"
  if bash scripts/mpc-netem.sh run join "\$N" "\$ROWS_N" "$PROFILE" > "\$CELL" 2>>"\$RESULTS/cells.log"; then
    cat "\$CELL"
  else
    # The ONE narrowly-handled failure: a single cell may fail (e.g. the 10^8-open
    # ceiling probe) and is recorded honestly; setup/build failures never get here.
    echo "{\"status\":\"cell-failed\",\"parties\":\$N,\"rows\":\$ROWS_N,\"profile\":\"$PROFILE\"}" > "\$CELL"
    echo "cell N=\$N rows=\$ROWS_N FAILED (see cells.log)"
    FAILED_CELLS=\$((FAILED_CELLS + 1))
  fi
  if [ -z "\${MPC_CEILING_TEST:-}" ]; then rm -rf /tmp/* 2>/dev/null || true; fi  # /tmp cleanup between cells (disk hygiene)
done <<CELLS
$cells
CELLS

ls -la "\$RESULTS" || true
if [ "\$TOTAL_CELLS" -eq 0 ] || [ "\$FAILED_CELLS" -eq "\$TOTAL_CELLS" ]; then
  # Every cell failed (or none ran) → environment/driver breakage, not a ceiling
  # measurement. Fail closed: FAILED sentinel, never DONE.
  echo "all \$TOTAL_CELLS cells failed" > "\$RUN_ROOT/MPC_CEILING_FAILED"
  sync
  exit 1
fi
# sentinel LAST — written ONLY after setup+build succeeded and every cell reached an
# explicit terminal state; the orchestrator waits for it, pulls, then terminates.
echo "\$SHA" > "\$RUN_ROOT/MPC_CEILING_DONE"
sync
# Do NOT shut down here — the watchdogs are the only auto-terminate (orphan backstop).
UD
}

# --- hermetic self-test (no aws; asserts the safety + honesty invariants) ---------------
self_test() {
  local fails=0
  _check() {
    if [ "$2" = "$3" ]; then printf '  ok   %s\n' "$1"
    else printf '  FAIL %s\n       got:  %q\n       want: %q\n' "$1" "$2" "$3"; fails=$((fails + 1)); fi
  }
  _grep() {
    if printf '%s' "$2" | grep -qF -- "$3"; then printf '  ok   %s\n' "$1"
    else printf '  FAIL %s (missing: %q)\n' "$1" "$3"; fails=$((fails + 1)); fi
  }
  log "running --self-test (hermetic; no aws calls)"

  # The row cap is enforced.
  ( ROWS="$ROWS_MAX" validate_rows ) && _check "rows == ROWS_MAX accepted" "0" "0"
  if ( ROWS="$((ROWS_MAX + 1))" validate_rows ) 2>/dev/null; then
    _check "rows > ROWS_MAX refused" "accepted" "refused"
  else
    _check "rows > ROWS_MAX refused" "refused" "refused"
  fi

  # Injection-shaped launcher inputs are refused BEFORE any aws call: every value
  # interpolated into the root-run user-data must be inert (quotes, newlines,
  # heredoc terminators, and shell metacharacters must all die in validation).
  _refused() { # <label> <cmd...> — the command is expected to die
    local label="$1"; shift
    if ( "$@" ) 2>/dev/null; then _check "$label" "accepted" "refused"
    else _check "$label" "refused" "refused"; fi
  }
  _with_parties()  { PARTIES="$1"; validate_parties; }
  _with_watchdog() { WATCHDOG_SECS="$1"; validate_inputs; }
  _with_dfmin()    { DF_MIN_MB="$1"; validate_inputs; }
  _with_rows()     { ROWS="$1"; validate_rows; }
  ( validate_inputs ) && _check "default env inputs accepted" "0" "0"
  ( validate_branch "feature/x.y-z1" ) && _check "plain branch ref accepted" "0" "0"
  _refused "branch: embedded quote"           validate_branch 'main"; touch /tmp/pwn; "'
  _refused "branch: command substitution"     validate_branch 'main$(id)'
  _refused "branch: backtick"                 validate_branch 'main`id`'
  _refused "branch: whitespace"               validate_branch 'main branch'
  _refused "branch: newline"                  validate_branch "$(printf 'main\nCELLS')"
  _refused "branch: heredoc terminator UD"    validate_branch "$(printf 'main\nUD')"
  _refused "branch: leading dash"             validate_branch '-oProxyCommand=x'
  _refused "branch: dotdot"                   validate_branch 'a..b'
  _refused "branch: empty"                    validate_branch ''
  _refused "parties: heredoc terminator"      _with_parties '7 CELLS'
  _refused "parties: shell metacharacters"    _with_parties '7; shutdown -h now'
  _refused "parties: newline injection"       _with_parties "$(printf '7\nUD')"
  _refused "parties: below minimum"           _with_parties '1'
  _refused "parties: above PARTIES_MAX"       _with_parties "$((PARTIES_MAX + 1))"
  _refused "watchdog: shell metacharacters"   _with_watchdog '60; rm -rf /'
  _refused "watchdog: above 24h cap"          _with_watchdog "$((WATCHDOG_SECS_MAX + 1))"
  _refused "df floor: command substitution"   _with_dfmin '$(reboot)'
  _refused "rows: zero"                       _with_rows '0'

  # Launcher lifetime covers the in-box watchdog: the poll budget is DERIVED from
  # WATCHDOG_SECS (+ bounded startup grace), so the launcher can never be the
  # shorter deadline that kills a healthy multi-hour sweep before DONE is written.
  _budget_covers() { # <label> <watchdog_secs> <grace_secs>
    local label="$1" budget want
    budget=$( ( WATCHDOG_SECS="$2"; POLL_GRACE_SECS="$3"; echo $(( $(poll_attempts) * POLL_INTERVAL_SECS )) ) )
    want=$(( $2 + $3 ))
    if [ "$budget" -ge "$want" ]; then _check "$label" "covered" "covered"
    else _check "$label" "budget ${budget}s < ${want}s" "covered"; fi
  }
  _budget_covers "poll budget covers current watchdog+grace" "$WATCHDOG_SECS" "$POLL_GRACE_SECS"
  _budget_covers "poll budget covers the 4h default watchdog" 14400 1800
  _budget_covers "poll budget covers the 24h max watchdog"    "$WATCHDOG_SECS_MAX" 1800

  # Fail-closed launcher outcome: success ONLY when DONE was observed AND the
  # result archive transferred+extracted; every other combination exits non-zero.
  _outcome() { # <label> <done> <pulled> <want ok|fail>
    local got
    if launcher_outcome "$2" "$3"; then got=ok; else got=fail; fi
    _check "$1" "$got" "$4"
  }
  _outcome "outcome: DONE + results pulled -> success" 1 1 ok
  _outcome "outcome: DONE but pull failed -> fail"     1 0 fail
  _outcome "outcome: no DONE sentinel -> fail"         0 1 fail
  _outcome "outcome: neither -> fail"                  0 0 fail

  # The matrix is exactly the specced {7,9,11} × {100,1000,10000} product.
  local cells; cells="$(ceiling_cells)"
  _check "cell count == 9"          "$(printf '%s\n' "$cells" | grep -c .)" "9"
  _grep  "cell N=7 rows=10000"      "$cells" "7 10000"
  _grep  "cell N=11 rows=100"       "$cells" "11 100"

  # The user-data carries every safety rail + honesty marker.
  local ud; ud="$(build_userdata "https://example/repo.git" "main" "$cells")"
  _grep "watchdog: detached sleep"      "$ud" "sleep $WATCHDOG_SECS; shutdown -h now"
  _grep "watchdog: systemd-run timer"   "$ud" "systemd-run --on-active=$WATCHDOG_SECS"
  _grep "watchdog: df floor"            "$ud" "DF-WATCHDOG"
  _grep "tmp cleanup between cells"     "$ud" "rm -rf /tmp/*"
  _grep "runs under netem lan profile"  "$ud" "mpc-netem.sh run join"
  _grep "no-data: dishonest-majority"   "$ud" "dishonest-majority"
  _grep "no-data: wan-at-scale"         "$ud" "wan-at-scale"
  _grep "sentinel written last"         "$ud" "MPC_CEILING_DONE"
  _grep "does NOT eagerly shut down"    "$ud" "watchdogs are the only auto-terminate"
  _grep "fail-closed shell options"     "$ud" "set -euxo pipefail"
  _grep "distinct failure sentinel"     "$ud" "MPC_CEILING_FAILED"

  # Fail-closed hermetic runs (stubbed commands; no aws, no network, no root): a
  # clone failure, a build failure, and an all-cells-failed sweep must each exit
  # non-zero with NO MPC_CEILING_DONE and a distinct MPC_CEILING_FAILED — a broken
  # setup can never masquerade as a completed ceiling run.
  _failclosed() { # <label> <git_mode ok|fail> <cargo_mode ok|fail>
    local label="$1" git_mode="$2" cargo_mode="$3"
    local tdir c rc=0
    tdir="$(mktemp -d)"
    mkdir -p "$tdir/bin" "$tdir/root"
    for c in apt-get curl shutdown systemd-run; do
      printf '#!/bin/sh\nexit 0\n' > "$tdir/bin/$c"
    done
    if [ "$git_mode" = fail ]; then
      printf '#!/bin/sh\nexit 1\n' > "$tdir/bin/git"
    else
      cat > "$tdir/bin/git" <<'GIT_STUB'
#!/bin/sh
if [ "$1" = clone ]; then
  for d in "$@"; do :; done   # last arg = target dir
  mkdir -p "$d"
elif [ "$1" = rev-parse ]; then
  echo stub1234
fi
exit 0
GIT_STUB
    fi
    if [ "$cargo_mode" = fail ]; then printf '#!/bin/sh\nexit 1\n' > "$tdir/bin/cargo"
    else printf '#!/bin/sh\nexit 0\n' > "$tdir/bin/cargo"; fi
    chmod +x "$tdir/bin/"*
    build_userdata "https://example/repo.git" "main" "7 100" > "$tdir/ud.sh"
    ( cd "$tdir/root" && PATH="$tdir/bin:$PATH" MPC_CEILING_TEST=1 \
        MPC_CEILING_ROOT="$tdir/root" MPC_CEILING_LOG="$tdir/ud.log" \
        bash "$tdir/ud.sh" ) >/dev/null 2>&1 || rc=$?
    if [ "$rc" -ne 0 ]; then _check "$label: exits non-zero" "nonzero" "nonzero"
    else _check "$label: exits non-zero" "zero" "nonzero"; fi
    if [ -f "$tdir/root/MPC_CEILING_DONE" ]; then _check "$label: no DONE sentinel" "present" "absent"
    else _check "$label: no DONE sentinel" "absent" "absent"; fi
    if [ -f "$tdir/root/MPC_CEILING_FAILED" ]; then _check "$label: FAILED sentinel written" "present" "present"
    else _check "$label: FAILED sentinel written" "absent" "present"; fi
    rm -rf "$tdir"
  }
  _failclosed "fail-closed: git clone failure"  fail ok
  _failclosed "fail-closed: cargo build failure" ok  fail
  _failclosed "fail-closed: every cell failed"   ok  ok

  echo
  if [ "$fails" -eq 0 ]; then log "self-test PASSED"; return 0; fi
  die "self-test FAILED ($fails check(s))"
}

# --- arg parsing -----------------------------------------------------------------------
case "${1:-}" in
  --self-test) self_test; exit 0 ;;
  -h|--help)   sed -n '2,60p' "$0"; exit 0 ;;
  '')          die "usage: $0 <branch> [region]   (or --self-test)" ;;
esac

BRANCH="$1"
REGION="${2:-${AWS_REGION:-eu-west-2}}"
ITYPE="${MPC_ITYPE:-c7g.2xlarge}"            # 8 vCPU arm64 — the driver spawns N party procs
REPO="https://github.com/sparq-org/sparq.git"
TAGSPEC='ResourceType=instance,Tags=[{Key=purpose,Value=sparq-bench}]'
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RESULTS_DIR="$ROOT/bench/mpc-ceiling-results"

# Refuse anything injection-shaped BEFORE any aws call — these values are
# interpolated into the root-run user-data.
validate_branch "$BRANCH"
validate_inputs
CELLS="$(ceiling_cells)"

command -v aws >/dev/null 2>&1 || die "aws CLI not found — this launcher needs it (try --self-test for the hermetic rail check)"

WORK="$(mktemp -d)"
KEYFILE="$WORK/key"
KEY_NAME="sparq-bench-$$-${RANDOM}"
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
  # Belt-and-braces: surface any leaked sibling bench box (dry-run; never auto-kills prod/dev).
  [ -x "$SCRIPT_DIR/orphan-check-bench.sh" ] && "$SCRIPT_DIR/orphan-check-bench.sh" --region "$REGION" || true
}
trap cleanup EXIT

log "resolve AMI / network in $REGION"
AMI=$(aws ec2 describe-images --region "$REGION" --owners 099720109477 \
  --filters "Name=name,Values=ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-arm64-server-*" "Name=state,Values=available" \
  --query 'sort_by(Images,&CreationDate)[-1].ImageId' --output text)
VPC=$(aws ec2 describe-vpcs --region "$REGION" --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)
SUBNET=$(aws ec2 describe-subnets --region "$REGION" --filters Name=vpc-id,Values="$VPC" "Name=default-for-az,Values=true" --query 'Subnets[0].SubnetId' --output text)
MYIP=$(curl -s https://checkip.amazonaws.com | tr -d '[:space:]')
[ -n "$MYIP" ] || die "could not determine public IP (checkip returned empty)"
log "AMI=$AMI VPC=$VPC SUBNET=$SUBNET MYIP=$MYIP"

log "keypair + locked-down security group (ssh from $MYIP/32 only)"
aws ec2 import-key-pair --region "$REGION" --key-name "$KEY_NAME" --public-key-material "fileb://${KEYFILE}.pub" >/dev/null
SG_ID=$(aws ec2 create-security-group --region "$REGION" --group-name "$KEY_NAME" --description "sparq mpc ceiling (ephemeral)" --vpc-id "$VPC" --query 'GroupId' --output text)
aws ec2 authorize-security-group-ingress --region "$REGION" --group-id "$SG_ID" --protocol tcp --port 22 --cidr "${MYIP}/32" >/dev/null

USERDATA="$(build_userdata "$REPO" "$BRANCH" "$CELLS")"

log "launching $ITYPE (matrix: N∈{$PARTIES} × rows∈{$ROWS}, profile=$PROFILE, watchdog=${WATCHDOG_SECS}s)"
INSTANCE_ID=$(aws ec2 run-instances --region "$REGION" --image-id "$AMI" --instance-type "$ITYPE" \
  --instance-initiated-shutdown-behavior terminate \
  --key-name "$KEY_NAME" --security-group-ids "$SG_ID" \
  --subnet-id "$SUBNET" --associate-public-ip-address \
  --block-device-mappings '[{"DeviceName":"/dev/sda1","Ebs":{"VolumeSize":30,"VolumeType":"gp3","DeleteOnTermination":true}}]' \
  --tag-specifications "$TAGSPEC" \
  --user-data "$USERDATA" \
  --query 'Instances[0].InstanceId' --output text)
case "$INSTANCE_ID" in
  i-*) ;;
  *) INSTANCE_ID=""; die "run-instances did not return a valid instance id (launch failed) — aborting" ;;
esac
log "launched INSTANCE_ID=$INSTANCE_ID"
aws ec2 wait instance-running --region "$REGION" --instance-ids "$INSTANCE_ID"
IP=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)
log "public IP=$IP; waiting for sshd"
SSH_UP=0
for i in $(seq 1 40); do ssh $SSHO "ubuntu@$IP" true 2>/dev/null && { log "ssh up"; SSH_UP=1; break; }; sleep 10; done
[ "$SSH_UP" = 1 ] || die "sshd never became reachable on $IP after 40 attempts — aborting (cleanup trap terminates $INSTANCE_ID)"

mkdir -p "$RESULTS_DIR"
POLL_ATTEMPTS="$(poll_attempts)"
log "polling for /root/MPC_CEILING_DONE every ${POLL_INTERVAL_SECS}s for up to $((POLL_ATTEMPTS * POLL_INTERVAL_SECS))s (covers the ${WATCHDOG_SECS}s in-box watchdog + ${POLL_GRACE_SECS}s startup grace)…"
DONE=0
for i in $(seq 1 "$POLL_ATTEMPTS"); do
  sleep "$POLL_INTERVAL_SECS"
  if ssh $SSHO "ubuntu@$IP" "sudo test -f /root/MPC_CEILING_DONE" 2>/dev/null; then
    log "  [$i] sentinel present — pulling results"
    DONE=1; break
  fi
  if ssh $SSHO "ubuntu@$IP" "sudo test -f /root/MPC_CEILING_FAILED" 2>/dev/null; then
    log "  [$i] FAILED sentinel present — setup/build/sweep failed on-instance (fail-closed; no results to trust)"
    break
  fi
  STATE=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" --query 'Reservations[0].Instances[0].State.Name' --output text 2>/dev/null || echo unknown)
  log "  [$i] state=$STATE; waiting for sentinel"
  [ "$STATE" = "terminated" ] && { log "  instance terminated before sentinel — results may be lost"; break; }
done

PULLED=0
if [ "$DONE" = 1 ]; then
  if ssh $SSHO "ubuntu@$IP" "sudo tar -C /root -cf - mpc-ceiling-results 2>/dev/null" | tar -C "$ROOT/bench" -xf -; then
    PULLED=1
    log "pulled ceiling results into $RESULTS_DIR (git-ignored):"
    for f in "$RESULTS_DIR"/*.json; do [ -f "$f" ] || continue; echo "--- $(basename "$f") ---"; cat "$f"; echo; done
  else
    log "tar pull/extract FAILED — DONE was observed but the result archive did not transfer"
  fi
else
  # Best-effort diagnostics only — the run is already a failure at this point.
  log "NO sentinel — pulling /var/log/mpc-ceiling.log for diagnosis"
  ssh $SSHO "ubuntu@$IP" "sudo tail -160 /var/log/mpc-ceiling.log 2>/dev/null" >&2 || true
fi

# Fail CLOSED at the launcher boundary: exit non-zero unless DONE was observed AND
# the result archive landed locally, so automation can never treat a failed or
# missing benchmark artifact as a successful ceiling run.
launcher_outcome "$DONE" "$PULLED" \
  || die "ceiling run NOT confirmed (done=$DONE pulled=$PULLED) — failing closed; cleanup trap terminates $INSTANCE_ID"
log "done; cleanup trap terminates $INSTANCE_ID, deletes keypair/SG, and runs orphan-check"
