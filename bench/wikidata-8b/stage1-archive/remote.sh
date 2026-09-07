#!/usr/bin/env bash
# Wikidata low-resource ingest curve — stage 1, r8g.large (2 vCPU / 16 GB).
# Runs unattended under nohup; everything logged under ~/r.
set -uo pipefail
R="$HOME/r"; mkdir -p "$R"
exec > >(tee -a "$R/driver.log") 2>&1
echo "=== driver start $(date -u +%FT%TZ) ==="

URL="https://dumps.wikimedia.org/wikidatawiki/entities/latest-truthy.nt.gz"
SHA_PIN="4aac23af80c8d239f0428051a85aa5990c50b569"   # public main at launch time
CHUNK_MILLIONS=32

step() { echo; echo "== $1 == $(date -u +%FT%TZ)"; }

# Dead-man: power off (-> terminate, shutdown-behavior=terminate) after 11 h no matter what.
sudo shutdown -P +660 || true

step "machine spec capture"
{ lscpu; free -m; df -h /; uname -a; } > "$R/machine.txt" 2>&1

step "apt deps"
sudo apt-get update -qq
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq build-essential pkg-config git pigz zstd time python3-pip sysstat >/dev/null
pip3 install --break-system-packages -q rapidgzip 2>/dev/null || true
if command -v rapidgzip >/dev/null; then DEC="rapidgzip -d -c -P 2"; else DEC="pigz -dc"; fi
echo "decompressor: $DEC"

step "download 16 GiB head of latest-truthy.nt.gz (2 ranged workers) — background"
mkdir -p "$HOME/dl"
( cd "$HOME/dl"
  t0=$SECONDS
  for w in 0 1; do
    ( i=$w
      while [ "$i" -lt 16 ]; do
        for try in 1 2 3 4 5; do
          curl -sS --retry 3 --retry-delay 5 \
            -r $((i*1073741824))-$(( (i+1)*1073741824 - 1 )) \
            -o "part$(printf %03d "$i")" "$URL" && break
          sleep 20
        done
        i=$((i+2))
      done ) &
  done
  wait
  cat part* > truthy-head.gz && rm -f part*
  echo "download done in $((SECONDS-t0)) s, size: $(stat -c%s truthy-head.gz)" > "$HOME/dl/download.time"
) &
DLPID=$!

step "rust + clone + build sparq-cli (overlapped with download)"
command -v cargo >/dev/null || curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null 2>&1
. "$HOME/.cargo/env"
git clone -q https://github.com/sparq-org/sparq.git "$HOME/sparq"
cd "$HOME/sparq" && git checkout -q "$SHA_PIN" && git rev-parse HEAD > "$R/sha.txt"
t0=$SECONDS
cargo build --release -q -p sparq-cli 2> "$R/cargo-build.log"
echo "cargo build: $((SECONDS-t0)) s"
CLI="$HOME/sparq/target/release/sparq-cli"
"$CLI" --help > "$R/cli-help.txt" 2>&1 || true

step "swapfile 64G (for honest over-RAM measurement; per-run swap usage sampled)"
sudo fallocate -l 64G /swapfile && sudo chmod 600 /swapfile && sudo mkswap /swapfile -f >/dev/null && sudo swapon /swapfile
free -m

step "wait for download"
wait "$DLPID" || true
cat "$HOME/dl/download.time" || { echo "DOWNLOAD FAILED"; exit 1; }

step "slice creation (one streaming pass per scale; gz=pigz -6, zst=zstd -3)"
mkdir -p "$HOME/data"
for N in 100000000 300000000 1000000000; do
  L="$((N/1000000))M"
  step "slice $L"
  t0=$SECONDS
  $DEC "$HOME/dl/truthy-head.gz" 2>/dev/null \
    | head -n "$N" \
    | tee >(pigz -p 2 -6 -c > "$HOME/data/$L.nt.gz") >(wc -l > "$HOME/data/$L.count") \
    | zstd -3 -T2 -q -f -o "$HOME/data/$L.nt.zst"
  echo "slice $L: $((SECONDS-t0)) s, lines=$(cat "$HOME/data/$L.count"), gz=$(stat -c%s "$HOME/data/$L.nt.gz"), zst=$(stat -c%s "$HOME/data/$L.nt.zst")" | tee -a "$R/slices.txt"
  df -h / | tail -1
done

# memory sampler: 15 s resolution, tracks swap + mem for the whole bench window
( while true; do echo "$(date -u +%FT%TZ) $(free -m | awk '/Mem:/{printf "memused=%s ", $3} /Swap:/{printf "swapused=%s\n", $3}')"; sleep 15; done ) > "$R/mem-sampler.log" 2>&1 &
SAMPLER=$!

