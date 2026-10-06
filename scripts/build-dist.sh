#!/usr/bin/env bash
# Build hardware-tiered distribution binaries of sparq-cli.
#
# WHY TIERS (and why so few): the hardware-optimization study
# (research/hardware/optimization-findings.md) measured that on aarch64-apple-darwin,
# `-C target-cpu=native`/`apple-m1` unlock ZERO new instructions over the default target
# (the Apple-Darwin baseline already enables all 28 features: neon, aes, sha3, dotprod,
# fp16, lse, rcpc2, …) and give NO measurable speedup. So Apple silicon ships exactly ONE
# binary. Microarch tiering is real only on x86-64 (and, mildly, generic aarch64-linux),
# where a higher target-cpu genuinely unlocks AVX2/BMI2/FMA (and LSE atomics).
#
# Usage:
#   scripts/build-dist.sh                # build every tier this HOST can build natively
#   scripts/build-dist.sh <tier>         # build one tier (see TIERS below)
#   PGO=1 scripts/build-dist.sh <tier>   # profile-guided: instrument → train → rebuild
#
# Tiers (name → rustc target + target-cpu):
#   arm64-darwin   aarch64-apple-darwin            (no target-cpu — already optimal)
#   arm64-linux    aarch64-unknown-linux-gnu       -Ctarget-cpu=neoverse-n1   (+lse atomics)
#   x64-baseline   x86_64-unknown-linux-gnu        -Ctarget-cpu=x86-64        (SSE2, runs anywhere)
#   x64-v3         x86_64-unknown-linux-gnu        -Ctarget-cpu=x86-64-v3     (AVX2/BMI2/FMA — ~95% of live servers)
#   x64-v4         x86_64-unknown-linux-gnu        -Ctarget-cpu=x86-64-v4     (AVX-512 — newest Intel/AMD)
#
# Cross-compiling the non-host tiers needs a cross toolchain (this repo's CI does it on
# native runners — see .github/workflows/dist.yml). Locally, this script builds only the
# tiers whose rustc target is installed; it prints the recipe for the rest.
#
# SUPPLY CHAIN: each dist binary embeds its resolved dependency manifest via cargo-auditable
# (sq-ytnq), matching dist.yml/release.yml — `cargo audit bin dist/sparq-cli-<tier>` works
# post-build. Install once: `cargo install cargo-auditable` (the script warns + falls back
# to a plain build if it is missing).
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=dist
mkdir -p "$OUT"

# sq-ytnq (area:release): embed the resolved dependency manifest INTO the
# shipped dist binary via cargo-auditable, so a `dist/sparq-cli-<tier>` built with this
# script is self-describing for post-incident audit (`cargo audit bin dist/sparq-cli-<tier>`)
# — exactly as the CI dist/release matrices already do (dist.yml + release.yml#package,
# sq-toze.8/.23). `cargo auditable build` is a drop-in for `cargo build` with identical
# flags + output path; it only embeds a compressed dep graph (near-zero size, no runtime
# cost). This is a LOCAL helper, not a CI runner that pre-installs the tool, so if
# cargo-auditable is absent we warn once and fall back to plain `cargo build` rather than
# hard-failing a developer's build — but the self-describing path is the default.
CARGO_BUILD="cargo build"
if cargo auditable --version >/dev/null 2>&1; then
  CARGO_BUILD="cargo auditable build"
else
  echo "WARN: cargo-auditable not found — dist binaries will NOT embed their dependency" >&2
  echo "      manifest (sq-ytnq). Install for self-describing artifacts: cargo install cargo-auditable" >&2
fi

# tier → "rustc-target target-cpu"
tier_spec() {
  case "$1" in
    arm64-darwin) echo "aarch64-apple-darwin" "" ;;
    arm64-linux)  echo "aarch64-unknown-linux-gnu" "neoverse-n1" ;;
    x64-baseline) echo "x86_64-unknown-linux-gnu" "x86-64" ;;
    x64-v3)       echo "x86_64-unknown-linux-gnu" "x86-64-v3" ;;
    x64-v4)       echo "x86_64-unknown-linux-gnu" "x86-64-v4" ;;
    *) echo "unknown tier: $1" >&2; return 1 ;;
  esac
}

ALL_TIERS="arm64-darwin arm64-linux x64-baseline x64-v3 x64-v4"

build_one() {
  local tier="$1"
  read -r target cpu < <(tier_spec "$tier")
  local flags=""
  [ -n "$cpu" ] && flags="-Ctarget-cpu=$cpu"
  # LSE atomics on the aarch64-linux server tier (neoverse implies it, but be explicit).
  [ "$tier" = "arm64-linux" ] && flags="$flags -Ctarget-feature=+lse"

  if ! rustup target list --installed 2>/dev/null | grep -qx "$target"; then
    echo "SKIP $tier — rustc target '$target' not installed."
    echo "     recipe:  RUSTFLAGS=\"$flags\" $CARGO_BUILD --release -p sparq-cli --target $target"
    return 0
  fi

  echo "==> $tier  ($target  ${cpu:-default-cpu})"
  if [ "${PGO:-0}" = "1" ]; then
    build_pgo "$tier" "$target" "$flags"
  else
    RUSTFLAGS="$flags" $CARGO_BUILD --release -p sparq-cli --target "$target"
  fi
  cp "target/$target/release/sparq-cli" "$OUT/sparq-cli-$tier"
  echo "    -> $OUT/sparq-cli-$tier"
}

# Profile-guided optimization: instrument, train on the synthetic workload, rebuild.
# ~3–8% on the branchy planner/eval paths (research/hardware/optimization-findings.md §3).
build_pgo() {
  local tier="$1" target="$2" flags="$3"
  local prof; prof="$(pwd)/target/pgo-$tier"
  local profdata
  profdata="$(find "$HOME/.rustup/toolchains" -name llvm-profdata 2>/dev/null | head -1)"
  [ -z "$profdata" ] && { echo "need llvm-tools: rustup component add llvm-tools-preview" >&2; exit 1; }
  rm -rf "$prof"; mkdir -p "$prof"

  RUSTFLAGS="$flags -Cprofile-generate=$prof" $CARGO_BUILD --release -p sparq-cli --target "$target"
  local TRAIN="${TRAIN_NT:-bench/qlever-synthetic/synthetic.nt}"
  local Q="${TRAIN_Q:-bench/qlever-synthetic/queries}"
  if [ -f "$TRAIN" ]; then
    # JSON output exercises joins + materialise + escaping — the branchy hot paths.
    "target/$target/release/sparq-cli" bench "$TRAIN" ntriples "$Q" 3 json >/dev/null 2>&1 || true
  else
    echo "WARN: no training data at $TRAIN — PGO profile will be thin." >&2
  fi
  "$profdata" merge -o "$prof/merged.profdata" "$prof"/*.profraw
  # The profile-USE rebuild is the SHIPPED binary, so it carries the embedded dep manifest.
  RUSTFLAGS="$flags -Cprofile-use=$prof/merged.profdata" $CARGO_BUILD --release -p sparq-cli --target "$target"
}

if [ $# -ge 1 ]; then
  build_one "$1"
else
  echo "Building all tiers this host can build natively; printing recipes for the rest."
  for t in $ALL_TIERS; do build_one "$t"; done
fi

echo "Done. Artifacts in $OUT/"
