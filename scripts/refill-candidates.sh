#!/usr/bin/env bash
# Orchestration automation — Phase F of research/orchestration-automation-design.md
# (PR #374).
#
# refill-candidates.sh   (READ-ONLY; advisory only — NO mutation, NO dispatch)
#
# Print the `bd ready` beads grouped by crate/surface, and FLAG each surface that already
# has an open PR / in-flight worktree (contention). This is the SUBSTRATE for the
# orchestrator's refill DECISION (design §1.1 T5 / §5 Bead F), NOT the decision: choosing
# *which* bead to dispatch, writing the brief, and respecting the one-branch-per-contended-
# surface rule is JUDGMENT and stays with the orchestrator. This script only assembles the
# "ready, grouped, contention-flagged" candidate list so that judgment is well-informed.
#
# It is strictly read-only: it runs `bd ready`, `gh pr list`, and `git worktree list`, and
# prints. It NEVER closes/creates/dispatches anything.
#
# Contention is reserved by OPEN PR + worktree branches with UNPUSHED local commits only
# — NOT every git worktree branch. The harness never auto-removes a finished agent's
# worktree, so hundreds of stale branches pile up; counting all of them reserved EVERY
# crate and made the launchable frontier spuriously empty (sq-8rpq part 3). A pushed /
# squash-merged branch (unpushed=0) is ignored — note we use the UNPUSHED test, NOT
# "ancestor of origin/main": PRs are squash-merged, so a merged feature branch is NOT an
# ancestor of main yet WAS pushed, and an ancestor test would re-introduce the bug. The
# unpushed test mirrors scripts/worktree-gc.sh's "never-dropped" predicate.
#
# Surface derivation (best-effort, deterministic): a bead's surface is inferred from its
# title — a leading `crate:` / `crate(...)` / `[tag]` token, or any `sparq-<crate>` mention,
# matched against the actual crate list under crates/. Beads with no recognised surface are
# bucketed under "(unscoped)".
#
# Contention: a surface is "in flight" if an OPEN PR's head branch — or a local worktree
# branch that has UNPUSHED local commits — contains the surface token. Pushed / stale
# worktree branches are ignored (see above). This is heuristic (branch names are
# free-form) and advisory — a flag is a prompt to check, not a hard block.
#
# Run:
#   scripts/refill-candidates.sh                    # the candidate briefing
#   scripts/refill-candidates.sh --dry-run-self-test   # hermetic self-test (no bd/gh/git)
set -euo pipefail

PROG="refill-candidates"
log()  { printf '[%s] %s\n' "$PROG" "$*" >&2; }
die()  { printf '[%s] ERROR: %s\n' "$PROG" "$*" >&2; exit 1; }

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# --- surface inference (pure-text; the hermetic, testable core) ------------------------
# Given a bead TITLE on $1 and a space-separated list of known crate short-names on $2,
# echo the inferred surface token (e.g. "sparq-core", or a bracket/prefix tag), else "".
# Precedence: explicit sparq-<crate> mention > leading prefix:/[tag] > "".
infer_surface() {
  local title="$1" crates="$2" c tag
  # 1. an explicit sparq-<crate> mention anywhere in the title.
  for c in $crates; do
    case "$title" in
      *"$c"*) printf '%s\n' "$c"; return 0 ;;
    esac
  done
  # 2. a leading bracket tag: "[cert] ...", "[epic] ..." -> "cert" / "epic".
  case "$title" in
    \[*\]*)
      tag="${title#\[}"; tag="${tag%%\]*}"
      printf '%s\n' "$tag"; return 0 ;;
  esac
  # 3. a leading "prefix: ..." or "prefix(scope): ..." token (e.g. "site:", "SBOM:").
  case "$title" in
    *:*)
      tag="${title%%:*}"
      # strip a "(scope)" suffix and lowercase; reject if it looks like prose (has spaces).
      tag="${tag%%(*}"
      case "$tag" in
        *' '*) ;;                       # has a space -> it's prose, not a prefix tag.
        *) printf '%s\n' "$tag"; return 0 ;;
      esac ;;
  esac
  printf '\n'
}

# sq-8rpq part 3: contention by UNPUSHED worktree branches (not every branch).
# wt_contention_branch <branch> <unpushed-count> : echo <branch> iff it is a CONTENTION
# source, i.e. iff it has UNPUSHED local commits (fresh work, no PR yet). <unpushed-count>
# is the number of commits on HEAD absent from the push target; "0" = fully pushed (or
# squash-merged) => NOT contention; any non-"0" (incl. the 999999 never-pushed sentinel)
# => genuinely in-flight. The UNPUSHED test (not "ancestor of main") is load-bearing:
# PRs are squash-merged, so a finished feature branch is NOT an ancestor of main yet WAS
# pushed (unpushed=0); an ancestor/`--no-merged` test would flag all the hundreds of
# lingering `.claude/worktrees/` branches and reserve every crate, emptying the frontier
# (the sq-8rpq bug). Pure (no git): the count is injected, so this is hermetically
# testable. Mirrors scripts/worktree-gc.sh's `wt_unpushed_count` predicate.
wt_contention_branch() {
  local branch="$1" unpushed="$2"
  [ -n "$branch" ] || return 0
  [ "$unpushed" = "0" ] && return 0        # fully pushed / squash-merged -> not contention.
  printf '%s\n' "$branch"
}

