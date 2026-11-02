#!/usr/bin/env bash
# Run the Touchstone LWS conformance suite (https://github.com/ebremer/touchstone) against
# sparq-lws-core in LWS mode, as a secured target: the harness holds the authorization server's
# signing key (HarnessIssuedTokens), serves alice's and bob's identity documents and webhook
# inboxes from its fixture host (ReachableFixtures), its SAML identity provider is trusted
# (SamlTrust, when openssl is available), and alice owns the storage.
#
#   TOUCHSTONE=../touchstone crates/sparq-lws-core/conformance/lws/touchstone.sh [module]
#
# module: all (default), core, auth, notifications/webhook, index, or one manifest or test.
# Needs JDK 21 and a built touchstone.jar (cd $TOUCHSTONE && ./mvnw -q -pl harness-cli -am
# -Dmaven.test.skip=true package). Reports land in $OUT (default target/lws-touchstone).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
TOUCHSTONE=${TOUCHSTONE:-$repo/../touchstone}
MODULE=${1:-all}
PORT=${PORT:-3917}
FIXTURES_PORT=${FIXTURES_PORT:-3918}
OUT=${OUT:-$repo/target/lws-touchstone}
BIN=${BIN:-${CARGO_TARGET_DIR:-$repo/target}/debug/sparq-lws-core}
SERVER_BIN=$BIN
SPARQ_BACKEND=${PSS_SPARQ_BACKEND:-memory}
case "$SPARQ_BACKEND" in
  memory|embedded|durable) ;;
  *) echo "ERROR: expected memory, embedded or durable" >&2; exit 1 ;;
esac
if [ "$SPARQ_BACKEND" = durable ] && [ "$MODULE" != all ]; then
  echo "ERROR: durable conformance requires the full suite" >&2
  exit 1
fi
# shellcheck source=crates/sparq-lws-core/conformance/server-lifecycle.sh
source "$here/../server-lifecycle.sh"
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
key="$OUT/as-key.json"
rm -f "$key"
if [ "$SPARQ_BACKEND" = durable ]; then
  rm -f "$OUT/persistence.json" "$OUT/server-term.log" "$OUT/server-kill.log"
  rm -rf -- "$OUT/post-term" "$OUT/post-kill"
fi
[ -x "$BIN" ] || cargo build -q -p sparq-lws-core --features authn-saml --manifest-path "$repo/Cargo.toml"

fixtures="http://localhost:$FIXTURES_PORT/"

