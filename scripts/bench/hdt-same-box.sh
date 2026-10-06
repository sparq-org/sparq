#!/usr/bin/env bash
# sq-hmd7l.4 — same-box HDT LOAD-AND-DECODE comparison harness:
# sparq-hdt vs hdt-cpp on the SAME .hdt archive (the snikmeta fixture by default),
# decode-to-native ONLY (load+decode wall-clock), emitting one canonical-competitor-results
# ENVELOPE per gather (the exact JSON shape of bench/canonical-competitor-results/2026-07-07/
# canonical-<suite>-*.json, so scripts/bench/ingest-canonical-competitors.mjs's data flow can
# pick a future canonical HDT gather up unchanged).
#
# HARD COMPARABILITY CAVEAT (already codified in bench/CATALOG.md): load-and-decode-to-native
# is the ONLY like-for-like axis — sparq-hdt decodes HDT into sparq's own Dict/Graph (HDT as an
# ingest format) then queries its own permutation indexes, whereas hdt-cpp/java query the
# compressed BitmapTriples IN PLACE (HDT as a queryable store). A query-over-HDT head-to-head
# (hdtSearch / triple-pattern over the compressed bitmap) is NOT like-for-like and is OUT OF
# SCOPE (different data structure, different work). This script compares DECODE ONLY: the
# structural analogue of sparq-hdt's load+decode-to-native. Cross-check the decoded TRIPLE COUNT
# vs sparq-hdt store.len() BEFORE trusting any timing.
#
# WORKLOAD:
#   * the vendored real-world snikmeta.hdt (crates/sparq-hdt/tests/fixtures/snikmeta.hdt,
#     328 triples, hdt-cpp/java-shaped FourSectionDictionary+BitmapTriples)
#
# METHODOLOGY:
#   * sparq-hdt: examples/bench_oracle decodes the archive IN-PROCESS min-of-N and emits
#     `snikmeta_decode_s` (plus the deterministic count metrics the crosscheck consumes).
#   * hdt-cpp: hdt2rdf decode timed IN-CONTAINER best-of-N — one
#     `docker run` executes a small /bin/sh loop that nanosecond-times each
#     `hdt2rdf <in.hdt> <out.nt>` invocation, so the recorded `decode_us` EXCLUDES the
#     docker container spawn (which dominated the old whole-`docker run` wall figure) but
#     necessarily INCLUDES hdt2rdf's process start + its N-Triples serialisation to file
#     (hdt2rdf is a CLI tool — there is no in-process harness; recorded in the mode string,
#     never hidden). If the in-container timing loop fails (e.g. an image without
#     GNU-date %N), the harness falls back to the old single-run path: count crosscheck +
#     ADVISORY outer wall only, decode_us honestly absent.
#   * per-run wall-clock cap (TIMEOUT_S); a timeout/error degrades to an honest ERROR row,
#     never a fabricated number.
#   * decoded triple-count cross-checked engine-vs-engine: sparq-hdt store.len() == hdt2rdf
#     decoded line count. The archive-derived sparq count is the expected count; no dataset
#     size is pinned in this harness. Only trust timing if counts agree.
#
# USAGE
#   scripts/bench/hdt-same-box.sh            # both engines
#   ONLY="sparq" scripts/bench/hdt-same-box.sh   # sparq-hdt smoke (snikmeta)
#
# TUNABLES (env; all have safe defaults):
#   ONLY          engine subset of "sparq hdt-cpp" (default all)
#   HDT_ITERS     hdt-cpp in-container decode iterations, best-of-N (default 5)
#   TIMEOUT_S     per-run wall-clock cap for EVERY engine, s   (default 60)
#   OUT_DIR       envelope output dir  (default /tmp/hdt-same-box-results;
#                 a canonical run points this at
#                 bench/canonical-competitor-results/<date>/)
#   CANONICAL     1 = a dedicated quiet-box run (default 0: NON-canonical)
#   HDT_ARCHIVE   the .hdt file to decode (default: snikmeta.hdt fixture)
#   BENCH_ORACLE  the sparq-hdt examples/bench_oracle binary (default: build it)
#
# SCRATCH: none — all engines are either built in-tree or Docker-based (gather-only deps, NOT
# committed — engines stay out of git per AGENTS.md).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

