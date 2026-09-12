// [GPT-6] Execute admitted aggregates and exact profile rejections in the real guest.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sparq_proved_evaluator_methods::SPARQ_EXACT_GUEST_ELF;
use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, Dialect, Journal, Policy, PrivateDataset, ProofContract,
    Request, VERSION, Witness, bind_journal, dataset_commitment,
};
use std::path::PathBuf;

#[test]
fn actual_guest_executes_aggregate_profile_cases() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/aggregate-boundaries.json"
    ))
    .unwrap();
    let r0vm = std::env::var_os("RISC0_SERVER_PATH")
        .map(PathBuf::from)
        .expect("real local r0vm executable is required");
    let executor = ExternalProver::new("real-sparq-aggregate-boundary", r0vm);
    let policy = Policy::default();
    let mut accepted = 0;
    let mut rejected = 0;
    // Confirm successful real execution before checking remote rejection messages.
    for admitted in [true, false] {
        for case in corpus["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["admitted"] == admitted)
        {
            let dataset = PrivateDataset {
                ntriples: String::new(),
                salt: [41; 32],
            };
            let input = Witness {
                request: Request {
                    version: VERSION,
                    contract: ProofContract::ExactDataset,
                    dialect: Dialect::SparqSparql11SnapshotV1,
                    query: case["query"].as_str().unwrap().into(),
                    authority: DatasetAuthority::VerifierAgreed {
                        commitment: dataset_commitment(&dataset, &policy).unwrap(),
                    },
                    policy: policy.clone(),
                    nonce: [43; 32],
                },
                dataset,
            };
            let env = ExecutorEnv::builder()
                .session_limit(Some(1 << 24))
                .write(&input)
                .unwrap()
                .build()
                .unwrap();
            eprintln!("actual aggregate guest case {}", case["id"]);
            let execution = executor.execute(env, SPARQ_EXACT_GUEST_ELF);
            if admitted {
                let session = execution.unwrap_or_else(|error| panic!("{}: {error:#}", case["id"]));
                let journal: Journal = session.journal.decode().unwrap();
                bind_journal(&journal, &input.request).unwrap();
                let CanonicalResult::Select { rows, .. } = journal.result else {
                    panic!("SELECT expected")
                };
                assert_eq!(
                    serde_json::json!(rows),
                    case["expected_rows"],
                    "{}",
                    case["id"]
                );
                accepted += 1;
            } else {
                let error = execution.expect_err("profile rejection must not reach Halted(0)");
                let rejection = format!("{error:#}");
                assert!(
                    rejection.contains("Guest panicked:")
                        && rejection.contains("bounded exact-dataset relation rejected"),
                    "{}: expected relation rejection, got {rejection}",
                    case["id"]
                );
                rejected += 1;
            }
        }
    }
    assert_eq!((accepted, rejected), (12, 12));
}

#[test]
fn actual_v2_guest_executes_aggregate_profile_cases() {
    use sparq_proved_evaluator_model::v2::{
        Dialect, Journal, Policy, PrivateDataset, Request, VERSION, Witness, bind_journal,
        dataset_commitment,
    };
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/aggregate-boundaries.json"
    ))
    .unwrap();
    let r0vm = std::env::var_os("RISC0_SERVER_PATH")
        .map(PathBuf::from)
        .expect("real local r0vm executable is required");
    let executor = ExternalProver::new("real-sparq-aggregate-boundary", r0vm);
    let policy = Policy::default();
    let mut accepted = 0;
    let mut rejected = 0;
    // Confirm successful real execution before checking remote rejection messages.
    for admitted in [true, false] {
        for case in corpus["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["admitted"] == admitted)
        {
            let dataset = PrivateDataset {
                nquads: String::new(),
                named_graphs: Vec::new(),
                salt: [41; 32],
            };
            let input = Witness {
                request: Request {
                    version: VERSION,
                    contract: ProofContract::ExactDataset,
                    dialect: Dialect::SparqSparql11DatasetV2,
                    query: case["query"].as_str().unwrap().into(),
                    authority: DatasetAuthority::VerifierAgreed {
                        commitment: dataset_commitment(&dataset, &policy).unwrap(),
                    },
                    policy: policy.clone(),
                    nonce: [43; 32],
                },
                dataset,
            };
            let env = ExecutorEnv::builder()
                .session_limit(Some(1 << 24))
                .write(&input)
                .unwrap()
                .build()
                .unwrap();
            eprintln!("actual aggregate guest case {}", case["id"]);
            let execution = executor.execute(env, SPARQ_EXACT_GUEST_ELF);
            if admitted {
                let session = execution.unwrap_or_else(|error| panic!("{}: {error:#}", case["id"]));
                let journal: Journal = session.journal.decode().unwrap();
                bind_journal(&journal, &input.request).unwrap();
                let CanonicalResult::Select { rows, .. } = journal.result else {
                    panic!("SELECT expected")
                };
                assert_eq!(
                    serde_json::json!(rows),
                    case["expected_rows"],
                    "{}",
                    case["id"]
                );
                accepted += 1;
            } else {
                let error = execution.expect_err("profile rejection must not reach Halted(0)");
                let rejection = format!("{error:#}");
                assert!(
                    rejection.contains("Guest panicked:")
                        && rejection.contains("bounded exact-dataset relation rejected"),
                    "{}: expected relation rejection, got {rejection}",
                    case["id"]
                );
                rejected += 1;
            }
        }
    }
    assert_eq!((accepted, rejected), (12, 12));
}
