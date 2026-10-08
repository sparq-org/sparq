#!/usr/bin/env bash
# Hermetic both-direction self-tests for scripts/check-privacy-claims.sh
# (bead sq-qhy4 — the ZK-soundness honesty gate).
#
# WHY: the gate's `sound(ness)?-(verifier|proof)` pattern caught adjacency
# ("sound verifier") but NOT predicate-form overclaims ("the verifier is SOUND",
# "…are COMPLETE and SOUND", "provably/fully sound") — a real overclaim sat in
# zk/compose/STATUS.md undetected until a manual sweep (PR #391). This harness pins
# BOTH directions so the predicate-form coverage can't silently regress:
#   - should-FAIL : positive predicate-form soundness overclaims about a ZK/MPC
#                   artifact MUST be caught (exit 1).
#   - should-PASS : negated / qualified / hedged / allow-marked / non-crypto "sound"
#                   usages MUST NOT regress (exit 0).
#
# HERMETIC w.r.t. git/network/repo content: each case is a single fixture LINE fed to
# the gate's matching logic in isolation. We do NOT run the whole-repo scan here (that
# is the CI step `bash scripts/check-privacy-claims.sh`, run separately); we extract
# and exercise the gate's PATTERNS / SOUNDNESS_CLAIMS / NEGATOR_HEDGE_RE classification
# by sourcing the script's variable block and replaying the exact per-line decision.
#
# Run:  bash scripts/tests/test_privacy_claims.sh   (exit 0 = all pass, 1 = a case failed)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="${ROOT}/scripts/check-privacy-claims.sh"

[ -f "$GATE" ] || { echo "FATAL: gate script not found at ${GATE}"; exit 2; }

# --------------------------------------------------------------------------- #
# Extract the gate's regex definitions WITHOUT running its scan. The script is
# `set -euo pipefail` and runs the scan at source-time, so we instead lift just the
# variable-definition block (everything up to the `mapfile` that begins the scan).
# This keeps the test reading the SAME patterns the gate ships — no duplication.
#
# sq-mraf: the gate now LOADS its forbidden-phrase list from the shared
# scripts/honesty-phrases.json (via the _phrases_field/_phrases_array helpers defined
# from `PHRASES_JSON=` onward), so we lift the block from that anchor — exercising the
# real JSON-load path, not a hand-copied list. ROOT is set above; the helpers read the
# real shared file, so a drift between the JSON and what the gate uses is caught here.
# --------------------------------------------------------------------------- #
DEFS="$(sed -n '/^PHRASES_JSON=/,/^mapfile -t FILES/p' "$GATE" | sed '/^mapfile -t FILES/d')"
# shellcheck disable=SC1090,SC2086
eval "$DEFS"

# Rebuild the same union + tiered regexes the gate builds.
RE="$(printf '%s|' "${PATTERNS[@]}")"; RE="${RE%|}"
SOUNDNESS_RE="$(printf '%s|' "${SOUNDNESS_CLAIMS[@]}")"; SOUNDNESS_RE="${SOUNDNESS_RE%|}"
UNION_RE="${RE}|${SOUNDNESS_RE}"

# Replay the gate's exact per-line verdict for one content line.
# Echoes "FAIL" if the line would be reported as a violation, "PASS" otherwise.
verdict() {
  local content="$1"
  if printf '%s\n' "$content" | grep -qiF "$ALLOW_MARKER"; then echo PASS; return; fi
  if printf '%s\n' "$content" | grep -qiE "$UNION_RE"; then
    # Matched something. If it is ONLY a guarded soundness claim, a negator/hedge exempts.
    if ! printf '%s\n' "$content" | grep -qiE "$RE"; then
      if printf '%s\n' "$content" | grep -qiE "$NEGATOR_HEDGE_RE"; then echo PASS; return; fi
    fi
    echo FAIL; return
  fi
  echo PASS
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
# Direction A — these MUST be newly CAUGHT (predicate-form positive overclaims).
# --------------------------------------------------------------------------- #
check FAIL "The verifier is sound."
check FAIL "The v1 verifier is SOUND."
check FAIL "Our proofs are sound and complete."
check FAIL "The proof is sound under the BN254 assumption."
check FAIL "The protocol is sound against a malicious prover."
check FAIL "The circuit is sound."
check FAIL "These query proofs are provably sound."
check FAIL "The verifier is fully sound for production use."
check FAIL "The construction is sound; relying parties may trust it."
# The PR-#391 shape — conjoined adjectives between the copula and "sound" MUST be caught.
check FAIL "The composed verifier and proofs are COMPLETE and SOUND."
check FAIL "The recursive proof is correct, complete and sound."
# Adjacency form still caught (no regression of the original pattern).
check FAIL "Ship the sound verifier today."

# --------------------------------------------------------------------------- #
# Direction B — these MUST still PASS (negated / qualified / hedged / non-crypto).
# --------------------------------------------------------------------------- #
check PASS "The verifier is NOT-yet-sound."
check PASS "The verifier is not sound."
check PASS "The proof is not sound pending external audit."
check PASS "The verifier isn't sound yet."
check PASS "These proofs are never sound without the binding layer."
check PASS "The verifier is unsound (see the audit)."
check PASS "There is no soundness guarantee for the ZK estate."
check PASS "Whether the verifier is sound is an open question."
check PASS "Soundness is PENDING external cryptographer sign-off."
# The project's canonical hedged verdict — the load-bearing one that must survive.
check PASS 'The v1 verifier is SOUND as landed for the threat model the prior audit assumed.'
check PASS "the internal re-audit argues it is sound as landed) but the claim is unverified"
# Scoped / engineering / non-crypto "is sound" — subject anchoring keeps these clear.
check PASS "These are sound engineering choices for a ZK system."
check PASS "the GX-5 register records why each unsafe block is sound, how it is tested"
check PASS "the cache invalidation is SOUND; when in doubt it recomputes"
check PASS "MPC mismatch detection is sound; sharpening is heuristic"
check PASS "is sound or production-safe is a separate and currently open question"
# Legitimately AXIS-SCOPED hedge ("sound for integrity") still passes; an UNSCOPED
# "sound for <anything>" overclaim does NOT get the free pass.
check PASS "the Tier-A controls are sound for integrity, independent of the privacy axis"
check FAIL "the scheme is sound for production relying parties"
# Allow-marker still exempts an otherwise-forbidden line.
check PASS "The verifier is sound. <!-- privacy-claims-allow: illustrating the gate -->"
check PASS "Documents the phrase 'privacy-preserving'. privacy-claims-allow: phrase list"

# --------------------------------------------------------------------------- #
echo ""
echo "test_privacy_claims: ${pass} passed, ${fail} failed."
[ "$fail" -eq 0 ] || exit 1
echo "test_privacy_claims: OK — both-direction soundness-claim coverage holds."
