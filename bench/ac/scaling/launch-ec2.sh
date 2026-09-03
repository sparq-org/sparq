#!/usr/bin/env bash
# Launch one orphan-proof, dedicated EC2 host for the AC-SPARQL study.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
MODE="${1:-}"
PROFILE="${AWS_PROFILE:-pss}"
REGION="${AWS_REGION:-eu-west-2}"
INSTANCE_TYPE="${SPARQ_AC_INSTANCE_TYPE:-r7g.xlarge}"
VOLUME_GB="${SPARQ_AC_VOLUME_GB:-80}"
CPUSET="${SPARQ_AC_CPUSET:-1}"
WATCHDOG_SECONDS="${SPARQ_AC_WATCHDOG_SECONDS:-43200}"
POLL_DEADLINE_SECONDS="${SPARQ_AC_POLL_DEADLINE_SECONDS:-42600}"
POLL_INTERVAL_SECONDS="${SPARQ_AC_POLL_INTERVAL_SECONDS:-45}"
PRIOR_AWS_USD="${SPARQ_AC_PRIOR_AWS_USD:-0}"
RUN_STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RESULTS_LOCAL="${SPARQ_AC_RESULTS_LOCAL:-${ROOT}/bench/ac/scaling/results/ec2-${MODE}-${RUN_STAMP}}"
RUN_TOKEN="sparq-ac-${MODE}-${RUN_STAMP}-$$"

PROD_INSTANCE="i-090531b4ede8f2d3f"
DEV_INSTANCE="i-00f76802f345b6b77"
TAGSPEC="ResourceType=instance,Tags=[{Key=Name,Value=sparq-ac-study},{Key=Project,Value=sparq},{Key=purpose,Value=sparq-bench},{Key=study-run,Value=${RUN_TOKEN}}]"

log() { printf '[ac-ec2 %s] %s\n' "$(date -u +%H:%M:%S)" "$*" >&2; }
die() { printf '[ac-ec2] ERROR: %s\n' "$*" >&2; exit 1; }

case "${MODE}" in
  pilot|canonical) ;;
  *) die "usage: $0 pilot|canonical" ;;
esac
[[ "${REGION}" == "eu-west-2" ]] || die "the frozen study region is eu-west-2"
[[ "${INSTANCE_TYPE}" =~ ^[a-z0-9.]+$ ]] || die "invalid instance type"
if [[ ! "${VOLUME_GB}" =~ ^[0-9]+$ ]] || (( VOLUME_GB < 40 || VOLUME_GB > 200 )); then
  die "SPARQ_AC_VOLUME_GB must be between 40 and 200"
fi
[[ "${CPUSET}" =~ ^[0-9]+$ ]] || die "SPARQ_AC_CPUSET must name one logical CPU"
if [[ ! "${WATCHDOG_SECONDS}" =~ ^[0-9]+$ ]] || (( WATCHDOG_SECONDS > 43200 )); then
  die "watchdog must be at most 43200 seconds"
fi
if [[ ! "${POLL_DEADLINE_SECONDS}" =~ ^[0-9]+$ ]] \
  || (( POLL_DEADLINE_SECONDS >= WATCHDOG_SECONDS )); then
  die "poll deadline must be shorter than the watchdog"
fi

command -v aws >/dev/null || die "aws CLI is unavailable"
command -v git >/dev/null || die "git is unavailable"
command -v ssh >/dev/null || die "ssh is unavailable"
command -v scp >/dev/null || die "scp is unavailable"
command -v rsync >/dev/null || die "rsync is unavailable"
command -v curl >/dev/null || die "curl is unavailable"

[[ -z "$(git -C "${ROOT}" status --porcelain)" ]] \
  || die "the source tree must be clean before a canonical bundle is made"
SOURCE_COMMIT="$(git -C "${ROOT}" rev-parse HEAD)"
[[ "${SOURCE_COMMIT}" =~ ^[0-9a-f]{40}$ ]] || die "cannot resolve source commit"

mkdir -p "${RESULTS_LOCAL}"
WORK="$(mktemp -d)"
KEYFILE="${WORK}/key"
BUNDLE="${WORK}/sparq.bundle"
KEY_NAME="${RUN_TOKEN}"
INSTANCE_ID=""
SECURITY_GROUP_ID=""
SSH_RULE_ID=""
CURRENT_SSH_CIDR=""

