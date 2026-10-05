#!/usr/bin/env bash
# Recurrent maintenance — reconcile bd-OPEN beads against MERGED PRs.
# Bead sq-13uyp.
#
# reconcile-merged-beads.sh [--dry-run | --apply] [options]
#
# WHY THIS EXISTS
# ---------------
# 2026-06-21 (sq-bpoey): a bead was fixed AND merged via PR #1017 but left OPEN in bd,
# so it stayed on `scripts/push-frontier.sh`'s launchable frontier and an agent was
# dispatched only to find the work already done (~32k tokens wasted). The existing
# in-flight exclusion (sq-7mwun, baked into push-frontier.sh) subtracts beads with an
# OPEN PR — but here the PR was already MERGED, so that exclusion misses the whole
# class of "fixed-and-merged but the bead was never closed".
#
# This script is the COMPLEMENT to that open-PR exclusion: for each OPEN bead it asks
# "did this bead's fix already MERGE?" by looking for the bead's EXACT id token in a
# MERGED PR (title or head-branch) or in a commit subject on origin/main. A match is a
# CLOSE-CANDIDATE: the bead should be closed so it leaves the frontier.
#
#   open-PR exclusion (sq-7mwun)   handles beads whose work is IN FLIGHT (PR still OPEN).
#   this script (sq-13uyp)         handles beads whose work already LANDED (PR MERGED,
#                                  but the bead was never closed).
# Together they keep the frontier free of beads that should not be re-dispatched.
#
# DESIGN — the same discipline as bead-close-on-merge.sh / disk-guard.sh:
#   * DEFAULT IS --dry-run: it REPORTS each close-candidate (bead id + the merging PR#
#     or commit) and mutates NOTHING. Closing requires the explicit --apply flag.
#   * CONSERVATIVE EXACT MATCH: a bead matches ONLY on its EXACT dotted id token, with a
#     right-boundary that is NOT a digit and NOT a '.', so `sq-ixc3.1` never matches
#     `sq-ixc3.11` (and the base `sq-ixc3` never matches a `.N` molecule). Left boundary
#     is a non-id char (start, space, '(', '/', etc.). See bead_token_matches().
#   * OPEN-ONLY: only beads currently `open`/`in_progress` are considered (the frontier
#     statuses). `blocked`/`deferred`/`closed` are left alone.
#   * NEVER auto-close an EPIC or a needs:user / needs:maintainer / needs:decision /
#     decision bead — those are containers or human-gated; a merged child PR must not
#     silently close them. They are REPORTED for manual review instead, never --apply'd.
#   * IDEMPOTENT: a bead already closed is a no-op; a re-run produces no new closes.
#   * FAIL-SAFE: a per-bead lookup error (bd show failing, an unreadable label set)
#     SKIPS that one bead with a logged warning — it never aborts the sweep and never
#     mass-closes on a partial/failed lookup. The merged-blob is gathered once up front;
#     if it comes back EMPTY (gh + git both unavailable) the script closes NOTHING.
#
# Run:
#   scripts/reconcile-merged-beads.sh                 # dry-run (default): list candidates
#   scripts/reconcile-merged-beads.sh --apply         # close the matched, non-gated beads
#   scripts/reconcile-merged-beads.sh --json          # machine-readable candidate list
#   scripts/reconcile-merged-beads.sh --dry-run-self-test   # hermetic self-test (no net)
set -uo pipefail   # NOT -e: a maintenance tick is best-effort and must not abort its caller.

PROG="reconcile-merged-beads"
log()  { printf '[%s] %s\n' "$PROG" "$*" >&2; }
die()  { printf '[%s] ERROR: %s\n' "$PROG" "$*" >&2; exit 1; }

# Ensure the user-local bd is reachable even under a bare PATH (cron/hook context).
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) PATH="$PATH:$HOME/.local/bin" ;;
esac

