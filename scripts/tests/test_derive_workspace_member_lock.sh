#!/usr/bin/env bash
# sq-f04e — hermetic regression self-test for
# scripts/derive-workspace-member-lock.mjs, specifically its WORKSPACE-LINK (`link: true`)
# handling.
#
# WHY: gen-js-sbom.sh DERIVES a standalone per-member lock out of the repo-root
# package-lock.json, then runs cyclonedx-npm, which shells out to
# `npm ls --json --long --all --package-lock-only --omit=dev --workspaces=false`. The derive
# script USED TO re-home a workspace-to-workspace dep edge (a `link: true` node — e.g. a
# published member depending on a sibling `@sparq/*` in packages/*) into the standalone lock
# VERBATIM, as a dangling link stub whose target package node was absent. `npm ls` then crashed
# on the unresolved link with "Cannot read properties of undefined (reading 'extraneous')" ->
# exit 254 — the cryptic SBOM-lane failure discovered in #876/sq-jpki.2. ANY future
# workspace-to-workspace dep edge on a published member would re-trip it. The fix (sq-f04e)
# MATERIALIZES the link target's real package node + its nested closure so npm ls resolves it.
#
# This pins THREE load-bearing behaviours over a tiny SYNTHETIC root lock (no real repo content):
#   1. TEETH (the regression itself) — a member with a `link: true` workspace devDep must derive
#      a lock on which BOTH `npm ls --omit=dev` (runtime view) AND `npm ls` (full/dev view) exit
#      0. (Before the fix, --omit=dev exited 254 with the 'extraneous' crash.) The derived lock
#      must materialize the link TARGET node + its nested, version-conflicting closure.
#   2. RUNTIME INVARIANCE — the runtime (`--omit=dev`) closure must contain the member's real
#      runtime deps, NOT the dev-only workspace link, so the published SBOM's
#      runtime surface is unchanged by the presence of a workspace-link DEV edge.
#   3. ORDINARY NESTED CLOSURE — a runtime package whose transitive dependency conflicts with a
#      hoisted root version must retain its nested version in the derived member lock.
#
# HERMETIC: builds its own throwaway root package-lock.json under a mktemp dir; no repo content,
# no network (`--package-lock-only`). Requires `node` + `npm` (the ci-scripts runner ships both;
# the SBOM lane already depends on them). If either is absent we SKIP with a clear message rather
# than fail, so the harness never reds a contributor box that simply lacks the toolchain.
#
# Run:  bash scripts/tests/test_derive_workspace_member_lock.sh   (exit 0 = pass, 1 = a case failed)
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
#   * member "mbr"   — the "published" member (stands in for js/), runtime dep `fzstd`,
#                      dev deps `typescript` + a WORKSPACE LINK to sibling "pkgs/cli".
#   * "pkgs/cli"      — the link TARGET (stands in for packages/sparq-client), with its OWN
#                      dev dep `typescript` at a CONFLICTING version, pinned NESTED in the lock
#                      (so the materializer must preserve the nested key, not collide top-level).
# The derive script is invoked exactly as gen-js-sbom.sh invokes it: <member-dir> <out-lock>.
# NOTE: derive-workspace-member-lock.mjs resolves the root lock RELATIVE TO ITS OWN LOCATION
# (scripts/../package-lock.json), so the synthetic lock is written to the REPO ROOT under a
# unique temp name is NOT possible; instead we copy the script into the temp project so its
# `..` points at the synthetic root. This keeps the test fully hermetic.
# --------------------------------------------------------------------------- #
PROJ="${TMP}/proj"
mkdir -p "${PROJ}/scripts" "${PROJ}/mbr" "${PROJ}/pkgs/cli"
cp "$DERIVE" "${PROJ}/scripts/derive-workspace-member-lock.mjs"

cat > "${PROJ}/package-lock.json" <<'JSON'
{
  "name": "synthetic-monorepo",
  "lockfileVersion": 3,
  "requires": true,
  "packages": {
    "": { "name": "synthetic-monorepo", "workspaces": ["mbr", "pkgs/*"] },
    "mbr": {
      "name": "@synthetic/mbr",
      "version": "0.1.0",
      "license": "MIT",
      "dependencies": { "fzstd": "^0.1.1", "codec": "^1.0.0" },
      "devDependencies": { "typescript": "^6.0.0", "@synthetic/cli": "*" }
    },
    "pkgs/cli": {
      "name": "@synthetic/cli",
      "version": "0.2.0",
      "license": "MIT",
      "devDependencies": { "typescript": "^5.7.0" }
    },
    "node_modules/@synthetic/cli": { "resolved": "pkgs/cli", "link": true },
    "node_modules/fzstd": {
      "version": "0.1.1",
      "resolved": "https://registry.npmjs.org/fzstd/-/fzstd-0.1.1.tgz",
      "integrity": "sha512-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=="
    },
    "node_modules/codec": {
      "version": "1.0.0",
      "resolved": "https://registry.npmjs.org/codec/-/codec-1.0.0.tgz",
      "integrity": "sha512-DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD==",
      "dependencies": { "helper": "^1.0.0" }
    },
    "node_modules/helper": {
      "version": "2.0.0",
      "resolved": "https://registry.npmjs.org/helper/-/helper-2.0.0.tgz",
      "integrity": "sha512-EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE=="
    },
    "node_modules/codec/node_modules/helper": {
      "version": "1.2.0",
      "resolved": "https://registry.npmjs.org/helper/-/helper-1.2.0.tgz",
      "integrity": "sha512-FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF=="
    },
    "node_modules/typescript": {
      "version": "6.0.3",
      "resolved": "https://registry.npmjs.org/typescript/-/typescript-6.0.3.tgz",
      "integrity": "sha512-BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=="
    },
    "pkgs/cli/node_modules/typescript": {
      "version": "5.9.3",
      "resolved": "https://registry.npmjs.org/typescript/-/typescript-5.9.3.tgz",
      "integrity": "sha512-CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC=="
    }
  }
}
JSON

