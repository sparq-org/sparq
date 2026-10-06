#!/usr/bin/env bash
# Fetches the official W3C SPARQL test suites (github.com/w3c/rdf-tests) at a
# PINNED commit into the gitignored tests/w3c/ directory. Test data is never
# committed to this repo — run this script before `cargo run -p sparq-conformance`.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# sq-nj0pd: retry the shallow clone/fetch — a transient GitHub reset on
# this (the FIRST network op the SPARQL + inference conformance lanes run) used to
# red-gate the required check in ~18s with no real conformance error, stalling
# --auto merges on unrelated PRs. The pin check below is unchanged, so a retry can
# never substitute drifted test data.
# shellcheck source=scripts/lib/fetch-retry.sh
. "$ROOT/scripts/lib/fetch-retry.sh"

# Pinned w3c/rdf-tests commit (main, 2026-06). Bump deliberately: pass-rates are
# only comparable across runs when the suite revision is fixed.
PIN="f25dbc092c654d792974848e81bb519d7328f0e8"

DEST="$ROOT/tests/w3c/rdf-tests"

retry_git_clone_pinned "https://github.com/w3c/rdf-tests" "$DEST" "$PIN"
echo "rdf-tests pinned at $PIN."
