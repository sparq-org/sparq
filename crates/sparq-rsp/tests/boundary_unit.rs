// sq-qcnn.18 — Direct unit tests for sparq-rsp window boundaries,
// exact-value assertions, panic guards, accessors, and RSP-QL parser error paths.
//
// Design contract (sq-qcnn test-quality plan):
// * Every test asserts EXACT VALUES, not merely "no panic".
// * Window-open/close boundary tests pin the exact tick where closure fires,
//   killing `<` → `<=` comparison-flip mutants.
// * One direct test per public fn surface to avoid the #1250 indirect-coverage
//   trap (a public fn reached only through a high-level integration test still
//   sits at ~0% individual coverage).
// * The `advance()` heartbeat path, `TripleStream` helpers, all `WindowSpec`
//   panic guards, accessor fns (`sparql()`, `spec()`, `r2s()`, …), and the
//   RSP-QL parser's error paths are each exercised DIRECTLY here.

use oxrdf::{Literal, NamedNode, Term};
use sparq_rsp::{
    ContinuousAsk, ContinuousConstruct, ContinuousMultiQuery, ContinuousQuery, EvalMode, R2S,
    RspqlQuery, TimestampedTriple, TripleStream, Window, WindowSpec, WindowedStream,
};

// ───────────────────────────────────────────────────── helpers

fn iri(s: &str) -> Term {
    NamedNode::new_unchecked(format!("http://ex/{}", s)).into()
}

fn t(s: &str, p: &str, o: &str) -> [Term; 3] {
    [iri(s), iri(p), iri(o)]
}

fn objs(w: &Window) -> Vec<String> {
    w.triples
        .iter()
        .map(|tt| match &tt.triple[2] {
            Term::NamedNode(n) => n.as_str().trim_start_matches("http://ex/").to_owned(),
            other => other.to_string(),
        })
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════════
// A. TRIPLE-STREAM DIRECT TESTS — len/is_empty/items/push_item
// ═══════════════════════════════════════════════════════════════════════════════

// sq-qcnn.18: TripleStream::new() starts empty; len() and is_empty()
// both reflect the correct count before and after push; items() returns the
// pushed slice; push_item() appends a pre-wrapped TimestampedTriple.
#[test]
fn triple_stream_len_is_empty_items_push_item() {
    let mut s = TripleStream::new();
    assert_eq!(s.len(), 0);
    assert!(s.is_empty());

    s.push(t("s", "p", "a"), 1);
    assert_eq!(s.len(), 1);
    assert!(!s.is_empty());

    let item = TimestampedTriple { triple: t("s", "p", "b"), ts: 2 };
    s.push_item(item.clone());
    assert_eq!(s.len(), 2);

    let items = s.items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].ts, 1);
    assert_eq!(items[1].ts, 2);

    // into_items consumes and returns in push order
    let all = s.into_items();
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].triple, item.triple);
}

// ═══════════════════════════════════════════════════════════════════════════════
// B. WINDOWSPEC PANIC GUARDS (direct, each in its own #[should_panic] fn)
// ═══════════════════════════════════════════════════════════════════════════════

// sq-qcnn.18: time(0, 1) panics — zero range is meaningless.
#[test]
#[should_panic(expected = "window RANGE must be > 0")]
fn window_spec_time_range_zero_panics() {
    let _ = WindowSpec::time(0, 1);
}

// sq-qcnn.18: time(1, 0) panics — zero step loops forever.
#[test]
#[should_panic(expected = "window STEP must be > 0")]
fn window_spec_time_step_zero_panics() {
    let _ = WindowSpec::time(1, 0);
}

// sq-qcnn.18: count(0) panics — zero-row window is meaningless.
#[test]
#[should_panic(expected = "window ROWS must be > 0")]
fn window_spec_count_zero_rows_panics() {
    let _ = WindowSpec::count(0);
}

// sq-qcnn.18: with_max_delay on a count window panics (lateness
// tolerance is a time-window concept — no watermark on count windows).
#[test]
#[should_panic(expected = "max_delay applies to time windows only")]
fn with_max_delay_on_count_window_panics() {
    let _ = WindowSpec::count(3).with_max_delay(5);
}

