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
#[path = "support/solid_profile.rs"]
mod solid_profile;
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

#[test]
fn opaque_cells_cannot_be_abi_witnesses() {
    let gate = match support::gate("opaque_cells_cannot_be_abi_witnesses", &["nargo"]) {
        Some(g) => g,
        None => return,
    };
    assert_compiler(&gate);
    let scratch = Scratch::new();
    for (name, source) in [
        (
            "cell",
            "fn main(cell: core::term::TermCell) -> pub Field { cell.enc() }",
        ),
        (
            "checked",
            "fn main(cell: core::term::KeyCheckedCell) -> pub Field { cell.cell().enc() }",
        ),
        (
            "credential",
            "fn main(cell: core::suite_metadata::OpenedCredential<1>) -> pub Field { cell.root() }",
        ),
    ] {
        let path = scratch.0.join(name);
        project(&path, name, source);
        let out = support::live_tool(&gate, "nargo")
            .current_dir(&path)
            .args(["compile", "--force"])
            .output()
            .unwrap();
        let diagnostic = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !out.status.success(),
            "opaque {name} was accepted as a witness"
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("Invalid type found in the entry point"),
            "{diagnostic}"
        );
        assert!(diagnostic.contains("[u8; 0]"), "{diagnostic}");
    }
    let path = scratch.0.join("private_cell");
    project(
        &path,
        "private_cell",
        "fn main() -> pub Field { core::term::padding().enc }",
    );
    let out = support::live_tool(&gate, "nargo")
        .current_dir(&path)
        .args(["compile", "--force"])
        .output()
        .unwrap();
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "private cell fields were accessible");
    assert!(String::from_utf8_lossy(&out.stderr).contains("enc is private"), "{diagnostic}");
    support::complete(gate);
}

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

#[test]
fn solid_app_suite_profile_opens_in_revealed_mode() {
    use oxrdf::Term;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    let gate = match support::gate("solid_app_suite_profile_opens_in_revealed_mode", &["nargo"]) {
        Some(g) => g,
        None => return,
    };
    assert_compiler(&gate);
    let key = p::keypair_from_seed(&[7; 32]);
    let c = p::issue_deterministic(
        &solid_profile::document(),
        &solid_profile::configuration(),
        &key,
        &[9; 32],
    )
    .unwrap();
    assert!(p::verify(&c, &key.public_key()));
    let metadata = p::verify_and_open(&c, &key.public_key()).unwrap();
    fn padded(text: &str, capacity: usize) -> Vec<u8> {
        assert!(text.len() <= capacity);
        let mut bytes = text.as_bytes().to_vec();
        bytes.resize(capacity, 0);
        bytes
    }
    fn raw(term: Option<&Term>, l: usize, d: usize, g: usize) -> Value {
        let (kind, text, dt, lang, key) = match term {
            Some(Term::NamedNode(n)) => (1, n.as_str(), "", "", [0; 31]),
            Some(Term::BlankNode(n)) => (2, n.as_str(), "", "", [0; 31]),
            Some(Term::Literal(t)) => (
                3,
                t.value(),
                t.datatype().as_str(),
                t.language().unwrap_or(""),
                sparq_merkle_suite::comparison_key(t),
            ),
            Some(Term::Triple(_)) => panic!("triple terms are not admitted"),
            None => (0, "", "", "", [0; 31]),
        };
        json!({"bound":term.is_some(),"kind":kind,"lexical":padded(text,l),"lexical_len":text.len(),"datatype":padded(dt,d),"datatype_len":dt.len(),"language":padded(lang,g),"language_len":lang.len(),"key":key})
    }
    let rows = oxttl::NQuadsParser::new()
        .for_slice(c.document.as_bytes())
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 42);
    let mut opening = Vec::new();
    for q in rows {
        assert!(q.graph_name.is_default_graph());
        let mut graph = raw(None, 128, 64, 1);
        graph["bound"] = json!(true);
        opening.push(vec![
            raw(Some(&Term::from(q.subject)), 128, 64, 1),
            raw(Some(&Term::NamedNode(q.predicate)), 128, 64, 1),
            raw(Some(&q.object), 128, 64, 1),
            graph,
        ]);
    }
    opening.resize(64, vec![raw(None, 128, 64, 1); 4]);
    let mut salts = (0..c.count)
        .map(|i| field_to_hex(&p::leaf_salt(c.salt, i)))
        .collect::<Vec<_>>();
    salts.resize(64, field_to_hex(&Fr::from(0u64)));
    let config: [u8; 32] = Sha256::digest(c.proof_config.as_bytes()).into();
    let (x, y) = key.public_key().coords().unwrap();
    let message = p::signed_message(c.salt, c.count, c.root, &config, metadata.digest);
    let input = json!({"raw":opening,"salts":salts,"salt":field_to_hex(&c.salt),"count":c.count,
        "config":[u128::from_be_bytes(config[..16].try_into().unwrap()).to_string(),u128::from_be_bytes(config[16..].try_into().unwrap()).to_string()],
        "pk":{"x":field_to_hex(&x),"y":field_to_hex(&y)},
        "created_raw":raw(metadata.created.as_ref(),32,128,64),"method_raw":raw(metadata.verification_method.as_ref(),32,128,64),
        "purpose":padded("revocation",32),"purpose_len":10,"index":padded("0007",32),"index_len":4});
    // Narrow witnessed metadata buffers are widened with constant zero tails;
    // every length and every supplied byte is still checked by the core opener.
    let source = r#"use core::suite_entry::{BoundedTermOpening, TermOpening};
