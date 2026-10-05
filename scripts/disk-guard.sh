#!/usr/bin/env bash
# Disk guard — the per-tick disk-pressure broom (bead sq-4vo9m).
#
# disk-guard.sh [--dry-run | --apply] [--warn-gb N] [--critical-gb N]
#               [--mount PATH] [--main PATH] [--root DIR] [--base REF]
#               [--no-worktree-gc] [--no-reclaim-completed] [--reclaim-main-target]
#               [--dry-run-self-test]
#
# WHY: 2026-06-21 the work box filled to 99% (286/290G) and a wave-4 verify agent hit
# ENOSPC mid-build (could not run `--workspace` clippy). The autonomous loop spawns one
# worktree-isolated agent per bead, each accumulating a multi-GB `target/`, so disk
# re-fills every wave. The existing `worktree-gc.sh` (sq-6xdr) is the SAFE broom for
# finished worktrees, but it was a MANUAL / idle-time tool — nothing ran a `df` check or
# the prune ON EACH MAINTENANCE TICK. This script is that per-tick GUARD: it
#   (1) measures free space on the disk and classifies OK / WARN (<20G) / CRITICAL (<10G);
#   (2) prunes finished worktrees by DELEGATING to worktree-gc.sh (it does NOT reimplement
#       the safe predicate — composition, not duplication). It runs the broom with
#       --reclaim-completed (default ON, --no-reclaim-completed to disable) so it ALSO
#       reclaims COMPLETED-but-unmerged workflow worktrees — clean trees whose PR branch is
#       pushed but not yet merged, with no live owning task. Without this they linger forever
#       and starve the disk (observed 2026-06-22: ~229G of these, one 33G — bead sq-h34dc).
#       The in-use guard inside worktree-gc.sh KEEPS any tree a live process is using, so a
#       per-tick reclaim never touches an in-flight agent; and
#   (3) only when CRITICAL, and only when NO live cargo/rustc build is touching the
#       orchestrator main checkout's `target/`, reclaims that `target/` (regenerable:
#       agents build in their OWN worktrees, so the orchestrator's `target/` is not load-
#       bearing for in-flight work).
#
# DESIGN — same discipline as worktree-gc.sh / ci-free-disk.sh:
#   • DEFAULT IS --dry-run: it MEASURES + REPORTS + delegates a DRY-RUN prune, and prints
#     what it WOULD reclaim. It mutates NOTHING without --apply.
#   • --apply: runs `worktree-gc.sh --apply` (safe-set removal + prune) and, if CRITICAL
#     and `--reclaim-main-target` is set AND no live build is detected, deletes the main
#     checkout's `target/`. Even with --apply, the main-`target/` reclaim is GATED behind
#     CRITICAL + an explicit opt-in flag + a live-build check — it never fires casually.
#   • The main checkout is a HARD never-touch for worktree removal (worktree-gc.sh asserts
#     this); the ONLY thing this guard may remove from the main checkout is the regenerable
#     `target/` build dir, and only under the gates above.
#   • NON-FATAL by contract: a guard tick must never abort the caller. A failed `df`, an
#     absent worktree-gc.sh, or a busy main `target/` all degrade to a logged skip, not a
#     non-zero crash of the whole maintenance sweep. The EXIT CODE encodes the disk STATE
#     so a caller can act (0 = OK, 10 = WARN, 20 = CRITICAL), but those are advisory signals
#     a scheduler reads, NOT hard failures (see the scheduler/AGENTS.md wiring).
#
# Run:
#   scripts/disk-guard.sh                       # dry-run (default): df + dry-run prune + plan
#   scripts/disk-guard.sh --apply               # prune the safe worktree set, then report
#   scripts/disk-guard.sh --apply --reclaim-main-target   # also drop main target/ IF critical
#   scripts/disk-guard.sh --no-reclaim-completed --apply  # MERGED/GONE-only prune (old behaviour)
#   scripts/disk-guard.sh --warn-gb 30          # warn earlier
#   scripts/disk-guard.sh --dry-run-self-test   # hermetic self-test (no fs/git mutation)
set -uo pipefail   # NOT -e: a guard tick is best-effort and must not abort its caller.

