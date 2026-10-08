#!/usr/bin/env bash
# sq-52fo — dedicated same-box QLever index->server->query->teardown recipe.
#
# WHY THIS SCRIPT EXISTS
#   QLever is NOT a simple file-in/answer-out CLI like Oxigraph or EYE. To benchmark a
#   QLever query you must (1) build an on-disk INDEX over the dataset with qlever-index (the
#   modern binary; was IndexBuilderMain), then (2) start a long-lived qlever-server (was
#   ServerMain) HTTP server on a port, then (3) query it over HTTP, then (4) STOP the server
#   and delete the index. The old same-box gather inlined a
#   fragile version of this in gather-ec2-sparql.sh's user-data heredoc with NO server
#   teardown and weak bounds, so a failed/slow index build or a server that never became
#   ready left the gather hanging (~53 min observed) and could leak a running container.
#
#   This script is the SELF-CONTAINED, BOUNDED, ORPHAN-SAFE recipe. Every external process
#   it starts (the QLever server container) and every byte of scratch it writes (the temp
#   index dir) is removed by an EXIT trap — on success, on failure, on timeout, on SIGINT.
#   Every wait is hard-bounded; there is no unbounded poll anywhere.
#
# USAGE
#   scripts/qlever-same-box.sh <corpus-file> <format> <queries-dir> <iters> <out-tsv>
#     <corpus-file>  path to the dataset (Turtle .ttl or N-Triples .nt)
#     <format>       ttl | nt   (QLever -F file-format for IndexBuilderMain)
#     <queries-dir>  dir of *.rq SPARQL queries (e.g. bench/sp2b/queries)
#     <iters>        min-of-K per query
#     <out-tsv>      where to write the result TSV (<name>\t<rows>\t<best_us>)
#
#   Exit status: 0 if the index built, the server became ready, and at least one query
#   produced a non-ERROR row; non-zero otherwise. The caller treats a non-zero exit as
#   "qlever skipped/failed" and keeps the dashboard cell honest-n/a — it NEVER hangs the
#   gather, because every step here is timeout-bounded.
#
# TUNABLES (env; all have safe defaults):
#   QLEVER_IMAGE        Docker image                 (default docker.io/adfreiburg/qlever:latest)
#   QLEVER_PORT         server port                  (default 7001)
#   QLEVER_INDEX_TIMEOUT  index-build hard cap, s    (default 1200 = 20 min)
#   QLEVER_PULL_TIMEOUT   image-pull hard cap, s     (default 600  = 10 min)
#   QLEVER_READY_TIMEOUT  server-readiness cap, s    (default 120  = 2 min)
#   QLEVER_QUERY_TIMEOUT  per-HTTP-query cap, s      (default 60)
#   QLEVER_JOBS         ServerMain worker threads    (default 4)
#   QLEVER_MIN_FREE_GB  abort index if free disk <   (default 10)
#   QLEVER_NAME         container name               (default qlever-srv-$$)
#
# OUTPUT TSV FORMAT (matches the harness's <name>\t<rows>\t<best_us>):
#   q01<TAB>123<TAB>4567.8        — rows + best wall micros over <iters> runs
#   q07<TAB>ERROR<TAB>qlever      — query failed (server error / timeout)
# The caller (gather-ec2-sparql.sh) JSON-encodes this file into the same-box envelope's
# "qlever_tsv" field and sets qlever_status from this script's exit code.
#
# ORPHAN-SAFETY / BOUNDEDNESS GUARANTEES (the bug this fixes):
#   * trap-cleanup ALWAYS runs (EXIT) and: docker rm -f the server container (idempotent,
#     never errors if absent) + rm -rf the temp index dir. No orphan server, no leaked disk.
#   * docker pull / IndexBuilderMain / each HTTP query all run under `timeout` — no step can
#     block forever. The readiness poll is a bounded for-loop, not `while :`.
#   * df guard before the (disk-heavy) index build; refuses to start if free space is low.
set -euo pipefail

CORPUS="${1:?usage: qlever-same-box.sh <corpus> <ttl|nt> <queries-dir> <iters> <out-tsv>}"
FORMAT="${2:?missing format (ttl|nt)}"
QUERIES_DIR="${3:?missing queries dir}"
ITERS="${4:?missing iters}"
OUT_TSV="${5:?missing out-tsv}"