// sq-qcnn.18: with_slide on a time window panics (STEP drives time
// windows; SLIDE drives count windows).
#[test]
#[should_panic(expected = "slide applies to count windows only")]
fn with_slide_on_time_window_panics() {
    let _ = WindowSpec::time(10, 10).with_slide(2);
}

// sq-qcnn.18: with_slide(0) panics — zero slide would loop.
#[test]
#[should_panic(expected = "window SLIDE must be > 0")]
fn with_slide_zero_panics() {
    let _ = WindowSpec::count(3).with_slide(0);
}

// ═══════════════════════════════════════════════════════════════════════════════
// C. WINDOWED-STREAM ACCESSOR + ADVANCE DIRECT TESTS
// ═══════════════════════════════════════════════════════════════════════════════

// sq-qcnn.18: WindowedStream::spec() round-trips the WindowSpec it
// was built with — a simple accessor that shows up as uncovered when the spec
// is only accessed internally.
#[test]
fn windowed_stream_spec_accessor() {
    let spec = WindowSpec::time(20, 10);
    let ws: WindowedStream = WindowedStream::empty(spec);
    assert_eq!(ws.spec(), spec);

    let cspec = WindowSpec::count(5).with_slide(2);
    let ws2: WindowedStream = WindowedStream::empty(cspec);
    assert_eq!(ws2.spec(), cspec);
}

// sq-qcnn.18: advance() is the shared-clock heartbeat for multi-
// window joins — it drives the watermark forward WITHOUT inserting a triple.
// Test: advance to a timestamp that crosses a window boundary; the window
// closes and is empty (no triples were pushed). Then push a triple after the
// advance; it belongs to the NEXT window.
#[test]
fn advance_closes_empty_window_without_inserting() {
    let mut ws: WindowedStream = WindowedStream::empty(WindowSpec::time(10, 10));
    // push one triple into [0,10)
    ws.push(t("s", "p", "a"), 3);
    assert!(ws.take_closed().is_empty(), "watermark 3 < 10, no close yet");
    // advance to ts=10 (watermark 10 >= 10): closes [0,10) even though advance
    // inserts no triple
    ws.advance(10);
    let closed = ws.take_closed();
    assert_eq!(closed.len(), 1, "advance to 10 closes [0,10)");
    assert_eq!((closed[0].start, closed[0].end), (0, 10));
    assert_eq!(objs(&closed[0]), ["a"], "content is the earlier push, not advance");

    // Now push into [10,20) — advance did not insert into the buffer
    ws.push(t("s", "p", "b"), 15);
    let rest = ws.flush();
    assert_eq!(rest.len(), 1);
    assert_eq!((rest[0].start, rest[0].end), (10, 20));
    assert_eq!(objs(&rest[0]), ["b"]);
    assert_eq!(ws.late_dropped(), 0);
}

// sq-qcnn.18: advance() on a COUNT window is a no-op (count windows
// have no time axis). Verify it does not close anything and does not panic.
#[test]
fn advance_is_noop_on_count_window() {
    let mut ws: WindowedStream = WindowedStream::empty(WindowSpec::count(2));
    ws.advance(9999); // must not panic
    assert!(ws.take_closed().is_empty(), "count window ignores advance");
    ws.push(t("s", "p", "a"), 1);
    assert_eq!(ws.take_closed().len(), 1, "count window still fires on push");
}

// ═══════════════════════════════════════════════════════════════════════════════
// D. WINDOW-BOUNDARY EXACT-VALUE MUTATION KILLERS
// ═══════════════════════════════════════════════════════════════════════════════

// sq-qcnn.18: the watermark-driven close condition is
//   `watermark >= start + range` (equivalent: `!(watermark < start + range)`).
// A `<` → `<=` mutation would fire the close one tick TOO EARLY (at
// watermark = start+range - 1 instead of start+range). Pin the exact tick.
//
// Config: RANGE 10 STEP 10, max_delay 0 → watermark == max_ts.
// Window [0,10) should close when watermark REACHES 10, not at 9.
#[test]
fn close_fires_exactly_at_watermark_equal_to_end_not_before() {
    let mut ws: WindowedStream = WindowedStream::empty(WindowSpec::time(10, 10));
    ws.push(t("s", "p", "a"), 0); // window starts
    ws.push(t("s", "p", "b"), 9); // watermark = 9, end = 10: NOT YET closed
    assert!(ws.take_closed().is_empty(), "watermark 9 < end 10, window still open");
    // watermark = 10 == end: NOW closes
    ws.push(t("s", "p", "c"), 10); // belongs to [10,20); closes [0,10)
    let closed = ws.take_closed();
    assert_eq!(closed.len(), 1, "exactly one window closed");
    assert_eq!((closed[0].start, closed[0].end), (0, 10));
    // Content: a and b (ts 0 and 9), NOT c (ts 10 belongs to next window)
    assert_eq!(objs(&closed[0]), ["a", "b"]);
    // The remaining open [10,20) holds c
    let rest = ws.flush();
    assert_eq!(objs(&rest[0]), ["c"]);
}

