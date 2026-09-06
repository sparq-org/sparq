#!/usr/bin/env bash
# [GPT-6] Dedicated build/test host; never starts a benchmark campaign.
set -euo pipefail
exec bash "$(dirname "$0")/launch-ec2.sh" build
