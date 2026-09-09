#!/usr/bin/env bash
# [OPUS-4.8] gather-competitors.sh (sq-t0c3) — authored by Opus 4.8 (Fable
# unavailable; flag for re-review when Fable returns).
#
# Orchestrates gathering competitor-engine numbers for the perf dashboard's
# side-by-side comparison, recording each competitor's VERSION + ENVIRONMENT
# alongside its results. The registry of competitors (pinned versions, recipes,
# which sparq suites each is comparable on) is bench/competitors.json.
#
# SAFE BY DEFAULT — this script DRY-RUNS unless you pass --run:
#   * dry-run (default)  : prints exactly what WOULD happen per engine + a tool
#                          availability + env report. Runs NO benchmark, pulls
#                          NO image, builds NO dataset. Always safe.
#   * --run              : actually executes the gather for the selected engines.
#                          Heavy. Caps dataset size, watches `df`, cleans /tmp.
#
# Engines: oxigraph (embedded Rust dep) | qlever (Docker) | eye (N3 binary).
# Plus five SHARED reusable adapter KINDS (sq-eifd / sq-1fz0), dispatched off
# competitors.json `kind` and backed by scripts/bench-adapters/:
#   report-cli  : file-in/SHACL-report-out CLI (pySHACL, Jena `shacl validate`)
#   js-lib      : in-process Node/RDF-JS SHACL (rdf-validate-shacl, sparq's own WASM)
#   http-sparql : POST query -> parse SPARQL-JSON -> count (Fuseki, Virtuoso, QLever)
#   vector-lib  : in-process Python ANN -> recall@k vs exact-kNN (FAISS, hnswlib)
#   python-lib  : in-process IR kernel -> Recall@100/nDCG@10 deficits on a BEIR cut
#                 (lucene-anserini, the FTS IR-quality oracle; sq-1fz0)
# Select with --only <id[,id...]>; default is all engines in dry-run, but --run
# requires you to NAME the engines (no accidental "run everything heavy").
#
# Usage:
#   scripts/gather-competitors.sh                      # dry-run, all engines (safe)
#   scripts/gather-competitors.sh --only oxigraph      # dry-run, one engine
#   scripts/gather-competitors.sh --run --only oxigraph [--scale N] [--iters K]
#   scripts/gather-competitors.sh --run --only eye
#   scripts/gather-competitors.sh --run --only qlever --suite olympics [--pass compute|endtoend]
#   scripts/gather-competitors.sh --list               # list registry + exit
#   scripts/gather-competitors.sh --help
#
# qlever --pass selects the compare.py pass: `compute` (engine-only, the default)
# or `endtoend` (compute + full SPARQL-JSON serialisation).
#
# What a real (--run) gather does, per engine:
#   oxigraph : cargo run -p sparq-bench --release -- --scale N --iters K
#              (the compare path already times sparq AND Oxigraph + cross-checks
#              solution counts). Records the resolved oxigraph version from
#              Cargo.lock + the host env block.
#   eye      : delegates to bench/inference/eye-comparison.sh (sparq vs EYE N3
#              closure, min-of-N wall seconds). Records `eye --version` + env.
#   qlever   : delegates to bench/qlever-<suite>/compare.py against a running
#              QLever server (Docker image). Records the resolved image DIGEST
#              (the version pin) + env. compare.py does not report a server build
#              string, so the digest IS the version identifier; --run refuses a
#              live gather when the digest cannot be resolved (no Docker / image
#              not pulled). Requires Docker + a started server; the script tells
#              you the exact prep commands rather than starting a multi-GB server.
#
# Output (only with --run): one JSON results file per engine/suite under
#   bench/competitor-results/  (GIT-IGNORED, regenerable — see bench/CATALOG.md
#   disk discipline). The dashboard SEAM (engines/values) in bench/competitors.json
#   is committed ONLY deliberately by a maintainer from a reviewed results file —
#   this script never edits the tracked JSON.
#
# Disk discipline (per bench/CATALOG.md): caps the synthetic --scale, runs a `df`
# watchdog (aborts when free space < MIN_FREE_GB), uses mktemp -d + trap-clean for
# scratch, and never deletes tracked files.

set -euo pipefail

# --- locate repo root (works from a worktree too) ----------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

REGISTRY="bench/competitors.json"
RESULTS_DIR="bench/competitor-results"
# [OPUS-4.8] sq-eifd: shared external-engine adapter KINDS live here. Each kind is
# a small parser/harness reused across engines; dispatch is off competitors.json's
# `kind` field (see the "adapter-kind dispatch" section below).
ADAPTERS_DIR="scripts/bench-adapters"

# --- defaults / knobs --------------------------------------------------------
DO_RUN=0                 # 0 = dry-run (safe default); 1 = actually run
ONLY=""                  # comma list of engine ids; empty = all
SUITE="olympics"         # which qlever-* suite (olympics|synthetic|100m)
PASS="compute"           # qlever compare.py pass (compute|endtoend)
SCALE="${SCALE:-20000}"  # oxigraph synthetic entities (capped below)
ITERS="${ITERS:-3}"      # min-of-K
MAX_SCALE="${MAX_SCALE:-200000}"   # hard cap on --scale (disk/cpu guard)
MIN_FREE_GB="${MIN_FREE_GB:-10}"   # df watchdog floor
EYE_BIN="${EYE:-eye}"
SPARQ_CLI="${SPARQ_CLI:-target/release/sparq-cli}"

