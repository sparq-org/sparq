#!/usr/bin/env bash
# sq-tvzyi — recover result envelopes from an EC2 serial-console dump.
#
# 🤖 SPARQ agent. The canonical gather instance scripts cat every results envelope
# into the console log between
#   ===ENVELOPE-BEGIN <basename>.json===   /   ===ENVELOPE-END===
# markers (NO space before the closing ===), so `aws ec2 get-console-output` is the
# authoritative recovery channel when the SSH pull path dies. This extractor parses
# those blocks back into files. Load-bearing details:
#   * the trailing === delimiter is STRIPPED from the BEGIN marker, so recovered
#     files keep their exact `*.json` basenames (a naive `name=$2` kept the ===,
#     producing `<name>.json===` files the launcher's *.json listing then dropped);
#   * serial-console CR line endings are stripped;
#   * the recovered name is ALLOWLISTED (plain alnum/._- basename ending in .json,
#     no leading dot/dash, no slashes) — console content is instance-controlled, so
#     a garbled or hostile marker must never become a path or an odd filename.
#
# Usage: extract-console-envelopes.sh <console.txt> <out-dir>
# Fixture-tested by scripts/tests/test_extract_console_envelopes.sh.
set -euo pipefail

CONSOLE="${1:?usage: extract-console-envelopes.sh <console.txt> <out-dir>}"
OUTDIR="${2:?usage: extract-console-envelopes.sh <console.txt> <out-dir>}"
[ -f "$CONSOLE" ] || { echo "extract-console-envelopes: no such file: $CONSOLE" >&2; exit 1; }
mkdir -p "$OUTDIR"

awk -v dir="$OUTDIR" '
  { sub(/\r$/, "") }
  /===ENVELOPE-BEGIN .+===/ {
    name = $0
    sub(/.*===ENVELOPE-BEGIN /, "", name)
    sub(/===.*$/, "", name)
    if (name ~ /^[A-Za-z0-9][A-Za-z0-9._-]*\.json$/) {
      f = dir "/" name
      capturing = 1
    } else {
      printf "extract-console-envelopes: SKIP unexpected envelope name: %s\n", name > "/dev/stderr"
      capturing = 0
    }
    next
  }
  /===ENVELOPE-END===/ { if (capturing) close(f); capturing = 0; next }
  capturing { print > f }
' "$CONSOLE"
