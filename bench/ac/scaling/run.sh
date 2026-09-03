#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
RESULTS="${SPARQ_AC_RESULTS:-${ROOT}/bench/ac/scaling/results}"
MIN_FREE_KIB=$((25 * 1024 * 1024))

usage() {
  cat <<'EOF'
Usage: bench/ac/scaling/run.sh --correctness | --smoke | --pod-scaling |
       --sensitivity | --scenarios | --instrumentation-smoke |
       --pod-scaling-instrumentation | --sensitivity-instrumentation |
       --scenarios-instrumentation

Environment for --correctness:
  SPARQ_AC_CORRECTNESS_LANES  space-separated lanes (default: materialized http)

Environment for --pod-scaling:
  SPARQ_AC_PODS        space-separated Pod counts (default: 1 8 64 512 2048)
  SPARQ_AC_DOMAINS     space-separated domains (default: social health)
  SPARQ_AC_LANES       space-separated lanes (default: materialized http)
  SPARQ_AC_BLOCKS      process-block count (default: 5)
  SPARQ_AC_REPETITIONS raw repetitions per query/path (default: 30)
  SPARQ_AC_WARMUPS     warm-ups per query/path (default: 10)
  SPARQ_AC_RESULTS     output directory (default: bench/ac/scaling/results)

Environment for --sensitivity and --scenarios:
  SPARQ_AC_FACTOR_BLOCKS       process-block count (default: 5)
  SPARQ_AC_FACTOR_REPETITIONS  repetitions per query/path (default: 30)
  SPARQ_AC_FACTOR_WARMUPS      warm-ups per query/path (default: 10)
  SPARQ_AC_DOMAINS             space-separated domains (default: social health)
  SPARQ_AC_LANES               space-separated lanes (default: materialized http)

Environment for all modes:
  SPARQ_AC_CPUSET          one logical CPU for taskset (required for canonical runs)
  SPARQ_AC_MEMORY_FRACTION resident-memory stop fraction (default: 0.70)

Instrumentation modes use the same matrices but compile deterministic allocation and
backend-operation counters into the runner. Their defaults are one warm-up and one
repetition; their wall time is never pooled with the production-allocator timing modes.

The Pod-count intervention always uses all-private documents so Pod 0's owner's
authorized slice remains fixed as background Pods are added.
EOF
}

free_kib() {
  df -Pk "${ROOT}" | awk 'NR == 2 { print $4 }'
}

guard_disk() {
  local available
  available="$(free_kib)"
  if [[ -z "${available}" || ! "${available}" =~ ^[0-9]+$ ]]; then
    echo "cannot determine free disk" >&2
    return 1
  fi
  if (( available < MIN_FREE_KIB )); then
    echo "resource guard: ${available} KiB free is below the 25 GiB floor" >&2
    return 1
  fi
}

guard_resources() {
  guard_disk
  python3 "${ROOT}/bench/ac/scaling/resource_guard.py" "${RESULTS}" \
    --fraction "${SPARQ_AC_MEMORY_FRACTION:-0.70}" --quiet
}

run_runner() {
  if [[ -n "${SPARQ_AC_CPUSET:-}" ]]; then
    if [[ ! "${SPARQ_AC_CPUSET}" =~ ^[0-9]+$ ]]; then
      echo "SPARQ_AC_CPUSET must name exactly one non-negative logical CPU" >&2
      return 2
    fi
    if ! command -v taskset >/dev/null 2>&1; then
      echo "SPARQ_AC_CPUSET requires taskset, but taskset is unavailable" >&2
      return 2
    fi
    taskset -c "${SPARQ_AC_CPUSET}" "${RUNNER_PATH}" "$@"
  else
    "${RUNNER_PATH}" "$@"
  fi
}

checksum_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${path}" >"${path}.sha256"
  else
    shasum -a 256 "${path}" >"${path}.sha256"
  fi
}

