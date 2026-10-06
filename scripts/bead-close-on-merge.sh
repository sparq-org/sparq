#!/usr/bin/env bash
# Orchestration automation — Phase A of research/orchestration-automation-design.md
# (PR #374).
#
# bead-close-on-merge.sh <pr> [--apply]
#
# Close the bead a PR maps to — but ONLY IF the PR *genuinely merged*. This is the
# single most error-prone bookkeeping step the orchestrator does by hand (design §1.1
# T2 / §5 Bead A), and the one with a sharp failure mode: closing a bead for a PR that
# did NOT merge (closed-unmerged, or a stale/spoofed signal). The guardrail (design §6,
# "Auto-close a bead for a PR that DIDN'T merge"):
#
#   * The merge is VERIFIED against the GitHub API — `gh pr view --json mergedAt` must be
#     non-null. We NEVER infer a merge from a parsed log/monitor line; a monitor event is
#     only a *wake*, and this script re-checks the API as the source of truth.
#   * DEFAULT is --dry-run: it prints WHAT it would close and exits 0 without mutating bd.
#     Closing requires the explicit --apply flag.
#   * Idempotent: a bead already closed is a no-op (reported, not re-closed).
#
# Bead-id resolution (in priority order):
#   1. `closingIssuesReferences` — but those are GitHub *issues*, not beads. We only honour
#      them when the issue title / the linked-issue body carries an sq-XXXX bead token
#      (the flow-on bridge mints GH issues that name their bead). A bare GH issue number is
#      NOT a bead id and is ignored (with a note).
#   2. an sq-XXXX (or sq-XXXX.NN) token in the PR title — the convention the brief documents
#      ("a sq-XXXX token in the title").
# If neither yields a bead id, the script reports "no mapped bead" and exits 0 (nothing to
# do is not an error — many PRs are chores with no bead).
#
# Run:
#   scripts/bead-close-on-merge.sh 374            # dry-run (default): what WOULD close
#   scripts/bead-close-on-merge.sh 374 --apply    # actually close, iff verified-merged
#   scripts/bead-close-on-merge.sh --dry-run-self-test   # hermetic self-test (no network)
set -euo pipefail

PROG="bead-close-on-merge"
log()  { printf '[%s] %s\n' "$PROG" "$*" >&2; }
die()  { printf '[%s] ERROR: %s\n' "$PROG" "$*" >&2; exit 1; }

# --- bead-id extraction (pure-text; the hermetic, testable core) -----------------------
# Echo every distinct sq-XXXX / sq-XXXX.NN token found in stdin, one per line, in order.
# Bead ids are `sq-` + a short base36-ish slug, optionally a `.NN` molecule suffix
# (e.g. sq-qhy4, sq-toze.37). We match conservatively and de-duplicate preserving order.
extract_bead_ids() {
  grep -oE 'sq-[a-z0-9]+(\.[0-9]+)?' 2>/dev/null | awk '!seen[$0]++' || true
}

# --- the --dry-run self-test (no network, asserts the extraction + guard logic) --------
self_test() {
  local fails=0 got
  _check() { # _check "<label>" "<got>" "<want>"
    if [ "$2" = "$3" ]; then
      printf '  ok   %s\n' "$1"
    else
      printf '  FAIL %s\n       got:  %q\n       want: %q\n' "$1" "$2" "$3"
      fails=$((fails + 1))
    fi
  }
  log "running --dry-run self-test (hermetic; no gh/bd calls)"

  got="$(printf 'fix(sparq-core): dict spill (sq-glw2)\n' | extract_bead_ids)"
  _check "title token sq-glw2"                     "$got" "sq-glw2"

  got="$(printf 'cert(sbom): purl thing (sq-tmyw, sq-toze.27)\n' | extract_bead_ids)"
  _check "two tokens incl molecule suffix"         "$got" "$(printf 'sq-tmyw\nsq-toze.27')"

  got="$(printf 'closes sq-abcd and again sq-abcd\n' | extract_bead_ids)"
  _check "dedupe preserves first occurrence"       "$got" "sq-abcd"

  got="$(printf 'chore(orchestration): no bead here #374\n' | extract_bead_ids)"
  _check "no token => empty"                        "$got" ""

  # A bare GH issue reference (#123) must NOT be mistaken for a bead.
  got="$(printf 'closingIssuesReferences: #123 #456\n' | extract_bead_ids)"
  _check "bare GH issue numbers are not beads"      "$got" ""

  # The verified-merge guard predicate (mergedAt non-null) — exercised directly.
  is_verified_merged 'null'  && fails=$((fails + 1)) && printf '  FAIL null mergedAt treated as merged\n' || printf '  ok   null mergedAt => NOT merged\n'
  is_verified_merged ''      && fails=$((fails + 1)) && printf '  FAIL empty mergedAt treated as merged\n' || printf '  ok   empty mergedAt => NOT merged\n'
  if is_verified_merged '2026-06-16T17:08:58Z'; then printf '  ok   real timestamp => merged\n'; else printf '  FAIL real timestamp not treated as merged\n'; fails=$((fails + 1)); fi

  echo
  if [ "$fails" -eq 0 ]; then
    log "self-test PASSED"
    return 0
  fi
  die "self-test FAILED ($fails check(s))"
}

