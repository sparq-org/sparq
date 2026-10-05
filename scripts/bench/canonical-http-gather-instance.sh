#!/usr/bin/env bash
# [FABLE-5] sq-7d3dj.34 — INSTANCE-side canonical HTTP/TTFB competitor gather.
#
# Runs ON the dedicated quiet EC2 box (launched by scripts/bench/canonical-competitor-bench.sh)
# from a cloned sparq checkout, as root. Produces the D9/D10 panel the 2026-07 perf-dominance
# gap table flagged: ALL FIVE engines measured in the SAME HTTP regime — sparq itself in HTTP
# server mode (sparq-server) next to Oxigraph (`oxigraph serve-read-only`), Fuseki (offline
# tdb2.tdbloader -> fuseki-server, the intended bulk path that FIXES the 2026-07-07 FAILED
# rows), Virtuoso and QLever — measuring BOTH full-request latency AND TTFB in BOTH
# connection regimes (keep-alive AND fresh-connect; adapter --profile, 6-col TSVs).
#
# Envelopes: bench/competitor-results/canonical-<suite>-http-<UTC>.json (suite field
# "<suite>-http"), one per suite per gather; the whole panel runs GATHERS times
# back-to-back so the ingest can assert count stability and take best-of.
# Writes /root/GATHER_DONE when every envelope is on disk. NEVER shuts the box down —
# the launcher terminates it (its user-data watchdog is the orphan-proof backstop).
#
# Env knobs (all defaulted): SP2B_TRIPLES=250000 WATDIV_SF=1 ITERS=3 QTO=300 GATHERS=2
#   OXI_VERSION/OXI_SHA256_X86_64 (prebuilt Oxigraph CLI pin, same as gather-ec2-sparql.sh)
#   BENCH_RESULTS_S3_URI/BENCH_EGRESS_REGION (sq-ffaa9): when the launcher passes a
#   run-scoped S3 prefix, each envelope is ALSO uploaded there as soon as it is written —
#   the only channel that survives an AMI whose serial console returns nothing usable.
set -uo pipefail   # NOT -e: one failed engine must never kill the gather (honest-n/a instead)

SP2B_TRIPLES="${SP2B_TRIPLES:-250000}"
WATDIV_SF="${WATDIV_SF:-1}"
ITERS="${ITERS:-3}"
QTO="${QTO:-300}"          # per-query adapter cap, s (covers 2 regimes x ITERS requests)
GATHERS="${GATHERS:-2}"
SUITES="${SUITES:-sp2b watdiv}"   # space-separated suite filter (partial re-runs)
OXI_VERSION="${OXI_VERSION:-v0.5.9}"
OXI_SHA256_X86_64="${OXI_SHA256_X86_64:-4be355715ba3945e8fb8c94a06662a29808683bf2ec355894dba9e82762e7cc7}"

step() { echo "[STEP $(date -u +%Y-%m-%dT%H:%M:%SZ)] $*" | tee -a /root/GATHER_STEP >&2; }

