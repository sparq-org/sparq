//! sq-xqu (#2490): per-registered-query [`QueryBudget`] for
//! window evaluation. Each registered form applies its budget to EVERY window
//! (or tick) evaluation; a tripped budget surfaces as the evaluation error of
//! the `push`/`flush` that closed the window, and a generous budget changes
//! nothing.
//!
//! Determinism notes: the engine polls the budget at coarse sites (operator
//! entry, per outer scan/join iteration), so a pre-set cancel flag and a
//! `Duration::ZERO` per-window timeout (`Instant::now() >= deadline` at the
//! first poll) trip reliably; the row cap trips on any working set larger
//! than the cap. No test here depends on real elapsed time.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use oxrdf::{Literal, NamedNode, Term};
use sparq_rsp::{
    ContinuousAsk, ContinuousConstruct, ContinuousMultiQuery, ContinuousQuery, QueryBudget,
    WindowSpec,
};

/// `<http://ex/sN> <http://ex/value> vN` — one distinct row per `n`.
fn value_triple(n: i32) -> [Term; 3] {
    [
        NamedNode::new_unchecked(format!("http://ex/s{}", n)).into(),
        NamedNode::new_unchecked("http://ex/value").into(),
        Literal::from(n).into(),
    ]
}

fn row_cap(max_rows: usize) -> QueryBudget {
    QueryBudget { max_rows: Some(max_rows), ..QueryBudget::unlimited() }
}

#[test]
fn select_budget_max_rows_trips_on_window_close() {
    let mut q = ContinuousQuery::register(
        "SELECT ?s ?v WHERE { ?s <http://ex/value> ?v }",
        WindowSpec::time(10, 10),
    )
    .unwrap()
    .with_budget(row_cap(1));
    // Three rows buffered into [0,10) — under the cap nothing evaluates yet.
    for n in 0..3 {
        q.push(value_triple(n), n as u64, |_| panic!("no window closed yet")).unwrap();
    }
    // Closing the window evaluates 3 rows against max_rows=1: the budget trips
    // and the error is the push's evaluation error.
    let err = q.push(value_triple(9), 12, |_| panic!("budget must trip")).unwrap_err();
    assert!(err.contains("query budget exceeded (max-rows)"), "got: {}", err);
}

#[test]
fn select_generous_budget_matches_unbudgeted_results() {
    let run = |q: &mut ContinuousQuery| {
        let mut rows = Vec::new();
        for n in 0..3 {
            q.push(value_triple(n), n as u64, |_| {}).unwrap();
        }
        q.flush(|r| rows.extend(r.rows)).unwrap();
        rows
    };
    let sparql = "SELECT ?s ?v WHERE { ?s <http://ex/value> ?v }";
    let mut plain = ContinuousQuery::register(sparql, WindowSpec::time(10, 10)).unwrap();
    let mut budgeted = ContinuousQuery::register(sparql, WindowSpec::time(10, 10))
        .unwrap()
        .with_budget(row_cap(1_000_000));
    assert_eq!(run(&mut plain), run(&mut budgeted));
}

#[test]
fn select_window_timeout_bounds_each_evaluation() {
    let mut q = ContinuousQuery::register(
        "SELECT ?s ?v WHERE { ?s <http://ex/value> ?v }",
        WindowSpec::time(10, 10),
    )
    .unwrap()
    .with_window_timeout(Duration::ZERO);
    for n in 0..3 {
        q.push(value_triple(n), n as u64, |_| {}).unwrap();
    }
    let err = q.flush(|_| panic!("timeout must trip")).unwrap_err();
    assert!(err.contains("query budget exceeded (timeout)"), "got: {}", err);
}