// sq-qcnn.18: With max_delay > 0 the watermark is max_ts - max_delay.
// Close fires when watermark (= max_ts - max_delay) >= end (= range).
// A `<`→`<=` mutant would close one tick early.
// Pin: RANGE 10 STEP 10 max_delay 3. end=10. Close when max_ts - 3 >= 10,
// i.e. max_ts >= 13. At max_ts=12 (watermark=9): NOT closed. At max_ts=13: closed.
#[test]
fn close_with_max_delay_fires_at_exact_boundary() {
    let mut ws: WindowedStream =
        WindowedStream::empty(WindowSpec::time(10, 10).with_max_delay(3));
    ws.push(t("s", "p", "a"), 5);
    ws.push(t("s", "p", "b"), 12); // watermark = 12 - 3 = 9 < 10: still open
    assert!(ws.take_closed().is_empty(), "watermark 9 < end 10, not closed");
    ws.push(t("s", "p", "c"), 13); // watermark = 13 - 3 = 10 >= 10: now closes
    let closed = ws.take_closed();
    assert_eq!(closed.len(), 1);
    assert_eq!((closed[0].start, closed[0].end), (0, 10));
    // Only "a" (ts=5) is in [0,10). "b" (ts=12) and "c" (ts=13) go into [10,20).
    let obj_set: Vec<String> = objs(&closed[0]);
    assert_eq!(obj_set, vec!["a".to_string()], "only a is in [0,10)");
    // Neither b nor c bled into the closed window
    assert!(!obj_set.contains(&"b".to_string()), "b (ts=12) is in [10,20)");
    assert!(!obj_set.contains(&"c".to_string()), "c (ts=13) is in [10,20)");
}

// sq-qcnn.18: gap-detection boundary (step > range).
// `rel - k_max * step < range` determines if a timestamp lands in a window.
// When ts == range (= 5 with RANGE 5 STEP 10), rel=5, k_max=0, 5-0=5 which
// is NOT < 5 → ts=5 is a GAP (no window covers it). A `<` → `<=` mutant
// would include ts=5 in window [0,5) (wrong). Pin exactly.
#[test]
fn gap_boundary_ts_equal_to_range_is_not_in_window() {
    // RANGE 5 STEP 10: windows [0,5) [10,15) ...
    let mut ws: WindowedStream = WindowedStream::empty(WindowSpec::time(5, 10));
    ws.push(t("s", "p", "last_in"), 4); // ts=4: in [0,5)
    ws.push(t("s", "p", "gap_at_5"), 5); // ts=5: GAP, NOT in [0,5)
    ws.push(t("s", "p", "force"), 12); // closes [0,5) via watermark advance
    let closed = ws.take_closed();
    assert_eq!(closed.len(), 1);
    assert_eq!((closed[0].start, closed[0].end), (0, 5));
    assert_eq!(objs(&closed[0]), ["last_in"], "ts=5 is outside [0,5): it is a gap");
    // Gap triple was not counted as late (no window ever covered ts=5)
    assert_eq!(ws.late_dropped(), 0, "gap timestamps are NOT counted late");
}