usage() {
  # print the header comment block (lines 2 .. the line before `set -euo pipefail`)
  local last; last="$(grep -n '^set -euo pipefail' "${BASH_SOURCE[0]}" | head -1 | cut -d: -f1)"
  sed -n "2,$((last - 2))p" "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

log()  { printf '%s\n' "$*" >&2; }
hdr()  { printf '\n=== %s ===\n' "$*" >&2; }
warn() { printf 'WARN: %s\n' "$*" >&2; }
die()  { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

have() { command -v "$1" >/dev/null 2>&1; }

# require_int <flag-name> <value> [min] — fail with a clear message (not a raw
# `integer expression expected` from a later `[ ... -gt ... ]`) when a numeric
# knob is empty / non-numeric (digits only — these knobs are all non-negative) /
# below a floor.
require_int() {
  local name="$1" val="$2" min="${3:-}"
  case "$val" in
    ''|*[!0-9]*) die "$name must be a non-negative integer (got '${val}')" ;;
  esac
  if [ -n "$min" ] && [ "$val" -lt "$min" ]; then
    die "$name must be >= $min (got '$val')"
  fi
}

# need_val <flag> <count-remaining> — clear error when a value-taking flag has no
# argument. Without it, the inner `shift` would consume the flag and the loop's
# trailing `shift` would then run on an empty arg list, which under `set -e`
# aborts the script SILENTLY (no message) — the same cryptic failure mode the
# numeric guards below avoid.
need_val() { [ "$2" -ge 2 ] || die "$1 requires a value (e.g. $1 <value>)"; }

# --- arg parse ---------------------------------------------------------------
while [ $# -gt 0 ]; do
  case "$1" in
    --run)    DO_RUN=1 ;;
    --only)   need_val "$1" "$#"; ONLY="$2"; shift ;;
    --only=*) ONLY="${1#*=}" ;;
    --suite)  need_val "$1" "$#"; SUITE="$2"; shift ;;
    --suite=*) SUITE="${1#*=}" ;;
    --pass)   need_val "$1" "$#"; PASS="$2"; shift ;;
    --pass=*) PASS="${1#*=}" ;;
    --scale)  need_val "$1" "$#"; SCALE="$2"; shift ;;
    --scale=*) SCALE="${1#*=}" ;;
    --iters)  need_val "$1" "$#"; ITERS="$2"; shift ;;
    --iters=*) ITERS="${1#*=}" ;;
    --list)   ACTION_LIST=1 ;;
    -h|--help) usage 0 ;;
    *) die "unknown arg: $1 (try --help)" ;;
  esac
  shift
done

# --- validate numeric knobs up front -----------------------------------------
# Without this, an empty/non-numeric value (e.g. `--scale` with no argument, or
# SCALE=abc) would reach a `[ ... -gt ... ]` and, under `set -e`, abort with a
# cryptic `integer expression expected` instead of a clear message.
require_int "--scale"   "$SCALE"       1
require_int "--iters"   "$ITERS"       1
require_int "MAX_SCALE" "$MAX_SCALE"   1
require_int "MIN_FREE_GB" "$MIN_FREE_GB" 0

# --- validate the qlever compare.py pass -------------------------------------
case "$PASS" in
  compute|endtoend) ;;
  *) die "--pass must be 'compute' or 'endtoend' (got '$PASS')" ;;
esac

# --- cap the synthetic scale (disk/cpu guard) --------------------------------
if [ "$SCALE" -gt "$MAX_SCALE" ]; then
  warn "--scale $SCALE exceeds MAX_SCALE $MAX_SCALE; capping to $MAX_SCALE (raise MAX_SCALE to override)"
  SCALE="$MAX_SCALE"
fi

# --- registry presence -------------------------------------------------------
[ -f "$REGISTRY" ] || die "registry $REGISTRY not found (run from a sparq checkout)"
if have jq; then
  jq . "$REGISTRY" >/dev/null || die "$REGISTRY does not parse as JSON"
fi

want() { # want <id> -> 0 if this engine is selected
  [ -z "$ONLY" ] && return 0
  case ",$ONLY," in *",$1,"*) return 0 ;; *) return 1 ;; esac
}

# --- df watchdog -------------------------------------------------------------
free_gb() { df -BG --output=avail . 2>/dev/null | tail -1 | tr -dc '0-9'; }
check_df() {
  local f; f="$(free_gb || true)"
  [ -n "$f" ] || { warn "could not read free disk space; skipping df guard"; return 0; }
  log "disk: ${f} GB free (floor ${MIN_FREE_GB} GB)"
  [ "$f" -ge "$MIN_FREE_GB" ] || die "free disk ${f} GB < floor ${MIN_FREE_GB} GB — aborting (per bench disk discipline)"
}

# --- host env block (recorded in every results file) -------------------------
env_json() {
  local cpu nproc_n os kernel host_class quiet
  cpu="$(LC_ALL=C grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2- | sed 's/^ *//' || true)"
  [ -n "$cpu" ] || cpu="$(uname -p 2>/dev/null || echo unknown)"
  nproc_n="$(nproc 2>/dev/null || echo 0)"
  os="$(uname -s 2>/dev/null || echo unknown)"
  kernel="$(uname -r 2>/dev/null || echo unknown)"
  # host_class: best-effort label — "github-runner" if GITHUB_ACTIONS, else hostname.
  if [ "${GITHUB_ACTIONS:-}" = "true" ]; then host_class="github-runner"; else host_class="$(hostname 2>/dev/null || echo local)"; fi
  # quiet_box: only EC2/dedicated quiet boxes give trustworthy absolute wall-clock.
  # Normalize to a strict JSON boolean — QUIET_BOX is emitted UNQUOTED below, so a
  # non-boolean value (e.g. QUIET_BOX=1) would otherwise produce invalid JSON.
  # Treat 1/true/yes/on (any case) as true; everything else (incl. unset) as false.
  case "$(printf '%s' "${QUIET_BOX:-}" | tr '[:upper:]' '[:lower:]')" in
    1|true|yes|on) quiet="true" ;;
    *)             quiet="false" ;;
  esac
  printf '{"host_class":"%s","cpu_model":"%s","nproc":%s,"os":"%s","kernel":"%s","quiet_box":%s,"gathered_at_utc":"%s","git_commit":"%s"}' \
    "$host_class" "$cpu" "${nproc_n:-0}" "$os" "$kernel" "$quiet" \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
}

