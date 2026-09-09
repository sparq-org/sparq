#!/usr/bin/env bash
# [OPUS-4.8] sq-pl1p — hermetic regression self-test guarding the js-sbom lane against EXTERNAL
# (registry) devDependencies added to a PUBLISHED workspace member (e.g. js/ = @sparq-org/sparq).
# Authored by Opus 4.8 (Fable unavailable; flag for re-review when Fable returns).
#
# WHY (the exact break this pins): gen-js-sbom.sh DERIVES a standalone per-member lock out of the
# repo-root package-lock.json (derive-workspace-member-lock.mjs), then runs cyclonedx-npm twice:
#   * RUNTIME SBOM — cyclonedx shells out to `npm ls ... --package-lock-only --omit=dev`.
#   * DEV SBOM     — cyclonedx shells out to `npm ls ... --package-lock-only` (NO --omit).
# cyclonedx-npm THROWS (exit 254) if its npm ls exits non-zero and --ignore-npm-errors is unset.
# The sq-f04e hardening (#887) only covered workspace LINK deps. It did NOT cover EXTERNAL dev-deps:
# when a published member declares one, the derive script projects that dep's FULL transitive
# closure into the standalone lock; a real dev-dep closure (e.g. @solid/acl-check -> rdflib ->
# undici, or n3) routinely carries UNSATISFIED peerDependencies / version-overlap that
# `npm ls --package-lock-only` (the FULL view) flags as missing/invalid -> ELSPROBLEMS -> exit 254,
# GATING the SBOM lane (the #922 class). #922's workaround relocated the test-only dev-deps to the
# repo-root package.json; sq-pl1p is the proper fix:
#   1. the RUNTIME (`--omit=dev`) view PRUNES the entire dev subtree BEFORE validation, so it stays
#      green AND STRICT (no --ignore-npm-errors) — a real RUNTIME-graph conflict still reds the lane;
#   2. the DEV (full) SBOM passes cyclonedx `--ignore-npm-errors`, which suppresses only npm ls's
#      non-zero EXIT (install-free lock-VALIDATION noise) while STILL enumerating the full component
#      set — nothing is dropped from the build-time SBOM.
#
# This pins THREE load-bearing behaviours over a tiny SYNTHETIC root lock (no real repo content),
# matching the EXACT npm ls flags cyclonedx-npm@5.0.0 uses (builders.js fetchNpmLs):
#   A. RUNTIME STRICT — a published member with an external dev-dep whose closure has an UNSATISFIED
#      peer must still yield a derived lock on which `npm ls --omit=dev` exits 0 (the runtime SBOM
#      keeps strict validation; the dev noise is pruned, never suppressed in the runtime view).
#   B. DEV BREAK IS REAL — the FULL `npm ls` (no --omit) on that SAME lock exits NON-ZERO
#      (ELSPROBLEMS) — proving the break mechanism is live and the --ignore-npm-errors on the dev
#      SBOM is load-bearing, not decorative.
#   C. RUNTIME HONESTY — the runtime (`--omit=dev`) closure still contains the member's REAL runtime
#      dep (fzstd) and NOT the dev-only tree — the published runtime surface is unchanged + complete.
#
# HERMETIC: builds its own throwaway root package-lock.json under a mktemp dir; no repo content, no
# network (`--package-lock-only`). Requires `node` + `npm`. If either is absent we SKIP with a clear
# message rather than fail, so the harness never reds a box that simply lacks the toolchain.
#
# Run:  bash scripts/tests/test_js_sbom_external_devdep.sh   (exit 0 = pass, 1 = a case failed)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DERIVE="${ROOT}/scripts/derive-workspace-member-lock.mjs"

[ -f "$DERIVE" ] || { echo "FATAL: derive script not found at ${DERIVE}"; exit 2; }

