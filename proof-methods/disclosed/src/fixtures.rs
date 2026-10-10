//! Synthetic payment-history credentials and the query set Q1–Q5 for
//! comparing proof methods. Public test data only.
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

const V: &str = "https://bank.example/vocab#";
const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
const XSD_DATE_TIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const CRED: &str = "https://www.w3.org/2018/credentials#";

/// One query of the comparison set.
#[derive(Clone, Copy, Debug)]
pub struct Query {
    pub id: &'static str,
    pub text: &'static str,
}

/// Q1–Q5: false ASK, bag SELECT, CONSTRUCT, `xsd:decimal` FILTER, string FILTER.
pub const QUERIES: [Query; 5] = [
    Query {
        id: "Q1",
        text: "ASK { ?p a <https://bank.example/vocab#Payment> ; \
               <https://bank.example/vocab#paymentStatus> <https://bank.example/vocab#Returned> }",
    },
    Query {
        id: "Q2",
        text: "SELECT ?amount WHERE { ?p a <https://bank.example/vocab#Payment> ; \
               <https://bank.example/vocab#amount> ?amount }",
    },
    Query {
        id: "Q3",
        text: "CONSTRUCT { ?s <https://bank.example/vocab#paid> ?p } \
               WHERE { ?s <https://bank.example/vocab#payment> ?p }",
    },
    Query {
        id: "Q4",
        text: "SELECT ?p WHERE { ?p <https://bank.example/vocab#amount> ?a FILTER(?a > 1300.00) }",
    },
    Query {
        id: "Q5",
        text: "SELECT ?p WHERE { ?p <https://bank.example/vocab#reference> ?r \
               FILTER(STRSTARTS(?r, \"RENT-\")) }",
    },
];

fn iri(value: &str) -> String {
    format!("<{value}>")
}

/// Canonical N-Quads of credential `index` with `payments` payments.
///
/// The document has no blank nodes, so its RDFC-1.0 canonical form is its
/// sorted, de-duplicated lines.
pub fn document(index: usize, payments: usize) -> String {
    let subject = iri("did:example:abcdefgh");
    let credential = iri(&format!("urn:uuid:00000000-0000-4000-8000-{index:012}"));
    let mut lines = vec![
        format!(
            "{credential} {} {} .",
            iri(RDF_TYPE),
            iri(&format!("{V}PaymentHistoryCredential"))
        ),
        format!(
            "{credential} {} {} .",
            iri(RDF_TYPE),
            iri(&format!("{CRED}VerifiableCredential"))
        ),
        format!(
            "{credential} {} {subject} .",
            iri(&format!("{CRED}credentialSubject"))
        ),
        format!(
            "{credential} {} {} .",
            iri(&format!("{CRED}issuer")),
            iri(ISSUER)
        ),
        format!(
            "{credential} {} \"2026-09-01T00:00:00Z\"^^{} .",
            iri(&format!("{CRED}validFrom")),
            iri(XSD_DATE_TIME)
        ),
    ];
    for k in 0..payments {
        let payment = iri(&format!("https://bank.example/payments/{index}-{k}"));
        let cents = 125_000 + 6_050 * ((index * payments + k) % 4);
        let reference = if k % 2 == 0 { "RENT" } else { "UTIL" };
        lines.push(format!(
            "{subject} {} {payment} .",
            iri(&format!("{V}payment"))
        ));
        lines.push(format!(
            "{payment} {} {} .",
            iri(RDF_TYPE),
            iri(&format!("{V}Payment"))
        ));
        lines.push(format!(
            "{payment} {} \"{}.{:02}\"^^{} .",
            iri(&format!("{V}amount")),
            cents / 100,
            cents % 100,
            iri(XSD_DECIMAL)
        ));
        lines.push(format!(
            "{payment} {} {} .",
            iri(&format!("{V}paymentStatus")),
            iri(&format!("{V}Settled"))
        ));
        lines.push(format!(
            "{payment} {} \"{reference}-{index:04}-{k:04}\" .",
            iri(&format!("{V}reference"))
        ));
    }
    lines.sort();
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

/// `n` signed credentials of `payments` payments each.
pub fn credentials(n: usize, payments: usize) -> Vec<SignedCredential> {
    (0..n).map(|i| sign(document(i, payments))).collect()
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
