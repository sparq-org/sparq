#!/usr/bin/env bash
# Orchestration automation — the push SCHEDULER (decision layer).
# Bead sq-o09o. Design: research/orchestration-automation-design.md §8.
#
# push-frontier.sh   (READ-ONLY; NO mutation, NO dispatch — fast, deterministic)
#
# Prints the beads that are SAFE TO LAUNCH NOW — the set the orchestrator can feed
# straight into a refill->verify fan-out without creating a conflict or exceeding
# the build-farm CPU ceiling. It is the *decision* layer that sits on top of the
# advisory `refill-candidates.sh` substrate (which only groups + flags contention).
#
# The launchable frontier is computed as:
#
#     bd ready                                    (the real unlock-frontier; deps
#                                                  must be accurate — see sq-o09o
#                                                  deliverable 2 / `bd dep`)
#   MINUS  in-flight beads                        (already has an OPEN PR — i.e. work
#                                                  is in progress; do not re-dispatch)
#   MINUS  conflict-collisions                    (emit at most ONE bead per crate /
#                                                  surface; serialise the hot lanes:
#                                                  the site, and the sparq-server
#                                                  http.rs auth path — <=1 in flight.
#                                                  A bead is reserved on TWO lanes: its
#                                                  label SURFACE *and* its primary-CODE
#                                                  crate — the crate its touched .rs
#                                                  paths land in (sq-6ip4) — so two beads
#                                                  editing one crate never co-launch even
#                                                  when their labels diverge)
#   MINUS  needs:user / needs:maintainer beads      (LABELLED gated-on-a-human-action: publish a
#                                                  license, flip a Pages setting, file an upstream
#                                                  issue. Not agent-launchable, so never in the
#                                                  autonomous frontier. bd ready returns them, so
#                                                  filter on the needs: label here — sq-p7nw.)
#   MINUS  umbrellas (epics + PARENTS-with-ready-children)
#                                                 (issue_type=epic, OR a "[epic]"-tagged title,
#                                                  OR any bead that is the PARENT of >=1 OTHER
#                                                  ready bead (a decomposed feature/task whose
#                                                  sq-<id>.N children are the real atomic work).
#                                                  An umbrella is a container, not a work unit —
#                                                  you dispatch its CHILDREN, never "the umbrella"
#                                                  — so it is excluded and its ready children are
#                                                  surfaced in its place. sq-p7nw: feature-typed
#                                                  parents (sq-bif, sq-5o5, ...) used to leak in
#                                                  here and starve their children of slots.)
#   MINUS  HELD-scope beads                         (a maintainer can HOLD an entire impl
#                                                  lane behind one open "design"/research PR;
#                                                  while #514 (the signed typed-value design)
#                                                  is open, the maintainer holds "#514 + ANY
#                                                  sparq-zk value-lane impl" — so EVERY
#                                                  sparq-zk / sparq-zk-compose value-lane bead
#                                                  is reserved, not just #514's research/*.md
#                                                  surface. The scheduler reserves by OPEN PR
#                                                  and the design PR's surface is just "research",
#                                                  which left the zk CRATES free and let a held
#                                                  bead (sq-mslu/#637) launch — sq-751l. See
#                                                  is_zk_value_lane + the HELD table below.)
#   CAPPED at the CPU ceiling                     (min(16, nproc-2); the build farm
#                                                  cannot usefully run more parallel
#                                                  cargo builds than it has cores).
#
# Output: one line per launchable bead — "<id>  <surface>  <title>" — ordered by
# bead priority (P0 first) then id, ready to hand to the orchestrator. Anything not
# emitted is either in-flight, would collide with an already-emitted/in-flight
# surface, or fell past the CPU-ceiling cap.
#
# HONEST design notes on the in-flight signal:
#   * The AUTHORITATIVE in-flight signal is the set of OPEN PRs (`gh pr list`). A bead
#     whose id appears in an open PR's head-branch name or title is in flight.
#   * `git branch -r` is NOT a reliable in-flight signal in this repo: PRs are
#     squash-merged, so a merged feature branch does NOT become an ancestor of main
#     and `git branch -r --no-merged origin/main` flags ~all 370+ stale branches as
#     "unmerged". Using that would subtract almost everything. We therefore treat a
#     remote branch as in-flight ONLY when it also backs an OPEN PR (so it collapses
#     to the PR signal); stale merged branches are correctly ignored. This is called
#     out so a future reader does not "fix" it by trusting `--no-merged`.
#
# Conflict-partition surfaces (one in-flight at a time):
#   * Each crate short-name under crates/ is a surface (>=1 launched per crate).
#   * "site"            — the Next.js site/ tree; serialise to <=1.
#   * "server-auth"     — the sparq-server http.rs auth path (the hot rebase seam);
#                         serialise to <=1 ON TOP OF the per-crate sparq-server cap,
#                         so an auth-path bead and another sparq-server bead never
#                         both launch (they share the crate cap anyway, but the
#                         auth-path lane is named explicitly per the brief).
#
# Run:
#   scripts/push-frontier.sh                       # the launchable frontier
#   scripts/push-frontier.sh --explain             # + a per-bead reason it was kept/dropped (stderr)
#   scripts/push-frontier.sh --dry-run-self-test   # hermetic self-test (no bd/gh/git)
set -euo pipefail

