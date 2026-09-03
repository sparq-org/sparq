#!/usr/bin/env bash
# [OPUS-4.8] sq-toze.35 (gate) + sq-qhy4 (the ZK-soundness external-audit gate).
#
# PRIVACY-CLAIM HONESTY GATE — the empirical-honesty mandate (MEMORY: feedback-
# empirical-honesty) in CI form.
#
# WHY: the v1 ZK query-proof verifier is NOT yet externally audited and `sparq-mpc`
# is honest-majority semi-honest only — see SECURITY.md and research/zk-*-audit.md.
# The documented, load-bearing verdict is that the ZK/MPC estate provides NO
# production privacy/soundness guarantee to a relying party pending an external
# cryptographer's sign-off (bead sq-qhy4). So NO doc / README / SKILL / site copy /
# marketing string may state a ZK/MPC privacy-or-soundness property as a *settled,
# achieved* fact without the not-yet-sound caveat. A hedged / aspirational / "model"
# / negative ("NOT sound") mention is fine; an unqualified claim of an achieved
# property is a high-severity honesty defect — this script fails the build on one.
#
# WHAT: greps a fixed user-facing surface (root docs, skills/, site copy, the
# compliance index) for a small set of FORBIDDEN unqualified-claim phrases. A hit is
# a failure UNLESS the offending line carries the inline allow-list marker
#   privacy-claims-allow: <justification>
# which records *why* that exact line is a legitimate hedged/negative/illustrative
# usage (the empirical-honesty audit trail). Whole files that are by-nature
# discussion of the (un)soundness — the research/ design records and the audit docs
# themselves — are path-excluded (they MUST be free to name the properties to argue
# about them); the honesty surface this gate defends is the *outward* claim surface.
#
# This is intentionally a coarse phrase gate, not an NLP classifier: it is tuned so
# the repo passes CLEAN today (after the sq-toze.35 fixes). A future unqualified
# claim trips it; the author then either (a) hedges the wording, or (b) — if the line
# really is a legitimate hedged/negative usage the regex over-matched — appends the
# `privacy-claims-allow:` marker with a one-line justification. Weakening the regex
# or blanket-excluding a live doc surface to "make it pass" defeats the gate and is
# itself an honesty defect.
#
# Run locally:  scripts/check-privacy-claims.sh   (exit 0 = clean, 1 = unqualified claim)
set -euo pipefail

# Repo root (this script lives in scripts/).
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# [OPUS-4.8] sq-mraf (Option C) — the FORBIDDEN-PHRASE patterns are loaded from the SINGLE
# shared source of truth `scripts/honesty-phrases.json` so this gate and the build-boundary
# assertion in site/scripts/build-papers.mjs cannot drift. The JSON is the canonical list;
# the per-line classification logic (which tier exempts how) stays HERE, unchanged.
#
# Inline allow-list marker: a scanned line containing this token is exempt. The text
# AFTER the marker is the required human justification (audit trail). [OPUS-4.8]
PHRASES_JSON="${ROOT}/scripts/honesty-phrases.json"
[ -f "$PHRASES_JSON" ] || { echo "::error::shared honesty-phrase list not found at ${PHRASES_JSON} (sq-mraf)"; exit 2; }

# Read a top-level string field from the shared JSON. python3 is already a hard CI dep
# (scripts/check-no-perf-numbers.py); using it (not jq) keeps the toolchain identical.
_phrases_field() {  # _phrases_field <jsonpath-key>
  python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$PHRASES_JSON" "$1"
}
# Read a top-level array field as NUL-delimited entries (so a pattern may contain any char).
_phrases_array() {  # _phrases_array <jsonpath-key> -> writes \0-joined items to stdout
  python3 -c 'import json,sys
items = json.load(open(sys.argv[1]))[sys.argv[2]]
sys.stdout.write("".join(item + "\0" for item in items))' "$PHRASES_JSON" "$1"
}

ALLOW_MARKER="$(_phrases_field allowMarker)"

