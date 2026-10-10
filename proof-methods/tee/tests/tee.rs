//! TEE attestation round trips and rejections against a test certificate
//! authority shaped like the Nitro one (P-384 root, intermediate, leaf; ES384
//! COSE_Sign1 documents). Real Nitro documents are checked on Nitro hardware.

use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ciborium::Value;
use der::Encode;
use p384::ecdsa::signature::Signer;
use p384::ecdsa::{DerSignature, Signature, SigningKey};
use rand_core::OsRng;
use sparq_proved_evaluator_model::DatasetAuthority;
use sparq_proved_evaluator_model::authenticated_rdf::{self as auth, Request};
use sparq_proved_evaluator_model::v3::CanonicalResult;
use sparq_vcq_disclosed::fixtures::{self, QUERIES};
use sparq_vcq_tee::protocol::{self, Attester, EnclaveRequest, EnclaveResponse};
use sparq_vcq_tee::{Rejected, Statement, TeeProof, TrustPolicy, nitro, verify};
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::spki::SubjectPublicKeyInfoOwned;
use x509_cert::time::Validity;

const SALT: [u8; 32] = [0xa5; 32];
const NONCE: [u8; 32] = [7; 32];
const PCR0: [u8; 48] = [0x42; 48];
const MAX_AGE_MS: u64 = 5 * 60 * 1000;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn cert(profile: Profile, subject: &str, key: &SigningKey, signer: &SigningKey) -> Vec<u8> {
    let spki = SubjectPublicKeyInfoOwned::from_key(*key.verifying_key()).unwrap();
    let builder = CertificateBuilder::new(
        profile,
        SerialNumber::from(1u32),
        Validity::from_now(Duration::from_secs(3600)).unwrap(),
        Name::from_str(subject).unwrap(),
        spki,
        signer,
    )
    .unwrap();
    builder.build::<DerSignature>().unwrap().to_der().unwrap()
}

/// A root, an intermediate and a leaf key, and documents signed by the leaf.
struct TestNitro {
    root: Vec<u8>,
    intermediate: Vec<u8>,
    leaf: Vec<u8>,
    leaf_key: SigningKey,
    pcr0: [u8; 48],
    timestamp: Option<u64>,
}

impl TestNitro {
    fn new() -> Self {
        let root_key = SigningKey::random(&mut OsRng);
        let mid_key = SigningKey::random(&mut OsRng);
        let leaf_key = SigningKey::random(&mut OsRng);
        let root = cert(Profile::Root, "CN=test-root", &root_key, &root_key);
        let intermediate = cert(
            Profile::SubCA {
                issuer: Name::from_str("CN=test-root").unwrap(),
                path_len_constraint: None,
            },
            "CN=test-intermediate",
            &mid_key,
            &root_key,
        );
        let leaf = cert(
            Profile::Leaf {
                issuer: Name::from_str("CN=test-intermediate").unwrap(),
                enable_key_agreement: false,
                enable_key_encipherment: false,
            },
            "CN=test-enclave",
            &leaf_key,
            &mid_key,
        );
        Self {
            root,
            intermediate,
            leaf,
            leaf_key,
            pcr0: PCR0,
            timestamp: None,
        }
    }

    fn policy(&self) -> TrustPolicy {
        TrustPolicy {
            root: self.root.clone(),
            pcr0: vec![PCR0],
            max_age_ms: MAX_AGE_MS,
        }
    }

    fn payload(&self, user_data: &[u8], nonce: &[u8]) -> Value {
        let text = |s: &str| Value::Text(s.into());
        Value::Map(vec![
            (text("module_id"), text("i-test-enc0")),
            (text("digest"), text("SHA384")),
            (
                text("timestamp"),
                Value::Integer(self.timestamp.unwrap_or_else(now_ms).into()),
            ),
            (
                text("pcrs"),
                Value::Map(vec![
                    (Value::Integer(0.into()), Value::Bytes(self.pcr0.to_vec())),
                    (Value::Integer(1.into()), Value::Bytes(vec![0x11; 48])),
                ]),
            ),
            (text("certificate"), Value::Bytes(self.leaf.clone())),
            (
                text("cabundle"),
                Value::Array(vec![
                    Value::Bytes(self.root.clone()),
                    Value::Bytes(self.intermediate.clone()),
                ]),
            ),
            (text("public_key"), Value::Null),
            (text("user_data"), Value::Bytes(user_data.to_vec())),
            (text("nonce"), Value::Bytes(nonce.to_vec())),
        ])
    }

