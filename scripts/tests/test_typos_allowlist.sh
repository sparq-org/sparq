#!/usr/bin/env bash
# Hermetic both-direction self-tests for the _typos.toml allow-list
# (bead sq-hyzq, tightened in sq-uiie — CI hygiene: stop recurring SPARQL-keyword false-
# positives blocking the docs-quality `typos` gate).
#
# WHY: typos (crate-ci/typos) splits an ALL-CAPS SPARQL keyword written with a lowercase
# English suffix at the case boundary and flags the keyword-minus-tail or the dangling
# 3-char fragment — DELETEd->DELET, INSERTed->INSER, "3 OPTIONALs"->OPTIONA, LOADed->Ded.
# These are not typos; they are how anyone naturally writes SPARQL verbs in prose, so they
# kept blocking the HARD gate and forcing pointless reword round-trips on doc PRs. The fix
# is an `extend-ignore-re` regex anchored to a CLOSED alternation of known SPARQL/SQL/RDF-
# update keyword roots (SELECT|INSERT|DELETE|…) each followed by a 1-3 char lowercase tail,
# plus an `invokable` word entry. (sq-uiie: the original sq-hyzq form `\b[A-Z]{3,}[a-z]{1,3}\b`
# was over-broad — it shielded ANY all-caps run with a short lowercase tail, silently hiding
# genuine misspelled roots whose suffixed form typos DOES flag: SEPERATEd/MULTIPLYed/OCURRed
# (typos splits at the case boundary -> SEPERAT/MULTIPL/OCUR -> SEPARATE/MULTIPLE/OCCUR). The
# keyword anchor closes that hole. sq-k041: the glued mixed-case forms RECIEVEd/ADRESs are NOT
# such a case — typos never flags those, regex or no; only the bare all-caps RECIEVE/ADRES do.)
# This
# harness pins BOTH directions so the allow-list can't silently over- or under-reach later:
#   - should-PASS (SUPPRESSED): the keyword-suffix family + `invokable` must NOT be flagged.
#   - should-FAIL (STILL FIRES): a genuine ALL-CAPS misspelling (bare OR with a lowercase
#                                suffix) and ordinary lowercase typos MUST still be caught,
#                                so the gate keeps its teeth.
#
# HERMETIC w.r.t. repo content/network: every case is a one-line fixture written to a
# temp file and run through the SHIPPED `typos --config _typos.toml` binary in isolation —
# no repo scan, no network. It exercises the exact config the gate ships (no duplication).
#
# Run:  bash scripts/tests/test_typos_allowlist.sh   (exit 0 = all pass, 1 = a case failed)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CONFIG="${ROOT}/_typos.toml"

[ -f "$CONFIG" ] || { echo "FATAL: _typos.toml not found at ${CONFIG}"; exit 2; }
command -v typos >/dev/null 2>&1 || { echo "FATAL: 'typos' binary not on PATH"; exit 2; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
FIX="${TMP}/fixture.md"

# Echo "FAIL" if typos would report >=1 misspelling on this line, "PASS" otherwise.
# Uses the SHIPPED config; `--isolated` ignores any ambient/parent _typos.toml so only the
# project config under test applies.
verdict() {
  printf '%s\n' "$1" > "$FIX"
  if typos --isolated --config "$CONFIG" "$FIX" >/dev/null 2>&1; then
    echo PASS   # exit 0 => no typo found => suppressed/clean
  else
    echo FAIL   # non-zero => typo reported
  fi
}

pass=0
fail=0
check() {  # check <expected FAIL|PASS> <line>
  local want="$1" line="$2" got
  got="$(verdict "$line")"
  if [ "$got" = "$want" ]; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    printf 'CASE FAILED: wanted=%-4s got=%-4s  | %s\n' "$want" "$got" "$line"
  fi
}

# --------------------------------------------------------------------------- #
# Direction A — these MUST be SUPPRESSED (recurring false-positives the bead names).
# The whole "ALL-CAPS SPARQL keyword + lowercase suffix" family, plus `invokable`.
# --------------------------------------------------------------------------- #
check PASS "The triples were DELETEd from the default graph."
check PASS "We INSERTed three quads and then SELECTed the bindings."
check PASS "The query CONSTRUCTed a graph and DESCRIBEd the resource."
check PASS "The named graph was CLEARed and the dump was LOADed."
check PASS "We UNIONed two patterns and MATCHed the BGP."
check PASS "Product detail with 3 OPTIONALs and one nested OPTIONALs."
check PASS "This helper is invokable from the CLI."

# --------------------------------------------------------------------------- #
# Direction B — these MUST still FIRE (the gate keeps catching real misspellings).
# --------------------------------------------------------------------------- #
# A genuine ALL-CAPS misspelling has no lowercase suffix, so the regex does NOT shield it.
check FAIL "The header says RECIEVE instead of RECEIVE."
# Ordinary lowercase typos are untouched by the keyword regex.
check FAIL "Fix teh wording in the paragraph."
check FAIL "Re-order hte two clauses."
# A bare keyword with a real lowercase typo glued on a DIFFERENT word must still fire:
# `seperate` is lowercase, unrelated to the regex, and stays caught.
check FAIL "Keep the two stores seperate."
# --------------------------------------------------------------------------- #
# Direction B (regression — bead sq-uiie). The sq-hyzq form `\b[A-Z]{3,}[a-z]{1,3}\b`
# was over-broad: it matched ANY >=3-capital run plus a 1-3 char lowercase tail, so a
# genuine ALL-CAPS misspelling written WITH such a suffix was silently suppressed. Anchoring
# the prefix to a closed keyword list closes that hole — a misspelled root is not in the
# list, so its suffixed form fires again. These MUST FIRE (would have been WRONGLY PASSed by
# the old blanket regex):
check FAIL "The bytes were SEPERATEd into two buffers."  # SEPERATE is not a keyword
check FAIL "Each value was MULTIPLYed by two."            # MULTIPLY+ed, not a SPARQL verb
check FAIL "The fault OCURRed under load."                # OCURR(ed) is not a keyword

# --------------------------------------------------------------------------- #
# Pin (bead sq-k041): the glued mixed-case forms RECIEVEd/ADRESs were cited by the sq-uiie
# comment as misspellings the new anchor "recovers" — but typos NEVER flags those glued forms
# (case-boundary split leaves a mixed-case token it won't correct), regex or no. They are NOT
# the recovered class. These MUST PASS unconditionally — independent of the keyword anchor —
# so the comment can't drift back to citing them. (The bare all-caps RECIEVE still FAILs above
# at line ~81, which is the form typos actually catches.)
check PASS "The handler RECIEVEd the message."           # glued mixed-case — typos-invisible
check PASS "The ADRESs field was left blank."            # glued mixed-case — typos-invisible

# --------------------------------------------------------------------------- #
echo ""
echo "test_typos_allowlist: ${pass} passed, ${fail} failed."
[ "$fail" -eq 0 ] || exit 1
echo "test_typos_allowlist: OK — keyword-suffix false-positives suppressed, real typos still caught."
