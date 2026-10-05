#!/usr/bin/env bash
# sq-vw3ax.12.1 — dedicated same-box Apache Jena Fuseki load->serve->query->teardown
# recipe.
#
# WHY THIS SCRIPT EXISTS
#   Fuseki (competitors.json id `fuseki`, kind `http-sparql`) is the Tier-1 SPARQL-1.1
#   reference SERVER + a correctness oracle. Like QLever it is NOT a file-in/answer-out CLI:
#   to benchmark it you must (1) BULK-LOAD the corpus into a TDB2 store (tdb2.tdbloader),
#   then (2) start a long-lived fuseki-server HTTP endpoint, then (3) query it over HTTP via
#   the SHARED scripts/bench-adapters/http_sparql_adapter.py (the same POST->SPARQL-JSON->count
#   unit the QLever + Virtuoso paths use), then (4) STOP the server and delete the store.
#
#   This is the self-contained, BOUNDED, ORPHAN-SAFE recipe (the Fuseki sibling of
#   scripts/qlever-same-box.sh + scripts/virtuoso-same-box.sh). Every process it starts and
#   every byte of scratch it writes is removed by an EXIT trap — on success, failure, timeout,
#   or SIGINT. Every wait is hard-bounded; there is no unbounded poll anywhere.
#
# TWO BACKENDS (same Apache-2.0 bits -> identical results; pick per box):
#   * jena  : the apache-jena / apache-jena-fuseki tarballs (pure Java, NO Docker/root).
#             Set FUSEKI_SERVER=<.../fuseki-server> and TDBLOADER=<.../tdb2.tdbloader>
#             (or put them on PATH), or let the recipe AUTO-FETCH the sha512-PINNED
#             Apache tarballs (FUSEKI_FETCH=1, the default when java is present) into
#             FUSEKI_CACHE_DIR. This is Fuseki's INTENDED bulk path (offline
#             tdb2.tdbloader, then serve) and the exact competitors.json run_recipe.
#   * docker: a community Fuseki image (default stain/jena-fuseki). CAUTION — root-caused
#             sq-7d3dj.34: that image (a) ships NO tdb2.tdbloader at all (only
#             TDB1 tdbloader/tdbloader2, not on PATH), and (b) its /docker-entrypoint.sh
#             does `exec "$@" &` then polls http://localhost:3030 in an UNBOUNDED loop —
#             so a one-shot loader command never exits and the container hangs forever.
#             That produced the deterministic 1800 s "fuseki load timeout" FAILED rows in
#             the 2026-07-07 canonical matrix. The docker backend now (1) PREFLIGHTS that
#             the image actually contains tdb2.tdbloader (bounded, fails FAST with this
#             root cause) and (2) bypasses the entrypoint with --entrypoint for both the
#             loader and the server.
#   Default: `jena` when java (or an auto-fetch) is available — the intended bulk path;
#   else `docker` when a daemon is reachable. Override with FUSEKI_BACKEND=jena|docker.
#
# USAGE
#   scripts/fuseki-same-box.sh <corpus-file> <ttl|nt> <queries-dir> <iters> <out-tsv>
#     <corpus-file>  dataset (Turtle .ttl or N-Triples .nt)
#     <format>       ttl | nt
#     <queries-dir>  dir of *.rq SPARQL queries (e.g. bench/sp2b/queries)
#     <iters>        min-of-K per query
#     <out-tsv>      where to write the result TSV (<name>\t<rows>\t<best_us>)
#
#   Exit status: 0 if the store loaded, the server became ready, and >=1 query produced a
#   non-ERROR row; non-zero otherwise. The caller treats a non-zero exit as "fuseki
#   skipped/failed" and keeps the dashboard cell honest-n/a — it NEVER hangs the gather.
#
# TUNABLES (env; all have safe defaults):
#   FUSEKI_BACKEND        jena | docker                (default: jena if java/fetch, else docker)
#   FUSEKI_SERVER         path to fuseki-server        (jena backend; else PATH/auto-fetch)
#   TDBLOADER             path to tdb2.tdbloader       (jena backend; else PATH/auto-fetch)
#   FUSEKI_FETCH          1 = allow auto-fetch of the pinned Apache tarballs (default 1)
#   FUSEKI_JENA_VERSION   Apache Jena version to fetch (default 6.1.0; pins below match it)
#   FUSEKI_CACHE_DIR      tarball/extract cache        (default /tmp/sparq-jena-cache)
#   FUSEKI_IMAGE          Docker image (docker backend)(default docker.io/stain/jena-fuseki)
#   FUSEKI_PORT           HTTP port                    (default 3030)
#   FUSEKI_DS             dataset path segment         (default ds)
#   FUSEKI_LOAD_TIMEOUT   bulk-load hard cap, s        (default 900)
#   FUSEKI_PULL_TIMEOUT   image-pull hard cap, s       (default 600, docker backend)
#   FUSEKI_READY_TIMEOUT  server-readiness cap, s      (default 120)
#   FUSEKI_QUERY_TIMEOUT  per-HTTP-query cap, s        (default 60)
#   FUSEKI_MIN_FREE_GB    abort load if free disk <    (default 10)
#   FUSEKI_NAME           docker container name        (default fuseki-srv-$$)
#   JAVA_HOME             JDK home for the jena backend (auto-detected from `java` if unset)
#   HTTP_PROFILE          1 = 6-col TTFB/keep-alive/fresh profile rows (default 0)
#
# OUTPUT TSV FORMAT (matches the harness's <name>\t<rows>\t<best_us>, like sparq/oxigraph):
#   q01<TAB>1<TAB>4567.8        — rows + best wall micros over <iters> runs
#   q07<TAB>ERROR<TAB>fuseki    — query failed (server error / timeout)
# With HTTP_PROFILE=1 rows are 6-col:
#   <name>\t<rows>\t<ka_best_us>\t<ka_ttfb_us>\t<fresh_best_us>\t<fresh_ttfb_us>
set -euo pipefail