// sq-qcnn.18: too-late detection. A triple is late when its LAST
// covering window (k_max) has already advanced past next_close. The check is
// `k_max < next_close` (where next_close is the index of the oldest unclosed
// window). A `<`→`<=` mutant would drop triples in the CURRENT open window
// as though they were late. Pin: a triple with k_max == next_close is NOT late.
#[test]
fn triple_at_open_window_boundary_is_not_late() {
    let mut ws: WindowedStream = WindowedStream::empty(WindowSpec::time(10, 10));
    // Force next_close to advance to 1 by closing [0,10)
    ws.push(t("s", "p", "a"), 1);
    ws.push(t("s", "p", "b"), 10); // closes [0,10); next_close = 1 (k of [10,20))
    let _ = ws.take_closed();
    // Now push a triple into [10,20): k_max = 10/10 = 1 == next_close = 1.
    // Must NOT be dropped as late.
    ws.push(t("s", "p", "c"), 10); // ts 10 → k_max = 1 = next_close (open window)
    assert_eq!(ws.late_dropped(), 0, "ts=10 is in the open window, not late");
    let rest = ws.flush();
    let window_10 = rest.iter().find(|w| w.start == 10).expect("[10,20) exists");
    assert!(objs(window_10).contains(&"c".to_string()), "c entered [10,20)");
}

// ═══════════════════════════════════════════════════════════════════════════════
// E. CONTINUOUS-QUERY ACCESSOR TESTS
// ═══════════════════════════════════════════════════════════════════════════════

// sq-qcnn.18: Accessor methods on all three continuous-query types.
// These are SIMPLE GETTERS that tests often reach only indirectly (via push/flush
// callbacks) or not at all — the #1250 direct-test discipline requires one direct
// call per public fn.

#[test]
fn continuous_query_accessors() {
    let sparql = "SELECT ?o WHERE { ?s ?p ?o }";
    let spec = WindowSpec::time(10, 5);
    let q = ContinuousQuery::register(sparql, spec).unwrap().with_r2s(R2S::IStream);
    assert_eq!(q.sparql(), sparql);
    assert_eq!(q.spec(), spec);
    assert_eq!(q.mode(), EvalMode::PersistentDict); // the default
    assert_eq!(q.late_dropped(), 0);
}

#[test]
fn continuous_query_with_mode_roundtrip() {
    let q = ContinuousQuery::register("SELECT ?o WHERE { ?s ?p ?o }", WindowSpec::time(10, 10))
        .unwrap()
        .with_mode(EvalMode::Rebuild);
    assert_eq!(q.mode(), EvalMode::Rebuild);
}

#[test]
fn continuous_construct_accessors() {
    let sparql = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
    let spec = WindowSpec::time(10, 10);
    let q = ContinuousConstruct::register(sparql, spec).unwrap();
    assert_eq!(q.sparql(), sparql);
    assert_eq!(q.spec(), spec);
    assert_eq!(q.late_dropped(), 0);
}

#[test]
fn continuous_construct_with_mode_roundtrip() {
    let q = ContinuousConstruct::register(
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
        WindowSpec::time(10, 10),
    )
    .unwrap()
    .with_mode(EvalMode::Delta);
    // with_mode returns Self; the mode change is internal (no public accessor on
    // ContinuousConstruct) but the push path still works correctly.
    let mut out = Vec::new();
    let mut q2 = q;
    q2.flush(|r| out.push(r)).unwrap(); // no-op (nothing pushed), must not panic
    assert!(out.is_empty(), "flush() with no pushes must emit nothing, got {:?} windows", out.len()); //
}

#[test]
fn continuous_ask_accessors_and_with_mode() {
    let sparql = "ASK { ?s ?p ?o }";
    let spec = WindowSpec::time(10, 10);
    let q = ContinuousAsk::register(sparql, spec)
        .unwrap()
        .with_mode(EvalMode::PersistentDict);
    assert_eq!(q.sparql(), sparql);
    assert_eq!(q.spec(), spec);
    assert_eq!(q.late_dropped(), 0);
}

#[test]
fn continuous_ask_late_dropped_increments() {
    let spec = WindowSpec::time(10, 10);
    let mut q = ContinuousAsk::register("ASK { ?s ?p ?o }", spec).unwrap();
    let mut results = Vec::new();
    q.push(t("s", "p", "a"), 5, |r| results.push(r)).unwrap();
    q.push(t("s", "p", "b"), 15, |r| results.push(r)).unwrap(); // closes [0,10)
    q.push(t("s", "p", "late"), 3, |r| results.push(r)).unwrap(); // dropped
    q.flush(|r| results.push(r)).unwrap();
    assert_eq!(q.late_dropped(), 1);
}

