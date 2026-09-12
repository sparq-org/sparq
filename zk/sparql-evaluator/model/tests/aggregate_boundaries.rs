// [GPT-6] Executable profile restrictions; not a claim of complete aggregate semantics.
#![cfg(feature = "evaluate")]
use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, Dialect, Policy, PrivateDataset, ProofContract, Request,
    VERSION, Witness, admit, dataset_commitment, evaluate,
};

#[test]
fn aggregate_profile_rejects_nullable_and_fallible_operands() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/aggregate-boundaries.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let dataset = PrivateDataset {
            ntriples: String::new(),
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
                "aggregate operand may be unbound or erroneous",
                "{}",
                case["id"]
            );
            assert_eq!(
                evaluate(&Witness { request, dataset }).unwrap_err().0,
                "aggregate operand may be unbound or erroneous",
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
