#!/usr/bin/env bash
# [OPUS-5] 🤖 SPARQ agent — hermetic end-to-end tests for scripts/beads-export-push.sh
# (issue #6088: the box-side `bd export` producer for the committed bead mirror).
#
# WHY THIS HARNESS EXISTS. The script's own `--dry-run-self-test` covers the PURE
# predicates (the resync-branch matcher, the retention floor, the order-insensitive
# record diff). It cannot cover the part that is actually dangerous: this is a cron
# that PUSHES, so a regression could commit an empty/truncated mirror, disturb the
# orchestrator's live checkout, or open a PR every tick. Those are behaviours of the
# git-plumbing path, so they need a real repository to observe.
#
# HERMETIC: a real `git` against a LOCAL BARE remote (no network), with `bd` and `gh`
# PATH-shadowed by fixture-backed stubs. Everything lives in a mktemp sandbox removed
# on exit. The real bead tracker, the real repo and the real GitHub are never touched.
#
# The invariants pinned here — each is a way the producer could do damage:
#   1. NO DRIFT => no branch is pushed and nothing is created (the common case; a cron
#      that pushed a no-op commit per tick would flood the merge train).
#   2. DRY-RUN (the DEFAULT) reports the drift but pushes NOTHING.
#   3. --apply pushes ONE `chore-beads-resync-*` branch whose `.beads/issues.jsonl` is
#      byte-identical to the `bd export` output, and whose diff vs main touches ONLY
#      that file.
#   4. --apply DOES NOT DISTURB THE CHECKOUT — same HEAD, same branch, clean status,
#      untouched working-tree copy of the mirror. This is the load-bearing claim that
#      makes it safe to cron on the orchestrator's own main checkout.
#   5. An ALREADY-OPEN resync PR makes --apply a no-op (no PR-per-tick spam).
#   6. An EMPTY export is FATAL and pushes nothing (it would delete the whole mirror).
#   7. A TRUNCATED export (below the retention floor) is FATAL and pushes nothing.
#   8. A MALFORMED export is FATAL and pushes nothing.
#   9. A `bd export` that EXITS NON-ZERO is FATAL and pushes nothing.
#  10. A LINKED WORKTREE is refused (AGENTS.md: never touch .beads/ in a worktree).
#  11. `bd` absent is a loud failure, not a silent success.
#  12. --json emits the drift report a scheduler can read.
#  13. A FAILED FETCH is fatal under --apply (a stale base would open a redundant or
#      conflicting PR) but only a warning in dry-run.
#  14. LOCK CONTENTION skips the tick (exit 0, nothing pushed) instead of racing the
#      bead DB — and the skip is not sticky: the next free tick pushes.
#
# Run:  bash scripts/tests/test_beads_export_push.sh   (exit 0 = all pass, 1 = a failure)
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="${ROOT}/scripts/beads-export-push.sh"
[ -f "$SRC" ] || { echo "FATAL: script not found at ${SRC}"; exit 2; }

pass=0; fail=0
note_pass() { pass=$((pass + 1)); }
note_fail() { fail=$((fail + 1)); printf 'CASE FAILED: %s\n' "$1"; }
want_contains() { if printf '%s' "$2" | grep -qF -- "$3"; then note_pass; else note_fail "$1 (expected to CONTAIN '$3')"; fi; }
want_eq()       { if [ "$2" = "$3" ]; then note_pass; else note_fail "$1 (got '$2', want '$3')"; fi; }

SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT
BIN="${SANDBOX}/bin"
mkdir -p "$BIN"

BD_FIXTURE="${SANDBOX}/bd-export.jsonl"
BD_FAIL_FLAG="${SANDBOX}/bd-must-fail"
GH_OPEN_HEADS="${SANDBOX}/gh-open-heads.txt"
GH_PR_LOG="${SANDBOX}/gh-pr-create.log"

# --- stub `bd`: `bd export` emits the fixture. Honours a fail flag so case 9 can drive
#     the "exporter exited non-zero" path without a malformed fixture.
cat >"${BIN}/bd" <<EOF
#!/usr/bin/env bash
if [ "\${1:-}" = "export" ]; then
  if [ -f "${BD_FAIL_FLAG}" ]; then echo "bd: dolt read failed" >&2; exit 1; fi
  cat "${BD_FIXTURE}"
  exit 0
fi
exit 0
EOF