use std::collections::bounded_vec::BoundedVec;
fn widen(a: BoundedTermOpening<128,64,1>) -> TermOpening<128> {
    assert((a.lexical_len<=128)&(a.datatype_len<=64)&(a.language_len<=1));
    let mut dt = [0;128]; for i in 0..64 { dt[i] = a.datatype[i]; }
    let mut lang = [0;64]; lang[0] = a.language[0];
    TermOpening { bound:a.bound,kind:a.kind,lexical:a.lexical,lexical_len:a.lexical_len,
        datatype:dt,datatype_len:a.datatype_len,language:lang,language_len:a.language_len,key:a.key }
}
fn absent() -> TermOpening<128> {
    TermOpening { bound:false,kind:0,lexical:[0;128],lexical_len:0,datatype:[0;128],datatype_len:0,language:[0;64],language_len:0,key:[0;31] }
}
fn main(raw:[[BoundedTermOpening<128,64,1>;4];64],salts:[Field;64],salt:Field,count:u32,
    config:pub [u128;2],pk:pub compose::issuer::Point,created_raw:TermOpening<32>,method_raw:TermOpening<32>,
    purpose:[u8;32],purpose_len:u32,index:[u8;32],index_len:u32) -> pub Field {
    let mut full = [[absent();4];64];
    for i in 0..64 { for j in 0..4 { full[i][j] = widen(raw[i][j]); } }
    let opened = core::suite::open_credential_revealed(full,salts,salt,count,config,created_raw,method_raw,pk,0);
    assert(opened.count()==42);
    assert(opened.created().known());
    let status = opened.property(opened.credential_subject(),core::term::iri_credential_status());
    let purpose_cell = opened.property(status,core::term::iri_status_purpose());
    let index_cell = opened.property(status,core::term::iri_status_index());
    assert(!core::suite_metadata::open_status_purpose(purpose_cell,BoundedVec::from_parts_unchecked(purpose,purpose_len)));
    assert(core::suite_metadata::open_status_index(index_cell,BoundedVec::from_parts_unchecked(index,index_len))==7);
    let message = opened.message(); println(message); message
}"#;
    let scratch = Scratch::new();
    project(&scratch.0, "solid_profile", source);
    prover(&scratch.0, &input);
    let stdout = execute(&gate, &scratch.0);
    assert!(
        stdout.contains(&field_to_hex(&message)),
        "revealed profile message differs: {stdout}"
    );
    support::complete(gate);
}