# --- versions per engine -----------------------------------------------------
oxigraph_version() {
  # exact resolved version from Cargo.lock; fall back to the Cargo.toml pin.
  local v=""
  if [ -f Cargo.lock ]; then
    v="$(awk '/^name = "oxigraph"$/{f=1} f&&/^version = /{gsub(/[",]/,"",$3);print $3;exit}' Cargo.lock || true)"
  fi
  [ -n "$v" ] || v="$(have jq && jq -r '.competitors[]|select(.id=="oxigraph").pinned_version' "$REGISTRY" || echo unknown)"
  printf '%s' "$v"
}
eye_version() { "$EYE_BIN" --version 2>/dev/null | head -1 || echo "not-installed"; }
qlever_image_digest() {
  have docker || { echo "docker-not-installed"; return; }
  docker inspect --format '{{index .RepoDigests 0}}' docker.io/adfreiburg/qlever:latest 2>/dev/null \
    || echo "image-not-pulled"
}

mkdir_results() { mkdir -p "$RESULTS_DIR"; }
stamp() { date -u +%Y%m%dT%H%M%SZ; }

# write a results envelope: write_result <engine> <suite> <version> <payload-json>
write_result() {
  local engine="$1" suite="$2" version="$3" payload="$4"
  mkdir_results
  local out
  out="$RESULTS_DIR/${engine}-${suite}-$(stamp).json"
  printf '{"engine":"%s","suite":"%s","version":"%s","env":%s,"result":%s}\n' \
    "$engine" "$suite" "$version" "$(env_json)" "$payload" > "$out"
  log "wrote $out"
  if have jq; then jq . "$out" >/dev/null || warn "$out did not validate as JSON"; fi
}

# =============================================================================
# --list
# =============================================================================
if [ "${ACTION_LIST:-0}" = 1 ]; then
  if have jq; then
    jq -r '.competitors[] | "\(.id)\t\(.kind)\t\(.pinned_version)\n  comparable: \([.comparable_suites[].sparq_suite]|join(", "))"' "$REGISTRY"
  else
    cat "$REGISTRY"
  fi
  exit 0
fi

# =============================================================================
# env + tooling report (always shown)
# =============================================================================
hdr "host environment"
log "$(env_json)"

hdr "tooling availability"
for t in cargo docker python3 jq "$EYE_BIN"; do
  if have "$t"; then log "  [present] $t"; else log "  [MISSING] $t"; fi
done

if [ "$DO_RUN" -eq 1 ]; then
  hdr "MODE: --run (live)  selected=${ONLY:-<none — refusing, see below>}"
  [ -n "$ONLY" ] || die "--run requires --only <id[,id...]> (refusing to run every heavy gather by default)"
  check_df
else
  hdr "MODE: dry-run (default — safe). Pass --run --only <id> to execute."
fi

# =============================================================================
# oxigraph (embedded Rust dep)
# =============================================================================
if want oxigraph; then
  hdr "oxigraph (embedded-rust-dep)"
  OXI_V="$(oxigraph_version)"
  log "  version (resolved): $OXI_V"
  log "  would run: cargo run -p sparq-bench --release -- --scale $SCALE --iters $ITERS"
  log "  comparable sparq suites: sparq-bench-compare, sp2b, watdiv, bsbm, dbpsb, lubm(extensional)"
  if [ "$DO_RUN" -eq 1 ]; then
    have cargo || die "cargo not found — cannot run the oxigraph harness"
    log "  running sparq-bench compare (scale=$SCALE iters=$ITERS)…"
    OUT="$(cargo run -p sparq-bench --release -- --scale "$SCALE" --iters "$ITERS" 2>&1)" \
      || die "sparq-bench failed:\n$OUT"
    # Store the raw harness stdout as the payload (the compare table is the result;
    # a maintainer maps it into the dashboard seam deliberately — no auto-commit).
    PAYLOAD="$(printf '%s' "$OUT" | python3 -c 'import json,sys; print(json.dumps({"raw": sys.stdin.read()}))' 2>/dev/null \
      || printf '{"raw":"see stderr"}')"
    write_result oxigraph "sparq-bench-compare" "$OXI_V" "$PAYLOAD"
    check_df
  fi
fi

# =============================================================================
# eye (N3 reasoner binary)
# =============================================================================
if want eye; then
  hdr "eye (binary — N3 reasoner)"
  EYE_V="$(eye_version)"
  log "  version: $EYE_V  (binary: $EYE_BIN)"
  log "  would run: EYE=$EYE_BIN SPARQ_CLI=$SPARQ_CLI bench/inference/eye-comparison.sh"
  log "  comparable sparq suites: inference-eye-comparison (socrates/DeepTaxonomy/anc500/grid30)"
  if [ "$DO_RUN" -eq 1 ]; then
    have "$EYE_BIN" || die "eye binary '$EYE_BIN' not found — set EYE=/path/to/eye (see bench/competitors.json install_recipe)"
    [ -x "$SPARQ_CLI" ] || die "sparq-cli not built at '$SPARQ_CLI' — run: cargo build -p sparq-cli --release"
    log "  running eye-comparison.sh (this skips EYE on dt100k unless EYE_DT100K=1)…"
    OUT="$(EYE="$EYE_BIN" SPARQ_CLI="$SPARQ_CLI" bash bench/inference/eye-comparison.sh 2>&1)" \
      || die "eye-comparison.sh failed:\n$OUT"
    PAYLOAD="$(printf '%s' "$OUT" | python3 -c 'import json,sys; print(json.dumps({"raw": sys.stdin.read()}))' 2>/dev/null \
      || printf '{"raw":"see stderr"}')"
    write_result eye "inference-eye-comparison" "$EYE_V" "$PAYLOAD"
    check_df
  fi
