#!/usr/bin/env bash
# sq-hmd7l.3 — same-box GeoSPARQL comparison harness: sparq-geo vs
# jena-fuseki-geosparql (the registered like-for-like compliance peer: the only
# triplestore with full GML+WKT, per bench/competitors.json `geosparql-jena`).
# Built on the scripts/bench/shacl-same-box.sh template; emits one
# canonical-competitor-results-shaped ENVELOPE JSON so a future canonical gather
# is ingestible by scripts/bench/ingest-canonical-competitors.mjs unchanged.
#
# GeoSPARQL had NO competitor baseline (registered gather-only, never run); this
# script is the durable, reusable gather recipe. A run on the shared work box is
# NON-canonical (canonical:false in the envelope, always); the canonical run is
# the quiet-box wave (design record research/comparative-benchmarking-everything.md
# §4; execution bead sq-hmd7l.26). CANONICAL=1 only there.
#
# WORKLOADS (shared corpus: the FIXED ~100k-point CRS84 corpus, bench/geo/gen.sh,
# seed 20260615 — the same substrate the per-commit bench/geo gate asserts):
#   within10km / within50km   great-circle radius counts around POINT(0 51)
#   within10km_indexed        Jena-only index-accelerated rendering of
#                             within10km over a feature-modelled corpus variant
#   nearest_k10 / nearest_k100 k-nearest (rendered for SPARQL peers as
#                              ORDER BY geof:distance ... LIMIT k — the
#                              standard-visible form; see queries-jena/*.rq)
#   geof_within                geof:sfWithin(point, 1°x1° box) filter count
# geo_compliance_pass (the OGC fixture ratchet) is sparq-only and carried in
# sparq_tsv but is NOT a competitor row (replaying the fixture set against Jena
# is out of scope here; Jena is the like-for-like COVERAGE bar per the registry).
#
# METHODOLOGY / INVARIANT (counts-not-coordinates except nearest entity IDs):
# sq-6jl8z
#   * sparq is timed IN-PROCESS via bench/geo/run.sh (examples/bench_geo: load +
#     index once, best-of-N per workload), which HARD-asserts every count vs
#     bench/geo/expected.tsv — the sparq-side oracle.
#   * jena-geosparql is timed over HTTP: the corpus is loaded into an in-memory
#     jena-fuseki-geosparql dataset, then each pinned query in
#     bench/geo/queries-jena/*.rq is POSTed via
#     scripts/bench-adapters/http_sparql_adapter.py (best-of-N; query_us includes
#     HTTP framing + SPARQL-JSON parse — a recorded mode ASYMMETRY vs sparq's
#     in-process index surface, never adjusted away).
#   * NO TIMING WITHOUT RESULT AGREEMENT per query: radius/box workloads compare
#     result-set size; nearest_k workloads compare the exact entity-IRI SET from
#     an untimed oracle replay in addition to size. A disagreement is itself a
#     recorded RESULT (both results kept, timing withheld). Both engines'
#     within* metric is spherical great-circle on the same mean sphere (sparq:
#     haversine; jena: spatialF:nearby — the standard geof:distance+uom:metre
#     form is non-executable on jena 5.4.0, root-caused in sq-a8anf /
#     research/gap-geo-2026-07.md §6d), so only float-boundary edge points may
#     legitimately differ.
#   * per-workload wall-clock cap (TIMEOUT_S, covering the whole best-of-N
#     series); a timeout/error degrades to an honest ERROR row, never a number.
#   * a competitor that cannot run (no java, no jar, download or start-up
#     failure) is GRACEFULLY SKIPPED: the envelope records the reason and the
#     script still exits 0 (a skip is a missing column + a re-run action, never
#     a sparq win — design record §4 point 6).
#
# GEOGRAPHICA (opt-in second workload family, GEO_GEOGRAPHICA=1)
# (sq-hmd7l.29): the reviewer-recognized real-world suite — the LGD/GeoNames
# slices of Geographica (ISWC 2013) fetched+pinned+normalised by
# bench/geo/geographica.sh (gather-only, /tmp), replayed from the pinned
# translations in bench/geo/queries-geographica/ (micro non-topological +
# spatial selections + the one LGD/GeoNames spatial join). Same invariant:
# sparq runs in-process (`bench_geo query`, counts recorded vs the pinned
# expected-geographica.tsv oracle), jena replays the same pinned queries over
# HTTP (a `.jena.rq` rendering only where the standard form is non-executable
# on jena 5.4.0 — research/gap-geo-2026-07.md §6d), and NO TIMING enters the
# family envelope without result-set-size agreement. Emits a SECOND envelope
# (geo-geographica-<ts>.json); the base fixed-corpus flow is unchanged.
#
# USAGE
#   scripts/bench/geo-same-box.sh                 # both engines, full iters
#   ONLY=sparq GEO_SMOKE=1 scripts/bench/geo-same-box.sh   # the acceptance smoke
#   GEO_GEOGRAPHICA=1 scripts/bench/geo-same-box.sh        # + the Geographica family
#
# TUNABLES (env; all have safe defaults):
#   ITERS           best-of-N per workload      (default 3; 1 under GEO_SMOKE=1)
#   TIMEOUT_S       per-workload cap (whole best-of-N series), s   (default 300)
#   ONLY            engine subset of "sparq jena-geosparql"        (default both)
#   OUT_DIR         envelope output dir (default /tmp/geo-same-box-results; a
#                   canonical run points this at
#                   bench/canonical-competitor-results/<date>/)
#   CANONICAL       1 = dedicated quiet-box run (default 0: NON-canonical)
#   GEO_SMOKE       1 = smoke mode (iters=1, envelope flagged smoke:true)
#   FUSEKI_GEO_VERSION  jena-fuseki-geosparql version       (default 5.4.0)
#   FUSEKI_GEO_JAR  path to the executable uber-jar; auto-downloaded from Maven
#                   Central to /tmp/jena-geosparql/ if absent and GEO_FETCH_JENA=1
#   GEO_FETCH_JENA  0 = never download (absent jar -> graceful skip; default 1)
#   FUSEKI_PORT     jena-fuseki-geosparql port              (default 3039)
#   INDEX_FUSEKI_PORT  indexed-radius fuseki port      (default FUSEKI_PORT+2)
#   FUSEKI_READY_S  server load+spatial-index readiness cap (default 300)
#   FUSEKI_INDEX_READY_S indexed feature-probe cap after startup (default 60)
#   FUSEKI_STOP_S   graceful server shutdown cap             (default 30)
#   BENCH_GEO       the sparq bench_geo binary (default: build it)
#   GEO_GEOGRAPHICA 1 = also run the Geographica real-world family (default 0)
#   GG_WORKLOADS    Geographica workload subset (default: all pinned .rq stems)
#   GG_TIMEOUT_S    per-workload cap for the family, s      (default TIMEOUT_S)
#   GG_FUSEKI_PORT  the family's own fuseki port            (default FUSEKI_PORT+1)
#
# SCRATCH: the jar lives under /tmp/jena-geosparql (gather-only dep, NOT
# committed — engines stay out of git per AGENTS.md): rm -rf /tmp/jena-geosparql
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

