#!/usr/bin/env bash
# Hermetic self-tests for scripts/disk-guard.sh (bead sq-4vo9m —
# the per-tick disk guard: df-check + merged-worktree prune + gated main-target
# reclaim).
#
# WHY: the guard's correctness rests on a few load-bearing invariants that a future
# refactor must not silently break, so we pin them with a hermetic harness (the
# pure-predicate self-test inside the script covers the threshold math; THIS harness
# covers the END-TO-END behaviour under stubbed df/pgrep, where the dangerous mutation
# — deleting the main checkout's target/ — actually lives):
#   1. ESCALATION IS GATED — when disk is CRITICAL, the main-checkout target/ is removed
#      ONLY with --apply AND --reclaim-main-target. CRITICAL alone, or --apply without the
#      opt-in flag, must KEEP it.
#   2. BUSY-BUILD GUARD — even CRITICAL + --apply + --reclaim-main-target must KEEP target/
#      if a live cargo/rustc process references the main checkout (removing it mid-build
#      would abort that build — the exact ENOSPC-adjacent failure mode the bead is about).
#   3. STATE → EXIT-CODE — OK⇒0, WARN⇒10, CRITICAL⇒20 (advisory signals a scheduler reads).
#   4. NON-FATAL DELEGATION — disk-guard always invokes the worktree prune and never aborts
#      its caller; an absent/erroring worktree-gc.sh degrades to a logged skip, not a crash.
#
# HERMETIC w.r.t. the real system: `df` and `pgrep` are PATH-shadowed by stubs, the main
# checkout + worktree root are scratch dirs under a mktemp sandbox, and worktree-gc.sh is
# replaced by a no-op stub next to the script copy. The harness never reads real disk free
# space, never touches a real worktree, and never deletes anything outside the sandbox.
#
# Run:  bash scripts/tests/test_disk_guard.sh   (exit 0 = all pass, 1 = a failure)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="${ROOT}/scripts/disk-guard.sh"
WT_GC_SRC="${ROOT}/scripts/worktree-gc.sh"

[ -f "$SRC" ] || { echo "FATAL: script not found at ${SRC}"; exit 2; }

pass=0
fail=0
note_pass() { pass=$((pass + 1)); }
note_fail() { fail=$((fail + 1)); printf 'CASE FAILED: %s\n' "$1"; }
check_rc() {  # check_rc <description> <expected-rc> <actual-rc>
  if [ "$3" = "$2" ]; then note_pass; else note_fail "$1 (wanted rc=$2, got rc=$3)"; fi
}

# --------------------------------------------------------------------------- #
# Sandbox: a scratch dir with a stub `df` + `pgrep` on PATH, a no-op worktree-gc
# stub sitting beside a COPY of disk-guard.sh (so its `dirname $0/worktree-gc.sh`
# resolves to the stub, not the real broom), and a scratch main checkout + target/.
# --------------------------------------------------------------------------- #
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT
BIN="${SANDBOX}/bin"
SCRIPTS="${SANDBOX}/scripts"
MAIN="${SANDBOX}/main"
WTROOT="${SANDBOX}/wtroot"
mkdir -p "$BIN" "$SCRIPTS" "$MAIN" "$WTROOT"

cp "$SRC" "${SCRIPTS}/disk-guard.sh"
GUARD="${SCRIPTS}/disk-guard.sh"

# No-op worktree-gc stub beside the copied guard — records that it was invoked.
WT_GC_LOG="${SANDBOX}/wtgc.log"
: >"$WT_GC_LOG"
cat >"${SCRIPTS}/worktree-gc.sh" <<EOF
#!/usr/bin/env bash
echo "INVOKED \$*" >>"${WT_GC_LOG}"
exit 0
EOF
chmod +x "${SCRIPTS}/worktree-gc.sh"

# Stub pgrep that reports NO matching build by default (idle box).
write_idle_pgrep() {
  cat >"${BIN}/pgrep" <<'EOF'
#!/usr/bin/env bash
exit 1   # no processes matched
EOF
  chmod +x "${BIN}/pgrep"
}
# Stub pgrep that reports a live cargo build referencing the sandbox main checkout.
write_busy_pgrep() {
  cat >"${BIN}/pgrep" <<EOF
#!/usr/bin/env bash
echo "4242 cargo build --manifest-path ${MAIN}/Cargo.toml"
exit 0
EOF
  chmod +x "${BIN}/pgrep"
}
# Stub df that reports a given GiB-free value (CRITICAL/WARN/OK as chosen by caller).
write_df_free() {  # write_df_free <free_gb>
  cat >"${BIN}/df" <<EOF
#!/usr/bin/env bash
echo "Filesystem 1G-blocks Used Available Use% Mounted on"
echo "stub 290G \$((290 - $1))G ${1}G $(( (290-$1)*100/290 ))% /"
EOF
  chmod +x "${BIN}/df"
}

