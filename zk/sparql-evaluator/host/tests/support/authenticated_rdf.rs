// [OPUS-5.5] zkp-14.5: public W3C fixture shared by the V5 host and guest tests.
//! Published W3C vector: vc-di-eddsa Recommendation 2025-05-15, the
//! `eddsa-rdfc-2022` representation, examples 7, 9, 10, 12, 13 and 15
//! (<https://www.w3.org/TR/vc-di-eddsa/>). The bytes, public key and signature
//! equal those in `model/tests/authenticated_rdf.rs`. Data only: no secret key is
//! involved, and nothing here signs.
// `expect` would be unfulfilled in whichever test crate uses every item.
#![allow(dead_code, reason = "each including test crate uses a different subset")]

use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator::{Error, Nonces, Presentation};
use sparq_proved_evaluator_model::authenticated_rdf::{
    self as auth, AuthorizedKey, Policy, PrivateCredentials, Request, SignedCredential, Witness,
};
use sparq_proved_evaluator_model::{DatasetAuthority, RowOrder, v3};
use std::collections::BTreeSet;

pub const W3C_DOCUMENT: &str = concat!(
    r#"<did:example:abcdefgh> <https://www.w3.org/ns/credentials/examples#alumniOf> "The School of Examples" ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/ns/credentials/examples#AlumniCredential> ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://schema.org/description> "A minimum viable example of an Alumni Credential." ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://schema.org/name> "Alumni Credential" ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://www.w3.org/2018/credentials#credentialSubject> <did:example:abcdefgh> ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://www.w3.org/2018/credentials#issuer> <https://vc.example/issuers/5678> ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://www.w3.org/2018/credentials#validFrom> "2023-01-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ."#, "\n",
);
pub const W3C_PROOF: &str = concat!(
    r#"_:c14n0 <http://purl.org/dc/terms/created> "2023-02-24T23:36:38Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ."#, "\n",
    r#"_:c14n0 <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://w3id.org/security#DataIntegrityProof> ."#, "\n",
    r#"_:c14n0 <https://w3id.org/security#cryptosuite> "eddsa-rdfc-2022"^^<https://w3id.org/security#cryptosuiteString> ."#, "\n",
    r#"_:c14n0 <https://w3id.org/security#proofPurpose> <https://w3id.org/security#assertionMethod> ."#, "\n",
    r#"_:c14n0 <https://w3id.org/security#verificationMethod> <did:key:z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2#z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2> ."#, "\n",
);
pub const W3C_DOCUMENT_SHA256: &str = "517744132ae165a5349155bef0bb0cf2258fff99dfe1dbd914b938d775a36017";
pub const W3C_PROOF_SHA256: &str = "bea7b7acfbad0126b135104024a5f1733e705108f42d59668b05c0c50004c6b0";
pub const W3C_PUBLIC_KEY: &str = "b00d8d938e7f773d51565aad36a623f5344f7f5d1960f9cf3e8e12620ea2810f";
pub const W3C_SIGNATURE: &str = "4d8e53c2d5b3f2a7891753eb16ca993325bdb0d3cfc5be1093d0a18426f5ef8578cadc0fd4b5f4dd0d1ce0aefd15ab120b7a894d0eb094ffda4e6553cd1ed50d";
pub const W3C_ISSUER: &str = "https://vc.example/issuers/5678";
pub const W3C_VM: &str = "did:key:z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2#z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2";

/// Another valid public key: RFC 8032 section 7.1, TEST 1. Public data only.
pub const RFC8032_TEST1_PUBLIC_KEY: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
/// Synthetic issuer and method that the published credential does not name.
pub const OTHER_ISSUER: &str = "https://vc.example/issuers/9999";
pub const OTHER_VM: &str = "did:example:other-issuer#key-1";

/// Fixed public synthetic salt; never a deployment salt.
pub const SALT: [u8; 32] = [0xa5; 32];
/// A second public synthetic salt, for an anchor over another blinding.
pub const OTHER_SALT: [u8; 32] = [0x5a; 32];

/// Bag SELECT; `VALUES` repeats the one match so the bag keeps a duplicate.
pub const SELECT_BAG: &str =
    "SELECT ?name WHERE { ?c <https://schema.org/name> ?name VALUES ?copy { 1 2 } }";
pub const ASK_ISSUER: &str =
    "ASK { ?c <https://www.w3.org/2018/credentials#issuer> <https://vc.example/issuers/5678> }";
pub const ASK_OTHER_ISSUER: &str =
    "ASK { ?c <https://www.w3.org/2018/credentials#issuer> <https://vc.example/issuers/9999> }";