PROG="disk-guard"
log()  { printf '[%s] %s\n' "$PROG" "$*" >&2; }
die()  { printf '[%s] ERROR: %s\n' "$PROG" "$*" >&2; exit 1; }

# --- configuration (overridable by flags) -----------------------------------------------
DEFAULT_MOUNT="/"
DEFAULT_MAIN="/home/ubuntu/sparq"
DEFAULT_ROOT="/home/ubuntu/sparq/.claude/worktrees"
DEFAULT_BASE="origin/main"
DEFAULT_WARN_GB=20       # bead: "warn if <20G free"
DEFAULT_CRITICAL_GB=10   # below this, escalate to the gated main-target reclaim

# --- pure, testable helpers -------------------------------------------------------------
# free_gb_from_df_line <df-line>: extract the "Available" column (4th field, in GiB from
# `df -BG`) as a bare integer. Pure string parse — no fs access — so it is unit-testable.
# Accepts a value with or without a trailing 'G'. Prints the integer, or empty on a line we
# cannot parse (caller treats empty as "unknown" → skip, never crash).
free_gb_from_df_line() {
  local line="$1" avail
  # df -BG row: "Filesystem 1G-blocks Used Available Use% Mounted" → field 4 is Available.
  avail="$(printf '%s\n' "$line" | awk 'NR==1{print $4}')"
  avail="${avail%G}"
  case "$avail" in
    ''|*[!0-9]*) printf '' ;;     # non-numeric ⇒ unknown
    *)           printf '%s' "$avail" ;;
  esac
}

# classify_state <free_gb> <warn_gb> <critical_gb>: echo OK | WARN | CRITICAL | UNKNOWN.
# Pure arithmetic; the single source of the threshold decision so it is testable in one
# place and the exit-code mapping below cannot drift from it.
classify_state() {
  local free="$1" warn="$2" crit="$3"
  case "$free" in ''|*[!0-9]*) echo "UNKNOWN"; return 0 ;; esac
  if [ "$free" -lt "$crit" ]; then echo "CRITICAL"; return 0; fi
  if [ "$free" -lt "$warn" ]; then echo "WARN";     return 0; fi
  echo "OK"
}

# state_exit_code <state>: 0 OK/UNKNOWN, 10 WARN, 20 CRITICAL. UNKNOWN maps to 0 (a df we
# could not parse must not, by itself, look like a crisis — we already logged it).
state_exit_code() {
  case "$1" in
    CRITICAL) echo 20 ;;
    WARN)     echo 10 ;;
    *)        echo 0  ;;
  esac
}

