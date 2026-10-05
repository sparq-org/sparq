#!/usr/bin/env bash
# [OPUS-5] sq-ffaa9 — ONE-TIME MAINTAINER bootstrap of the sparq-bench result bucket and
# the IAM instance profile that lets a self-terminating gather box write to it.
#
# 🤖 SPARQ agent. This is the [MAINTAINER credential/access] half of sq-ffaa9: it needs
# S3 + IAM permissions the bench role does not hold (the 2026-07-10 x86_64 attempt failed
# precisely because `AWSReservedSSO_PSSSingleInstanceDeploy_*` may not create buckets,
# put objects, or attach an instance profile — research/gap-vector-2026-07.md). Run it ONCE
# with credentials that can, then export the two knobs it prints; from then on every
# canonical launcher in scripts/bench/ can carry results off the box durably instead of
# relying on `aws ec2 get-console-output`, which returns nothing usable on AL2023/Nitro.
#
# It is idempotent (re-running an already-bootstrapped account is a no-op) and it grants
# the narrowest useful permission set: the role may PUT objects under ONE prefix of ONE
# bucket and nothing else — no read, no delete, no list, no other bucket.
#
# Usage:
#   scripts/bench/bootstrap-bench-iam.sh --print-policy            # JSON only, no AWS call
#   scripts/bench/bootstrap-bench-iam.sh --dry-run                 # print the exact calls
#   scripts/bench/bootstrap-bench-iam.sh                           # create for real
#   scripts/bench/bootstrap-bench-iam.sh --reader-arn arn:aws:iam::123:role/Launcher
#
# Options: --bucket NAME --prefix KEY --role NAME --profile-name NAME --region R
#          --expire-days N (0 disables the lifecycle rule) --reader-arn ARN
#
# Pinned by scripts/tests/test_bench_result_egress.sh (policy shape + least privilege +
# "--dry-run makes no AWS call, with or without --bucket" + first-run/re-run/conflicting-role
# behaviour of the instance profile). The AWS-side calls themselves are DOCUMENTED-UNTESTED
# here: this repo's CI has no AWS account, so validate them on the first real run.
set -euo pipefail

REGION="${REGION:-${AWS_REGION:-eu-west-2}}"
BUCKET="${BENCH_RESULTS_BUCKET:-}"
PREFIX="${BENCH_RESULTS_PREFIX:-gathers}"
ROLE_NAME="${BENCH_IAM_ROLE:-sparq-bench-results-writer}"
PROFILE_NAME="${BENCH_IAM_PROFILE:-sparq-bench-results}"
EXPIRE_DAYS="${BENCH_RESULTS_EXPIRE_DAYS:-90}"
READER_ARN=""
DRY_RUN=0
PRINT_POLICY_ONLY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --bucket) BUCKET="${2:?--bucket needs a value}"; shift 2 ;;
    --prefix) PREFIX="${2:?--prefix needs a value}"; shift 2 ;;
    --role) ROLE_NAME="${2:?--role needs a value}"; shift 2 ;;
    --profile-name) PROFILE_NAME="${2:?--profile-name needs a value}"; shift 2 ;;
    --region) REGION="${2:?--region needs a value}"; shift 2 ;;
    --expire-days) EXPIRE_DAYS="${2:?--expire-days needs a value}"; shift 2 ;;
    --reader-arn) READER_ARN="${2:?--reader-arn needs a value}"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    --print-policy) PRINT_POLICY_ONLY=1; DRY_RUN=1; shift ;;
    -h|--help) sed -n '1,30p' "$0"; exit 0 ;;
    *) echo "bootstrap-bench-iam: unknown argument '$1'" >&2; exit 2 ;;
  esac
done

log() { printf '[bootstrap-bench-iam] %s\n' "$*" >&2; }
die() { printf '[bootstrap-bench-iam] ERROR: %s\n' "$*" >&2; exit 1; }

PREFIX="${PREFIX#/}"; PREFIX="${PREFIX%/}"
[ -n "$PREFIX" ] || die "--prefix must not be empty (the write grant is scoped to it)"
[[ "$PREFIX" =~ ^[A-Za-z0-9][A-Za-z0-9._/-]*$ ]] || die "--prefix '$PREFIX' is not a safe S3 key prefix"

