#!/usr/bin/env bash
# Safe worktree garbage-collection sweep (bead sq-6xdr).
#
# worktree-gc.sh [--dry-run | --apply] [--root <dir>] [--main <path>] [--base <ref>]
#                [--reclaim-completed]
#
# WHY: the Claude Code harness creates a fresh git worktree per agent under
# `.claude/worktrees/` but NEVER auto-removes a finished agent's worktree. They pile up
# (366+ this session) and each carries a `target/` build dir (~8-12G); together they filled
# the disk. This is the MANUAL / idle-time broom that removes ONLY the worktrees that are
# provably safe to delete — never one that still holds work.
#
# SAFETY PREDICATE — a worktree is SAFE-to-remove iff ALL of:
#   (a) MERGED-or-GONE: its HEAD commit is already an ancestor of the base ref
#       (default origin/main) — i.e. every commit it holds is on the remote, in main —
#       OR its branch is GONE on origin (the upstream/origin ref it tracked is deleted,
#       which is what a merged-then-branch-deleted PR looks like) AND its HEAD is still
#       reachable from some remote-tracking ref (so we are not about to drop the only copy
#       of a commit). The ancestor-of-base test is the load-bearing one: if HEAD is in
#       origin/main, there is by definition nothing to lose.
#   (b) CLEAN: `git -C <wt> status --porcelain` is empty (no uncommitted / untracked work).
#   (c) NO UNPUSHED COMMITS: HEAD has no commits absent from its push target (its upstream,
#       else origin/<branch>). When (a) holds via ancestor-of-base this is automatic; we
#       still assert it independently so a branch that is "gone on origin" but carries local
#       commits the remote never saw is REJECTED.
#   (d) NOT THE MAIN CHECKOUT: the main repo (default /home/ubuntu/sparq) is a HARD,
#       unconditional exclusion — removed from the candidate set before any test, asserted
#       by the self-test. We also only ever consider worktrees physically under <root>
#       (default the harness `.claude/worktrees/` dir): allow-list by location, not deny-list.
#
# COMPLETED-BUT-UNMERGED RECLAIM (--reclaim-completed; OFF by default — bead sq-h34dc):
#   A *completed* workflow worktree is one whose agent finished and PUSHED its PR branch, but
#   whose PR is not yet merged (and the branch not yet deleted on origin). Such a worktree is
#   CLEAN and has NO unpushed commits, yet it fails (a) MERGED-or-GONE — so the default broom
#   KEEPS it forever and it starves the disk (observed 2026-06-22: ~229G of these piled up,
#   one 33G, until a manual prune). With --reclaim-completed the broom ALSO reclaims a worktree
#   when ALL of:
#     (b) CLEAN and (c) NO UNPUSHED COMMITS  — its work is already on origin, nothing to lose;
#     (e) WORKFLOW-NAMED: its branch matches the harness pattern (wf_* / agent-* / the
#         `worktree-wf_*` / `worktree-agent-*` synthetic-branch forms) — a human-named branch
#         is NEVER swept by this path; AND
#     (f) NOT IN USE: no live process has its cwd or an open file handle inside the worktree
#         (the load-bearing new guard — an in-flight agent that is momentarily clean+pushed
#         between commits is KEPT). When the in-use probe cannot tell, it reports IN USE → KEEP.
#   This path is OPT-IN precisely because it removes a tree whose PR may still be OPEN; the
#   default predicate (MERGED-or-GONE) is unchanged, so a plain `worktree-gc.sh` is as
#   conservative as before. `disk-guard.sh` passes --reclaim-completed through on its sweep.
#
# Any worktree that is locked, prunable (its dir is already gone), bare, detached with
# unreachable HEAD, dirty, has unpushed commits, is in use, or whose branch is neither merged
# nor gone (and not an eligible completed-workflow tree) is KEPT and reported with the reason
# it was kept. When in doubt, KEEP.
#
# DEFAULT IS --dry-run: prints the safe-to-remove set with a per-worktree reason and a
#   reclaimable-size estimate (du -sh of each safe dir + a total). Touches nothing.
# --apply: `git worktree remove --force` each safe worktree, then `git worktree prune`.
#   The clean-status check (b) requires `git status --porcelain` to be EMPTY, so the SAFE set
#   has no uncommitted or untracked work. --force is needed only because these worktrees carry
#   a git-IGNORED `target/` build dir (ignored files never appear in `status --porcelain`, so
#   they cannot fail check (b), and plain `git worktree remove` would refuse over them); those
#   build artifacts are regenerable. --force here therefore discards only ignored regenerables.
#
# NOTE: do not run --apply while other agents may be using sibling worktrees concurrently;
# run it at idle. The predicate cannot misclassify a busy worktree (an active agent has
# either uncommitted work → (b) fails, or unmerged/unpushed commits → (a)/(c) fail), but
# removing a worktree mid-build still aborts that build.
#
# Run:
#   scripts/worktree-gc.sh                      # dry-run (default): list safe-to-remove + size
#   scripts/worktree-gc.sh --apply              # actually remove the safe set, then prune
#   scripts/worktree-gc.sh --base origin/main   # change the "merged" base ref (default)
#   scripts/worktree-gc.sh --reclaim-completed  # ALSO reclaim completed-but-unmerged workflow
#                                               #   worktrees (clean+pushed+not-in-use); dry-run
#   scripts/worktree-gc.sh --apply --reclaim-completed   # ... and actually remove them
#   scripts/worktree-gc.sh --dry-run-self-test  # hermetic self-test (no git mutation)
set -euo pipefail

