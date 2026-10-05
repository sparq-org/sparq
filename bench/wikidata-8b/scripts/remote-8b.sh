#!/usr/bin/env bash
# On-box driver for the full-truthy (~8-9.4B) build on r8g.large/16GB.
# Adapted from stage-1 remote.sh + remote2.sh (/tmp/sparq-wdbench archive).
# Runs unattended under nohup; everything logged under ~/r.
#
# Usage: remote-8b.sh <sparq_sha> <chunk_millions> <deadman_min> <build_timeout_s> \
#                     <swap_abort_mib> <disk_abort_free_gb> <parse_checkpoint_s> \
#                     <dump_url> <dl_workers> "<dict_spill_env>" "<cargo_features>"
# DRY_RUN=1 prints the plan without executing (set -n style walk-through).
set -uo pipefail

SHA_PIN="${1:?sparq sha}"; CHUNK_MILLIONS="${2:-32}"; DEADMAN_MIN="${3:-2400}"
BUILD_TIMEOUT_S="${4:-86400}"; SWAP_ABORT_MIB="${5:-24576}"; DISK_ABORT_FREE_GB="${6:-50}"
PARSE_CHECKPOINT_S="${7:-18000}"
DUMP_URL="${8:-https://dumps.wikimedia.org/wikidatawiki/entities/latest-truthy.nt.gz}"
DL_WORKERS="${9:-4}"
DICT_SPILL_ENV="${10:-SPARQ_DICT_SPILL=1 SPARQ_DICT_SPILL_BUDGET_MB=8192}"  # merged env gate (SpillConfig::from_lookup)
CARGO_FEATURES="${11:-}"  # dict-spill is a DEFAULT sparq-cli feature post-merge; no extra --features needed

DRY="${DRY_RUN:-0}"
X() { if [ "$DRY" = 1 ]; then echo "DRY+ $*"; else eval "$*"; fi; }

R="$HOME/r"; mkdir -p "$R"
[ "$DRY" = 1 ] || exec > >(tee -a "$R/driver.log") 2>&1
echo "=== wd8b driver start $(date -u +%FT%TZ) sha=$SHA_PIN chunk=${CHUNK_MILLIONS}M dry=$DRY ==="
step() { echo; echo "== $1 == $(date -u +%FT%TZ)"; }

# Dead-man: power off (-> terminate, shutdown-behavior=terminate) no matter what.
step "re-arm dead-man +${DEADMAN_MIN}m"
X "sudo shutdown -P +${DEADMAN_MIN} || true"

step "machine spec capture"
X "{ lscpu; free -m; df -h /; uname -a; } > '$R/machine.txt' 2>&1"

step "apt deps"
X "sudo apt-get update -qq"
X "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq build-essential pkg-config git pigz zstd time python3-pip sysstat >/dev/null"
X "pip3 install --break-system-packages -q rapidgzip 2>/dev/null || true"
if [ "$DRY" = 1 ] || command -v rapidgzip >/dev/null; then DEC="rapidgzip -d -c -P 2"; else DEC="pigz -dc"; fi
echo "decompressor: $DEC"

step "verify dump headers + record expected size"
X "curl -sI '$DUMP_URL' | tee '$R/dump-headers.txt'"
if [ "$DRY" = 1 ]; then EXPECTED=70654648844; else
  EXPECTED=$(grep -i '^Content-Length:' "$R/dump-headers.txt" | tr -dc 0-9)
fi
echo "expected dump bytes: $EXPECTED"
GIBS=$(( (EXPECTED + 1073741823) / 1073741824 ))

