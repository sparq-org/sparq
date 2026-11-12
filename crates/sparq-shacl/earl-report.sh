#!/usr/bin/env bash
# Builds sparq's EARL implementation reports for the W3C SHACL test suites
# (w3c/data-shapes): one for the SHACL 1.0 suite (data-shapes-test-suite) and one
# for the SHACL 1.2 suite (shacl12-test-suite), in the format the Data Shapes WG
# collects under each suite's reports/ folder.
#
#   crates/sparq-shacl/earl-report.sh [OUT_DIR]   (default: target/earl)
#
# The suite revision is upstream HEAD, or SHACL_TESTS_PIN if set.
#
# Runs the suite runners with SPARQ_EARL_OUT set and the shacl-af feature on, so
# every entry sparq can attempt is attempted. A runner's ratchet assertion may
# fail (its exit status is ignored here); the reported outcome of every entry
# is what goes in the report, failures included. SHACL 1.2 inference rules
# (sht:Infer, tests/inference-rules) have no runner yet and are not reported.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
out="${1:-$root/target/earl}"
suite="$root/crates/sparq-shacl/tests/shacl/data-shapes"
runners=(w3c_core w3c_sparql w3c_sparql_component
         w3c_core_full_shacl12 w3c_sparql_shacl12 w3c_node_expr w3c_node_expr_constraints)

# Report against the suite as it stands upstream unless SHACL_TESTS_PIN names a
# revision. (Afterwards, fetch-shacl-tests.sh with no override restores the pin.)
SHACL_TESTS_PIN="${SHACL_TESTS_PIN:-$(git ls-remote https://github.com/w3c/data-shapes HEAD | cut -f1)}" \
  "$root/crates/sparq-shacl/fetch-shacl-tests.sh"
mkdir -p "$out"
raw="$out/outcomes.tsv"
rm -f "$raw"

cd "$root"
args=(-p sparq-shacl --features shacl-af)
for r in "${runners[@]}"; do args+=(--test "$r"); done
cargo test "${args[@]}" --no-run
for r in "${runners[@]}"; do
  SPARQ_EARL_OUT="$raw" cargo test -p sparq-shacl --features shacl-af --test "$r" \
    > "$out/$r.log" 2>&1 || true
done

# The runners canonicalize manifest paths, so match on the resolved suite path.
suite="$(cd "$suite" && pwd -P)"
suite_commit="$(git -C "$suite" rev-parse HEAD)"
sparq_commit="$(git -C "$root" rev-parse HEAD)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
name="sparq $version (commit ${sparq_commit:0:7})"

report() { # <suite dir> <output file> <spec>
  grep -F "file://$suite/$1/tests/" "$raw" > "$out/$1.tsv"
  python3 "$root/scripts/earl-report.py" "$out/$1.tsv" \
    --map "file://$suite/$1/tests/=urn:x-shacl-test:/" \
    --name "$name" --revision "$sparq_commit" \
    --description "sparq, an RDF store and SPARQL engine in Rust, built from commit $sparq_commit with the sparq-shacl crate's shacl-af feature on. Run against the w3c/data-shapes $1 at commit $suite_commit using crates/sparq-shacl/earl-report.sh. $3" \
    --output "$out/$2"
}
report data-shapes-test-suite sparq-shacl-earl.ttl "Covers SHACL Core and SHACL-SPARQL."
report shacl12-test-suite sparq-shacl12-earl.ttl \
  "Covers SHACL 1.2 Core, SHACL-SPARQL validation and node expressions. sparq does not yet implement SHACL 1.2 inference rules, so tests/inference-rules is not reported."