GEO_SMOKE="${GEO_SMOKE:-0}"
if [ "$GEO_SMOKE" = 1 ]; then ITERS="${ITERS:-1}"; else ITERS="${ITERS:-3}"; fi
TIMEOUT_S="${TIMEOUT_S:-300}"
ONLY="${ONLY:-sparq jena-geosparql}"
OUT_DIR="${OUT_DIR:-/tmp/geo-same-box-results}"
CANONICAL="${CANONICAL:-0}"
FUSEKI_GEO_VERSION="${FUSEKI_GEO_VERSION:-5.4.0}"
FUSEKI_GEO_JAR="${FUSEKI_GEO_JAR:-/tmp/jena-geosparql/jena-fuseki-geosparql-$FUSEKI_GEO_VERSION.jar}"
GEO_FETCH_JENA="${GEO_FETCH_JENA:-1}"
FUSEKI_PORT="${FUSEKI_PORT:-3039}"
INDEX_FUSEKI_PORT="${INDEX_FUSEKI_PORT:-$((FUSEKI_PORT + 2))}"
FUSEKI_READY_S="${FUSEKI_READY_S:-300}"
FUSEKI_INDEX_READY_S="${FUSEKI_INDEX_READY_S:-60}"
FUSEKI_STOP_S="${FUSEKI_STOP_S:-30}"
BENCH_GEO="${BENCH_GEO:-$ROOT/target/release/examples/bench_geo}"
ADAPTER="$ROOT/scripts/bench-adapters/http_sparql_adapter.py"
QUERIES="$ROOT/bench/geo/queries-jena"
# The fixed-corpus workloads plus a Jena-only indexed rendering whose oracle is
# sparq's unchanged within10km row. (sq-enfy3)
BASE_WORKLOADS="within10km within50km nearest_k10 nearest_k100 geof_within"
INDEX_WORKLOADS="within10km_indexed"
WORKLOADS="$BASE_WORKLOADS $INDEX_WORKLOADS"
# ---- the Geographica real-world family (OPT-IN) sq-hmd7l.29 ----
GEO_GEOGRAPHICA="${GEO_GEOGRAPHICA:-0}"
GG_TIMEOUT_S="${GG_TIMEOUT_S:-$TIMEOUT_S}"
GG_FUSEKI_PORT="${GG_FUSEKI_PORT:-$((FUSEKI_PORT + 1))}"
GG_QUERIES="$ROOT/bench/geo/queries-geographica"
GG_EXPECTED="$GG_QUERIES/expected-geographica.tsv"
# Every pinned .rq stem: micro non-topological (q04/q05), spatial selections
# (q07..q17) and the one LGD/GeoNames spatial join (q19 — typically an honest
# ERROR(timeout) row on BOTH engines at the default cap; that IS the result).
GG_WORKLOADS="${GG_WORKLOADS:-q04_buffer_geonames q05_buffer_lgd q07_equals_lgd_line \
q09_intersects_lgd_poly q12_crosses_lgd_line q13_within_geonames_poly \
q14_within_geonames_pointbuffer q15_distance_geonames_point \
q16_disjoint_geonames_poly q17_disjoint_lgd_poly q19_join_intersects_geonames_lgd}"

log() { printf '[geo-same-box] %s\n' "$*" >&2; }
want() { [[ " $ONLY " == *" $1 "* ]]; }

# Shared by the base and Geographica families (sq-hmd7l.29): resolve
# java + the executable uber-jar. Prints a "skipped: ..." reason on stdout and
# returns 1 when the engine cannot run (the graceful-skip contract).
ensure_jena_jar() {
  if ! command -v java >/dev/null 2>&1; then
    echo "skipped: java not on PATH"
    return 1
  fi
  if [ ! -f "$FUSEKI_GEO_JAR" ]; then
    if [ "$GEO_FETCH_JENA" = 1 ]; then
      log "downloading jena-fuseki-geosparql $FUSEKI_GEO_VERSION to $(dirname "$FUSEKI_GEO_JAR") (gather-only dep)"
      mkdir -p "$(dirname "$FUSEKI_GEO_JAR")"
      local url="https://repo1.maven.org/maven2/org/apache/jena/jena-fuseki-geosparql/$FUSEKI_GEO_VERSION/jena-fuseki-geosparql-$FUSEKI_GEO_VERSION.jar"
      if ! curl -fsSL -o "$FUSEKI_GEO_JAR.tmp" "$url"; then
        rm -f "$FUSEKI_GEO_JAR.tmp"
        echo "skipped: jar download failed ($url)"
        return 1
      fi
      mv "$FUSEKI_GEO_JAR.tmp" "$FUSEKI_GEO_JAR"
    else
      echo "skipped: jar absent at $FUSEKI_GEO_JAR and GEO_FETCH_JENA=0"
      return 1
    fi
  fi
}

# Start jena-fuseki-geosparql over <corpus.nt> on <port>, logging to <logfile>;
# wait for SPARQL readiness (cap FUSEKI_READY_S). The PID is published through
# a file because callers capture stdout in a command substitution (a subshell);
# prints the ready-seconds on stdout, or returns 1 on failure.
start_fuseki() { # <corpus.nt> <port> <logfile> [readiness-ask]
  local t0 deadline ready
  local default_ask='ASK {}'
  local readiness_ask="${4:-$default_ask}"
  # Format is inferred from the .nt extension; -rf = read file, -p = port.
  java -jar "$FUSEKI_GEO_JAR" -rf "$1" -p "$2" > "$3" 2>&1 &
  FUSEKI_PID=$!
  printf '%s\n' "$FUSEKI_PID" > "$TMP/fuseki.pid"
  t0="$(date +%s)"
  deadline=$((t0 + FUSEKI_READY_S))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    if ! kill -0 "$FUSEKI_PID" 2>/dev/null; then break; fi
    if ready="$(python3 "$ADAPTER" --endpoint "http://127.0.0.1:$2/ds" \
        --query "$readiness_ask" --engine ready 2>/dev/null)" \
        && [ "$(printf '%s\n' "$ready" | cut -f2)" = 1 ]; then
      echo "$(($(date +%s) - t0))"
      return 0
    fi
    sleep 2
  done
  return 1
}

