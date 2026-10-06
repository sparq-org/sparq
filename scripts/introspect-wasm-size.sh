#!/usr/bin/env bash
# sq-aq5 — sparq-introspect wasm bundle-size measurement (INFORMATIONAL).
#
# sparq-introspect is a pure library (rlib-only): every statistic it mines is a sorted
# scan over sparq-core's already-built permutation indexes — no threads, no syscalls, no
# wall clock, no network — so the bead's expectation was that it ports to wasm trivially.
# This script makes the "trivial" claim a measured number rather than an assertion: it
# builds the crate's headless test image for `wasm32-unknown-unknown` in release and
# reports the `.wasm` byte size.
#
# This is a MEASUREMENT, not a gated baseline. Unlike `wasm_bundle_bytes` (the perf-gate
# metric for the SHIPPED sparq-wasm browser bundle), introspect is not in any shipped wasm
# bundle's dependency graph, so there is nothing to ratchet — a number in the CI job
# summary is enough to make a future regression visible. The figure includes the whole
# linked sparq-core/oxrdf engine plus the wasm-bindgen-test harness, so it is an UPPER
# bound on what introspect would add to a browser bundle, not the marginal cost.
#
# Exit 0 always (informational): skips gracefully when the wasm target is absent.
set -euo pipefail

TARGET="wasm32-unknown-unknown"

if ! rustup target list --installed 2>/dev/null | grep -q "$TARGET"; then
  echo "introspect-wasm-size: $TARGET target not installed — skipping (informational)."
  exit 0
fi

# Build the headless test image in release. `--tests` produces the `tests/wasm.rs`
# binary; release strips dead code so the figure reflects the reachable surface.
cargo build --release -q -p sparq-introspect --target "$TARGET" --tests

WASM_BIN="$(find "target/$TARGET/release/deps" -name 'wasm-*.wasm' -printf '%s %p\n' 2>/dev/null \
  | sort -rn | head -1 | cut -d' ' -f2-)"

if [ -z "$WASM_BIN" ] || [ ! -f "$WASM_BIN" ]; then
  echo "introspect-wasm-size: no .wasm test image found — skipping (informational)."
  exit 0
fi

BYTES="$(wc -c < "$WASM_BIN" | tr -d ' ')"
KIB=$(( BYTES / 1024 ))
echo "### sparq-introspect wasm bundle size"
echo ""
echo "- target: \`$TARGET\` (release, headless test image — upper bound, includes engine + test harness)"
echo "- artifact: \`$WASM_BIN\`"
echo "- size: **${BYTES} bytes** (~${KIB} KiB)"
