#!/usr/bin/env bash
# Prints the UltraHonk gate count of each circuit run.sh built, one JSON
# object per line. Run run.sh first; leaf::N sets the number of openings.
set -euo pipefail
cd "$(dirname "$0")/circuits"
BB=${BB:-bb}
n=$(sed -n 's/^pub global N: u32 = \([0-9]*\);/\1/p' leaf/src/lib.nr)
for p in baseline noir_var noir_tuned spliced hand; do
  gates=$("$BB" gates -s ultra_honk -b "target/$p.json" 2>/dev/null \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["functions"][0]["circuit_size"])')
  printf '{"circuit": "%s", "openings": %s, "ultra_honk_gates": %s}\n' "$p" "$n" "$gates"
done