CORPUS="${1:?usage: fuseki-same-box.sh <corpus> <ttl|nt> <queries-dir> <iters> <out-tsv>}"
FORMAT="${2:?missing format (ttl|nt)}"
QUERIES_DIR="${3:?missing queries dir}"
ITERS="${4:?missing iters}"
OUT_TSV="${5:?missing out-tsv}"

FUSEKI_PORT="${FUSEKI_PORT:-3030}"
FUSEKI_DS="${FUSEKI_DS:-ds}"
FUSEKI_IMAGE="${FUSEKI_IMAGE:-docker.io/stain/jena-fuseki}"
FUSEKI_LOAD_TIMEOUT="${FUSEKI_LOAD_TIMEOUT:-900}"
FUSEKI_PULL_TIMEOUT="${FUSEKI_PULL_TIMEOUT:-600}"
FUSEKI_READY_TIMEOUT="${FUSEKI_READY_TIMEOUT:-120}"
FUSEKI_QUERY_TIMEOUT="${FUSEKI_QUERY_TIMEOUT:-60}"
FUSEKI_MIN_FREE_GB="${FUSEKI_MIN_FREE_GB:-10}"
FUSEKI_NAME="${FUSEKI_NAME:-fuseki-srv-$$}"
FUSEKI_FETCH="${FUSEKI_FETCH:-1}"
FUSEKI_JENA_VERSION="${FUSEKI_JENA_VERSION:-6.1.0}"
FUSEKI_CACHE_DIR="${FUSEKI_CACHE_DIR:-/tmp/sparq-jena-cache}"
HTTP_PROFILE="${HTTP_PROFILE:-0}"

# sq-7d3dj.34 — sha512 pins for the auto-fetched Apache tarballs (Jena 6.1.0,
# from downloads.apache.org/jena/binaries/*.sha512, verified 2026-07-07). Overriding
# FUSEKI_JENA_VERSION requires overriding BOTH pins too (the fetch refuses unpinned bits).
FUSEKI_JENA_SHA512="${FUSEKI_JENA_SHA512:-6aa4bb8eeb41c0d05c30f3c91a7eb065bd867af00a6a95fd10f7873b90271c62734b28aebd7ae648d5be6b1e185c9037df90633c471a68b791b19026fd03ea3a}"
FUSEKI_FUSEKI_SHA512="${FUSEKI_FUSEKI_SHA512:-75457f45d14397876a41ed51abe7ae5d2f1e708dfe1315765f858158bc5c6813bc036ec1539ddc4dffd26201f5cc31fadec299ca5c3dc2548b723513ed31d326}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ADAPTER="$SCRIPT_DIR/bench-adapters/http_sparql_adapter.py"