#[test]
fn select_oversized_window_timeout_is_unlimited_not_a_panic() {
    // `now + Duration::MAX` is not representable as an `Instant`; the policy
    // is "a timeout that can never trip is unlimited", so push/flush must
    // evaluate normally instead of panicking on `Instant` overflow.
    let run = |q: &mut ContinuousQuery| {
        let mut rows = Vec::new();
        for n in 0..3 {
            q.push(value_triple(n), n as u64, |_| {}).unwrap();
        }
        q.flush(|r| rows.extend(r.rows)).unwrap();
        rows
    };
    let sparql = "SELECT ?s ?v WHERE { ?s <http://ex/value> ?v }";
    let mut plain = ContinuousQuery::register(sparql, WindowSpec::time(10, 10)).unwrap();
    let mut oversized = ContinuousQuery::register(sparql, WindowSpec::time(10, 10))
        .unwrap()
        .with_window_timeout(Duration::MAX);
    assert_eq!(run(&mut plain), run(&mut oversized));
}

#[test]
fn select_expired_absolute_deadline_not_extended_by_window_timeout() {
    // An already-passed ABSOLUTE deadline must keep failing every window even
    // when a generous per-window timeout is also set: the effective deadline
    // is the EARLIER of the two, so the per-window refresh can never grant
    // time past the absolute limit. `Instant::now()` taken at registration is
    // in the past by the time the window evaluates (same determinism as the
    // `Duration::ZERO` timeout tests: the first poll sees `now >= deadline`).
    let expired = QueryBudget {
        deadline: Some(std::time::Instant::now()),
        ..QueryBudget::unlimited()
    };
    let mut q = ContinuousQuery::register(
        "SELECT ?s ?v WHERE { ?s <http://ex/value> ?v }",
        WindowSpec::time(10, 10),
    )
    .unwrap()
    .with_budget(expired)
    .with_window_timeout(Duration::from_secs(3600));
    for n in 0..3 {
        q.push(value_triple(n), n as u64, |_| {}).unwrap();
    }
    let err = q.flush(|_| panic!("absolute deadline must trip")).unwrap_err();
    assert!(err.contains("query budget exceeded (timeout)"), "got: {}", err);
}

#[test]
fn select_zero_window_timeout_trips_despite_generous_absolute_deadline() {
    // The converse ordering: a far-future absolute deadline must not loosen a
    // `Duration::ZERO` per-window timeout — the earlier (window) deadline wins.
    let generous = QueryBudget {
        deadline: std::time::Instant::now().checked_add(Duration::from_secs(3600)),
        ..QueryBudget::unlimited()
    };
    let mut q = ContinuousQuery::register(
        "SELECT ?s ?v WHERE { ?s <http://ex/value> ?v }",
        WindowSpec::time(10, 10),
    )
    .unwrap()
    .with_budget(generous)
    .with_window_timeout(Duration::ZERO);
    for n in 0..3 {
        q.push(value_triple(n), n as u64, |_| {}).unwrap();
    }
    let err = q.flush(|_| panic!("window timeout must trip")).unwrap_err();
    assert!(err.contains("query budget exceeded (timeout)"), "got: {}", err);
}

#[test]
fn construct_budget_max_rows_trips() {
    let mut q = ContinuousConstruct::register(
        "CONSTRUCT { ?s <http://ex/observed> ?v } WHERE { ?s <http://ex/value> ?v }",
        WindowSpec::time(10, 10),
    )
    .unwrap()
    .with_budget(row_cap(1));
    for n in 0..3 {
        q.push(value_triple(n), n as u64, |_| {}).unwrap();
    }
    let err = q.flush(|_| panic!("budget must trip")).unwrap_err();
    assert!(err.contains("query budget exceeded (max-rows)"), "got: {}", err);
}

#[test]
fn construct_window_timeout_trips() {
    let mut q = ContinuousConstruct::register(
        "CONSTRUCT { ?s <http://ex/observed> ?v } WHERE { ?s <http://ex/value> ?v }",
        WindowSpec::time(10, 10),
    )
    .unwrap()
    .with_window_timeout(Duration::ZERO);
    q.push(value_triple(0), 0, |_| {}).unwrap();
    let err = q.flush(|_| panic!("timeout must trip")).unwrap_err();
    assert!(err.contains("query budget exceeded (timeout)"), "got: {}", err);
}

