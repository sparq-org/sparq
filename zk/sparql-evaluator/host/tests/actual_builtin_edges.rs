// [GPT-6] Actual guest execution over shared REC-derived and capacity goldens.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sparq_proved_evaluator_methods::SPARQ_EXACT_GUEST_ELF;
use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, Dialect, Journal, Policy, PrivateDataset, ProofContract,
    Request, VERSION, Witness, bind_journal, dataset_commitment,
};
use std::path::PathBuf;

#[test]
fn actual_guest_executes_shared_builtin_edges_and_labeled_capacity_controls() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../crates/sparq-engine/tests/fixtures/builtin_edges.json"
    ))
    .unwrap();
    let r0vm = std::env::var_os("RISC0_SERVER_PATH")
        .map(PathBuf::from)
        .expect("real local r0vm executable is required");
    let executor = ExternalProver::new("real-sparq-builtin-edges", r0vm);
    let policy = Policy::default();
    let mut rec_cases = 0;
    let mut capacity_controls = 0;
    for case in corpus["cases"].as_array().unwrap() {
        let dataset = PrivateDataset {
            ntriples: case
                .get("dataset_ntriples")
                .map(|value| value.as_str().expect("dataset_ntriples must be a string"))
                .unwrap_or_default()
                .to_owned(),
            salt: [17; 32],
        };
        match case["expectation_kind"].as_str().unwrap() {
            "published_recommendation" => rec_cases += 1,
            "implementation_capacity" => capacity_controls += 1,
            other => panic!("unclassified builtin expectation: {other}"),
        }
        let input = Witness {
            request: Request {
                version: VERSION,
                contract: ProofContract::ExactDataset,
                dialect: Dialect::SparqSparql11SnapshotV1,
                query: case["query"].as_str().unwrap().to_owned(),
                authority: DatasetAuthority::VerifierAgreed {
                    commitment: dataset_commitment(&dataset, &policy).unwrap(),
                },
                policy: policy.clone(),
                nonce: [29; 32],
            },
            dataset,
        };
        eprintln!("actual builtin guest case {}", case["id"]);
        let env = ExecutorEnv::builder()
            .session_limit(Some(1 << 24))
            .write(&input)
            .unwrap()
            .build()
            .unwrap();
        let session = executor
            .execute(env, SPARQ_EXACT_GUEST_ELF)
            .unwrap_or_else(|e| panic!("{}: actual guest execution {e:#}", case["id"]));
        let journal: Journal = session.journal.decode().unwrap();
        bind_journal(&journal, &input.request).unwrap();
        let CanonicalResult::Select { rows, .. } = journal.result else {
            panic!("{}: SELECT expected", case["id"])
        };
        assert_eq!(
            serde_json::json!(rows),
            case["expected_rows"],
            "{}",
            case["id"]
        );
    }
    assert_eq!(rec_cases, 137, "execute every REC-derived edge expectation");
    assert_eq!(
        capacity_controls, 2,
        "capacity controls are separate evidence"
    );
}