log()  { printf '[fuseki] %s\n' "$*" >&2; }
die()  { printf '[fuseki] ERROR: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

case "$FORMAT" in ttl|nt) ;; *) die "format must be 'ttl' or 'nt' (got '$FORMAT')" ;; esac
case "$ITERS" in ''|*[!0-9]*) die "iters must be a positive integer (got '$ITERS')" ;; esac
[ "$ITERS" -ge 1 ] || die "iters must be >= 1"
[ -f "$CORPUS" ] || die "corpus file not found: $CORPUS"
[ -d "$QUERIES_DIR" ] || die "queries dir not found: $QUERIES_DIR"
have python3 || die "python3 required (http_sparql_adapter client)"
[ -f "$ADAPTER" ] || die "shared adapter not found: $ADAPTER"

# ---- auto-fetch the pinned Apache tarballs (jena backend) -------------------------------
# sq-7d3dj.34: makes the INTENDED bulk path (offline tdb2.tdbloader -> serve)
# self-contained on a fresh gather box: download the two official Apache tarballs,
# sha512-verify against the pins above, extract into FUSEKI_CACHE_DIR (idempotent across
# recipe invocations — the canonical gather runs this 4x). Refuses unpinned bits.
fetch_jena() {
  local base="https://downloads.apache.org/jena/binaries"
  local alt="https://archive.apache.org/dist/jena/binaries"
  mkdir -p "$FUSEKI_CACHE_DIR"
  local name sha dir
  for name in "apache-jena-${FUSEKI_JENA_VERSION}" "apache-jena-fuseki-${FUSEKI_JENA_VERSION}"; do
    case "$name" in
      *fuseki*) sha="$FUSEKI_FUSEKI_SHA512" ;;
      *)        sha="$FUSEKI_JENA_SHA512" ;;
    esac
    dir="$FUSEKI_CACHE_DIR/$name"
    if [ -d "$dir" ]; then log "cached: $dir"; continue; fi
    local tgz="$FUSEKI_CACHE_DIR/$name.tar.gz"
    log "fetching $name.tar.gz (sha512-pinned)"
    timeout "$FUSEKI_PULL_TIMEOUT" curl -fsSL -o "$tgz" "$base/$name.tar.gz" \
      || timeout "$FUSEKI_PULL_TIMEOUT" curl -fsSL -o "$tgz" "$alt/$name.tar.gz" \
      || { rm -f "$tgz"; return 1; }
    local act; act="$(sha512sum "$tgz" | cut -d' ' -f1)"
    if [ "$act" != "$sha" ]; then
      rm -f "$tgz"
      die "sha512 MISMATCH for $name.tar.gz (expected $sha, got $act) — refusing unpinned bits"
    fi
    tar -C "$FUSEKI_CACHE_DIR" -xzf "$tgz" && rm -f "$tgz"
    [ -d "$dir" ] || return 1
  done
  TDBLOADER="$FUSEKI_CACHE_DIR/apache-jena-${FUSEKI_JENA_VERSION}/bin/tdb2.tdbloader"
  FUSEKI_SERVER="$FUSEKI_CACHE_DIR/apache-jena-fuseki-${FUSEKI_JENA_VERSION}/fuseki-server"
  chmod +x "$TDBLOADER" "$FUSEKI_SERVER" 2>/dev/null || true
  export TDBLOADER FUSEKI_SERVER
}