# --- pure, testable core ----------------------------------------------------------------
# bead_token_matches <bead-id> <haystack>: return 0 (match) iff the EXACT bead-id token
# appears in <haystack> as a whole token. The boundary rules are the load-bearing,
# conservative part of this script.
#
# VERIFIED INVARIANT: every bead id in this tracker matches `sq-[a-z0-9]+(\.[0-9]+)?` — a
# `sq-` prefix, an alnum slug with NO internal hyphen, and an optional `.NN` molecule
# suffix. So an id-CONTINUATION char (one that, if it followed our token, would mean we had
# only matched a PREFIX of a longer id) is exactly [A-Za-z0-9.] — letters, digits, dot —
# but NOT '-'. A '-' right after a complete id is always a branch/word separator
# (`fix/sq-bpoey-site-anchors`), never an id continuation. The match therefore requires the
# token to be flanked by non-[A-Za-z0-9.] chars (or the string ends):
#   * RIGHT boundary excludes [A-Za-z0-9.]: stops `sq-ixc3.1` matching `sq-ixc3.11` (next
#     char '1'), the base `sq-ixc3` matching `sq-ixc3.4` (next char '.'), and `sq-ix`
#     matching `sq-ixc3` (next char 'c'); but ALLOWS `sq-bpoey-site...` (next char '-').
#   * LEFT boundary excludes [A-Za-z0-9.]: stops a prefixed variant like `xsq-bpoey`.
# Pure string logic (no bd/gh/git), so the self-test exercises it hermetically.
bead_token_matches() {
  local id="$1" hay="$2"
  [ -n "$id" ] || return 1
  # Escape any regex metachars in the id for a literal match of the id body. Bead ids are
  # `sq-<slug>(.NN)?` so the only metachar in practice is '.', but we escape defensively.
  local esc
  esc="$(printf '%s' "$id" | sed -e 's/[][\\.^$*+?(){}|/]/\\&/g')"
  # grep -P (PCRE) for precise look-behind / look-ahead boundaries.
  if printf '%s' "$hay" | grep -qzP "(?<![A-Za-z0-9.])${esc}(?![A-Za-z0-9.])" 2>/dev/null; then
    return 0
  fi
  # POSIX-ERE fallback (no look-around): require the token to be preceded by start/non-id
  # and followed by end/non-id — emulated by wrapping the haystack in spaces so a match at
  # either end still sees a non-id boundary char.
  printf '%s' " $hay " | grep -qE "(^|[^A-Za-z0-9.])${esc}([^A-Za-z0-9.]|$)"
}

# is_gated_kind <issue_type> <labels-csv>: return 0 (TRUE => do NOT auto-close, report for
# manual review) iff the bead is an EPIC or carries a human-gated / decision label. The
# labels are a comma-separated lowercased string. Pure (no bd), so it is unit-testable.
is_gated_kind() {
  local itype="$1" labels="$2"
  case "$itype" in epic) return 0 ;; esac
  case ",$labels," in
    *,needs:user,*|*,needs:maintainer,*|*,needs:decision,*|*,decision,*|*,needs:design,*)
      return 0 ;;
  esac
  return 1
}

# is_epic_title <title>: a bead whose title leads with "[epic]" OR "EPIC:" is treated as an
# epic even if its issue_type column is something else. This matches the maintainer's two
# conventions ("[epic] ..." and "EPIC: ...") and mirrors push-frontier.sh's umbrella rule —
# e.g. sq-gum8 ("EPIC: Academic paper factory ...") is typed `task` but is plainly an epic.
is_epic_title() {
  local lc
  lc="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')"
  case "$lc" in
    \[epic\]*) return 0 ;;
    "epic:"*)  return 0 ;;
  esac
  return 1
}

# is_umbrella_parent <dependent_count>: return 0 (TRUE => gate, report-only) iff the bead has
# >=1 dependent — i.e. another bead depends on/blocks-behind it, so it is a PARENT / blocker /
# decomposed container (push-frontier.sh's "umbrella parent" rule). A single merged child PR
# must NOT close such a bead, so we report it for manual review instead of auto-closing. A
# genuine leaf has dependent_count 0 and is unaffected. Pure (no bd), so it is unit-testable.
is_umbrella_parent() {
  case "$1" in ''|*[!0-9]*) return 1 ;; esac   # unknown/non-numeric ⇒ not flagged as umbrella.
  [ "$1" -ge 1 ]
}