# wt_unpushed_count <wt> <branch> : commits on HEAD absent from the push target (upstream,
# else origin/<branch>); 999999 sentinel if never pushed. Copied verbatim from
# scripts/worktree-gc.sh so all three scripts share one definition of "unpushed".
wt_unpushed_count() {
  local wt="$1" branch="$2" upstream target
  upstream="$(git -C "$wt" rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null)" || upstream=""
  if [ -n "$upstream" ] && git -C "$wt" rev-parse --verify -q "$upstream" >/dev/null 2>&1; then
    target="$upstream"
  elif [ -n "$branch" ] && git -C "$wt" rev-parse --verify -q "origin/$branch" >/dev/null 2>&1; then
    target="origin/$branch"
  else
    printf '999999'   # no push target ⇒ treat as unpushed ⇒ in flight.
    return 0
  fi
  git -C "$wt" rev-list --count "$target..HEAD" 2>/dev/null || printf '999999'
}

# --- self-test (no external calls) -----------------------------------------------------
self_test() {
  local fails=0 got CR
  CR="sparq-core sparq-server sparq-hdt sparq-zk sparq-mpc"
  _check() {
    if [ "$2" = "$3" ]; then printf '  ok   %s\n' "$1"
    else printf '  FAIL %s\n       got:  %q\n       want: %q\n' "$1" "$2" "$3"; fails=$((fails + 1)); fi
  }
  log "running --dry-run self-test (hermetic; no bd/gh/git calls)"

  got="$(infer_surface 'fix(sparq-core): dict spill (sq-glw2)' "$CR")"
  _check "explicit crate mention"                  "$got" "sparq-core"

  got="$(infer_surface '[cert][cryptoreview] external audit' "$CR")"
  _check "leading bracket tag"                     "$got" "cert"

  got="$(infer_surface 'site: browser smoke test for ZK' "$CR")"
  _check "leading prefix: tag"                     "$got" "site"

  got="$(infer_surface 'SBOM: dedicated npm SBOM (GS-3)' "$CR")"
  _check "uppercase prefix tag"                    "$got" "SBOM"

  got="$(infer_surface 'cert(sbom): purl canonicality' "$CR")"
  _check "prefix(scope): tag strips scope"         "$got" "cert"

  got="$(infer_surface 'Federation TPF follow-ups: brTPF' "$CR")"
  _check "prose-before-colon => unscoped"          "$got" ""

  got="$(infer_surface 'server hardening for sparq-server limits' "$CR")"
  _check "crate mention wins over prose"           "$got" "sparq-server"

  # --- worktree-branch contention filter (sq-8rpq part 3): a worktree branch is a
  #     contention source ONLY if it has UNPUSHED local commits. A pushed / squash-merged
  #     branch (unpushed=0) must NOT reserve its surface — otherwise the ~500 lingering
  #     worktree branches reserve EVERY crate and the frontier is spuriously empty. The
  #     unpushed COUNT is injected so this stays hermetic (no git calls).
  got="$(wt_contention_branch 'sq-glw2-dict-spill' 2)"
  _check "branch w/ unpushed commits IS contention" "$got" "sq-glw2-dict-spill"

  got="$(wt_contention_branch 'sq-never-pushed' 999999)"
  _check "never-pushed branch IS contention"        "$got" "sq-never-pushed"

  got="$(wt_contention_branch 'sq-squash-merged' 0)"
  _check "pushed/squash-merged branch NOT contention" "$got" ""

  echo
  if [ "$fails" -eq 0 ]; then log "self-test PASSED"; return 0; fi
  die "self-test FAILED ($fails check(s))"
}

# --- arg parsing -----------------------------------------------------------------------
for arg in "$@"; do
  case "$arg" in
    --dry-run-self-test) self_test; exit 0 ;;
    -h|--help)           sed -n '2,30p' "$0"; exit 0 ;;
    *)                   die "unknown argument: $arg (try --dry-run-self-test, --help)" ;;
  esac
done

command -v bd >/dev/null 2>&1 || die "bd not found (this script lists 'bd ready' beads)"

# Known crate short-names (from crates/), longest-first so e.g. sparq-zk-compose is tried
# before sparq-zk (a substring match would otherwise mis-bucket it).
CRATES=""
if [ -d "$ROOT/crates" ]; then
  for d in "$ROOT"/crates/*/; do
    [ -d "$d" ] || continue
    name="${d%/}"; name="${name##*/}"
    CRATES+="$name"$'\n'
  done
  CRATES="$(printf '%s' "$CRATES" | awk 'NF { print length, $0 }' | sort -rn | cut -d' ' -f2- | tr '\n' ' ')"
fi
[ -n "$CRATES" ] || log "WARNING: no crates/ dir found — surface inference will fall back to title tags only."

