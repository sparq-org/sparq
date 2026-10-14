//! Multi-window joins (sq-9u1, part 2): an RSP-QL query opening more
//! than one named window and JOINing across them, evaluated correctly over the
//! per-window contents at each synchronized evaluation tick.
//! sq-2n1q3.3: 3+-window join correctness, the explicit
//! no-output-until-all-windows-populated property test, and ISTREAM/DSTREAM
//! over multi-window joins.

use oxrdf::{Literal, NamedNode, Term};
use sparq_rsp::{ContinuousMultiQuery, WindowResult};

fn nn(s: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("http://ex/{s}"))
}

fn iri(s: &str) -> Term {
    nn(s).into()
}

/// `<http://ex/{s}> <http://ex/{p}> {o}`
fn t(s: &str, p: &str, o: Term) -> [Term; 3] {
    [iri(s), iri(p), o]
}

const Q_JOIN: &str = "\
REGISTER STREAM <http://ex/out> AS
SELECT ?room ?v WHERE {
  WINDOW <http://ex/w1> { ?s <http://ex/value> ?v }
  WINDOW <http://ex/w2> { ?s <http://ex/in> ?room }
}
FROM NAMED WINDOW <http://ex/w1> ON <http://ex/temp> RANGE 10 STEP 10
FROM NAMED WINDOW <http://ex/w2> ON <http://ex/meta> RANGE 10 STEP 10";

/// A row `(room, v)` as the join would emit it.
fn row(room: &str, v: i32) -> Vec<Option<Term>> {
    vec![Some(iri(room)), Some(Literal::from(v).into())]
}

/// THE multi-window join: a sensor's READINGS on stream :temp (window w1) joined
/// with the ROOM that sensor is in on stream :meta (window w2), per synchronized
/// tumbling window. Pins correct results over a synthetic 2-stream input.
#[test]
fn multi_window_join_across_two_streams() {
    let mut q = ContinuousMultiQuery::register(Q_JOIN).unwrap();
    assert_eq!(q.window_iris().len(), 2);
    assert_eq!(q.output_stream().unwrap().as_str(), "http://ex/out");

    let temp = nn("temp");
    let meta = nn("meta");
    let mut results: Vec<WindowResult> = Vec::new();

    // --- window [0,10) ---
    // meta: s1 is in the kitchen, s2 is in the hall.
    q.push(&meta, t("s1", "in", iri("kitchen")), 1, |r| results.push(r)).unwrap();
    q.push(&meta, t("s2", "in", iri("hall")), 2, |r| results.push(r)).unwrap();
    // temp: s1 reads 21 and 23; s2 reads 30.
    q.push(&temp, t("s1", "value", Literal::from(21).into()), 3, |r| results.push(r)).unwrap();
    q.push(&temp, t("s1", "value", Literal::from(23).into()), 4, |r| results.push(r)).unwrap();
    q.push(&temp, t("s2", "value", Literal::from(30).into()), 5, |r| results.push(r)).unwrap();

    // A temp reading at ts 12 advances the SHARED clock to 12, closing [0,10) on
    // BOTH windows even though no meta arrived after ts 2.
    q.push(&temp, t("s1", "value", Literal::from(99).into()), 12, |r| results.push(r)).unwrap();

    assert_eq!(results.len(), 1, "one synchronized tick closed [0,10)");
    let mut got: Vec<_> = results[0].rows.clone();
    got.sort_by_key(|r| format!("{r:?}"));
    let mut want = vec![row("kitchen", 21), row("kitchen", 23), row("hall", 30)];
    want.sort_by_key(|r| format!("{r:?}"));
    assert_eq!(
        got, want,
        "each reading joins with its sensor's room ACROSS the two windows"
    );
    assert_eq!(
        results[0].vars.iter().map(|v| v.as_str()).collect::<Vec<_>>(),
        ["room", "v"]
    );

    // --- flush: window [10,20) holds only the ts-12 reading; no meta in it, so
    // the join produces NOTHING (w2's window is empty → GRAPH yields no rows). ---
    results.clear();
    q.flush(|r| results.push(r)).unwrap();
    let total_rows: usize = results.iter().map(|r| r.rows.len()).sum();
    assert_eq!(total_rows, 0, "no meta in [10,20): the join is empty");
}