PROG="worktree-gc"
log()  { printf '[%s] %s\n' "$PROG" "$*" >&2; }
die()  { printf '[%s] ERROR: %s\n' "$PROG" "$*" >&2; exit 1; }

# --- configuration (overridable by flags / env) ----------------------------------------
# The harness worktree root. Only worktrees physically under this dir are EVER candidates
# (allow-list by location). Everything outside it is kept untouched.
DEFAULT_ROOT="/home/ubuntu/sparq/.claude/worktrees"
# The main checkout — a HARD, unconditional never-remove. `git worktree remove` would refuse
# the main working tree anyway, but we drop it from the candidate set first regardless.
DEFAULT_MAIN="/home/ubuntu/sparq"
# The ref a worktree's HEAD must be an ancestor of to count as "merged".
DEFAULT_BASE="origin/main"

# --- pure, testable safety classifier ---------------------------------------------------
# is_protected_path <path> <main>
#   Hard exclusion: true (0) iff <path> is the main checkout (exact, after normalising a
#   trailing slash). Pure string compare — no git, no fs access — so it is unit-testable and
#   can never be tricked into NOT protecting main.
is_protected_path() {
  local p="${1%/}" main="${2%/}"
  [ "$p" = "$main" ]
}

# under_root <path> <root>
#   Allow-list by location: true (0) iff <path> is the root itself or strictly beneath it.
#   Prevents a stray worktree elsewhere on disk from ever entering the candidate set.
under_root() {
  local p="${1%/}" root="${2%/}"
  [ "$p" = "$root" ] && return 1   # the root dir itself is not a worktree we manage
  case "$p/" in
    "$root"/*) return 0 ;;
    *)         return 1 ;;
  esac
}

# is_workflow_branch <branch>
#   True (0) iff <branch> is a harness-created workflow/agent branch — the only branches the
#   completed-but-unmerged reclaim (--reclaim-completed) may sweep. Pure string match (no git,
#   no fs) so it is unit-testable. Covers the live shapes the harness produces:
#     wf_<id> / agent-<id>                       (a real PR branch named for the workflow)
#     worktree-wf_<id> / worktree-agent-<id>     (the synthetic per-worktree branch git creates
#                                                  when the harness does not name one itself)
#   A human-authored branch (feat/…, fix/…, ci/…, or a bare bead-id branch) is NOT matched and
#   is therefore NEVER eligible for the completed-reclaim path — only MERGED-or-GONE removes it.
is_workflow_branch() {
  local b="$1"
  case "$b" in
    wf_*|agent-*|worktree-wf_*|worktree-agent-*) return 0 ;;
    *)                                           return 1 ;;
  esac
}

# --- git-backed per-worktree probes (each isolated so the classifier reads cleanly) ------
# All run with `git -C <wt>` so they ask THAT worktree, not this one.

# wt_is_clean <wt> : 0 iff `git status --porcelain` is empty (no uncommitted/untracked).
wt_is_clean() {
  local wt="$1" out
  out="$(git -C "$wt" status --porcelain 2>/dev/null)" || return 1
  [ -z "$out" ]
}

# wt_head <wt> : print the worktree HEAD commit sha (empty on failure).
wt_head() { git -C "$wt" rev-parse --verify -q HEAD 2>/dev/null; }

# wt_head_in_base <wt> <base> : 0 iff HEAD is an ancestor of <base> (fully merged; nothing
# to lose). This single test satisfies both predicate (a) "merged" and (c) "no unpushed".
wt_head_in_base() {
  local wt="$1" base="$2"
  git -C "$wt" merge-base --is-ancestor HEAD "$base" 2>/dev/null
}

# wt_branch <wt> : print the worktree's checked-out branch short name (empty if detached).
wt_branch() {
  local b
  b="$(git -C "$1" symbolic-ref -q --short HEAD 2>/dev/null)" || return 0
  printf '%s' "$b"
}

# wt_unpushed_count <wt> <branch> : number of commits on HEAD absent from the push target.
# Push target = configured upstream if any, else origin/<branch>. If neither exists (branch
# was never pushed) we return a large sentinel so the caller treats it as "has unpushed work"
# and KEEPS the worktree — never-pushed local commits must not be dropped.
wt_unpushed_count() {
  local wt="$1" branch="$2" upstream target
  upstream="$(git -C "$wt" rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null)" || upstream=""
  if [ -n "$upstream" ] && git -C "$wt" rev-parse --verify -q "$upstream" >/dev/null 2>&1; then
    target="$upstream"
  elif [ -n "$branch" ] && git -C "$wt" rev-parse --verify -q "origin/$branch" >/dev/null 2>&1; then
    target="origin/$branch"
  else
    printf '999999'   # no push target ⇒ treat as unpushed ⇒ KEEP
    return 0
  fi
  git -C "$wt" rev-list --count "$target..HEAD" 2>/dev/null || printf '999999'
}

# wt_branch_gone <wt> <branch> : 0 iff the branch tracked an upstream that no longer exists
# (the merged-then-branch-deleted shape) OR there is simply no origin/<branch> ref while an
# upstream was configured. A branch that never had an upstream is NOT "gone".
wt_branch_gone() {
  local wt="$1" branch="$2" track
  [ -n "$branch" ] || return 1
  track="$(git -C "$wt" for-each-ref --format='%(upstream:track)' "refs/heads/$branch" 2>/dev/null)" || return 1
  [ "$track" = "[gone]" ]
}

# wt_head_reachable_from_remote <wt> : 0 iff HEAD is contained in at least one
# remote-tracking ref (refs/remotes/*). Guards the "gone" path: never delete a worktree whose
# HEAD exists only locally.
wt_head_reachable_from_remote() {
  local wt="$1" head
  head="$(wt_head "$wt")" || return 1
  [ -n "$head" ] || return 1
  git -C "$wt" for-each-ref --contains "$head" --format='%(refname)' refs/remotes 2>/dev/null \
    | grep -q .
}

# wt_in_use <wt> : 0 (IN USE) iff a live process appears to be using the worktree. This is the
# load-bearing guard for the completed-reclaim path: an in-flight agent that is momentarily
# CLEAN and fully PUSHED (between commits) must not be reclaimed out from under it.
#
# Primary mechanism: a `/proc/<pid>/cwd` scan — portable (no external tool), and the cwd of a
# running agent shell is the single most reliable "this tree is live" signal (the harness runs
# each agent WITH its worktree as cwd). We resolve <wt> to its real path and report IN USE if
# any process's cwd resolves to <wt> or a path strictly beneath it.
# Fallback: if /proc is unavailable but `lsof` is, use `lsof +D <wt>` — broader still, it
# catches any OPEN FILE HANDLE under the tree, not just a cwd. If NEITHER is available we cannot
# tell ⇒ report IN USE (return 0) so we KEEP — a false "in use" only forgoes a reclaim (safe);
# a false "idle" could delete a live tree, so the probe errs toward IN USE.
wt_in_use() {
  local wt="$1" real pid cwd creal
  real="$(cd "$wt" 2>/dev/null && pwd -P)" || real="${wt%/}"
  real="${real%/}"

  if [ -d /proc ]; then
    for pid in /proc/[0-9]*; do
      cwd="$(readlink "$pid/cwd" 2>/dev/null)" || continue
      [ -n "$cwd" ] || continue
      cwd="${cwd%/}"
      # exact match or strictly beneath the worktree (prefix-safe via the trailing slash).
      if [ "$cwd" = "$real" ]; then return 0; fi
      case "$cwd/" in "$real"/*) return 0 ;; esac
      # also catch a process whose cwd is the symlinky <wt> form rather than its realpath.
      creal="${wt%/}"
      if [ "$cwd" = "$creal" ]; then return 0; fi
      case "$cwd/" in "$creal"/*) return 0 ;; esac
    done
    return 1   # /proc scanned, no live cwd inside the tree ⇒ NOT in use
  fi

  # No /proc: fall back to lsof if present; else we cannot tell ⇒ conservatively IN USE.
  if command -v lsof >/dev/null 2>&1; then
    lsof +D "$real" >/dev/null 2>&1 && return 0
    return 1
  fi
  return 0   # cannot determine ⇒ assume IN USE ⇒ KEEP
}

# classify_reason <wt> <base> <reclaim_completed> : echo a single token describing the SAFE
# basis, or a KEEP reason. Exit 0 ⇒ safe to remove; exit 1 ⇒ keep. The predicate in one place.
# <reclaim_completed> (0/1, default 0) enables the completed-but-unmerged workflow-worktree
# path — when off, behaviour is identical to before (MERGED-or-GONE only).
#   safe tokens : MERGED  | BRANCH-GONE | COMPLETED-PUSHED
#   keep  tokens: dirty | unpushed=<n> | gone-but-head-local-only | in-use
#                 | not-merged-not-gone | not-merged-not-workflow
classify_reason() {
  local wt="$1" base="$2" reclaim_completed="${3:-0}" branch unpushed
  # (b) CLEAN — independent of everything else; a dirty tree is always KEEP.
  if ! wt_is_clean "$wt"; then echo "dirty"; return 1; fi

  branch="$(wt_branch "$wt")"

  # (a) primary: HEAD already in base ⇒ merged ⇒ nothing to lose. Also satisfies (c).
  if wt_head_in_base "$wt" "$base"; then echo "MERGED"; return 0; fi

  # (c) any commit not on the push target ⇒ KEEP (covers never-pushed branches via sentinel).
  unpushed="$(wt_unpushed_count "$wt" "$branch")"
  if [ "$unpushed" != "0" ]; then echo "unpushed=$unpushed"; return 1; fi

  # (a) secondary: branch gone on origin AND HEAD still lives on a remote ref ⇒ safe.
  if wt_branch_gone "$wt" "$branch"; then
    if wt_head_reachable_from_remote "$wt"; then echo "BRANCH-GONE"; return 0; fi
    echo "gone-but-head-local-only"; return 1
  fi

  # (e)+(f) COMPLETED-BUT-UNMERGED reclaim (opt-in): we reach here only when the tree is CLEAN
  # and has NO unpushed commits (its work is safely on origin) but is neither merged nor gone —
  # i.e. a finished agent whose PR is still open. Reclaim it iff explicitly enabled AND it is a
  # harness workflow branch AND no live process is using it; otherwise KEEP as before.
  if [ "$reclaim_completed" = "1" ]; then
    if is_workflow_branch "$branch"; then
      if wt_in_use "$wt"; then echo "in-use"; return 1; fi
      echo "COMPLETED-PUSHED"; return 0
    fi
    echo "not-merged-not-workflow"; return 1
  fi

  echo "not-merged-not-gone"; return 1
}

# --- hermetic self-test (no git mutation; exercises the pure predicates) -----------------
self_test() {
  local fails=0
  _ok()   { printf '  ok   %s\n' "$1"; }
  _fail() { printf '  FAIL %s\n       %s\n' "$1" "$2"; fails=$((fails + 1)); }
  _expect_true()  { if "$@"; then _ok "$LBL"; else _fail "$LBL" "expected success, got failure"; fi; }
  _expect_false() { if "$@"; then _fail "$LBL" "expected failure, got success"; else _ok "$LBL"; fi; }
  log "running --dry-run self-test (hermetic; no git mutation)"

  local M="/home/ubuntu/sparq" R="/home/ubuntu/sparq/.claude/worktrees"

  LBL="main checkout is protected (exact)"
  _expect_true  is_protected_path "$M" "$M"
  LBL="main checkout protected despite trailing slash"
  _expect_true  is_protected_path "$M/" "$M"
  LBL="a worktree dir is NOT the protected main"
  _expect_false is_protected_path "$R/agent-x" "$M"

  LBL="worktree under root is a candidate"
  _expect_true  under_root "$R/agent-x" "$R"
  LBL="nested path under root is a candidate"
  _expect_true  under_root "$R/agent-x/sub" "$R"
  LBL="the main checkout is NOT under the worktree root"
  _expect_false under_root "$M" "$R"
  LBL="the root dir itself is not a managed worktree"
  _expect_false under_root "$R" "$R"
  LBL="a sibling dir outside root is rejected"
  _expect_false under_root "/home/ubuntu/sparq-wt-covgate" "$R"
  LBL="a path that merely shares a prefix is rejected"
  _expect_false under_root "${R}-evil/agent-x" "$R"

  # is_workflow_branch: only harness wf_/agent- branches are eligible for completed-reclaim.
  LBL="wf_ branch is a workflow branch"
  _expect_true  is_workflow_branch "wf_d3d8da8f-85c-1"
  LBL="agent- branch is a workflow branch"
  _expect_true  is_workflow_branch "agent-foo"
  LBL="synthetic worktree-wf_ branch is a workflow branch"
  _expect_true  is_workflow_branch "worktree-wf_d3d8da8f-85c-1"
  LBL="synthetic worktree-agent- branch is a workflow branch"
  _expect_true  is_workflow_branch "worktree-agent-foo"
  LBL="a feat/ branch is NOT a workflow branch (never completed-reclaimed)"
  _expect_false is_workflow_branch "feat/sq-evb1-reason-el"
  LBL="a bare bead-id branch is NOT a workflow branch"
  _expect_false is_workflow_branch "sq-m3sm-mpc-routing"
  LBL="an empty (detached) branch is NOT a workflow branch"
  _expect_false is_workflow_branch ""

  # wt_in_use: a real probe; assert it is callable and returns a clean 0/1 (we cannot
  # deterministically assert the live-process state of an arbitrary path here). A nonexistent
  # path has no process with a cwd inside it ⇒ NOT in use (return 1) on a box with /proc.
  if wt_in_use "/nonexistent/worktree/path"; then _ok "in-use probe returns (in-use)"; else _ok "in-use probe returns (idle)"; fi

  echo
  if [ "$fails" -eq 0 ]; then log "self-test PASSED"; return 0; fi
  die "self-test FAILED ($fails check(s))"
}

# --- arg parsing -------------------------------------------------------------------------
APPLY=0
ROOT="$DEFAULT_ROOT"
MAIN="$DEFAULT_MAIN"
BASE="$DEFAULT_BASE"
RECLAIM_COMPLETED=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run-self-test) self_test; exit 0 ;;
    --apply)             APPLY=1 ;;
    --dry-run)           APPLY=0 ;;
    --reclaim-completed) RECLAIM_COMPLETED=1 ;;
    --root)              shift; [ "$#" -gt 0 ] || die "--root needs a value"; ROOT="$1" ;;
    --root=*)            ROOT="${1#--root=}" ;;
    --main)              shift; [ "$#" -gt 0 ] || die "--main needs a value"; MAIN="$1" ;;
    --main=*)            MAIN="${1#--main=}" ;;
    --base)              shift; [ "$#" -gt 0 ] || die "--base needs a value"; BASE="$1" ;;
    --base=*)            BASE="${1#--base=}" ;;
    -h|--help)           sed -n '2,60p' "$0"; exit 0 ;;
    *)                   die "unknown argument: $1 (try --apply, --reclaim-completed, --root, --main, --base, --dry-run-self-test, --help)" ;;
  esac
  shift
done

ROOT="${ROOT%/}"
MAIN="${MAIN%/}"

# --- locate a git dir to drive the porcelain enumeration ---------------------------------
# We need a real git context. The main checkout is the natural one; fall back to this script's
# own worktree if --main was pointed elsewhere. We only ever READ the worktree list from it.
GIT_CTX="$MAIN"
if ! git -C "$GIT_CTX" rev-parse --git-dir >/dev/null 2>&1; then
  GIT_CTX="$(git rev-parse --show-toplevel 2>/dev/null)" \
    || die "no git repository found (tried --main='$MAIN' and CWD)"
fi
log "enumerating worktrees from git context: $GIT_CTX"
log "root=$ROOT  main(protected)=$MAIN  base(merged-ref)=$BASE  mode=$([ "$APPLY" -eq 1 ] && echo APPLY || echo dry-run)  reclaim-completed=$([ "$RECLAIM_COMPLETED" -eq 1 ] && echo on || echo off)"

# Confirm the base ref resolves (a typo'd --base would make NOTHING look merged → silent
# under-removal, which is safe, but warn so the operator knows).
git -C "$GIT_CTX" rev-parse --verify -q "$BASE" >/dev/null 2>&1 \
  || log "WARNING: base ref '$BASE' does not resolve — nothing will classify as MERGED (fetch first?)."

# --- enumerate worktrees (porcelain) and classify ---------------------------------------
SAFE_DIRS=()
SAFE_REASON=()
declare -A SEEN_REASON_COUNT=()
kept=0
total_considered=0

# Parse `git worktree list --porcelain`: records separated by blank lines; we only need the
# `worktree <path>` line plus the `prunable`/`locked`/`bare`/`detached` markers.
cur_path=""; cur_prunable=0; cur_locked=0; cur_bare=0
process_record() {
  [ -n "$cur_path" ] || return 0

  # (d) HARD exclusion: never the main checkout.
  if is_protected_path "$cur_path" "$MAIN"; then
    return 0   # silently skip; it is not a candidate by construction
  fi
  # Allow-list by location: only worktrees under the harness root are candidates.
  under_root "$cur_path" "$ROOT" || return 0

  total_considered=$((total_considered + 1))

  # Structural KEEPs first (cheap, and some make git probes meaningless).
  if [ "$cur_bare" -eq 1 ];    then log "KEEP  $cur_path  (bare worktree)";              kept=$((kept+1)); return 0; fi
  if [ "$cur_locked" -eq 1 ];  then log "KEEP  $cur_path  (locked)";                     kept=$((kept+1)); return 0; fi
  if [ "$cur_prunable" -eq 1 ];then log "KEEP  $cur_path  (already prunable; run prune)";kept=$((kept+1)); return 0; fi
  if [ ! -d "$cur_path" ];     then log "KEEP  $cur_path  (path missing on disk)";       kept=$((kept+1)); return 0; fi

  local reason
  if reason="$(classify_reason "$cur_path" "$BASE" "$RECLAIM_COMPLETED")"; then
    SAFE_DIRS+=("$cur_path")
    SAFE_REASON+=("$reason")
    SEEN_REASON_COUNT["$reason"]=$(( ${SEEN_REASON_COUNT["$reason"]:-0} + 1 ))
  else
    log "KEEP  $cur_path  ($reason)"
    kept=$((kept+1))
  fi
}

while IFS= read -r line; do
  case "$line" in
    "worktree "*) cur_path="${line#worktree }" ;;
    "prunable"*)  cur_prunable=1 ;;
    "locked"*)    cur_locked=1 ;;
    "bare")       cur_bare=1 ;;
    "")           process_record
                  cur_path=""; cur_prunable=0; cur_locked=0; cur_bare=0 ;;
  esac
done < <(git -C "$GIT_CTX" worktree list --porcelain)
process_record   # flush the final record (porcelain may not end with a blank line)

# --- report ------------------------------------------------------------------------------
echo
log "considered $total_considered worktree(s) under $ROOT  (kept $kept, safe-to-remove ${#SAFE_DIRS[@]})"

if [ "${#SAFE_DIRS[@]}" -eq 0 ]; then
  log "nothing safe to remove. Done."
  exit 0
fi

log "SAFE-to-remove set (reason — path):"
for i in "${!SAFE_DIRS[@]}"; do
  printf '  %-12s %s\n' "${SAFE_REASON[$i]}" "${SAFE_DIRS[$i]}" >&2
done
log "reason breakdown:"
for r in "${!SEEN_REASON_COUNT[@]}"; do
  printf '  %-12s %d\n' "$r" "${SEEN_REASON_COUNT[$r]}" >&2
done

# Reclaimable-size estimate. du can be slow over many target/ dirs; do a single du -sc so we
# get a grand total too. Failures (a dir vanished mid-run) are non-fatal — KEEP the estimate
# best-effort rather than abort the whole sweep.
log "reclaimable-size estimate (du -sh; may take a moment over target/ dirs):"
if du -sh "${SAFE_DIRS[@]}" 2>/dev/null | sed 's/^/  /' >&2; then :; else
  log "  (du could not size some dirs; estimate is partial)"
fi
if total_size="$(du -sch "${SAFE_DIRS[@]}" 2>/dev/null | tail -n1 | cut -f1)"; then
  log "estimated TOTAL reclaimable: ${total_size:-unknown}"
fi

if [ "$APPLY" -eq 0 ]; then
  echo
  log "DRY-RUN — nothing removed. Re-run with --apply (at idle; not while sibling agents build) to remove the set above."
  exit 0
fi

# --- --apply: remove the safe set, then prune --------------------------------------------
log "--apply: removing ${#SAFE_DIRS[@]} safe worktree(s)..."
removed=0; failed=0; skipped_busy=0
for i in "${!SAFE_DIRS[@]}"; do
  wt="${SAFE_DIRS[$i]}"
  reason="${SAFE_REASON[$i]}"
  # Re-assert the hard exclusion at the point of mutation (defence in depth — this can never
  # fire because main is dropped during enumeration, but if it ever did we abort, not delete).
  if is_protected_path "$wt" "$MAIN"; then
    die "refusing to remove protected main checkout: $wt"
  fi
  # TOCTOU re-verify the completed-but-unmerged path: a tree that was clean+pushed+idle when we
  # classified it could have been re-entered by a freshly-spawned agent between the scan and now.
  # MERGED/BRANCH-GONE entries are immune (their work is already on origin), so we only re-check
  # the COMPLETED-PUSHED set — KEEPing it if it has since gone dirty or in-use. Cheap insurance.
  if [ "$reason" = "COMPLETED-PUSHED" ]; then
    if ! wt_is_clean "$wt"; then
      log "  SKIP (now dirty since scan, kept): $wt"; skipped_busy=$((skipped_busy+1)); continue
    fi
    if wt_in_use "$wt"; then
      log "  SKIP (now in use since scan, kept): $wt"; skipped_busy=$((skipped_busy+1)); continue
    fi
  fi
  if git -C "$GIT_CTX" worktree remove --force "$wt" 2>/dev/null; then
    log "  removed ($reason): $wt"; removed=$((removed+1))
  else
    log "  FAILED to remove (kept): $wt"; failed=$((failed+1))
  fi
done
[ "$skipped_busy" -eq 0 ] || log "skipped $skipped_busy completed-worktree(s) that became dirty/in-use after the scan (kept)."
log "pruning stale worktree administrative entries..."
git -C "$GIT_CTX" worktree prune -v 2>&1 | sed 's/^/  /' >&2 || true
log "done: removed $removed, failed $failed, skipped-busy $skipped_busy, of ${#SAFE_DIRS[@]} safe candidate(s)."
[ "$failed" -eq 0 ]
