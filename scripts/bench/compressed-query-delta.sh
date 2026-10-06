#!/usr/bin/env bash
# sq-7d3dj.32.2.2 — compressed-query-delta harness:
# per-query latency comparison raw vs SPARQ_STORE_PROFILE=compressed on the
# WatDiv suite (bench/watdiv/queries, gen.sh corpus), emitting the house JSON
# envelope with per-query best-of-N rows.
#
# Methodology:
#   * Build sparq-cli once (release, default features).
#   * Generate (or reuse cached) the WatDiv SF corpus via bench/watdiv/gen.sh.
#   * Run `sparq-cli bench` TWICE with the SAME command line — once WITHOUT
#     SPARQ_STORE_PROFILE (raw six-permutation layout) and once WITH
#     SPARQ_STORE_PROFILE=compressed (block-compressed perms + blob dict via
#     Graph::into_compressed, wired by sq-7d3dj.32.2.1).
#   * CORRECTNESS INVARIANT: cross-check EVERY query's solution count in BOTH
#     modes against bench/watdiv/expected-rows.tsv; a mismatch FAILS the run
#     (exit 1). This is an answer-safety check, not a perf claim.
#   * Emit one JSON envelope on stdout (format below) + write to OUT if given.
#
# Envelope schema (a SUBSET of the house format; fully self-describing):
#   { instrument, version, sha, date, machine, corpus,
#     rows: [{ query, rows, raw_us, comp_us }] }
#
# NON-CANONICAL NOTE: timings from the shared work box are NON-canonical (box
# is shared, load-variable). Canonical runs belong on a dedicated quiet EC2
# instance (bench/ec2-bench.sh); this script is the durable, reusable harness.
# Do NOT commit timing numbers from this box to markdown.
#
# Usage:
#   scripts/bench/compressed-query-delta.sh [out.json]
#
# Tunables (env):
#   SF         WatDiv scale factor          (default 1; SF=1 -> ~106k triples)
#   ITERS      best-of-N per query/mode     (default 3)
#   WATDIV_CACHE  corpus cache dir          (default /tmp/watdiv)
#   KEEP_CORPUS   1 = keep corpus after run (default 0)
#
# Disk discipline: check free space before corpus generation; corpus is
# gitignored and regenerable; /tmp scratch is cleaned on exit.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

SF="${SF:-1}"
ITERS="${ITERS:-3}"
CACHE="${WATDIV_CACHE:-/tmp/watdiv}"
KEEP_CORPUS="${KEEP_CORPUS:-0}"

CLI="$ROOT/target/release/sparq-cli"
QDIR="$ROOT/bench/watdiv/queries"
EXP="$ROOT/bench/watdiv/expected-rows.tsv"
OUT="${1:-}"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

log() { echo "[cqd] $*" >&2; }

