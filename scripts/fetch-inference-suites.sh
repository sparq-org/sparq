#!/usr/bin/env bash
# Fetches the inference/reasoning conformance suites into the gitignored
# tests/w3c/ directory, each at a PINNED revision (pass-rates are only
# comparable when the suite revision is fixed). Test data is never committed.
# Run once, then everything is runnable offline:
#   cargo run --release -p sparq-conformance --bin sparq-inference-conformance
#
# Suites:
#   1. w3c/rdf-tests           — RDF Semantics (rdf/rdf11/rdf-mt) + sparql11/entailment.
#      Same pinned clone as scripts/fetch-conformance.sh (delegated to it).
#   2. w3c/N3                  — the N3 Community Group test suite (tests/N3Tests),
#      the manifests EYE and cwm run.
#   3. OWL 2 test cases        — the W3C OWL WG test repository export (all.rdf,
#      every test case incl. premise/conclusion ontologies as literals). The
#      original wiki (owl.semanticweb.org) is offline; the canonical bulk export
#      is preserved by the Internet Archive. The snapshot TIMESTAMP in the URL is
#      the pin; the sha256 check makes any drift loud.
#   4. W3C RIF WG test cases   — the RIF Working Group test cases, Core dialect
#      (Core_v1.22.zip from www.w3.org/2005/rules/test/repository/zips/). Fetched,
#      NOT vendored (no redistribution license — see step 4 below); the versioned
#      zip + sha256 is the pin; the rif-wg-core lane SKIPS when absent.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# [OPUS-4.8] sq-nj0pd: `retry` + the pinned-clone idiom now live in the shared
# lib so every conformance suite-fetch retries identically (sq-y5dz seeded this
# resilience here; centralising it also covers the delegated fetch-conformance.sh
# in step 1 below — the w3c/rdf-tests clone that was the remaining un-retried
# network op in the inference lane). GitHub and the Internet Archive both reset CI
# transfers occasionally; one blip should not red-gate every engine PR.
# shellcheck source=scripts/lib/fetch-retry.sh
. "$ROOT/scripts/lib/fetch-retry.sh"

# --- 1. w3c/rdf-tests (shared pin with the SPARQL conformance harness) -------
"$ROOT/scripts/fetch-conformance.sh"

# --- 2. w3c/N3 ----------------------------------------------------------------
# Pinned w3c/N3 commit (master, 2026-06).
N3_PIN="23ccf3d56b25cb60a68878a04aae0d52493080f0"
N3_DEST="$ROOT/tests/w3c/n3"

retry_git_clone_pinned "https://github.com/w3c/N3" "$N3_DEST" "$N3_PIN"
echo "w3c/N3 pinned at $N3_PIN."

# --- 3. OWL 2 test cases (W3C OWL WG export, archived) -------------------------
# Pin = the Internet Archive snapshot timestamp (an immutable capture of
# http://owl.semanticweb.org/exports/all.rdf, the full test-case export of the
# OWL WG wiki referenced by the OWL 2 Conformance REC) + sha256 of the payload.
OWL_SNAPSHOT="20160703034201"
OWL_URL="http://web.archive.org/web/${OWL_SNAPSHOT}if_/http://owl.semanticweb.org/exports/all.rdf"
OWL_SHA256="446e9eae0488e7eb58a8bd7db92b5fb358316c63e3cad1749103cd912664bee4"
OWL_DEST="$ROOT/tests/w3c/owl2/all.rdf"

# [OPUS-5] #4935 — the sha256-gated fetch now lives in the shared lib as
# `fetch_pinned_file` (with the `--retry-all-errors` rationale sq-y5dz added
# here). Extracting it made the OFFLINE-ON-HIT property — a payload already at
# the pinned digest costs ZERO network calls — directly testable, which is what
# scripts/tests/test_inference_corpus_cache.py asserts and what makes the CI
# Actions-cache restore of tests/w3c/owl2 genuinely decouple this GATING lane
# from web.archive.org's availability.
if ! fetch_pinned_file "$OWL_URL" "$OWL_DEST" "$OWL_SHA256" \
        "OWL 2 test cases (archived snapshot $OWL_SNAPSHOT)"; then
    echo "  This is the Internet Archive snapshot of the (offline) OWL WG wiki" >&2
    echo "  export; transient connection resets are common. Re-run the job, or" >&2
    echo "  if the archive is persistently unreachable, see scripts/fetch-inference-suites.sh." >&2
    exit 1