#[test]
fn continuous_construct_late_dropped_increments() {
    let spec = WindowSpec::time(10, 10);
    let mut q = ContinuousConstruct::register(
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
        spec,
    )
    .unwrap();
    let mut out = Vec::new();
    q.push(t("s", "p", "a"), 5, |r| out.push(r)).unwrap();
    q.push(t("s", "p", "b"), 15, |r| out.push(r)).unwrap(); // closes [0,10)
    q.push(t("s", "p", "late"), 3, |r| out.push(r)).unwrap(); // dropped
    q.flush(|r| out.push(r)).unwrap();
    assert_eq!(q.late_dropped(), 1);
}

// ═══════════════════════════════════════════════════════════════════════════════
// F. CONTINUOUS-MULTI-QUERY ACCESSOR + ERROR TESTS
// ═══════════════════════════════════════════════════════════════════════════════

const Q2: &str = "\
REGISTER DSTREAM <http://ex/out> AS
SELECT ?o WHERE {
  WINDOW <http://ex/w1> { ?s <http://ex/p> ?o }
  WINDOW <http://ex/w2> { ?s <http://ex/q> ?o }
}
FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> RANGE 10 STEP 10
FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 10 STEP 10";

// sq-qcnn.18: All ContinuousMultiQuery accessors are called directly.
#[test]
fn continuous_multi_query_accessors() {
    let q = ContinuousMultiQuery::register(Q2).unwrap();
    assert!(q.rspql().contains("DSTREAM"));
    // sparql() is the rewritten form: WINDOW → GRAPH
    assert!(q.sparql().contains("GRAPH <http://ex/w1>"), "got: {}", q.sparql());
    assert!(!q.sparql().contains("WINDOW"), "WINDOW not in rewritten sparql");
    let iris = q.window_iris();
    assert_eq!(iris.len(), 2);
    assert_eq!(iris[0].as_str(), "http://ex/w1");
    assert_eq!(iris[1].as_str(), "http://ex/w2");
    assert_eq!(q.output_stream().unwrap().as_str(), "http://ex/out");
    assert_eq!(q.r2s(), R2S::DStream);
}

// sq-qcnn.18: from_rspql rejects non-SELECT embedded queries.
#[test]
fn multi_query_rejects_non_select_sparql() {
    // ASK body: WINDOW gets rewritten to GRAPH but the result is parsed as ASK,
    // not SELECT.
    let ask_rspql = "\
ASK { WINDOW <http://ex/w1> { ?s ?p ?o } WINDOW <http://ex/w2> { ?s ?q ?r } }
FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> RANGE 10
FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 10";
    match ContinuousMultiQuery::register(ask_rspql) {
        Ok(_) => panic!("ASK should be rejected by multi-window register"),
        Err(e) => assert!(e.contains("SELECT"), "got: {}", e),
    }
}

// sq-qcnn.18: from_rspql rejects fewer than 2 windows.
#[test]
fn multi_query_rejects_single_window() {
    let one = "\
SELECT * WHERE { WINDOW <http://ex/w1> { ?s ?p ?o } }
FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> RANGE 10";
    match ContinuousMultiQuery::register(one) {
        Ok(_) => panic!("single window should be rejected"),
        Err(e) => assert!(e.contains("ContinuousQuery") || e.contains("2"), "got: {}", e),
    }
}

