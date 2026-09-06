#!/usr/bin/env bash
# [OPUS-5] 🤖 SPARQ agent — the BOX-SIDE beads producer (issue #6088, the residual
# half of sq-2xdg). Authored by Opus 5.
#
#   beads-export-push.sh [--dry-run | --apply] [--json] [--base <ref>]
#                        [--no-fetch] [--dry-run-self-test]
#
# WHY THIS EXISTS
# ---------------
# `.beads/issues.jsonl` is the committed source-of-record for the bead tracker
# (AGENTS.md § "Task tracking — beads, not markdown TODOs"), but nothing PRODUCES it
# on a schedule. It is regenerated only when the orchestrator happens to run
# `bd export` by hand onto a `chore-beads-resync-*` branch. A bead created with
# `bd create` on the work box and never re-exported is therefore invisible to CI: the
# Dolt DB is git-ignored (`.beads/.gitignore` ignores `dolt/`), so a GitHub-hosted
# runner checking out this repo never sees it, and the issue board only covers the
# beads that were migrated to a GitHub issue (#2475). No lane running on a checkout
# of this repo can recover the rest — only a producer on the box that HAS the DB can.
#
# THE DECISION THIS SCRIPT ENCODES (issue #6088 asked for it explicitly).
# The alternative reading — "the bd->issues migration made the issue board the sole
# source of record, so retire the committed JSONL" — is NOT taken here, because the
# repo still treats bd as the planned-task graph on every path that matters:
# AGENTS.md names `.beads/issues.jsonl` the committed source-of-record and instructs
# every agent to `bd create`; `scripts/push-frontier.sh` dispatches off `bd ready`;
# `scripts/ci-close-merged-beads.py` keeps a JSONL edit mode for orchestrator use;
# and the self-improvement channel is deliberately split (issues = newly-DISCOVERED
# work, beads = the PLANNED graph). Retiring the mirror would delete the only
# git-visible copy of that graph. So: keep the mirror, and give it a producer.
# The other producer option the issue floated — `bd-to-issues.py --apply`, which
# bulk-creates hundreds of GitHub issues — is deliberately NOT used: that script's
# own header holds `--apply` for the maintainer's explicit go-ahead, and a cron that
# mass-creates issues is not a reversible action.
#
# WHAT IT DOES
#   1. Runs `bd export` (the same command the orchestrator runs by hand).
#   2. Diffs it, KEY-WISE and ORDER-INSENSITIVELY, against `.beads/issues.jsonl` as
#      it exists on the BASE ref (`origin/main`) — not against the local worktree
#      copy, because the base is what a PR would actually change.
#   3. DEFAULT (--dry-run): reports the drift (added / removed / changed bead ids)
#      and mutates nothing, locally or remotely.
#   4. --apply: builds a one-file commit on top of the base ref and opens a
#      `chore-beads-resync-*` PR for it.
#
# NON-DISRUPTIVE BY CONSTRUCTION. --apply NEVER touches the checkout: no branch
# switch, no `git add`, no index write, no stash. It builds the commit with git
# PLUMBING against a private temporary index (`git read-tree` -> `update-index` ->
# `write-tree` -> `commit-tree`) and pushes the resulting commit object straight to
# a new remote branch (`git push origin <sha>:refs/heads/<branch>`), so not even a
# local ref is created. This matters because the work box's main checkout is the
# orchestrator's own working copy (AGENTS.md § "the orchestrator keeps the main
# checkout for itself") and a cron must never move it out from under a live session.
#
# FAIL-SAFES (a cron that can push must fail CLOSED, never destructively):
#   * EMPTY EXPORT IS FATAL. A `bd export` that fails, or emits zero records, would
#     otherwise commit an empty file and delete the entire mirror. Refused.
#   * RETENTION FLOOR. An export carrying fewer than MIN_RETAIN_PCT% of the base's
#     record count is refused as a suspected partial/truncated DB read. A genuine
#     bulk close still SHRINKS the file — closed beads stay in the export — so this
#     floor is about truncation, not about legitimate churn.
#   * MALFORMED EXPORT IS FATAL. Every line must parse as JSON and carry a unique
#     `id`; a duplicate or unparseable record is refused rather than committed.
#   * ONE OPEN RESYNC PR AT A TIME. If a `chore-beads-resync-*` PR is already open,
#     --apply is a logged no-op. Without this a cron opens a new PR every tick while
#     the previous one waits for the merge train.
#   * SINGLE-WRITER LOCK. An flock in the git dir serialises ticks against each
#     other (and against a concurrent hand-run), so two exports never race the DB.
#   * MAIN-CHECKOUT ONLY. Refuses to run from a linked worktree: `.beads/` in a
#     worktree is the merge-conflict footgun AGENTS.md forbids touching.
#   * A FETCH FAILURE IS FATAL IN --apply. Comparing against a stale base would open
#     a redundant or conflicting PR. In --dry-run it is only a warning (the report
#     is advisory).
#
# ORDER-INSENSITIVE DIFF, RAW COMMIT. This deliberately does NOT assume `bd export`
# emits records — or the keys within a record — in a stable order. (The committed
# mirror is not id-sorted, so the order is bd's own and is not a documented contract.)
# Under a byte-comparison, either kind of reordering would look like drift and spam a
# PR every tick, forever. So the DECISION is made on canonicalised records
# (keyed by `id`, each compared as `json.dumps(..., sort_keys=True)`), while the
# COMMITTED bytes are the raw `bd export` output — the same form a hand re-export
# produces, so this script and the orchestrator cannot fight over the file's shape.
#
# INSTALL (work box, hourly; the cadence is a safety net, not the clock):
#   17 * * * * cd /home/ubuntu/sparq && flock -n /tmp/sparq-beads-export.cron \
#     scripts/beads-export-push.sh --apply >> /tmp/beads-export-push.log 2>&1
#
# Run:
#   scripts/beads-export-push.sh                      # dry-run (default): report drift
#   scripts/beads-export-push.sh --json               # machine-readable drift report
#   scripts/beads-export-push.sh --apply              # open the chore-beads-resync PR
#   scripts/beads-export-push.sh --dry-run-self-test  # hermetic self-test (no net, no bd)
set -uo pipefail   # NOT -e: every failure path below is handled explicitly.