# ---- pick a backend --------------------------------------------------------------------
docker_up() { have docker && docker info >/dev/null 2>&1; }
jena_binaries_present() { have "${TDBLOADER:-tdb2.tdbloader}" && have "${FUSEKI_SERVER:-fuseki-server}"; }
BACKEND="${FUSEKI_BACKEND:-}"
if [ -z "$BACKEND" ]; then
  # Default preference is the INTENDED bulk path: jena when its binaries are already
  # available, or when java + the auto-fetch can provide them; docker is the fallback.
  if jena_binaries_present; then
    BACKEND="jena"
  elif [ "$FUSEKI_FETCH" = "1" ] && have java && have curl; then
    BACKEND="jena"
  elif docker_up; then
    BACKEND="docker"
  else
    BACKEND="jena"   # fails below with the actionable fuseki-server/tdbloader message
  fi
fi
case "$BACKEND" in
  jena|docker) ;;
  *) die "FUSEKI_BACKEND must be 'jena' or 'docker' (got '$BACKEND')" ;;
esac
if [ "$BACKEND" = "jena" ] && ! jena_binaries_present; then
  if [ "$FUSEKI_FETCH" = "1" ] && have java && have curl; then
    fetch_jena || die "auto-fetch of the pinned Apache Jena ${FUSEKI_JENA_VERSION} tarballs failed"
  fi
fi
log "backend: $BACKEND (port $FUSEKI_PORT, dataset /$FUSEKI_DS)"

