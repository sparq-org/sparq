//! (sq-b1hn) RSP-QL bench RUNNER — the self-asserting TSV emitter the
//! `bench/rsp/run.sh` per-commit gate consumes.
//!
//! sparq-rsp is **clock-free**: the windowed pipeline is a pure function of the
//! pushed `(triple, ts)` sequence (see the crate docs / README). That is the
//! design point this bench exploits: replaying a FIXED script yields a fully
//! DETERMINISTIC per-window result, so the gate is a diff of per-window
//! result-row counts against an expected TSV — a *stronger* correctness gate
//! than any wall-clock RSP benchmark can offer (no timing flake, no scheduler
//! nondeterminism). Design: research/capability-benchmark-program.md §3.6.
//!
//! It emits, on STDOUT, the 3-column contract `<metric>\t<count>\t<unit>` that
//! the ci-bench `rsp` hook (and `run.sh`) harvest:
//!
//!   1. DETERMINISTIC per-window row-count gate (`unit = rows`). For each
//!      scenario × `EvalMode`, one `rsp_<scenario>_<mode>_w<k>_rows` line per
//!      closed window. run.sh asserts every one against `bench/rsp/expected.tsv`;
//!      a divergence is an S2R/R2R/eval-mode regression and fails the run. The
//!      3-EvalMode equivalence is implicit: Rebuild / PersistentDict / Delta each
//!      emit their own lines and expected.tsv pins them all to the same counts.
//!   2. SRBench correctness ORACLE (`unit = rows`): an SRBench-shaped multi-window
//!      JOIN over two LOD-weather streams (observations ⋈ station metadata),
//!      `rsp_srbench_<q>_w<k>_rows` per tick. This is the honest comparison
//!      surface — the natural RSP peers (C-SPARQL / CQELS / RSP4J) are
//!      wall-clock service engines, so a throughput head-to-head is
//!      apples-to-oranges (different time model). Correctness/expressivity is the
//!      meaningful axis; perf head-to-head is OUT OF SCOPE (see README).
//!   3. ADVISORY throughput (`unit = triples_per_s`): a single
//!      `rsp_persistentdict_triples_per_s` line — trend-only, NOT a gate (it is
//!      machine-sensitive, and the deterministic counts above are the real gate).
//!
//! Run directly:  cargo run --release -p sparq-rsp --example rsp_oracle

use std::time::Instant;

use oxrdf::{Literal, NamedNode, Term};
use sparq_rsp::{ContinuousMultiQuery, ContinuousQuery, EvalMode, WindowResult, WindowSpec};

// ---------------------------------------------------------------- term helpers

fn iri(s: &str) -> Term {
    NamedNode::new_unchecked(format!("http://ex/{s}")).into()
}

fn nn(s: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("http://ex/{s}"))
}

/// `<http://ex/{s}> <http://ex/value> "v"^^xsd:integer`
fn val(s: &str, v: i32) -> [Term; 3] {
    [iri(s), iri("value"), Literal::from(v).into()]
}

/// `<http://ex/{s}> <http://ex/{p}> <http://ex/{o}>`
fn t(s: &str, p: &str, o: &str) -> [Term; 3] {
    [iri(s), iri(p), iri(o)]
}

// ----------------------------------------------- the FIXED single-window script
//
// One pinned `(triple, ts)` replay sequence shared by every single-window
// scenario. Hand-built to exercise: a multi-sensor join+aggregate, set semantics
// (a duplicate triple), an EMPTY window (a gap in the data), a non-zero start,
// and boundary-internal arrivals. Because the pipeline is clock-free this is a
// pure input → every closed window's row count is deterministic and diffable.
fn single_window_script() -> Vec<([Term; 3], u64)> {
    vec![
        (t("s1", "in", "kitchen"), 0),
        (val("s1", 20), 1),
        (val("s1", 22), 4),
        (t("s2", "in", "hall"), 5),
        (val("s2", 30), 6),
        (val("s2", 30), 8), // duplicate of the previous triple → set semantics
        // gap: nothing in [10,20) → an EMPTY window
        (t("s1", "in", "kitchen"), 21),
        (val("s1", 100), 22),
        (val("s1", 26), 28), // boundary-internal
        (t("s3", "in", "hall"), 31),
        (val("s3", 5), 33),
        (val("s3", 7), 39),
        (val("s1", 9), 41), // [40,50) (or its slide) — keeps the slide overlapping
    ]
}