PROG="beads-export-push"
MIN_RETAIN_PCT=50          # refuse an export smaller than this % of the base record count
# How long to wait for the single-writer lock before skipping the tick. Overridable so an
# operator (and the test harness) can drive the contention path without a 2-minute stall.
LOCK_WAIT_SECS="${BEADS_EXPORT_LOCK_WAIT:-120}"
RESYNC_PREFIX="chore-beads-resync-"
TRACKED_FILE=".beads/issues.jsonl"

log() { printf '[%s] %s\n' "$PROG" "$*" >&2; }
die() { printf '[%s] ERROR: %s\n' "$PROG" "$*" >&2; exit 1; }

# Ensure the user-local bd/gh are reachable under cron's bare PATH.
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) PATH="$PATH:$HOME/.local/bin" ;;
esac

MODE="dry-run"; JSON=0; BASE_REF="origin/main"; DO_FETCH=1; SELFTEST=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) MODE="dry-run" ;;
    --apply)   MODE="apply" ;;
    --json)    JSON=1 ;;
    --no-fetch) DO_FETCH=0 ;;
    --base)    shift; BASE_REF="${1:-}"; [ -n "$BASE_REF" ] || die "--base needs a ref" ;;
    --dry-run-self-test) SELFTEST=1 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
  shift
done

# --- pure, testable core --------------------------------------------------------------
# is_resync_branch <branch>: TRUE iff the branch is one of OUR snapshot branches. This is
# the open-PR guard's predicate — it must not match an unrelated `chore-…` branch, or the
# producer would silently stop when some other chore PR is open.
is_resync_branch() {
  case "${1:-}" in
    "${RESYNC_PREFIX}"?*) return 0 ;;
    *) return 1 ;;
  esac
}

