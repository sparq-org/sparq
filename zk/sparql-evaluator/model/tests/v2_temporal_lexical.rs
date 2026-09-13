// [GPT-6] Native profile admission/evaluation, separate from guest proof evidence.
#![cfg(feature = "evaluate")]

use sparq_proved_evaluator_model::v2::{
    Dialect, Policy, PrivateDataset, Request, VERSION, Witness, admit, dataset_commitment, evaluate,
};
use sparq_proved_evaluator_model::{CanonicalResult, DatasetAuthority, ProofContract};

#[test]
fn v2_temporal_lexical_boundaries_are_evaluated_by_the_native_model() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../crates/sparq-engine/tests/fixtures/temporal_lexical.json"
    ))
    .unwrap();
    let policy = Policy::default();
    let mut failures = Vec::new();
    for case in corpus["cases"].as_array().unwrap() {
        let dataset = PrivateDataset {
            nquads: case
                .get("dataset_ntriples")
                .map(|value| value.as_str().expect("dataset_ntriples must be a string"))
                .unwrap_or_default()
                .to_owned(),
            named_graphs: vec![],
            salt: [17; 32],
        };
        let request = Request {
            version: VERSION,
            contract: ProofContract::ExactDataset,
            dialect: Dialect::SparqSparql11DatasetV2,
            query: case["query"].as_str().unwrap().to_owned(),
            authority: DatasetAuthority::VerifierAgreed {
                commitment: dataset_commitment(&dataset, &policy).unwrap(),
            },
            policy: policy.clone(),
            nonce: [29; 32],
        };
        admit(&request).unwrap_or_else(|e| panic!("{}: admission {e:?}", case["id"]));
        let journal = evaluate(&Witness { request, dataset })
            .unwrap_or_else(|e| panic!("{}: evaluation {e:?}", case["id"]));
        let CanonicalResult::Select { rows, .. } = journal.result else {
            panic!("SELECT expected")
        };
        let actual = serde_json::json!(rows);
        if actual != case["expected_rows"] {
            failures.push(format!(
                "{}: expected {}, actual {}",
                case["id"], case["expected_rows"], actual
            ));
        }
    }
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 37);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn v2_temporal_constructor_preprocessing_cannot_bypass_model_capacity() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../crates/sparq-engine/tests/fixtures/temporal_lexical.json"
    ))
    .unwrap();
    for case in corpus["capacity_cases"].as_array().unwrap() {
        let dataset = PrivateDataset {
            nquads: case["dataset_ntriples"].as_str().unwrap().into(),
            named_graphs: vec![],
            salt: [17; 32],
        };
        let policy = Policy::default();
        let input = Witness {
            request: Request {
                version: VERSION,
                contract: ProofContract::ExactDataset,
                dialect: Dialect::SparqSparql11DatasetV2,
                query: case["query"].as_str().unwrap().into(),
                authority: DatasetAuthority::VerifierAgreed {
                    commitment: dataset_commitment(&dataset, &policy).unwrap(),
                },
                policy,
                nonce: [29; 32],
            },
            dataset,
        };
        admit(&input.request).unwrap_or_else(|e| panic!("{}: admission {e:?}", case["id"]));
        assert_eq!(
            evaluate(&input).unwrap_err().0,
            "query evaluation or resource budget rejected",
            "{} must reject the whole relation",
            case["id"]
        );
    }
    assert_eq!(corpus["capacity_cases"].as_array().unwrap().len(), 3);
}