mk_target() { rm -rf "${MAIN}/target"; mkdir -p "${MAIN}/target/debug"; : >"${MAIN}/target/debug/blob"; }
target_present() { [ -d "${MAIN}/target" ]; }

run_guard() {  # run_guard <expected-rc> <args...>; sets last_rc. (expected-rc is consumed by
               # the caller's check_rc; we drop it here so it is not passed to the guard.)
  shift
  set +e
  PATH="${BIN}:${PATH}" bash "$GUARD" --main "$MAIN" --root "$WTROOT" "$@" >/dev/null 2>&1
  last_rc=$?
  set -e
}

# --------------------------------------------------------------------------- #
# 1. CRITICAL + --apply but NO --reclaim-main-target ⇒ KEEP target/, exit 20.
# --------------------------------------------------------------------------- #
write_idle_pgrep; write_df_free 5; mk_target
run_guard 20 --apply
check_rc "CRITICAL + apply, no opt-in: exit 20" 20 "$last_rc"
if target_present; then note_pass; else note_fail "CRITICAL + apply, no opt-in: target/ wrongly removed"; fi

# --------------------------------------------------------------------------- #
# 2. CRITICAL + --apply + --reclaim-main-target + idle ⇒ REMOVE target/, exit 20.
# --------------------------------------------------------------------------- #
write_idle_pgrep; write_df_free 5; mk_target
run_guard 20 --apply --reclaim-main-target
check_rc "CRITICAL + apply + opt-in (idle): exit 20" 20 "$last_rc"
if target_present; then note_fail "CRITICAL + apply + opt-in (idle): target/ NOT reclaimed"; else note_pass; fi

# --------------------------------------------------------------------------- #
# 3. CRITICAL + --apply + --reclaim-main-target BUT a live build ⇒ KEEP target/.
#    (the busy-build guard — must never abort an in-flight build.)
# --------------------------------------------------------------------------- #
write_busy_pgrep; write_df_free 5; mk_target
run_guard 20 --apply --reclaim-main-target
check_rc "CRITICAL + apply + opt-in (BUSY): exit 20" 20 "$last_rc"
if target_present; then note_pass; else note_fail "CRITICAL + apply + opt-in (BUSY): target/ removed mid-build!"; fi

# --------------------------------------------------------------------------- #
# 4. CRITICAL DRY-RUN + opt-in ⇒ PLAN only, KEEP target/, exit 20.
# --------------------------------------------------------------------------- #
write_idle_pgrep; write_df_free 5; mk_target
run_guard 20 --reclaim-main-target
check_rc "CRITICAL dry-run + opt-in: exit 20" 20 "$last_rc"
if target_present; then note_pass; else note_fail "CRITICAL dry-run: target/ removed (must only PLAN)"; fi

# --------------------------------------------------------------------------- #
# 5. WARN (15G) ⇒ prune only, no escalation, exit 10, target/ untouched even with opt-in.
# --------------------------------------------------------------------------- #
write_idle_pgrep; write_df_free 15; mk_target
run_guard 10 --apply --reclaim-main-target
check_rc "WARN: exit 10" 10 "$last_rc"
if target_present; then note_pass; else note_fail "WARN: target/ removed (escalation must be CRITICAL-only)"; fi

# --------------------------------------------------------------------------- #
# 6. OK (200G) ⇒ exit 0, target/ untouched.
# --------------------------------------------------------------------------- #
write_idle_pgrep; write_df_free 200; mk_target
run_guard 0 --apply --reclaim-main-target
check_rc "OK: exit 0" 0 "$last_rc"
if target_present; then note_pass; else note_fail "OK: target/ removed (must only act when CRITICAL)"; fi

# --------------------------------------------------------------------------- #
# 7. NON-FATAL DELEGATION — the worktree prune is ALWAYS invoked (every tick), and
#    a worktree-gc that EXITS NON-ZERO does not crash the guard.
# --------------------------------------------------------------------------- #
: >"$WT_GC_LOG"
write_idle_pgrep; write_df_free 200; mk_target
run_guard 0 --dry-run
if grep -q '^INVOKED' "$WT_GC_LOG"; then note_pass; else note_fail "worktree prune was NOT invoked on an OK tick"; fi