fi

# =============================================================================
# qlever (Docker image)
# =============================================================================
if want qlever; then
  hdr "qlever (docker-image)"
  Q_DIGEST="$(qlever_image_digest)"
  log "  image digest: $Q_DIGEST"
  log "  suite: bench/qlever-${SUITE}  (olympics|synthetic|100m)"
  log "  PREP (you run these — heavy, multi-GB, NOT auto-started here):"
  log "    cd bench/qlever-${SUITE} && qlever get-data && qlever index && qlever start"
  log "  would run: python3 bench/qlever-${SUITE}/compare.py $ITERS $PASS"
  log "  comparable sparq suites: qlever-${SUITE}, watdiv, bsbm, lubm(extensional)"
  if [ "$DO_RUN" -eq 1 ]; then
    have python3 || die "python3 not found — needed by compare.py"
    CMP="bench/qlever-${SUITE}/compare.py"
    [ -f "$CMP" ] || die "no harness for suite '$SUITE' ($CMP missing); valid: olympics|synthetic|100m"
    [ -x "$SPARQ_CLI" ] || die "sparq-cli not built at '$SPARQ_CLI' — run: cargo build -p sparq-cli --release"
    # The DIGEST is the only version identifier we can record (compare.py reports
    # no server build string), so a result with a sentinel digest would not be
    # "versioned". Refuse a live gather unless the digest resolved — fail fast
    # with the actionable prep instead of writing an unversioned result file.
    case "$Q_DIGEST" in
      docker-not-installed)
        die "qlever --run needs Docker to resolve the image digest (the version pin); install Docker, then: docker pull docker.io/adfreiburg/qlever:latest" ;;
      image-not-pulled)
        die "qlever image not pulled — cannot record a version digest. Run: docker pull docker.io/adfreiburg/qlever:latest  (then start the server per PREP above)" ;;
    esac
    # We do NOT pull/start a multi-GB QLever server automatically. compare.py needs
    # a server already running on its PORT; it fails fast (HTTP error) if not.
    warn "qlever gather assumes a QLever server is already running for suite '$SUITE'"
    warn "  (run the PREP commands above first). compare.py will error out if not."
    OUT="$(python3 "$CMP" "$ITERS" "$PASS" 2>&1)" \
      || { warn "compare.py failed (server not up? see PREP above):\n$OUT"; OUT="FAILED: $OUT"; }
    PAYLOAD="$(printf '%s' "$OUT" | python3 -c 'import json,sys; print(json.dumps({"raw": sys.stdin.read()}))' 2>/dev/null \
      || printf '{"raw":"see stderr"}')"
    write_result qlever "qlever-${SUITE}-${PASS}" "$Q_DIGEST" "$PAYLOAD"
    check_df
  fi
fi

# =============================================================================
# adapter-kind dispatch (sq-eifd) — the SHARED reusable kinds, dispatched off the
# competitors.json `kind` field instead of one bespoke recipe per engine.
#
#   report-cli   : file-in / SHACL-validation-report-out CLI (pySHACL, Jena
#                  `shacl validate`, TopBraid). Shared report->count parser
#                  (count sh:result, read sh:conforms) so #violations is directly
#                  comparable to sparq's report.results.len().  [REQUIRED by sq-7iai]
#   js-lib       : in-process Node/RDF-JS harness (rdf-validate-shacl, shacl-engine,
#                  sparq's own @sparq-org/sparq WASM SHACL) — same count semantics.
#   http-sparql  : POST query -> parse SPARQL-JSON -> count (Fuseki/Virtuoso + the
#                  QLever HTTP path). End-to-end gather-only (needs a live server);
#                  parser fixture-verified here.
#   vector-lib   : in-process Python ANN harness (FAISS/hnswlib) — recall@k vs an
#                  exact-kNN oracle, recall-deficit TSV. End-to-end gather-only
#                  (needs numpy/hnswlib); recall scorer fixture-verified here.
#
# Each new-kind registry entry carries an `adapter` object (the recipe). The gather
# box installs the (gather-only, NOT committed) engine and supplies the data/shapes
# (or endpoint/dataset) via the flags below. Per AGENTS.md the engines + the
# resulting numbers stay OUT of git — this dispatch just runs the shared harness and
# writes a git-ignored results file like the other kinds.
#
# Env inputs (per-kind; only consulted under --run):
#   report-cli / js-lib : SHACL_DATA + SHACL_SHAPES (paths to the data+shapes graphs)
#   http-sparql         : SPARQL_ENDPOINT + SPARQL_QUERY_FILE
#   vector-lib          : VECTOR_NPZ (npz with {data,queries}) [hnswlib mode]
# =============================================================================
adapter_kind_of() { # adapter_kind_of <id> -> the engine's `kind` from the registry
  have jq || { echo ""; return; }
  # `first(...)` so we emit EXACTLY one line (the matching engine's kind), not one
  # empty line per non-matching competitor.
  jq -r --arg id "$1" 'first(.competitors[]|select(.id==$id)).kind // ""' "$REGISTRY"
}
adapter_recipe_of() { # the engine's `.adapter` recipe object as compact JSON
  have jq || { echo "{}"; return; }
  jq -c --arg id "$1" 'first(.competitors[]|select(.id==$id)).adapter // {}' "$REGISTRY"
}