# ---- 0. disk guard -------------------------------------------------------------------
free_bytes() { df --output=avail -B1 "$1" 2>/dev/null | tail -1 | tr -d ' '; }
NEED=$(( 106000 * SF * 250 ))   # corpus ~115 B/triple * SF triples; extra headroom
FREE="$(free_bytes "${CACHE%/*}" || echo 0)"
mkdir -p "$CACHE"
if [ "${FREE:-0}" -lt "$NEED" ]; then
  log "FATAL: need ~$NEED bytes free for SF=$SF corpus, have $FREE"
  exit 1
fi

# ---- 1. build sparq-cli (release) ----------------------------------------------------
log "building sparq-cli (release)..."
( cd "$ROOT" && cargo build --release -p sparq-cli 2>&1 | tail -3 >&2 )
[ -x "$CLI" ] || { log "FATAL: $CLI missing after build"; exit 1; }

# ---- 2. generate WatDiv corpus -------------------------------------------------------
log "generating WatDiv SF=$SF corpus (deterministic seed-1)..."
CORPUS="$(bash "$ROOT/bench/watdiv/gen.sh" "$SF")"
[ -s "$CORPUS" ] || { log "FATAL: corpus not produced"; exit 1; }
log "corpus: $CORPUS ($(wc -l < "$CORPUS") triples)"

# ---- 3. run sparq-cli bench in RAW mode ----------------------------------------------
log "raw mode: sparq-cli bench (iters=$ITERS)..."
RAW_TSV="$SCRATCH/raw.tsv"
SPARQ_STORE_PROFILE="" "$CLI" bench "$CORPUS" ntriples "$QDIR" "$ITERS" count \
  2>/dev/null > "$RAW_TSV" || true

# ---- 4. run sparq-cli bench in COMPRESSED mode ---------------------------------------
log "compressed mode: sparq-cli bench (iters=$ITERS)..."
COMP_TSV="$SCRATCH/comp.tsv"
SPARQ_STORE_PROFILE=compressed "$CLI" bench "$CORPUS" ntriples "$QDIR" "$ITERS" count \
  2>/dev/null > "$COMP_TSV" || true

# ---- 5. cross-check counts against expected-rows.tsv in BOTH modes ------------------
log "cross-checking counts against $EXP in both modes..."
fail=0

check_mode() {
  local label="$1"; local tsv="$2"
  while IFS=$'\t' read -r q exp; do
    case "$q" in '#'*|"") continue;; esac
    got_raw="$(awk -F'\t' -v k="$q" '$1==k{print $2}' "$tsv" 2>/dev/null || echo MISSING)"
    if [ "${got_raw:-MISSING}" != "$exp" ]; then
      log "ERROR [$label] $q rows=${got_raw:-MISSING} expected=$exp"
      fail=1
    fi
  done < "$EXP"
}

check_mode "raw" "$RAW_TSV"
check_mode "compressed" "$COMP_TSV"

if [ "$fail" -ne 0 ]; then
  log "FATAL: row-count mismatch in one or more modes — check output above"
  exit 1
fi
log "OK — all expected-rows matched in both modes"

# ---- 6. build result rows for envelope -----------------------------------------------
# TSV format: <name>\t<rows>\t<min_us>
# The expected-rows.tsv drives the query list (only per-commit Basic-Testing queries).
ROWS_JSON="$(python3 - "$EXP" "$RAW_TSV" "$COMP_TSV" <<'PY'
import sys, json

exp_file, raw_file, comp_file = sys.argv[1], sys.argv[2], sys.argv[3]

def read_tsv(path):
    out = {}
    try:
        for line in open(path).read().splitlines():
            if not line.strip() or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) >= 3:
                out[parts[0]] = {"rows": parts[1], "us": parts[2]}
    except FileNotFoundError:
        pass
    return out

raw = read_tsv(raw_file)
comp = read_tsv(comp_file)

queries = []
for line in open(exp_file).read().splitlines():
    if not line.strip() or line.startswith("#"):
        continue
    queries.append(line.split("\t")[0])

rows = []
for q in queries:
    r = raw.get(q, {})
    c = comp.get(q, {})
    try:
        row_count = int(r.get("rows", c.get("rows", 0)))
    except (ValueError, TypeError):
        row_count = 0
    try:
        raw_us = round(float(r.get("us", 0)))
    except (ValueError, TypeError):
        raw_us = 0
    try:
        comp_us = round(float(c.get("us", 0)))
    except (ValueError, TypeError):
        comp_us = 0
    rows.append({"query": q, "rows": row_count, "raw_us": raw_us, "comp_us": comp_us})

print(json.dumps(rows, separators=(",", ":")))
PY
)"

# ---- 7. emit JSON envelope -----------------------------------------------------------
CORPUS_LABEL="watdiv-sf${SF}-seed1"
SHA="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
DATE_NOW="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
MACHINE="$(uname -m)"
VERSION="$(cd "$ROOT" && cargo metadata --no-deps --format-version 1 2>/dev/null \
  | python3 -c 'import json,sys; ws=json.load(sys.stdin)["packages"]; \
    p=[x for x in ws if x["name"]=="sparq-cli"]; print(p[0]["version"] if p else "unknown")' \
  2>/dev/null || echo unknown)"

ENV_JSON="$(python3 - "$SHA" "$DATE_NOW" "$MACHINE" "$CORPUS_LABEL" "$VERSION" "$ROWS_JSON" <<'PY'
import json, sys
sha, date, machine, corpus, version, rows_json = sys.argv[1:7]
print(json.dumps({
    "instrument": "compressed-query-delta",
    "version": 1,
    "sha": sha,
    "date": date,
    "machine": machine,
    "corpus": corpus,
    "sparq_version": version,
    "rows": json.loads(rows_json),
}, separators=(",", ":")))
PY
)"

echo "$ENV_JSON"
[ -n "$OUT" ] && printf '%s\n' "$ENV_JSON" > "$OUT"

# ---- 8. corpus cleanup ---------------------------------------------------------------
[ "$KEEP_CORPUS" = "1" ] || rm -f "$CORPUS"

log "done."