    fn sign(&self, payload: &Value, algorithm: i64) -> Vec<u8> {
        let encode = |v: &Value| {
            let mut out = Vec::new();
            ciborium::into_writer(v, &mut out).unwrap();
            out
        };
        let protected = encode(&Value::Map(vec![(
            Value::Integer(1.into()),
            Value::Integer(algorithm.into()),
        )]));
        let payload = encode(payload);
        let tbs = encode(&Value::Array(vec![
            Value::Text("Signature1".into()),
            Value::Bytes(protected.clone()),
            Value::Bytes(Vec::new()),
            Value::Bytes(payload.clone()),
        ]));
        let signature: Signature = self.leaf_key.sign(&tbs);
        encode(&Value::Tag(
            18,
            Box::new(Value::Array(vec![
                Value::Bytes(protected),
                Value::Map(Vec::new()),
                Value::Bytes(payload),
                Value::Bytes(signature.to_bytes().to_vec()),
            ])),
        ))
    }
}

impl Attester for TestNitro {
    fn attest(&mut self, user_data: &[u8; 32], nonce: &[u8; 32]) -> Result<Vec<u8>, Rejected> {
        Ok(self.sign(&self.payload(user_data, nonce), -35))
    }
}

fn query(id: &str) -> &'static str {
    QUERIES.iter().find(|q| q.id == id).expect("query id").text
}

fn present(nitro: &mut TestNitro, request: &Request, n: usize) -> (Statement, TeeProof) {
    let message = serde_json::to_vec(&EnclaveRequest {
        request: request.clone(),
        credentials: fixtures::credentials(n),
        salt: SALT,
    })
    .unwrap();
    match protocol::handle(&message, nitro) {
        EnclaveResponse::Presentation { statement, proof } => (statement, proof),
        EnclaveResponse::Rejected(reason) => panic!("{reason}"),
    }
}

#[test]
fn every_query_round_trips_for_one_and_four_credentials() {
    let mut nitro = TestNitro::new();
    let policy = nitro.policy();
    for n in [1, 4] {
        for q in QUERIES {
            let request = fixtures::request(q.text, DatasetAuthority::HolderDeclared, NONCE);
            let (statement, proof) = present(&mut nitro, &request, n);
            assert_eq!(
                verify(&request, &statement, &proof, &policy, now_ms()).unwrap(),
                statement
            );
            // Same statement as disclosed re-evaluation over the same input.
            let (disclosed, _) =
                sparq_vcq_disclosed::present(&request, fixtures::credentials(n), SALT).unwrap();
            assert_eq!(statement, disclosed, "{} n={n}", q.id);
        }
    }
}

#[test]
fn embedded_aws_root_matches_its_published_fingerprint() {
    assert!(!nitro::aws_root_g1().is_empty());
}