completed_cell_exists() {
  local path="$1" sidecar="${1}.sha256" expected actual
  if [[ ! -e "${path}" && ! -e "${sidecar}" ]]; then
    return 1
  fi
  if [[ ! -s "${path}" || ! -s "${sidecar}" ]]; then
    echo "resume refused: ${path} and its checksum must both be nonempty" >&2
    return 2
  fi
  read -r expected _ <"${sidecar}"
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "${path}" | awk '{print $1}')"
  else
    actual="$(shasum -a 256 "${path}" | awk '{print $1}')"
  fi
  expected="$(printf '%s' "${expected}" | tr '[:upper:]' '[:lower:]')"
  if [[ ! "${expected}" =~ ^[0-9a-f]{64}$ || "${actual}" != "${expected}" ]]; then
    echo "resume refused: checksum mismatch for ${path}" >&2
    return 2
  fi
  return 0
}

seed_for_block() {
  case "$1" in
    0) echo 17 ;;
    1) echo 42 ;;
    2) echo 101 ;;
    3) echo 314 ;;
    4) echo 2718 ;;
    *) echo $((17 + "$1" * 31)) ;;
  esac
}

RUN_PROFILE="timing"
RUNNER_PATH=""

build_runner() {
  guard_resources
  local built="${ROOT}/target/release/examples/ac_query_scale"
  if [[ "${RUN_PROFILE}" == "instrumentation" ]]; then
    cargo build --locked --release -p sparq-lws-core --example ac_query_scale \
      --features ac-query-scale-instrumentation
  else
    cargo build --locked --release -p sparq-lws-core --example ac_query_scale
  fi
  RUNNER_PATH="${built}-${RUN_PROFILE}"
  cp "${built}" "${RUNNER_PATH}"
}

run_cell() {
  local lane="$1" domain="$2" pods="$3" block="$4" reps="$5" warmups="$6"
  local order_index="${7:-}" order_seed="${8:-}"
  local topology="origin-per-pod"
  if [[ "${lane}" == "http" ]]; then
    topology="shared-origin"
  fi
  guard_resources
  mkdir -p "${RESULTS}"
  local seed
  seed="$(seed_for_block "${block}")"
  local campaign="pod-scaling"
  if [[ "${RUN_PROFILE}" == "instrumentation" ]]; then
    campaign="pod-scaling-instrumentation"
  fi
  local run_id="${campaign}-${lane}-${domain}-p${pods}-b${block}-s${seed}"
  local final="${RESULTS}/${run_id}.jsonl"
  local partial="${final}.partial"
  local log="${RESULTS}/${run_id}.stderr.log"
  if completed_cell_exists "${final}"; then
    echo "skip existing ${final}" >&2
    return 0
  else
    local resume_status=$?
    (( resume_status == 1 )) || return "${resume_status}"
  fi
  echo "run ${run_id}" >&2
  if [[ -n "${order_index}" && -n "${order_seed}" ]]; then
    set -- --configuration-order "${order_index}" --configuration-order-seed "${order_seed}"
  else
    set --
  fi
  RAYON_NUM_THREADS="${RAYON_NUM_THREADS:-1}" \
    run_runner \
      --lane "${lane}" --campaign "${campaign}" --cell-label "pods-${pods}" \
      --pods "${pods}" --documents 16 --triples 8 --depth 3 \
      --acl-coverage 250 --public 0 --private 1000 --shared 0 \
      --domain "${domain}" --topology "${topology}" --principal owner \
      --seed "${seed}" --warmups "${warmups}" \
      --repetitions "${reps}" --process-block "${block}" --run-id "${run_id}" \
      "$@" \
      >"${partial}" 2>"${log}"
  mv "${partial}" "${final}"
  checksum_file "${final}"
}

