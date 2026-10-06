#!/usr/bin/env bash
# Hermetic end-to-end self-test for scripts/worktree-gc.sh's COMPLETED-but-unmerged
# workflow-worktree reclaim (--reclaim-completed — bead sq-h34dc).
#
# WHY: the completed-reclaim path removes a worktree whose PR branch is PUSHED but NOT yet merged
# — strictly more aggressive than the default MERGED-or-GONE broom. Its safety rests on three
# load-bearing invariants that a refactor must never silently break, so we pin them with a
# hermetic harness that builds REAL git repos (a bare "origin" + a clone with worktrees) and
# drives the actual `worktree-gc.sh`:
#   1. OPT-IN — without --reclaim-completed a completed-but-unmerged workflow worktree is KEPT
#      (default behaviour is unchanged: MERGED-or-GONE only).
#   2. WORKFLOW-NAMED ONLY — with --reclaim-completed a clean+pushed wf_/agent- worktree is
#      reclaimed, but a clean+pushed *human-named* (feat/…) worktree is KEPT — only the harness
#      naming pattern is eligible.
#   3. KEEP-IF-UNSAFE — dirty, unpushed, and (the new guard) IN-USE worktrees are KEPT even with
#      --reclaim-completed. The in-use case is exercised by spawning a live process whose cwd is
#      inside the worktree and asserting --apply does NOT remove it.
#   Plus: MERGED still classifies as MERGED regardless of the flag, and --apply actually removes
#   the reclaimed set (so the disk is genuinely freed).
#
# HERMETIC w.r.t. the real system: everything lives under a mktemp sandbox; the harness creates
# its own bare origin + clone + worktrees and never reads or touches the real repo, the real
# `.claude/worktrees/`, or any real process. It removes nothing outside the sandbox.
#
# Run:  bash scripts/tests/test_worktree_gc_completed.sh   (exit 0 = all pass, 1 = a failure)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GC="${ROOT}/scripts/worktree-gc.sh"
[ -f "$GC" ] || { echo "FATAL: script not found at ${GC}"; exit 2; }

pass=0
fail=0
note_pass() { pass=$((pass + 1)); }
note_fail() { fail=$((fail + 1)); printf 'CASE FAILED: %s\n' "$1"; }

SANDBOX="$(mktemp -d)"
cleanup() {
  # kill any lingering in-use stub process, then drop the sandbox.
  if [ -n "${INUSE_PID:-}" ]; then kill "$INUSE_PID" 2>/dev/null || true; fi
  rm -rf "$SANDBOX"
}
trap cleanup EXIT

ORIGIN="${SANDBOX}/origin.git"
CLONE="${SANDBOX}/clone"
WTROOT="${CLONE}/.claude/worktrees"   # mimic the real layout: worktrees under the clone

export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t

# --- build a bare origin with a `main`, then a clone tracking it --------------------------
git init -q --bare "$ORIGIN"
git clone -q "$ORIGIN" "$CLONE" 2>/dev/null   # ignore the benign "empty repository" notice
( cd "$CLONE"
  git checkout -q -b main 2>/dev/null || git checkout -q main
  echo seed > seed.txt; git add seed.txt; git commit -qm seed
  git push -q origin main
)
mkdir -p "$WTROOT"

# add_wt <name> <branch> : create a worktree on a NEW branch off main.
add_wt() { git -C "$CLONE" worktree add -q -b "$2" "${WTROOT}/$1" main; }

# commit_in <name> <msg> : add a commit inside a worktree.
commit_in() { ( cd "${WTROOT}/$1" && echo "$2" >> file.txt && git add file.txt && git commit -qm "$2" ); }

# push_wt <name> <branch> : push the worktree branch to origin (so it has NO unpushed commits).
push_wt() { git -C "${WTROOT}/$1" push -q -u origin "$2"; }

