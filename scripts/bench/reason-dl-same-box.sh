#!/usr/bin/env bash
# sq-hmd7l.10 (epic sq-hmd7l) — same-box OWL DL CONSISTENCY comparison:
# sparq-reason-dl (scoped ALCH tableau) vs HermiT and Openllet (full OWL 2 DL tableau
# reasoners) on ORE (OWL Reasoner Evaluation) competition corpus ontologies, emitting one
# competitor-results ENVELOPE per ontology. Mirrors scripts/bench/reason-el-same-box.sh.
#
# HONEST FRAMING (research/gap-reason-dl-2026-07.md). sparq-reason-dl is a scoped ALCH
# profile-checker/tableau, NOT a full OWL 2 DL reasoner: on real ORE corpora it is
# EXPECTED to abstain (verdict=unknown(out-of-fragment)) on every ontology outside ALCH.
# An abstention is recorded as an abstention — never as a verdict, never as a timing row.
#
# VERDICT-BEFORE-TIMING (the sq-hmd7l.10 INVARIANT — stricter than the reason-el count
# flag). Every engine's CONSISTENCY VERDICT is recorded and cross-checked FIRST; a timing
# row is emitted ONLY when at least two engines returned a definitive verdict AND all
# definitive verdicts AGREE. A disagreement is a CORRECTNESS FINDING: the envelope records
# verdicts_agree=false with timings NULLED, and the operator must file a bug bead
# (`bd create`) before any rerun — NEVER time past a disagreement. A sparq abstention
# yields agreement=n/a for sparq (abstentions cannot agree or disagree).
#
# TIMING BOUNDARY (one comparable scope for every engine): consistency_s is the
# END-TO-END wall time of ONE CLI process per engine per ontology — process start →
# exit — measured by this wrapper with the same clock for all three columns. So sparq's
# binary startup + input read + parse + extract → profile → tableau are INSIDE its
# timing, exactly as JVM startup + ontology load + reasoning are inside HermiT's and
# Openllet's. ore_bench's self-reported extract_s/check_s are recorded ONLY as a
# sparq-internal breakdown, never as the comparison metric, and the wrapper asserts
# process wall >= extract_s + check_s so the selected field provably sits at the outer
# boundary. Known residual caveat (recorded in every envelope): sparq parses
# riot-converted N-Triples while the JVM CLIs parse the original OWL file; the riot
# conversion itself runs outside every engine's timing.
#
# --smoke  the fast, hermetic ACCEPTANCE path — build + run the sparq example on the
#          VENDORED ORE-style fixtures (crates/sparq-reason-dl/examples/data/ore_smoke_*),
#          asserting their pinned verdicts. NO downloads, NO JVM, NO network.
#              bash scripts/bench/reason-dl-same-box.sh --smoke
#
# FULL MODE (gather; needs a JRE + riot for OWL→NT + a local ORE corpus):
#              ORE_CORPUS_DIR=/path/to/ore-owl-files bash scripts/bench/reason-dl-same-box.sh
#              ONLY="sparq openllet" ORE_CORPUS_DIR=… bash scripts/bench/reason-dl-same-box.sh
#
# The ORE 2014/2015 corpora are NOT auto-downloaded (multi-GB; hosting has moved over the
# years — see the ORE dataset repository / Zenodo mirrors referenced from
# bench/competitors.json `hermit-openllet`). Point ORE_CORPUS_DIR at a directory of
# .owl/.ttl/.nt ontology files; each file becomes one envelope.
#
# TUNABLES (env; all have safe defaults):
#   ORE_CORPUS_DIR   REQUIRED in full mode: dir of ontology files (.owl/.rdf/.ttl/.nt)
#   ORE_MAX_ONTOLOGIES  cap on corpus files processed this run     (default 25)
#   ONLY             engine subset of "sparq hermit openllet"      (default all three)
#   OUT_DIR          envelope dir (default /tmp/reason-dl-same-box-results; a canonical
#                    run points this at bench/canonical-competitor-results/<date>/)
#   GATHER           scratch dir for converted NT + raw engine output
#                    (default /tmp/reason-dl-gather)
#   CANONICAL        1 = dedicated quiet-box run                   (default 0: NON-canonical)
#   TIMEOUT_S        per-engine consistency cap, seconds           (default 600)
#   HERMIT_JAR       path to HermiT.jar         (LGPL-3.0; NOT auto-downloaded)
#   OPENLLET_JAR     path to openllet-cli jar   (Apache-2.0; NOT auto-downloaded — get it
#                    from https://github.com/Galigator/openllet/releases; prefer Openllet
#                    for publishable numbers per bench/competitors.json licensing note)
#   JENA_VERSION     apache-jena for `riot` OWL→NT                 (default 5.4.0)
#   JENA_HOME        jena dist root (auto-downloaded to /tmp if unset)
#
# Reasoner jars + corpora are gather-only deps — NOT committed (engines + big corpora stay
# out of git per AGENTS.md). Delete when done:  rm -rf /tmp/reason-dl-gather /tmp/jena-riot
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

