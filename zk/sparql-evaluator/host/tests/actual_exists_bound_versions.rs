// [GPT-6] The shared captured-BOUND profile executes through each later wire version.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sparq_proved_evaluator::embedded_artifact;
use sparq_proved_evaluator_model::{DatasetAuthority, ProofContract, v2, v3};
use std::path::PathBuf;

fn run(version: u32) {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/exists-bound-boundaries.json"
    ))
    .unwrap();
    let executor = ExternalProver::new(
        "sparq-versioned-bound-profile",
        PathBuf::from(std::env::var_os("RISC0_SERVER_PATH").expect("local r0vm required")),
    );
    let mut counts = (0, 0);
    for admitted in [true, false] {
        for case in corpus["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["admitted"] == admitted)
        {
            let dataset = v2::PrivateDataset {
                nquads: case["dataset_ntriples"].as_str().unwrap_or("").into(),
                named_graphs: vec![],
                salt: [89; 32],
            };
            let request = v2::Request {
                version: v2::VERSION,
                contract: ProofContract::ExactDataset,
                dialect: v2::Dialect::SparqSparql11DatasetV2,
                query: case["query"].as_str().unwrap().into(),
                authority: DatasetAuthority::HolderDeclared,
                policy: v2::Policy::default(),
                nonce: [97; 32],
            };
            let v3_request = v3::Request {
                version: v3::VERSION,
                contract: ProofContract::ExactDataset,
                dialect: v3::Dialect::SparqSparql11GraphResultsV3,
                query: request.query.clone(),
                authority: request.authority.clone(),
                policy: v3::Policy::default(),
                nonce: request.nonce,
            };
            let words = match version {
                2 => risc0_zkvm::serde::to_vec(&v2::Witness {
                    request: request.clone(),
                    dataset,
                })
                .unwrap(),
                3 => risc0_zkvm::serde::to_vec(&v3::Witness {
                    request: v3_request.clone(),
                    dataset,
                })
                .unwrap(),
                _ => panic!("unsupported fixture version"),
            };
            let bytes: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
            let env = ExecutorEnv::builder()
                .session_limit(Some(1 << 24))
                .write_slice(&bytes)
                .build()
                .unwrap();
            let execution = executor.execute(env, embedded_artifact());
            if admitted {
                let session = execution.unwrap_or_else(|error| panic!("{}: {error:#}", case["id"]));
                let rows = if version == 2 {
                    let journal: v2::Journal = session.journal.decode().unwrap();
                    v2::bind_journal(&journal, &request).unwrap();
                    let sparq_proved_evaluator_model::CanonicalResult::Select { rows, .. } =
                        journal.result
                    else {
                        panic!("SELECT")
                    };
                    rows
                } else {
                    let journal: v3::Journal = session.journal.decode().unwrap();
                    v3::bind_journal(&journal, &v3_request).unwrap();
                    let v3::CanonicalResult::Select { rows, .. } = journal.result else {
                        panic!("SELECT")
                    };
                    rows
                };
                assert_eq!(
                    serde_json::json!(rows),
                    case["expected_rows"],
                    "{}",
                    case["id"]
                );
                counts.0 += 1;
            } else {
                let error = execution.expect_err("captured BOUND must reject");
                let text = format!("{error:#}");
                assert!(
                    text.contains("Guest panicked:")
                        && text.contains("bounded exact-dataset relation rejected"),
                    "unexpected executor error: {text}"
                );
                counts.1 += 1;
            }
        }
    }
    assert_eq!(counts, (8, 12));
}

#[test]
fn actual_v2_captured_bound_profile() {
    run(2);
}

#[test]
fn actual_v3_captured_bound_profile() {
    run(3);
}
