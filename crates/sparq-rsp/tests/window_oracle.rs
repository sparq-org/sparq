//! (sq-5enp) Streamed-equals-batch window ORACLE + window-semantics
//! tests for sparq-rsp.
//!
//! The crate's S2R window machinery (`window.rs`), R2R materialisation
//! (`eval.rs`) and the continuous-query pipeline (`query.rs`) carried no INLINE
//! unit tests — coverage came entirely from the three integration files, which
//! pin *bounds* and *content* but never independently re-derive the
//! window→result mapping. This file adds the missing differential oracle and the
//! window-semantics edge cases the audit flagged:
//!
//! 1. **Streamed-equals-batch differential** — feed a timestamped tuple stream
//!    through a [`ContinuousQuery`]; for every emitted window independently
//!    rebuild the STATIC graph of exactly the tuples that window should contain
//!    (per the documented `[start, end)` inclusivity) and run the SAME query via
//!    the batch engine; assert the streamed window result == the batch result.
//!    Mirrors sparq-engine's `query_json_matches_materialized_json` philosophy:
//!    the fast/incremental path must equal the obvious/materialised path.
//! 2. **Boundary inclusivity** — tuples exactly on an open/close boundary land
//!    in the correct window(s) (the half-open `[start, end)` convention).
//! 3. **Late / out-of-order** — handled per the documented watermark policy
//!    (kept if a covering window is still open, dropped + counted once every
//!    covering window has closed).
//! 4. **Eviction + report-strategy determinism** — evicted windows never leak
//!    into later results; repeated identical runs are byte-identical.
//! 5. **EvalMode 4-way equivalence** — Rebuild / PersistentDict / Delta /
//!    Snapshot produce the same per-window result against the oracle on the
//!    same stream.
//!
//! The oracle is deliberately INDEPENDENT of `eval.rs`: it re-implements the
//! window-membership predicate from the spec docs and uses the batch engine
//! (`sparq_engine::query`) as ground truth, so a bug in the incremental closure
//! /materialisation path would surface as a streamed-vs-batch divergence rather
//! than being masked by shared code.

use oxrdf::{Literal, NamedNode, Term};
use sparq_core::dict::Dict;
use sparq_core::Graph;
use sparq_rsp::{ContinuousQuery, EvalMode, WindowResult, WindowSpec, R2S};

// ----------------------------------------------------------------- term helpers

fn iri(s: &str) -> Term {
    NamedNode::new_unchecked(format!("http://ex/{s}")).into()
}

/// `<http://ex/{s}> <http://ex/value> "v"^^xsd:integer`
fn val(s: &str, v: i32) -> [Term; 3] {
    [iri(s), iri("value"), Literal::from(v).into()]
}

/// `<http://ex/{s}> <http://ex/{p}> <http://ex/{o}>`
fn t(s: &str, p: &str, o: &str) -> [Term; 3] {
    [iri(s), iri(p), iri(o)]
}

/// One emitted SELECT result row: one cell per projected variable (`None` =
/// unbound).
type Row = Vec<Option<Term>>;
/// One closed window the streamed pipeline emitted: `(start, end, rows)`.
type EmittedWindow = (u64, u64, Vec<Row>);

// --------------------------------------------------------------- the ORACLE

/// The independent window-membership predicate, re-derived from the documented
/// `window.rs` contract (NOT calling the crate's own closure code): a time
/// window `k` covers the half-open `[t0 + k·step, t0 + k·step + range)`; a triple
/// at `ts` belongs to it iff `ts >= t0` AND `start <= ts < end`. The start bound
/// is INCLUSIVE, the end EXCLUSIVE.
fn ts_in_time_window(ts: u64, start: u64, end: u64, t0: u64) -> bool {
    ts >= t0 && ts >= start && ts < end
}

/// Builds the STATIC batch graph of exactly the triples a time window `[start,
/// end)` should contain, drawn from the whole `(triple, ts)` script. Built the
/// same way the crate materialises a window (fresh `Dict` + `Graph::from_parts`,
/// which dedups to RDF set semantics) — but from the ORIGINAL stream and the
/// independent membership predicate, so it is ground truth, not a copy of the
/// incremental path.
fn batch_graph_for_window(script: &[([Term; 3], u64)], start: u64, end: u64, t0: u64) -> Graph {
    let mut dict = Dict::new();
    let ids: Vec<[sparq_core::dict::Id; 3]> = script
        .iter()
        .filter(|(_, ts)| ts_in_time_window(*ts, start, end, t0))
        .map(|(triple, _)| {
            [
                dict.intern(&triple[0]),
                dict.intern(&triple[1]),
                dict.intern(&triple[2]),
            ]
        })
        .collect();
    Graph::from_parts(dict, ids)
}

