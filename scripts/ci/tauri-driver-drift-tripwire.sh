#!/usr/bin/env bash
# sq-t6492 — tauri-driver ↔ WebKitGTK capability DRIFT TRIPWIRE + regression guard
#
# The native GUI smoke (nightly-full-sweep.yml `tauri-smoke`, gui.yml `tauri-e2e`) launches the
# Tauri shell through `tauri-driver`, which proxies to the runner's WebKitWebDriver (from the
# `webkit2gtk-driver` apt package). The WebDriver server and the app's webview come from the SAME
# runner-image apt version of webkit2gtk, so they are internally consistent — the drift that broke
# the lane is between the CLIENT capability shape (gui/e2e/run-e2e.mjs) and the WebKitGTK the runner
# ships, which GitHub bumps on ITS schedule, not ours.
#
# CONFIRMED ROOT CAUSE (issue #1740, bead sq-t6492; WebKit source-verified):
#   WebKit's matchCapabilities (Source/WebDriver/WebDriverService.cpp) rejects a session with
#   "session not created: Failed to match capabilities" whenever a NON-NULL `browserName` capability
#   is supplied that does not equal the driver's own reported browser name (case-insensitive). Newer
#   WebKitGTK (the runner's webkit2gtk 2.5x) reports `browserName: "MiniBrowser"`. The historic
#   tauri#8828 workaround sent `browserName: "wry"`, so once WebKitGTK began populating that field
#   `"wry" != "MiniBrowser"` → HARD REJECT during POST /session, BEFORE WebKitWebDriver logs a
#   session → the driver log is 0 bytes. tauri-driver 2.0.6 forwards the client browserName verbatim
#   on Linux (it only rewrites to "webview2" on Windows), so the fix is CLIENT-SIDE: OMIT
#   browserName entirely (the guard only fires when a MISMATCHING name is supplied, so omitting it
#   matches on both old and new WebKitGTK). See tauri-apps/tauri#8828, #10670, SeleniumHQ/selenium#10178.
#
# THIS SCRIPT DOES TWO THINGS:
#   1. REGRESSION GUARD (primary): fail if gui/e2e/run-e2e.mjs re-introduces `browserName: "wry"`
#      (or any hard-coded browserName), which would reinstate the exact #1740 failure. This is the
#      one deterministic check that should NEVER pass silently, so it exits non-zero on a hit.
#   2. VERSION TRIPWIRE (diagnostic): log the webkit2gtk + tauri-driver versions so the NEXT drift
#      is a one-line read instead of a 0-byte-log hunt, and surface a loud annotation if a future
#      WebKitGTK/tauri-driver combination looks suspicious. Diagnostic only — does not gate.
#
# Usage:
#   scripts/ci/tauri-driver-drift-tripwire.sh [--strict] [--tauri-driver-version X.Y.Z] [--harness PATH]
# --strict            also exit non-zero on the version-tripwire suspicion (unused today; reserved
#                     for a future gating promotion). The regression guard ALWAYS exits non-zero.
# --harness PATH      path to run-e2e.mjs (default: resolved relative to this script).
# --tauri-driver-version omit to read from the installed `tauri-driver` binary.

set -euo pipefail

STRICT=0
TAURI_DRIVER_VERSION=""
HARNESS=""

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# repo-root/scripts/ci/… → repo-root is two levels up.
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --strict) STRICT=1; shift ;;
    --tauri-driver-version) TAURI_DRIVER_VERSION="${2:-}"; shift 2 ;;
    --tauri-driver-version=*) TAURI_DRIVER_VERSION="${1#*=}"; shift ;;
    --harness) HARNESS="${2:-}"; shift 2 ;;
    --harness=*) HARNESS="${1#*=}"; shift ;;
    *) echo "tauri-drift-tripwire: unknown arg: $1" >&2; exit 2 ;;
  esac
done

[ -n "$HARNESS" ] || HARNESS="$REPO_ROOT/gui/e2e/run-e2e.mjs"