QLEVER_IMAGE="${QLEVER_IMAGE:-docker.io/adfreiburg/qlever:latest}"
QLEVER_PORT="${QLEVER_PORT:-7001}"
QLEVER_INDEX_TIMEOUT="${QLEVER_INDEX_TIMEOUT:-1200}"
QLEVER_PULL_TIMEOUT="${QLEVER_PULL_TIMEOUT:-600}"
QLEVER_READY_TIMEOUT="${QLEVER_READY_TIMEOUT:-120}"
QLEVER_QUERY_TIMEOUT="${QLEVER_QUERY_TIMEOUT:-60}"
QLEVER_JOBS="${QLEVER_JOBS:-4}"
QLEVER_MIN_FREE_GB="${QLEVER_MIN_FREE_GB:-10}"
QLEVER_NAME="${QLEVER_NAME:-qlever-srv-$$}"
HTTP_PROFILE="${HTTP_PROFILE:-0}"

# sq-7d3dj.34: HTTP_PROFILE=1 routes the query loop through the SHARED
# scripts/bench-adapters/http_sparql_adapter.py in --profile mode (6-col rows: keep-alive
# + fresh-connect full-request latency AND TTFB). The default 3-col inline loop below is
# byte-for-byte unchanged.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ADAPTER="$SCRIPT_DIR/bench-adapters/http_sparql_adapter.py"

log()  { printf '[qlever] %s\n' "$*" >&2; }
die()  { printf '[qlever] ERROR: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

case "$FORMAT" in ttl|nt) ;; *) die "format must be 'ttl' or 'nt' (got '$FORMAT')" ;; esac
case "$ITERS" in ''|*[!0-9]*) die "iters must be a positive integer (got '$ITERS')" ;; esac
[ "$ITERS" -ge 1 ] || die "iters must be >= 1"
[ -f "$CORPUS" ] || die "corpus file not found: $CORPUS"
[ -d "$QUERIES_DIR" ] || die "queries dir not found: $QUERIES_DIR"
have docker || die "docker not installed (required for the QLever image)"
# sq-vw3ax.12.1 — DAEMON PREFLIGHT (fixes the Wave-0 fast-fail). Wave 0 apt-installed
# docker.io but the daemon never came up (`systemctl start docker` swallowed by `|| true`), so
# `have docker` PASSED, the bounded `docker pull` printed "pull failed/slow — continuing", and the
# first `docker run` (index build) failed ~instantly with "Cannot connect to the Docker daemon" —
# an opaque ~9s cascade recorded only as qlever_status:"failed". Probe the daemon ONCE, up front,
# and die with an ACTIONABLE message instead. (gather-ec2-sparql.sh now also does
# `systemctl enable --now docker` + a socket wait before invoking this recipe.)
docker info >/dev/null 2>&1 || die "Docker daemon not reachable (start it: 'sudo systemctl enable --now docker' and wait for the socket) — refusing to run; this is the Wave-0 fast-fail (binary present, daemon down)"
have python3 || die "python3 required (HTTP query client + TSV emit)"

# ---- scratch index dir + ALWAYS-RUN teardown -------------------------------------------
INDEX_DIR="$(mktemp -d "${TMPDIR:-/tmp}/qlever-idx.XXXXXX")"
INDEX_BASE="$INDEX_DIR/idx"   # IndexBuilderMain -i base name (writes idx.* alongside)

cleanup() {
  set +e
  # sq-52fo: tear the server container down UNCONDITIONALLY (rm -f is idempotent —
  # exits 0 even if the container was never created or already gone), then drop the temp
  # index. This is the fix for the leaked-server / leaked-disk bug: it fires on success,
  # on `die`, on a `timeout` SIGTERM bubbling up, and on Ctrl-C.
  docker rm -f "$QLEVER_NAME" >/dev/null 2>&1
  rm -rf "$INDEX_DIR" >/dev/null 2>&1
  log "teardown: removed container '$QLEVER_NAME' (if any) + index dir"
}
trap cleanup EXIT
trap 'exit 130' INT TERM   # ensure the EXIT trap runs on signal

# ---- df guard (index build is the disk-heavy step) -------------------------------------
free_gb="$(df -BG --output=avail "$INDEX_DIR" 2>/dev/null | tail -1 | tr -dc '0-9' || true)"
if [ -n "$free_gb" ]; then
  log "disk: ${free_gb} GB free (floor ${QLEVER_MIN_FREE_GB} GB)"
  [ "$free_gb" -ge "$QLEVER_MIN_FREE_GB" ] \
    || die "free disk ${free_gb} GB < floor ${QLEVER_MIN_FREE_GB} GB — refusing QLever index build"
