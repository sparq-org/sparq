#!/usr/bin/env bash
# sq-tg6b — apply a tc/netem LAN or WAN profile to the loopback
# interface and run the MPC network-tier driver under it (Tier 3 of the MPC
# benchmark matrix). The unshaped loopback run is Tier 2 (no privilege needed);
# the LAN/WAN-shaped runs are Tier 3 and need CAP_NET_ADMIN (root / sudo) for
# `tc qdisc`. The dev box is unprivileged, so Tier 3 runs on a PRIVILEGED host or
# the orphan-proof EC2 bench (bead sq-hoaj) — this script is the canonical way to
# apply the shaping there.
#
# The netem PARAMETERS are the single-source-of-truth in
# crates/sparq-mpc/src/netprofiles.rs (NetemProfile::lan()/wan()); this script
# mirrors them so it is self-contained for a privileged host that only has the
# repo checkout. If you change a profile, change it in BOTH (the Rust test
# `netem_args_render_the_literature_profiles` pins the canonical strings).
#
# Profiles (design record §5.3 — MP-SPDZ / Secrecy / ORQ canonical values).
# PR #96: the `tc` `delay` value is ONE-WAY; the emulated RTT is twice
# it (matching NetemProfile::rtt_ms() == 2 × delay_ms). Labels state the RTT.
#   loopback : unshaped, ~0 RTT                                 (Tier 2 — no tc, no privilege)
#   lan      : delay 1ms  rate 1000000kbit                      (1 Gbit/s,  1ms one-way ⇒ 2ms RTT)
#   wan      : delay 50ms 5ms rate 100000kbit loss 0.1%         (100 Mbit/s, 50ms one-way ⇒ 100ms RTT, 0.1% loss)
#
# Usage:
#   sudo scripts/mpc-netem.sh apply  <lan|wan> [iface=lo]      # apply shaping
#   sudo scripts/mpc-netem.sh clear  [iface=lo]                # remove shaping
#        scripts/mpc-netem.sh run    <aggregate|join> <N> [scale=4] [profile=loopback]
#   sudo scripts/mpc-netem.sh matrix [iface=lo]                # full Tier-2+3 sweep + LAN->WAN multiplier
#
# `run` builds + invokes the driver (examples/mpc_net_bench.rs); when `profile` is
# lan/wan it FIRST applies the shaping (needs sudo), runs, then clears. The
# `--features insecure-test-rng` flag only makes the SIMULATION reproducible; it is
# never a production claim (the feature name is the warning).
#
# HONESTY: a Tier-3 number only exists when `tc` actually ran. This script refuses
# to label a run "lan"/"wan" if `tc qdisc add` failed (e.g. no privilege) — it
# errors rather than emitting an unshaped wall-clock under a shaped label.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
export CARGO_TARGET_DIR
FEATURES="insecure-test-rng"

netem_args() {
  # Mirror NetemProfile::{lan,wan}().netem_args() — keep in sync with netprofiles.rs.
  case "$1" in
    lan) echo "delay 1ms rate 1000000kbit" ;;
    wan) echo "delay 50ms 5ms rate 100000kbit loss 0.1%" ;;
    loopback) echo "" ;;
    *) echo "unknown profile '$1' (use loopback|lan|wan)" >&2; return 1 ;;
  esac
}

apply_profile() {
  local profile="$1" iface="${2:-lo}"
  local args; args="$(netem_args "$profile")"
  if [[ -z "$args" ]]; then
    echo "profile '$profile' is unshaped — no tc applied." >&2
    return 0
  fi
  # Clear any prior qdisc first (idempotent), then apply.
  tc qdisc del dev "$iface" root 2>/dev/null || true
  # shellcheck disable=SC2086 # netem args are intentionally word-split
  if ! tc qdisc add dev "$iface" root netem $args; then
    echo "FATAL: 'tc qdisc add' failed (need CAP_NET_ADMIN / sudo). Tier-3 shaped" >&2
    echo "runs require a privileged host or the EC2 bead sq-hoaj — refusing to" >&2
    echo "report an UNSHAPED wall-clock under the '$profile' label." >&2
    return 1
  fi
  echo "applied netem '$profile' on $iface: $args" >&2
}

clear_profile() {
  local iface="${1:-lo}"
  tc qdisc del dev "$iface" root 2>/dev/null || true
  echo "cleared netem on $iface" >&2
}

build_examples() {
  ( cd "$REPO_ROOT" && cargo build -q -p sparq-mpc --features "$FEATURES" --examples ) >&2
}

run_cell() {
  local query="$1" parties="$2" scale="${3:-4}" profile="${4:-loopback}" iface="${5:-lo}"
  build_examples
  local applied=0
  if [[ "$profile" != "loopback" ]]; then
    apply_profile "$profile" "$iface"
    applied=1
  fi
  # PR #96: crash-safety via an EXIT trap, NOT RETURN. Under `set -e` a
  # failing `cargo run` aborts the whole script, so a RETURN trap (which only fires
  # on normal function return) would never run and the host would stay shaped. An
  # EXIT trap fires on any termination — including the `set -e` abort and Ctrl-C.
  # We ALSO clear explicitly right after the run so a matrix sweep does not keep
  # one cell's shaping applied while the next cell runs (the EXIT trap is purely
  # the safety net for the crash path).
  if [[ "$applied" == 1 ]]; then
    trap 'clear_profile "$iface"' EXIT
  fi
  ( cd "$REPO_ROOT" && cargo run -q -p sparq-mpc --features "$FEATURES" \
      --example mpc_net_bench -- --query "$query" --parties "$parties" --scale "$scale" --json )
  # Explicit clear on the success path + drop the safety-net trap so a subsequent
  # matrix cell registers its own (and we never double-clear at script exit).
  if [[ "$applied" == 1 ]]; then
    clear_profile "$iface"
    trap - EXIT
  fi
}

# Full sweep: both representative cells across N, under loopback/lan/wan, emitting
# each cell's JSON + the LAN->WAN wall-clock multiplier (the ORQ-style ratio).
run_matrix() {
  local iface="${1:-lo}"
  build_examples
  echo "=== MPC network-tier matrix (Tier 2 loopback + Tier 3 lan/wan) ==="
  for query in aggregate join; do
    for parties in 3 5 7; do
      local scale=4
      [[ "$query" == aggregate ]] && scale=1
      for profile in loopback lan wan; do
        echo "--- $query N=$parties scale=$scale profile=$profile ---"
        run_cell "$query" "$parties" "$scale" "$profile" "$iface" || \
          echo "(skipped $profile: shaping unavailable — needs CAP_NET_ADMIN)"
      done
    done
  done
}

cmd="${1:-}"; shift || true
case "$cmd" in
  apply)  apply_profile "$@" ;;
  clear)  clear_profile "$@" ;;
  run)    run_cell "$@" ;;
  matrix) run_matrix "$@" ;;
  *)
    echo "usage: $0 {apply <lan|wan> [iface] | clear [iface] | run <aggregate|join> <N> [scale] [profile] | matrix [iface]}" >&2
    exit 2
    ;;
esac