# run the broom; capture combined output into a global (rc is not asserted here).
run_gc() {  # run_gc <args...>
  set +e
  GC_OUT="$(bash "$GC" --main "$CLONE" --root "$WTROOT" --base origin/main "$@" 2>&1)"
  set -e
}
# helper assertions over the last run's output
out_has()    { printf '%s\n' "$GC_OUT" | grep -qF "$1"; }
wt_exists()  { [ -d "${WTROOT}/$1" ]; }

# --------------------------------------------------------------------------- #
# Fixtures (each on its own NEW branch off main):
#   completed-wf   : wf_-named, 1 pushed commit, clean        → COMPLETED-PUSHED (eligible)
#   completed-human: feat/-named, 1 pushed commit, clean      → not-merged-not-workflow (KEEP)
#   dirty-wf       : wf_-named, pushed commit + untracked file → dirty (KEEP)
#   unpushed-wf    : wf_-named, 1 LOCAL-only commit            → unpushed=1 (KEEP)
#   merged-wf      : wf_-named, commit merged into origin/main → MERGED (always safe)
# --------------------------------------------------------------------------- #
add_wt completed-wf    wf_completed-001
commit_in completed-wf "work"; push_wt completed-wf wf_completed-001

add_wt completed-human feat/human-feature
commit_in completed-human "work"; push_wt completed-human feat/human-feature

add_wt dirty-wf        wf_dirty-002
commit_in dirty-wf "work"; push_wt dirty-wf wf_dirty-002
echo scratch > "${WTROOT}/dirty-wf/untracked.tmp"      # make it dirty (untracked)

add_wt unpushed-wf     wf_unpushed-003
commit_in unpushed-wf "local-only"                      # committed but NEVER pushed

add_wt merged-wf       wf_merged-004
commit_in merged-wf "to-merge"; push_wt merged-wf wf_merged-004
# merge that branch into origin/main so its HEAD becomes an ancestor of the base ref. The clone's
# own `main` worktree is at $CLONE; merge there (fast-forward) and push, then refresh the
# remote-tracking origin/main so the broom's `HEAD is-ancestor origin/main` test sees the merge.
git -C "$CLONE" checkout -q main
git -C "$CLONE" merge -q --no-edit wf_merged-004
git -C "$CLONE" push -q origin main
git -C "$CLONE" fetch -q origin

# --------------------------------------------------------------------------- #
# 1. WITHOUT --reclaim-completed: completed-wf is KEPT (not-merged-not-gone); default unchanged.
# --------------------------------------------------------------------------- #
run_gc --dry-run
if out_has "not-merged-not-gone" && out_has "$WTROOT/completed-wf"; then note_pass; else
  note_fail "default (no flag): completed-wf should be KEPT as not-merged-not-gone"; fi
# and it must NOT appear as a safe COMPLETED-PUSHED removal.
if printf '%s\n' "$GC_OUT" | grep -q 'COMPLETED-PUSHED'; then
  note_fail "default (no flag): COMPLETED-PUSHED leaked without --reclaim-completed"; else note_pass; fi

# --------------------------------------------------------------------------- #
# 2. WITH --reclaim-completed (dry-run): completed-wf is COMPLETED-PUSHED (safe to remove)…
# --------------------------------------------------------------------------- #
run_gc --dry-run --reclaim-completed
if printf '%s\n' "$GC_OUT" | grep -F "$WTROOT/completed-wf" | grep -q 'COMPLETED-PUSHED'; then note_pass; else
  note_fail "reclaim-completed: completed-wf should be COMPLETED-PUSHED"; fi
# …the human-named completed worktree is NOT (workflow-named only)…
if out_has "not-merged-not-workflow" && out_has "$WTROOT/completed-human"; then note_pass; else
  note_fail "reclaim-completed: feat/-named completed worktree should be not-merged-not-workflow (KEEP)"; fi
# …dirty-wf is KEPT (dirty)…
if printf '%s\n' "$GC_OUT" | grep -F "$WTROOT/dirty-wf" | grep -q 'dirty'; then note_pass; else
  note_fail "reclaim-completed: dirty-wf should be KEPT (dirty)"; fi
