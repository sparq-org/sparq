#!/usr/bin/env bash
# [SONNET-4.6] run_differential_harness.sh — PROOF M1 (sq-3x7dl.14.2).
#
# Drives the XPath differential oracle harness:
#   1. Run the Rust harness's own unit tests (the F&O references, the CPython
#      codepoint-slicing cross-check of the fn:substring window, the codepoint->byte
#      boundary-conversion property test (sq-hjvte), the self-expiring oracle-divergence
#      guards, the per-test-function fault-injection coverage checks).
#   2. Build the harness (zk/xpath/differential/).
#   3. Generate the oracle Noir test file — every expected value read back from sparq's
#      own Rust SPARQL/XSD scalar evaluator, cross-checked bit-for-bit against native f64
#      (doubles) and against an explicit XPath F&O 3.1 sec. 5.4.3 fn:substring window.
#   4. Create a temporary Nargo package depending on the RELEASED noir_XPath face repo.
#   5. Run `nargo test` — every differential test function must pass.
#   6. SELF-TEST: for EACH generated test function, re-generate the file with one
#      corrupted expected value in that function alone (--inject-fault-in) and assert
#      `nargo test` FAILS. A pass there means that test function is VACUOUS (not wired to
#      the circuit) and this script exits non-zero. One variant per function, because a
#      single run over an all-faults file yields ONE exit status: nonzero there says only
#      that SOME test failed, so nine dead sections would hide behind one live one.
#
# Usage:
#   ./scripts/run_differential_harness.sh                     # full run (normal CI)
#   ./scripts/run_differential_harness.sh --oracle-only       # skip the fault self-test
#   ./scripts/run_differential_harness.sh --generate-only     # no nargo; oracle file only
#   ./scripts/run_differential_harness.sh --update-committed  # refresh the committed golden
#
# --generate-only exists because the ORACLE half needs only cargo. CI runs it on every
# PR as a cheap drift guard; the nargo half is gated on the pinned Noir toolchain.
#
# The noir_XPath source is NOT in this repo — it was externalized to the
# sparq-org/noir_XPath face repo (sq-5reoy / #1599). Override the pinned dependency with:
#   XPATH_GIT=https://github.com/sparq-org/noir_XPath  XPATH_TAG=v0.3.0
#   XPATH_DIRECTORY=xpath                       # the PACKAGE inside that repo's workspace
#   XPATH_PATH=/local/checkout/xpath            # a local path dep instead of the git dep
#
# Requirements: cargo always; nargo unless --generate-only (see
# .github/workflows/zk-toolchain.yml for the pinned toolchain install).
#
# This is VERIFICATION, not proof — see zk/xpath/differential/README.md for the TCB.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
XPATH_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
HARNESS_DIR="${XPATH_DIR}/differential"
COMMITTED_ORACLE="${XPATH_DIR}/tests/differential_oracle/src/lib.nr"

# Pinned released face repo. A floating default would silently change what is being
# verified, so both the URL and the TAG are explicit and overridable.
XPATH_GIT="${XPATH_GIT:-https://github.com/sparq-org/noir_XPath}"
XPATH_TAG="${XPATH_TAG:-v0.3.0}"
XPATH_PATH="${XPATH_PATH:-}"
# The PACKAGE within the face repo's Nargo WORKSPACE — required for the git form, see
# write_nargo_toml below.
XPATH_DIRECTORY="${XPATH_DIRECTORY:-xpath}"

ORACLE_ONLY=false
GENERATE_ONLY=false
UPDATE_COMMITTED=false
for arg in "$@"; do
    case "$arg" in
        --oracle-only) ORACLE_ONLY=true ;;
        --generate-only) GENERATE_ONLY=true ;;
        --update-committed) UPDATE_COMMITTED=true ;;
        *) echo "ERROR: unknown argument: $arg" >&2; exit 2 ;;
    esac
done

if ! command -v cargo &> /dev/null; then
    echo "ERROR: cargo not found — install the Rust toolchain first" >&2
    exit 1
fi
if [ "${GENERATE_ONLY}" = "false" ] && ! command -v nargo &> /dev/null; then
    echo "ERROR: nargo not found — see .github/workflows/zk-toolchain.yml for the pinned install," >&2
    echo "       or pass --generate-only to run just the oracle half." >&2
    exit 1
fi

