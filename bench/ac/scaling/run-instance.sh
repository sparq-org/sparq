#!/usr/bin/env bash
# Run the AC-SPARQL pilot or canonical campaign on one disposable Linux host.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
MODE="${1:-}"
OUT_ROOT="${SPARQ_AC_INSTANCE_RESULTS:-/var/tmp/sparq-ac-study}"
CPUSET="${SPARQ_AC_CPUSET:-1}"

case "${MODE}" in
  pilot|canonical) ;;
  *) echo "usage: $0 pilot|canonical" >&2; exit 2 ;;
esac
if [[ ! "${CPUSET}" =~ ^[0-9]+$ ]]; then
  echo "SPARQ_AC_CPUSET must name one logical CPU" >&2
  exit 2
fi

mkdir -p "${OUT_ROOT}"
exec > >(tee -a "${OUT_ROOT}/study.log") 2>&1

finish() {
  local status=$?
  local manifest_tmp
  date -u +%FT%TZ >"${OUT_ROOT}/finished-at.txt"
  if (( status == 0 )); then
    : >"${OUT_ROOT}/DONE"
  else
    printf '%s\n' "${status}" >"${OUT_ROOT}/FAILED"
  fi
  manifest_tmp="$(mktemp)"
  (
    cd "${OUT_ROOT}"
    find . -type f ! -name MANIFEST.sha256 ! -name study.log -print0 \
      | sort -z \
      | xargs -0 sha256sum >"${manifest_tmp}"
  )
  mv "${manifest_tmp}" "${OUT_ROOT}/MANIFEST.sha256"
}
trap finish EXIT

stage() {
  printf '%s\t%s\n' "$(date -u +%FT%TZ)" "$*" | tee "${OUT_ROOT}/stage.txt"
}

imds_value() {
  local path="$1"
  curl -fsS --max-time 2 -H "X-aws-ec2-metadata-token: ${IMDS_TOKEN}" \
    "http://169.254.169.254/latest/meta-data/${path}"
}

verify_cgroup_limit() {
  [[ "$(stat -fc %T /sys/fs/cgroup)" == "cgroup2fs" ]] || {
    echo "canonical memory guard requires cgroup v2" >&2
    return 1
  }
  local relative maximum total_kib allowed
  relative="$(awk -F: '$1 == "0" { print $3 }' /proc/self/cgroup)"
  maximum="$(<"/sys/fs/cgroup${relative}/memory.max")"
  total_kib="$(awk '$1 == "MemTotal:" { print $2 }' /proc/meminfo)"
  [[ "${maximum}" =~ ^[0-9]+$ && "${total_kib}" =~ ^[0-9]+$ ]] || {
    echo "canonical memory guard is absent or unbounded" >&2
    return 1
  }
  allowed=$((total_kib * 1024 * 70 / 100 + 1048576))
  if (( maximum > allowed )); then
    echo "cgroup memory.max=${maximum} exceeds the 70% ceiling ${allowed}" >&2
    return 1
  fi
  printf '%s\n' "${maximum}" >"${OUT_ROOT}/cgroup-memory-max-bytes.txt"
}

stage "environment preflight"
[[ "$(uname -s)" == "Linux" ]] || { echo "Linux is required" >&2; exit 1; }
command -v taskset >/dev/null
command -v cargo >/dev/null
command -v python3 >/dev/null
[[ -z "$(git -C "${ROOT}" status --porcelain)" ]] || {
  echo "source checkout is dirty" >&2
  exit 1
}
verify_cgroup_limit
git -C "${ROOT}" rev-parse --verify origin/main >/dev/null
python3 "${ROOT}/scripts/preflight.py" --base origin/main

IMDS_TOKEN="$(curl -fsS --max-time 2 -X PUT \
  -H 'X-aws-ec2-metadata-token-ttl-seconds: 21600' \
  http://169.254.169.254/latest/api/token)"
SPARQ_BENCH_INSTANCE_ID="$(imds_value instance-id)"
SPARQ_BENCH_INSTANCE_TYPE="$(imds_value instance-type)"
SPARQ_BENCH_REGION="$(imds_value placement/region)"
SPARQ_BENCH_HOST="$(hostname)"
export SPARQ_BENCH_INSTANCE_ID SPARQ_BENCH_INSTANCE_TYPE SPARQ_BENCH_REGION SPARQ_BENCH_HOST
export SPARQ_AC_CPUSET="${CPUSET}"
export SPARQ_AC_MEMORY_FRACTION=0.70
export RAYON_NUM_THREADS=1