# --- stub `gh`: `pr list` serves the open-head fixture; `pr create` is logged and
#     returns a URL. Never touches the network.
cat >"${BIN}/gh" <<EOF
#!/usr/bin/env bash
case "\${2:-}" in
  list)   cat "${GH_OPEN_HEADS}" ;;
  create) printf '%s\n' "\$*" >> "${GH_PR_LOG}"; echo "https://github.com/jeswr/sparq/pull/9999" ;;
  *)      : ;;
esac
exit 0
EOF
chmod +x "${BIN}/bd" "${BIN}/gh"

# --------------------------------------------------------------------------- #
# A real git repo with a local BARE "origin" — the plumbing path is under test, so
# git itself is NOT stubbed.
# --------------------------------------------------------------------------- #
ORIGIN="${SANDBOX}/origin.git"
WORK="${SANDBOX}/work"

BASE_JSONL='{"id":"sq-a","title":"A","status":"open"}
{"id":"sq-b","title":"B","status":"open"}
{"id":"sq-c","title":"C","status":"open"}
{"id":"sq-d","title":"D","status":"open"}'

setup_repo() {
  rm -rf "$ORIGIN" "$WORK"
  git init --quiet --bare "$ORIGIN"
  git init --quiet "$WORK"
  (
    cd "$WORK" || exit 1
    git config user.email "test@example.invalid"
    git config user.name "sparq test"
    git config commit.gpgsign false
    git symbolic-ref HEAD refs/heads/main
    mkdir -p .beads
    printf '%s\n' "$BASE_JSONL" > .beads/issues.jsonl
    echo "# sandbox" > README.md
    git add .beads/issues.jsonl README.md
    git commit --quiet -m "base"
    git remote add origin "$ORIGIN"
    git push --quiet origin main
  )
}

# run <args...>: drive the script from the sandbox checkout with the stubs on PATH.
# Captures stdout+stderr into $out and the exit code into $rc.
out=""; rc=0
run() {
  out="$(cd "$WORK" && PATH="${BIN}:${PATH}" bash "$SRC" "$@" 2>&1)"
  rc=$?
}

# resync_branches: the resync branches that exist on the REMOTE (the only place this
# script is allowed to create anything).
resync_branches() { git --git-dir="$ORIGIN" for-each-ref --format='%(refname:short)' refs/heads/ | grep '^chore-beads-resync-' ; }
resync_count() { resync_branches | grep -c . ; }

# --------------------------------------------------------------------------- #
# 1. NO DRIFT => nothing pushed.
# --------------------------------------------------------------------------- #
setup_repo
printf '%s\n' "$BASE_JSONL" > "$BD_FIXTURE"
: > "$GH_OPEN_HEADS"; : > "$GH_PR_LOG"
run --apply
want_eq "no-drift: exit 0" "$rc" "0"
want_contains "no-drift: says it is in sync" "$out" "already matches"
want_eq "no-drift: pushed no branch" "$(resync_count)" "0"
want_eq "no-drift: opened no PR" "$(grep -c . "$GH_PR_LOG")" "0"

# Key-order churn inside a record is NOT drift. The script does not assume bd's key
# order is stable; if this regressed, the cron would open a PR every tick forever.
printf '%s\n' \
  '{"status":"open","title":"A","id":"sq-a"}' \
  '{"title":"B","id":"sq-b","status":"open"}' \
  '{"id":"sq-c","title":"C","status":"open"}' \
  '{"id":"sq-d","title":"D","status":"open"}' > "$BD_FIXTURE"
run --apply
want_contains "key-order churn is not drift" "$out" "already matches"
want_eq "key-order churn: pushed no branch" "$(resync_count)" "0"

# Record ORDER churn is not drift either (the committed mirror is not id-sorted, so
# bd's record order is its own and is not relied on).
printf '%s\n' \
  '{"id":"sq-d","title":"D","status":"open"}' \
  '{"id":"sq-b","title":"B","status":"open"}' \
  '{"id":"sq-a","title":"A","status":"open"}' \
  '{"id":"sq-c","title":"C","status":"open"}' > "$BD_FIXTURE"
run --apply
want_contains "record-order churn is not drift" "$out" "already matches"
want_eq "record-order churn: pushed no branch" "$(resync_count)" "0"

