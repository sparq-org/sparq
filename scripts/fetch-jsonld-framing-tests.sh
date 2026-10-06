#!/usr/bin/env bash
# sq-oy1f.19 — fetches the official W3C JSON-LD 1.1 Framing test suite
# (github.com/w3c/json-ld-framing) at a PINNED commit into the gitignored
# tests/w3c/json-ld-framing directory. Framing is a SEPARATE W3C repo from the
# JSON-LD API suite (toRdf/fromRdf/compact live in w3c/json-ld-api, fetched by
# scripts/fetch-jsonld-tests.sh); the frame manifest + fixtures live here.
# Mirrors scripts/fetch-jsonld-tests.sh: test data is never committed — run this
# before the JSON-LD `frame` conformance runner. The runner SKIPS itself when the
# suite is absent, so a fresh checkout stays green offline.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# sq-nj0pd: retry the shallow clone/fetch (shared helper) so a transient
# GitHub reset doesn't red-gate the JSON-LD frame conformance lane. Pin unchanged.
# shellcheck source=scripts/lib/fetch-retry.sh
. "$ROOT/scripts/lib/fetch-retry.sh"

# Pinned w3c/json-ld-framing commit (main, 2026-06). Bump deliberately: the
# JSON-LD framing pass-count floor in crates/sparq-conformance/src/floors/frame.rs
# (sq-oy1f.40 — the lib-side single source, imported by the runner +
# scoreboard + ci grep) is calibrated against THIS suite revision — pass counts are
# only comparable across runs when the suite is fixed.
PIN="3bf782ba9a40dd1b143435abe386d38df64f2b47"

DEST="$ROOT/tests/w3c/json-ld-framing"

retry_git_clone_pinned "https://github.com/w3c/json-ld-framing" "$DEST" "$PIN"
echo "json-ld-framing pinned at $PIN."