/// Sanity: the join is genuinely a CROSS-WINDOW join, not a per-window union.
/// A reading whose sensor has NO room in the meta window does not join; a room
/// whose sensor has NO reading does not join.
#[test]
fn multi_window_join_only_matches_shared_subject() {
    let mut q = ContinuousMultiQuery::register(Q_JOIN).unwrap();
    let temp = nn("temp");
    let meta = nn("meta");
    let mut results: Vec<WindowResult> = Vec::new();

    // s1 has a room but no reading; s2 has a reading but no room.
    q.push(&meta, t("s1", "in", iri("kitchen")), 1, |r| results.push(r)).unwrap();
    q.push(&temp, t("s2", "value", Literal::from(30).into()), 2, |r| results.push(r)).unwrap();
    q.flush(|r| results.push(r)).unwrap();

    let total_rows: usize = results.iter().map(|r| r.rows.len()).sum();
    assert_eq!(total_rows, 0, "no shared subject → empty join");
}

/// Two windows of DIFFERENT step over the same conceptual time: a slow window's
/// content is held steady across the fast window's ticks (the snapshot rule).
#[test]
fn multi_window_join_with_differing_steps() {
    // w1 (readings) tumbles every 10; w2 (rooms) tumbles every 20, so w2's
    // content spans two of w1's ticks.
    let q_text = "\
REGISTER STREAM <http://ex/out> AS
SELECT ?room ?v WHERE {
  WINDOW <http://ex/w1> { ?s <http://ex/value> ?v }
  WINDOW <http://ex/w2> { ?s <http://ex/in> ?room }
}
FROM NAMED WINDOW <http://ex/w1> ON <http://ex/temp> RANGE 10 STEP 10
FROM NAMED WINDOW <http://ex/w2> ON <http://ex/meta> RANGE 20 STEP 20";
    let mut q = ContinuousMultiQuery::register(q_text).unwrap();
    let temp = nn("temp");
    let meta = nn("meta");
    let mut results: Vec<WindowResult> = Vec::new();

    // meta: s1 in kitchen at ts 1 (lands in w2's [0,20)).
    q.push(&meta, t("s1", "in", iri("kitchen")), 1, |r| results.push(r)).unwrap();
    // temp: s1 reads 21 at ts 2 (w1's [0,10)) and 23 at ts 13 (w1's [10,20)).
    q.push(&temp, t("s1", "value", Literal::from(21).into()), 2, |r| results.push(r)).unwrap();
    q.push(&temp, t("s1", "value", Literal::from(23).into()), 13, |r| results.push(r)).unwrap();
    // Advance the shared clock to 22: closes w1's [0,10) AND [10,20), and w2's [0,20).
    q.push(&temp, t("s1", "value", Literal::from(99).into()), 22, |r| results.push(r)).unwrap();

    // Collect all emitted rows tagged with their tick end.
    let mut by_tick: Vec<(u64, Vec<Vec<Option<Term>>>)> =
        results.iter().map(|r| (r.end, r.rows.clone())).collect();
    by_tick.sort_by_key(|(e, _)| *e);

    // Tick at end=10: w1 has reading 21; w2 (slow, not yet closed at boundary 10)
    // has NO closed content → empty named graph → no join rows.
    // Tick at end=20: w1 has reading 23 ([10,20)); w2's [0,20) closed with the
    // kitchen fact → join: (kitchen, 23).
    let tick20 = by_tick.iter().find(|(e, _)| *e == 20).map(|(_, r)| r.clone());
    assert_eq!(tick20, Some(vec![row("kitchen", 23)]), "w2 content joins w1's [10,20)");

    // The end=10 tick must NOT have a join row (w2 had no closed content yet).
    let tick10 = by_tick.iter().find(|(e, _)| *e == 10).map(|(_, r)| r.clone());
    assert_eq!(tick10, Some(vec![]), "slow window not closed at boundary 10 → empty join");
}

// ==================== sq-2n1q3.3: new tests below ====================

