#!/usr/bin/env bash
# sq-oy1f.2 — fetches the official W3C JSON-LD 1.1 API test suite
# (github.com/w3c/json-ld-api) at a PINNED commit into the gitignored
# tests/w3c/json-ld-api directory. Mirrors scripts/fetch-conformance.sh (the
# SPARQL rdf-tests fetch): test data is never committed to this repo — run this
# before the JSON-LD conformance runner. The runner SKIPS itself when the suite
# is absent, so a fresh checkout stays green offline.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# sq-nj0pd: retry the shallow clone/fetch (shared helper) so a transient
# GitHub reset doesn't red-gate the JSON-LD conformance lane. Pin check unchanged.
# shellcheck source=scripts/lib/fetch-retry.sh
. "$ROOT/scripts/lib/fetch-retry.sh"

# Pinned w3c/json-ld-api commit (main, 2026-06). Bump deliberately: the
# JSON-LD conformance pass-count floors (toRdf / fromRdf / compact / expand /
# flatten) in crates/sparq-conformance/src/floors/<lane>.rs (sq-oy1f.40 —
# the lib-side single source, imported by the runner + scoreboard + ci grep) are
# calibrated against THIS suite revision — they are only comparable across runs
# when the suite is fixed.
PIN="8654ac22b6cf4f441d2fee915ae634d36b5a8067"

DEST="$ROOT/tests/w3c/json-ld-api"

retry_git_clone_pinned "https://github.com/w3c/json-ld-api" "$DEST" "$PIN"
echo "json-ld-api pinned at $PIN."
