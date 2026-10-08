#!/usr/bin/env bash
# sq-hmd7l.9 (epic sq-hmd7l) — same-box OWL 2 QL REWRITING comparison:
# sparq-reason-ql's PerfectRef rewriter vs Ontop (the mainstream OBDA/QL system) on the
# NPD benchmark + the Requiem test-suite ontologies/queries. Mirrors
# scripts/bench/reason-el-same-box.sh (the sq-hmd7l.8 gather recipe) and the
# competitor-results ENVELOPE shape.
#
# TWO METRICS, BOTH REQUIRED (the bead's win condition): rewrite WALL TIME and output UCQ
# SIZE (disjunct count). A smaller-or-equal UCQ at lower latency is the honest win; a
# smaller UCQ alone is a WIN on the size axis even at equal time.
#
# REGIME LABELS (the sq-hmd7l.9 INVARIANT — every column is labelled). Ontop couples
# rewriting to SQL translation: its CLI exposes NO isolated rewriter phase, so the Ontop
# column is END-TO-END (SPARQL in, answers out, over its OBDA stack) and is comparable
# ONLY to sparq's e2e_ms column (rewrite + execute over the same data), never to sparq's
# rewriter-phase columns. sparq's raw_rewrite_ms / min_rewrite_ms are REWRITER-PHASE ONLY.
# Isolating Ontop's rewriter needs a small Java driver against its internal API — a
# recorded follow-up, not silently faked here.
#
# UCQ-EQUIVALENCE BEFORE TIMING. The sparq example (ql_npd_requiem_bench) executes the raw
# PerfectRef UCQ and the minimised production UCQ over the same data (a deterministic
# PER-QUERY witness ABox — the frozen canonical instances of every disjunct of the
# original/raw/minimised queries — or --abox real data) and ABORTS on any
# result-set disagreement — no timing row exists without the equivalence check having
# passed. Witness mode is FAIL-CLOSED on FILTER/VALUES modifiers (freezing ignores their
# semantics, so the check could agree vacuously): such queries get a needs-abox row with
# NO timings and are only timed under --abox real data. Cross-engine (sparq vs Ontop)
# per-query answer COUNTS are recorded and compared in the envelope when both sides ran
# over the same data (counts_agree per query; a disagreement is recorded and
# investigated, NEVER silently adjusted).
#
# --smoke  (ONLY=sparq): the fast, hermetic ACCEPTANCE path — build + run the sparq example
#          on embedded NPD/Requiem-shaped fixtures with hand-verified closed-form UCQ sizes.
#          NO downloads, NO JVM, NO network.
#              ONLY=sparq bash scripts/bench/reason-ql-same-box.sh --smoke
#
# FULL MODE (gather; needs network + git + a JRE for Ontop + riot for OWL→NT):
#              bash scripts/bench/reason-ql-same-box.sh                 # NPD + Requiem
#              REASON_QL_SUITES=requiem ONLY=sparq bash scripts/bench/reason-ql-same-box.sh
#
# TUNABLES (env; all have safe defaults):
#   REASON_QL_SUITES   space list of "npd requiem"          (default "npd requiem")
#   ONLY               engine subset of "sparq ontop"       (default "sparq ontop")
#   OUT_DIR            envelope dir (default /tmp/reason-ql-same-box-results; a canonical
#                      run points this at bench/canonical-competitor-results/<date>/)
#   CANONICAL          1 = dedicated quiet-box run          (default 0: NON-canonical)
#   TIMEOUT_S          per-query cap, seconds               (default 600)
#   NPD_REPO_URL       npd-benchmark git repo               (default github.com/ontop/npd-benchmark)
#   NPD_JDBC_PROPERTIES  jdbc .properties of a LOADED NPD PostgreSQL instance (enables the
#                      Ontop NPD end-to-end column + `ontop materialize` for sparq's --abox
#                      same-data leg; unset → those columns record an honest ERROR)
#   NPD_ABOX_NT        pre-materialised NPD RDF (skips `ontop materialize`)
#   REQUIEM_ZIP        local Requiem test-suite zip (skips download)
#   REQUIEM_ZIP_URL    Requiem suite URL (default the Oxford ISG tools page; set REQUIEM_ZIP
#                      if the URL has rotted — recorded as an honest ERROR, never guessed)
#   REQUIEM_ONTOLOGIES space list of suite stems            (default "A S U V P5 UX")
#   ONTOP_VERSION      Ontop CLI release                    (default 5.3.0)
#   ONTOP_DIR          Ontop CLI root (auto-downloaded to /tmp if unset)
#   JENA_VERSION       apache-jena for `riot` OWL→NT        (default 5.4.0)
#   JENA_HOME          jena dist root (auto-downloaded to /tmp if unset)
#
# Suites + the Ontop/Jena distributions are gather-only deps under /tmp — NOT committed
# (engines + corpora stay out of git per AGENTS.md). Delete when done:
#   rm -rf /tmp/reason-ql-gather /tmp/ontop-cli /tmp/jena-riot
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