# ---- scratch store dir + ALWAYS-RUN teardown -------------------------------------------
STORE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/fuseki-db.XXXXXX")"
SRV_PID=""
cleanup() {
  set +e
  # kill the jena-backend server if we started one, and rm -f the docker container if any
  [ -n "$SRV_PID" ] && kill "$SRV_PID" >/dev/null 2>&1
  if [ "$BACKEND" = "docker" ] && have docker; then
    docker rm -f "$FUSEKI_NAME" >/dev/null 2>&1
  fi
  rm -rf "$STORE_DIR" >/dev/null 2>&1
  log "teardown: stopped server (if any) + removed store dir"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# ---- df guard (bulk load is the disk-heavy step) ---------------------------------------
free_gb="$(df -BG --output=avail "$STORE_DIR" 2>/dev/null | tail -1 | tr -dc '0-9' || true)"
if [ -n "$free_gb" ]; then
  log "disk: ${free_gb} GB free (floor ${FUSEKI_MIN_FREE_GB} GB)"
  [ "$free_gb" -ge "$FUSEKI_MIN_FREE_GB" ] \
    || die "free disk ${free_gb} GB < floor ${FUSEKI_MIN_FREE_GB} GB — refusing Fuseki bulk load"
else
  log "could not read free disk space; skipping df guard"
fi

CORPUS_DIR="$(cd "$(dirname "$CORPUS")" && pwd)"
CORPUS_FILE="$(basename "$CORPUS")"
ENDPOINT="http://localhost:${FUSEKI_PORT}/${FUSEKI_DS}/query"

# ======================================================================================
# BACKEND: jena tarball (pure Java; the competitors.json run_recipe)
# ======================================================================================
start_jena() {
  local fs="${FUSEKI_SERVER:-fuseki-server}" tl="${TDBLOADER:-tdb2.tdbloader}"
  have "$fs" || die "fuseki-server not found — set FUSEKI_SERVER=<apache-jena-fuseki/fuseki-server> or put it on PATH"
  have "$tl" || die "tdb2.tdbloader not found — set TDBLOADER=<apache-jena/bin/tdb2.tdbloader> or put it on PATH"
  # ensure JAVA_HOME so the jena launcher scripts find a JDK
  if [ -z "${JAVA_HOME:-}" ] && have java; then
    JAVA_HOME="$(dirname "$(dirname "$(readlink -f "$(command -v java)")")")"
    export JAVA_HOME
  fi
  log "bulk-load (<= ${FUSEKI_LOAD_TIMEOUT}s) via $tl --loc $STORE_DIR $CORPUS_FILE [$FORMAT]"
  if ! timeout "$FUSEKI_LOAD_TIMEOUT" "$tl" --loc "$STORE_DIR" "$CORPUS" >&2; then
    local rc=$?
    [ "$rc" = 124 ] && die "tdb2.tdbloader hit the ${FUSEKI_LOAD_TIMEOUT}s timeout"
    die "tdb2.tdbloader failed (rc=$rc)"
  fi
  log "starting fuseki-server on :$FUSEKI_PORT over the TDB2 store"
  # FUSEKI_BASE into the scratch dir so fuseki-server does NOT litter the caller's
  # cwd with a `run/` working area (backups/logs/system_files/templates). Without this the repo
  # root gets polluted every run; the scratch dir is removed by the EXIT trap.
  export FUSEKI_BASE="$STORE_DIR/fuseki-base"
  mkdir -p "$FUSEKI_BASE"
  # --tdb2 --loc serves the bulk-loaded store; /$FUSEKI_DS mounts query+data endpoints.
  "$fs" --port "$FUSEKI_PORT" --tdb2 --loc "$STORE_DIR" "/$FUSEKI_DS" >>"$STORE_DIR/fuseki.log" 2>&1 &
  SRV_PID=$!
}

# ======================================================================================
# BACKEND: docker (official/community Fuseki image; bundles tdb2.tdbloader)
# ======================================================================================
start_docker() {
  docker_up || die "docker backend selected but the Docker daemon is not reachable (start it, or FUSEKI_BACKEND=jena)"
  log "pulling $FUSEKI_IMAGE (<= ${FUSEKI_PULL_TIMEOUT}s)"
  timeout "$FUSEKI_PULL_TIMEOUT" docker pull -q "$FUSEKI_IMAGE" >/dev/null \
    || log "pull failed/slow — continuing with any locally cached image"
  # sq-7d3dj.34 — PREFLIGHT + ENTRYPOINT BYPASS (the 2026-07-07 canonical-run
  # root cause). stain/jena-fuseki's /docker-entrypoint.sh does `exec "$@" &` then polls
  # http://localhost:3030 in an UNBOUNDED `until curl` loop, so a one-shot loader command
  # hangs the container FOREVER; and that image ships no tdb2.tdbloader anyway (TDB1
  # tools only). So: (1) verify the loader actually exists in the image (bounded, fails
  # FAST with the actionable message), (2) run both loader and server with --entrypoint,
  # never through the image entrypoint.
  log "preflight: does $FUSEKI_IMAGE contain tdb2.tdbloader?"
  if ! timeout 60 docker run --rm --entrypoint sh "$FUSEKI_IMAGE" \
        -c 'command -v tdb2.tdbloader >/dev/null 2>&1 || [ -x /jena/bin/tdb2.tdbloader ] || [ -x /jena-fuseki/bin/tdb2.tdbloader ]' >/dev/null 2>&1; then
    die "image $FUSEKI_IMAGE has NO tdb2.tdbloader (stain/jena-fuseki ships only TDB1 tools; its entrypoint also hangs forever on non-server commands — the 2026-07-07 canonical 1800s 'load timeout'). Use FUSEKI_BACKEND=jena (auto-fetches the pinned Apache tarballs when java is present) or point FUSEKI_IMAGE at an image that ships TDB2 tools."
  fi
  log "bulk-load (<= ${FUSEKI_LOAD_TIMEOUT}s) via tdb2.tdbloader inside $FUSEKI_IMAGE (--entrypoint bypass)"
  if ! timeout "$FUSEKI_LOAD_TIMEOUT" docker run --rm --entrypoint sh \
        -v "$CORPUS_DIR":/in:ro -v "$STORE_DIR":/db "$FUSEKI_IMAGE" \
        -c "exec \"\$(command -v tdb2.tdbloader || echo /jena/bin/tdb2.tdbloader)\" --loc /db '/in/$CORPUS_FILE'" >&2; then
    local rc=$?
    [ "$rc" = 124 ] && die "tdb2.tdbloader (docker) hit the ${FUSEKI_LOAD_TIMEOUT}s timeout"
    die "tdb2.tdbloader (docker) failed (rc=$rc)"
  fi
  log "starting fuseki-server container '$FUSEKI_NAME' on :$FUSEKI_PORT (--entrypoint bypass)"
  docker rm -f "$FUSEKI_NAME" >/dev/null 2>&1 || true
  docker run -d --name "$FUSEKI_NAME" -p "${FUSEKI_PORT}:${FUSEKI_PORT}" \
    -v "$STORE_DIR":/db --entrypoint /jena-fuseki/fuseki-server "$FUSEKI_IMAGE" \
    --port "$FUSEKI_PORT" --tdb2 --loc /db "/$FUSEKI_DS" >/dev/null \
    || die "failed to start fuseki-server container"
}

case "$BACKEND" in
  jena)   start_jena ;;
  docker) start_docker ;;