/// 3-window RSP-QL query text: observation stream (w1) joined with type stream
/// (w2) joined with location stream (w3).  All three windows must be populated
/// for any row to appear; a missing window contributes an empty GRAPH pattern
/// that matches nothing, collapsing the join to 0 rows.
const Q_THREE: &str = "\
REGISTER STREAM <http://ex/out> AS
SELECT ?type ?loc ?v WHERE {
  WINDOW <http://ex/w1> { ?s <http://ex/value> ?v }
  WINDOW <http://ex/w2> { ?s <http://ex/type> ?type }
  WINDOW <http://ex/w3> { ?s <http://ex/loc> ?loc }
}
FROM NAMED WINDOW <http://ex/w1> ON <http://ex/obs>  RANGE 10 STEP 10
FROM NAMED WINDOW <http://ex/w2> ON <http://ex/meta1> RANGE 10 STEP 10
FROM NAMED WINDOW <http://ex/w3> ON <http://ex/meta2> RANGE 10 STEP 10";

/// A `(type, loc, v)` join row as the 3-window query emits it.
fn row3(typ: &str, loc: &str, v: i32) -> Vec<Option<Term>> {
    vec![Some(iri(typ)), Some(iri(loc)), Some(Literal::from(v).into())]
}

/// sq-2n1q3.3 — EXPLICIT non-vacuous property test.
///
/// Three windows are declared.  Data is pushed to w1 (obs) and w2 (meta1) but
/// NOT to w3 (meta2).  When the shared clock closes [0,10) on all three
/// windows, the join must produce ZERO rows because w3 holds no content.
///
/// NON-VACUOUSNESS: if the implementation violated the invariant — treating an
/// unpopulated window as "unconstrained" rather than contributing an empty
/// GRAPH — sensor s1 would match across w1 and w2 and the result would be 1
/// row (the join of value 42 with type "sensorA").  The assertion below would
/// then fail, making this test's falsification explicit.  The test is therefore
/// a sound guard on the no-output-until-every-joined-window-has-content
/// property.
#[test]
fn no_output_until_all_windows_populated_is_enforced() {
    let mut q = ContinuousMultiQuery::register(Q_THREE).unwrap();
    assert_eq!(q.window_iris().len(), 3, "three windows registered");

    let obs   = nn("obs");
    let meta1 = nn("meta1");
    // meta2 is deliberately never pushed to.
    let mut results: Vec<WindowResult> = Vec::new();

    // Push s1's VALUE into obs (w1) and its TYPE into meta1 (w2).
    q.push(&obs,   t("s1", "value", Literal::from(42).into()), 1, |r| results.push(r)).unwrap();
    q.push(&meta1, t("s1", "type",  iri("sensorA")),            2, |r| results.push(r)).unwrap();

    // Advance the shared clock past [0,10) — meta2 (w3) receives NO push, so
    // its w3 window has no content even when [0,10) closes on all streams.
    q.push(&obs, t("s1", "value", Literal::from(0).into()), 12, |r| results.push(r)).unwrap();

    // Exactly one synchronized tick fires, but it must produce 0 rows because
    // w3 is empty — its GRAPH pattern matches nothing, collapsing the join.
    // (If the invariant were violated the result would be 1 row.)
    assert_eq!(results.len(), 1, "one synchronized tick fired");
    assert_eq!(
        results[0].rows.len(),
        0,
        "unpopulated w3 makes the 3-window join empty; \
         1 row here means the no-output-until-all-windows-populated invariant is broken"
    );
}

