// [GPT-6] Actual guest execution over shared REC-derived and capacity goldens.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sparq_proved_evaluator_methods::SPARQ_EXACT_GUEST_ELF;
use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, Dialect, Journal, Policy, PrivateDataset, ProofContract,
    Request, VERSION, Witness, bind_journal, dataset_commitment,
};
use std::path::PathBuf;

#[test]
fn actual_guest_executes_exact_temporal_values_and_capacity_rejections() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../crates/sparq-engine/tests/fixtures/exact_temporal.json"
    ))
    .unwrap();
    let r0vm = std::env::var_os("RISC0_SERVER_PATH")
        .map(PathBuf::from)
        .expect("real local r0vm executable is required");
    let executor = ExternalProver::new("real-sparq-builtin-edges", r0vm);
    let policy = Policy::default();
    let mut rec_cases = 0;
    for case in corpus["cases"].as_array().unwrap() {
        let dataset = PrivateDataset {
            ntriples: case
                .get("dataset_ntriples")
                .map(|value| value.as_str().expect("dataset_ntriples must be a string"))
                .unwrap_or_default()
                .to_owned(),
            salt: [17; 32],
        };
        rec_cases += 1;
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
    assert_eq!(rec_cases, 28, "execute every exact temporal expectation");
    // Positive executions above distinguish an actual relation rejection from a broken executor.
    let rejected: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/temporal-capacity.json"
    ))
    .unwrap();
    for case in rejected["cases"].as_array().unwrap() {
        let dataset = PrivateDataset {
            ntriples: case["dataset_ntriples"].as_str().unwrap().into(),
            salt: [17; 32],
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
                nonce: [29; 32],
            },
            dataset,
        };
        let env = ExecutorEnv::builder()
            .session_limit(Some(1 << 24))
            .write(&input)
            .unwrap()
            .build()
            .unwrap();
        let error = executor
            .execute(env, SPARQ_EXACT_GUEST_ELF)
            .expect_err("capacity violation must not reach Halted(0)");
        let rejection = format!("{error:#}");
        assert!(
            rejection.contains("Guest panicked:")
                && rejection.contains("bounded exact-dataset relation rejected"),
            "{}: expected relation rejection, got {rejection}",
            case["id"]
        );
    }
    assert_eq!(rejected["cases"].as_array().unwrap().len(), 13);
}

#[test]
fn actual_v2_guest_executes_exact_temporal_values_and_capacity_rejections() {
    use sparq_proved_evaluator_model::v2::{
        Dialect, Journal, Policy, PrivateDataset, Request, VERSION, Witness, bind_journal,
        dataset_commitment,
    };
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../crates/sparq-engine/tests/fixtures/exact_temporal.json"
    ))
    .unwrap();
    let r0vm = std::env::var_os("RISC0_SERVER_PATH")
        .map(PathBuf::from)
        .expect("real local r0vm executable is required");
    let executor = ExternalProver::new("real-sparq-builtin-edges", r0vm);
    let policy = Policy::default();
    let mut rec_cases = 0;
    for case in corpus["cases"].as_array().unwrap() {
        let dataset = PrivateDataset {
            nquads: case
                .get("dataset_ntriples")
                .map(|value| value.as_str().expect("dataset_ntriples must be a string"))
                .unwrap_or_default()
                .to_owned(),
            named_graphs: Vec::new(),
            salt: [17; 32],
        };
        rec_cases += 1;
        let input = Witness {
            request: Request {
                version: VERSION,
                contract: ProofContract::ExactDataset,
                dialect: Dialect::SparqSparql11DatasetV2,
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
    assert_eq!(rec_cases, 28, "execute every exact temporal expectation");
    // Positive executions above distinguish an actual relation rejection from a broken executor.
    let rejected: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/temporal-capacity.json"
    ))
    .unwrap();
    for case in rejected["cases"].as_array().unwrap() {
        let dataset = PrivateDataset {
            nquads: case["dataset_ntriples"].as_str().unwrap().into(),
            named_graphs: Vec::new(),
            salt: [17; 32],
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
                nonce: [29; 32],
            },
            dataset,
        };
        let env = ExecutorEnv::builder()
            .session_limit(Some(1 << 24))
            .write(&input)
            .unwrap()
            .build()
            .unwrap();
        let error = executor
            .execute(env, SPARQ_EXACT_GUEST_ELF)
            .expect_err("capacity violation must not reach Halted(0)");
        let rejection = format!("{error:#}");
        assert!(
            rejection.contains("Guest panicked:")
                && rejection.contains("bounded exact-dataset relation rejected"),
            "{}: expected relation rejection, got {rejection}",
            case["id"]
        );
    }
    assert_eq!(rejected["cases"].as_array().unwrap().len(), 13);
}