pub const CONSTRUCT: &str = "CONSTRUCT { ?s <http://ex/alumniOf> ?o } \
     WHERE { ?s <https://www.w3.org/ns/credentials/examples#alumniOf> ?o }";

/// Hand-written from the published document; never derived from an evaluator.
pub const NAME_CELL: &str = "\"Alumni Credential\"";
pub const GRAPH: &str = "<did:example:abcdefgh> <http://ex/alumniOf> \"The School of Examples\" .\n";

/// Hand-defined expected result of one fixture query.
#[derive(Clone, Copy, Debug)]
pub enum Expect {
    Bag,
    Ask(bool),
    Graph,
}

impl Expect {
    pub fn result(self) -> v3::CanonicalResult {
        match self {
            Self::Bag => v3::CanonicalResult::Select {
                variables: vec!["name".to_owned()],
                order: RowOrder::Bag,
                rows: vec![vec![Some(NAME_CELL.to_owned())]; 2],
            },
            Self::Ask(value) => v3::CanonicalResult::Ask(value),
            Self::Graph => v3::CanonicalResult::Graph {
                ntriples: GRAPH.to_owned(),
            },
        }
    }
}

pub fn hex<const N: usize>(text: &str) -> [u8; N] {
    assert_eq!(text.len(), 2 * N);
    std::array::from_fn(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
}

pub fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Decodes Bitcoin-alphabet base58 (multibase `z`), for the published `did:key`.
fn base58btc(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    // Big-endian magnitude; each digit multiplies it by 58 and adds the digit.
    let mut magnitude: Vec<u8> = Vec::new();
    for digit in text.bytes() {
        let mut carry = ALPHABET
            .iter()
            .position(|&symbol| symbol == digit)
            .expect("base58btc digit") as u32;
        for byte in magnitude.iter_mut().rev() {
            carry += u32::from(*byte) * 58;
            *byte = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            magnitude.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let zeros = text.bytes().take_while(|&digit| digit == b'1').count();
    [vec![0; zeros], magnitude].concat()
}

/// The one line of `text` containing `needle`; fails on zero or several.
fn only_line<'a>(text: &'a str, needle: &str) -> &'a str {
    let mut lines = text.lines().filter(|line| line.contains(needle));
    let (Some(line), None) = (lines.next(), lines.next()) else {
        panic!("expected exactly one fixture line containing {needle}");
    };
    line
}

/// Fails unless the embedded bytes are exactly the published canonical inputs
/// and every hand-written expectation and key is read from them.
pub fn check_published_vector() {
    assert_eq!(hex_string(&Sha256::digest(W3C_DOCUMENT)), W3C_DOCUMENT_SHA256);
    assert_eq!(hex_string(&Sha256::digest(W3C_PROOF)), W3C_PROOF_SHA256);
    // The key table entry: the published issuer, method and the method's key.
    let issuer = only_line(W3C_DOCUMENT, "<https://www.w3.org/2018/credentials#issuer>");
    assert!(issuer.ends_with(&format!(" <{W3C_ISSUER}> .")), "{issuer}");
    let method = only_line(W3C_PROOF, "<https://w3id.org/security#verificationMethod>");
    assert!(method.ends_with(&format!(" <{W3C_VM}> .")), "{method}");
    let multibase = W3C_VM
        .strip_prefix("did:key:z")
        .and_then(|rest| rest.split('#').next())
        .expect("did:key method");
    assert_eq!(W3C_VM, format!("did:key:z{multibase}#z{multibase}"));
    // Multicodec ed25519-pub (0xed 0x01) followed by the raw 32-byte key.
    let mut multikey = vec![0xed_u8, 0x01];
    multikey.extend_from_slice(&hex::<32>(W3C_PUBLIC_KEY));
    assert_eq!(base58btc(multibase), multikey);
    // SELECT: the only schema:name statement; VALUES repeats it into two rows.
    let name = only_line(W3C_DOCUMENT, "<https://schema.org/name>");
    assert!(name.ends_with(&format!(" {NAME_CELL} .")), "{name}");
    // CONSTRUCT: the only alumniOf statement under the template's predicate.
    let alumni = "<https://www.w3.org/ns/credentials/examples#alumniOf>";
    let source = only_line(W3C_DOCUMENT, alumni);
    assert_eq!(GRAPH, format!("{}\n", source.replace(alumni, "<http://ex/alumniOf>")));
    // ASK false: the other issuer appears nowhere in the signed inputs.
    assert!(!W3C_DOCUMENT.contains(OTHER_ISSUER) && !W3C_PROOF.contains(OTHER_ISSUER));
    assert!(!W3C_PROOF.contains(OTHER_VM));
}

/// A FAKE receipt claiming `image_id` and carrying `journal`; verifiers must reject it.
pub fn fake_receipt(image_id: [u32; 8], journal: Vec<u8>) -> Presentation {
    Presentation {
        receipt: Receipt::new(
            InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(image_id, journal.clone()))),
            journal,
        ),
    }
}