# [OPUS-4.8] sq-8dp3: SHACL_SHAPES is a DIRECTORY in the committed suite
# (bench/shacl/shapes/ — five hand-authored shape graphs, one workload each).
# sparq-shacl validates the data against EACH shape file separately and
# expected.tsv records a per-workload (conforms,violations,focus_nodes). An
# external engine handed the *directory* as a single graph arg loads NO shapes and
# reports conforms:true / 0 violations — the apples-to-oranges bug the prior gather
# hit. So: when SHACL_SHAPES is a directory, expand it per `*.ttl` shape file and
# run the adapter once per workload (the SAME per-shape decomposition sparq uses),
# keyed by the shape-file stem; when it is a single file, run it once (workload="").
# Echoes each chosen shape path so the apples-to-apples expansion is auditable.
shacl_shape_files() { # <shapes-path> -> one path per line (the file itself, or dir/*.ttl)
  local p="$1"
  if [ -d "$p" ]; then
    # [OPUS-4.8] sq-8dp3: fail CLOSED if a shapes dir expands to zero *.ttl files.
    # A silently-empty expansion makes the per-shape loop a no-op and produces ZERO
    # result envelopes — exactly the "0 violations / missing-shapes" apples-to-oranges
    # class of bug this expansion exists to prevent. Better to die loudly than to
    # emit no comparison at all.
    local files; files="$(find "$p" -maxdepth 1 -type f -name '*.ttl' | LC_ALL=C sort)"
    [ -n "$files" ] || die "SHACL_SHAPES dir '$p' contains no *.ttl shape files — refusing to run a no-op comparison"
    printf '%s\n' "$files"
  else
    printf '%s\n' "$p"
  fi
}
shacl_workload_of() { # <shape-path> -> workload stem (basename minus .ttl), or "" for a non-dir single file
  # [OPUS-4.8] sq-8dp3: use a literal string comparison, NOT `case` — `case` performs
  # glob matching, so a single-file shapes path containing glob metacharacters
  # (e.g. '[', '*', '?') could mis-match the pattern. `[ x = y ]` is an exact match.
  if [ "${SHACL_SHAPES:-}" = "$1" ]; then
    printf ''                                       # single-file shapes: no per-workload split
  else
    local b; b="$(basename "$1")"; printf '%s' "${b%.ttl}"
  fi
}

run_report_cli_engine() { # <id>
  local id="$1" recipe; recipe="$(adapter_recipe_of "$id")"
  hdr "$id (report-cli)"
  log "  shared report->count parser: $ADAPTERS_DIR/shacl_report_count.py (count sh:result, read sh:conforms)"
  log "  recipe: $recipe"
  if [ "$DO_RUN" -eq 1 ]; then
    have python3 || die "python3 needed for the report-cli adapter"
    have jq || die "jq needed for the report-cli adapter (to parse the --json sidecar)"  # [OPUS-4.8] match js-lib's jq guard
    python3 -c 'import rdflib' 2>/dev/null || die "report-cli adapter needs rdflib (pip install rdflib)"
    { [ -n "${SHACL_DATA:-}" ] && [ -n "${SHACL_SHAPES:-}" ]; } || die "report-cli --run needs SHACL_DATA + SHACL_SHAPES"
    [ -d "$SHACL_SHAPES" ] && log "  SHACL_SHAPES is a directory — expanding per shape file (one workload each, matching sparq-shacl)"
    local rcver=""
    local shape workload
    while IFS= read -r shape; do
      [ -n "$shape" ] || continue
      workload="$(shacl_workload_of "$shape")"
      log "  shape: $shape  (workload=${workload:-<single>})"
      # [OPUS-4.8] The adapter's --json sidecar (stderr) carries the RESOLVED engine
      # version + the conforms bit; capture it (not /dev/null) so the results
      # envelope records what the engine actually reported, per the script header
      # and the adapter design — rather than the pinned_version placeholder.
      local errf; errf="$(mktemp)"
      OUT="$(python3 "$ADAPTERS_DIR/report_cli_adapter.py" --recipe "$recipe" \
              --data "$SHACL_DATA" --shapes "$shape" --engine "$id" --iters "$ITERS" --json 2>"$errf")" \
        || { rm -f "$errf"; die "report-cli adapter failed for $id (shape $shape)"; }
      PAYLOAD="$(tail -n1 "$errf" | jq -c --arg w "$workload" '{engine,version,conforms,violations,validate_us,workload:$w}' 2>/dev/null)"
      rm -f "$errf"
      # [OPUS-4.8] sq-8dp3: the fallback (when the --json sidecar can't be parsed) must
      # ALSO carry the workload field, so a parse-failure result is still attributed to
      # its shape/workload — matching the primary path above. Pass $workload as argv[1].
      [ -n "$PAYLOAD" ] || PAYLOAD="$(printf '%s' "$OUT" | python3 -c 'import json,sys; e,v,u=sys.stdin.read().split(); print(json.dumps({"engine":e,"violations":int(v),"validate_us":int(u),"workload":sys.argv[1]}))' "$workload")"
      [ -n "$rcver" ] || rcver="$(printf '%s' "$PAYLOAD" | jq -r '.version // empty' 2>/dev/null)"
      [ -n "$rcver" ] || rcver="$(jq -r --arg id "$id" 'first(.competitors[]|select(.id==$id)).pinned_version//"unknown"' "$REGISTRY")"
      write_result "$id" "shacl-validate-bench${workload:+-$workload}" "$rcver" "$PAYLOAD"
    done < <(shacl_shape_files "$SHACL_SHAPES")
    check_df
  fi
}