# FORBIDDEN phrases — settled-property claim shapes for the ZK/MPC estate. Case-
# insensitive, extended-regex. Each is a phrasing that, UNHEDGED, asserts a privacy
# or soundness guarantee the estate does not yet provide.
#
# Two tiers (different exemption rules):
#   PATTERNS         — ABSOLUTE forbidden phrases. A hit fails UNLESS the line carries
#                      the `privacy-claims-allow:` marker. These shapes have no honest
#                      unhedged reading (e.g. "provably secure", "privacy-preserving"),
#                      so an inline-marker justification is the only escape.
#   SOUNDNESS_CLAIMS — PREDICATE-FORM positive soundness overclaims about a ZK/MPC
#                      *artifact* (the verifier / proof / protocol / scheme / circuit /
#                      prover / SNARK / system / estate / construction is/are SOUND, or
#                      "provably/fully/… sound"). [OPUS-4.8] sq-qhy4: the prior
#                      `sound(ness)?-(verifier|proof)` adjacency pattern missed
#                      predicate-form overclaims ("the verifier is SOUND",
#                      "…are COMPLETE and SOUND") — one sat in zk/compose/STATUS.md
#                      undetected until a manual sweep (PR #391). These are subject-
#                      ANCHORED (a ZK/MPC artifact noun) so they don't trip honest
#                      non-crypto "is sound" uses (an unsafe-code invariant, a cache
#                      invalidation, an MPC detection heuristic, "sound engineering
#                      choices"). A hit is EXEMPT if the line carries the allow-marker
#                      OR a same-line negator/hedge (NEGATOR_HEDGE_RE) — so honest
#                      "NOT sound", "is not sound", "unsound", and the project's
#                      canonical hedged verdict "SOUND as landed for the … threat
#                      model" all still PASS.
# Loaded from the shared list (`absoluteForbiddenPatterns`) — see sq-mraf note above.
# macOS still ships Bash 3.2, which has no `mapfile`. A trailing NUL from
# `_phrases_array` ensures the final pattern is not dropped; `read -d ''` preserves
# every other character in each regex.
PATTERNS=()
while IFS= read -r -d '' item; do
  PATTERNS+=("$item")
done < <(_phrases_array absoluteForbiddenPatterns)

# [OPUS-4.8] Predicate-form soundness overclaim, subject-anchored on a ZK/MPC artifact
# noun + copula + "sound" (e.g. "the verifier is sound", "the proofs are sound", "the
# protocol is sound"). The anchoring keeps it OFF honest non-crypto "is sound" lines.
# Both parts come from the shared list so build-papers.mjs reuses the identical regex.
ZK_ARTIFACT="$(_phrases_field soundnessArtifact)"
# Copula clause: the artifact noun, then is/are/'s/'re, then "sound" — allowing up to two
# intervening conjoined adjectives ("complete and sound", "correct, complete and sound")
# so the PR-#391 shape ("…are COMPLETE and SOUND") is caught, not just bare "is sound".
COPULA="$(_phrases_field soundnessCopula)"
SOUNDNESS_CLAIMS=(
  "${ZK_ARTIFACT}s?${COPULA}"
)

# Same-line NEGATOR / HEDGE exemption — applies ONLY to a SOUNDNESS_CLAIMS hit (not to
# the absolute PATTERNS). If any of these is present on the offending line, the soundness
# mention is honest (negated / qualified / aspirational) and PASSES:
#   - negators:  not / never / no / isn't / aren't / n't / un(sound) / yet-(not-) ...
#   - the project's load-bearing hedge: "sound AS LANDED [for the … threat model]" and
#     scoped "sound FOR <axis>" (e.g. "sound for integrity"), plus an explicit open/
#     pending/unverified/whether/question uncertainty qualifier on the same line.
# Deliberately NARROW (structural negators + the canonical hedges + genuine-uncertainty
# words) — NO broad lexical escape hatch like a bare "if"/"claim", so an overclaim can't
# slip past on an incidental word; verified that no honest in-repo line relies on those.
# An over-matched legitimate line still has the explicit `privacy-claims-allow:` escape.
# Case-insensitive (matched with grep -i below). Loaded from the shared list.
NEGATOR_HEDGE_RE="$(_phrases_field negatorHedge)"

# Path surface to scan = the OUTWARD claim surface (docs/READMEs/skills/site copy /
# the compliance index / the published-paper + /specs Typst sources). Excludes:
#   - research/ + the audit docs : design records that must name the properties to
#     argue (un)soundness — the honesty source of truth, not a claim surface.
#   - this script + its CI wiring : they QUOTE the forbidden phrases by definition.
#   - the privacy/cryptoreview compliance bodies : they exist to discuss the gap and
#     are line-marked where they legitimately name a phrase.
#   - .git/target/node_modules/vendor : not authored claim surface.
#
# [OPUS-4.8] bead sq-mkza: the paper-factory `.typ` sources (`site/papers/*.typ` +
# `site/papers/**/*.typ`, i.e. the pilot papers AND `_lib/*.typ`) are added — a `.typ`
# paper IS a published outward claim, so an unqualified ZK/MPC soundness/privacy claim
# typed into paper prose must trip the SAME absolute patterns as any README/site copy.
# The exact inline-allow marker (`privacy-claims-allow: <why>`) and the same-line
# negator/hedge exemption apply UNCHANGED (the per-line classification below is reused
# verbatim). Typst `//`/`/* */` comments are NOT stripped here — a forbidden phrase in
# a paper comment is still authored text, matching this gate's coarse-phrase posture.
# This catches the COARSE unqualified-claim class; a subtle semantic overclaim phrased
# around the patterns remains Stage-5 human review.
#
# [OPUS-4.8] bead sq-rvgr2.5: the spec-factory `.typ` sources (`site/specs/*.typ` +
# `site/specs/**/*.typ`, i.e. every Proposed-Specification draft AND `_lib/*.typ`) are added
# for the SAME reason — a published /specs draft is an outward claim surface, and the ZK/MPC
# spec CONTENT (zkSPARQL / MPC-SPARQL, beads sq-rvgr2.2/.3) will author real cryptographic
# prose. Extending this surface BEFORE that content lands closes the gap the sq-rvgr2.1 infra
# left open (the claim-free template is unaffected). The inline-allow marker + the negator/
# hedge exemption apply UNCHANGED; the per-line classification below is reused verbatim, so a
# spec draft may hedge, negate, or `privacy-claims-allow:`-mark a line exactly like a paper.
#
# [OPUS-4.8] bead sq-4hga: the paper-factory EVIDENCE file
# `site/src/data/paper-evidence.json` is also added — its human `note` / free-text fields
# are published-paper provenance prose (they surface inline via the `provenance()` helper),
# so an unqualified ZK/MPC claim hidden in a record `note` must trip the SAME patterns. The
# whole JSON is scanned line-by-line (the coarse posture); a JSON-string `note` may carry
# the inline `privacy-claims-allow:` marker just like any other line. The accessor-driven
# numeric values are NOT prose and are unaffected. (The perf-number half of the
# evidence-prose scan lives in scripts/check-no-perf-numbers.py, its natural home.)
FILES=()
while IFS= read -r item; do
  FILES+=("$item")
