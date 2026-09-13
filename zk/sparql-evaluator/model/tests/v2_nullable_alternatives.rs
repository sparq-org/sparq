// [GPT-6] Native relation checks; actual guest evidence is a separate runner.
#![cfg(feature = "evaluate")]
use sparq_proved_evaluator_model::v2::{
    Dialect, Policy, PrivateDataset, Request, VERSION, Witness, admit, dataset_commitment, evaluate,
};
use sparq_proved_evaluator_model::{CanonicalResult, DatasetAuthority, ProofContract};

#[test]
fn v2_nullable_alternatives_match_rec_and_preserve_restrictions() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/nullable-alternatives.json"
    ))
    .unwrap();
    let mut counts = (0, 0, 0);
    for case in corpus["cases"].as_array().unwrap() {
        let dataset = PrivateDataset {
            nquads: case["dataset_ntriples"].as_str().unwrap().into(),
            named_graphs: vec![],
            salt: [71; 32],
        };
        let policy = Policy {
            max_rows: case["max_rows"]
                .as_u64()
                .map_or(Policy::default().max_rows, |n| u32::try_from(n).unwrap()),
            ..Policy::default()
        };
        let witness = Witness {
            request: Request {
                version: VERSION,
                contract: ProofContract::ExactDataset,
                dialect: Dialect::SparqSparql11DatasetV2,
                query: case["query"].as_str().unwrap().into(),
                authority: DatasetAuthority::VerifierAgreed {
                    commitment: dataset_commitment(&dataset, &policy).unwrap(),
                },
                policy,
                nonce: [73; 32],
            },
            dataset,
        };
        if case["expected_admission_error"] == true {
            counts.2 += 1;
            assert!(admit(&witness.request).is_err(), "{}", case["id"]);
            assert!(evaluate(&witness).is_err(), "{}", case["id"]);
            continue;
        }
        admit(&witness.request).unwrap_or_else(|e| panic!("{}: {e}", case["id"]));
        if case["expected_capacity_error"] == true {
            counts.1 += 1;
            assert_eq!(
                evaluate(&witness).unwrap_err().0,
                "query evaluation or resource budget rejected",
                "{}",
                case["id"]
            );
        } else {
            counts.0 += 1;
            let expected: CanonicalResult =
                serde_json::from_value(case["expected_result"].clone()).unwrap();
            assert_eq!(
                evaluate(&witness).unwrap().result,
                expected,
                "{}",
                case["id"]
            );
        }
    }
    assert_eq!(counts, (15, 1, 6));
}