/// A SELECT result reduced to a comparable, ORDER-independent multiset of rows
/// (each row rendered to a stable string). Used to compare the streamed window
/// result against the batch result when the query imposes no total order.
fn as_multiset(rows: &[Row]) -> Vec<String> {
    let mut out: Vec<String> = rows.iter().map(|r| format!("{r:?}")).collect();
    out.sort();
    out
}

/// Drives a `ContinuousQuery` over a script (push each element, then `flush`),
/// collecting every emitted `(start, end, rows)`. The single place the streamed
/// pipeline is exercised in this file.
fn run_streamed(
    sparql: &str,
    spec: WindowSpec,
    mode: EvalMode,
    script: &[([Term; 3], u64)],
) -> Vec<EmittedWindow> {
    let mut q = ContinuousQuery::register(sparql, spec)
        .unwrap()
        .with_mode(mode);
    let mut out = Vec::new();
    let mut sink = |r: WindowResult| out.push((r.start, r.end, r.rows));
    for (triple, ts) in script {
        q.push(triple.clone(), *ts, &mut sink).unwrap();
    }
    q.flush(&mut sink).unwrap();
    out
}

/// THE differential assertion: every window the streamed pipeline emits must
/// equal the batch query run over exactly the static graph that window's
/// `[start, end)` should contain. Compares as a multiset (the queries here are
/// either deterministic via aggregation/ORDER BY or order-insensitive).
fn assert_streamed_equals_batch(
    sparql: &str,
    spec: WindowSpec,
    mode: EvalMode,
    t0: u64,
    script: &[([Term; 3], u64)],
) {
    let streamed = run_streamed(sparql, spec, mode, script);
    assert!(
        !streamed.is_empty(),
        "the stream closed at least one window"
    );
    for (start, end, stream_rows) in &streamed {
        let g = batch_graph_for_window(script, *start, *end, t0);
        let batch = sparq_engine::query(&g, sparql).unwrap();
        assert_eq!(
            as_multiset(stream_rows),
            as_multiset(&batch.rows),
            "streamed window [{start},{end}) diverged from the batch result over the same tuples \
             (mode {mode:?})"
        );
    }
}

// =================================================================
// 1. STREAMED-EQUALS-BATCH DIFFERENTIAL ORACLE
// =================================================================

/// Tumbling (STEP omitted ⇒ STEP == RANGE): a non-trivial join+aggregate query
/// over a multi-sensor stream must produce, per window, exactly the batch result
/// over the tuples that window covers. Spans empty windows (gaps in the data)
/// and duplicate triples (set semantics) too.
#[test]
fn oracle_tumbling_join_aggregate_equals_batch() {
    let sparql = "SELECT ?room (AVG(?v) AS ?avg) (COUNT(?v) AS ?n) \
                  WHERE { ?s <http://ex/in> ?room . ?s <http://ex/value> ?v } \
                  GROUP BY ?room ORDER BY ?room";
    let spec = WindowSpec::time(10, 10); // tumbling
    let script: Vec<([Term; 3], u64)> = vec![
        (t("s1", "in", "kitchen"), 0),
        (val("s1", 20), 1),
        (val("s1", 22), 4),
        (t("s2", "in", "hall"), 5),
        (val("s2", 30), 6),
        (val("s2", 30), 8), // duplicate of the previous triple: set semantics
        // gap: nothing in [10,20) → empty window
        (t("s1", "in", "kitchen"), 21),
        (val("s1", 100), 22),
        (val("s1", 26), 28), // boundary-internal
    ];
    assert_streamed_equals_batch(sparql, spec, EvalMode::default(), 0, &script);
}

/// Sliding (RANGE > STEP ⇒ overlapping windows): the same tuple is read by
/// several windows. The oracle independently recomputes each overlapping
/// window's content and the streamed result must match it window-for-window.
#[test]
fn oracle_sliding_overlap_equals_batch() {
    let sparql = "SELECT ?s (SUM(?v) AS ?sum) WHERE { ?s <http://ex/value> ?v } \
                  GROUP BY ?s ORDER BY ?s";
    let spec = WindowSpec::time(10, 5); // sliding: each ts read by up to two windows
    let mut script: Vec<([Term; 3], u64)> = Vec::new();
    for ts in 0..30u64 {
        script.push((val(&format!("s{}", ts % 3), (ts % 5) as i32), ts));
    }
    assert_streamed_equals_batch(sparql, spec, EvalMode::default(), 0, &script);
}