# The harness's OWN tests gate everything below: they pin the F&O reference
# implementations, assert each recorded sparq-engine divergence still reproduces (so a
# stale workaround cannot survive an engine fix), and prove --inject-fault corrupts
# exactly one LIVE assertion. Deliberately NOT --locked: the harness path-deps on
# in-repo crates whose manifests bump on lanes that never run this script, so a pinned
# lock drifts silently on main and would fail an innocent PR (the sq-q134e lesson).
echo "[xpath-differential] Running harness unit tests..."
cargo test --manifest-path "${HARNESS_DIR}/Cargo.toml"

echo "[xpath-differential] Building the Rust harness..."
cargo build --manifest-path "${HARNESS_DIR}/Cargo.toml" --release
HARNESS_BIN="${HARNESS_DIR}/target/release/sparq-xpath-differential"

if [ "${UPDATE_COMMITTED}" = "true" ]; then
    echo "[xpath-differential] Updating the committed oracle: ${COMMITTED_ORACLE}"
    "${HARNESS_BIN}" --output "${COMMITTED_ORACLE}"
    echo "[xpath-differential] Committed oracle updated (stage and commit the change)."
fi

if [ "${GENERATE_ONLY}" = "true" ]; then
    GEN_TMP="$(mktemp "${TMPDIR:-/tmp}/sparq-xpath-oracle.XXXXXX.nr")"
    trap 'rm -f "${GEN_TMP}"' EXIT
    "${HARNESS_BIN}" --output "${GEN_TMP}"
    if ! diff -u "${COMMITTED_ORACLE}" "${GEN_TMP}"; then
        echo "ERROR: the committed oracle file has DRIFTED from the generator's output." >&2
        echo "       Regenerate with: bash zk/xpath/scripts/run_differential_harness.sh --update-committed" >&2
        exit 1
    fi
    echo "[xpath-differential] --generate-only: committed oracle matches the generator. No drift."
    exit 0
fi

# Temporary Nargo packages for the oracle and fault runs; cleaned on exit.
TMPDIR_BASE="$(mktemp -d "${TMPDIR:-/tmp}/sparq-xpath-differential.XXXXXX")"
cleanup() { rm -rf "${TMPDIR_BASE}"; }
trap cleanup EXIT

# Write a Nargo.toml for a test package depending on noir_XPath. The dependency KEY is
# the module alias the generated file imports (`use xpath::…`), so it stays `xpath`
# regardless of the face repo's own package name.
#
# `directory` is REQUIRED for the git form: the face repo's ROOT Nargo.toml is a
# `[workspace]` (members = xpath, xpath_unit_tests, test_packages/*), not a package, so a
# git dep without it fails resolution with "Unexpected workspace definition found ... you
# may need to add a `directory` field". The library package lives in `xpath/` — which is
# also why XPATH_PATH is documented as pointing at `<checkout>/xpath`, not the checkout
# root. (Contrast sparq-org/noir_IEEE754, whose package IS at the repo root; that dep in
# zk/compose/compose_core/Nargo.toml correctly needs no `directory`.)
write_nargo_toml() {
    local pkg_dir="$1"
    local pkg_name="$2"
    mkdir -p "${pkg_dir}/src"
    {
        echo "[package]"
        echo "name = \"${pkg_name}\""
        echo "type = \"lib\""
        echo "authors = [\"\"]"
        echo
        echo "[dependencies]"
        if [ -n "${XPATH_PATH}" ]; then
            echo "xpath = { path = \"${XPATH_PATH}\" }"
        else
            echo "xpath = { git = \"${XPATH_GIT}\", tag = \"${XPATH_TAG}\", directory = \"${XPATH_DIRECTORY}\" }"
        fi
    } > "${pkg_dir}/Nargo.toml"
}

if [ -n "${XPATH_PATH}" ]; then
    echo "[xpath-differential] noir_XPath source: local path ${XPATH_PATH}"
else
    echo "[xpath-differential] noir_XPath source: ${XPATH_GIT} @ ${XPATH_TAG}"
fi

# ---------------------------------------------------------------------------
# Step 1: oracle mode — generate the correct expected values and run nargo test.
# ---------------------------------------------------------------------------
ORACLE_DIR="${TMPDIR_BASE}/oracle"
write_nargo_toml "${ORACLE_DIR}" "sparq_xpath_differential_oracle"

echo "[xpath-differential] Generating the oracle Noir test file..."
"${HARNESS_BIN}" --output "${ORACLE_DIR}/src/lib.nr"

# Drift guard: the committed golden must equal what the generator just produced, so a
# corpus or oracle change cannot land without the reviewable diff.
if ! diff -u "${COMMITTED_ORACLE}" "${ORACLE_DIR}/src/lib.nr"; then
    echo "ERROR: the committed oracle file has DRIFTED from the generator's output." >&2
    echo "       Regenerate with: bash zk/xpath/scripts/run_differential_harness.sh --update-committed" >&2
    exit 1