PROG="push-frontier"
log()  { printf '[%s] %s\n' "$PROG" "$*" >&2; }
die()  { printf '[%s] ERROR: %s\n' "$PROG" "$*" >&2; exit 1; }

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXPLAIN=0

# Ensure the user-local bd is reachable even under a bare PATH (cron/hook context).
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) PATH="$PATH:$HOME/.local/bin" ;;
esac

# --- surface inference (pure-text; the hermetic, testable core) ------------------------
# Given a bead TITLE on $1 and a space-separated list of known crate short-names on $2,
# echo the inferred CONFLICT SURFACE for partitioning. Precedence:
#   1. the sparq-server http.rs AUTH path        -> "server-auth"  (hot rebase seam)
#   2. an explicit sparq-<crate> mention          -> "<crate>"
#   3. the front-end site (site:/ "site"/"page:")  -> "site"
#   4. a leading bracket tag "[tag] ..."           -> "tag"
#   5. a leading "prefix: ..."/"prefix(scope): .." -> "prefix"
#   6. otherwise                                   -> ""  (unscoped; never collides)
infer_surface() {
  local title="$1" crates="$2" c tag lc
  lc="$(printf '%s' "$title" | tr '[:upper:]' '[:lower:]')"

  # 1. The sparq-server auth path — serialise hardest. A bead is on this lane if it
  #    mentions sparq-server (or "server") AND an auth/authz/http.rs token.
  case "$lc" in
    *server*|*http.rs*)
      case "$lc" in
        *auth*|*http.rs*|*bearer*|*wac*|*acl*|*acp*)
          # but only if it's genuinely the server (not e.g. sparq-solid ACP modelling)
          case "$lc" in
            *sparq-server*|*http.rs*|*" server "*|"server "*|*" server:"*)
              printf 'server-auth\n'; return 0 ;;
          esac ;;
      esac ;;
  esac

  # 2. an explicit sparq-<crate> mention anywhere in the title (longest-first list).
  for c in $crates; do
    case "$title" in
      *"$c"*) printf '%s\n' "$c"; return 0 ;;
    esac
  done

  # 3. the front-end site surface.
  case "$lc" in
    site:*|"site "*|*" site:"*|page:*|"page "*|*/surface/*|*"site/src"*|*"tier-"*"wasm"*)
      printf 'site\n'; return 0 ;;
  esac

  # 4. a leading bracket tag: "[cert] ...", "[epic] ..." -> "cert" / "epic".
  case "$title" in
    \[*\]*)
      tag="${title#\[}"; tag="${tag%%\]*}"
      printf '%s\n' "$tag"; return 0 ;;
  esac

  # 5. a leading "prefix: ..." or "prefix(scope): ..." token (e.g. "SBOM:", "deps:").
  case "$title" in
    *:*)
      tag="${title%%:*}"
      tag="${tag%%(*}"                 # strip a "(scope)" suffix.
      case "$tag" in
        *' '*) ;;                       # has a space -> prose, not a prefix tag.
        *) printf '%s\n' "$tag"; return 0 ;;
      esac ;;
  esac

  printf '\n'
}

# sq-6ip4: PRIMARY-CODE-CRATE probe (partition by touched .rs paths, not just label).
# Given a bead's TITLE+DESCRIPTION text on $1 and the known crate short-names on $2, echo the
# crate that the bead's CODE changes land in -- inferred from an explicit `crates/<crate>/.../*.rs`
# path mention -- or "" if none is found. This is a SEPARATE signal from infer_surface: a bead's
# conflict SURFACE (label) can be e.g. "genai"/"introspect" (its READMEs/SKILLs) while its actual
# code lives in sparq-core. The wave wm5fcnlqj bug: #597 and #598 both edited
# crates/sparq-core/src/lib.rs but had DIFFERENT surface labels, so the label-only partition let
# BOTH launch -> merge-conflict risk. By ALSO reserving the primary-code crate, two beads that
# touch the same crate's .rs files never launch in one wave even when their labels diverge.
#
# Only an explicit `crates/<crate>/<...>.rs` path counts (a precise, low-false-positive signal):
#   - the crate segment must be a KNOWN crate short-name (validated against $2), so a stray
#     `crates/foo/` for a non-existent crate is ignored;
#   - the path must end in `.rs`, so a bare doc/readme reference doesn't reserve the crate.
# The FIRST matching crate wins (one primary-code crate per bead; deterministic). Pure text (no
# bd/git), so the self-test exercises it hermetically.
infer_code_crate() {
  local text="$1" crates="$2" c lc seg
  # Normalise the haystack to lowercase (crate dirs are lowercase by convention).
  lc="$(printf '%s' "$text" | tr '[:upper:]' '[:lower:]')"
  # Pull every `crates/<seg>/.../<file>.rs` occurrence in order; return the first whose <seg>
  # is a KNOWN crate. grep -o yields one match per line; we validate each against $crates.
  while IFS= read -r seg; do
    [ -n "$seg" ] || continue
    for c in $crates; do
      if [ "$seg" = "$c" ]; then printf '%s\n' "$c"; return 0; fi
    done
  done < <(printf '%s\n' "$lc" | grep -oE 'crates/[a-z0-9_-]+/[a-z0-9_./-]*\.rs' 2>/dev/null \
            | sed -E 's#^crates/([a-z0-9_-]+)/.*#\1#')
  printf '\n'
}

# sq-751l: HELD-scope predicate — is a bead in the sparq-zk value-lane that the
# maintainer holds while PR #514 (the signed typed-value-representation design) is open?
#
# WHY this exists separately from infer_surface: the scheduler reserves a surface only when an
# OPEN PR's branch/title carries that surface's token. #514 is a research/design PR — its
# inferred surface is just "research" (a `research:`-prefixed title) — so it reserved the
# research-doc surface but left the sparq-zk + sparq-zk-compose CRATES free. A value-lane impl
# bead (sq-mslu/#637, "sparq-zk-compose: general xsd:double RNE-parser FILTER member") therefore
# launched even though the maintainer holds "#514 + ANY sparq-zk value-lane impl" (the design may
# obsolete the whole in-circuit-parsing lane). The orchestrator caught it at arm time and parked
# it; this predicate makes the scheduler exclude the lane up front.
#
# A bead is in the held zk value-lane (given its TITLE on $1) iff:
#   * it mentions a sparq-zk crate (sparq-zk / sparq-zk-compose) — the value-lane CRATES; OR
#   * its title leads with a "zk" word token — the maintainer's convention for a zk-impl bead
#     ("ZK: ...", "ZK join: ...", "zk-...", "[zk] ..."). A bare "zk" mid-prose does NOT match
#     (we only treat a LEADING zk token as the lane marker), so an unrelated bead that merely
#     references zk in passing is not over-excluded.
# Pure text (no bd/gh/git) -> hermetically tested in self_test. The HOLD is only ENFORCED when
# #514 is open (see HELD_ZK_VALUE_LANE below); this predicate just classifies the lane.
is_zk_value_lane() {
  local title="$1" lc lead
  lc="$(printf '%s' "$title" | tr '[:upper:]' '[:lower:]')"
  # 1. an explicit sparq-zk / sparq-zk-compose mention -> value-lane crate.
  case "$lc" in
    *sparq-zk*) return 0 ;;
  esac
  # 2. a LEADING "zk" word token: "zk:", "zk ", "zk-", or a "[zk]"/"[zk ...]" bracket tag.
  lead="${lc#\[}"                       # tolerate a leading "[zk]"/"[zk-foo]" bracket.
  case "$lead" in
    zk:*|"zk "*|zk-*|"zk]"*) return 0 ;;
  esac
  return 1
}

# sq-8rpq: in-flight reservation by UNPUSHED worktree branches (not every branch).
# inflight_wt_branch <branch> <unpushed-count> : echo <branch> iff it is an IN-FLIGHT
# signal, i.e. iff it has UNPUSHED local commits. <unpushed-count> is the number of
# commits on the branch's HEAD that are absent from its push target (origin/<branch> or
# its upstream); "0" = fully pushed (or squash-merged) -> NOT in flight; any non-"0"
# (including the 999999 never-pushed sentinel) -> genuinely fresh work with no PR yet ->
# in flight. The UNPUSHED test (not "ancestor of main") is the load-bearing choice:
# squash-merged feature branches are NOT ancestors of main but they WERE pushed
# (unpushed=0), so an ancestor/`--no-merged` test would flag all ~500 lingering worktree
# branches and reserve every crate, emptying the frontier (the sq-8rpq bug). Pure (no
# git): the count is injected, so this is hermetically testable. Mirrors
# scripts/worktree-gc.sh's `wt_unpushed_count` predicate.
inflight_wt_branch() {
  local branch="$1" unpushed="$2"
  [ -n "$branch" ] || return 0
  [ "$unpushed" = "0" ] && return 0        # fully pushed / squash-merged -> not in flight.
  printf '%s\n' "$branch"
}

# wt_unpushed_count <wt> <branch> : number of commits on HEAD absent from the push target.
# Push target = configured upstream if any, else origin/<branch>. If neither exists (the
# branch was never pushed) return a large sentinel so the caller treats it as "has
# unpushed work" (genuinely fresh, no PR yet). Copied verbatim from scripts/worktree-gc.sh
# so the two scripts share one definition of "unpushed".
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

# --- CPU ceiling ----------------------------------------------------------------------
# The build farm cannot usefully parallelise more cargo builds than it has cores; the
# orchestrator brief uses min(16, nproc-2). nproc-2 leaves headroom for the orchestrator
# + the OS. Floor at 1 so a 1-2 core box still emits something.
cpu_ceiling() {
  local n cap
  n="$(nproc 2>/dev/null || echo 2)"
  cap=$(( n - 2 ))
  [ "$cap" -lt 1 ] && cap=1
  [ "$cap" -gt 16 ] && cap=16
  printf '%s\n' "$cap"
}

# --- self-test (no external calls) -----------------------------------------------------
self_test() {
  local fails=0 got CR
  CR="sparq-zk-compose sparq-server sparq-reason sparq-core sparq-solid sparq-hdt sparq-zk sparq-mpc"
  _check() {
    if [ "$2" = "$3" ]; then printf '  ok   %s\n' "$1"
    else printf '  FAIL %s\n       got:  %q\n       want: %q\n' "$1" "$2" "$3"; fails=$((fails + 1)); fi
  }
  log "running --dry-run self-test (hermetic; no bd/gh/git calls)"

  got="$(infer_surface 'sparq-server: HTTP integration tests for PSS named-graph query+update' "$CR")"
  _check "server (non-auth) -> crate"              "$got" "sparq-server"

  got="$(infer_surface 'sparq-server: bearer-gated auth path hardening on http.rs' "$CR")"
  _check "server auth path -> server-auth"         "$got" "server-auth"

  got="$(infer_surface 'fix(sparq-core): dict spill (sq-glw2)' "$CR")"
  _check "explicit crate mention"                  "$got" "sparq-core"

  got="$(infer_surface 'sparq-solid: support acp:issuer principal' "$CR")"
  _check "solid ACP is NOT server-auth"            "$got" "sparq-solid"

  got="$(infer_surface 'page: /surface/zk (tier-c live) + /surface/mpc walkthrough' "$CR")"
  _check "page: -> site"                           "$got" "site"

  got="$(infer_surface 'site: browser smoke test for ZK prover pre-warm' "$CR")"
  _check "site: -> site"                           "$got" "site"

  got="$(infer_surface 'tier-b wasm bundle: sparq-reason (W-reason)' "$CR")"
  _check "crate mention wins over tier-b-wasm"     "$got" "sparq-reason"

  got="$(infer_surface '[cert][cryptoreview] external audit' "$CR")"
  _check "leading bracket tag"                     "$got" "cert"

  got="$(infer_surface 'deps: hdt 0.4 -> 0.6 (#116)' "$CR")"
  _check "prefix: tag"                             "$got" "deps"

  got="$(infer_surface 'Federation TPF follow-ups: brTPF DoS cap' "$CR")"
  _check "prose-before-colon => unscoped"          "$got" ""

  # --- in-flight worktree-branch filter (sq-8rpq) -------------------------------------
  # A worktree branch is an in-flight signal ONLY if it has UNPUSHED local commits (fresh
  # work, no PR yet). A pushed / squash-merged branch (unpushed=0) must NOT reserve its
  # surface — else the hundreds of lingering worktree branches reserve every crate and the
  # frontier is spuriously empty. The unpushed COUNT is injected so this stays hermetic
  # (no git calls); the count semantics mirror worktree-gc.sh's wt_unpushed_count.
  got="$(inflight_wt_branch 'sq-glw2-dict-spill' 2)"
  _check "branch w/ unpushed commits IS in-flight" "$got" "sq-glw2-dict-spill"

  got="$(inflight_wt_branch 'sq-never-pushed' 999999)"
  _check "never-pushed branch IS in-flight"        "$got" "sq-never-pushed"

  got="$(inflight_wt_branch 'sq-squash-merged' 0)"
  _check "pushed/squash-merged branch NOT in-flight" "$got" ""

  # --- primary-code-crate probe (sq-6ip4) ---------------------------------------------
  # The code-crate is inferred from an explicit `crates/<crate>/.../*.rs` path in the bead's
  # title+description, INDEPENDENT of its label surface. This is the wave wm5fcnlqj fix: a
  # bead labelled genai/introspect whose code lives in crates/sparq-core/src/lib.rs must
  # reserve sparq-core so a second sparq-core bead can't launch in the same wave.
  got="$(infer_code_crate 'named-graph scoping for GenAI introspect — edits crates/sparq-core/src/lib.rs' "$CR")"
  _check "desc .rs path -> code crate (the bug)"   "$got" "sparq-core"

  got="$(infer_code_crate 'sq-6ip4 scheduler: code lives in crates/sparq-core/src/lib.rs' "$CR")"
  _check "explicit crates/<crate>/...rs path"      "$got" "sparq-core"

  got="$(infer_code_crate 'forge gate regression in crates/sparq-zk-compose/tests/forge_gates.rs' "$CR")"
  _check "tests/ .rs path under a crate"           "$got" "sparq-zk-compose"

  got="$(infer_code_crate 'no code path here, just prose about sparq-core internals' "$CR")"
  _check "crate NAME but no .rs path -> none"      "$got" ""

  got="$(infer_code_crate 'doc tweak to crates/sparq-core/README.md only' "$CR")"
  _check "non-.rs path (README) -> none"           "$got" ""

  got="$(infer_code_crate 'edits crates/not-a-crate/src/lib.rs (unknown crate)' "$CR")"
  _check "unknown crate segment -> none"           "$got" ""

  got="$(infer_code_crate 'first crates/sparq-core/src/a.rs then crates/sparq-mpc/src/b.rs' "$CR")"
  _check "first known .rs crate wins"              "$got" "sparq-core"

  # --- HELD zk value-lane predicate (sq-751l) -----------------------------------------
  # While #514 is open, the maintainer holds "#514 + ANY sparq-zk value-lane impl". A bead is
  # in the held lane iff it mentions a sparq-zk crate OR leads with a "zk" word token. The
  # exemplar held beads from the bug report (sq-mslu/sq-lxi7/.../sq-uii0) must all classify
  # as held; the design PR itself and unrelated beads must NOT.
  _zk() { if is_zk_value_lane "$1"; then printf 'held'; else printf 'free'; fi; }

  got="$(_zk 'sparq-zk-compose: general xsd:double RNE-parser FILTER member')"
  _check "sparq-zk-compose crate -> held (sq-mslu)" "$got" "held"

  got="$(_zk 'ZK: general xsd:double decimal->IEEE-754 RNE parser')"
  _check "leading 'ZK:' token -> held (sq-lxi7)"    "$got" "held"

  got="$(_zk 'ZK join: optional v2 cardinality-hiding join_eq_rN')"
  _check "leading 'ZK join:' -> held (sq-uii0)"     "$got" "held"

  got="$(_zk '[zk] in-circuit salt binding privacy residuals')"
  _check "leading '[zk]' bracket -> held"           "$got" "held"

  got="$(_zk 'sparq-zk: regression-gate holder_pok gate count')"
  _check "sparq-zk crate -> held"                   "$got" "held"

  # The design/research PR #514's own title is research-surface, NOT a value-lane impl.
  got="$(_zk 'research: signed typed-value representation to remove in-circuit literal parsing')"
  _check "research design title -> free (#514 self)" "$got" "free"

  # Unrelated beads, including ones that mention zk only mid-prose, are NOT held.
  got="$(_zk 'fix(sparq-core): dict spill (sq-glw2)')"
  _check "non-zk bead -> free"                       "$got" "free"

  got="$(_zk 'sparq-server: tighten error contract for the zk proof endpoint')"
  _check "mid-prose 'zk' (not leading) -> free"      "$got" "free"

  got="$(_zk 'docs: explain zk value-lane in the README')"
  _check "mid-prose value-lane mention -> free"      "$got" "free"

  echo
  if [ "$fails" -eq 0 ]; then log "self-test PASSED"; return 0; fi
  die "self-test FAILED ($fails check(s))"
}

# --- arg parsing -----------------------------------------------------------------------
for arg in "$@"; do
  case "$arg" in
    --dry-run-self-test) self_test; exit 0 ;;
    --explain)           EXPLAIN=1 ;;
    -h|--help)           sed -n '2,60p' "$0"; exit 0 ;;
    *)                   die "unknown argument: $arg (try --explain, --dry-run-self-test, --help)" ;;
  esac
done

command -v bd >/dev/null 2>&1 || die "bd not found (this script reads 'bd ready'); add ~/.local/bin to PATH"

CAP="$(cpu_ceiling)"
[ "$EXPLAIN" -eq 1 ] && log "CPU ceiling = $CAP (min(16, nproc-2); nproc=$(nproc 2>/dev/null || echo '?'))"

# Known crate short-names (from crates/), LONGEST-first so e.g. sparq-zk-compose is
# matched before sparq-zk (a substring would otherwise mis-bucket it).
CRATES=""
if [ -d "$ROOT/crates" ]; then
  for d in "$ROOT"/crates/*/; do
    [ -d "$d" ] || continue
    name="${d%/}"; name="${name##*/}"
    CRATES+="$name"$'\n'
  done
  CRATES="$(printf '%s' "$CRATES" | awk 'NF { print length, $0 }' | sort -rn | cut -d' ' -f2- | tr '\n' ' ')"
