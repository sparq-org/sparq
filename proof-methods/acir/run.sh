#!/usr/bin/env bash
# Builds the three openings of N private xsd:integer literals and proves and
# verifies each with UltraHonk:
#   baseline  - full_u64_check: 20 fixed-length BLAKE3 black boxes per value
#   noir_var  - one variable-length BLAKE3 per value, written in Noir
#   hand      - the same variable-length BLAKE3 as hand-written ACIR,
#               appended to the compiled `spliced` program
# Each value is compared with value <= 2^64 - 1 (op 1), which every u64
# satisfies, so the public inputs do not reveal the values.
# Usage: run.sh VALUE...   (exactly N = leaf::N values)
set -euo pipefail
cd "$(dirname "$0")/circuits"
NARGO=${NARGO:-nargo}
BB=${BB:-bb}
n=$(sed -n 's/^pub global N: u32 = \([0-9]*\);/\1/p' leaf/src/lib.nr)
[ $# -eq "$n" ] || { echo "expected $n values" >&2; exit 2; }
"$NARGO" compile --workspace >/dev/null
join() { local IFS=,; echo "$*"; }
operands=() hs=() ops=() bounds=() values=()
for v in "$@"; do
  h=$(python3 ../blake3_leaf.py hs "$v")
  echo "hs = \"$h\"" > encode/Prover.toml
  operands+=("\"$("$NARGO" execute --package encode 2>&1 | sed -n 's/.*Circuit output: //p')\"")
  hs+=("\"$h\"") ops+=(1) bounds+=("\"18446744073709551615\"") values+=("\"$v\"")
done
common="operands = [$(join "${operands[@]}")]
ops = [$(join "${ops[@]}")]
bounds = [$(join "${bounds[@]}")]
values = [$(join "${values[@]}")]"
printf '%s\n' "$common" > baseline/Prover.toml
printf '%s\n' "$common" > noir_var/Prover.toml
printf '%s\nhs = [%s]\n' "$common" "$(join "${hs[@]}")" > spliced/Prover.toml
for p in baseline noir_var spliced; do
  "$NARGO" execute --package "$p" >/dev/null
done
python3 ../blake3_leaf.py splice target/spliced.json target/spliced.gz target/hand.json target/hand.gz >&2
# bb has no UltraHonk witness check, so prove and verify.
for p in baseline noir_var hand; do
  out=target/proof-$p
  rm -rf "$out" && mkdir -p "$out"
  "$BB" prove -s ultra_honk -b "target/$p.json" -w "target/$p.gz" -o "$out" --write_vk >"$out/log" 2>&1 \
    && "$BB" verify -s ultra_honk -p "$out/proof" -k "$out/vk" -i "$out/public_inputs" >>"$out/log" 2>&1 \
    || { echo "$p does not prove and verify (see $out/log)" >&2; exit 1; }
done
echo "values $*: all three prove and verify" >&2
