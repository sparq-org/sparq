// [GPT-6] The named-dataset relation retains shared correlation boundaries.
#![cfg(feature = "evaluate")]
use sparq_proved_evaluator_model::{CanonicalResult, DatasetAuthority, ProofContract, v2};

fn input(query: &str, data: &str) -> v2::Witness {
    v2::Witness {
        request: v2::Request {
            version: v2::VERSION,
            contract: ProofContract::ExactDataset,
            dialect: v2::Dialect::SparqSparql11DatasetV2,
            query: query.into(),
            authority: DatasetAuthority::HolderDeclared,
            policy: v2::Policy::default(),
            nonce: [97; 32],
        },
        dataset: v2::PrivateDataset {
            nquads: data.into(),
            named_graphs: vec![],
            salt: [89; 32],
        },
    }
}

#[test]
fn v2_captured_bound_profile_preserves_every_original_case() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/exists-bound-boundaries.json"
    ))
    .unwrap();
    let mut counts = (0, 0);
    for case in corpus["cases"].as_array().unwrap() {
        let witness = input(
            case["query"].as_str().unwrap(),
            case["dataset_ntriples"].as_str().unwrap_or(""),
        );
        if case["admitted"] == false {
            for error in [
                v2::admit(&witness.request).unwrap_err(),
                v2::evaluate(&witness).unwrap_err(),
            ] {
                assert_eq!(
                    error.0,
                    "captured BOUND is outside the SPARQL 1.1 substitution profile"
                );
            }
            counts.1 += 1;
        } else {
            v2::admit(&witness.request).unwrap();
            let journal = v2::evaluate(&witness).unwrap();
            v2::bind_journal(&journal, &witness.request).unwrap();
            let CanonicalResult::Select { rows, .. } = journal.result else {
                panic!("SELECT expected")
            };
            assert_eq!(
                serde_json::json!(rows),
                case["expected_rows"],
                "{}",
                case["id"]
            );
            counts.0 += 1;
        }
    }
    assert_eq!(counts, (8, 12));
}

#[test]
fn v2_minus_domains_preserve_goldens_and_false_ask() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/exists-minus-domains.json"
    ))
    .unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    for case in cases {
        let witness = input(
            case["query"].as_str().unwrap(),
            corpus["dataset_ntriples"].as_str().unwrap(),
        );
        v2::admit(&witness.request).unwrap();
        let journal = v2::evaluate(&witness).unwrap();
        v2::bind_journal(&journal, &witness.request).unwrap();
        let expected: CanonicalResult =
            serde_json::from_value(case["expected_result"].clone()).unwrap();
        assert_eq!(journal.result, expected, "{}", case["id"]);
    }
    let witness = input(
        include_str!("../../fixtures/false-absence.rq"),
        include_str!("../../fixtures/default.nt"),
    );
    assert_eq!(
        v2::evaluate(&witness).unwrap().result,
        CanonicalResult::Ask(false)
    );
}