done < <(
  git ls-files \
    '*.md' '*.mdx' '*.tsx' '*.ts' \
    'site/papers/*.typ' 'site/papers/**/*.typ' \
    'site/specs/*.typ' 'site/specs/**/*.typ' \
    'site/src/data/paper-evidence.json' \
    'site/src/data/paper-evidence.canonical-timing.generated.json' \
    ':!:research/**' \
    ':!:**/*audit*.md' \
    ':!:.claude/agents/**' \
    ':!:scripts/check-privacy-claims.sh' \
    ':!:.github/workflows/docs-quality.yml' \
    ':!:.git/**' ':!:target/**' ':!:**/target/**' \
    ':!:**/node_modules/**' ':!:vendor/**' ':!:**/vendor/**' \
    | sort -u
)

# Build the two alternation regexes.
#   RE          — absolute forbidden phrases (allow-marker is the only exemption).
#   SOUNDNESS_RE — predicate-form soundness overclaims (allow-marker OR same-line
#                  negator/hedge exempts).
RE="$(printf '%s|' "${PATTERNS[@]}")"
RE="${RE%|}"
SOUNDNESS_RE="$(printf '%s|' "${SOUNDNESS_CLAIMS[@]}")"
SOUNDNESS_RE="${SOUNDNESS_RE%|}"
# A single union for the per-line scan; the per-line classification below decides
# whether a given hit was an absolute one or a guarded soundness one.
UNION_RE="${RE}|${SOUNDNESS_RE}"

fail=0
hits=0
for f in "${FILES[@]}"; do
  [ -f "$f" ] || continue
  # -n line numbers, -i case-insensitive, -E extended.
  while IFS= read -r line; do
    # `line` is "LINENO:CONTENT".
    content="${line#*:}"   # strip the "LINENO:" prefix for content-only checks.
    if printf '%s\n' "$line" | grep -qiF "$ALLOW_MARKER"; then
      continue   # explicitly allow-listed with an inline justification.
    fi
    # If the ONLY thing this line matched was a guarded soundness claim (it does NOT
    # match an absolute PATTERN), a same-line negator/hedge exempts it as honest.
    if ! printf '%s\n' "$content" | grep -qiE "$RE"; then
      if printf '%s\n' "$content" | grep -qiE "$NEGATOR_HEDGE_RE"; then
        continue   # negated / qualified / aspirational soundness mention — honest.
      fi
    fi
    hits=$((hits + 1))
    if [ "$fail" -eq 0 ]; then
      echo "::error::unqualified ZK/MPC privacy/soundness claim(s) found — hedge the wording or add an inline '${ALLOW_MARKER}: <why>' marker (see SECURITY.md; sq-qhy4 / sq-toze.35)."
      echo "Forbidden claim phrases (must carry the not-yet-sound caveat):"
    fi
    echo "  ${f}:${line}"
    fail=1
  done < <(grep -niE "$UNION_RE" "$f" 2>/dev/null || true)
done

if [ "$fail" -ne 0 ]; then
  echo ""
  echo "check-privacy-claims: FAILED — ${hits} unqualified claim line(s) above."
  echo "The v1 ZK verifier is pending external audit (sq-qhy4); sparq-mpc is semi-honest only."
  echo "Either add the not-yet-sound caveat to the wording, or — if the line is a legitimate"
  echo "hedged/negative/illustrative usage the regex over-matched — append on that line:"
  echo "    ${ALLOW_MARKER}: <one-line justification>"
  exit 1
fi

echo "check-privacy-claims: OK — scanned ${#FILES[@]} file(s); no unqualified ZK/MPC privacy/soundness claim."