# retain_ok <export_records> <base_records>: TRUE iff the export is large enough to be
# plausible. A base of 0 (no committed mirror yet) accepts any non-empty export. Integer
# math only: export*100 >= base*MIN_RETAIN_PCT.
retain_ok() {
  local got="${1:-0}" base="${2:-0}"
  [ "$got" -gt 0 ] || return 1
  [ "$base" -gt 0 ] || return 0
  [ $(( got * 100 )) -ge $(( base * MIN_RETAIN_PCT )) ]
}

# beads_diff <base-file> <export-file>: emit `key=value` lines describing the drift.
# Keys: records_base, records_export, added, removed, changed (comma-separated id lists
# for added/removed/changed). Exits 3 on a malformed or duplicate-id export — the caller
# treats that as fatal, so a bad export is never committed.
beads_diff() {
  python3 - "$1" "$2" <<'PY'
import json, re, sys

# Every bead id in this tracker is `sq-<slug>(.NN)?`. We hold ids to a conservative
# token charset because they are interpolated UNESCAPED into this script's --json
# output, its log lines and the generated PR body — an id carrying a quote, a
# backslash or a newline would corrupt all three. Rejecting is the safe direction:
# the export is refused rather than committed or mis-reported.
ID_OK = re.compile(r"^[A-Za-z0-9._-]+$")

def load(path, label):
    recs = {}
    try:
        fh = open(path, encoding="utf-8")
    except FileNotFoundError:
        return recs          # absent base == an empty mirror, not an error
    with fh:
        for n, line in enumerate(fh, 1):
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except ValueError as exc:
                sys.exit(f"{label}:{n}: not valid JSON ({exc})")
            if not isinstance(obj, dict) or not obj.get("id"):
                sys.exit(f"{label}:{n}: record has no `id`")
            rid = obj["id"]
            if not isinstance(rid, str) or not ID_OK.match(rid):
                sys.exit(f"{label}:{n}: id {rid!r} is outside the safe token charset [A-Za-z0-9._-]")
            if rid in recs:
                sys.exit(f"{label}:{n}: duplicate id {rid}")
            # Canonical form: key order inside a record is not drift.
            recs[rid] = json.dumps(obj, sort_keys=True, separators=(",", ":"))
    return recs

base = load(sys.argv[1], "base")
new = load(sys.argv[2], "export")
added = sorted(set(new) - set(base))
removed = sorted(set(base) - set(new))
changed = sorted(i for i in (set(base) & set(new)) if base[i] != new[i])
out = sys.stdout
print(f"records_base={len(base)}", file=out)
print(f"records_export={len(new)}", file=out)
print("added=" + ",".join(added), file=out)
print("removed=" + ",".join(removed), file=out)
print("changed=" + ",".join(changed), file=out)
PY
  # python3's sys.exit(str) is exit status 1; normalise it to our "malformed" code.
  local rc=$?
  [ "$rc" -eq 0 ] || return 3
}

