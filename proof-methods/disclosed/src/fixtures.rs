//! The paper's payment-history credential, copies of it for more holders, and
//! the query set Q1–Q5, for comparing proof methods. Public test data only.
//!
//! Every credential is signed with the RFC 8032 section 7.1 TEST 1 key, whose
//! secret key is published, so none of these is a deployment credential.

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator_model::DatasetAuthority;
use sparq_proved_evaluator_model::authenticated_rdf::{
    self as auth, AuthorizedKey, Policy, Request, SignedCredential,
};

/// RFC 8032 section 7.1 TEST 1 secret key (published).
pub const SECRET_KEY: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];
/// Synthetic issuer.
pub const ISSUER: &str = "https://bank.example/issuers/1";
/// `did:key` verification method of the TEST 1 key.
pub const VERIFICATION_METHOD: &str = "did:key:z6MktwupdmLXVVqTzCw4i46r4uGyosGXRnR3XjN4Zq7oMMsw#z6MktwupdmLXVVqTzCw4i46r4uGyosGXRnR3XjN4Zq7oMMsw";

const XSD_DATE_TIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// The synthetic payment-history credential behind the paper's false-ASK
/// evidence (`PAYMENT_DOCUMENT` in
/// `zk/sparql-evaluator/host/tests/support/authenticated_rdf.rs`), in canonical
/// RDFC-1.0 N-Quads.
pub const PAYMENT_DOCUMENT: &str = concat!(
    r#"<did:example:abcdefgh> <https://bank.example/vocab#payment> <https://bank.example/payments/2026-06> ."#,
    "\n",
    r#"<did:example:abcdefgh> <https://bank.example/vocab#payment> <https://bank.example/payments/2026-07> ."#,
    "\n",
    r#"<did:example:abcdefgh> <https://bank.example/vocab#payment> <https://bank.example/payments/2026-08> ."#,
    "\n",
    r#"<https://bank.example/payments/2026-06> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://bank.example/vocab#Payment> ."#,
    "\n",
    r#"<https://bank.example/payments/2026-06> <https://bank.example/vocab#amount> "1250.00"^^<http://www.w3.org/2001/XMLSchema#decimal> ."#,
    "\n",
    r#"<https://bank.example/payments/2026-06> <https://bank.example/vocab#paymentStatus> <https://bank.example/vocab#Settled> ."#,
    "\n",
    r#"<https://bank.example/payments/2026-07> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://bank.example/vocab#Payment> ."#,
    "\n",
    r#"<https://bank.example/payments/2026-07> <https://bank.example/vocab#amount> "1250.00"^^<http://www.w3.org/2001/XMLSchema#decimal> ."#,
    "\n",
    r#"<https://bank.example/payments/2026-07> <https://bank.example/vocab#paymentStatus> <https://bank.example/vocab#Settled> ."#,
    "\n",
    r#"<https://bank.example/payments/2026-08> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://bank.example/vocab#Payment> ."#,
    "\n",
    r#"<https://bank.example/payments/2026-08> <https://bank.example/vocab#amount> "1310.50"^^<http://www.w3.org/2001/XMLSchema#decimal> ."#,
    "\n",
    r#"<https://bank.example/payments/2026-08> <https://bank.example/vocab#paymentStatus> <https://bank.example/vocab#Settled> ."#,
    "\n",
    r#"<urn:uuid:7d1c2b4e-6f0a-4c39-9a51-2b8e3f4d5a60> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://bank.example/vocab#PaymentHistoryCredential> ."#,
    "\n",
    r#"<urn:uuid:7d1c2b4e-6f0a-4c39-9a51-2b8e3f4d5a60> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> ."#,
    "\n",
    r#"<urn:uuid:7d1c2b4e-6f0a-4c39-9a51-2b8e3f4d5a60> <https://www.w3.org/2018/credentials#credentialSubject> <did:example:abcdefgh> ."#,
    "\n",
    r#"<urn:uuid:7d1c2b4e-6f0a-4c39-9a51-2b8e3f4d5a60> <https://www.w3.org/2018/credentials#issuer> <https://bank.example/issuers/1> ."#,
    "\n",
    r#"<urn:uuid:7d1c2b4e-6f0a-4c39-9a51-2b8e3f4d5a60> <https://www.w3.org/2018/credentials#validFrom> "2026-09-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ."#,
    "\n",
);
/// Recorded `eddsa-rdfc-2022` signature on [`PAYMENT_DOCUMENT`] (hex).
pub const PAYMENT_SIGNATURE: &str = "0091a31ff65b96f9ec4fde510c5cc083691182e7fd3743b3b8a19dc0209523341e7534a4aee5fa29136407dcc6d8d6d9d09cfece61b0d70eeaad8701e2899405";

/// One query of the comparison set.
#[derive(Clone, Copy, Debug)]
pub struct Query {
    pub id: &'static str,
    pub text: &'static str,
}

