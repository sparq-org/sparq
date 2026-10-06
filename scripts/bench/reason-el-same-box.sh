#!/usr/bin/env bash
# sq-hmd7l.8 (epic sq-hmd7l) — same-box OWL 2 EL CLASSIFICATION comparison:
# sparq-reason-el vs ELK (the canonical consequence-based EL classifier, Apache-2.0) on
# real ontologies (Gene Ontology + OpenGALEN), emitting one competitor-results ENVELOPE
# per ontology. Mirrors scripts/bench/shacl-same-box.sh (the SHACL gather recipe) and the
# canonical-competitor-results JSON shape.
#
# ORACLE-BEFORE-TIMING (the sq-hmd7l.8 INVARIANT). For every ontology BOTH engines' proper
# named subsumption COUNT is recorded and cross-checked FIRST; a timing row is emitted ONLY
# when both counts are present. NEITHER engine is ground truth — a disagreement is recorded
# with `counts_agree=false` and investigated, NEVER silently adjusted. Work-box timings are
# NON-canonical (canonical:false always) — the harness is the durable deliverable; a future
# dedicated quiet-box run sets CANONICAL=1.
#
# METRIC. "proper named subsumption pairs" = the COMPLETE transitive closure of C ⊑ D over
# named classes (C ≠ D, D ≠ owl:Thing, both named IRIs). sparq reports it directly
# (examples/reason_el_real_bench GATHER mode); ELK's inferred DIRECT taxonomy is transitively
# closed in the count step so both engines are compared on the SAME closure notion.
#
# --smoke  (ONLY=sparq): the fast, hermetic ACCEPTANCE path — build + run the sparq example
#          on the small VENDORED fixture (crates/sparq-reason-el/examples/data/el_smoke.ttl),
#          asserting its pinned subsumption count. NO downloads, NO JVM, NO network.
#              ONLY=sparq bash scripts/bench/reason-el-same-box.sh --smoke
#
# FULL MODE (gather; needs network + a JRE + riot for OWL→NT):
#              bash scripts/bench/reason-el-same-box.sh            # GO + OpenGALEN, both engines
#              REASON_EL_ONTOLOGIES=go ONLY=sparq  bash scripts/bench/reason-el-same-box.sh
#
# TUNABLES (env; all have safe defaults):
#   REASON_EL_ONTOLOGIES  space list of ontology keys        (default "go opengalen")
#   ONLY                  engine subset of "sparq elk"       (default "sparq elk")
#   OUT_DIR               envelope dir (default /tmp/reason-el-same-box-results; a canonical
#                         run points this at bench/canonical-competitor-results/<date>/)
#   CANONICAL             1 = dedicated quiet-box run        (default 0: NON-canonical)
#   TIMEOUT_S             per-engine classify cap, seconds   (default 1800)
#   ELK_VERSION           ELK CLI distribution version       (default 0.6.0)
#   ELK_JAR               path to elk.jar (auto-downloaded to /tmp if unset)
#   JENA_VERSION          apache-jena for `riot` OWL→NT      (default 5.4.0)
#   JENA_HOME             jena dist root (auto-downloaded to /tmp if unset)
#   GO_OWL_URL            Gene Ontology OWL URL              (default go-basic.owl release)
#   OPENGALEN_OWL_URL     OpenGALEN OWL URL                  (default OpenGALEN 8 full)
#
# Ontology OWL + the ELK/Jena jars are gather-only deps under /tmp — NOT committed (engines
# + big corpora stay out of git per AGENTS.md). Delete when done:
#   rm -rf /tmp/reason-el-gather /tmp/elk-cli /tmp/jena-riot
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

ONLY="${ONLY:-sparq elk}"
OUT_DIR="${OUT_DIR:-/tmp/reason-el-same-box-results}"
CANONICAL="${CANONICAL:-0}"
TIMEOUT_S="${TIMEOUT_S:-1800}"
REASON_EL_ONTOLOGIES="${REASON_EL_ONTOLOGIES:-go opengalen}"
ELK_VERSION="${ELK_VERSION:-0.6.0}"
JENA_VERSION="${JENA_VERSION:-5.4.0}"
JENA_HOME="${JENA_HOME:-/tmp/jena-riot/apache-jena-$JENA_VERSION}"
GO_OWL_URL="${GO_OWL_URL:-http://purl.obolibrary.org/obo/go/go-basic.owl}"
OPENGALEN_OWL_URL="${OPENGALEN_OWL_URL:-https://www.opengalen.org/download/OpenGALEN8-OWL.zip}"