fi
[ -n "$CRATES" ] || log "WARNING: no crates/ dir — surface inference falls back to title tags only."

# --- 1. in-flight bead ids -------------------------------------------------------------
# A bead is in flight if its id token appears in:
#   (a) an open PR's head-branch name or title  — the authoritative, primary signal; OR
#   (b) a worktree branch with UNPUSHED local commits — work an agent has committed but
#       not yet pushed/PR'd, so the PR signal can't see it yet (sq-8rpq).
# Load-bearing on WHY (b) is the UNPUSHED test and NOT "branch not an ancestor of main":
# PRs are squash-merged, so a finished/merged feature branch is NEVER an ancestor of main
# and a `--no-merged`/ancestor test flags ALL ~500 lingering worktree branches as "in
# flight" — that reserves every crate and empties the frontier (the exact sq-8rpq bug).
# A squash-merged branch was PUSHED (origin/<branch>..HEAD == 0), so the unpushed test
# excludes it; only a genuinely-fresh, not-yet-pushed branch (no PR possible yet) survives.
# This mirrors scripts/worktree-gc.sh's `wt_unpushed_count` "never-dropped" predicate.
if command -v gh >/dev/null 2>&1; then
  # head-branch names + titles of every open PR, joined per-line for substring search.
  PR_BLOB="$(gh pr list --state open --limit 300 --json headRefName,title \
    -q '.[] | (.headRefName + " " + .title)' 2>/dev/null || true)"
