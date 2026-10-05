#!/usr/bin/env bash
# bytes-per-triple.sh (sq-7d3dj.32) — deterministic bytes/triple AT SCALE, in BOTH
# framings the RDFox comparison needs (research/rdfox-claims-inventory.md §2.4):
#
#   1. IN-MEMORY  : `sparq-cli memstat` self-accounted heap B/triple, decomposed into
#                   dictionary / six-permutation store / literal-value caches, plus the
#                   kernel's post-load VmRSS and peak VmHWM (allocator + parse transients);
#                   measured in BOTH in-RAM storage modes — raw and the memory-bound
#                   `into_compressed` (block-compressed perms + blob dict).
#   2. EXTERNAL   : on-disk index directory size (raw and block-compressed `save`), and the
#                   peak RSS of the BOUNDED-RAM external `build` (sparq's pinned low-resource
#                   story — a different axis from RDFox's in-memory numbers; measure both,
#                   editorialise in neither).
#
# Corpus: the REAL WatDiv generator (bench/watdiv/gen.sh, deterministic seed-1) at ~1M/10M/50M
# triples (SF = scale/100000). The CI-scale ancestor of this instrument is the perf-gate metric
# `store_bytes_per_triple` (scripts/ci-bench.sh); this script exists because that figure is
# scale-sensitive (fixed dictionary/index costs amortise differently) and the RDFox claims are
# large-scale. Canonical runs go on ONE dedicated quiet EC2 box via bench/ec2-bench.sh
# (bench id `bytes-per-triple`); work-box runs are smoke tests, not canonical numbers.
#
# CONSOLE DISCIPLINE (64KB serial buffer, see bench/ec2-bench.sh): stdout carries ONLY the
# compact per-scale `MEMBPT ...` result lines + one final single-line JSON envelope; all
# progress goes to stderr; cargo/watdiv build noise is tail-trimmed.
#
# DISK DISCIPLINE (feedback-disk-space): free space is checked BEFORE each generation
# (corpus ~115 B/triple + raw indexes 72 B/triple + compressed indexes); index dirs are
# removed after measurement; the corpus cache is removed too unless KEEP_CORPUS=1.
#
#   usage: scripts/bench/bytes-per-triple.sh [out.json]
#   env  : SCALES="1000000 10000000 50000000"  target triple counts (SF rounds to /100000)
#          WATDIV_CACHE=/tmp/watdiv            corpus cache dir
#          KEEP_CORPUS=0                       keep generated corpora
#          SKIP_EXTERNAL=0                     skip the save/build external-memory arm
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="${1:-}"
SCALES="${SCALES:-1000000 10000000 50000000}"
CACHE="${WATDIV_CACHE:-/tmp/watdiv}"
KEEP_CORPUS="${KEEP_CORPUS:-0}"
SKIP_EXTERNAL="${SKIP_EXTERNAL:-0}"
CLI="$REPO/target/release/sparq-cli"
SCRATCH="$(mktemp -d)"
# On a non-zero exit, surface the tail of the quiet-arm log before the scratch dir goes away.
trap 'rc=$?; if [ "$rc" -ne 0 ] && [ -s "$SCRATCH/save.log" ]; then echo "[bpt] save/build log tail:" >&2; tail -5 "$SCRATCH/save.log" >&2; fi; rm -rf "$SCRATCH"; exit "$rc"' EXIT

log() { echo "[bpt] $*" >&2; }

# Free bytes on the filesystem holding $1.
free_bytes() { df --output=avail -B1 "$1" 2>/dev/null | tail -1 | tr -d ' '; }

log "building sparq-cli (release, default features: mmap+mimalloc+dict-spill+jsonld)..."
(cd "$REPO" && cargo build --release -p sparq-cli 2>&1 | tail -3 >&2)
[ -x "$CLI" ] || { log "FATAL: $CLI missing after build"; exit 1; }

command -v /usr/bin/time >/dev/null || { log "FATAL: /usr/bin/time (GNU time) required for peak-RSS"; exit 1; }

