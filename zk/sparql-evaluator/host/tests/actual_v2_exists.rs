// [GPT-6] Actual named-dataset guest execution; native corpus success is separate.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sparq_proved_evaluator::embedded_artifact;
use sparq_proved_evaluator_model::{CanonicalResult, DatasetAuthority, ProofContract, v2};
use std::path::PathBuf;

fn executor() -> ExternalProver {
    ExternalProver::new(
        "sparq-v2-exists-profile",
        PathBuf::from(std::env::var_os("RISC0_SERVER_PATH").expect("local r0vm required")),
    )
}

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

fn execute(executor: &ExternalProver, witness: &v2::Witness) -> Result<v2::Journal, String> {
    let env = ExecutorEnv::builder()
        .session_limit(Some(1 << 24))
        .write(witness)
        .unwrap()
        .build()
        .unwrap();
    let session = executor
        .execute(env, embedded_artifact())
        .map_err(|error| format!("{error:#}"))?;
    let journal: v2::Journal = session.journal.decode().unwrap();
    v2::bind_journal(&journal, &witness.request).unwrap();
    Ok(journal)
}

#[test]
fn actual_v2_captured_bound_profile() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/exists-bound-boundaries.json"
    ))
    .unwrap();
    let executor = executor();
    let mut counts = (0, 0);
    for admitted in [true, false] {
        for case in corpus["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["admitted"] == admitted)
        {
            let witness = input(
                case["query"].as_str().unwrap(),
                case["dataset_ntriples"].as_str().unwrap_or(""),
            );
            let result = execute(&executor, &witness);
            if admitted {
                let CanonicalResult::Select { rows, .. } = result.unwrap().result else {
                    panic!("SELECT expected")
                };
                assert_eq!(
                    serde_json::json!(rows),
                    case["expected_rows"],
                    "{}",
                    case["id"]
                );
                counts.0 += 1;
            } else {
                let error = result.expect_err("captured BOUND must reject");
                assert!(
                    error.contains("Guest panicked:")
                        && error.contains("bounded exact-dataset relation rejected"),
                    "unexpected executor error: {error}"
                );
                counts.1 += 1;
            }
        }
    }
    assert_eq!(counts, (8, 12));
}

#[test]
fn actual_v2_minus_domains_and_false_ask() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/exists-minus-domains.json"
    ))
    .unwrap();
    let executor = executor();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    for case in cases {
        let witness = input(
            case["query"].as_str().unwrap(),
            corpus["dataset_ntriples"].as_str().unwrap(),
        );
        let expected: CanonicalResult =
            serde_json::from_value(case["expected_result"].clone()).unwrap();
        assert_eq!(
            execute(&executor, &witness).unwrap().result,
            expected,
            "{}",
            case["id"]
        );
    }
    let witness = input(
        include_str!("../../fixtures/false-absence.rq"),
        include_str!("../../fixtures/default.nt"),
    );
    assert_eq!(
        execute(&executor, &witness).unwrap().result,
        CanonicalResult::Ask(false)
    );
}
