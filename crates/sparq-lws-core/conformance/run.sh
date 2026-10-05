#!/usr/bin/env bash
# [SONNET-4.6] sq-gg0qq.7 — run the Solid Conformance Test Harness (CTH) against the EXPERIMENTAL
# `sparq-lws-core` binary. Ported from jeswr/solid-server-rs@1e555b10 `conformance/run.sh` (the repo
# `sparq-lws-core` itself was imported from, sq-gg0qq.2). See README.md for the port deltas.
#
# Boots sparq-lws-core with the IN-MEMORY store doubles (CompositeStore over InMemorySparqClient +
# InMemoryBlobStore — NO S3, NO live SPARQ; S3 is explicitly out of scope) terminating TLS in-process,
# seeded with the conformance test users (SOLID_SERVER_SEED_CONFORMANCE=1), then drives the harness's
# protocol + WAC suites against it and tears everything down.
#
# Prerequisites (all EXTERNAL to this repo — see README.md "Prerequisites"):
#   - The `solid` Keycloak realm up at http://localhost:8080/realms/solid with the conformance-alice /
#     conformance-bob DPoP service-account clients. NOTHING about that realm is modified by this script.
#   - An ath-patched CTH docker image (default `pss-cth:ath`): the published harness omits the RFC 9449
#     DPoP `ath` claim and cannot authenticate against a server that enforces it. Override CTH_IMAGE.
#   - The solid-contrib/specification-tests manifests. Unlike the source repo (which reused a sibling
#     prod-solid-server clone), this port CLONES them into a gitignored cache on first run; override
#     SPEC_TESTS to point at an existing clone.
#   - A release build of the binary (`cargo build --release -p sparq-lws-core`); override SERVER_BIN.
#
# Networking (the load-bearing part — see README.md "Networking"):
#   The server runs on the HOST and trusts/validates against a `localhost`-based issuer/audience, because
#   the verifier's SSRF guard only permits an http: issuer that resolves to LOOPBACK, and Keycloak
#   echoes its issuer from the request host (so the token `iss` MUST be `localhost:8080`). The harness
#   runs with `--network host`, which on Docker Desktop shares the Linux VM's network namespace where
#   Keycloak (a container) is reachable at `localhost:8080` and discovery returns `iss=localhost:8080`.
#   The VM cannot reach a macOS-host-bound process via `localhost`, so a `--network host` `socat`
#   sidecar forwards the VM's `localhost:3000` → the macOS host's `:3000` (host.docker.internal). The
#   net effect: harness, server, and Keycloak all agree on `localhost:3000` / `localhost:8080`, the
#   DPoP `htu` matches, and the http issuer resolves to loopback. On a NATIVE-LINUX engine `--network
#   host` is the literal host netns, so the harness already reaches the host-bound server and the
#   sidecar is NOT started — it is not a harmless passthrough there, it cannot even bind :3000 because
#   the server holds it. The script PROBES which case it is rather than assuming.
#
# Produces an EARL + HTML + summary report under conformance/reports/, then RATCHETS the generated
# score against the pinned floor in `baseline.json` (the "keep CTH green through any later change"
# standing rule). Scores are GENERATED here, never committed as prose — see SCORE.md.
#
# Result integrity (do NOT regress — the whole point of running this is a trustworthy baseline):
#   - The report dir is CLEARED before every run, so no stale report from a prior run can be mistaken
#     for this run's output.
#   - The harness exit status is CAPTURED (never `|| true`-masked), and the run is only treated as
#     valid if a FRESH EARL report (report.ttl) was actually produced by THIS run.
#   - A non-zero harness exit WITH a fresh report is a REAL result (the CTH exits non-zero when
#     scenarios fail) and is tolerated; a non-zero exit WITHOUT a fresh report is a SCRIPT/HARNESS
#     error and FAILS loudly.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
# [SONNET-4.6] PORT DELTA: the source repo was its own cargo root, so `$HERE/..` was both the crate
# and the target dir. Here the crate is `crates/sparq-lws-core` inside the sparq workspace, and cargo
# puts the binary in the WORKSPACE target dir — three levels up, not one.
CRATE="$(cd "$HERE/.." && pwd)"
WORKSPACE="$(cd "$CRATE/../.." && pwd)"
REPORTS="$HERE/reports"
CACHE="$HERE/.cache"
ENV_FILE="${ENV_FILE:-$HERE/config/sparq-lws-core.env}"
IMAGE="${CTH_IMAGE:-pss-cth:ath}"
SERVER_BIN="${SERVER_BIN:-$WORKSPACE/target/release/sparq-lws-core}"
SPEC_TESTS="${SPEC_TESTS:-$CACHE/specification-tests}"
SPEC_TESTS_REPO="${SPEC_TESTS_REPO:-https://github.com/solid-contrib/specification-tests}"
BASELINE="${CTH_BASELINE:-$HERE/baseline.json}"
CERT="$HERE/tls/server-cert.pem"
KEY="$HERE/tls/server-key.pem"
SAN_CNF="$HERE/tls/san.cnf"

