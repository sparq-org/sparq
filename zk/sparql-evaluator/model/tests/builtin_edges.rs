// [GPT-6] Native profile admission/evaluation, separate from guest proof evidence.
#![cfg(feature = "evaluate")]

use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, Dialect, Policy, PrivateDataset, ProofContract, Request,
    VERSION, Witness, admit, dataset_commitment, evaluate,
};

#[test]
fn shared_builtin_goldens_are_admitted_and_evaluated_by_the_native_model() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../crates/sparq-engine/tests/fixtures/builtin_edges.json"
    ))
    .unwrap();
    let policy = Policy::default();
    let mut failures = Vec::new();
    for case in corpus["cases"].as_array().unwrap() {
        let dataset = PrivateDataset {
            ntriples: case
                .get("dataset_ntriples")
                .map(|value| value.as_str().expect("dataset_ntriples must be a string"))
                .unwrap_or_default()
                .to_owned(),
            salt: [17; 32],
        };
        let request = Request {
            version: VERSION,
            contract: ProofContract::ExactDataset,
            dialect: Dialect::SparqSparql11SnapshotV1,
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
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
