#!/usr/bin/env bash
# sq-hmd7l.5 — SPARQL-UPDATE same-box competitor comparison harness:
# sparq vs Fuseki/TDB2 + Oxigraph server on the PSS LDP-CRUD update set, emitting
# one canonical-competitor-results ENVELOPE per run (the exact JSON shape expected by
# the comparative-benchmarking program, research/comparative-benchmarking-everything.md
# §4 row 16).
#
# This script is the companion to bench/pss-update-set/compare.py (which does the
# per-engine HTTP measurement and store-state oracle) and follows the shacl-same-box.sh
# template: envelope JSON, TIMEOUT_S, ONLY= filtering, canonical:false off the quiet box.
#
# WORKLOAD: the PSS LDP-CRUD update set (bench/pss-update-set/compare.py): interleaved
# DELETE/INSERT/DROP operations representing a Solid pod-server's CRUD stream. All engines
# receive the SAME sequence of SPARQL UPDATE POST requests; their post-workload quad
# COUNT is cross-checked for store-state agreement BEFORE any latency row is trusted.
#
# INVARIANT (sq-hmd7l.5): after the workload, a COUNT of named-graph quads is run
# against each active engine's query endpoint. Disagreement is recorded honestly in the
# envelope (count_crosscheck.all_agree = false); numbers are never adjusted or suppressed.
#
# PARITY GATE: relative — sparq p99 ≤ competitor p99 × tolerance. No absolute wall-clock
# threshold is ever baked in (forbidden by AGENTS.md). A single-engine (smoke) run skips
# the gate.
#
# USAGE
#   scripts/bench/update-same-box.sh                     # all engines
#   ONLY=sparq scripts/bench/update-same-box.sh --smoke  # loopback sparq only (smoke)
#   ONLY="sparq fuseki" scripts/bench/update-same-box.sh
#
# TUNABLES (env; all have safe defaults):
#   ITERS       update count per engine (default 200; --smoke overrides to 5)
#   TIMEOUT_S   per-engine workload wall-clock cap, seconds (default 300)
#   ONLY        engine subset of "sparq fuseki oxigraph" (default all three)
#   OUT_DIR     envelope output dir (default /tmp/update-same-box-results;
#               a canonical run points this at bench/canonical-competitor-results/<date>/)
#   CANONICAL   1 = a dedicated quiet-box run (default 0: NON-canonical)
#   SPARQ_BIN   sparq-server binary path (default: build from workspace if not present)
#   SPARQ_PORT  loopback port for sparq-server (default 7020)
#   FUSEKI_PORT loopback port for the local Fuseki server (default 3030)
#   FUSEKI_VERSION apache-jena-fuseki version (default 5.4.0, archive.apache.org)
#   FUSEKI_DIR  unpacked Fuseki distribution root (default /tmp/jena-fuseki/...)
#   OXI_PORT    loopback port for the Oxigraph Docker container (default 7878)
#
# ENGINES NOT RUNNABLE (java missing / dist download fails for Fuseki; Docker
# unavailable or image pull fails for Oxigraph; build fails for sparq):
#   A graceful skip with status "absent" in the envelope. Never a fabricated row.
#
# FUSEKI RECIPE (sq-l7diu): the direct apache-jena-fuseki dist (archive.apache.org
# tarball + local java), NOT the stain/jena-fuseki container — the container never
# became ready within 30 s on a fresh quiet box (wave-1 canonical run sq-hmd7l.26),
# while the direct dist used by fts-same-box.sh was ready in seconds on the SAME box.
#
# SCRATCH: the Fuseki tarball and the Oxigraph image are transient gather-only deps —
# NOT committed to git. Delete them when done:
#   rm -rf /tmp/jena-fuseki && docker rmi oxigraph/oxigraph
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${HERE}/../.." && pwd)"
cd "${ROOT}"

