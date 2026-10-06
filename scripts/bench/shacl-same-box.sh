#!/usr/bin/env bash
# sq-7d3dj.33 — same-box SHACL VALIDATION comparison harness:
# sparq-shacl vs pySHACL vs Apache Jena SHACL on the SAME (data x shapes)
# workloads, emitting one canonical-competitor-results ENVELOPE per scale
# (the exact JSON shape of bench/canonical-competitor-results/2026-07-07/
# canonical-<suite>-*.json, so scripts/bench/ingest-canonical-competitors.mjs's
# data flow can pick a future canonical SHACL gather up unchanged).
#
# SHACL had NO competitor baseline in the canonical matrix (NOT-MEASURED); this
# script is the durable, reusable gather recipe. A run on the shared work box is
# NON-canonical (canonical:false in the envelope, always); a future dedicated
# quiet-box EC2 run sets CANONICAL=1.
#
# WORKLOADS (shared across all three engines):
#   * the 5 committed per-commit gate shapes (bench/shacl/shapes/*.ttl)
#   * the SPARQL-constraint-HEAVY set (bench/shacl/shapes-sparql/sparql_heavy.ttl)
#   over the deterministic LUBM(univ) ABox (bench/shacl/gen.sh), at each scale
#   in $SHACL_UNIVS (default "1 10": ~103k and ~1.3M triples).
#
# METHODOLOGY (mirrors bench/shacl/run.sh + the canonical SPARQL gathers):
#   * ALL engines are timed IN-PROCESS, validate-only, best-of-N on a loaded
#     graph: sparq via examples/bench_shacl, pySHACL via
#     scripts/bench/pyshacl-shacl-bench.py, Jena via
#     scripts/bench/JenaShaclBench.java. Interpreter/JVM start-up and data
#     parse are OUTSIDE the timed section (parse is recorded separately as the
#     advisory load metric).
#   * per-workload wall-clock cap (TIMEOUT_S); a timeout/error degrades to an
#     honest ERROR row, never a fabricated number.
#   * counts cross-checked engine-vs-engine per workload (count_crosscheck).
#     CAVEAT (bench/shacl/expected.tsv): sparq reports one result PER
#     OCCURRENCE (no dedup across traversal routes) — engines that dedup may
#     legitimately differ on shapes with multiple routes; the committed gate
#     shapes are single-route so counts are expected to agree.
#
# USAGE
#   scripts/bench/shacl-same-box.sh            # both scales, all engines
#   SHACL_UNIVS=1 ONLY="sparq pyshacl" scripts/bench/shacl-same-box.sh
#
# TUNABLES (env; all have safe defaults):
#   SHACL_UNIVS   LUBM scales to run                     (default "1 10")
#   SHACL_ITERS   best-of-N per scale, parallel list     (default "3 1")
#   TIMEOUT_S     per-workload cap for EVERY engine, s   (default 900)
#   ONLY          engine subset of "sparq pyshacl jena-shacl" (default all)
#   OUT_DIR       envelope output dir  (default /tmp/shacl-same-box-results;
#                 a canonical run points this at
#                 bench/canonical-competitor-results/<date>/)
#   CANONICAL     1 = a dedicated quiet-box run (default 0: NON-canonical)
#   JENA_HOME     apache-jena distribution root; auto-downloaded to
#                 /tmp/jena-shacl/apache-jena-$JENA_VERSION if unset
#   JENA_VERSION  default 5.4.0 (archive.apache.org)
#   PYSHACL_VENV  venv with pyshacl; auto-created at /tmp/shacl-bench-venv
#   BENCH_SHACL   the sparq bench_shacl binary (default: build it)
#
# SCRATCH: the Jena tarball + pip venv live under /tmp (gather-only deps, NOT
# committed — engines stay out of git per AGENTS.md); delete them when done:
#   rm -rf /tmp/jena-shacl /tmp/shacl-bench-venv
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