/// Q1–Q5, copied verbatim from `zk/sparql-evaluator/fixtures/paper/*.rq`.
pub const QUERIES: [Query; 5] = [
    Query {
        id: "Q1",
        text: "PREFIX bank: <https://bank.example/vocab#>\n\
               ASK { ?p a bank:Payment ; bank:paymentStatus bank:Returned }\n",
    },
    Query {
        id: "Q2",
        text: "PREFIX bank: <https://bank.example/vocab#>\n\
               SELECT ?amount WHERE { ?p a bank:Payment ; bank:amount ?amount }\n",
    },
    Query {
        id: "Q3",
        text: "PREFIX bank: <https://bank.example/vocab#>\n\
               CONSTRUCT { ?p bank:amount ?amount } WHERE { ?p a bank:Payment ; bank:amount ?amount }\n",
    },
    Query {
        id: "Q4",
        text: "PREFIX bank: <https://bank.example/vocab#>\n\
               SELECT ?p WHERE { ?p bank:amount ?amount FILTER(?amount > 1300.00) }\n",
    },
    Query {
        id: "Q5",
        text: "PREFIX bank: <https://bank.example/vocab#>\n\
               SELECT ?p WHERE { ?p a bank:Payment \
               FILTER(STRSTARTS(STR(?p), \"https://bank.example/payments/2026-07\")) }\n",
    },
];

fn iri(value: &str) -> String {
    format!("<{value}>")
}

/// Canonical N-Quads of credential `index`.
///
/// Index 0 is [`PAYMENT_DOCUMENT`] byte for byte. Index `i > 0` is a copy for
/// subject `did:example:holder-i`, credential id `...-{i:012}`, with every
/// payment month moved `3 * i` months later. The document has no blank nodes,
/// so its canonical form is its sorted lines.
pub fn document(index: usize) -> String {
    if index == 0 {
        return PAYMENT_DOCUMENT.to_owned();
    }
    let mut text = PAYMENT_DOCUMENT
        .replace(
            "did:example:abcdefgh",
            &format!("did:example:holder-{index}"),
        )
        .replace(
            "urn:uuid:7d1c2b4e-6f0a-4c39-9a51-2b8e3f4d5a60",
            &format!("urn:uuid:7d1c2b4e-6f0a-4c39-9a51-{index:012}"),
        );
    for month in [6, 7, 8] {
        let shifted = month - 1 + 3 * index;
        text = text.replace(
            &format!("payments/2026-{month:02}>"),
            &format!(
                "payments/{}-{:02}#m>",
                2026 + shifted / 12,
                shifted % 12 + 1
            ),
        );
    }
    let text = text.replace("#m>", ">");
    let mut lines: Vec<&str> = text.lines().collect();
    lines.sort_unstable();
    lines.dedup();
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Canonical N-Quads of the proof configuration, without `proofValue`.
pub fn proof_config() -> String {
    [
        format!(
            "_:c14n0 <http://purl.org/dc/terms/created> \"2026-09-01T00:00:00Z\"^^{} .",
            iri(XSD_DATE_TIME)
        ),
        format!("_:c14n0 {} <https://w3id.org/security#DataIntegrityProof> .", iri(RDF_TYPE)),
        "_:c14n0 <https://w3id.org/security#cryptosuite> \"eddsa-rdfc-2022\"^^<https://w3id.org/security#cryptosuiteString> ."
            .to_owned(),
        "_:c14n0 <https://w3id.org/security#proofPurpose> <https://w3id.org/security#assertionMethod> ."
            .to_owned(),
        format!("_:c14n0 <https://w3id.org/security#verificationMethod> {} .", iri(VERIFICATION_METHOD)),
    ]
    .join("\n")
        + "\n"
}

/// Issuer step: signs `document` under `eddsa-rdfc-2022` with [`SECRET_KEY`].
///
/// Both inputs must already be in canonical form; this function does not
/// canonicalize.
pub fn sign(document: String) -> SignedCredential {
    let config = proof_config();
    let mut message = Vec::with_capacity(64);
    message.extend_from_slice(&Sha256::digest(config.as_bytes()));
    message.extend_from_slice(&Sha256::digest(document.as_bytes()));
    let signature = SigningKey::from_bytes(&SECRET_KEY).sign(&message);
    SignedCredential {
        document,
        proof_config: config,
        signature: signature.to_bytes().to_vec(),
    }
}

/// `n` signed credentials: [`PAYMENT_DOCUMENT`] and `n - 1` copies.
pub fn credentials(n: usize) -> Vec<SignedCredential> {
    (0..n).map(|i| sign(document(i))).collect()
}

/// The verifier's one-entry key table.
pub fn policy() -> Policy {
    Policy::new(vec![AuthorizedKey {
        issuer: ISSUER.to_owned(),
        verification_method: VERIFICATION_METHOD.to_owned(),
        public_key: SigningKey::from_bytes(&SECRET_KEY)
            .verifying_key()
            .to_bytes(),
    }])
}

/// A request for `query` under `authority`.
pub fn request(query: &str, authority: DatasetAuthority, nonce: [u8; 32]) -> Request {
    Request {
        version: auth::VERSION,
        query: query.to_owned(),
        authority,
        policy: policy(),
        nonce,
    }
}
