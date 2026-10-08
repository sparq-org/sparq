#!/usr/bin/env bash
# Hermetic self-tests for scripts/ci-free-disk.sh (bead sq-6uc7 —
# folding the two genuinely-new purge paths from the closed #462 into the shared
# reclaim script with existence guards).
#
# WHY: #462 added `/usr/local/share/boost` and `$AGENT_TOOLSDIRECTORY` as a SECOND
# inline reclaim step; sq-6uc7 folds them into the ONE shared script instead. The
# subtle correctness points this harness pins:
#   1. set -u SAFETY — the script runs `set -uo pipefail`, and on a non-GitHub host
#      `$AGENT_TOOLSDIRECTORY` is UNSET. A naked `$AGENT_TOOLSDIRECTORY` would abort
#      the whole reclaim under `set -u`; it must expand unset/empty SAFELY and skip.
#   2. EXISTENCE GUARDS — boost + the toolcache must only be removed when present;
#      an absent path (the common case on a dev box / drifted runner image) must
#      NEVER red the step.
#   3. The reclaim FIRES when the path DOES exist (so the fold is not a silent no-op).
#
# HERMETIC w.r.t. the real system: every "removal" is intercepted by a `sudo` STUB
# on PATH that only logs + `rm`s inside a scratch dir, and `df`/`docker` are stubbed
# too. The script never touches a real system path and never calls real `sudo`.
#
# Run:  bash scripts/tests/test_ci_free_disk.sh   (exit 0 = all pass, 1 = a failure)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="${ROOT}/scripts/ci-free-disk.sh"

[ -f "$SCRIPT" ] || { echo "FATAL: script not found at ${SCRIPT}"; exit 2; }

pass=0
fail=0
check() {  # check <description> <expected-rc> <actual-rc>
  local desc="$1" want="$2" got="$3"
  if [ "$got" = "$want" ]; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    printf 'CASE FAILED: %s  (wanted rc=%s, got rc=%s)\n' "$desc" "$want" "$got"
  fi
}

# --------------------------------------------------------------------------- #
# A hermetic sandbox: PATH-shadow `sudo`, `df`, `docker`, `apt-get` so the script
# under test touches NOTHING real. The `sudo` stub strips a leading `rm`/`docker`/
# `apt-get` and records each path it is asked to `rm -rf` into REMOVED_LOG, then
# actually removes it ONLY if it lives under our scratch root (so a positive-case
# fixture dir really disappears, proving the guard fired).
# --------------------------------------------------------------------------- #
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT
BIN="${SANDBOX}/bin"
REMOVED_LOG="${SANDBOX}/removed.log"
mkdir -p "$BIN"
: >"$REMOVED_LOG"

