// [OPUS-5.5] Native V5 authenticated-RDF model tests; no guest, receipt or proof coverage.
#![cfg(feature = "authenticated-rdf")]
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator_model::authenticated_rdf::{
    self as auth, AuthorizedKey, Journal, MAX_DOCUMENT_BYTES, MAX_DOCUMENT_QUADS,
    MAX_PROOF_CONFIG_BYTES, Policy, PrivateCredentials, Provenance, Request, SignedCredential,
    Witness,
};
use sparq_proved_evaluator_model::{DatasetAuthority, ProofContract, Rejected, RowOrder, v3};

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
            "cryptosuite must be the typed eddsa-rdfc-2022 value",
        ),
        // The plain-literal form is not the standard cryptosuiteString value.
        (
            sign(&alice, &config(VM_A).replace(&typed_suite, "\"eddsa-rdfc-2022\""), 1),
            "cryptosuite must be the typed eddsa-rdfc-2022 value",
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