# …unpushed-wf is KEPT (unpushed)…
if printf '%s\n' "$GC_OUT" | grep -F "$WTROOT/unpushed-wf" | grep -q 'unpushed='; then note_pass; else
  note_fail "reclaim-completed: unpushed-wf should be KEPT (unpushed=…)"; fi
# …merged-wf is MERGED (always safe, flag-independent).
if printf '%s\n' "$GC_OUT" | grep -F "$WTROOT/merged-wf" | grep -q 'MERGED'; then note_pass; else
  note_fail "reclaim-completed: merged-wf should be MERGED"; fi

# --------------------------------------------------------------------------- #
# 3. IN-USE guard: spawn a live process whose cwd is inside completed-wf, then --apply with
#    --reclaim-completed. The tree MUST be KEPT (in use), and dirty/unpushed/human also kept.
# --------------------------------------------------------------------------- #
( cd "${WTROOT}/completed-wf" && exec sleep 30 ) &
INUSE_PID=$!
# give the kernel a moment to publish the child's cwd in /proc (poll, no fixed sleep).
for _ in 1 2 3 4 5 6 7 8 9 10; do
  cwd="$(readlink "/proc/${INUSE_PID}/cwd" 2>/dev/null || true)"
  case "$cwd" in *"/completed-wf") break ;; esac
done
run_gc --apply --reclaim-completed
# completed-wf still exists (kept: in use) …
if wt_exists completed-wf; then note_pass; else note_fail "IN-USE: completed-wf removed despite a live process inside it"; fi
# … and the broom logged it as in-use somewhere (SKIP or in-use keep).
if printf '%s\n' "$GC_OUT" | grep -qiE 'in.?use'; then note_pass; else
  note_fail "IN-USE: broom did not report completed-wf as in-use"; fi
kill "$INUSE_PID" 2>/dev/null || true; wait "$INUSE_PID" 2>/dev/null || true; INUSE_PID=""

# the human / dirty / unpushed trees must also have survived the --apply.
if wt_exists completed-human; then note_pass; else note_fail "--apply: human-named worktree wrongly removed"; fi
if wt_exists dirty-wf;        then note_pass; else note_fail "--apply: dirty worktree wrongly removed"; fi
if wt_exists unpushed-wf;     then note_pass; else note_fail "--apply: unpushed worktree wrongly removed"; fi
# the MERGED worktree IS removed by --apply (it was always safe).
if wt_exists merged-wf; then note_fail "--apply: MERGED worktree was NOT removed"; else note_pass; fi

# --------------------------------------------------------------------------- #
# 4. APPLY actually reclaims the now-idle completed worktree (disk genuinely freed).
# --------------------------------------------------------------------------- #
run_gc --apply --reclaim-completed
if wt_exists completed-wf; then note_fail "--apply (idle): completed-wf was NOT reclaimed"; else note_pass; fi

# --------------------------------------------------------------------------- #
# 5. STATIC: the load-bearing predicates are present in source (a refactor can't silently drop).
# --------------------------------------------------------------------------- #
if grep -q 'is_workflow_branch' "$GC"; then note_pass; else note_fail "is_workflow_branch removed from source"; fi
if grep -q 'wt_in_use'          "$GC"; then note_pass; else note_fail "wt_in_use guard removed from source"; fi
if grep -q 'COMPLETED-PUSHED'   "$GC"; then note_pass; else note_fail "COMPLETED-PUSHED token removed from source"; fi
# the main checkout must never be a candidate — its self-test still passes.
if bash "$GC" --dry-run-self-test >/dev/null 2>&1; then note_pass; else note_fail "worktree-gc.sh pure self-test failed"; fi

# --------------------------------------------------------------------------- #
echo ""
echo "test_worktree_gc_completed: ${pass} passed, ${fail} failed."
[ "$fail" -eq 0 ] || exit 1
echo "test_worktree_gc_completed: OK — opt-in + workflow-named-only + keep-if-dirty/unpushed/in-use hold."