#[test]
fn rejections() {
    let mut nitro = TestNitro::new();
    let policy = nitro.policy();
    let request = fixtures::request(query("Q2"), DatasetAuthority::HolderDeclared, NONCE);
    let (statement, proof) = present(&mut nitro, &request, 1);
    let now = now_ms();
    let reject = |statement: &Statement, proof: &TeeProof, policy: &TrustPolicy, now: u64| {
        verify(&request, statement, proof, policy, now)
            .unwrap_err()
            .0
    };

    // A changed result no longer matches user_data.
    let mut changed = statement.clone();
    let CanonicalResult::Select { rows, .. } = &mut changed.result else {
        panic!()
    };
    rows.pop();
    assert_eq!(
        reject(&changed, &proof, &policy, now),
        "attestation does not bind the stated statement"
    );

    // A request with another nonce does not match the statement's request digest.
    let other = fixtures::request(query("Q2"), DatasetAuthority::HolderDeclared, [8; 32]);
    assert!(verify(&other, &statement, &proof, &policy, now).is_err());

    // Another trust anchor (the real AWS root) rejects the test chain.
    let aws = TrustPolicy {
        root: nitro::aws_root_g1(),
        ..policy.clone()
    };
    assert_eq!(
        reject(&statement, &proof, &aws, now),
        "attestation chain does not start at the trusted root"
    );

    // An enclave image the verifier does not accept.
    let strict = TrustPolicy {
        pcr0: vec![[0x43; 48]],
        ..policy.clone()
    };
    assert_eq!(
        reject(&statement, &proof, &strict, now),
        "enclave image is not accepted"
    );

    // Freshness.
    assert_eq!(
        reject(&statement, &proof, &policy, now + MAX_AGE_MS + 120_000),
        "attestation is too old"
    );
    assert_eq!(
        reject(&statement, &proof, &policy, now - 120_000),
        "attestation is dated in the future"
    );

    // Any changed byte of the document fails to parse or to verify.
    let mut tampered = proof.clone();
    let middle = tampered.attestation.len() / 2;
    tampered.attestation[middle] ^= 1;
    assert!(verify(&request, &statement, &tampered, &policy, now).is_err());

    // A document over the right digest but the wrong nonce.
    let digest = sparq_vcq_tee::statement_digest(&statement);
    let wrong_nonce = TeeProof {
        attestation: nitro.sign(&nitro.payload(&digest, &[9; 32]), -35),
        ..proof.clone()
    };
    assert_eq!(
        reject(&statement, &wrong_nonce, &policy, now),
        "attestation does not bind the request nonce"
    );

    // A document whose protected header names another algorithm.
    let es256 = TeeProof {
        attestation: nitro.sign(&nitro.payload(&digest, &NONCE), -7),
        ..proof.clone()
    };
    assert_eq!(
        reject(&statement, &es256, &policy, now),
        "attestation is not signed with ES384"
    );

    // A debug-mode enclave reports all-zero PCRs.
    nitro.pcr0 = [0; 48];
    let debug = TeeProof {
        attestation: nitro.sign(&nitro.payload(&digest, &NONCE), -35),
        ..proof.clone()
    };
    assert_eq!(
        reject(&statement, &debug, &policy, now),
        "attestation is from a debug-mode enclave"
    );
    nitro.pcr0 = PCR0;

    // A leaf signed by a key outside the chain.
    let mut forged = TestNitro::new();
    forged.root = nitro.root.clone();
    forged.intermediate = nitro.intermediate.clone();
    let forged_doc = TeeProof {
        attestation: forged.sign(&forged.payload(&digest, &NONCE), -35),
        ..proof.clone()
    };
    assert_eq!(
        reject(&statement, &forged_doc, &policy, now),
        "certificate signature does not verify"
    );

    // A document dated outside the certificates' validity.
    nitro.timestamp = Some(now + 2 * 3600 * 1000);
    let expired = TeeProof {
        attestation: nitro.sign(&nitro.payload(&digest, &NONCE), -35),
        ..proof.clone()
    };
    assert_eq!(
        reject(&statement, &expired, &policy, now + 2 * 3600 * 1000),
        "attestation certificate is not valid at the document's time"
    );
    nitro.timestamp = None;

    // Oversized documents are rejected before parsing.
    let huge = TeeProof {
        attestation: vec![0; nitro::MAX_DOCUMENT_BYTES + 1],
        ..proof.clone()
    };
    assert_eq!(
        reject(&statement, &huge, &policy, now),
        "attestation document capacity"
    );

    // Version, platform and signature mode.
    let v2 = TeeProof {
        version: 2,
        ..proof.clone()
    };
    assert_eq!(
        reject(&statement, &v2, &policy, now),
        "unsupported TEE proof version"
    );
    let sev = TeeProof {
        platform: "amd-sev-snp".into(),
        ..proof.clone()
    };
    assert_eq!(
        reject(&statement, &sev, &policy, now),
        "unsupported TEE platform"
    );
    let mut revealed = request.clone();
    revealed.policy = revealed
        .policy
        .with_signature_mode(auth::SignatureMode::Revealed);
    assert_eq!(
        verify(&revealed, &statement, &proof, &policy, now)
            .unwrap_err()
            .0,
        "TEE attestation supports the hidden signature mode only"
    );
}

#[test]
fn enclave_rejects_what_disclosed_rejects() {
    let mut nitro = TestNitro::new();
    let request = fixtures::request(
        "DESCRIBE <https://bank.example/payments/2026-06>",
        DatasetAuthority::HolderDeclared,
        NONCE,
    );
    let message = serde_json::to_vec(&EnclaveRequest {
        request,
        credentials: fixtures::credentials(1),
        salt: SALT,
    })
    .unwrap();
    assert!(matches!(
        protocol::handle(&message, &mut nitro),
        EnclaveResponse::Rejected(_)
    ));
    assert!(matches!(
        protocol::handle(b"not json", &mut nitro),
        EnclaveResponse::Rejected(_)
    ));
}

#[test]
fn messages_are_length_prefixed_and_bounded() {
    let mut buffer = Vec::new();
    protocol::write_message(&mut buffer, b"hello").unwrap();
    assert_eq!(
        protocol::read_message(&mut buffer.as_slice()).unwrap(),
        b"hello"
    );
    let too_long = ((protocol::MAX_MESSAGE_BYTES + 1) as u32).to_be_bytes();
    assert!(protocol::read_message(&mut too_long.as_slice()).is_err());
}