else
  log "gh not found — open-PR in-flight subtraction will be EMPTY (unpushed worktrees still used)."
  PR_BLOB=""
fi

# Append UNPUSHED-worktree branch names to the in-flight blob (one per line). A worktree
# with detached HEAD (no branch line) is skipped; a branch with no local commits beyond
# its push target (pushed/squash-merged) is intentionally omitted.
WT_PATH="" WT_BRANCH=""
while IFS= read -r line; do
  case "$line" in
    "worktree "*) WT_PATH="${line#worktree }"; WT_BRANCH="" ;;
    "branch refs/heads/"*)
      WT_BRANCH="${line#branch refs/heads/}"
      if [ -n "$WT_BRANCH" ] && [ -d "$WT_PATH" ]; then
        b="$(inflight_wt_branch "$WT_BRANCH" "$(wt_unpushed_count "$WT_PATH" "$WT_BRANCH")")"
        [ -n "$b" ] && PR_BLOB+="$b"$'\n'
      fi
      ;;
  esac
done < <(git -C "$ROOT" worktree list --porcelain 2>/dev/null || true)

# Is bead id $1 in flight (its id appears in any open-PR branch/title or unpushed-worktree branch)?
is_inflight() {
  local id="$1"
  [ -n "$PR_BLOB" ] || return 1
  printf '%s\n' "$PR_BLOB" | grep -qiF -- "$id"
}

