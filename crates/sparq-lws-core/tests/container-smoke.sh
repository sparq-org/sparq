#!/usr/bin/env bash
# [GPT-5.6] sq-lmz40 — build and exercise the native sparq-lws-core image.
#
# Usage: crates/sparq-lws-core/tests/container-smoke.sh [IMAGE_REF] [HOST_PORT]
# Requires Linux Docker, curl, and Node.js (built-in crypto/http only) for the
# temporary loopback issuer used by the authenticated durable scenario.
# Set LWS_CONTAINER_SKIP_BUILD=1 only when the caller already built that exact
# IMAGE_REF (the release workflow does this to share its BuildKit cache).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
IMAGE="${1:-sparq-lws-core:smoke}"
PORT="${2:-3000}"
NAME="sparq-lws-smoke-$$"
BASE="http://127.0.0.1:${PORT}"
DURABLE_NAME="${NAME}-durable"
VOLUME="${NAME}-data"
IDP_PID=""
VOLUME_CREATED=""
umask 077
WORK="$(mktemp -d)"

cleanup() {
  echo "::group::container logs (${NAME})"
  docker logs "${NAME}" 2>&1 || true
  echo "::endgroup::"
  docker rm -fv "${NAME}" >/dev/null 2>&1 || true
  echo "::group::durable container logs (${DURABLE_NAME})"
  docker logs "${DURABLE_NAME}" 2>&1 || true
  echo "::endgroup::"
  docker rm -fv "${DURABLE_NAME}" >/dev/null 2>&1 || true
  if [ -n "${IDP_PID}" ]; then
    kill "${IDP_PID}" 2>/dev/null || true
    wait "${IDP_PID}" 2>/dev/null || true
  fi
  rm -rf "${WORK}"
  if [ -n "${VOLUME_CREATED}" ]; then
    docker volume rm "${VOLUME}" >/dev/null
  fi
}
trap cleanup EXIT

cd "${ROOT}"

if [ "${LWS_CONTAINER_SKIP_BUILD:-0}" != "1" ]; then
  build_args=()
  if [ -n "${CARGO_FLAGS:-}" ]; then
    build_args+=(--build-arg "CARGO_FLAGS=${CARGO_FLAGS}")
  fi
  echo ">> building ${IMAGE}"
  docker build \
    --file crates/sparq-lws-core/Dockerfile \
    --tag "${IMAGE}" \
    "${build_args[@]}" \
    .
else
  echo ">> using caller-built ${IMAGE}"
fi

# Check the image declaration as well as the live process. An empty/root user is
# a packaging regression even on a host that happens to remap container uid 0.
configured_user="$(docker image inspect --format '{{.Config.User}}' "${IMAGE}")"
case "${configured_user}" in
  ""|0|0:0|root|root:root)
    echo "!! image is configured to run as root (${configured_user:-<empty>})" >&2
    exit 1
    ;;
esac

echo ">> running ${IMAGE} as configured user ${configured_user}"
docker run -d \
  --name "${NAME}" \
  -p "127.0.0.1:${PORT}:3000" \
  "${IMAGE}" >/dev/null

# Ask the daemon for the live UID; the client may use a different PID namespace.
runtime_uid="$(docker top "${NAME}" -eo pid,uid | awk 'NR == 2 { print $2; exit }')"
case "${runtime_uid}" in
  ""|0|*[!0-9]*)
    echo "!! container process is running as uid ${runtime_uid:-<unknown>}" >&2
    exit 1
    ;;
esac
echo ">> process uid ${runtime_uid} is non-root"

