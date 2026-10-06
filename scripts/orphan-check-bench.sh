#!/usr/bin/env bash
# Orchestration automation — Phase C of research/orchestration-automation-design.md
# (PR #374).
#
# orphan-check-bench.sh [--apply] [--region <r>]
#
# Find — and, with --apply, terminate — ORPHANED benchmark EC2 instances: instances that
# carry the EXACT tag purpose=sparq-bench and are still running/pending after their bench
# should have self-terminated (the gather launchers set
# --instance-initiated-shutdown-behavior=terminate + a 3h watchdog, but an agent death or a
# stuck gather can leave a box alive and burning ~$5/day — MEMORY: feedback-ec2-benchmarks).
# It also greps the local process table for in-flight `gather-*` launchers so a fresh
# session can see what is (or isn't) still driving a box.
#
# SAFETY (design §6, "Orphan-check terminates a PROD/DEV box" — Critical/irreversible):
#   * DEFAULT --dry-run: prints the orphans it WOULD terminate and exits 0 WITHOUT calling
#     terminate-instances. Actually terminating requires the explicit --apply flag.
#   * ALLOW-LIST (not deny-list) semantics: an instance is a termination candidate ONLY IF
#     it carries the EXACT tag purpose=sparq-bench. We never terminate by "everything except
#     the excludes"; the filter is positive.
#   * HARD exclusion list on top of that: the prod box (i-090531b4ede8f2d3f) and the dev box
#     (i-00f76802f345b6b77) are NEVER terminated even if (mis-)tagged. They are removed from
#     the kill set unconditionally, and the self-test asserts they can never be in it.
#   * The EC2 query itself filters on tag:purpose=sparq-bench AND state in {running,pending}
#     server-side, so the prod/dev boxes never even enter the candidate set under normal
#     conditions; the hard exclusion is the belt-and-braces second line.
#
# AWS auth: this account's credentials live under the `pss` named profile (region eu-west-2).
# AWS_PROFILE defaults to pss when unset and is exported + passed to every aws call, so the
# EC2 query actually authenticates instead of silently no-op'ing (dogfooding #376 / sq-b1fo).
# Override by exporting AWS_PROFILE=<other> (or AWS_REGION) before invoking.
#
# Run:
#   scripts/orphan-check-bench.sh                       # dry-run (default): list orphans
#   scripts/orphan-check-bench.sh --apply               # terminate tag-matched orphans
#   scripts/orphan-check-bench.sh --region us-east-1    # override region
#   AWS_PROFILE=other scripts/orphan-check-bench.sh     # override the pss profile
#   scripts/orphan-check-bench.sh --dry-run-self-test   # hermetic self-test (no aws)
set -euo pipefail

PROG="orphan-check-bench"
log()  { printf '[%s] %s\n' "$PROG" "$*" >&2; }
die()  { printf '[%s] ERROR: %s\n' "$PROG" "$*" >&2; exit 1; }

# AWS credentials live under the `pss` named profile on this account (the EC2 boxes are in
# eu-west-2). Without this, every aws call here falls back to the default profile, finds no
# usable credentials, and the EC2 query silently no-ops — so orphan bench boxes go undetected
# (dogfooding finding #376, bead sq-b1fo). Default the profile to pss when unset and export it
# so it is honoured by BOTH the describe-instances query and the --apply terminate call below.
: "${AWS_PROFILE:=pss}"
export AWS_PROFILE

# The EXACT tag that marks a disposable bench box (allow-list key). Matches the gather
# launchers' TAGSPEC (scripts/gather-ec2*.sh: Tags=[{Key=purpose,Value=sparq-bench}]).
readonly BENCH_TAG_KEY="purpose"
readonly BENCH_TAG_VALUE="sparq-bench"

# HARD never-touch instance ids (prod + dev). Editing this list is the ONLY way to change
# what is protected; nothing else can add to or bypass it.
readonly PROD_INSTANCE="i-090531b4ede8f2d3f"
readonly DEV_INSTANCE="i-00f76802f345b6b77"
readonly -a NEVER_TOUCH=("$PROD_INSTANCE" "$DEV_INSTANCE")

# Drop any hard-excluded id from a newline-separated id list on stdin. Pure-text + testable.
drop_excluded() {
  local ids; ids="$(cat)"
  local id keep
  while IFS= read -r id; do
    [ -n "$id" ] || continue
    keep=1
    for ex in "${NEVER_TOUCH[@]}"; do
      [ "$id" = "$ex" ] && { keep=0; break; }
    done
    [ "$keep" -eq 1 ] && printf '%s\n' "$id"
  done <<< "$ids"
  return 0
}

# --- the --dry-run self-test (no aws; asserts the exclusion + allow-list invariants) ---
self_test() {
  local fails=0 got
  _check() {
    if [ "$2" = "$3" ]; then printf '  ok   %s\n' "$1"
    else printf '  FAIL %s\n       got:  %q\n       want: %q\n' "$1" "$2" "$3"; fails=$((fails + 1)); fi
  }
  log "running --dry-run self-test (hermetic; no aws calls)"

  # The load-bearing invariant: prod/dev ids are NEVER in the kill set, even if a bench-
  # tagged query (wrongly) returned them.
  got="$(printf '%s\n%s\n%s\n' "i-aaaa111122223333a" "$PROD_INSTANCE" "$DEV_INSTANCE" | drop_excluded)"
  _check "prod+dev dropped, bench kept"            "$got" "i-aaaa111122223333a"

  got="$(printf '%s\n%s\n' "$PROD_INSTANCE" "$DEV_INSTANCE" | drop_excluded)"
  _check "only prod+dev => empty kill set"         "$got" ""

  got="$(printf 'i-orphan1\ni-orphan2\n' | drop_excluded)"
  _check "two genuine orphans both kept"           "$got" "$(printf 'i-orphan1\ni-orphan2')"

  got="$(printf '\n\n' | drop_excluded)"
  _check "blank input => empty"                    "$got" ""

  # Allow-list key shape (the server-side filter we will pass to aws).
  _check "tag filter key"                          "$BENCH_TAG_KEY"   "purpose"
  _check "tag filter value"                        "$BENCH_TAG_VALUE" "sparq-bench"

  # Defensive: the hard list contains exactly prod + dev.
  _check "never-touch count is 2"                  "${#NEVER_TOUCH[@]}" "2"

  echo
  if [ "$fails" -eq 0 ]; then log "self-test PASSED"; return 0; fi
  die "self-test FAILED ($fails check(s))"
}