/// One single-window scenario: a label, its `WindowSpec`, and the SPARQL.
struct Scenario {
    label: &'static str,
    spec: WindowSpec,
    sparql: &'static str,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // Tumbling AVG: one result row per non-empty window, zero for empties.
        Scenario {
            label: "tumbling_avg",
            spec: WindowSpec::time(10, 10),
            sparql: "SELECT (AVG(?v) AS ?avg) WHERE { ?s <http://ex/value> ?v }",
        },
        // Sliding SUM per sensor (RANGE 20 STEP 10 → 50% overlap): one row per
        // distinct sensor present in each (overlapping) window.
        Scenario {
            label: "sliding_sum",
            spec: WindowSpec::time(20, 10),
            sparql: "SELECT ?s (SUM(?v) AS ?sum) WHERE { ?s <http://ex/value> ?v } \
                     GROUP BY ?s ORDER BY ?s",
        },
        // Tumbling GROUP BY join: room ⋈ value, one row per room with readings.
        Scenario {
            label: "tumbling_groupby_join",
            spec: WindowSpec::time(20, 20),
            sparql: "SELECT ?room (AVG(?v) AS ?avg) (COUNT(?v) AS ?n) \
                     WHERE { ?s <http://ex/in> ?room . ?s <http://ex/value> ?v } \
                     GROUP BY ?room ORDER BY ?room",
        },
    ]
}

/// Runs one scenario × one `EvalMode` over the fixed script, emitting one
/// `rsp_<scenario>_<mode>_w<k>_rows<TAB><count><TAB>rows` line per closed window.
fn run_single_window_gate(sc: &Scenario, mode: EvalMode, script: &[([Term; 3], u64)]) {
    let mode_tag = match mode {
        EvalMode::Rebuild => "rebuild",
        EvalMode::PersistentDict => "pdict",
        EvalMode::Delta => "delta",
        // (sq-j39) Snapshot is observationally identical to Delta
        // (same per-window content), and the 4-way equivalence is pinned by the
        // crate tests; the deterministic gate's golden output keeps the original
        // three tags, so Snapshot is not added to the gate's iteration loop.
        EvalMode::Snapshot => "snapshot",
    };
    let mut q = ContinuousQuery::register(sc.sparql, sc.spec)
        .expect("valid query")
        .with_mode(mode);
    let mut k = 0u32;
    let mut emit = |r: WindowResult| {
        println!("rsp_{}_{}_w{}_rows\t{}\trows", sc.label, mode_tag, k, r.rows.len());
        k += 1;
    };
    for (triple, ts) in script {
        q.push(triple.clone(), *ts, &mut emit).expect("eval");
    }
    q.flush(&mut emit).expect("eval");
}

// ------------------------------------------------ SRBench correctness ORACLE
//
// SRBench (Zhang et al.) is the canonical RSP correctness/expressivity suite over
// real Linked-Open-Data WEATHER streams: sensor OBSERVATIONS joined against
// STATION metadata. We reproduce that SHAPE deterministically: an observations
// stream (a value per station per tick) and a station-metadata stream (each
// station's US state, re-announced each window the way a metadata stream emits a
// per-window snapshot), joined per synchronized window. The point is EXPRESSIVITY
// + CORRECTNESS (cross-window named-graph JOIN + aggregation), not speed — the
// time model differs from a wall-clock RSP engine, so perf is out of scope.
// Counts are deterministic, asserted in expected.tsv.
//
// Both windows tumble RANGE 10 STEP 10 so they close on the SAME boundaries and
// the join is evaluated per synchronized window: at each tick the obs window and
// the meta window each contribute their just-closed content, and the engine's
// cross-named-graph join binds `?st` between them.

/// A fixed SRBench-shaped script: `(stream, triple, ts)`. Stream `obs` carries
/// `<station> <value> v`; stream `meta` carries `<station> <state> <usstate>`,
/// re-announced inside each observation window so the join has metadata to bind.
fn srbench_script() -> Vec<(NamedNode, [Term; 3], u64)> {
    let obs = nn("obs");
    let meta = nn("meta");
    let mut s: Vec<(NamedNode, [Term; 3], u64)> = Vec::new();
    // Two stations in NY, one in CA. Metadata snapshot re-announced each window
    // (a metadata stream emits the current registry per window).
    let announce_meta = |s: &mut Vec<(NamedNode, [Term; 3], u64)>, base: u64| {
        s.push((meta.clone(), t("stA", "state", "NY"), base));
        s.push((meta.clone(), t("stB", "state", "NY"), base));
        s.push((meta.clone(), t("stC", "state", "CA"), base));
    };
    // --- Window 0 = [0,10): A, B, C each report → join yields all three.
    announce_meta(&mut s, 0);
    s.push((obs.clone(), val("stA", 12), 1));
    s.push((obs.clone(), val("stB", 15), 3));
    s.push((obs.clone(), val("stC", 20), 7));
    // --- Window 1 = [10,20): only A and C report (B silent) → two joined rows.
    announce_meta(&mut s, 10);
    s.push((obs.clone(), val("stA", 11), 12));
    s.push((obs.clone(), val("stC", 25), 18));
    // --- Window 2 = [20,30): A reports twice (set semantics on equal triples) +
    // an UNKNOWN station stZ with NO metadata → it must NOT join (inner join).
    announce_meta(&mut s, 20);
    s.push((obs.clone(), val("stA", 9), 21));
    s.push((obs.clone(), val("stA", 9), 22)); // duplicate value-triple
    s.push((obs.clone(), val("stZ", 99), 24)); // no metadata for stZ → unjoined
    s.push((obs.clone(), val("stC", 30), 28));
    // A heartbeat on obs at ts 35 closes window 2.
    s.push((obs.clone(), val("stA", 1), 35));
    s
}