cleanup() {
  local original_status=$? cleanup_failed=0
  local purpose="" study_run="" discovered="" candidates="" handled="" candidate=""
  local attempt
  trap - EXIT
  set +e
  # `run-instances` can succeed server-side while its response is lost. The unique
  # idempotency/tag token lets cleanup discover that instance even if INSTANCE_ID was
  # never assigned locally. Retry boundedly for tag eventual consistency. Both exact
  # tags are re-read before any termination call.
  for attempt in 1 2 3 4; do
    if discovered="$(aws ec2 describe-instances --profile "${PROFILE}" --region "${REGION}" \
      --filters "Name=tag:study-run,Values=${RUN_TOKEN}" \
        'Name=instance-state-name,Values=pending,running,stopping,stopped' \
      --query 'Reservations[].Instances[].InstanceId' --output text 2>/dev/null)"; then
      [[ -n "${discovered}" && "${discovered}" != "None" ]] && break
    else
      log "cleanup discovery attempt ${attempt} failed"
      cleanup_failed=1
    fi
    [[ -n "${INSTANCE_ID}" ]] && break
    (( attempt < 4 )) && sleep 5
  done
  candidates="${INSTANCE_ID} ${discovered//$'\t'/ }"
  for candidate in ${candidates}; do
    case " ${handled} " in *" ${candidate} "*) continue ;; esac
    handled="${handled} ${candidate}"
    case "${candidate}" in
      "${PROD_INSTANCE}"|"${DEV_INSTANCE}")
        log "REFUSING to terminate protected instance ${candidate}"
        ;;
      i-*)
        # shellcheck disable=SC2016 # Backticks are JMESPath syntax, not shell expansion.
        purpose="$(aws ec2 describe-instances --profile "${PROFILE}" --region "${REGION}" \
          --instance-ids "${candidate}" \
          --query 'Reservations[0].Instances[0].Tags[?Key==`purpose`]|[0].Value' \
          --output text 2>/dev/null)"
        # shellcheck disable=SC2016 # Backticks are JMESPath syntax, not shell expansion.
        study_run="$(aws ec2 describe-instances --profile "${PROFILE}" --region "${REGION}" \
          --instance-ids "${candidate}" \
          --query 'Reservations[0].Instances[0].Tags[?Key==`study-run`]|[0].Value' \
          --output text 2>/dev/null)"
        if [[ "${purpose}" == "sparq-bench" && "${study_run}" == "${RUN_TOKEN}" ]]; then
          log "terminating ${candidate}"
          if ! aws ec2 terminate-instances --profile "${PROFILE}" --region "${REGION}" \
            --instance-ids "${candidate}" >/dev/null 2>&1; then
            log "ERROR: termination request failed for ${candidate}"
            cleanup_failed=1
          elif ! aws ec2 wait instance-terminated --profile "${PROFILE}" --region "${REGION}" \
            --instance-ids "${candidate}" >/dev/null 2>&1; then
            log "ERROR: ${candidate} did not reach terminated state"
            cleanup_failed=1
          fi
        else
          log "REFUSING cleanup for ${candidate}: purpose=${purpose:-absent} study-run=${study_run:-absent}"
          cleanup_failed=1
        fi
        ;;
    esac
  done
  if [[ -n "${SECURITY_GROUP_ID}" ]]; then
    if ! aws ec2 delete-security-group --profile "${PROFILE}" --region "${REGION}" \
      --group-id "${SECURITY_GROUP_ID}" >/dev/null 2>&1; then
      log "ERROR: failed to delete security group ${SECURITY_GROUP_ID}"
      cleanup_failed=1
    fi
  fi
  aws ec2 delete-key-pair --profile "${PROFILE}" --region "${REGION}" \
    --key-name "${KEY_NAME}" >/dev/null 2>&1
  rm -rf "${WORK}"
  if (( cleanup_failed != 0 )); then
    printf 'cleanup_failed\n' >"${RESULTS_LOCAL}/cleanup-status.txt"
    exit 1
  fi
  printf 'cleanup_complete\n' >"${RESULTS_LOCAL}/cleanup-status.txt"
  exit "${original_status}"
}
trap cleanup EXIT

stage_pull() {
  rsync -az --partial -e "ssh ${SSH_OPTIONS[*]}" \
    "ubuntu@${PUBLIC_IP}:/var/tmp/sparq-ac-study/" "${RESULTS_LOCAL}/" 2>/dev/null || true
}

