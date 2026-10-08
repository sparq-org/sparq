#!/usr/bin/env bash
# Hermetic self-test for scripts/check-release-aliases.sh (sq-vw3ax.11.3 — the
# fail-closed release alias contract). PR #3528 review: a weak count FLOOR on the extracted
# alias sets could approve a PARTIALLY extracted contract — a reformat/refactor of
# download-client.tsx that made the regexes miss some (but not all) site-referenced aliases
# still passed, and a release could ship silent-404 download buttons. The gate now derives
# each alias class TWICE via structurally independent patterns and requires exact agreement;
# this harness pins that design in BOTH directions:
#
#   1. GOLDEN — the contract extracted from the REAL in-repo component equals the pinned
#      expected set below. On an INTENTIONAL site alias change, update the site and this
#      golden in the same PR (this is the synchronization point: the change is acknowledged
#      here at PR time, not discovered at release time).
#   2. CORRECTNESS — with every alias staged, the gate passes (exit 0); quote-style-only
#      reformats of the component neither fail nor shrink the contract.
#   3. TEETH — one staged alias missing => exit 1 (the release-blocking path).
#   4. PARTIAL-EXTRACTION MUTATIONS — each mutation that makes ONE derivation lose one or
#      more references (key/literal reformatted onto two lines, key renamed, CLI pair split,
#      pattern-map key drift, alias prefix rename) must exit 2 (extraction-broken), NEVER
#      approve a reduced contract.
#
# HERMETIC: mutations are sed-applied copies of the real component under mktemp; no network.
# Run:  bash scripts/tests/test_check_release_aliases.sh   (exit 0 = pass, 1 = a case failed)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="${ROOT}/scripts/check-release-aliases.sh"
CLIENT="${ROOT}/site/src/app/download/download-client.tsx"