/// sq-2n1q3.3 — 3-window join correctness.
///
/// All three windows are populated.  The join produces one row per sensor that
/// is present in all three windows' content.
#[test]
fn three_window_join_correctness() {
    let mut q = ContinuousMultiQuery::register(Q_THREE).unwrap();

    let obs   = nn("obs");
    let meta1 = nn("meta1");
    let meta2 = nn("meta2");
    let mut results: Vec<WindowResult> = Vec::new();

    // Window [0,10): two sensors in all three windows.
    // s1: value 42, type sensorA, location "london"
    // s2: value 99, type sensorB, location "paris"
    q.push(&obs,   t("s1", "value", Literal::from(42).into()), 1, |r| results.push(r)).unwrap();
    q.push(&obs,   t("s2", "value", Literal::from(99).into()), 2, |r| results.push(r)).unwrap();
    q.push(&meta1, t("s1", "type",  iri("sensorA")),            3, |r| results.push(r)).unwrap();
    q.push(&meta1, t("s2", "type",  iri("sensorB")),            4, |r| results.push(r)).unwrap();
    q.push(&meta2, t("s1", "loc",   iri("london")),             5, |r| results.push(r)).unwrap();
    q.push(&meta2, t("s2", "loc",   iri("paris")),              6, |r| results.push(r)).unwrap();

    // Advance the clock to close [0,10) on all streams.
    q.push(&obs, t("s1", "value", Literal::from(0).into()), 12, |r| results.push(r)).unwrap();

    assert_eq!(results.len(), 1, "one synchronized tick for window [0,10)");
    let mut got = results[0].rows.clone();
    got.sort_by_key(|r| format!("{r:?}"));
    let mut want = vec![row3("sensorA", "london", 42), row3("sensorB", "paris", 99)];
    want.sort_by_key(|r| format!("{r:?}"));
    assert_eq!(got, want, "both sensors join correctly across all three windows");
    assert_eq!(q.window_iris().len(), 3);

    // Flush: w1 has the ts-12 heartbeat, w2 and w3 have nothing in [10,20).
    // The join must be empty (same no-output-until-all-populated invariant).
    results.clear();
    q.flush(|r| results.push(r)).unwrap();
    let total: usize = results.iter().map(|r| r.rows.len()).sum();
    assert_eq!(total, 0, "missing w2/w3 content in [10,20) → empty 3-window join");
}

/// sq-2n1q3.3 — ISTREAM over a multi-window join.
///
/// ISTREAM emits only rows that APPEARED in the join result relative to the
/// previous tick.  The first tick diffs against the empty multiset and emits
/// everything; subsequent ticks emit only the delta.
#[test]
fn istream_over_multi_window_join() {
    // Two-window query with ISTREAM.  Uses two tumbling windows of step 10.
    let q_istream = "\
REGISTER ISTREAM <http://ex/out> AS
SELECT ?room ?v WHERE {
  WINDOW <http://ex/w1> { ?s <http://ex/value> ?v }
  WINDOW <http://ex/w2> { ?s <http://ex/in> ?room }
}
FROM NAMED WINDOW <http://ex/w1> ON <http://ex/temp> RANGE 10 STEP 10
FROM NAMED WINDOW <http://ex/w2> ON <http://ex/meta> RANGE 10 STEP 10";

    let mut q = ContinuousMultiQuery::register(q_istream).unwrap();
    assert!(matches!(q.r2s(), sparq_rsp::R2S::IStream), "parsed ISTREAM header");

    let temp = nn("temp");
    let meta = nn("meta");
    let mut results: Vec<WindowResult> = Vec::new();

    // Window [0,10): s1 in kitchen with value 10.
    q.push(&meta, t("s1", "in",    iri("kitchen")),             1, |r| results.push(r)).unwrap();
    q.push(&temp, t("s1", "value", Literal::from(10).into()),   2, |r| results.push(r)).unwrap();
    // Advance clock to 12: closes [0,10) on both windows.
    q.push(&temp, t("s1", "value", Literal::from(20).into()),   12, |r| results.push(r)).unwrap();

    // Window [10,20): s1 still in kitchen (re-announced in [10,20) meta),
    // value 20 (already pushed at ts 12) and 30 (pushed below).
    q.push(&meta, t("s1", "in", iri("kitchen")),                13, |r| results.push(r)).unwrap();
    q.push(&temp, t("s1", "value", Literal::from(30).into()),   14, |r| results.push(r)).unwrap();
    // Advance clock to 22: closes [10,20).
    q.push(&temp, t("s1", "value", Literal::from(0).into()),    22, |r| results.push(r)).unwrap();

    assert_eq!(results.len(), 2, "two ticks fired (one per tumbling window)");

    // RSTREAM for [0,10) would be: [(kitchen, 10)].
    // ISTREAM tick 0: prev = {}, cur = {(kitchen,10)} → emit {(kitchen,10)}.
    let tick0_rows = results[0].rows.len();
    assert_eq!(tick0_rows, 1, "ISTREAM tick 0: (kitchen,10) is new vs empty prev");
    assert_eq!(results[0].rows[0], vec![Some(iri("kitchen")), Some(Literal::from(10).into())]);

    // RSTREAM for [10,20) would be: [(kitchen,20), (kitchen,30)].
    // ISTREAM tick 1: prev={(kitchen,10)}, cur={(kitchen,20),(kitchen,30)}.
    // cur ∖ prev = {(kitchen,20),(kitchen,30)} (values differ, so both are "new").
    let tick1_rows = results[1].rows.len();
    assert_eq!(tick1_rows, 2, "ISTREAM tick 1: both new rows vs (kitchen,10) prev");
}