run_build() { # label file timeout_s
  local LBL="$1" F="$2" TMO="$3"
  step "BUILD $LBL"
  rm -rf "$HOME/idx"
  sync; echo 3 | sudo tee /proc/sys/vm/drop_caches >/dev/null
  local sw0; sw0=$(free -m | awk '/Swap:/{print $3}')
  SPARQ_SHARDED_DICT=1 SPARQ_BUILD_TIMING=1 timeout "$TMO" /usr/bin/time -v \
    "$CLI" build "$F" ntriples "$HOME/idx" "$CHUNK_MILLIONS" > "$R/build-$LBL.log" 2>&1
  local RC=$?
  local sw1; sw1=$(free -m | awk '/Swap:/{print $3}')
  echo "build $LBL rc=$RC swap_delta_MiB=$((sw1-sw0))" | tee -a "$R/builds.txt"
  grep -E "Elapsed|Maximum resident" "$R/build-$LBL.log" | tee -a "$R/builds.txt"
  du -sb "$HOME/idx" 2>/dev/null | tee -a "$R/builds.txt"
  if [ $RC -eq 0 ]; then
    for q in \
      'SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' \
      'SELECT (COUNT(*) AS ?c) WHERE { ?s <http://www.wikidata.org/prop/direct/P31> ?o }' \
      'SELECT (COUNT(*) AS ?c) WHERE { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?o }'; do
      echo "Q[$LBL]: $q" >> "$R/queries-$LBL.log"
      /usr/bin/time -v "$CLI" query-mmap "$HOME/idx" "$q" >> "$R/queries-$LBL.log" 2>&1
    done
    grep -E "^Q\[|count|Elapsed|Maximum resident|^\?c|^[0-9]" "$R/queries-$LBL.log" | head -40 | tee -a "$R/builds.txt"
  fi
  rm -rf "$HOME/idx"
  return $RC
}

run_build 100M-gz  "$HOME/data/100M.nt.gz"  3600;  B_100GZ=$?
run_build 100M-zst "$HOME/data/100M.nt.zst" 3600;  B_100ZS=$?

step "IN-MEMORY load attempt @100M (cgroup MemoryMax=15G, no swap — OOM is a finding)"
sudo systemd-run --scope --wait --collect -p MemoryMax=15G -p MemorySwapMax=0 \
  /usr/bin/time -v "$CLI" query "$HOME/data/100M.nt.zst" ntriples \
  'SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' > "$R/inmem-100M.log" 2>&1
echo "inmem-100M rc=$?" | tee -a "$R/builds.txt"
tail -25 "$R/inmem-100M.log" | tee -a "$R/builds.txt"

run_build 300M-gz  "$HOME/data/300M.nt.gz"  10800; B_300GZ=$?
run_build 300M-zst "$HOME/data/300M.nt.zst" 10800; B_300ZS=$?

step "choose 1B format"
WGZ=$(grep -A2 "build 300M-gz"  "$R/builds.txt" | grep -oP 'Elapsed.*?(\d+):(\d+\.?\d*)' | tail -1 || true)
# parse elapsed seconds for both 300M runs from time -v logs
esec() { grep "Elapsed (wall clock)" "$R/build-$1.log" | awk '{print $NF}' | awk -F: '{ if (NF==3) print $1*3600+$2*60+$3; else print $1*60+$2 }'; }
EG=$(esec 300M-gz); EZ=$(esec 300M-zst)
echo "300M elapsed: gz=${EG:-fail}s zst=${EZ:-fail}s" | tee -a "$R/builds.txt"
ONEB="zst"
if [ -n "${EG:-}" ] && [ -n "${EZ:-}" ]; then
  awk -v g="$EG" -v z="$EZ" 'BEGIN{exit !(g<z)}' && ONEB="gz"
fi
echo "1B format: $ONEB" | tee -a "$R/builds.txt"

if [ "${B_300GZ:-1}" -eq 0 ] || [ "${B_300ZS:-1}" -eq 0 ]; then
  run_build 1B-$ONEB "$HOME/data/1B.nt.$ONEB" 18000
else
  echo "SKIP 1B: both 300M builds failed" | tee -a "$R/builds.txt"
fi

kill $SAMPLER 2>/dev/null
step "package results"
df -h / | tail -1 > "$R/disk-final.txt"
tar -C "$HOME" -czf "$HOME/results.tgz" r
echo "=== driver done $(date -u +%FT%TZ) ==="
touch "$HOME/DONE"
