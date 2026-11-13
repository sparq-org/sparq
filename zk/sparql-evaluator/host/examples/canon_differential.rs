//! Compare the unchanged native canonicaliser with the zkVM-target backend.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

fn main() {
    let path = std::env::args_os().nth(1).expect("input JSON file");
    let inputs: Vec<Value> = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let runtime = PathBuf::from(std::env::var_os("RISC0_SERVER_PATH").expect("executor"));
    let executor = ExternalProver::new("canon-differential", runtime);
    for input in inputs {
        let text = input["input"].as_str().unwrap();
        let sha384 = input["sha384"].as_bool().unwrap();
        let native = sparq_canon::parse_nquads(text)
            .and_then(|quads| {
                if sha384 {
                    sparq_canon::canonicalize_quads_with::<sha2::Sha384>(&quads)
                } else {
                    sparq_canon::canonicalize_quads_with::<sha2::Sha256>(&quads)
                }
            })
            .map_err(|error| error.to_string());
        let env = ExecutorEnv::builder()
            .session_limit(Some(1 << 32))
            .segment_limit_po2(20)
            .write(&(text, sha384))
            .unwrap()
            .build()
            .unwrap();
        let session = executor
            .execute(
                env,
                sparq_proved_evaluator_methods::SPARQ_CANON_CHECK_GUEST_ELF,
            )
            .expect("canonicalisation guest execution");
        let guest: Result<String, String> = session.journal.decode().unwrap();
        assert_eq!(guest, native, "{}", input["id"]);
        if let Some(expected) = input.get("expected") {
            assert_eq!(
                guest.as_ref().unwrap(),
                expected.as_str().unwrap(),
                "{}",
                input["id"]
            );
        }
        if input.get("negative").and_then(Value::as_bool) == Some(true) {
            assert!(guest.is_err(), "{}", input["id"]);
        }
        println!(
            "{}",
            json!({"id": input["id"], "sha384": sha384, "output": guest,
            "journal_sha256": format!("{:x}", Sha256::digest(&session.journal.bytes))})
        );
    }
}