valid_ipv4() {
  local address="$1" octet
  [[ "${address}" =~ ^([0-9]{1,3}\.){3}[0-9]{1,3}$ ]] || return 1
  for octet in ${address//./ }; do
    (( 10#${octet} <= 255 )) || return 1
  done
}

refresh_ssh_ingress() {
  local current_ip new_cidr previous_cidr
  [[ -n "${SECURITY_GROUP_ID}" && -n "${SSH_RULE_ID}" ]] || return 0
  if ! current_ip="$(curl -4 -fsS --max-time 10 https://checkip.amazonaws.com \
    | tr -d '[:space:]')"; then
    log "could not refresh the client public IP; retaining ${CURRENT_SSH_CIDR}"
    return 0
  fi
  if ! valid_ipv4 "${current_ip}"; then
    log "public-IP refresh returned an invalid address; retaining ${CURRENT_SSH_CIDR}"
    return 0
  fi
  new_cidr="${current_ip}/32"
  [[ "${new_cidr}" == "${CURRENT_SSH_CIDR}" ]] && return 0
  previous_cidr="${CURRENT_SSH_CIDR}"
  log "rotating the single SSH ingress rule from ${previous_cidr} to ${new_cidr}"
  if ! aws ec2 modify-security-group-rules --profile "${PROFILE}" --region "${REGION}" \
    --group-id "${SECURITY_GROUP_ID}" \
    --security-group-rules \
      "SecurityGroupRuleId=${SSH_RULE_ID},SecurityGroupRule={IpProtocol=tcp,FromPort=22,ToPort=22,CidrIpv4=${new_cidr}}" \
    >/dev/null; then
    log "ERROR: could not rotate the exact SSH ingress rule"
    return 1
  fi
  CURRENT_SSH_CIDR="${new_cidr}"
  printf '%s\t%s\t%s\n' "$(date -u +%FT%TZ)" "${previous_cidr}" "${new_cidr}" \
    >>"${RESULTS_LOCAL}/ssh-ingress-rotations.tsv"
}

ssh_with_ingress_retry() {
  local attempt status
  for attempt in $(seq 1 20); do
    refresh_ssh_ingress || return 1
    # shellcheck disable=SC2029 # Callers provide the complete validated remote command.
    if ssh "${SSH_OPTIONS[@]}" "ubuntu@${PUBLIC_IP}" "$@"; then
      return 0
    else
      status=$?
    fi
    # An established remote command failed for a reason other than transport. Preserve that
    # failure instead of masking a source-verification or service-launch error with retries.
    (( status == 255 )) || return "${status}"
    log "SSH transport attempt ${attempt}/20 failed after ingress synchronization"
    sleep 3
  done
  return 255
}

scp_with_ingress_retry() {
  local source="$1" destination="$2" attempt
  for attempt in $(seq 1 20); do
    refresh_ssh_ingress || return 1
    if scp "${SSH_OPTIONS[@]}" "${source}" "ubuntu@${PUBLIC_IP}:${destination}"; then
      return 0
    fi
    log "SCP transport attempt ${attempt}/20 failed after ingress synchronization"
    sleep 3
  done
  return 1
}

orphan_preflight() {
  if (( BASH_VERSINFO[0] >= 4 )); then
    AWS_PROFILE="${PROFILE}" bash "${ROOT}/scripts/orphan-check-bench.sh" --region "${REGION}"
    return
  fi
  # The repository checker uses Bash 4's mapfile after its hermetic safety assertions.
  # macOS ships Bash 3, so run those assertions and then issue the identical read-only,
  # exact-tag query here rather than weakening or skipping the preflight.
  bash "${ROOT}/scripts/orphan-check-bench.sh" --dry-run-self-test
  local live
  live="$(aws ec2 describe-instances --profile "${PROFILE}" --region "${REGION}" \
    --filters 'Name=tag:purpose,Values=sparq-bench' \
      'Name=instance-state-name,Values=running,pending' \
    --query 'Reservations[].Instances[].InstanceId' --output text)"
  [[ -z "${live}" || "${live}" == "None" ]] \
    || die "orphan preflight found live sparq-bench instance(s): ${live}"
}

log "orphan check, identity, and current price quote"
orphan_preflight
aws sts get-caller-identity --profile "${PROFILE}" --output json \
  >"${RESULTS_LOCAL}/aws-identity.json"
date -u +%FT%TZ >"${RESULTS_LOCAL}/price-checked-at.txt"
PRICE_QUERY_OK=0
if aws pricing get-products --profile "${PROFILE}" --region us-east-1 \
  --service-code AmazonEC2 \
  --filters \
    "Type=TERM_MATCH,Field=location,Value=EU (London)" \
    "Type=TERM_MATCH,Field=instanceType,Value=${INSTANCE_TYPE}" \
    "Type=TERM_MATCH,Field=operatingSystem,Value=Linux" \
    "Type=TERM_MATCH,Field=tenancy,Value=Shared" \
    "Type=TERM_MATCH,Field=preInstalledSw,Value=NA" \
    "Type=TERM_MATCH,Field=capacitystatus,Value=Used" \
  --max-results 100 --output json >"${WORK}/pricing.json" \
  2>"${RESULTS_LOCAL}/aws-pricing-query-error.txt"; then
  if python3 "${ROOT}/bench/ac/scaling/aws_price.py" "${WORK}/pricing.json" \
    --hours 12 \
    --ancillary-reserve 5 --prior-spend "${PRIOR_AWS_USD}" \
    >"${RESULTS_LOCAL}/cost-estimate.json" \
    2>"${RESULTS_LOCAL}/aws-pricing-query-parse-error.txt"; then
    PRICE_QUERY_OK=1
    cp "${WORK}/pricing.json" "${RESULTS_LOCAL}/aws-pricing-response.json"
  fi
fi
if (( PRICE_QUERY_OK == 0 )); then
  BULK_PRICE_URL="https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/current/${REGION}/index.csv"
  log "signed Price List query unavailable; using the official public regional bulk file"
  printf '%s\n' "${BULK_PRICE_URL}" >"${RESULTS_LOCAL}/aws-bulk-pricing-url.txt"
  curl --fail --silent --show-error --location --retry 3 --retry-all-errors \
    --connect-timeout 30 --max-time 600 \
    --dump-header "${RESULTS_LOCAL}/aws-bulk-pricing-headers.txt" \
    --output "${WORK}/pricing.csv" "${BULK_PRICE_URL}"
  if command -v sha256sum >/dev/null; then
    PRICE_DIGEST="$(sha256sum "${WORK}/pricing.csv")"
  else
    PRICE_DIGEST="$(shasum -a 256 "${WORK}/pricing.csv")"
  fi
  printf '%s\n' "${PRICE_DIGEST%% *}" \
    >"${RESULTS_LOCAL}/aws-bulk-pricing.sha256"
  python3 "${ROOT}/bench/ac/scaling/aws_price.py" "${WORK}/pricing.csv" \
    --bulk-csv --instance-type "${INSTANCE_TYPE}" \
    --location 'EU (London)' --region-code "${REGION}" \
    --hours 12 --ancillary-reserve 5 --prior-spend "${PRIOR_AWS_USD}" \
    >"${RESULTS_LOCAL}/cost-estimate.json"
fi

log "creating exact source bundle for ${SOURCE_COMMIT}"
# Include the review base as an explicitly named prerequisite-free ref. The remote clone
# fetches it below so Linux preflight can judge this exact diff, not an empty checkout.
git -C "${ROOT}" bundle create "${BUNDLE}" HEAD origin/main
git -C "${ROOT}" bundle verify "${BUNDLE}" >/dev/null
ssh-keygen -t ed25519 -N '' -f "${KEYFILE}" -q
SSH_OPTIONS=(
  -i "${KEYFILE}"
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
  -o ConnectTimeout=15
  -o ServerAliveInterval=30
  -o ServerAliveCountMax=3
)

log "resolving Ubuntu 24.04 arm64 image and locked-down network"
AMI="$(aws ec2 describe-images --profile "${PROFILE}" --region "${REGION}" \
  --owners 099720109477 \
  --filters 'Name=name,Values=ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-arm64-server-*' \
    'Name=state,Values=available' \
  --query 'sort_by(Images,&CreationDate)[-1].ImageId' --output text)"
[[ "${AMI}" == ami-* ]] || die "could not resolve the Ubuntu arm64 AMI"
VPC="$(aws ec2 describe-vpcs --profile "${PROFILE}" --region "${REGION}" \
  --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)"
SUBNET="$(aws ec2 describe-subnets --profile "${PROFILE}" --region "${REGION}" \
  --filters Name=vpc-id,Values="${VPC}" Name=default-for-az,Values=true \
  --query 'Subnets[0].SubnetId' --output text)"
PUBLIC_CIDR="$(curl -4 -fsS https://checkip.amazonaws.com | tr -d '[:space:]')/32"
valid_ipv4 "${PUBLIC_CIDR%/32}" || die "public-IP lookup returned an invalid IPv4 address"

aws ec2 import-key-pair --profile "${PROFILE}" --region "${REGION}" \
  --key-name "${KEY_NAME}" --public-key-material "fileb://${KEYFILE}.pub" >/dev/null
SECURITY_GROUP_ID="$(aws ec2 create-security-group --profile "${PROFILE}" \
  --region "${REGION}" --group-name "${KEY_NAME}" \
  --description 'ephemeral SPARQ access-control study' --vpc-id "${VPC}" \
  --query GroupId --output text)"
SSH_RULE_ID="$(aws ec2 authorize-security-group-ingress --profile "${PROFILE}" \
  --region "${REGION}" \
  --group-id "${SECURITY_GROUP_ID}" --protocol tcp --port 22 \
  --cidr "${PUBLIC_CIDR}" --query 'SecurityGroupRules[0].SecurityGroupRuleId' \
  --output text)"
[[ "${SSH_RULE_ID}" == sgr-* ]] || die "AWS did not return the SSH security-group rule ID"
CURRENT_SSH_CIDR="${PUBLIC_CIDR}"

cat >"${WORK}/user-data.sh" <<USERDATA
#!/bin/bash
set -euo pipefail
( sleep ${WATCHDOG_SECONDS}; shutdown -h now ) &
systemd-run --unit=sparq-ac-watchdog --on-active=${WATCHDOG_SECONDS} /sbin/shutdown -h now || true
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq git curl build-essential pkg-config libssl-dev python3 rsync
sudo -u ubuntu env HOME=/home/ubuntu bash -c \
  "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
touch /var/tmp/SPARQ_AC_BOOTSTRAP_DONE
USERDATA

log "launching ${INSTANCE_TYPE}, ${VOLUME_GB} GiB gp3, ${WATCHDOG_SECONDS}s watchdog"
INSTANCE_ID="$(aws ec2 run-instances --profile "${PROFILE}" --region "${REGION}" \
  --image-id "${AMI}" --instance-type "${INSTANCE_TYPE}" \
  --client-token "${RUN_TOKEN}" \
  --instance-initiated-shutdown-behavior terminate \
  --key-name "${KEY_NAME}" --security-group-ids "${SECURITY_GROUP_ID}" \
  --subnet-id "${SUBNET}" --associate-public-ip-address \
  --block-device-mappings \
    "[{\"DeviceName\":\"/dev/sda1\",\"Ebs\":{\"VolumeSize\":${VOLUME_GB},\"VolumeType\":\"gp3\",\"DeleteOnTermination\":true}}]" \
  --tag-specifications "${TAGSPEC}" --user-data "file://${WORK}/user-data.sh" \
  --query 'Instances[0].InstanceId' --output text)"
case "${INSTANCE_ID}" in
  "${PROD_INSTANCE}"|"${DEV_INSTANCE}") die "AWS returned a protected instance ID" ;;
  i-*) ;;
  *) INSTANCE_ID=""; die "run-instances did not return an instance ID" ;;
esac
printf '%s\n' "${INSTANCE_ID}" >"${RESULTS_LOCAL}/instance-id.txt"
printf '%s\n' "${SOURCE_COMMIT}" >"${RESULTS_LOCAL}/source-commit.txt"
printf '%s\n' "${AMI}" >"${RESULTS_LOCAL}/ami-id.txt"
printf '%s\n' "${RUN_TOKEN}" >"${RESULTS_LOCAL}/study-run-token.txt"

aws ec2 wait instance-running --profile "${PROFILE}" --region "${REGION}" \
  --instance-ids "${INSTANCE_ID}"
PUBLIC_IP="$(aws ec2 describe-instances --profile "${PROFILE}" --region "${REGION}" \
  --instance-ids "${INSTANCE_ID}" \
  --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)"
[[ "${PUBLIC_IP}" =~ ^[0-9A-Fa-f:.]+$ ]] || die "AWS returned an invalid public IP"
log "instance ${INSTANCE_ID} at ${PUBLIC_IP}; waiting for cloud-init"
for _ in $(seq 1 60); do
  if ssh "${SSH_OPTIONS[@]}" "ubuntu@${PUBLIC_IP}" \
    'test -f /var/tmp/SPARQ_AC_BOOTSTRAP_DONE' 2>/dev/null; then
    break
  fi
  refresh_ssh_ingress
  sleep 10
done
refresh_ssh_ingress
ssh_with_ingress_retry \
  'test -f /var/tmp/SPARQ_AC_BOOTSTRAP_DONE' \
  || die "bootstrap did not complete"

log "uploading source bundle and verifying exact commit"
scp_with_ingress_retry "${BUNDLE}" /var/tmp/sparq.bundle
# shellcheck disable=SC2029 # Validated commit expands locally into the remote assertion.
ssh_with_ingress_retry \
  "git clone -q /var/tmp/sparq.bundle /var/tmp/sparq-source \
   && git -C /var/tmp/sparq-source fetch -q origin \
        refs/remotes/origin/main:refs/remotes/origin/main \
   && test \"\$(git -C /var/tmp/sparq-source rev-parse HEAD)\" = '${SOURCE_COMMIT}'"

REMOTE_UID="$(ssh_with_ingress_retry 'id -u')"
REMOTE_GID="$(ssh_with_ingress_retry 'id -g')"
[[ "${REMOTE_UID}" =~ ^[0-9]+$ && "${REMOTE_GID}" =~ ^[0-9]+$ ]] \
  || die "remote uid/gid are not numeric"
log "starting ${MODE} as a 70%-memory-capped transient service"
# shellcheck disable=SC2029 # Validated numeric values intentionally expand client-side.
ssh_with_ingress_retry \
  "sudo systemd-run --unit=sparq-ac-study --collect \
    --property=MemoryMax=70% --property=KillMode=control-group \
    --uid=${REMOTE_UID} --gid=${REMOTE_GID} --working-directory=/var/tmp/sparq-source \
    --setenv=HOME=/home/ubuntu \
    --setenv=PATH=/home/ubuntu/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    --setenv=SPARQ_AC_CPUSET=${CPUSET} \
    /bin/bash /var/tmp/sparq-source/bench/ac/scaling/run-instance.sh ${MODE}"

START_EPOCH="$(date +%s)"
SUCCESS=0
while (( $(date +%s) - START_EPOCH < POLL_DEADLINE_SECONDS )); do
  sleep "${POLL_INTERVAL_SECONDS}"
  refresh_ssh_ingress
  stage_pull
  if ssh "${SSH_OPTIONS[@]}" "ubuntu@${PUBLIC_IP}" \
    'test -f /var/tmp/sparq-ac-study/DONE' 2>/dev/null; then
    SUCCESS=1
    log "completion sentinel received"
    break
  fi
  if ssh "${SSH_OPTIONS[@]}" "ubuntu@${PUBLIC_IP}" \
    'test -f /var/tmp/sparq-ac-study/FAILED' 2>/dev/null; then
    log "failure sentinel received"
    break
  fi
  UNIT_STATE="$(ssh "${SSH_OPTIONS[@]}" "ubuntu@${PUBLIC_IP}" \
    'systemctl is-active sparq-ac-study 2>/dev/null || true' 2>/dev/null || true)"
  CURRENT_STAGE="$(cat "${RESULTS_LOCAL}/stage.txt" 2>/dev/null || true)"
  log "unit=${UNIT_STATE:-unreachable}; stage=${CURRENT_STAGE:-bootstrapping}"
  case "${UNIT_STATE}" in
    active|activating|reloading) ;;
    failed|inactive|deactivating)
      log "study service stopped without a completion sentinel"
      break
      ;;
  esac
done

refresh_ssh_ingress
stage_pull
ssh "${SSH_OPTIONS[@]}" "ubuntu@${PUBLIC_IP}" \
  'sudo journalctl -u sparq-ac-study --no-pager' \
  >"${RESULTS_LOCAL}/systemd-journal.txt" 2>/dev/null || true
date -u +%FT%TZ >"${RESULTS_LOCAL}/retrieved-at.txt"
if [[ -f "${RESULTS_LOCAL}/MANIFEST.sha256" ]]; then
  (cd "${RESULTS_LOCAL}" && sha256sum -c MANIFEST.sha256)
fi
(( SUCCESS == 1 )) || die "the ${MODE} run did not complete; partial results were retained"
log "results verified at ${RESULTS_LOCAL}; cleanup will terminate ${INSTANCE_ID}"
