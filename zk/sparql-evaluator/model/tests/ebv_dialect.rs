// [GPT-6] Native relation controls; actual guest execution is a separate gate.
#![cfg(feature = "evaluate")]
use sparq_proved_evaluator_model::*;

#[test]
fn proof_profile_pins_rec2013_and_refuses_version_conflicts() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/conformance/ebv-dialect.json")).unwrap();
    let mut counts = (0, 0);
    for case in corpus["cases"].as_array().unwrap() {
        let dataset = PrivateDataset {
            ntriples: String::new(),
            salt: [79; 32],
        };
        let policy = Policy::default();
        let input = Witness {
            request: Request {
                version: VERSION,
                contract: ProofContract::ExactDataset,
                dialect: Dialect::SparqSparql11SnapshotV1,
                query: case["query"].as_str().unwrap().into(),
                authority: DatasetAuthority::VerifierAgreed {
                    commitment: dataset_commitment(&dataset, &policy).unwrap(),
                },
                policy,
                nonce: [83; 32],
            },
            dataset,
        };
        if case["admitted"] == true {
            admit(&input.request).unwrap();
            let expected: CanonicalResult =
                serde_json::from_value(case["expected_result"].clone()).unwrap();
            assert_eq!(evaluate(&input).unwrap().result, expected, "{}", case["id"]);
            counts.0 += 1;
        } else {
            assert!(admit(&input.request).is_err(), "{}", case["id"]);
            assert!(evaluate(&input).is_err(), "{}", case["id"]);
            counts.1 += 1;
        }
    }
    assert_eq!(counts, (3, 5));
}