# Poll an indexed property-function ASK after the server itself is ready.
# Adapter stderr is retained so a permanent HTTP/query error is diagnosable.
# sq-enfy3
wait_index_probe() { # <port> <readiness-ask> <adapter-stderr>
  local t0 deadline ready
  t0="$(date +%s)"
  deadline=$((t0 + FUSEKI_INDEX_READY_S))
  : > "$3"
  while [ "$(date +%s)" -lt "$deadline" ]; do
    if ready="$(python3 "$ADAPTER" --endpoint "http://127.0.0.1:$1/ds" \
        --query "$2" --engine ready 2>> "$3")" \
        && [ "$(printf '%s\n' "$ready" | cut -f2)" = 1 ]; then
      echo "$(($(date +%s) - t0))"
      return 0
    fi
    sleep 2
  done
  return 1
}

# Stop the current fuseki instance (idempotent; used between families so two
# JVMs never compete for the box during timing).
stop_fuseki() {
  if [ -s "$TMP/fuseki.pid" ]; then
    FUSEKI_PID="$(cat "$TMP/fuseki.pid")"
    kill "$FUSEKI_PID" 2>/dev/null || true
    if ! wait_fuseki_stopped "" "$FUSEKI_PID"; then
      kill -9 "$FUSEKI_PID" 2>/dev/null || true
      wait_fuseki_stopped "" "$FUSEKI_PID" || true
    fi
    if ! kill -0 "$FUSEKI_PID" 2>/dev/null; then
      rm -f "$TMP/fuseki.pid"
      FUSEKI_PID=""
    fi
  fi
}