BASE_URL="https://localhost:3000"
AUDIENCE="https://localhost:3000"
ISSUER="http://localhost:8080/realms/solid"
# PORT DELTA: renamed from `srs-conformance-fwd` so a stray container from a sibling solid-server-rs
# checkout on the same box is never force-removed by this script (`docker rm -f` on cleanup).
FWD_NAME="sparq-lws-cth-fwd"
# The image used BOTH for the reachability probe and (when needed) the forwarder itself, so the probe
# and the thing it gates can never disagree about what `--network host` means on this engine.
FWD_IMAGE="${CTH_FWD_IMAGE:-alpine/socat:latest}"
# PORT DELTA: the harness `--target` selects the TestSubject node in config/test-subjects.ttl, which
# this port renamed from `solid-server-rs` to the crate it now drives.
TARGET_SUBJECT="sparq-lws-core"

# Which SPARQ data-path backend to boot the server with. DEFAULT `memory` (the in-memory double — the
# byte-identical conformance baseline). Set PSS_SPARQ_BACKEND=embedded to run the same conformance
# suite against the IN-PROCESS engine over an EPHEMERAL in-memory graph (no SOLID_SERVER_SPARQ_DIR).
# PORT DELTA: the source repo needed `--features embedded-sparq` for that leg; in this workspace
# `embedded-sparq` is DEFAULT-ON (sq-gg0qq.3), so a plain release build serves both legs.
# The embedded leg seeds an ephemeral test instance, so it sets SOLID_SERVER_ALLOW_SEED_NONMEMORY=1 to
# satisfy the startup seed-guard (the guard otherwise refuses to seed a non-memory backend). The
# default memory leg is unaffected (seeding memory is always allowed).
SPARQ_BACKEND="${PSS_SPARQ_BACKEND:-memory}"

# --- pre-flight ---------------------------------------------------------------------------------
[ -x "$SERVER_BIN" ] || { echo "ERROR: server binary not found: $SERVER_BIN (run: cargo build --release -p sparq-lws-core)" >&2; exit 1; }

# PORT DELTA: the source repo COMMITTED conformance/tls/server-{cert,key}.pem. This port commits only
# the `san.cnf` that defines them and GENERATES the self-signed loopback pair on first run (both are
# gitignored). Rationale: no new long-lived private-key material enters this repo, and the SANs stay
# single-sourced in san.cnf. The generated cert is functionally identical to the source fixture.
if [ ! -f "$CERT" ] || [ ! -f "$KEY" ]; then
  command -v openssl >/dev/null 2>&1 || { echo "ERROR: openssl not found — needed to generate the self-signed conformance TLS pair from $SAN_CNF" >&2; exit 1; }
  [ -f "$SAN_CNF" ] || { echo "ERROR: $SAN_CNF missing — cannot generate the TLS pair" >&2; exit 1; }
  echo ">> Generating the self-signed conformance TLS pair (SANs from tls/san.cnf) ..."
  openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
    -keyout "$KEY" -out "$CERT" -config "$SAN_CNF" >/dev/null 2>&1 \
    || { echo "ERROR: openssl failed to generate the conformance TLS pair" >&2; exit 1; }
  chmod 600 "$KEY"
fi