ONLY="${ONLY:-sparq hdt-cpp}"
HDT_ITERS="${HDT_ITERS:-5}"
TIMEOUT_S="${TIMEOUT_S:-60}"
OUT_DIR="${OUT_DIR:-/tmp/hdt-same-box-results}"
CANONICAL="${CANONICAL:-0}"
HDT_ARCHIVE="${HDT_ARCHIVE:-$ROOT/crates/sparq-hdt/tests/fixtures/snikmeta.hdt}"
BENCH_ORACLE="${BENCH_ORACLE:-$ROOT/target/release/examples/bench_oracle}"

log() { printf '[hdt-same-box] %s\n' "$*" >&2; }
want() { [[ " $ONLY " == *" $1 "* ]]; }

mkdir -p "$OUT_DIR"
TMP="$(mktemp -d /tmp/hdt-same-box.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

if [ ! -f "$HDT_ARCHIVE" ]; then
  log "ERROR: HDT_ARCHIVE not found: $HDT_ARCHIVE"
  exit 1
fi

# ---- 0. engines --------------------------------------------------------------
if want sparq && [ ! -x "$BENCH_ORACLE" ]; then
  log "building sparq bench_oracle (cargo -p sparq-hdt)"
  cargo build --release -p sparq-hdt --example bench_oracle
fi

# Check if hdt-cpp can run (Docker available)
HDT_CPP_AVAILABLE=0
if want hdt-cpp; then
  if command -v docker &> /dev/null && docker info &> /dev/null; then
    # Try a quick docker pull to check if the image is available or can be pulled
    if docker pull rdfhdt/hdt-cpp:latest &> /dev/null || docker inspect rdfhdt/hdt-cpp:latest &> /dev/null; then
      HDT_CPP_AVAILABLE=1
      HDT_CPP_DIGEST="$(docker inspect --format '{{index .RepoDigests 0}}' rdfhdt/hdt-cpp:latest 2>/dev/null || echo 'rdfhdt/hdt-cpp:latest')"
      log "hdt-cpp available: $HDT_CPP_DIGEST"
    else
      log "hdt-cpp Docker image not available (docker pull failed or image not found)"
    fi
  else
    log "hdt-cpp Docker not available (docker command not found or daemon not running)"
  fi
fi

GIT_COMMIT="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"

# ---- 1. run decode for each engine -------------------------------------------
log "=== HDT decode-only ($(basename "$HDT_ARCHIVE")) ==="

SCALE_TMP="$TMP"
mkdir -p "$SCALE_TMP"

# -- sparq: bench_oracle emits <metric>\t<value>\t<unit> TSV
if want sparq; then
  log "sparq-hdt: bench_oracle decode"
  if ! timeout "$TIMEOUT_S" "$BENCH_ORACLE" "$HDT_ARCHIVE" \
      > "$SCALE_TMP/sparq.tsv" 2> "$SCALE_TMP/sparq.err"; then
    log "sparq-hdt FAILED/timeout (see $SCALE_TMP/sparq.err)"; : > "$SCALE_TMP/sparq.tsv"
  fi
fi

