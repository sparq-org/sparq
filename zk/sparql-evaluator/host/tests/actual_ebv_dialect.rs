// [GPT-6] Real execution confirms the serialized proof dialect cannot be overridden by VERSION.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sparq_proved_evaluator_methods::SPARQ_EXACT_GUEST_ELF;
use sparq_proved_evaluator_model::*;
use std::path::PathBuf;

#[test]
fn actual_guest_pins_rec2013_before_version_rejections() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/conformance/ebv-dialect.json")).unwrap();
    let r0vm = std::env::var_os("RISC0_SERVER_PATH")
        .map(PathBuf::from)
        .expect("real r0vm is required");
    let executor = ExternalProver::new("real-sparq-ebv-dialect", r0vm);
    let mut counts = (0, 0);
    // A successful real execution must precede all classified guest rejections.
    for accepted in [true, false] {
        for case in corpus["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["admitted"] == accepted)
        {
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
            eprintln!("actual EBV dialect case {}", case["id"]);
            let env = ExecutorEnv::builder()
                .session_limit(Some(1 << 24))
                .write(&input)
                .unwrap()
                .build()
                .unwrap();
            let execution = executor.execute(env, SPARQ_EXACT_GUEST_ELF);
            if accepted {
                let session = execution.unwrap_or_else(|error| panic!("{}: {error:#}", case["id"]));
                let journal: Journal = session.journal.decode().unwrap();
                bind_journal(&journal, &input.request).unwrap();
                let expected: CanonicalResult =
                    serde_json::from_value(case["expected_result"].clone()).unwrap();
                assert_eq!(journal.result, expected, "{}", case["id"]);
                counts.0 += 1;
            } else {
                let error = execution.expect_err("conflicting VERSION must not produce a journal");
                let message = format!("{error:#}");
                assert!(
                    message.contains("Guest panicked:")
                        && message.contains("bounded exact-dataset relation rejected"),
                    "{}: {message}",
                    case["id"]
                );
                counts.1 += 1;
            }
        }
    }
    assert_eq!(counts, (3, 5));
}