// sq-qcnn.18: a WINDOW IRI referenced in the WHERE but not declared
// with FROM NAMED WINDOW is an error.
#[test]
fn multi_query_rejects_undeclared_window_reference() {
    let bad = "\
SELECT * WHERE { WINDOW <http://ex/w1> { ?s ?p ?o } WINDOW <http://ex/UNDECLARED> { ?s ?q ?r } }
FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> RANGE 10
FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 10";
    match ContinuousMultiQuery::register(bad) {
        Ok(_) => panic!("undeclared window reference should be rejected"),
        Err(e) => assert!(
            e.contains("UNDECLARED") || e.contains("declared"),
            "got: {}",
            e
        ),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// G. RSP-QL PARSER ERROR PATHS
// ═══════════════════════════════════════════════════════════════════════════════

// sq-qcnn.18: parse_duration_token rejects unsupported ISO-8601 units
// (years and months have no fixed second count).
#[test]
fn rspql_rejects_year_and_month_durations() {
    let year_q = "SELECT * WHERE { WINDOW <http://ex/w1> { ?s ?p ?o } WINDOW <http://ex/w2> { ?s ?q ?r } }\n\
                  FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> RANGE P1Y\n\
                  FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 5";
    assert!(RspqlQuery::parse(year_q).is_err(), "P1Y should be rejected");

    let month_q = "SELECT * WHERE { WINDOW <http://ex/w1> { ?s ?p ?o } WINDOW <http://ex/w2> { ?s ?q ?r } }\n\
                   FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> RANGE P2M\n\
                   FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 5";
    assert!(RspqlQuery::parse(month_q).is_err(), "P2M (months) should be rejected");
}

// sq-qcnn.18: parse_duration_token rejects bad time-unit chars.
#[test]
fn rspql_rejects_bad_time_unit_in_duration() {
    let bad_unit = "SELECT * WHERE { WINDOW <http://ex/w1> { ?s ?p ?o } WINDOW <http://ex/w2> { ?s ?q ?r } }\n\
                    FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> RANGE PT10X\n\
                    FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 5";
    assert!(RspqlQuery::parse(bad_unit).is_err(), "PT10X has unknown unit X");
}

// sq-qcnn.18: take_duration rejects an empty string after RANGE/STEP.
#[test]
fn rspql_rejects_empty_duration() {
    // The `{` immediately after RANGE is an empty duration token.
    let empty_dur = "SELECT * WHERE { WINDOW <http://ex/w1> { ?s ?p ?o } WINDOW <http://ex/w2> { ?s ?q ?r } }\n\
                     FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> RANGE {\n\
                     FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 5";
    assert!(RspqlQuery::parse(empty_dur).is_err(), "empty duration should be rejected");
}

// sq-qcnn.18: parse_window_spec rejects an unterminated bracket.
#[test]
fn rspql_rejects_unterminated_bracket() {
    let unclosed = "SELECT * WHERE { WINDOW <http://ex/w1> { ?s ?p ?o } WINDOW <http://ex/w2> { ?s ?q ?r } }\n\
                    FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> [RANGE 10 STEP 5\n\
                    FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 5";
    assert!(RspqlQuery::parse(unclosed).is_err(), "unclosed [ should be rejected");
}

// sq-qcnn.18: REGISTER header with no AS keyword is rejected.
#[test]
fn rspql_rejects_register_without_as() {
    let no_as = "REGISTER <http://ex/out>\n\
                 SELECT * WHERE { WINDOW <http://ex/w1> { ?s ?p ?o } }\n\
                 FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> RANGE 10";
    assert!(RspqlQuery::parse(no_as).is_err());
}

// sq-qcnn.18: REGISTER DSTREAM / REGISTER RSTREAM keywords parse
// correctly (the parser accepts the full R2S keyword set: STREAM, RSTREAM,
// ISTREAM, DSTREAM).
#[test]
fn rspql_register_dstream_and_rstream_keywords() {
    let mk = |kw: &str| {
        format!(
            "REGISTER {kw} <http://ex/out> AS\n\
             SELECT * WHERE {{ WINDOW <http://ex/w1> {{ ?s ?p ?o }} WINDOW <http://ex/w2> {{ ?s ?q ?r }} }}\n\
             FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> RANGE 10\n\
             FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 10"
        )
    };
    let ds = RspqlQuery::parse(&mk("DSTREAM")).unwrap();
    assert_eq!(ds.r2s, R2S::DStream);

    let rs = RspqlQuery::parse(&mk("RSTREAM")).unwrap();
    assert_eq!(rs.r2s, R2S::RStream);

    let stream = RspqlQuery::parse(&mk("STREAM")).unwrap();
    assert_eq!(stream.r2s, R2S::RStream);
}

// sq-qcnn.18: ISO-8601 days (P1D) and combined minutes+seconds
// (PT1M30S) parse to the correct second counts.
#[test]
fn rspql_iso8601_days_and_combined_duration() {
    let q = |op: &str| {
        format!(
            "SELECT * WHERE {{ WINDOW <http://ex/w1> {{ ?s ?p ?o }} WINDOW <http://ex/w2> {{ ?s ?q ?r }} }}\n\
             FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> {op}\n\
             FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 5"
        )
    };
    // P1D = 86400 seconds (one day)
    let p = RspqlQuery::parse(&q("RANGE P1D")).unwrap();
    assert_eq!(p.windows[0].spec, WindowSpec::time(86_400, 86_400));

    // PT1M30S = 60 + 30 = 90 seconds
    let p2 = RspqlQuery::parse(&q("RANGE PT1M30S STEP PT30S")).unwrap();
    assert_eq!(p2.windows[0].spec, WindowSpec::time(90, 30));

    // PT2H = 7200 seconds
    let p3 = RspqlQuery::parse(&q("RANGE PT2H STEP PT1H")).unwrap();
    assert_eq!(p3.windows[0].spec, WindowSpec::time(7_200, 3_600));
}

// sq-qcnn.18: FROM NAMED WINDOW with a missing window IRI is rejected.
#[test]
fn rspql_rejects_missing_window_iri() {
    let bad = "SELECT * WHERE { WINDOW <http://ex/w1> { ?s ?p ?o } WINDOW <http://ex/w2> { ?s ?q ?r } }\n\
               FROM NAMED WINDOW ON <http://ex/s1> RANGE 10\n\
               FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 10";
    assert!(RspqlQuery::parse(bad).is_err());
}

// sq-qcnn.18: FROM NAMED WINDOW with a missing ON / stream IRI is rejected.
#[test]
fn rspql_rejects_missing_stream_iri() {
    let bad = "SELECT * WHERE { WINDOW <http://ex/w1> { ?s ?p ?o } WINDOW <http://ex/w2> { ?s ?q ?r } }\n\
               FROM NAMED WINDOW <http://ex/w1> <http://ex/s1> RANGE 10\n\
               FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 10";
    assert!(RspqlQuery::parse(bad).is_err());
}

// sq-qcnn.18: a WINDOW keyword INSIDE a quoted string is NOT rewritten
// to GRAPH (the scanner skips string literals). This pins that the string-scanner
// path is exercised and does not corrupt the output.
#[test]
fn rspql_window_inside_string_literal_is_not_rewritten() {
    let q = "PREFIX ex: <http://ex/>\n\
             SELECT * WHERE {\n\
               WINDOW ex:w1 { ?s ex:label \"this is a WINDOW label\" }\n\
               WINDOW ex:w2 { ?s ex:q ?r }\n\
             }\n\
             FROM NAMED WINDOW ex:w1 ON ex:s1 RANGE 10\n\
             FROM NAMED WINDOW ex:w2 ON ex:s2 RANGE 10";
    let parsed = RspqlQuery::parse(q).unwrap();
    // The string literal is preserved verbatim
    assert!(parsed.sparql.contains("\"this is a WINDOW label\""));
    // Exactly 2 WINDOW → GRAPH rewrites (the keywords in WINDOW clauses)
    assert_eq!(parsed.sparql.matches("GRAPH").count(), 2, "got: {}", parsed.sparql);
}

// sq-qcnn.18: single-quote string literals are also scanned correctly.
#[test]
fn rspql_window_inside_single_quote_string_not_rewritten() {
    let q = "PREFIX ex: <http://ex/>\n\
             SELECT * WHERE {\n\
               WINDOW ex:w1 { ?s ex:p 'WINDOW' }\n\
               WINDOW ex:w2 { ?s ex:q ?r }\n\
             }\n\
             FROM NAMED WINDOW ex:w1 ON ex:s1 RANGE 5\n\
             FROM NAMED WINDOW ex:w2 ON ex:s2 RANGE 5";
    let parsed = RspqlQuery::parse(q).unwrap();
    assert!(parsed.sparql.contains("'WINDOW'"), "single-quote string preserved");
    assert_eq!(parsed.sparql.matches("GRAPH").count(), 2);
}

// sq-qcnn.18: parse_duration_token with a 'not a duration' prefix
// (no 'P' prefix for ISO-8601 and not a bare integer).
#[test]
fn rspql_rejects_non_duration_token() {
    let bad = "SELECT * WHERE { WINDOW <http://ex/w1> { ?s ?p ?o } WINDOW <http://ex/w2> { ?s ?q ?r } }\n\
               FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> RANGE TEN\n\
               FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 5";
    assert!(RspqlQuery::parse(bad).is_err());
}

// ═══════════════════════════════════════════════════════════════════════════════
// H. RSTREAM / ISTREAM / DSTREAM EXACT-VALUE DELTA TESTS
// ═══════════════════════════════════════════════════════════════════════════════

// sq-qcnn.18: multiset_minus semantics — a row appearing N times in
// the current window but only M < N times in the previous is emitted N-M times.
// This pins the budget-decrement path inside multiset_minus directly.
#[test]
fn istream_dstream_multiset_exact_counts() {
    let sparql = "SELECT ?v WHERE { ?s <http://ex/value> ?v }";
    let spec = WindowSpec::time(10, 10);

    // Window [0,10): sensor s1 and s2 both read 7 → row (7) twice
    // Window [10,20): s1 reads 7 once → row (7) once
    // ISTREAM diff: {7,7} → {7}: one occurrence of (7) removed → ISTREAM emits nothing for w2
    //   actually: prev={7,7}, cur={7}. IStream = cur \ prev = {7} \ {7,7} = {} (7 cancelled once)
    // DSTREAM diff: prev={7,7}, cur={7}. DStream = prev \ cur = {7,7} \ {7} = {7} (one copy lost)

    let run = |r2s: R2S| -> Vec<usize> {
        let mut q = ContinuousQuery::register(sparql, spec).unwrap().with_r2s(r2s);
        let mut counts = Vec::new();
        let mut sink = |r: sparq_rsp::WindowResult| counts.push(r.rows.len());
        let val = |v: i32| -> [Term; 3] {
            [iri("s1"), iri("value"), Literal::from(v).into()]
        };
        let val2 = |v: i32| -> [Term; 3] {
            [iri("s2"), iri("value"), Literal::from(v).into()]
        };
        q.push(val(7), 1, &mut sink).unwrap();
        q.push(val2(7), 2, &mut sink).unwrap(); // [0,10): (7) twice
        q.push(val(7), 11, &mut sink).unwrap(); // closes [0,10); [10,20): (7) once
        q.flush(&mut sink).unwrap();
        counts
    };

    let istream = run(R2S::IStream);
    let dstream = run(R2S::DStream);
    // ISTREAM: w1 emits 2 rows (both new); w2 emits 0 (both existing cancelled)
    assert_eq!(istream, vec![2, 0], "istream exact counts: {istream:?}");
    // DSTREAM: w1 emits 0 (nothing was prev); w2 emits 1 (one copy of (7) gone)
    assert_eq!(dstream, vec![0, 1], "dstream exact counts: {dstream:?}");
}

// sq-qcnn.18: CONSTRUCT R2S DSTREAM — set-difference test.
// set_minus fn is exercised through ContinuousConstruct::with_r2s(R2S::DStream).
// Pin that the exact triple set diff is computed (not just "non-empty result").
#[test]
fn construct_dstream_set_minus_exact() {
    let sparql = "CONSTRUCT { ?s <http://ex/seen> ?o } WHERE { ?s <http://ex/p> ?o }";
    let spec = WindowSpec::time(10, 10);
    let mut q = ContinuousConstruct::register(sparql, spec).unwrap().with_r2s(R2S::DStream);
    let mut out: Vec<sparq_rsp::GraphResult> = Vec::new();
    // Window [0,10): a, b → DSTREAM first window emits nothing (no prev)
    q.push(t("s", "p", "a"), 1, |r| out.push(r)).unwrap();
    q.push(t("s", "p", "b"), 5, |r| out.push(r)).unwrap();
    // Window [10,20): b, c → DSTREAM emits a (removed from prev={a,b})
    q.push(t("s", "p", "b"), 11, |r| out.push(r)).unwrap();
    q.push(t("s", "p", "c"), 15, |r| out.push(r)).unwrap();
    q.flush(|r| out.push(r)).unwrap();

    assert_eq!(out.len(), 2);
    // First window DStream: empty (no previous result)
    assert_eq!(out[0].triples.len(), 0, "DSTREAM first window always empty");
    // Second window DStream: exactly 'a' was removed
    assert_eq!(out[1].triples.len(), 1);
    assert_eq!(out[1].triples[0].object.to_string(), "<http://ex/a>");
}
