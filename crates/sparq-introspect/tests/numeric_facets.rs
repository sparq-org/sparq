#![cfg(feature = "numeric-facets")]

use sparq_core::Graph;
use sparq_introspect::{facets, FacetRequest, FacetResponse};

const AGE: &str = "http://example.com/age";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

fn requested_age() -> FacetRequest {
    FacetRequest {
        facet_predicates: Some(vec![AGE.into()]),
        top_k: usize::MAX,
        ..FacetRequest::default()
    }
}

#[test]
fn numeric_range_and_ten_bucket_oracle_are_exact() {
    // Mutation witness for sq-lsp7k.29: removing numeric collection or
    // changing the bucket assignment makes the exact vector below fail.
    let graph = Graph::load_str(
        &format!(
            "<http://e/a> <{AGE}> \"10\"^^<{XSD}integer> .\n\
             <http://e/b> <{AGE}> \"20\"^^<{XSD}decimal> .\n\
             <http://e/c> <{AGE}> \"30\"^^<{XSD}double> .\n\
             <http://e/d> <{AGE}> \"40\"^^<{XSD}float> .\n"
        ),
        "ntriples",
    )
    .unwrap();

    let response = facets(&graph, &requested_age());
    assert_eq!(response.numeric.len(), 1);
    let summary = &response.numeric[0];
    assert_eq!(summary.predicate, AGE);
    assert_eq!((summary.min, summary.max, summary.count), (10.0, 40.0, 4));
    assert_eq!(summary.buckets.len(), 10);
    assert_eq!(
        summary
            .buckets
            .iter()
            .map(|bucket| (bucket.lo, bucket.hi, bucket.count))
            .collect::<Vec<_>>(),
        vec![
            (10.0, 13.0, 1),
            (13.0, 16.0, 0),
            (16.0, 19.0, 0),
            (19.0, 22.0, 1),
            (22.0, 25.0, 0),
            (25.0, 28.0, 0),
            (28.0, 31.0, 1),
            (31.0, 34.0, 0),
            (34.0, 37.0, 0),
            (37.0, 40.0, 1),
        ]
    );
    assert_eq!(
        summary
            .buckets
            .iter()
            .map(|bucket| bucket.count)
            .sum::<u64>(),
        4
    );

    let json = response.to_json();
    let decoded: FacetResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, response);
}

#[test]
fn mixed_values_exclude_non_numeric_objects_from_the_summary() {
    let graph = Graph::load_str(
        &format!(
            "<http://e/a> <{AGE}> \"7\"^^<{XSD}int> .\n\
             <http://e/b> <{AGE}> \"eleven\" .\n\
             <http://e/c> <{AGE}> \"11.5\"^^<{XSD}decimal> .\n\
             <http://e/d> <{AGE}> <http://e/unknown> .\n"
        ),
        "ntriples",
    )
    .unwrap();

    let response = facets(&graph, &requested_age());
    assert_eq!(
        response.values[0]
            .values
            .iter()
            .map(|v| v.count)
            .sum::<u64>(),
        4
    );
    let summary = &response.numeric[0];
    assert_eq!((summary.min, summary.max, summary.count), (7.0, 11.5, 2));
    assert_eq!(
        summary
            .buckets
            .iter()
            .map(|bucket| bucket.count)
            .sum::<u64>(),
        2
    );
}

#[test]
fn non_numeric_only_predicate_has_no_numeric_summary() {
    let graph = Graph::load_str(
        &format!(
            "<http://e/a> <{AGE}> \"unknown\" .\n\
             <http://e/b> <{AGE}> <http://e/not-a-number> .\n"
        ),
        "ntriples",
    )
    .unwrap();

    let response = facets(&graph, &requested_age());
    assert_eq!(response.values[0].values.len(), 2);
    assert!(response.numeric.is_empty());
}

#[test]
fn constant_numeric_facet_uses_the_closed_final_bucket() {
    let graph = Graph::load_str(
        &format!(
            "<http://e/a> <{AGE}> \"5\"^^<{XSD}integer> .\n\
             <http://e/b> <{AGE}> \"5.0\"^^<{XSD}decimal> .\n"
        ),
        "ntriples",
    )
    .unwrap();

    let response = facets(&graph, &requested_age());
    let summary = &response.numeric[0];
    assert_eq!((summary.min, summary.max, summary.count), (5.0, 5.0, 2));
    assert!(summary.buckets[..9].iter().all(|bucket| bucket.count == 0));
    assert_eq!(summary.buckets[9].count, 2);
}