ONLY="${ONLY:-sparq ontop}"
OUT_DIR="${OUT_DIR:-/tmp/reason-ql-same-box-results}"
CANONICAL="${CANONICAL:-0}"
TIMEOUT_S="${TIMEOUT_S:-600}"
REASON_QL_SUITES="${REASON_QL_SUITES:-npd requiem}"
NPD_REPO_URL="${NPD_REPO_URL:-https://github.com/ontop/npd-benchmark}"
REQUIEM_ZIP_URL="${REQUIEM_ZIP_URL:-https://www.cs.ox.ac.uk/isg/tools/Requiem/Requiem.zip}"
REQUIEM_ONTOLOGIES="${REQUIEM_ONTOLOGIES:-A S U V P5 UX}"
ONTOP_VERSION="${ONTOP_VERSION:-5.3.0}"
ONTOP_DIR="${ONTOP_DIR:-/tmp/ontop-cli}"
JENA_VERSION="${JENA_VERSION:-5.4.0}"
JENA_HOME="${JENA_HOME:-/tmp/jena-riot/apache-jena-$JENA_VERSION}"

log() { printf '[reason-ql-same-box] %s\n' "$*" >&2; }
want() { [[ " $ONLY " == *" $1 "* ]]; }

SPARQ_BIN="$ROOT/target/release/examples/ql_npd_requiem_bench"

# ---- --smoke: the hermetic, sparq-only acceptance path -----------------------
if [[ "${1:-}" == "--smoke" ]]; then
  log "SMOKE: sparq-only, embedded NPD/Requiem-shaped fixtures (no network/JVM)"
  cargo build --release -p sparq-reason-ql --features experimental --example ql_npd_requiem_bench
  "$SPARQ_BIN" --smoke
  log "SMOKE OK"
  exit 0
fi

mkdir -p "$OUT_DIR"
GATHER="/tmp/reason-ql-gather"
mkdir -p "$GATHER"
GIT_COMMIT="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"

# ---- 0. engines --------------------------------------------------------------
if want sparq; then
  log "building sparq ql_npd_requiem_bench (--features experimental, release)"
  cargo build --release -p sparq-reason-ql --features experimental --example ql_npd_requiem_bench
fi

ONTOP_BIN="$ONTOP_DIR/ontop"
if want ontop && [ ! -x "$ONTOP_BIN" ]; then
  mkdir -p "$ONTOP_DIR"
  log "downloading Ontop CLI $ONTOP_VERSION to $ONTOP_DIR (Apache-2.0, gather-only)"
  curl -sSL -o "$ONTOP_DIR/ontop-cli.zip" \
    "https://github.com/ontop/ontop/releases/download/ontop-$ONTOP_VERSION/ontop-cli-$ONTOP_VERSION.zip" || true
  [ -f "$ONTOP_DIR/ontop-cli.zip" ] && unzip -oq "$ONTOP_DIR/ontop-cli.zip" -d "$ONTOP_DIR" || true
  [ -x "$ONTOP_BIN" ] || log "Ontop CLI unavailable — Ontop columns will record an honest ERROR"