ITERS="${ITERS:-200}"
TIMEOUT_S="${TIMEOUT_S:-300}"
ONLY="${ONLY:-sparq fuseki oxigraph}"
OUT_DIR="${OUT_DIR:-/tmp/update-same-box-results}"
CANONICAL="${CANONICAL:-0}"
SPARQ_PORT="${SPARQ_PORT:-7020}"
FUSEKI_PORT="${FUSEKI_PORT:-3030}"
FUSEKI_VERSION="${FUSEKI_VERSION:-5.4.0}"
FUSEKI_DIR="${FUSEKI_DIR:-/tmp/jena-fuseki/apache-jena-fuseki-${FUSEKI_VERSION}}"
OXI_PORT="${OXI_PORT:-7878}"
SPARQ_BIN="${SPARQ_BIN:-${ROOT}/target/release/sparq-server}"

SMOKE=0
for _arg in "$@"; do
  if [[ "${_arg}" == "--smoke" ]]; then
    SMOKE=1
  fi
done
if [[ "${SMOKE}" == "1" ]]; then
  ITERS=5
fi

log() { printf '[update-same-box] %s\n' "$*" >&2; }
want() { [[ " ${ONLY} " == *" $1 "* ]]; }
docker_ok() { docker info >/dev/null 2>&1; }

mkdir -p "${OUT_DIR}"
TMP="$(mktemp -d /tmp/update-same-box.XXXXXX)"

# --- Cleanup: stop servers on EXIT -------------------------------------------
SPARQ_PID=""
FUSEKI_PID=""
OXI_CID=""

cleanup() {
  if [[ -n "${SPARQ_PID}" ]]; then
    kill "${SPARQ_PID}" 2>/dev/null || true
  fi
  if [[ -n "${FUSEKI_PID}" ]]; then
    kill "${FUSEKI_PID}" 2>/dev/null || true
    wait "${FUSEKI_PID}" 2>/dev/null || true
  fi
  if [[ -n "${OXI_CID}" ]]; then
    docker stop "${OXI_CID}" >/dev/null 2>&1 || true
  fi
  rm -rf "${TMP}"
}
trap 'cleanup' EXIT

GIT_COMMIT="$(git -C "${ROOT}" rev-parse --short HEAD 2>/dev/null || echo unknown)"

# ---- 1. sparq ---------------------------------------------------------------
SPARQ_STATUS=absent
SPARQ_VERSION=""
if want sparq; then
  if [[ ! -x "${SPARQ_BIN}" ]]; then
    log "building sparq-server (cargo build --release -p sparq-server)..."
    cargo build --release -p sparq-server 2>&1 | tail -3 >&2
  fi
  if [[ -x "${SPARQ_BIN}" ]]; then
    log "starting sparq-server on 127.0.0.1:${SPARQ_PORT}"
    "${SPARQ_BIN}" --addr "127.0.0.1:${SPARQ_PORT}" \
      > "${TMP}/sparq-server.log" 2>&1 &
    SPARQ_PID=$!
    # probe until ready using sparq's built-in health-probe (up to 30 s)
    _ready=0
    for _i in $(seq 1 60); do
      if "${SPARQ_BIN}" --health-probe \
            --health-probe-addr "127.0.0.1:${SPARQ_PORT}" \
            >/dev/null 2>&1; then
        _ready=1
        break
      fi
      sleep 0.5
    done
    if [[ "${_ready}" == "1" ]]; then
      SPARQ_STATUS=ok
      SPARQ_VERSION="${GIT_COMMIT}"
      log "sparq-server ready (pid ${SPARQ_PID})"
    else
      log "sparq-server did not become ready in 30 s; status=absent"
      kill "${SPARQ_PID}" 2>/dev/null || true
      SPARQ_PID=""
    fi
  else
    log "sparq-server binary not found after build; status=absent"
  fi
fi

