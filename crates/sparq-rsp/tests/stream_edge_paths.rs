// sq-bif.20: Direct regressions for stream batching and error/late-arrival edges.

use oxrdf::{NamedNode, Term};
use sparq_rsp::{ContinuousQuery, TimestampedTriple, TripleStream, WindowSpec, WindowedStream};

fn triple(object: &str) -> [Term; 3] {
    [
        NamedNode::new_unchecked("http://example.com/subject").into(),
        NamedNode::new_unchecked("http://example.com/predicate").into(),
        NamedNode::new_unchecked(format!("http://example.com/{object}")).into(),
    ]
}

fn item(object: &str, ts: u64) -> TimestampedTriple {
    TimestampedTriple {
        triple: triple(object),
        ts,
    }
}

#[test]
fn push_batch_preserves_item_order_and_content() {
    let first = item("first", 9);
    let second = item("second", 3);
    let third = item("third", 7);
    let mut stream = TripleStream::new();

    stream.push_batch([first.clone(), second.clone(), third.clone()]);

    assert_eq!(
        stream.items(),
        [first.clone(), second.clone(), third.clone()]
    );
    assert_eq!(stream.into_items(), vec![first, second, third]);
}

#[test]
fn push_item_then_into_items_preserves_single_item() {
    let expected = item("only", 42);
    let mut stream = TripleStream::new();

    stream.push_item(expected.clone());

    assert_eq!(stream.into_items(), vec![expected]);
}

#[test]
fn continuous_query_register_reports_nonempty_malformed_query_error() {
    let result = ContinuousQuery::register("SELECT INVALID WHERE {", WindowSpec::time(10, 10));
    let Err(error) = result else {
        panic!("malformed SPARQL must fail during registration");
    };

    assert!(
        !error.trim().is_empty(),
        "registration error must explain the failure"
    );
}

#[test]
fn zero_delay_drops_arrival_older_than_the_first_open_window() {
    let mut stream = WindowedStream::empty(WindowSpec::time(10, 10));

    stream.push(triple("at-ten"), 10);
    stream.push(triple("late-five"), 5);
    stream.push(triple("at-fifteen"), 15);
    let closed = stream.flush();

    // The module-level docs in src/window.rs specify that initial-watermark windows are
    // skipped and an arrival whose last covering window has closed is counted as late.
    assert_eq!(stream.late_dropped(), 1);
    assert_eq!(closed.len(), 1);
    assert_eq!((closed[0].start, closed[0].end), (10, 20));
    assert_eq!(
        closed[0]
            .triples
            .iter()
            .map(|item| item.ts)
            .collect::<Vec<_>>(),
        [10, 15]
    );
}