if [ -z "$BUCKET" ]; then
  if [ "$DRY_RUN" = 1 ]; then
    # NEITHER --dry-run NOR --print-policy may touch AWS, and deriving the default bucket
    # name needs the account id — so the dry run PRINTS the lookup a real run would make
    # and then previews the rest of the plan against an obvious placeholder account.
    if [ "$PRINT_POLICY_ONLY" != 1 ]; then
      printf '  aws sts get-caller-identity --query Account --output text   # => ACCOUNT\n'
      log "no --bucket given: previewing against the placeholder account 000000000000"
    fi
    BUCKET="sparq-bench-results-000000000000"
  else
    command -v aws >/dev/null || die "aws CLI not found (needed to resolve the account id)"
    ACCOUNT=$(aws sts get-caller-identity --query Account --output text) \
      || die "could not resolve the account id — pass --bucket explicitly"
    BUCKET="sparq-bench-results-${ACCOUNT}"
  fi
fi
[[ "$BUCKET" =~ ^[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]$ ]] || die "--bucket '$BUCKET' is not a valid S3 bucket name"

# ---- the three policy documents -------------------------------------------------------
# WRITE-ONLY AND PREFIX-SCOPED. The gather box gets s3:PutObject (+ the multipart abort a
# retried upload needs) under arn:aws:s3:::<bucket>/<prefix>/* and NOTHING else — no
# GetObject, no DeleteObject, no ListBucket, no second bucket, no iam:*. A compromised or
# buggy bench box can therefore only ADD objects under its own results prefix.
TRUST_POLICY=$(cat <<'JSON'
{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"ec2.amazonaws.com"},"Action":"sts:AssumeRole"}]}
JSON
)
PERMISSIONS_POLICY=$(cat <<JSON
{"Version":"2012-10-17","Statement":[{"Sid":"WriteBenchResultsOnly","Effect":"Allow","Action":["s3:PutObject","s3:AbortMultipartUpload"],"Resource":"arn:aws:s3:::${BUCKET}/${PREFIX}/*"}]}
JSON
)
READER_POLICY=""
if [ -n "$READER_ARN" ] || [ "$PRINT_POLICY_ONLY" = 1 ]; then
  READER_POLICY=$(cat <<JSON
{"Version":"2012-10-17","Statement":[{"Sid":"SparqBenchReaderObjects","Effect":"Allow","Principal":{"AWS":"${READER_ARN:-arn:aws:iam::ACCOUNT:role/READER}"},"Action":["s3:GetObject"],"Resource":"arn:aws:s3:::${BUCKET}/${PREFIX}/*"},{"Sid":"SparqBenchReaderList","Effect":"Allow","Principal":{"AWS":"${READER_ARN:-arn:aws:iam::ACCOUNT:role/READER}"},"Action":["s3:ListBucket"],"Resource":"arn:aws:s3:::${BUCKET}","Condition":{"StringLike":{"s3:prefix":"${PREFIX}/*"}}}]}
JSON
)
fi

if [ "$PRINT_POLICY_ONLY" = 1 ]; then
  printf '{"bucket":"%s","prefix":"%s","role":"%s","instance_profile":"%s","trust":%s,"permissions":%s,"reader":%s}\n' \
    "$BUCKET" "$PREFIX" "$ROLE_NAME" "$PROFILE_NAME" "$TRUST_POLICY" "$PERMISSIONS_POLICY" "$READER_POLICY"
  exit 0
fi

# ---- execution -------------------------------------------------------------------------
run_aws() {
  if [ "$DRY_RUN" = 1 ]; then
    printf '  aws'; printf ' %q' "$@"; printf '\n'
    return 0
  fi
  aws "$@"
}
# Creation calls tolerate "already exists" so a re-run is a no-op.
run_aws_idempotent() {
  if [ "$DRY_RUN" = 1 ]; then run_aws "$@"; return 0; fi
  local err rc=0
  err=$(aws "$@" 2>&1) || rc=$?
  if [ "$rc" -ne 0 ]; then
    case "$err" in
      *AlreadyExists*|*AlreadyOwnedByYou*|*EntityAlreadyExists*|*already\ exists*)
        log "already present (ok): ${1:-} ${2:-}" ;;
      *) printf '%s\n' "$err" >&2; return "$rc" ;;
    esac
  fi
  return 0
}

[ "$DRY_RUN" = 1 ] || command -v aws >/dev/null || die "aws CLI not found"
log "region=$REGION bucket=$BUCKET prefix=$PREFIX role=$ROLE_NAME profile=$PROFILE_NAME expire_days=$EXPIRE_DAYS dry_run=$DRY_RUN"

