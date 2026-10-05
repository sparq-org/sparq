#!/usr/bin/env bash
# sq-hmd7l.7 — same-box MATERIALIZATION (deductive-closure) comparison harness:
# sparq `reason` (OWL-RL / RDFS) vs Apache Jena rule reasoners vs VLog vs Nemo computing the
# SAME closure over the SAME LUBM (ABox + TBox) N-Triples, emitting one canonical-competitor-
# results ENVELOPE per scale (the JSON shape of scripts/bench/shacl-same-box.sh, so
# scripts/bench/ingest-canonical-competitors.mjs can pick a future canonical gather up
# unchanged).
#
# 🤖 SPARQ agent. Materialization (D6 in the RDFox gap matrix) had NO competitor baseline
# (NOT-MEASURED); this script is the durable, reusable gather recipe. A run on the shared
# work box is NON-canonical (canonical:false in the envelope, always); the univ>=100 EC2 run
# belongs to the canonical wave sq-hmd7l.26 — this harness does NOT launch EC2.
#
# WORKLOAD (shared across all engines): the deductive closure of the deterministic LUBM(univ)
# ABox + the Univ-Bench OWL TBox (bench/lubm/gen.sh — READ-ONLY; run.sh is NOT touched), at
# each scale in $LUBM_UNIVS (default "1 10").
#
# ── PROFILE / RULE-SET FIDELITY (recorded per-column, NEVER silently absorbed) ─────────────
# The compared "closure" is only meaningful if the RULE SET matches. It does NOT match across
# engines, and the envelope records the exact rule set each column ran so the closure-size
# delta is ATTRIBUTABLE, not hidden:
#   * sparq  `reason ... owl`  = the FULL W3C OWL 2 RL/RDF rule table (cls-*/cax-*/scm-*/prp-*
#            incl. prp-trp/prp-inv/cls-svf/cls-int — the exact rules LUBM Q6/Q9/Q11/Q12/Q13
#            depend on; crates/sparq-reason/src/owl.rs). `reason ... rdfs` = the RDFS subset.
#   * Jena   has NO full OWL 2 RL reasoner. OWL_MICRO/OWL_MINI/OWL are Jena's own OWL-SUBSET
#            rule reasoners (incomplete vs OWL 2 RL); RDFS is Jena's RDFS rule set. Jena also
#            adds axiomatic/reflexive triples sparq does not, and de-dups the ABox on load, so
#            its closure size DIFFERS by construction. This is a PROFILE difference, not a bug.
#   * VLog / Nemo are GENERAL Datalog engines. A like-for-like closure needs a Datalog encoding
#            of the SAME rule set (.dlog / .rls) that REPRODUCES sparq's closure count. Those
#            encodings now exist and are VALIDATED set-for-set against sparq (sq-hmd7l.30/.31):
#            bench/reason-encodings/{vlog/*.dlog,nemo/*.rls}. When the vlog/nmo binary is present
#            the adapters run them and the closure count AGREES with sparq (owl=150589,
#            rdfs=126732 @ LUBM(1)); when the binary or encoding is absent the column emits an
#            HONEST NOT-RUN-LOCALLY with the exact blocker, never a fabricated number.
#
# ORACLE: closure-size cross-check. sparq's `reason` self-reports its closure count on stderr
# (`reasoned [OwlRl]: <base> -> <closure> triples ...`); the harness asserts that count against
# a pinned per-scale/per-profile expected (KNOWN_CLOSURE below) at univ=1, and records every
# engine's closure size in the envelope's count_crosscheck with `all_agree` = engines that ran
# the SAME rule set and agree. INVARIANT: no throughput row without a closure-count agreement
# or an explicitly-recorded semantic-difference caveat.
#
# METHODOLOGY (every engine's timed figure = MATERIALIZE-ON-A-LOADED-GRAPH, same basis):
#   * sparq: `sparq-cli reason <combined> ntriples <profile> <out.nt>` best-of-N; the timed
#     figure is sparq's SELF-REPORTED materialize time (parse EXCLUDED — comparable to the
#     other engines' timed-materialize-on-a-loaded-graph). Load is recorded separately.
#   * Jena: one JVM per (profile) under `timeout`; the Java driver loads the graph once
#     (advisory load), then times InfModel materialization best-of-N (JVM start-up + parse
#     stay OUTSIDE the timed section). A timeout/error degrades to an honest ERROR row.
#   * VLog / Nemo: python adapters run the VALIDATED bench/reason-encodings/{vlog,nemo}/
#     program (default per profile); if the binary is absent an honest NOT-RUN-LOCALLY row
#     (never a fabricated number). Their closure count AGREES with sparq set-for-set.
#     their timed figure is the ENGINE'S OWN loaded-graph
#     materialization report (VLog `Runtime materialization`, Nemo's `Reasoning:` breakdown)
#     — NOT the whole-subprocess wall, which at univ>=100 would fold the multi-GB .nt load
#     + closure export into the competitor's time (an overstatement biased for sparq). The
#     per-row TSV carries a 4th `timed=` basis column; if the self-report is ever absent the
#     adapters fall back to whole-process wall and SAY SO in that column, never silently.
#
# USAGE
#   scripts/bench/materialize-same-box.sh                 # both scales, all engines
#   ONLY=sparq LUBM_UNIVS=1 scripts/bench/materialize-same-box.sh   # acceptance: exit 0
#
# TUNABLES (env; all have safe defaults):
#   LUBM_UNIVS    LUBM scales to run                      (default "1 10")
#   MAT_ITERS     best-of-N per scale, parallel list      (default "3 1")
#   MAT_PROFILES  sparq reasoning profiles to run         (default "owl rdfs")
#   TIMEOUT_S     per-engine materialize cap, s           (default 900)
#   ONLY          engine subset of "sparq jena vlog nemo" (default all)
#   OUT_DIR       envelope output dir (default /tmp/materialize-same-box-results;
#                 a canonical run points this at bench/canonical-competitor-results/<date>/)
#   CANONICAL     1 = a dedicated quiet-box run (default 0: NON-canonical)
#   CLI           the sparq-cli binary (default: build it)
#   JENA_HOME     apache-jena distribution root; auto-downloaded under /tmp/jena-reason if unset
#   JENA_VERSION  default 5.4.0 (archive.apache.org)
#   JENA_PROFILES Jena reasoner profiles to run (default "rdfs owl-micro"); owl-mini/owl are
#                 SLOW on LUBM(1) and typically time out — add them explicitly if wanted.
#   JAVA_XMX      JVM max heap for the Jena driver, e.g. 24g (default: JVM default — at
#                 univ>=100 the default ~25%-of-RAM heap OOMs the InfModel; set it)
#   VLOG / NEMO   paths to the vlog / nmo binaries (adapters emit NOT-RUN-LOCALLY if absent)
#   VLOG_VERSION / NEMO_VERSION   engine version strings recorded in the envelope's
#                 engines map (e.g. the pinned git commit / release tag a canonical
#                 gather built — provenance for citable numbers; default empty)
#
# SCRATCH: the Jena tarball lives under /tmp/jena-reason (gather-only dep, NOT committed —
# engines stay out of git per AGENTS.md); the LUBM corpus is gitignored + regenerable. Delete
# when done:  rm -rf /tmp/jena-reason /tmp/lubm /tmp/materialize-same-box.*
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