pub fn authorized(issuer: &str, method: &str, public_key: &str) -> AuthorizedKey {
    AuthorizedKey {
        issuer: issuer.into(),
        verification_method: method.into(),
        public_key: hex(public_key),
    }
}

/// The verifier's own one-entry table for the published issuer and method.
pub fn policy() -> Policy {
    Policy::new(vec![authorized(W3C_ISSUER, W3C_VM, W3C_PUBLIC_KEY)])
}

/// The published credential, split into its signed parts.
pub fn credential() -> SignedCredential {
    SignedCredential {
        document: W3C_DOCUMENT.into(),
        proof_config: W3C_PROOF.into(),
        signature: hex::<64>(W3C_SIGNATURE).to_vec(),
    }
}

pub fn credentials(list: Vec<SignedCredential>, salt: [u8; 32]) -> PrivateCredentials {
    PrivateCredentials {
        credentials: list,
        salt,
    }
}

/// The verifier's anchor, from its own independently obtained copy and salt.
pub fn anchor(salt: [u8; 32]) -> [u8; 32] {
    auth::dataset_commitment(&credentials(vec![credential()], salt), &policy())
        .expect("published vector authenticates")
}

pub fn request(query: &str, authority: DatasetAuthority, nonce: [u8; 32]) -> Request {
    Request {
        version: auth::VERSION,
        query: query.into(),
        authority,
        policy: policy(),
        nonce,
    }
}

/// The published credential and [`SALT`] under the given request fields.
pub fn witness(query: &str, authority: DatasetAuthority, nonce: [u8; 32]) -> Witness {
    Witness {
        request: request(query, authority, nonce),
        dataset: credentials(vec![credential()], SALT),
    }
}

/// Error returned by a deliberately broken test nonce store.
pub const STORE_FAILURE: Error = Error("test nonce store failure");

/// In-memory counting test double; never a production nonce store.
#[derive(Default)]
pub struct CountingNonces {
    seen: BTreeSet<[u8; 32]>,
    pub calls: usize,
    pub broken: bool,
}

impl CountingNonces {
    pub fn broken() -> Self {
        Self {
            broken: true,
            ..Self::default()
        }
    }
}

impl Nonces for CountingNonces {
    fn consume(&mut self, nonce: [u8; 32]) -> Result<bool, Error> {
        self.calls += 1;
        if self.broken {
            return Err(STORE_FAILURE);
        }
        Ok(self.seen.insert(nonce))
    }
}

// [OPUS-5.5] zkp-14.6: synthetic payment-history credential for the false-ASK example.
// Signed with the RFC 8032 section 7.1 TEST 1 key, whose secret key is public;
// the fixture is public test data, never a deployment credential.