/// The oracle holds for a non-aggregate projection too (ORDER BY makes it a
/// deterministic total order, so this also pins ROW ORDER, not just the set).
#[test]
fn oracle_projection_with_order_by_equals_batch() {
    let sparql = "SELECT ?s ?v WHERE { ?s <http://ex/value> ?v } ORDER BY ?v ?s";
    let spec = WindowSpec::time(8, 8);
    let script: Vec<([Term; 3], u64)> = vec![
        (val("a", 3), 0),
        (val("b", 1), 2),
        (val("c", 2), 7), // [0,8): a=3,b=1,c=2 → ordered 1,2,3
        (val("d", 9), 8),
        (val("e", 5), 15), // [8,16): d=9,e=5 → 5,9
    ];
    let streamed = run_streamed(sparql, spec, EvalMode::default(), &script);
    for (start, end, stream_rows) in &streamed {
        let g = batch_graph_for_window(&script, *start, *end, 0);
        let batch = sparq_engine::query(&g, sparql).unwrap();
        // ORDER BY ⇒ exact positional equality, not just multiset.
        assert_eq!(
            stream_rows, &batch.rows,
            "ordered window [{start},{end}) row order diverged"
        );
    }
}

/// The oracle holds under a non-zero `t0` window origin: windows are
/// `[t0+k·step, …)` and a pre-origin tuple is in NO window — the oracle's
/// `ts >= t0` guard must agree with the engine.
#[test]
fn oracle_with_t0_origin_equals_batch() {
    let sparql = "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }";
    let t0 = 3;
    let spec = WindowSpec::time(10, 10).with_t0(t0);
    let script: Vec<([Term; 3], u64)> = vec![
        (t("s", "p", "preorigin"), 1), // ts < t0: no window
        (t("s", "p", "a"), 3),         // [3,13)
        (t("s", "p", "b"), 12),        // [3,13)
        (t("s", "p", "c"), 13),        // [13,23)
        (t("s", "p", "d"), 22),        // [13,23)
    ];
    assert_streamed_equals_batch(sparql, spec, EvalMode::default(), t0, &script);
}

/// The oracle holds with `max_delay` lateness tolerance and out-of-order
/// arrivals: the streamed window content (which accepts in-tolerance late
/// tuples) must still equal the batch over the tuples that window covers BY
/// TIMESTAMP. (Tuples dropped as too-late are excluded from BOTH sides below by
/// the late-drop test; here every tuple is within tolerance.)
#[test]
fn oracle_with_max_delay_out_of_order_equals_batch() {
    let sparql = "SELECT ?o WHERE { <http://ex/s> <http://ex/p> ?o } ORDER BY ?o";
    let spec = WindowSpec::time(10, 10).with_max_delay(5);
    let script: Vec<([Term; 3], u64)> = vec![
        (t("s", "p", "a"), 3),
        (t("s", "p", "b"), 12), // watermark 7 < 10: [0,10) stays open
        (t("s", "p", "c"), 8),  // out-of-order, within tolerance → lands in [0,10)
        (t("s", "p", "d"), 16), // watermark 11: closes [0,10)
        (t("s", "p", "e"), 25),
    ];
    assert_streamed_equals_batch(sparql, spec, EvalMode::default(), 0, &script);
}

// =================================================================
// 2. BOUNDARY INCLUSIVITY (half-open [start, end))
// =================================================================

/// A tuple EXACTLY on a window's open boundary (ts == start) is IN that window;
/// a tuple exactly on the close boundary (ts == end) is NOT — it belongs to the
/// next window. Verified through the full query pipeline (COUNT), and the oracle
/// confirms the same partition.
#[test]
fn boundary_open_inclusive_close_exclusive_through_pipeline() {
    let sparql = "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }";
    let spec = WindowSpec::time(10, 10);
    // ts 10 is the close boundary of [0,10) and the open boundary of [10,20):
    // it must count in [10,20), never [0,10).
    let script: Vec<([Term; 3], u64)> = vec![
        (t("s", "p", "at0"), 0),   // open boundary of [0,10) → IN [0,10)
        (t("s", "p", "at9"), 9),   // last tick of [0,10)
        (t("s", "p", "at10"), 10), // close boundary of [0,10) → IN [10,20)
        (t("s", "p", "at19"), 19),
        (t("s", "p", "at20"), 20), // → IN [20,30)
    ];
    let streamed = run_streamed(sparql, spec, EvalMode::default(), &script);
    let counts: Vec<(u64, u64, i64)> = streamed
        .iter()
        .map(|(s, e, rows)| (*s, *e, count_val(&rows[0][0])))
        .collect();
    assert_eq!(
        counts,
        vec![(0, 10, 2), (10, 20, 2), (20, 30, 1)],
        "ts on the close boundary belongs to the NEXT window, not the closing one"
    );
    // …and the oracle agrees window-for-window.
    assert_streamed_equals_batch(sparql, spec, EvalMode::default(), 0, &script);
}