step "download full dump ($GIBS GiB, $DL_WORKERS ranged workers) — background"
X "mkdir -p '$HOME/dl'"
download() {
  cd "$HOME/dl"; local t0=$SECONDS
  for w in $(seq 0 $((DL_WORKERS-1))); do
    ( i=$w
      while [ "$i" -lt "$GIBS" ]; do
        for try in 1 2 3 4 5; do
          curl -sS --retry 3 --retry-delay 5 \
            -r $((i*1073741824))-$(( (i+1)*1073741824 - 1 )) \
            -o "part$(printf %05d "$i")" "$DUMP_URL" && break
          sleep 20
        done
        i=$((i+DL_WORKERS))
      done ) &
  done
  wait
  cat part* > truthy.nt.gz && rm -f part*
  local SZ; SZ=$(stat -c%s truthy.nt.gz)
  echo "download done in $((SECONDS-t0)) s, size: $SZ (expected $EXPECTED)" | tee "$HOME/dl/download.time"
  [ "$SZ" = "$EXPECTED" ] || { echo "DOWNLOAD SIZE MISMATCH" | tee -a "$HOME/dl/download.time"; return 1; }
}
if [ "$DRY" = 1 ]; then echo "DRY+ download() with $DL_WORKERS workers, 1GiB ranges, size-verified"; DLPID=""; else
  download & DLPID=$!
fi

step "rust + clone + build sparq-cli @$SHA_PIN (overlapped with download)"
X "command -v cargo >/dev/null || curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null 2>&1"
X ". '$HOME/.cargo/env'"
X "git clone -q https://github.com/sparq-org/sparq.git '$HOME/sparq'"
X "cd '$HOME/sparq' && git checkout -q '$SHA_PIN' && git rev-parse HEAD > '$R/sha.txt'"
# dict-spill is a DEFAULT sparq-cli feature post-merge, so the plain build compiles the
# spill path; $CARGO_FEATURES is empty unless an A/B run overrides it (e.g. to add a feature).
X "cd '$HOME/sparq' && cargo build --release -q -p sparq-cli $CARGO_FEATURES 2> '$R/cargo-build.log'"
CLI="$HOME/sparq/target/release/sparq-cli"
X "'$CLI' --help > '$R/cli-help.txt' 2>&1 || true"

step "swapfile 64G (safety net; with dict-spill it should stay ~unused — sampler proves it)"
X "sudo fallocate -l 64G /swapfile && sudo chmod 600 /swapfile && sudo mkswap /swapfile -f >/dev/null && sudo swapon /swapfile"
X "free -m"

step "wait for download"
if [ -n "${DLPID:-}" ]; then wait "$DLPID" || { echo "DOWNLOAD FAILED"; exit 1; }; fi
X "cat '$HOME/dl/download.time'"

step "recompress gz -> zst (one streaming pass; line count == triple count denominator)"
X "mkdir -p '$HOME/data'"
# rapidgzip -P2 decompress | tee line-count | zstd -3 -T2. Stage-1: ~1179 s/B-lines incl. a pigz tee we drop.
X "t0=\$SECONDS; set -o pipefail; $DEC '$HOME/dl/truthy.nt.gz' \
     | tee >(wc -l > '$HOME/data/truthy.count') \
     | zstd -3 -T2 -q -f -o '$HOME/data/truthy.nt.zst' \
   && echo \"recompress: \$((SECONDS-t0)) s, lines=\$(cat '$HOME/data/truthy.count'), zst=\$(stat -c%s '$HOME/data/truthy.nt.zst')\" | tee '$R/recompress.txt'"
step "delete .gz (frees ~71 GB the disk budget needs)"
X "[ -s '$HOME/data/truthy.nt.zst' ] && [ -s '$HOME/data/truthy.count' ] && rm -f '$HOME/dl/truthy.nt.gz'"
X "df -h / | tail -1"

step "start mem+disk sampler (30 s) + watchdog"
if [ "$DRY" = 1 ]; then
  echo "DRY+ bash mem-sampler.sh 30 > $R/mem-sampler.log &  (format: <UTC> memused=MiB swapused=MiB diskfree=GiB)"
  echo "DRY+ watchdog: kill build if swapused>$SWAP_ABORT_MIB MiB x10 samples, or diskfree<$DISK_ABORT_FREE_GB GB, or parse marker absent at T+${PARSE_CHECKPOINT_S}s"
else
  bash "$HOME/mem-sampler.sh" 30 >> "$R/mem-sampler.log" 2>&1 & SAMPLER=$!
fi