/// sq-2n1q3.3 — DSTREAM over a multi-window join.
///
/// DSTREAM emits only rows that DISAPPEARED from the join result relative to the
/// previous tick.  The first tick always emits nothing (nothing can disappear
/// from an empty previous result).
#[test]
fn dstream_over_multi_window_join() {
    let q_dstream = "\
REGISTER DSTREAM <http://ex/out> AS
SELECT ?room ?v WHERE {
  WINDOW <http://ex/w1> { ?s <http://ex/value> ?v }
  WINDOW <http://ex/w2> { ?s <http://ex/in> ?room }
}
FROM NAMED WINDOW <http://ex/w1> ON <http://ex/temp> RANGE 10 STEP 10
FROM NAMED WINDOW <http://ex/w2> ON <http://ex/meta> RANGE 10 STEP 10";

    let mut q = ContinuousMultiQuery::register(q_dstream).unwrap();
    assert!(matches!(q.r2s(), sparq_rsp::R2S::DStream), "parsed DSTREAM header");

    let temp = nn("temp");
    let meta = nn("meta");
    let mut results: Vec<WindowResult> = Vec::new();

    // Window [0,10): s1 in kitchen with values 10 and 20.
    q.push(&meta, t("s1", "in",    iri("kitchen")),             1, |r| results.push(r)).unwrap();
    q.push(&temp, t("s1", "value", Literal::from(10).into()),   2, |r| results.push(r)).unwrap();
    q.push(&temp, t("s1", "value", Literal::from(20).into()),   3, |r| results.push(r)).unwrap();
    // Advance clock to 12: closes [0,10).
    q.push(&temp, t("s1", "value", Literal::from(0).into()),    12, |r| results.push(r)).unwrap();

    // Window [10,20): no meta for s1, so the join is empty.
    q.push(&temp, t("s1", "value", Literal::from(99).into()),   22, |r| results.push(r)).unwrap();

    assert_eq!(results.len(), 2, "two ticks fired");

    // DSTREAM tick 0: prev = {} (empty), so nothing disappears.
    assert_eq!(
        results[0].rows.len(), 0,
        "DSTREAM tick 0: first tick never emits anything (prev was empty)"
    );

    // DSTREAM tick 1: prev={(kitchen,10),(kitchen,20)}, cur={} (no meta in [10,20)).
    // prev ∖ cur = both rows from tick 0.
    assert_eq!(
        results[1].rows.len(), 2,
        "DSTREAM tick 1: both rows from tick 0 disappeared (no meta in [10,20))"
    );
}

/// Registration errors: a single-window RSP-QL query (use ContinuousQuery), a
/// WHERE WINDOW with no declaration, and a non-SELECT body.
#[test]
fn multi_query_registration_validates() {
    // Only one window declared.
    let one = "SELECT * WHERE { WINDOW <http://ex/w> { ?s ?p ?o } }\n\
               FROM NAMED WINDOW <http://ex/w> ON <http://ex/s> RANGE 10";
    assert!(ContinuousMultiQuery::register(one).is_err());

    // WHERE references an undeclared window.
    let undeclared = "SELECT * WHERE {\n\
                        WINDOW <http://ex/w1> { ?s ?p ?o }\n\
                        WINDOW <http://ex/wX> { ?s ?q ?r }\n\
                      }\n\
                      FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> RANGE 10\n\
                      FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 10";
    assert!(ContinuousMultiQuery::register(undeclared).is_err());

    // Non-SELECT.
    let ask = "ASK { WINDOW <http://ex/w1> { ?s ?p ?o } WINDOW <http://ex/w2> { ?s ?q ?r } }\n\
               FROM NAMED WINDOW <http://ex/w1> ON <http://ex/s1> RANGE 10\n\
               FROM NAMED WINDOW <http://ex/w2> ON <http://ex/s2> RANGE 10";
    assert!(ContinuousMultiQuery::register(ask).is_err());
}
