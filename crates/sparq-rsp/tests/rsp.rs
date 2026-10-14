//! Pins the documented stream-processing semantics: window boundary
//! inclusivity, overlap, count windows, lateness, R2S diffs, and the
//! end-to-end ContinuousQuery pipeline.

use oxrdf::{Literal, NamedNode, Term};
use sparq_rsp::{ContinuousQuery, R2S, TripleStream, WindowSpec, WindowedStream};

fn iri(s: &str) -> Term {
    NamedNode::new_unchecked(format!("http://ex/{s}")).into()
}

/// `<http://ex/{s}> <http://ex/{p}> <http://ex/{o}>`
fn t(s: &str, p: &str, o: &str) -> [Term; 3] {
    [iri(s), iri(p), iri(o)]
}

/// `<http://ex/{s}> <http://ex/value> "v"^^xsd:integer`
fn val(s: &str, v: i32) -> [Term; 3] {
    [iri(s), iri("value"), Literal::from(v).into()]
}

/// The object IRIs of a window's triples, e.g. `["a", "b"]`, for terse asserts.
fn objs(w: &sparq_rsp::Window) -> Vec<String> {
    w.triples
        .iter()
        .map(|tt| match &tt.triple[2] {
            Term::NamedNode(n) => n.as_str().trim_start_matches("http://ex/").to_owned(),
            other => other.to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------- boundaries

/// THE boundary-inclusivity contract: windows are half-open `[k·step,
/// k·step + range)` — start INCLUSIVE, end EXCLUSIVE. With RANGE 10 STEP 10,
/// ts 0 and 9 land in `[0,10)`; ts 10 lands in `[10,20)`, never both.
#[test]
fn time_window_boundaries_are_start_inclusive_end_exclusive() {
    let mut ws = WindowedStream::empty(WindowSpec::time(10, 10));
    ws.push(t("s", "p", "at0"), 0);
    ws.push(t("s", "p", "at9"), 9);
    ws.push(t("s", "p", "at10"), 10); // closes [0,10), belongs to [10,20)
    let closed = ws.take_closed();
    assert_eq!(closed.len(), 1);
    assert_eq!((closed[0].start, closed[0].end), (0, 10));
    assert_eq!(objs(&closed[0]), ["at0", "at9"]);

    let rest = ws.flush();
    assert_eq!(rest.len(), 1);
    assert_eq!((rest[0].start, rest[0].end), (10, 20));
    assert_eq!(objs(&rest[0]), ["at10"]);
}

/// Tumbling windows partition the axis: every triple in exactly one window.
#[test]
fn tumbling_windows_partition_without_double_counting() {
    let mut ws = WindowedStream::empty(WindowSpec::time(10, 10));
    for ts in 0..35 {
        ws.push(t("s", "p", &format!("o{ts}")), ts);
    }
    let windows = ws.flush();
    assert_eq!(windows.len(), 4); // [0,10) [10,20) [20,30) [30,40)
    let total: usize = windows.iter().map(|w| w.triples.len()).sum();
    assert_eq!(total, 35);
    assert_eq!(windows[2].triples.iter().map(|x| x.ts).collect::<Vec<_>>(), (20..30).collect::<Vec<_>>());
}

/// Sliding windows overlap: RANGE 10 STEP 5 puts ts 7 in BOTH [0,10) and [5,15).
#[test]
fn sliding_windows_overlap() {
    let mut ws = WindowedStream::empty(WindowSpec::time(10, 5));
    ws.push(t("s", "p", "a"), 2); // [0,10) only (and [-5,5) does not exist: k ≥ 0)
    ws.push(t("s", "p", "b"), 7); // [0,10) and [5,15)
    ws.push(t("s", "p", "c"), 12); // [5,15) and [10,20)
    let windows = ws.flush();
    assert_eq!(windows.len(), 3);
    assert_eq!((windows[0].start, windows[0].end), (0, 10));
    assert_eq!(objs(&windows[0]), ["a", "b"]);
    assert_eq!((windows[1].start, windows[1].end), (5, 15));
    assert_eq!(objs(&windows[1]), ["b", "c"]);
    assert_eq!((windows[2].start, windows[2].end), (10, 20));
    assert_eq!(objs(&windows[2]), ["c"]);
}

/// A stream starting far from 0 does not replay the empty prefix of the axis,
/// but gaps BETWEEN data are reported as empty windows (DSTREAM needs them).
#[test]
fn empty_windows_reported_between_data_but_not_before_first_triple() {
    let mut ws = WindowedStream::empty(WindowSpec::time(10, 10));
    ws.push(t("s", "p", "a"), 1_000); // first window is [1000,1010), not [0,10)
    ws.push(t("s", "p", "b"), 1_035);
    let windows = ws.flush();
    let bounds: Vec<(u64, u64)> = windows.iter().map(|w| (w.start, w.end)).collect();
    assert_eq!(bounds, [(1000, 1010), (1010, 1020), (1020, 1030), (1030, 1040)]);
    assert!(windows[1].triples.is_empty() && windows[2].triples.is_empty());
}

/// step > range leaves uncovered gaps; a gap timestamp is in no window
/// (ignored, NOT counted late).
#[test]
fn step_greater_than_range_gap_triples_belong_to_no_window() {
    let mut ws = WindowedStream::empty(WindowSpec::time(5, 10)); // covers [0,5) [10,15) …
    ws.push(t("s", "p", "in"), 3);
    ws.push(t("s", "p", "gap"), 7); // covered by no window
    ws.push(t("s", "p", "in2"), 12);
    let windows = ws.flush();
    assert_eq!(windows.len(), 2);
    assert_eq!(objs(&windows[0]), ["in"]);
    assert_eq!(objs(&windows[1]), ["in2"]);
    assert_eq!(ws.late_dropped(), 0, "gap triples are not late");
}

/// A gap arrival still ADVANCES event time: it closes earlier windows even
/// though it enters none itself (roborev 1687 regression).
#[test]
fn gap_triples_advance_the_watermark_and_close_earlier_windows() {
    let mut ws = WindowedStream::empty(WindowSpec::time(5, 10)); // covers [0,5) [10,15) …
    ws.push(t("s", "p", "in"), 3);
    assert!(ws.take_closed().is_empty(), "[0,5) still open at watermark 3");
    ws.push(t("s", "p", "gap"), 7); // no window, but watermark → 7 ≥ 5
    let closed = ws.take_closed();
    assert_eq!(closed.len(), 1, "gap arrival closed [0,5)");
    assert_eq!((closed[0].start, closed[0].end), (0, 5));
    assert_eq!(objs(&closed[0]), ["in"]);
}

/// A GAP arrival as the FIRST arrival still anchors the starting window on its
/// watermark: a window max_delay holds open at that point must be tracked and
/// eventually emitted EMPTY — not silently skipped (roborev 1693 regression).
#[test]
fn gap_first_arrival_anchors_and_empty_window_is_still_emitted() {
    // Covers [0,5) [10,15) …; first arrival ts 7 is in the gap, watermark 3:
    // [0,5) must stay open (3 < 5) even though nothing was ever buffered.
    let mut ws = WindowedStream::empty(WindowSpec::time(5, 10).with_max_delay(4));
    ws.push(t("s", "p", "gap"), 7);
    assert!(ws.take_closed().is_empty(), "[0,5) still open at watermark 3");
    ws.push(t("s", "p", "in"), 13); // watermark 9 ≥ 5: closes [0,5), empty
    let closed = ws.take_closed();
    assert_eq!(closed.len(), 1);
    assert_eq!((closed[0].start, closed[0].end), (0, 5));
    assert!(closed[0].triples.is_empty(), "emitted empty, not skipped");
    let rest = ws.flush();
    assert_eq!((rest[0].start, rest[0].end), (10, 15));
    assert_eq!(objs(&rest[0]), ["in"]);
    assert_eq!(ws.late_dropped(), 0);
}

// ------------------------------------------------------------- out-of-order

/// max_delay holds windows open: an out-of-order triple within the tolerance
/// still lands in its window; one beyond it is dropped and counted.
#[test]
fn out_of_order_within_max_delay_is_kept_beyond_is_dropped_and_counted() {
    let mut ws = WindowedStream::empty(WindowSpec::time(10, 10).with_max_delay(5));
    ws.push(t("s", "p", "a"), 3);
    ws.push(t("s", "p", "b"), 12); // watermark 12−5=7 < 10: [0,10) still open
    assert!(ws.take_closed().is_empty());
    ws.push(t("s", "p", "late-ok"), 8); // out-of-order, lands in open [0,10)
    ws.push(t("s", "p", "c"), 15); // watermark 10: closes [0,10)
    let closed = ws.take_closed();
    assert_eq!(closed.len(), 1);
    assert_eq!(objs(&closed[0]), ["a", "late-ok"]);
    assert_eq!(ws.late_dropped(), 0);

    ws.push(t("s", "p", "too-late"), 9); // [0,10) is gone
    assert_eq!(ws.late_dropped(), 1);
    let windows = ws.flush();
    assert_eq!(objs(&windows[0]), ["b", "c"], "dropped triple never surfaces");
}

/// The lateness contract holds across the FIRST push too: the starting window
/// is anchored on the initial watermark (first ts − max_delay), not on the
/// first ts itself, so an out-of-order arrival older than the first triple but
/// within the tolerance still lands in its window.
#[test]
fn out_of_order_before_first_triple_within_max_delay_is_kept() {
    let mut ws = WindowedStream::empty(WindowSpec::time(10, 10).with_max_delay(5));
    ws.push(t("s", "p", "first"), 12); // watermark 7: [0,10) must stay open
    ws.push(t("s", "p", "older"), 8); // before the first triple, within delay
    assert_eq!(ws.late_dropped(), 0, "ts 8 is within max_delay of ts 12");
    let windows = ws.flush();
    assert_eq!((windows[0].start, windows[0].end), (0, 10));
    assert_eq!(objs(&windows[0]), ["older"]);
    assert_eq!(objs(&windows[1]), ["first"]);
}

/// With max_delay 0 the watermark is max_ts itself: the first newer-window
/// triple closes everything before it.
#[test]
fn zero_max_delay_closes_eagerly() {
    let mut ws = WindowedStream::empty(WindowSpec::time(10, 10));
    ws.push(t("s", "p", "a"), 3);
    ws.push(t("s", "p", "b"), 10);
    assert_eq!(ws.take_closed().len(), 1);
    ws.push(t("s", "p", "late"), 9);
    assert_eq!(ws.late_dropped(), 1);
}

// ------------------------------------------------------------ count windows

/// ROWS n: the last n arrivals in ARRIVAL order, reported on every arrival;
/// bounds are the inclusive [first.ts, last.ts] of the content.
#[test]
fn rows_window_keeps_last_n_arrivals_and_reports_each_arrival() {
    let mut ws = WindowedStream::empty(WindowSpec::count(3));
    for (i, ts) in [(1, 10), (2, 20), (3, 30), (4, 40), (5, 50)] {
        ws.push(t("s", "p", &format!("o{i}")), ts);
    }
    let reports = ws.take_closed();
    assert_eq!(reports.len(), 5, "reported on every arrival");
    assert_eq!(objs(&reports[0]), ["o1"]);
    assert_eq!(objs(&reports[2]), ["o1", "o2", "o3"]);
    assert_eq!(objs(&reports[4]), ["o3", "o4", "o5"], "oldest evicted");
    assert_eq!((reports[4].start, reports[4].end), (30, 50), "inclusive content bounds");
    assert!(ws.flush().is_empty(), "flush is a no-op for count windows");
}

/// ROWS n SLIDE s reports every s-th arrival only.
#[test]
fn rows_window_with_slide_reports_every_slide_arrivals() {
    let mut ws = WindowedStream::empty(WindowSpec::count(2).with_slide(2));
    for i in 1..=5 {
        ws.push(t("s", "p", &format!("o{i}")), i as u64);
    }
    let reports = ws.take_closed();
    assert_eq!(reports.len(), 2); // after arrivals 2 and 4
    assert_eq!(objs(&reports[0]), ["o1", "o2"]);
    assert_eq!(objs(&reports[1]), ["o3", "o4"]);
}

/// Out-of-order timestamps do NOT affect count-window membership (arrival
/// order rules), they only show up in the reported bounds.
#[test]
fn rows_window_membership_is_arrival_order_not_timestamp_order() {
    let mut ws = WindowedStream::empty(WindowSpec::count(2));
    ws.push(t("s", "p", "a"), 100);
    ws.push(t("s", "p", "b"), 50); // older timestamp, newer arrival
    let reports = ws.take_closed();
    assert_eq!(objs(&reports[1]), ["a", "b"]);
    assert_eq!((reports[1].start, reports[1].end), (100, 50), "bounds are content ts, not sorted");
}

/// A scripted TripleStream replays its history through WindowedStream::new.
#[test]
fn scripted_stream_replays_history() {
    let mut s = TripleStream::new();
    s.push(t("s", "p", "a"), 1).push(t("s", "p", "b"), 11);
    let mut ws = WindowedStream::new(s, WindowSpec::time(10, 10));
    let closed = ws.take_closed();
    assert_eq!(closed.len(), 1, "ts 11 already closed [0,10)");
    assert_eq!(objs(&closed[0]), ["a"]);
}

// -------------------------------------------------------- continuous queries

/// End-to-end: AVG of sensor readings per tumbling window, RSTREAM.
#[test]
fn continuous_avg_per_window() {
    let mut q = ContinuousQuery::register(
        "SELECT (AVG(?v) AS ?avg) WHERE { ?s <http://ex/value> ?v }",
        WindowSpec::time(10, 10),
    )
    .unwrap();
    let mut avgs: Vec<(u64, u64, Option<Term>)> = Vec::new();
    let mut sink = |r: sparq_rsp::WindowResult| avgs.push((r.start, r.end, r.rows[0][0].clone()));

    // [0,10): 2, 4, 6 → 4.0 ; [10,20): 10 → 10.0 ; [20,30): 7, 9 → 8.0
    q.push(val("s1", 2), 1, &mut sink).unwrap();
    q.push(val("s2", 4), 5, &mut sink).unwrap();
    q.push(val("s1", 6), 9, &mut sink).unwrap();
    q.push(val("s1", 10), 12, &mut sink).unwrap(); // closes [0,10)
    q.push(val("s2", 7), 20, &mut sink).unwrap(); // closes [10,20)
    q.push(val("s1", 9), 25, &mut sink).unwrap();
    q.flush(&mut sink).unwrap(); // closes [20,30)

    // The engine returns integer averages as xsd:decimal (per SPARQL AVG typing).
    let avg = |x: &str| {
        Some(Term::from(Literal::new_typed_literal(x, oxrdf::vocab::xsd::DECIMAL)))
    };
    assert_eq!(avgs, vec![(0, 10, avg("4.0")), (10, 20, avg("10.0")), (20, 30, avg("8.0"))]);
}

/// RSTREAM emits the FULL result of every window, joins included.
#[test]
fn continuous_rstream_full_result_with_join() {
    let mut q = ContinuousQuery::register(
        "SELECT ?room ?v WHERE { ?s <http://ex/in> ?room . ?s <http://ex/value> ?v } ORDER BY ?v",
        WindowSpec::time(10, 10),
    )
    .unwrap();
    let mut results = Vec::new();
    let mut sink = |r: sparq_rsp::WindowResult| results.push(r);
    q.push(t("s1", "in", "kitchen"), 1, &mut sink).unwrap();
    q.push(val("s1", 21), 2, &mut sink).unwrap();
    q.push(val("s1", 23), 7, &mut sink).unwrap();
    q.flush(&mut sink).unwrap();

    assert_eq!(results.len(), 1);
    let r = &results[0];
    assert_eq!(r.vars.iter().map(|v| v.as_str()).collect::<Vec<_>>(), ["room", "v"]);
    assert_eq!(r.rows.len(), 2, "s1 joins with both of its readings");
    assert_eq!(r.rows[0], vec![Some(iri("kitchen")), Some(Literal::from(21).into())]);
    assert_eq!(r.rows[1], vec![Some(iri("kitchen")), Some(Literal::from(23).into())]);
}

/// Scripted ISTREAM: only rows ADDED relative to the previous window appear.
/// Window 1 {a,b} → emits a,b. Window 2 {b,c} → emits c. Window 3 {} → emits
/// nothing. Window 4 {a} → a is back: emitted again.
#[test]
fn istream_emits_only_added_rows() {
    let mut q = ContinuousQuery::register(
        "SELECT ?o WHERE { <http://ex/s> <http://ex/p> ?o } ORDER BY ?o",
        WindowSpec::time(10, 10),
    )
    .unwrap()
    .with_r2s(R2S::IStream);
    let mut emitted: Vec<(u64, Vec<Option<Term>>)> = Vec::new();
    let mut sink = |r: sparq_rsp::WindowResult| {
        for row in r.rows {
            emitted.push((r.start, row));
        }
    };

    q.push(t("s", "p", "a"), 1, &mut sink).unwrap(); // window [0,10): a, b
    q.push(t("s", "p", "b"), 2, &mut sink).unwrap();
    q.push(t("s", "p", "b"), 11, &mut sink).unwrap(); // [10,20): b, c
    q.push(t("s", "p", "c"), 12, &mut sink).unwrap();
    // [20,30) empty; [30,40): a — push ts 30 then flush closes both.
    q.push(t("s", "p", "a"), 30, &mut sink).unwrap();
    q.flush(&mut sink).unwrap();

    let expect = |w: u64, o: &str| (w, vec![Some(iri(o))]);
    assert_eq!(
        emitted,
        vec![expect(0, "a"), expect(0, "b"), expect(10, "c"), expect(30, "a")],
        "b not re-emitted in [10,20); empty [20,30) adds nothing; a re-added in [30,40)"
    );
}

/// Scripted DSTREAM: only rows REMOVED relative to the previous window appear.
/// Window 1 {a,b} → nothing (nothing before it). Window 2 {b,c} → a. Window 3
/// {} (empty) → b, c: the empty window is what makes results disappear.
#[test]
fn dstream_emits_only_removed_rows_including_via_empty_window() {
    let mut q = ContinuousQuery::register(
        "SELECT ?o WHERE { <http://ex/s> <http://ex/p> ?o } ORDER BY ?o",
        WindowSpec::time(10, 10),
    )
    .unwrap()
    .with_r2s(R2S::DStream);
    let mut emitted: Vec<(u64, Vec<Option<Term>>)> = Vec::new();
    let mut sink = |r: sparq_rsp::WindowResult| {
        for row in r.rows {
            emitted.push((r.start, row));
        }
    };

    q.push(t("s", "p", "a"), 1, &mut sink).unwrap(); // [0,10): a, b
    q.push(t("s", "p", "b"), 2, &mut sink).unwrap();
    q.push(t("s", "p", "b"), 11, &mut sink).unwrap(); // [10,20): b, c
    q.push(t("s", "p", "c"), 12, &mut sink).unwrap();
    q.push(t("s", "p", "z"), 35, &mut sink).unwrap(); // closes [10,20) AND empty [20,30)
    q.flush(&mut sink).unwrap();

    let expect = |w: u64, o: &str| (w, vec![Some(iri(o))]);
    assert_eq!(
        emitted,
        vec![
            expect(10, "a"), // [10,20) closed: a gone
            expect(20, "b"), // empty [20,30): b and c gone
            expect(20, "c"),
            // z (window [30,40)) is NOT DSTREAMed: no window closes after it,
            // so its disappearance is never observed — correct DSTREAM behaviour.
        ]
    );
}

/// ISTREAM/DSTREAM are MULTISET diffs: a duplicate row appearing once more /
/// once less is emitted exactly once.
#[test]
fn istream_dstream_diffs_are_multiset() {
    // ?v projected per reading: two sensors reading 7 → the row (7) twice.
    let sparql = "SELECT ?v WHERE { ?s <http://ex/value> ?v }";
    let mut qi = ContinuousQuery::register(sparql, WindowSpec::time(10, 10)).unwrap().with_r2s(R2S::IStream);
    let mut qd = ContinuousQuery::register(sparql, WindowSpec::time(10, 10)).unwrap().with_r2s(R2S::DStream);
    let mut got_i: Vec<usize> = Vec::new(); // emitted row count per window
    let mut got_d: Vec<usize> = Vec::new();

    for (q, got) in [(&mut qi, &mut got_i), (&mut qd, &mut got_d)] {
        let mut sink = |r: sparq_rsp::WindowResult| got.push(r.rows.len());
        // [0,10): {7} ; [10,20): {7, 7} ; [20,30): {7}
        q.push(val("s1", 7), 1, &mut sink).unwrap();
        q.push(val("s1", 7), 11, &mut sink).unwrap();
        q.push(val("s2", 7), 12, &mut sink).unwrap();
        q.push(val("s1", 7), 21, &mut sink).unwrap();
        q.flush(&mut sink).unwrap();
    }
    assert_eq!(got_i, [1, 1, 0], "ISTREAM: +1 of the duplicate in window 2");
    assert_eq!(got_d, [0, 0, 1], "DSTREAM: −1 of the duplicate in window 3");
}

/// ContinuousQuery + ROWS window: query result follows the last-n content.
#[test]
fn continuous_query_over_count_window() {
    let mut q = ContinuousQuery::register(
        "SELECT (SUM(?v) AS ?sum) WHERE { ?s <http://ex/value> ?v }",
        WindowSpec::count(2),
    )
    .unwrap();
    let mut sums = Vec::new();
    let mut sink = |r: sparq_rsp::WindowResult| sums.push(r.rows[0][0].clone());
    q.push(val("s1", 1), 0, &mut sink).unwrap(); // {1}
    q.push(val("s2", 2), 1, &mut sink).unwrap(); // {1,2}
    q.push(val("s3", 4), 2, &mut sink).unwrap(); // {2,4} — 1 evicted
    let sum = |x: i32| Some(Term::from(Literal::from(x)));
    assert_eq!(sums, vec![sum(1), sum(3), sum(6)]);
}

/// sq-f0v71: count-window R2S diffs compare consecutive
/// *reported* windows when the slide skips arrivals. The transient window
/// after `v3` would be `{v2, v3}`; incorrectly using it as the diff base would
/// emit only `v4` for ISTREAM and `v2` for DSTREAM, which these assertions
/// reject.
#[test]
fn continuous_query_count_window_slide_diffs_consecutive_reports() {
    let run = |r2s| {
        let mut q = ContinuousQuery::register(
            "SELECT ?v WHERE { ?s <http://ex/value> ?v } ORDER BY ?v",
            WindowSpec::count(2).with_slide(2),
        )
        .unwrap()
        .with_r2s(r2s);
        let mut reports = Vec::new();
        let mut sink = |r: sparq_rsp::WindowResult| reports.push(r.rows);

        for i in 1..=4 {
            q.push(t(&format!("s{i}"), "value", &format!("v{i}")), i, &mut sink)
                .unwrap();
        }
        reports
    };
    let rows = |values: &[&str]| {
        values
            .iter()
            .map(|value| vec![Some(iri(value))])
            .collect::<Vec<_>>()
    };

    assert_eq!(
        run(R2S::IStream),
        vec![rows(&["v1", "v2"]), rows(&["v3", "v4"])]
    );
    assert_eq!(run(R2S::DStream), vec![rows(&[]), rows(&["v1", "v2"])]);
}

/// Lateness propagates through ContinuousQuery: late triples never reach a
/// result; the counter is observable.
#[test]
fn continuous_query_counts_late_drops() {
    let mut q = ContinuousQuery::register(
        "SELECT ?o WHERE { ?s ?p ?o }",
        WindowSpec::time(10, 10),
    )
    .unwrap();
    let mut n_rows = 0usize;
    let mut sink = |r: sparq_rsp::WindowResult| n_rows += r.rows.len();
    q.push(t("s", "p", "a"), 5, &mut sink).unwrap();
    q.push(t("s", "p", "b"), 15, &mut sink).unwrap(); // closes [0,10)
    q.push(t("s", "p", "late"), 3, &mut sink).unwrap(); // dropped
    q.flush(&mut sink).unwrap();
    assert_eq!(q.late_dropped(), 1);
    assert_eq!(n_rows, 2, "only a and b ever appear");
}

/// Registration validates the query up front.
#[test]
fn register_rejects_non_select_and_malformed_queries() {
    let spec = WindowSpec::time(10, 10);
    assert!(ContinuousQuery::register("ASK { ?s ?p ?o }", spec).is_err());
    assert!(ContinuousQuery::register("CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }", spec).is_err());
    assert!(ContinuousQuery::register("SELECT WHERE garbage", spec).is_err());
    assert!(ContinuousQuery::register("SELECT * WHERE { ?s ?p ?o }", spec).is_ok());
}

/// Windows are RDF graphs, i.e. SETS: the same triple at two timestamps in one
/// window matches once.
#[test]
fn window_materialisation_has_set_semantics() {
    let mut q = ContinuousQuery::register(
        "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }",
        WindowSpec::time(10, 10),
    )
    .unwrap();
    let mut counts = Vec::new();
    let mut sink = |r: sparq_rsp::WindowResult| counts.push(r.rows[0][0].clone());
    q.push(t("s", "p", "a"), 1, &mut sink).unwrap();
    q.push(t("s", "p", "a"), 5, &mut sink).unwrap(); // same triple, later ts
    q.flush(&mut sink).unwrap();
    assert_eq!(counts, vec![Some(Term::from(Literal::from(1)))]);
}

// ---------------------------------------------------------------- window origin t0

/// The RSP-QL window origin: with t0 = 3, RANGE 10 STEP 10 windows are
/// [3,13) [13,23) … — and a pre-origin timestamp belongs to no window (it is
/// NOT late: no window ever covered it) while still advancing the watermark.
#[test]
fn t0_shifts_window_origin_and_pre_origin_arrivals_join_no_window() {
    let mut ws = WindowedStream::empty(WindowSpec::time(10, 10).with_t0(3));
    ws.push(t("s", "p", "pre"), 1); // before t0: no window, not late
    ws.push(t("s", "p", "a"), 3);
    ws.push(t("s", "p", "b"), 12);
    ws.push(t("s", "p", "c"), 13); // closes [3,13), belongs to [13,23)
    let closed = ws.take_closed();
    assert_eq!(closed.len(), 1);
    assert_eq!((closed[0].start, closed[0].end), (3, 13));
    assert_eq!(objs(&closed[0]), ["a", "b"]);
    let rest = ws.flush();
    assert_eq!((rest[0].start, rest[0].end), (13, 23));
    assert_eq!(objs(&rest[0]), ["c"]);
    assert_eq!(ws.late_dropped(), 0, "pre-origin arrivals are not late");
}

/// t0 anchoring skips the empty prefix relative to t0, exactly like t0 = 0.
#[test]
fn t0_far_first_arrival_does_not_replay_empty_prefix() {
    let mut ws = WindowedStream::empty(WindowSpec::time(10, 10).with_t0(5));
    ws.push(t("s", "p", "a"), 1_002); // first window is [995,1005), not [5,15)
    let windows = ws.flush();
    assert_eq!(windows.len(), 1);
    assert_eq!((windows[0].start, windows[0].end), (995, 1005));
}

#[test]
#[should_panic(expected = "t0 applies to time windows only")]
fn t0_on_count_window_panics() {
    let _ = WindowSpec::count(3).with_t0(1);
}

// ---------------------------------------------------------------- eval modes

/// All four materialisation strategies are OBSERVATIONALLY IDENTICAL: the
/// same scripted stream (sliding overlap, duplicate triples across windows,
/// numeric aggregation, empty windows) produces the same per-window results.
#[test]
fn eval_modes_produce_identical_results() {
    use sparq_rsp::EvalMode;
    type Rows = Vec<Vec<Option<Term>>>;
    let run = |mode: EvalMode| -> Vec<(u64, u64, Rows)> {
        let mut q = ContinuousQuery::register(
            "SELECT ?s (AVG(?v) AS ?avg) WHERE { ?s <http://ex/value> ?v } GROUP BY ?s ORDER BY ?s",
            WindowSpec::time(10, 5), // sliding: consecutive windows share a suffix
        )
        .unwrap()
        .with_mode(mode);
        assert_eq!(q.mode(), mode);
        let mut out = Vec::new();
        let mut sink = |r: sparq_rsp::WindowResult| out.push((r.start, r.end, r.rows));
        for ts in 0..40u64 {
            // Two sensors; duplicate the SAME triple at several timestamps so
            // set semantics and the delta diff are both exercised.
            q.push(val(&format!("s{}", ts % 2), (ts % 7) as i32), ts, &mut sink).unwrap();
        }
        q.push(t("x", "p", "gap"), 60, &mut sink).unwrap(); // empty windows in between
        q.flush(&mut sink).unwrap();
        out
    };
    let rebuild = run(EvalMode::Rebuild);
    let persistent = run(EvalMode::PersistentDict);
    let delta = run(EvalMode::Delta);
    let snapshot = run(EvalMode::Snapshot);
    assert!(!rebuild.is_empty());
    assert_eq!(rebuild, persistent, "PersistentDict diverged from the rebuild baseline");
    assert_eq!(rebuild, delta, "Delta diverged from the rebuild baseline");
    assert_eq!(rebuild, snapshot, "Snapshot diverged from the rebuild baseline");
}

/// Delta mode stays correct across overlay compactions (churn > window) and
/// respects set semantics when a triple occurs at several timestamps and only
/// some of them are evicted by the slide.
#[test]
fn delta_mode_compaction_and_multi_timestamp_eviction() {
    use sparq_rsp::EvalMode;
    let run = |mode: EvalMode| -> Vec<Vec<Vec<Option<Term>>>> {
        let mut q = ContinuousQuery::register(
            "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }",
            WindowSpec::time(100, 50),
        )
        .unwrap()
        .with_mode(mode);
        let mut out = Vec::new();
        let mut sink = |r: sparq_rsp::WindowResult| out.push(r.rows);
        for ts in 0..1500u64 {
            // Distinct triple per tick → enough churn to cross the compaction
            // threshold several times…
            q.push(t("s", "p", &format!("o{ts}")), ts, &mut sink).unwrap();
            // …plus one RECURRING triple every 30 ticks: present at several
            // timestamps per window, evicted only when ALL occurrences leave.
            if ts % 30 == 0 {
                q.push(t("s", "p", "recurring"), ts, &mut sink).unwrap();
            }
        }
        q.flush(&mut sink).unwrap();
        out
    };
    assert_eq!(run(EvalMode::Delta), run(EvalMode::Rebuild));
    // (sq-j39) Snapshot shares Delta's live-graph maintenance (incl.
    // the same overlay-compaction trigger and multi-timestamp eviction), so it
    // too must track the rebuild baseline across many compactions.
    assert_eq!(run(EvalMode::Snapshot), run(EvalMode::Rebuild));
}

/// PersistentDict dictionary compaction (sq-lhg): a long synthetic
/// stream with HIGH VOCABULARY CHURN — every tick contributes a brand-new
/// subject and object IRI — must NOT grow the lifetime dictionary without
/// bound. The dictionary is reclaimed as terms age out of every live window, so
/// it stays proportional to the live window content, not to the all-time
/// vocabulary; and the per-window results stay byte-identical to the
/// uncompacted `Rebuild` reference (the remap is a bijection on live ids).
#[test]
fn persistent_dict_compaction_bounds_growth_on_churning_vocabulary() {
    use sparq_rsp::EvalMode;

    // (start, end, rows) per closed window — the full observable per-window result.
    type WindowRows = (u64, u64, Vec<Vec<Option<Term>>>);

    const TICKS: u64 = 30_000; // long enough to cross the 1024-dead trigger many times
    let q_str = "SELECT ?s ?o WHERE { ?s <http://ex/value> ?o } ORDER BY ?s ?o";
    // Tumbling RANGE 10 STEP 10: each window holds ~10 ticks' vocabulary, and a
    // closed window's terms age out immediately — so the all-time vocabulary
    // (~2·TICKS distinct IRIs) dwarfs any single window's.
    let spec = WindowSpec::time(10, 10);

    // Distinct subject AND object every tick: unbounded vocabulary growth.
    let triple = |ts: u64| -> [Term; 3] {
        [
            iri(&format!("subj{ts}")),
            iri("value"),
            iri(&format!("obj{ts}")),
        ]
    };

    // --- Reference: Rebuild mode (fresh dict per window, never persists). ---
    let mut reference: Vec<WindowRows> = Vec::new();
    {
        let mut q = ContinuousQuery::register(q_str, spec).unwrap().with_mode(EvalMode::Rebuild);
        assert_eq!(q.dict_len(), None, "Rebuild has no lifetime dictionary");
        let mut sink = |r: sparq_rsp::WindowResult| reference.push((r.start, r.end, r.rows));
        for ts in 0..TICKS {
            q.push(triple(ts), ts, &mut sink).unwrap();
        }
        q.flush(&mut sink).unwrap();
    }

    // --- Under test: PersistentDict, sampling the dictionary size over time. ---
    let mut got: Vec<WindowRows> = Vec::new();
    let mut max_dict_len = 0usize;
    let mut samples: Vec<usize> = Vec::new();
    {
        let mut q =
            ContinuousQuery::register(q_str, spec).unwrap().with_mode(EvalMode::PersistentDict);
        let mut sink = |r: sparq_rsp::WindowResult| got.push((r.start, r.end, r.rows));
        for ts in 0..TICKS {
            q.push(triple(ts), ts, &mut sink).unwrap();
            let len = q.dict_len().expect("PersistentDict exposes its dict size");
            max_dict_len = max_dict_len.max(len);
            if ts % 3000 == 0 {
                samples.push(len);
            }
        }
        q.flush(&mut sink).unwrap();
    }

    // 1) Results are IDENTICAL to the uncompacted reference — the compaction
    //    remap preserved every still-referenced term and every window's rows.
    assert_eq!(got, reference, "PersistentDict (compacting) diverged from the Rebuild reference");
    assert!(!reference.is_empty());

    // 2) The dictionary is BOUNDED: the all-time vocabulary is ~2·TICKS + 1
    //    distinct IRIs (a subject and an object per tick, plus <value>), but the
    //    dictionary never holds more than a tiny multiple of one window's
    //    vocabulary. A monotonic (un-compacted) dictionary would reach ~60 000
    //    here; the bound is well under a few thousand.
    let all_time_vocab = 2 * TICKS as usize + 1;
    assert!(
        max_dict_len < all_time_vocab / 10,
        "dictionary grew toward the all-time vocabulary ({max_dict_len} vs all-time {all_time_vocab}); \
         compaction is not reclaiming aged-out terms"
    );
    // Measured peak here is ~2 063 (≈ the 1024-dead trigger + ~half the live
    // set + one window), versus 60 001 all-time. 3 000 pins the bound tightly
    // while leaving headroom (dict_len counts terms, so it is deterministic).
    assert!(
        max_dict_len < 3_000,
        "dictionary not bounded near the live window content: peaked at {max_dict_len} terms"
    );

    // 3) The dictionary does NOT grow monotonically: it oscillates (grows as
    //    fresh vocabulary arrives, DROPS at each compaction as aged-out terms
    //    are reclaimed). The defining evidence of reclamation is that the size
    //    decreases at least once — a monotonic dictionary never would — while
    //    every sample stays within the bound established above.
    assert!(
        samples.iter().copied().max().unwrap_or(0) <= max_dict_len,
        "samples exceeded the tracked maximum (bookkeeping bug)"
    );
    let shrinks = samples.windows(2).any(|w| w[1] < w[0]);
    assert!(
        shrinks,
        "dictionary never shrank across the stream (samples {samples:?}) — terms are not being reclaimed"
    );
}

// ---------------------------------------------------------------- CONSTRUCT / ASK

fn construct_objs(triples: &[oxrdf::Triple]) -> Vec<String> {
    triples
        .iter()
        .map(|t| match &t.object {
            Term::NamedNode(n) => n.as_str().trim_start_matches("http://ex/").to_owned(),
            other => other.to_string(),
        })
        .collect()
}

/// CONSTRUCT per window: stream-to-stream transformation, RSTREAM default.
#[test]
fn continuous_construct_emits_transformed_window_graphs() {
    use sparq_rsp::ContinuousConstruct;
    let mut q = ContinuousConstruct::register(
        "CONSTRUCT { ?s <http://ex/seen> ?o } WHERE { ?s <http://ex/p> ?o }",
        WindowSpec::time(10, 10),
    )
    .unwrap();
    let mut out = Vec::new();
    let mut sink = |r: sparq_rsp::GraphResult| out.push(r);
    q.push(t("s", "p", "a"), 1, &mut sink).unwrap();
    q.push(t("s", "p", "a"), 5, &mut sink).unwrap(); // duplicate: sets stay sets
    q.push(t("s", "p", "b"), 12, &mut sink).unwrap(); // closes [0,10)
    q.flush(&mut sink).unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!((out[0].start, out[0].end), (0, 10));
    assert_eq!(construct_objs(&out[0].triples), ["a"]);
    assert!(out[0].triples.iter().all(|t| t.predicate.as_str() == "http://ex/seen"));
    assert_eq!(construct_objs(&out[1].triples), ["b"]);
}

/// CONSTRUCT ISTREAM/DSTREAM are exact SET differences of consecutive
/// windows' constructed graphs.
#[test]
fn continuous_construct_istream_dstream_set_diffs() {
    use sparq_rsp::ContinuousConstruct;
    let sparql = "CONSTRUCT { ?s <http://ex/seen> ?o } WHERE { ?s <http://ex/p> ?o }";
    let script: &[([Term; 3], u64)] = &[
        (t("s", "p", "a"), 1),
        (t("s", "p", "b"), 5),
        (t("s", "p", "a"), 12), // [10,20): a stays, b leaves
        (t("s", "p", "c"), 15),
    ];
    let run = |r2s: R2S| -> Vec<Vec<String>> {
        let mut q = ContinuousConstruct::register(sparql, WindowSpec::time(10, 10))
            .unwrap()
            .with_r2s(r2s);
        let mut out = Vec::new();
        let mut sink = |r: sparq_rsp::GraphResult| out.push(construct_objs(&r.triples));
        for (triple, ts) in script {
            q.push(triple.clone(), *ts, &mut sink).unwrap();
        }
        q.flush(&mut sink).unwrap();
        out
    };
    let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(run(R2S::RStream), vec![v(&["a", "b"]), v(&["a", "c"])]);
    assert_eq!(run(R2S::IStream), vec![v(&["a", "b"]), v(&["c"])]);
    assert_eq!(run(R2S::DStream), vec![v(&[]), v(&["b"])]);
}

/// ASK per window: one boolean per closed window, including empty windows.
#[test]
fn continuous_ask_reports_per_window_booleans() {
    use sparq_rsp::ContinuousAsk;
    let mut q = ContinuousAsk::register(
        "ASK { ?s <http://ex/p> <http://ex/alert> }",
        WindowSpec::time(10, 10),
    )
    .unwrap();
    let mut out = Vec::new();
    let mut sink = |r: sparq_rsp::AskResult| out.push((r.start, r.value));
    q.push(t("s", "p", "alert"), 1, &mut sink).unwrap();
    q.push(t("s", "p", "calm"), 12, &mut sink).unwrap();
    q.push(t("s", "p", "alert"), 35, &mut sink).unwrap(); // [20,30) closes empty
    q.flush(&mut sink).unwrap();
    assert_eq!(out, vec![(0, true), (10, false), (20, false), (30, true)]);
}

/// CONSTRUCT/ASK registration validates the query form up front.
#[test]
fn construct_and_ask_registration_validate_query_form() {
    use sparq_rsp::{ContinuousAsk, ContinuousConstruct};
    let spec = WindowSpec::time(10, 10);
    assert!(ContinuousConstruct::register("SELECT * WHERE { ?s ?p ?o }", spec).is_err());
    assert!(ContinuousConstruct::register("ASK { ?s ?p ?o }", spec).is_err());
    assert!(ContinuousConstruct::register("CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }", spec).is_ok());
    assert!(ContinuousAsk::register("SELECT * WHERE { ?s ?p ?o }", spec).is_err());
    assert!(ContinuousAsk::register("ASK { ?s ?p ?o }", spec).is_ok());
}
