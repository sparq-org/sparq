// [GPT-6] Execute the complete EBV goldens before accepting typed capacity rejections.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sparq_proved_evaluator_methods::SPARQ_EXACT_GUEST_ELF;
use sparq_proved_evaluator_model::v2::{
    Dialect, Journal, Policy, PrivateDataset, Request, VERSION, Witness, bind_journal,
    dataset_commitment,
};
use sparq_proved_evaluator_model::{CanonicalResult, DatasetAuthority, ProofContract};
use std::path::PathBuf;

#[test]
fn actual_v2_guest_numeric_ebv_and_capacity_controls() {
    let r0vm = std::env::var_os("RISC0_SERVER_PATH")
        .map(PathBuf::from)
        .expect("real local r0vm executable is required");
    let executor = ExternalProver::new("real-sparq-v2-numeric-ebv", r0vm);
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/conformance/numeric-ebv.json")).unwrap();
    let mut counts = (0, 0);
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
        eprintln!("actual V2 numeric EBV guest case {}", case["id"]);
        let env = ExecutorEnv::builder()
            .session_limit(Some(1 << 24))
            .write(&witness)
            .unwrap()
            .build()
            .unwrap();
        let result = executor.execute(env, SPARQ_EXACT_GUEST_ELF);
        if case["expected_capacity_error"] == true {
            assert_eq!(counts.0, 17, "all normative executions must succeed first");
            counts.1 += 1;
            let error = result.expect_err("capacity must not reach Halted(0)");
            let rejection = format!("{error:#}");
            assert!(
                rejection.contains("Guest panicked:")
                    && rejection.contains("bounded exact-dataset relation rejected"),
                "{}: expected relation rejection, got {rejection}",
                case["id"]
            );
        } else {
            counts.0 += 1;
            let session =
                result.unwrap_or_else(|e| panic!("{}: guest execution {e:#}", case["id"]));
            let journal: Journal = session.journal.decode().unwrap();
            bind_journal(&journal, &witness.request).unwrap();
            let expected: CanonicalResult =
                serde_json::from_value(case["expected_result"].clone()).unwrap();
            assert_eq!(journal.result, expected, "{}", case["id"]);
        }
    }
    assert_eq!(counts, (17, 3));
}