# --------------------------------------------------------------------------- #
# 7b. COMPLETED-RECLAIM PASS-THROUGH (sq-h34dc) — by default disk-guard delegates with
#     --reclaim-completed (so the per-tick sweep also reclaims completed-but-unmerged workflow
#     worktrees); --no-reclaim-completed omits it (old MERGED/GONE-only behaviour).
# --------------------------------------------------------------------------- #
: >"$WT_GC_LOG"
write_idle_pgrep; write_df_free 200; mk_target
run_guard 0 --dry-run
if grep -q 'INVOKED.*--reclaim-completed' "$WT_GC_LOG"; then note_pass; else
  note_fail "default tick did NOT pass --reclaim-completed to worktree-gc.sh"; fi

: >"$WT_GC_LOG"
write_idle_pgrep; write_df_free 200; mk_target
run_guard 0 --dry-run --no-reclaim-completed
if grep -q '^INVOKED' "$WT_GC_LOG" && ! grep -q -- '--reclaim-completed' "$WT_GC_LOG"; then note_pass; else
  note_fail "--no-reclaim-completed still passed --reclaim-completed (or skipped the prune)"; fi

# erroring worktree-gc ⇒ guard still exits on the disk-state code (non-fatal delegation)
cat >"${SCRIPTS}/worktree-gc.sh" <<'EOF'
#!/usr/bin/env bash
echo "boom" >&2
exit 3
EOF
chmod +x "${SCRIPTS}/worktree-gc.sh"
write_idle_pgrep; write_df_free 200
run_guard 0 --apply
check_rc "erroring worktree-gc is non-fatal (exit on disk state)" 0 "$last_rc"
# restore the no-op stub
cat >"${SCRIPTS}/worktree-gc.sh" <<EOF
#!/usr/bin/env bash
echo "INVOKED \$*" >>"${WT_GC_LOG}"
exit 0
EOF
chmod +x "${SCRIPTS}/worktree-gc.sh"

# --------------------------------------------------------------------------- #
# 8. UNKNOWN df (unparseable) ⇒ exit 0 (a df we cannot read is NOT, by itself, a crisis),
#    no escalation, target/ untouched even with --apply --reclaim-main-target.
# --------------------------------------------------------------------------- #
cat >"${BIN}/df" <<'EOF'
#!/usr/bin/env bash
echo "Filesystem 1G-blocks Used Available Use% Mounted on"
echo "totally unparseable"
EOF
chmod +x "${BIN}/df"
write_idle_pgrep; mk_target
run_guard 0 --apply --reclaim-main-target
check_rc "UNKNOWN df: exit 0" 0 "$last_rc"
if target_present; then note_pass; else note_fail "UNKNOWN df: target/ removed (must not escalate)"; fi

# --------------------------------------------------------------------------- #
# 9. STATIC: bead invariants present in the source (a refactor cannot silently drop them).
# --------------------------------------------------------------------------- #
if grep -q 'main_target_build_busy' "$SRC"; then note_pass; else note_fail "busy-build guard removed from source"; fi
if grep -q 'RECLAIM_MAIN_TARGET' "$SRC";    then note_pass; else note_fail "main-target reclaim opt-in removed from source"; fi
if grep -q 'worktree-gc.sh' "$SRC";         then note_pass; else note_fail "worktree-gc delegation removed from source"; fi
# the guard must DELEGATE the safe predicate, not reimplement a worktree-removal predicate:
if grep -q 'merge-base --is-ancestor' "$SRC"; then
  note_fail "disk-guard reimplements the worktree safe predicate — it must DELEGATE to worktree-gc.sh"
else
  note_pass
fi

# 10. The real worktree-gc.sh still passes its OWN self-test (composition integrity).
if [ -f "$WT_GC_SRC" ]; then
  if bash "$WT_GC_SRC" --dry-run-self-test >/dev/null 2>&1; then note_pass; else note_fail "real worktree-gc.sh self-test failed"; fi
else
  note_fail "real worktree-gc.sh missing — disk-guard delegates to it"
fi

# --------------------------------------------------------------------------- #
echo ""
echo "test_disk_guard: ${pass} passed, ${fail} failed."
[ "$fail" -eq 0 ] || exit 1
echo "test_disk_guard: OK — gated escalation + busy-build guard + state→exit + non-fatal delegation hold."