wait_fuseki_stopped() { # <port> [pid]
  local port="$1"
  local pid="${2:-}"
  local deadline=$(( $(date +%s) + FUSEKI_STOP_S ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    if [ -n "$pid" ]; then
      if ! kill -0 "$pid" 2>/dev/null; then return 0; fi
    elif ! (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
      return 0
    fi
    sleep 1
  done
  return 1
}

mkdir -p "$OUT_DIR"
TMP="$(mktemp -d /tmp/geo-same-box.XXXXXX)"
FUSEKI_PID=""
# shellcheck disable=SC2317 # invoked via trap
cleanup() {
  stop_fuseki
  rm -rf "$TMP"
}
trap cleanup EXIT

# ---- 0. sparq binary + the shared fixed corpus --------------------------------
if [ ! -x "$BENCH_GEO" ]; then
  log "building sparq bench_geo (cargo -p sparq-geo --example bench_geo)"
  cargo build --release -q -p sparq-geo --example bench_geo
  BENCH_GEO="$ROOT/target/release/examples/bench_geo"
fi
# The FIXED ~100k-point corpus (cached, deterministic). expected.tsv is pinned to
# this scale, so the corpus size is intentionally NOT tunable here.
CORPUS="$(BENCH_GEO="$BENCH_GEO" "$ROOT/bench/geo/gen.sh" 100000)"
NPOINTS="$(wc -l < "$CORPUS")"
FEATURE_CORPUS="n/a"
FEATURE_TRIPLES="n/a"
log "corpus=$CORPUS (~$NPOINTS triples)"

GIT_COMMIT="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"

# ---- 1. sparq: the self-asserting in-process leg (counts gated vs expected.tsv)
SPARQ_STATUS="not-run"
if want sparq; then
  log "sparq: bench/geo/run.sh x$ITERS (cap $((TIMEOUT_S * 6))s; counts hard-asserted vs expected.tsv)"
  if ITERS="$ITERS" BENCH_GEO="$BENCH_GEO" timeout "$((TIMEOUT_S * 6))" \
      bash "$ROOT/bench/geo/run.sh" > "$TMP/sparq.tsv" 2> "$TMP/sparq.err"; then
    SPARQ_STATUS="ok"
  else
    SPARQ_STATUS="failed"
    log "sparq FAILED/timeout or count drift (see $TMP/sparq.err):"
    tail -5 "$TMP/sparq.err" >&2 || true
  fi
  if [ "$SPARQ_STATUS" = "ok" ]; then
    for spec in "nearest_k10 10" "nearest_k100 100"; do
      read -r wl k <<< "$spec"
      if ! timeout "$TIMEOUT_S" "$BENCH_GEO" nearest-ids "$CORPUS" "$k" \
          > "$TMP/sparq-$wl.ids"; then
        log "sparq: $wl entity-ID oracle FAILED; timing will be withheld"
        rm -f "$TMP/sparq-$wl.ids"
      fi
    done
  fi
fi

# ---- 2. jena-geosparql: HTTP leg (graceful skip when the engine is absent) -----
JENA_STATUS="not-run"
JENA_VER=""
JENA_JAR_SHA=""
JENA_READY_S="n/a"
JENA_INDEX_READY_S="n/a"
ENDPOINT="http://127.0.0.1:$FUSEKI_PORT/ds"
if want jena-geosparql; then
  if SKIP_REASON="$(ensure_jena_jar)"; then
    JENA_STATUS="pending"
  else
    JENA_STATUS="$SKIP_REASON"
  fi

  if [ "$JENA_STATUS" = "pending" ]; then
    FEATURE_CORPUS="$(BENCH_GEO="$BENCH_GEO" "$ROOT/bench/geo/gen.sh" 100000 feature)"
    FEATURE_TRIPLES="$(wc -l < "$FEATURE_CORPUS")"
    log "feature-corpus=$FEATURE_CORPUS ($FEATURE_TRIPLES triples; Jena indexed-radius variant)"
    JENA_VER="jena-fuseki-geosparql-$FUSEKI_GEO_VERSION ($(java -version 2>&1 | head -1))"
    JENA_JAR_SHA="$(sha256sum "$FUSEKI_GEO_JAR" | cut -d' ' -f1)"
    log "starting $ENDPOINT (in-memory dataset, corpus loaded as N-Triples; readiness cap ${FUSEKI_READY_S}s)"
    if ! JENA_READY_S="$(start_fuseki "$CORPUS" "$FUSEKI_PORT" "$TMP/fuseki.log")"; then
      JENA_READY_S="n/a"
      JENA_STATUS="skipped: server not ready in ${FUSEKI_READY_S}s (see fuseki.log tail below)"
      tail -5 "$TMP/fuseki.log" >&2 || true
    else
      log "jena-geosparql ready in ${JENA_READY_S}s (load + spatial index; advisory)"
      : > "$TMP/jena-geosparql.tsv"
      for wl in $BASE_WORKLOADS; do
        log "jena-geosparql: $wl x$ITERS (cap ${TIMEOUT_S}s)"
        if timeout "$TIMEOUT_S" python3 "$ADAPTER" --endpoint "$ENDPOINT" \
            --query-file "$QUERIES/$wl.rq" --engine jena-geosparql \
            --iters "$ITERS" --json > "$TMP/$wl.raw" 2> "$TMP/$wl.json"; then
          # --json writes the result JSON to stderr; a COUNT-wrapped query's
          # comparable size is count_value (the scalar), else len(bindings).
          if PARSED="$(python3 - "$TMP/$wl.json" <<'PYEOF'
import json, sys
line = open(sys.argv[1]).read().strip().splitlines()[-1]
d = json.loads(line)
c = d["count_value"] if d.get("count_value") is not None else d["count"]
print("%s\t%s" % (c, d["query_us"]))
PYEOF
          )"; then
            printf '%s\t%s\n' "$wl" "$PARSED" >> "$TMP/jena-geosparql.tsv"
            if [[ "$wl" == nearest_k* ]]; then
              if ! TIMEOUT_S="$TIMEOUT_S" timeout "$TIMEOUT_S" python3 - \
                  "$ENDPOINT" "$QUERIES/$wl.rq" "$TMP/jena-$wl.ids" <<'PYEOF'
import json, os, sys, urllib.parse, urllib.request
endpoint, query_path, out_path = sys.argv[1:]
query = open(query_path, encoding="utf-8").read()
request = urllib.request.Request(
    endpoint,
    data=urllib.parse.urlencode({"query": query}).encode(),
    headers={"Accept": "application/sparql-results+json"},
)
with urllib.request.urlopen(
    request, timeout=int(os.environ["TIMEOUT_S"])
) as response:
    bindings = json.load(response)["results"]["bindings"]
entities = sorted(row["e"]["value"] for row in bindings)
with open(out_path, "w", encoding="utf-8") as output:
    for entity in entities:
        output.write("<%s>\n" % entity)
PYEOF
              then
                log "jena-geosparql: $wl entity-ID oracle FAILED; timing will be withheld"
                rm -f "$TMP/jena-$wl.ids"
              fi
            fi
          else
            printf '%s\tERROR\tsidecar-parse-error\n' "$wl" >> "$TMP/jena-geosparql.tsv"
          fi
        else
          log "jena-geosparql: $wl FAILED/timeout"
          printf '%s\tERROR\ttimeout-or-error\n' "$wl" >> "$TMP/jena-geosparql.tsv"
        fi
      done
      # The fixed corpus deliberately remains unchanged. Restart Jena over the
      # feature-modelled variant so spatial:withinCircle exercises its spatial
      # index, then append the Jena-only result to the same TSV.
      stop_fuseki
      INDEX_PORT="$INDEX_FUSEKI_PORT"
      INDEX_ENDPOINT="http://127.0.0.1:$INDEX_PORT/ds"
      FEATURE_READY_ASK='PREFIX spatial: <http://jena.apache.org/spatial#> PREFIX uom: <http://www.opengis.net/def/uom/OGC/1.0/> ASK { ?e spatial:withinCircle (51 0 10000 uom:metre -1) }'
      log "restarting $INDEX_ENDPOINT with feature-modelled corpus for indexed radius"
      if ! wait_fuseki_stopped "$FUSEKI_PORT"; then
        JENA_INDEX_READY_S="n/a"
        JENA_STATUS="partial: base server did not stop; indexed variant not run"
        log "jena-geosparql: base server did not stop; recording indexed ERROR"
        for wl in $INDEX_WORKLOADS; do
          printf '%s\tERROR\tbase-server-still-running\n' "$wl"
        done >> "$TMP/jena-geosparql.tsv"
      elif ! INDEX_SERVER_READY_S="$(start_fuseki "$FEATURE_CORPUS" "$INDEX_PORT" \
          "$TMP/fuseki-index.log")"; then
        JENA_INDEX_READY_S="n/a"
        JENA_STATUS="partial: indexed-variant server not ready"
        log "jena-geosparql: indexed-radius server not ready; recording ERROR"
        for wl in $INDEX_WORKLOADS; do
          printf '%s\tERROR\tserver-not-ready\n' "$wl"
        done >> "$TMP/jena-geosparql.tsv"
      elif ! INDEX_PROBE_READY_S="$(wait_index_probe "$INDEX_PORT" \
          "$FEATURE_READY_ASK" "$TMP/index-probe.stderr")"; then
        JENA_INDEX_READY_S="n/a"
        JENA_STATUS="partial: indexed-variant feature probe failed"
        log "jena-geosparql: indexed-radius feature probe failed; adapter stderr: $TMP/index-probe.stderr"
        for wl in $INDEX_WORKLOADS; do
          printf '%s\tERROR\tindex-probe-failed\n' "$wl"
        done >> "$TMP/jena-geosparql.tsv"
      else
        JENA_INDEX_READY_S=$((INDEX_SERVER_READY_S + INDEX_PROBE_READY_S))
        for wl in $INDEX_WORKLOADS; do
          log "jena-geosparql: $wl x$ITERS (cap ${TIMEOUT_S}s)"
          if timeout "$TIMEOUT_S" python3 "$ADAPTER" --endpoint "$INDEX_ENDPOINT" \
              --query-file "$QUERIES/$wl.rq" --engine jena-geosparql \
              --iters "$ITERS" --json > "$TMP/$wl.raw" 2> "$TMP/$wl.json"; then
            if PARSED="$(python3 - "$TMP/$wl.json" <<'PYEOF'
import json, sys
line = open(sys.argv[1]).read().strip().splitlines()[-1]
d = json.loads(line)
c = d["count_value"] if d.get("count_value") is not None else d["count"]
print("%s\t%s" % (c, d["query_us"]))
PYEOF
            )"; then
              printf '%s\t%s\n' "$wl" "$PARSED" >> "$TMP/jena-geosparql.tsv"
            else
              printf '%s\tERROR\tsidecar-parse-error\n' "$wl" >> "$TMP/jena-geosparql.tsv"
            fi
          else
            log "jena-geosparql: $wl FAILED/timeout"
            printf '%s\tERROR\ttimeout-or-error\n' "$wl" >> "$TMP/jena-geosparql.tsv"
          fi
        done
      fi
      [ "$JENA_STATUS" = "pending" ] && JENA_STATUS="ok"
    fi
  fi
  [ "$JENA_STATUS" = "ok" ] || log "jena-geosparql: $JENA_STATUS"
fi

# ---- 3. assemble the envelope (canonical-competitor-results JSON shape) --------
TS="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="$OUT_DIR/geo-points100k-${TS}.json"
CANONICAL="$CANONICAL" GEO_SMOKE="$GEO_SMOKE" GIT_COMMIT="$GIT_COMMIT" \
CORPUS="$CORPUS" FEATURE_CORPUS="$FEATURE_CORPUS" NPOINTS="$NPOINTS" \
FEATURE_TRIPLES="$FEATURE_TRIPLES" \
ITERS="$ITERS" TIMEOUT_S="$TIMEOUT_S" \
ONLY="$ONLY" WORKLOADS="$WORKLOADS" TMP="$TMP" OUT="$OUT" \
SPARQ_STATUS="$SPARQ_STATUS" JENA_STATUS="$JENA_STATUS" JENA_VER="$JENA_VER" \
JENA_JAR_SHA="$JENA_JAR_SHA" JENA_READY_S="$JENA_READY_S" \
JENA_INDEX_READY_S="$JENA_INDEX_READY_S" \
python3 - <<'PYEOF'
import json, os, platform

tmp = os.environ["TMP"]
only = os.environ["ONLY"].split()
workloads = os.environ["WORKLOADS"].split()
canonical = os.environ["CANONICAL"] == "1"
smoke = os.environ["GEO_SMOKE"] == "1"


def read_tsv(engine):
    path = os.path.join(tmp, "%s.tsv" % engine)
    rows = {}
    if os.path.exists(path):
        for line in open(path):
            f = line.rstrip("\n").split("\t")
            if len(f) >= 3:
                rows[f[0]] = {"count": f[1], "us": f[2]}
    return rows


def read_ids(engine, workload):
    path = os.path.join(tmp, "%s-%s.ids" % (engine, workload))
    if not os.path.exists(path):
        return None
    return sorted(line.strip() for line in open(path) if line.strip())


engines_meta = {}
if "sparq" in only:
    engines_meta["sparq"] = {
        "version": os.environ["GIT_COMMIT"],
        "mode": (
            "in-process (bench/geo/run.sh -> examples/bench_geo: load + index "
            "once, best-of-N per workload; every count HARD-asserted vs "
            "bench/geo/expected.tsv)"
        ),
    }
if "jena-geosparql" in only:
    engines_meta["jena-geosparql"] = {
        "version": os.environ.get("JENA_VER", ""),
        "jar_sha256": os.environ.get("JENA_JAR_SHA", ""),
        "mode": (
            "HTTP (jena-fuseki-geosparql in-memory dataset over the SAME corpus; "
            "scripts/bench-adapters/http_sparql_adapter.py POST -> SPARQL-JSON, "
            "best-of-N; query_us includes HTTP framing + result parse — a "
            "recorded mode ASYMMETRY vs sparq's in-process index surface)"
        ),
        "load_ready_s_advisory": os.environ.get("JENA_READY_S", "n/a"),
        "indexed_variant_load_ready_s_advisory": os.environ.get(
            "JENA_INDEX_READY_S", "n/a"
        ),
    }

data = {e: read_tsv(e) for e in engines_meta}
statuses = {}
if "sparq" in engines_meta:
    statuses["sparq"] = os.environ["SPARQ_STATUS"]
if "jena-geosparql" in engines_meta:
    statuses["jena-geosparql"] = os.environ["JENA_STATUS"]

note_canonical = (
    "CANONICAL: dedicated quiet box, one engine active at a time on the SAME "
    "fixed CRS84 corpus + pinned query files; counts cross-checked before any "
    "timing is trusted."
    if canonical
    else "NON-canonical FIRST READ: shared work box (not a dedicated quiet "
    "instance). Timings are directional only — do NOT bake into docs or "
    "dashboards. The harness (scripts/bench/geo-same-box.sh) is the durable "
    "deliverable; the canonical run is the quiet-box wave (sq-hmd7l.26)."
)

# INVARIANT: no timing without count agreement, plus exact entity-set agreement
# for nearest-neighbour workloads.
sparq_rows = data.get("sparq", {})
cross = {}
timings = {}
for w in workloads:
    oracle_workload = "within10km" if w == "within10km_indexed" else w
    s = sparq_rows.get(oracle_workload, {}).get("count", "n/a")
    j = data.get("jena-geosparql", {}).get(w, {}).get("count", "n/a")
    count_agree = s == j and s not in ("n/a", "ERROR")
    sparq_ids = read_ids("sparq", w) if w.startswith("nearest_k") else None
    jena_ids = read_ids("jena", w) if w.startswith("nearest_k") else None
    expected_k = int(w.removeprefix("nearest_k")) if w.startswith("nearest_k") else None
    ids_agree = (
        sparq_ids == jena_ids
        and sparq_ids is not None
        and len(sparq_ids) == expected_k
        and len(set(sparq_ids)) == expected_k
        if w.startswith("nearest_k")
        else None
    )
    agree = count_agree and (ids_agree is not False)
    cross[w] = {
        "sparq": s,
        "jena-geosparql": j,
        "agree": agree,
        "sparq_oracle_workload": oracle_workload,
    }
    if w == "within10km_indexed":
        cross[w]["corpus_variant"] = (
            "feature-modelled (2x triples, separate server)"
        )
    if w.startswith("nearest_k"):
        cross[w].update({
            "oracle": "entity-id-set",
            "expected_k": expected_k,
            "sparq_entity_ids": sparq_ids if sparq_ids is not None else "n/a",
            "jena_entity_ids": jena_ids if jena_ids is not None else "n/a",
            "entity_ids_agree": ids_agree,
        })
    row = {}
    if "sparq" in engines_meta and w != "within10km_indexed":
        row["sparq_us"] = sparq_rows.get(w, {}).get("us", "n/a")
    if "jena-geosparql" in engines_meta:
        ju = data["jena-geosparql"].get(w, {}).get("us", "n/a")
        if agree:
            row["jena_geosparql_us"] = ju
        elif j in ("n/a", "ERROR"):
            row["jena_geosparql_us"] = j
        else:
            # Disagreement IS the result: both counts stay recorded above; the
            # timing is WITHHELD (never adjusted, never silently dropped).
            if not count_agree:
                reason = "count-disagree: sparq=%s jena=%s" % (s, j)
            else:
                sparq_id_set = set(sparq_ids or [])
                jena_id_set = set(jena_ids or [])
                reason = (
                    "entity-list-disagree: counts=%s, "
                    "sparq-rows=%d jena-rows=%d, "
                    "|sparq\\jena|=%d |jena\\sparq|=%d"
                    % (
                        s,
                        len(sparq_ids or []),
                        len(jena_ids or []),
                        len(sparq_id_set - jena_id_set),
                        len(jena_id_set - sparq_id_set),
                    )
                )
            row["jena_geosparql_us"] = "WITHHELD(%s)" % reason
    timings[w] = row

scale = "fixed CRS84 point corpus, %s triples (%s)" % (
    os.environ["NPOINTS"],
    os.environ["CORPUS"],
)
if os.environ["FEATURE_CORPUS"] != "n/a":
    scale += (
        "; Jena indexed-radius variant has %s triples and preserves the points "
        "with feature->geometry modelling (%s)"
        % (
            os.environ["FEATURE_TRIPLES"],
            os.environ["FEATURE_CORPUS"],
        )
    )

envelope = {
    "gather": "geo-same-box-comparison",
    "wave": "geo-competitor-baseline (sq-hmd7l.3)",
    "canonical": canonical,
    "canonical_note": note_canonical,
    "smoke": smoke,
    "git_commit": os.environ["GIT_COMMIT"],
    "suite": "geo",
    "scale": scale,
    "iters": int(os.environ["ITERS"]),
    "timeout_s_per_workload": int(os.environ["TIMEOUT_S"]),
    "queries": "bench/geo/queries-jena/<workload>.rq (pinned per-engine renderings)",
    "engines": engines_meta,
    "statuses": statuses,
    "count_crosscheck": cross,
    "count_crosscheck_note": (
        "COUNTS-NOT-COORDINATES for radius/box workloads: result-set sizes are "
        "compared because float geometry is not bit-stable. nearest_k instead "
        "requires exact entity-IRI-set equality from untimed oracle replays, "
        "preventing a degenerate ORDER BY from passing merely by returning k "
        "rows. sparq counts are "
        "additionally hard-gated vs bench/geo/expected.tsv by run.sh. A "
        "disagreement is a recorded RESULT; the competitor timing for that "
        "workload is withheld, never adjusted. Metric note (sq-a8anf): both "
        "engines' within* is spherical great-circle on the same mean sphere "
        "(sparq haversine; jena spatialF:nearby — jena 5.4.0 geof:distance is "
        "planar SRS-native-unit Euclidean and its degree->metre conversion "
        "throws, so the standard geof:distance+uom:metre rendering is "
        "non-executable there; research/gap-geo-2026-07.md §6d), so only "
        "float-boundary edge points may legitimately differ."
    ),
    "timings": timings,
    "timings_note": (
        "per-workload best-of-N microseconds. INVARIANT: a jena timing appears "
        "ONLY where count_crosscheck agrees. MODE ASYMMETRY: sparq is in-process "
        "(index surface), jena is a full HTTP SPARQL round-trip — recorded, not "
        "adjusted; sparq-geo is not wired into sparq-server, so a symmetric "
        "HTTP-vs-HTTP row is not currently possible."
    ),
    "sparq_only_rows_note": (
        "geo_compliance_pass (OGC fixture ratchet) rides in sparq_tsv but is "
        "NOT a competitor row: replaying the fixture set against Jena is out of "
        "scope here; Jena is the like-for-like GML+WKT COVERAGE bar per "
        "bench/competitors.json."
    ),
    "geographica": (
        "WIRED (sq-hmd7l.29): the Geographica real-world LGD/GeoNames family "
        "is the opt-in GEO_GEOGRAPHICA=1 second workload family of this same "
        "script (pinned recipe bench/geo/geographica.sh, pinned queries "
        "bench/geo/queries-geographica/); it emits its own "
        "geo-geographica-<ts>.json envelope."
    ),
    "env": {
        "host": platform.node(),
        "machine": platform.machine(),
        "os": platform.platform(),
    },
}
for e in engines_meta:
    envelope["%s_tsv" % e.replace("-", "_")] = "\n".join(
        "%s\t%s\t%s" % (w, r["count"], r["us"]) for w, r in data[e].items()
    )

with open(os.environ["OUT"], "w") as fh:
    json.dump(envelope, fh, indent=2)
    fh.write("\n")
print(os.environ["OUT"])
PYEOF
log "envelope: $OUT"

# ---- 4. Geographica real-world family (OPT-IN: GEO_GEOGRAPHICA=1) --------------
# sq-hmd7l.29 — the LGD/GeoNames slices of the Geographica suite,
# replayed from the pinned translations in bench/geo/queries-geographica/ under
# the SAME counts-before-timing invariant, into a SECOND envelope. All failure
# modes degrade gracefully (a skip/ERROR is a recorded result, never an abort):
# the hard per-commit correctness gate stays bench/geo/run.sh, untouched.
if [ "$GEO_GEOGRAPHICA" = 1 ]; then
  GG_FAMILY_STATUS="ok"
  GG_SPARQ_STATUS="not-run"
  GG_JENA_STATUS="not-run"
  GG_JENA_READY_S="n/a"
  GG_CORPUS=""
  GG_CORPUS_SHA=""
  GG_NTRIPLES=""
  if ! GG_CORPUS="$(bash "$ROOT/bench/geo/geographica.sh")"; then
    GG_FAMILY_STATUS="skipped: dataset unavailable (bench/geo/geographica.sh failed: download failure or upstream sha256 pin mismatch — see its stderr above)"
    log "geographica: $GG_FAMILY_STATUS"
  fi

  if [ "$GG_FAMILY_STATUS" = ok ]; then
    GG_CORPUS_SHA="$(sha256sum "$GG_CORPUS" | cut -d' ' -f1)"
    GG_NTRIPLES="$(wc -l < "$GG_CORPUS")"
    log "geographica corpus=$GG_CORPUS ($GG_NTRIPLES triples, sha256 $GG_CORPUS_SHA)"

    # sparq: in-process replay via `bench_geo query` (load + geof: registry +
    # full SPARQL eval per pinned .rq). Counts are recorded RAW; the envelope
    # cross-checks them against the pinned expected-geographica.tsv oracle (a
    # drift WITHHOLDS the timing and is itself the recorded result).
    if want sparq; then
      GG_SPARQ_STATUS="ok"
      : > "$TMP/gg-sparq.tsv"
      for wl in $GG_WORKLOADS; do
        qf="$GG_QUERIES/$wl.rq"
        if [ ! -f "$qf" ]; then
          printf '%s\tERROR\tno-query-file\n' "$wl" >> "$TMP/gg-sparq.tsv"
          continue
        fi
        log "sparq(geographica): $wl x$ITERS (cap ${GG_TIMEOUT_S}s)"
        if GG_LINE="$(timeout "$GG_TIMEOUT_S" "$BENCH_GEO" query "$GG_CORPUS" "$qf" "$ITERS")"; then
          printf '%s\n' "$GG_LINE" >> "$TMP/gg-sparq.tsv"
        else
          log "sparq(geographica): $wl FAILED/timeout"
          printf '%s\tERROR\ttimeout-or-error\n' "$wl" >> "$TMP/gg-sparq.tsv"
        fi
      done
    fi

    # jena-geosparql: HTTP replay of the SAME pinned queries against a fresh
    # in-memory dataset holding the geographica corpus (its own port; the base
    # instance is stopped first so two JVMs never compete during timing). A
    # `.jena.rq` rendering is used only where the standard form is
    # non-executable on jena 5.4.0 (research/gap-geo-2026-07.md §6d).
    if want jena-geosparql; then
      if SKIP_REASON="$(ensure_jena_jar)"; then
        GG_JENA_STATUS="pending"
      else
        GG_JENA_STATUS="$SKIP_REASON"
      fi
      if [ "$GG_JENA_STATUS" = pending ]; then
        stop_fuseki
        GG_ENDPOINT="http://127.0.0.1:$GG_FUSEKI_PORT/ds"
        JENA_VER="jena-fuseki-geosparql-$FUSEKI_GEO_VERSION ($(java -version 2>&1 | head -1))"
        JENA_JAR_SHA="$(sha256sum "$FUSEKI_GEO_JAR" | cut -d' ' -f1)"
        log "starting $GG_ENDPOINT (geographica corpus; readiness cap ${FUSEKI_READY_S}s)"
        if ! GG_JENA_READY_S="$(start_fuseki "$GG_CORPUS" "$GG_FUSEKI_PORT" "$TMP/gg-fuseki.log")"; then
          GG_JENA_READY_S="n/a"
          GG_JENA_STATUS="skipped: server not ready in ${FUSEKI_READY_S}s (see gg-fuseki.log tail below)"
          tail -5 "$TMP/gg-fuseki.log" >&2 || true
        else
          log "jena-geosparql(geographica) ready in ${GG_JENA_READY_S}s (load; advisory)"
          : > "$TMP/gg-jena-geosparql.tsv"
          for wl in $GG_WORKLOADS; do
            qf="$GG_QUERIES/$wl.jena.rq"
            [ -f "$qf" ] || qf="$GG_QUERIES/$wl.rq"
            if [ ! -f "$qf" ]; then
              printf '%s\tERROR\tno-query-file\n' "$wl" >> "$TMP/gg-jena-geosparql.tsv"
              continue
            fi
            log "jena-geosparql(geographica): $wl x$ITERS (cap ${GG_TIMEOUT_S}s)"
            if timeout "$GG_TIMEOUT_S" python3 "$ADAPTER" --endpoint "$GG_ENDPOINT" \
                --query-file "$qf" --engine jena-geosparql \
                --iters "$ITERS" --json > "$TMP/gg-$wl.raw" 2> "$TMP/gg-$wl.json"; then
              # Same sidecar convention as the base family: a COUNT-wrapped
              # query's comparable size is count_value, else len(bindings).
              if GG_PARSED="$(python3 - "$TMP/gg-$wl.json" <<'PYEOF'
import json, sys
line = open(sys.argv[1]).read().strip().splitlines()[-1]
d = json.loads(line)
c = d["count_value"] if d.get("count_value") is not None else d["count"]
print("%s\t%s" % (c, d["query_us"]))
PYEOF
              )"; then
                printf '%s\t%s\n' "$wl" "$GG_PARSED" >> "$TMP/gg-jena-geosparql.tsv"
              else
                printf '%s\tERROR\tsidecar-parse-error\n' "$wl" >> "$TMP/gg-jena-geosparql.tsv"
              fi
            else
              log "jena-geosparql(geographica): $wl FAILED/timeout"
              printf '%s\tERROR\ttimeout-or-error\n' "$wl" >> "$TMP/gg-jena-geosparql.tsv"
            fi
          done
          GG_JENA_STATUS="ok"
        fi
      fi
      [ "$GG_JENA_STATUS" = ok ] || log "jena-geosparql(geographica): $GG_JENA_STATUS"
    fi
  fi

  # ---- the family envelope (same canonical-competitor-results JSON shape) ------
  GG_TS="$(date -u +%Y%m%dT%H%M%SZ)"
  GG_OUT="$OUT_DIR/geo-geographica-${GG_TS}.json"
  CANONICAL="$CANONICAL" GEO_SMOKE="$GEO_SMOKE" GIT_COMMIT="$GIT_COMMIT" \
  GG_CORPUS="$GG_CORPUS" GG_CORPUS_SHA="$GG_CORPUS_SHA" GG_NTRIPLES="$GG_NTRIPLES" \
  ITERS="$ITERS" GG_TIMEOUT_S="$GG_TIMEOUT_S" ONLY="$ONLY" \
  GG_WORKLOADS="$GG_WORKLOADS" GG_EXPECTED="$GG_EXPECTED" TMP="$TMP" GG_OUT="$GG_OUT" \
  GG_FAMILY_STATUS="$GG_FAMILY_STATUS" GG_SPARQ_STATUS="$GG_SPARQ_STATUS" \
  GG_JENA_STATUS="$GG_JENA_STATUS" JENA_VER="$JENA_VER" \
  JENA_JAR_SHA="$JENA_JAR_SHA" GG_JENA_READY_S="$GG_JENA_READY_S" \
  python3 - <<'PYEOF'