if ! command -v node >/dev/null 2>&1 || ! command -v npm >/dev/null 2>&1; then
  echo "SKIP: 'node' and/or 'npm' not on PATH — both are needed to run this self-test."
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# --------------------------------------------------------------------------- #
# Synthetic repo-root lock (lockfileVersion 3) mirroring the real npm-workspaces shape:
#   * member "mbr"  — the "published" member (stands in for js/), runtime dep `fzstd`, EXTERNAL
#                     (registry) dev dep `tooly`.
#   * "tooly"       — an external dev-dep whose transitive closure carries an UNSATISFIED
#                     peerDependency (`peerthing`), exactly the shape a real dev-dep tree
#                     (rdflib/undici/n3 …) surfaces under `npm ls --package-lock-only`. `peerthing`
#                     is intentionally NOT present in the lock — npm reports `missing: peerthing`.
# derive-workspace-member-lock.mjs resolves the root lock RELATIVE TO ITS OWN LOCATION
# (scripts/../package-lock.json), so we copy the script into the temp project so its `..` points at
# the synthetic root, keeping the test fully hermetic.
# --------------------------------------------------------------------------- #
PROJ="${TMP}/proj"
mkdir -p "${PROJ}/scripts" "${PROJ}/mbr"
cp "$DERIVE" "${PROJ}/scripts/derive-workspace-member-lock.mjs"

cat > "${PROJ}/package-lock.json" <<'JSON'
{
  "name": "synthetic-monorepo",
  "lockfileVersion": 3,
  "requires": true,
  "packages": {
    "": { "name": "synthetic-monorepo", "workspaces": ["mbr"] },
    "mbr": {
      "name": "@synthetic/mbr",
      "version": "0.1.0",
      "license": "MIT",
      "dependencies": { "fzstd": "^0.1.1" },
      "devDependencies": { "tooly": "^1.0.0" }
    },
    "node_modules/fzstd": {
      "version": "0.1.1",
      "resolved": "https://registry.npmjs.org/fzstd/-/fzstd-0.1.1.tgz",
      "integrity": "sha512-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=="
    },
    "node_modules/tooly": {
      "version": "1.0.0",
      "resolved": "https://registry.npmjs.org/tooly/-/tooly-1.0.0.tgz",
      "integrity": "sha512-BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB==",
      "peerDependencies": { "peerthing": "^3.0.0" }
    }
  }
}
JSON

# Member manifest co-located with the derived lock (npm reconciles it against the lock root).
cat > "${PROJ}/mbr/package.json" <<'JSON'
{
  "name": "@synthetic/mbr",
  "version": "0.1.0",
  "dependencies": { "fzstd": "^0.1.1" },
  "devDependencies": { "tooly": "^1.0.0" }
}
JSON

OUT_LOCK="${PROJ}/mbr/package-lock.json"
if ! node "${PROJ}/scripts/derive-workspace-member-lock.mjs" mbr "$OUT_LOCK" >/tmp/derive-extdev.log 2>&1; then
  echo "FAIL: derive script exited non-zero on a member with an external dev dep:"
  sed 's/^/    /' /tmp/derive-extdev.log
  exit 1
fi

# Sanity: the external dev-dep WAS projected into the derived lock (the derive script does its job).
node - "$OUT_LOCK" <<'NODE'
const fs = require("fs");
const p = JSON.parse(fs.readFileSync(process.argv[2], "utf8")).packages;
const fail = (m) => { console.error("FAIL: " + m); process.exit(1); };
if (!p["node_modules/fzstd"]) fail("the member's real runtime dep fzstd was not projected");
if (!p["node_modules/tooly"]) fail("the member's external dev dep tooly was not projected into the lock");
console.log("PASS: derive projected both the runtime dep (fzstd) and the external dev dep (tooly)");
NODE

