// [GPT-6] Execute the complete nullable goldens before accepting typed capacity rejections.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sparq_proved_evaluator_methods::SPARQ_EXACT_GUEST_ELF;
use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, Dialect, Journal, Policy, PrivateDataset, ProofContract,
    Request, VERSION, Witness, bind_journal, dataset_commitment,
};
use std::path::PathBuf;

#[test]
fn actual_guest_nullable_alternatives_and_capacity_controls() {
    let r0vm = std::env::var_os("RISC0_SERVER_PATH")
        .map(PathBuf::from)
        .expect("real local r0vm executable is required");
    let executor = ExternalProver::new("real-sparq-nullable-alternatives", r0vm);
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/nullable-alternatives.json"
    ))
    .unwrap();
    // Preserve the original query, core dataset and published two-row golden
    // in actual execution too, rather than only its equivalent empty-graph case.
    let conformance: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/conformance/cases.json")).unwrap();
    let original = conformance["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["id"] == "nullable-path-alternative-absent-constant")
        .unwrap();
    let original = serde_json::json!({
        "id": original["id"],
        "query": original["query"],
        "dataset_ntriples": include_str!("../../fixtures/conformance/core.nt"),
        "expected_result": original["expected"]["result"],
    });
    let mut counts = (0, 0, 0);
    for case in std::iter::once(&original).chain(corpus["cases"].as_array().unwrap()) {
        let dataset = PrivateDataset {
            ntriples: case["dataset_ntriples"].as_str().unwrap().into(),
            salt: [71; 32],
        };
        let policy = Policy {
            max_rows: case["max_rows"]
                .as_u64()
                .map_or(Policy::default().max_rows, |n| u32::try_from(n).unwrap()),
            ..Policy::default()
        };
        let witness = Witness {
            request: Request {
                version: VERSION,
                contract: ProofContract::ExactDataset,
                dialect: Dialect::SparqSparql11SnapshotV1,
                query: case["query"].as_str().unwrap().into(),
                authority: DatasetAuthority::VerifierAgreed {
                    commitment: dataset_commitment(&dataset, &policy).unwrap(),
                },
                policy,
                nonce: [73; 32],
            },
            dataset,
        };
        eprintln!("actual nullable alternative guest case {}", case["id"]);
        let env = ExecutorEnv::builder()
            .session_limit(Some(1 << 24))
            .write(&witness)
            .unwrap()
            .build()
            .unwrap();
        let result = executor.execute(env, SPARQ_EXACT_GUEST_ELF);
        if case["expected_capacity_error"] == true || case["expected_admission_error"] == true {
            assert_eq!(counts.0, 16, "all normative executions must succeed first");
            if case["expected_admission_error"] == true {
                counts.2 += 1;
            } else {
                counts.1 += 1;
            }
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
    assert_eq!(counts, (16, 1, 6));
}
