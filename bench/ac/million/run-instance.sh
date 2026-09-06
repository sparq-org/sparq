#!/usr/bin/env bash
# [GPT-6] Disposable Linux experiment entrypoint; source/data stay outside results.
set -euo pipefail

mode="${1:-pilot}"
case "${mode}" in pilot|canonical) ;; *) exit 2 ;; esac
root="$(cd "$(dirname "$0")/../../.." && pwd)"
results=/var/tmp/sparq-pod-study
corpora=/var/tmp/sparq-pod-corpus
auth=/var/tmp/sparq-pod-auth
mkdir -p "${results}" "${corpora}"
write_manifest() {
  date -u +%FT%TZ > "${results}/finished-at.txt"
  manifest_tmp=$(mktemp /var/tmp/pod-results-manifest.XXXXXX)
  (cd "${results}" && find . -maxdepth 1 -type f ! -name MANIFEST.sha256 ! -name DONE -print0 | sort -z | xargs -0 sha256sum) > "${manifest_tmp}"
  mv "${manifest_tmp}" "${results}/MANIFEST.sha256"
}
date -u +%FT%TZ > "${results}/started-at.txt"
printf '%s\n' setup > "${results}/stage.txt"
cat > "${results}/runner-scope.json" <<'JSON'
{"record_type":"runner-scope","status":"exploratory-pilot","requester":"owner","query_templates":"uniform across population templates, not journey-weighted","content_writes":"replace one existing record value; no added records","policy_administration":"shared manifest-owner authorization for WAC and ACP; not ACP ACR-edit authorization","latency":"complete local HTTP response-body latency for conservative local SLO; server header diagnostic only","memory":"source-byte cache limit, with outer study cgroup; not minimum server RAM tier"}
JSON
server_pid=""
cleanup() {
  if [[ -n "${server_pid}" ]]; then kill "${server_pid}" 2>/dev/null || true; fi
}
failure() { printf '%s\n' "run failed at line $1" > "${results}/FAILED"; printf '%s\n' failed > "${results}/stage.txt"; write_manifest; }
trap 'failure ${LINENO}' ERR
trap cleanup EXIT
cd "${root}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export CARGO_TARGET_DIR=/var/tmp/sparq-pod-target
{
  uname -a
  rustc --version
  cargo --version
  lscpu
  df -B1 /var/tmp
} > "${results}/environment.txt"
git rev-parse HEAD > "${results}/source-commit.txt"
sha256sum Cargo.lock bench/ac/million/protocol.json bench/ac/million/workload.json > "${results}/input-hashes.txt"
examples=(--example pod_population_http)
if [[ "${mode}" == canonical ]]; then examples+=(--example indexed_population_preview); fi
cargo build --release --locked -p sparq-lws-core "${examples[@]}" > "${results}/build.log" 2>&1 &
build_pid=$!
while kill -0 "${build_pid}" 2>/dev/null; do
  printf '%s\n' 'Building shipping release benchmark example'
  sleep 30
done
wait "${build_pid}"
binary="${CARGO_TARGET_DIR}/release/examples/pod_population_http"
"${binary}" auth --auth-dir "${auth}" > "${results}/auth.jsonl"

finish_results() {
  printf '%s\n' complete > "${results}/stage.txt"
  write_manifest
  touch "${results}/DONE"
}

if [[ "${mode}" == canonical ]]; then
  printf '%s\n' '{"record_type":"runner-scope","status":"frozen-campaign","workload":"journeys schema v2","network_journeys":"unmeasured","memory":"separate server cgroups inside overall capped study slice"}' > "${results}/runner-scope.json"
  python3 bench/ac/million/run-campaign.py --campaign "${SPARQ_POD_CAMPAIGN:-bench/ac/million/campaign-20260906.json}" --binary "${binary}" --auth "${auth}" --results "${results}" --corpora "${corpora}"
  finish_results
  exit 0
fi
profiles=(smoke history entropy)
counts=(8 64)
requests=128
rate=4

for profile in "${profiles[@]}"; do
  profile_counts=("${counts[@]}")
  if [[ "${mode}" == pilot && "${profile}" == entropy ]]; then profile_counts=(8); fi
  for count in "${profile_counts[@]}"; do
    for policy in wac acp; do
      label="${mode}-${profile}-${count}-${policy}"
      printf '%s\n' "${label}-pack" > "${results}/stage.txt"
      corpus="${corpora}/${label}"
      free_bytes=$(df -B1 --output=avail /var/tmp | tail -1 | tr -d ' ')
      if (( free_bytes < 26843545600 )); then
        printf '{"record_type":"storage-admission-stop","label":"%s","free_bytes":%s}\n' "${label}" "${free_bytes}" > "${results}/${label}-storage-stop.json"
        touch "${results}/DONE"
        exit 0
      fi
      "${binary}" pack --corpus "${corpus}" --profile "${profile}" --pods "${count}" --model "${policy}" > "${results}/${label}-pack.jsonl"
      cp "${corpus}/manifest.json" "${results}/${label}-manifest.json"
      cp "${corpus}/pod-summaries.jsonl" "${results}/${label}-pod-summaries.jsonl"
      du -B1 "${corpus}" > "${results}/${label}-disk.txt"
      printf '%s\n' "${label}-verify" > "${results}/stage.txt"
      if ! "${binary}" verify --corpus "${corpus}" --verify-pods 8 --cache-pods 2 --cache-bytes 536870912 --max-pod-bytes 536870912 > "${results}/${label}-verify.jsonl" 2> "${results}/${label}-verify-error.txt"; then
        printf '{"record_type":"configuration-outcome","label":"%s","status":"verification-or-admission-failed","timed":false}\n' "${label}" > "${results}/${label}-outcome.json"
        continue
      fi
      taskset -c "${SPARQ_POD_CPUSET:-1}" "${binary}" serve --corpus "${corpus}" --auth-dir "${auth}" --cache-pods 4 --cache-bytes 536870912 --max-pod-bytes 536870912 --queue-capacity 256 > "${results}/${label}-server.jsonl" 2>&1 &
      server_pid=$!
      for attempt in $(seq 1 50); do
        if curl -s -o /dev/null http://127.0.0.1:3100/; then break; fi
        kill -0 "${server_pid}"
        sleep 0.2
      done
      cat "/proc/${server_pid}/status" > "${results}/${label}-status-before.txt"
      cat "/proc/${server_pid}/io" > "${results}/${label}-io-before.txt"
      printf '%s\n' "${label}-measure" > "${results}/stage.txt"
      taskset -c 0 "${binary}" load --corpus "${corpus}" --auth-dir "${auth}" --pods "${count}" --requests "${requests}" --rate "${rate}" --query-set population --mix population --workload-file "${root}/bench/ac/million/workload.json" --arrival poisson --selection uniform --seed 2026090601 --timeout-ms 5000 --out "${results}/${label}-requests.jsonl"
      cat "/proc/${server_pid}/status" > "${results}/${label}-status-after.txt"
      cat "/proc/${server_pid}/io" > "${results}/${label}-io-after.txt"
      cat "/proc/${server_pid}/smaps_rollup" > "${results}/${label}-smaps.txt"
      kill "${server_pid}"
      wait "${server_pid}" || true
      server_pid=""
    done
  done
done
finish_results