log() { printf '[reason-el-same-box] %s\n' "$*" >&2; }
want() { [[ " $ONLY " == *" $1 "* ]]; }

# ---- --smoke: the hermetic, sparq-only acceptance path -----------------------
if [[ "${1:-}" == "--smoke" ]]; then
  log "SMOKE: sparq-only, pinned vendored fixture (no network/JVM)"
  cargo build --release -p sparq-reason-el --example reason_el_real_bench
  "$ROOT/target/release/examples/reason_el_real_bench" --smoke
  log "SMOKE OK"
  exit 0
fi

mkdir -p "$OUT_DIR"
GATHER="/tmp/reason-el-gather"
mkdir -p "$GATHER"
GIT_COMMIT="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"

# ---- 0. engines --------------------------------------------------------------
SPARQ_BIN="$ROOT/target/release/examples/reason_el_real_bench"
if want sparq; then
  log "building sparq reason_el_real_bench (--features rbox for real role chains)"
  # rbox: real biomedical ontologies (GO part_of, SNOMED-style chains) need the role box for a
  # complete closure. The example's metric is unchanged in feature-OFF; rbox only ADDS edges the
  # ontology's role axioms entail. Built with rbox so the real-ontology count is faithful.
  cargo build --release -p sparq-reason-el --features rbox --example reason_el_real_bench
fi

if want elk; then
  ELK_JAR="${ELK_JAR:-/tmp/elk-cli/elk-standalone.jar}"
  if [ ! -f "$ELK_JAR" ]; then
    mkdir -p /tmp/elk-cli
    log "downloading ELK $ELK_VERSION CLI to /tmp/elk-cli (Apache-2.0, gather-only)"
    curl -sSL -o /tmp/elk-cli/elk.zip \
      "https://github.com/liveontologies/elk-reasoner/releases/download/v$ELK_VERSION/elk-distribution-cli-$ELK_VERSION.zip" || {
        log "ELK download failed — set ELK_JAR to a local elk-standalone.jar and rerun"; }
    if [ -f /tmp/elk-cli/elk.zip ]; then
      unzip -oq /tmp/elk-cli/elk.zip -d /tmp/elk-cli || true
      found_jar="$(find /tmp/elk-cli -name 'elk-standalone*.jar' | head -1 || true)"
      [ -n "$found_jar" ] && cp "$found_jar" "$ELK_JAR"
    fi
  fi
  if [ ! -f "$ELK_JAR" ]; then
    log "ELK jar unavailable ($ELK_JAR) — ELK columns will record an honest ERROR"
  else
    ELK_VER="$(java -jar "$ELK_JAR" --version 2>/dev/null | head -1 || echo "elk-$ELK_VERSION")"
    log "ELK: $ELK_VER"
  fi
fi

# riot (Apache Jena) converts each OWL ontology → N-Triples for sparq's parser.
if [ ! -x "$JENA_HOME/bin/riot" ]; then
  log "downloading apache-jena $JENA_VERSION to /tmp/jena-riot (for riot OWL→NT, gather-only)"
  mkdir -p /tmp/jena-riot
  curl -sSL -o "/tmp/jena-riot/apache-jena-$JENA_VERSION.tar.gz" \
    "https://archive.apache.org/dist/jena/binaries/apache-jena-$JENA_VERSION.tar.gz"
  tar xzf "/tmp/jena-riot/apache-jena-$JENA_VERSION.tar.gz" -C /tmp/jena-riot
fi
RIOT="$JENA_HOME/bin/riot"

# ---- 1. resolve each ontology to a local OWL file ----------------------------
resolve_owl() {
  case "$1" in
    go)        echo "$GO_OWL_URL" ;;
    opengalen) echo "$OPENGALEN_OWL_URL" ;;
    *)         echo "" ;;
  esac
}