LUBM_UNIVS="${LUBM_UNIVS:-1 10}"
MAT_ITERS="${MAT_ITERS:-3 1}"
MAT_PROFILES="${MAT_PROFILES:-owl rdfs}"
TIMEOUT_S="${TIMEOUT_S:-900}"
ONLY="${ONLY:-sparq jena vlog nemo}"
OUT_DIR="${OUT_DIR:-/tmp/materialize-same-box-results}"
CANONICAL="${CANONICAL:-0}"
JENA_VERSION="${JENA_VERSION:-5.4.0}"
JENA_HOME="${JENA_HOME:-/tmp/jena-reason/apache-jena-$JENA_VERSION}"
JENA_PROFILES="${JENA_PROFILES:-rdfs owl-micro}"
JAVA_XMX="${JAVA_XMX:-}"
VLOG_VERSION="${VLOG_VERSION:-}"
NEMO_VERSION="${NEMO_VERSION:-}"
CLI="${CLI:-$ROOT/target/release/sparq-cli}"

# Pinned KNOWN closure counts for the deterministic LUBM(1) corpus — the acceptance oracle.
# sparq OWL-RL closure = 150589, RDFS closure = 126732 (gen.sh -univ 1 -seed 0; verified
# against `sparq-cli reason`). Only univ=1 is pinned (deterministic); larger scales are
# cross-checked engine-vs-engine, not against a committed constant.
declare -A KNOWN_CLOSURE=( ["1:owl"]="150589" ["1:rdfs"]="126732" )

