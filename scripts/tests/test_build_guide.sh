#!/usr/bin/env bash
# [OPUS-5] issue #5022 — hermetic both-direction self-test for scripts/build-guide.sh.
#
# WHY: build-guide.sh exists for ONE non-obvious reason — mdBook EXITS 0 on a broken
# {{#include}} / missing anchor (rust-lang/mdBook#1094) and only says so on stderr as an
# `[ERROR]` line. The whole guide is {{#include}} wrappers around canonical README /
# SKILL.md anchors, and pages.yml now PUBLISHES that render, so if the log-grep teeth ever
# regress, an anchor rename ships a guide full of raw `{{#include ...}}` directives with a
# green CI. The teeth are therefore the thing worth pinning, in both directions:
#   1. TEETH   — an mdbook run that exits 0 but logs [ERROR] MUST fail the script.
#   2. NO FALSE POSITIVE — a clean run MUST pass (a false positive blocks every Pages deploy).
# Plus the two structural failures the publish path depends on: a non-zero mdbook exit must
# propagate through the `| tee` (pipefail), and a run that renders no index.html must fail.
#
# HERMETIC: drives build-guide.sh against a STUB `mdbook` on PATH (a few lines of shell) over
# a throwaway book dir. No real mdbook, no network, no repo content — so this runs anywhere
# and pins the script's own logic rather than mdBook's behaviour.
#
# Run:  bash scripts/tests/test_build_guide.sh   (exit 0 = pass, 1 = a case failed)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="${ROOT}/scripts/build-guide.sh"

[ -f "$SCRIPT" ] || { echo "FATAL: builder not found at ${SCRIPT}"; exit 2; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

failures=0

# Install a stub `mdbook` whose behaviour is controlled by two env vars:
#   STUB_LOG_ERROR=1 -> emit an mdBook-shaped "[ERROR]" line but still exit 0 (the #1094 shape)
#   STUB_EXIT=<n>    -> exit with <n> (default 0)
#   STUB_RENDER=0    -> do not write the rendered index.html
mkdir -p "${TMP}/bin"
cat > "${TMP}/bin/mdbook" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
book_dir="${2:-book}"
echo "[INFO] (mdbook::book): Book building has started"
if [ "${STUB_LOG_ERROR:-0}" = "1" ]; then
  # Exactly the shape mdBook emits for an unresolvable include — note the exit 0 below.
  echo "[ERROR] (mdbook::preprocess::links): Error found: Could not find anchor 'lead'"
fi
if [ "${STUB_RENDER:-1}" = "1" ]; then
  mkdir -p "${book_dir}/book"
  echo "<html><!-- Book generated using mdBook --></html>" > "${book_dir}/book/index.html"
fi
exit "${STUB_EXIT:-0}"
STUB
chmod +x "${TMP}/bin/mdbook"

# A minimal book source dir (build-guide.sh only requires book.toml to exist).
BOOK="${TMP}/book"
mkdir -p "${BOOK}"
printf '[book]\ntitle = "t"\n[build]\nbuild-dir = "book"\n' > "${BOOK}/book.toml"

run_case() {
  # run_case <label> <expect: pass|fail> <env assignments...>
  local label="$1" expect="$2"; shift 2
  rm -rf "${BOOK}/book"
  local rc=0
  env PATH="${TMP}/bin:${PATH}" "$@" bash "$SCRIPT" "$BOOK" >"${TMP}/out.log" 2>&1 || rc=$?
  local got="pass"; [ "$rc" -eq 0 ] || got="fail"
  if [ "$got" = "$expect" ]; then
    echo "  [PASS] ${label}: ${got} (rc=${rc})"
  else
    echo "  [FAIL] ${label}: got ${got} (rc=${rc}), want ${expect}"
    sed 's/^/         | /' "${TMP}/out.log"
    failures=$((failures + 1))
  fi
}

echo "test_build_guide: driving scripts/build-guide.sh against a stub mdbook"

# 1. NO FALSE POSITIVE — a clean build passes.
run_case "clean build passes" pass STUB_LOG_ERROR=0

# 2. THE TEETH — mdBook's exit-0-with-[ERROR] (rust-lang/mdBook#1094) must FAIL.
#    This is the case the script exists for; if it ever goes green, delete the script.
run_case "[ERROR] line with exit 0 fails (mdBook#1094)" fail STUB_LOG_ERROR=1

# 3. A genuinely non-zero mdbook exit must propagate through the `| tee` (needs pipefail).
run_case "non-zero mdbook exit propagates through tee" fail STUB_EXIT=1

# 4. A build that renders nothing must fail (publishing an empty /guide/ is worse than red CI).
run_case "missing rendered index.html fails" fail STUB_RENDER=0

# 5. A missing book.toml is a clear error, not a silent success.
rc=0
env PATH="${TMP}/bin:${PATH}" bash "$SCRIPT" "${TMP}/not-a-book" >/dev/null 2>&1 || rc=$?
if [ "$rc" -ne 0 ]; then
  echo "  [PASS] missing book.toml fails (rc=${rc})"
else
  echo "  [FAIL] missing book.toml: got pass, want fail"
  failures=$((failures + 1))
fi

# 6. No mdbook on PATH → 127 with an actionable message, never a silent pass.
rc=0
env PATH="/nonexistent" bash "$SCRIPT" "$BOOK" >/dev/null 2>&1 || rc=$?
if [ "$rc" -eq 127 ]; then
  echo "  [PASS] absent mdbook exits 127"
else
  echo "  [FAIL] absent mdbook: got rc=${rc}, want 127"
  failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
  echo ""
  echo "test_build_guide: ${failures} case(s) FAILED"
  exit 1
fi
echo ""
echo "test_build_guide: all cases PASS"