# A merge is verified IFF mergedAt is a non-empty, non-"null" value. Pure predicate so the
# self-test can assert it without a network round-trip.
is_verified_merged() {
  local merged_at="${1-}"
  [ -n "$merged_at" ] && [ "$merged_at" != "null" ]
}

# --- arg parsing -----------------------------------------------------------------------
APPLY=0
PR=""
for arg in "$@"; do
  case "$arg" in
    --dry-run-self-test) self_test; exit 0 ;;
    --apply)             APPLY=1 ;;
    --dry-run)           APPLY=0 ;;
    -h|--help)
      sed -n '2,40p' "$0"; exit 0 ;;
    -*)
      die "unknown flag: $arg (try --apply, --dry-run, --dry-run-self-test, --help)" ;;
    *)
      [ -z "$PR" ] || die "unexpected extra argument: $arg"
      PR="$arg" ;;
  esac
done

[ -n "$PR" ] || die "usage: $PROG <pr> [--apply]  (default is --dry-run)"
case "$PR" in
  ''|*[!0-9]*) die "PR must be a number, got: $PR" ;;
esac

command -v gh >/dev/null 2>&1 || die "gh CLI not found (needed to verify the merge via the API)"
command -v bd >/dev/null 2>&1 || die "bd not found (needed to close the bead)"

# --- 1. VERIFY the merge against the API (never a parsed log line) ----------------------
log "querying gh for PR #$PR (mergedAt,state,title,closingIssuesReferences)"
PR_JSON="$(gh pr view "$PR" --json mergedAt,state,title,closingIssuesReferences 2>/dev/null)" \
  || die "gh pr view #$PR failed (does the PR exist? is gh authenticated?)"

MERGED_AT="$(printf '%s' "$PR_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("mergedAt") or "null")')"
STATE="$(printf '%s' "$PR_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("state") or "")')"
TITLE="$(printf '%s' "$PR_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("title") or "")')"

if ! is_verified_merged "$MERGED_AT"; then
  log "PR #$PR is NOT verified-merged (state=$STATE, mergedAt=$MERGED_AT) — refusing to close any bead."
  log "This is the guardrail: a bead is closed ONLY for an API-verified merge (mergedAt != null)."
  exit 0
fi
log "verified merge: PR #$PR mergedAt=$MERGED_AT state=$STATE"

# --- 2. RESOLVE the bead id ------------------------------------------------------------
# (a) sq-XXXX tokens carried by linked issues' titles/bodies (the flow-on bead bridge).
LINKED_TOKENS="$(printf '%s' "$PR_JSON" \
  | python3 -c 'import json,sys
d=json.load(sys.stdin)
refs=d.get("closingIssuesReferences") or []
for r in refs:
    print(r.get("title") or "")
    print(r.get("body") or "")' 2>/dev/null | extract_bead_ids)"

# (b) sq-XXXX tokens in the PR title (the documented convention).
TITLE_TOKENS="$(printf '%s\n' "$TITLE" | extract_bead_ids)"

mapfile -t BEAD_IDS < <(printf '%s\n%s\n' "$LINKED_TOKENS" "$TITLE_TOKENS" | grep -E '^sq-' | awk '!seen[$0]++' || true)

if [ "${#BEAD_IDS[@]}" -eq 0 ]; then
  log "PR #$PR merged but maps to NO bead (no sq-XXXX token in title or linked issues) — nothing to close."
  exit 0
fi

log "resolved bead id(s) for merged PR #$PR:"
printf '  %s\n' "${BEAD_IDS[@]}" >&2

# --- 3. CLOSE (or, in dry-run, report) — idempotent ------------------------------------
rc=0
for bid in "${BEAD_IDS[@]}"; do
  # Idempotency: skip beads already closed.
  status="$(bd show "$bid" --json 2>/dev/null \
    | python3 -c 'import json,sys
try:
    d=json.load(sys.stdin)
    if isinstance(d, list): d = d[0] if d else {}
    print(d.get("status") or "")
except Exception:
    print("")' 2>/dev/null || true)"

  if [ -z "$status" ]; then
    log "  $bid: not found in bd (or bd show failed) — skipping (no-op)."
    continue
  fi
  if [ "$status" = "closed" ]; then
    log "  $bid: already closed — no-op (idempotent)."
    continue
  fi

  if [ "$APPLY" -eq 1 ]; then
    if bd close "$bid" --reason "merged via PR #$PR (verified mergedAt=$MERGED_AT)" >/dev/null 2>&1; then
      log "  $bid: CLOSED (merged via PR #$PR)."
    else
      log "  $bid: bd close FAILED."
      rc=1
    fi
  else
    log "  $bid: WOULD close (currently '$status'). Re-run with --apply to act."
  fi
done

if [ "$APPLY" -eq 0 ]; then
  log "dry-run (default): no beads were modified. Re-run with --apply to close the above."
fi
exit "$rc"
