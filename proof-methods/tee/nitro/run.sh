#!/usr/bin/env bash
# Builds the enclave image, starts it in a Nitro Enclave, runs the holder
# driver over Q1-Q5 for 1 and 4 credentials, and stops the enclave.
#
# usage: proof-methods/tee/nitro/run.sh [reps] > tee-attestation.jsonl
#
# Needs an EC2 instance launched with Nitro Enclaves enabled (for example
# m7i.2xlarge), running Amazon Linux 2023, with a Rust toolchain; run it as
# root (nitro-cli and Docker need it). With SETUP=1 it first installs and
# starts nitro-cli and Docker and sizes the enclave allocator to 2 vCPUs and
# 2048 MiB.
#
# The image's PCR0 is printed to stderr and recorded in build.json; the
# holder accepts only that PCR0.
set -euo pipefail
REPS=${1:-21}
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
WORK=$(mktemp -d)
trap 'nitro-cli terminate-enclave --all >/dev/null 2>&1 || true; rm -rf "$WORK"' EXIT

if [ "${SETUP:-0}" = 1 ]; then
  dnf install -y aws-nitro-enclaves-cli aws-nitro-enclaves-cli-devel docker jq >&2
  printf -- "---\nmemory_mib: 2048\ncpu_count: 2\n" > /etc/nitro_enclaves/allocator.yaml
  systemctl enable --now nitro-enclaves-allocator.service docker >&2
fi

cargo build --locked --release --manifest-path "$ROOT/proof-methods/Cargo.toml" \
  -p sparq-vcq-tee --features enclave,host >&2
cp "$ROOT/proof-methods/target/release/sparq-vcq-tee-enclave" "$WORK/"
cp "$ROOT/proof-methods/tee/nitro/Dockerfile" "$WORK/"
docker build -q -t sparq-vcq-tee-enclave "$WORK" >&2
nitro-cli build-enclave --docker-uri sparq-vcq-tee-enclave --output-file "$WORK/enclave.eif" > "$WORK/build.json"
PCR0=$(jq -r .Measurements.PCR0 "$WORK/build.json")
echo "PCR0 $PCR0" >&2
cp "$WORK/build.json" "$ROOT/proof-methods/target/tee-build.json"

nitro-cli run-enclave --eif-path "$WORK/enclave.eif" --cpu-count 2 --memory 2048 > "$WORK/run.json"
CID=$(jq -r .EnclaveCID "$WORK/run.json")
"$ROOT/proof-methods/target/release/sparq-vcq-tee-holder" "$CID" "$PCR0" "$REPS"
