#!/usr/bin/env bash
# sq-7d3dj.5 (epic sq-7d3dj) — canonical loopback HTTP-throughput harness runner.
#
# Sibling of scripts/ci-bench.sh / hw-bench.sh / ec2-bench.sh. Builds and runs the
# bench/serve-throughput harness (a standalone cargo project outside the root workspace),
# which stands up the REAL in-process sparq-server (sparq_server::serve) on an ephemeral
# 127.0.0.1:0 loopback port and drives concurrent SPARQL SELECT/ASK queries, reporting
# {req/s, p50/p99 ms, peak RSS}.
#
#   scripts/serve-throughput-bench.sh [--smoke] [--json PATH] [-- <harness args...>]
#
# HONESTY: req/s + latency are WALL-CLOCK sensitive. On a shared box the numbers are
# directional brackets ONLY, never a claim (matching how the competitor-gather + perf-gate
# treat wall-clock metrics). The canonical numbers come from a quiet EC2 runner. This runner
# does NOT feed the deterministic perf-gate — req/s is a trend/EC2 metric, so nothing here is
# wired into scripts/perf-gate.py or bench/benchmarks emit. It is deliberately kept OUT of
# scripts/ci-bench.sh for exactly that reason.
set -euo pipefail
cd "$(dirname "$0")/.."

HARNESS_DIR="bench/serve-throughput"
JSON=""
SMOKE=0
EXTRA=()

while [ "$#" -gt 0 ]; do
  case "$1" in
    --smoke) SMOKE=1; shift ;;
    --json) JSON="${2:?--json needs a path}"; shift 2 ;;
    --) shift; EXTRA=("$@"); break ;;
    -h|--help)
      sed -n '2,18p' "$0"; exit 0 ;;
    *) EXTRA+=("$1"); shift ;;
  esac
done

echo "# building $HARNESS_DIR (release) ..." >&2
( cd "$HARNESS_DIR" && cargo build --release --quiet )

ARGS=()
[ "$SMOKE" -eq 1 ] && ARGS+=(--smoke)
[ -n "$JSON" ] && ARGS+=(--json "$JSON")
ARGS+=("${EXTRA[@]:-}")
# Drop an empty trailing element the ${EXTRA[@]:-} default may introduce under `set -u`.
CLEAN=(); for a in "${ARGS[@]}"; do [ -n "$a" ] && CLEAN+=("$a"); done

# Run via `cargo run` from the harness dir so cargo — not a hard-coded path — resolves the
# binary. This tracks CARGO_TARGET_DIR / an explicit --target (common in CI/dev shells),
# where the artifact is NOT under `$HARNESS_DIR/target/release/`. --quiet keeps STDOUT the
# harness's own output only (cargo status goes to STDERR); after the build above it is a fast
# freshness check, no rebuild.
echo "# running: cargo run --release --bin serve_throughput -- ${CLEAN[*]}" >&2
( cd "$HARNESS_DIR" && cargo run --release --quiet --bin serve_throughput -- "${CLEAN[@]}" )
