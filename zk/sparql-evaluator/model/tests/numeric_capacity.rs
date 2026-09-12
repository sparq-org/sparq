// [GPT-6] These are native model checks, not guest execution or proof evidence.
#![cfg(feature = "evaluate")]

use sparq_proved_evaluator_model::{
    admit, dataset_commitment, evaluate, CanonicalResult, DatasetAuthority, Dialect, Policy,
    PrivateDataset, ProofContract, Request, Witness, VERSION,
};

fn witness(case: &serde_json::Value) -> Witness {
    let dataset = PrivateDataset {
        ntriples: case["dataset_ntriples"].as_str().unwrap_or_default().into(),
        salt: [17; 32],
    };
    let policy = Policy::default();
    Witness {
        request: Request {
            version: VERSION,
            contract: ProofContract::ExactDataset,
            dialect: Dialect::SparqSparql11SnapshotV1,
            query: case["query"].as_str().unwrap().into(),
            authority: DatasetAuthority::VerifierAgreed {
                commitment: dataset_commitment(&dataset, &policy).unwrap(),
            },
            policy,
            nonce: [29; 32],
        },
        dataset,
    }
}

#[test]
fn numeric_capacity_is_rejected_by_the_evaluation_relation() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/numeric-capacity.json"
    ))
    .unwrap();
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 21);
    for case in corpus["cases"].as_array().unwrap() {
        let input = witness(case);
        admit(&input.request).unwrap_or_else(|e| panic!("{}: admission {e:?}", case["id"]));
        let error = evaluate(&input).unwrap_err();
        assert_eq!(
            error.0, "query evaluation or resource budget rejected",
            "{}",
            case["id"]
        );
    }
}

#[test]
fn strict_profile_retains_normative_builtin_goldens_and_rejects_capacity_controls() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../crates/sparq-engine/tests/fixtures/builtin_edges.json"
    ))
    .unwrap();
    let mut normative = 0;
    let mut capacity = 0;
    for case in corpus["cases"].as_array().unwrap() {
        let input = witness(case);
        admit(&input.request).unwrap();
        let result = evaluate(&input);
        if case["expectation_kind"] == "implementation_capacity" {
            capacity += 1;
            assert_eq!(
                result.unwrap_err().0,
                "query evaluation or resource budget rejected"
            );
        } else {
            normative += 1;
            let CanonicalResult::Select { rows, .. } = result
                .unwrap_or_else(|e| panic!("{}: {e:?}", case["id"]))
                .result
            else {
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
    assert_eq!((normative, capacity), (167, 2));
}
