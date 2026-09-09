#!/usr/bin/env bash
# EC2-deferred heavy benchmark (roadmap G3). Invoked by .github/workflows/bench-ec2.yml AFTER its
# OIDC step has configured short-lived AWS credentials. Launches ONE spot instance, runs the
# benchmark at a larger scale than free GitHub runners allow, fetches the results JSON, and ALWAYS
# tears everything down. Mirrors the (manually proven) hardware-campaign recipe. Cost guards: spot
# pricing, an always-run cleanup trap, and the weekly/manual-only + timeout limits in the workflow.
#
# Writes ec2-bench-results.json (github-action-benchmark customSmallerIsBetter) to the CWD.
# Requires: aws cli (auth already configured), ssh. No long-lived secrets — an ephemeral keypair is
# generated per run and deleted in cleanup.
set -euo pipefail

REGION="${AWS_REGION:-eu-west-2}"
ITYPE="${BENCH_INSTANCE_TYPE:-c7g.4xlarge}"     # arm64 spot; 16 vCPU, far more than a GH runner
SCALE="${BENCH_SCALE:-2000000}"                 # ~10x the in-CI scale
SHA="${GITHUB_SHA:-$(git rev-parse HEAD)}"
REPO="https://github.com/sparq-org/sparq.git"
TAGSPEC='ResourceType=instance,Tags=[{Key=purpose,Value=sparq-ci-bench}]'
# [FABLE-5] (sq-6vshe.6, MAINTAINER-DIRECTED) OUTPUT + FULL-SUITE knobs so the nightly lane can reuse
# this SAME script (single source of truth) instead of a forked copy:
#   BENCH_OUT         — results filename written to the CWD (default ec2-bench-results.json, weekly heavy).
#   BENCH_FULL_SUITE  — when set (=1), the instance installs the well-known-suite toolchains
#                       (g++/Boost/JRE/JDK/rapper/unzip/binaryen) AND runs ci-bench.sh with
#                       GITHUB_REF=refs/heads/main, so EVERY latency suite (sp2b/dbpsb/watdiv/bsbm/lubm
#                       + the cargo-only fts/geo/vector/rsp/hdt/solid/nlq/zk main-tier hooks) runs — the
#                       full NOISY timing series that was RELOCATED off the per-PR / merge_group bench
#                       lane. Empty (weekly heavy default) = today's toolchain-light run, unchanged.
BENCH_OUT="${BENCH_OUT:-ec2-bench-results.json}"
BENCH_FULL_SUITE="${BENCH_FULL_SUITE:-}"

WORK="$(mktemp -d)"
KEYFILE="$WORK/key"
KEY_NAME="sparq-ci-$$-${RANDOM}"
INSTANCE_ID=""; SG_ID=""
ssh-keygen -t ed25519 -N '' -f "$KEYFILE" -q

cleanup() {
  set +e
  echo "::group::cleanup"
  [ -n "$INSTANCE_ID" ] && aws ec2 terminate-instances --region "$REGION" --instance-ids "$INSTANCE_ID" >/dev/null 2>&1
  [ -n "$INSTANCE_ID" ] && aws ec2 wait instance-terminated --region "$REGION" --instance-ids "$INSTANCE_ID" >/dev/null 2>&1
  [ -n "$SG_ID" ] && aws ec2 delete-security-group --region "$REGION" --group-id "$SG_ID" >/dev/null 2>&1
  aws ec2 delete-key-pair --region "$REGION" --key-name "$KEY_NAME" >/dev/null 2>&1
  echo "cleanup done"; echo "::endgroup::"
}
trap cleanup EXIT   # ALWAYS terminate — even on error/cancel — so a run never leaks a running instance

SSHO="-i $KEYFILE -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=15"

echo "== resolve AMI / network =="
AMI=$(aws ec2 describe-images --region "$REGION" --owners 099720109477 \
  --filters "Name=name,Values=ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-arm64-server-*" "Name=state,Values=available" \
  --query 'sort_by(Images,&CreationDate)[-1].ImageId' --output text)