# ---- 2. Fuseki (direct apache-jena-fuseki dist; sq-l7diu) -------------------
FUSEKI_STATUS=absent
FUSEKI_VER=""
if want fuseki; then
  FUSEKI_SKIP=""
  if ! command -v java >/dev/null 2>&1; then
    FUSEKI_SKIP="java not available"
  fi
  if [[ -z "${FUSEKI_SKIP}" && ! -f "${FUSEKI_DIR}/fuseki-server.jar" ]]; then
    log "downloading apache-jena-fuseki ${FUSEKI_VERSION} to /tmp/jena-fuseki (gather-only dep)"
    mkdir -p /tmp/jena-fuseki
    if ! curl -sSfL -o "/tmp/jena-fuseki/apache-jena-fuseki-${FUSEKI_VERSION}.tar.gz" \
        "https://archive.apache.org/dist/jena/binaries/apache-jena-fuseki-${FUSEKI_VERSION}.tar.gz" \
        || ! tar xzf "/tmp/jena-fuseki/apache-jena-fuseki-${FUSEKI_VERSION}.tar.gz" -C /tmp/jena-fuseki; then
      FUSEKI_SKIP="fuseki download/unpack failed"
    fi
  fi
  if [[ -z "${FUSEKI_SKIP}" ]]; then
    log "starting Fuseki on 127.0.0.1:${FUSEKI_PORT} (in-memory /ds, updates enabled)"
    mkdir -p "${TMP}/fuseki-base"
    FUSEKI_BASE="${TMP}/fuseki-base" java -jar "${FUSEKI_DIR}/fuseki-server.jar" \
      --port="${FUSEKI_PORT}" --update --mem /ds \
      > "${TMP}/fuseki.log" 2>&1 &
    FUSEKI_PID=$!
    # probe the Fuseki ping endpoint (up to 30 s)
    _ready=0
    for _i in $(seq 1 60); do
      if curl -sf --max-time 1 \
            "http://127.0.0.1:${FUSEKI_PORT}/\$/ping" \
            >/dev/null 2>&1; then
        _ready=1
        break
      fi
      sleep 0.5
    done
    if [[ "${_ready}" == "1" ]]; then
      FUSEKI_STATUS=ok
      FUSEKI_VER="apache-jena-fuseki-${FUSEKI_VERSION}"
      log "Fuseki ready (pid ${FUSEKI_PID})"
    else
      log "Fuseki did not become ready in 30 s; status=absent. Log tail:"
      tail -20 "${TMP}/fuseki.log" >&2 || true
      kill "${FUSEKI_PID}" 2>/dev/null || true
      FUSEKI_PID=""
    fi
  else
    log "fuseki SKIP: ${FUSEKI_SKIP}; status=absent"
  fi
fi

# ---- 3. Oxigraph (Docker / oxigraph/oxigraph) --------------------------------
OXI_STATUS=absent
OXI_VERSION=""
OXI_IMAGE="oxigraph/oxigraph"
if want oxigraph; then
  if docker_ok; then
    log "pulling ${OXI_IMAGE} (if not cached)..."
    if timeout 120 docker pull "${OXI_IMAGE}" >/dev/null 2>&1; then
      log "starting Oxigraph on 127.0.0.1:${OXI_PORT}"
      OXI_CID="$(docker run -d --rm \
        -p "127.0.0.1:${OXI_PORT}:7878" \
        "${OXI_IMAGE}" 2>/dev/null)"
      # probe the root endpoint (up to 30 s)
      _ready=0
      for _i in $(seq 1 60); do
        if curl -sf --max-time 1 "http://127.0.0.1:${OXI_PORT}/" \
              >/dev/null 2>&1; then
          _ready=1
          break
        fi
        sleep 0.5
      done
      if [[ "${_ready}" == "1" ]]; then
        OXI_STATUS=ok
        OXI_VERSION="${OXI_IMAGE}"
        log "Oxigraph ready (container ${OXI_CID})"
      else
        log "Oxigraph did not become ready in 30 s; status=absent"
        docker stop "${OXI_CID}" >/dev/null 2>&1 || true
        OXI_CID=""
      fi
    else
      log "docker pull ${OXI_IMAGE} failed; oxigraph status=absent"
    fi
  else
    log "Docker not available; oxigraph status=absent"
  fi
fi

# ---- 4. Build compare.py invocation -----------------------------------------
COMPARE_ARGS=()
if [[ "${SPARQ_STATUS}" == "ok" ]]; then
  COMPARE_ARGS+=(--sparq "http://127.0.0.1:${SPARQ_PORT}/sparql")
