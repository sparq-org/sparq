#!/usr/bin/env bash
# Rung-5 on-box plan: T2 partial-scale (1 B-triple real-Wikidata ingest) + x86 homogeneous-core
# scaling sweep, sized for an on-demand r7i.2xlarge (8 vCPU, 64 GB, 1 NUMA node).
# Adapted from hwrun/remote.sh (written for the 192-core box). Differences:
#   * threads 1,2,4,8; no NUMA A/B (single node)
#   * scaling sweep runs WHILE the dump prefix downloads (saves ~10 min wall)
#   * iostat -xm captured during the T2 build for disk-IO notes
# Everything it produces goes to ~/results, which the orchestrator scp's back
# before terminating the instance. A dead-man `shutdown -h +150` is set in user-data.
#
#   bash rung5-remote.sh <slice_triples> <threads_csv> <sha>
set -u
SLICE_N="${1:-1000000000}"
THREADS="${2:-1,2,4,8}"
# [OPUS-4.8 sq-3l43] Post-`dict-consolidation` build by default (see rung5-launch.sh).
SHA="${3:-origin/main}"

R="$HOME/results"; mkdir -p "$R"
exec > >(tee -a "$R/onbox.log") 2>&1
echo "== rung5 remote start $(date -u +%FT%TZ) slice=$SLICE_N threads=$THREADS sha=$SHA =="
nproc; free -g | head -2; df -h / | tail -1

# ---------- stage 0: topology evidence ----------
lscpu > "$R/lscpu.txt"
lscpu | grep -i 'numa\|socket\|model name' || true

# ---------- stage 1: deps + ranged Wikidata prefix download (parallel, background) ----------
sudo apt-get update -qq
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
  build-essential pkg-config git zstd pigz curl python3-pip time sysstat >/dev/null
pip3 install -q --break-system-packages rapidgzip 2>/dev/null || true
export PATH="$HOME/.local/bin:$PATH"

URL="https://dumps.wikimedia.org/wikidatawiki/entities/latest-truthy.nt.gz"
# ~115 B/triple plaintext at ~17-18x gz ratio -> ~6.6 GB gz per 1 B triples; fetch 10 GiB
# (~50% headroom). ATTEMPT-1 LESSON (i-06b07ada7108200a1): 14 parallel range requests got
# rate-limited and curl without -f silently wrote the HTTP error bodies as "parts" -> broken
# gz, empty slice. Now: max 2 concurrent connections (wikimedia's courtesy limit), -f +
# retries, exact-size check per part, and only a CONTIGUOUS verified prefix is concatenated.
GB_NEEDED=$(( SLICE_N / 1000000000 * 10 ))
PARTSZ=1073741824
echo "== downloading first ${GB_NEEDED} GiB of truthy dump (2-conn ranged, verified, background) =="
mkdir -p "$HOME/dl"
( cd "$HOME/dl"
  t0=$SECONDS
  fetch_part() {  # $1 = part index; succeeds only when the part is exactly PARTSZ bytes
    local i=$1 out code sz try
    out="part$(printf %03d "$i")"
    for try in 1 2 3 4 5; do
      code=$(curl -sS -f --connect-timeout 20 -m 1800 --retry 2 --retry-delay 5 \
        -r $((i*PARTSZ))-$(( (i+1)*PARTSZ - 1 )) -o "$out" -w '%{http_code}' "$URL" 2>&1) || true
      sz=$(stat -c %s "$out" 2>/dev/null || echo 0)
      echo "part $i try $try: http=${code##*$'\n'} size=$sz"
      [ "$sz" = "$PARTSZ" ] && return 0
      sleep $(( try * 10 ))
    done
    return 1
  }
  i=0
  while [ "$i" -lt "$GB_NEEDED" ]; do  # 2 connections at a time
    fetch_part "$i" & P1=$!
    P2=""
    [ $((i+1)) -lt "$GB_NEEDED" ] && { fetch_part $((i+1)) & P2=$!; }
    wait "$P1"; RC1=$?
    RC2=0; [ -n "$P2" ] && { wait "$P2"; RC2=$?; }
    if [ "$RC1" != 0 ] || [ "$RC2" != 0 ]; then echo "part fetch failed at pair starting $i"; break; fi
    i=$((i+2))
  done
  # concatenate only the contiguous verified prefix
  N=0
  while [ -f "part$(printf %03d "$N")" ] && [ "$(stat -c %s "part$(printf %03d "$N")")" = "$PARTSZ" ]; do N=$((N+1)); done
  if [ "$N" -gt 0 ]; then
    for j in $(seq 0 $((N-1))); do P="part$(printf %03d "$j")"; cat "$P"; rm -f "$P"; done > truthy-head.gz
    rm -f part*
    echo "download: $((SECONDS-t0))s for $(du -m truthy-head.gz | cut -f1) MB (${N}/${GB_NEEDED} contiguous verified GiB parts)" | tee "$HOME/dl/DL_DONE"
  else
    echo "download FAILED: no verified part000 after retries"
  fi
) > "$R/download.log" 2>&1 &
DL_PID=$!

# ---------- stage 2: toolchain + release build (overlaps the download) ----------
command -v cargo >/dev/null 2>&1 || \
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null 2>&1
. "$HOME/.cargo/env"
git clone -q https://github.com/sparq-org/sparq.git "$HOME/sparq" && cd "$HOME/sparq" && git checkout -q "$SHA"
# [OPUS-4.8 sq-3l43] Record the resolved commit so the dict-bucket result is attributable
# to a specific engine revision (the original ~200 s figure was at ef86e66, PRE-consolidation).
git rev-parse HEAD | tee "$R/engine-sha.txt"
t0=$SECONDS
cargo build --release -q -p sparq-cli -p sparq-bench
echo "cargo build: $((SECONDS-t0))s"
CLI="$HOME/sparq/target/release/sparq-cli"
BENCH="$HOME/sparq/target/release/sparq-bench"