# --- arg parsing -----------------------------------------------------------------------
APPLY=0
REGION="${AWS_REGION:-eu-west-2}"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run-self-test) self_test; exit 0 ;;
    --apply)             APPLY=1 ;;
    --dry-run)           APPLY=0 ;;
    --region)            shift; [ "$#" -gt 0 ] || die "--region needs a value"; REGION="$1" ;;
    --region=*)          REGION="${1#--region=}" ;;
    -h|--help)           sed -n '2,40p' "$0"; exit 0 ;;
    *)                   die "unknown argument: $1 (try --apply, --region, --dry-run-self-test, --help)" ;;
  esac
  shift
done

# --- 1. grep the local process table for in-flight gather launchers (advisory) ---------
# (SC2009: pgrep can't print etime+args in one shot, and we want both for the operator's
#  briefing, so an explicit ps|grep is intended here. The grep -v grep drops the matcher.)
log "in-flight gather launchers on THIS host (advisory):"
# shellcheck disable=SC2009
if procs="$(ps -eo pid,etime,args 2>/dev/null | grep -E 'gather-(ec2|competitors|ec2-sparql)\.sh' | grep -v grep)"; then
  printf '%s\n' "$procs" | sed 's/^/  /' >&2
else
  log "  (none)"
fi

# --- 2. query EC2 for running/pending instances with the EXACT bench tag ---------------
command -v aws >/dev/null 2>&1 || {
  log "aws CLI not found — process-table scan done; EC2 query skipped (graceful no-op)."
  exit 0
}

log "querying EC2 (profile=$AWS_PROFILE region=$REGION) for state in {running,pending} AND tag:$BENCH_TAG_KEY=$BENCH_TAG_VALUE"
# Allow-list filter: ONLY instances carrying the exact tag are even returned.
# We capture stderr to a temp file (instead of discarding it) so a credentials/token failure
# surfaces a CLEAR message rather than silently no-op'ing and leaving real orphans undetected.
aws_err="$(mktemp)"
trap 'rm -f "$aws_err"' EXIT
if ! INSTANCES_RAW="$(aws ec2 describe-instances \
  --profile "$AWS_PROFILE" \
  --region "$REGION" \
  --filters "Name=tag:${BENCH_TAG_KEY},Values=${BENCH_TAG_VALUE}" \
            "Name=instance-state-name,Values=running,pending" \
  --query 'Reservations[].Instances[].InstanceId' \
  --output text 2>"$aws_err" | tr '\t' '\n' | sed '/^$/d')"; then
  log "aws describe-instances failed (profile=$AWS_PROFILE region=$REGION) — no action taken. AWS said:"
  sed 's/^/  aws: /' "$aws_err" >&2 || true
  exit 1
fi

mapfile -t INSTANCES <<< "$INSTANCES_RAW"
# Drop the trailing empty element a here-string leaves when the input is empty.
[ "${#INSTANCES[@]}" -eq 1 ] && [ -z "${INSTANCES[0]}" ] && INSTANCES=()

if [ "${#INSTANCES[@]}" -eq 0 ]; then
  log "no running/pending sparq-bench instances — clean (no orphans)."
  exit 0
fi

log "tag-matched running/pending instances:"
printf '  %s\n' "${INSTANCES[@]}" >&2

# Belt-and-braces: drop the hard never-touch ids before they can ever reach terminate.
mapfile -t KILL_SET < <(printf '%s\n' "${INSTANCES[@]}" | drop_excluded)

# If anything was filtered, say so loudly (a prod/dev box wrongly carrying the bench tag is
# itself a finding worth surfacing — but we still never terminate it).
for ex in "${NEVER_TOUCH[@]}"; do
  if printf '%s\n' "${INSTANCES[@]}" | grep -qxF "$ex"; then
    log "WARNING: protected instance $ex carries the bench tag but is HARD-EXCLUDED — NOT terminating it."
  fi
done

if [ "${#KILL_SET[@]}" -eq 0 ]; then
  log "after hard exclusion, no terminable orphans remain."
  exit 0
fi

log "ORPHAN candidates (tag-matched, prod/dev excluded):"
printf '  %s\n' "${KILL_SET[@]}" >&2

if [ "$APPLY" -eq 0 ]; then
  log "dry-run (default): nothing terminated. Re-run with --apply to terminate the above."
  exit 0
fi

# --- 3. --apply: terminate ONLY the kill set (tag-matched, prod/dev already removed) ---
log "--apply: terminating orphan(s): ${KILL_SET[*]}"
if aws ec2 terminate-instances --profile "$AWS_PROFILE" --region "$REGION" --instance-ids "${KILL_SET[@]}" >/dev/null 2>"$aws_err"; then
  log "terminate-instances submitted for: ${KILL_SET[*]}"
else
  log "terminate-instances FAILED (profile=$AWS_PROFILE region=$REGION). AWS said:"
  sed 's/^/  aws: /' "$aws_err" >&2 || true
  die "terminate-instances FAILED for: ${KILL_SET[*]}"
fi