fi
if [[ "${FUSEKI_STATUS}" == "ok" ]]; then
  COMPARE_ARGS+=(--fuseki "http://127.0.0.1:${FUSEKI_PORT}/ds")
fi
if [[ "${OXI_STATUS}" == "ok" ]]; then
  COMPARE_ARGS+=(--oxigraph "http://127.0.0.1:${OXI_PORT}")
fi

# ---- 5. Run the workload through compare.py ----------------------------------
COMPARE_JSON="${TMP}/compare-out.json"
COMPARE_LOG="${TMP}/compare.log"
COMPARE_EXIT=0

if [[ "${#COMPARE_ARGS[@]}" -gt 0 ]]; then
  if [[ "${SPARQ_STATUS}" != "ok" ]]; then
    # compare.py defaults --sparq ON; an empty URL disables it so an ONLY= subset
    # without sparq (or a failed sparq start) doesn't abort the whole workload.
    COMPARE_ARGS+=(--sparq "")
  fi
  log "running compare.py: iters=${ITERS}, engines=[${COMPARE_ARGS[*]}]"
  set +e
  timeout "${TIMEOUT_S}" \
      python3 "${ROOT}/bench/pss-update-set/compare.py" \
      "${ITERS}" "${COMPARE_ARGS[@]}" \
      --json-out "${COMPARE_JSON}" \
      > "${COMPARE_LOG}" 2>&1
  COMPARE_EXIT=$?
  set -e
  if [[ "${COMPARE_EXIT}" -ne 0 ]]; then
    log "compare.py exited non-zero or timed out (see ${COMPARE_LOG})"
    # A parity-gate failure is measured output: compare.py writes its
    # complete JSON summary before exiting non-zero. Preserve that summary and use
    # the stub only for a crash/timeout that left no valid machine-readable result.
    if ! python3 -c 'import json, sys; json.load(open(sys.argv[1], encoding="utf-8"))' \
        "${COMPARE_JSON}" >/dev/null 2>&1; then
      printf '{"latency_ms":{},"throughput_s":{},"state_count":{},"state_agree":null,"parity_gate":"error","parity_note":"compare.py failed"}\n' \
        > "${COMPARE_JSON}"
    fi
  fi
  cat "${COMPARE_LOG}" >&2
else
  log "no engines available; skipping workload"
  printf '{"latency_ms":{},"throughput_s":{},"state_count":{},"state_agree":null,"parity_gate":"no-engines","parity_note":"all engines absent"}\n' \
    > "${COMPARE_JSON}"
fi

# ---- 6. Stop servers before assembling the envelope -------------------------
if [[ -n "${SPARQ_PID}" ]]; then
  kill "${SPARQ_PID}" 2>/dev/null || true
  SPARQ_PID=""
fi
if [[ -n "${FUSEKI_PID}" ]]; then
  kill "${FUSEKI_PID}" 2>/dev/null || true
  wait "${FUSEKI_PID}" 2>/dev/null || true
  FUSEKI_PID=""
fi
if [[ -n "${OXI_CID}" ]]; then
  docker stop "${OXI_CID}" >/dev/null 2>&1 || true
  OXI_CID=""
fi

# ---- 7. Assemble the envelope (canonical-competitor-results JSON shape) -----
TS="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="${OUT_DIR}/update-${TS}.json"

SPARQ_STATUS="${SPARQ_STATUS}" \
SPARQ_VERSION="${SPARQ_VERSION}" \
FUSEKI_STATUS="${FUSEKI_STATUS}" \
FUSEKI_VERSION="${FUSEKI_VER}" \
OXI_STATUS="${OXI_STATUS}" \
OXI_VERSION="${OXI_VERSION}" \
ITERS="${ITERS}" \
GIT_COMMIT="${GIT_COMMIT}" \
CANONICAL="${CANONICAL}" \
TIMEOUT_S="${TIMEOUT_S}" \
COMPARE_JSON="${COMPARE_JSON}" \
OUT="${OUT}" \
python3 - <<'PYEOF'
import json
import os
import platform