import json, os, platform

tmp = os.environ["TMP"]
only = os.environ["ONLY"].split()
workloads = os.environ["GG_WORKLOADS"].split()
canonical = os.environ["CANONICAL"] == "1"
smoke = os.environ["GEO_SMOKE"] == "1"


def read_tsv(name):
    path = os.path.join(tmp, "gg-%s.tsv" % name)
    rows = {}
    if os.path.exists(path):
        for line in open(path):
            f = line.rstrip("\n").split("\t")
            if len(f) >= 3:
                rows[f[0]] = {"count": f[1], "us": f[2]}
    return rows


expected = {}
if os.path.exists(os.environ["GG_EXPECTED"]):
    for line in open(os.environ["GG_EXPECTED"]):
        line = line.strip()
        if line and not line.startswith("#"):
            f = line.split("\t")
            if len(f) >= 2:
                expected[f[0]] = f[1]

engines_meta = {}
if "sparq" in only:
    engines_meta["sparq"] = {
        "version": os.environ["GIT_COMMIT"],
        "mode": (
            "in-process (bench_geo query: load + geof: registry + full SPARQL "
            "eval per pinned .rq, best-of-N; counts cross-checked vs the pinned "
            "expected-geographica.tsv oracle)"
        ),
    }
if "jena-geosparql" in only:
    engines_meta["jena-geosparql"] = {
        "version": os.environ.get("JENA_VER", ""),
        "jar_sha256": os.environ.get("JENA_JAR_SHA", ""),
        "mode": (
            "HTTP (jena-fuseki-geosparql in-memory dataset over the SAME "
            "normalised corpus; http_sparql_adapter.py POST -> SPARQL-JSON, "
            "best-of-N; query_us includes HTTP framing + result parse — a "
            "recorded mode ASYMMETRY vs sparq's in-process surface)"
        ),
        "load_ready_s_advisory": os.environ.get("GG_JENA_READY_S", "n/a"),
    }