# --- hermetic self-test (no fs/git mutation; exercises the pure predicates) --------------
self_test() {
  local fails=0
  _ok()   { printf '  ok   %s\n' "$1"; }
  _fail() { printf '  FAIL %s\n       %s\n' "$1" "$2"; fails=$((fails + 1)); }
  _eq()   { if [ "$2" = "$3" ]; then _ok "$1"; else _fail "$1" "expected '$3', got '$2'"; fi; }
  log "running --dry-run self-test (hermetic; no fs/git mutation)"

  # free_gb_from_df_line: parse the Available column from a df -BG data row.
  _eq "parse plain integer avail"        "$(free_gb_from_df_line '/dev/root 290G 141G 149G 49% /')" "149"
  _eq "parse 0G avail"                   "$(free_gb_from_df_line 'fs 290G 290G 0G 100% /')"          "0"
  _eq "short line (no \$4) ⇒ empty"      "$(free_gb_from_df_line 'total garbage')"                   ""
  _eq "non-numeric avail ⇒ empty"        "$(free_gb_from_df_line 'a b c notanumber d e')"            ""

  # classify_state: the threshold decision (warn=20, critical=10).
  _eq "200G free ⇒ OK"                   "$(classify_state 200 20 10)" "OK"
  _eq "exactly 20G ⇒ OK (>=warn)"        "$(classify_state 20 20 10)"  "OK"
  _eq "19G ⇒ WARN"                       "$(classify_state 19 20 10)"  "WARN"
  _eq "exactly 10G ⇒ WARN (>=crit)"      "$(classify_state 10 20 10)"  "WARN"
  _eq "9G ⇒ CRITICAL"                    "$(classify_state 9 20 10)"   "CRITICAL"
  _eq "0G ⇒ CRITICAL"                    "$(classify_state 0 20 10)"   "CRITICAL"
  _eq "empty free ⇒ UNKNOWN"            "$(classify_state '' 20 10)"  "UNKNOWN"

  # state_exit_code: the advisory exit-code mapping.
  _eq "OK ⇒ exit 0"                      "$(state_exit_code OK)"       "0"
  _eq "UNKNOWN ⇒ exit 0"                 "$(state_exit_code UNKNOWN)"  "0"
  _eq "WARN ⇒ exit 10"                   "$(state_exit_code WARN)"     "10"
  _eq "CRITICAL ⇒ exit 20"               "$(state_exit_code CRITICAL)" "20"

  # main_target_build_busy: a real `pgrep`-backed probe; just assert it is callable and
  # returns a clean 0/1 (we cannot deterministically assert a system state here).
  if main_target_build_busy "/nonexistent/main"; then _ok "busy-probe returns (busy)"; else _ok "busy-probe returns (idle)"; fi

  echo
  if [ "$fails" -eq 0 ]; then log "self-test PASSED"; return 0; fi
  die "self-test FAILED ($fails check(s))"
}

# main_target_build_busy <main>: 0 (busy) iff a live cargo/rustc process appears to be
# building under <main>/target. Best-effort heuristic — when in doubt, report BUSY (1's
# inverse) so we KEEP the dir. We look for cargo/rustc processes whose cwd or args mention
# the main checkout path; a false "busy" only means we skip a reclaim (safe), whereas a
# false "idle" could abort a build, so the probe is deliberately conservative.
main_target_build_busy() {
  local main="${1%/}"
  command -v pgrep >/dev/null 2>&1 || return 0   # cannot tell ⇒ assume busy ⇒ keep
  # Any cargo/rustc/rustdoc process at all, whose command line references the main path.
  if pgrep -af 'cargo|rustc|rustdoc' 2>/dev/null | grep -qF -- "$main"; then
    return 0   # busy
  fi
  return 1     # idle
}

