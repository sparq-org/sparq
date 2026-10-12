#!/usr/bin/env bash
# Proves the same ACIR circuit with UltraHonk (bb) and with QuickSilver
# (Diet Mac'n'Cheese over BN254), and prints one JSON line per circuit.
#
# usage: bench.sh DIETMC REPS ARTIFACT.json WITNESS.gz [ARTIFACT WITNESS ...]
# Needs nargo-compiled artifacts, bb on PATH with its CRS, python3 with msgpack.
#
# LPN (default large) selects the Diet Mac'n'Cheese LPN parameter size and
# BB_TARGET (default noir-recursive, the target sparq-zk-compose proves for)
# the bb verifier target.
#
# Each timed QuickSilver run is a direct connection; one further run per circuit
# goes through a relay that counts the bytes in each direction.
set -euo pipefail
DIETMC=$1; REPS=$2; shift 2
LPN=${LPN:-large}; BB_TARGET=${BB_TARGET:-noir-recursive}
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
printf "lpn = '%s'\nno_batching = false\nthreads = 1\n" "$LPN" > "$WORK/dmc.toml"
ms() { echo $(( ($(date +%s%N) - $1) / 1000000 )); }
port=5600
while [ $# -gt 0 ]; do
  art=$1; wit=$2; shift 2
  name=$(basename "$art" .json); out="$WORK/$name"; mkdir -p "$out/uh"
  gates=$(bb gates -b "$art" 2>/dev/null | tr -d ' \n' | sed 's/.*"circuit_size":\([0-9]*\).*/\1/')
  bb write_vk -t "$BB_TARGET" -b "$art" -o "$out/uh" >/dev/null 2>&1
  summary=$(python3 "$HERE/acir_to_sieve.py" "$art" "$wit" "$out/ir")
  uhp=(); uhv=(); qs=()
  for _ in $(seq "$REPS"); do
    t=$(date +%s%N); bb prove -t "$BB_TARGET" -b "$art" -w "$wit" -k "$out/uh/vk" -o "$out/uh" >/dev/null 2>&1; uhp+=("$(ms "$t")")
    t=$(date +%s%N); bb verify -t "$BB_TARGET" -p "$out/uh/proof" -i "$out/uh/public_inputs" -k "$out/uh/vk" >/dev/null 2>&1; uhv+=("$(ms "$t")")
    port=$((port + 1))
    qs+=("$(python3 "$HERE/quicksilver_run.py" "$DIETMC" "$WORK/dmc.toml" "$out/ir" "$port")")
  done
  # Both proof systems must have the same public inputs, in the same order.
  python3 - "$out/uh/public_inputs" "$out/ir/public.txt" <<'PY'
import re, sys
raw = open(sys.argv[1], "rb").read()
bb = [int.from_bytes(raw[i:i + 32], "big") for i in range(0, len(raw), 32)]
ir = [int(v) for v in re.findall(r"^<(\d+)>;$", open(sys.argv[2]).read(), re.M)]
sys.exit(0 if bb == ir else f"public inputs differ: bb {len(bb)}, IR {len(ir)}")
PY
  port=$((port + 1))
  traffic=$(python3 "$HERE/quicksilver_run.py" "$DIETMC" "$WORK/dmc.toml" "$out/ir" "$port" --count-bytes)
  join() { local IFS=,; echo "$*"; }
  echo "{\"circuit\":\"$name\",\"lpn\":\"$LPN\",\"bb_target\":\"$BB_TARGET\",\"ultrahonk_gates\":$gates,\"ultrahonk_proof_bytes\":$(stat -c%s "$out/uh/proof"),\"ultrahonk_prove_ms\":[$(join "${uhp[@]}")],\"ultrahonk_verify_ms\":[$(join "${uhv[@]}")],\"quicksilver\":$summary,\"quicksilver_runs\":[$(join "${qs[@]}")],\"quicksilver_traffic\":$traffic}"
done
