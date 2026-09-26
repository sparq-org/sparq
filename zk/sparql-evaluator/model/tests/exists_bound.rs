// [GPT-6] Captured BOUND is an ambiguous 2013 shape, not a normative false Boolean.
#![cfg(feature = "evaluate")]
use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, Dialect, Policy, PrivateDataset, ProofContract, Request,
    VERSION, Witness, admit, dataset_commitment, evaluate,
};

#[test]
fn exists_bound_scope_preserves_local_variables() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/exists-bound-boundaries.json"
    ))
    .unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(
        cases.iter().filter(|case| case["admitted"] == true).count(),
        8
    );
    assert_eq!(
        cases
            .iter()
            .filter(|case| case["admitted"] == false)
            .count(),
        12
    );
    for case in cases {
        let dataset = PrivateDataset {
            ntriples: case["dataset_ntriples"].as_str().unwrap_or("").into(),
            salt: [41; 32],
        };
        let policy = Policy::default();
        let request = Request {
            version: VERSION,
            contract: ProofContract::ExactDataset,
            dialect: Dialect::SparqSparql11SnapshotV1,
            query: case["query"].as_str().unwrap().to_owned(),
            authority: DatasetAuthority::VerifierAgreed {
                commitment: dataset_commitment(&dataset, &policy).unwrap(),
            },
            policy,
            nonce: [43; 32],
        };
        if case["admitted"] == false {
            assert_eq!(
                admit(&request).unwrap_err().0,
                "captured BOUND is outside the SPARQL 1.1 substitution profile",
                "{}",
                case["id"]
            );
            assert_eq!(
                evaluate(&Witness { request, dataset }).unwrap_err().0,
                "captured BOUND is outside the SPARQL 1.1 substitution profile",
                "{}",
                case["id"]
            );
        } else {
            admit(&request).unwrap_or_else(|e| panic!("{}: {e:?}", case["id"]));
            let result = evaluate(&Witness { request, dataset }).unwrap().result;
            let CanonicalResult::Select { rows, .. } = result else {
                panic!("SELECT")
            };
            assert_eq!(
                serde_json::json!(rows),
                case["expected_rows"],
                "{}",
                case["id"]
            );
        }
    }
}

#[test]
fn native_practical_captured_bound_behavior_is_unchanged() {
    // This historical native behavior is not asserted as the 2013 normative rule.
    let graph = sparq_core::Graph::from_parts(sparq_core::dict::Dict::new(), vec![]);
    let result = sparq_engine::query(
        &graph,
        "SELECT ?n { VALUES ?n {1} FILTER EXISTS { FILTER(BOUND(?n)) } }",
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0][0].as_ref().unwrap().to_string(),
        "\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    );
}
