#!/usr/bin/env bash
# sq-7d3dj.34 — dedicated same-box Oxigraph SERVER load->serve->query->teardown
# recipe: the prebuilt, sha256-pinned Oxigraph CLI in `serve` mode, so the HTTP panel has a
# clean Rust apples-to-apples column next to sparq-server (the CLI matrix already has the
# in-process Oxigraph column; this is its HTTP twin).
#
#   1. `oxigraph load  --location <tmp store> --file <corpus>`   (offline bulk load, timed)
#   2. `oxigraph serve --location <tmp store> --bind 127.0.0.1:PORT`  (SPARQL at /query)
#   3. query via the SHARED scripts/bench-adapters/http_sparql_adapter.py
#   4. EXIT trap kills the server + removes the store. Every wait hard-bounded.
#
# USAGE
#   scripts/oxigraph-serve-same-box.sh <corpus-file> <ttl|nt> <queries-dir> <iters> <out-tsv>
#   Rows: 3-col `<name>\t<rows>\t<best_us>`, or 6-col with HTTP_PROFILE=1
#   (`<name>\t<rows>\t<ka_best_us>\t<ka_ttfb_us>\t<fresh_best_us>\t<fresh_ttfb_us>`).
#   ALSO writes `<out-tsv>.load_s` = pure bulk-load seconds (the offline load step).
#
# TUNABLES (env; safe defaults):
#   OXIGRAPH_BIN            path to the oxigraph CLI  (default /usr/local/bin/oxigraph, else PATH)
#   OXIGRAPH_HTTP_PORT      HTTP port                 (default 7878)
#   OXIGRAPH_LOAD_TIMEOUT   bulk-load cap, s          (default 900)
#   OXIGRAPH_READY_TIMEOUT  readiness cap, s          (default 120)
#   OXIGRAPH_QUERY_TIMEOUT  per-query cap, s          (default 60)
#   HTTP_PROFILE            1 = 6-col TTFB/keep-alive/fresh profile rows (default 0)
set -euo pipefail

CORPUS="${1:?usage: oxigraph-serve-same-box.sh <corpus> <ttl|nt> <queries-dir> <iters> <out-tsv>}"
FORMAT="${2:?missing format (ttl|nt)}"
QUERIES_DIR="${3:?missing queries dir}"
ITERS="${4:?missing iters}"
OUT_TSV="${5:?missing out-tsv}"

OXIGRAPH_BIN="${OXIGRAPH_BIN:-}"
OXIGRAPH_HTTP_PORT="${OXIGRAPH_HTTP_PORT:-7878}"
OXIGRAPH_LOAD_TIMEOUT="${OXIGRAPH_LOAD_TIMEOUT:-900}"
OXIGRAPH_READY_TIMEOUT="${OXIGRAPH_READY_TIMEOUT:-120}"
OXIGRAPH_QUERY_TIMEOUT="${OXIGRAPH_QUERY_TIMEOUT:-60}"
HTTP_PROFILE="${HTTP_PROFILE:-0}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ADAPTER="$SCRIPT_DIR/bench-adapters/http_sparql_adapter.py"

log() { printf '[oxigraph-http] %s\n' "$*" >&2; }
die() { printf '[oxigraph-http] ERROR: %s\n' "$*" >&2; exit 1; }

case "$FORMAT" in ttl|nt) ;; *) die "format must be 'ttl' or 'nt' (got '$FORMAT')" ;; esac
case "$ITERS" in ''|*[!0-9]*) die "iters must be a positive integer (got '$ITERS')" ;; esac
[ -f "$CORPUS" ] || die "corpus file not found: $CORPUS"
[ -d "$QUERIES_DIR" ] || die "queries dir not found: $QUERIES_DIR"
command -v python3 >/dev/null 2>&1 || die "python3 required"
[ -f "$ADAPTER" ] || die "shared adapter not found: $ADAPTER"
if [ -z "$OXIGRAPH_BIN" ]; then
  if [ -x /usr/local/bin/oxigraph ]; then OXIGRAPH_BIN=/usr/local/bin/oxigraph
  elif command -v oxigraph >/dev/null 2>&1; then OXIGRAPH_BIN="$(command -v oxigraph)"
  else die "oxigraph CLI not found — fetch the sha256-pinned release binary first (see scripts/gather-ec2-sparql.sh OXI_* pins)"; fi