fi

echo "[xpath-differential] Running nargo test (oracle mode)..."
(cd "${ORACLE_DIR}" && nargo test)

echo "[xpath-differential] Oracle mode: all differential tests passed."

if [ "${ORACLE_ONLY}" = "true" ]; then
    echo "[xpath-differential] --oracle-only: skipping the fault self-test."
    exit 0
fi

# ---------------------------------------------------------------------------
# Step 2: self-test (inject-fault) — prove EACH generated test function can fail.
# A deliberately corrupted expected value MUST make nargo test fail. If it passes, the
# generated file is not actually exercising the circuit and every green run is worthless.
#
# ONE VARIANT PER TEST FUNCTION, not one all-faults file. A `nargo test` run reports ONE
# exit status for the whole file, so a single run over a file with ten faults proves only
# that SOME test failed — nine sections that had quietly stopped being able to fail (a corpus
# that emptied, an expected value derived from the very call it asserts) would hide behind
# the one that still works. Each variant here is byte-identical to the oracle file that
# just ran GREEN except for one corrupted value inside one test function, so its failure
# is attributable to that function. That attribution assumes generation and `nargo test`
# are deterministic — generation is pinned by a generator unit test, and the clean run
# immediately above is the control.
# ---------------------------------------------------------------------------
FAULT_DIR="${TMPDIR_BASE}/fault"
write_nargo_toml "${FAULT_DIR}" "sparq_xpath_differential_fault"

# The generator names the test functions it emits. Looping over ITS list, rather than one
# hard-coded here, means a renamed or dropped section changes the loop instead of silently
# shrinking the set of functions this self-test proves anything about.
FAULT_TESTS=()
while IFS= read -r fault_test; do
    # `if`, not `[ … ] && …`: under `set -e` a failing AND-list as the last command of the
    # loop body would abort the script on a blank line rather than skip it.
    if [ -n "${fault_test}" ]; then
        FAULT_TESTS+=("${fault_test}")
    fi
done < <("${HARNESS_BIN}" --list-fault-sites)

# Cross-check against the file that just ran green: every test function it contains must
# get a variant, or that function's green run stands unproven.
ORACLE_TESTS=()
while IFS= read -r oracle_test; do
    if [ -n "${oracle_test}" ]; then
        ORACLE_TESTS+=("${oracle_test}")
    fi
done < <(sed -n 's/^fn \(differential_oracle_[A-Za-z0-9_]*\)() {$/\1/p' "${ORACLE_DIR}/src/lib.nr")

if [ "${#FAULT_TESTS[@]}" -eq 0 ]; then
    echo "ERROR: the generator reported no fault sites — the self-test would be a no-op." >&2
    exit 1
fi
if [ "${FAULT_TESTS[*]:-}" != "${ORACLE_TESTS[*]:-}" ]; then
    echo "ERROR: the fault-site list does not match the test functions in the oracle file:" >&2
    echo "       sites:  ${FAULT_TESTS[*]:-}" >&2
    echo "       oracle: ${ORACLE_TESTS[*]:-}" >&2
    echo "       a test that just ran green would go unproven." >&2
    exit 1
fi

echo "[xpath-differential] Self-test: ${#FAULT_TESTS[@]} single-fault variants, each MUST fail..."
for fault_test in "${FAULT_TESTS[@]}"; do
    "${HARNESS_BIN}" --inject-fault-in "${fault_test}" "${FAULT_DIR}/src/lib.nr" \
        2> "${TMPDIR_BASE}/inject.log" || { cat "${TMPDIR_BASE}/inject.log" >&2; exit 1; }
    if (cd "${FAULT_DIR}" && nargo test) > "${TMPDIR_BASE}/nargo.log" 2>&1; then
        cat "${TMPDIR_BASE}/nargo.log" >&2
        echo "ERROR: nargo test PASSED with a deliberately corrupted expected value in" >&2
        echo "       ${fault_test} — that test function is VACUOUS: it is not wired to the" >&2
        echo "       Noir circuit, so its green run in oracle mode proves nothing." >&2
        exit 1
    fi
    echo "[xpath-differential]   ${fault_test}: fault detected."
done

echo "[xpath-differential] Self-test PASSED: every generated test function failed on its own fault."
echo "[xpath-differential] The differential harness is non-vacuous and wired to noir_XPath."
