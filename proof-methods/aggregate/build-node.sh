#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 1 || -e "$1" || -L "$1" ]]; then
  echo 'usage: build-node.sh <new-output-directory>' >&2
  exit 1
fi
crate_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
version=$(python3 - "$crate_dir/Cargo.toml" <<'PY'
import sys, tomllib
with open(sys.argv[1], 'rb') as source:
    print(tomllib.load(source)['dependencies']['wasm-bindgen']['version'].removeprefix('='))
PY
)
if [[ $(wasm-bindgen --version) != "wasm-bindgen $version" ]]; then
  echo "Install the pinned CLI: cargo install wasm-bindgen-cli --version $version --locked" >&2
  exit 1
fi
cargo build --locked --manifest-path "$crate_dir/../Cargo.toml" \
  -p sparq-verifiable-aggregate --release --target wasm32-unknown-unknown --features wasm
target_dir=$(cargo metadata --manifest-path "$crate_dir/../Cargo.toml" --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
mkdir -- "$1"
wasm-bindgen "$target_dir/wasm32-unknown-unknown/release/sparq_verifiable_aggregate.wasm" \
  --target nodejs --out-dir "$1" --out-name aggregate
cp -- "$crate_dir/wire.d.ts" "$1/wire.d.ts"
cp -- "$crate_dir/README.md" "$1/README.md"
cp -- "$crate_dir/../../LICENSE" "$1/LICENSE"
python3 "$crate_dir/license-notices.py" "$crate_dir/Cargo.toml" "$1/THIRD_PARTY_LICENSES"
cat >> "$1/aggregate.d.ts" <<'TYPES'
export type * from './wire';
TYPES
cat > "$1/package.json" <<'JSON'
{"name":"sparq-verifiable-aggregate","version":"0.1.0","private":true,"license":"MIT","main":"aggregate.js","types":"aggregate.d.ts","files":["aggregate.js","aggregate.d.ts","aggregate_bg.wasm","aggregate_bg.wasm.d.ts","wire.d.ts","README.md","LICENSE","THIRD_PARTY_LICENSES"],"engines":{"node":">=24"}}
JSON