# --- self-test ------------------------------------------------------------------------
if [ "$SELFTEST" -eq 1 ]; then
  p=0; f=0
  ok()  { p=$((p + 1)); }
  bad() { f=$((f + 1)); printf 'SELFTEST FAILED: %s\n' "$1"; }
  check() { if [ "$1" = "$2" ]; then ok; else bad "$3 (got '$1', want '$2')"; fi; }

  # is_resync_branch: matches our prefix WITH a suffix; never a bare prefix or a sibling chore.
  if is_resync_branch "chore-beads-resync-20260903T101500Z"; then ok; else bad "resync branch not matched"; fi
  if is_resync_branch "chore-beads-resync-"; then bad "bare prefix must not match (no snapshot id)"; else ok; fi
  if is_resync_branch "chore-beads-something-else"; then bad "unrelated chore branch matched"; else ok; fi
  if is_resync_branch "fix/sq-2xdg-beads"; then bad "unrelated branch matched"; else ok; fi
  if is_resync_branch ""; then bad "empty branch matched"; else ok; fi

  # retain_ok: the truncation floor. 50% of 100 is the boundary and is ACCEPTED.
  if retain_ok 100 100; then ok; else bad "equal counts rejected"; fi
  if retain_ok 50 100;  then ok; else bad "exactly at the floor rejected"; fi
  if retain_ok 49 100;  then bad "below the floor accepted"; else ok; fi
  if retain_ok 0 0;     then bad "empty export accepted (empty is always fatal)"; else ok; fi
  if retain_ok 1 0;     then ok; else bad "first-ever export against an empty base rejected"; fi
  if retain_ok 500 100; then ok; else bad "growth rejected"; fi

  # beads_diff: added / removed / changed / key-order-is-not-drift / malformed is fatal.
  st="$(mktemp -d)"; trap 'rm -rf "$st"' EXIT
  printf '%s\n' \
    '{"id":"sq-a","title":"A","status":"open"}' \
    '{"id":"sq-b","title":"B","status":"open"}' \
    '{"id":"sq-c","title":"C","status":"open"}' > "$st/base.jsonl"
  # sq-a: same record, keys REORDERED (must NOT be drift). sq-b: status changed.
  # sq-c: removed. sq-d: added.
  printf '%s\n' \
    '{"status":"open","title":"A","id":"sq-a"}' \
    '{"id":"sq-b","title":"B","status":"closed"}' \
    '{"id":"sq-d","title":"D","status":"open"}' > "$st/new.jsonl"
  d="$(beads_diff "$st/base.jsonl" "$st/new.jsonl")"; check "$?" "0" "beads_diff exit"
  check "$(printf '%s\n' "$d" | sed -n 's/^added=//p')"   "sq-d" "added"
  check "$(printf '%s\n' "$d" | sed -n 's/^removed=//p')" "sq-c" "removed"
  check "$(printf '%s\n' "$d" | sed -n 's/^changed=//p')" "sq-b" "changed (sq-a reordered keys must not appear)"
  check "$(printf '%s\n' "$d" | sed -n 's/^records_base=//p')"   "3" "records_base"
  check "$(printf '%s\n' "$d" | sed -n 's/^records_export=//p')" "3" "records_export"

  # An absent base is an empty mirror, not an error.
  d="$(beads_diff "$st/nope.jsonl" "$st/new.jsonl")"; check "$?" "0" "absent base tolerated"
  check "$(printf '%s\n' "$d" | sed -n 's/^records_base=//p')" "0" "absent base record count"

  # Malformed / duplicate-id exports are FATAL (exit 3) — never committed.
  printf '%s\n' '{"id":"sq-a"' > "$st/bad.jsonl"
  beads_diff "$st/base.jsonl" "$st/bad.jsonl" >/dev/null 2>&1; check "$?" "3" "malformed JSON must be fatal"
  printf '%s\n' '{"id":"sq-a"}' '{"id":"sq-a"}' > "$st/dup.jsonl"
  beads_diff "$st/base.jsonl" "$st/dup.jsonl" >/dev/null 2>&1; check "$?" "3" "duplicate id must be fatal"
  printf '%s\n' '{"title":"no id"}' > "$st/noid.jsonl"
  beads_diff "$st/base.jsonl" "$st/noid.jsonl" >/dev/null 2>&1; check "$?" "3" "missing id must be fatal"
  # An id outside the safe token charset would be interpolated UNESCAPED into --json,
  # the log lines and the PR body. Refuse the export instead.
  printf '%s\n' '{"id":"sq-a\",\"evil\":\"x"}' > "$st/badid.jsonl"
  beads_diff "$st/base.jsonl" "$st/badid.jsonl" >/dev/null 2>&1; check "$?" "3" "quote in an id must be fatal"
  printf '%s\n' '{"id":"sq-a b"}' > "$st/spaceid.jsonl"
  beads_diff "$st/base.jsonl" "$st/spaceid.jsonl" >/dev/null 2>&1; check "$?" "3" "space in an id must be fatal"
  # ...but the real id shapes this tracker uses (slug and `.NN` molecule) are accepted.
  printf '%s\n' '{"id":"sq-ixc3"}' '{"id":"sq-ixc3.11"}' > "$st/realids.jsonl"
  beads_diff "$st/base.jsonl" "$st/realids.jsonl" >/dev/null 2>&1; check "$?" "0" "real bead id shapes must be accepted"

  echo ""
  echo "${PROG} self-test: ${p} passed, ${f} failed."
  [ "$f" -eq 0 ] || exit 1
  echo "${PROG} self-test: OK — resync-branch predicate, retention floor, order-insensitive diff, malformed/unsafe-id export refusal."
  exit 0