# --- 1. gather contention signals: open-PR branches + UNPUSHED worktree branches -------
# A surface is genuinely in flight iff it is touched by (a) an OPEN PR, or (b) a worktree
# branch with UNPUSHED local commits (work committed but not yet pushed/PR'd). A pushed
# branch (incl. one that was squash-merged) is NOT contention — the harness never
# auto-removes finished worktrees, so hundreds pile up; reserving on *every* worktree
# branch (the old behaviour) reserved every crate and left the frontier spuriously empty
# (sq-8rpq part 3). The unpushed test mirrors scripts/worktree-gc.sh.
CONTENTION_BRANCHES=""
if command -v gh >/dev/null 2>&1; then
  CONTENTION_BRANCHES="$(gh pr list --state open --limit 200 --json headRefName \
    -q '.[].headRefName' 2>/dev/null || true)"
else
  log "gh not found — open-PR contention flags will be omitted (unpushed worktrees still used)."
fi

# Walk each worktree; keep its branch only if it has UNPUSHED local commits (fresh work,
# no PR yet). A pushed / squash-merged branch (unpushed=0) is skipped. Worktrees with a
# detached HEAD (no branch line) are skipped automatically.
WT_BRANCHES=""
WT_PATH="" WT_BRANCH=""
while IFS= read -r line; do
  case "$line" in
    "worktree "*) WT_PATH="${line#worktree }"; WT_BRANCH="" ;;
    "branch refs/heads/"*)
      WT_BRANCH="${line#branch refs/heads/}"
      if [ -n "$WT_BRANCH" ] && [ -d "$WT_PATH" ]; then
        WT_BRANCHES+="$(wt_contention_branch "$WT_BRANCH" "$(wt_unpushed_count "$WT_PATH" "$WT_BRANCH")")"$'\n'
      fi
      ;;
  esac
done < <(git -C "$ROOT" worktree list --porcelain 2>/dev/null || true)

ALL_BRANCHES="$(printf '%s\n%s\n' "$CONTENTION_BRANCHES" "$WT_BRANCHES" | sed '/^$/d' | sort -u)"

# Is surface $1 contended (its token appears in any open-PR / unpushed-worktree branch name)?
is_contended() {
  local surface="$1"
  [ -n "$surface" ] || return 1
  printf '%s\n' "$ALL_BRANCHES" | grep -qiF -- "$surface"
}

# --- 2. read bd ready and group by inferred surface ------------------------------------
READY_JSON="$(bd ready --json 2>/dev/null)" || die "bd ready --json failed"

# Emit "surface<TAB>id<TAB>P<priority><TAB>title" lines via python (robust JSON parse).
ROWS="$(printf '%s' "$READY_JSON" | python3 -c '
import json,sys
data=json.load(sys.stdin)
for it in data:
    print("\t".join([
        it.get("id",""),
        str(it.get("priority","")),
        (it.get("title","") or "").replace("\t"," ").replace("\n"," "),
    ]))
' 2>/dev/null)" || die "could not parse bd ready --json"

[ -n "$ROWS" ] || { log "no ready beads — nothing to refill."; exit 0; }

# Build a temp grouping: surface => list of "id Pn title".
declare -A GROUP
declare -A GROUP_COUNT
total=0
while IFS=$'\t' read -r id prio title; do
  [ -n "$id" ] || continue
  surface="$(infer_surface "$title" "$CRATES")"
  [ -n "$surface" ] || surface="(unscoped)"
  GROUP["$surface"]+="    $id  P$prio  $title"$'\n'
  GROUP_COUNT["$surface"]=$(( ${GROUP_COUNT["$surface"]:-0} + 1 ))
  total=$((total + 1))
done <<< "$ROWS"

# --- 3. print the briefing (surfaces sorted; contended ones flagged) -------------------
echo "refill-candidates — $total ready bead(s), grouped by inferred surface (READ-ONLY, advisory)"
echo "  contention sources: $(printf '%s\n' "$ALL_BRANCHES" | sed '/^$/d' | wc -l | tr -d ' ') open-PR + unpushed-worktree branch name(s)"
echo

# List free (uncontended) surfaces first — those are the safest refill targets.
for pass in free contended; do
  for surface in $(printf '%s\n' "${!GROUP[@]}" | sort); do
    if is_contended "$surface"; then state="contended"; else state="free"; fi
    [ "$state" = "$pass" ] || continue
    if [ "$state" = "contended" ]; then
      printf '== %s  (%d ready)  [CONTENDED: open PR/worktree touches this surface — confirm before dispatch] ==\n' \
        "$surface" "${GROUP_COUNT[$surface]}"
    else
      printf '== %s  (%d ready)  [free] ==\n' "$surface" "${GROUP_COUNT[$surface]}"
    fi
    printf '%s' "${GROUP[$surface]}"
    echo
  done
done

echo "NOTE: advisory only. The orchestrator chooses WHICH beads to dispatch (judgment); this"
echo "      script does not dispatch, close, or mutate anything (design §1.1 T5)."