ONLY="${ONLY:-sparq hermit openllet}"
OUT_DIR="${OUT_DIR:-/tmp/reason-dl-same-box-results}"
CANONICAL="${CANONICAL:-0}"
TIMEOUT_S="${TIMEOUT_S:-600}"
ORE_MAX_ONTOLOGIES="${ORE_MAX_ONTOLOGIES:-25}"
JENA_VERSION="${JENA_VERSION:-5.4.0}"
JENA_HOME="${JENA_HOME:-/tmp/jena-riot/apache-jena-$JENA_VERSION}"

log() { printf '[reason-dl-same-box] %s\n' "$*" >&2; }
want() { [[ " $ONLY " == *" $1 "* ]]; }

# ---- --smoke: the hermetic, sparq-only acceptance path -----------------------
if [[ "${1:-}" == "--smoke" ]]; then
  log "SMOKE: sparq-only, pinned vendored ORE-style fixtures (no network/JVM)"
  cargo build --release -p sparq-reason-dl --example ore_bench
  "$ROOT/target/release/examples/ore_bench" --smoke
  log "SMOKE OK"
  exit 0
fi

# ---- full mode ---------------------------------------------------------------
ORE_CORPUS_DIR="${ORE_CORPUS_DIR:-}"
if [ -z "$ORE_CORPUS_DIR" ] || [ ! -d "$ORE_CORPUS_DIR" ]; then
  log "ORE_CORPUS_DIR is unset or not a directory — full mode needs a local ORE corpus"
  log "(the ORE 2014/2015 corpora are gather-only downloads; see the header). Aborting."
  exit 2
fi

CORPUS_ROOT="${ORE_CORPUS_DIR%/}"

mkdir -p "$OUT_DIR"
GATHER="${GATHER:-/tmp/reason-dl-gather}"
mkdir -p "$GATHER"
GIT_COMMIT="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"

# ---- 0. engines --------------------------------------------------------------
SPARQ_BIN="$ROOT/target/release/examples/ore_bench"
if want sparq; then
  log "building sparq ore_bench example"
  cargo build --release -p sparq-reason-dl --example ore_bench
fi

HERMIT_JAR="${HERMIT_JAR:-}"
if want hermit && [ ! -f "$HERMIT_JAR" ]; then
  log "HERMIT_JAR unset/missing — HermiT columns will record an honest ERROR"
  log "(HermiT is LGPL-3.0: download it yourself from https://github.com/owlcs/HermiT"
  log " and note the licensing caveat in bench/competitors.json before publishing numbers)"
fi
OPENLLET_JAR="${OPENLLET_JAR:-}"
if want openllet && [ ! -f "$OPENLLET_JAR" ]; then
  log "OPENLLET_JAR unset/missing — Openllet columns will record an honest ERROR"
  log "(download openllet-cli from https://github.com/Galigator/openllet/releases)"
