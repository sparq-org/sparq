#!/usr/bin/env bash
# Fetches the W3C SHACL test suite (w3c/data-shapes) at a pinned commit into
# crates/sparq-shacl/tests/shacl/ (gitignored). The `w3c_core` integration test
# skips itself when the suite is absent, so this is opt-in.
set -euo pipefail

# Pinned 2026-11-12 (main). Bump deliberately; the scoreboard baseline in
# tests/w3c_core.rs is calibrated against this revision.
PIN=04ecbb4a5b930f4fa9df6c399ac5c98ccec672c9
# SHACL_TESTS_PIN overrides the pin for one run (earl-report.sh uses it to report
# against the current suite). The runners' ratchets hold only at PIN.
PIN="${SHACL_TESTS_PIN:-$PIN}"

dir="$(cd "$(dirname "$0")" && pwd)/tests/shacl"
repo="$dir/data-shapes"

if [ -d "$repo/.git" ]; then
  if [ "$(git -C "$repo" rev-parse HEAD)" = "$PIN" ]; then
    echo "SHACL test suite already present at $PIN"
    exit 0
  fi
  git -C "$repo" fetch origin "$PIN"
else
  mkdir -p "$dir"
  git clone https://github.com/w3c/data-shapes "$repo"
fi
git -C "$repo" checkout --detach "$PIN"
echo "SHACL test suite ready: $repo @ $PIN"