run_factor_cell() {
  local campaign="$1" label="$2" lane="$3" domain="$4" pods="$5"
  local documents="$6" triples="$7" depth="$8" coverage="$9"
  shift 9
  local public="$1" private="$2" shared="$3" principal="$4" seed="$5"
  local block="$6" reps="$7" warmups="$8" order_index="$9"
  shift 9
  local order_seed="$1" queries="$2"
  local topology="origin-per-pod"
  if [[ "${lane}" == "http" ]]; then
    topology="shared-origin"
  fi
  guard_resources
  mkdir -p "${RESULTS}"
  local run_id="${campaign}-${label}-${lane}-${domain}-b${block}-s${seed}"
  local final="${RESULTS}/${run_id}.jsonl"
  local partial="${final}.partial"
  local log="${RESULTS}/${run_id}.stderr.log"
  if completed_cell_exists "${final}"; then
    echo "skip existing ${final}" >&2
    return 0
  else
    local resume_status=$?
    (( resume_status == 1 )) || return "${resume_status}"
  fi
  echo "run ${run_id}" >&2
  RAYON_NUM_THREADS="${RAYON_NUM_THREADS:-1}" \
    run_runner \
      --lane "${lane}" --campaign "${campaign}" --cell-label "${label}" \
      --pods "${pods}" --documents "${documents}" \
      --triples "${triples}" --depth "${depth}" --acl-coverage "${coverage}" \
      --public "${public}" --private "${private}" --shared "${shared}" \
      --domain "${domain}" --topology "${topology}" --principal "${principal}" \
      --seed "${seed}" --warmups "${warmups}" --repetitions "${reps}" \
      --process-block "${block}" --run-id "${run_id}" --query "${queries}" \
      --configuration-order "${order_index}" \
      --configuration-order-seed "${order_seed}" \
      >"${partial}" 2>"${log}"
  mv "${partial}" "${final}"
  checksum_file "${final}"
}

run_correctness() {
  cargo test --locked -p sparq-acbench --test deployment
  cargo test --locked -p sparq-solid --test acbench_deployment
  cargo test --locked -p sparq-lws-core --features sparql-endpoint --test sparql_endpoint
  build_runner
  local correctness_lanes="${SPARQ_AC_CORRECTNESS_LANES:-materialized http}"
  local lane domain principal coverage seed pods
  for lane in ${correctness_lanes}; do
    for domain in social health; do
      for principal in owner recipient stranger anonymous; do
        for coverage in 100 1000; do
          for seed in 17 42 101; do
            for pods in 1 8 32; do
              guard_resources
              local topology="origin-per-pod"
              [[ "${lane}" == "http" ]] && topology="shared-origin"
              local id="correctness-${lane}-${domain}-${principal}-p${pods}-a${coverage}-s${seed}"
              local final="${RESULTS}/${id}.jsonl"
              if completed_cell_exists "${final}"; then
                echo "skip existing ${final}" >&2
                continue
              else
                local resume_status=$?
                (( resume_status == 1 )) || return "${resume_status}"
              fi
              RAYON_NUM_THREADS="${RAYON_NUM_THREADS:-1}" run_runner \
                --lane "${lane}" --campaign correctness --cell-label "${principal}-${coverage}" \
                --pods "${pods}" --documents 8 --triples 8 --depth 3 \
                --acl-coverage "${coverage}" --public 300 --private 400 --shared 300 \
                --domain "${domain}" --topology "${topology}" --principal "${principal}" \
                --seed "${seed}" --warmups 0 --repetitions 1 --process-block 0 \
                --run-id "${id}" >"${final}"
              checksum_file "${final}"
            done
          done
        done
      done
    done
  done
}

run_smoke() {
  build_runner
  local reps=3 warmups=2
  if [[ "${RUN_PROFILE}" == "instrumentation" ]]; then
    reps="${SPARQ_AC_INSTRUMENTATION_REPETITIONS:-1}"
    warmups="${SPARQ_AC_INSTRUMENTATION_WARMUPS:-1}"
  fi
  local lane domain pods
  for lane in materialized http; do
    for domain in social health; do
      for pods in 1 8; do
        run_cell "${lane}" "${domain}" "${pods}" 0 "${reps}" "${warmups}"
      done
    done
  done
}

run_pod_scaling() {
  build_runner
  local pod_values="${SPARQ_AC_PODS:-1 8 64 512 2048}"
  local domains="${SPARQ_AC_DOMAINS:-social health}"
  local lanes="${SPARQ_AC_LANES:-materialized http}"
  local blocks="${SPARQ_AC_BLOCKS:-5}"
  local reps="${SPARQ_AC_REPETITIONS:-30}"
  local warmups="${SPARQ_AC_WARMUPS:-10}"
  local block lane domain pods order_index order_seed
  for ((block = 0; block < blocks; block++)); do
    # A pinned seed randomizes the complete lane/domain/Pod cross-product and is
    # recorded in every raw row, preventing size from aligning with thermal drift.
    while IFS=$'\t' read -r order_index lane domain pods order_seed; do
      run_cell "${lane}" "${domain}" "${pods}" "${block}" "${reps}" "${warmups}" \
        "${order_index}" "${order_seed}"
    done < <(
      python3 "${ROOT}/bench/ac/scaling/schedule.py" \
        --lanes "${lanes}" --domains "${domains}" --pods "${pod_values}" \
        --seed "$((20260903 + block))"
    )
  done
}