fi

# riot (Apache Jena) converts each OWL ontology → N-Triples for sparq's TBox parser.
if [ ! -x "$JENA_HOME/bin/riot" ]; then
  log "downloading apache-jena $JENA_VERSION to /tmp/jena-riot (for riot OWL→NT, gather-only)"
  mkdir -p /tmp/jena-riot
  curl -sSL -o "/tmp/jena-riot/apache-jena-$JENA_VERSION.tar.gz" \
    "https://archive.apache.org/dist/jena/binaries/apache-jena-$JENA_VERSION.tar.gz"
  tar xzf "/tmp/jena-riot/apache-jena-$JENA_VERSION.tar.gz" -C /tmp/jena-riot
fi
RIOT="$JENA_HOME/bin/riot"

# ---- helper: translate Requiem datalog-style queries into SELECT DISTINCT SPARQL --------
# Requiem query lines look like  Q(?0) <- worksFor(?0,?1), Employee(?0) . Predicates are
# local names resolved against the ontology's IRIs (scanned from the riot NT). A query with
# an unresolvable predicate is SKIPPED and logged — never guessed.
translate_requiem_queries() { # $1=tbox.nt $2=query-src-file $3=out-dir $4=suite
  TBOX_NT="$1" QSRC="$2" QOUT="$3" SUITE="$4" python3 - <<'PYEOF'
import os, re, sys

tbox_nt, qsrc, qout, suite = (os.environ[k] for k in ("TBOX_NT", "QSRC", "QOUT", "SUITE"))
os.makedirs(qout, exist_ok=True)

vocab = {}
iri_re = re.compile(r"<([^>]+)>")
for line in open(tbox_nt, encoding="utf-8", errors="replace"):
    for iri in iri_re.findall(line):
        local = iri.rsplit("#", 1)[-1].rsplit("/", 1)[-1]
        if local:
            vocab.setdefault(local.lower(), iri)

atom_re = re.compile(r"([A-Za-z_][\w.\-]*)\s*\(\s*([^)]*?)\s*\)")
written = skipped = 0
for n, line in enumerate(open(qsrc, encoding="utf-8", errors="replace"), 1):
    line = line.strip()
    if "<-" not in line:
        continue
    head_src, body_src = line.split("<-", 1)
    head = atom_re.search(head_src)
    if not head:
        continue
    def term(t):
        t = t.strip()
        if t.startswith("?"):
            return "?v" + re.sub(r"\W", "", t)
        low = t.lower()
        if low in vocab:
            return "<%s>" % vocab[low]
        return '"%s"' % t.replace('"', '\\"')
    patterns, ok = [], True
    for pred, args_src in atom_re.findall(body_src):
        args = [a for a in (x.strip() for x in args_src.split(",")) if a]
        iri = vocab.get(pred.lower())
        if iri is None:
            sys.stderr.write("[reason-ql-same-box] %s q%d: unresolvable predicate %r — SKIPPED\n" % (suite, n, pred))
            ok = False
            break
        if len(args) == 1:
            patterns.append("%s a <%s> ." % (term(args[0]), iri))
        elif len(args) == 2:
            patterns.append("%s <%s> %s ." % (term(args[0]), iri, term(args[1])))
        else:
            sys.stderr.write("[reason-ql-same-box] %s q%d: %d-ary atom — SKIPPED\n" % (suite, n, len(args)))
            ok = False
            break
    if not ok or not patterns:
        skipped += 1
        continue
    head_vars = " ".join(term(a) for a in head.group(2).split(",") if a.strip())
    sparql = "SELECT DISTINCT %s WHERE { %s }\n" % (head_vars or "*", " ".join(patterns))
    with open(os.path.join(qout, "%s-q%02d.rq" % (suite, n)), "w") as fh:
        fh.write(sparql)
    written += 1
print("%d written, %d skipped" % (written, skipped))
PYEOF
}