# The EXACT npm ls flags cyclonedx-npm@5.0.0 emits (builders.js fetchNpmLs); gen-js-sbom.sh passes
# --no-workspaces -> cyclonedx maps that to `--workspaces=false`.
RUNTIME_LS=(npm ls --json --long --all --package-lock-only --omit=dev --workspaces=false)
DEV_LS=(npm ls --json --long --all --package-lock-only --workspaces=false)

# --------------------------------------------------------------------------- #
# Case A (TEETH — RUNTIME STRICT): the runtime SBOM's npm ls (--omit=dev) must exit 0. The dev tree
# (incl. the unsatisfied peer) is pruned BEFORE validation, so the runtime SBOM stays green WITHOUT
# any --ignore-npm-errors — a genuine runtime conflict would still red this.
# --------------------------------------------------------------------------- #
if ( cd "${PROJ}/mbr" && "${RUNTIME_LS[@]}" ) >/tmp/extdev-omitdev.json 2>/tmp/extdev-omitdev.err; then
  echo "PASS: runtime npm ls --omit=dev exits 0 on a derived lock carrying an external dev-dep tree"
else
  echo "FAIL: runtime npm ls --omit=dev errored on a derived lock with an external dev-dep (sq-pl1p regression):"
  sed 's/^/    /' /tmp/extdev-omitdev.err
  exit 1
fi

# --------------------------------------------------------------------------- #
# Case B (the break is REAL): the FULL npm ls (no --omit) on the SAME derived lock MUST exit
# non-zero (ELSPROBLEMS: missing peer). This proves the #922 break mechanism is live and that the
# `--ignore-npm-errors` on the DEV SBOM in gen-js-sbom.sh is load-bearing, not decorative. If npm
# ever stops flagging this (so the full view goes green on its own) the test SKIPS this assertion
# loudly rather than failing — the hardening is then simply belt-and-braces.
# --------------------------------------------------------------------------- #
if ( cd "${PROJ}/mbr" && "${DEV_LS[@]}" ) >/tmp/extdev-full.json 2>/tmp/extdev-full.err; then
  echo "NOTE: full npm ls (no --omit) unexpectedly exited 0 — this npm no longer flags the unsatisfied"
  echo "      peer under --package-lock-only; the dev-SBOM --ignore-npm-errors is then belt-and-braces."
else
  echo "PASS: full npm ls (no --omit) exits NON-ZERO on the external-dev-dep lock — the #922 break is real,"
  echo "      so the dev-SBOM cyclonedx --ignore-npm-errors in gen-js-sbom.sh is load-bearing."
  # Confirm it is the expected lock-validation noise (ELSPROBLEMS), not an unrelated hard crash.
  if ! grep -q "ELSPROBLEMS" /tmp/extdev-full.err; then
    echo "FAIL: the full npm ls failed for a NON-ELSPROBLEMS reason (unexpected) — investigate:"
    sed 's/^/    /' /tmp/extdev-full.err
    exit 1
  fi
fi

# --------------------------------------------------------------------------- #
# Case C (RUNTIME HONESTY): the runtime (--omit=dev) closure must still contain the member's REAL
# runtime dep fzstd and MUST NOT contain the dev-only `tooly` — the published runtime surface is
# unchanged and complete (the fix never drops a real runtime dep to make the lane pass).
# --------------------------------------------------------------------------- #
node - /tmp/extdev-omitdev.json <<'NODE'
const fs = require("fs");
const tree = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const deps = Object.keys(tree.dependencies || {});
const fail = (m) => { console.error("FAIL: " + m + " (runtime deps were: " + JSON.stringify(deps) + ")"); process.exit(1); };
if (!deps.includes("fzstd")) fail("runtime (--omit=dev) tree is missing the real runtime dep fzstd");
if (deps.includes("tooly")) fail("the external dev-only dep (tooly) leaked into the runtime (--omit=dev) tree");
console.log("PASS: runtime (--omit=dev) closure is exactly the real runtime dep set (fzstd); no dev-dep leak");
NODE

echo "All js-sbom external-dev-dep self-tests passed."