# PORT DELTA: the source repo REQUIRED a sibling prod-solid-server clone of specification-tests and
# hard-failed if absent. The manifests are a public repo, so this port shallow-clones them into a
# gitignored cache instead — one fewer external prerequisite for a reproducible local run.
if [ ! -d "$SPEC_TESTS" ]; then
  echo ">> specification-tests not found at $SPEC_TESTS — cloning $SPEC_TESTS_REPO ..."
  mkdir -p "$(dirname "$SPEC_TESTS")"
  git clone --depth 1 "$SPEC_TESTS_REPO" "$SPEC_TESTS" \
    || { echo "ERROR: could not clone $SPEC_TESTS_REPO — clone it manually and set SPEC_TESTS" >&2; exit 1; }
fi
[ -d "$SPEC_TESTS" ] || { echo "ERROR: specification-tests not found at $SPEC_TESTS (override SPEC_TESTS)" >&2; exit 1; }

docker image inspect "$IMAGE" >/dev/null 2>&1 || { echo "ERROR: CTH image '$IMAGE' not present. Build the ath-patched image (see conformance/README.md 'DPoP ath') or set CTH_IMAGE." >&2; exit 1; }
# Obtain the probe/forwarder image UP FRONT. The networking probe below reads a failed `docker run` as
# "the harness netns cannot reach the server"; if the image were merely missing, that misreads as the
# VM-backed case and starts a forwarder on an engine that does not need one.
docker image inspect "$FWD_IMAGE" >/dev/null 2>&1 || docker pull "$FWD_IMAGE" >/dev/null 2>&1 \
  || { echo "ERROR: could not obtain the probe/forwarder image '$FWD_IMAGE' (override CTH_FWD_IMAGE)." >&2; exit 1; }
curl -s -m 5 "${ISSUER}/.well-known/openid-configuration" -o /dev/null || { echo "ERROR: Keycloak realm unreachable at ${ISSUER} — is the conformance IdP stack up?" >&2; exit 1; }

# Clear the report dir so a FAILED run can never leave stale report.ttl/HTML behind that then look
# like a fresh baseline. (`reports/` is gitignored — nothing committed lives here.) The marker file
# pins THIS run's start time; the freshness assertion later requires the EARL report to be newer.
rm -rf "${REPORTS:?REPORTS unset}"
mkdir -p "$REPORTS"
RUN_MARKER="$REPORTS/.run-start"
: > "$RUN_MARKER"
EARL_REPORT="$REPORTS/report.ttl"

# --- boot the server in its OWN session/process-group (load-bearing) -----------------------------
# We launch the server DETACHED into a new session (`setsid` on Linux, an `os.setsid()` python
# wrapper on macOS where `setsid` is absent) instead of a bare `&`. Why: the harness runs as
# `docker run -i --network host …`, and on Docker Desktop that `-i` foreground attach can FORWARD a
# SIGTERM to this script's process group when the container exits — which would reach the
# same-process-group server and (now that the binary handles SIGTERM with a graceful drain — and even
# before, since SIGTERM's default action is to TERMINATE the process) kill it MID-RUN, so the harness's
# very first TLS request races a shutting-down server ("Remote host terminated the handshake"). Putting
# the server in its own session means a TERM delivered to the SCRIPT's group never reaches the server;
# we tear it down explicitly in `cleanup`.
# Export the seed-guard override for the embedded leg ONLY (an `export` is robust — unlike a
# `${VAR:+ASSIGN}` env-prefix expansion, which bash parses as a COMMAND word, not an assignment).
if [ "$SPARQ_BACKEND" = "embedded" ]; then
  export SOLID_SERVER_ALLOW_SEED_NONMEMORY=1
  echo ">> SPARQ backend = EMBEDDED (in-process engine, ephemeral graph; seed-guard override set)."
else
  unset SOLID_SERVER_ALLOW_SEED_NONMEMORY
  echo ">> SPARQ backend = ${SPARQ_BACKEND} (default conformance baseline)."
fi

