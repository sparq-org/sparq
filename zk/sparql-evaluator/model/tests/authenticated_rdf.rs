// Native V5 authenticated-RDF model tests; no guest, receipt or proof coverage.
#![cfg(feature = "authenticated-rdf")]
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator_model::authenticated_rdf::{
    self as auth, AuthorizedKey, Journal, MAX_DOCUMENT_BYTES, MAX_DOCUMENT_QUADS,
    MAX_PROOF_CONFIG_BYTES, Policy, PrivateCredentials, Provenance, Request, SignedCredential,
    Witness,
};
use sparq_proved_evaluator_model::merkle_suite as merkle;
use sparq_proved_evaluator_model::{DatasetAuthority, MAX_ROWS, ProofContract, Rejected, RowOrder, v3};

// Published W3C vector: vc-di-eddsa REC 2025-05-15, eddsa-rdfc-2022 representation,
// examples 7, 9, 10, 12, 13 and 15. Data only; no secret key is involved.
const W3C_DOCUMENT: &str = concat!(
    r#"<did:example:abcdefgh> <https://www.w3.org/ns/credentials/examples#alumniOf> "The School of Examples" ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/ns/credentials/examples#AlumniCredential> ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://schema.org/description> "A minimum viable example of an Alumni Credential." ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://schema.org/name> "Alumni Credential" ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://www.w3.org/2018/credentials#credentialSubject> <did:example:abcdefgh> ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://www.w3.org/2018/credentials#issuer> <https://vc.example/issuers/5678> ."#, "\n",
    r#"<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://www.w3.org/2018/credentials#validFrom> "2023-01-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ."#, "\n",
);
const W3C_PROOF: &str = concat!(
    r#"_:c14n0 <http://purl.org/dc/terms/created> "2023-02-24T23:36:38Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ."#, "\n",
    r#"_:c14n0 <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://w3id.org/security#DataIntegrityProof> ."#, "\n",
    r#"_:c14n0 <https://w3id.org/security#cryptosuite> "eddsa-rdfc-2022"^^<https://w3id.org/security#cryptosuiteString> ."#, "\n",
    r#"_:c14n0 <https://w3id.org/security#proofPurpose> <https://w3id.org/security#assertionMethod> ."#, "\n",
    r#"_:c14n0 <https://w3id.org/security#verificationMethod> <did:key:z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2#z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2> ."#, "\n",
);
const W3C_DOCUMENT_SHA256: &str = "517744132ae165a5349155bef0bb0cf2258fff99dfe1dbd914b938d775a36017";
const W3C_PROOF_SHA256: &str = "bea7b7acfbad0126b135104024a5f1733e705108f42d59668b05c0c50004c6b0";
const W3C_PUBLIC_KEY: &str = "b00d8d938e7f773d51565aad36a623f5344f7f5d1960f9cf3e8e12620ea2810f";
const W3C_SIGNATURE: &str = "4d8e53c2d5b3f2a7891753eb16ca993325bdb0d3cfc5be1093d0a18426f5ef8578cadc0fd4b5f4dd0d1ce0aefd15ab120b7a894d0eb094ffda4e6553cd1ed50d";
const W3C_ISSUER: &str = "https://vc.example/issuers/5678";
const W3C_VM: &str = "did:key:z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2#z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2";

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const CRED: &str = "https://www.w3.org/2018/credentials#";
const SEC: &str = "https://w3id.org/security#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const ISSUER_A: &str = "https://issuer.example/a";
const VM_A: &str = "https://issuer.example/a#key-1";
const ISSUER_B: &str = "https://issuer.example/b";
const VM_B: &str = "https://issuer.example/b#key-1";
const VM_UNLISTED: &str = "https://issuer.example/x#key-1";