# TSV field out of a memstat capture.
ms_field() { awk -F'\t' -v k="$2" '$1==k{print $2}' "$1"; }
# "Maximum resident set size (kbytes)" out of a GNU-time -v capture, in bytes.
time_hwm_bytes() { awk '/Maximum resident set size/{print $NF*1024}' "$1"; }

RESULTS=()
for SCALE in $SCALES; do
  SF=$(( SCALE / 100000 )); [ "$SF" -ge 1 ] || SF=1
  CORPUS="$CACHE/watdiv-sf$SF.nt"

  # Disk watch: corpus (~115 B/triple) + raw indexes (~90 B/triple) + compressed (~40).
  NEED=$(( SCALE * 250 ))
  FREE="$(free_bytes "${CACHE%/*}" || echo 0)"
  mkdir -p "$CACHE"
  if [ ! -s "$CORPUS" ] && [ "${FREE:-0}" -lt "$NEED" ]; then
    log "SKIP scale=$SCALE: need ~$NEED bytes free, have $FREE"
    echo "MEMBPT scale=$SCALE status=skipped-disk free=$FREE need=$NEED"
    continue
  fi

  log "scale=$SCALE: WatDiv SF=$SF -> $CORPUS"
  bash "$REPO/bench/watdiv/gen.sh" "$SF" "$CORPUS" >/dev/null
  TRIPLES_RAW=$(wc -l < "$CORPUS" | tr -d ' ')
  CORPUS_BYTES=$(wc -c < "$CORPUS" | tr -d ' ')
  # Sanity: the generator must land near the target (WatDiv SF=1 ~ 105K triples).
  LO=$(( SCALE * 90 / 100 )); HI=$(( SCALE * 130 / 100 ))
  if [ "$TRIPLES_RAW" -lt "$LO" ] || [ "$TRIPLES_RAW" -gt "$HI" ]; then
    log "FATAL scale=$SCALE: generator produced $TRIPLES_RAW lines, expected $LO..$HI"
    echo "MEMBPT scale=$SCALE status=generator-off lines=$TRIPLES_RAW"
    exit 1
  fi

  # ---- framing 1: in-memory ------------------------------------------------------------
  MS="$SCRATCH/memstat-$SCALE.tsv"; TV="$SCRATCH/time-load-$SCALE.txt"
  log "scale=$SCALE: memstat (in-RAM load)..."
  /usr/bin/time -v -o "$TV" "$CLI" memstat "$CORPUS" ntriples > "$MS"
  TRIPLES=$(ms_field "$MS" triples)          # deduped triple count (the denominator of record)
  TERMS=$(ms_field "$MS" dict_terms)
  HEAP_BPT=$(ms_field "$MS" heap_b_per_triple)
  STORE_BPT=$(ms_field "$MS" store_b_per_triple)
  DICT_BPT=$(ms_field "$MS" dict_b_per_triple)
  CACHES_BPT=$(ms_field "$MS" caches_b_per_triple)
  DICT_BPTERM=$(ms_field "$MS" dict_b_per_term)
  RSS_BPT=$(ms_field "$MS" rss_b_per_triple)
  HWM_BPT=$(ms_field "$MS" hwm_b_per_triple)
  LOAD_S=$(ms_field "$MS" load_s)
  GNU_HWM=$(time_hwm_bytes "$TV")            # cross-check of memstat's VmHWM

  # In-RAM MEMORY-BOUND mode (block-compressed perms + blob dict, Graph::into_compressed) —
  # the other end of the in-memory footprint/latency trade. Byte counts are deterministic.
  MSC="$SCRATCH/memstat-comp-$SCALE.tsv"
  log "scale=$SCALE: memstat compressed (memory-bound in-RAM mode)..."
  "$CLI" memstat "$CORPUS" ntriples compressed > "$MSC"
  COMP_HEAP_BPT=$(ms_field "$MSC" heap_b_per_triple)
  COMP_STORE_BPT=$(ms_field "$MSC" store_b_per_triple)
  COMP_DICT_BPT=$(ms_field "$MSC" dict_b_per_triple)

  # ---- framing 2: external-memory / on-disk ---------------------------------------------
  DISK_RAW_BPT=na; DISK_COMP_BPT=na; EXT_HWM_BPT=na
  if [ "$SKIP_EXTERNAL" != "1" ]; then
    RAWDIR="$SCRATCH/idx-raw-$SCALE"; COMPDIR="$SCRATCH/idx-comp-$SCALE"; EXTDIR="$SCRATCH/idx-ext-$SCALE"
    log "scale=$SCALE: save (raw on-disk indexes)..."
    "$CLI" save "$CORPUS" ntriples "$RAWDIR" 2>>"$SCRATCH/save.log"
    DISK_RAW_BPT=$(python3 -c "import sys;print(round(int(sys.argv[1])/int(sys.argv[2]),2))" \
      "$(du -sb "$RAWDIR" | cut -f1)" "$TRIPLES")
    log "scale=$SCALE: save compressed (block-compressed on-disk indexes)..."
    "$CLI" save "$CORPUS" ntriples "$COMPDIR" compressed 2>>"$SCRATCH/save.log"
    DISK_COMP_BPT=$(python3 -c "import sys;print(round(int(sys.argv[1])/int(sys.argv[2]),2))" \
      "$(du -sb "$COMPDIR" | cut -f1)" "$TRIPLES")
    rm -rf "$RAWDIR" "$COMPDIR"
    log "scale=$SCALE: external-memory build (bounded-RAM peak RSS)..."
    TB="$SCRATCH/time-build-$SCALE.txt"
    /usr/bin/time -v -o "$TB" "$CLI" build "$CORPUS" ntriples "$EXTDIR" 8 2>>"$SCRATCH/save.log"
    EXT_HWM_BPT=$(python3 -c "import sys;print(round(int(sys.argv[1])/int(sys.argv[2]),2))" \
      "$(time_hwm_bytes "$TB")" "$TRIPLES")
    rm -rf "$EXTDIR"
  fi

  LINE="MEMBPT scale=$SCALE sf=$SF triples=$TRIPLES terms=$TERMS corpus_bytes=$CORPUS_BYTES load_s=$LOAD_S heap_bpt=$HEAP_BPT store_bpt=$STORE_BPT dict_bpt=$DICT_BPT caches_bpt=$CACHES_BPT dict_bpterm=$DICT_BPTERM rss_bpt=$RSS_BPT hwm_bpt=$HWM_BPT gnu_hwm_bytes=$GNU_HWM comp_heap_bpt=$COMP_HEAP_BPT comp_store_bpt=$COMP_STORE_BPT comp_dict_bpt=$COMP_DICT_BPT disk_raw_bpt=$DISK_RAW_BPT disk_comp_bpt=$DISK_COMP_BPT extbuild_hwm_bpt=$EXT_HWM_BPT"
  echo "$LINE"
  RESULTS+=("$LINE")
  [ "$KEEP_CORPUS" = "1" ] || rm -f "$CORPUS"
done

# One compact single-line JSON envelope (console-safe; also written to $OUT when given).
ENV_JSON=$(python3 - "$REPO" "${RESULTS[@]}" <<'PY'
import json, platform, subprocess, sys, datetime
repo, lines = sys.argv[1], sys.argv[2:]
rows = []
for l in lines:
    d = {}
    for kv in l.split()[1:]:
        k, v = kv.split("=", 1)
        try:
            d[k] = int(v)
        except ValueError:
            try:
                d[k] = float(v)
            except ValueError:
                d[k] = v
    rows.append(d)
sha = subprocess.run(["git", "-C", repo, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
print(json.dumps({
    "instrument": "bytes-per-triple", "version": 1, "sha": sha,
    "date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "machine": platform.machine(), "corpus": "watdiv-seed1",
    "rows": rows,
}, separators=(",", ":")))
PY
)
echo "MEMBPT_JSON $ENV_JSON"
[ -n "$OUT" ] && printf '%s\n' "$ENV_JSON" > "$OUT"
log "done."
