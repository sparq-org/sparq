#!/usr/bin/env bash
# sq-7d3dj.34 — dedicated same-box sparq-server load->serve->query->teardown
# recipe: sparq measured in the SAME HTTP regime as Fuseki/Virtuoso/QLever.
#
# WHY THIS SCRIPT EXISTS
#   The 2026-07-07 canonical competitor matrix measured sparq in CLI mode while the
#   competitors ran HTTP servers — an asymmetry the gap table (research/
#   perf-dominance-gap-2026-07.md D9) flagged as NOT-MEASURED. This recipe serves the
#   corpus with `sparq-server` (SPARQL 1.1 Protocol at /sparql) and queries it through
#   the SHARED scripts/bench-adapters/http_sparql_adapter.py — the same POST ->
#   SPARQL-JSON -> count unit the competitor recipes use — so a sparq HTTP number is
#   directly comparable to a competitor HTTP number.
#
#   Same contract as the sibling recipes (scripts/{fuseki,virtuoso,qlever}-same-box.sh):
#   bounded waits everywhere, an EXIT trap that always stops the server, non-zero exit =
#   "sparq-http failed" and the caller keeps the cell honest-n/a.
#
# USAGE
#   scripts/sparq-server-same-box.sh <corpus-file> <ttl|nt> <queries-dir> <iters> <out-tsv>
#
#   Writes <out-tsv> rows `<name>\t<rows>\t<best_us>` (3-col), or the 6-col profile rows
#   `<name>\t<rows>\t<ka_best_us>\t<ka_ttfb_us>\t<fresh_best_us>\t<fresh_ttfb_us>` when
#   HTTP_PROFILE=1 (keep-alive + fresh-connect + TTFB; see the adapter header).
#   ALSO writes `<out-tsv>.load_s` = seconds from server spawn to first successful HTTP
#   answer (load + index build + bind; the honest HTTP-mode "time to serving").
#
# TUNABLES (env; safe defaults):
#   SPARQ_SERVER_BIN      server binary       (default target/release/sparq-server)
#   SPARQ_HTTP_PORT       HTTP port           (default 3035)
#   SPARQ_READY_TIMEOUT   readiness cap, s    (default 300 — covers corpus load)
#   SPARQ_QUERY_TIMEOUT   per-query cap, s    (default 60)
#   SPARQ_SERVER_ARGS     extra server flags  (default "--query-timeout 120")
#   HTTP_PROFILE          1 = 6-col TTFB/keep-alive/fresh profile rows (default 0)
set -euo pipefail

CORPUS="${1:?usage: sparq-server-same-box.sh <corpus> <ttl|nt> <queries-dir> <iters> <out-tsv>}"
FORMAT="${2:?missing format (ttl|nt)}"
QUERIES_DIR="${3:?missing queries dir}"
ITERS="${4:?missing iters}"
OUT_TSV="${5:?missing out-tsv}"

SPARQ_SERVER_BIN="${SPARQ_SERVER_BIN:-target/release/sparq-server}"
SPARQ_HTTP_PORT="${SPARQ_HTTP_PORT:-3035}"
SPARQ_READY_TIMEOUT="${SPARQ_READY_TIMEOUT:-300}"
SPARQ_QUERY_TIMEOUT="${SPARQ_QUERY_TIMEOUT:-60}"
SPARQ_SERVER_ARGS="${SPARQ_SERVER_ARGS:---query-timeout 120}"
HTTP_PROFILE="${HTTP_PROFILE:-0}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ADAPTER="$SCRIPT_DIR/bench-adapters/http_sparql_adapter.py"

log() { printf '[sparq-http] %s\n' "$*" >&2; }
die() { printf '[sparq-http] ERROR: %s\n' "$*" >&2; exit 1; }

case "$FORMAT" in
  ttl) SFMT=turtle ;;
  nt)  SFMT=ntriples ;;
  *)   die "format must be 'ttl' or 'nt' (got '$FORMAT')" ;;
