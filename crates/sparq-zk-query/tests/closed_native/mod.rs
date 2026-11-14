#![allow(dead_code)]
#[path = "../support/authenticated.rs"]
mod authenticated;
pub fn canonical_policy() -> sparq_zk_query::envelope_policy::PolicyInput {
    authenticated::canonical_policy()
}

pub fn canonical_credential(
    source: &str,
    issuer: &str,
    key: &sparq_zk::sig::SecretKey,
    salt: u64,
) -> sparq_merkle_suite::poseidon::Credential {
    let template = authenticated::canonical_sign(source, salt);
    let document = template
        .credential
        .document
        .replace("<urn:issuer>", &format!("<{issuer}>"));
    sparq_merkle_suite::poseidon::issue(
        &document,
        &template.credential.proof_config,
        key,
        sparq_merkle_suite::Fr::from(salt + 101),
    )
    .unwrap()
}

// Temporary directories are unique across sandbox PID namespaces as well as threads.
pub struct Workspace(pub tempfile::TempDir);
impl Workspace {
    pub fn new(package: &sparq_zk_query::codegen::Package) -> Self {
        let root = tempfile::tempdir().unwrap();
        for (name, body) in &package.files {
            let p = root.path().join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        std::os::unix::fs::symlink(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../zk")
                .canonicalize()
                .unwrap(),
            root.path().join("zk"),
        )
        .unwrap();
        Self(root)
    }
}