fi

# --- preconditions --------------------------------------------------------------------
ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || die "not inside a git repository"
cd "$ROOT" || die "cannot cd to $ROOT"

GIT_DIR_ABS="$(git rev-parse --absolute-git-dir 2>/dev/null)" || die "cannot resolve the git dir"
COMMON_DIR_ABS="$(cd "$(git rev-parse --git-common-dir)" && pwd)" || die "cannot resolve the git common dir"
if [ "$GIT_DIR_ABS" != "$COMMON_DIR_ABS" ]; then
  die "refusing to run from a linked worktree ($ROOT) — the beads DB and .beads/ belong to the main checkout (AGENTS.md: never touch .beads/ in a worktree)"
fi

command -v python3 >/dev/null 2>&1 || die "python3 not found (needed for the record diff)"
command -v bd >/dev/null 2>&1 || die "bd not found on PATH — this producer must run on the work box, where the bead DB lives"

# Single writer: serialise ticks against each other and against a hand-run export.
LOCK="${GIT_DIR_ABS}/${PROG}.lock"
exec 9>"$LOCK" || die "cannot open lock $LOCK"
if ! flock -w "$LOCK_WAIT_SECS" 9; then
  log "another ${PROG} holds the lock after ${LOCK_WAIT_SECS}s — skipping this tick (not an error)"
  exit 0
fi

TMP="$(mktemp -d)" || die "mktemp failed"
trap 'rm -rf "$TMP"' EXIT

# --- base ref -------------------------------------------------------------------------
if [ "$DO_FETCH" -eq 1 ]; then
  if ! git fetch --quiet origin main 2>"${TMP}/fetch.err"; then
    fetch_err="$(tr '\n' ' ' < "${TMP}/fetch.err" | cut -c1-200)"
    if [ "$MODE" = "apply" ]; then
      die "git fetch origin main failed (${fetch_err}) — refusing to --apply against a possibly stale base"
    fi
    log "WARNING: git fetch origin main failed (${fetch_err}); the dry-run report is against a possibly stale ${BASE_REF}"
  fi
fi
BASE_SHA="$(git rev-parse --verify "${BASE_REF}^{commit}" 2>/dev/null)" || die "cannot resolve base ref ${BASE_REF}"

# The base copy of the mirror. An absent path means "no mirror yet" (empty), not an error.
if git cat-file -e "${BASE_SHA}:${TRACKED_FILE}" 2>/dev/null; then
  git show "${BASE_SHA}:${TRACKED_FILE}" > "${TMP}/base.jsonl" || die "cannot read ${TRACKED_FILE} at ${BASE_REF}"
else
  : > "${TMP}/base.jsonl"
  log "note: ${TRACKED_FILE} does not exist at ${BASE_REF} — treating the base mirror as empty"
fi

# --- export ---------------------------------------------------------------------------
bd export > "${TMP}/export.jsonl" 2>"${TMP}/export.err"
BD_RC=$?
if [ "$BD_RC" -ne 0 ]; then
  die "bd export failed (exit ${BD_RC}): $(tr '\n' ' ' < "${TMP}/export.err" | cut -c1-300)"
fi
[ -s "${TMP}/export.jsonl" ] || die "bd export produced NO output — refusing to commit an empty mirror (this would delete every bead record)"