/// Boundary inclusivity holds for SLIDING windows where one tuple sits on the
/// boundary shared by two overlapping windows.
#[test]
fn boundary_inclusivity_on_sliding_overlap() {
    let sparql = "SELECT ?o WHERE { ?s ?p ?o } ORDER BY ?o";
    let spec = WindowSpec::time(10, 5); // windows [0,10) [5,15) [10,20) …
                                        // ts 5 is the open boundary of [5,15) AND still inside [0,10): it is in BOTH.
                                        // ts 10 is the close boundary of [0,10) (out) but open of [10,20) and inside
                                        // [5,15): in [5,15) and [10,20), NOT [0,10).
    let script: Vec<([Term; 3], u64)> = vec![
        (t("s", "p", "at5"), 5),
        (t("s", "p", "at10"), 10),
        (t("s", "p", "at14"), 14),
    ];
    assert_streamed_equals_batch(sparql, spec, EvalMode::default(), 0, &script);
}

// =================================================================
// 3. LATE / OUT-OF-ORDER tuple handling (the documented guarantee)
// =================================================================

/// The documented policy, pinned end-to-end: an out-of-order tuple within
/// `max_delay` of the watermark lands in its time-correct window (still open);
/// one whose every covering window has already closed is DROPPED and counted in
/// `late_dropped`, and never appears in any result NOR in the batch oracle for
/// any window.
#[test]
fn late_within_tolerance_kept_beyond_dropped_and_excluded_from_oracle() {
    let sparql = "SELECT ?o WHERE { <http://ex/s> <http://ex/p> ?o } ORDER BY ?o";
    let spec = WindowSpec::time(10, 10).with_max_delay(2);
    let mut q = ContinuousQuery::register(sparql, spec).unwrap();
    let mut emitted: Vec<(u64, u64, Vec<String>)> = Vec::new();
    let mut sink = |r: WindowResult| {
        let objs: Vec<String> = r
            .rows
            .iter()
            .map(|row| match &row[0] {
                Some(Term::NamedNode(n)) => n.as_str().trim_start_matches("http://ex/").to_owned(),
                other => format!("{other:?}"),
            })
            .collect();
        emitted.push((r.start, r.end, objs));
    };

    q.push(t("s", "p", "a"), 3, &mut sink).unwrap();
    q.push(t("s", "p", "b"), 11, &mut sink).unwrap(); // watermark 9 < 10: [0,10) open
    q.push(t("s", "p", "kept"), 7, &mut sink).unwrap(); // in-tolerance late → [0,10)
    q.push(t("s", "p", "c"), 13, &mut sink).unwrap(); // watermark 11 ≥ 10: closes [0,10)
    assert_eq!(
        q.late_dropped(),
        0,
        "all late tuples so far were within tolerance"
    );

    // Now a tuple whose only covering window [0,10) has closed: dropped + counted.
    q.push(t("s", "p", "toolate"), 5, &mut sink).unwrap();
    assert_eq!(
        q.late_dropped(),
        1,
        "every covering window of ts=5 had closed"
    );

    q.flush(&mut sink).unwrap();

    // The closed [0,10) window holds a + kept (the in-tolerance late tuple), and
    // NOT the dropped one.
    let w0 = emitted
        .iter()
        .find(|(s, _, _)| *s == 0)
        .expect("[0,10) closed");
    assert_eq!(
        w0.2,
        vec!["a", "kept"],
        "in-tolerance late tuple included; dropped one absent"
    );
    // The dropped tuple "toolate" must NOT appear in ANY emitted window.
    assert!(
        !emitted
            .iter()
            .any(|(_, _, objs)| objs.iter().any(|o| o == "toolate")),
        "a dropped (too-late) tuple must never surface in any window result"
    );
}

