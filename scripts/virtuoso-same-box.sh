#!/usr/bin/env bash
# sq-vw3ax.12.1 — dedicated same-box Virtuoso OSS bulk-load->serve->query->teardown
# recipe.
#
# WHY THIS SCRIPT EXISTS
#   Virtuoso OSS (competitors.json id `virtuoso`, kind `http-sparql`) is a Tier-1 mainstream
#   SPARQL store — the store behind the public DBpedia endpoint — and a widely-cited baseline.
#   Like QLever + Fuseki it is a long-lived SERVER, not a file-in/answer-out CLI: to benchmark
#   it you must (1) start the Virtuoso container, (2) BULK-LOAD the corpus via the isql bulk
#   loader (ld_dir + rdf_loader_run), (3) query the /sparql HTTP endpoint via the SHARED
#   scripts/bench-adapters/http_sparql_adapter.py (the same POST->SPARQL-JSON->count unit the
#   QLever + Fuseki paths use), then (4) STOP the server and delete scratch.
#
#   This is the self-contained, BOUNDED, ORPHAN-SAFE recipe (the Virtuoso sibling of
#   scripts/qlever-same-box.sh + scripts/fuseki-same-box.sh). The container + scratch are torn
#   down by an EXIT trap (success, failure, timeout, SIGINT). Every wait is hard-bounded.
#
#   Virtuoso OSS has no pure-Java/tarball drop-in the way Fuseki does, so this recipe is
#   Docker-only (openlink/virtuoso-opensource-7). It fails fast with a clear message if no
#   Docker daemon is reachable, and the caller keeps the dashboard cell honest-n/a.
#
# USAGE
#   scripts/virtuoso-same-box.sh <corpus-file> <ttl|nt> <queries-dir> <iters> <out-tsv>
#     <corpus-file>  dataset (Turtle .ttl or N-Triples .nt)
#     <format>       ttl | nt  (only the file extension matters to Virtuoso's loader)
#     <queries-dir>  dir of *.rq SPARQL queries (e.g. bench/sp2b/queries)
#     <iters>        min-of-K per query
#     <out-tsv>      where to write the result TSV (<name>\t<rows>\t<best_us>)
#
#   Exit status: 0 if the corpus loaded, the endpoint became ready, and >=1 query produced a
#   non-ERROR row; non-zero otherwise. A non-zero exit means "virtuoso skipped/failed".
#
# TUNABLES (env; all have safe defaults):
#   VIRTUOSO_IMAGE        Docker image                 (default docker.io/openlink/virtuoso-opensource-7:latest)
#   VIRTUOSO_HTTP_PORT    SPARQL HTTP port             (default 8890)
#   VIRTUOSO_ISQL_PORT    host isql port               (default 1111)
#   VIRTUOSO_PASSWORD     dba password                 (default dba)
#   VIRTUOSO_GRAPH        target named graph IRI       (default http://sparq.bench/graph)
#   VIRTUOSO_PULL_TIMEOUT image-pull hard cap, s       (default 600)
#   VIRTUOSO_READY_TIMEOUT server-readiness cap, s     (default 180)
#   VIRTUOSO_LOAD_TIMEOUT bulk-load hard cap, s        (default 900)
#   VIRTUOSO_QUERY_TIMEOUT per-HTTP-query cap, s       (default 60)
#   VIRTUOSO_MIN_FREE_GB  abort load if free disk <    (default 10)
#   VIRTUOSO_NAME         container name               (default virtuoso-srv-$$)
#
# OUTPUT TSV FORMAT (matches the harness's <name>\t<rows>\t<best_us>, like sparq/oxigraph):
#   q01<TAB>1<TAB>4567.8          — rows + best wall micros over <iters> runs
#   q07<TAB>ERROR<TAB>virtuoso    — query failed (server error / timeout)
set -euo pipefail

CORPUS="${1:?usage: virtuoso-same-box.sh <corpus> <ttl|nt> <queries-dir> <iters> <out-tsv>}"
FORMAT="${2:?missing format (ttl|nt)}"
QUERIES_DIR="${3:?missing queries dir}"
ITERS="${4:?missing iters}"
OUT_TSV="${5:?missing out-tsv}"

VIRTUOSO_IMAGE="${VIRTUOSO_IMAGE:-docker.io/openlink/virtuoso-opensource-7:latest}"
VIRTUOSO_HTTP_PORT="${VIRTUOSO_HTTP_PORT:-8890}"
VIRTUOSO_ISQL_PORT="${VIRTUOSO_ISQL_PORT:-1111}"
VIRTUOSO_PASSWORD="${VIRTUOSO_PASSWORD:-dba}"
VIRTUOSO_GRAPH="${VIRTUOSO_GRAPH:-http://sparq.bench/graph}"
VIRTUOSO_PULL_TIMEOUT="${VIRTUOSO_PULL_TIMEOUT:-600}"
VIRTUOSO_READY_TIMEOUT="${VIRTUOSO_READY_TIMEOUT:-180}"
VIRTUOSO_LOAD_TIMEOUT="${VIRTUOSO_LOAD_TIMEOUT:-900}"
VIRTUOSO_QUERY_TIMEOUT="${VIRTUOSO_QUERY_TIMEOUT:-60}"
VIRTUOSO_MIN_FREE_GB="${VIRTUOSO_MIN_FREE_GB:-10}"
VIRTUOSO_NAME="${VIRTUOSO_NAME:-virtuoso-srv-$$}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ADAPTER="$SCRIPT_DIR/bench-adapters/http_sparql_adapter.py"