# --- arg parsing -------------------------------------------------------------------------
APPLY=0
MOUNT="$DEFAULT_MOUNT"
MAIN="$DEFAULT_MAIN"
ROOT="$DEFAULT_ROOT"
BASE="$DEFAULT_BASE"
WARN_GB="$DEFAULT_WARN_GB"
CRITICAL_GB="$DEFAULT_CRITICAL_GB"
RUN_WT_GC=1
RECLAIM_MAIN_TARGET=0
RECLAIM_COMPLETED=1   # default ON: the per-tick guard reclaims completed-but-unmerged workflow
                     # worktrees too (sq-h34dc). worktree-gc.sh's in-use guard keeps live trees.

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run-self-test)     self_test; exit 0 ;;
    --apply)                 APPLY=1 ;;
    --dry-run)               APPLY=0 ;;
    --warn-gb)               shift; [ "$#" -gt 0 ] || die "--warn-gb needs a value"; WARN_GB="$1" ;;
    --warn-gb=*)             WARN_GB="${1#--warn-gb=}" ;;
    --critical-gb)           shift; [ "$#" -gt 0 ] || die "--critical-gb needs a value"; CRITICAL_GB="$1" ;;
    --critical-gb=*)         CRITICAL_GB="${1#--critical-gb=}" ;;
    --mount)                 shift; [ "$#" -gt 0 ] || die "--mount needs a value"; MOUNT="$1" ;;
    --mount=*)               MOUNT="${1#--mount=}" ;;
    --main)                  shift; [ "$#" -gt 0 ] || die "--main needs a value"; MAIN="$1" ;;
    --main=*)                MAIN="${1#--main=}" ;;
    --root)                  shift; [ "$#" -gt 0 ] || die "--root needs a value"; ROOT="$1" ;;
    --root=*)                ROOT="${1#--root=}" ;;
    --base)                  shift; [ "$#" -gt 0 ] || die "--base needs a value"; BASE="$1" ;;
    --base=*)                BASE="${1#--base=}" ;;
    --no-worktree-gc)        RUN_WT_GC=0 ;;
    --reclaim-completed)     RECLAIM_COMPLETED=1 ;;   # accepted for symmetry (already the default)
    --no-reclaim-completed)  RECLAIM_COMPLETED=0 ;;
    --reclaim-main-target)   RECLAIM_MAIN_TARGET=1 ;;
    -h|--help)               sed -n '2,60p' "$0"; exit 0 ;;
    *)                       die "unknown argument: $1 (try --apply, --warn-gb, --critical-gb, --no-reclaim-completed, --reclaim-main-target, --dry-run-self-test, --help)" ;;
  esac
  shift
done

case "$WARN_GB" in     ''|*[!0-9]*) die "--warn-gb must be a non-negative integer (got '$WARN_GB')" ;; esac
case "$CRITICAL_GB" in ''|*[!0-9]*) die "--critical-gb must be a non-negative integer (got '$CRITICAL_GB')" ;; esac
[ "$CRITICAL_GB" -le "$WARN_GB" ] || die "--critical-gb ($CRITICAL_GB) must be <= --warn-gb ($WARN_GB)"
MAIN="${MAIN%/}"
ROOT="${ROOT%/}"

# --- 1. measure free space ---------------------------------------------------------------
log "mode=$([ "$APPLY" -eq 1 ] && echo APPLY || echo dry-run)  mount=$MOUNT  warn=<${WARN_GB}G  critical=<${CRITICAL_GB}G"

DF_LINE="$(df -BG "$MOUNT" 2>/dev/null | awk 'NR==2{print}')" || DF_LINE=""
if [ -z "$DF_LINE" ]; then
  log "WARNING: could not read 'df -BG $MOUNT' — skipping disk measurement (non-fatal)."
  FREE_GB=""
else
  FREE_GB="$(free_gb_from_df_line "$DF_LINE")"
fi

STATE="$(classify_state "${FREE_GB:-}" "$WARN_GB" "$CRITICAL_GB")"
case "$STATE" in
  OK)       log "disk OK: ${FREE_GB}G free on $MOUNT (>= ${WARN_GB}G warn threshold)." ;;
  WARN)     log "DISK WARN: only ${FREE_GB}G free on $MOUNT (< ${WARN_GB}G) — pruning merged worktrees." ;;
  CRITICAL) log "DISK CRITICAL: only ${FREE_GB}G free on $MOUNT (< ${CRITICAL_GB}G) — pruning + escalating." ;;
  UNKNOWN)  log "disk state UNKNOWN (df unparsed) — running the safe prune anyway, skipping escalation." ;;
esac

# --- 2. prune finished worktrees (delegate to worktree-gc.sh) ----------------------------
# We ALWAYS run the worktree prune (its safe predicate KEEPS anything unmerged/dirty/in-use, so
# it is harmless when disk is fine and frees the most when it is not) — this is the per-tick
# worktree prune the bead asks for. We do not reimplement the predicate. By default we pass
# --reclaim-completed so the broom ALSO reclaims clean+pushed completed-but-unmerged workflow
# worktrees with no live owning task (sq-h34dc) — the in-use guard inside worktree-gc.sh keeps
# any tree a live process is using, so this never touches an in-flight agent.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WT_GC="${SCRIPT_DIR}/worktree-gc.sh"

