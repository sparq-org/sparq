#!/usr/bin/env bash
# sq-eifd: cross-engine SHACL COUNT-AGREEMENT check.
#
# The headline verification for the report-cli + js-lib adapters: validate the SAME
# (data.ttl, shapes.ttl) fixture with sparq-shacl, pySHACL (report-cli) and
# rdf-validate-shacl (js-lib), reduce all three to (conforms, #violations) through
# the SHARED shacl_report_count semantics, and ASSERT they agree. Skips any engine
# that is not installed (these are gather-only deps, not committed) and only
# asserts agreement across the engines that DID run — but requires >= 2 to make a
# real cross-check.
#
# Env knobs (all optional; point at wherever you installed the gather-only engines):
#   SPARQ_VALIDATE   path to the sparq-shacl `validate` example binary
#                    (default: target/release/examples/validate)
#   PYSHACL          pyshacl CLI (default: pyshacl on PATH)
#   RDFLIB_PYTHON    a python with rdflib (for the shared parser; default python3)
#   JS_MODULES_DIR   node_modules parent for rdf-validate-shacl (default: cwd)
#   FIXTURE_DIR      data.ttl+shapes.ttl dir (default: this script's fixtures/)
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

FIXTURE_DIR="${FIXTURE_DIR:-$HERE/fixtures}"
DATA="$FIXTURE_DIR/data.ttl"
SHAPES="$FIXTURE_DIR/shapes.ttl"
SPARQ_VALIDATE="${SPARQ_VALIDATE:-$ROOT/target/release/examples/validate}"
PYSHACL="${PYSHACL:-pyshacl}"
RDFLIB_PYTHON="${RDFLIB_PYTHON:-python3}"
JS_MODULES_DIR="${JS_MODULES_DIR:-$PWD}"

COUNTER="$HERE/shacl_report_count.py"
log() { printf '%s\n' "$*" >&2; }

have() { command -v "$1" >/dev/null 2>&1; }
rdflib_ok() { "$RDFLIB_PYTHON" -c 'import rdflib' >/dev/null 2>&1; }

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
declare -A VIOL CONF
ran=0

# --- sparq-shacl (its OWN --turtle report -> shared parser) ------------------
if [ -x "$SPARQ_VALIDATE" ] && rdflib_ok; then
  "$SPARQ_VALIDATE" "$DATA" "$SHAPES" --turtle > "$tmp/sparq.ttl" || true
  out="$("$RDFLIB_PYTHON" "$COUNTER" --format turtle "$tmp/sparq.ttl")"
  VIOL[sparq]="$(printf '%s' "$out" | "$RDFLIB_PYTHON" -c 'import json,sys;print(json.load(sys.stdin)["violations"])')"
  CONF[sparq]="$(printf '%s' "$out" | "$RDFLIB_PYTHON" -c 'import json,sys;print(json.load(sys.stdin)["conforms"])')"
  ran=$((ran+1)); log "sparq-shacl : violations=${VIOL[sparq]} conforms=${CONF[sparq]}"
else
  log "sparq-shacl : SKIP (build target/release/examples/validate + need rdflib)"
fi

# --- pySHACL via the report-cli adapter --------------------------------------
if have "$PYSHACL" && rdflib_ok; then
  recipe="{\"cmd\":[\"$PYSHACL\",\"-s\",\"{shapes}\",\"-f\",\"turtle\",\"-o\",\"{report}\",\"{data}\"],\"report_format\":\"turtle\",\"report_to\":\"file\",\"version_cmd\":[\"$PYSHACL\",\"--version\"]}"
  "$RDFLIB_PYTHON" "$HERE/report_cli_adapter.py" --recipe "$recipe" \
    --data "$DATA" --shapes "$SHAPES" --engine pyshacl --json > "$tmp/py.tsv" 2>"$tmp/py.json" || true
  if [ -s "$tmp/py.tsv" ]; then
    VIOL[pyshacl]="$(cut -f2 "$tmp/py.tsv")"
    CONF[pyshacl]="$("$RDFLIB_PYTHON" -c 'import json,sys;print(json.load(open(sys.argv[1]))["conforms"])' "$tmp/py.json")"
    ran=$((ran+1)); log "pyshacl     : violations=${VIOL[pyshacl]} conforms=${CONF[pyshacl]}"
  else log "pyshacl     : SKIP (ran but produced no row — see stderr)"; fi
else
  log "pyshacl     : SKIP (pip install pyshacl + need rdflib)"
fi

# --- rdf-validate-shacl via the js-lib adapter -------------------------------
if have node && [ -d "$JS_MODULES_DIR/node_modules/rdf-validate-shacl" ]; then
  node "$HERE/js_shacl_adapter.mjs" --data "$DATA" --shapes "$SHAPES" \
    --engine rdf-validate-shacl --modules-dir "$JS_MODULES_DIR" --json \
    > "$tmp/js.tsv" 2>"$tmp/js.json" || true
  if [ -s "$tmp/js.tsv" ]; then
    VIOL[rvs]="$(cut -f2 "$tmp/js.tsv")"
    CONF[rvs]="$(node -e 'const fs=require("fs");process.stdout.write(String(JSON.parse(fs.readFileSync(process.argv[1],"utf8")).conforms))' "$tmp/js.json")"
    ran=$((ran+1)); log "rdf-validate-shacl: violations=${VIOL[rvs]} conforms=${CONF[rvs]}"
  else log "rdf-validate-shacl: SKIP (ran but produced no row — see stderr)"; fi
else
  log "rdf-validate-shacl: SKIP (npm i rdf-validate-shacl + set JS_MODULES_DIR)"
fi

# --- assert agreement across the engines that ran ----------------------------
if [ "$ran" -lt 2 ]; then
  log "NOTE: only $ran engine(s) available — install >=2 for a real cross-check."
  exit 0
fi
first_v=""; first_c=""
for e in "${!VIOL[@]}"; do
  v="${VIOL[$e]}"; c="$(printf '%s' "${CONF[$e]}" | tr '[:upper:]' '[:lower:]')"
  if [ -z "$first_v" ]; then first_v="$v"; first_c="$c"; continue; fi
  [ "$v" = "$first_v" ] || { log "DISAGREE: $e violations=$v != $first_v"; exit 1; }
  [ "$c" = "$first_c" ] || { log "DISAGREE: $e conforms=$c != $first_c"; exit 1; }
done
log "AGREE: $ran engines all report violations=$first_v conforms=$first_c"
