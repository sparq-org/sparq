#![cfg(feature = "authenticated-rdf")]

#[path = "../examples/authrdf_cli/nonces.rs"]
mod nonces;

use nonces::FileNonces;
use serde_json::{Value, json};
use sparq_proved_evaluator::{
    Nonces, Presentation, embedded_authrdf_artifact, embedded_authrdf_pin,
};
use sparq_proved_evaluator_model::{DatasetAuthority, authenticated_rdf as model};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

static CLI: OnceLock<PathBuf> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(0);
const SEED: &str = "0101010101010101010101010101010101010101010101010101010101010101";
const NONCE: &str = "0202020202020202020202020202020202020202020202020202020202020202";
const SALT: &str = "0303030303030303030303030303030303030303030303030303030303030303";

fn cli() -> &'static Path {
    CLI.get_or_init(|| {
        if let Some(binary) = std::env::var_os("AUTHRDF_CLI_TEST_BINARY") {
            return PathBuf::from(binary);
        }
        let status = Command::new(env!("CARGO"))
            .args([
                "build",
                "--locked",
                "--manifest-path",
                concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.toml"),
                "-p",
                "sparq-proved-evaluator",
                "--features",
                "authenticated-rdf",
                "--example",
                "authrdf_cli",
            ])
            .status()
            .unwrap();
        assert!(status.success(), "example must build");
        std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples/authrdf_cli")
    })
}
fn run(args: &[&str]) -> Output {
    Command::new(cli()).args(args).output().unwrap()
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn failure(output: Output, reason: &str) {
    assert!(
        !output.status.success(),
        "unexpected acceptance: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(reason),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}
fn json_file(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn put(path: &Path, value: &impl serde::Serialize) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "authrdf-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn issue(
        &self,
        name: &str,
        issuer: &str,
        method: &str,
        seed: &str,
        predicate: &str,
    ) -> PathBuf {
        let doc = self.path(&format!("{name}.nq"));
        // Deliberately unsorted, with a blank node, to exercise model canonicalization.
        fs::write(&doc, format!("<urn:person> <{predicate}> \"yes\" .\n_:cred <https://www.w3.org/2018/credentials#issuer> <{issuer}> .\n_:cred <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> .\n")).unwrap();
        let out = self.path(&format!("{name}.json"));
        success(run(&[
            "issue",
            "--seed-hex",
            seed,
            "--issuer",
            issuer,
            "--verification-method",
            method,
            "--document",
            text(&doc),
            "--created",
            "2026-01-01T00:00:00Z",
            "--out",
            text(&out),
        ]));
        out
    }
    fn request(&self, credentials: &[PathBuf], query: &str) -> PathBuf {
        let query_path = self.path("query.rq");
        fs::write(&query_path, query).unwrap();
        let keys = self.path("keys.json");
        let entries: Vec<Value> = credentials.iter().map(|p| {
            let c = json_file(p);
            json!({"issuer": c["issuer"], "verification_method": c["verification_method"], "public_key_hex": c["public_key_hex"]})
        }).collect();
        put(&keys, &entries);
        let out = self.path("request.json");
        let stdout = success(run(&[
            "request",
            "--query",
            text(&query_path),
            "--keys",
            text(&keys),
            "--authority",
            "holder-declared",
            "--nonce-hex",
            NONCE,
            "--out",
            text(&out),
        ]));
        let request: model::Request = serde_json::from_value(json_file(&out)).unwrap();
        model::validate_request(&request).unwrap();
        assert_eq!(request.query, query);
        assert_eq!(request.nonce, [2; 32]);
        assert_eq!(
            request.policy.evaluation,
            sparq_proved_evaluator_model::v3::Policy::default()
        );
        let digest: String = model::request_digest(&request)
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(stdout, json!({"request_digest": digest}));
        out
    }
    fn guest(&self) -> PathBuf {
        let dir = self.path("guest");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("guest.bin"), embedded_authrdf_artifact()).unwrap();
        put(&dir.join("pin.json"), &embedded_authrdf_pin());
        dir
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn signed(value: &Value) -> model::SignedCredential {
    let sig = value["signature_hex"].as_str().unwrap();
    model::SignedCredential {
        document: value["document"].as_str().unwrap().into(),
        proof_config: value["proof_config"].as_str().unwrap().into(),
        signature: sig
            .as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect(),
    }
}

#[test]
fn issued_credentials_and_request_authenticate_and_evaluate() {
    let f = Fixture::new();
    let a = f.issue("a", "urn:issuer:a", "urn:key:a", SEED, "urn:birth");
    let b = f.issue("b", "urn:issuer:b", "urn:key:b", NONCE, "urn:valid");
    let request_path = f.request(
        &[a.clone(), b.clone()],
        "ASK { ?p <urn:birth> \"yes\" . ?p <urn:valid> \"yes\" }",
    );
    // Valid inputs reach guest loading without invoking a prover.
    failure(
        run(&[
            "prove",
            "--request",
            text(&request_path),
            "--credentials",
            text(&a),
            text(&b),
            "--salt-hex",
            SALT,
            "--guest",
            "/missing",
            "--r0vm",
            "/missing",
            "--out",
            text(&f.path("proof.json")),
        ]),
        "input file read failed",
    );
    let request = serde_json::from_value(json_file(&request_path)).unwrap();
    let journal = model::evaluate(&model::Witness {
        request,
        dataset: model::PrivateCredentials {
            credentials: vec![signed(&json_file(&a)), signed(&json_file(&b))],
            salt: [3; 32],
        },
    })
    .unwrap();
    assert_eq!(
        journal.result,
        sparq_proved_evaluator_model::v3::CanonicalResult::Ask(true)
    );
    // An independently changed signature must fail native authentication.
    let mut credential = signed(&json_file(&a));
    credential.signature[0] ^= 1;
    let request: model::Request = serde_json::from_value(json_file(&request_path)).unwrap();
    assert!(
        model::dataset_commitment(
            &model::PrivateCredentials {
                credentials: vec![credential],
                salt: [3; 32]
            },
            &request.policy
        )
        .is_err()
    );
}

#[test]
fn strict_arguments_and_hex_reject_before_file_access() {
    let f = Fixture::new();
    f.issue("a", "urn:issuer:a", "urn:key:a", SEED, "urn:birth");
    for invalid in [
        "ab",
        &"00".repeat(33),
        &"AB".repeat(32),
        &"gg".repeat(32),
        &"é".repeat(32),
    ] {
        let out = f.path("invalid.json");
        failure(
            run(&[
                "issue",
                "--seed-hex",
                invalid,
                "--issuer",
                "urn:issuer:a",
                "--verification-method",
                "urn:key:a",
                "--document",
                "/missing",
                "--out",
                text(&out),
            ]),
            "hex",
        );
        failure(
            run(&[
                "request",
                "--nonce-hex",
                invalid,
                "--authority",
                "holder-declared",
                "--query",
                "/missing",
                "--keys",
                "/missing",
                "--out",
                text(&out),
            ]),
            "hex",
        );
        failure(
            run(&[
                "prove",
                "--salt-hex",
                invalid,
                "--request",
                "/missing",
                "--credentials",
                "/missing",
                "--guest",
                "/missing",
                "--r0vm",
                "/missing",
                "--out",
                text(&out),
            ]),
            "hex",
        );
        assert!(!out.exists());
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let output = Command::new(cli())
            .arg("verify")
            .arg(std::ffi::OsString::from_vec(vec![0xff]))
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["reason"],
            "non-Unicode argument rejected"
        );
    }
    for extra in [["--seed-hex", NONCE], ["--unknown", "x"]] {
        let out = f.path("rejected.json");
        failure(
            run(&[
                "issue",
                "--seed-hex",
                SEED,
                "--issuer",
                "urn:issuer:a",
                "--verification-method",
                "urn:key:a",
                "--document",
                text(&f.path("a.nq")),
                "--out",
                text(&out),
                extra[0],
                extra[1],
            ]),
            "unknown or duplicate option",
        );
        assert!(!out.exists());
    }
    for args in [
        vec!["issue", "--seed-hex"],
        vec!["issue", "--unknown", "x"],
        vec!["issue", "--seed-hex", SEED, "--seed-hex", NONCE],
        vec!["issue", "stray"],
        vec!["digest-query"],
    ] {
        assert!(!run(&args).status.success());
    }
    failure(
        run(&[
            "request",
            "--nonce-hex",
            NONCE,
            "--authority",
            "verifier-agreed",
        ]),
        "anchor",
    );
    failure(
        run(&[
            "request",
            "--nonce-hex",
            NONCE,
            "--authority",
            "verifier-agreed:ab",
        ]),
        "hex",
    );
    failure(
        run(&[
            "request",
            "--nonce-hex",
            NONCE,
            "--authority",
            "holder-declared",
            "--signature-mode",
            "other",
        ]),
        "signature mode",
    );
}

#[test]
fn output_files_are_never_overwritten() {
    let f = Fixture::new();
    let c = f.issue("a", "urn:issuer:a", "urn:key:a", SEED, "urn:birth");
    let original = fs::read(&c).unwrap();
    #[cfg(unix)]
    {
        let link = f.path("dangling.json");
        std::os::unix::fs::symlink(f.path("missing-target"), &link).unwrap();
        failure(
            run(&[
                "issue",
                "--seed-hex",
                SEED,
                "--issuer",
                "urn:issuer:a",
                "--verification-method",
                "urn:key:a",
                "--document",
                text(&f.path("a.nq")),
                "--out",
                text(&link),
            ]),
            "output already exists",
        );
        assert!(!f.path("missing-target").exists());
    }
    failure(
        run(&[
            "issue",
            "--seed-hex",
            SEED,
            "--issuer",
            "urn:issuer:a",
            "--verification-method",
            "urn:key:a",
            "--document",
            text(&f.path("a.nq")),
            "--out",
            text(&c),
        ]),
        "output already exists",
    );
    assert_eq!(fs::read(&c).unwrap(), original);
    let request = f.request(std::slice::from_ref(&c), "ASK {}");
    let original = fs::read(&request).unwrap();
    failure(
        run(&[
            "request",
            "--nonce-hex",
            NONCE,
            "--authority",
            "holder-declared",
            "--query",
            text(&f.path("query.rq")),
            "--keys",
            text(&f.path("keys.json")),
            "--out",
            text(&request),
        ]),
        "output already exists",
    );
    assert_eq!(fs::read(&request).unwrap(), original);
    let out = f.path("presentation.json");
    fs::write(&out, "preserve").unwrap();
    failure(
        run(&[
            "prove",
            "--salt-hex",
            SALT,
            "--request",
            text(&request),
            "--credentials",
            text(&c),
            "--guest",
            "/missing",
            "--r0vm",
            "/missing",
            "--out",
            text(&out),
        ]),
        "output already exists",
    );
    assert_eq!(fs::read_to_string(out).unwrap(), "preserve");
}

#[test]
fn unadmitted_documents_and_created_are_refused() {
    let f = Fixture::new();
    let c = f.issue("a", "urn:issuer:a", "urn:key:a", SEED, "urn:birth");
    let optional = f.path("optional.json");
    success(run(&[
        "issue",
        "--seed-hex",
        SEED,
        "--issuer",
        "urn:issuer:a",
        "--verification-method",
        "urn:key:a",
        "--document",
        text(&f.path("a.nq")),
        "--out",
        text(&optional),
    ]));
    assert!(
        !json_file(&optional)["proof_config"]
            .as_str()
            .unwrap()
            .contains("created")
    );
    for (document, created, reason) in [
        (
            "<urn:a> <urn:b> <urn:c> <urn:graph> .",
            "2026-01-01T00:00:00Z",
            "default graph",
        ),
        (
            json_file(&c)["document"].as_str().unwrap(),
            "not-a-date",
            "created",
        ),
    ] {
        fs::write(f.path("bad.nq"), document).unwrap();
        failure(
            run(&[
                "issue",
                "--seed-hex",
                SEED,
                "--issuer",
                "urn:issuer:a",
                "--verification-method",
                "urn:key:a",
                "--document",
                text(&f.path("bad.nq")),
                "--created",
                created,
                "--out",
                text(&f.path("bad.json")),
            ]),
            reason,
        );
        assert!(!f.path("bad.json").exists());
    }
}

#[test]
fn unauthorized_credentials_refuse_before_guest_or_prover_access() {
    let f = Fixture::new();
    let c = f.issue("a", "urn:issuer:a", "urn:key:a", SEED, "urn:birth");
    let request = f.request(std::slice::from_ref(&c), "ASK {}");
    let original = json_file(&c);
    for field in [
        "verification_method",
        "proof_config",
        "signature_hex",
        "public_key_hex",
    ] {
        let mut changed = original.clone();
        changed[field] = match field {
            "verification_method" => json!("urn:key:unknown"),
            "proof_config" => json!(
                original[field]
                    .as_str()
                    .unwrap()
                    .replace("urn:key:a", "urn:key:unknown")
            ),
            "signature_hex" => json!("AB".repeat(64)),
            _ => json!("ab"),
        };
        put(&c, &changed);
        let output = run(&[
            "prove",
            "--request",
            text(&request),
            "--credentials",
            text(&c),
            "--salt-hex",
            SALT,
            "--guest",
            "/missing",
            "--r0vm",
            "/missing",
            "--out",
            text(&f.path("proof.json")),
        ]);
        failure(
            output,
            match field {
                "verification_method" => "not authorized",
                "proof_config" => "verification method",
                _ => "hex",
            },
        );
        assert!(!f.path("proof.json").exists());
    }
    let mut changed = original;
    changed["signature_hex"] = json!("00".repeat(64));
    put(&c, &changed);
    failure(
        run(&[
            "prove",
            "--request",
            text(&request),
            "--credentials",
            text(&c),
            "--salt-hex",
            SALT,
            "--guest",
            "/missing",
            "--r0vm",
            "/missing",
            "--out",
            text(&f.path("proof.json")),
        ]),
        "signature verification failed",
    );
}

#[test]
fn query_profile_and_key_hex_are_checked() {
    let f = Fixture::new();
    let c = f.issue("a", "urn:issuer:a", "urn:key:a", SEED, "urn:birth");
    f.request(std::slice::from_ref(&c), "ASK {}");
    fs::remove_file(f.path("request.json")).unwrap();
    for query in ["DESCRIBE <urn:p>", "ASK FROM <urn:g> {}", "not sparql"] {
        fs::write(f.path("query.rq"), query).unwrap();
        let out = run(&[
            "request",
            "--query",
            text(&f.path("query.rq")),
            "--keys",
            text(&f.path("keys.json")),
            "--authority",
            "holder-declared",
            "--nonce-hex",
            NONCE,
            "--out",
            text(&f.path("request.json")),
        ]);
        assert!(!out.status.success());
        assert!(!f.path("request.json").exists());
    }
    let mut keys = json_file(&f.path("keys.json"));
    keys[0]["public_key_hex"] = json!("AB".repeat(32));
    put(&f.path("keys.json"), &keys);
    failure(
        run(&[
            "request",
            "--query",
            "/missing",
            "--keys",
            text(&f.path("keys.json")),
            "--authority",
            "holder-declared",
            "--nonce-hex",
            NONCE,
            "--out",
            text(&f.path("request.json")),
        ]),
        "hex",
    );
}

#[test]
fn revealed_mode_and_agreed_anchor_are_bound_in_request() {
    let f = Fixture::new();
    let c = f.issue("a", "urn:issuer:a", "urn:key:a", SEED, "urn:birth");
    f.request(std::slice::from_ref(&c), "ASK {}");
    fs::remove_file(f.path("request.json")).unwrap();
    success(run(&[
        "request",
        "--query",
        text(&f.path("query.rq")),
        "--keys",
        text(&f.path("keys.json")),
        "--authority",
        &format!("verifier-agreed:{SALT}"),
        "--nonce-hex",
        NONCE,
        "--signature-mode",
        "revealed",
        "--out",
        text(&f.path("request.json")),
    ]));
    let request: model::Request =
        serde_json::from_value(json_file(&f.path("request.json"))).unwrap();
    assert_eq!(
        request.policy.signature_mode,
        model::SignatureMode::Revealed
    );
    assert_eq!(
        request.authority,
        DatasetAuthority::VerifierAgreed {
            commitment: [3; 32]
        }
    );
    let mut request = request;
    request.authority = DatasetAuthority::HolderDeclared;
    put(&f.path("request.json"), &request);
    failure(
        run(&[
            "prove",
            "--request",
            text(&f.path("request.json")),
            "--credentials",
            text(&c),
            "--salt-hex",
            SALT,
            "--guest",
            "/missing",
            "--r0vm",
            "/missing",
            "--out",
            text(&f.path("proof.json")),
        ]),
        "input file read failed",
    );
    failure(
        run(&[
            "verify",
            "--request",
            text(&f.path("request.json")),
            "--presentation",
            "/missing",
            "--guest",
            "/missing",
            "--nonce-store",
            text(&f.path("nonces.json")),
        ]),
        "signatures are required",
    );
    put(&f.path("signatures.json"), &vec!["AB".repeat(64)]);
    failure(
        run(&[
            "verify",
            "--request",
            text(&f.path("request.json")),
            "--presentation",
            "/missing",
            "--guest",
            "/missing",
            "--nonce-store",
            text(&f.path("nonces.json")),
            "--signatures",
            text(&f.path("signatures.json")),
        ]),
        "hex",
    );
}

#[test]
fn fake_presentation_is_rejected_without_consuming_nonce() {
    use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
    let f = Fixture::new();
    let c = f.issue("a", "urn:issuer:a", "urn:key:a", SEED, "urn:birth");
    let request = f.request(std::slice::from_ref(&c), "ASK {}");
    let mut r: model::Request = serde_json::from_value(json_file(&request)).unwrap();
    let journal = model::evaluate(&model::Witness {
        request: r.clone(),
        dataset: model::PrivateCredentials {
            credentials: vec![signed(&json_file(&c))],
            salt: [3; 32],
        },
    })
    .unwrap();
    let encoded = risc0_zkvm::serde::to_vec(&journal)
        .unwrap()
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect::<Vec<_>>();
    let pin = embedded_authrdf_pin();
    let receipt = Receipt::new(
        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
            pin.image_id,
            encoded.clone(),
        ))),
        encoded,
    );
    let p = f.path("fake.json");
    put(&p, &Presentation { receipt });
    let guest = f.guest();
    let store = f.path("nonces.json");
    put(
        &f.path("signatures.json"),
        &vec![json_file(&c)["signature_hex"].as_str().unwrap()],
    );
    for mode in [model::SignatureMode::Hidden, model::SignatureMode::Revealed] {
        r.policy.signature_mode = mode;
        put(&request, &r);
        let mut command = Command::new(cli());
        command.env("RISC0_DEV_MODE", "1").args([
            "verify",
            "--presentation",
            text(&p),
            "--request",
            text(&request),
            "--guest",
            text(&guest),
            "--nonce-store",
            text(&store),
        ]);
        if mode == model::SignatureMode::Revealed {
            command.args(["--signatures", text(&f.path("signatures.json"))]);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["verified"], false);
        assert!(response["reason"].as_str().unwrap().contains("succinct"));
        assert!(!store.exists());
        assert!(!f.path("nonces.json.lock").exists());
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!combined.contains("urn:birth"));
    }
    let error = run(&["verify", "--unknown", "secret-witness"]);
    assert_eq!(
        serde_json::from_slice::<Value>(&error.stdout).unwrap()["verified"],
        false
    );
    assert!(!String::from_utf8_lossy(&error.stderr).contains("secret-witness"));
}