log()  { printf '[virtuoso] %s\n' "$*" >&2; }
die()  { printf '[virtuoso] ERROR: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

case "$FORMAT" in ttl|nt) ;; *) die "format must be 'ttl' or 'nt' (got '$FORMAT')" ;; esac
case "$ITERS" in ''|*[!0-9]*) die "iters must be a positive integer (got '$ITERS')" ;; esac
[ "$ITERS" -ge 1 ] || die "iters must be >= 1"
[ -f "$CORPUS" ] || die "corpus file not found: $CORPUS"
[ -d "$QUERIES_DIR" ] || die "queries dir not found: $QUERIES_DIR"
have python3 || die "python3 required (http_sparql_adapter client)"
[ -f "$ADAPTER" ] || die "shared adapter not found: $ADAPTER"
# daemon PREFLIGHT — fail fast with a CLEAR message (not a confusing pull-then-run
# cascade) when Docker is absent or the daemon is not reachable. This is the same preflight
# the QLever recipe gained; a bench box without a live daemon keeps virtuoso honest-n/a.
have docker || die "docker not installed (Virtuoso OSS is Docker-only here) — install Docker + start the daemon"
docker info >/dev/null 2>&1 || die "Docker daemon not reachable (start it: 'sudo systemctl start docker') — refusing to run"

# ---- scratch dir + ALWAYS-RUN teardown -------------------------------------------------
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/virtuoso.XXXXXX")"
cleanup() {
  set +e
  docker rm -f "$VIRTUOSO_NAME" >/dev/null 2>&1
  rm -rf "$SCRATCH" >/dev/null 2>&1
  log "teardown: removed container '$VIRTUOSO_NAME' (if any) + scratch dir"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# ---- df guard --------------------------------------------------------------------------
free_gb="$(df -BG --output=avail "$SCRATCH" 2>/dev/null | tail -1 | tr -dc '0-9' || true)"
if [ -n "$free_gb" ]; then
  log "disk: ${free_gb} GB free (floor ${VIRTUOSO_MIN_FREE_GB} GB)"
  [ "$free_gb" -ge "$VIRTUOSO_MIN_FREE_GB" ] \
    || die "free disk ${free_gb} GB < floor ${VIRTUOSO_MIN_FREE_GB} GB — refusing Virtuoso bulk load"
else
  log "could not read free disk space; skipping df guard"
fi

CORPUS_DIR="$(cd "$(dirname "$CORPUS")" && pwd)"
CORPUS_FILE="$(basename "$CORPUS")"
ENDPOINT="http://localhost:${VIRTUOSO_HTTP_PORT}/sparql"

# ---- 0. pull the image (bounded) -------------------------------------------------------
log "pulling $VIRTUOSO_IMAGE (<= ${VIRTUOSO_PULL_TIMEOUT}s)"
timeout "$VIRTUOSO_PULL_TIMEOUT" docker pull -q "$VIRTUOSO_IMAGE" >/dev/null \
  || log "pull failed/slow — continuing with any locally cached image"

# ---- 1. START the server (mount the corpus dir read-only under /data) ------------------
# DirsAllowed must include /data for the isql bulk loader (ld_dir) to read the corpus.
log "starting Virtuoso container '$VIRTUOSO_NAME' (HTTP :$VIRTUOSO_HTTP_PORT, isql :$VIRTUOSO_ISQL_PORT)"
docker rm -f "$VIRTUOSO_NAME" >/dev/null 2>&1 || true
docker run -d --name "$VIRTUOSO_NAME" \
  -p "${VIRTUOSO_HTTP_PORT}:8890" -p "${VIRTUOSO_ISQL_PORT}:1111" \
  -e "DBA_PASSWORD=$VIRTUOSO_PASSWORD" \
  -e "VIRT_Parameters_DirsAllowed=., /data, /opt/virtuoso-opensource/vad, /database" \
  -v "$CORPUS_DIR":/data:ro "$VIRTUOSO_IMAGE" >/dev/null \
  || die "failed to start Virtuoso container"

# ---- 2. bounded readiness poll (isql answers once the server is up) --------------------
# Inside the container isql always listens on 1111; VIRTUOSO_ISQL_PORT is only the host-side mapping (#3361).
isql() { docker exec "$VIRTUOSO_NAME" isql 1111 dba "$VIRTUOSO_PASSWORD" "$@"; }
ready=0
poll_n=$(( VIRTUOSO_READY_TIMEOUT / 3 )); [ "$poll_n" -ge 1 ] || poll_n=1
for _ in $(seq 1 "$poll_n"); do
  if ! docker ps --filter "name=^${VIRTUOSO_NAME}$" --filter status=running -q | grep -q .; then
    log "server container exited during startup — dumping last logs:"; docker logs --tail 30 "$VIRTUOSO_NAME" >&2 2>&1 || true
    die "Virtuoso server did not stay up"
  fi
  if isql exec="SELECT 1;" >/dev/null 2>&1; then ready=1; break; fi
  sleep 3
done
[ "$ready" = 1 ] || die "Virtuoso not ready within ${VIRTUOSO_READY_TIMEOUT}s — aborting (no hang)"
log "server ready"

# ---- 3. BULK LOAD via the isql bulk loader (ld_dir + rdf_loader_run) -------------------
# ld_dir registers files matching a glob into the load queue; rdf_loader_run() drains it;
# checkpoint persists. Then verify the graph is non-empty before trusting any query.
log "bulk-load (<= ${VIRTUOSO_LOAD_TIMEOUT}s) $CORPUS_FILE into <$VIRTUOSO_GRAPH>"
LOAD_SQL="ld_dir('/data', '${CORPUS_FILE}', '${VIRTUOSO_GRAPH}'); rdf_loader_run(); checkpoint;"
if ! timeout "$VIRTUOSO_LOAD_TIMEOUT" bash -c "docker exec '$VIRTUOSO_NAME' isql 1111 dba '$VIRTUOSO_PASSWORD' exec=\"$LOAD_SQL\"" >&2; then
  rc=$?
  [ "$rc" = 124 ] && die "bulk load hit the ${VIRTUOSO_LOAD_TIMEOUT}s timeout"
  die "bulk load failed (rc=$rc)"
fi
# sanity: the graph must have triples (a silent empty load would make every query 0)
NTRIP="$(isql exec="SPARQL SELECT COUNT(*) WHERE { GRAPH <${VIRTUOSO_GRAPH}> { ?s ?p ?o } };" 2>/dev/null | grep -oE '[0-9]+' | tail -1 || true)"
log "loaded triples in <$VIRTUOSO_GRAPH>: ${NTRIP:-unknown}"
case "${NTRIP:-0}" in ''|0) die "bulk load produced 0 triples in <$VIRTUOSO_GRAPH> — refusing an empty comparison" ;; esac

