#!/usr/bin/env bash
# Independently enforce the disposable study host's deadline. [GPT-6]
set -euo pipefail

RESULTS="${1:?usage: supervise-instance.sh RESULTS_DIRECTORY}"
PROFILE="${AWS_PROFILE:-pss}"
REGION="${AWS_REGION:-eu-west-2}"
[[ "${REGION}" == eu-west-2 ]] || exit 2
for filename in instance-id.txt study-run-token.txt price-checked-at.txt; do
  [[ -r "${RESULTS}/${filename}" ]] || { echo "missing ${filename}" >&2; exit 2; }
done
INSTANCE_ID="$(<"${RESULTS}/instance-id.txt")"
RUN_TOKEN="$(<"${RESULTS}/study-run-token.txt")"
[[ "${INSTANCE_ID}" =~ ^i-[0-9a-f]+$ ]] || exit 2
[[ "${RUN_TOKEN}" =~ ^sparq-pod-(pilot|canonical)-[0-9TZ]+-[0-9]+$ ]] || exit 2
case "${INSTANCE_ID}" in
  i-090531b4ede8f2d3f|i-00f76802f345b6b77) echo 'protected instance' >&2; exit 2 ;;
esac
DEADLINE="$(python3 - "${RESULTS}/price-checked-at.txt" <<'PY'
import datetime,sys
from pathlib import Path
started=datetime.datetime.fromisoformat(Path(sys.argv[1]).read_text().strip().replace('Z','+00:00'))
print(int(started.timestamp())+43200)
PY
)"
[[ "${DEADLINE}" =~ ^[0-9]+$ ]] || exit 2
printf '%s supervisor-start %s deadline=%s\n' "$(date -u +%FT%TZ)" "${INSTANCE_ID}" "${DEADLINE}"
while true; do
  # The exact study token, positive benchmark tag and protected-ID exclusion all
  # apply again immediately before termination. Missing tags fail closed.
  if MATCH="$(aws ec2 describe-instances --profile "${PROFILE}" --region "${REGION}" \
    --instance-ids "${INSTANCE_ID}" \
    --filters 'Name=tag:purpose,Values=sparq-bench' "Name=tag:study-run,Values=${RUN_TOKEN}" \
    --query 'Reservations[].Instances[].State.Name' --output text)"; then
    case "${MATCH}" in
      terminated|shutting-down) echo 'supervisor-complete'; exit 0 ;;
      pending|running|stopping|stopped) ;;
      *) echo 'supervisor-refused: exact tags/state unavailable' >&2; exit 1 ;;
    esac
    if (( $(date +%s) >= DEADLINE )); then
      aws ec2 terminate-instances --profile "${PROFILE}" --region "${REGION}" \
        --instance-ids "${INSTANCE_ID}" --query 'TerminatingInstances[].CurrentState.Name' --output text
      echo 'supervisor-deadline-termination-requested'
      exit 0
    fi
    printf '%s supervisor-active %s\n' "$(date -u +%FT%TZ)" "${MATCH}"
  else
    echo 'supervisor-read-failed: retaining deadline and retrying' >&2
  fi
  sleep 45
done