fi

# riot (Apache Jena) converts each OWL ontology → N-Triples for sparq's parser.
if want sparq && [ ! -x "$JENA_HOME/bin/riot" ]; then
  log "downloading apache-jena $JENA_VERSION to /tmp/jena-riot (for riot OWL→NT, gather-only)"
  mkdir -p /tmp/jena-riot
  curl -sSL -o "/tmp/jena-riot/apache-jena-$JENA_VERSION.tar.gz" \
    "https://archive.apache.org/dist/jena/binaries/apache-jena-$JENA_VERSION.tar.gz"
  tar xzf "/tmp/jena-riot/apache-jena-$JENA_VERSION.tar.gz" -C /tmp/jena-riot
fi
RIOT="$JENA_HOME/bin/riot"

# Seconds-since-epoch wall clock — the ONE timing source for every engine column
# (process start → exit), so all three consistency_s values share the same boundary.
now_s() { python3 -c 'import time;print(f"{time.time():.6f}")'; }

N=0
while IFS= read -r OWL; do
  [ "$N" -ge "$ORE_MAX_ONTOLOGIES" ] && { log "ORE_MAX_ONTOLOGIES=$ORE_MAX_ONTOLOGIES reached — stopping (cap logged, not silent)"; break; }
  N=$((N + 1))
  # Collision-resistant per-input key: the walk admits subdirectories, so two corpus
  # files may share a basename (a/foo.owl vs b/foo.owl), and the human-readable stem is
  # produced by a LOSSY sanitization (tr) that can ALIAS distinct paths — a/foo.owl and
  # a_foo.owl both sanitize to "a_foo". So the key does NOT rely on the sanitized stem
  # for uniqueness: it binds a 12-hex SHA-256 prefix of the corpus-RELATIVE PATH AND a
  # 12-hex SHA-256 prefix of the file CONTENT (the sanitized stem is a readable label
  # only). Every derived artifact — converted NT, riot stderr, raw per-engine .out,
  # envelope filename — carries this key, so the [ -f "$NT" ] conversion cache can only
  # ever hit for byte-identical content at the SAME relative path, and no later row can
  # overwrite an earlier one (even when two distinct paths sanitize to the same stem).
  REL="${OWL#"$CORPUS_ROOT"/}"
  OWL_SHA="$(sha256sum "$OWL" | cut -d' ' -f1)"
  REL_SHA="$(printf '%s' "$REL" | sha256sum | cut -d' ' -f1)"
  KEY="$(printf '%s' "${REL%.*}" | tr -c 'A-Za-z0-9._-' '_')-p${REL_SHA:0:12}-${OWL_SHA:0:12}"
  log "=== ontology $N: $REL ==="

  # -- 1a. sparq: verdict (+ profile row) printed BEFORE timing -----------------
  # consistency_s = end-to-end process wall around the ore_bench invocation (same
  # boundary as the JVM columns); extract_s/check_s from ore_bench stdout are kept
  # ONLY as a sparq-internal breakdown.
  SPARQ_ROW=""; SPARQ_VERDICT=""; SPARQ_S=""; SPARQ_EXTRACT_S=""; SPARQ_CHECK_S=""
  if want sparq; then
    NT="$GATHER/$KEY.nt"
    case "$OWL" in
      *.nt) NT="$OWL" ;;
      *) [ -f "$NT" ] || "$RIOT" --output=ntriples "$OWL" > "$NT" 2>"$GATHER/$KEY.riot.err" \
           || { log "riot conversion failed (see $GATHER/$KEY.riot.err)"; NT=""; } ;;
    esac
    if [ -n "$NT" ]; then
      log "sparq: consistency $REL (cap ${TIMEOUT_S}s)"
      START="$(now_s)"
      if timeout "$TIMEOUT_S" "$SPARQ_BIN" "$NT" ntriples > "$GATHER/$KEY.sparq.out" 2>&1; then
        END="$(now_s)"; SPARQ_S="$(python3 -c "print(f'{$END-$START:.6f}')")"
        SPARQ_ROW="$(cat "$GATHER/$KEY.sparq.out")"
        SPARQ_VERDICT="$(sed -n 's/.* verdict=\([^ ]*\).*/\1/p' "$GATHER/$KEY.sparq.out" | head -1)"
        SPARQ_EXTRACT_S="$(sed -n 's/.*extract_s=\([0-9.]*\).*/\1/p' "$GATHER/$KEY.sparq.out" | head -1)"
        SPARQ_CHECK_S="$(sed -n 's/.*check_s=\([0-9.]*\).*/\1/p' "$GATHER/$KEY.sparq.out" | head -1)"
        # Boundary assertion: the selected metric must be the OUTER process wall — it
        # can never undercut the tool's own internal phase sum. A violation means the
        # wrapper is timing the wrong field; abort rather than emit a bogus envelope.
        if [ -n "$SPARQ_EXTRACT_S" ] && [ -n "$SPARQ_CHECK_S" ]; then
          python3 -c "import sys; sys.exit(0 if float('$SPARQ_S') >= float('$SPARQ_EXTRACT_S') + float('$SPARQ_CHECK_S') else 1)" \
            || { log "TIMING-BOUNDARY VIOLATION on $REL: process wall ${SPARQ_S}s < internal extract_s+check_s — consistency_s is not the end-to-end boundary; aborting"; exit 3; }
        fi
      else
        log "sparq FAILED/timeout on $REL"; SPARQ_ROW="ERROR: timeout/failure"
      fi
    else
      SPARQ_ROW="ERROR: riot conversion failed"
    fi
  fi

  # -- 1b. HermiT: CLI consistency (-k) -----------------------------------------
  HERMIT_ROW=""; HERMIT_VERDICT=""; HERMIT_S=""
  if want hermit && [ -f "$HERMIT_JAR" ]; then
    log "hermit: consistency $REL (cap ${TIMEOUT_S}s)"
    START="$(now_s)"
    if timeout "$TIMEOUT_S" java -jar "$HERMIT_JAR" -k "file://$OWL" \
        > "$GATHER/$KEY.hermit.out" 2>&1; then
      END="$(now_s)"; HERMIT_S="$(python3 -c "print(f'{$END-$START:.6f}')")"
      HERMIT_ROW="$(cat "$GATHER/$KEY.hermit.out")"
      if grep -qi 'true\|is consistent' "$GATHER/$KEY.hermit.out"; then HERMIT_VERDICT="consistent"
      elif grep -qi 'false\|is inconsistent' "$GATHER/$KEY.hermit.out"; then HERMIT_VERDICT="inconsistent"
      fi
    else
      log "hermit FAILED/timeout on $REL"; HERMIT_ROW="ERROR: timeout/failure"
    fi
  elif want hermit; then
    HERMIT_ROW="ERROR: HERMIT_JAR unavailable"
  fi

  # -- 1c. Openllet: CLI consistency --------------------------------------------
  OPENLLET_ROW=""; OPENLLET_VERDICT=""; OPENLLET_S=""
  if want openllet && [ -f "$OPENLLET_JAR" ]; then
    log "openllet: consistency $REL (cap ${TIMEOUT_S}s)"
    START="$(now_s)"
    if timeout "$TIMEOUT_S" java -jar "$OPENLLET_JAR" consistency "$OWL" \
        > "$GATHER/$KEY.openllet.out" 2>&1; then
      END="$(now_s)"; OPENLLET_S="$(python3 -c "print(f'{$END-$START:.6f}')")"
      OPENLLET_ROW="$(cat "$GATHER/$KEY.openllet.out")"
      if grep -qi 'inconsistent' "$GATHER/$KEY.openllet.out"; then OPENLLET_VERDICT="inconsistent"
      elif grep -qi 'consistent' "$GATHER/$KEY.openllet.out"; then OPENLLET_VERDICT="consistent"
      fi
    else
      log "openllet FAILED/timeout on $REL"; OPENLLET_ROW="ERROR: timeout/failure"
    fi
  elif want openllet; then
    OPENLLET_ROW="ERROR: OPENLLET_JAR unavailable"
  fi

  # -- 2. VERDICT-BEFORE-TIMING cross-check (the sq-hmd7l.10 invariant) ---------
  # Definitive = "consistent"/"inconsistent". sparq's unknown(...) abstentions are
  # honest non-answers: excluded from agreement, and they null sparq's timing.
  AGREE="n/a"; DEFINITIVE=""
  for V in "$SPARQ_VERDICT" "$HERMIT_VERDICT" "$OPENLLET_VERDICT"; do
    case "$V" in consistent|inconsistent) DEFINITIVE="$DEFINITIVE $V" ;; esac
  done
  NDEF="$(echo "$DEFINITIVE" | wc -w)"
  NUNIQ="$(echo "$DEFINITIVE" | tr ' ' '\n' | sed '/^$/d' | sort -u | wc -l)"
  if [ "$NDEF" -ge 2 ]; then
    if [ "$NUNIQ" -eq 1 ]; then AGREE="true"; else AGREE="false"; fi
  fi
  log "$REL verdicts — sparq=${SPARQ_VERDICT:-none} hermit=${HERMIT_VERDICT:-none} openllet=${OPENLLET_VERDICT:-none} agree=$AGREE"
  if [ "$AGREE" = "false" ]; then
    log "DISAGREEMENT: correctness finding — timings NULLED in the envelope."
    log "ACTION REQUIRED: file a bug bead (bd create) referencing this envelope before rerunning."
  fi
  if [ "$AGREE" != "true" ]; then
    # No timing row without verdict agreement — null every engine timing (the
    # sparq-internal breakdown included: an untrusted row gets no timings at all).
    SPARQ_S=""; SPARQ_EXTRACT_S=""; SPARQ_CHECK_S=""; HERMIT_S=""; OPENLLET_S=""
  fi
  case "$SPARQ_VERDICT" in
    consistent|inconsistent) ;;
    *) SPARQ_S=""; SPARQ_EXTRACT_S=""; SPARQ_CHECK_S="" ;;
  esac

  # -- 3. envelope ---------------------------------------------------------------
  TS="$(python3 -c 'import time;print(time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()))')"
  OUT="$OUT_DIR/reason-dl-${KEY}-${TS}.json"
  CANONICAL="$CANONICAL" ONT="$REL" OWL_SHA="$OWL_SHA" GIT_COMMIT="$GIT_COMMIT" \
  ONLY="$ONLY" TIMEOUT_S="$TIMEOUT_S" OUT="$OUT" AGREE="$AGREE" \
  SPARQ_ROW="$SPARQ_ROW" SPARQ_VERDICT="$SPARQ_VERDICT" SPARQ_S="$SPARQ_S" \
  SPARQ_EXTRACT_S="$SPARQ_EXTRACT_S" SPARQ_CHECK_S="$SPARQ_CHECK_S" \
  HERMIT_ROW="$HERMIT_ROW" HERMIT_VERDICT="$HERMIT_VERDICT" HERMIT_S="$HERMIT_S" \
  OPENLLET_ROW="$OPENLLET_ROW" OPENLLET_VERDICT="$OPENLLET_VERDICT" OPENLLET_S="$OPENLLET_S" \
  python3 - <<'PYEOF'