if [ "$RUN_WT_GC" -eq 1 ]; then
  if [ -x "$WT_GC" ] || [ -f "$WT_GC" ]; then
    WT_GC_EXTRA=()
    [ "$RECLAIM_COMPLETED" -eq 1 ] && WT_GC_EXTRA+=(--reclaim-completed)
    log "delegating worktree prune to worktree-gc.sh ($([ "$APPLY" -eq 1 ] && echo --apply || echo --dry-run)$([ "$RECLAIM_COMPLETED" -eq 1 ] && echo ' --reclaim-completed')):"
    if [ "$APPLY" -eq 1 ]; then
      bash "$WT_GC" --apply --root "$ROOT" --main "$MAIN" --base "$BASE" "${WT_GC_EXTRA[@]+"${WT_GC_EXTRA[@]}"}" || \
        log "WARNING: worktree-gc.sh --apply returned non-zero (some worktrees kept) — non-fatal."
    else
      bash "$WT_GC" --dry-run --root "$ROOT" --main "$MAIN" --base "$BASE" "${WT_GC_EXTRA[@]+"${WT_GC_EXTRA[@]}"}" || \
        log "WARNING: worktree-gc.sh --dry-run returned non-zero — non-fatal."
    fi
  else
    log "WARNING: worktree-gc.sh not found at $WT_GC — skipping worktree prune (non-fatal)."
  fi
else
  log "worktree prune skipped (--no-worktree-gc)."
fi

# --- 3. CRITICAL escalation: reclaim the orchestrator main checkout's target/ ------------
# Regenerable (agents build in their OWN worktrees). GATED behind ALL of:
#   • STATE == CRITICAL, AND
#   • --reclaim-main-target opt-in, AND
#   • no live cargo/rustc build referencing the main checkout, AND
#   • --apply (otherwise we only PLAN it).
MAIN_TARGET="${MAIN}/target"
if [ "$STATE" = "CRITICAL" ]; then
  if [ "$RECLAIM_MAIN_TARGET" -eq 1 ]; then
    if [ -d "$MAIN_TARGET" ]; then
      if main_target_build_busy "$MAIN"; then
        log "CRITICAL but a live cargo/rustc build references $MAIN — KEEPING $MAIN_TARGET (a reclaim would abort it)."
      else
        local_size="$(du -sh "$MAIN_TARGET" 2>/dev/null | cut -f1)"
        if [ "$APPLY" -eq 1 ]; then
          log "reclaiming main checkout build dir: rm -rf $MAIN_TARGET (was ${local_size:-?}; regenerable)."
          rm -rf "$MAIN_TARGET" || log "WARNING: failed to remove $MAIN_TARGET — non-fatal."
        else
          log "PLAN (dry-run): would rm -rf $MAIN_TARGET (${local_size:-?} reclaimable) — re-run with --apply --reclaim-main-target."
        fi
      fi
    else
      log "CRITICAL: no $MAIN_TARGET to reclaim."
    fi
  else
    log "CRITICAL: main-target reclaim available but NOT opted in (pass --reclaim-main-target to enable)."
  fi
fi

# --- final df + advisory exit code -------------------------------------------------------
if [ "$APPLY" -eq 1 ] && [ -n "$DF_LINE" ]; then
  AFTER_LINE="$(df -BG "$MOUNT" 2>/dev/null | awk 'NR==2{print}')" || AFTER_LINE=""
  if [ -n "$AFTER_LINE" ]; then
    AFTER_GB="$(free_gb_from_df_line "$AFTER_LINE")"
    log "after sweep: ${AFTER_GB:-?}G free on $MOUNT."
  fi
fi

EXIT_CODE="$(state_exit_code "$STATE")"
log "done (state=$STATE, advisory exit=$EXIT_CODE)."
exit "$EXIT_CODE"
