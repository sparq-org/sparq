#!/usr/bin/env bash
# sq-tmsd6: Fetches the SolidLab ODRL-Test-Suite at a PINNED commit into
# the gitignored tests/odrl-test-suite/ directory. Test data is FETCHED, never
# committed to this repo (mirrors scripts/fetch-conformance.sh / the W3C suites) —
# run this before `cargo test -p sparq-policy --test odrl_test_suite`.
#
# Suite: github.com/SolidLabResearch/ODRL-Test-Suite (MIT, LICENSE.md). Its
# self-describing Turtle cases under data/ are the differential oracle the
# crate-local runner evaluates through sparq-policy's real evaluate() path.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# sq-nj0pd: retry the shallow clone/fetch (shared helper) so a transient
# GitHub reset doesn't red-gate the ODRL conformance lane. Pin check unchanged.
# shellcheck source=scripts/lib/fetch-retry.sh
. "$ROOT/scripts/lib/fetch-retry.sh"

# Pinned SolidLabResearch/ODRL-Test-Suite commit (main, 2025-10-15). Bump
# deliberately: pass-rates are only comparable across runs when the suite
# revision is fixed (same discipline as the W3C suite pin).
PIN="7958238e72511059478e43ec9e57b053504cfd2c"

DEST="$ROOT/tests/odrl-test-suite"

retry_git_clone_pinned "https://github.com/SolidLabResearch/ODRL-Test-Suite" "$DEST" "$PIN"
echo "ODRL-Test-Suite pinned at $PIN."