# --- hermetic self-test (no bd/gh/git; no mutation) -------------------------------------
self_test() {
  local fails=0
  _ok()   { printf '  ok   %s\n' "$1"; }
  _fail() { printf '  FAIL %s\n       %s\n' "$1" "$2"; fails=$((fails + 1)); }
  _match()    { if bead_token_matches "$1" "$2"; then _ok "$3"; else _fail "$3" "expected MATCH for id='$1' in '$2'"; fi; }
  _nomatch()  { if bead_token_matches "$1" "$2"; then _fail "$3" "expected NO match for id='$1' in '$2'"; else _ok "$3"; fi; }
  log "running --dry-run self-test (hermetic; no bd/gh/git calls)"

  # --- exact-token matching ---------------------------------------------------
  _match   "sq-bpoey" "fix(site): broken anchors (sq-bpoey)"        "id in PR title (parenthesised)"
  _match   "sq-bpoey" "fix/sq-bpoey-site-anchors"                              "id in head-branch name"
  _match   "sq-bpoey" "fix(site): broken anchors (sq-bpoey) (#1017)" "id alongside a #PR number"
  _match   "sq-u6nmt" "ci(feature-matrix): gate sparq-kb (sq-u6nmt)" "real merged-PR title token"
  _match   "sq-ixc3.1" "feat: thing (sq-ixc3.1)"                    "molecule id .1 matches itself"
  _match   "sq-ixc3"   "feat: base thing (sq-ixc3) only"                       "base id matches itself"

  # --- the dotted-id FALSE-MATCH guard (the whole point) ----------------------
  _nomatch "sq-ixc3.1"  "feat: thing (sq-ixc3.11)"                  ".1 must NOT match .11 (right digit)"
  _nomatch "sq-ixc3.1"  "feat: thing (sq-ixc3.10)"                            ".1 must NOT match .10"
  _nomatch "sq-ixc3"    "feat: molecule (sq-ixc3.4)"                          "base id must NOT match a .N molecule"
  _nomatch "sq-ixc3"    "feat: molecule (sq-ixc3.11)"                         "base id must NOT match .11"
  _nomatch "sq-ix"      "feat: longer id (sq-ixc3)"                           "prefix id must NOT match longer id"
  _nomatch "sq-bpoey"   "fix: unrelated bead (sq-other9)"                     "unrelated id is NOT a match"
  _nomatch "sq-bpoey"   "fix: substring xsq-bpoey embedded"                   "left boundary: x-prefixed is NOT a match"

  # an open-PR-only bead: nothing in the MERGED blob mentions it ⇒ not flagged. (We model
  # "open-PR-only" simply as: its id is absent from the merged haystack — exactly how main
  # treats it: the merged blob is built ONLY from merged PRs + origin/main commits.)
  _nomatch "sq-openpr1" "feat: a totally different merged thing (sq-merged9)"  "open-PR-only id absent from merged blob"

  # --- gated-kind classification ----------------------------------------------
  _gated()   { if is_gated_kind "$1" "$2"; then _ok "$3"; else _fail "$3" "expected GATED (no auto-close)"; fi; }
  _ungated() { if is_gated_kind "$1" "$2"; then _fail "$3" "expected UNGATED (auto-closeable)"; else _ok "$3"; fi; }

  _gated   "epic" ""                                  "issue_type=epic is gated"
  _gated   "task" "area:site,needs:user"              "needs:user label is gated"
  _gated   "feature" "from:maintainer,needs:maintainer" "needs:maintainer label is gated"
  _gated   "bug"  "needs:decision"                    "needs:decision label is gated"
  _gated   "task" "decision"                          "decision label is gated"
  _ungated "bug"  "area:site,from:agent"              "ordinary bug w/ benign labels is auto-closeable"
  _ungated "task" ""                                  "ordinary task w/ no labels is auto-closeable"
  _ungated "feature" "area:zk"                        "ordinary feature is auto-closeable"

  if is_epic_title "[epic] big umbrella"; then _ok "[epic]-titled is an epic"; else _fail "[epic] title" "expected epic"; fi
  if is_epic_title "EPIC: Academic paper factory"; then _ok "EPIC:-titled is an epic (sq-gum8 class)"; else _fail "EPIC: title" "expected epic"; fi
  if is_epic_title "fix: not an epic"; then _fail "non-epic title" "expected NOT epic"; else _ok "non-[epic] title is not an epic"; fi
  if is_epic_title "feat: epically fast loader"; then _fail "mid-word 'epic'" "expected NOT epic"; else _ok "mid-word 'epic' is not an epic title"; fi

  # --- umbrella-parent (>=1 dependent) gating ---------------------------------
  if is_umbrella_parent "31"; then _ok "31 dependents -> umbrella (sq-bif class)"; else _fail "umbrella 31" "expected umbrella"; fi
  if is_umbrella_parent "1";  then _ok "1 dependent -> umbrella"; else _fail "umbrella 1" "expected umbrella"; fi
  if is_umbrella_parent "0";  then _fail "0 dependents" "expected leaf (not umbrella)"; else _ok "0 dependents -> leaf (auto-closeable)"; fi
  if is_umbrella_parent "";   then _fail "unknown count" "expected NOT umbrella on unknown"; else _ok "unknown dependent-count -> not umbrella"; fi

  echo
  if [ "$fails" -eq 0 ]; then log "self-test PASSED"; return 0; fi
  die "self-test FAILED ($fails check(s))"
}