log "1/6 create the results bucket (private)"
if [ "$REGION" = "us-east-1" ]; then
  run_aws_idempotent s3api create-bucket --bucket "$BUCKET" --region "$REGION"
else
  run_aws_idempotent s3api create-bucket --bucket "$BUCKET" --region "$REGION" \
    --create-bucket-configuration "LocationConstraint=$REGION"
fi
run_aws s3api put-public-access-block --bucket "$BUCKET" \
  --public-access-block-configuration BlockPublicAcls=true,IgnorePublicAcls=true,BlockPublicPolicy=true,RestrictPublicBuckets=true

if [ "$EXPIRE_DAYS" != "0" ]; then
  log "2/6 lifecycle: expire $PREFIX/ after $EXPIRE_DAYS days (bounds storage cost)"
  run_aws s3api put-bucket-lifecycle-configuration --bucket "$BUCKET" \
    --lifecycle-configuration "{\"Rules\":[{\"ID\":\"sparq-bench-results-expiry\",\"Status\":\"Enabled\",\"Filter\":{\"Prefix\":\"${PREFIX}/\"},\"Expiration\":{\"Days\":${EXPIRE_DAYS}}}]}"
else
  log "2/6 lifecycle skipped (--expire-days 0)"
fi

log "3/6 IAM role $ROLE_NAME (EC2 trust)"
run_aws_idempotent iam create-role --role-name "$ROLE_NAME" \
  --assume-role-policy-document "$TRUST_POLICY" \
  --description "sparq-bench gather boxes: write result envelopes to s3://$BUCKET/$PREFIX/ (sq-ffaa9)"

log "4/6 inline write-only policy scoped to s3://$BUCKET/$PREFIX/"
run_aws iam put-role-policy --role-name "$ROLE_NAME" \
  --policy-name sparq-bench-results-write --policy-document "$PERMISSIONS_POLICY"

# An instance profile holds AT MOST ONE role, and AWS reports a re-attach as a
# LimitExceeded quota error rather than any AlreadyExists string — so the generic allowlist
# in run_aws_idempotent cannot make this call idempotent. Ask first instead: the same role
# is a no-op, a DIFFERENT role is a real conflict the maintainer must resolve explicitly.
attach_role_to_profile() {
  if [ "$DRY_RUN" = 1 ]; then
    run_aws iam get-instance-profile --instance-profile-name "$PROFILE_NAME"
    run_aws iam add-role-to-instance-profile \
      --instance-profile-name "$PROFILE_NAME" --role-name "$ROLE_NAME"
    return 0
  fi
  local attached
  attached=$(aws iam get-instance-profile --instance-profile-name "$PROFILE_NAME" \
    --query 'InstanceProfile.Roles[0].RoleName' --output text 2>/dev/null) || attached=""
  case "$attached" in
    "$ROLE_NAME") log "role $ROLE_NAME is already attached to $PROFILE_NAME (ok)"; return 0 ;;
    ""|None) ;;
    *) die "instance profile $PROFILE_NAME already holds role '$attached' (a profile holds at most one role) — detach it, or re-run with --profile-name to use a different profile" ;;
  esac
  aws iam add-role-to-instance-profile \
    --instance-profile-name "$PROFILE_NAME" --role-name "$ROLE_NAME"
}

log "5/6 instance profile $PROFILE_NAME"
run_aws_idempotent iam create-instance-profile --instance-profile-name "$PROFILE_NAME"
attach_role_to_profile

if [ -n "$READER_ARN" ]; then
  log "6/6 bucket policy granting read-back to $READER_ARN"
  run_aws s3api put-bucket-policy --bucket "$BUCKET" --policy "$READER_POLICY"
else
  log "6/6 no --reader-arn given — the launcher can only read back if its own role already"
  log "    has s3:GetObject on this bucket; otherwise retrieve with maintainer credentials"
fi

cat <<EXPORTS
[bootstrap-bench-iam] done. Export these before a canonical gather:

  export BENCH_IAM_PROFILE=$PROFILE_NAME
  export BENCH_RESULTS_S3=s3://$BUCKET/$PREFIX

Then run any canonical launcher as usual, e.g.
  AWS_PROFILE=pss scripts/bench/canonical-materialize-bench.sh main
Envelopes are uploaded to s3://$BUCKET/$PREFIX/<run-id>/ as they are produced and synced
back into RESULTS_LOCAL/s3/ at the end of the run. See scripts/bench/README.md.
EXPORTS
