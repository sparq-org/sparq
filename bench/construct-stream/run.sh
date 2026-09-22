#!/usr/bin/env bash
# [FABLE-5] sq-0kq6k — streamed-vs-buffered CONSTRUCT/DESCRIBE response measurement.
#
# THE HYPOTHESIS UNDER TEST. sparq-server now writes a CONSTRUCT / DESCRIBE response body
# through a chunk sink instead of rendering the whole RDF document into one `String` first
# (`stream_graph_result` in crates/sparq-server/src/http.rs). Two things should follow, and
# this harness measures BOTH rather than asserting them:
#
#   1. PEAK RSS — the server should no longer hold the rendered document on top of the
#      materialised `Vec<Triple>`, so peak resident memory for a large CONSTRUCT should drop
#      by roughly the rendered document's size.
#   2. TTFB — the first bytes should reach the socket after ~one chunk instead of after the
#      whole document is rendered, so time-to-first-byte should fall while total time does not
#      rise (the same serialisation work is done either way).
#
# THE CONTROL IS THE REAL BUFFERED PATH, NOT A FLAG. A `HEAD` request takes the SAME
# CONSTRUCT through the SAME engine and the SAME serialiser and then renders the whole
# document into a `String` — that is exactly the code the `GET` path ran before this change
# (HEAD must advertise the `Content-Length` a GET would carry, so it cannot stream). So:
#
#     buffered arm = HEAD  (renders the whole document, then answers)
#     streamed arm = GET   (answers after the first chunk, streams the rest)
#
# Each arm runs against its OWN server process, because peak RSS (`VmHWM`) is monotonic — one
# process cannot report a "before" and an "after".
#
# NO NUMBERS ARE BAKED IN. This script measures and prints; it asserts no thresholds and
# carries no expected values. Readings taken on a shared/work box are NON-CANONICAL by
# construction — a canonical reading needs the quiet-box protocol (see bench/ec2-bench.sh and
# the `quiet_box_sensitive` flag on this benchmark in bench/benchmarks.toml).
#
# GATES BEFORE STOPWATCH (repo convention). Before any timing is reported the run proves the
# two arms agree: the streamed GET body must be byte-length-identical to the `Content-Length`
# the buffered HEAD advertises, and the streamed body must re-parse to the expected triple
# count. A red gate exits non-zero and reports NO timings.
#
# USAGE
#   bash bench/construct-stream/run.sh --smoke   # tiny corpus, gates only; exit 0 = green
#   bash bench/construct-stream/run.sh           # full sweep
#
# TUNABLES (env; safe defaults)
#   SPARQ_SERVER_BIN   server binary       (default target/release/sparq-server)
#   CS_PORT            loopback port       (default 7041)
#   CS_SUBJECTS        corpus subjects     (default 400000; --smoke uses 2000)
#   CS_ITERS           timed iterations    (default 5; --smoke uses 2)
#   CS_ACCEPT          response syntax     (default application/n-triples; try text/turtle)
#   CS_READY_TIMEOUT   readiness cap, s    (default 60)
#   CS_FANOUT          result multiplier   (default 1) — the CONSTRUCT template emits this many
#                      triples per matched solution. THE IMPORTANT KNOB FOR THE RSS HYPOTHESIS:
#                      `CS_SUBJECTS` scales the store and the result TOGETHER, so the rendered
#                      document stays a fixed fraction of a process whose high-water mark was
#                      already set during load, and the allocator absorbs the transient render.
#                      Raising `CS_FANOUT` grows the rendered document WITHOUT growing the
#                      store — the regime where a buffered render should actually dominate
#                      peak RSS, and therefore the regime a canonical run must include.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

SMOKE=0
for arg in "$@"; do
  case "$arg" in
    --smoke) SMOKE=1 ;;
    *) echo "[construct-stream] unknown arg: $arg (usage: run.sh [--smoke])" >&2; exit 2 ;;
  esac
done

SPARQ_SERVER_BIN="${SPARQ_SERVER_BIN:-$ROOT/target/release/sparq-server}"
CS_PORT="${CS_PORT:-7041}"
CS_ACCEPT="${CS_ACCEPT:-application/n-triples}"
CS_READY_TIMEOUT="${CS_READY_TIMEOUT:-60}"
if [ "$SMOKE" = 1 ]; then
  CS_SUBJECTS="${CS_SUBJECTS:-2000}"
  CS_ITERS="${CS_ITERS:-2}"
else
  CS_SUBJECTS="${CS_SUBJECTS:-400000}"
  CS_ITERS="${CS_ITERS:-5}"
fi
CS_FANOUT="${CS_FANOUT:-1}"

for tool in curl awk; do
  command -v "$tool" >/dev/null 2>&1 || { echo "[construct-stream] missing required tool: $tool" >&2; exit 2; }