# -- hdt-cpp: hdt2rdf <in.hdt> <out.nt> — the tool REQUIRES an explicit output
# operand ("ERROR: You must supply an input and output" observed verbatim on the
# wave-1 canonical box when stdout-piping was attempted, sq-hmd7l.26); decode into
# a rw-mounted scratch file instead.
#
# sq-hmd7l.33 — NUMERIC decode_us: ONE container runs a /bin/sh loop that
# nanosecond-times each of $HDT_ITERS `hdt2rdf` invocations (GNU date +%s%N — present
# in the debian-based rdfhdt/hdt-cpp image), emitting `HDT_CPP_ITER_NS <n>` lines.
# best-of-N of those is the envelope's decode_us: container spawn EXCLUDED, hdt2rdf
# process start + N-Triples file serialisation INCLUDED (CLI tool; stated in the mode
# string). Lines are digit-validated; if the loop or the validation fails we FALL BACK
# to the old single-run path (count crosscheck + ADVISORY outer wall, decode_us
# honestly absent) rather than record a garbage number.
HDT_CPP_WALL_S="n/a"
HDT_CPP_DECODE_US="n/a"
if want hdt-cpp && [ "$HDT_CPP_AVAILABLE" -eq 1 ]; then
  log "hdt-cpp: hdt2rdf decode x$HDT_ITERS (in-container timing)"
  T0="$(date +%s.%N)"
  if timeout "$TIMEOUT_S" docker run --rm -v "$HDT_ARCHIVE:$HDT_ARCHIVE:ro" \
      -v "$SCALE_TMP:/hdt-out" \
      rdfhdt/hdt-cpp:latest /bin/sh -c '
        set -e
        i=1
        while [ "$i" -le "$2" ]; do
          t0=$(date +%s%N)
          hdt2rdf "$1" /hdt-out/hdt-cpp.nt > /dev/null
          t1=$(date +%s%N)
          echo "HDT_CPP_ITER_NS $((t1-t0))"
          i=$((i+1))
        done' sh "$HDT_ARCHIVE" "$HDT_ITERS" \
      > "$SCALE_TMP/hdt-cpp.out" 2> "$SCALE_TMP/hdt-cpp.err"; then
    HDT_CPP_WALL_S="$(awk -v t0="$T0" -v t1="$(date +%s.%N)" 'BEGIN{printf "%.3f", t1-t0}')"
    # digit-validate every iteration line, then best-of-N (min) → µs.
    BEST_NS="$(grep -E '^HDT_CPP_ITER_NS [0-9]+$' "$SCALE_TMP/hdt-cpp.out" \
      | awk '{ if (min == "" || $2 < min) min = $2 } END { print min }')"
    N_OK="$(grep -cE '^HDT_CPP_ITER_NS [0-9]+$' "$SCALE_TMP/hdt-cpp.out" || true)"
    if [ -n "$BEST_NS" ] && [ "$N_OK" -eq "$HDT_ITERS" ]; then
      HDT_CPP_DECODE_US="$(awk -v ns="$BEST_NS" 'BEGIN{printf "%.1f", ns/1000.0}')"
      log "hdt-cpp decode ok: best-of-$HDT_ITERS ${HDT_CPP_DECODE_US}us (container spawn excluded); outer wall ${HDT_CPP_WALL_S}s advisory"
    else
      log "hdt-cpp: in-container timing lines invalid/incomplete ($N_OK/$HDT_ITERS) — decode_us honestly absent, wall advisory only"
    fi
  else
    # In-container loop failed (image without sh/GNU-date, or decode error): fall back
    # to the original single-run invocation so the COUNT crosscheck still happens.
    log "hdt-cpp: in-container timing loop failed — falling back to single untimed run"
    head -5 "$SCALE_TMP/hdt-cpp.err" 2>/dev/null | while IFS= read -r ln; do
      log "hdt-cpp.err: $ln"
    done
    T0="$(date +%s.%N)"
    if ! timeout "$TIMEOUT_S" docker run --rm -v "$HDT_ARCHIVE:$HDT_ARCHIVE:ro" \
        -v "$SCALE_TMP:/hdt-out" \
        rdfhdt/hdt-cpp:latest hdt2rdf "$HDT_ARCHIVE" /hdt-out/hdt-cpp.nt \
        > "$SCALE_TMP/hdt-cpp.out" 2> "$SCALE_TMP/hdt-cpp.err"; then
      log "hdt-cpp FAILED/timeout (see $SCALE_TMP/hdt-cpp.err)"; : > "$SCALE_TMP/hdt-cpp.nt"
      # Surface the failure reason into the run log itself: $SCALE_TMP is /tmp
      # scratch that between-axis cleanup prunes, so "see the err file" is a
      # dead pointer on a self-terminating gather box (bit sq-hmd7l.26 wave-1).
      head -5 "$SCALE_TMP/hdt-cpp.err" 2>/dev/null | while IFS= read -r ln; do
        log "hdt-cpp.err: $ln"
      done
    else
      HDT_CPP_WALL_S="$(awk -v t0="$T0" -v t1="$(date +%s.%N)" 'BEGIN{printf "%.3f", t1-t0}')"
      log "hdt-cpp decode ok: wall ${HDT_CPP_WALL_S}s ADVISORY (includes container spawn)"
    fi
  fi
elif want hdt-cpp; then
  log "hdt-cpp: SKIPPED (Docker not available or image not found)"
  : > "$SCALE_TMP/hdt-cpp.nt"
fi

