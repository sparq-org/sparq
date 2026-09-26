// [GPT-6] Real guest execution is required; these are not mock receipt checks.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sparq_proved_evaluator_methods::SPARQ_EXACT_GUEST_ELF;
use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, Dialect, Policy, PrivateDataset, ProofContract, Request,
    VERSION, Witness, dataset_commitment,
};
use sparq_proved_evaluator_model::{Journal, bind_journal};
use std::path::PathBuf;

fn input(query: &str, ntriples: &str) -> Witness {
    let policy = Policy::default();
    let dataset = PrivateDataset {
        ntriples: ntriples.into(),
        salt: [61; 32],
    };
    Witness {
        request: Request {
            version: VERSION,
            contract: ProofContract::ExactDataset,
            dialect: Dialect::SparqSparql11SnapshotV1,
            query: query.into(),
            authority: DatasetAuthority::VerifierAgreed {
                commitment: dataset_commitment(&dataset, &policy).unwrap(),
            },
            policy,
            nonce: [67; 32],
        },
        dataset,
    }
}

fn execute(executor: &ExternalProver, witness: &Witness, expected: CanonicalResult) {
    let env = ExecutorEnv::builder()
        .session_limit(Some(1 << 24))
        .write(witness)
        .unwrap()
        .build()
        .unwrap();
    let session = executor
        .execute(env, SPARQ_EXACT_GUEST_ELF)
        .expect("real guest execution");
    let journal: Journal = session.journal.decode().unwrap();
    bind_journal(&journal, &witness.request).unwrap();
    assert_eq!(journal.result, expected);
}

#[test]
fn actual_guest_executes_minus_domains_and_false_ask_discriminator() {
    let r0vm = std::env::var_os("RISC0_SERVER_PATH")
        .map(PathBuf::from)
        .expect("real local r0vm executable is required");
    let executor = ExternalProver::new("real-sparq-exists-minus-domains", r0vm);
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/exists-minus-domains.json"
    ))
    .unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    for case in cases {
        eprintln!("actual EXISTS/MINUS guest case {}", case["id"]);
        let witness = input(
            case["query"].as_str().unwrap(),
            corpus["dataset_ntriples"].as_str().unwrap(),
        );
        let expected = serde_json::from_value(case["expected_result"].clone()).unwrap();
        execute(&executor, &witness, expected);
    }
    eprintln!("actual EXISTS/MINUS guest false-ASK discriminator");
    let witness = input(
        include_str!("../../fixtures/false-absence.rq"),
        include_str!("../../fixtures/default.nt"),
    );
    execute(&executor, &witness, CanonicalResult::Ask(false));
}
