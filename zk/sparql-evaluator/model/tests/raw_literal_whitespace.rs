// [GPT-6] Native relation checks; actual guest evidence is a separate runner.
#![cfg(feature = "evaluate")]
use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, Dialect, Policy, PrivateDataset, ProofContract, Request,
    VERSION, Witness, admit, dataset_commitment, evaluate,
};

#[test]
fn raw_literal_whitespace_preserves_the_constructor_boundary() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/raw-literal-whitespace.json"
    ))
    .unwrap();
    let mut count = 0;
    for case in corpus["cases"].as_array().unwrap() {
        let dataset = PrivateDataset {
            ntriples: case["dataset_ntriples"].as_str().unwrap().into(),
            salt: [71; 32],
        };
        let policy = Policy::default();
        let witness = Witness {
            request: Request {
                version: VERSION,
                contract: ProofContract::ExactDataset,
                dialect: Dialect::SparqSparql11SnapshotV1,
                query: case["query"].as_str().unwrap().into(),
                authority: DatasetAuthority::VerifierAgreed {
                    commitment: dataset_commitment(&dataset, &policy).unwrap(),
                },
                policy,
                nonce: [73; 32],
            },
            dataset,
        };
        admit(&witness.request).unwrap_or_else(|e| panic!("{}: {e}", case["id"]));
        count += 1;
        let expected: CanonicalResult =
            serde_json::from_value(case["expected_result"].clone()).unwrap();
        assert_eq!(
            evaluate(&witness).unwrap().result,
            expected,
            "{}",
            case["id"]
        );
    }
    assert_eq!(count, 32);
}