# --- 1b. HELD-scope detection (sq-751l) ------------------------------------------------
# The maintainer holds "#514 + ANY sparq-zk value-lane impl" while the signed typed-value
# DESIGN PR #514 is open. The scheduler reserves a surface only when an open PR carries its
# token; #514's surface is just "research", so the zk crates stayed free and a value-lane impl
# bead launched (sq-mslu/#637). We therefore add an explicit HOLD: while #514 is open, every
# bead in the zk value-lane (is_zk_value_lane) is excluded from the frontier.
#
# #514 is treated as open iff `gh pr view 514` reports OPEN. If gh is unavailable we fall back
# to scanning the open-PR blob for #514's head-branch token (the design branch); if neither
# signal is available the hold is conservatively OFF (we never silently empty the zk lane on a
# transient gh outage — the orchestrator's arm-time check remains the backstop).
HELD_ZK_VALUE_LANE=0
HELD_514_BRANCH="zk-signed-credential-representation-design"
if command -v gh >/dev/null 2>&1; then
  pr514_state="$(gh pr view 514 --json state -q .state 2>/dev/null || true)"
  if [ "$pr514_state" = "OPEN" ]; then
    HELD_ZK_VALUE_LANE=1
  elif [ -z "$pr514_state" ] && printf '%s\n' "$PR_BLOB" | grep -qiF -- "$HELD_514_BRANCH"; then
    # gh pr view failed (e.g. rate-limited) but the design branch is in the open-PR blob.
    HELD_ZK_VALUE_LANE=1
  fi
