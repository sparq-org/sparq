#!/usr/bin/env bash
# sq-hmd7l.2 — same-box FULL-TEXT-SEARCH comparison harness:
# sparq-text (embedded BM25 inverted index) vs Apache Jena Fuseki + jena-text
# (Lucene) on the SAME synthetic Zipf corpus and the SAME pinned 200-pair query
# set, emitting one canonical-competitor-results ENVELOPE per run (the
# scripts/bench/shacl-same-box.sh template shape).
#
# FTS had a REGISTERED-but-never-run competitor (bench/competitors.json id
# `jena-text`, gather-only); this script is the durable, reusable gather recipe.
# A run on the shared work box is NON-canonical (canonical:false, always); the
# canonical wave-1 EC2 run (sq-hmd7l.26) sets CANONICAL=1 on a quiet box.
#
# WORKLOADS (bench/fts suite; translations pinned in bench/fts/queries-jena-text/):
#   and_terms / or_terms / prefix4 / phrase  — comparable result-SET counts
#     (Lucene StandardAnalyzer and sparq-text's UAX#29 tokenizer segment this
#     ASCII corpus identically; ranking/score order differs and is OUT of scope).
#   near_slop2 — sparq-only: Lucene's '"a b"~N' is transposition-tolerant
#     (unordered) vs sparq's ordered-within-gap text:near => different result
#     sets, so the column is honestly ABSENT (recorded in the envelope).
#
# METHODOLOGY (mirrors shacl-same-box.sh + the canonical SPARQL gathers):
#   * sparq: in-process via examples/bench_text (build once, query best-of-N);
#     jena-text: HTTP keep-alive loopback via scripts/bench/jena-text-fts-driver.py
#     against a Fuseki mem dataset + mem Lucene index (fuseki-config.ttl, pinned).
#     The MODE ASYMMETRY (embedded vs HTTP server surface) is recorded in the
#     envelope — counts are unaffected, jena-text timings include HTTP round-trip.
#   * INVARIANT — oracle before stopwatch: per-workload hit counts are
#     cross-checked engine-vs-engine (count_crosscheck) and recorded AS RETURNED;
#     a mismatch is a recorded result (agree:false => that workload's comparative
#     timing is NOT citable), never an adjustment or a rerun-until-it-matches.
#   * corpus + query pairs for the competitor come from
#     scripts/bench/fts-corpus-gen (a pinned-rand replica of bench_text's
#     in-process generator, verified against bench/fts/expected.tsv); the pinned
#     bench/fts/queries-jena-text/pairs.tsv is re-derived every run and a drift
#     is RECORDED (pairs_pinned_check), never patched over.
#   * per-workload wall-clock cap (TIMEOUT_S); timeout/error degrades to an
#     honest ERROR row. An absent/unprovisionable competitor is a RECORDED skip
#     (absent tool => absent column), exit 0 — except under CANONICAL=1, where a
#     requested-but-missing competitor fails the run (a canonical gather must
#     not silently drop a column).
#
# USAGE
#   scripts/bench/fts-same-box.sh                       # both engines, pinned corpus
#   ONLY=sparq FTS_SMOKE=1 scripts/bench/fts-same-box.sh   # fast sparq-only smoke
#   JENA_TEXT_ENDPOINT=http://host:3030/fts/query ...   # external pre-loaded Fuseki
#
# TUNABLES (env; all have safe defaults):
#   FTS_N          corpus literals            (default 100000; FTS_SMOKE: 20000)
#   FTS_SEED       corpus RNG seed            (default 0 — the expected.tsv pin)
#   FTS_ITERS      best-of-N per workload     (default 3; FTS_SMOKE: 1)
#   FTS_SMOKE      1 = fast smoke defaults + smoke:true in the envelope (default 0)
#   TIMEOUT_S      per-workload cap, seconds  (default 900)
#   ONLY           engine subset of "sparq jena-text" (default both)
#   OUT_DIR        envelope dir (default /tmp/fts-same-box-results; a canonical
#                  run points this at bench/canonical-competitor-results/<date>/)
#   CANONICAL      1 = dedicated quiet-box run (default 0: NON-canonical)
#   FUSEKI_VERSION apache-jena-fuseki version (default 5.4.0, archive.apache.org)
#   FUSEKI_DIR     unpacked distribution root (default /tmp/jena-fuseki/...)
#   JENA_TEXT_PORT local Fuseki port          (default 3047)
#   JENA_TEXT_ENDPOINT  external ALREADY-LOADED Fuseki+jena-text query endpoint;
#                  skips download/start/load (the registry's endpoint_env)
#   BENCH_TEXT     sparq bench_text binary    (default: build it)
#
# SCRATCH: the Fuseki tarball lives under /tmp (gather-only dep, NOT committed —
# engines stay out of git per AGENTS.md): rm -rf /tmp/jena-fuseki when done.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