SHACL_UNIVS="${SHACL_UNIVS:-1 10}"
SHACL_ITERS="${SHACL_ITERS:-3 1}"
TIMEOUT_S="${TIMEOUT_S:-900}"
ONLY="${ONLY:-sparq pyshacl jena-shacl}"
OUT_DIR="${OUT_DIR:-/tmp/shacl-same-box-results}"
CANONICAL="${CANONICAL:-0}"
JENA_VERSION="${JENA_VERSION:-5.4.0}"
JENA_HOME="${JENA_HOME:-/tmp/jena-shacl/apache-jena-$JENA_VERSION}"
PYSHACL_VENV="${PYSHACL_VENV:-/tmp/shacl-bench-venv}"
BENCH_SHACL="${BENCH_SHACL:-$ROOT/target/release/examples/bench_shacl}"

log() { printf '[shacl-same-box] %s\n' "$*" >&2; }
want() { [[ " $ONLY " == *" $1 "* ]]; }

mkdir -p "$OUT_DIR"
TMP="$(mktemp -d /tmp/shacl-same-box.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

# ---- 0. engines --------------------------------------------------------------
if want sparq && [ ! -x "$BENCH_SHACL" ]; then
  log "building sparq bench_shacl (cargo -p sparq-shacl)"
  cargo build --release -p sparq-shacl --example bench_shacl
fi

if want pyshacl; then
  if [ ! -x "$PYSHACL_VENV/bin/python" ]; then
    log "creating pyshacl venv at $PYSHACL_VENV (gather-only dep)"
    python3 -m venv "$PYSHACL_VENV"
    "$PYSHACL_VENV/bin/pip" -q install pyshacl
  fi
  PYSHACL_VER="$("$PYSHACL_VENV/bin/python" -c 'import pyshacl,rdflib;print(f"pyshacl {pyshacl.__version__} / rdflib {rdflib.__version__}")')"
  log "pyshacl: $PYSHACL_VER"
fi