fi
if [ "$EXPLAIN" -eq 1 ]; then
  if [ "$HELD_ZK_VALUE_LANE" -eq 1 ]; then
    log "HOLD active: #514 open -> sparq-zk value-lane beads RESERVED (sq-751l)"
  else
    log "HOLD inactive: #514 not detected open -> zk value-lane NOT reserved by this hold"
  fi
fi

# --- 2. read bd ready ------------------------------------------------------------------
# `--limit 0` => UNLIMITED. `bd ready` defaults to --limit 100 (and `bd list` to 50): without
# this the frontier was silently truncated to the first 100 ready beads by (priority,id) sort,
# so lower-sorting LAUNCHABLE beads — in particular many epic-CHILD beads (sq-bif.6/.8/.9,
# sq-5o5.*, sq-jpki.*, ...) — fell off the end before the cap/dedup even ran. The scheduler then
# starved on epics + a handful of top-level beads while clean, atomic children sat invisible
# (sq-p7nw). We pull the FULL ready set here and let the CPU-ceiling cap (§3) do the bounding.
READY_JSON="$(bd ready --json --limit 0 2>/dev/null)" || die "bd ready --json failed"

# Emit "id<TAB>priority<TAB>title<TAB>description" lines, sorted by priority asc (P0 first) then
# id. EXCLUDE umbrellas — a parent bead is a container, not a dispatchable work unit (see header).
# The DESCRIPTION column (sq-6ip4) feeds the primary-code-crate probe: a bead's code may live in a
# crate its title never names, so we scan the description for an explicit `crates/<crate>/*.rs`
# path. Tabs/newlines are flattened so each bead stays on one row.
#
# sq-p7nw: "umbrella" is NOT just `issue_type=="epic"`. A bead can be a feature/task-typed PARENT
# whose decomposed CHILDREN (sq-<parent>.N, linked by a parent-child dependency) are the real
# atomic work — e.g. sq-bif (type=feature) is the parent of sq-bif.6.../, sq-5o5 (type=feature)
# of sq-5o5.*. The OLD filter only dropped literal-epic / "[epic]"-titled beads, so these
# feature-typed parents were emitted AS launchable while their ready children competed for (and
# lost) the same frontier slots — the scheduler launched "the umbrella" instead of its children.
# We now also drop any bead that is the PARENT of one or more OTHER beads present in this ready
# set: that parent's actual work has been decomposed into children which are themselves ready and
# WILL be surfaced. A literal epic is still always dropped even when it has no ready children
# (an epic is never itself a work unit). A childless non-epic parent reference (parent points at a
# bead not in the ready set) does NOT make anything an umbrella — only beads with >=1 READY child
# are treated as containers, so we never drop a genuine leaf.
ROWS="$(printf '%s' "$READY_JSON" | python3 -c '
import json,sys
data=json.load(sys.stdin)
# Parents that have at least one OTHER bead in the ready set pointing at them as parent.
# Both an explicit "parent" field and the parent-child dependency edge are honoured (bd
# exposes either depending on version); a self-referential parent is ignored.
ready_ids={it.get("id") for it in data}
umbrella_parents=set()
for it in data:
    cid=it.get("id")
    pid=it.get("parent")
    if pid and pid!=cid:
        umbrella_parents.add(pid)
    for dep in (it.get("dependencies") or []):
        if (dep.get("type") or "")=="parent-child":
            p=dep.get("depends_on_id")
            if p and p!=cid:
                umbrella_parents.add(p)
