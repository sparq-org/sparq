//! Compare the unchanged native canonicaliser with the zkVM-target backend.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

type Outputs = (
    Result<String, String>,
    Result<String, String>,
    Result<String, String>,
);

fn outputs<D: sparq_canon::Digest>(text: &str) -> Outputs {
    let quads = match sparq_canon::parse_nquads(text) {
        Ok(quads) => quads,
        Err(error) => {
            let message = error.to_string();
            return (Err(message.clone()), Err(message.clone()), Err(message));
        }
    };
    let limits = sparq_canon::CanonicalizationLimits {
        max_quads: 100_000,
        max_input_bytes: 16 << 20,
        max_output_bytes: 16 << 20,
        max_hndq_calls: 4000,
        max_permutation_steps: 1 << 28,
    };
    (
        sparq_canon::canonicalize_quads_with::<D>(&quads).map_err(|e| e.to_string()),
        sparq_canon::canonicalize_quads_bounded_with::<D>(&quads, &limits)
            .map_err(|e| e.to_string()),
        sparq_canon::issue_quads_bounded_ordered_with::<D>(&quads, &limits)
            .map(|labels| {
                // Compare the issuer through its relabelled dataset: symmetric
                // nodes can have interchangeable labels in the canonical form.
                let relabel = |term: String| match term.strip_prefix("_:") {
                    Some(label) => format!("_:{}", labels[label]),
                    None => term,
                };
                let mut lines: Vec<_> = quads
                    .iter()
                    .map(|quad| {
                        let graph = if quad.graph_name.is_default_graph() {
                            String::new()
                        } else {
                            format!(" {}", relabel(quad.graph_name.to_string()))
                        };
                        format!(
                            "{} {} {}{graph} .\n",
                            relabel(quad.subject.to_string()),
                            quad.predicate,
                            relabel(quad.object.to_string())
                        )
                    })
                    .collect();
                lines.sort();
                lines.dedup();
                lines.concat()
            })
            .map_err(|e| e.to_string()),
    )
}

fn main() {
    let path = std::env::args_os().nth(1).expect("input JSON file");
    let inputs: Vec<Value> = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let runtime = PathBuf::from(std::env::var_os("RISC0_SERVER_PATH").expect("executor"));
    let executor = ExternalProver::new("canon-differential", runtime);
    for input in inputs {
        let text = input["input"].as_str().unwrap();
        let sha384 = input["sha384"].as_bool().unwrap();
        let native = if sha384 {
            outputs::<sha2::Sha384>(text)
        } else {
            outputs::<sha2::Sha256>(text)
        };
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
        let guest: Outputs = session.journal.decode().unwrap();
        assert_eq!(guest, native, "{}", input["id"]);
        for output in [&guest.1, &guest.2].into_iter().flatten() {
            assert_eq!(output, guest.0.as_ref().unwrap(), "{}", input["id"]);
        }
        if let Some(expected) = input.get("expected") {
            assert_eq!(
                guest.0.as_ref().unwrap(),
                expected.as_str().unwrap(),
                "{}",
                input["id"]
            );
        }
        if input.get("negative").and_then(Value::as_bool) == Some(true) {
            assert!(guest.0.is_err(), "{}", input["id"]);
        }
        println!(
            "{}",
            json!({"id": input["id"], "sha384": sha384, "output": guest.0, "bounded_output": guest.1, "ordered_issuer_output": guest.2,
            "journal_sha256": format!("{:x}", Sha256::digest(&session.journal.bytes))})
        );
    }
}
