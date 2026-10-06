#!/usr/bin/env bash
# Hermetic end-to-end tests for scripts/reconcile-merged-beads.sh (bead
# sq-13uyp — the recurrent sweep that closes bd-OPEN beads whose fix already MERGED, so
# they stop wasting frontier dispatches).
#
# WHY: the script's value rests on a few load-bearing, dangerous-if-wrong invariants that a
# future refactor must not silently break. The in-script self-test covers the PURE
# predicates (boundary matching + gated-kind + umbrella). THIS harness covers the
# END-TO-END behaviour under STUBBED bd/gh/git, where the actual close happens — i.e. that
# the right beads (and ONLY the right beads) are flagged/closed against a realistic merged
# blob. The four invariants the bead requires, pinned:
#   1. A bead whose EXACT id is in a MERGED PR (title or head-branch) — OR an origin/main
#      commit subject — is FLAGGED as a close-candidate.
#   2. An OPEN-PR-ONLY bead (its PR is open, not merged) and an UNRELATED bead are NOT
#      flagged (the merged blob contains only merged signals — open PRs never enter it).
#   3. The dotted-id FALSE-MATCH is avoided: `sq-ixc3.1` must NOT be flagged by a merged
#      `sq-ixc3.11`, and a base id must NOT be flagged by a `.N` molecule.
#   4. Epics / umbrella-parents (>=1 dependent) / needs:user beads are NEVER auto-closed —
#      they are reported for manual review even when their id IS in a merged PR.
# Plus: --apply actually closes the (non-gated) candidates and is IDEMPOTENT; a bd-show
# lookup error SKIPS that bead (fail-safe, never mass-closes).
#
# HERMETIC: `bd`, `gh`, and `git` are PATH-shadowed by stubs backed by tiny fixture files.
# The harness never touches the real bead tracker, never calls the network, and writes only
# inside a mktemp sandbox it removes on exit.
#
# Run:  bash scripts/tests/test_reconcile_merged_beads.sh   (exit 0 = all pass, 1 = a failure)
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="${ROOT}/scripts/reconcile-merged-beads.sh"
[ -f "$SRC" ] || { echo "FATAL: script not found at ${SRC}"; exit 2; }

pass=0; fail=0
note_pass() { pass=$((pass + 1)); }
note_fail() { fail=$((fail + 1)); printf 'CASE FAILED: %s\n' "$1"; }
want_contains()  { if printf '%s' "$2" | grep -qF -- "$3"; then note_pass; else note_fail "$1 (expected to CONTAIN '$3')"; fi; }
want_absent()    { if printf '%s' "$2" | grep -qF -- "$3"; then note_fail "$1 (expected NOT to contain '$3')"; else note_pass; fi; }

# --------------------------------------------------------------------------- #
# Sandbox with stub bd / gh / git on PATH.
# --------------------------------------------------------------------------- #
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT
BIN="${SANDBOX}/bin"
mkdir -p "$BIN"

# --- stub `gh`: serves a fixed MERGED-PR list (number, headRefName, title). The script
#     calls: gh pr list --state merged --limit N --json number,headRefName,title -q '...'.
#     We ignore flags and emit the same per-line shape the -q template produces:
#       "#<number> <headRefName> <title>"
#     Crucially this contains ONLY MERGED PRs — an open PR is NOT here, modelling reality.
cat >"${BIN}/gh" <<'EOF'
#!/usr/bin/env bash
# Only `gh pr list --state merged ...` is exercised. Emit fixture lines.
printf '%s\n' \
  "#101 fix/sq-merged1-thing fix(core): the merged fix (sq-merged1)" \
  "#102 feat/sq-epicmerge-x EPIC: paper factory umbrella merged child (sq-epicmerge)" \
  "#103 feat/sq-parent9-roll feat: parent with children, a child PR landed (sq-parent9)" \
  "#104 feat/sq-needs1-ui needs-user surface shipped (sq-needs1)" \
  "#105 feat/sq-ixc3.11-big feat(gui): the .11 molecule (sq-ixc3.11)" \
  "#106 fix/sq-brokey-fail bd-show-will-fail bead landed (sq-brokey)" \
  "#107 fix/sq-onbranch-only branch-only match, no id in title here"