fi

# --- 4. W3C RIF Working Group test cases (Core subset) ------------------------
# [FABLE-5] sq-pbz04.5.5 (epic sq-pbz04.5) — the W3C RIF WG test cases, Core dialect,
# for the `rif-wg-core` conformance arm (crates/sparq-conformance/tests/rif_wg_core_suite.rs).
#
# FIXTURES ARE FETCHED, NOT VENDORED (license gate — verified 2026-07). The RIF test
# cases live at https://www.w3.org/2005/rules/test/repository/ (still live in 2026),
# but they carry NO explicit redistribution/test-suite/BSD license marker: the W3C
# default (https://www.w3.org/copyright/intellectual-rights/) is the W3C DOCUMENT
# LICENSE, which forbids derivative works — and W3C explicitly treats "the creation of
# a subset of a test suite" as a prohibited derivative
# (https://www.w3.org/copyright/test-suite-license-2023/). There is no affirmative
# grant to copy the Core subset into this repo, so we use the same fetch-at-test-time +
# skip-if-absent pattern the D-entailment / EL lanes use: the data lands in the
# gitignored tests/w3c/, and the runner SKIPS when it is absent (a fresh offline
# checkout stays green). NO fabricated fixtures.
#
# Pin = the versioned Core_v1.22.zip archive (an immutable convenience bundle the W3C
# publishes alongside the per-test tree) + its sha256. The zip's directory layout —
# `Core_v1.22/Approved/<TestType>/<id>/<id>-{premise,conclusion,input,nonconclusion}.rif`
# — encodes the test type and Approved status by path, so the runner needs no manifest
# parse. Any payload drift is made loud by the sha256 gate below.
RIF_URL="https://www.w3.org/2005/rules/test/repository/zips/Core_v1.22.zip"
RIF_SHA256="421e80f2e2671e4ee95bf0f9c424d0fd83955f5cecf67fb9104422eeb9f3887a"
RIF_DEST="$ROOT/tests/w3c/rif-core"
RIF_STAMP="$RIF_DEST/.Core_v1.22.sha256"

if [ -f "$RIF_STAMP" ] && [ "$(cat "$RIF_STAMP")" = "$RIF_SHA256" ]; then
    echo "RIF WG Core test cases already present (sha256 ok) — nothing to do."
else
    mkdir -p "$RIF_DEST"
    echo "Downloading the W3C RIF WG Core test cases (Core_v1.22.zip)…"
    # www.w3.org occasionally 503s / resets large-archive transfers from CI runner IP
    # ranges (observed intermittently on this endpoint); --retry-all-errors covers the
    # reset/503 that plain --retry misses. The sha256 gate makes drift loud, so
    # retrying is safe.
    if ! curl -sSfL \
            --retry 5 --retry-delay 5 --retry-all-errors --retry-connrefused \
            --connect-timeout 30 --max-time 300 \
            -o "$RIF_DEST/Core_v1.22.zip.tmp" "$RIF_URL"; then
        rm -f "$RIF_DEST/Core_v1.22.zip.tmp"
        echo "ERROR: could not download the RIF WG Core test cases after retries." >&2
        echo "  URL: $RIF_URL" >&2
        echo "  This is a W3C-hosted archive; transient 503/connection resets are" >&2
        echo "  common. Re-run the job; the rif-wg-core lane SKIPS when the data is" >&2
        echo "  absent so a fresh offline checkout stays green." >&2
        exit 1
    fi
    HAVE_SHA="$(sha256_of "$RIF_DEST/Core_v1.22.zip.tmp")"
    if [ "$HAVE_SHA" != "$RIF_SHA256" ]; then
        rm -f "$RIF_DEST/Core_v1.22.zip.tmp"
        echo "ERROR: RIF Core zip checksum mismatch (got $HAVE_SHA, want $RIF_SHA256)." >&2
        exit 1
    fi
    # Extract the Approved/ tree next to the stamp; the runner reads it directly.
    ( cd "$RIF_DEST" \
        && rm -rf Core_v1.22 \
        && unzip -q -o Core_v1.22.zip.tmp \
        && rm -f Core_v1.22.zip.tmp )
    echo "$RIF_SHA256" > "$RIF_STAMP"
    echo "RIF WG Core test cases pinned (Core_v1.22, sha256 ok)."
fi