fn hex<const N: usize>(text: &str) -> [u8; N] {
    assert_eq!(text.len(), 2 * N);
    std::array::from_fn(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// Deterministic synthetic test keys; never real secret material.
fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn authorized(issuer: &str, method: &str, seed: u8) -> AuthorizedKey {
    AuthorizedKey {
        issuer: issuer.into(),
        verification_method: method.into(),
        public_key: key(seed).verifying_key().to_bytes(),
    }
}

fn table() -> Vec<AuthorizedKey> {
    vec![authorized(ISSUER_A, VM_A, 1), authorized(ISSUER_B, VM_B, 2)]
}

fn config(method: &str) -> String {
    format!(
        "_:proof <{RDF_TYPE}> <{SEC}DataIntegrityProof> .\n\
         _:proof <{SEC}cryptosuite> \"eddsa-rdfc-2022\"^^<{SEC}cryptosuiteString> .\n\
         _:proof <{SEC}verificationMethod> <{method}> .\n\
         _:proof <{SEC}proofPurpose> <{SEC}assertionMethod> .\n"
    )
}

fn with_created(config: String, lexical: &str, datatype: &str) -> String {
    format!("{config}_:proof <http://purl.org/dc/terms/created> \"{lexical}\"^^<{XSD}{datatype}> .\n")
}

fn document(id: &str, issuer: &str, claims: &str) -> String {
    format!("<{id}> <{RDF_TYPE}> <{CRED}VerifiableCredential> .\n<{id}> <{CRED}issuer> <{issuer}> .\n{claims}")
}

// Signs hashData = SHA-256(canonical config) || SHA-256(canonical document).
fn sign(document: &str, config: &str, seed: u8) -> SignedCredential {
    let config_hash = Sha256::digest(sparq_canon::canonicalize_nquads(config).unwrap());
    let document_hash = Sha256::digest(sparq_canon::canonicalize_nquads(document).unwrap());
    let hash_data = [config_hash.as_slice(), document_hash.as_slice()].concat();
    SignedCredential {
        document: document.into(),
        proof_config: config.into(),
        signature: key(seed).sign(&hash_data).to_bytes().to_vec(),
    }
}

// For inputs rejected before any signature check.
fn unsigned(document: &str, config: &str) -> SignedCredential {
    SignedCredential {
        document: document.into(),
        proof_config: config.into(),
        signature: vec![0; 64],
    }
}

fn alice_document() -> String {
    document("urn:vc:alice", ISSUER_A, "<did:example:alice> <http://ex/name> \"Alice\" .\n")
}

fn alice() -> SignedCredential {
    sign(&alice_document(), &config(VM_A), 1)
}

fn bob() -> SignedCredential {
    let claims = "<did:example:bob> <http://ex/name> \"Bob\" .\n";
    sign(&document("urn:vc:bob", ISSUER_B, claims), &config(VM_B), 2)
}

fn named() -> SignedCredential {
    let claims = format!("<urn:vc:named> <{CRED}credentialSubject> _:subject .\n_:subject <http://ex/name> \"Shared\" .\n");
    sign(&document("urn:vc:named", ISSUER_A, &claims), &config(VM_A), 1)
}

fn aged() -> SignedCredential {
    let claims = format!("<urn:vc:aged> <{CRED}credentialSubject> _:subject .\n_:subject <http://ex/age> \"41\"^^<{XSD}integer> .\n");
    sign(&document("urn:vc:aged", ISSUER_B, &claims), &config(VM_B), 2)
}

fn credentials(list: Vec<SignedCredential>) -> PrivateCredentials {
    PrivateCredentials {
        credentials: list,
        salt: [17; 32],
    }
}

fn request(query: &str, authority: DatasetAuthority, policy: Policy) -> Request {
    Request {
        version: auth::VERSION,
        query: format!("PREFIX ex: <http://ex/> {query}"),
        authority,
        policy,
        nonce: [9; 32],
    }
}

fn run(
    query: &str,
    authority: DatasetAuthority,
    policy: Policy,
    list: Vec<SignedCredential>,
) -> Result<Journal, Rejected> {
    auth::evaluate(&Witness {
        request: request(query, authority, policy),
        dataset: credentials(list),
    })
}

fn holder(query: &str, list: Vec<SignedCredential>) -> Result<Journal, Rejected> {
    run(query, DatasetAuthority::HolderDeclared, Policy::new(table()), list)
}

fn authenticate(list: Vec<SignedCredential>) -> Result<[u8; 32], Rejected> {
    auth::dataset_commitment(&credentials(list), &Policy::new(table()))
}

fn reason<T: std::fmt::Debug>(result: Result<T, Rejected>) -> &'static str {
    result.unwrap_err().0
}

fn rows(result: &v3::CanonicalResult) -> Vec<Vec<Option<String>>> {
    let v3::CanonicalResult::Select { rows, .. } = result else {
        panic!("expected a table, got {result:?}")
    };
    rows.clone()
}

fn cell(text: &str) -> Option<String> {
    Some(text.into())
}

fn w3c_policy(issuer: &str, method: &str) -> Policy {
    Policy::new(vec![AuthorizedKey {
        issuer: issuer.into(),
        verification_method: method.into(),
        public_key: hex(W3C_PUBLIC_KEY),
    }])
}

fn w3c_credential(document: &str, proof_config: &str) -> SignedCredential {
    SignedCredential {
        document: document.into(),
        proof_config: proof_config.into(),
        signature: hex::<64>(W3C_SIGNATURE).to_vec(),
    }
}

#[test]
fn published_w3c_vector_verifies_and_maps_exactly_its_canonical_bytes() {
    // The published hashes pin the exact canonical bytes the issuer signed, and
    // the reused canonicalizer is a fixed point on them.
    assert_eq!(hex_string(&Sha256::digest(W3C_DOCUMENT)), W3C_DOCUMENT_SHA256);
    assert_eq!(hex_string(&Sha256::digest(W3C_PROOF)), W3C_PROOF_SHA256);
    assert_eq!(sparq_canon::canonicalize_nquads(W3C_DOCUMENT).unwrap(), W3C_DOCUMENT);
    assert_eq!(sparq_canon::canonicalize_nquads(W3C_PROOF).unwrap(), W3C_PROOF);

    let policy = w3c_policy(W3C_ISSUER, W3C_VM);
    let published = w3c_credential(W3C_DOCUMENT, W3C_PROOF);
    let commitment =
        auth::dataset_commitment(&credentials(vec![published.clone()]), &policy).unwrap();
    let journal = run(
        "SELECT ?name ?from WHERE { ?c <https://schema.org/name> ?name ; \
         <https://www.w3.org/2018/credentials#validFrom> ?from }",
        DatasetAuthority::VerifierAgreed { commitment },
        policy.clone(),
        vec![published.clone()],
    )
    .unwrap();
    assert_eq!(journal.provenance, Provenance::VerifierAgreedAuthenticated);
    assert_eq!(journal.dataset_commitment, commitment);
    assert_eq!(
        rows(&journal.result),
        vec![vec![
            cell("\"Alumni Credential\""),
            Some(format!("\"2023-01-01T00:00:00Z\"^^<{XSD}dateTime>")),
        ]]
    );

    // A holder-relabeled, reordered proof node canonicalizes to the signed bytes.
    let relabeled: String = W3C_PROOF
        .lines()
        .rev()
        .map(|line| format!("{}\n", line.replace("_:c14n0", "_:holderLabel")))
        .collect();
    let relabeled = w3c_credential(W3C_DOCUMENT, &relabeled);
    assert_eq!(
        auth::dataset_commitment(&credentials(vec![relabeled]), &policy).unwrap(),
        commitment
    );

    let mut forged = published.clone();
    forged.signature[0] ^= 1;
    for tampered in [
        w3c_credential(
            &W3C_DOCUMENT.replace("The School of Examples", "The School of Exampled"),
            W3C_PROOF,
        ),
        w3c_credential(W3C_DOCUMENT, &W3C_PROOF.replace("23:36:38Z", "23:36:39Z")),
        forged,
    ] {
        assert_eq!(
            reason(auth::dataset_commitment(&credentials(vec![tampered]), &policy)),
            "Ed25519 signature verification failed"
        );
    }

    // The genuine published signature does not satisfy a different pinned issuer or method.
    let other_issuer = w3c_policy("https://vc.example/issuers/9999", W3C_VM);
    assert_eq!(
        reason(auth::dataset_commitment(&credentials(vec![published.clone()]), &other_issuer)),
        "credential issuer does not match the authorized issuer"
    );
    let other_method = w3c_policy(W3C_ISSUER, "did:example:issuer#other-key");
    assert_eq!(
        reason(auth::dataset_commitment(&credentials(vec![published]), &other_method)),
        "verification method is not authorized"
    );
}

#[test]
fn select_ask_and_construct_bind_both_authorities_to_the_authenticated_commitment() {
    let policy = Policy::new(table());
    let select = "SELECT ?name WHERE { ?person ex:name ?name }";
    let holder_request = request(select, DatasetAuthority::HolderDeclared, policy.clone());
    let selected = holder(select, vec![alice(), bob()]).unwrap();
    auth::bind_journal(&selected, &holder_request).unwrap();
    assert_eq!(selected.provenance, Provenance::HolderSelectedAuthenticated);
    let v3::CanonicalResult::Select { variables, order, rows } = &selected.result else {
        panic!("table")
    };
    assert_eq!(variables, &["name"]);
    assert_eq!(*order, RowOrder::Bag);
    assert_eq!(rows, &[vec![cell("\"Alice\"")], vec![cell("\"Bob\"")]]);

    // Presentation order is not significant to the commitment.
    let commitment = auth::dataset_commitment(&credentials(vec![bob(), alice()]), &policy).unwrap();
    assert_eq!(selected.dataset_commitment, commitment);
    let agreed = DatasetAuthority::VerifierAgreed { commitment };
    let ask_query = "ASK { ?person ex:name \"Bob\" }";
    let ask = run(ask_query, agreed.clone(), policy.clone(), vec![alice(), bob()]).unwrap();
    assert_eq!(ask.result, v3::CanonicalResult::Ask(true));
    assert_eq!(ask.provenance, Provenance::VerifierAgreedAuthenticated);
    let agreed_request = request(ask_query, agreed.clone(), policy.clone());
    auth::bind_journal(&ask, &agreed_request).unwrap();

    let construct = run(
        "CONSTRUCT { ?person ex:label ?name } WHERE { ?person ex:name ?name }",
        agreed.clone(),
        policy.clone(),
        vec![alice(), bob()],
    )
    .unwrap();
    assert_eq!(
        construct.result,
        v3::CanonicalResult::Graph {
            ntriples: "<did:example:alice> <http://ex/label> \"Alice\" .\n\
                       <did:example:bob> <http://ex/label> \"Bob\" .\n"
                .into()
        }
    );

    // Another salt or an omitted credential does not match the agreed anchor.
    let resalted = PrivateCredentials {
        credentials: vec![alice(), bob()],
        salt: [18; 32],
    };
    assert_eq!(
        reason(auth::evaluate(&Witness {
            request: agreed_request.clone(),
            dataset: resalted,
        })),
        "authenticated dataset anchor mismatch"
    );
    assert_eq!(
        reason(run(ask_query, agreed, policy, vec![alice()])),
        "authenticated dataset anchor mismatch"
    );

    // Provenance and anchors cannot cross authorities after evaluation.
    let mut downgraded = ask.clone();
    downgraded.provenance = Provenance::HolderSelectedAuthenticated;
    assert_eq!(
        reason(auth::bind_journal(&downgraded, &agreed_request)),
        "authenticated journal dataset authority mismatch"
    );
    let mut moved = ask;
    moved.dataset_commitment[0] ^= 1;
    assert_eq!(
        reason(auth::bind_journal(&moved, &agreed_request)),
        "authenticated journal dataset authority mismatch"
    );
    let mut upgraded = selected;
    upgraded.provenance = Provenance::VerifierAgreedAuthenticated;
    assert_eq!(
        reason(auth::bind_journal(&upgraded, &holder_request)),
        "authenticated journal holder-selected provenance mismatch"
    );
}

#[test]
fn valid_signatures_do_not_authorize_wrong_issuer_method_purpose_or_suite() {
    let alice = alice_document();
    let typed_suite = format!("\"eddsa-rdfc-2022\"^^<{SEC}cryptosuiteString>");
    let cases = [
        (
            sign(&document("urn:vc:alice", ISSUER_B, ""), &config(VM_A), 1),
            "credential issuer does not match the authorized issuer",
        ),
        (sign(&alice, &config(VM_UNLISTED), 3), "verification method is not authorized"),
        (sign(&alice, &config(VM_A), 2), "Ed25519 signature verification failed"),
        (
            sign(&alice, &config(VM_A).replace("#assertionMethod", "#authentication"), 1),
            "proof purpose must be assertionMethod",
        ),
        (
            sign(&alice, &config(VM_A).replace("\"eddsa-rdfc-2022\"", "\"ecdsa-rdfc-2019\""), 1),
            "cryptosuite must be the policy's typed cryptosuite value",
        ),
        // The plain-literal form is not the standard cryptosuiteString value.
        (
            sign(&alice, &config(VM_A).replace(&typed_suite, "\"eddsa-rdfc-2022\""), 1),
            "cryptosuite must be the policy's typed cryptosuite value",
        ),
        (
            sign(&alice, &config(VM_A).replace("#DataIntegrityProof", "#Ed25519Signature2020"), 1),
            "proof type must be DataIntegrityProof",
        ),
    ];
    for (credential, expected) in cases {
        assert_eq!(reason(authenticate(vec![credential])), expected);
    }
    authenticate(vec![sign(&alice, &config(VM_A), 1)]).unwrap();
}

#[test]
fn document_structure_is_checked_on_the_signed_canonical_bytes() {
    let signed = |document: String| sign(&document, &config(VM_A), 1);
    let bare = |id: &str| format!("<{id}> <{RDF_TYPE}> <{CRED}VerifiableCredential> .\n");
    let cases = [
        (
            signed(document(
                "urn:vc:a",
                ISSUER_A,
                &format!("<urn:vc:a> <{SEC}proof> _:p .\n_:p <{RDF_TYPE}> <{SEC}DataIntegrityProof> .\n"),
            )),
            "embedded proofs are not admitted",
        ),
        (
            signed(document("urn:vc:a", ISSUER_A, &document("urn:vc:b", ISSUER_A, ""))),
            "document must have exactly one VerifiableCredential node",
        ),
        (
            signed("<urn:x> <http://ex/p> \"no credential\" .\n".into()),
            "document must have exactly one VerifiableCredential node",
        ),
        (signed(bare("urn:vc:a")), "credential must have exactly one issuer"),
        (
            signed(document("urn:vc:a", ISSUER_A, &format!("<urn:vc:a> <{CRED}issuer> <{ISSUER_B}> .\n"))),
            "credential must have exactly one issuer",
        ),
        (
            signed(format!("{}<urn:vc:a> <{CRED}issuer> _:issuer .\n", bare("urn:vc:a"))),
            "credential issuer does not match the authorized issuer",
        ),
    ];
    for (credential, expected) in cases {
        assert_eq!(reason(authenticate(vec![credential])), expected);
    }

    let alice = alice_document();
    let early = [
        (
            unsigned(&format!("{alice}<urn:x> <http://ex/p> <urn:y> <urn:graph> .\n"), &config(VM_A)),
            "only the default graph is admitted",
        ),
        (
            unsigned(
                &alice,
                &config(VM_A).replace(
                    &format!("<{SEC}assertionMethod> ."),
                    &format!("<{SEC}assertionMethod> <urn:graph> ."),
                ),
            ),
            "only the default graph is admitted",
        ),
        (
            unsigned(&format!("{alice}<urn:x> <http://ex/p> <<( <urn:a> <urn:b> <urn:c> )>> .\n"), &config(VM_A)),
            "RDF 1.2 triple terms are not admitted",
        ),
        (
            unsigned(&format!("{alice}<urn:x> <http://ex/p> \"x\"@en--ltr .\n"), &config(VM_A)),
            "directional language strings are not admitted",
        ),
        (unsigned("not n-quads\n", &config(VM_A)), "authenticated RDF N-Quads parse rejected"),
    ];
    for (credential, expected) in early {
        assert_eq!(reason(authenticate(vec![credential])), expected);
    }
}

#[test]
fn unknown_duplicate_and_missing_proof_options_reject() {
    let alice = alice_document();
    let option = |line: String| config(VM_A) + &line;
    let without_method: String = config(VM_A)
        .lines()
        .filter(|line| !line.contains("verificationMethod"))
        .map(|line| format!("{line}\n"))
        .collect();
    let cases = [
        (option(format!("_:proof <{SEC}challenge> \"abc\" .\n")), "unsupported proof configuration option"),
        (
            option(format!("_:proof <{SEC}expiration> \"2030-01-01T00:00:00Z\"^^<{XSD}dateTime> .\n")),
            "unsupported proof configuration option",
        ),
        (option(format!("_:proof <{SEC}previousProof> <urn:proof:1> .\n")), "unsupported proof configuration option"),
        (option(format!("_:proof <{SEC}proofValue> \"z1\" .\n")), "unsupported proof configuration option"),
        (
            with_created(with_created(config(VM_A), "2023-02-24T23:36:38Z", "dateTime"), "2023-02-24T23:36:39Z", "dateTime"),
            "duplicate proof configuration predicate",
        ),
        (
            option(format!("_:proof <{SEC}proofPurpose> <{SEC}authentication> .\n")),
            "duplicate proof configuration predicate",
        ),
        (without_method, "proof configuration is missing a required field"),
        (
            option(format!("_:other <{SEC}proofPurpose> <{SEC}assertionMethod> .\n")),
            "proof configuration must describe one proof node",
        ),
        (
            config(VM_A).replace(&format!("<{VM_A}>"), &format!("\"{VM_A}\"")),
            "verification method must be an IRI",
        ),
    ];
    for (proof_config, expected) in cases {
        assert_eq!(
            reason(authenticate(vec![sign(&alice, &proof_config, 1)])),
            expected,
            "{proof_config}"
        );
    }
}

#[test]
fn created_uses_a_declared_restricted_profile_and_rejects_malformed_values() {
    let alice = alice_document();
    let created = |lexical: &str, datatype: &str| {
        authenticate(vec![sign(&alice, &with_created(config(VM_A), lexical, datatype), 1)])
    };
    // `created` is optional; a leap day inside the profile is accepted.
    authenticate(vec![sign(&alice, &config(VM_A), 1)]).unwrap();
    created("2024-02-29T00:00:00Z", "dateTime").unwrap();
    for malformed in [
        "2023-02-29T00:00:00Z",
        "1900-02-29T00:00:00Z",
        "2023-02-24T23:36:38",
        "2023-02-24T24:00:01Z",
        "2023-02-24T23:36:38+14:01",
    ] {
        assert_eq!(
            reason(created(malformed, "dateTime")),
            "created is not a valid XSD 1.1 dateTimeStamp",
            "{malformed}"
        );
    }
    // Valid XSD 1.1 values, including year 0000, outside the whole-second UTC profile.
    for unsupported in [
        "0000-02-29T00:00:00Z",
        "10000-01-01T00:00:00Z",
        "2023-02-24T23:36:38.5Z",
        "2023-02-24T23:36:38+01:00",
        "2023-02-24T24:00:00Z",
    ] {
        assert_eq!(
            reason(created(unsupported, "dateTime")),
            "created is outside the restricted whole-second UTC profile",
            "{unsupported}"
        );
    }
    for datatype in ["string", "dateTimeStamp"] {
        assert_eq!(
            reason(created("2023-02-24T23:36:38Z", datatype)),
            "created must be an xsd:dateTime literal"
        );
    }
}

#[test]
fn authorization_table_is_validated_and_bound_into_request_and_commitment() {
    let query = "ASK { ?person ex:name \"Alice\" }";
    let holder_declared = || DatasetAuthority::HolderDeclared;
    let invalid = |keys: Vec<AuthorizedKey>| {
        reason(auth::validate_request(&request(query, holder_declared(), Policy::new(keys))))
    };
    assert_eq!(invalid(Vec::new()), "authorization table must hold 1 to 16 keys");
    assert_eq!(
        invalid((0..17).map(|i| authorized(ISSUER_A, &format!("{ISSUER_A}#key-{i}"), 1)).collect()),
        "authorization table must hold 1 to 16 keys"
    );
    assert_eq!(
        invalid(vec![authorized(ISSUER_A, VM_A, 1), authorized(ISSUER_B, VM_A, 2)]),
        "authorization table verification methods are not unique"
    );
    // The identity (y = 1) and the order-4 point (y = 0) decompress but have small order.
    for small in [[1, 0], [0, 0]] {
        let mut entry = authorized(ISSUER_A, VM_A, 1);
        entry.public_key = [0; 32];
        entry.public_key[..2].copy_from_slice(&small);
        assert_eq!(invalid(vec![entry]), "authorized key has small order");
    }
    assert_eq!(invalid(vec![authorized("issuer-a", VM_A, 1)]), "authorization table IRI rejected");
    assert_eq!(
        invalid(vec![authorized(ISSUER_A, &format!("{ISSUER_A}#{}", "k".repeat(512)), 1)]),
        "authorization table IRI rejected"
    );

    // Table order carries no meaning; table content does.
    let base = Policy::new(table());
    let digest = auth::request_digest(&request(query, holder_declared(), base.clone())).unwrap();
    let mut reordered = table();
    reordered.reverse();
    let reordered = Policy::new(reordered);
    assert_eq!(
        auth::request_digest(&request(query, holder_declared(), reordered.clone())).unwrap(),
        digest
    );
    assert_eq!(
        auth::dataset_commitment(&credentials(vec![alice()]), &reordered).unwrap(),
        authenticate(vec![alice()]).unwrap()
    );
    let mut rekeyed = table();
    rekeyed[1].public_key = key(5).verifying_key().to_bytes();
    assert_ne!(
        auth::request_digest(&request(query, holder_declared(), Policy::new(rekeyed))).unwrap(),
        digest
    );

    let mut widened = table();
    widened.push(authorized("https://issuer.example/c", "https://issuer.example/c#key-1", 4));
    let widened = Policy::new(widened);
    let journal = run(query, holder_declared(), widened.clone(), vec![alice()]).unwrap();
    auth::bind_journal(&journal, &request(query, holder_declared(), widened.clone())).unwrap();
    assert_eq!(
        reason(auth::bind_journal(&journal, &request(query, holder_declared(), base.clone()))),
        "authenticated journal request mismatch"
    );
    // A commitment accepted under one policy cannot anchor another policy.
    let commitment = auth::dataset_commitment(&credentials(vec![alice()]), &base).unwrap();
    assert_ne!(journal.dataset_commitment, commitment);
    assert_eq!(
        reason(run(query, DatasetAuthority::VerifierAgreed { commitment }, widened, vec![alice()])),
        "authenticated dataset anchor mismatch"
    );
}

// The commitment API enforces the same V3 policy ceilings as the request.
#[test]
fn dataset_commitment_rejects_evaluation_policies_outside_v3_program_ceilings() {
    fn with_evaluation(edit: impl FnOnce(&mut v3::Policy)) -> Policy {
        let mut policy = Policy::new(table());
        edit(&mut policy.evaluation);
        policy
    }
    const RESOURCE: &str = "V2 resource policy exceeds program bounds";
    const CANONICALIZATION: &str = "V3 canonicalization policy exceeds program bounds";
    let ceiling = v3::CanonicalizationPolicy::default();
    let query = "ASK { ?person ex:name \"Alice\" }";
    let cases = [
        (with_evaluation(|p| p.dataset.max_rows = 0), RESOURCE),
        (with_evaluation(|p| p.dataset.max_rows = MAX_ROWS + 1), RESOURCE),
        (with_evaluation(|p| p.canonicalization.max_quads = 0), CANONICALIZATION),
        (
            with_evaluation(|p| p.canonicalization.max_permutation_steps = ceiling.max_permutation_steps + 1),
            CANONICALIZATION,
        ),
    ];
    for (policy, expected) in cases {
        let expected: Result<(), Rejected> = Err(Rejected(expected));
        assert_eq!(auth::dataset_commitment(&credentials(vec![alice()]), &policy).map(drop), expected);
        assert_eq!(
            auth::validate_request(&request(query, DatasetAuthority::HolderDeclared, policy.clone())),
            expected
        );
        assert_eq!(
            run(query, DatasetAuthority::HolderDeclared, policy.clone(), vec![alice()]).map(drop),
            expected
        );
        // The policy rejects before any table, salt or credential check.
        let unchecked = PrivateCredentials {
            credentials: Vec::new(),
            salt: [0; 32],
        };
        let untabled = Policy {
            authorization: Vec::new(),
            ..policy
        };
        assert_eq!(auth::dataset_commitment(&unchecked, &untabled).map(drop), expected);
    }

    // Positive control: the default policy still commits and anchors evaluation.
    let default = Policy::new(table());
    let commitment = auth::dataset_commitment(&credentials(vec![alice()]), &default).unwrap();
    let journal = run(query, DatasetAuthority::VerifierAgreed { commitment }, default, vec![alice()]).unwrap();
    assert_eq!(journal.result, v3::CanonicalResult::Ask(true));
}

#[test]
fn witness_bounds_reject_before_parsing_and_duplicates_after_canonicalization() {
    let garbage = |document: String| unsigned(&document, "garbage");
    assert_eq!(reason(authenticate(Vec::new())), "authenticated credential count must be 1 to 4");
    assert_eq!(
        reason(authenticate(vec![garbage("x".into()); 5])),
        "authenticated credential count must be 1 to 4"
    );
    for oversized in [
        garbage("x".repeat(MAX_DOCUMENT_BYTES + 1)),
        garbage("x\n".repeat(MAX_DOCUMENT_QUADS + 1)),
        unsigned("x", &"x".repeat(MAX_PROOF_CONFIG_BYTES + 1)),
        unsigned("x", &"x\n".repeat(9)),
    ] {
        assert_eq!(reason(authenticate(vec![oversized])), "authenticated credential input capacity");
    }
    assert_eq!(
        reason(authenticate(vec![garbage("x".repeat(MAX_DOCUMENT_BYTES)); 4])),
        "authenticated credential total capacity"
    );
    assert_eq!(
        reason(authenticate(vec![garbage("x\n".repeat(100)); 3])),
        "authenticated credential total capacity"
    );
    // Within bounds, the same garbage reaches the parser.
    assert_eq!(reason(authenticate(vec![garbage("x".into())])), "authenticated RDF N-Quads parse rejected");
    let mut short = alice();
    short.signature.pop();
    assert_eq!(reason(authenticate(vec![short])), "Ed25519 signature must be 64 bytes");
    let unblinded = PrivateCredentials {
        credentials: vec![alice()],
        salt: [0; 32],
    };
    assert_eq!(
        reason(auth::dataset_commitment(&unblinded, &Policy::new(table()))),
        "authenticated dataset blinding rejected"
    );

    // Duplicate canonical documents reject even under different proofs or labels.
    let resigned = sign(&alice_document(), &with_created(config(VM_A), "2024-01-01T00:00:00Z", "dateTime"), 1);
    let original = named();
    let relabeled = SignedCredential {
        document: original.document.replace("_:subject", "_:other"),
        ..original.clone()
    };
    for duplicate in [vec![alice(), alice()], vec![alice(), resigned], vec![original, relabeled]] {
        assert_eq!(reason(authenticate(duplicate)), "duplicate authenticated credential document");
    }
}

#[test]
fn signed_lexical_forms_are_preserved_in_the_query_dataset() {
    let claims = format!(
        "<did:example:alice> <http://ex/code> \"018\"^^<{XSD}integer> .\n\
         <did:example:alice> <http://ex/ratio> \"1.50\"^^<{XSD}decimal> .\n"
    );
    let signed = || sign(&document("urn:vc:codes", ISSUER_A, &claims), &config(VM_A), 1);
    let journal = holder("SELECT ?code ?ratio WHERE { ?p ex:code ?code ; ex:ratio ?ratio }", vec![signed()]).unwrap();
    assert_eq!(
        rows(&journal.result),
        vec![vec![
            Some(format!("\"018\"^^<{XSD}integer>")),
            Some(format!("\"1.50\"^^<{XSD}decimal>")),
        ]]
    );
    // Value comparison still sees 18 while the lexical form stays "018".
    let ask = holder("ASK { ?p ex:code ?c FILTER(?c = 18 && STR(?c) = \"018\") }", vec![signed()]).unwrap();
    assert_eq!(ask.result, v3::CanonicalResult::Ask(true));
}

#[test]
fn blank_nodes_are_scoped_per_credential_and_never_join() {
    let join = "SELECT ?name ?age WHERE { ?s ex:name ?name ; ex:age ?age }";
    // Both credentials canonicalize their subject to _:c14n0; the scopes must not merge.
    assert!(rows(&holder(join, vec![named(), aged()]).unwrap().result).is_empty());
    let subjects = rows(
        &holder(&format!("SELECT ?s WHERE {{ ?vc <{CRED}credentialSubject> ?s }}"), vec![named(), aged()])
            .unwrap()
            .result,
    );
    assert_eq!(subjects.len(), 2);
    assert_ne!(subjects[0], subjects[1]);

    // Positive control: one credential stating both facts about one node joins.
    let claims = format!(
        "<urn:vc:both> <{CRED}credentialSubject> _:subject .\n\
         _:subject <http://ex/name> \"Shared\" .\n\
         _:subject <http://ex/age> \"41\"^^<{XSD}integer> .\n"
    );
    let both = sign(&document("urn:vc:both", ISSUER_A, &claims), &config(VM_A), 1);
    assert_eq!(
        rows(&holder(join, vec![both]).unwrap().result),
        vec![vec![cell("\"Shared\""), Some(format!("\"41\"^^<{XSD}integer>"))]]
    );
}

#[test]
fn holder_relabeling_and_reordering_leave_commitment_and_journal_unchanged() {
    let reversed = |text: &str| text.lines().rev().map(|line| format!("{line}\n")).collect::<String>();
    let original = vec![named(), aged(), alice()];
    // Same signatures: only labels and statement/credential order change.
    let rewritten = original
        .iter()
        .rev()
        .map(|credential| SignedCredential {
            document: reversed(&credential.document.replace("_:subject", "_:holderChosen")),
            proof_config: reversed(&credential.proof_config.replace("_:proof", "_:p0")),
            signature: credential.signature.clone(),
        })
        .collect();
    let query = "SELECT ?s ?name WHERE { ?s ex:name ?name }";
    let expected = holder(query, original).unwrap();
    assert_eq!(rows(&expected.result).len(), 2);
    assert_eq!(holder(query, rewritten).unwrap(), expected);
}

#[test]
fn journal_version_request_and_result_substitutions_reject() {
    let policy = Policy::new(table());
    let query = "ASK { ?person ex:name \"Alice\" }";
    let expected = request(query, DatasetAuthority::HolderDeclared, policy.clone());
    let journal = holder(query, vec![alice()]).unwrap();
    auth::bind_journal(&journal, &expected).unwrap();

    let mut earlier = journal.clone();
    earlier.version = v3::VERSION;
    assert_eq!(reason(auth::bind_journal(&earlier, &expected)), "authenticated journal request mismatch");
    let mut earlier_request = expected.clone();
    earlier_request.version = v3::VERSION;
    assert_eq!(
        reason(auth::bind_journal(&journal, &earlier_request)),
        "unsupported authenticated RDF version"
    );

    // A genuine journal for another query is not a result for this request.
    let other = holder("ASK { ?person ex:name \"Mallory\" }", vec![alice()]).unwrap();
    assert_eq!(other.result, v3::CanonicalResult::Ask(false));
    assert_eq!(reason(auth::bind_journal(&other, &expected)), "authenticated journal request mismatch");

    let mut renonced = expected.clone();
    renonced.nonce[0] ^= 1;
    let mut tightened = expected.clone();
    tightened.policy.evaluation.dataset.max_rows -= 1;
    for substituted in [renonced, tightened] {
        assert_eq!(
            reason(auth::bind_journal(&journal, &substituted)),
            "authenticated journal request mismatch"
        );
    }

    // The embedded V3 request keeps its own digest domain.
    let inner = v3::Request {
        version: v3::VERSION,
        contract: ProofContract::ExactDataset,
        dialect: v3::Dialect::SparqSparql11GraphResultsV3,
        query: expected.query.clone(),
        authority: DatasetAuthority::HolderDeclared,
        policy: policy.evaluation,
        nonce: expected.nonce,
    };
    assert_ne!(v3::request_digest(&inner).unwrap(), journal.request_digest);
}

// Revealed mode: the witness carries no signature; the verifier checks it.
fn strip(credential: SignedCredential) -> (SignedCredential, Vec<u8>) {
    let signature = credential.signature.clone();
    (SignedCredential { signature: Vec::new(), ..credential }, signature)
}

fn revealed() -> Policy {
    Policy::new(table()).with_signature_mode(auth::SignatureMode::Revealed)
}

#[test]
fn revealed_mode_publishes_exact_signed_messages_and_hidden_digests_are_unchanged() {
    assert_eq!(Policy::new(table()).signature_mode, auth::SignatureMode::Hidden);
    assert_eq!(auth::SignatureMode::Hidden.as_str(), "hidden");
    assert_eq!(auth::SignatureMode::Revealed.as_str(), "revealed");
    // A hidden policy serialized before the field existed decodes to Hidden.
    let mut json = serde_json::to_value(Policy::new(table())).unwrap();
    json.as_object_mut().unwrap().remove("signature_mode");
    assert_eq!(serde_json::from_value::<Policy>(json).unwrap(), Policy::new(table()));

    let query = "SELECT ?n WHERE { ?s ex:name ?n }";
    let hidden = holder(query, vec![alice(), bob()]).unwrap();
    assert!(hidden.signed_messages.is_empty());
    let (a, sig_a) = strip(alice());
    let (b, sig_b) = strip(bob());
    let journal = run(query, DatasetAuthority::HolderDeclared, revealed(), vec![a.clone(), b.clone()]).unwrap();
    assert_eq!(journal.result, hidden.result);
    // The policy digest, and so the commitment and request digest, bind the mode.
    assert_ne!(journal.dataset_commitment, hidden.dataset_commitment);
    assert_ne!(journal.request_digest, hidden.request_digest);
    assert_eq!(journal.signed_messages.len(), 2);
    let mut expected = Vec::new();
    for (credential, method) in [(&alice(), VM_A), (&bob(), VM_B)] {
        let config_hash = Sha256::digest(sparq_canon::canonicalize_nquads(&credential.proof_config).unwrap());
        let document_hash = Sha256::digest(sparq_canon::canonicalize_nquads(&credential.document).unwrap());
        expected.push((document_hash.to_vec(), method, [config_hash.as_slice(), document_hash.as_slice()].concat()));
    }
    expected.sort();
    for (entry, (_, method, message)) in journal.signed_messages.iter().zip(&expected) {
        assert_eq!(entry.verification_method, *method);
        assert_eq!(&entry.message, message);
    }
    let req = request(query, DatasetAuthority::HolderDeclared, revealed());
    auth::bind_journal(&journal, &req).unwrap();
    // Signatures go in journal order (sorted by document hash).
    let ordered: Vec<Vec<u8>> = journal
        .signed_messages
        .iter()
        .map(|m| if m.verification_method == VM_A { sig_a.clone() } else { sig_b.clone() })
        .collect();
    auth::check_revealed_signatures(&journal, &req, &ordered).unwrap();
    let swapped: Vec<Vec<u8>> = ordered.iter().rev().cloned().collect();
    assert_eq!(reason(auth::check_revealed_signatures(&journal, &req, &swapped)), "Ed25519 signature verification failed");
    assert_eq!(reason(auth::check_revealed_signatures(&journal, &req, &ordered[..1])), "revealed signature count mismatch");
    let mut forged = ordered.clone();
    forged[0][0] ^= 1;
    assert!(auth::check_revealed_signatures(&journal, &req, &forged).is_err());
    assert_eq!(reason(auth::check_revealed_signatures(&journal, &req, &[vec![0; 63], ordered[1].clone()])), "Ed25519 signature must be 64 bytes");
    // A hidden request never accepts a revealed journal, and the reverse.
    let hidden_req = request(query, DatasetAuthority::HolderDeclared, Policy::new(table()));
    assert!(auth::bind_journal(&journal, &hidden_req).is_err());
    assert!(auth::bind_journal(&hidden, &req).is_err());
    assert_eq!(reason(auth::check_revealed_signatures(&hidden, &hidden_req, &[])), "revealed signature count mismatch");
    let mut emptied = journal.clone();
    emptied.signed_messages.clear();
    assert_eq!(reason(auth::bind_journal(&emptied, &req)), "authenticated journal signature mode mismatch");
}

#[test]
fn revealed_mode_rejects_witness_signatures_and_still_enforces_the_table() {
    let query = "ASK { ?s ex:name ?n }";
    assert_eq!(
        reason(run(query, DatasetAuthority::HolderDeclared, revealed(), vec![alice()])),
        "revealed mode carries no witness signature"
    );
    let (a, _) = strip(alice());
    assert_eq!(
        reason(run(query, DatasetAuthority::HolderDeclared, Policy::new(table()), vec![a])),
        "Ed25519 signature must be 64 bytes"
    );
    // Unlisted methods and issuer mismatches still reject inside the proof.
    let (unlisted, _) = strip(sign(&alice_document(), &config(VM_UNLISTED), 1));
    assert_eq!(
        reason(run(query, DatasetAuthority::HolderDeclared, revealed(), vec![unlisted])),
        "verification method is not authorized"
    );
    // A verifier-agreed anchor binds the mode it was computed under.
    let (a, _) = strip(alice());
    let anchor = auth::dataset_commitment(&credentials(vec![a.clone()]), &revealed()).unwrap();
    let journal =
        run(query, DatasetAuthority::VerifierAgreed { commitment: anchor }, revealed(), vec![a.clone()]).unwrap();
    assert_eq!(journal.provenance, Provenance::VerifierAgreedAuthenticated);
    let hidden_anchor = authenticate(vec![alice()]).unwrap();
    assert_eq!(
        reason(run(query, DatasetAuthority::VerifierAgreed { commitment: hidden_anchor }, revealed(), vec![a])),
        "authenticated dataset anchor mismatch"
    );
}

// eddsa-sha256-merkle-2026: issue by ordering the parsed quads by leaf.
fn merkle_config(method: &str) -> String {
    config(method).replace("\"eddsa-rdfc-2022\"", "\"eddsa-sha256-merkle-2026\"")
}

fn merkle_parts(document: &str, config: &str, salt: [u8; 32]) -> (String, [u8; 32]) {
    use oxrdf::Quad;
    let quads: Vec<Quad> = oxttl::NQuadsParser::new()
        .for_slice(document.as_bytes())
        .collect::<Result<_, _>>()
        .unwrap();
    let mut leaves: Vec<([u8; 32], &Quad)> = quads.iter().map(|q| (merkle::leaf(q).unwrap(), q)).collect();
    leaves.sort_by_key(|(leaf, _)| *leaf);
    leaves.dedup_by_key(|(leaf, _)| *leaf);
    let ordered: String = leaves.iter().map(|(_, q)| format!("{q} .\n")).collect();
    let row: Vec<[u8; 32]> = leaves.iter().map(|(leaf, _)| *leaf).collect();
    let root = merkle::root(&row).unwrap();
    let config_hash: [u8; 32] = Sha256::digest(sparq_canon::canonicalize_nquads(config).unwrap()).into();
    (ordered, merkle::signed_message(&salt, row.len() as u32, &root, &config_hash))
}

fn merkle_sign(document: &str, config: &str, seed: u8, salt: [u8; 32]) -> SignedCredential {
    let (ordered, message) = merkle_parts(document, config, salt);
    let mut proof_value = key(seed).sign(&message).to_bytes().to_vec();
    proof_value.extend_from_slice(&salt);
    SignedCredential {
        document: ordered,
        proof_config: config.into(),
        signature: proof_value,
    }
}

fn merkle_policy() -> Policy {
    Policy::new(table()).with_cryptosuite(auth::Cryptosuite::EddsaSha256Merkle2026)
}

fn merkle_alice() -> SignedCredential {
    let claims = format!(
        "<urn:vc:m-alice> <{CRED}credentialSubject> _:subject .\n\
         _:subject <http://ex/name> \"Alice\" .\n\
         _:subject <http://ex/balance> \"1250.50\"^^<{XSD}decimal> .\n"
    );
    merkle_sign(&document("urn:vc:m-alice", ISSUER_A, &claims), &merkle_config(VM_A), 1, [0x41; 32])
}

fn merkle_bob() -> SignedCredential {
    let claims = format!("<did:example:bob> <http://ex/balance> \"99\"^^<{XSD}integer> .\n");
    merkle_sign(&document("urn:vc:m-bob", ISSUER_B, &claims), &merkle_config(VM_B), 2, [0x42; 32])
}

#[test]
fn merkle_suite_answers_with_blank_nodes_in_both_modes() {
    let query = "SELECT ?b WHERE { ?s ex:balance ?b FILTER(?b > 100) }";
    let hidden = run(query, DatasetAuthority::HolderDeclared, merkle_policy(), vec![merkle_alice(), merkle_bob()]).unwrap();
    assert!(hidden.signed_messages.is_empty());
    let rows = format!("{:?}", hidden.result);
    assert!(rows.contains("1250.50") && !rows.contains("\"99\""), "{rows}");
    // The eddsa-rdfc-2022 policy never accepts Merkle credentials, nor the reverse.
    assert_eq!(
        reason(run(query, DatasetAuthority::HolderDeclared, Policy::new(table()), vec![merkle_bob()])),
        "Ed25519 signature must be 64 bytes"
    );
    let mut eddsa = bob();
    eddsa.signature.extend_from_slice(&[0x42; 32]);
    assert_eq!(
        reason(run(query, DatasetAuthority::HolderDeclared, merkle_policy(), vec![eddsa])),
        "cryptosuite must be the policy's typed cryptosuite value"
    );

    // Revealed: the witness keeps only the salt; the message is salted.
    let revealed_policy = merkle_policy().with_signature_mode(auth::SignatureMode::Revealed);
    let (alice, bob) = (merkle_alice(), merkle_bob());
    let signatures: Vec<Vec<u8>> = [&alice, &bob].iter().map(|c| c.signature[..64].to_vec()).collect();
    let strip = |c: &SignedCredential| SignedCredential { signature: c.signature[64..].to_vec(), ..c.clone() };
    let journal = run(
        query,
        DatasetAuthority::HolderDeclared,
        revealed_policy.clone(),
        vec![strip(&alice), strip(&bob)],
    )
    .unwrap();
    assert_eq!(journal.result, hidden.result);
    assert_eq!(journal.signed_messages.len(), 2);
    let req = request(query, DatasetAuthority::HolderDeclared, revealed_policy.clone());
    let mut ordered = signatures.clone();
    let alice_message = merkle_parts(&alice.document, &alice.proof_config, [0x41; 32]).1;
    if journal.signed_messages[0].message != alice_message {
        ordered.reverse();
    }
    for entry in &journal.signed_messages {
        assert_eq!(entry.message.len(), 32);
    }
    auth::check_revealed_signatures(&journal, &req, &ordered).unwrap();
    ordered.reverse();
    assert!(auth::check_revealed_signatures(&journal, &req, &ordered).is_err());
    // A witness that still carries the signature in revealed mode rejects.
    assert_eq!(
        reason(run(query, DatasetAuthority::HolderDeclared, revealed_policy, vec![alice])),
        "Merkle proof value must be the signature (hidden mode only) and salt"
    );
}

#[test]
fn merkle_credential_order_depends_only_on_salted_messages() {
    // A known reference credential next to a private one: the published order must
    // follow the salted messages, never the unsalted roots.
    let query = "ASK { ?s ex:balance ?b }";
    let policy = merkle_policy().with_signature_mode(auth::SignatureMode::Revealed);
    let reference = merkle_bob();
    let (_, reference_message) = merkle_parts(&reference.document, &reference.proof_config, [0x42; 32]);
    let identity = |c: &SignedCredential| {
        let (_, root, n) = {
            let quads: Vec<oxrdf::Quad> = oxttl::NQuadsParser::new()
                .for_slice(c.document.as_bytes())
                .collect::<Result<_, _>>()
                .unwrap();
            let leaves: Vec<[u8; 32]> = quads.iter().map(|q| merkle::leaf(q).unwrap()).collect();
            ((), merkle::root(&leaves).unwrap(), leaves.len() as u32)
        };
        let mut hash = Sha256::new();
        hash.update(n.to_be_bytes());
        hash.update(root);
        <[u8; 32]>::from(hash.finalize())
    };
    let mut disagreements = 0;
    for amount in 100..140 {
        let claims = format!("<did:example:carol> <http://ex/balance> \"{amount}\"^^<{XSD}integer> .\n");
        let private = merkle_sign(&document("urn:vc:m-carol", ISSUER_A, &claims), &merkle_config(VM_A), 1, [0x44; 32]);
        let (_, private_message) = merkle_parts(&private.document, &private.proof_config, [0x44; 32]);
        let strip = |c: &SignedCredential| SignedCredential { signature: c.signature[64..].to_vec(), ..c.clone() };
        let journal =
            run(query, DatasetAuthority::HolderDeclared, policy.clone(), vec![strip(&reference), strip(&private)]).unwrap();
        let by_message = private_message < reference_message;
        let by_identity = identity(&private) < identity(&reference);
        disagreements += usize::from(by_message != by_identity);
        let first = &journal.signed_messages[0].message;
        assert_eq!(first == &private_message.to_vec(), by_message, "amount {amount}");
    }
    // The loop exercises cases where the two orders differ.
    assert!(disagreements > 0);
}

#[test]
fn merkle_suite_rejects_reordered_tampered_or_resalted_credentials() {
    let query = "ASK { ?s ex:balance ?b }";
    let run_one = |credential: SignedCredential| {
        run(query, DatasetAuthority::HolderDeclared, merkle_policy(), vec![credential])
    };
    run_one(merkle_bob()).unwrap();
    let mut reordered = merkle_bob();
    let mut lines: Vec<&str> = reordered.document.lines().collect();
    lines.reverse();
    reordered.document = lines.join("\n") + "\n";
    assert_eq!(reason(run_one(reordered)), "Merkle leaves must be non-empty and strictly increasing");
    let mut duplicated = merkle_bob();
    let first = duplicated.document.lines().next().unwrap().to_owned();
    duplicated.document = format!("{first}\n{}", duplicated.document);
    assert_eq!(reason(run_one(duplicated)), "Merkle leaves must be non-empty and strictly increasing");
    let mut tampered = merkle_bob();
    tampered.document = tampered.document.replace("\"99\"", "\"990\"");
    // Changing a leaf may also break the order; either way it never verifies.
    assert!(run_one(tampered).is_err());
    let mut resalted = merkle_bob();
    resalted.signature[64] ^= 1;
    assert_eq!(reason(run_one(resalted)), "Ed25519 signature verification failed");
    // The same document reissued under a fresh salt is still a duplicate.
    let claims = format!("<did:example:bob> <http://ex/balance> \"99\"^^<{XSD}integer> .\n");
    let reissued = merkle_sign(&document("urn:vc:m-bob", ISSUER_B, &claims), &merkle_config(VM_B), 2, [0x43; 32]);
    assert_eq!(
        reason(run(query, DatasetAuthority::HolderDeclared, merkle_policy(), vec![merkle_bob(), reissued])),
        "duplicate authenticated credential document"
    );
    let mut short = merkle_bob();
    short.signature.truncate(64);
    assert_eq!(reason(run_one(short)), "Merkle proof value must be the signature (hidden mode only) and salt");
}

#[test]
fn phases_are_reported_in_order_and_do_not_change_the_journal() {
    use auth::Phase::*;
    let query = "SELECT ?n WHERE { ?s ex:name ?n }";
    let witness = |policy: Policy, list| Witness {
        request: request(query, DatasetAuthority::HolderDeclared, policy),
        dataset: credentials(list),
    };
    let hidden = witness(Policy::new(table()), vec![alice(), bob()]);
    let mut phases = Vec::new();
    let journal = auth::evaluate_observed(&hidden, &mut |phase| phases.push(phase)).unwrap();
    assert_eq!(journal, auth::evaluate(&hidden).unwrap());
    let per = [ProofConfig, Document, Signature];
    let expected: Vec<_> = [Request].into_iter().chain(per).chain(per).chain([Mapping, Query, Journal]).collect();
    assert_eq!(phases, expected);
    let (a, _) = strip(alice());
    let revealed = witness(Policy::new(table()).with_signature_mode(auth::SignatureMode::Revealed), vec![a]);
    phases.clear();
    auth::evaluate_observed(&revealed, &mut |phase| phases.push(phase)).unwrap();
    assert_eq!(phases, [Request, ProofConfig, Document, Mapping, Query, Journal]);
}
