//! [GPT-6] Synthetic API checks; this harness never installs or publishes artifacts.

pub use rzup::{Result, RzupError};

// Compile the unchanged upstream signature implementation with synthetic keys.
// These test-only RSA dependencies never enter either evaluator dependency graph.
mod upstream_signature {
    include!(concat!(
        env!("SPARQ_SDK_VENDOR_ROOT"),
        "/rzup-0.5.2/src/distribution/signature.rs"
    ));
}

#[test]
fn discovery_reads_only_the_synthetic_toolchain_catalog() {
    let root = std::path::PathBuf::from(std::env::var_os("RISC0_HOME").unwrap());
    assert!(root.starts_with(std::env::var_os("SPARQ_SDK_SMOKE_ROOT").unwrap()));
    let tool = rzup::Rzup::new().unwrap();
    assert!(tool
        .get_default_version(&rzup::Component::RustToolchain)
        .unwrap()
        .is_none());
    let version_dir = root.join(format!(
        "toolchains/v1.97.0-rust-{}-{}",
        std::env::consts::ARCH,
        if cfg!(target_os = "macos") {
            "apple-darwin"
        } else {
            "unknown-linux-gnu"
        }
    ));
    std::fs::create_dir_all(&version_dir).unwrap();
    std::fs::write(
        root.join("settings.toml"),
        "[default_versions]\nrust = \"1.97.0\"\n",
    )
    .unwrap();
    let tool = rzup::Rzup::new().unwrap();
    let (version, actual) = tool
        .get_default_version(&rzup::Component::RustToolchain)
        .unwrap()
        .unwrap();
    assert_eq!(version, rzup::Version::new(1, 97, 0));
    assert_eq!(actual, version_dir);
}

#[test]
fn unchanged_signature_code_rejects_wrong_message_key_and_encoding() {
    use upstream_signature::{PrivateKey, PublicKey, Signature};
    let mut rng = rand::thread_rng();
    let key: PrivateKey = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap().into();
    let other: PrivateKey = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap().into();
    let signature = key.sign(b"synthetic SDK fixture");
    key.public_key()
        .verify(b"synthetic SDK fixture", &signature)
        .unwrap();
    assert!(key
        .public_key()
        .verify(b"tampered SDK fixture", &signature)
        .is_err());
    assert!(other
        .public_key()
        .verify(b"synthetic SDK fixture", &signature)
        .is_err());
    assert!(PublicKey::official()
        .verify(b"synthetic SDK fixture", &signature)
        .is_err());
    assert!("invalid-hex".parse::<Signature>().is_err());
    assert!(PrivateKey::new("not a private key").is_err());
}

#[test]
fn constraint_layer_captures_the_span_and_preserves_constraint_outcomes() {
    use ark_bn254::Fr;
    use ark_relations::{
        lc,
        r1cs::{ConstraintLayer, ConstraintSystem, ConstraintTrace, TracingMode},
    };
    use tracing_subscriber::prelude::*;
    let subscriber = tracing_subscriber::Registry::default()
        .with(ConstraintLayer::new(TracingMode::OnlyConstraints));
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!(target: "r1cs", "sdk_constraint_fixture");
        let _entered = span.enter();
        let trace = ConstraintTrace::capture().expect("active constraint span must be captured");
        assert!(trace.to_string().contains("sdk_constraint_fixture"));
        for expected in [6u64, 7] {
            let cs = ConstraintSystem::<Fr>::new_ref();
            let a = cs.new_witness_variable(|| Ok(Fr::from(2u64))).unwrap();
            let b = cs.new_witness_variable(|| Ok(Fr::from(3u64))).unwrap();
            let result = cs.new_input_variable(|| Ok(Fr::from(expected))).unwrap();
            cs.enforce_constraint(lc!() + a, lc!() + b, lc!() + result)
                .unwrap();
            assert_eq!(cs.is_satisfied().unwrap(), expected == 6);
        }
    });
}