# --- arg parsing ------------------------------------------------------------------------
APPLY=0
JSON=0
MERGED_LIMIT=400      # how many merged PRs to scan (newest-first).
COMMIT_LIMIT=600      # how many origin/main commit subjects to scan.
for arg in "$@"; do
  case "$arg" in
    --dry-run-self-test) self_test; exit 0 ;;
    --apply)             APPLY=1 ;;
    --dry-run)           APPLY=0 ;;
    --json)              JSON=1 ;;
    --merged-limit=*)    MERGED_LIMIT="${arg#--merged-limit=}" ;;
    --commit-limit=*)    COMMIT_LIMIT="${arg#--commit-limit=}" ;;
    -h|--help)           sed -n '2,60p' "$0"; exit 0 ;;
    *)                   die "unknown argument: $arg (try --apply, --json, --dry-run-self-test, --help)" ;;
  esac
done

command -v bd >/dev/null 2>&1 || die "bd not found (this script reads 'bd list'); add ~/.local/bin to PATH"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# --- 1. gather the MERGED blob (merged-PR titles + head-branches + origin/main subjects) -
# Built ONCE, up front. It is the ONLY haystack we search for bead ids. If BOTH sources
# are unavailable it stays empty and we close NOTHING (fail-safe: no signal => no action).
MERGED_BLOB=""
if command -v gh >/dev/null 2>&1; then
  # We keep the PR NUMBER on each line ("#<n> <branch> <title>") so a match can be
  # attributed to a specific PR for the close note.
  PR_LINES="$(gh pr list --state merged --limit "$MERGED_LIMIT" \
      --json number,headRefName,title \
      -q '.[] | "#\(.number) \(.headRefName) \(.title)"' 2>/dev/null || true)"
  [ -n "$PR_LINES" ] && MERGED_BLOB+="$PR_LINES"$'\n'
else
  log "gh not found — MERGED-PR signal will be EMPTY (origin/main commit subjects still used)."
fi

# origin/main commit subjects (squash-merge puts "(sq-XXXX)" in the subject). Prefix each
# with its short SHA so a commit-only match can be attributed for the note.
if command -v git >/dev/null 2>&1; then
  COMMIT_LINES="$(git -C "$ROOT" log origin/main --no-merges --format='commit:%h %s' -n "$COMMIT_LIMIT" 2>/dev/null || true)"
  [ -n "$COMMIT_LINES" ] && MERGED_BLOB+="$COMMIT_LINES"$'\n'