sparq_rows = read_tsv("sparq")
jena_rows = read_tsv("jena-geosparql")
statuses = {"family": os.environ["GG_FAMILY_STATUS"]}
if "sparq" in engines_meta:
    statuses["sparq"] = os.environ["GG_SPARQ_STATUS"]
if "jena-geosparql" in engines_meta:
    statuses["jena-geosparql"] = os.environ["GG_JENA_STATUS"]

# INVARIANT (three-way): sparq timing enters only where sparq's count equals
# the PINNED oracle; a jena timing enters only where jena's count equals
# sparq's live count. Every disagreement keeps both counts recorded and the
# timing WITHHELD — never adjusted, never silently dropped.
cross = {}
timings = {}
for w in workloads:
    e = expected.get(w, "n/a")
    s = sparq_rows.get(w, {}).get("count", "n/a")
    j = jena_rows.get(w, {}).get("count", "n/a")
    s_ok = s == e and s not in ("n/a", "ERROR")
    agree = s == j and s not in ("n/a", "ERROR")
    cross[w] = {
        "expected": e,
        "sparq": s,
        "jena-geosparql": j,
        "sparq_matches_expected": s_ok,
        "agree": agree,
    }
    row = {}
    if "sparq" in engines_meta:
        su = sparq_rows.get(w, {}).get("us", "n/a")
        if s in ("n/a", "ERROR"):
            row["sparq_us"] = su
        elif s_ok:
            row["sparq_us"] = su
        else:
            row["sparq_us"] = "WITHHELD(count-drift: sparq=%s expected=%s)" % (s, e)
    if "jena-geosparql" in engines_meta:
        ju = jena_rows.get(w, {}).get("us", "n/a")
        if j in ("n/a", "ERROR"):
            row["jena_geosparql_us"] = ju
        elif agree:
            row["jena_geosparql_us"] = ju
        else:
            row["jena_geosparql_us"] = (
                "WITHHELD(count-disagree: sparq=%s jena=%s)" % (s, j)
            )
    timings[w] = row