for ONT in $REASON_EL_ONTOLOGIES; do
  URL="$(resolve_owl "$ONT")"
  [ -z "$URL" ] && { log "unknown ontology key '$ONT' — skipping"; continue; }
  log "=== ontology: $ONT ($URL) ==="

  OWL="$GATHER/$ONT.owl"
  NT="$GATHER/$ONT.nt"
  if [ ! -f "$OWL" ]; then
    log "downloading $ONT OWL (gather-only)"
    if [[ "$URL" == *.zip ]]; then
      curl -sSL -o "$GATHER/$ONT.zip" "$URL"
      (cd "$GATHER" && unzip -oq "$ONT.zip" && \
        find . -maxdepth 2 -iname '*.owl' | head -1 | xargs -I{} cp {} "$OWL")
    else
      curl -sSL -o "$OWL" "$URL"
    fi
  fi
  if [ ! -f "$OWL" ]; then log "$ONT OWL unavailable — skipping"; continue; fi
  OWL_SHA="$(sha256sum "$OWL" | cut -d' ' -f1)"

  if want sparq && [ ! -f "$NT" ]; then
    log "riot: converting $ONT OWL → N-Triples"
    "$RIOT" --output=ntriples "$OWL" > "$NT" 2>"$GATHER/$ONT.riot.err" || {
      log "riot conversion failed (see $GATHER/$ONT.riot.err) — skipping $ONT"; continue; }
  fi

  # -- 2a. sparq: subsumption count + classify time (count printed FIRST) -------
  SPARQ_ROW=""; SPARQ_COUNT=""; SPARQ_S=""
  if want sparq; then
    log "sparq: classify $ONT (cap ${TIMEOUT_S}s)"
    if timeout "$TIMEOUT_S" "$SPARQ_BIN" "$NT" ntriples > "$GATHER/$ONT.sparq.out" 2>&1; then
      SPARQ_ROW="$(cat "$GATHER/$ONT.sparq.out")"
      SPARQ_COUNT="$(sed -n 's/.*subsumptions=\([0-9]*\).*/\1/p' "$GATHER/$ONT.sparq.out")"
      SPARQ_S="$(sed -n 's/.*classify_s=\([0-9.]*\).*/\1/p' "$GATHER/$ONT.sparq.out")"
    else
      log "sparq FAILED/timeout on $ONT"; SPARQ_ROW="ERROR: timeout/failure"
    fi
  fi

  # -- 2b. ELK: classify → inferred taxonomy → transitively-closed named count --
  ELK_ROW=""; ELK_COUNT=""; ELK_S=""
  if want elk && [ -f "${ELK_JAR:-/nonexistent}" ]; then
    log "elk: classify $ONT (cap ${TIMEOUT_S}s)"
    ELK_OUT="$GATHER/$ONT.elk-taxonomy.owl"
    START="$(python3 -c 'import time;print(f"{time.time():.6f}")')"
    if timeout "$TIMEOUT_S" java -jar "$ELK_JAR" \
        --input "$OWL" --output "$ELK_OUT" classify 2>"$GATHER/$ONT.elk.err"; then
      END="$(python3 -c 'import time;print(f"{time.time():.6f}")')"
      ELK_S="$(python3 -c "print(f'{$END-$START:.6f}')")"
      # Transitively close ELK's inferred DIRECT SubClassOf taxonomy over named classes and count
      # proper pairs — aligning ELK's metric with sparq's complete-closure count.
      "$RIOT" --output=ntriples "$ELK_OUT" > "$GATHER/$ONT.elk.nt" 2>/dev/null || true
      ELK_COUNT="$(ELK_NT="$GATHER/$ONT.elk.nt" python3 - <<'PYEOF'
import os, sys
sco = "<http://www.w3.org/2000/01/rdf-schema#subClassOf>"
thing = "<http://www.w3.org/2002/07/owl#Thing>"
sup = {}
path = os.environ["ELK_NT"]
if not os.path.exists(path):
    print(""); sys.exit(0)
for line in open(path):
    p = line.rstrip(" .\n").split(" ", 2)
    if len(p) == 3 and p[1] == sco and p[0].startswith("<") and p[2].startswith("<"):
        s, o = p[0], p[2]
        if s != o and o != thing:
            sup.setdefault(s, set()).add(o)
# transitive closure over the direct taxonomy
closure = {c: set(v) for c, v in sup.items()}
changed = True
while changed:
    changed = False
    for c in list(closure):
        add = set()
        for d in closure[c]:
            add |= sup.get(d, set())
        add -= closure[c]; add.discard(c)
        if add:
            closure[c] |= add; changed = True