if want jena-shacl; then
  if [ ! -d "$JENA_HOME/lib" ]; then
    log "downloading apache-jena $JENA_VERSION to /tmp/jena-shacl (gather-only dep)"
    mkdir -p /tmp/jena-shacl
    curl -sSL -o "/tmp/jena-shacl/apache-jena-$JENA_VERSION.tar.gz" \
      "https://archive.apache.org/dist/jena/binaries/apache-jena-$JENA_VERSION.tar.gz"
    tar xzf "/tmp/jena-shacl/apache-jena-$JENA_VERSION.tar.gz" -C /tmp/jena-shacl
  fi
  log "compiling JenaShaclBench against $JENA_HOME/lib"
  javac -proc:none -cp "$JENA_HOME/lib/*" -d "$TMP/jena-classes" "$HERE/JenaShaclBench.java"
  JENA_VER="$(java -cp "$JENA_HOME/lib/*" org.apache.jena.riot.riotcmd.riot --version 2>/dev/null | head -1 || echo "apache-jena-$JENA_VERSION")"
fi

# ---- 1. shared workload dir (gate shapes + the sparql-heavy set) -------------
SHAPES="$TMP/shapes"
mkdir -p "$SHAPES"
cp "$ROOT/bench/shacl/shapes/"*.ttl "$ROOT/bench/shacl/shapes-sparql/"*.ttl "$SHAPES/"

GIT_COMMIT="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"

# ---- 2. one gather per scale --------------------------------------------------
read -r -a UNIVS_ARR <<< "$SHACL_UNIVS"
read -r -a ITERS_ARR <<< "$SHACL_ITERS"

for i in "${!UNIVS_ARR[@]}"; do
  UNIV="${UNIVS_ARR[$i]}"
  ITERS="${ITERS_ARR[$i]:-1}"
  log "=== scale LUBM($UNIV), iters=$ITERS ==="
  mapfile -t ARTS < <("$ROOT/bench/shacl/gen.sh" "$UNIV" 0)
  DATA="${ARTS[0]}"
  NTRIPLES="$(wc -l < "$DATA")"
  log "data=$DATA (~$NTRIPLES triples)"

  SCALE_TMP="$TMP/u$UNIV"
  mkdir -p "$SCALE_TMP"

  # -- sparq: one in-process run over the whole shapes dir (per-workload cap
  #    would need per-shape invocation; the whole-dir run is capped as a unit).
  if want sparq; then
    log "sparq: bench_shacl x$ITERS"
    if ! timeout "$((TIMEOUT_S * 7))" "$BENCH_SHACL" "$DATA" ntriples "$SHAPES" "$ITERS" \
        > "$SCALE_TMP/sparq.tsv" 2> "$SCALE_TMP/sparq.err"; then
      log "sparq FAILED/timeout (see $SCALE_TMP/sparq.err)"; : > "$SCALE_TMP/sparq.tsv"
    fi
  fi

  # -- pySHACL: one in-process run, per-workload signal.alarm cap inside.
  if want pyshacl; then
    log "pyshacl: in-process driver x$ITERS (per-workload cap ${TIMEOUT_S}s)"
    if ! "$PYSHACL_VENV/bin/python" "$HERE/pyshacl-shacl-bench.py" \
        "$DATA" nt "$SHAPES" "$ITERS" "$TIMEOUT_S" \
        > "$SCALE_TMP/pyshacl.tsv" 2> "$SCALE_TMP/pyshacl.err"; then
      log "pyshacl FAILED (see $SCALE_TMP/pyshacl.err)"; : > "$SCALE_TMP/pyshacl.tsv"
    fi
  fi

  # -- Jena: one JVM per workload (clean `timeout` isolation; JVM start-up and
  #    the per-invocation data re-parse stay OUTSIDE the timed validate).
  if want jena-shacl; then
    : > "$SCALE_TMP/jena-shacl.tsv"
    for shp in "$SHAPES"/*.ttl; do
      wl="$(basename "$shp" .ttl)"
      log "jena-shacl: $wl x$ITERS (cap ${TIMEOUT_S}s)"
      if ! timeout "$TIMEOUT_S" java -cp "$JENA_HOME/lib/*:$TMP/jena-classes" \
          JenaShaclBench "$DATA" "$shp" "$ITERS" \
          >> "$SCALE_TMP/jena-shacl.tsv" 2>> "$SCALE_TMP/jena-shacl.err"; then
        printf '%s\tERROR\ttimeout\tna\tna\tna\n' "$wl" >> "$SCALE_TMP/jena-shacl.tsv"
      fi
    done
  fi

  # ---- 3. assemble the envelope (canonical-competitor-results JSON shape) ----
  TS="$(date -u +%Y%m%dT%H%M%SZ)"
  OUT="$OUT_DIR/shacl-lubm${UNIV}-${TS}.json"
  CANONICAL="$CANONICAL" UNIV="$UNIV" ITERS="$ITERS" DATA="$DATA" NTRIPLES="$NTRIPLES" \
  GIT_COMMIT="$GIT_COMMIT" SCALE_TMP="$SCALE_TMP" OUT="$OUT" ONLY="$ONLY" \
  PYSHACL_VER="${PYSHACL_VER:-}" JENA_VER="${JENA_VER:-}" TIMEOUT_S="$TIMEOUT_S" \
  python3 - <<'PYEOF'
import json, os, platform

scale_tmp = os.environ["SCALE_TMP"]
only = os.environ["ONLY"].split()
canonical = os.environ["CANONICAL"] == "1"

def read_tsv(engine):
    path = os.path.join(scale_tmp, f"{engine}.tsv")
    rows = {}
    if os.path.exists(path):
        for line in open(path):
            f = line.rstrip("\n").split("\t")
            if len(f) >= 6:
                # bench_shacl 6-col: name violations us conforms focus load_us
                rows[f[0]] = dict(violations=f[1], us=f[2], conforms=f[3],
                                  focus=f[4], load_us=f[5])
    return rows

engines_meta = {}
if "sparq" in only:
    engines_meta["sparq"] = {
        "version": os.environ["GIT_COMMIT"],
        "mode": "in-process (examples/bench_shacl: load once, validate best-of-N)",
    }
if "pyshacl" in only:
    engines_meta["pyshacl"] = {
        "version": os.environ.get("PYSHACL_VER", ""),
        "mode": "in-process (scripts/bench/pyshacl-shacl-bench.py: rdflib load once, pyshacl.validate best-of-N, signal.alarm cap)",
    }
if "jena-shacl" in only:
    engines_meta["jena-shacl"] = {
        "version": os.environ.get("JENA_VER", ""),
        "mode": "in-process (scripts/bench/JenaShaclBench.java: RDFDataMgr load, ShaclValidator best-of-N; one JVM per workload under `timeout`)",
    }

data = {e: read_tsv(e.replace("jena-shacl", "jena-shacl")) for e in engines_meta}
workloads = sorted({w for rows in data.values() for w in rows})

note_canonical = (
    "CANONICAL: dedicated quiet box, one engine active at a time on the SAME "
    "LUBM ABox + SAME shape graphs; validate-only best-of-N on a loaded graph."
    if canonical else
    "NON-canonical FIRST READ: shared work box (not a dedicated quiet instance). "
    "Timings are directional only — do NOT bake into docs/dashboards. The "
    "harness (scripts/bench/shacl-same-box.sh) is the durable deliverable; "
    "rerun with CANONICAL=1 on a dedicated EC2 box for citable numbers."
)

cross = {}
for w in workloads:
    row = {}
    counts = set()
    for e in engines_meta:
        r = data[e].get(w)
        v = r["violations"] if r else "n/a"
        row[e] = v
        row[f"{e}_conforms"] = (r or {}).get("conforms", "n/a")
        if v not in ("n/a", "ERROR"):
            counts.add(v)
    row["all_agree"] = len(counts) == 1
    cross[w] = row

env = {
    "host": platform.node(),
    "machine": platform.machine(),
    "os": platform.platform(),
    "timeout_s_per_workload": int(os.environ["TIMEOUT_S"]),
}

envelope = {
    "gather": "shacl-same-box-comparison",
    "wave": "shacl-competitor-baseline (sq-7d3dj.33)",
    "canonical": canonical,
    "canonical_note": note_canonical,
    "git_commit": os.environ["GIT_COMMIT"],
    "suite": "shacl",
    "scale": f"LUBM({os.environ['UNIV']}) ABox, {os.environ['NTRIPLES']} triples ({os.environ['DATA']})",
    "iters": int(os.environ["ITERS"]),
    "tsv_format": "<workload>\\t<violations|ERROR>\\t<validate_best_us|reason>",
    "engines": engines_meta,
    "load": {
        f"{e}_load_s": (
            f"{float(next(iter(data[e].values()))['load_us']) / 1e6:.3f}"
            if data[e] and next(iter(data[e].values()))["load_us"] not in ("na",)
            else "n/a"
        )
        for e in engines_meta
    },
    "statuses": {e: ("ok" if data[e] else "failed") for e in engines_meta},
    "count_crosscheck": cross,
    "count_crosscheck_note": (
        "per-workload #violations engine-vs-engine (+ conforms). sparq reports "
        "one result PER OCCURRENCE (no dedup across traversal routes; see "
        "bench/shacl/expected.tsv caveat) — a dedup engine may legitimately "
        "differ on multi-route shapes; the committed workloads are single-route."
    ),
    "env": env,
}
for e in engines_meta:
    envelope[f"{e.replace('-', '_')}_tsv"] = "\n".join(
        f"{w}\t{data[e][w]['violations']}\t{data[e][w]['us']}"
        for w in workloads if w in data[e]
    )

with open(os.environ["OUT"], "w") as fh:
    json.dump(envelope, fh, indent=2)
    fh.write("\n")
print(os.environ["OUT"])
PYEOF
  log "envelope: $OUT"
done

log "done. Scratch deps live in /tmp/jena-shacl + $PYSHACL_VENV (delete when finished)."