exit 0
EOF
chmod +x "${BIN}/gh"

# --- stub `git`: serves origin/main commit subjects. The script calls:
#       git -C <root> log origin/main --no-merges --format='commit:%h %s' -n N
#     We emit "commit:<sha> <subject>" lines (one carries sq-commitonly to prove the
#     commit-subject signal flags a bead with NO PR). Any other git invocation is a no-op 0.
cat >"${BIN}/git" <<'EOF'
#!/usr/bin/env bash
for a in "$@"; do
  if [ "$a" = "log" ]; then
    printf '%s\n' \
      "commit:abc1234 fix(nlq): commit-only landed fix (sq-commitonly) (#999)" \
      "commit:def5678 chore: unrelated commit no bead token"
    exit 0
  fi
done
exit 0
EOF
chmod +x "${BIN}/git"

# --- stub `bd`: backed by a fixture file mapping id -> "status|issue_type|title|labels|dependent_count".
#     Implements the three subcommands the script uses:
#       bd list --status open       --json --limit 0
#       bd list --status in_progress --json --limit 0
#       bd show <id> --json
#       bd close <id> --reason <r>
#     Closes are recorded by appending the id to ${BD_CLOSED_LOG}. A bead whose status is the
#     sentinel "SHOWFAIL" makes `bd show` exit 1 (to exercise the fail-safe skip path).
BD_FIXTURE="${SANDBOX}/beads.tsv"
BD_CLOSED_LOG="${SANDBOX}/closed.log"
: >"$BD_CLOSED_LOG"
# id  status  issue_type  title  labels(csv)  dependent_count
cat >"$BD_FIXTURE" <<'EOF'
sq-merged1	open	bug	fix(core): the merged fix		0
sq-epicmerge	open	task	EPIC: paper factory umbrella		0
sq-parent9	open	feature	feat: parent with children		23
sq-needs1	open	task	needs-user surface	area:site,needs:user	0
sq-ixc3.1	open	feature	feat(gui): the .1 molecule		0
sq-commitonly	open	task	commit-only landed fix		0
sq-openonly	open	bug	bead whose PR is still OPEN		0
sq-unrelated	open	bug	totally unrelated open bead		0
sq-brokey	SHOWFAIL	bug	bd show will fail for this one		0
EOF

cat >"${BIN}/bd" <<EOF
#!/usr/bin/env bash
FIX="${BD_FIXTURE}"
CLOSED_LOG="${BD_CLOSED_LOG}"
EOF
cat >>"${BIN}/bd" <<'EOF'
sub="${1:-}"; shift || true
emit_json_for_status() {
  local want="$1" first=1
  echo -n "["
  while IFS=$'\t' read -r id status itype title labels deps; do
    [ -n "$id" ] || continue
    # SHOWFAIL beads still appear in `bd list` as open (so the script tries to show them).
    local liststatus="$status"
    [ "$status" = "SHOWFAIL" ] && liststatus="open"
    [ "$liststatus" = "$want" ] || continue
    [ "$first" = 1 ] || echo -n ","
    first=0
    printf '{"id":"%s","status":"%s","issue_type":"%s","title":"%s"}' "$id" "$liststatus" "$itype" "$title"
  done <"$FIX"
  echo "]"
}
case "$sub" in
  list)
    status="open"
    for ((i=1;i<=$#;i++)); do
      if [ "${!i}" = "--status" ]; then j=$((i+1)); status="${!j}"; fi
    done
    emit_json_for_status "$status"
    ;;
  show)
    id="$1"
    # Extract fields with awk (NOT `read -r`): bash IFS=$'\t' read COLLAPSES consecutive
    # tabs because tab is an IFS-whitespace char, so an empty middle field (e.g. no labels)
    # would shift the columns and mis-assign dependent_count. awk -F'\t' preserves empties.
    line="$(awk -F'\t' -v id="$id" '$1==id{print; exit}' "$FIX")"
    [ -n "$line" ] || { echo "[]"; exit 0; }
    fstatus="$(printf '%s' "$line" | cut -f2)"
    fitype="$(printf '%s' "$line" | cut -f3)"
    ftitle="$(printf '%s' "$line" | cut -f4)"
    flabels="$(printf '%s' "$line" | cut -f5)"
    fdeps="$(printf '%s' "$line" | cut -f6)"
    if [ "$fstatus" = "SHOWFAIL" ]; then exit 1; fi   # exercise the fail-safe skip path
    # labels CSV -> JSON array
    labjson="[]"
    if [ -n "$flabels" ]; then
      labjson="$(printf '%s' "$flabels" | awk -F, '{out="["; for(i=1;i<=NF;i++){ if(i>1)out=out","; out=out "\"" $i "\"" } out=out "]"; print out}')"
    fi
    printf '[{"id":"%s","status":"%s","issue_type":"%s","title":"%s","labels":%s,"dependent_count":%s}]\n' \
      "$id" "$fstatus" "$fitype" "$ftitle" "$labjson" "${fdeps:-0}"
    ;;
  close)
    id="$1"
    echo "$id" >>"$CLOSED_LOG"
    # Flip the bead to closed in the fixture so a re-run sees it closed (idempotency test).
    tmp="$(mktemp)"
    awk -F'\t' -v id="$id" 'BEGIN{OFS="\t"} $1==id{$2="closed"} {print}' "$FIX" >"$tmp" && mv "$tmp" "$FIX"
    exit 0
    ;;
  *) exit 0 ;;