run_js_lib_engine() { # <id>
  local id="$1" recipe; recipe="$(adapter_recipe_of "$id")"
  local jsengine; jsengine="$(printf '%s' "$recipe" | jq -r '.js_engine // "rdf-validate-shacl"')"
  hdr "$id (js-lib node-rdfjs)"
  log "  node harness: $ADAPTERS_DIR/js_shacl_adapter.mjs (engine=$jsengine; same count semantics as report-cli)"
  if [ "$DO_RUN" -eq 1 ]; then
    have node || die "node needed for the js-lib adapter"
    have jq || die "jq needed for the js-lib adapter (to build the result payload)"
    { [ -n "${SHACL_DATA:-}" ] && [ -n "${SHACL_SHAPES:-}" ]; } || die "js-lib --run needs SHACL_DATA + SHACL_SHAPES"
    local mdir; mdir="${JS_MODULES_DIR:-$PWD}"
    [ -d "$SHACL_SHAPES" ] && log "  SHACL_SHAPES is a directory — expanding per shape file (one workload each, matching sparq-shacl)"
    local shape workload
    while IFS= read -r shape; do
      [ -n "$shape" ] || continue
      workload="$(shacl_workload_of "$shape")"
      log "  shape: $shape  (workload=${workload:-<single>})"
      # [OPUS-4.8] Build the payload from the adapter's --json sidecar (stderr,
      # well-formed JSON) via jq — not a python3 TSV reparse (python3 was never
      # required for js-lib) and not a printf %s of raw $OUT (whose unescaped
      # trailing newline/tab would emit invalid JSON and fail write_result's
      # validation under set -euo pipefail).
      local errf; errf="$(mktemp)"
      OUT="$(node "$ADAPTERS_DIR/js_shacl_adapter.mjs" --data "$SHACL_DATA" --shapes "$shape" \
              --engine "$jsengine" --modules-dir "$mdir" --iters "$ITERS" --json 2>"$errf")" \
        || { rm -f "$errf"; die "js-lib adapter failed for $id (npm i $jsengine in $mdir? shape $shape)"; }
      PAYLOAD="$(tail -n1 "$errf" | jq -c --arg w "$workload" '{engine,conforms,violations,validate_us,workload:$w}' 2>/dev/null)"
      rm -f "$errf"
      [ -n "$PAYLOAD" ] || die "js-lib adapter produced no parseable --json sidecar for $id (shape $shape)"
      write_result "$id" "shacl-validate-bench${workload:+-$workload}" "$jsengine" "$PAYLOAD"
    done < <(shacl_shape_files "$SHACL_SHAPES")
    check_df
  fi
}

run_http_sparql_engine() { # <id>
  local id="$1"
  hdr "$id (http-sparql)"
  log "  adapter: $ADAPTERS_DIR/http_sparql_adapter.py (POST query -> parse SPARQL-JSON -> count)"
  log "  PREP (gather box): start the engine's HTTP endpoint, then set SPARQL_ENDPOINT + SPARQL_QUERY_FILE"
  # [OPUS-4.8] sq-gbq0: a Docker-backed http-sparql engine (Fuseki, Virtuoso, the
  # jena-* SAILs) records its IMAGE DIGEST as the version identifier (the endpoint
  # reports no build string), and tags the result per SUITE so the file lands as
  # <engine>-<suite>-<UTC>.json per bench/CATALOG.md (e.g. fuseki-sp2b-<UTC>.json).
  # Both are OPTIONAL env knobs, backward-compatible: default suite stays
  # "http-sparql" and the version falls back to the registry pinned_version.
  log "  OPTIONAL: SPARQL_SUITE=<sp2b|watdiv|lubm|bsbm|dbpsb> tags the result file; SPARQL_IMAGE=<image:tag> records the resolved Docker DIGEST as the version"
  if [ "$DO_RUN" -eq 1 ]; then
    have python3 || die "python3 needed for the http-sparql adapter"
    { [ -n "${SPARQL_ENDPOINT:-}" ] && [ -n "${SPARQL_QUERY_FILE:-}" ]; } || die "http-sparql --run needs SPARQL_ENDPOINT + SPARQL_QUERY_FILE"
    OUT="$(python3 "$ADAPTERS_DIR/http_sparql_adapter.py" --endpoint "$SPARQL_ENDPOINT" \
            --query-file "$SPARQL_QUERY_FILE" --engine "$id" --iters "$ITERS" --json 2>/dev/null)" \
      || die "http-sparql adapter failed for $id (endpoint up?)"
    PAYLOAD="$(printf '%s' "$OUT" | python3 -c 'import json,sys; e,c,u=sys.stdin.read().split(); print(json.dumps({"engine":e,"count":int(c),"query_us":int(u)}))')"
    # Version: a resolved Docker image DIGEST (SPARQL_IMAGE) IS the version pin for a
    # Dockerized server; fall back to the registry pinned_version when not Dockerized.
    local hver=""
    if [ -n "${SPARQL_IMAGE:-}" ] && have docker; then
      hver="$(docker inspect --format '{{index .RepoDigests 0}}' "$SPARQL_IMAGE" 2>/dev/null || true)"
    fi
    [ -n "$hver" ] || hver="$(jq -r --arg id "$id" 'first(.competitors[]|select(.id==$id)).pinned_version//"unknown"' "$REGISTRY")"
    write_result "$id" "${SPARQL_SUITE:-http-sparql}" "$hver" "$PAYLOAD"
    check_df
  fi
}