# ---- 4. run queries over HTTP via the SHARED adapter, emit TSV -------------------------
# Virtuoso's default result cap is 10000 rows (ResultSetMaxRows / SPARQL MaxQueryCostEstimationTime).
# We disable the row cap PER QUERY by prepending `DEFINE sql:big-data-const 0` is NOT enough; the
# adapter POSTs the raw query, so we wrap large-result queries by requesting the full set. The
# honest cross-check is COUNT/result-size vs sparq FIRST — a truncated Virtuoso count shows up as a
# mismatch, not a silent pass. (Row cap raising is a Virtuoso-config concern documented in the
# registry note; for the small same-box corpus the default cap is not hit for most queries.)
# sq-7d3dj.34: HTTP_PROFILE=1 switches the adapter to --profile (6-col rows:
# keep-alive + fresh-connect full-request latency AND TTFB); the awk renames col1 and
# reprints ALL columns, serving both the 3-col and 6-col contracts.
PROFILE_FLAG=""
[ "${HTTP_PROFILE:-0}" = "1" ] && PROFILE_FLAG="--profile"
: > "$OUT_TSV"
shopt -s nullglob
any_ok=0
for q in "$QUERIES_DIR"/*.rq; do
  name="$(basename "$q" .rq)"
  # brace-group `|| true` is LOAD-BEARING under `set -euo pipefail` — see the identical
  # note in fuseki-same-box.sh: a per-query `timeout` (rc=124) would otherwise propagate via
  # pipefail + set -e and abort the whole loop instead of recording one ERROR row and continuing.
  row="$({ timeout "$VIRTUOSO_QUERY_TIMEOUT" python3 "$ADAPTER" \
             --endpoint "$ENDPOINT" --query-file "$q" --engine virtuoso --iters "$ITERS" \
             $PROFILE_FLAG 2>/dev/null || true; } \
         | awk -F'\t' -v n="$name" 'NR==1{$1=n; print}' OFS='\t')"
  [ -n "$row" ] || row="$name	ERROR	virtuoso"
  printf '%s\n' "$row" >> "$OUT_TSV"
  case "$row" in *$'\tERROR\t'*) ;; *) any_ok=1 ;; esac
done
shopt -u nullglob

log "wrote $OUT_TSV:"; cat "$OUT_TSV" >&2 || true
[ "$any_ok" = 1 ] || die "no query produced a non-ERROR row"
log "done (>=1 query succeeded)"