step "BUILD full truthy (chunk=${CHUNK_MILLIONS}M, timeout ${BUILD_TIMEOUT_S}s)"
X "rm -rf '$HOME/idx'"
X "sync; echo 3 | sudo tee /proc/sys/vm/drop_caches >/dev/null"
BUILD_CMD="SPARQ_SHARDED_DICT=1 SPARQ_BUILD_TIMING=1 $DICT_SPILL_ENV timeout $BUILD_TIMEOUT_S /usr/bin/time -v '$CLI' build '$HOME/data/truthy.nt.zst' ntriples '$HOME/idx' $CHUNK_MILLIONS > '$R/build-8B.log' 2>&1"
if [ "$DRY" = 1 ]; then
  echo "DRY+ $BUILD_CMD"
  echo "DRY+ (watchdog runs alongside; see above)"
  RC=0
else
  date -u +%s > "$R/build-start-epoch"
  eval "$BUILD_CMD" & BPID=$!
  # --- watchdog: swap / disk / parse-checkpoint aborts (RUNBOOK §9) ---
  ( SWAPHITS=0
    while kill -0 "$BPID" 2>/dev/null; do
      sleep 30
      SW=$(free -m | awk '/Swap:/{print $3}')
      FREE_GB=$(df -BG --output=avail / | tail -1 | tr -dc 0-9)
      if [ "${SW:-0}" -gt "$SWAP_ABORT_MIB" ]; then SWAPHITS=$((SWAPHITS+1)); else SWAPHITS=0; fi
      if [ "$SWAPHITS" -ge 10 ]; then
        echo "ABORT: swap ${SW}MiB > ${SWAP_ABORT_MIB}MiB for 10 samples — dict-spill not bounding memory" | tee -a "$R/abort.txt"
        kill "$BPID"; break
      fi
      if [ "${FREE_GB:-9999}" -lt "$DISK_ABORT_FREE_GB" ]; then
        echo "ABORT: disk free ${FREE_GB}GB < ${DISK_ABORT_FREE_GB}GB" | tee -a "$R/abort.txt"
        kill "$BPID"; break
      fi
      EL=$(( $(date -u +%s) - $(cat "$R/build-start-epoch") ))
      if [ "$EL" -gt "$PARSE_CHECKPOINT_S" ] && ! grep -q 'parse+intern+spill done' "$R/build-8B.log"; then
        echo "ABORT: parse phase not done at T+${EL}s (> ${PARSE_CHECKPOINT_S}s) — throughput floor breached" | tee -a "$R/abort.txt"
        kill "$BPID"; break
      fi
    done ) & WATCHDOG=$!
  wait "$BPID"; RC=$?
  kill "$WATCHDOG" 2>/dev/null
fi
echo "build rc=$RC" | tee -a "$R/builds.txt"
X "grep -E 'Elapsed|Maximum resident|built on-disk' '$R/build-8B.log' | tee -a '$R/builds.txt'"
X "du -sb '$HOME/idx' 2>/dev/null | tee -a '$R/builds.txt'"

if [ "$RC" -eq 0 ]; then
  step "validation queries (mmap open + COUNTs, stage-1 probes)"
  for q in \
    'SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }' \
    'SELECT (COUNT(*) AS ?c) WHERE { ?s <http://www.wikidata.org/prop/direct/P31> ?o }' \
    'SELECT (COUNT(*) AS ?c) WHERE { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?o }'; do
    if [ "$DRY" = 1 ]; then echo "DRY+ /usr/bin/time -v $CLI query-mmap ~/idx \"$q\""; else
      echo "Q[8B]: $q" >> "$R/queries-8B.log"
      /usr/bin/time -v "$CLI" query-mmap "$HOME/idx" "$q" >> "$R/queries-8B.log" 2>&1
    fi
  done
  X "tail -60 '$R/queries-8B.log' | tee -a '$R/builds.txt'"
else
  step "BUILD FAILED/ABORTED rc=$RC — keeping logs; index dir left for post-mortem du"
fi

[ "$DRY" = 1 ] || kill "${SAMPLER:-0}" 2>/dev/null
step "package results"
X "df -h / | tail -1 > '$R/disk-final.txt'"
X "tar -C '$HOME' -czf '$HOME/results.tgz' r"
echo "=== wd8b driver done $(date -u +%FT%TZ) rc=$RC ==="
X "touch '$HOME/DONE'"
