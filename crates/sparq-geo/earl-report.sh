#!/usr/bin/env bash
# Write results.md and sparq-geosparql-earl.ttl; default OUT_DIR: target/earl/geo.
# Default suite: upstream HEAD. GEOSPARQL_BENCHMARK_PIN overrides that revision;
# GEOSPARQL_BENCHMARK_DIR uses an existing checkout without touching it.
# A complete run below the ratchet can still report its failed query outcomes.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
out="${1:-$root/target/earl/geo}"
mkdir -p "$out"
out="$(cd "$out" && pwd -P)"
rm -f "$out/outcomes.tsv" "$out/earl-input.tsv" "$out/results.md" "$out/sparq-geosparql-earl.ttl"
suite="$(GEOSPARQL_BENCHMARK_PIN="${GEOSPARQL_BENCHMARK_PIN:-HEAD}" bash "$root/crates/sparq-geo/fetch-geosparql-benchmark.sh")"
suite_commit="$(git -C "$suite" rev-parse HEAD)"
sparq_commit="$(git -C "$root" rev-parse HEAD)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
cd "$root"
args=(-p sparq-geo --features geosparql_rewrite,geof_accessors --test geosparql_benchmark)
cargo test "${args[@]}" --no-run
rc=0
GEOSPARQL_BENCHMARK_DIR="$suite" SPARQ_EARL_OUT="$out/outcomes.tsv" \
  cargo test "${args[@]}" -- --nocapture > "$out/run.log" 2>&1 || rc=$?
if ! grep -qx 'EARL complete geosparql-benchmark' "$out/run.log"; then
  echo "incomplete geosparql-benchmark run (exit $rc); see $out/run.log" >&2
  exit 1
fi
python3 "$root/crates/sparq-geo/benchmark-report.py" "$out/outcomes.tsv" \
  "$suite/src/main/resources" "$suite_commit" "$out/results.md"
sed 's/\t/.rq\t/' "$out/outcomes.tsv" > "$out/earl-input.tsv"
python3 "$root/scripts/earl-report.py" "$out/earl-input.tsv" \
  --map "query-=https://github.com/OpenLinkSoftware/GeoSPARQLBenchmark/blob/$suite_commit/src/main/resources/gsb_queries/query-" \
  --name "sparq $version (commit ${sparq_commit:0:7})" --revision "$sparq_commit" \
  --description "GeoSPARQL Compliance Benchmark at $suite_commit; sparq-geo with RDFS materialization, geof_accessors and geosparql_rewrite. Community benchmark, not an official OGC ETS or certification." \
  --output "$out/sparq-geosparql-earl.ttl"