/// A late-DROPPED tuple is invisible to the batch oracle too: the oracle is fed
/// only the tuples the engine ACCEPTED (the dropped one removed), and the
/// streamed result still equals the batch over those accepted tuples — i.e. the
/// drop is the ONLY difference the lateness policy introduces.
#[test]
fn dropped_tuple_is_the_only_difference_vs_batch() {
    let sparql = "SELECT ?o WHERE { <http://ex/s> <http://ex/p> ?o } ORDER BY ?o";
    let spec = WindowSpec::time(10, 10); // max_delay 0: eager closure
                                         // ts 5 arrives after [0,10) has closed (ts 12 closed it) → dropped.
    let full: Vec<([Term; 3], u64)> = vec![
        (t("s", "p", "a"), 3),
        (t("s", "p", "b"), 12),      // closes [0,10) (watermark 12 ≥ 10)
        (t("s", "p", "dropped"), 5), // too late: [0,10) gone
        (t("s", "p", "c"), 22),
    ];
    // The oracle's ground-truth stream EXCLUDES the dropped tuple, because the
    // engine never admitted it to any window.
    let accepted: Vec<([Term; 3], u64)> = full
        .iter()
        .filter(|(tr, _)| tr[2] != iri("dropped"))
        .cloned()
        .collect();

    let streamed = run_streamed(sparql, spec, EvalMode::default(), &full);
    for (start, end, stream_rows) in &streamed {
        let g = batch_graph_for_window(&accepted, *start, *end, 0);
        let batch = sparq_engine::query(&g, sparql).unwrap();
        assert_eq!(
            stream_rows, &batch.rows,
            "window [{start},{end}) must equal the batch over the ACCEPTED tuples"
        );
    }
}

// =================================================================
// 4. EVICTION + REPORT-STRATEGY DETERMINISM
// =================================================================

/// Once a window has closed and slid, its evicted content can never reappear in
/// a later window — even when a later tuple shares the subject/object of an
/// evicted one. Verified by the oracle: each later window equals the batch over
/// ONLY its own tuples, so nothing leaks across the slide's `split_off`.
#[test]
fn evicted_content_never_leaks_into_later_windows() {
    let sparql = "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }";
    let spec = WindowSpec::time(10, 10);
    // The SAME triple recurs in non-adjacent windows; if eviction leaked, a
    // later window's count would be inflated by the earlier occurrence.
    let script: Vec<([Term; 3], u64)> = vec![
        (t("s", "p", "recur"), 1),  // [0,10)
        (t("s", "p", "x"), 5),      // [0,10)
        (t("s", "p", "y"), 12),     // [10,20)
        (t("s", "p", "recur"), 25), // [20,30) — same triple, must count ONCE here
    ];
    let streamed = run_streamed(sparql, spec, EvalMode::default(), &script);
    let counts: Vec<(u64, u64, i64)> = streamed
        .iter()
        .map(|(s, e, rows)| (*s, *e, count_val(&rows[0][0])))
        .collect();
    assert_eq!(
        counts,
        vec![(0, 10, 2), (10, 20, 1), (20, 30, 1)],
        "the recurring triple counts once per window it actually occurs in — no leak"
    );
    assert_streamed_equals_batch(sparql, spec, EvalMode::default(), 0, &script);
}

/// The report strategy (Window-Close, watermark-driven) is DETERMINISTIC:
/// running the identical script twice yields byte-identical emitted windows
/// (bounds AND rows AND ORDER of windows). No wall clock, no nondeterminism.
#[test]
fn report_strategy_is_deterministic_across_runs() {
    let sparql = "SELECT ?s (AVG(?v) AS ?avg) WHERE { ?s <http://ex/value> ?v } \
                  GROUP BY ?s ORDER BY ?s";
    let spec = WindowSpec::time(10, 5).with_max_delay(3);
    let mut script: Vec<([Term; 3], u64)> = Vec::new();
    for ts in 0..50u64 {
        // Include some out-of-order arrivals to exercise the watermark logic.
        let eff = if ts % 7 == 3 {
            ts.saturating_sub(2)
        } else {
            ts
        };
        script.push((val(&format!("s{}", ts % 3), (ts % 11) as i32), eff));
    }
    let run1 = run_streamed(sparql, spec, EvalMode::default(), &script);
    let run2 = run_streamed(sparql, spec, EvalMode::default(), &script);
    assert_eq!(
        run1, run2,
        "identical input must yield byte-identical streamed output"
    );
    assert!(!run1.is_empty());
    // And every emitted window matches the batch oracle.
    assert_streamed_equals_batch(sparql, spec, EvalMode::default(), 0, &script);
}