esac
EOF
chmod +x "${BIN}/bd"

run() {  # run <args...> ; captures combined stdout+stderr into $out
  out="$(PATH="${BIN}:${PATH}" HOME="${SANDBOX}" bash "$SRC" "$@" 2>&1)"
}

# --------------------------------------------------------------------------- #
# A. DRY-RUN classification.
# --------------------------------------------------------------------------- #
run --dry-run

# 1. merged-PR-title match IS flagged as a close-candidate.
want_contains "sq-merged1 flagged (merged PR title)"     "$out" "sq-merged1"
# 1b. commit-subject-only match IS flagged (no PR, only an origin/main commit).
want_contains "sq-commitonly flagged (commit subject)"   "$out" "sq-commitonly"

# 2. open-PR-only + unrelated beads are NOT flagged at all (absent from the merged blob).
want_absent "open-PR-only NOT flagged"  "$out" "sq-openonly"
want_absent "unrelated NOT flagged"     "$out" "sq-unrelated"

# 3. dotted-id false-match avoided: a merged sq-ixc3.11 must NOT flag the OPEN sq-ixc3.1.
#    (sq-ixc3.1 is the open bead; sq-ixc3.11 is the merged one. The .1 must not appear.)
if printf '%s' "$out" | grep -qE 'sq-ixc3\.1([^0-9]|$)'; then
  note_fail "sq-ixc3.1 wrongly flagged by merged sq-ixc3.11 (.1 vs .11 false-match)"
else
  note_pass
fi
# and the merged .11 itself is not an open bead in the fixture, so it appears nowhere as a
# candidate id either.
want_absent "sq-ixc3.11 is not an open bead -> not a candidate" "$out" "candidate.*sq-ixc3.11"

# 4. epic / umbrella-parent / needs:user are GATED (report-only), NOT close-candidates.
#    They are present in the GATED section but must not be in the CLOSE-CANDIDATES section.
cand_section="$(printf '%s' "$out" | sed -n '/^CLOSE-CANDIDATES/,/^GATED/p')"
gated_section="$(printf '%s' "$out" | sed -n '/^GATED/,$p')"
want_absent   "epic NOT a close-candidate"            "$cand_section" "sq-epicmerge"
want_contains "epic IS in the gated report"           "$gated_section" "sq-epicmerge"
want_absent   "umbrella-parent NOT a close-candidate" "$cand_section" "sq-parent9"
want_contains "umbrella-parent IS in gated report"    "$gated_section" "sq-parent9"
want_absent   "needs:user NOT a close-candidate"      "$cand_section" "sq-needs1"
want_contains "needs:user IS in the gated report"     "$gated_section" "sq-needs1"

