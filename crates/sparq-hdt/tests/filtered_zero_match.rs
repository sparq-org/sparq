//! sq-bif.17: fully-bound zero-match and wildcard fast-path coverage
//! for the opt-in HDT load filter.

#![cfg(feature = "load-filter")]

use oxrdf::{NamedNode, NamedOrBlankNode, Term};
use sparq_hdt::{HdtStats, TriplePattern};
use std::io::Cursor;

fn fixture_bytes() -> Vec<u8> {
    include_bytes!("fixtures/snikmeta.hdt").to_vec()
}

fn zero_match_pattern() -> TriplePattern {
    let subject = NamedOrBlankNode::from(
        NamedNode::new("http://www.snik.eu/ontology/meta/ApplicationComponent")
            .expect("valid subject IRI"),
    );
    let predicate =
        NamedNode::new("http://www.w3.org/2000/01/rdf-schema#label").expect("valid predicate IRI");
    let object = Term::from(
        NamedNode::new("http://www.w3.org/2002/07/owl#Class").expect("valid object IRI"),
    );
    (Some(subject), Some(predicate), Some(object))
}

#[test]
fn fully_bound_zero_match_returns_empty_graph_and_archive_wide_stats() {
    let bytes = fixture_bytes();
    let pattern = zero_match_pattern();
    let graph = sparq_hdt::load_reader_filtered(Cursor::new(bytes.clone()), &pattern)
        .expect("loading a zero-match filter");
    let stats = sparq_hdt::stats_reader_filtered(Cursor::new(bytes), &pattern)
        .expect("counting a zero-match filter");

    assert!(
        graph.is_empty(),
        "the fully-bound pattern must match no triples"
    );
    assert_eq!(graph.dict.len(), 0, "an empty result interns no terms");
    assert_eq!(
        stats,
        HdtStats {
            triples: 0,
            shared: 43,
            subjects_only: 6,
            objects_only: 133,
            predicates: 23,
        },
        "filtered stats retain the complete archive's dictionary cardinalities"
    );
}

#[test]
fn all_wildcard_stats_equal_unfiltered_stats() {
    let bytes = fixture_bytes();
    let expected = sparq_hdt::stats_reader(Cursor::new(bytes.clone()))
        .expect("reading unfiltered fixture stats");
    let actual = sparq_hdt::stats_reader_filtered(Cursor::new(bytes), &(None, None, None))
        .expect("reading all-wildcard fixture stats");

    assert_eq!(
        expected,
        HdtStats {
            triples: 328,
            shared: 43,
            subjects_only: 6,
            objects_only: 133,
            predicates: 23,
        },
        "fixture statistics changed"
    );
    assert_eq!(
        actual, expected,
        "all wildcards must use stats_reader semantics"
    );
}