DIFF_OUT="$(beads_diff "${TMP}/base.jsonl" "${TMP}/export.jsonl" 2>"${TMP}/diff.err")"
DIFF_RC=$?
if [ "$DIFF_RC" -ne 0 ]; then
  die "the export is malformed, so it will NOT be committed: $(tr '\n' ' ' < "${TMP}/diff.err" | cut -c1-300)"
fi

records_base=0; records_export=0; added=""; removed=""; changed=""
while IFS='=' read -r key value; do
  case "$key" in
    records_base)   records_base="$value" ;;
    records_export) records_export="$value" ;;
    added)          added="$value" ;;
    removed)        removed="$value" ;;
    changed)        changed="$value" ;;
  esac
done <<EOF
$DIFF_OUT
EOF

count_ids() { [ -z "${1:-}" ] && { echo 0; return; }; printf '%s' "$1" | tr ',' '\n' | grep -c . ; }
n_added="$(count_ids "$added")"; n_removed="$(count_ids "$removed")"; n_changed="$(count_ids "$changed")"
n_drift=$(( n_added + n_removed + n_changed ))

if ! retain_ok "$records_export" "$records_base"; then
  die "bd export returned ${records_export} records against ${records_base} on ${BASE_REF} — below the ${MIN_RETAIN_PCT}% retention floor. Refusing: this looks like a truncated or partial DB read, not real churn."
fi

# --- report ---------------------------------------------------------------------------
BRANCH=""; PR_URL=""; APPLIED=0

# preview <comma-list>: the id list, truncated so a 1200-bead churn does not paste the
# whole tracker into a log line or a PR body.
preview() { printf '%s' "${1:-}" | cut -c1-200; }

# json_arr <comma-list>: the id list as a JSON array. Safe without JSON escaping ONLY
# because beads_diff already REFUSED any export whose ids leave the [A-Za-z0-9._-]
# token charset (see ID_OK) — that refusal is what makes this interpolation sound.
json_arr() {
  local out="" id
  while IFS= read -r id; do
    [ -n "$id" ] || continue
    out="${out}${out:+,}\"${id}\""
  done <<EOF
$(printf '%s' "${1:-}" | tr ',' '\n')
EOF
  printf '[%s]' "$out"
}

# bool <n>: the JSON literal for "n is non-zero".
bool() { if [ "${1:-0}" -ne 0 ]; then printf 'true'; else printf 'false'; fi; }

emit_report() {
  if [ "$JSON" -eq 1 ]; then
    printf '{"drift":%s,"records_base":%s,"records_export":%s,"added":%s,"removed":%s,"changed":%s,"applied":%s,"branch":"%s","pr":"%s","base":"%s"}\n' \
      "$(bool "$n_drift")" \
      "$records_base" "$records_export" \
      "$(json_arr "$added")" "$(json_arr "$removed")" "$(json_arr "$changed")" \
      "$(bool "$APPLIED")" \
      "$BRANCH" "$PR_URL" "$BASE_SHA"
  else
    log "base ${BASE_REF} (${BASE_SHA}): ${records_base} records; bd export: ${records_export} records"
    log "drift: ${n_added} added, ${n_removed} removed, ${n_changed} changed"
    [ "$n_added" -gt 0 ]   && log "  added:   $(preview "$added")"
    [ "$n_removed" -gt 0 ] && log "  removed: $(preview "$removed")"
    [ "$n_changed" -gt 0 ] && log "  changed: $(preview "$changed")"
    [ -n "$PR_URL" ] && log "opened: ${PR_URL}"
    return 0
  fi
}

if [ "$n_drift" -eq 0 ]; then
  log "${TRACKED_FILE} on ${BASE_REF} already matches \`bd export\` — nothing to push."
  emit_report
  exit 0
fi

if [ "$MODE" != "apply" ]; then
  emit_report
  log "dry-run: re-run with --apply to open the ${RESYNC_PREFIX}* PR."
  exit 0
fi

# --- apply ----------------------------------------------------------------------------
command -v gh >/dev/null 2>&1 || die "gh not found on PATH — needed to open the resync PR"