echo ">> Booting sparq-lws-core (in-memory doubles, TLS, seeded) at ${BASE_URL} ..."
server_env() {
  SOLID_SERVER_BIND=0.0.0.0:3000 \
  SOLID_SERVER_BASE_URL="$BASE_URL" \
  SOLID_SERVER_AUDIENCE="$AUDIENCE" \
  SOLID_SERVER_ALLOW_LOOPBACK=1 \
  SOLID_SERVER_BIDIRECTIONAL=off \
  SOLID_SERVER_TRUSTED_ISSUER="$ISSUER" \
  SOLID_SERVER_SEED_CONFORMANCE=1 \
  SOLID_SERVER_TLS_CERT="$CERT" \
  SOLID_SERVER_TLS_KEY="$KEY" \
  SOLID_SERVER_RATE_LIMIT_PER_IP=off \
  PSS_SPARQ_BACKEND="$SPARQ_BACKEND" \
  "$@"
}
# ^ SOLID_SERVER_RATE_LIMIT_PER_IP=off — disable the pre-crypto per-IP rate limiter for the harness run.
# ALL harness traffic shares ONE source IP on either networking branch: via the socat sidecar it arrives
# from a single NON-loopback Docker-VM gateway IP (host.docker.internal), and on a native-Linux host
# netns it arrives from loopback directly.
# The WAC suite's rapid PARALLEL setup bursts (many common.feature callonce iterations + pool threads,
# all one source) would otherwise drain that single IP's token bucket → 429s → false WAC failures. The
# CTH is a TRUSTED single-source load generator, so exempting it is legitimate (the limiter's actual
# per-IP protection is validated by the unit + tests/rate_limit_http.rs suites, NOT the harness).
if command -v setsid >/dev/null 2>&1; then
  server_env setsid "$SERVER_BIN" > "$REPORTS/server.log" 2>&1 &
elif command -v python3 >/dev/null 2>&1; then
  # macOS has no `setsid`; a 1-line python wrapper does the os.setsid()+exec.
  server_env python3 -c 'import os,sys; os.setsid(); os.execvp(sys.argv[1], sys.argv[1:])' "$SERVER_BIN" \
    > "$REPORTS/server.log" 2>&1 &
else
  # Last resort: bare background (the original behaviour) — still works when no TERM is forwarded.
  server_env "$SERVER_BIN" > "$REPORTS/server.log" 2>&1 &
fi
LAUNCH_PID=$!
SERVER_PID=""

# Locate OUR server process — NARROWLY. `pgrep -f "$SERVER_BIN"` substring-matches ANY process whose
# command line merely CONTAINS the binary path — e.g. `strace/perf/gdb <bin>`, `tail -f` on a
# same-named log, an editor with the file open, or a SECOND conformance/bench run from the same
# checkout — and cleanup would then TERM that innocent process's whole group (a real hazard on a
# shared box). All three launch paths above exec the binary with NO arguments, so the server's full
# command line is EXACTLY the binary path: match it exactly (-x with -f = whole-cmdline match),
# restrict to our own user (-u), and take the newest (-n) — i.e. the one just launched.
find_server_pid() {
  pgrep -n -u "$(id -un)" -fx "$SERVER_BIN" 2>/dev/null || true
}

