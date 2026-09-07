#!/usr/bin/env bash
# On-box plan for the T2 (Wikidata ingestion) + T12/T1 (NUMA scaling) hardware validation.
# Runs as ubuntu on a freshly launched EC2 box. Everything it produces goes to ~/results,
# which the orchestrator scp's back before terminating the instance.
#
#   bash remote.sh <slice_triples> <threads_csv> <numa_ab: 0|1>
#
# Deliberately NOT `set -e`: each stage is guarded so a single failure still yields
# partial results. A dead-man `shutdown -h +NN` was set in user-data by the launcher.
set -u
SLICE_N="${1:-2000000000}"
THREADS="${2:-1,2,4,8,16,32,64,96,128,192}"
NUMA_AB="${3:-1}"
SHA="${4:-72b35e3}"

R="$HOME/results"; mkdir -p "$R"
exec > >(tee -a "$R/onbox.log") 2>&1
echo "== remote start $(date -u +%FT%TZ) slice=$SLICE_N threads=$THREADS numa_ab=$NUMA_AB =="
nproc; free -g | head -2; df -h / | tail -1

# ---------- stage 0: topology (T12 evidence) ----------
lscpu > "$R/lscpu.txt"
lscpu | grep -i 'numa\|socket\|model name' || true
numactl --hardware > "$R/numactl-hardware.txt" 2>/dev/null || true

# ---------- stage 1: deps + ranged Wikidata prefix download (parallel) ----------
sudo apt-get update -qq
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
  build-essential pkg-config git numactl zstd pigz curl python3-pip time >/dev/null
pip3 install -q --break-system-packages rapidgzip 2>/dev/null || true

URL="https://dumps.wikimedia.org/wikidatawiki/entities/latest-truthy.nt.gz"
# We need ~115 B/triple plaintext at ~18x gz ratio -> ~6.5 GB gz per 1 B triples. Fetch a
# prefix with headroom via parallel 1 GiB range requests (wikimedia throttles per-connection).
GB_NEEDED=$(( SLICE_N / 1000000000 * 8 + 6 ))
echo "== downloading first ${GB_NEEDED} GiB of truthy dump (ranged, parallel) =="
mkdir -p "$HOME/dl"
( cd "$HOME/dl"
  t0=$SECONDS
  for i in $(seq 0 $((GB_NEEDED-1))); do
    curl -sS --retry 3 --retry-delay 2 \
      -r $((i*1073741824))-$(( (i+1)*1073741824 - 1 )) \
      -o "part$(printf %03d "$i")" "$URL" &
  done
  wait
  cat part* > truthy-head.gz && rm -f part*
  echo "download: $((SECONDS-t0))s for $(du -m truthy-head.gz | cut -f1) MB" | tee "$HOME/dl/DL_DONE"
) > "$R/download.log" 2>&1 &
DL_PID=$!

# ---------- stage 2: toolchain + build (overlaps the download) ----------
command -v cargo >/dev/null 2>&1 || \
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null 2>&1
. "$HOME/.cargo/env"
git clone -q https://github.com/sparq-org/sparq.git "$HOME/sparq" && cd "$HOME/sparq" && git checkout -q "$SHA"
t0=$SECONDS
cargo build --release -q -p sparq-cli -p sparq-bench
echo "cargo build: $((SECONDS-t0))s"
CLI="$HOME/sparq/target/release/sparq-cli"
BENCH="$HOME/sparq/target/release/sparq-bench"

# ---------- stage 3: synthetic fixture for T12 (~8 triples/entity -> ~16 M) ----------
t0=$SECONDS
"$BENCH" dump 2000000 "$HOME/data.nt" 2>&1 | tail -1
echo "dump: $((SECONDS-t0))s ($(wc -l < "$HOME/data.nt") triples)"

# ---------- stage 4: slice the dump prefix to exactly SLICE_N triples as .zst ----------
echo "== waiting for download =="
wait "$DL_PID" || true
cat "$R/download.log" || true
if [ -f "$HOME/dl/truthy-head.gz" ]; then
  t0=$SECONDS
  if command -v rapidgzip >/dev/null 2>&1; then
    DEC="rapidgzip -d -c -P $(nproc)"
  else
    DEC="pigz -dc"
  fi
  # The prefix gz is truncated -> the decompressor errors at EOF, but head exits first.
  ( $DEC "$HOME/dl/truthy-head.gz" 2>/dev/null | head -n "$SLICE_N" \
      | zstd -1 -T0 -q -f -o "$HOME/slice.nt.zst" ) || true
  echo "slice prep ($DEC -> head $SLICE_N -> zstd -1): $((SECONDS-t0))s, $(du -m "$HOME/slice.nt.zst" | cut -f1) MB"
  SLICE_LINES=$(zstd -dc "$HOME/slice.nt.zst" 2>/dev/null | head -c 200 | wc -l) # cheap sanity that it decodes
  rm -rf "$HOME/dl"  # reclaim disk for the index
else
  echo "DOWNLOAD FAILED — skipping T2" | tee "$R/T2-SKIPPED"
fi
df -h / | tail -1

# ---------- stage 5: T12 — thread-scaling sweep on a quiet machine ----------
QD="$HOME/sparq/bench/qlever-synthetic/queries"
echo "== T12 scaling sweep (threads=$THREADS) =="
"$CLI" scaling "$HOME/data.nt" ntriples "$QD" "$THREADS" 2 > "$R/scaling-default.tsv" 2> "$R/scaling-default.err" || true
tail -5 "$R/scaling-default.tsv" || true

NODES=$(lscpu | awk -F: '/NUMA node\(s\)/{gsub(/ /,"");print $2}')
if [ "$NUMA_AB" = "1" ] && [ "${NODES:-1}" -ge 2 ]; then
  echo "== T12 NUMA A/B (nodes=$NODES) =="
  HI=$(echo "$THREADS" | tr ',' '\n' | tail -1)
  HALF=$(( HI / 2 ))
  numactl --interleave=all "$CLI" scaling "$HOME/data.nt" ntriples "$QD" "$HALF,$HI" 2 \
    > "$R/scaling-interleave.tsv" 2>&1 || true
  numactl --cpunodebind=0 --membind=0 "$CLI" scaling "$HOME/data.nt" ntriples "$QD" "$HALF" 2 \
    > "$R/scaling-node0.tsv" 2>&1 || true
  # forced-remote diagnostic: cpu on node0, memory on node1 -> direct remote-access penalty
  numactl --cpunodebind=0 --membind=1 "$CLI" scaling "$HOME/data.nt" ntriples "$QD" "$HALF" 2 \
    > "$R/scaling-remote.tsv" 2>&1 || true
else
  echo "NUMA A/B skipped (nodes=${NODES:-?}, numa_ab=$NUMA_AB)" | tee "$R/numa-ab-skipped.txt"
fi

# ---------- stage 6: T2 — timed ingestion of the Wikidata slice ----------
if [ -f "$HOME/slice.nt.zst" ]; then
  echo "== T2 build (sharded dict, build timing on) =="
  mkdir -p "$HOME/idx"
  SPARQ_SHARDED_DICT=1 SPARQ_BUILD_TIMING=1 /usr/bin/time -v \
    "$CLI" build "$HOME/slice.nt.zst" ntriples "$HOME/idx" 64 \
    > "$R/t2-build.log" 2>&1
  BUILD_RC=$?
  tail -25 "$R/t2-build.log"
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
