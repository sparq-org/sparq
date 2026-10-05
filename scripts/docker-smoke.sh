#!/usr/bin/env bash
# [OPUS-4.8] sq-n6rv — sparq-server container smoke test.
#
# WHY: the released ghcr.io image is the only surface where a `docker run` that exits
# immediately ships silently. sq-n6rv was exactly that: after the fail-closed bind posture
# (sq-o4qf) landed, the ENTRYPOINT's non-loopback `--addr 0.0.0.0:3030` was REFUSED at
# startup, so every `docker run ghcr.io/sparq-org/sparq-server` from a release tag died on
# boot — and nothing caught it because no test ever actually RAN the built image. This
# script does: it runs an already-built image, then proves over the network that it serves.
#
# It deliberately does NOT build — the caller (ci.yml / release.yml) builds (or pulls) the
# image and passes its ref, so the same artifact that ships is the one tested. The runtime
# stage is distroless (no shell, no curl), so every probe is done from the HOST against the
# published port, never via `docker exec`.
#
# Usage:   scripts/docker-smoke.sh <IMAGE_REF> [HOST_PORT]
# Example: scripts/docker-smoke.sh sparq-server:smoke 3030
#
# Exit 0 iff the container boots, /health returns "ok", and a trivial SPARQL query returns
# a well-formed SPARQL-JSON result. Any failure prints the container logs and exits non-zero.
set -euo pipefail

IMAGE="${1:?usage: docker-smoke.sh <IMAGE_REF> [HOST_PORT]}"
PORT="${2:-3030}"
NAME="sparq-smoke-$$"
BASE="http://127.0.0.1:${PORT}"

cleanup() {
  # Surface logs on ANY exit path before tearing the container down — a boot failure
  # (the sq-n6rv regression) is only legible in the logs the no-shell image emits.
  echo "::group::container logs (${NAME})"
  docker logs "${NAME}" 2>&1 || true
  echo "::endgroup::"
  docker rm -f "${NAME}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo ">> running ${IMAGE} (publishing :${PORT}, no auth — default boot posture)"
# No -e SPARQ_AUTH_TOKEN: this asserts the OUT-OF-THE-BOX boot path (the bug's blast radius).
# The image's ENV SPARQ_ALLOW_REMOTE=1 is what lets the 0.0.0.0 bind proceed; if that env or
# the entrypoint regresses, the container exits and the health poll below fails fast.
docker run -d --name "${NAME}" -p "127.0.0.1:${PORT}:3030" "${IMAGE}" >/dev/null

echo ">> waiting for /health"
ok=""
for i in $(seq 1 30); do
  # If the container has already exited (the sq-n6rv failure mode), stop polling and report.
  if [ "$(docker inspect -f '{{.State.Running}}' "${NAME}" 2>/dev/null || echo false)" != "true" ]; then
    echo "!! container is not running (exited on boot) after ${i} attempt(s)" >&2
    exit 1
  fi
  body="$(curl -fsS "${BASE}/health" 2>/dev/null || true)"
  if [ "${body}" = "ok" ]; then ok=1; echo ">> /health -> ok (attempt ${i})"; break; fi
  sleep 1
done
if [ -z "${ok}" ]; then
  echo "!! /health did not return \"ok\" within 30s" >&2
  exit 1
fi

echo ">> issuing a trivial SPARQL query"
# ASK {} is the cheapest well-formed query; an empty default graph still answers it.
resp="$(curl -fsS -G "${BASE}/sparql" \
  -H 'Accept: application/sparql-results+json' \
  --data-urlencode 'query=ASK {}')"
echo "   response: ${resp}"
# The empty default graph yields a valid SPARQL-JSON ASK result; "boolean" must be present.
case "${resp}" in
  *'"boolean"'*) echo ">> query path OK" ;;
  *) echo "!! SPARQL query did not return a SPARQL-JSON boolean result" >&2; exit 1 ;;
esac

echo ">> SMOKE PASS: ${IMAGE} boots and serves /health + /sparql"