VPC=$(aws ec2 describe-vpcs --region "$REGION" --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)
SUBNET=$(aws ec2 describe-subnets --region "$REGION" --filters Name=vpc-id,Values="$VPC" "Name=default-for-az,Values=true" --query 'Subnets[0].SubnetId' --output text)
MYIP=$(curl -s https://checkip.amazonaws.com)

echo "== keypair + security group =="
aws ec2 import-key-pair --region "$REGION" --key-name "$KEY_NAME" --public-key-material "fileb://${KEYFILE}.pub" >/dev/null
SG_ID=$(aws ec2 create-security-group --region "$REGION" --group-name "$KEY_NAME" --description "sparq ci bench (ephemeral)" --vpc-id "$VPC" --query 'GroupId' --output text)
aws ec2 authorize-security-group-ingress --region "$REGION" --group-id "$SG_ID" --protocol tcp --port 22 --cidr "${MYIP}/32" >/dev/null

echo "== launch spot $ITYPE =="
INSTANCE_ID=$(aws ec2 run-instances --region "$REGION" --image-id "$AMI" --instance-type "$ITYPE" \
  --key-name "$KEY_NAME" --security-group-ids "$SG_ID" --subnet-id "$SUBNET" --associate-public-ip-address \
  --instance-market-options 'MarketType=spot' \
  --block-device-mappings '[{"DeviceName":"/dev/sda1","Ebs":{"VolumeSize":40,"VolumeType":"gp3","DeleteOnTermination":true}}]' \
  --tag-specifications "$TAGSPEC" --query 'Instances[0].InstanceId' --output text)
aws ec2 wait instance-running --region "$REGION" --instance-ids "$INSTANCE_ID"
IP=$(aws ec2 describe-instances --region "$REGION" --instance-ids "$INSTANCE_ID" --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)

echo "== wait for sshd =="
for _ in $(seq 1 30); do ssh $SSHO "ubuntu@$IP" true 2>/dev/null && break; sleep 10; done

echo "== build + bench on the instance (public repo: clone with no creds) =="
# The instance clones the public repo at this SHA, installs Rust, builds, runs the same emitter at
# a larger scale, and prints ONLY the results JSON on stdout (captured below).
#
# [FABLE-5] (sq-6vshe.6, MAINTAINER-DIRECTED) FULL-SUITE mode: when BENCH_FULL_SUITE is set, ALSO
# install the well-known-suite toolchains + the wasm target and run ci-bench.sh with
# GITHUB_REF=refs/heads/main so the instance runs EVERY latency suite (the noisy timing series moved
# off the per-PR / merge_group bench lane). These commands mirror bench.yml's "Install well-known-suite
# toolchains" + the wasm target the per-commit lane sets. In the default (weekly heavy) mode the block
# is a no-op — today's toolchain-light run is unchanged.
if [ -n "$BENCH_FULL_SUITE" ]; then
  SUITE_APT='sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq libboost-all-dev default-jdk raptor2-utils unzip binaryen >/dev/null 2>&1'
  SUITE_WASM='rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true'
  SUITE_REF='export GITHUB_REF=refs/heads/main'
else
  SUITE_APT='true'
  SUITE_WASM='true'
  SUITE_REF='true'
fi
if [ -n "$BENCH_FULL_SUITE" ]; then
  SP2B_EC2_ENV='export SP2B_EC2=1'
else
  SP2B_EC2_ENV='true'
fi
REMOTE=$(cat <<REMOTE_EOF
set -e
sudo apt-get update -qq && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq build-essential pkg-config git >/dev/null 2>&1
$SUITE_APT
command -v cargo >/dev/null || { curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null 2>&1; }
. "\$HOME/.cargo/env"
$SUITE_WASM
git clone -q "$REPO" sparq && cd sparq && git checkout -q "$SHA"
cargo build --release -q -p sparq-cli -p sparq-bench
$SUITE_REF
$SP2B_EC2_ENV
bash scripts/ci-bench.sh "$SCALE" /tmp/out.json >/dev/null 2>&1
cat /tmp/out.json
REMOTE_EOF
)
ssh $SSHO "ubuntu@$IP" "$REMOTE" > "$BENCH_OUT"
echo "== results ($BENCH_OUT) =="; cat "$BENCH_OUT"
python3 -c "import json,sys; json.load(open('$BENCH_OUT')); print('valid results JSON')"