/// Synthetic issuer of the payment-history credential.
pub const PAYMENT_ISSUER: &str = "https://bank.example/issuers/1";
/// `did:key` of [`RFC8032_TEST1_PUBLIC_KEY`], in the published vector's `#`-fragment form.
pub const PAYMENT_VM: &str = "did:key:z6MktwupdmLXVVqTzCw4i46r4uGyosGXRnR3XjN4Zq7oMMsw#z6MktwupdmLXVVqTzCw4i46r4uGyosGXRnR3XjN4Zq7oMMsw";
/// Canonical RDFC-1.0 N-Quads of the unsecured document; no blank nodes.
pub const PAYMENT_DOCUMENT: &str = concat!(
    r#"<did:example:abcdefgh> <https://bank.example/vocab#payment> <https://bank.example/payments/2026-06> ."#, "\n",
    r#"<did:example:abcdefgh> <https://bank.example/vocab#payment> <https://bank.example/payments/2026-07> ."#, "\n",
    r#"<did:example:abcdefgh> <https://bank.example/vocab#payment> <https://bank.example/payments/2026-08> ."#, "\n",
    r#"<https://bank.example/payments/2026-06> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://bank.example/vocab#Payment> ."#, "\n",
    r#"<https://bank.example/payments/2026-06> <https://bank.example/vocab#amount> "1250.00"^^<http://www.w3.org/2001/XMLSchema#decimal> ."#, "\n",
    r#"<https://bank.example/payments/2026-06> <https://bank.example/vocab#paymentStatus> <https://bank.example/vocab#Settled> ."#, "\n",
    r#"<https://bank.example/payments/2026-07> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://bank.example/vocab#Payment> ."#, "\n",
    r#"<https://bank.example/payments/2026-07> <https://bank.example/vocab#amount> "1250.00"^^<http://www.w3.org/2001/XMLSchema#decimal> ."#, "\n",
    r#"<https://bank.example/payments/2026-07> <https://bank.example/vocab#paymentStatus> <https://bank.example/vocab#Settled> ."#, "\n",
    r#"<https://bank.example/payments/2026-08> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://bank.example/vocab#Payment> ."#, "\n",
    r#"<https://bank.example/payments/2026-08> <https://bank.example/vocab#amount> "1310.50"^^<http://www.w3.org/2001/XMLSchema#decimal> ."#, "\n",
    r#"<https://bank.example/payments/2026-08> <https://bank.example/vocab#paymentStatus> <https://bank.example/vocab#Settled> ."#, "\n",
    r#"<urn:uuid:7d1c2b4e-6f0a-4c39-9a51-2b8e3f4d5a60> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://bank.example/vocab#PaymentHistoryCredential> ."#, "\n",
    r#"<urn:uuid:7d1c2b4e-6f0a-4c39-9a51-2b8e3f4d5a60> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> ."#, "\n",
    r#"<urn:uuid:7d1c2b4e-6f0a-4c39-9a51-2b8e3f4d5a60> <https://www.w3.org/2018/credentials#credentialSubject> <did:example:abcdefgh> ."#, "\n",
    r#"<urn:uuid:7d1c2b4e-6f0a-4c39-9a51-2b8e3f4d5a60> <https://www.w3.org/2018/credentials#issuer> <https://bank.example/issuers/1> ."#, "\n",
    r#"<urn:uuid:7d1c2b4e-6f0a-4c39-9a51-2b8e3f4d5a60> <https://www.w3.org/2018/credentials#validFrom> "2026-09-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ."#, "\n",
);
/// Canonical RDFC-1.0 N-Quads of the proof configuration, without `proofValue`.
pub const PAYMENT_PROOF: &str = concat!(
    r#"_:c14n0 <http://purl.org/dc/terms/created> "2026-09-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ."#, "\n",
    r#"_:c14n0 <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://w3id.org/security#DataIntegrityProof> ."#, "\n",
    r#"_:c14n0 <https://w3id.org/security#cryptosuite> "eddsa-rdfc-2022"^^<https://w3id.org/security#cryptosuiteString> ."#, "\n",
    r#"_:c14n0 <https://w3id.org/security#proofPurpose> <https://w3id.org/security#assertionMethod> ."#, "\n",
    r#"_:c14n0 <https://w3id.org/security#verificationMethod> <did:key:z6MktwupdmLXVVqTzCw4i46r4uGyosGXRnR3XjN4Zq7oMMsw#z6MktwupdmLXVVqTzCw4i46r4uGyosGXRnR3XjN4Zq7oMMsw> ."#, "\n",
);
pub const PAYMENT_DOCUMENT_SHA256: &str = "c9b4d8917d6fa0802e762e80a5bc38c987436f322aec3bd50d391c6ca7bf91c5";
pub const PAYMENT_PROOF_SHA256: &str = "b76ec93a8c2f2121288bc717aaad5ffc5fda6e657f80aa18dcd3a1190386127c";
/// Ed25519 over `SHA-256(PAYMENT_PROOF) || SHA-256(PAYMENT_DOCUMENT)`.
pub const PAYMENT_SIGNATURE: &str = "0091a31ff65b96f9ec4fde510c5cc083691182e7fd3743b3b8a19dc0209523341e7534a4aee5fa29136407dcc6d8d6d9d09cfece61b0d70eeaad8701e2899405";

/// "Was any payment returned?" False: every payment in the credential is settled.
pub const ASK_PAYMENT_RETURNED: &str = "ASK { ?p a <https://bank.example/vocab#Payment> ; \
     <https://bank.example/vocab#paymentStatus> <https://bank.example/vocab#Returned> }";
/// The same shape for settled payments; true. Used as the changed-query control.
pub const ASK_PAYMENT_SETTLED: &str = "ASK { ?p a <https://bank.example/vocab#Payment> ; \
     <https://bank.example/vocab#paymentStatus> <https://bank.example/vocab#Settled> }";