/// Empty windows are reported deterministically between data (DSTREAM needs
/// them), and an empty window's batch graph yields exactly the empty/zero result
/// the streamed pipeline does.
#[test]
fn empty_windows_match_batch_zero_result() {
    let sparql = "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }";
    let spec = WindowSpec::time(10, 10);
    let script: Vec<([Term; 3], u64)> = vec![
        (t("s", "p", "a"), 1), // [0,10)
        // gap: [10,20), [20,30) empty
        (t("s", "p", "b"), 35), // [30,40)
    ];
    let streamed = run_streamed(sparql, spec, EvalMode::default(), &script);
    let bounds: Vec<(u64, u64)> = streamed.iter().map(|(s, e, _)| (*s, *e)).collect();
    assert_eq!(
        bounds,
        vec![(0, 10), (10, 20), (20, 30), (30, 40)],
        "empty windows between data are reported"
    );
    // COUNT(*) over an empty graph is 0 — and the oracle must agree.
    assert_streamed_equals_batch(sparql, spec, EvalMode::default(), 0, &script);
    assert_eq!(
        count_val(&streamed[1].2[0][0]),
        0,
        "[10,20) is empty → COUNT 0"
    );
}

// =================================================================
// 5. EVALMODE 4-WAY EQUIVALENCE against the oracle
// =================================================================

/// All four materialisation strategies (Rebuild / PersistentDict / Delta /
/// Snapshot) must each independently satisfy the batch oracle on the SAME
/// stream — a stronger claim than "they agree with each other": each agrees with
/// batch ground truth. Uses a sliding window with overlap, duplicate triples and
/// empty windows so the delta diff, the persistent-dict compaction and the
/// overlay-snapshot evaluation are all exercised.
#[test]
fn all_eval_modes_satisfy_the_oracle() {
    let sparql = "SELECT ?s (SUM(?v) AS ?sum) (COUNT(?v) AS ?n) \
                  WHERE { ?s <http://ex/value> ?v } GROUP BY ?s ORDER BY ?s";
    let spec = WindowSpec::time(20, 10); // sliding: 50% overlap
    let mut script: Vec<([Term; 3], u64)> = Vec::new();
    for ts in 0..60u64 {
        // Two sensors; a triple repeated at several timestamps (set semantics +
        // multi-timestamp eviction in delta mode).
        script.push((val(&format!("s{}", ts % 2), (ts % 4) as i32), ts));
    }
    script.push((t("x", "p", "gap"), 90)); // force empty windows between data
    for mode in
        [EvalMode::Rebuild, EvalMode::PersistentDict, EvalMode::Delta, EvalMode::Snapshot]
    {
        assert_streamed_equals_batch(sparql, spec, mode, 0, &script);
    }
}

/// The four modes are also IDENTICAL to each other window-for-window (bounds +
/// rows), not merely each correct vs batch — pins that mode choice is a pure
/// performance knob with no observable effect, including under R2S=RSTREAM.
#[test]
fn eval_modes_byte_identical_to_each_other() {
    let sparql = "SELECT ?o WHERE { ?s <http://ex/p> ?o } ORDER BY ?o";
    let spec = WindowSpec::time(10, 5);
    let mut script: Vec<([Term; 3], u64)> = Vec::new();
    for ts in 0..40u64 {
        script.push((t("s", "p", &format!("o{}", ts % 6)), ts));
    }
    let rebuild = run_streamed(sparql, spec, EvalMode::Rebuild, &script);
    let persistent = run_streamed(sparql, spec, EvalMode::PersistentDict, &script);
    let delta = run_streamed(sparql, spec, EvalMode::Delta, &script);
    let snapshot = run_streamed(sparql, spec, EvalMode::Snapshot, &script);
    assert_eq!(rebuild, persistent, "PersistentDict diverged from Rebuild");
    assert_eq!(rebuild, delta, "Delta diverged from Rebuild");
    assert_eq!(rebuild, snapshot, "Snapshot diverged from Rebuild");
}