// Child process entry point exercises the production store without forging a
// verified receipt or adding a verification bypass to the example.
#[test]
fn nonce_worker() {
    if let Some(path) = std::env::var_os("AUTHRDF_NONCE_TEST_PATH") {
        let nonce = std::env::var("AUTHRDF_NONCE_TEST_VALUE")
            .unwrap()
            .parse::<u8>()
            .unwrap();
        let result = FileNonces(PathBuf::from(path)).consume([nonce; 32]);
        println!("nonce-result={result:?}");
    }
}
fn worker(path: &Path, nonce: u8) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "nonce_worker", "--nocapture"])
        .env("AUTHRDF_NONCE_TEST_PATH", path)
        .env("AUTHRDF_NONCE_TEST_VALUE", nonce.to_string());
    command
}
fn worker_result(output: Output, result: &str) {
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(&format!("nonce-result={result}")),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn nonce_store_persists_and_refuses_replay_across_processes() {
    let f = Fixture::new();
    let store = f.path("nonces.json");
    worker_result(worker(&store, 7).output().unwrap(), "Ok(true)");
    let first = fs::read(&store).unwrap();
    let mut original_inode = fs::File::open(&store).unwrap();
    worker_result(worker(&store, 7).output().unwrap(), "Ok(false)");
    assert_eq!(fs::read(&store).unwrap(), first);
    worker_result(worker(&store, 8).output().unwrap(), "Ok(true)");
    let mut original_bytes = Vec::new();
    std::io::Read::read_to_end(&mut original_inode, &mut original_bytes).unwrap();
    assert_eq!(
        original_bytes, first,
        "updates must replace the inode atomically"
    );
    worker_result(worker(&store, 7).output().unwrap(), "Ok(false)");
    assert_eq!(
        serde_json::from_slice::<Vec<[u8; 32]>>(&fs::read(&store).unwrap()).unwrap(),
        vec![[7; 32], [8; 32]]
    );
}

#[test]
fn nonce_store_serializes_concurrent_writers_and_fails_closed() {
    let f = Fixture::new();
    let store = f.path("nonces.json");
    let children = (0..8)
        .map(|_| {
            worker(&store, 9)
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let outputs = children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .collect::<Vec<_>>();
    assert!(outputs.iter().all(|o| o.status.success()));
    assert_eq!(
        outputs
            .iter()
            .filter(|o| String::from_utf8_lossy(&o.stdout).contains("nonce-result=Ok(true)"))
            .count(),
        1
    );
    assert_eq!(
        outputs
            .iter()
            .filter(|o| String::from_utf8_lossy(&o.stdout).contains("nonce-result=Ok(false)"))
            .count(),
        7
    );
    fs::write(&store, "corrupt").unwrap();
    worker_result(
        worker(&store, 10).output().unwrap(),
        "Err(Error(\"nonce storage failed\"))",
    );
    assert_eq!(fs::read_to_string(&store).unwrap(), "corrupt");
    let missing_parent = f.path("missing/nonces.json");
    worker_result(
        worker(&missing_parent, 10).output().unwrap(),
        "Err(Error(\"nonce storage failed\"))",
    );
    #[cfg(unix)]
    {
        let victim = f.path("victim");
        fs::write(&victim, "preserve").unwrap();
        fs::remove_file(&store).unwrap();
        std::os::unix::fs::symlink(&victim, &store).unwrap();
        worker_result(
            worker(&store, 10).output().unwrap(),
            "Err(Error(\"nonce storage failed\"))",
        );
        assert_eq!(fs::read_to_string(&victim).unwrap(), "preserve");
    }
}