import json, os, platform

canonical = os.environ["CANONICAL"] == "1"
only = os.environ["ONLY"].split()
agree = os.environ["AGREE"]

def engine(key, mode):
    return {
        "verdict": os.environ[f"{key}_VERDICT"] or None,
        "consistency_s": os.environ[f"{key}_S"] or None,
        "mode": mode,
        "raw": os.environ[f"{key}_ROW"][:2000],
    }

engines = {}
if "sparq" in only:
    engines["sparq"] = engine("SPARQ", "one-shot CLI (examples/ore_bench GATHER): "
                              "consistency_s is end-to-end process wall — spawn → exit, "
                              "so binary startup + N-Triples read/parse + extract → "
                              "profile → ALCH tableau are all inside it, matching the "
                              "JVM CLI boundary; scoped ALCH — abstains "
                              "unknown(out-of-fragment) outside the fragment")
    engines["sparq"]["version"] = os.environ["GIT_COMMIT"]
    engines["sparq"]["internal_breakdown_s"] = {
        "extract_s": os.environ["SPARQ_EXTRACT_S"] or None,
        "check_s": os.environ["SPARQ_CHECK_S"] or None,
        "note": ("sparq-internal phase timings self-reported by ore_bench — a breakdown "
                 "of consistency_s, NOT comparable to the other engine columns; the "
                 "cross-engine metric is consistency_s (end-to-end process wall) for "
                 "every engine, asserted >= extract_s + check_s by the wrapper"),
    }