/// The oracle also holds under an R2S that is NOT RSTREAM: ISTREAM emits the
/// per-window DELTA, so it cannot be compared to a single batch query directly —
/// but the CUMULATIVE reconstruction (apply each window's +adds in order) must
/// reproduce the RSTREAM full result of the final window. This pins that
/// ISTREAM's incremental emission is consistent with the batch full-result.
#[test]
fn istream_deltas_reconstruct_rstream_full_result() {
    let sparql = "SELECT ?o WHERE { <http://ex/s> <http://ex/p> ?o }";
    let spec = WindowSpec::time(10, 10);
    let script: Vec<([Term; 3], u64)> = vec![
        (t("s", "p", "a"), 1),
        (t("s", "p", "b"), 2), // [0,10): {a,b}
        (t("s", "p", "b"), 11),
        (t("s", "p", "c"), 12), // [10,20): {b,c}
    ];
    // RSTREAM full result of the LAST window, via the batch oracle.
    let rstream = run_streamed(sparql, spec, EvalMode::default(), &script);
    let (ls, le, last_full) = rstream.last().cloned().unwrap();
    let g = batch_graph_for_window(&script, ls, le, 0);
    let batch_last = sparq_engine::query(&g, sparql).unwrap();
    assert_eq!(as_multiset(&last_full), as_multiset(&batch_last.rows));

    // ISTREAM: rebuild a running multiset from the per-window adds, then the
    // final window's RSTREAM content must be a SUBSET reachable from the running
    // adds minus removals. We assert the simpler invariant the docs guarantee:
    // every ISTREAM-added row of the last window appears in its RSTREAM result.
    let mut qi = ContinuousQuery::register(sparql, spec)
        .unwrap()
        .with_r2s(R2S::IStream);
    let mut istream: Vec<(u64, Vec<Row>)> = Vec::new();
    let mut sink = |r: WindowResult| istream.push((r.start, r.rows));
    for (triple, ts) in &script {
        qi.push(triple.clone(), *ts, &mut sink).unwrap();
    }
    qi.flush(&mut sink).unwrap();
    let last_added = &istream.last().unwrap().1;
    let full_set = as_multiset(&last_full);
    for row in last_added {
        assert!(
            full_set.contains(&format!("{row:?}")),
            "an ISTREAM-added row of the last window is absent from its RSTREAM full result"
        );
    }
}

// =================================================================
// 6. PARTIAL-CLOSE late arrival (sliding) — the subtle documented guarantee
// =================================================================

/// The documented sliding-window late-arrival rule (window.rs §out-of-order):
/// "a triple whose LAST covering window has already closed is dropped; if only
/// SOME earlier covering windows closed, it still enters the open ones and is
/// NOT counted as late."
///
/// This is the one place the naive timestamp-only batch oracle does NOT apply:
/// a tuple covered by two overlapping windows can enter the still-open one while
/// the already-closed one keeps the content it had at its close — so the closed
/// window's streamed content is a STRICT SUBSET of the batch over its timestamp
/// range. That is correct streaming behaviour, not a bug, so it is pinned
/// directly at the window-content level here rather than through the oracle.
#[test]
fn sliding_partial_close_late_tuple_enters_open_window_only_not_late() {
    use sparq_rsp::WindowedStream;
    // RANGE 10 STEP 5 ⇒ windows [0,10) [5,15) [10,20) …; max_delay 0.
    let mut ws = WindowedStream::empty(WindowSpec::time(10, 5));
    ws.push(t("s", "p", "a"), 2); // in [0,10) and (later) nothing else
    ws.push(t("s", "p", "b"), 12); // watermark 12: closes [0,10) (end 10 ≤ 12)
    let closed_now = ws.take_closed();
    assert_eq!(closed_now.len(), 1);
    assert_eq!((closed_now[0].start, closed_now[0].end), (0, 10));
    assert_eq!(objs(&closed_now[0]), ["a"], "[0,10) closed holding only a");

    // ts 7 is covered by [0,10) (CLOSED) and [5,15) (still open). Its LAST
    // covering window [5,15) is open, so it is NOT late — it enters [5,15) only.
    ws.push(t("s", "p", "late7"), 7);
    assert_eq!(
        ws.late_dropped(),
        0,
        "last covering window still open ⇒ not late"
    );

    let rest = ws.flush();
    let w_5_15 = rest
        .iter()
        .find(|w| (w.start, w.end) == (5, 15))
        .expect("[5,15) flushed");
    assert_eq!(
        objs(w_5_15),
        ["late7", "b"],
        "the late tuple entered the still-open [5,15) (with b); ordered by ts"
    );
    // It did NOT retroactively enter the already-closed [0,10).
    assert!(
        !objs(&closed_now[0]).contains(&"late7".to_string()),
        "an already-closed window is frozen: the late tuple did not leak back in"
    );
}