// A single-pattern ASK is answered straight from the index count, and tiny
// conjunctive shapes can complete their LIMIT-1 streaming scan before the
// first coarse poll — there is nothing to bound there. The ASK tests use a
// UNION shape, which enters the evaluator and polls at operator entry.
const ASK_UNION: &str =
    "ASK { { ?s <http://ex/value> ?v } UNION { ?s <http://ex/other> ?v } }";

#[test]
fn ask_budget_cancel_flag_aborts() {
    // ASK early-exits on the first solution, so the deterministic per-window
    // limit to exercise is the cooperative cancellation flag.
    let flag = Arc::new(AtomicBool::new(true));
    let mut q = ContinuousAsk::register(ASK_UNION, WindowSpec::time(10, 10))
        .unwrap()
        .with_budget(QueryBudget::cancelled_by(flag));
    q.push(value_triple(0), 0, |_| {}).unwrap();
    let err = q.flush(|_| panic!("cancel must trip")).unwrap_err();
    assert!(err.contains("query budget exceeded (cancelled)"), "got: {}", err);
}

#[test]
fn ask_window_timeout_trips() {
    let mut q = ContinuousAsk::register(ASK_UNION, WindowSpec::time(10, 10))
        .unwrap()
        .with_window_timeout(Duration::ZERO);
    q.push(value_triple(0), 0, |_| {}).unwrap();
    let err = q.flush(|_| panic!("timeout must trip")).unwrap_err();
    assert!(err.contains("query budget exceeded (timeout)"), "got: {}", err);
}

const MULTI_RSPQL: &str = "\
REGISTER STREAM <http://ex/out> AS
SELECT ?room ?v WHERE {
  WINDOW <http://ex/w1> { ?s <http://ex/value> ?v }
  WINDOW <http://ex/w2> { ?s <http://ex/in> ?room }
}
FROM NAMED WINDOW <http://ex/w1> ON <http://ex/temp> RANGE 10 STEP 10
FROM NAMED WINDOW <http://ex/w2> ON <http://ex/meta> RANGE 10 STEP 10";

#[test]
fn multi_budget_max_rows_trips_on_tick() {
    let mut q = ContinuousMultiQuery::register(MULTI_RSPQL).unwrap().with_budget(row_cap(1));
    let temp = NamedNode::new_unchecked("http://ex/temp");
    let meta = NamedNode::new_unchecked("http://ex/meta");
    let in_room = |s: &str| -> [Term; 3] {
        [
            NamedNode::new_unchecked(format!("http://ex/{}", s)).into(),
            NamedNode::new_unchecked("http://ex/in").into(),
            NamedNode::new_unchecked("http://ex/kitchen").into(),
        ]
    };
    let reading = |s: &str, v: i32| -> [Term; 3] {
        [
            NamedNode::new_unchecked(format!("http://ex/{}", s)).into(),
            NamedNode::new_unchecked("http://ex/value").into(),
            Literal::from(v).into(),
        ]
    };
    // Two joined rows in the tick's working set vs max_rows=1.
    q.push(&meta, in_room("s1"), 1, |_| {}).unwrap();
    q.push(&meta, in_room("s2"), 2, |_| {}).unwrap();
    q.push(&temp, reading("s1", 21), 3, |_| {}).unwrap();
    q.push(&temp, reading("s2", 22), 4, |_| {}).unwrap();
    let err = q.flush(|_| panic!("budget must trip")).unwrap_err();
    assert!(err.contains("query budget exceeded (max-rows)"), "got: {}", err);
}

#[test]
fn multi_window_timeout_trips_on_tick() {
    let mut q =
        ContinuousMultiQuery::register(MULTI_RSPQL).unwrap().with_window_timeout(Duration::ZERO);
    let meta = NamedNode::new_unchecked("http://ex/meta");
    let triple: [Term; 3] = [
        NamedNode::new_unchecked("http://ex/s1").into(),
        NamedNode::new_unchecked("http://ex/in").into(),
        NamedNode::new_unchecked("http://ex/kitchen").into(),
    ];
    q.push(&meta, triple, 1, |_| {}).unwrap();
    let err = q.flush(|_| panic!("timeout must trip")).unwrap_err();
    assert!(err.contains("query budget exceeded (timeout)"), "got: {}", err);
}