run_vector_lib_engine() { # <id>
  local id="$1"
  hdr "$id (vector-lib)"
  log "  adapter: $ADAPTERS_DIR/vector_lib_adapter.py (build index -> query -> recall@k vs exact-kNN oracle)"
  # [OPUS-4.8] sq-aiup: TWO gather modes.
  #   * SINGLE-POINT (default): VECTOR_NPZ (npz with {data,queries}) -> one recall@k point.
  #   * PARETO (gather-tier, the published-dataset recall-QPS curve): set VECTOR_DATASET +
  #     VECTOR_ROOT to sweep `ef` over SIFT1M (sift-128-euclidean) or glove-100-angular and
  #     emit the recall-QPS Pareto at MATCHED recall (never a single latency). The corpora
  #     are NOT redistributable in-repo (download/gather step) — design §3.3(c).
  log "  PREP single-point: pip install numpy hnswlib; set VECTOR_NPZ to an npz with {data,queries}"
  log "  PREP Pareto (sq-aiup): pip install numpy hnswlib [h5py for glove]; download SIFT1M/GloVe under"
  log "    VECTOR_ROOT (SIFT: <root>/sift/sift_{base,query}.fvecs + sift_groundtruth.ivecs;"
  log "    GloVe: <root>/glove-100-angular.hdf5), then set VECTOR_DATASET=sift-128-euclidean|glove-100-angular"
  if [ "$DO_RUN" -eq 1 ]; then
    have python3 || die "python3 needed for the vector-lib adapter"
    python3 -c 'import numpy, hnswlib' 2>/dev/null || die "vector-lib adapter needs numpy + hnswlib (pip install numpy hnswlib)"
    # [FABLE-5] sq-5o5.4: record the resolved gather-box distribution, not the
    # registry's intentionally version-agnostic placeholder.
    local vector_version
    vector_version="$(python3 -c 'import importlib.metadata; print(importlib.metadata.version("hnswlib"))')" \
      || die "could not resolve the installed hnswlib distribution version"
    if [ -n "${VECTOR_DATASET:-}" ]; then
      # PARETO mode: published-dataset recall-QPS curve (sq-aiup).
      [ -n "${VECTOR_ROOT:-}" ] || die "vector-lib Pareto --run needs VECTOR_ROOT (dir holding the SIFT1M/GloVe corpus)"
      log "  Pareto: dataset=$VECTOR_DATASET root=$VECTOR_ROOT ef=${VECTOR_EF:-16,32,64,128,256} k=${VECTOR_K:-10} match_recall=${VECTOR_MATCH_RECALL:-0.9}"
      local errf; errf="$(mktemp)"
      # stdout = the per-ef recall-QPS TSV rows; stderr = the --json Pareto envelope.
      python3 "$ADAPTERS_DIR/vector_lib_adapter.py" --pareto --dataset "$VECTOR_DATASET" \
        --root "$VECTOR_ROOT" --ef "${VECTOR_EF:-16,32,64,128,256}" --k "${VECTOR_K:-10}" \
        --match-recall "${VECTOR_MATCH_RECALL:-0.9}" --engine "$id" --json >/dev/null 2>"$errf" \
        || { rm -f "$errf"; die "vector-lib Pareto adapter failed for $id (dataset $VECTOR_DATASET under $VECTOR_ROOT)"; }
      PAYLOAD="$(tail -n1 "$errf" | jq -c . 2>/dev/null)"
      rm -f "$errf"
      [ -n "$PAYLOAD" ] || die "vector-lib Pareto produced no parseable --json envelope for $id"
      write_result "$id" "vectors-recall-qps-${VECTOR_DATASET}" "$vector_version" "$PAYLOAD"
    else
      [ -n "${VECTOR_NPZ:-}" ] || die "vector-lib --run needs VECTOR_NPZ (npz with {data,queries}) or VECTOR_DATASET+VECTOR_ROOT (Pareto)"
      OUT="$(python3 "$ADAPTERS_DIR/vector_lib_adapter.py" --hnswlib --npz "$VECTOR_NPZ" --k "${VECTOR_K:-10}" --engine "$id" --json 2>/dev/null)" \
        || die "vector-lib adapter failed for $id"
      PAYLOAD="$(printf '%s' "$OUT" | python3 -c 'import json,sys; e,d,u=sys.stdin.read().split(); print(json.dumps({"engine":e,"recall_deficit_milli":int(d),"query_us":int(u)}))')"
      write_result "$id" "vectors-recall" "$vector_version" "$PAYLOAD"
    fi
    check_df
  fi
}

# beir_deficits_json <tsv> — fold the adapter's `<engine>\t<metric>\t<deficit_milli>`
# lines (recall_at_100, ndcg_at_10) into ONE `{engine, deficits_milli:{metric:val}}`
# JSON object, so each engine's results envelope is a single comparable record (the
# same deficit shape the vector-lib path emits).
beir_deficits_json() {
  printf '%s' "$1" | python3 -c 'import json,sys
m={}; e=None
for ln in sys.stdin.read().splitlines():
    if not ln.strip(): continue
    e,k,v=ln.split("\t"); m[k]=int(v)
print(json.dumps({"engine":e,"deficits_milli":m}))'
}