/// One SRBench query: a label and its RSP-QL multi-window text.
struct SrbenchQuery {
    label: &'static str,
    rspql: &'static str,
}

fn srbench_queries() -> Vec<SrbenchQuery> {
    vec![
        // Q1 (SRBench-style join): every observation joined to its station's
        // state — the canonical "annotate the stream from the static graph"
        // pattern. One row per joined observation per window.
        SrbenchQuery {
            label: "join",
            rspql: "\
REGISTER STREAM <http://ex/out> AS
SELECT ?st ?state ?v WHERE {
  WINDOW <http://ex/wo> { ?st <http://ex/value> ?v }
  WINDOW <http://ex/wm> { ?st <http://ex/state> ?state }
}
FROM NAMED WINDOW <http://ex/wo> ON <http://ex/obs> RANGE 10 STEP 10
FROM NAMED WINDOW <http://ex/wm> ON <http://ex/meta> RANGE 10 STEP 10",
        },
        // Q2 (SRBench-style aggregate-after-join): readings per US state per
        // window — the join feeds a GROUP BY. One row per distinct joined state.
        SrbenchQuery {
            label: "groupby_state",
            rspql: "\
REGISTER STREAM <http://ex/out> AS
SELECT ?state (COUNT(?v) AS ?n) WHERE {
  WINDOW <http://ex/wo> { ?st <http://ex/value> ?v }
  WINDOW <http://ex/wm> { ?st <http://ex/state> ?state }
}
GROUP BY ?state ORDER BY ?state
FROM NAMED WINDOW <http://ex/wo> ON <http://ex/obs> RANGE 10 STEP 10
FROM NAMED WINDOW <http://ex/wm> ON <http://ex/meta> RANGE 10 STEP 10",
        },
    ]
}

/// Runs one SRBench query over the fixed script, emitting one
/// `rsp_srbench_<q>_w<k>_rows` line per join evaluation tick.
fn run_srbench_oracle(sq: &SrbenchQuery, script: &[(NamedNode, [Term; 3], u64)]) {
    let mut q = ContinuousMultiQuery::register(sq.rspql).expect("valid RSP-QL");
    let mut k = 0u32;
    let mut emit = |r: WindowResult| {
        println!("rsp_srbench_{}_w{}_rows\t{}\trows", sq.label, k, r.rows.len());
        k += 1;
    };
    for (stream, triple, ts) in script {
        q.push(stream, triple.clone(), *ts, &mut emit).expect("eval");
    }
    q.flush(&mut emit).expect("eval");
}

// ----------------------------------------------------- ADVISORY throughput
//
// Trend-only `rsp_persistentdict_triples_per_s` — NOT a gate (machine-sensitive).
// The deterministic per-window counts above are the gate.

const N_THROUGHPUT_TRIPLES: u64 = 200_000;
const N_SENSORS: u64 = 100;

fn run_throughput() {
    let sparql = "SELECT (AVG(?v) AS ?avg) WHERE { ?s <http://ex/value> ?v }";
    let mut q = ContinuousQuery::register(sparql, WindowSpec::time(1_000, 1_000))
        .expect("valid query")
        .with_mode(EvalMode::PersistentDict);
    let start = Instant::now();
    for ts in 0..N_THROUGHPUT_TRIPLES {
        let s = format!("sensor/{}", ts % N_SENSORS);
        q.push(val(&s, (ts % 50) as i32), ts, |_| {}).expect("eval");
    }
    q.flush(|_| {}).expect("eval");
    let dt = start.elapsed().as_secs_f64();
    let tps = if dt > 0.0 {
        (N_THROUGHPUT_TRIPLES as f64 / dt).round() as u64
    } else {
        0
    };
    println!("rsp_persistentdict_triples_per_s\t{tps}\ttriples_per_s");
}

fn main() {
    // 1. DETERMINISTIC per-window row-count gate: every scenario × every EvalMode.
    let script = single_window_script();
    for sc in scenarios() {
        for mode in [EvalMode::Rebuild, EvalMode::PersistentDict, EvalMode::Delta] {
            run_single_window_gate(&sc, mode, &script);
        }
    }

    // 2. SRBench correctness oracle (the EYE role; correctness/expressivity only).
    let sr_script = srbench_script();
    for sq in srbench_queries() {
        run_srbench_oracle(&sq, &sr_script);
    }

    // 3. ADVISORY throughput (trend-only).
    run_throughput();
}