note_canonical = (
    "CANONICAL: dedicated quiet box, one engine active at a time on the SAME "
    "pinned corpus + pinned query files; counts cross-checked before any "
    "timing is trusted."
    if canonical
    else "NON-canonical FIRST READ: shared work box (not a dedicated quiet "
    "instance). Timings are directional only — do NOT bake into docs or "
    "dashboards. The harness + pinned recipe are the durable deliverable; the "
    "canonical run is the quiet-box wave (sq-hmd7l.26)."
)

envelope = {
    "gather": "geo-geographica-same-box-comparison",
    "wave": "geo Geographica real-world family (sq-hmd7l.29)",
    "canonical": canonical,
    "canonical_note": note_canonical,
    "smoke": smoke,
    "git_commit": os.environ["GIT_COMMIT"],
    "suite": "geo-geographica",
    "scale": "Geographica real-world LGD/GeoNames slices, %s triples (%s, sha256 %s)"
    % (
        os.environ.get("GG_NTRIPLES", "n/a"),
        os.environ.get("GG_CORPUS", "n/a"),
        os.environ.get("GG_CORPUS_SHA", "n/a"),
    ),
    "corpus_provenance": (
        "bench/geo/geographica.sh: the LGD/GeoNames slices of the Geographica "
        "real-world workload (geographica2.di.uoa.gr), upstream tarballs and "
        "the merged corpus PINNED by sha256 in the recipe. NORMALISATION: the "
        "upstream Strabon-era `<.../crs/EPSG/4326> ` lon-lat anchor is "
        "stripped -> bare CRS84 (long/lat as written), so both engines "
        "interpret every literal identically (left as-is, jena would "
        "axis-swap per the EPSG registry while sparq sees a non-OGC-form IRI "
        "as an opaque CRS — divergent geometry either way)."
    ),
    "iters": int(os.environ["ITERS"]),
    "timeout_s_per_workload": int(os.environ["GG_TIMEOUT_S"]),
    "queries": (
        "bench/geo/queries-geographica/<workload>.rq — pinned COUNT-wrapped "
        "translations of the upstream micro queries (provenance + the exact "
        "translation deltas in each file header); <workload>.jena.rq only "
        "where the standard form is non-executable on jena 5.4.0 (q15: "
        "geof:distance+uom:metre -> spatialF:nearby, same great-circle "
        "semantic; research/gap-geo-2026-07.md §6d)."
    ),
    "engines": engines_meta,
    "statuses": statuses,
    "count_crosscheck": cross,
    "count_crosscheck_note": (
        "COUNTS-NOT-COORDINATES: only result-set SIZES are compared. "
        "expected-geographica.tsv is the pinned oracle (counts derived by "
        "RUNNING both engines on the pinned corpus at pin time, exact "
        "agreement on every bounded workload). sparq timing requires "
        "sparq == expected; a jena timing requires jena == sparq (live). "
        "Disagreement is a recorded RESULT; the timing is withheld, never "
        "adjusted."
    ),
    "timings": timings,
    "timings_note": (
        "per-workload best-of-N microseconds. MODE ASYMMETRY: sparq is "
        "in-process, jena is a full HTTP SPARQL round-trip — recorded, not "
        "adjusted. q19 (the naive cross-product spatial join) typically "
        "records honest ERROR(timeout) rows on BOTH engines at the default "
        "cap: neither engine has an indexed spatial-join path — that "
        "absence IS the comparative result."
    ),
    "env": {
        "host": platform.node(),
        "machine": platform.machine(),
        "os": platform.platform(),
    },
}
for e, rows in (("sparq", sparq_rows), ("jena_geosparql", jena_rows)):
    envelope["%s_tsv" % e] = "\n".join(
        "%s\t%s\t%s" % (w, r["count"], r["us"]) for w, r in rows.items()
    )

with open(os.environ["GG_OUT"], "w") as fh:
    json.dump(envelope, fh, indent=2)
    fh.write("\n")
print(os.environ["GG_OUT"])
PYEOF
  log "geographica envelope: $GG_OUT"
fi

log "done. Gather-only scratch: /tmp/jena-geosparql + /tmp/geographica (delete when finished)."