if "hermit" in only:
    engines["hermit"] = engine("HERMIT", "HermiT CLI -k (full OWL 2 DL; LGPL-3.0 — note the "
                               "licensing caveat before publishing numbers)")
if "openllet" in only:
    engines["openllet"] = engine("OPENLLET", "openllet-cli consistency (full OWL 2 DL, Apache-2.0)")

note = (
    "CANONICAL: dedicated quiet box, one engine active at a time, SAME ontology file."
    if canonical else
    "NON-canonical FIRST READ: shared work box (not a dedicated quiet instance). Timings are "
    "directional only — do NOT bake into docs/dashboards. The harness "
    "(scripts/bench/reason-dl-same-box.sh) is the durable deliverable; rerun CANONICAL=1 on a "
    "dedicated EC2 box for citable numbers."
)

envelope = {
    "gather": "reason-dl-same-box-comparison",
    "wave": "reason-dl ORE baseline (sq-hmd7l.10)",
    "canonical": canonical,
    "canonical_note": note,
    "git_commit": os.environ["GIT_COMMIT"],
    "suite": "reason-dl-ore",
    "ontology": os.environ["ONT"],
    "ontology_sha256": os.environ["OWL_SHA"],
    "metric": ("ontology CONSISTENCY verdict (consistent/inconsistent) + end-to-end "
               "consistency wall time. TIMING BOUNDARY: consistency_s is ONE full CLI "
               "process per engine per ontology (process start → exit, wrapper-clocked), "
               "so engine startup (native or JVM) and ontology load/parse sit INSIDE "
               "every engine's timing. Residual input caveat: sparq parses riot-converted "
               "N-Triples while HermiT/Openllet parse the original OWL file; the riot "
               "conversion runs outside every engine's timing. sparq-reason-dl is a "
               "SCOPED ALCH tableau, not full OWL 2 DL: out-of-fragment ontologies record "
               "verdict=unknown(out-of-fragment) — an honest abstention, never a verdict "
               "and never a timing row"),
    "verdict_before_timing": {
        "sparq_verdict": os.environ["SPARQ_VERDICT"] or None,
        "hermit_verdict": os.environ["HERMIT_VERDICT"] or None,
        "openllet_verdict": os.environ["OPENLLET_VERDICT"] or None,
        "verdicts_agree": agree,
        "policy": ("NO timing row without verdict agreement (>=2 definitive verdicts, all equal). "
                   "A disagreement is a CORRECTNESS FINDING: timings are nulled and a bug bead "
                   "must be filed before rerunning — never time past it. Abstentions "
                   "(unknown(...)) are excluded from agreement; verdicts_agree=n/a means fewer "
                   "than two definitive verdicts."),
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
done < <(find "$ORE_CORPUS_DIR" -maxdepth 2 -type f \
           \( -name '*.owl' -o -name '*.rdf' -o -name '*.ttl' -o -name '*.nt' \) | sort)

log "done. Gather deps live under /tmp/reason-dl-gather + /tmp/jena-riot (delete when finished)."