done
if [ ! -x "$SPARQ_SERVER_BIN" ]; then
  echo "[construct-stream] server binary not found at $SPARQ_SERVER_BIN" >&2
  echo "[construct-stream] build it first: cargo build --release -p sparq-server" >&2
  exit 2
fi

WORK="$(mktemp -d)"
SERVER_PID=""
cleanup() {
  if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

# --- corpus ------------------------------------------------------------------
# Deterministic, blank-node-free, ASCII N-Triples. One subject, two triples each, so the
# rendered Turtle exercises subject-block grouping as well as the flat N-Triples line form.
CORPUS="$WORK/corpus.nt"
echo "[construct-stream] generating $CS_SUBJECTS subjects -> $CORPUS"
awk -v n="$CS_SUBJECTS" 'BEGIN {
  for (i = 0; i < n; i++) {
    printf "<http://ex/s%d> <http://xmlns.com/foaf/0.1/name> \"subject-%d-padding-padding\" .\n", i, i
    printf "<http://ex/s%d> <http://xmlns.com/foaf/0.1/knows> <http://ex/s%d> .\n", i, (i + 1) % n
  }
}' > "$CORPUS"
EXPECT_TRIPLES=$((CS_SUBJECTS * 2))

# The CONSTRUCT template: `CS_FANOUT` copies of each matched solution, so the rendered
# document is `CS_FANOUT` x the store's triple count while the store itself is unchanged.
#
# Each extra copy uses a FRESH PREDICATE (not a fresh subject). A CONSTRUCT result is an RDF
# GRAPH — a SET — so any template clause that can coincide with another collapses and the
# triple-count gate goes red. Varying the predicate keeps `(?s, copyN, ?o)` distinct from both
# the source triple and every other copy, for every solution.
build_query() {
  local tmpl="?s ?p ?o ." i
  for i in $(seq 2 "$CS_FANOUT"); do
    tmpl="$tmpl ?s <http://ex/copy$i> ?o ."
  done
  printf 'CONSTRUCT { %s } WHERE { ?s ?p ?o }' "$tmpl"
}
QUERY="$(build_query)"
EXPECT_TRIPLES=$((EXPECT_TRIPLES * CS_FANOUT))

# --- server lifecycle --------------------------------------------------------
start_server() {
  "$SPARQ_SERVER_BIN" --addr "127.0.0.1:$CS_PORT" --format ntriples "$CORPUS" \
    >"$WORK/server.log" 2>&1 &
  SERVER_PID=$!
  local waited=0
  while [ "$waited" -lt "$CS_READY_TIMEOUT" ]; do
    if curl -fsS -o /dev/null "http://127.0.0.1:$CS_PORT/health" 2>/dev/null; then
      return 0
    fi
    kill -0 "$SERVER_PID" 2>/dev/null || { echo "[construct-stream] server died:"; cat "$WORK/server.log"; exit 1; }
    sleep 1
    waited=$((waited + 1))
  done
  echo "[construct-stream] server not ready after ${CS_READY_TIMEOUT}s" >&2
  cat "$WORK/server.log" >&2
  exit 1
}

stop_server() {
  [ -n "$SERVER_PID" ] || return 0
  kill "$SERVER_PID" 2>/dev/null || true
  wait "$SERVER_PID" 2>/dev/null || true
  SERVER_PID=""
}

# Peak resident set of the LIVE server process, in KiB (Linux only; VmHWM is the kernel's own
# high-water mark, so it needs no sampling loop).
peak_rss_kib() {
  awk '/^VmHWM:/ {print $2}' "/proc/$SERVER_PID/status" 2>/dev/null || echo ""
}

# --- one timed request -------------------------------------------------------
# Prints "<ttfb_seconds> <total_seconds> <bytes>" for one request.
# $1 = curl method flag set ("" for GET, "--head" for HEAD).
timed_request() {
  local method_flag="$1"
  # shellcheck disable=SC2086 # $method_flag is an intentional word-split flag slot
  curl -sS -o /dev/null $method_flag \
    -H "Accept: $CS_ACCEPT" \
    --data-urlencode "query=$QUERY" \
    -G "http://127.0.0.1:$CS_PORT/sparql" \
    -w '%{time_starttransfer} %{time_total} %{size_download}\n'
}

median() { sort -n | awk '{v[NR]=$1} END {if (NR==0) {print "n/a"} else if (NR%2) {print v[(NR+1)/2]} else {printf "%.6f", (v[NR/2]+v[NR/2+1])/2}}'; }

# --- gates before stopwatch --------------------------------------------------
echo "[construct-stream] === correctness gates ==="
start_server

HEAD_LEN="$(curl -sS --head -H "Accept: $CS_ACCEPT" --data-urlencode "query=$QUERY" \
  -G "http://127.0.0.1:$CS_PORT/sparql" | awk 'BEGIN{IGNORECASE=1} /^content-length:/ {gsub(/\r/,""); print $2}')"