{
  printf 'mode=%s\n' "${MODE}"
  printf 'source_commit=%s\n' "$(git -C "${ROOT}" rev-parse HEAD)"
  printf 'source_status=%s\n' "$(git -C "${ROOT}" status --porcelain=v1)"
  printf 'instance_id=%s\n' "${SPARQ_BENCH_INSTANCE_ID}"
  printf 'instance_type=%s\n' "${SPARQ_BENCH_INSTANCE_TYPE}"
  printf 'region=%s\n' "${SPARQ_BENCH_REGION}"
  printf 'cpuset=%s\n' "${CPUSET}"
  printf 'rustc=%s\n' "$(rustc --version --verbose | tr '\n' ';')"
  printf 'cargo=%s\n' "$(cargo --version)"
  printf 'python=%s\n' "$(python3 --version)"
  printf 'kernel=%s\n' "$(uname -a)"
  printf 'cgroup=%s\n' "$(cat /proc/self/cgroup)"
  printf 'memory_max_bytes=%s\n' "$(cat "${OUT_ROOT}/cgroup-memory-max-bytes.txt")"
  printf 'meminfo=%s\n' "$(grep -E '^(MemTotal|MemAvailable):' /proc/meminfo | tr '\n' ';')"
  printf 'lscpu=%s\n' "$(lscpu | tr '\n' ';')"
  printf 'governor=%s\n' "$(cat "/sys/devices/system/cpu/cpu${CPUSET}/cpufreq/scaling_governor" 2>/dev/null || printf unavailable)"
} >"${OUT_ROOT}/environment.txt"
date -u +%FT%TZ >"${OUT_ROOT}/started-at.txt"

run_benchmark() {
  local label="$1" results="$2"
  shift 2
  stage "${label}"
  mkdir -p "${results}"
  SPARQ_AC_RESULTS="${results}" "$@"
}

if [[ "${MODE}" == "pilot" ]]; then
  run_benchmark "full correctness matrix" "${OUT_ROOT}/correctness" \
    bash "${ROOT}/bench/ac/scaling/run.sh" --correctness
  run_benchmark "largest-cell timing pilot" "${OUT_ROOT}/scenarios-timing" \
    env SPARQ_AC_FACTOR_BLOCKS=1 SPARQ_AC_FACTOR_REPETITIONS=1 \
      SPARQ_AC_FACTOR_WARMUPS=1 \
      bash "${ROOT}/bench/ac/scaling/run.sh" --scenarios
  run_benchmark "largest-cell instrumentation pilot" "${OUT_ROOT}/scenarios-instrumentation" \
    env SPARQ_AC_FACTOR_BLOCKS=1 SPARQ_AC_INSTRUMENTATION_REPETITIONS=1 \
      SPARQ_AC_INSTRUMENTATION_WARMUPS=1 \
      bash "${ROOT}/bench/ac/scaling/run.sh" --scenarios-instrumentation
  run_benchmark "primary-endpoint timing pilot" "${OUT_ROOT}/pod-scaling" \
    env SPARQ_AC_PODS="1 64 2048" SPARQ_AC_BLOCKS=1 \
      SPARQ_AC_REPETITIONS=3 SPARQ_AC_WARMUPS=2 \
      bash "${ROOT}/bench/ac/scaling/run.sh" --pod-scaling
else
  run_benchmark "full correctness matrix" "${OUT_ROOT}/correctness" \
    bash "${ROOT}/bench/ac/scaling/run.sh" --correctness
  run_benchmark "primary timing campaign" "${OUT_ROOT}/raw" \
    bash "${ROOT}/bench/ac/scaling/run.sh" --pod-scaling
  run_benchmark "sensitivity timing campaign" "${OUT_ROOT}/raw" \
    bash "${ROOT}/bench/ac/scaling/run.sh" --sensitivity
  run_benchmark "scenario timing campaign" "${OUT_ROOT}/raw" \
    bash "${ROOT}/bench/ac/scaling/run.sh" --scenarios
  run_benchmark "primary instrumentation campaign" "${OUT_ROOT}/raw" \
    bash "${ROOT}/bench/ac/scaling/run.sh" --pod-scaling-instrumentation
  run_benchmark "sensitivity instrumentation campaign" "${OUT_ROOT}/raw" \
    bash "${ROOT}/bench/ac/scaling/run.sh" --sensitivity-instrumentation
  run_benchmark "scenario instrumentation campaign" "${OUT_ROOT}/raw" \
    bash "${ROOT}/bench/ac/scaling/run.sh" --scenarios-instrumentation
  stage "canonical validation and analysis"
  python3 "${ROOT}/bench/ac/scaling/analyze.py" "${OUT_ROOT}/raw" \
    --out "${OUT_ROOT}/derived" --bootstrap-draws 10000 \
    --require-canonical --expected-commit "$(git -C "${ROOT}" rev-parse HEAD)"
fi

stage "complete"
