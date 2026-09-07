#!/usr/bin/env bash
# [GPT-6] Ready-only disposable native evaluation host; no campaign auto-run.
set -euo pipefail
exec bash "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/launch-ec2.sh" native