# ---- 2. assemble the envelope (canonical-competitor-results JSON shape) ----
TS="$(date -u +%Y%m%dT%H%M%SZ)"
ARCHIVE_STEM="$(printf '%s' "$(basename "$HDT_ARCHIVE" .hdt)" | tr -cs '[:alnum:]_.-' '-')"
OUT="$OUT_DIR/hdt-${ARCHIVE_STEM}-${TS}.json"
CANONICAL="$CANONICAL" HDT_ARCHIVE="$HDT_ARCHIVE" ARCHIVE_NAME="$(basename "$HDT_ARCHIVE")" \
GIT_COMMIT="$GIT_COMMIT" SCALE_TMP="$SCALE_TMP" OUT="$OUT" ONLY="$ONLY" \
TIMEOUT_S="$TIMEOUT_S" HDT_CPP_AVAILABLE="$HDT_CPP_AVAILABLE" \
HDT_CPP_DIGEST="${HDT_CPP_DIGEST:-}" HDT_CPP_WALL_S="$HDT_CPP_WALL_S" \
HDT_CPP_DECODE_US="$HDT_CPP_DECODE_US" HDT_ITERS="$HDT_ITERS" \
python3 - <<'PYEOF'
import json, os, platform

scale_tmp = os.environ["SCALE_TMP"]
only = os.environ["ONLY"].split()
canonical = os.environ["CANONICAL"] == "1"
hdt_cpp_available = os.environ["HDT_CPP_AVAILABLE"] == "1"

# Parse sparq TSV: <metric>\t<value>\t<unit>
sparq_data = {}
sparq_path = os.path.join(scale_tmp, "sparq.tsv")
if os.path.exists(sparq_path):
    for line in open(sparq_path):
        f = line.rstrip("\n").split("\t")
        if len(f) >= 3:
            metric = f[0]
            value = f[1]
            unit = f[2]
            sparq_data[metric] = {"value": value, "unit": unit}

# Count hdt2rdf decoded triples from N-Triples output
hdt_cpp_count = "n/a"
hdt_cpp_path = os.path.join(scale_tmp, "hdt-cpp.nt")
if os.path.exists(hdt_cpp_path) and os.path.getsize(hdt_cpp_path) > 0:
    try:
        with open(hdt_cpp_path, "r") as f:
            hdt_cpp_count = str(sum(1 for line in f if line.strip()))
    except:
        hdt_cpp_count = "ERROR"

engines_meta = {}
if "sparq" in only:
    engines_meta["sparq"] = {
        "version": os.environ["GIT_COMMIT"],
        "mode": "in-process (crates/sparq-hdt/examples/bench_oracle: load+decode from .hdt to native Graph, decode_s + optional triple-pattern resolution)",
    }
if "hdt-cpp" in only:
    engines_meta["hdt-cpp"] = {
        "version": os.environ.get("HDT_CPP_DIGEST", "not-available"),
        "mode": (
            "Docker container (rdfhdt/hdt-cpp:latest, hdt2rdf <archive> <out.nt>): decode .hdt "
            "to an N-Triples file in a rw scratch mount. decode_us = best-of-"
            f"{os.environ.get('HDT_ITERS', '?')} IN-CONTAINER hdt2rdf process wall-clock "
            "(docker container spawn EXCLUDED; hdt2rdf process start + N-Triples file "
            "serialisation INCLUDED — hdt2rdf is a CLI tool with no in-process harness)"
        ) if hdt_cpp_available else "status:absent (Docker not available)",
    }

note_canonical = (
    "CANONICAL: dedicated quiet box, one engine active at a time on the SAME "
    "HDT archive; decode-only, best-of-N on a loaded archive."
    if canonical else
    "NON-canonical FIRST READ: shared work box (not a dedicated quiet instance). "
    "Timings are directional only — do NOT bake into docs/dashboards. The "
    "harness (scripts/bench/hdt-same-box.sh) is the durable deliverable; "
    "rerun with CANONICAL=1 on a dedicated EC2 box for citable numbers."
)

# Decode count cross-check
count_check = {}
sparq_triples = sparq_data.get("snikmeta_triples", {}).get("value", "n/a")
if sparq_triples not in ("n/a", "ERROR"):
    count_check["sparq_triples"] = sparq_triples
if hdt_cpp_count not in ("n/a", "ERROR"):
    count_check["hdt_cpp_triples"] = hdt_cpp_count

# sq-45zbg — derive the oracle from sparq's decoded store rather
# than pinning the tiny fixture's count. hdt2rdf must agree before its timing is
# considered comparable.
agreement = {}
expected = sparq_triples
all_agree = True
if expected not in ("n/a", "ERROR"):
    agreement["sparq"] = sparq_triples
else:
    all_agree = False
if hdt_cpp_count not in ("n/a", "ERROR"):
    agreement["hdt_cpp"] = hdt_cpp_count
    if hdt_cpp_count != expected:
        all_agree = False

env = {
    "host": platform.node(),
    "machine": platform.machine(),
    "os": platform.platform(),
    "timeout_s_per_run": int(os.environ["TIMEOUT_S"]),
}

