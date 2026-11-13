#!/usr/bin/env bash
# Pinned ODRL decision-projection EARL and README-shaped results table.
# Usage: crates/sparq-policy/earl-report.sh [OUT_DIR] (default target/earl).
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
out="${1:-$root/target/earl}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
report="$out/sparq-odrl-earl.ttl"
table="$out/sparq-odrl-results.md"
raw="$out/odrl-outcomes.tsv"
rm -f "$report" "$table" "$raw" "$out/odrl-revisions.tsv"
suite="$root/tests/odrl-test-suite"
if [ ! -d "$suite/data/test_cases" ]; then
  echo "Fetch the pinned suite first: bash scripts/fetch-odrl-suite.sh" >&2
  exit 1
fi
suite_commit="$(git -C "$suite" rev-parse HEAD)"
git -C "$suite" diff --quiet HEAD -- data || { echo "ODRL suite data has local changes" >&2; exit 1; }
sparq_commit="$(git -C "$root" rev-parse HEAD)"
cd "$root"
cargo test -p sparq-policy --test odrl_test_suite --no-run
status=0
ODRL_TESTS_DIR="$suite/data" SPARQ_EARL_OUT="$raw" SPARQ_ODRL_TABLE_OUT="$table" \
  cargo test -p sparq-policy --test odrl_test_suite odrl_test_suite_decision_parity \
    -- --exact --nocapture > "$out/odrl.log" 2>&1 || status=$?
count="$(wc -l < "$raw" 2>/dev/null | tr -d ' ' || true)"
if [ ! -s "$raw" ] || ! grep -qx "EARL complete odrl $count" "$out/odrl.log"; then
  rm -f "$raw" "$table"
  echo "Incomplete ODRL run (exit $status); see $out/odrl.log" >&2
  exit 1
fi
python3 "$root/scripts/earl-report.py" "$raw" --map urn:uuid:=urn:uuid: \
  --name "sparq-policy (commit ${sparq_commit:0:7})" --revision "$sparq_commit" \
  --description "ODRL-Test-Suite $suite_commit, fetched by scripts/fetch-odrl-suite.sh. Expected rule activation projected to allow/deny, not full compliance-report graph equality. Every case is included; unsupported cases and harness errors are failed. Runner exit $status; existing ratchets unchanged." \
  --output "$report"
printf 'sparq\t%s\nodrl-test-suite\t%s\nrunner-exit\t%s\n' \
  "$sparq_commit" "$suite_commit" "$status" > "$out/odrl-revisions.tsv"
exit "$status"