cd "$(dirname "${BASH_SOURCE[0]}")/../.."   # repo root
# [OPUS-5] sq-ffaa9 — durable result egress. bench_egress_push is a successful no-op
# unless the launcher passed BENCH_RESULTS_S3_URI in, so a run without the instance
# profile attached behaves exactly as before (console + SSH pull only).
. scripts/bench/bench-result-egress.sh
SHA=$(git rev-parse --short HEAD)
step "canonical-http gather @ $SHA (sp2b=$SP2B_TRIPLES watdiv_sf=$WATDIV_SF iters=$ITERS qto=$QTO gathers=$GATHERS suites='$SUITES')"
want_suite() { case " $SUITES " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

step "cargo build --release -p sparq-server"
cargo build --release -q -p sparq-server || step "WARN: sparq-server build failed"

# --- prebuilt Oxigraph CLI (x86_64), sha256-pinned ---
step "fetch oxigraph CLI $OXI_VERSION (x86_64)"
OXI_BIN=/usr/local/bin/oxigraph
curl -fsSL -o "$OXI_BIN" "https://github.com/oxigraph/oxigraph/releases/download/${OXI_VERSION}/oxigraph_${OXI_VERSION}_x86_64_linux_gnu" || step "WARN: oxigraph fetch failed"
ACT=$(sha256sum "$OXI_BIN" 2>/dev/null | cut -d' ' -f1)
if [ "$ACT" = "$OXI_SHA256_X86_64" ]; then chmod +x "$OXI_BIN"; step "oxigraph sha256 OK"; else step "FATAL: oxigraph sha256 mismatch ($ACT) — removing"; rm -f "$OXI_BIN"; fi
OXI_V=$("$OXI_BIN" --version 2>/dev/null | awk '{print $2}' || echo "$OXI_VERSION")

# --- pull the docker competitor images ONCE, capture digests (fuseki = jena tarball, no image) ---
step "docker pull competitor images (virtuoso, qlever)"
VIRTUOSO_IMG=docker.io/openlink/virtuoso-opensource-7:latest
QLEVER_IMG=docker.io/adfreiburg/qlever:latest
docker pull -q "$VIRTUOSO_IMG" || step "WARN: virtuoso pull failed"
docker pull -q "$QLEVER_IMG"   || step "WARN: qlever pull failed"
VIRTUOSO_DIG=$(docker inspect --format '{{if .RepoDigests}}{{index .RepoDigests 0}}{{end}}' "$VIRTUOSO_IMG" 2>/dev/null || echo "$VIRTUOSO_IMG")
QLEVER_DIG=$(docker inspect --format '{{if .RepoDigests}}{{index .RepoDigests 0}}{{end}}' "$QLEVER_IMG" 2>/dev/null || echo "$QLEVER_IMG")

# --- generate corpora (deterministic; only for the SELECTED suites) ---
# [FABLE-5] a FAILED generation is a LOUD "FATAL" step (with the gen stderr tail inlined),
# not a silent empty variable — run 1 (2026-07-07) skipped the whole watdiv suite as a
# quiet "NO CORPUS" because the box was missing Boost and nothing shouted.
SP2B_CORPUS=""; WATDIV_CORPUS=""
if want_suite sp2b; then
  step "sp2b gen ($SP2B_TRIPLES triples)"
  SP2B_CORPUS=$(bash bench/sp2b/gen.sh "$SP2B_TRIPLES" 2>/tmp/sp2b_gen.err | tail -n1)
  [ -n "$SP2B_CORPUS" ] && [ -f "$SP2B_CORPUS" ] \
    || step "FATAL: sp2b gen produced no corpus — $(tail -3 /tmp/sp2b_gen.err 2>/dev/null | tr '\n' ' ')"
  step "sp2b corpus=$SP2B_CORPUS"
fi
if want_suite watdiv; then
  step "watdiv gen (SF=$WATDIV_SF)"
  WATDIV_CORPUS=$(bash bench/watdiv/gen.sh "$WATDIV_SF" 2>/tmp/watdiv_gen.err | tail -n1)
  [ -n "$WATDIV_CORPUS" ] && [ -f "$WATDIV_CORPUS" ] \
    || step "FATAL: watdiv gen produced no corpus — $(tail -3 /tmp/watdiv_gen.err 2>/dev/null | tr '\n' ' ')"
  step "watdiv corpus=$WATDIV_CORPUS"
fi

CPU=$(LC_ALL=C grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2- | sed 's/^ *//' || uname -p)
NPROC=$(nproc); KERNEL=$(uname -r)

FUSEKI_JENA_VERSION="${FUSEKI_JENA_VERSION:-6.1.0}"
ENGINES_JSON=$(jq -n \
  --arg sparq_v "$SHA" --arg oxi_v "$OXI_V" --arg jena_v "$FUSEKI_JENA_VERSION" \
  --arg virtuoso_dig "$VIRTUOSO_DIG" --arg qlever_dig "$QLEVER_DIG" \
  '{sparq:{version:$sparq_v,mode:"HTTP server (sparq-server, SPARQL 1.1 protocol at /sparql) via http_sparql_adapter.py --profile"},
    oxigraph:{version:$oxi_v,mode:"HTTP server (prebuilt sha256-pinned oxigraph serve-read-only, /query) via http_sparql_adapter.py --profile"},
    fuseki:{version:("apache-jena-fuseki " + $jena_v + " (official Apache tarball)"),mode:"HTTP server (OFFLINE tdb2.tdbloader bulk load -> fuseki-server --tdb2, the intended bulk path; fixes the 2026-07-07 docker-image load hang) via http_sparql_adapter.py --profile"},
    virtuoso:{version:"openlink/virtuoso-opensource-7",image_digest:$virtuoso_dig,mode:"HTTP server (isql ld_dir bulk load, /sparql) via http_sparql_adapter.py --profile"},
    qlever:{version:"adfreiburg/qlever",image_digest:$qlever_dig,mode:"HTTP server (qlever-index -> qlever-server) via http_sparql_adapter.py --profile"}}')
CONNECTION_JSON=$(jq -n \
  '{modes_measured:["keep-alive","fresh-connect"],primary:"keep-alive",
    note:"EVERY query was measured in BOTH connection regimes (the fairness question research/perf-dominance-gap-2026-07.md D9 left open): cols 3-4 = keep-alive (ONE persistent HTTP/1.1 connection reused across iterations; min-of-N discards the connect-bearing first iteration), cols 5-6 = fresh-connect (a NEW TCP connection per iteration, connect cost included). TTFB = time from request start to status-line+headers received; full = entire body read. All values are per-mode minima over the iterations."}')
TSV_FORMAT='<query>\t<rows|ERROR>\t<ka_best_us>\t<ka_ttfb_us>\t<fresh_best_us>\t<fresh_ttfb_us>'

mkdir -p bench/competitor-results /root/work
export QUIET_BOX=true

run_suite_http() {
  # $1 suite  $2 corpus  $3 fmt(ttl|nt)  $4 qdir  $5 expected-rows  $6 scale-label  $7 gather-idx
  local suite="$1" corpus="$2" fmt="$3" qdir="$4" exp="$5" scale="$6" gidx="$7"
  local W="/root/work/${suite}-http-g${gidx}"; mkdir -p "$W"
  step "[$suite-http g$gidx] corpus=$corpus fmt=$fmt"
  [ -n "$corpus" ] && [ -f "$corpus" ] || { step "[$suite-http g$gidx] NO CORPUS — skipping"; return 0; }
  local t0 t1

  # 1) sparq — HTTP server mode
  local ST_SPARQ=failed SPARQ_LOAD=""
  step "[$suite-http g$gidx] sparq-server"
  if HTTP_PROFILE=1 SPARQ_QUERY_TIMEOUT=$QTO SPARQ_READY_TIMEOUT=600 \
       timeout 1800 bash scripts/sparq-server-same-box.sh "$corpus" "$fmt" "$qdir" "$ITERS" "$W/sparq.tsv"; then ST_SPARQ=ok; fi
  SPARQ_LOAD=$(cat "$W/sparq.tsv.load_s" 2>/dev/null || true)
  [ -f "$W/sparq.tsv" ] || : > "$W/sparq.tsv"

  # 2) oxigraph — HTTP server mode (prebuilt CLI serve-read-only)
  local ST_OXI=failed OXI_LOAD=""
  step "[$suite-http g$gidx] oxigraph serve"
  if HTTP_PROFILE=1 OXIGRAPH_BIN="$OXI_BIN" OXIGRAPH_QUERY_TIMEOUT=$QTO OXIGRAPH_LOAD_TIMEOUT=1800 \
       timeout 3600 bash scripts/oxigraph-serve-same-box.sh "$corpus" "$fmt" "$qdir" "$ITERS" "$W/oxigraph.tsv"; then ST_OXI=ok; fi
  OXI_LOAD=$(cat "$W/oxigraph.tsv.load_s" 2>/dev/null || true)
  [ -f "$W/oxigraph.tsv" ] || : > "$W/oxigraph.tsv"

  # 3) fuseki — jena tarball backend (offline tdb2.tdbloader), the D10 fix
  local ST_FU=failed FU_WALL
  step "[$suite-http g$gidx] fuseki (jena tarball backend)"
  t0=$(date +%s%N)
  if HTTP_PROFILE=1 FUSEKI_BACKEND=jena FUSEKI_QUERY_TIMEOUT=$QTO FUSEKI_LOAD_TIMEOUT=1800 FUSEKI_READY_TIMEOUT=180 \
       timeout 3600 bash scripts/fuseki-same-box.sh "$corpus" "$fmt" "$qdir" "$ITERS" "$W/fuseki.tsv"; then ST_FU=ok; fi
  t1=$(date +%s%N); FU_WALL=$(awk "BEGIN{printf \"%.1f\", ($t1-$t0)/1e9}")
  [ -f "$W/fuseki.tsv" ] || : > "$W/fuseki.tsv"

  # 4) virtuoso
  local ST_VI=failed VI_WALL
  step "[$suite-http g$gidx] virtuoso"
  t0=$(date +%s%N)
  if HTTP_PROFILE=1 VIRTUOSO_QUERY_TIMEOUT=$QTO VIRTUOSO_LOAD_TIMEOUT=1800 VIRTUOSO_READY_TIMEOUT=240 \
       timeout 3600 bash scripts/virtuoso-same-box.sh "$corpus" "$fmt" "$qdir" "$ITERS" "$W/virtuoso.tsv"; then ST_VI=ok; fi
  t1=$(date +%s%N); VI_WALL=$(awk "BEGIN{printf \"%.1f\", ($t1-$t0)/1e9}")
  [ -f "$W/virtuoso.tsv" ] || : > "$W/virtuoso.tsv"

  # 5) qlever
  local ST_QL=failed QL_WALL
  step "[$suite-http g$gidx] qlever"
  t0=$(date +%s%N)
  if HTTP_PROFILE=1 QLEVER_QUERY_TIMEOUT=$QTO QLEVER_INDEX_TIMEOUT=1800 QLEVER_READY_TIMEOUT=180 \
       timeout 3600 bash scripts/qlever-same-box.sh "$corpus" "$fmt" "$qdir" "$ITERS" "$W/qlever.tsv"; then ST_QL=ok; fi
  t1=$(date +%s%N); QL_WALL=$(awk "BEGIN{printf \"%.1f\", ($t1-$t0)/1e9}")
  [ -f "$W/qlever.tsv" ] || : > "$W/qlever.tsv"

  # assemble meta.json + emit the envelope
  local EXPECTED_JSON STATUSES_JSON LOAD_JSON ENV_JSON NOW
  NOW=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  ENV_JSON=$(jq -n --arg cpu "$CPU" --argjson nproc "$NPROC" --arg kernel "$KERNEL" --arg now "$NOW" --arg sha "$SHA" \
    '{host_class:"dedicated-quiet-ec2-c6i.4xlarge (CANONICAL 16 vCPU / 32 GiB x86_64)",cpu_model:$cpu,nproc:$nproc,os:"Linux",kernel:$kernel,quiet_box:true,gathered_at_utc:$now,git_commit:$sha}')
  EXPECTED_JSON=$(grep -vE '^\s*#' "$exp" 2>/dev/null | awk -F'\t' 'NF>=2{print $1"\t"$2}' | jq -R -s 'split("\n")|map(select(length>0)|split("\t"))|map({(.[0]):(.[1])})|add // {}')
  STATUSES_JSON=$(jq -n --arg s "$ST_SPARQ" --arg o "$ST_OXI" --arg f "$ST_FU" --arg v "$ST_VI" --arg q "$ST_QL" '{sparq:$s,oxigraph:$o,fuseki:$f,virtuoso:$v,qlever:$q}')
  LOAD_JSON=$(jq -n --arg sl "${SPARQ_LOAD:-}" --arg ol "${OXI_LOAD:-}" --arg fw "$FU_WALL" --arg vw "$VI_WALL" --arg qw "$QL_WALL" \
    '{sparq_load_s:($sl|select(.!="")//null),oxigraph_load_s:($ol|select(.!="")//null),fuseki_wall_s:$fw,virtuoso_wall_s:$vw,qlever_wall_s:$qw,note:"sparq = sparq-server spawn->first-HTTP-answer seconds (load+bind); oxigraph = pure offline bulk-load seconds; fuseki/virtuoso/qlever = whole-recipe wall (load+serve+query+teardown)"}')
  jq -n --arg suite "${suite}-http" --arg scale "$scale" --argjson iters "$ITERS" --arg gc "$SHA" \
     --arg wave "canonical-http-ttfb-panel (sq-7d3dj.34)" --arg tsvf "$TSV_FORMAT" \
     --argjson engines "$ENGINES_JSON" --argjson statuses "$STATUSES_JSON" --argjson load "$LOAD_JSON" \
     --argjson env "$ENV_JSON" --argjson expected "$EXPECTED_JSON" --argjson connection "$CONNECTION_JSON" \
     '{suite:$suite,scale:$scale,iters:$iters,git_commit:$gc,canonical:true,wave:$wave,tsv_format:$tsvf,connection:$connection,engines:$engines,statuses:$statuses,load:$load,env:$env,expected:$expected}' \
     > "$W/meta.json"

  local OUT="bench/competitor-results/canonical-${suite}-http-$(date -u +%Y%m%dT%H%M%SZ).json"
  python3 scripts/bench/emit_envelope.py "$W" "$OUT" || step "[$suite-http g$gidx] WARN emit failed"
  jq . "$OUT" >/dev/null 2>&1 && step "[$suite-http g$gidx] envelope OK: $OUT" || step "[$suite-http g$gidx] WARN envelope invalid"
  # Upload as soon as the envelope exists — a box that dies mid-panel then still leaves
  # the completed suites in S3 (no-op unless BENCH_RESULTS_S3_URI was passed in).
  bench_egress_push "$OUT"
}

g=1
while [ "$g" -le "$GATHERS" ]; do
  want_suite sp2b && run_suite_http sp2b "$SP2B_CORPUS" ttl bench/sp2b/queries bench/sp2b/expected-rows.tsv "SP2Bench $SP2B_TRIPLES triples (real Freiburg sp2b_gen, deterministic)" "$g"
  want_suite watdiv && run_suite_http watdiv "$WATDIV_CORPUS" nt bench/watdiv/queries bench/watdiv/expected-rows.tsv "WatDiv SF=$WATDIV_SF (real Waterloo watdiv, deterministic seed=1)" "$g"
  g=$(( g + 1 ))
done

ls -la bench/competitor-results/
step "GATHER_DONE"
git rev-parse --short HEAD > /root/GATHER_DONE
sync