FTS_SMOKE="${FTS_SMOKE:-0}"
if [ "$FTS_SMOKE" = 1 ]; then
  FTS_N="${FTS_N:-20000}"
  FTS_ITERS="${FTS_ITERS:-1}"
else
  FTS_N="${FTS_N:-100000}"
  FTS_ITERS="${FTS_ITERS:-3}"
fi
FTS_SEED="${FTS_SEED:-0}"
TIMEOUT_S="${TIMEOUT_S:-900}"
ONLY="${ONLY:-sparq jena-text}"
OUT_DIR="${OUT_DIR:-/tmp/fts-same-box-results}"
CANONICAL="${CANONICAL:-0}"
FUSEKI_VERSION="${FUSEKI_VERSION:-5.4.0}"
FUSEKI_DIR="${FUSEKI_DIR:-/tmp/jena-fuseki/apache-jena-fuseki-$FUSEKI_VERSION}"
JENA_TEXT_PORT="${JENA_TEXT_PORT:-3047}"
JENA_TEXT_ENDPOINT="${JENA_TEXT_ENDPOINT:-}"
BENCH_TEXT="${BENCH_TEXT:-$ROOT/target/release/examples/bench_text}"
CORPUS_GEN="$ROOT/scripts/bench/fts-corpus-gen/target/release/fts-corpus-gen"
QUERIES_DIR="$ROOT/bench/fts/queries-jena-text"

log() { printf '[fts-same-box] %s\n' "$*" >&2; }
want() { [[ " $ONLY " == *" $1 "* ]]; }