# [OPUS-4.8] sq-1fz0: PYTHON-LIB kind — the BEIR IR-quality axis of the FTS suite
# (design §3.4). The ONLY python-lib engine is lucene-anserini, the kernel BM25
# ORACLE: it downloads a small BEIR cut (SciFact / TREC-COVID + qrels), BM25-retrieves
# with Anserini/pyserini (k1=1.2/b=0.75, matching sparq-text), and scores Recall@100 /
# nDCG@10, emitted as DEFICITS (G4) on the SAME cut + qrels sparq-text is scored on.
# GATHER-ONLY: the BEIR corpus is not redistributable in-repo, so this is a download
# step, never a committed per-PR gate. The scoring half is fixture-unit-tested in
# scripts/bench-adapters/test_adapters.py WITHOUT pyserini/beir installed.
run_python_lib_engine() { # <id>
  local id="$1"
  hdr "$id (python-lib — BEIR IR-quality oracle)"
  log "  adapter: $ADAPTERS_DIR/beir_ir_adapter.py (download BEIR cut -> Anserini BM25 retrieve -> Recall@100/nDCG@10 deficits)"
  log "  scoring half is fixture-unit-tested in $ADAPTERS_DIR/test_adapters.py (no pyserini/beir needed)"
  log "  PREP (gather box): pip install pyserini beir; set BEIR_CUT (scifact|trec-covid); optional BEIR_DATA_DIR"
  log "  OPTIONAL apples-to-apples: set SPARQ_TEXT_BEIR=target/release/examples/beir_text to ALSO"
  log "    score sparq-text on the SAME cut+qrels (cargo build -p sparq-text --release --example beir_text)"
  log "  OFF the dashboard (no RDF/SPARQL surface) — the kernel BM25 reference, 'sub-component, not an RDF benchmark'"
  if [ "$DO_RUN" -eq 1 ]; then
    have python3 || die "python3 needed for the python-lib (BEIR) adapter"
    python3 -c 'import pyserini, beir' 2>/dev/null || die "python-lib adapter needs pyserini + beir (pip install pyserini beir)"
    local cut; cut="${BEIR_CUT:-scifact}"
    # 1. the kernel BM25 ORACLE: Anserini retrieve + score on the cut + qrels.
    OUT="$(python3 "$ADAPTERS_DIR/beir_ir_adapter.py" --anserini --dataset "$cut" --k "${BEIR_K:-100}" --engine "$id" --json 2>/dev/null)" \
      || die "python-lib (BEIR) adapter failed for $id (cut '$cut'; pyserini index downloadable?)"
    PAYLOAD="$(beir_deficits_json "$OUT")"
    write_result "$id" "beir-ir-${cut}" "$(jq -r --arg id "$id" 'first(.competitors[]|select(.id==$id)).pinned_version//"unknown"' "$REGISTRY")" "$PAYLOAD"
    # 2. OPTIONAL: score sparq-text on the SAME cut+qrels via the shared scorer, so the
    #    comparison is apples-to-apples (same scorer, same judgements; design §3.4).
    if [ -n "${SPARQ_TEXT_BEIR:-}" ]; then
      [ -x "$SPARQ_TEXT_BEIR" ] || die "SPARQ_TEXT_BEIR='$SPARQ_TEXT_BEIR' is not an executable (build the beir_text example)"
      local prep; prep="$(mktemp -d)"
      python3 "$ADAPTERS_DIR/beir_ir_adapter.py" --prepare-sparq --dataset "$cut" --out "$prep" \
        || { rm -rf "$prep"; die "could not prepare sparq inputs for cut '$cut'"; }
      "$SPARQ_TEXT_BEIR" "$prep/corpus.nt" "$prep/queries.tsv" "${BEIR_K:-100}" sparq-text > "$prep/run.txt" \
        || { rm -rf "$prep"; die "sparq-text beir_text retrieval failed for cut '$cut'"; }
      local SOUT
      SOUT="$(python3 "$ADAPTERS_DIR/beir_ir_adapter.py" --score --run "$prep/run.txt" --qrels "$prep/qrels.txt" --k "${BEIR_K:-100}" --engine sparq-text 2>/dev/null)" \
        || { rm -rf "$prep"; die "could not score the sparq-text run for cut '$cut'"; }
      write_result "sparq-text" "beir-ir-${cut}" "$(git rev-parse --short HEAD 2>/dev/null || echo unknown)" "$(beir_deficits_json "$SOUT")"
      rm -rf "$prep"
    fi
    check_df
  fi
}

# Walk every registry engine of a SHARED-ADAPTER kind (skipping the three bespoke
# kinds handled above) and dispatch. Engines stay empty in git, so this is a no-op
# until a maintainer adds report-cli/js-lib/http-sparql/vector-lib/python-lib entries.
if have jq; then
  while IFS= read -r aid; do
    [ -n "$aid" ] || continue
    want "$aid" || continue
    case "$(adapter_kind_of "$aid")" in
      report-cli)  run_report_cli_engine  "$aid" ;;
      js-lib)      run_js_lib_engine      "$aid" ;;
      http-sparql) run_http_sparql_engine "$aid" ;;
      vector-lib)  run_vector_lib_engine  "$aid" ;;
      python-lib)  run_python_lib_engine  "$aid" ;;
    esac
  done < <(jq -r '.competitors[] | select(.kind=="report-cli" or .kind=="js-lib" or .kind=="http-sparql" or .kind=="vector-lib" or .kind=="python-lib") | .id' "$REGISTRY")
fi

hdr "done"
if [ "$DO_RUN" -eq 0 ]; then
  log "Dry-run only — nothing was executed. To gather for real:"
  log "  scripts/gather-competitors.sh --run --only oxigraph --scale 50000 --iters 4"
  log "  scripts/gather-competitors.sh --run --only eye"
  log "  scripts/gather-competitors.sh --run --only qlever --suite olympics --pass compute   (server must be up)"
  log "Results land (git-ignored) in $RESULTS_DIR/ ; commit a dashboard snapshot"
  log "into bench/competitors.json (engines/values) only deliberately, from a reviewed result."
fi
