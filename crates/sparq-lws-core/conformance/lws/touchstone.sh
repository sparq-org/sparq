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
BIN=${BIN:-$repo/target/debug/sparq-lws-core}
mkdir -p "$OUT"
key="$OUT/as-key.json"
rm -f "$key"
[ -x "$BIN" ] || cargo build -q -p sparq-lws-core --manifest-path "$repo/Cargo.toml"

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
SOLID_SERVER_PROTOCOL=lws \
SOLID_SERVER_BIND=127.0.0.1:$PORT \
SOLID_SERVER_BASE_URL=http://localhost:$PORT \
SOLID_SERVER_LWS_OWNER="${fixtures}agents/alice" \
SOLID_SERVER_LWS_AS_KEY_FILE="$key" \
SOLID_SERVER_LWS_PAGE_SIZE=${PAGE_SIZE:-4} \
SOLID_SERVER_LWS_ALLOW_INSECURE_FETCH=1 \
SOLID_SERVER_LWS_SAML_IDPS_FILE="$([ -s "$saml_idps" ] && echo "$saml_idps")" \
  "$BIN" > "$OUT/server.log" 2>&1 &
pid=$!
trap 'kill $pid 2>/dev/null || true' EXIT
for _ in $(seq 1 100); do
  [ -s "$key" ] && curl -s -o /dev/null "http://localhost:$PORT/" && break
  sleep 0.2
done
signing_key=$(cat "$key")
cat > "$OUT/targets.yaml" <<YAML
targets:
  sparq:
    baseUrl: http://localhost:$PORT/
    adapter: env
    capabilities: [$capabilities]
    properties:
      saml.idpKey: '$saml_jwk'
      as.signingKey: '$signing_key'
      fixtures.baseUrl: '$fixtures'
      webid.alice: '${fixtures}agents/alice'
      webid.bob: '${fixtures}agents/bob'
YAML
cd "$TOUCHSTONE"
set +e
java -jar harness-cli/target/touchstone.jar run --target sparq --targets "$OUT/targets.yaml" \
  --module "$MODULE" --report-dir "$OUT/runs" 2>&1 | grep -v '^Picked up JAVA_TOOL_OPTIONS'
status=${PIPESTATUS[0]}
exit "$status"