# The harness SAML identity provider's key (SamlTrust): an RSA key pair whose private JWK the
# harness signs assertions with, and whose public key the authorization server trusts for the
# entity ${fixtures}idp. Needs openssl; without it the SAML tests are inapplicable.
saml_idps="$OUT/saml-idps.json"
saml_jwk=""
capabilities="Authentication, HarnessIssuedTokens, ReachableFixtures"
rm -f "$saml_idps"
if command -v openssl > /dev/null && command -v python3 > /dev/null; then
  openssl genrsa -out "$OUT/saml-idp.pem" 2048 2> /dev/null
  openssl rsa -in "$OUT/saml-idp.pem" -pubout -out "$OUT/saml-idp.pub.pem" 2> /dev/null
  saml_jwk=$(openssl rsa -in "$OUT/saml-idp.pem" -text -noout 2> /dev/null | python3 -c '
import base64, json, re, sys
text = sys.stdin.read()
def field(name):
    m = re.search(r"^" + name + r":\s*((?:\s+[0-9a-f:]+\n?)+)", text, re.M)
    return int(m.group(1).replace(":", "").replace("\n", "").replace(" ", ""), 16)
def b64(n):
    return base64.urlsafe_b64encode(n.to_bytes((n.bit_length() + 7) // 8, "big")).rstrip(b"=").decode()
e = int(re.search(r"publicExponent: (\d+)", text).group(1))
names = {"n": "modulus", "d": "privateExponent", "p": "prime1", "q": "prime2", "dp": "exponent1", "dq": "exponent2", "qi": "coefficient"}
jwk = {"kty": "RSA", "kid": "saml-idp", "e": b64(e)}
jwk.update({k: b64(field(v)) for k, v in names.items()})
print(json.dumps(jwk))
')
  python3 -c 'import json, sys; print(json.dumps({sys.argv[1]: open(sys.argv[2]).read()}))' \
    "${fixtures}idp" "$OUT/saml-idp.pub.pem" > "$saml_idps"
  capabilities="$capabilities, SamlTrust"
fi
server_env() {
  unset PSS_SPARQ_BACKEND SOLID_SERVER_DATA_DIR SOLID_SERVER_SPARQ_DIR
  unset SOLID_SERVER_WAIT_FOR_LOCK_SECS SOLID_SERVER_ALLOW_INCONSISTENT_DURABILITY SOLID_SERVER_LWS_OPEN
  unset SOLID_SERVER_SEED_CONFORMANCE SOLID_SERVER_SEED_DEMO SOLID_SERVER_SEED_BENCH
  unset SOLID_SERVER_ALLOW_SEED_NONMEMORY
  if [ "$SPARQ_BACKEND" = durable ]; then
    export SOLID_SERVER_DATA_DIR="$DATA_DIR" SOLID_SERVER_WAIT_FOR_LOCK_SECS=10
  else
    export PSS_SPARQ_BACKEND="$SPARQ_BACKEND"
  fi
  SOLID_SERVER_PROTOCOL=lws \
  SOLID_SERVER_BIND=127.0.0.1:$PORT \
  SOLID_SERVER_BASE_URL=http://localhost:$PORT \
  SOLID_SERVER_LWS_OWNER="${fixtures}agents/alice" \
  SOLID_SERVER_LWS_AS_KEY_FILE="$key" \
  SOLID_SERVER_LWS_PAGE_SIZE=${PAGE_SIZE:-4} \
  SOLID_SERVER_LWS_ALLOW_INSECURE_FETCH=1 \
  SOLID_SERVER_LWS_SAML_IDPS_FILE="$([ -s "$saml_idps" ] && echo "$saml_idps")" \
    "$@"
}
server_ready() {
  [ -s "$key" ] && [ "$(curl -s --max-time 2 -o /dev/null -w '%{http_code}' \
    "http://localhost:$PORT/.well-known/lws-configuration")" = 200 ]
}
trap cleanup_server EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
if [ "$SPARQ_BACKEND" = durable ]; then make_data_dir; fi
start_server "$OUT/server.log"

run_touchstone() {
  local output=$1 status=0 parallelism=16
  # Keep the root-listing probe stable and avoid competing durable mutations.
  if [ "$SPARQ_BACKEND" = durable ]; then parallelism=1; fi
  mkdir -p "$output"
  # The floor reader must never accept the previous invocation's report.
  rm -rf -- "$output/runs"
  signing_key=$(cat "$key")
  cat > "$output/targets.yaml" <<YAML
targets:
  sparq:
    baseUrl: http://localhost:$PORT/
    adapter: env
    capabilities: [$capabilities]
    properties:
      parallelism: '$parallelism'
      saml.idpKey: '$saml_jwk'
      as.signingKey: '$signing_key'
      fixtures.baseUrl: '$fixtures'
      webid.alice: '${fixtures}agents/alice'
      webid.bob: '${fixtures}agents/bob'
YAML
  cd "$TOUCHSTONE"
  java -jar harness-cli/target/touchstone.jar run --target sparq --targets "$output/targets.yaml" \
    --module "$MODULE" --report-dir "$output/runs" 2>&1 | grep -v '^Picked up JAVA_TOOL_OPTIONS' || status=${PIPESTATUS[0]}
  return "$status"
}

if [ "$SPARQ_BACKEND" != durable ]; then
  # The floor check gates suite outcomes; only harness and shutdown failures fail here.
  run_touchstone "$OUT" || true
  stop_server TERM
  exit 0
fi

run_touchstone "$OUT" || true
python3 "$here/check-floor.py" "$OUT/runs"
previous_runs="$OUT/runs"
for phase in term kill; do
  python3 "$here/../durable-check.py" capture --protocol lws --base "http://localhost:$PORT" \
    --key "$key" --owner "${fixtures}agents/alice" --runs "$previous_runs" \
    --manifest "$OUT/persistence.json" --phase "$phase"
  # All PUTs have completed and the suite has exited: the kill is mid-idle.
  if [ "$phase" = term ]; then stop_server TERM; else stop_server KILL; fi
  start_server "$OUT/server-$phase.log"
  python3 "$here/../durable-check.py" check --protocol lws --base "http://localhost:$PORT" \
    --key "$key" --owner "${fixtures}agents/alice" --manifest "$OUT/persistence.json"
  run_touchstone "$OUT/post-$phase" || true
  python3 "$here/check-floor.py" "$OUT/post-$phase/runs"
  previous_runs="$OUT/post-$phase/runs"
done
stop_server TERM