mkdir -p "$OUT_DIR"
TMP="$(mktemp -d /tmp/fts-same-box.XXXXXX)"
FUSEKI_PID=""
cleanup() {
  if [ -n "$FUSEKI_PID" ] && kill -0 "$FUSEKI_PID" 2>/dev/null; then
    kill "$FUSEKI_PID" 2>/dev/null || true
    wait "$FUSEKI_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT

GIT_COMMIT="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
SPARQ_STATUS="not-requested (ONLY)"
JENA_STATUS="not-requested (ONLY)"
PAIRS_CHECK="skipped (jena-text not active)"
DOC_COUNT_CHECK="n/a"
JENA_LOAD_S="n/a"
JENA_VER=""
JENA_MODE=""
: > "$TMP/sparq.tsv"
: > "$TMP/jena-text.tsv"

# ---- 1. sparq: in-process bench_text over the (FTS_N, FTS_SEED) corpus --------------
if want sparq; then
  if [ ! -x "$BENCH_TEXT" ]; then
    log "building sparq bench_text (cargo -p sparq-text --example bench_text)"
    cargo build --release -p sparq-text --example bench_text
  fi
  log "sparq: bench_text N=$FTS_N seed=$FTS_SEED iters=$FTS_ITERS"
  if timeout "$((TIMEOUT_S * 6))" "$BENCH_TEXT" "$FTS_N" "$FTS_SEED" "$FTS_ITERS" \
      > "$TMP/sparq.tsv" 2> "$TMP/sparq.err"; then
    SPARQ_STATUS="ok"
  else
    SPARQ_STATUS="failed"
    log "sparq FAILED/timeout (see $TMP/sparq.err)"
    : > "$TMP/sparq.tsv"
  fi
fi

# ---- 2. jena-text: Fuseki + Lucene over the SAME corpus + pinned translations -------
if want jena-text; then
  JENA_SKIP=""

  # 2a. java (Fuseki runtime).
  if ! command -v java >/dev/null 2>&1; then
    JENA_SKIP="java not available"
  fi

  # 2b. the corpus/pairs generator (pinned-rand replica of bench_text's generator).
  if [ -z "$JENA_SKIP" ] && [ ! -x "$CORPUS_GEN" ]; then
    log "building fts-corpus-gen (standalone, rand pinned to the workspace version)"
    if ! cargo build --release \
        --manifest-path "$ROOT/scripts/bench/fts-corpus-gen/Cargo.toml" \
        > "$TMP/corpus-gen-build.log" 2>&1; then
      JENA_SKIP="fts-corpus-gen build failed (see $TMP/corpus-gen-build.log)"
    fi
  fi

  # 2c. pairs_pinned_check: re-derive the pinned pairs; a drift is RECORDED (the
  #     pinned file stays the translation source of truth — never auto-adjusted).
  if [ -z "$JENA_SKIP" ]; then
    "$CORPUS_GEN" queries > "$TMP/pairs-live.tsv"
    grep -v '^#' "$QUERIES_DIR/pairs.tsv" > "$TMP/pairs-pinned.tsv"
    if diff -q "$TMP/pairs-pinned.tsv" "$TMP/pairs-live.tsv" >/dev/null; then
      PAIRS_CHECK="ok (pinned pairs == regenerated pairs)"
    else
      PAIRS_CHECK="DRIFT (pinned bench/fts/queries-jena-text/pairs.tsv != regenerated; workspace rand stream moved? recorded, not adjusted — re-derive the pin + expected.tsv together)"
      log "WARNING: $PAIRS_CHECK"
    fi
  fi

  # 2d. endpoint: external (pre-loaded) or a local provisioned Fuseki.
  EP="$JENA_TEXT_ENDPOINT"
  if [ -z "$JENA_SKIP" ] && [ -z "$EP" ]; then
    if [ ! -f "$FUSEKI_DIR/fuseki-server.jar" ]; then
      log "downloading apache-jena-fuseki $FUSEKI_VERSION to /tmp/jena-fuseki (gather-only dep)"
      mkdir -p /tmp/jena-fuseki
      if ! curl -sSfL -o "/tmp/jena-fuseki/apache-jena-fuseki-$FUSEKI_VERSION.tar.gz" \
          "https://archive.apache.org/dist/jena/binaries/apache-jena-fuseki-$FUSEKI_VERSION.tar.gz" \
          || ! tar xzf "/tmp/jena-fuseki/apache-jena-fuseki-$FUSEKI_VERSION.tar.gz" -C /tmp/jena-fuseki; then
        JENA_SKIP="fuseki download/unpack failed"
      fi
    fi
    if [ -z "$JENA_SKIP" ]; then
      log "starting Fuseki (port $JENA_TEXT_PORT, mem dataset + mem Lucene index)"
      mkdir -p "$TMP/fuseki-base"
      FUSEKI_BASE="$TMP/fuseki-base" java -jar "$FUSEKI_DIR/fuseki-server.jar" \
        --config="$QUERIES_DIR/fuseki-config.ttl" --port="$JENA_TEXT_PORT" \
        > "$TMP/fuseki.log" 2>&1 &
      FUSEKI_PID=$!
      up=""
      for _ in $(seq 1 60); do
        if curl -sf "http://127.0.0.1:${JENA_TEXT_PORT}/\$/ping" >/dev/null 2>&1; then
          up=1; break
        fi
        sleep 1
      done
      if [ -z "$up" ]; then
        JENA_SKIP="fuseki failed to start (see $TMP/fuseki.log)"
      else
        EP="http://127.0.0.1:${JENA_TEXT_PORT}/fts/query"
        # Load the SAME corpus (indexing happens on commit => load time INCLUDES
        # Lucene indexing; recorded as the advisory load metric).
        log "generating + loading corpus (N=$FTS_N seed=$FTS_SEED) via GSP"
        "$CORPUS_GEN" corpus "$FTS_N" "$FTS_SEED" > "$TMP/corpus.nt"
        t0=$(date +%s.%N)
        if curl -sSf -o /dev/null -X POST -H 'Content-Type: application/n-triples' \
            --data-binary @"$TMP/corpus.nt" \
            --max-time "$TIMEOUT_S" \
            "http://127.0.0.1:${JENA_TEXT_PORT}/fts/data?default"; then
          JENA_LOAD_S="$(printf '%s %s' "$t0" "$(date +%s.%N)" | awk '{printf "%.1f", $2 - $1}')"
        else
          JENA_SKIP="corpus load failed (see $TMP/fuseki.log)"
        fi
      fi
    fi
    JENA_VER="apache-jena-fuseki-$FUSEKI_VERSION ($(java -version 2>&1 | head -1))"
    JENA_MODE="HTTP keep-alive loopback (local Fuseki, mem dataset + mem Lucene index per bench/fts/queries-jena-text/fuseki-config.ttl; scripts/bench/jena-text-fts-driver.py)"
  elif [ -z "$JENA_SKIP" ]; then
    JENA_VER="external endpoint: $EP (version not probed)"
    JENA_MODE="HTTP keep-alive vs an EXTERNAL pre-loaded Fuseki+jena-text endpoint (JENA_TEXT_ENDPOINT; corpus load not performed by this harness)"
  fi

  # 2e. doc-count oracle: the loaded store must hold exactly FTS_N triples.
  if [ -z "$JENA_SKIP" ]; then
    got="$(python3 -c '
import sys
sys.path.insert(0, "scripts/bench-adapters")
from http_sparql_adapter import do_query, parse_sparql_json
r = parse_sparql_json(do_query(sys.argv[1], "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }"))
print(r["count_value"])
' "$EP" 2>/dev/null || echo ERROR)"
    if [ "$got" = "$FTS_N" ]; then
      DOC_COUNT_CHECK="ok (store holds $got triples == N)"
    else
      DOC_COUNT_CHECK="MISMATCH (store holds '$got', expected N=$FTS_N; recorded, not adjusted)"
      log "WARNING: $DOC_COUNT_CHECK"
    fi
  fi

  # 2f. the pinned-translation query run (3-col TSV; ERROR rows are honest results).
  if [ -z "$JENA_SKIP" ]; then
    log "jena-text: driver x$FTS_ITERS (per-workload cap ${TIMEOUT_S}s)"
    if python3 "$HERE/jena-text-fts-driver.py" "$EP" "$QUERIES_DIR" \
        "$QUERIES_DIR/pairs.tsv" "$FTS_ITERS" "$TIMEOUT_S" \
        > "$TMP/jena-text.tsv" 2> "$TMP/jena-text.err"; then
      JENA_STATUS="ok"
    else
      JENA_STATUS="failed (see $TMP/jena-text.err)"
      log "jena-text driver FAILED (see $TMP/jena-text.err)"
    fi
  else
    JENA_STATUS="skipped: $JENA_SKIP"
    log "jena-text SKIP: $JENA_SKIP (absence recorded in the envelope)"
  fi
fi

# ---- 3. envelope (canonical-competitor-results JSON shape) --------------------------
TS="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="$OUT_DIR/fts-n${FTS_N}-${TS}.json"
CANONICAL="$CANONICAL" SMOKE="$FTS_SMOKE" N="$FTS_N" SEED="$FTS_SEED" ITERS="$FTS_ITERS" \
GIT_COMMIT="$GIT_COMMIT" TMP="$TMP" OUT="$OUT" ONLY="$ONLY" TIMEOUT_S="$TIMEOUT_S" \
SPARQ_STATUS="$SPARQ_STATUS" JENA_STATUS="$JENA_STATUS" PAIRS_CHECK="$PAIRS_CHECK" \
DOC_COUNT_CHECK="$DOC_COUNT_CHECK" JENA_LOAD_S="$JENA_LOAD_S" JENA_VER="$JENA_VER" \
JENA_MODE="$JENA_MODE" \
python3 - <<'PYEOF'
import json, os, platform

tmp = os.environ["TMP"]
only = os.environ["ONLY"].split()
canonical = os.environ["CANONICAL"] == "1"
smoke = os.environ["SMOKE"] == "1"

def read_tsv(name):
    rows = {}
    path = os.path.join(tmp, name + ".tsv")
    if os.path.exists(path):
        for line in open(path):
            f = line.rstrip("\n").split("\t")
            if len(f) >= 3:
                rows[f[0]] = (f[1], f[2])  # (count|ERROR, us|reason)
    return rows

sparq = read_tsv("sparq")
jena = read_tsv("jena-text")

engines_meta = {}
if "sparq" in only:
    engines_meta["sparq"] = {
        "version": os.environ["GIT_COMMIT"],
        "mode": "in-process (examples/bench_text: build_with_positions once, "
                "query best-of-N over the embedded BM25 index)",
    }
if "jena-text" in only:
    engines_meta["jena-text"] = {
        "version": os.environ["JENA_VER"],
        "mode": os.environ["JENA_MODE"] or "not provisioned",
    }

TRANSLATED = ["and_terms", "or_terms", "prefix4", "phrase"]
cross = {}
for w in TRANSLATED:
    s = sparq.get(w, ("n/a", ""))[0]
    j = jena.get(w, ("n/a", ""))[0]
    numeric = all(v not in ("n/a", "ERROR") for v in (s, j))
    row = {
        "sparq": s,
        "jena_text": j,
        "agree": (s == j) if numeric else "n/a",
    }
    # INVARIANT: no comparative timing without hit-count agreement. The counts
    # above are recorded AS RETURNED (a mismatch stays in the record).
    row["comparative_timing_ok"] = row["agree"] is True
    cross[w] = row
cross["near_slop2"] = {
    "sparq": sparq.get("near_slop2", ("n/a", ""))[0],
    "jena_text": "n/a (untranslated: Lucene '~N' proximity is transposition-tolerant/"
                 "unordered vs sparq text:near's ordered-within-gap — different result "
                 "sets; see bench/fts/queries-jena-text/README.md)",
    "agree": "n/a",
    "comparative_timing_ok": False,
}

note_canonical = (
    "CANONICAL: dedicated quiet box, one engine active at a time on the SAME "
    "synthetic corpus + SAME pinned 200-pair query set; best-of-N on a loaded/"
    "warmed index." if canonical else
    "NON-canonical FIRST READ: shared work box (not a dedicated quiet instance). "
    "Timings are directional only — do NOT bake into docs/dashboards. The harness "
    "(scripts/bench/fts-same-box.sh) is the durable deliverable; the canonical "
    "wave-1 EC2 run (sq-hmd7l.26) reruns it with CANONICAL=1 for citable numbers."
)

def sparq_build_s():
    r = sparq.get("bytes_per_doc")
    if not r or r[1] in ("", "na"):
        return "n/a"
    try:
        return "%.3f" % (float(r[1]) / 1e6)
    except ValueError:
        return "n/a"

envelope = {
    "gather": "fts-same-box-comparison",
    "wave": "fts-competitor-baseline (sq-hmd7l.2)",
    "canonical": canonical,
    "canonical_note": note_canonical,
    "smoke": smoke,
    "git_commit": os.environ["GIT_COMMIT"],
    "suite": "fts",
    "registry_suite": "text-index-bench",
    "scale": "synthetic Zipf corpus, N=%s eight-word literals, seed=%s "
             "(bench/fts/gen.sh parameters; pinned per-commit tier is N=100000 seed=0)"
             % (os.environ["N"], os.environ["SEED"]),
    "iters": int(os.environ["ITERS"]),
    "queries": "the FIXED 200 two-term pairs (bench/fts/queries-jena-text/pairs.tsv; "
               "counts are TOTALS summed over the set)",
    "tsv_format": "<workload>\\t<total_hits|ERROR>\\t<best_us_per_query|reason>",
    "engines": engines_meta,
    "mode_asymmetry_note": (
        "sparq-text is an EMBEDDED in-process index; jena-text is measured over HTTP "
        "(Fuseki is its deployed surface, per the bench/competitors.json adapter kind). "
        "jena-text per-query timings therefore include loopback HTTP round-trip "
        "overhead; result-set COUNTS are unaffected. Ranking/score order is out of "
        "scope (analyzers/BM25 statistics differ — registry caveat)."
    ),
    "load": {
        "sparq_build_s": sparq_build_s(),
        "jena_text_load_s": os.environ["JENA_LOAD_S"]
                            + ("" if os.environ["JENA_LOAD_S"] == "n/a"
                               else " (GSP POST incl. Lucene indexing)"),
    },
    "statuses": {
        "sparq": os.environ["SPARQ_STATUS"],
        "jena-text": os.environ["JENA_STATUS"],
    },
    "pairs_pinned_check": os.environ["PAIRS_CHECK"],
    "doc_count_check": os.environ["DOC_COUNT_CHECK"],
    "count_crosscheck": cross,
    "count_crosscheck_note": (
        "per-workload TOTAL matching-doc counts, engine-vs-engine, recorded AS "
        "RETURNED — a disagreement is a recorded result (agree:false disqualifies "
        "that workload's comparative timing), never an adjustment or a rerun-until-"
        "it-matches. near_slop2 has no honest jena-text translation (semantics "
        "mismatch) and bytes_per_doc is a sparq-internal footprint gate; both are "
        "sparq-only by design. jena-text queries pin an explicit 10000000 result "
        "limit (its 10000 default would truncate prefix4 counts)."
    ),
    "env": {
        "host": platform.node(),
        "machine": platform.machine(),
        "os": platform.platform(),
        "timeout_s_per_workload": int(os.environ["TIMEOUT_S"]),
    },
    "sparq_tsv": "\n".join(
        "%s\t%s\t%s" % (w, c, u) for w, (c, u) in sorted(sparq.items())
    ),
    "jena_text_tsv": "\n".join(
        "%s\t%s\t%s" % (w, c, u) for w, (c, u) in sorted(jena.items())
    ),
}

with open(os.environ["OUT"], "w") as fh:
    json.dump(envelope, fh, indent=2)
    fh.write("\n")
print(os.environ["OUT"])
PYEOF

# Self-assert: the envelope must be well-formed JSON (the acceptance check).
python3 -m json.tool "$OUT" > /dev/null
log "envelope: $OUT"

# A CANONICAL run must not silently drop a requested competitor column.
if [ "$CANONICAL" = 1 ]; then
  if want sparq && [ "$SPARQ_STATUS" != ok ]; then
    log "CANONICAL run: sparq status '$SPARQ_STATUS' — failing loudly"; exit 1
  fi
  if want jena-text && [ "$JENA_STATUS" != ok ]; then
    log "CANONICAL run: jena-text status '$JENA_STATUS' — failing loudly"; exit 1
  fi
fi

log "done. Scratch dep lives in /tmp/jena-fuseki (delete when finished)."