# ---- helper: run the Ontop end-to-end column over a query dir ---------------------------
run_ontop_endtoend() { # $1=owl $2=mapping $3=properties $4=queries-dir $5=out-tsv
  local owl="$1" mapping="$2" props="$3" qdir="$4" out="$5" q id start end elapsed rows
  : > "$out"
  for q in "$qdir"/*.rq; do
    [ -e "$q" ] || continue
    id="$(basename "$q" .rq)"
    start="$(python3 -c 'import time;print(f"{time.time():.6f}")')"
    if timeout "$TIMEOUT_S" "$ONTOP_BIN" query -t "$owl" -m "$mapping" -p "$props" \
        -q "$q" -o "$GATHER/ontop-ans-$id.csv" >/dev/null 2>"$GATHER/ontop-$id.err"; then
      end="$(python3 -c 'import time;print(f"{time.time():.6f}")')"
      elapsed="$(python3 -c "print(f'{($end-$start)*1000:.3f}')")"
      rows="$(($(wc -l < "$GATHER/ontop-ans-$id.csv") - 1))"; [ "$rows" -lt 0 ] && rows=0
      printf '%s\t%s\t%s\n' "$id" "$rows" "$elapsed" >> "$out"
    else
      printf '%s\tERROR\tERROR\n' "$id" >> "$out"
      log "ontop FAILED/timeout on $id (see $GATHER/ontop-$id.err)"
    fi
  done
}

# ---- helper: emit one competitor-results envelope per suite -----------------------------
emit_envelope() { # $1=suite $2=tbox $3=sparq-tsv $4=ontop-tsv-or-empty $5=ontop-note $6=source-note
  local suite="$1" tbox="$2" sparq_tsv="$3" ontop_tsv="$4" ontop_note="$5" source_note="$6"
  local ts sha out
  ts="$(python3 -c 'import time;print(time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()))')"
  sha="$( [ -f "$tbox" ] && sha256sum "$tbox" | cut -d' ' -f1 || echo unavailable)"
  out="$OUT_DIR/reason-ql-${suite}-${ts}.json"
  CANONICAL="$CANONICAL" SUITE="$suite" TBOX_SHA="$sha" GIT_COMMIT="$GIT_COMMIT" ONLY="$ONLY" \
  TIMEOUT_S="$TIMEOUT_S" OUT="$out" SPARQ_TSV="$sparq_tsv" ONTOP_TSV="$ontop_tsv" \
  ONTOP_NOTE="$ontop_note" ONTOP_VERSION="$ONTOP_VERSION" SOURCE_NOTE="$source_note" \
  python3 - <<'PYEOF'
import json, os, platform

canonical = os.environ["CANONICAL"] == "1"
only = os.environ["ONLY"].split()

def read(path):
    if path and os.path.exists(path):
        return open(path, encoding="utf-8", errors="replace").read()
    return ""

sparq_raw = read(os.environ["SPARQ_TSV"])
ontop_raw = read(os.environ["ONTOP_TSV"])

# Per-query cross-engine answer-count oracle (only meaningful when the sparq rows carry an
# --abox answers column over the SAME data the Ontop column queried).
sparq_answers, ucq_sizes = {}, {}
for line in sparq_raw.splitlines():
    f = line.split("\t")
    if len(f) >= 12 and f[2] == "ok":
        ucq_sizes[f[1]] = int(f[4])
        sparq_answers[f[1]] = f[9]
ontop_answers = {}
for line in ontop_raw.splitlines():
    f = line.split("\t")
    if len(f) == 3:
        ontop_answers[f[0]] = f[1]
agreement = {}
for qid in sorted(set(sparq_answers) & set(ontop_answers)):
    a, b = sparq_answers[qid], ontop_answers[qid]
    agreement[qid] = "true" if (a == b and a != "ERROR") else "false"

engines = {}
if "sparq" in only:
    engines["sparq"] = {
        "version": os.environ["GIT_COMMIT"],
        "regime": ("rewriter-phase (raw_rewrite_ms/min_rewrite_ms: in-process "
                   "rewrite/rewrite_production, no execution) + e2e_ms (rewrite + minimised-UCQ "
                   "execution over the loaded data; the only column comparable to Ontop's)"),
        "ucq_size_metric": "min_disjuncts = MINIMISED UCQ disjunct count (the emitted query)",
        "raw": sparq_raw,
    }
if "ontop" in only:
    engines["ontop"] = {
        "version": os.environ["ONTOP_VERSION"],
        "regime": ("END-TO-END ONLY (ontop query CLI: SPARQL in, answers out over the OBDA "
                   "stack). The CLI exposes no isolated rewriter phase and no UCQ-size "
                   "readout; comparable ONLY to sparq's e2e_ms column."),
        "note": os.environ["ONTOP_NOTE"],
        "raw": ontop_raw,
    }

note = (
    "CANONICAL: dedicated quiet box, one engine active at a time, SAME suite inputs."
    if canonical else
    "NON-canonical FIRST READ: shared work box (not a dedicated quiet instance). Timings are "
    "directional only — do NOT bake into docs/dashboards. The harness "
    "(scripts/bench/reason-ql-same-box.sh) is the durable deliverable; rerun CANONICAL=1 on a "
    "dedicated EC2 box for citable numbers. UCQ disjunct COUNTS are deterministic and "
    "load-robust; wall times are not."
)

envelope = {
    "gather": "reason-ql-same-box-comparison",
    "wave": "reason-ql rewriting vs Ontop (sq-hmd7l.9)",
    "canonical": canonical,
    "canonical_note": note,
    "git_commit": os.environ["GIT_COMMIT"],
    "suite": os.environ["SUITE"],
    "suite_source": os.environ["SOURCE_NOTE"],
    "tbox_sha256": os.environ["TBOX_SHA"],
    "metrics": {
        "rewrite_wall_time": "per-query rewriter-phase ms (sparq only; Ontop's is not CLI-isolable)",
        "ucq_size": "per-query minimised UCQ disjunct count (sparq; deterministic, load-robust)",
        "end_to_end": "per-query ms SPARQL-in→answers-out (sparq e2e_ms vs ontop; same data only)",
    },
    "equivalence_before_timing": {
        "in_engine": ("raw PerfectRef UCQ vs minimised production UCQ executed over the same data "
                      "inside ql_npd_requiem_bench; any result-set disagreement ABORTS the sparq "
                      "leg before a timing row exists"),
        "cross_engine_counts_agree": agreement or None,
        "policy": ("NEITHER engine is ground truth. A cross-engine per-query answer-count "
                   "disagreement (false) is recorded and INVESTIGATED, never adjusted; absent "
                   "means the two engines did not run over the same data in this gather."),
    },
    "ucq_sizes": ucq_sizes or None,
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
  log "envelope: $out"
}

# ---- 1. suites ---------------------------------------------------------------
for SUITE in $REASON_QL_SUITES; do
  case "$SUITE" in

  npd)
    log "=== suite: npd ($NPD_REPO_URL) ==="
    NPD_DIR="$GATHER/npd-benchmark"
    if [ ! -d "$NPD_DIR" ]; then
      git clone --depth 1 "$NPD_REPO_URL" "$NPD_DIR" 2>/dev/null || {
        log "npd-benchmark clone failed — suite npd SKIPPED (set NPD_REPO_URL)"; continue; }
    fi
    NPD_OWL="$(find "$NPD_DIR" -iname '*ql*.owl' | head -1 || true)"
    [ -z "$NPD_OWL" ] && NPD_OWL="$(find "$NPD_DIR" -iname 'npd*.owl' | head -1 || true)"
    [ -z "$NPD_OWL" ] && NPD_OWL="$(find "$NPD_DIR" -iname '*.owl' | head -1 || true)"
    [ -z "$NPD_OWL" ] && { log "no .owl in npd-benchmark clone — suite npd SKIPPED"; continue; }
    NPD_NT="$GATHER/npd.nt"
    [ -f "$NPD_NT" ] || "$RIOT" --output=ntriples "$NPD_OWL" > "$NPD_NT" 2>"$GATHER/npd.riot.err" || {
      log "riot OWL→NT failed for $NPD_OWL — suite npd SKIPPED"; continue; }
    NPD_QUERIES="$GATHER/npd-queries"
    mkdir -p "$NPD_QUERIES"
    find "$NPD_DIR" \( -iname '*.rq' -o -iname '*.sparql' \) | sort | while read -r q; do
      cp "$q" "$NPD_QUERIES/$(echo "${q#"$NPD_DIR"/}" | tr '/' '_' | sed 's/\.sparql$/.rq/')"
    done
    [ -n "$(ls -A "$NPD_QUERIES" 2>/dev/null)" ] || { log "no NPD queries found — suite npd SKIPPED"; continue; }

    # Same-data leg: materialise the NPD OBDA instance to RDF so BOTH engines answer over
    # the same data (the issue's fallback regime when the rewriter phase is not isolable).
    NPD_ABOX="${NPD_ABOX_NT:-}"
    NPD_MAPPING="$(find "$NPD_DIR" -iname '*.obda' | head -1 || true)"
    if [ -z "$NPD_ABOX" ] && [ -n "${NPD_JDBC_PROPERTIES:-}" ] && [ -x "$ONTOP_BIN" ] && [ -n "$NPD_MAPPING" ]; then
      log "ontop materialize: NPD OBDA instance → RDF (same-data leg)"
      NPD_ABOX="$GATHER/npd-data.nt"
      timeout "$TIMEOUT_S" "$ONTOP_BIN" materialize -t "$NPD_OWL" -m "$NPD_MAPPING" \
        -p "$NPD_JDBC_PROPERTIES" -f ntriples -o "$NPD_ABOX" 2>"$GATHER/npd-materialize.err" \
        || { log "ontop materialize failed — sparq runs witness-ABox equivalence only"; NPD_ABOX=""; }
    fi

    SPARQ_TSV="$GATHER/npd-sparq.tsv"
    if want sparq; then
      log "sparq: NPD rewriter-phase leg (equivalence-before-timing enforced in-process)"
      if [ -n "$NPD_ABOX" ] && [ -f "$NPD_ABOX" ]; then
        "$SPARQ_BIN" npd "$NPD_NT" "$NPD_QUERIES" --abox "$NPD_ABOX" > "$SPARQ_TSV"
      else
        "$SPARQ_BIN" npd "$NPD_NT" "$NPD_QUERIES" > "$SPARQ_TSV"
      fi
      grep '^#' "$SPARQ_TSV" >&2 || true
    fi

    ONTOP_TSV=""; ONTOP_NOTE=""
    if want ontop; then
      if [ -x "$ONTOP_BIN" ] && [ -n "${NPD_JDBC_PROPERTIES:-}" ] && [ -n "$NPD_MAPPING" ]; then
        log "ontop: NPD end-to-end column (regime: OBDA CLI, NOT rewriter-phase)"
        ONTOP_TSV="$GATHER/npd-ontop.tsv"
        run_ontop_endtoend "$NPD_OWL" "$NPD_MAPPING" "$NPD_JDBC_PROPERTIES" "$NPD_QUERIES" "$ONTOP_TSV"
        ONTOP_NOTE="end-to-end over the NPD PostgreSQL instance ($NPD_MAPPING)"
      else
        ONTOP_NOTE="ERROR: NPD end-to-end needs the Ontop CLI + NPD_JDBC_PROPERTIES pointing at a loaded NPD PostgreSQL instance (see github.com/ontop/npd-benchmark); recorded honestly, not faked"
        log "ontop npd: $ONTOP_NOTE"
      fi
    fi
    emit_envelope npd "$NPD_NT" "$SPARQ_TSV" "$ONTOP_TSV" "$ONTOP_NOTE" \
      "NPD benchmark ontology+queries from $NPD_REPO_URL (ontology $(basename "$NPD_OWL"))"
    ;;

  requiem)
    log "=== suite: requiem ==="
    REQ_ZIP="${REQUIEM_ZIP:-$GATHER/requiem.zip}"
    if [ ! -f "$REQ_ZIP" ]; then
      log "downloading Requiem suite ($REQUIEM_ZIP_URL)"
      curl -fsSL -o "$REQ_ZIP" "$REQUIEM_ZIP_URL" || {
        log "Requiem download failed — set REQUIEM_ZIP to a local copy; suite SKIPPED"; continue; }
    fi
    REQ_DIR="$GATHER/requiem"
    [ -d "$REQ_DIR" ] || { mkdir -p "$REQ_DIR"; unzip -oq "$REQ_ZIP" -d "$REQ_DIR"; }

    for ONT in $REQUIEM_ONTOLOGIES; do
      OWL="$(find "$REQ_DIR" -iname "$ONT.owl" | head -1 || true)"
      [ -z "$OWL" ] && { log "requiem/$ONT: ontology not found in zip — skipping"; continue; }
      NT="$GATHER/requiem-$ONT.nt"
      [ -f "$NT" ] || "$RIOT" --output=ntriples "$OWL" > "$NT" 2>"$GATHER/requiem-$ONT.riot.err" || {
        log "riot OWL→NT failed for $OWL — skipping $ONT"; continue; }
      QSRC="$(grep -rl -- '<-' "$REQ_DIR" 2>/dev/null | grep -i "/$ONT[._-]" | head -1 || true)"
      [ -z "$QSRC" ] && QSRC="$(find "$REQ_DIR" -iname "$ONT*.txt" | head -1 || true)"
      [ -z "$QSRC" ] && { log "requiem/$ONT: no datalog query file found — skipping"; continue; }
      QDIR="$GATHER/requiem-$ONT-queries"
      log "requiem/$ONT: translating $(basename "$QSRC") → SPARQL: $(translate_requiem_queries "$NT" "$QSRC" "$QDIR" "$ONT")"
      [ -n "$(ls -A "$QDIR" 2>/dev/null)" ] || { log "requiem/$ONT: 0 translated queries — skipping"; continue; }

      SPARQ_TSV="$GATHER/requiem-$ONT-sparq.tsv"
      if want sparq; then
        log "sparq: requiem/$ONT rewriter-phase leg"
        "$SPARQ_BIN" "requiem-$ONT" "$NT" "$QDIR" > "$SPARQ_TSV"
        grep '^#' "$SPARQ_TSV" >&2 || true
      fi
      # Requiem ships ontologies+queries only (no data, no mappings): there is nothing for
      # Ontop's OBDA CLI to answer over, and its rewriter phase is not CLI-isolable — the
      # Ontop column for Requiem is therefore an honest NOT-APPLICABLE, pending the Java
      # rewriter-phase driver follow-up. sparq's UCQ sizes + rewrite times stand alone.
      emit_envelope "requiem-$ONT" "$NT" "$SPARQ_TSV" "" \
        "not-applicable: Requiem has no data/mappings for the OBDA CLI and Ontop's rewriter phase is not CLI-isolable (Java-driver follow-up recorded)" \
        "Requiem test suite ontology $ONT + datalog queries, translated to SPARQL by this harness ($REQUIEM_ZIP_URL)"
    done
    ;;

  *) log "unknown suite key '$SUITE' — skipping" ;;
  esac
done

log "done. Gather deps live under /tmp/reason-ql-gather + /tmp/ontop-cli + /tmp/jena-riot (delete when finished)."