else
  log "git not found — origin/main commit-subject signal will be EMPTY."
fi

if [ -z "$MERGED_BLOB" ]; then
  log "no merged-PR or commit signal available (gh + git both empty) — closing NOTHING (fail-safe)."
  [ "$JSON" -eq 1 ] && printf '{"candidates":[],"count":0,"applied":false,"reason":"no-merged-signal"}\n'
  exit 0
fi

# --- 2. read the OPEN frontier-status beads --------------------------------------------
# Only open + in_progress are frontier statuses; blocked/deferred/closed are left alone.
OPEN_JSON="$(bd list --status open --json --limit 0 2>/dev/null)" || die "bd list --status open --json failed"
INPROG_JSON="$(bd list --status in_progress --json --limit 0 2>/dev/null || echo '[]')"

# Merge the two arrays into "id<TAB>issue_type<TAB>title" rows. `bd list` does NOT carry
# labels (only `bd show` does), so the per-bead label fetch (the gated-kind check) happens
# below, lazily and fail-safe.
# shellcheck disable=SC2016 # the single-quoted block is Python, not shell — no shell expansion intended
ROWS="$(printf '%s\n%s\n' "$OPEN_JSON" "$INPROG_JSON" | python3 -c '
import json,sys
rows=[]
buf=sys.stdin.read()
# Two JSON arrays concatenated with a newline — parse each non-empty line-group. Simplest:
# split on the array boundary by trying to decode greedily.
dec=json.JSONDecoder()
i=0; s=buf.strip()
items=[]
while i < len(s):
    while i < len(s) and s[i] in " \n\r\t": i+=1
    if i>=len(s): break
    obj,end=dec.raw_decode(s,i)
    if isinstance(obj,list): items.extend(obj)
    i=end
seen=set()
def flat(x): return (x or "").replace("\t"," ").replace("\n"," ").replace("\r"," ")
for it in items:
    iid=it.get("id","")
    if not iid or iid in seen: continue
    seen.add(iid)
    # issue_type defaults to the sentinel "none" (never a real type) so a TAB-delimited
    # `read` cannot collapse an empty middle field and shift the columns (bash IFS=$"\t"
    # treats tab as whitespace and DOES collapse consecutive tabs).
    print("\t".join([iid, (it.get("issue_type") or "none"), flat(it.get("title",""))]))
' 2>/dev/null)" || die "could not parse bd list --json"

[ -n "$ROWS" ] || { log "no open/in_progress beads — nothing to reconcile."; [ "$JSON" -eq 1 ] && printf '{"candidates":[],"count":0,"applied":false}\n'; exit 0; }

# --- 3. MATCH each OPEN bead against the merged blob (one fast pass) --------------------
# Doing this in shell (grep-per-bead-per-line) is O(beads x blob-lines) subprocess spawns
# and was unusably slow on a ~300-bead backlog. We instead do the matching in ONE Python
# pass that applies the EXACT SAME boundary regex the shell `bead_token_matches` uses
# (verified char-for-char by the self-test: `(?<![A-Za-z0-9.])<id>(?![A-Za-z0-9.])`), and
# emit only the MATCHED rows ("id<TAB>ref<TAB>title"). Only those (a small set) then incur a
# per-bead `bd show` for labels. ROWS + MERGED_BLOB are passed via env to avoid arg limits.
MATCHED="$(BEAD_ROWS="$ROWS" MERGED_BLOB="$MERGED_BLOB" python3 -c '
import os,re,sys
rows=os.environ.get("BEAD_ROWS","")
blob=os.environ.get("MERGED_BLOB","")
blob_lines=[ln for ln in blob.split("\n") if ln.strip()]

def attribution_ref(line):
    if line.startswith("commit:"):
        m=re.match(r"^commit:([0-9a-f]+)",line)
        return "commit "+(m.group(1) if m else "?")
    if line.startswith("#"):
        m=re.match(r"^#([0-9]+)",line)
        return "#"+(m.group(1) if m else "?")
    return "merged"

