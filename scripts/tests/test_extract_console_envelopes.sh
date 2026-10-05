#!/usr/bin/env bash
# Fixture tests for scripts/bench/extract-console-envelopes.sh — the
# serial-console envelope recovery channel of the canonical gathers (PR #3488
# review round 1). The instance scripts emit `===ENVELOPE-BEGIN <name>.json===`
# with NO space before the closing ===; a naive awk `name=$2` therefore recovered
# `<name>.json===` files, which the launcher's later *.json listing silently
# dropped — breaking the advertised authoritative recovery channel exactly when
# SSH is down. This harness pins:
#   1. the exact .json basename is recovered, with byte-identical valid JSON;
#   2. multiple envelope blocks in one console dump are all recovered;
#   3. CRLF serial-console line endings are stripped;
#   4. slashed / dotfile / garbled marker names are REJECTED (nothing written);
#   5. STATIC: the instance emitter still uses this exact marker format, so the
#      emitter and this extractor cannot silently drift apart.
# Hermetic: pure text fixtures in a scratch dir; no AWS, no network.
# Run: bash scripts/tests/test_extract_console_envelopes.sh   (exit 0 = all pass)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXTRACT="$ROOT/scripts/bench/extract-console-envelopes.sh"
EMITTER="$ROOT/scripts/bench/canonical-beir-gather-instance.sh"
[ -f "$EXTRACT" ] || { echo "FATAL: $EXTRACT not found"; exit 2; }

pass=0; fail=0
ok()  { pass=$((pass + 1)); }
bad() { fail=$((fail + 1)); printf 'CASE FAILED: %s\n' "$1"; }

SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT
CON="$SANDBOX/console.txt"
OUT="$SANDBOX/out"

PAYLOAD='{"engine": "lucene-anserini", "suite": "beir-ir-scifact", "values": {"recall_100_deficit_milli": 0}}'
NAME1="lucene-anserini-beir-ir-scifact-20260701T000000Z.json"
NAME2="sparq-text-beir-ir-scifact-20260701T000001Z.json"

# ---- 1+2. exact filenames + byte-identical JSON, two blocks, console noise ----------
{
  echo "[STEP 2026-07-01T00:00:00Z] envelopes in /root/sparq/bench/competitor-results:"
  echo "===ENVELOPE-BEGIN ${NAME1}==="
  echo "$PAYLOAD"
  echo "===ENVELOPE-END==="
  echo "unrelated console noise"
  echo "===ENVELOPE-BEGIN ${NAME2}==="
  echo "$PAYLOAD"
  echo "===ENVELOPE-END==="
} > "$CON"
bash "$EXTRACT" "$CON" "$OUT"
for n in "$NAME1" "$NAME2"; do
  if [ -f "$OUT/$n" ]; then ok; else bad "expected exact recovered name $n (got: $(ls "$OUT" 2>/dev/null | tr '\n' ' '))"; fi
done
if [ "$(cat "$OUT/$NAME1" 2>/dev/null)" = "$PAYLOAD" ]; then ok; else bad "recovered $NAME1 is not byte-identical to the emitted payload"; fi
if python3 -c 'import json, sys; json.load(open(sys.argv[1]))' "$OUT/$NAME1" 2>/dev/null; then ok; else bad "recovered $NAME1 is not valid JSON"; fi
if ls "$OUT"/*'===' >/dev/null 2>&1; then bad "a recovered file kept the === delimiter in its name"; else ok; fi

# ---- 3. CRLF serial-console line endings are stripped -------------------------------
rm -rf "$OUT"
printf '===ENVELOPE-BEGIN %s===\r\n%s\r\n===ENVELOPE-END===\r\n' "$NAME1" "$PAYLOAD" > "$CON"
bash "$EXTRACT" "$CON" "$OUT"
if [ -f "$OUT/$NAME1" ] && ! grep -q $'\r' "$OUT/$NAME1"; then ok; else bad "CRLF console block not recovered CR-free as $NAME1"; fi
if python3 -c 'import json, sys; json.load(open(sys.argv[1]))' "$OUT/$NAME1" 2>/dev/null; then ok; else bad "CRLF-recovered $NAME1 is not valid JSON"; fi

# ---- 4. hostile / garbled names are rejected, nothing written -----------------------
rm -rf "$OUT"
{
  printf '===ENVELOPE-BEGIN %s===\n{}\n===ENVELOPE-END===\n' "/etc/hostile.json"
  printf '===ENVELOPE-BEGIN %s===\n{}\n===ENVELOPE-END===\n' "../traversal.json"
  printf '===ENVELOPE-BEGIN %s===\n{}\n===ENVELOPE-END===\n' ".dotfile.json"
  printf '===ENVELOPE-BEGIN %s===\n{}\n===ENVELOPE-END===\n' "not-json.txt"
  printf '===ENVELOPE-BEGIN garbled with spaces.json===\n{}\n===ENVELOPE-END===\n'
} > "$CON"
bash "$EXTRACT" "$CON" "$OUT" 2>/dev/null
if [ -z "$(ls -A "$OUT" 2>/dev/null)" ]; then ok; else bad "rejected names still produced files: $(ls -A "$OUT" | tr '\n' ' ')"; fi
if [ ! -e "$SANDBOX/traversal.json" ] && [ ! -e /etc/hostile.json ]; then ok; else bad "a rejected name escaped the out-dir"; fi

# ---- 5. STATIC: emitter format lock (instance script still emits <name>=== markers) --
if grep -qF 'echo "===ENVELOPE-BEGIN $(basename "$f")==="' "$EMITTER"; then
  ok
else
  bad "emitter marker format changed in $EMITTER — update the extractor + this test together"
fi

echo ""
echo "test_extract_console_envelopes: ${pass} passed, ${fail} failed."
[ "$fail" -eq 0 ] || exit 1
echo "test_extract_console_envelopes: OK — exact names, CRLF strip, allowlist, format lock."
