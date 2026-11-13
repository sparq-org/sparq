//! Live credential compatibility checks, using the existing fail-closed gate.
use serde_json::Value;
use sparq_merkle_suite::{poseidon as p, Fr};
use sparq_zk::field::field_to_hex;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
#[path = "support/cases.rs"]
mod cases;
#[path = "support/credential.rs"]
mod credential;
#[path = "../../sparq-zk/tests/support/mod.rs"]
mod support;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "sparq-merkle-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn assert_compiler(gate: &support::LiveToolchain) {
    let out = support::live_tool(gate, "nargo")
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success(), "nargo version command failed");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("is dirty: false"),
        "credential suite requires the pinned release compiler"
    );
    let lines: Vec<_> = stdout.lines().take(2).collect();
    assert_eq!(
        lines,
        [
            "nargo version = 1.0.0-beta.21",
            "noirc version = 1.0.0-beta.21+89a0f0faf3a5f1273c8ac4843b7877882437e277"
        ],
        "credential suite compiler pin drifted"
    );
}
// One constant per path: the out-of-crate input audit attributes each literal
// path that follows CARGO_MANIFEST_DIR up to the next semicolon.
const NOIR_CORE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../zk/noir-query/core");
const COMPOSE_CORE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../zk/compose/compose_core");
const NOIR_FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../zk/noir-query/fixtures");
fn toml(v: &Value) -> String {
    match v {
        Value::Array(a) => format!("[{}]", a.iter().map(toml).collect::<Vec<_>>().join(",")),
        Value::Object(o) => format!(
            "{{{}}}",
            o.iter()
                .map(|(k, v)| format!("{k}={}", toml(v)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => v.to_string(),
    }
}
fn prover(path: &Path, value: &Value) {
    fs::write(
        path.join("Prover.toml"),
        value
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| format!("{k}={}\n", toml(v)))
            .collect::<String>(),
    )
    .unwrap();
}
fn execute(g: &support::LiveToolchain, path: &Path) -> String {
    let out = support::live_tool(g, "nargo")
        .arg("execute")
        .current_dir(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "nargo execute failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}
fn project(path: &Path, name: &str, source: &str) {
    fs::create_dir_all(path.join("src")).unwrap();
    fs::write(path.join("Nargo.toml"), format!("[package]\nname=\"{name}\"\ntype=\"bin\"\ncompiler_version=\">=1.0.0\"\n[dependencies]\ncore={{path={:?}}}\ncompose={{path={:?}}}\n", NOIR_CORE, COMPOSE_CORE)).unwrap();
    fs::write(path.join("src/main.nr"), source).unwrap();
}

#[test]
fn random_literals_execute_against_rust_and_engine() {
    let gate = match support::gate(
        "random_literals_execute_against_rust_and_engine",
        &["nargo"],
    ) {
        Some(g) => g,
        None => return,
    };
    assert_compiler(&gate);
    let scratch = Scratch::new();
    let mut workers = [Vec::new(), Vec::new()];
    for (i, fixture) in cases::fixtures().into_iter().enumerate() {
        workers[i % 2].push(fixture);
    }
    std::thread::scope(|scope| {
        for fixtures in workers {
            let gate_ref = &gate;
            let root = &scratch.0;
            scope.spawn(move || {
                for ((datatype, bucket), inputs) in fixtures {
                    let path = root.join(format!("{datatype}-{bucket}"));
                    project(&path, "merkle_keys", &cases::program(&datatype, bucket));
                    for input in inputs {
                        prover(&path, &input);
                        execute(gate_ref, &path);
                    }
                }
            });
        }
    });
    support::complete(gate);
}

#[test]
fn credential_modes_match_vectors_and_compiled_public_abi() {
    let gate = match support::gate(
        "credential_modes_match_vectors_and_compiled_public_abi",
        &["nargo"],
    ) {
        Some(g) => g,
        None => return,
    };
    assert_compiler(&gate);
    let scratch = Scratch::new();
    for mode in ["hidden", "revealed"] {
        let fixture = Path::new(NOIR_FIXTURES).join(mode);
        let path = scratch.0.join(mode);
        project(
            &path,
            &format!("credential_{mode}"),
            &fs::read_to_string(fixture.join("src/main.nr")).unwrap(),
        );
        for (salt, config) in [
            (7, [9; 32]),
            (11, [9; 32]),
            (7, std::array::from_fn(|i| i as u8)),
        ] {
            let (mut input, message) = if salt == 11 {
                credential::input_created(salt, config, Some("2026-01-01T00:00:00Z"))
            } else {
                credential::input(salt, config)
            };
            if mode == "revealed" {
                for k in ["r", "s", "e", "e_k"] {
                    input.as_object_mut().unwrap().remove(k);
                }
            }
            prover(&path, &input);
            let stdout = execute(&gate, &path);
            if mode == "revealed" {
                assert!(
                    stdout.contains(&field_to_hex(&message)),
                    "revealed message differs: {stdout}"
                );
            }
        }
        let artifact: Value = serde_json::from_slice(
            &fs::read(path.join(format!("target/credential_{mode}.json"))).unwrap(),
        )
        .unwrap();
        let public: Vec<_> = artifact["abi"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["visibility"] == "public")
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            public,
            if mode == "hidden" {
                vec!["pk", "config"]
            } else {
                vec!["config", "pk"]
            }
        );
        let return_type = &artifact["abi"]["return_type"];
        if mode == "hidden" {
            assert!(return_type.is_null());
        } else {
            assert_eq!(return_type["visibility"], "public");
            assert_eq!(return_type["abi_type"]["kind"], "field");
        }
        // The only released credential field is independently reconstructed
        // from salted leaves, and changing the private salt changes that field.
        let q = p::hash_quad([Fr::from(1u64); 4]);
        let a = p::signed_message(
            Fr::from(7u64),
            1,
            p::root(&[q], Fr::from(7u64)).unwrap(),
            &[9; 32],
            Fr::from(0u64),
        );
        let b = p::signed_message(
            Fr::from(8u64),
            1,
            p::root(&[q], Fr::from(8u64)).unwrap(),
            &[9; 32],
            Fr::from(0u64),
        );
        assert_ne!(a, b);
    }
    support::complete(gate);
}