# For each open bead, find the FIRST merged-blob line containing its EXACT id token.
# Boundary: id flanked by non-[A-Za-z0-9.] (or string end) — identical to the shell matcher.
for row in rows.split("\n"):
    if not row.strip(): continue
    parts=row.split("\t")
    if len(parts)<3: continue
    iid,itype,title=parts[0],parts[1],parts[2]
    if not iid: continue
    pat=re.compile(r"(?<![A-Za-z0-9.])"+re.escape(iid)+r"(?![A-Za-z0-9.])")
    hit=None
    for ln in blob_lines:
        if pat.search(ln):
            hit=ln; break
    if hit is None: continue
    print("\t".join([iid, itype, attribution_ref(hit), title]))
' 2>/dev/null)" || die "merged-blob match pass failed"

# --- 4. classify the matched set: gated (report-only) vs close-candidate ---------------
declare -a CAND_IDS CAND_REFS CAND_TITLES
declare -a GATED_IDS GATED_REFS GATED_TITLES
n_cand=0 n_gated=0 n_closed=0 n_skiperr=0

# Lazy, fail-safe per-bead fetch: echoes "<labels-csv>\t<dependent_count>" (labels lowercased,
# comma-joined), or the sentinel "__LOOKUP_ERROR__" if bd show fails (caller then SKIPS that
# bead — never closes on a partial/failed read).
bead_meta() {
  local id="$1" out
  out="$(bd show "$id" --json 2>/dev/null | python3 -c '
import json,sys
try:
    d=json.load(sys.stdin)
    if isinstance(d,list): d=d[0] if d else {}
    labs=[ (l or "").lower() for l in (d.get("labels") or []) ]
    dc=d.get("dependent_count")
    print(",".join(labs)+"\t"+(str(dc) if isinstance(dc,int) else ""))
except Exception:
    print("__LOOKUP_ERROR__")
' 2>/dev/null)" || { printf '__LOOKUP_ERROR__'; return 0; }
  printf '%s' "$out"
}

# Only the matched beads reach here (a small set), so a per-bead bd show is cheap.
if [ -n "$MATCHED" ]; then
  while IFS=$'\t' read -r id itype ref title; do
    [ -n "$id" ] || continue

    # An "[epic]"/"EPIC:"-titled bead is an epic regardless of its issue_type column.
    is_epic_title "$title" && itype="epic"

    # Fetch labels + dependent-count (fail-safe). A lookup error SKIPS this bead.
    meta="$(bead_meta "$id")"
    if [ "$meta" = "__LOOKUP_ERROR__" ]; then
      log "  $id: meta lookup FAILED — skipping (fail-safe; never close on a partial read)."
      n_skiperr=$((n_skiperr + 1))
      continue
    fi
    labels="${meta%%$'\t'*}"
    dependents="${meta#*$'\t'}"
    [ "$labels" = "$meta" ] && dependents=""   # no tab => unknown dependent_count.

    # GATE (report-only, NEVER auto-close): epics / needs:* / decision beads, AND any bead
    # that is a PARENT (>=1 dependent) — a single merged child PR must not close an umbrella.
    if is_gated_kind "$itype" "$labels" || is_umbrella_parent "$dependents"; then
      GATED_IDS+=("$id"); GATED_REFS+=("$ref"); GATED_TITLES+=("$title")
      n_gated=$((n_gated + 1))
      continue
    fi

    CAND_IDS+=("$id"); CAND_REFS+=("$ref"); CAND_TITLES+=("$title")
    n_cand=$((n_cand + 1))
  done <<< "$MATCHED"
fi