envelope = {
    "gather": "hdt-same-box-decode-only",
    "wave": "hdt-competitor-baseline (sq-hmd7l.4)",
    "canonical": canonical,
    "canonical_note": note_canonical,
    "git_commit": os.environ["GIT_COMMIT"],
    "suite": "hdt",
    "scale": f"{os.environ['ARCHIVE_NAME']} ({expected} triples, {os.environ['HDT_ARCHIVE']})",
    "archive_path": os.environ["HDT_ARCHIVE"],
    "iters": 1,
    "mode": "decode-only (load+decode wall-clock); query-over-HDT is OUT OF SCOPE (not like-for-like)",
    "caveat": "HARD COMPARABILITY CAVEAT: load-and-decode-to-native is the ONLY like-for-like axis. sparq-hdt decodes HDT into sparq's own Dict/Graph (HDT as an ingest format) then queries its own permutation indexes. hdt-cpp/java query the compressed BitmapTriples IN PLACE (HDT as a queryable store). A query-over-HDT head-to-head (hdtSearch) is NOT like-for-like and is OUT OF SCOPE.",
    "tsv_format": "sparq: <metric>\\t<value>\\t<unit> (from bench_oracle); hdt-cpp: decoded N-Triples file line count",
    "engines": engines_meta,
    "decode_triple_count_crosscheck": count_check,
    "count_agreement": {
        "expected": expected,
        "observed": agreement,
        "all_agree": all_agree,
        "note": "Expected count is derived from sparq-hdt's decoded store; hdt2rdf must match. Only trust timing if counts agree."
    },
    "env": env,
}

# Add metric data from sparq
if sparq_data:
    envelope["sparq_metrics"] = sparq_data

# sq-hmd7l.33 — hdt-cpp NUMERIC decode timing: best-of-N in-container
# hdt2rdf wall-clock (container spawn excluded). This is the field the ingest's
# normalizeHdt renders as the hdt-cpp column cell.
if all_agree and os.environ.get("HDT_CPP_DECODE_US", "n/a") != "n/a":
    envelope["hdt_cpp_metrics"] = {
        "decode_us": {
            "value": os.environ["HDT_CPP_DECODE_US"],
            "unit": "us",
            "note": (
                f"best-of-{os.environ.get('HDT_ITERS', '?')} in-container hdt2rdf process "
                "wall-clock: docker container spawn EXCLUDED; hdt2rdf process start + "
                "N-Triples file serialisation INCLUDED (CLI tool, no in-process harness)"
            ),
        }
    }

# hdt-cpp whole-`docker run` wall-clock — ADVISORY ONLY (includes docker container
# spawn overhead, which dominates on the tiny snikmeta fixture; never a headline).
if os.environ.get("HDT_CPP_WALL_S", "n/a") != "n/a":
    envelope["hdt_cpp_decode_wall_s_advisory"] = {
        "value": os.environ["HDT_CPP_WALL_S"],
        "unit": "s",
        "note": "whole `docker run` incl. container spawn (and all HDT_ITERS decodes when the in-container loop ran); advisory only",
    }

# Add raw hdt-cpp output snippet (first 5 lines, last 5 lines for visibility)
if os.path.exists(hdt_cpp_path) and os.path.getsize(hdt_cpp_path) > 0:
    with open(hdt_cpp_path, "r") as f:
        lines = f.readlines()
        if len(lines) <= 10:
            envelope["hdt_cpp_nt_sample"] = "".join(lines)
        else:
            envelope["hdt_cpp_nt_sample_first_5"] = "".join(lines[:5])
            envelope["hdt_cpp_nt_sample_last_5"] = "".join(lines[-5:])

# Statuses
statuses = {}
if "sparq" in only:
    statuses["sparq"] = "ok" if sparq_data else "failed"
if "hdt-cpp" in only:
    if hdt_cpp_available:
        if os.path.exists(hdt_cpp_path) and os.path.getsize(hdt_cpp_path) > 0:
            statuses["hdt-cpp"] = "ok" if all_agree else "count-mismatch"
        else:
            statuses["hdt-cpp"] = "failed"
    else:
        statuses["hdt-cpp"] = "absent"

envelope["statuses"] = statuses

with open(os.environ["OUT"], "w") as fh:
    json.dump(envelope, fh, indent=2)
    fh.write("\n")
print(os.environ["OUT"])
PYEOF

log "envelope: $OUT"

log "done."