esac
case "$ITERS" in ''|*[!0-9]*) die "iters must be a positive integer (got '$ITERS')" ;; esac
[ "$ITERS" -ge 1 ] || die "iters must be >= 1"
[ -f "$CORPUS" ] || die "corpus file not found: $CORPUS"
[ -d "$QUERIES_DIR" ] || die "queries dir not found: $QUERIES_DIR"
[ -x "$SPARQ_SERVER_BIN" ] || die "sparq-server not built at '$SPARQ_SERVER_BIN' — run: cargo build -p sparq-server --release"
command -v python3 >/dev/null 2>&1 || die "python3 required (http_sparql_adapter client)"
[ -f "$ADAPTER" ] || die "shared adapter not found: $ADAPTER"

SRV_PID=""
SRV_LOG="$(mktemp "${TMPDIR:-/tmp}/sparq-server.XXXXXX.log")"
cleanup() {
  set +e
  [ -n "$SRV_PID" ] && kill "$SRV_PID" >/dev/null 2>&1
  rm -f "$SRV_LOG" >/dev/null 2>&1
  log "teardown: stopped sparq-server (if any)"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

ENDPOINT="http://127.0.0.1:${SPARQ_HTTP_PORT}/sparql"

# ---- spawn the server over the corpus; time spawn -> first successful answer ----------
log "starting sparq-server on :$SPARQ_HTTP_PORT over $CORPUS ($SFMT); args: $SPARQ_SERVER_ARGS"
T_START=$(date +%s.%N)
# shellcheck disable=SC2086  # SPARQ_SERVER_ARGS is deliberately word-split (extra flags)
"$SPARQ_SERVER_BIN" --addr "127.0.0.1:${SPARQ_HTTP_PORT}" --format "$SFMT" $SPARQ_SERVER_ARGS "$CORPUS" >"$SRV_LOG" 2>&1 &
SRV_PID=$!

ready=0
poll_n=$(( SPARQ_READY_TIMEOUT * 2 )); [ "$poll_n" -ge 1 ] || poll_n=1
for _ in $(seq 1 "$poll_n"); do
  if ! kill -0 "$SRV_PID" >/dev/null 2>&1; then
    log "sparq-server exited during startup — last log:"; tail -20 "$SRV_LOG" >&2 || true
    die "sparq-server did not stay up"
  fi
  if curl -fsS --max-time 5 --data-urlencode "query=SELECT * WHERE { ?s ?p ?o } LIMIT 1" \
      -H 'Accept: application/sparql-results+json' "$ENDPOINT" >/dev/null 2>&1; then
    ready=1; break
  fi
  sleep 0.5
done
[ "$ready" = 1 ] || die "server not ready within ${SPARQ_READY_TIMEOUT}s — aborting (no hang)"
LOAD_S=$(awk "BEGIN{printf \"%.3f\", $(date +%s.%N) - $T_START}")
printf '%s\n' "$LOAD_S" > "${OUT_TSV}.load_s"
log "server ready at $ENDPOINT after ${LOAD_S}s (spawn -> first answer, incl. load)"

# ---- run queries via the SHARED adapter, emit TSV --------------------------------------
PROFILE_FLAG=""
[ "$HTTP_PROFILE" = "1" ] && PROFILE_FLAG="--profile"
: > "$OUT_TSV"
shopt -s nullglob
any_ok=0
for q in "$QUERIES_DIR"/*.rq; do
  name="$(basename "$q" .rq)"
  # Adapter emits `<engine>\t<count>\t<cols...>`; rename col1 to the query name. The
  # brace-group `|| true` is LOAD-BEARING under set -euo pipefail (see fuseki recipe).
  row="$({ timeout "$SPARQ_QUERY_TIMEOUT" python3 "$ADAPTER" \
             --endpoint "$ENDPOINT" --query-file "$q" --engine sparq --iters "$ITERS" \
             $PROFILE_FLAG 2>/dev/null || true; } \
         | awk -F'\t' -v n="$name" 'NR==1{$1=n; print}' OFS='\t')"
  [ -n "$row" ] || row="$name	ERROR	sparq"
  printf '%s\n' "$row" >> "$OUT_TSV"
  case "$row" in *$'\tERROR\t'*) ;; *) any_ok=1 ;; esac
done
shopt -u nullglob

log "wrote $OUT_TSV:"; cat "$OUT_TSV" >&2 || true
[ "$any_ok" = 1 ] || die "no query produced a non-ERROR row"
log "done (>=1 query succeeded)"
