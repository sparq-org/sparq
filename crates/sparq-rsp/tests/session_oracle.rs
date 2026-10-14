#![cfg(feature = "session_windows")]

// sq-zckkq: deterministic, hand-derived session-window oracle.

use oxrdf::{NamedNode, Term};
use sparq_rsp::{ContinuousQuery, Window, WindowResult, WindowSpec, WindowedStream, R2S};

const GAP: u64 = 10;
const QUERY: &str = "SELECT ?s WHERE { ?s <http://example/p> <http://example/o> } ORDER BY ?s";

fn subject(name: &str) -> Term {
    NamedNode::new_unchecked(format!("http://example/{name}")).into()
}

fn triple(name: &str) -> [Term; 3] {
    [
        subject(name),
        NamedNode::new_unchecked("http://example/p").into(),
        NamedNode::new_unchecked("http://example/o").into(),
    ]
}

fn two_burst_script() -> Vec<([Term; 3], u64)> {
    vec![
        (triple("a"), 1),
        (triple("b"), 2),
        (triple("c"), 3),
        (triple("b"), 100),
        (triple("d"), 101),
    ]
}

fn collect_low_level(script: Vec<([Term; 3], u64)>) -> Vec<Window> {
    let mut stream = WindowedStream::empty(WindowSpec::session(GAP));
    let mut closed = Vec::new();
    for (triple, ts) in script {
        stream.push(triple, ts);
        closed.extend(stream.take_closed());
    }
    closed.extend(stream.flush());
    closed
}

fn collect_query(r2s: R2S) -> Vec<WindowResult> {
    let mut query = ContinuousQuery::register(QUERY, WindowSpec::session(GAP))
        .unwrap()
        .with_r2s(r2s);
    let mut results = Vec::new();
    for (triple, ts) in two_burst_script() {
        query
            .push(triple, ts, |result| results.push(result))
            .unwrap();
    }
    query.flush(|result| results.push(result)).unwrap();
    results
}

fn rows(names: &[&str]) -> Vec<Vec<Option<Term>>> {
    names.iter().map(|name| vec![Some(subject(name))]).collect()
}

#[test]
fn two_bursts_close_as_two_exact_inclusive_sessions() {
    let sessions = collect_low_level(two_burst_script());
    assert_eq!(sessions.len(), 2);

    assert_eq!((sessions[0].start, sessions[0].end), (1, 3));
    assert_eq!(
        sessions[0]
            .triples
            .iter()
            .map(|item| item.ts)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        sessions[0]
            .triples
            .iter()
            .map(|item| item.triple[0].clone())
            .collect::<Vec<_>>(),
        vec![subject("a"), subject("b"), subject("c")]
    );

    assert_eq!((sessions[1].start, sessions[1].end), (100, 101));
    assert_eq!(
        sessions[1]
            .triples
            .iter()
            .map(|item| item.ts)
            .collect::<Vec<_>>(),
        vec![100, 101]
    );
    assert_eq!(
        sessions[1]
            .triples
            .iter()
            .map(|item| item.triple[0].clone())
            .collect::<Vec<_>>(),
        vec![subject("b"), subject("d")]
    );
}

fn scalar_sessions(timestamps: &[u64]) -> Vec<Window<u64>> {
    let mut stream = WindowedStream::empty(WindowSpec::session(GAP));
    let mut closed = Vec::new();
    for &ts in timestamps {
        stream.push(ts, ts);
        closed.extend(stream.take_closed());
    }
    closed.extend(stream.flush());
    closed
}

#[test]
fn exact_gap_splits_but_gap_minus_one_extends() {
    let split = scalar_sessions(&[5, 15]);
    assert_eq!(split.len(), 2);
    assert_eq!((split[0].start, split[0].end), (5, 5));
    assert_eq!((split[1].start, split[1].end), (15, 15));
    assert_eq!(split[0].triples[0].triple, 5);
    assert_eq!(split[1].triples[0].triple, 15);

    let joined = scalar_sessions(&[5, 14]);
    assert_eq!(joined.len(), 1);
    assert_eq!((joined[0].start, joined[0].end), (5, 14));
    assert_eq!(
        joined[0]
            .triples
            .iter()
            .map(|item| item.triple)
            .collect::<Vec<_>>(),
        vec![5, 14]
    );
}

#[test]
fn heartbeat_closes_at_the_exact_inactivity_deadline() {
    let mut stream = WindowedStream::empty(WindowSpec::session(GAP));
    stream.push(5_u64, 5);
    stream.advance(14);
    assert!(stream.take_closed().is_empty());

    stream.advance(15);
    let closed = stream.take_closed();
    assert_eq!(closed.len(), 1);
    assert_eq!((closed[0].start, closed[0].end), (5, 5));
    assert!(stream.flush().is_empty());
}

#[test]
fn r2s_operators_emit_exact_hand_derived_multiset_diffs() {
    let rstream = collect_query(R2S::RStream);
    assert_eq!(rstream.len(), 2);
    assert_eq!((rstream[0].start, rstream[0].end), (1, 3));
    assert_eq!(rstream[0].rows, rows(&["a", "b", "c"]));
    assert_eq!((rstream[1].start, rstream[1].end), (100, 101));
    assert_eq!(rstream[1].rows, rows(&["b", "d"]));

    let istream = collect_query(R2S::IStream);
    assert_eq!(istream.len(), 2);
    assert_eq!(istream[0].rows, rows(&["a", "b", "c"]));
    assert_eq!(istream[1].rows, rows(&["d"]));

    let dstream = collect_query(R2S::DStream);
    assert_eq!(dstream.len(), 2);
    assert!(dstream[0].rows.is_empty());
    assert_eq!(dstream[1].rows, rows(&["a", "c"]));
}

#[test]
fn out_of_order_events_join_only_a_still_open_session() {
    let mut stream = WindowedStream::empty(WindowSpec::session(GAP));
    stream.push(100_u64, 100);
    stream.push(95, 95);
    stream.push(50, 50);
    assert_eq!(stream.late_dropped(), 1);

    let sessions = stream.flush();
    assert_eq!(sessions.len(), 1);
    assert_eq!((sessions[0].start, sessions[0].end), (95, 100));
    assert_eq!(
        sessions[0]
            .triples
            .iter()
            .map(|item| item.triple)
            .collect::<Vec<_>>(),
        vec![95, 100]
    );
}

#[test]
fn flushing_an_empty_stream_does_not_make_timestamp_zero_late() {
    let mut stream = WindowedStream::empty(WindowSpec::session(GAP));
    assert!(stream.flush().is_empty());

    stream.push(0_u64, 0);
    assert_eq!(stream.late_dropped(), 0);
    let sessions = stream.flush();
    assert_eq!(sessions.len(), 1);
    assert_eq!((sessions[0].start, sessions[0].end), (0, 0));
}

#[test]
#[should_panic(expected = "session window GAP must be > 0")]
fn zero_gap_is_rejected() {
    let _ = WindowSpec::session(0);
}