run_factor_campaign() {
  local campaign="$1"
  local record_campaign="${campaign}"
  if [[ "${RUN_PROFILE}" == "instrumentation" ]]; then
    record_campaign="${campaign}-instrumentation"
  fi
  build_runner
  local domains="${SPARQ_AC_DOMAINS:-social health}"
  local lanes="${SPARQ_AC_LANES:-materialized http}"
  local blocks="${SPARQ_AC_FACTOR_BLOCKS:-5}"
  local reps="${SPARQ_AC_FACTOR_REPETITIONS:-30}"
  local warmups="${SPARQ_AC_FACTOR_WARMUPS:-10}"
  local block seed order_seed
  for ((block = 0; block < blocks; block++)); do
    seed="$(seed_for_block "${block}")"
    if [[ "${campaign}" == "sensitivity" ]]; then
      order_seed=$((20260913 + block))
    else
      order_seed=$((20260923 + block))
    fi
    while IFS=$'\t' read -r order_index label lane domain pods documents triples depth \
      coverage public private shared principal emitted_seed emitted_order_seed queries; do
      if [[ "${emitted_seed}" != "${seed}" || "${emitted_order_seed}" != "${order_seed}" ]]; then
        echo "factor schedule metadata mismatch" >&2
        return 1
      fi
      run_factor_cell "${record_campaign}" "${label}" "${lane}" "${domain}" "${pods}" \
        "${documents}" "${triples}" "${depth}" "${coverage}" "${public}" \
        "${private}" "${shared}" "${principal}" "${seed}" "${block}" "${reps}" \
        "${warmups}" "${order_index}" "${order_seed}" "${queries}"
    done < <(
      python3 "${ROOT}/bench/ac/scaling/factor_schedule.py" \
        --campaign "${campaign}" --lanes "${lanes}" --domains "${domains}" \
        --corpus-seed "${seed}" --order-seed "${order_seed}"
    )
  done
}

cd "${ROOT}"
mkdir -p "${RESULTS}"
case "${1:-}" in
  --correctness) run_correctness ;;
  --smoke) run_smoke ;;
  --pod-scaling) run_pod_scaling ;;
  --sensitivity) run_factor_campaign sensitivity ;;
  --scenarios) run_factor_campaign scenarios ;;
  --instrumentation-smoke)
    RUN_PROFILE="instrumentation"
    run_smoke
    ;;
  --pod-scaling-instrumentation)
    RUN_PROFILE="instrumentation"
    SPARQ_AC_REPETITIONS="${SPARQ_AC_INSTRUMENTATION_REPETITIONS:-1}" \
      SPARQ_AC_WARMUPS="${SPARQ_AC_INSTRUMENTATION_WARMUPS:-1}" run_pod_scaling
    ;;
  --sensitivity-instrumentation)
    RUN_PROFILE="instrumentation"
    SPARQ_AC_FACTOR_REPETITIONS="${SPARQ_AC_INSTRUMENTATION_REPETITIONS:-1}" \
      SPARQ_AC_FACTOR_WARMUPS="${SPARQ_AC_INSTRUMENTATION_WARMUPS:-1}" \
      run_factor_campaign sensitivity
    ;;
  --scenarios-instrumentation)
    RUN_PROFILE="instrumentation"
    SPARQ_AC_FACTOR_REPETITIONS="${SPARQ_AC_INSTRUMENTATION_REPETITIONS:-1}" \
      SPARQ_AC_FACTOR_WARMUPS="${SPARQ_AC_INSTRUMENTATION_WARMUPS:-1}" \
      run_factor_campaign scenarios
    ;;
  *) usage; exit 2 ;;
esac