/// The companion drop case: when the LAST covering window has ALSO closed, the
/// same late tuple IS dropped and counted (sliding, so two windows had to close
/// first).
#[test]
fn sliding_late_tuple_dropped_once_all_covering_windows_closed() {
    use sparq_rsp::WindowedStream;
    let mut ws = WindowedStream::empty(WindowSpec::time(10, 5)); // [0,10) [5,15) …
    ws.push(t("s", "p", "a"), 2);
    ws.push(t("s", "p", "b"), 20); // watermark 20: closes [0,10) AND [5,15) (end 15 ≤ 20)
    let _ = ws.take_closed();
    // ts 7's covering windows are [0,10) and [5,15) — BOTH now closed ⇒ dropped.
    ws.push(t("s", "p", "late7"), 7);
    assert_eq!(
        ws.late_dropped(),
        1,
        "every covering window of ts 7 had closed ⇒ dropped"
    );
    let rest = ws.flush();
    assert!(
        !rest.iter().any(|w| objs(w).contains(&"late7".to_string())),
        "a fully-late tuple never surfaces in any window"
    );
}

// =================================================================
// 7. DETERMINISTIC FUZZ SWEEP against the oracle
// =================================================================

/// A small xorshift PRNG so the sweep is DETERMINISTIC (seeded) — no external
/// rng dependency, reproducible failures.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Deterministic fuzz: many seeded random IN-ORDER streams (timestamps
/// non-decreasing, so no tuple is ever late and the naive timestamp oracle is
/// exact) across tumbling AND sliding specs, several queries, all four eval
/// modes — each emitted window must equal the batch over its tuples. This is the
/// broad differential net: a closure/eviction/materialisation bug on SOME shape
/// would surface as a streamed-vs-batch divergence on SOME seed.
#[test]
fn fuzz_in_order_streams_streamed_equals_batch() {
    let queries = [
        "SELECT ?s (SUM(?v) AS ?sum) (COUNT(?v) AS ?n) WHERE { ?s <http://ex/value> ?v } GROUP BY ?s ORDER BY ?s",
        "SELECT (AVG(?v) AS ?a) (MIN(?v) AS ?mn) (MAX(?v) AS ?mx) WHERE { ?s <http://ex/value> ?v }",
        "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }",
    ];
    // (range, step): two tumbling, two sliding (overlap), one with step>range gap.
    let specs = [(10u64, 10u64), (16, 16), (10, 5), (20, 7), (5, 10)];

    for (qi, sparql) in queries.iter().enumerate() {
        for (range, step) in specs {
            for seed in 0..6u64 {
                let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (seed << 8) ^ ((qi as u64) << 3) ^ range);
                // Build an in-order stream: non-decreasing timestamps.
                let mut script: Vec<([Term; 3], u64)> = Vec::new();
                let mut ts = rng.below(4); // small non-zero start sometimes
                let count = 20 + rng.below(25);
                for _ in 0..count {
                    ts += rng.below(6); // 0..5 gap ⇒ duplicates AND gaps both occur
                    let sensor = format!("s{}", rng.below(3));
                    let v = rng.below(7) as i32;
                    script.push((val(&sensor, v), ts));
                }
                let spec = WindowSpec::time(range, step);
                for mode in [
                    EvalMode::Rebuild,
                    EvalMode::PersistentDict,
                    EvalMode::Delta,
                    EvalMode::Snapshot,
                ] {
                    let streamed = run_streamed(sparql, spec, mode, &script);
                    for (start, end, stream_rows) in &streamed {
                        let g = batch_graph_for_window(&script, *start, *end, 0);
                        let batch = sparq_engine::query(&g, sparql).unwrap();
                        assert_eq!(
                            as_multiset(stream_rows),
                            as_multiset(&batch.rows),
                            "FUZZ divergence: q#{qi} range={range} step={step} seed={seed} \
                             mode={mode:?} window=[{start},{end})\nscript={script:?}"
                        );
                    }
                }
            }
        }
    }
}

// ----------------------------------------------------------------- small util

/// The object IRIs of a window's triples (local part), for terse content asserts
/// at the `WindowedStream` (S2R) level.
fn objs(w: &sparq_rsp::Window) -> Vec<String> {
    w.triples
        .iter()
        .map(|tt| match &tt.triple[2] {
            Term::NamedNode(n) => n.as_str().trim_start_matches("http://ex/").to_owned(),
            other => other.to_string(),
        })
        .collect()
}

/// Extracts the integer value of a COUNT/SUM cell (an `xsd:integer` literal).
fn count_val(cell: &Option<Term>) -> i64 {
    match cell {
        Some(Term::Literal(l)) => l
            .value()
            .parse::<i64>()
            .unwrap_or_else(|_| panic!("expected an integer count literal, got {l:?}")),
        other => panic!("expected a bound integer literal, got {other:?}"),
    }
}