else
  log "could not read free disk space; skipping df guard"
fi

# ---- 0. pull the image (bounded) -------------------------------------------------------
log "pulling $QLEVER_IMAGE (<= ${QLEVER_PULL_TIMEOUT}s)"
timeout "$QLEVER_PULL_TIMEOUT" docker pull -q "$QLEVER_IMAGE" \
  || log "pull failed/slow — continuing with any locally cached image"

# Resolve a host path the container can bind-mount. Both the corpus and the index dir are
# mounted read-only/read-write respectively under /data.
CORPUS_DIR="$(cd "$(dirname "$CORPUS")" && pwd)"
CORPUS_FILE="$(basename "$CORPUS")"

# ---- 1. INDEX BUILD (bounded) ----------------------------------------------------------
# sq-vw3ax.12.1 — REWRITTEN for the modern QLever image (>= 0.5.x; tested against
# 0.5.48). TWO recipe bugs are fixed here (the real breakage under the Wave-0 daemon symptom):
#   (a) BINARY RENAME: `IndexBuilderMain` -> `qlever-index`, `ServerMain` -> `qlever-server`.
#       The old names are gone, so the old `docker run ... IndexBuilderMain` failed outright.
#   (b) SETTINGS-JSON QUOTING: the old `bash -lc "IndexBuilderMain ... -s '$SETTINGS' < file"`
#       embedded a JSON blob CONTAINING double-quotes inside a double-quoted `-lc` string, so
#       the shell aborted with `unexpected EOF while looking for matching "` (rc=2) before the
#       build even started. The settings file is OPTIONAL; we drop it and pass the corpus as a
#       plain FILE (`-f`) instead of stdin, which removes the `bash -lc "... < file"` wrapper
#       (and its quoting hazard) entirely.
# We also BYPASS the image ENTRYPOINT (`--entrypoint qlever-index`) — that entrypoint remaps
# UID/GID and demands the `-c "..."` form — and run as root (`-u 0:0`) so writes to the mounted
# index dir Just Work regardless of the host uid. The binaries live in /qlever (already on PATH).
#   qlever-index -i <base> -F <fmt> -f <file>   (writes <base>.index.* files)
log "building index (<= ${QLEVER_INDEX_TIMEOUT}s) from $CORPUS_FILE [$FORMAT]"
if timeout "$QLEVER_INDEX_TIMEOUT" docker run --rm -u 0:0 --entrypoint qlever-index \
      -v "$CORPUS_DIR":/in:ro -v "$INDEX_DIR":/data \
      "$QLEVER_IMAGE" \
      -i /data/idx -F "$FORMAT" -f "/in/$CORPUS_FILE" \
      >&2; then
  log "index build OK"
else
  rc=$?
  [ "$rc" = 124 ] && die "index build hit the ${QLEVER_INDEX_TIMEOUT}s timeout — aborting (no hang)"
  die "index build failed (rc=$rc)"
fi
# Modern QLever writes permutation files as <base>.index.<perm>; keep the legacy .meta probe too.
[ -e "${INDEX_BASE}.index.pos" ] || [ -e "${INDEX_BASE}.meta" ] || ls "$INDEX_DIR" >&2 || true

# ---- 2. START SERVER + bounded readiness poll ------------------------------------------
# qlever-server (was ServerMain) serves the built index over HTTP:
#   -i <base>   the index base built above
#   -p <port>   listen port
#   -j <n>      simultaneous queries
# Bypass the entrypoint + run as root (same reasons as the index build). Run detached (-d)
# with a fixed --name so teardown can target it; publish the port.
log "starting qlever-server on :$QLEVER_PORT (container '$QLEVER_NAME')"
docker rm -f "$QLEVER_NAME" >/dev/null 2>&1 || true   # belt-and-braces: no stale same-name
docker run -d --name "$QLEVER_NAME" -u 0:0 --entrypoint qlever-server \
  -p "${QLEVER_PORT}:${QLEVER_PORT}" \
  -v "$INDEX_DIR":/data "$QLEVER_IMAGE" \
  -i /data/idx -p "$QLEVER_PORT" -j "$QLEVER_JOBS" >/dev/null \
  || die "failed to start qlever-server container"