def key(it):
    try: p=int(it.get("priority",9))
    except Exception: p=9
    return (p, it.get("id",""))
def flat(s):
    return (s or "").replace("\t"," ").replace("\n"," ").replace("\r"," ")
def is_needs_gated(it):
    # A "needs:user"/"needs:maintainer"-LABELLED bead is gated on a human action (publish a
    # license, flip a GitHub Pages setting, file an upstream issue, ...). It is NOT
    # agent-launchable, so it must never enter the autonomous frontier. bd ready returns these
    # (they are open + unblocked by bead-deps), so we filter on the label here. The label is the
    # reliable signal — the title prefix is inconsistent (some carry it, some do not). sq-p7nw:
    # before --limit 0 these mostly fell past the 100-row truncation by accident; now that the
    # full ready set is read, the filter must be explicit so a needs-gated bead cannot leak in.
    for lbl in (it.get("labels") or []):
        if (lbl or "").lower().startswith("needs:"):
            return True
    return False
for it in sorted(data, key=key):
    iid=it.get("id","")
    # Always drop a literal epic, or a bead the maintainer tagged "[epic]" in its title.
    if (it.get("issue_type") or "") == "epic":
        continue
    if (it.get("title") or "").lstrip().lower().startswith("[epic]"):
        continue
    # Drop human-gated beads (needs:user / needs:maintainer) — not agent-launchable (sq-p7nw).
    if is_needs_gated(it):
        continue
    # sq-p7nw: drop a PARENT whose children (>=1) are present in this ready set — it is an
    # umbrella; its ready children are surfaced in its place. (A childless parent is a leaf.)
    if iid in umbrella_parents:
        continue
    print("\t".join([
        iid,
        str(it.get("priority","")),
        flat(it.get("title","")),
        flat(it.get("description","")),
    ]))
