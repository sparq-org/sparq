#!/usr/bin/env bash
# Run the LWS.net conformance harness (https://github.com/langsamu/LWS.net) with the YAML-LD
# suite definition from elf-pavlik/lws-test-suite (branch yaml), as lws-contrib/dagger-workspace
# does, against sparq-lws-core in LWS open mode (the harness sends no credentials).
#
#   LWS_NET=../LWS.net LWS_TESTS=../lws-test-suite/lws10/tests.yaml \
#     crates/sparq-lws-core/conformance/lws/lws-net.sh
#
# LWS_TESTS=builtin runs the definitions LWS.net itself carries (Suite/Model/Resources/new.ttl)
# instead. LWS_NET_EXPECTED_FAILURES names the JSON exception list; together with
# LWS_NET_EXPECTED_TOTAL it requires exactly those failed test names. A listed pass or missing
# test, an unlisted failure, an inconsistent report or an aborted harness fails the run.
# Each failed test is printed by name (lws-net-report.py; `--self-test` checks it).
#
# Needs the .NET 10 SDK and Node.js (to convert the YAML-LD definition to N-Triples).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
LWS_NET=${LWS_NET:-$repo/../LWS.net}
LWS_TESTS=${LWS_TESTS:-$repo/../lws-test-suite/lws10/tests.yaml}
PORT=${PORT:-3919}
OUT=${OUT:-$repo/target/lws-net}
BIN=${BIN:-$repo/target/debug/sparq-lws-core}
mkdir -p "$OUT"
[ -x "$BIN" ] || cargo build -q -p sparq-lws-core --manifest-path "$repo/Cargo.toml"

# YAML-LD -> N-Triples, mounted as the harness's embedded suite definition (Resources/new.ttl).
conv="$OUT/convert"
mkdir -p "$conv"
if [ ! -d "$conv/node_modules/jsonld" ]; then
  (cd "$conv" && npm init -y >/dev/null && npm install --silent jsonld@8.3.3 yaml@2.9.1 >/dev/null)
fi
cat > "$conv/convert.mjs" <<'JS'
import { readFile, writeFile } from "node:fs/promises";
import jsonld from "jsonld";
import { parse } from "yaml";
const [, , src, dst] = process.argv;
const doc = parse(await readFile(src, "utf8"));
await writeFile(dst, await jsonld.toRDF(doc, { format: "application/n-quads" }));
JS
if [ "$LWS_TESTS" != builtin ]; then
  (cd "$conv" && node convert.mjs "$LWS_TESTS" "$LWS_NET/Suite/Model/Resources/new.ttl")
fi

SOLID_SERVER_PROTOCOL=lws \
SOLID_SERVER_LWS_OPEN=1 \
SOLID_SERVER_BIND=127.0.0.1:$PORT \
SOLID_SERVER_BASE_URL=http://localhost:$PORT \
  "$BIN" > "$OUT/server.log" 2>&1 &
pid=$!
trap 'kill $pid 2>/dev/null || true' EXIT
for _ in $(seq 1 100); do
  curl -s -o /dev/null "http://localhost:$PORT/" && break
  sleep 0.2
done
cd "$LWS_NET"
# A report left by an earlier run must not stand in for this one.
rm -f "$OUT/lws-net.trx"
status=0
# LWS.net's global.json selects Microsoft.Testing.Platform, which takes --report-trx rather than
# VSTest's --logger (it exits 5, invalid arguments, on --logger).
Suite__BaseUri="http://localhost:$PORT/" dotnet test Suite/Test \
  --report-trx --report-trx-filename lws-net.trx --results-directory "$OUT" || status=$?
# Without an exception list the harness status stands (the tests.yaml step).
python3 -B "$here/lws-net-report.py" "$OUT/lws-net.trx" --status "$status" \
  ${GITHUB_ACTIONS:+--gha} \
  ${LWS_NET_EXPECTED_FAILURES:+--expected-failures "$LWS_NET_EXPECTED_FAILURES"} \
  ${LWS_NET_EXPECTED_TOTAL:+--expected-total "$LWS_NET_EXPECTED_TOTAL"} || exit $?