# --------------------------------------------------------------------------- #
# 2. DRY-RUN (the default) reports drift and pushes NOTHING.
# --------------------------------------------------------------------------- #
setup_repo
DRIFT_JSONL='{"id":"sq-a","title":"A","status":"open"}
{"id":"sq-b","title":"B","status":"closed"}
{"id":"sq-d","title":"D","status":"open"}
{"id":"sq-e","title":"E — created on the box, never migrated","status":"open"}'
printf '%s\n' "$DRIFT_JSONL" > "$BD_FIXTURE"
: > "$GH_OPEN_HEADS"; : > "$GH_PR_LOG"
run
want_eq "dry-run: exit 0" "$rc" "0"
want_contains "dry-run: reports the added bead" "$out" "sq-e"
want_contains "dry-run: reports the removed bead" "$out" "sq-c"
want_contains "dry-run: reports the changed bead" "$out" "sq-b"
want_contains "dry-run: 1 added, 1 removed, 1 changed" "$out" "1 added, 1 removed, 1 changed"
want_contains "dry-run: points at --apply" "$out" "--apply"
want_eq "dry-run: pushed no branch" "$(resync_count)" "0"
want_eq "dry-run: opened no PR" "$(grep -c . "$GH_PR_LOG")" "0"

# 12. --json shape for a scheduler.
run --json
want_contains "json: drift flag" "$out" '"drift":true'
want_contains "json: added array" "$out" '"added":["sq-e"]'
want_contains "json: removed array" "$out" '"removed":["sq-c"]'
want_contains "json: changed array" "$out" '"changed":["sq-b"]'
want_contains "json: not applied in dry-run" "$out" '"applied":false'
if printf '%s' "$out" | grep -o '{.*}' | python3 -c 'import json,sys; json.loads(sys.stdin.read())' 2>/dev/null; then
  note_pass; else note_fail "json: --json output does not parse as JSON"; fi

# --------------------------------------------------------------------------- #
# 3 + 4. --apply pushes exactly one snapshot branch, with the right content, and
#        DOES NOT DISTURB THE CHECKOUT.
# --------------------------------------------------------------------------- #
head_before="$(git -C "$WORK" rev-parse HEAD)"
branch_before="$(git -C "$WORK" rev-parse --abbrev-ref HEAD)"
mirror_before="$(md5sum < "${WORK}/.beads/issues.jsonl")"
run --apply
want_eq "apply: exit 0" "$rc" "0"
want_eq "apply: pushed exactly one resync branch" "$(resync_count)" "1"
BR="$(resync_branches | head -1)"
want_contains "apply: branch name carries the snapshot prefix" "$BR" "chore-beads-resync-"
want_contains "apply: opened the PR" "$(cat "$GH_PR_LOG")" "chore(beads): re-export .beads/issues.jsonl"
want_contains "apply: reports the PR url" "$out" "pull/9999"

# The pushed blob is byte-identical to what `bd export` emitted.
pushed="$(git --git-dir="$ORIGIN" show "${BR}:.beads/issues.jsonl")"
want_eq "apply: pushed mirror == the bd export bytes" "$pushed" "$(cat "$BD_FIXTURE")"
# ...and NOTHING else changed relative to main.
changed_paths="$(git --git-dir="$ORIGIN" diff --name-only main "$BR" | tr '\n' ' ')"
want_eq "apply: only the mirror changed" "$changed_paths" ".beads/issues.jsonl "
# ...on top of main, not a detached/unrelated history.
want_eq "apply: parent is main" "$(git --git-dir="$ORIGIN" rev-parse "${BR}^")" "$(git --git-dir="$ORIGIN" rev-parse main)"

# 4. The live checkout is untouched: same HEAD, same branch, clean tree, and the
#    working-tree mirror is byte-for-byte what it was.
want_eq "apply: HEAD unmoved" "$(git -C "$WORK" rev-parse HEAD)" "$head_before"
want_eq "apply: branch unchanged" "$(git -C "$WORK" rev-parse --abbrev-ref HEAD)" "$branch_before"
want_eq "apply: working tree still clean" "$(git -C "$WORK" status --porcelain | grep -c .)" "0"
want_eq "apply: working-tree mirror untouched" "$(md5sum < "${WORK}/.beads/issues.jsonl")" "$mirror_before"
want_eq "apply: created no local branch" "$(git -C "$WORK" for-each-ref --format='%(refname:short)' refs/heads/ | grep -c '^chore-beads-resync-')" "0"