cleanup() {
  # Prefer the PID captured right after boot; fall back to the narrow lookup. `|| true` throughout so
  # cleanup never fails the run.
  local srv="${SERVER_PID:-}"
  [ -n "$srv" ] || srv="$(find_server_pid)"
  if [ -n "$srv" ]; then
    if [ "$(ps -o sess= -p "$srv" 2>/dev/null | tr -d '[:space:]')" = "$srv" ]; then
      # The server is its own session leader (the setsid/python launch paths): TERM the whole
      # session's process group so nothing it spawned lingers.
      kill -TERM "-$srv" 2>/dev/null || true
    else
      # Bare-background fallback path: the server shares OUR process group — a group-kill would TERM
      # this script (and any sibling) too. PID-only is exact and sufficient (the server forks nothing).
      kill -TERM "$srv" 2>/dev/null || true
    fi
  fi
  kill "$LAUNCH_PID" 2>/dev/null || true
  docker rm -f "$FWD_NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

# Wait for the seeded WebID (server up + seeding done). Probe 127.0.0.1 directly (host-reachable).
for i in $(seq 1 30); do
  if curl -sk -o /dev/null -w '%{http_code}' "https://127.0.0.1:3000/alice/profile/card" 2>/dev/null | grep -q 200; then
    echo ">> Server ready (alice WebID readable)."; break
  fi
  sleep 0.5
  [ "$i" = 30 ] && { echo "ERROR: server did not become ready; log:" >&2; cat "$REPORTS/server.log" >&2; exit 1; }
done
# Pin the exact server PID for cleanup now that it is provably up (the newest exact-cmdline match by
# our user, captured immediately after OUR launch — not a pattern re-derived at teardown time).
SERVER_PID="$(find_server_pid)"

# --- the VM-side socat forwarder: VM localhost:3000 -> host :3000 (VM-BACKED ENGINES ONLY) -------
# The harness runs `--network host`, but what that MEANS differs by engine, and the difference decides
# whether a forwarder is needed AT ALL:
#   - Docker Desktop / colima / any VM-backed engine: the "host" netns is the Linux VM's. It cannot
#     reach a process bound on the real host via `localhost`, so a `--network host` socat sidecar must
#     forward the VM's localhost:3000 -> host.docker.internal:3000.
#   - NATIVE LINUX: the "host" netns is the literal host netns, so the harness ALREADY reaches the
#     server at localhost:3000. Starting socat there is NOT a harmless passthrough: the server holds
#     0.0.0.0:3000, so socat's `TCP-LISTEN:3000` fails with EADDRINUSE and the sidecar dies on the spot
#     (`reuseaddr` is SO_REUSEADDR, which does not permit a second listener on a LIVE socket). The
#     result was a silently-dead container masquerading as the load-bearing hop.
# So PROBE instead of assuming: ask a throwaway `--network host` container whether it can already open
# a TCP connection to localhost:3000, and start the forwarder ONLY when it cannot. The probe tests the
# exact property that matters, so it is correct on engines neither branch was written for.
harness_netns_reaches_server() {
  docker run --rm --network host "$FWD_IMAGE" -u /dev/null TCP:localhost:3000 >/dev/null 2>&1
}

if harness_netns_reaches_server; then
  echo ">> A --network host container already reaches the server at localhost:3000 (native-Linux host netns) — no forwarder needed."
else
  echo ">> A --network host container cannot reach localhost:3000 (VM-backed engine) — starting the socat forwarder ..."
  docker rm -f "$FWD_NAME" >/dev/null 2>&1 || true
  docker run -d --name "$FWD_NAME" --network host --add-host host.docker.internal:host-gateway \
    "$FWD_IMAGE" TCP-LISTEN:3000,fork,reuseaddr TCP:host.docker.internal:3000 >/dev/null
  sleep 1
  # A detached `docker run` exits 0 once the container STARTS, so a socat that then failed to bind
  # would sail past `set -e`. Re-probe and fail loudly rather than hand the harness a dead hop.
  harness_netns_reaches_server || {
    echo "ERROR: the socat forwarder did not make the server reachable at localhost:3000 from a --network host container." >&2
    echo "       Forwarder log:" >&2
    { docker logs "$FWD_NAME" 2>&1 || true; } >&2
    exit 1
  }
fi

# --- run the harness ----------------------------------------------------------------------------
echo ">> Running the harness (${IMAGE}) — protocol + WAC suites ..."
# Capture the harness exit status — do NOT `|| true`-mask it (that swallowed startup/config/mount
# errors and made a broken run look successful). `|| harness_rc=$?` neutralises `set -e` for this one
# command while preserving the real exit code for the validity check below.
#
# --skip-teardown: the harness writes the EARL/HTML report BEFORE its recursive-DELETE teardown, which
# hangs against this server (the published-harness teardown bug). The in-memory store is discarded
# with the server on EXIT, so per-resource teardown is dead time.
harness_rc=0
docker run -i --rm \
  --network host \
  -e ALLOW_SELF_SIGNED_CERTS=true \
  -v "$HERE/config:/app/config:ro" \
  -v "$SPEC_TESTS:/data" \
  -v "$REPORTS:/reports" \
  --env-file="$ENV_FILE" \
  "$IMAGE" \
  --output /reports \
  --target "$TARGET_SUBJECT" \
  --skip-teardown || harness_rc=$?

# --- validate the result --------------------------------------------------------------------------
# A run is only valid if a FRESH EARL report was produced by THIS invocation: report.ttl must exist
# AND be newer than the run-start marker written just before boot. If the report is missing or stale,
# the run is untrustworthy regardless of the harness exit code.
fresh_report=false
if [ -f "$EARL_REPORT" ] && [ "$EARL_REPORT" -nt "$RUN_MARKER" ]; then
  fresh_report=true
fi

if [ "$fresh_report" != true ]; then
  echo "ERROR: no FRESH EARL report at $EARL_REPORT (harness exit code: ${harness_rc})." >&2
  echo "       The harness did not produce a report for this run — treat the result as INVALID." >&2
  echo "       Server log: $REPORTS/server.log" >&2
  exit 1
fi

# Parse the per-test outcomes from the EARL report (`earl:outcome earl:passed|failed|...`).
count_outcome() { grep -cE "earl:outcome[[:space:]]+earl:$1\b" "$EARL_REPORT" || true; }
passed=$(count_outcome passed)
failed=$(count_outcome failed)
untested=$(count_outcome untested)
inapplicable=$(count_outcome inapplicable)
total=$((passed + failed + untested + inapplicable))

# A non-zero harness exit WITH a fresh report is a REAL result (the CTH exits non-zero when scenarios
# fail) — tolerate it and report the score. A zero exit is a clean pass.
echo ">> Reports in $REPORTS (report.html / report.ttl EARL). Server log: $REPORTS/server.log"
echo ">> CONFORMANCE RESULT: passed=${passed} failed=${failed} untested=${untested} inapplicable=${inapplicable} total=${total} (harness exit code: ${harness_rc})"

# --- ratchet against the pinned floor -------------------------------------------------------------
# [SONNET-4.6] PORT DELTA (sq-gg0qq.7): the source repo recorded its score as PROSE in SCORE.md, which
# drifts. Here the score is GENERATED above and compared against the machine-readable floor in
# baseline.json, so "keep CTH green through any later change" is ENFORCED rather than asserted. Set
# CTH_ENFORCE_BASELINE=0 for an exploratory run that should not fail on a regression.
if [ "${CTH_ENFORCE_BASELINE:-1}" = "0" ]; then
  echo ">> Baseline ratchet DISABLED (CTH_ENFORCE_BASELINE=0) — score reported, not enforced."
  exit 0
fi
[ -f "$BASELINE" ] || { echo "ERROR: baseline floor $BASELINE not found (override CTH_BASELINE)" >&2; exit 1; }

python3 - "$BASELINE" "$passed" "$failed" "$total" <<'PY'
import json, sys

baseline_path, passed, failed, total = sys.argv[1], *map(int, sys.argv[2:5])
with open(baseline_path, encoding="utf-8") as fh:
    base = json.load(fh)

want_passed = base["min_passed"]
want_total = base["expected_total"]
problems = []
if total != want_total:
    problems.append(
        "suite SIZE changed: expected_total=%d but the EARL report has %d cases. The manifests or the "
        "TestSubject skip tags moved — re-triage before touching the floor." % (want_total, total)
    )
if passed < want_passed:
    problems.append("REGRESSION: passed=%d is below the pinned floor min_passed=%d" % (passed, want_passed))
if failed > base["max_failed"]:
    problems.append("REGRESSION: failed=%d exceeds the pinned ceiling max_failed=%d" % (failed, base["max_failed"]))

if problems:
    print("\n>> BASELINE RATCHET FAILED", file=sys.stderr)
    for p in problems:
        print("   - " + p, file=sys.stderr)
    print("   Per-case verdicts are in the EARL report; do NOT lower the floor to go green.", file=sys.stderr)
    sys.exit(1)

print(">> BASELINE RATCHET OK: passed=%d/%d (floor %d), failed=%d (ceiling %d)."
      % (passed, total, want_passed, failed, base["max_failed"]))
if passed > want_passed:
    print(">> The score IMPROVED over the floor — raise `min_passed` in baseline.json to lock it in.")
PY