# Bounded readiness poll — a FOR loop with a hard count, NOT `while :`. Probes a trivial
# query; the server answers it once the index is mmapped and the HTTP listener is up.
PROBE='SELECT * WHERE { ?s ?p ?o } LIMIT 1'
ready=0
poll_n=$(( QLEVER_READY_TIMEOUT / 2 ))
[ "$poll_n" -ge 1 ] || poll_n=1
for _ in $(seq 1 "$poll_n"); do
  # If the container has already died, stop polling immediately (don't burn the budget).
  if ! docker ps --filter "name=^${QLEVER_NAME}$" --filter status=running -q | grep -q .; then
    log "server container exited during startup — dumping last logs:"
    docker logs --tail 30 "$QLEVER_NAME" >&2 2>&1 || true
    die "QLever server did not stay up"
  fi
  if curl -fsS --max-time 5 \
      --data-urlencode "query=$PROBE" \
      -H 'Accept: application/sparql-results+json' \
      "http://localhost:${QLEVER_PORT}" >/dev/null 2>&1; then
    ready=1; break
  fi
  sleep 2
done
[ "$ready" = 1 ] || die "server not ready within ${QLEVER_READY_TIMEOUT}s — aborting (no hang)"
log "server ready"

# ---- 3. RUN QUERIES over HTTP, emit TSV ------------------------------------------------
# One bounded HTTP request per (query, iter); min wall micros over <iters>. The Python
# client uses urllib (always present) with a per-request timeout, so a single slow query
# cannot exceed QLEVER_QUERY_TIMEOUT and the whole loop is bounded by
# (#queries * iters * QLEVER_QUERY_TIMEOUT) in the absolute worst case.
: > "$OUT_TSV"
shopt -s nullglob
any_ok=0
for q in "$QUERIES_DIR"/*.rq; do
  name="$(basename "$q" .rq)"
  if [ "$HTTP_PROFILE" = "1" ]; then
    # 6-col profile rows via the shared adapter (keep-alive/fresh + TTFB). brace-group
    # `|| true` is LOAD-BEARING under set -euo pipefail (see fuseki-same-box.sh).
    row="$({ timeout "$QLEVER_QUERY_TIMEOUT" python3 "$ADAPTER" \
               --endpoint "http://localhost:${QLEVER_PORT}" --query-file "$q" \
               --engine qlever --iters "$ITERS" --profile 2>/dev/null || true; } \
           | awk -F'\t' -v n="$name" 'NR==1{$1=n; print}' OFS='\t')"
    [ -n "$row" ] || row="$name	ERROR	qlever"
    printf '%s\n' "$row" >> "$OUT_TSV"
    case "$row" in *$'\tERROR\t'*) ;; *) any_ok=1 ;; esac
    continue
  fi
  row="$(QLEVER_PORT="$QLEVER_PORT" QLEVER_QUERY_TIMEOUT="$QLEVER_QUERY_TIMEOUT" ITERS="$ITERS" \
    python3 - "$q" "$name" <<'PY'
import os, sys, time, json, urllib.parse, urllib.request
qf, name = sys.argv[1], sys.argv[2]
port = os.environ["QLEVER_PORT"]
to   = float(os.environ["QLEVER_QUERY_TIMEOUT"])
iters = int(os.environ["ITERS"])
endpoint = f"http://localhost:{port}"
query = open(qf, encoding="utf-8").read()
best = None
rows = "ERROR"
for _ in range(iters):
    data = urllib.parse.urlencode({"query": query}).encode()
    req = urllib.request.Request(
        endpoint, data=data,
        headers={"Accept": "application/sparql-results+json"})
    t = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=to) as r:
            obj = json.loads(r.read())
    except Exception:
        rows = "ERROR"; best = None; break
    res = obj.get("results")
    if isinstance(res, dict) and "bindings" in res:
        rows = len(res["bindings"])
    elif "boolean" in obj:
        rows = 1 if obj["boolean"] else 0
    else:
        rows = "ERROR"; best = None; break
    us = (time.perf_counter() - t) * 1e6
    best = us if best is None else min(best, us)
if rows == "ERROR" or best is None:
    print(f"{name}\tERROR\tqlever")
else:
    print(f"{name}\t{rows}\t{best:.1f}")
PY
)"
  printf '%s\n' "$row" >> "$OUT_TSV"
  case "$row" in *$'\tERROR\t'*) ;; *) any_ok=1 ;; esac
done
shopt -u nullglob

log "wrote $OUT_TSV:"
cat "$OUT_TSV" >&2 || true

# trap cleanup tears down the server + index on the way out (any exit path).
[ "$any_ok" = 1 ] || die "no query produced a non-ERROR row"
log "done (>=1 query succeeded)"