# --- 5. apply (close) or report ---------------------------------------------------------
apply_one() {  # apply_one <id> <ref> ; idempotent + fail-safe
  local id="$1" ref="$2" prn note
  # Idempotency: skip a bead already closed (status changed since we read the list).
  local status
  status="$(bd show "$id" --json 2>/dev/null | python3 -c '
import json,sys
try:
    d=json.load(sys.stdin)
    if isinstance(d,list): d=d[0] if d else {}
    print(d.get("status") or "")
except Exception: print("")' 2>/dev/null || true)"
  if [ "$status" = "closed" ]; then
    log "  $id: already closed — no-op (idempotent)."
    return 0
  fi
  if [ -z "$status" ]; then
    log "  $id: status lookup failed at apply-time — skipping (fail-safe)."
    return 0
  fi
  # Build the note. Prefer "#N" form ("fix merged via #N") when the match was a PR.
  prn="${ref#\#}"
  case "$ref" in
    \#*) note="reconcile: fix merged via #${prn}" ;;
    *)   note="reconcile: fix merged via ${ref}" ;;
  esac
  if bd close "$id" --reason "$note" >/dev/null 2>&1; then
    log "  $id: CLOSED ($note)."
    n_closed=$((n_closed + 1))
  else
    log "  $id: bd close FAILED — leaving open (fail-safe)."
  fi
}

emit_human_report() {
  log "merged-but-bead-still-open CLOSE-CANDIDATES: $n_cand  |  gated (manual review): $n_gated  |  skipped-on-error: $n_skiperr"
  if [ "$n_cand" -gt 0 ]; then
    echo "CLOSE-CANDIDATES (open bead whose fix already MERGED):"
    local i=0
    while [ "$i" -lt "$n_cand" ]; do
      printf '  %s  [%s]  %s\n' "${CAND_IDS[$i]}" "${CAND_REFS[$i]}" "${CAND_TITLES[$i]}"
      i=$((i + 1))
    done
  fi
  if [ "$n_gated" -gt 0 ]; then
    echo "GATED — REPORT-ONLY (epic / umbrella-parent / needs:user / decision — NEVER auto-closed):"
    local i=0
    while [ "$i" -lt "$n_gated" ]; do
      printf '  %s  [%s]  %s\n' "${GATED_IDS[$i]}" "${GATED_REFS[$i]}" "${GATED_TITLES[$i]}"
      i=$((i + 1))
    done
  fi
}

emit_json_report() {
  python3 -c '
import json,sys
ids=sys.argv[1].split("\x1f") if sys.argv[1] else []
refs=sys.argv[2].split("\x1f") if sys.argv[2] else []
titles=sys.argv[3].split("\x1f") if sys.argv[3] else []
gids=sys.argv[4].split("\x1f") if sys.argv[4] else []
grefs=sys.argv[5].split("\x1f") if sys.argv[5] else []
gtitles=sys.argv[6].split("\x1f") if sys.argv[6] else []
applied=sys.argv[7]=="1"
cands=[{"id":i,"ref":r,"title":t} for i,r,t in zip(ids,refs,titles)]
gated=[{"id":i,"ref":r,"title":t} for i,r,t in zip(gids,grefs,gtitles)]
print(json.dumps({"candidates":cands,"count":len(cands),"gated":gated,"gated_count":len(gated),"applied":applied}))
' \
    "$(IFS=$'\x1f'; echo "${CAND_IDS[*]-}")" \
    "$(IFS=$'\x1f'; echo "${CAND_REFS[*]-}")" \
    "$(IFS=$'\x1f'; echo "${CAND_TITLES[*]-}")" \
    "$(IFS=$'\x1f'; echo "${GATED_IDS[*]-}")" \
    "$(IFS=$'\x1f'; echo "${GATED_REFS[*]-}")" \
    "$(IFS=$'\x1f'; echo "${GATED_TITLES[*]-}")" \
    "$APPLY"
}

if [ "$APPLY" -eq 1 ]; then
  log "APPLY: closing $n_cand close-candidate bead(s) (gated beads are NEVER auto-closed)."
  i=0
  while [ "$i" -lt "$n_cand" ]; do
    apply_one "${CAND_IDS[$i]}" "${CAND_REFS[$i]}"
    i=$((i + 1))
  done
  log "closed $n_closed bead(s); $n_gated gated bead(s) left for manual review; $n_skiperr skipped on error."
else
  log "dry-run (default): NO beads modified. Re-run with --apply to close the close-candidates."
fi

if [ "$JSON" -eq 1 ]; then
  emit_json_report
else
  emit_human_report
fi
exit 0