# GitHub Actions annotation helpers degrade to plain echo when run locally (no $GITHUB_ACTIONS).
notice()  { if [ "${GITHUB_ACTIONS:-}" = "true" ]; then echo "::notice::$*";  else echo "NOTICE: $*";  fi; }
error()   { if [ "${GITHUB_ACTIONS:-}" = "true" ]; then echo "::error::$*";   else echo "ERROR: $*";   fi; }

echo "── tauri-driver drift tripwire + regression guard (bead sq-t6492 / issue #1740) ──────────"

# --- 1. REGRESSION GUARD: the client must NOT hard-code a mismatching browserName ---------------
# The failure is fully attributable to `browserName: "wry"` in the capabilities. Guard against its
# return. We match a browserName key set to any quoted string inside the capabilities object; the
# fix is to omit browserName, so ANY hard-coded browserName is treated as the regression.
if [ ! -f "$HARNESS" ]; then
  error "e2e harness not found at $HARNESS — cannot run the browserName regression guard (bead sq-t6492)."
  exit 1
fi

# Strip line comments so the explanatory '// DO NOT re-add browserName: "wry"' note is not a hit,
# then look for an ACTIVE `browserName:` capability assignment.
if grep -vE '^\s*//' "$HARNESS" | grep -qE 'browserName\s*:\s*["'\'']'; then
  OFFENDING="$(grep -nvE '^\s*//' "$HARNESS" | grep -E 'browserName\s*:\s*["'\'']' | head -1)"
  error "REGRESSION (bead sq-t6492 / issue #1740): gui/e2e/run-e2e.mjs sets a hard-coded browserName \
capability — this reinstates the WebKitGTK 'Failed to match capabilities' failure (WebKit rejects a \
supplied browserName that != the driver's 'MiniBrowser'). OMIT browserName from the capabilities. \
Offending line: ${OFFENDING}"
  exit 1
fi
echo "  regression guard : PASS — run-e2e.mjs does not hard-code a browserName capability"

# --- 2. VERSION TRIPWIRE (diagnostic) ----------------------------------------------------------

WEBKIT_APT_FULL="$(dpkg-query -W -f='${Version}' webkit2gtk-driver 2>/dev/null || echo "")"
WEBKIT_VER="$(printf '%s' "$WEBKIT_APT_FULL" | grep -oE '^[0-9]+\.[0-9]+\.[0-9]+' || echo "")"
WKWD_BANNER="$(WebKitWebDriver --version 2>&1 | head -1 || echo "unknown")"

if [ -z "$TAURI_DRIVER_VERSION" ]; then
  TAURI_DRIVER_VERSION="$(tauri-driver --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || echo "")"
fi

echo "  tauri-driver     : ${TAURI_DRIVER_VERSION:-unknown}"
echo "  webkit2gtk-driver: ${WEBKIT_APT_FULL:-unknown}  (parsed: ${WEBKIT_VER:-unknown})"
echo "  WebKitWebDriver  : ${WKWD_BANNER}"
echo "───────────────────────────────────────────────────────────────────────────────────────"

# Informational: WebKitGTK >= 2.48 is where the browserName reporting / driver-name tightening
# lands (the class that made the old "wry" capability fatal). With browserName now OMITTED this is
# expected to be FINE — but log it so that if the smoke ever fails on capability negotiation again,
# the version is right there and the next drift is a one-line diagnosis.
suspect=0
case "$WEBKIT_VER" in
  2.4[89].*|2.5[0-9].*|2.[6-9][0-9].*|3.*) suspect=1 ;;
esac

if [ "$suspect" = "1" ] && [ -n "$WEBKIT_VER" ]; then
  MSG="webkit2gtk ${WEBKIT_VER} is in the range (>=2.48) where WebKitWebDriver enforces browserName \
matching (bead sq-t6492 / issue #1740). The harness correctly OMITS browserName, so this should \
pass; if the smoke still fails on 'Failed to match capabilities', the WebKitGTK capability shape \
changed again — inspect gui/e2e/run-e2e.mjs capabilities and update this tripwire. (The 0-byte \
driver log is expected on such a failure — it precedes any WebDriver session.)"
  if [ "$STRICT" = "1" ]; then
    error "$MSG"; exit 1
  fi
  notice "$MSG"
fi

echo "tauri-driver drift tripwire: regression guard passed; versions logged."
exit 0