# ---------- stage 3: synthetic fixture (~16 M triples; same as the M1 fallback sweep) ----------
t0=$SECONDS
"$BENCH" dump 2000000 "$HOME/data.nt" 2>&1 | tail -1
echo "dump: $((SECONDS-t0))s ($(wc -l < "$HOME/data.nt") triples)"

# ---------- stage 4: x86 scaling sweep (homogeneous cores) — runs DURING the download ----------
QD="$HOME/sparq/bench/qlever-synthetic/queries"
echo "== x86 scaling sweep (threads=$THREADS, iters=2) =="
"$CLI" scaling "$HOME/data.nt" ntriples "$QD" "$THREADS" 2 > "$R/scaling-x86.tsv" 2> "$R/scaling-x86.err" || true
tail -8 "$R/scaling-x86.tsv" || true

# ---------- stage 5: slice the dump prefix to exactly SLICE_N triples as .zst ----------
echo "== waiting for download =="
wait "$DL_PID" || true
cat "$R/download.log" || true
if [ -f "$HOME/dl/truthy-head.gz" ]; then
  # Probe decompressors on the TRUNCATED gz (rapidgzip is ~5x faster but may refuse a
  # truncated stream; pigz streams until the truncation point). Pick the first that
  # actually yields N-Triples lines.
  DEC=""
  for cand in "rapidgzip -d -c -P $(nproc)" "pigz -dc"; do
    set -- $cand; command -v "$1" >/dev/null 2>&1 || continue
    NLINES=$($cand "$HOME/dl/truthy-head.gz" 2>/dev/null | head -c 1048576 | grep -c '^<' || true)
    echo "probe [$cand]: $NLINES triple-lines in first MiB"
    [ "${NLINES:-0}" -gt 1000 ] && { DEC="$cand"; break; }
  done
  if [ -n "$DEC" ]; then
    t0=$SECONDS
    # The prefix gz is truncated -> the decompressor errors at EOF, but head exits first.
    ( $DEC "$HOME/dl/truthy-head.gz" 2>/dev/null | head -n "$SLICE_N" \
        | tee >(wc -l > "$R/slice-lines.txt") \
        | zstd -1 -T0 -q -f -o "$HOME/slice.nt.zst" ) || true
    sleep 2  # let the >(wc -l) process substitution flush
    echo "slice prep ($DEC -> head $SLICE_N -> zstd -1): $((SECONDS-t0))s, $(du -m "$HOME/slice.nt.zst" | cut -f1) MB, $(cat "$R/slice-lines.txt" 2>/dev/null) lines"
    rm -rf "$HOME/dl"  # reclaim disk for the index
  else
    echo "GZ PREFIX UNDECODABLE — skipping T2" | tee "$R/T2-SKIPPED"
  fi
else
  echo "DOWNLOAD FAILED — skipping T2" | tee "$R/T2-SKIPPED"
fi
df -h / | tail -1

# ---------- stage 6: T2 — timed 1 B-triple ingestion (sharded dict, out-of-core) ----------
SLICE_LINES=$(cat "$R/slice-lines.txt" 2>/dev/null || echo 0)
if [ -f "$HOME/slice.nt.zst" ] && [ "${SLICE_LINES:-0}" -gt 1000000 ]; then
  echo "== T2 build @${SLICE_LINES} triples (SPARQ_SHARDED_DICT=1, 64M-triple runs, iostat sampling) =="
  mkdir -p "$HOME/idx"
  iostat -xm 30 > "$R/t2-iostat.log" 2>&1 &
  IOSTAT_PID=$!
  SPARQ_SHARDED_DICT=1 SPARQ_BUILD_TIMING=1 /usr/bin/time -v \
    "$CLI" build "$HOME/slice.nt.zst" ntriples "$HOME/idx" 64 \
    > "$R/t2-build.log" 2>&1
  BUILD_RC=$?
  kill "$IOSTAT_PID" 2>/dev/null
  tail -30 "$R/t2-build.log"
  # [OPUS-4.8 sq-3l43] The bead's deliverable: isolate the dict-consolidation bucket from the
  # phase report. On post-consolidation main these lines read `dict consolidate (into_merged)`
  # + `dict-consolidate(serial)` + `intern(parallel-occupancy)`/`triple-remap(pipelined-
  # occupancy)` — NOT the old `merge_remap(serial)`/`triple-remap(serial)` additive ~200 s/1 B.
  # Compare the `dict-consolidate(serial)` term against the projected single-digit seconds.
  grep -E '\[build-timing\]' "$R/t2-build.log" | tee "$R/t2-dict-bucket.txt" || true
  df -h / | tail -1; du -sh "$HOME/idx" | tee -a "$R/t2-build.log"
  if [ "$BUILD_RC" = "0" ]; then
    echo "== T2 sanity COUNT queries (mmap) =="
    for q in \
      'SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' \
      'SELECT (COUNT(*) AS ?c) WHERE { ?s <http://www.wikidata.org/prop/direct/P31> ?o }' \
      'SELECT (COUNT(*) AS ?c) WHERE { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?o }'; do
      echo "Q: $q"
      "$CLI" query-mmap "$HOME/idx" "$q" 2>&1 | tail -2
    done > "$R/t2-counts.log" 2>&1 || true
    cat "$R/t2-counts.log"
  fi
fi

echo "== remote done $(date -u +%FT%TZ) =="
