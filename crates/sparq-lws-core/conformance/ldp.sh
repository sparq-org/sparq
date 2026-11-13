#!/usr/bin/env bash
# Build the archived W3C suite and run Basic plus a Direct membership probe.
# Usage: LDP_TESTSUITE=/path/to/ldp-testsuite conformance/ldp.sh [OUT_DIR]
# Requires Maven and JDK 8 (the upstream pom targets Java 7).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
out="${1:-$root/target/earl}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
rm -f "$out/sparq-ldp-earl.ttl" "$out/sparq-ldp-direct-probe-earl.ttl" "$out/ldp-revisions.tsv"
suite="${LDP_TESTSUITE:?Set LDP_TESTSUITE to a w3c/ldp-testsuite source checkout}"
suite="$(cd "$suite" && pwd)"
suite_commit="$(git -C "$suite" rev-parse HEAD)"
sparq_commit="$(git -C "$root" rev-parse HEAD)"
cd "$root"
# Build without executing the suite's own integration tests during Maven packaging.
mvn -f "$suite/pom.xml" -DskipTests package > "$out/ldp-build.log" 2>&1
jars=("$suite"/target/ldp-testsuite-*-shaded.jar)
[ "${#jars[@]}" = 1 ] && [ -f "${jars[0]}" ] || { echo "Expected one shaded testsuite JAR" >&2; exit 1; }
cargo build -p sparq-lws-core --example ldp_conformance
SERVER_BIN="${CARGO_TARGET_DIR:-$root/target}/debug/examples/ldp_conformance"
# shellcheck source=crates/sparq-lws-core/conformance/server-lifecycle.sh
source "$here/server-lifecycle.sh"
server_env() { "$@"; }
server_ready() { [ "$(curl -s --max-time 2 -o /dev/null -w '%{http_code}' http://127.0.0.1:3001/readyz)" = 200 ]; }
trap cleanup_server EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
start_server "$out/ldp-server.log"
statuses=()
reports=()
for level in basic direct; do
  # Fresh per-level directory: --output DIR writes EARL under DIR/report/.
  run="$out/ldp-$level"
  rm -rf "$run"
  mkdir -p "$run"
  status=0
  java -jar "${jars[0]}" --server "http://127.0.0.1:3001/$level/" "--$level" \
    --includedGroups MUST SHOULD MAY --non-rdf --earl --output "$run" \
    --software "sparq-lws-core ($sparq_commit), $level fixture" \
    --developer 'sparq contributors' --language Rust \
    --homepage https://github.com/sparq-org/sparq \
    --assertor https://github.com/sparq-org --shortname sparq-lws-core \
    > "$run/runner.log" 2>&1 || status=$?
  native="$run/report/ldp-testsuite-execution-report-earl.ttl"
  [ -s "$native" ] || { echo "No fresh $level EARL (exit $status); see $run/runner.log" >&2; exit 1; }
  statuses+=("$status")
  reports+=("$native")
done
# Keep the submission report and the unclaimed Direct probe separate and unchanged.
cp "${reports[0]}" "$out/sparq-ldp-earl.ttl"
cp "${reports[1]}" "$out/sparq-ldp-direct-probe-earl.ttl"
printf 'sparq\t%s\nldp-testsuite\t%s\nbasic-exit\t%s\ndirect-exit\t%s\ngroups\tMUST SHOULD MAY\ndirect-scope\tmembership probe; support not claimed\n' \
  "$sparq_commit" "$suite_commit" "${statuses[0]}" "${statuses[1]}" > "$out/ldp-revisions.tsv"
stop_server TERM
# TestNG failures and skips can return nonzero with a valid report. Preserve that status.
for status in "${statuses[@]}"; do if [ "$status" != 0 ]; then exit "$status"; fi; done