# --------------------------------------------------------------------------- #
# 5. An already-open resync PR makes --apply a no-op (no PR-per-tick spam).
# --------------------------------------------------------------------------- #
setup_repo
printf '%s\n' "$DRIFT_JSONL" > "$BD_FIXTURE"
: > "$GH_PR_LOG"
printf '%s\n' "chore-beads-resync-20260101T000000Z" > "$GH_OPEN_HEADS"
run --apply
want_eq "open-PR guard: exit 0" "$rc" "0"
want_contains "open-PR guard: says why it skipped" "$out" "already open"
want_eq "open-PR guard: pushed no branch" "$(resync_count)" "0"
want_eq "open-PR guard: opened no PR" "$(grep -c . "$GH_PR_LOG")" "0"

# An UNRELATED open chore PR must NOT stop the producer (the predicate must be exact).
printf '%s\n' "chore-something-else" "fix/sq-2xdg-beads" > "$GH_OPEN_HEADS"
run --apply
want_eq "unrelated open PR: still pushes" "$(resync_count)" "1"

# --------------------------------------------------------------------------- #
# 6-9. Destructive-export fail-safes: each is FATAL and pushes NOTHING.
# --------------------------------------------------------------------------- #
: > "$GH_OPEN_HEADS"

# 6. EMPTY export would delete the entire mirror.
setup_repo; : > "$GH_PR_LOG"
: > "$BD_FIXTURE"
run --apply
want_eq "empty export: fatal" "$rc" "1"
want_contains "empty export: names the danger" "$out" "empty mirror"
want_eq "empty export: pushed no branch" "$(resync_count)" "0"

# 7. TRUNCATED export (1 of 4 records = 25%, below the 50% floor).
setup_repo; : > "$GH_PR_LOG"
printf '%s\n' '{"id":"sq-a","title":"A","status":"open"}' > "$BD_FIXTURE"
run --apply
want_eq "truncated export: fatal" "$rc" "1"
want_contains "truncated export: names the retention floor" "$out" "retention floor"
want_eq "truncated export: pushed no branch" "$(resync_count)" "0"

# ...but a legitimate shrink AT the floor (2 of 4 = 50%) is allowed through.
setup_repo; : > "$GH_PR_LOG"
printf '%s\n' '{"id":"sq-a","title":"A","status":"open"}' '{"id":"sq-b","title":"B","status":"open"}' > "$BD_FIXTURE"
run --apply
want_eq "at-floor shrink: allowed" "$rc" "0"
want_eq "at-floor shrink: pushed the snapshot" "$(resync_count)" "1"

# 8. MALFORMED export.
setup_repo; : > "$GH_PR_LOG"
printf '%s\n' '{"id":"sq-a","title":"A"' '{"id":"sq-b"}' '{"id":"sq-c"}' '{"id":"sq-d"}' > "$BD_FIXTURE"
run --apply
want_eq "malformed export: fatal" "$rc" "1"
want_contains "malformed export: says it will not commit" "$out" "malformed"
want_eq "malformed export: pushed no branch" "$(resync_count)" "0"

# 9. `bd export` EXITS NON-ZERO (a Dolt read error) — must not be mistaken for "no beads".
setup_repo; : > "$GH_PR_LOG"
printf '%s\n' "$DRIFT_JSONL" > "$BD_FIXTURE"
touch "$BD_FAIL_FLAG"
run --apply
rm -f "$BD_FAIL_FLAG"
want_eq "bd export failure: fatal" "$rc" "1"
want_contains "bd export failure: reported" "$out" "bd export failed"
want_eq "bd export failure: pushed no branch" "$(resync_count)" "0"

# --------------------------------------------------------------------------- #
# 10. A LINKED WORKTREE is refused (.beads/ in a worktree is the merge-conflict footgun).
# --------------------------------------------------------------------------- #
setup_repo
printf '%s\n' "$DRIFT_JSONL" > "$BD_FIXTURE"
: > "$GH_PR_LOG"
WT="${SANDBOX}/wt"
git -C "$WORK" worktree add --quiet -b side "$WT" >/dev/null 2>&1
out="$(cd "$WT" && PATH="${BIN}:${PATH}" bash "$SRC" --apply 2>&1)"; rc=$?
want_eq "worktree: refused" "$rc" "1"
want_contains "worktree: says why" "$out" "linked worktree"
want_eq "worktree: pushed no branch" "$(resync_count)" "0"
git -C "$WORK" worktree remove --force "$WT" >/dev/null 2>&1