# fail-safe: a bd-show lookup error SKIPS that bead (never flagged, never closed).
want_contains "show-fail bead is skipped (logged)"    "$out" "sq-brokey: meta lookup FAILED"
want_absent   "show-fail bead NOT a candidate"        "$cand_section" "sq-brokey"

# DRY-RUN must close NOTHING.
if [ -s "$BD_CLOSED_LOG" ]; then note_fail "dry-run closed beads (must not mutate)"; else note_pass; fi

# --------------------------------------------------------------------------- #
# B. --apply closes ONLY the non-gated candidates; leaves epics/needs/parents OPEN.
# --------------------------------------------------------------------------- #
: >"$BD_CLOSED_LOG"
run --apply
closed="$(cat "$BD_CLOSED_LOG")"
want_contains "apply closes sq-merged1"         "$closed" "sq-merged1"
want_contains "apply closes sq-commitonly"      "$closed" "sq-commitonly"
want_absent   "apply does NOT close epic"       "$closed" "sq-epicmerge"
want_absent   "apply does NOT close parent"     "$closed" "sq-parent9"
want_absent   "apply does NOT close needs:user" "$closed" "sq-needs1"
want_absent   "apply does NOT close sq-ixc3.1"  "$closed" "sq-ixc3.1"
# the close note uses the required "reconcile: fix merged via #N" form (visible in the log).
want_contains "apply note: reconcile via #101"  "$out" "reconcile: fix merged via #101"

# --------------------------------------------------------------------------- #
# C. IDEMPOTENT: a second --apply (the candidate beads are now closed, so `bd list
#    --status open` no longer returns them) closes NOTHING new and reports 0 candidates.
# --------------------------------------------------------------------------- #
: >"$BD_CLOSED_LOG"
run --apply
if [ -s "$BD_CLOSED_LOG" ]; then note_fail "second --apply re-closed already-closed beads (not idempotent)"; else note_pass; fi
want_contains "idempotent: 0 close-candidates on re-run" "$out" "CLOSE-CANDIDATES: 0"
# the previously-closed candidates must not reappear as candidates.
want_absent "idempotent: sq-merged1 not re-flagged" "$out" "sq-merged1"

# --------------------------------------------------------------------------- #
# D. FAIL-SAFE: with NO merged signal (gh + git both empty) the script closes NOTHING.
# --------------------------------------------------------------------------- #
cat >"${BIN}/gh"  <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat >"${BIN}/git" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "${BIN}/gh" "${BIN}/git"
# reset the fixture so beads are open again
cat >"$BD_FIXTURE" <<'EOF'
sq-merged1	open	bug	fix(core): the merged fix		0
EOF
: >"$BD_CLOSED_LOG"
run --apply
if [ -s "$BD_CLOSED_LOG" ]; then note_fail "no-signal --apply closed a bead (must fail-safe to no-op)"; else note_pass; fi
want_contains "no-signal: closes NOTHING (fail-safe)" "$out" "closing NOTHING"

# --------------------------------------------------------------------------- #
# E. The in-script pure-predicate self-test still passes (contract integrity).
# --------------------------------------------------------------------------- #
if PATH="${BIN}:${PATH}" bash "$SRC" --dry-run-self-test >/dev/null 2>&1; then note_pass; else note_fail "in-script --dry-run-self-test failed"; fi

# F. STATIC: the load-bearing invariants are present in the source (a refactor cannot drop them).
want_src() { if grep -q "$2" "$SRC"; then note_pass; else note_fail "$1"; fi; }
want_src "umbrella-parent gate removed from source"   'is_umbrella_parent'
want_src "gated-kind classifier removed from source"  'is_gated_kind'
want_src "exact-token matcher removed from source"    'bead_token_matches'
want_src "required close-note form removed from source" 'reconcile: fix merged via'

# --------------------------------------------------------------------------- #
echo ""
echo "test_reconcile_merged_beads: ${pass} passed, ${fail} failed."
[ "$fail" -eq 0 ] || exit 1
echo "test_reconcile_merged_beads: OK — exact-match + .1-vs-.11 guard + open-PR/unrelated exclusion + epic/umbrella/needs gating + apply/idempotency/fail-safe hold."