esac

# ---- bounded readiness poll (FOR loop with hard count, NOT `while :`) -------------------
ready=0
poll_n=$(( FUSEKI_READY_TIMEOUT / 2 )); [ "$poll_n" -ge 1 ] || poll_n=1
for _ in $(seq 1 "$poll_n"); do
  # docker backend: if the container has already died, stop polling immediately.
  if [ "$BACKEND" = "docker" ]; then
    if ! docker ps --filter "name=^${FUSEKI_NAME}$" --filter status=running -q | grep -q .; then
      log "server container exited during startup — dumping last logs:"; docker logs --tail 30 "$FUSEKI_NAME" >&2 2>&1 || true
      die "Fuseki server did not stay up"
    fi
  elif [ -n "$SRV_PID" ] && ! kill -0 "$SRV_PID" >/dev/null 2>&1; then
    log "fuseki-server process exited during startup — last log:"; tail -30 "$STORE_DIR/fuseki.log" >&2 2>/dev/null || true
    die "Fuseki server did not stay up"
  fi
  if curl -fsS --max-time 5 --data-urlencode "query=SELECT * WHERE { ?s ?p ?o } LIMIT 1" \
      -H 'Accept: application/sparql-results+json' "$ENDPOINT" >/dev/null 2>&1; then
    ready=1; break
  fi
  sleep 2
done
[ "$ready" = 1 ] || die "server not ready within ${FUSEKI_READY_TIMEOUT}s — aborting (no hang)"
log "server ready at $ENDPOINT"

# ---- run queries over HTTP via the SHARED adapter, emit TSV ----------------------------
# sq-7d3dj.34: HTTP_PROFILE=1 switches the adapter to --profile (6-col rows:
# keep-alive + fresh-connect full-request latency AND TTFB); the awk below renames col1
# and reprints ALL columns, so it serves both the 3-col and 6-col contracts.
PROFILE_FLAG=""
[ "$HTTP_PROFILE" = "1" ] && PROFILE_FLAG="--profile"
: > "$OUT_TSV"
shopt -s nullglob
any_ok=0
for q in "$QUERIES_DIR"/*.rq; do
  name="$(basename "$q" .rq)"
  # http_sparql_adapter.py emits `<engine>\t<count>\t<cols...>`; rename col1 to the query name.
  # A transport/parse error (adapter exit 1) or a per-query timeout becomes an ERROR row.
  # The brace-group `|| true` is LOAD-BEARING under `set -euo pipefail`: without it a
  # per-query `timeout` firing (rc=124) propagates through `pipefail` and set -e ABORTS the whole
  # script mid-loop (observed on a slow large-result query like SP2Bench q04) instead of recording
  # a single ERROR row and moving on. The group makes the left side of the pipe always exit 0.
  row="$({ timeout "$FUSEKI_QUERY_TIMEOUT" python3 "$ADAPTER" \
             --endpoint "$ENDPOINT" --query-file "$q" --engine fuseki --iters "$ITERS" \
             $PROFILE_FLAG 2>/dev/null || true; } \
         | awk -F'\t' -v n="$name" 'NR==1{$1=n; print}' OFS='\t')"
  [ -n "$row" ] || row="$name	ERROR	fuseki"
  printf '%s\n' "$row" >> "$OUT_TSV"
  case "$row" in *$'\tERROR\t'*) ;; *) any_ok=1 ;; esac
done
shopt -u nullglob

log "wrote $OUT_TSV:"; cat "$OUT_TSV" >&2 || true
[ "$any_ok" = 1 ] || die "no query produced a non-ERROR row"
log "done (>=1 query succeeded)"