log() { printf '[materialize-same-box] %s\n' "$*" >&2; }
want() { [[ " $ONLY " == *" $1 "* ]]; }

mkdir -p "$OUT_DIR"
TMP="$(mktemp -d /tmp/materialize-same-box.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

# ---- 0. engines --------------------------------------------------------------
if want sparq && [ ! -x "$CLI" ]; then
  log "building sparq-cli (cargo build --release -p sparq-cli)"
  cargo build --release -p sparq-cli
fi

if want jena; then
  if [ ! -d "$JENA_HOME/lib" ]; then
    log "downloading apache-jena $JENA_VERSION to /tmp/jena-reason (gather-only dep)"
    mkdir -p /tmp/jena-reason
    curl -sSL -o "/tmp/jena-reason/apache-jena-$JENA_VERSION.tar.gz" \
      "https://archive.apache.org/dist/jena/binaries/apache-jena-$JENA_VERSION.tar.gz"
    tar xzf "/tmp/jena-reason/apache-jena-$JENA_VERSION.tar.gz" -C /tmp/jena-reason
  fi
  log "compiling jena_reason_adapter against $JENA_HOME/lib"
  javac -proc:none -cp "$JENA_HOME/lib/*" -d "$TMP/jena-classes" \
    "$ROOT/scripts/bench-adapters/jena_reason_adapter.java"
  JENA_VER="$(java -cp "$JENA_HOME/lib/*" org.apache.jena.riot.riotcmd.riot --version 2>/dev/null | head -1 || echo "apache-jena-$JENA_VERSION")"
fi

GIT_COMMIT="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"

# ---- 1. one gather per scale --------------------------------------------------
read -r -a UNIVS_ARR <<< "$LUBM_UNIVS"
read -r -a ITERS_ARR <<< "$MAT_ITERS"
read -r -a PROFS_ARR <<< "$MAT_PROFILES"

# map a sparq profile to the closest Jena profile for the count/timing crosscheck.
jena_profile_for() {
  case "$1" in
    rdfs) echo "rdfs" ;;
    owl)  echo "owl-micro" ;;   # Jena has no full OWL 2 RL; owl-micro is the fast subset
    *)    echo "" ;;
  esac
}

ASSERT_FAIL=0

for i in "${!UNIVS_ARR[@]}"; do
  UNIV="${UNIVS_ARR[$i]}"
  ITERS="${ITERS_ARR[$i]:-1}"
  log "=== scale LUBM($UNIV), iters=$ITERS ==="

  # gen.sh emits two lines: data.nt then ontology.nt. Combine (ABox + TBox) — the SAME
  # input every engine closes.
  mapfile -t ARTS < <("$ROOT/bench/lubm/gen.sh" "$UNIV" 0)
  DATA="${ARTS[0]}"
  ONTO="${ARTS[1]}"
  COMBINED="$TMP/combined-u$UNIV.nt"
  cat "$DATA" "$ONTO" > "$COMBINED"
  NTRIPLES="$(wc -l < "$COMBINED")"
  log "combined (ABox+TBox) = $COMBINED (~$NTRIPLES triples)"

  SCALE_TMP="$TMP/u$UNIV"
  mkdir -p "$SCALE_TMP"
  : > "$SCALE_TMP/sparq.tsv"
  : > "$SCALE_TMP/jena.tsv"
  : > "$SCALE_TMP/vlog.tsv"
  : > "$SCALE_TMP/nemo.tsv"

  for prof in "${PROFS_ARR[@]}"; do
    # -- sparq: reason best-of-N; parse the self-reported closure count + materialize time.
    if want sparq; then
      log "sparq: reason $prof x$ITERS"
      best_us=""
      closure=""
      for _ in $(seq 1 "$ITERS"); do
        rerr="$SCALE_TMP/sparq-$prof.err"
        if ! timeout "$TIMEOUT_S" "$CLI" reason "$COMBINED" ntriples "$prof" "$SCALE_TMP/sparq-$prof-out.nt" \
            > /dev/null 2> "$rerr"; then
          log "sparq $prof FAILED/timeout (see $rerr)"
          closure="ERROR"; best_us="timeout"; break
        fi
        # line: "reasoned [OwlRl]: 103410 -> 150589 triples (+49751 entailed) in 0.227s"
        line="$(grep -oE 'reasoned \[[A-Za-z]+\]: [0-9]+ -> [0-9]+ triples \(\+[0-9]+ entailed\) in [0-9.]+s' "$rerr" | head -1)"
        c="$(echo "$line" | grep -oE '\-> [0-9]+' | grep -oE '[0-9]+')"
        b="$(echo "$line" | grep -oE '[0-9.]+s$' | tr -d 's')"
        [ -n "$c" ] && closure="$c"
        b_us="$(awk -v s="$b" 'BEGIN{printf "%.1f", s*1e6}')"
        if [ -z "$best_us" ] || awk -v a="$b_us" -v m="$best_us" 'BEGIN{exit !(a<m)}'; then best_us="$b_us"; fi
      done
      printf '%s\t%s\t%s\n' "$prof" "${closure:-ERROR}" "${best_us:-na}" >> "$SCALE_TMP/sparq.tsv"

      # ---- acceptance oracle: assert the pinned closure count at univ=1 ----
      exp="${KNOWN_CLOSURE["$UNIV:$prof"]:-}"
      if [ -n "$exp" ]; then
        if [ "${closure:-}" = "$exp" ]; then
          log "ORACLE OK: sparq $prof closure=$closure == expected $exp (LUBM($UNIV))"
        else
          log "ORACLE FAIL: sparq $prof closure=${closure:-none} != expected $exp (LUBM($UNIV))"
          ASSERT_FAIL=1
        fi
      fi
    fi

    # -- Jena: one JVM per (mapped) profile under `timeout`.
    if want jena; then
      jp="$(jena_profile_for "$prof")"
      # only run Jena for profiles it is registered as running (from $JENA_PROFILES)
      if [ -n "$jp" ] && [[ " $JENA_PROFILES " == *" $jp "* ]]; then
        log "jena: $jp (for sparq '$prof') x$ITERS (cap ${TIMEOUT_S}s${JAVA_XMX:+, -Xmx$JAVA_XMX})"
        if row="$(timeout "$TIMEOUT_S" java ${JAVA_XMX:+-Xmx"$JAVA_XMX"} -cp "$JENA_HOME/lib/*:$TMP/jena-classes" \
            JenaReasonAdapter "$COMBINED" "$jp" "$ITERS" 2>>"$SCALE_TMP/jena.err")"; then
          # rewrite the leading column from the Jena profile name to the sparq profile
          # name so the crosscheck aligns rows; keep the Jena profile in jena.err meta.
          echo "$row" | awk -F'\t' -v p="$prof" '{printf "%s\t%s\t%s\n", p, $2, $3}' >> "$SCALE_TMP/jena.tsv"
        else
          printf '%s\tERROR\ttimeout(%s)\n' "$prof" "$jp" >> "$SCALE_TMP/jena.tsv"
        fi
      fi
    fi

    # -- VLog: python adapter (NOT-RUN-LOCALLY if binary/encoding absent). The optional
    # 4th column is the adapter's honest `timed=` basis — preserve it.
    if want vlog; then
      log "vlog: $prof x$ITERS"
      TIMEOUT_S="$TIMEOUT_S" python3 "$ROOT/scripts/bench-adapters/vlog_adapter.py" \
        "$COMBINED" "$prof" "$ITERS" 2>>"$SCALE_TMP/vlog.err" \
        | awk -F'\t' -v p="$prof" '{printf "%s\t%s\t%s%s\n", p, $2, $3, (NF>=4 ? "\t"$4 : "")}' >> "$SCALE_TMP/vlog.tsv"
    fi

    # -- Nemo: python adapter (NOT-RUN-LOCALLY if binary/encoding absent). 4th col as above.
    if want nemo; then
      log "nemo: $prof x$ITERS"
      TIMEOUT_S="$TIMEOUT_S" python3 "$ROOT/scripts/bench-adapters/nemo_adapter.py" \
        "$COMBINED" "$prof" "$ITERS" 2>>"$SCALE_TMP/nemo.err" \
        | awk -F'\t' -v p="$prof" '{printf "%s\t%s\t%s%s\n", p, $2, $3, (NF>=4 ? "\t"$4 : "")}' >> "$SCALE_TMP/nemo.tsv"
    fi
  done

  # ---- 2. assemble the envelope (canonical-competitor-results JSON shape) ----
  TS="$(date -u +%Y%m%dT%H%M%SZ)"
  OUT="$OUT_DIR/materialize-lubm${UNIV}-${TS}.json"
  CANONICAL="$CANONICAL" UNIV="$UNIV" ITERS="$ITERS" COMBINED="$COMBINED" NTRIPLES="$NTRIPLES" \
  GIT_COMMIT="$GIT_COMMIT" SCALE_TMP="$SCALE_TMP" OUT="$OUT" ONLY="$ONLY" \
  JENA_VER="${JENA_VER:-}" JENA_PROFILES="$JENA_PROFILES" TIMEOUT_S="$TIMEOUT_S" \
  MAT_PROFILES="$MAT_PROFILES" VLOG_VERSION="$VLOG_VERSION" NEMO_VERSION="$NEMO_VERSION" \
  python3 - <<'PYEOF'
import json, os, platform

scale_tmp = os.environ["SCALE_TMP"]
only = os.environ["ONLY"].split()
canonical = os.environ["CANONICAL"] == "1"
engines = [e for e in ("sparq", "jena", "vlog", "nemo") if e in only]


def read_tsv(engine):
    path = os.path.join(scale_tmp, f"{engine}.tsv")
    rows = {}
    if os.path.exists(path):
        for line in open(path):
            f = line.rstrip("\n").split("\t")
            if len(f) >= 3:
                # optional 4th col = the adapter's honest `timed=` basis (vlog/nemo).
                rows[f[0]] = dict(closure=f[1], us=f[2], basis=(f[3] if len(f) > 3 else ""))
    return rows


data = {e: read_tsv(e) for e in engines}
profiles = [p for p in os.environ["MAT_PROFILES"].split() if any(p in data[e] for e in engines)]

engines_meta = {}
if "sparq" in engines:
    engines_meta["sparq"] = {
        "version": os.environ["GIT_COMMIT"],
        "rule_set": "FULL W3C OWL 2 RL/RDF rule table (owl) / RDFS subset (rdfs) — crates/sparq-reason",
        "mode": "sparq-cli reason best-of-N; timed = self-reported materialize (parse excluded)",
    }
if "jena" in engines:
    engines_meta["jena"] = {
        "version": os.environ.get("JENA_VER", ""),
        "rule_set": (
            "Jena's OWN rule reasoners — NOT full OWL 2 RL. Mapping: sparq owl -> Jena "
            "OWL_MICRO (fast OWL subset); sparq rdfs -> Jena RDFS ruleset. Jena adds "
            "axiomatic/reflexive triples + de-dups the ABox on load, so closure size "
            "DIFFERS by construction (documented PROFILE difference, not a bug). Jena "
            f"profiles enabled: {os.environ['JENA_PROFILES']}."
        ),
        "mode": "scripts/bench-adapters/jena_reason_adapter.java: load once (advisory), time InfModel materialize best-of-N; one JVM per profile under `timeout`",
    }
if "vlog" in engines:
    engines_meta["vlog"] = {
        "version": os.environ.get("VLOG_VERSION", ""),
        "rule_set": "general Datalog running the VALIDATED bench/reason-encodings/vlog/{owl-rl,rdfs}.dlog — the SAME rule set as sparq's compared profile, reproducing its closure SET-FOR-SET (sq-hmd7l.30). If the vlog binary or the encoding is absent, the column is NOT-RUN-LOCALLY.",
        "mode": "scripts/bench-adapters/vlog_adapter.py: writes an INMEMORY edb.conf binding the .nt to TE, runs `mat --storemat`, counts the materialized T relation; timed = VLog's SELF-REPORTED `Runtime materialization` (EDB load + storemat write EXCLUDED — same loaded-graph basis as sparq/Jena; per-row `timed=` basis column, wall fallback declared)",
    }
if "nemo" in engines:
    engines_meta["nemo"] = {
        "version": os.environ.get("NEMO_VERSION", ""),
        "rule_set": "general Datalog (Rust-native) running the VALIDATED bench/reason-encodings/nemo/{owl-rl,rdfs}.rls — the SAME rule set as sparq's compared profile, reproducing its closure SET-FOR-SET (sq-hmd7l.31). If the nmo binary or the encoding is absent, the column is NOT-RUN-LOCALLY.",
        "mode": "scripts/bench-adapters/nemo_adapter.py: substitutes the corpus path into the .rls, runs `nmo --report short`, counts the exported `closed` relation; timed = Nemo's SELF-REPORTED `Reasoning:` breakdown (import + export EXCLUDED — same loaded-graph basis as sparq/Jena; per-row `timed=` basis column, wall fallback declared)",
    }

note_canonical = (
    "CANONICAL: dedicated quiet box, one engine active at a time on the SAME LUBM "
    "(ABox+TBox) input; timed = materialize-on-a-loaded-graph best-of-N."
    if canonical else
    "NON-canonical FIRST READ: shared work box (not a dedicated quiet instance). "
    "Timings are directional only — do NOT bake into docs/dashboards. The harness "
    "(scripts/bench/materialize-same-box.sh) is the durable deliverable; rerun with "
    "CANONICAL=1 on a dedicated EC2 box (sq-hmd7l.26, univ>=100) for citable numbers."
)

# per-profile closure crosscheck. same_ruleset_agree folds together the engines that
# ran the SAME compared rule set: sparq PLUS VLog/Nemo when they ran the VALIDATED
# bench/reason-encodings/{vlog,nemo}/ program (which reproduces sparq's closure set-for-
# set, sq-hmd7l.30/.31). Jena runs its OWN OWL-subset/RDFS reasoner (a different rule set)
# and is recorded with an explicit profile caveat, NEVER silently reconciled.
SAME_RULESET_ENGINES = ("sparq", "vlog", "nemo")
cross = {}
for p in profiles:
    row = {}
    same_ruleset_counts = set()
    for e in engines:
        r = data[e].get(p)
        c = r["closure"] if r else "n/a"
        row[e] = c
        # sparq + the validated-encoding Datalog engines run the compared rule set; a
        # numeric closure from any of them is folded into the agreement check. A
        # NOT-RUN-LOCALLY / ERROR value is skipped (never counted as a disagreement).
        if e in SAME_RULESET_ENGINES and c not in ("n/a", "ERROR", "NOT-RUN-LOCALLY"):
            same_ruleset_counts.add(c)
    row["same_ruleset_agree"] = len(same_ruleset_counts) == 1
    row["profile_caveat"] = (
        "sparq + VLog + Nemo (where present with the validated encoding) run the SAME "
        "compared rule set and their closure sizes are expected to AGREE (folded into "
        "same_ruleset_agree). A Jena column runs a DIFFERENT (OWL-subset/RDFS) rule set; "
        "its closure size is a documented profile difference, not an agreement, and is "
        "NEVER reconciled to sparq's."
    )
    cross[p] = row

env = {
    "host": platform.node(),
    "machine": platform.machine(),
    "os": platform.platform(),
    "timeout_s_per_engine": int(os.environ["TIMEOUT_S"]),
}

envelope = {
    "gather": "materialize-same-box-comparison",
    "wave": "materialize-competitor-baseline (sq-hmd7l.7)",
    "canonical": canonical,
    "canonical_note": note_canonical,
    "git_commit": os.environ["GIT_COMMIT"],
    # sq-hmd7l.32 — the suite id carries the SCALE so envelopes gathered at
    # different LUBM(univ) never fold into one ingest group (the ingest's cross-gather
    # count assertion would rightly refuse two scales' differing closure counts).
    "suite": f"materialize-competitors-lubm{os.environ['UNIV']}",
    "scale": f"LUBM({os.environ['UNIV']}) ABox+TBox, {os.environ['NTRIPLES']} input triples ({os.environ['COMBINED']})",
    "iters": int(os.environ["ITERS"]),
    "profiles": profiles,
    "tsv_format": "<profile>\\t<closure_triples|ERROR|NOT-RUN-LOCALLY>\\t<materialize_best_us|reason>[\\t<timed=basis>]",
    "engines": engines_meta,
    "statuses": {
        e: ("ok" if any(v.get("closure") not in (None, "ERROR", "NOT-RUN-LOCALLY") for v in data[e].values()) else "not-run/failed")
        for e in engines
    },
    "count_crosscheck": cross,
    "count_crosscheck_note": (
        "per-profile closure size. same_ruleset_agree = the engines running the EXACT "
        "compared rule set — sparq PLUS VLog/Nemo with the validated encodings "
        "(bench/reason-encodings/, sq-hmd7l.30/.31) — agree on the closure count (the "
        "acceptance oracle pins univ=1: owl=150589, rdfs=126732; the validated encodings "
        "reproduce it SET-FOR-SET). A Jena column runs a DIFFERENT (OWL-subset/RDFS) rule "
        "set, recorded HONESTLY as a profile caveat, NEVER reconciled to sparq's. This is "
        "why COUNT is checked before any timing."
    ),
    "env": env,
}
for e in engines:
    envelope[f"{e}_tsv"] = "\n".join(
        f"{p}\t{data[e][p]['closure']}\t{data[e][p]['us']}"
        + (f"\t{data[e][p]['basis']}" if data[e][p].get("basis") else "")
        for p in profiles if p in data[e]
    )

with open(os.environ["OUT"], "w") as fh:
    json.dump(envelope, fh, indent=2)
    fh.write("\n")
print(os.environ["OUT"])
PYEOF
  log "envelope: $OUT"
done

if [ "$ASSERT_FAIL" != 0 ]; then
  log "FAILED: one or more pinned closure-count oracles diverged (correctness regression)"
  exit 1
fi
log "done. Scratch deps live in /tmp/jena-reason (delete when finished)."