wait_for_health() {
  local container_name="$1"
  local path reachable attempt
  for path in livez readyz; do
    echo ">> waiting for /${path}"
    reachable=""
    for attempt in $(seq 1 30); do
      if [ "$(docker inspect --format '{{.State.Running}}' "${container_name}" 2>/dev/null || echo false)" != "true" ]; then
        echo "!! container exited before /${path} became reachable" >&2
        exit 1
      fi
      if curl --fail --silent --show-error "${BASE}/${path}" >/dev/null 2>&1; then
        reachable=1
        echo ">> /${path} reachable (attempt ${attempt})"
        break
      fi
      sleep 1
    done
    if [ -z "${reachable}" ]; then
      echo "!! /${path} was not reachable within 30 seconds" >&2
      exit 1
    fi
  done
}
wait_for_health "${NAME}"

echo ">> asserting an anonymous mutation fails closed"
status="$(curl --silent --show-error \
  --output /dev/null \
  --write-out '%{http_code}' \
  --request PUT \
  --header 'Content-Type: text/turtle' \
  --data '<> <https://example.test/predicate> "value" .' \
  "${BASE}/container-smoke-resource")"
case "${status}" in
  401|403)
    echo ">> anonymous PUT rejected with HTTP ${status}"
    ;;
  *)
    echo "!! anonymous PUT returned HTTP ${status}; expected 401 or 403" >&2
    exit 1
    ;;
esac

echo ">> SMOKE PASS: health reachable, anonymous mutation rejected, process non-root"

# The default image must still be in-memory and fail closed. Use a separate,
# disposable instance for authenticated writes and the durable storage contract.
docker rm -fv "${NAME}" >/dev/null
docker image inspect --format '{{range $path, $_ := .Config.Volumes}}{{$path}}{{"\n"}}{{end}}' "${IMAGE}" \
  | grep -Fxq /data

# This fixture issuer serves only loopback. Its short-lived token is bound to a
# fresh DPoP key; the production auth middleware still verifies the PUT. The seed
# grants this synthetic HTTPS WebID control on a disposable bench pod. None of
# these test-only settings is part of the image or Helm defaults.
node - "${WORK}" "${BASE}" >"${WORK}/issuer.log" 2>&1 <<'JS' &
const { createServer } = require('node:http');
const { generateKeyPairSync, createHash, randomUUID, sign } = require('node:crypto');
const { writeFileSync } = require('node:fs');
const [work, base] = process.argv.slice(2);
const issuerKey = generateKeyPairSync('ec', { namedCurve: 'P-256' });
const proofKey = generateKeyPairSync('ec', { namedCurve: 'P-256' });
const jwk = proofKey.publicKey.export({ format: 'jwk' });
const digest = value => createHash('sha256').update(value).digest('base64url');
const thumbprint = digest(JSON.stringify({ crv: jwk.crv, kty: jwk.kty, x: jwk.x, y: jwk.y }));
const b64 = value => Buffer.from(JSON.stringify(value)).toString('base64url');
function jwt(key, header, claims) {
  const input = `${b64(header)}.${b64(claims)}`;
  const signature = sign('sha256', Buffer.from(input), {
    key: key.privateKey, dsaEncoding: 'ieee-p1363',
  }).toString('base64url');
  return `${input}.${signature}`;
}
let issuer;
const server = createServer((req, res) => {
  res.setHeader('Content-Type', 'application/json');
  if (req.url === '/.well-known/openid-configuration') {
    res.end(JSON.stringify({ issuer, jwks_uri: `${issuer}/jwks` }));
  } else if (req.url === '/jwks') {
    res.end(JSON.stringify({ keys: [issuerKey.publicKey.export({ format: 'jwk' })] }));
  } else {
    res.writeHead(404).end();
  }
});
server.listen(0, '127.0.0.1', () => {
  issuer = `http://127.0.0.1:${server.address().port}`;
  const iat = Math.floor(Date.now() / 1000);
  const webid = 'https://smoke.example/profile#me';
  const token = jwt(issuerKey, { alg: 'ES256', typ: 'at+jwt' }, {
    iss: issuer, sub: webid, webid, client_id: 'container-smoke',
    aud: base, iat, exp: iat + 300, jti: randomUUID(), cnf: { jkt: thumbprint },
  });
  const proof = jwt(proofKey, { alg: 'ES256', typ: 'dpop+jwt', jwk }, {
    htm: 'PUT', htu: `${base}/bench/public/container-smoke-resource`,
    iat, jti: randomUUID(), ath: digest(token),
  });
  writeFileSync(`${work}/put-headers`, `Authorization: DPoP ${token}\nDPoP: ${proof}\n`);
  writeFileSync(`${work}/issuer`, issuer);
});
JS
IDP_PID=$!
for attempt in $(seq 1 30); do
  if [ -s "${WORK}/issuer" ]; then
    break
  fi
  if ! kill -0 "${IDP_PID}" 2>/dev/null; then
    cat "${WORK}/issuer.log" >&2
    exit 1
  fi
  sleep 1