# Minimal member manifest so `npm ls` has a manifest co-located with the derived lock.
cat > "${PROJ}/mbr/package.json" <<'JSON'
{
  "name": "@synthetic/mbr",
  "version": "0.1.0",
  "dependencies": { "fzstd": "^0.1.1", "codec": "^1.0.0" },
  "devDependencies": { "typescript": "^6.0.0", "@synthetic/cli": "*" }
}
JSON

OUT_LOCK="${PROJ}/mbr/package-lock.json"
if ! node "${PROJ}/scripts/derive-workspace-member-lock.mjs" mbr "$OUT_LOCK" >/tmp/derive-link.log 2>&1; then
  echo "FAIL: derive script exited non-zero on a member with a workspace-link dev dep:"
  sed 's/^/    /' /tmp/derive-link.log
  exit 1
fi

# --------------------------------------------------------------------------- #
# Case 1 (TEETH): the link TARGET node + its nested closure must be materialized
# (not left as a dangling `link: true` stub).
# --------------------------------------------------------------------------- #
node - "$OUT_LOCK" <<'NODE'
const fs = require("fs");
const lock = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const p = lock.packages;
const fail = (m) => { console.error("FAIL: " + m); process.exit(1); };
if (!p["node_modules/@synthetic/cli"] || p["node_modules/@synthetic/cli"].link !== true) {
  fail("expected the workspace-link stub node_modules/@synthetic/cli (link:true) in the derived lock");
}
if (!p["pkgs/cli"] || p["pkgs/cli"].name !== "@synthetic/cli") {
  fail("workspace-link TARGET pkgs/cli was NOT materialized as a real package node (the sq-f04e regression)");
}
if (!p["pkgs/cli/node_modules/typescript"] || p["pkgs/cli/node_modules/typescript"].version !== "5.9.3") {
  fail("the link target's NESTED, version-conflicting dep (typescript@5.9.3) was not preserved under its nested key");
}
if (!p["node_modules/typescript"] || p["node_modules/typescript"].version !== "6.0.3") {
  fail("the member's own top-level typescript@6.0.3 must remain hoisted, distinct from the target's nested 5.9.3");
}
if (!p["node_modules/codec/node_modules/helper"] || p["node_modules/codec/node_modules/helper"].version !== "1.2.0") {
  fail("an ordinary runtime dependency's nested, version-conflicting helper was not preserved");
}
console.log("PASS: workspace-link target + nested closure materialized in the derived lock");
NODE

# --------------------------------------------------------------------------- #
# Case 2 (TEETH): `npm ls --omit=dev` (the RUNTIME view cyclonedx-npm runs) must exit 0 —
# this is the exact command that crashed with exit 254 before the fix.
# --------------------------------------------------------------------------- #
if ( cd "${PROJ}/mbr" && npm ls --json --long --all --package-lock-only --omit=dev --workspaces=false ) \
     >/tmp/npmls-omitdev.json 2>/tmp/npmls-omitdev.err; then
  echo "PASS: npm ls --omit=dev exits 0 on a derived lock with a workspace link (no 'extraneous' crash)"
else
  echo "FAIL: npm ls --omit=dev crashed on the derived lock (the sq-f04e regression — exit 254):"
  sed 's/^/    /' /tmp/npmls-omitdev.err
  exit 1
fi

# --------------------------------------------------------------------------- #
# Case 3 (TEETH): the FULL (dev) view npm ls must ALSO exit 0 — the materialized nested
# closure must not introduce an ELSPROBLEMS version-conflict.
# --------------------------------------------------------------------------- #
if ( cd "${PROJ}/mbr" && npm ls --json --long --all --package-lock-only --workspaces=false ) \
     >/tmp/npmls-full.json 2>/tmp/npmls-full.err; then
  echo "PASS: npm ls (full/dev view) exits 0 on the derived lock"
else
  echo "FAIL: npm ls (full/dev view) errored on the derived lock:"
  sed 's/^/    /' /tmp/npmls-full.err
  exit 1
fi

# --------------------------------------------------------------------------- #
# Case 4 (RUNTIME INVARIANCE): the runtime (`--omit=dev`) closure must contain the member's
# real runtime deps `fzstd`/`codec` and MUST NOT contain the dev-only workspace link or its dev-only
# `typescript` — i.e. a workspace-link DEV edge never leaks into the published runtime surface.
# --------------------------------------------------------------------------- #
node - /tmp/npmls-omitdev.json <<'NODE'
const fs = require("fs");
const tree = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const deps = Object.keys(tree.dependencies || {});
const fail = (m) => { console.error("FAIL: " + m + " (runtime deps were: " + JSON.stringify(deps) + ")"); process.exit(1); };
if (!deps.includes("fzstd")) fail("runtime (--omit=dev) tree is missing the real runtime dep fzstd");
if (!deps.includes("codec")) fail("runtime (--omit=dev) tree is missing the nested-closure fixture codec");
if (deps.includes("@synthetic/cli")) fail("the dev-only workspace LINK leaked into the runtime (--omit=dev) tree");
if (deps.includes("typescript")) fail("a dev-only dep (typescript) leaked into the runtime (--omit=dev) tree");
console.log("PASS: runtime (--omit=dev) closure contains the real runtime deps; no dev-link leak");
NODE

echo "All derive-workspace-member-lock workspace-link self-tests passed."