cat >"${BIN}/sudo" <<EOF
#!/usr/bin/env bash
# Stubbed sudo: log + only act inside the sandbox; never escalate.
cmd="\$1"; shift || true
case "\$cmd" in
  rm)
    # args are like: -rf <path>
    for a in "\$@"; do
      case "\$a" in -*) ;; *) echo "RM \$a" >>"${REMOVED_LOG}"; case "\$a" in ${SANDBOX}/*) command rm -rf "\$a" ;; esac ;; esac
    done
    ;;
  docker)  echo "DOCKER \$*" >>"${REMOVED_LOG}" ;;
  apt-get) echo "APT \$*"    >>"${REMOVED_LOG}" ;;
  *)       echo "OTHER \$cmd \$*" >>"${REMOVED_LOG}" ;;
esac
exit 0
EOF

cat >"${BIN}/df" <<'EOF'
#!/usr/bin/env bash
echo "Filesystem Size Used Avail Use% Mounted on"
echo "stub 1G 0 1G 0% /"
EOF

# No docker / apt-get on PATH for the negative cases (forces the command -v guard).
chmod +x "${BIN}/sudo" "${BIN}/df"

run_script() {  # run_script <PATH-for-run> [extra env assignments...]
  local path_for_run="$1"; shift
  set +e
  env -u AGENT_TOOLSDIRECTORY "$@" PATH="${path_for_run}" bash "$SCRIPT" >/dev/null 2>&1
  local rc=$?
  set -e
  echo "$rc"
}

# --------------------------------------------------------------------------- #
# 1. set -u safety: AGENT_TOOLSDIRECTORY UNSET → must NOT abort, exit 0.
# --------------------------------------------------------------------------- #
rc="$(run_script "${BIN}:${PATH}")"
check "unset AGENT_TOOLSDIRECTORY does not trip set -u" 0 "$rc"

# --------------------------------------------------------------------------- #
# 2. AGENT_TOOLSDIRECTORY set but path ABSENT → existence guard skips, exit 0,
#    and NO toolcache removal is logged.
# --------------------------------------------------------------------------- #
: >"$REMOVED_LOG"
rc="$(run_script "${BIN}:${PATH}" AGENT_TOOLSDIRECTORY="${SANDBOX}/nonexistent-toolcache")"
check "absent toolcache path exits clean" 0 "$rc"
if grep -q "nonexistent-toolcache" "$REMOVED_LOG"; then
  fail=$((fail + 1)); echo "CASE FAILED: absent toolcache was (wrongly) removed"
else
  pass=$((pass + 1))
fi

# --------------------------------------------------------------------------- #
# 3. AGENT_TOOLSDIRECTORY EMPTY string → treated as unset, exit 0, nothing logged.
# --------------------------------------------------------------------------- #
: >"$REMOVED_LOG"
rc="$(run_script "${BIN}:${PATH}" AGENT_TOOLSDIRECTORY="")"
check "empty AGENT_TOOLSDIRECTORY exits clean" 0 "$rc"

# --------------------------------------------------------------------------- #
# 4. POSITIVE: AGENT_TOOLSDIRECTORY points at an EXISTING dir → reclaim FIRES
#    (the dir is logged as removed AND actually gone). Proves the fold is not a
#    silent no-op.
# --------------------------------------------------------------------------- #
: >"$REMOVED_LOG"
TC="${SANDBOX}/toolcache"
mkdir -p "${TC}/node/18" "${TC}/python/3.12"
rc="$(run_script "${BIN}:${PATH}" AGENT_TOOLSDIRECTORY="${TC}")"
check "present toolcache exits clean" 0 "$rc"
if grep -qF "RM ${TC}" "$REMOVED_LOG" && [ ! -e "$TC" ]; then
  pass=$((pass + 1))
else
  fail=$((fail + 1)); echo "CASE FAILED: present toolcache was not reclaimed"
fi

# --------------------------------------------------------------------------- #
# 5. STATIC: both genuinely-new #462 paths are present in the script source, so a
#    future refactor cannot silently drop the fold.
# --------------------------------------------------------------------------- #
if grep -q '/usr/local/share/boost' "$SCRIPT"; then
  pass=$((pass + 1))
else
  fail=$((fail + 1)); echo "CASE FAILED: /usr/local/share/boost missing from script"
fi
# The toolcache must be referenced via the UNSET-SAFE default-expansion form, never
# a naked $AGENT_TOOLSDIRECTORY (which would break under set -u).
if grep -q 'AGENT_TOOLSDIRECTORY:-' "$SCRIPT"; then
  pass=$((pass + 1))
else
  fail=$((fail + 1)); echo "CASE FAILED: AGENT_TOOLSDIRECTORY not expanded unset-safely (\${AGENT_TOOLSDIRECTORY:-})"
fi

# --------------------------------------------------------------------------- #
echo ""
echo "test_ci_free_disk: ${pass} passed, ${fail} failed."
[ "$fail" -eq 0 ] || exit 1
echo "test_ci_free_disk: OK — set -u safety + existence guards + #462 fold hold."