print(sum(len(v) for v in closure.values()))
PYEOF
)"
    else
      log "elk FAILED/timeout on $ONT"; ELK_ROW="ERROR: timeout/failure"
    fi
  elif want elk; then
    ELK_ROW="ERROR: elk jar unavailable"
  fi

  # -- 3. ORACLE-BEFORE-TIMING: record counts + agreement, THEN timing ---------
  COUNTS_AGREE="n/a"
  if [ -n "$SPARQ_COUNT" ] && [ -n "$ELK_COUNT" ]; then
    [ "$SPARQ_COUNT" = "$ELK_COUNT" ] && COUNTS_AGREE="true" || COUNTS_AGREE="false"
    log "$ONT subsumptions — sparq=$SPARQ_COUNT elk=$ELK_COUNT agree=$COUNTS_AGREE"
    [ "$COUNTS_AGREE" = "false" ] && \
      log "DISAGREEMENT recorded (neither engine is ground truth; INVESTIGATE — not adjusted)"
  else
    log "$ONT: one or both subsumption counts missing — timing recorded WITHOUT an agreement flag"
  fi

  TS="$(python3 -c 'import time;print(time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()))')"
  OUT="$OUT_DIR/reason-el-${ONT}-${TS}.json"
  CANONICAL="$CANONICAL" ONT="$ONT" URL="$URL" OWL_SHA="$OWL_SHA" GIT_COMMIT="$GIT_COMMIT" \
  ONLY="$ONLY" TIMEOUT_S="$TIMEOUT_S" OUT="$OUT" \
  SPARQ_ROW="$SPARQ_ROW" SPARQ_COUNT="$SPARQ_COUNT" SPARQ_S="$SPARQ_S" \
  ELK_ROW="$ELK_ROW" ELK_COUNT="$ELK_COUNT" ELK_S="$ELK_S" ELK_VER="${ELK_VER:-}" \
  COUNTS_AGREE="$COUNTS_AGREE" \
  python3 - <<'PYEOF'
import json, os, platform

canonical = os.environ["CANONICAL"] == "1"
only = os.environ["ONLY"].split()
sparq_count = os.environ["SPARQ_COUNT"] or None
elk_count = os.environ["ELK_COUNT"] or None
agree = os.environ["COUNTS_AGREE"]

engines = {}
if "sparq" in only:
    engines["sparq"] = {
        "version": os.environ["GIT_COMMIT"],
        "mode": "in-process (examples/reason_el_real_bench GATHER: parse once, classify_graph); --features rbox",
        "subsumptions": sparq_count,
        "classify_s": os.environ["SPARQ_S"] or None,
        "raw": os.environ["SPARQ_ROW"],
    }
if "elk" in only:
    engines["elk"] = {
        "version": os.environ.get("ELK_VER", ""),
        "mode": "ELK CLI classify → inferred taxonomy → riot NT → transitively-closed named count",
        "subsumptions": elk_count,
        "classify_s": os.environ["ELK_S"] or None,
        "raw": os.environ["ELK_ROW"],
    }

note = (
    "CANONICAL: dedicated quiet box, one engine active at a time, SAME ontology OWL."
    if canonical else
    "NON-canonical FIRST READ: shared work box (not a dedicated quiet instance). Timings are "
    "directional only — do NOT bake into docs/dashboards. The harness "
    "(scripts/bench/reason-el-same-box.sh) is the durable deliverable; rerun CANONICAL=1 on a "
    "dedicated EC2 box for citable numbers."
)

envelope = {
    "gather": "reason-el-same-box-comparison",
    "wave": "reason-el competitor baseline (sq-hmd7l.8)",
    "canonical": canonical,
    "canonical_note": note,
    "git_commit": os.environ["GIT_COMMIT"],
    "suite": "reason-el-real",
    "ontology": os.environ["ONT"],
    "ontology_url": os.environ["URL"],
    "ontology_sha256": os.environ["OWL_SHA"],
    "metric": ("proper named subsumption pairs = complete transitive closure of C ⊑ D over named "
               "classes (C≠D, D≠owl:Thing, both named IRIs); ELK's direct taxonomy is transitively "
               "closed to match sparq's closure count"),
    "oracle_before_timing": {
        "sparq_subsumptions": sparq_count,
        "elk_subsumptions": elk_count,
        "counts_agree": agree,
        "policy": ("NEITHER engine is ground truth. A recorded count is required per ontology before "
                   "any timing is trusted; a disagreement (counts_agree=false) is INVESTIGATED, never "
                   "adjusted. counts_agree=n/a means a count was missing (engine error/absent)."),
    },
    "engines": engines,
    "env": {
        "host": platform.node(),
        "machine": platform.machine(),
        "os": platform.platform(),
        "timeout_s": int(os.environ["TIMEOUT_S"]),
    },
}
with open(os.environ["OUT"], "w") as fh:
    json.dump(envelope, fh, indent=2)
    fh.write("\n")
print(os.environ["OUT"])
PYEOF
  log "envelope: $OUT"
done

log "done. Gather deps live under /tmp/reason-el-gather + /tmp/elk-cli + /tmp/jena-riot (delete when finished)."