[ -f "$GATE" ] || { echo "FATAL: gate script not found at ${GATE}"; exit 2; }
[ -f "$CLIENT" ] || { echo "FATAL: download client not found at ${CLIENT}"; exit 2; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $1"; [ -f "${2:-}" ] && sed 's/^/    /' "$2"; exit 1; }

# The full site-referenced alias contract (sorted). Update alongside an intentional
# /download alias change — Case 1 fails loudly here, at PR time, until you do.
GOLDEN="${TMP}/golden-assets.txt"
sort > "$GOLDEN" <<'EOF'
sparq-gui-arm64-darwin.dmg
sparq-gui-x64-darwin.dmg
sparq-gui-win-x64.msi
sparq-gui-x64-linux.AppImage
sparq-gui-x64-linux.deb
sparq-cli-arm64-darwin.tar.gz
sparq-cli-x64-darwin.tar.gz
sparq-cli-win-x64.zip
sparq-cli-x64-linux.tar.gz
EOF

# --------------------------------------------------------------------------- #
# Case 1 (GOLDEN + should-PASS): real component + fully staged list => exit 0,
# and the printed contract is EXACTLY the golden set (no reference lost, none invented).
# --------------------------------------------------------------------------- #
if ! bash "$GATE" "$GOLDEN" "$CLIENT" > "${TMP}/good.log" 2>&1; then
  fail "gate rejected the real component with every alias staged (false positive)" "${TMP}/good.log"
fi
sed -n 's/^  - //p' "${TMP}/good.log" | sort > "${TMP}/extracted.txt"
if ! diff -u "$GOLDEN" "${TMP}/extracted.txt" > "${TMP}/golden.diff" 2>&1; then
  fail "extracted contract drifted from the golden set — if the /download aliases changed
      intentionally, update the golden in $0; otherwise the extraction lost/invented a reference:" \
    "${TMP}/golden.diff"
fi
echo "PASS: real component extracts exactly the golden ${GOLDEN##*/} contract (exit 0)"

# --------------------------------------------------------------------------- #
# Case 2 (TEETH, should-FAIL exit 1): one staged alias missing blocks the release.
# The dropped alias is a SECONDARY (`secondary.file`) — pins that secondaries are covered.
# --------------------------------------------------------------------------- #
grep -v '^sparq-gui-x64-linux\.deb$' "$GOLDEN" > "${TMP}/short-assets.txt"
rc=0; bash "$GATE" "${TMP}/short-assets.txt" "$CLIENT" > "${TMP}/teeth.log" 2>&1 || rc=$?
if [ "$rc" -ne 1 ]; then
  fail "missing staged alias exited $rc, want 1 (release-blocking teeth)" "${TMP}/teeth.log"
fi
echo "PASS: a missing staged alias (a secondary) blocks the release (exit 1)"

# --------------------------------------------------------------------------- #
# Partial-extraction mutations: each must exit 2 (extraction-broken, fail-closed),
# NOT exit 0 with a silently reduced contract. mutate <name> <sed-script> applies the
# mutation to a copy of the real component and asserts it actually changed the file
# (a no-op mutation would make the case vacuous).
# --------------------------------------------------------------------------- #
mutate() { # mutate <name> <sed-script>
  local out="${TMP}/$1.tsx"
  sed -E "$2" "$CLIENT" > "$out"
  if cmp -s "$CLIENT" "$out"; then
    echo "FAIL: mutation $1 did not change the component (vacuous case — update the sed)"
    exit 1
  fi
  printf '%s' "$out"
}

expect_extraction_broken() { # expect_extraction_broken <name> <mutated-path> <what>
  local rc=0
  bash "$GATE" "$GOLDEN" "$2" > "${TMP}/$1.log" 2>&1 || rc=$?
  if [ "$rc" -ne 2 ]; then
    fail "mutation $1 ($3) exited $rc, want 2 — a partial extraction was approved" "${TMP}/$1.log"
  fi
  echo "PASS: $3 is caught as extraction breakage (exit 2)"
}

# M1: one GUI `file:` key reformatted onto two lines — derivation (a) loses one alias while
# 4 remain (the exact shape the old count floor waved through).
m="$(mutate m1 's/file: "sparq-gui-arm64-darwin.dmg",/file:\n      "sparq-gui-arm64-darwin.dmg",/')"
expect_extraction_broken m1 "$m" "GUI key+literal split across lines"

# M2: one GUI `file:` key renamed — the alias literal survives under another key.
m="$(mutate m2 's/file: "sparq-gui-win-x64.msi",/installer: "sparq-gui-win-x64.msi",/')"
expect_extraction_broken m2 "$m" "GUI 'file:' key renamed"

# M3: one CLI `token/ext` pair split across lines — pair derivation loses one token.
m="$(mutate m3 's/token: "win-x64", ext: "zip",/token: "win-x64",\n        ext: "zip",/')"
expect_extraction_broken m3 "$m" "CLI token/ext pair split across lines"

# M4: CLI_PRERELEASE_PATTERNS key drift — the map's token set no longer matches cliTarget's.
m="$(mutate m4 's|"win-x64":( +)/\^sparq-cli|"win64":\1/^sparq-cli|')"
expect_extraction_broken m4 "$m" "CLI_PRERELEASE_PATTERNS key drift"

# M5: alias prefix rename — every `file:` key still extracts, but the alias-shape
# derivation goes empty; a wholesale rename must force a conscious script update.
m="$(mutate m5 's/sparq-gui-/sparq-desktop-/g')"
expect_extraction_broken m5 "$m" "GUI alias prefix rename"

# --------------------------------------------------------------------------- #
# Case 3 (robustness, should-PASS): a quote-style-only reformat (double -> single quotes on
# one literal) must neither fail nor shrink the contract.
# --------------------------------------------------------------------------- #
m="$(mutate quotes "s/file: \"sparq-gui-arm64-darwin.dmg\",/file: 'sparq-gui-arm64-darwin.dmg',/")"
if ! bash "$GATE" "$GOLDEN" "$m" > "${TMP}/quotes.log" 2>&1; then
  fail "quote-style reformat was rejected (extraction not quote-agnostic)" "${TMP}/quotes.log"
fi
sed -n 's/^  - //p' "${TMP}/quotes.log" | sort > "${TMP}/quotes-extracted.txt"
if ! cmp -s "$GOLDEN" "${TMP}/quotes-extracted.txt"; then
  fail "quote-style reformat shrank the extracted contract" "${TMP}/quotes.log"
fi
echo "PASS: quote-style reformat keeps the full contract (exit 0)"

echo "All check-release-aliases self-tests passed."