GET_LEN="$(curl -sS -o "$WORK/body.out" -H "Accept: $CS_ACCEPT" --data-urlencode "query=$QUERY" \
  -G "http://127.0.0.1:$CS_PORT/sparql" -w '%{size_download}')"

if [ -z "$HEAD_LEN" ]; then
  echo "[construct-stream] GATE RED: the buffered HEAD advertised no Content-Length" >&2
  exit 4
fi
if [ "$HEAD_LEN" != "$GET_LEN" ]; then
  echo "[construct-stream] GATE RED: streamed body is $GET_LEN bytes, buffered HEAD advertised $HEAD_LEN" >&2
  exit 4
fi
echo "[construct-stream]   streamed body == buffered length ($GET_LEN bytes) — OK"

# The streamed document must carry every triple. For N-Triples that is a line count; for any
# other syntax we only check the body is non-empty (re-parsing is the serializer oracle
# suite's job — crates/sparq-server/tests/graph_serializer_oracle.rs).
if [ "$CS_ACCEPT" = "application/n-triples" ]; then
  GOT_TRIPLES="$(wc -l < "$WORK/body.out" | tr -d ' ')"
  if [ "$GOT_TRIPLES" != "$EXPECT_TRIPLES" ]; then
    echo "[construct-stream] GATE RED: streamed body has $GOT_TRIPLES triples, expected $EXPECT_TRIPLES" >&2
    exit 4
  fi
  echo "[construct-stream]   streamed body carries all $EXPECT_TRIPLES triples — OK"
else
  [ -s "$WORK/body.out" ] || { echo "[construct-stream] GATE RED: empty streamed body" >&2; exit 4; }
  echo "[construct-stream]   streamed body non-empty ($GET_LEN bytes, $CS_ACCEPT) — OK"
fi

# The gate must be able to fail: a corrupted body must NOT match the advertised length.
printf 'x' >> "$WORK/body.out"
if [ "$(wc -c < "$WORK/body.out" | tr -d ' ')" = "$GET_LEN" ]; then
  echo "[construct-stream] SELF-CHECK RED: the length gate cannot distinguish a corrupted body" >&2
  exit 4
fi
echo "[construct-stream]   non-vacuity self-check (corrupted body is detected) — OK"
stop_server

if [ "$SMOKE" = 1 ]; then
  echo "[construct-stream] smoke run green (gates only; no timings claimed)"
  exit 0
fi

# --- measurement -------------------------------------------------------------
echo "[construct-stream] === measurement ($CS_ITERS iterations per arm, Accept: $CS_ACCEPT) ==="

measure_arm() {
  local label="$1" method_flag="$2" ttfbs="" totals="" i out
  start_server
  # One warm-up request, untimed and outside the RSS reading, so page-cache and allocator
  # warm-up do not land in either the timings or the high-water mark of the timed run.
  timed_request "$method_flag" >/dev/null
  for i in $(seq 1 "$CS_ITERS"); do
    out="$(timed_request "$method_flag")"
    ttfbs="$ttfbs$(echo "$out" | awk '{print $1}')"$'\n'
    totals="$totals$(echo "$out" | awk '{print $2}')"$'\n'
  done
  local rss; rss="$(peak_rss_kib)"
  stop_server
  local ttfb_med total_med
  ttfb_med="$(printf '%s' "$ttfbs" | median)"
  total_med="$(printf '%s' "$totals" | median)"
  awk -v l="$label" -v t="$ttfb_med" -v o="$total_med" -v r="${rss:-0}" \
    'BEGIN {printf "| %-8s | %14.1f | %14.1f | %13.1f |\n", l, t*1000, o*1000, r/1024}'
}

echo
echo "| arm      | median TTFB ms | median total ms | peak RSS MiB |"
echo "|----------|----------------|-----------------|--------------|"
measure_arm streamed ""
measure_arm buffered "--head"
echo
cat <<'NOTE'
Reading the table
  - `buffered` is a HEAD: it renders the whole document into one String and then answers, i.e.
    exactly what a GET did before sq-0kq6k. Its `total` therefore excludes body transfer, so
    ONLY the TTFB column and the peak-RSS column are a like-for-like comparison; do not read
    the streamed/buffered `total` columns against each other.
  - peak RSS is the process high-water mark (VmHWM) and includes the loaded store, which is
    identical in both arms — the DIFFERENCE between the arms is the rendered-document saving,
    not the absolute value.
  - These are readings from whatever box you ran on. They are NOT canonical unless gathered
    under the quiet-box protocol (bench/ec2-bench.sh). Do not transcribe them into any doc,
    README, dashboard or site page as a sparq performance number.
NOTE