done
if [ ! -s "${WORK}/issuer" ]; then
  echo "!! temporary issuer failed to start" >&2
  exit 1
fi

docker volume create "${VOLUME}" >/dev/null
VOLUME_CREATED=1
echo ">> starting a disposable durable instance"
# Host networking lets the Linux container reach the loopback-only test issuer;
# the server itself also binds only loopback. No production auth bypass is added.
docker run -d --name "${DURABLE_NAME}" --network host \
  --read-only --tmpfs /tmp \
  -v "${VOLUME}:/data" \
  -e SOLID_SERVER_DATA_DIR=/data \
  -e "SOLID_SERVER_BIND=127.0.0.1:${PORT}" \
  -e "SOLID_SERVER_BASE_URL=${BASE}" \
  -e "SOLID_SERVER_TRUSTED_ISSUER=$(cat "${WORK}/issuer")" \
  -e SOLID_SERVER_ALLOW_LOOPBACK=1 \
  -e SOLID_SERVER_BIDIRECTIONAL=off \
  -e SOLID_SERVER_SEED_BENCH=1 \
  -e SOLID_SERVER_SEED_BENCH_OWNER=https://smoke.example/profile#me \
  -e SOLID_SERVER_ALLOW_SEED_NONMEMORY=1 \
  "${IMAGE}" >/dev/null
wait_for_health "${DURABLE_NAME}"

# A non-RDF body exercises the blob path without serialization changes. The
# trailing newline and NUL also prevent a text-only comparison hiding byte loss.
printf 'container durable smoke\000body\r\n' >"${WORK}/expected"
resource="${BASE}/bench/public/container-smoke-resource"
status="$(curl --fail --silent --show-error \
  --output /dev/null --write-out '%{http_code}' \
  --request PUT --header "@${WORK}/put-headers" \
  --header 'Content-Type: application/octet-stream' \
  --data-binary "@${WORK}/expected" "${resource}")"
if [ "${status}" != "201" ]; then
  echo "!! durable PUT returned HTTP ${status}; expected 201" >&2
  exit 1
fi

get_resource() {
  local phase="$1"
  curl --fail --silent --show-error \
    --dump-header "${WORK}/${phase}.headers" \
    --output "${WORK}/${phase}.body" "${resource}"
  cmp "${WORK}/expected" "${WORK}/${phase}.body"
}
get_etag() {
  awk 'tolower($1) == "etag:" { sub(/^[^:]*:[ \t]*/, ""); sub(/\r$/, ""); print }' "$1"
}
get_resource before
etag_before="$(get_etag "${WORK}/before.headers")"
if [ -z "${etag_before}" ]; then
  echo "!! durable resource has no ETag" >&2
  exit 1
fi

echo ">> restarting durable instance with the same named volume"
docker restart "${DURABLE_NAME}" >/dev/null
wait_for_health "${DURABLE_NAME}"
get_resource after
etag_after="$(get_etag "${WORK}/after.headers")"
if [ "${etag_before}" != "${etag_after}" ]; then
  echo "!! durable resource ETag changed across restart" >&2
  exit 1
fi

echo ">> DURABLE SMOKE PASS: body and ETag survive docker restart"