' 2>/dev/null)" || die "could not parse bd ready --json"

[ -n "$ROWS" ] || { log "no ready beads — frontier is empty."; exit 0; }

# --- 3. compute the frontier: subtract in-flight, partition surfaces, cap at CPU -------
# A surface is "taken" once a bead on it is in flight OR has been emitted. The site and
# server-auth lanes are surfaces too, so the per-surface "<=1" rule serialises them.
declare -A TAKEN          # surface -> 1 once occupied (by in-flight or emitted)
emitted=0

# Seed TAKEN with the surfaces AND primary-code crates of in-flight beads so we never emit a
# second bead that would collide with work already running -- on EITHER lane (sq-6ip4): the
# label surface (infer_surface) and the touched-.rs crate (infer_code_crate). Reserving both
# means an in-flight bead whose code lives in sparq-core blocks another sparq-core bead even
# if the two carry different labels.
while IFS=$'\t' read -r id prio title desc; do
  [ -n "$id" ] || continue
  if is_inflight "$id"; then
    surface="$(infer_surface "$title" "$CRATES")"
    code_crate="$(infer_code_crate "$title $desc" "$CRATES")"
    [ -n "$surface" ] && TAKEN["$surface"]=1
    [ -n "$code_crate" ] && TAKEN["$code_crate"]=1
    [ "$EXPLAIN" -eq 1 ] && log "DROP  $id  [in-flight: open PR] surface=${surface:-(unscoped)} code-crate=${code_crate:-(none)}"
  fi
done <<< "$ROWS"

# Now walk in priority order and emit launchable beads.
OUT=""
while IFS=$'\t' read -r id prio title desc; do
  [ -n "$id" ] || continue

  if is_inflight "$id"; then
    continue                                   # already accounted for in the seed pass.
  fi

  # sq-751l: while #514 is open, the maintainer holds the entire sparq-zk value-lane. A
  # value-lane bead is RESERVED -- excluded from the frontier -- even though no open PR carries
  # the zk-crate token (#514's surface is just "research"). This is the fix for the launch of a
  # held bead (sq-mslu/#637); see the HELD_ZK_VALUE_LANE block above.
  if [ "$HELD_ZK_VALUE_LANE" -eq 1 ] && is_zk_value_lane "$title"; then
    [ "$EXPLAIN" -eq 1 ] && log "DROP  $id  [HELD: sparq-zk value-lane reserved while #514 open (sq-751l)]"
    continue
  fi

  surface="$(infer_surface "$title" "$CRATES")"
  # sq-6ip4: the primary CODE crate (touched .rs paths), probed from title+description. It is a
  # SECOND conflict lane on top of the label surface, so two beads that edit the same crate's
  # source never launch in one wave even when their labels diverge (the wave wm5fcnlqj bug).
  code_crate="$(infer_code_crate "$title $desc" "$CRATES")"

  if [ "$emitted" -ge "$CAP" ]; then
    [ "$EXPLAIN" -eq 1 ] && log "DROP  $id  [CPU ceiling $CAP reached]"
    continue
  fi

  if [ -n "$surface" ] && [ "${TAKEN[$surface]:-0}" = "1" ]; then
    [ "$EXPLAIN" -eq 1 ] && log "DROP  $id  [surface '$surface' already taken (conflict-collision)]"
    continue
  fi

  if [ -n "$code_crate" ] && [ "${TAKEN[$code_crate]:-0}" = "1" ]; then
    [ "$EXPLAIN" -eq 1 ] && log "DROP  $id  [code-crate '$code_crate' already taken (conflict-collision)]"
    continue
  fi

  # Launch it. Reserve BOTH lanes so a later bead colliding on either is held back.
  [ -n "$surface" ] && TAKEN["$surface"]=1
  [ -n "$code_crate" ] && TAKEN["$code_crate"]=1
  OUT+="$(printf '%s\t%s\t%s' "$id" "${surface:-(unscoped)}" "$title")"$'\n'
  emitted=$((emitted + 1))
  [ "$EXPLAIN" -eq 1 ] && log "KEEP  $id  surface=${surface:-(unscoped)} code-crate=${code_crate:-(none)}  P$prio"
done <<< "$ROWS"

# --- 4. print the launchable frontier --------------------------------------------------
if [ -z "$OUT" ]; then
  log "frontier is empty (all ready beads are in-flight or collide). CPU ceiling=$CAP."
  exit 0
fi

# Column-aligned: "<id>  <surface>  <title>".
printf '%s' "$OUT" | awk -F'\t' '
  { ids[NR]=$1; surf[NR]=$2; ttl[NR]=$3
    if (length($1)>iw) iw=length($1)
    if (length($2)>sw) sw=length($2) }
  END { for (i=1;i<=NR;i++) printf "%-*s  %-*s  %s\n", iw, ids[i], sw, surf[i], ttl[i] }
'
