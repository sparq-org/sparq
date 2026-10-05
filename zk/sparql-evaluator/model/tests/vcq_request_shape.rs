// [OPUS-5.5] Native actual-form classification used by the vcq adapter; no proofs.
#![cfg(feature = "graph-results")]

use sparq_proved_evaluator_model::v3::{QueryShape, ShapeError, query_shape};
use sparq_proved_evaluator_model::{MAX_QUERY_BYTES, Rejected};

#[test]
fn reports_the_parsed_form_not_a_label() {
    for (query, shape) in [
        ("SELECT ?s WHERE { ?s ?p ?o }", QueryShape::SelectBag),
        ("SELECT DISTINCT ?s WHERE { ?s ?p ?o } LIMIT 3", QueryShape::SelectBag),
        ("SELECT ?s WHERE { ?s ?p ?o } ORDER BY ?s", QueryShape::SelectSequence),
        ("SELECT ?s WHERE { ?s ?p ?o } ORDER BY ?s LIMIT 1", QueryShape::SelectSequence),
        // Subquery order does not order the outer SELECT result.
        (
            "SELECT ?s WHERE { { SELECT ?s WHERE { ?s ?p ?o } ORDER BY ?s } }",
            QueryShape::SelectBag,
        ),
        ("ASK { ?s ?p ?o }", QueryShape::Ask),
        ("CONSTRUCT { ?s <http://ex/q> ?o } WHERE { ?s ?p ?o }", QueryShape::Construct),
        ("DESCRIBE ?s WHERE { ?s ?p ?o }", QueryShape::Describe),
    ] {
        assert_eq!(query_shape(query), Ok(shape), "{query}");
    }
}

#[test]
fn rejects_before_or_during_admission_with_typed_causes() {
    assert_eq!(query_shape("SELECT WHERE"), Err(ShapeError::Parse));
    let long = format!("ASK {{ }}{}", " ".repeat(MAX_QUERY_BYTES));
    assert_eq!(
        query_shape(&long),
        Err(ShapeError::QueryBytes { len: long.len() })
    );
    assert_eq!(
        query_shape("SELECT * WHERE { SERVICE <http://ex/s> { ?s ?p ?o } }"),
        Err(ShapeError::NotAdmitted(Rejected(
            "GRAPH, SERVICE and LATERAL are not admitted"
        )))
    );
}

#[test]
fn agrees_with_v3_admission_on_accepted_queries() {
    use sparq_proved_evaluator_model::{DatasetAuthority, ProofContract, v3};
    for query in [
        "SELECT ?s WHERE { ?s ?p ?o }",
        "ASK { ?s ?p ?o }",
        "CONSTRUCT { ?s <http://ex/q> ?o } WHERE { ?s ?p ?o }",
    ] {
        let request = v3::Request {
            version: v3::VERSION,
            contract: ProofContract::ExactDataset,
            dialect: v3::Dialect::SparqSparql11GraphResultsV3,
            query: query.to_owned(),
            authority: DatasetAuthority::HolderDeclared,
            policy: v3::Policy::default(),
            nonce: [7; 32],
        };
        assert_eq!(v3::admit(&request), Ok(()), "{query}");
        assert!(query_shape(query).is_ok(), "{query}");
    }
}