/// Hand-written check of the synthetic payment fixture, mirroring
/// [`check_published_vector`]: hashes, issuer, method key and the ASK facts.
/// The signature itself is checked by the native model in each driver run.
pub fn check_payment_fixture() {
    assert_eq!(hex_string(&Sha256::digest(PAYMENT_DOCUMENT)), PAYMENT_DOCUMENT_SHA256);
    assert_eq!(hex_string(&Sha256::digest(PAYMENT_PROOF)), PAYMENT_PROOF_SHA256);
    let issuer = only_line(PAYMENT_DOCUMENT, "<https://www.w3.org/2018/credentials#issuer>");
    assert!(issuer.ends_with(&format!(" <{PAYMENT_ISSUER}> .")), "{issuer}");
    let method = only_line(PAYMENT_PROOF, "<https://w3id.org/security#verificationMethod>");
    assert!(method.ends_with(&format!(" <{PAYMENT_VM}> .")), "{method}");
    let multibase = PAYMENT_VM
        .strip_prefix("did:key:z")
        .and_then(|rest| rest.split('#').next())
        .expect("did:key method");
    let mut multikey = vec![0xed_u8, 0x01];
    multikey.extend_from_slice(&hex::<32>(RFC8032_TEST1_PUBLIC_KEY));
    assert_eq!(base58btc(multibase), multikey);
    // ASK false: no payment status other than Settled is stated anywhere.
    let statuses: Vec<&str> = PAYMENT_DOCUMENT
        .lines()
        .filter(|line| line.contains("<https://bank.example/vocab#paymentStatus>"))
        .collect();
    assert_eq!(statuses.len(), 3);
    assert!(statuses.iter().all(|line| line.ends_with(" <https://bank.example/vocab#Settled> .")));
    assert!(!PAYMENT_DOCUMENT.contains("Returned") && !PAYMENT_PROOF.contains("Returned"));
}

/// The signed fixture a genuine-driver case evaluates; one per job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dataset {
    /// The published W3C `eddsa-rdfc-2022` alumni vector.
    W3cAlumni,
    /// The synthetic payment-history credential signed with the RFC 8032 TEST 1 key.
    PaymentHistory,
}

impl Dataset {
    pub fn name(self) -> &'static str {
        match self {
            Self::W3cAlumni => "w3c-alumni",
            Self::PaymentHistory => "payment-history",
        }
    }

    pub fn check(self) {
        match self {
            Self::W3cAlumni => check_published_vector(),
            Self::PaymentHistory => check_payment_fixture(),
        }
    }

    /// The verifier's own one-entry table for this fixture's issuer and method.
    pub fn policy(self) -> Policy {
        match self {
            Self::W3cAlumni => policy(),
            Self::PaymentHistory => Policy::new(vec![authorized(
                PAYMENT_ISSUER,
                PAYMENT_VM,
                RFC8032_TEST1_PUBLIC_KEY,
            )]),
        }
    }

    /// The fixture's own public key, hex.
    pub fn public_key(self) -> &'static str {
        match self {
            Self::W3cAlumni => W3C_PUBLIC_KEY,
            Self::PaymentHistory => RFC8032_TEST1_PUBLIC_KEY,
        }
    }

    /// Another valid public key that does not sign this fixture.
    pub fn other_public_key(self) -> &'static str {
        match self {
            Self::W3cAlumni => RFC8032_TEST1_PUBLIC_KEY,
            Self::PaymentHistory => W3C_PUBLIC_KEY,
        }
    }

    pub fn credential(self) -> SignedCredential {
        match self {
            Self::W3cAlumni => credential(),
            Self::PaymentHistory => SignedCredential {
                document: PAYMENT_DOCUMENT.into(),
                proof_config: PAYMENT_PROOF.into(),
                signature: hex::<64>(PAYMENT_SIGNATURE).to_vec(),
            },
        }
    }

    pub fn credentials(self, salt: [u8; 32]) -> PrivateCredentials {
        credentials(vec![self.credential()], salt)
    }

    /// The verifier's anchor, from its own independently obtained copy and salt.
    pub fn anchor(self, salt: [u8; 32]) -> [u8; 32] {
        auth::dataset_commitment(&self.credentials(salt), &self.policy())
            .expect("fixture authenticates")
    }

    /// Source description and the signed inputs' hashes, for evidence metadata.
    pub fn source(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::W3cAlumni => (
                "W3C vc-di-eddsa REC 2025-05-15, eddsa-rdfc-2022 examples 7, 9, 10, 12, 13, 15",
                W3C_DOCUMENT_SHA256,
                W3C_PROOF_SHA256,
            ),
            Self::PaymentHistory => (
                "synthetic payment-history credential, eddsa-rdfc-2022 profile, signed with the public RFC 8032 section 7.1 TEST 1 key",
                PAYMENT_DOCUMENT_SHA256,
                PAYMENT_PROOF_SHA256,
            ),
        }
    }
}
