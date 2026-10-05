#!/usr/bin/env bash
# Run the LWS.net conformance harness (https://github.com/langsamu/LWS.net) with the YAML-LD
# suite definition from elf-pavlik/lws-test-suite (branch yaml), as lws-contrib/dagger-workspace
# does, against sparq-lws-core in LWS open mode (the harness sends no credentials).
#
#   LWS_NET=../LWS.net LWS_TESTS=../lws-test-suite/lws10/tests.yaml \
#     crates/sparq-lws-core/conformance/lws/lws-net.sh
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
  (cd "$conv" && npm init -y >/dev/null && npm install --silent jsonld@8 yaml@2 >/dev/null)
fi
cat > "$conv/convert.mjs" <<'JS'
import { readFile, writeFile } from "node:fs/promises";
import jsonld from "jsonld";
import { parse } from "yaml";
const [, , src, dst] = process.argv;
const doc = parse(await readFile(src, "utf8"));
await writeFile(dst, await jsonld.toRDF(doc, { format: "application/n-quads" }));
JS
(cd "$conv" && node convert.mjs "$LWS_TESTS" "$LWS_NET/Suite/Model/Resources/new.ttl")

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
status=0
Suite__BaseUri="http://localhost:$PORT/" dotnet test Suite/Test \
  --logger "trx;LogFileName=lws-net.trx" --results-directory "$OUT" || status=$?
# One line with the counts, as a CI annotation when running in GitHub Actions.
counts=$(grep -o '<Counters [^>]*>' "$OUT/lws-net.trx" 2>/dev/null | head -1 || true)
echo "${GITHUB_ACTIONS:+::notice title=lws-net::}lws-net ${counts:-no results}"
exit $status