canonical = os.environ["CANONICAL"] == "1"
note_canonical = (
    "CANONICAL: dedicated quiet box, one engine active at a time on the SAME"
    " PSS LDP-CRUD update set; min-of-N at each scale; per-engine workload cap recorded."
    if canonical else
    "NON-canonical FIRST READ: shared work box (not a dedicated quiet instance)."
    " Timings are directional only — do NOT bake into docs/dashboards."
    " The harness (scripts/bench/update-same-box.sh) is the durable deliverable;"
    " rerun with CANONICAL=1 on a dedicated quiet EC2 box for citable numbers (sq-hmd7l.26)."
)

cmp_path = os.environ["COMPARE_JSON"]
cmp = json.load(open(cmp_path)) if os.path.exists(cmp_path) else {}

statuses = {
    "sparq": os.environ["SPARQ_STATUS"],
    "fuseki": os.environ["FUSEKI_STATUS"],
    "oxigraph": os.environ["OXI_STATUS"],
}
versions = {
    "sparq": os.environ["SPARQ_VERSION"],
    "fuseki": os.environ["FUSEKI_VERSION"],
    "oxigraph": os.environ["OXI_VERSION"],
}

engines_meta = {}
for e, status in statuses.items():
    if status == "ok":
        engines_meta[e] = {
            "status": status,
            "version": versions[e],
            "update_endpoint": cmp.get("latency_ms", {}).get(e) and (
                f"http://127.0.0.1:<port>/sparql" if e == "sparq"
                else f"http://127.0.0.1:<port>/ds/update" if e == "fuseki"
                else f"http://127.0.0.1:<port>/update"
            ) or "(see log)",
        }
    else:
        engines_meta[e] = {"status": status}

# count_crosscheck: per-engine quad count post-workload
state_counts = cmp.get("state_count", {})
non_error = {e: v for e, v in state_counts.items() if not str(v).startswith("ERROR")}
all_agree = len(set(non_error.values())) <= 1 if len(non_error) >= 2 else None
count_crosscheck = dict(state_counts)
if all_agree is not None:
    count_crosscheck["all_agree"] = all_agree

env = {
    "host": platform.node(),
    "machine": platform.machine(),
    "os": platform.platform(),
    "timeout_s": int(os.environ["TIMEOUT_S"]),
}

envelope = {
    "gather": "update-same-box-comparison",
    "wave": "update-competitor-baseline (sq-hmd7l.5)",
    "canonical": canonical,
    "canonical_note": note_canonical,
    "git_commit": os.environ["GIT_COMMIT"],
    "suite": "pss-update-parity",
    "iters": int(os.environ["ITERS"]),
    "tsv_format": "<engine>\\t<p50_ms>\\t<p99_ms>\\t<max_ms>\\t<thr_per_s>",
    "engines": engines_meta,
    "statuses": statuses,
    "latency_ms": cmp.get("latency_ms", {}),
    "throughput_s": cmp.get("throughput_s", {}),
    "count_crosscheck": count_crosscheck,
    "count_crosscheck_note": (
        "named-graph quad COUNT after the full PSS update workload, per engine."
        " all_agree = every engine that produced a non-ERROR count agrees."
        " A disagreement is recorded HONESTLY (never adjusted); this is why COUNT"
        " is checked before trusting any latency row (sq-hmd7l.5 INVARIANT)."
    ),
    "parity_gate": cmp.get("parity_gate", "not-run"),
    "parity_note": cmp.get("parity_note", ""),
    "parity_gate_note": (
        "RELATIVE gate only (no absolute wall-clock threshold):"
        " sparq p99 ≤ competitor p99 × tolerance. Single-engine runs skip the gate."
    ),
    "env": env,
}

out_path = os.environ["OUT"]
with open(out_path, "w", encoding="utf-8") as fh:
    json.dump(envelope, fh, indent=2)
    fh.write("\n")
print(out_path)
PYEOF

log "envelope: ${OUT}"
exit "${COMPARE_EXIT}"
