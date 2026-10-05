// [GPT-6] Execute complete raw-literal and constructor goldens in the actual guest.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sparq_proved_evaluator_methods::SPARQ_EXACT_GUEST_ELF;
use sparq_proved_evaluator_model::v2::{
    Dialect, Journal, Policy, PrivateDataset, Request, VERSION, Witness, bind_journal,
    dataset_commitment,
};
use sparq_proved_evaluator_model::{CanonicalResult, DatasetAuthority, ProofContract};
use std::path::PathBuf;

#[test]
fn actual_v2_guest_raw_literal_whitespace() {
    let r0vm = std::env::var_os("RISC0_SERVER_PATH")
        .map(PathBuf::from)
        .expect("real local r0vm executable is required");
    let executor = ExternalProver::new("real-sparq-v2-raw-literal-whitespace", r0vm);
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/raw-literal-whitespace.json"
    ))
    .unwrap();
    let mut count = 0;
    for case in corpus["cases"].as_array().unwrap() {
        let dataset = PrivateDataset {
            nquads: case["dataset_ntriples"].as_str().unwrap().into(),
            named_graphs: vec![],
            salt: [71; 32],
        };
        let policy = Policy::default();
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
        eprintln!("actual V2 raw literal guest case {}", case["id"]);
        let env = ExecutorEnv::builder()
            .session_limit(Some(1 << 24))
            .write(&witness)
            .unwrap()
            .build()
            .unwrap();
        let result = executor.execute(env, SPARQ_EXACT_GUEST_ELF);
        count += 1;
        let session = result.unwrap_or_else(|e| panic!("{}: guest execution {e:#}", case["id"]));
        let journal: Journal = session.journal.decode().unwrap();
        bind_journal(&journal, &witness.request).unwrap();
        let expected: CanonicalResult =
            serde_json::from_value(case["expected_result"].clone()).unwrap();
        assert_eq!(journal.result, expected, "{}", case["id"]);
    }
    assert_eq!(count, 32);
}