# One open resync PR at a time: without this the cron opens a PR every tick while the
# previous snapshot waits for the merge train.
open_heads="$(gh pr list --state open --limit 100 --json headRefName -q '.[].headRefName' 2>/dev/null)"
while IFS= read -r head; do
  [ -n "$head" ] || continue
  if is_resync_branch "$head"; then
    log "a resync PR is already open on branch '${head}' — skipping (it will pick up this drift when it is re-run after that PR merges)"
    emit_report
    exit 0
  fi
done <<EOF
$open_heads
EOF

BRANCH="${RESYNC_PREFIX}$(date -u +%Y%m%dT%H%M%SZ)"

# Build the commit with PLUMBING against a private index — the checkout, its index, its
# HEAD and its branch are all untouched (see the NON-DISRUPTIVE note in the header).
export GIT_INDEX_FILE="${TMP}/index"
git read-tree "$BASE_SHA" || die "git read-tree ${BASE_SHA} failed"
BLOB="$(git hash-object -w --path "$TRACKED_FILE" "${TMP}/export.jsonl")" || die "git hash-object failed"
git update-index --add --cacheinfo "100644,${BLOB},${TRACKED_FILE}" || die "git update-index failed"
TREE="$(git write-tree)" || die "git write-tree failed"
unset GIT_INDEX_FILE

if [ "$TREE" = "$(git rev-parse "${BASE_SHA}^{tree}")" ]; then
  # Belt-and-braces: the record diff said there was drift but the bytes are identical
  # (only possible if the drift was purely key-order, which beads_diff already ignores).
  log "the resulting tree equals ${BASE_REF} — nothing to push."
  emit_report
  exit 0
fi

SUBJECT="chore(beads): re-export ${TRACKED_FILE} (box-side snapshot)"
BODY_FILE="${TMP}/body.md"
{
  printf '> 🤖 SPARQ agent — automated box-side `bd export` snapshot (`scripts/%s`, issue #6088).\n\n' "${PROG}.sh"
  printf 'Regenerates the committed bead mirror from the work-box bead DB so beads that were never migrated to a GitHub issue (the `unmapped_local` class) are recoverable from git.\n\n'
  printf 'Base `%s` (`%s`): %s records. `bd export`: %s records.\n\n' "$BASE_REF" "$BASE_SHA" "$records_base" "$records_export"
  printf '| drift | count |\n|---|---|\n| added | %s |\n| removed | %s |\n| changed | %s |\n\n' "$n_added" "$n_removed" "$n_changed"
  [ "$n_added" -gt 0 ]   && printf 'Added: `%s`\n\n' "$(preview "$added")"
  [ "$n_removed" -gt 0 ] && printf 'Removed: `%s`\n\n' "$(preview "$removed")"
  [ "$n_changed" -gt 0 ] && printf 'Changed: `%s`\n\n' "$(preview "$changed")"
  printf 'Generated by a cron on the work box; only `%s` is touched.\n' "$TRACKED_FILE"
} > "$BODY_FILE"

COMMIT="$(git commit-tree "$TREE" -p "$BASE_SHA" -m "$SUBJECT" -m "Automated box-side bd export snapshot (issue #6088). Only ${TRACKED_FILE} changes.")" \
  || die "git commit-tree failed (is the box's git user.name/user.email configured?)"

git push --quiet origin "${COMMIT}:refs/heads/${BRANCH}" || die "git push of ${BRANCH} failed"

APPLIED=1   # the snapshot commit is pushed from here on, whether or not the PR opens.
if ! PR_URL="$(gh pr create --base main --head "$BRANCH" --title "$SUBJECT" --body-file "$BODY_FILE" 2>&1)"; then
  log "WARNING: branch ${BRANCH} was pushed but \`gh pr create\` failed: ${PR_URL}"
  log "the snapshot is NOT lost — open the PR by hand from ${BRANCH}."
  PR_URL=""
  emit_report
  exit 1
fi
emit_report
exit 0