# --------------------------------------------------------------------------- #
# 11. `bd` absent is LOUD, not a silent success (a silent no-op is exactly the
#     invisible-beads failure this producer exists to end).
# --------------------------------------------------------------------------- #
EMPTYBIN="${SANDBOX}/emptybin"; mkdir -p "$EMPTYBIN"
cp "${BIN}/gh" "${EMPTYBIN}/gh"
out="$(cd "$WORK" && PATH="${EMPTYBIN}:/usr/bin:/bin" bash "$SRC" 2>&1)"; rc=$?
want_eq "bd absent: fatal" "$rc" "1"
want_contains "bd absent: names bd" "$out" "bd not found"

# --------------------------------------------------------------------------- #
# 13. A FAILED FETCH is fatal under --apply (a stale base would open a redundant or
#     conflicting PR) but only a WARNING in dry-run (the report is advisory).
# --------------------------------------------------------------------------- #
setup_repo
printf '%s\n' "$DRIFT_JSONL" > "$BD_FIXTURE"
: > "$GH_PR_LOG"; : > "$GH_OPEN_HEADS"
git -C "$WORK" remote set-url origin "${SANDBOX}/does-not-exist.git"
run --apply
want_eq "fetch failure: fatal under --apply" "$rc" "1"
want_contains "fetch failure: names the stale-base risk" "$out" "stale base"
want_eq "fetch failure: pushed no branch" "$(resync_count)" "0"
run
want_eq "fetch failure: only a warning in dry-run" "$rc" "0"
want_contains "fetch failure: warns in dry-run" "$out" "WARNING"
want_contains "fetch failure: still reports the drift" "$out" "sq-e"
git -C "$WORK" remote set-url origin "$ORIGIN"

# --------------------------------------------------------------------------- #
# 14. LOCK CONTENTION: a second tick overlapping the first SKIPS rather than racing
#     the bead DB — and skipping is exit 0, not a cron-alerting failure.
# --------------------------------------------------------------------------- #
setup_repo
printf '%s\n' "$DRIFT_JSONL" > "$BD_FIXTURE"
: > "$GH_PR_LOG"
LOCKFILE="$(git -C "$WORK" rev-parse --absolute-git-dir)/beads-export-push.lock"
touch "$LOCKFILE"
flock "$LOCKFILE" sleep 3 &
holder=$!
sleep 0.5
out="$(cd "$WORK" && PATH="${BIN}:${PATH}" BEADS_EXPORT_LOCK_WAIT=1 bash "$SRC" --apply 2>&1)"; rc=$?
want_eq "lock contention: exit 0 (a skipped tick is not an error)" "$rc" "0"
want_contains "lock contention: says it skipped" "$out" "skipping this tick"
want_eq "lock contention: pushed no branch" "$(resync_count)" "0"
wait "$holder" 2>/dev/null
# ...and once the lock is free the very next tick DOES push (the skip is not sticky).
run --apply
want_eq "lock released: the next tick pushes" "$(resync_count)" "1"

# --------------------------------------------------------------------------- #
# The in-script pure-predicate self-test still passes (contract integrity).
# --------------------------------------------------------------------------- #
if PATH="${BIN}:${PATH}" bash "$SRC" --dry-run-self-test >/dev/null 2>&1; then note_pass; else note_fail "in-script --dry-run-self-test failed"; fi

# STATIC: the load-bearing fail-safes are present in the source (a refactor cannot
# silently drop one — several of them have no observable effect until the day they fire).
want_src() { if grep -q "$2" "$SRC"; then note_pass; else note_fail "$1"; fi; }
want_src "retention floor removed from source"        'MIN_RETAIN_PCT'
want_src "resync-branch predicate removed from source" 'is_resync_branch'
want_src "worktree refusal removed from source"        'linked worktree'
want_src "single-writer lock removed from source"      'flock -w'
want_src "empty-export refusal removed from source"    'refusing to commit an empty mirror'
want_src "plumbing (non-disruptive) commit path removed" 'git commit-tree'

# --------------------------------------------------------------------------- #
echo ""
echo "test_beads_export_push: ${pass} passed, ${fail} failed."
[ "$fail" -eq 0 ] || exit 1
echo "test_beads_export_push: OK — no-drift/order-churn no-op, dry-run pushes nothing, --apply snapshots one branch without disturbing the checkout, open-PR guard, and every destructive-export path fails closed."
