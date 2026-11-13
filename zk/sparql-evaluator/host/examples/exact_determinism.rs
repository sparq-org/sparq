//! Executor-only native journal comparison for the production exact image.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sparq_proved_evaluator_model as model;
use std::path::PathBuf;

fn execute<T: Serialize, J: Serialize>(version: u32, witness: &T, native: &J) {
    let artifact = sparq_proved_evaluator_methods::SPARQ_EXACT_GUEST_ELF;
    let words = risc0_zkvm::serde::to_vec(witness).unwrap();
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let expected: Vec<u8> = risc0_zkvm::serde::to_vec(native)
        .unwrap()
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    let runtime = PathBuf::from(std::env::var_os("RISC0_SERVER_PATH").expect("executor"));
    let executor = ExternalProver::new("exact-determinism", runtime);
    let env = ExecutorEnv::builder()
        .session_limit(Some(1 << 25))
        .write_slice(&bytes)
        .build()
        .unwrap();
    let session = executor
        .execute(env, artifact)
        .expect("exact guest execution");
    assert_eq!(session.journal.bytes, expected);
    println!(
        "{}",
        json!({"version": version, "artifact_sha256": format!("{:x}", Sha256::digest(artifact)),
        "witness_sha256": format!("{:x}", Sha256::digest(bytes)),
        "journal_sha256": format!("{:x}", Sha256::digest(expected)),
        "user_cycles": session.cycles(),
        "padded_cycles": session.segments.iter().map(|s| 1u64 << s.po2).sum::<u64>(), "segments": session.segments.len()})
    );
}

fn main() {
    let witness = model::Witness {
        request: model::Request {
            version: model::VERSION,
            contract: model::ProofContract::ExactDataset,
            dialect: model::Dialect::SparqSparql11SnapshotV1,
            query: "SELECT DISTINCT ?s WHERE { ?s <urn:p> ?o }".into(),
            authority: model::DatasetAuthority::HolderDeclared,
            policy: model::Policy::default(),
            nonce: [31; 32],
        },
        dataset: model::PrivateDataset {
            ntriples: "<urn:a> <urn:p> <urn:o1> .\n<urn:b> <urn:p> <urn:o2> .\n".into(),
            salt: [37; 32],
        },
    };
    execute(
        model::VERSION,
        &witness,
        &model::evaluate(&witness).unwrap(),
    );
    let witness = model::v2::Witness {
        request: model::v2::Request {
            version: model::v2::VERSION,
            contract: model::ProofContract::ExactDataset,
            dialect: model::v2::Dialect::SparqSparql11DatasetV2,
            query: "SELECT DISTINCT ?s WHERE { GRAPH <urn:g> { ?s <urn:p> ?o } }".into(),
            authority: model::DatasetAuthority::HolderDeclared,
            policy: model::v2::Policy::default(),
            nonce: [41; 32],
        },
        dataset: model::v2::PrivateDataset {
            nquads: "<urn:a> <urn:p> <urn:o1> <urn:g> .\n<urn:b> <urn:p> <urn:o2> <urn:g> .\n"
                .into(),
            named_graphs: vec!["urn:g".into()],
            salt: [43; 32],
        },
    };
    execute(
        model::v2::VERSION,
        &witness,
        &model::v2::evaluate(&witness).unwrap(),
    );
}