fi

STORE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/oxi-serve.XXXXXX")"
SRV_PID=""
cleanup() {
  set +e
  [ -n "$SRV_PID" ] && kill "$SRV_PID" >/dev/null 2>&1
  rm -rf "$STORE_DIR" >/dev/null 2>&1
  log "teardown: stopped oxigraph serve (if any) + removed store dir"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

ENDPOINT="http://127.0.0.1:${OXIGRAPH_HTTP_PORT}/query"

log "bulk-load (<= ${OXIGRAPH_LOAD_TIMEOUT}s) into $STORE_DIR"
T0=$(date +%s.%N)
timeout "$OXIGRAPH_LOAD_TIMEOUT" "$OXIGRAPH_BIN" load --location "$STORE_DIR" --file "$CORPUS" --format "$FORMAT" >&2 \
  || die "oxigraph load failed/timed out"
LOAD_S=$(awk "BEGIN{printf \"%.3f\", $(date +%s.%N) - $T0}")
printf '%s\n' "$LOAD_S" > "${OUT_TSV}.load_s"
log "loaded in ${LOAD_S}s; starting oxigraph serve on :$OXIGRAPH_HTTP_PORT (read-only store)"

"$OXIGRAPH_BIN" serve-read-only --location "$STORE_DIR" --bind "127.0.0.1:${OXIGRAPH_HTTP_PORT}" >&2 &
SRV_PID=$!

ready=0
poll_n=$(( OXIGRAPH_READY_TIMEOUT )); [ "$poll_n" -ge 1 ] || poll_n=1
for _ in $(seq 1 "$poll_n"); do
  kill -0 "$SRV_PID" >/dev/null 2>&1 || die "oxigraph serve exited during startup"
  if curl -fsS --max-time 5 --data-urlencode "query=SELECT * WHERE { ?s ?p ?o } LIMIT 1" \
      -H 'Accept: application/sparql-results+json' "$ENDPOINT" >/dev/null 2>&1; then
    ready=1; break
  fi
  sleep 1
done
[ "$ready" = 1 ] || die "server not ready within ${OXIGRAPH_READY_TIMEOUT}s — aborting (no hang)"
log "server ready at $ENDPOINT"

PROFILE_FLAG=""
[ "$HTTP_PROFILE" = "1" ] && PROFILE_FLAG="--profile"
: > "$OUT_TSV"
shopt -s nullglob
any_ok=0
for q in "$QUERIES_DIR"/*.rq; do
  name="$(basename "$q" .rq)"
  # brace-group `|| true` is LOAD-BEARING under set -euo pipefail (see fuseki recipe).
  row="$({ timeout "$OXIGRAPH_QUERY_TIMEOUT" python3 "$ADAPTER" \
             --endpoint "$ENDPOINT" --query-file "$q" --engine oxigraph --iters "$ITERS" \
             $PROFILE_FLAG 2>/dev/null || true; } \
         | awk -F'\t' -v n="$name" 'NR==1{$1=n; print}' OFS='\t')"
  [ -n "$row" ] || row="$name	ERROR	oxigraph"
  printf '%s\n' "$row" >> "$OUT_TSV"
  case "$row" in *$'\tERROR\t'*) ;; *) any_ok=1 ;; esac
done
shopt -u nullglob

log "wrote $OUT_TSV:"; cat "$OUT_TSV" >&2 || true
[ "$any_ok" = 1 ] || die "no query produced a non-ERROR row"
log "done (>=1 query succeeded)"
