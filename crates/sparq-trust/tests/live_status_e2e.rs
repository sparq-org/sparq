//! End-to-end P6 status stratum (`sq-pfae.7`): the REAL admission → derivation pipeline
//! gated on a **live W3C Bitstring Status List**, plus the minimal PROV-O denial
//! justification.
//!
//! It exercises the REAL path — a CHECKED issuer Schnorr signature over the RDFC-1.0
//! commitment (sparq-zk), statement-type scoping via a real SHACL shape (sparq-shacl),
//! freshness, the clear-WebID holder binding, AND the new live-status gate
//! ([`sparq_trust::admit_with_status`]) over a decoded Bitstring Status List — then the N3
//! merge of the admitted fact with the `.acr` ABAC rule (sparq-reason) to derive the
//! `auth:read` grant. The load-bearing INVARIANT under test: the live-status gate is
//! **fail-closed** — only an unset-bit, in-range, fresh, resolvable status admits; a set
//! bit, an unknown/undecodable list, or a stale snapshot ALL deny the whole credential, and
//! the deny carries a minimal justification recording WHY.
//!
//! This suite is `#![cfg(feature = "status-list")]`, so it ONLY compiles/runs with the
//! feature ON (wired as the trailing `sparq-trust (status-list)` leg in feature-matrix.yml).
//!
//! sq-pfae.7 (issue #940 / gh-940). 🤖 SPARQ agent — trust-graph live-status gate.
#![cfg(feature = "status-list")]

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_trust::admit::{admit_with_status, PresentedCredential, Session};
use sparq_trust::policy::{parse_policy, ControlGate, TrustRule};
use sparq_trust::status_list::{
    justify_status_decision, IdentityGzipDecoder, LiveStatus, LiveStatusCheck, StatusListEntry,
    StatusListResolver, StatusPurpose,
};
use sparq_trust::vocab;
use sparq_trust::wire::derive_grants;
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::sig::{public_key_to_hex, PublicKey, SecretKey};

const JESSE: &str = "https://jesse.ex/card#me";
const GOV_ISSUER: &str = "https://gov.example/issuer";
const SCHEMA_AGE: &str = "http://schema.org/age";
const RESOURCE_X: &str = "https://pod.ex/resourceX";
const STATUS_LIST: &str = "https://gov.example/status/3#list";
const SALT_BYTES: [u8; 32] = [7u8; 32];
const NOW: i64 = 1_700_000_000;
const ISSUED_FRESH: i64 = NOW - 86_400; // 1 day ago

const ACR_ABAC_RULE: &str = r#"@prefix schema: <http://schema.org/> .
@prefix math:   <http://www.w3.org/2000/10/swap/math#> .
@prefix auth:   <https://sparq.dev/ns/auth#> .
{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <https://pod.ex/resourceX> } .
"#;

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

fn gov_key() -> (SecretKey, PublicKey) {
    let sk = SecretKey::from_seed(0xC0FFEE);
    let pk = sk.public_key();
    (sk, pk)
}

fn age_graph(subject: &str, value: &str) -> Vec<Triple> {
    vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri(subject)),
        iri(SCHEMA_AGE),
        Term::Literal(Literal::new_typed_literal(
            value,
            iri("http://www.w3.org/2001/XMLSchema#integer"),
        )),
    )]
}

fn sign_graph(sk: &SecretKey, graph: &[Triple]) -> String {
    let salt = salt_from_bytes(&SALT_BYTES);
    let commitment = commit_triples(graph, salt).expect("commits");
    sk.sign_commitment(&commitment.commitment)
}

fn fresh_cred(sk: &SecretKey, graph: Vec<Triple>) -> PresentedCredential {
    let sig = sign_graph(sk, &graph);
    PresentedCredential {
        graph,
        issuer_signature_hex: sig,
        salt: SALT_BYTES,
        issued_at_unix_secs: ISSUED_FRESH,
        revoked: false,
    }
}

fn gov_age_policy(key: &PublicKey) -> Vec<TrustRule> {
    let rule = NamedNode::new("https://pod.ex/.acr#trustrule").unwrap();
    let policy: Vec<Triple> = vec![
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::RDF_TYPE),
            Term::NamedNode(iri(vocab::TRUST_RULE)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::SOURCE),
            Term::NamedNode(iri(GOV_ISSUER)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::ISSUER_KEY),
            Term::Literal(Literal::new_simple_literal(public_key_to_hex(key))),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::FOR_PREDICATE),
            Term::NamedNode(iri(SCHEMA_AGE)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::SCOPE),
            Term::NamedNode(iri(RESOURCE_X)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule),
            iri(vocab::FRESH_WITHIN),
            Term::Literal(Literal::new_typed_literal(
                "P30D",
                iri("http://www.w3.org/2001/XMLSchema#duration"),
            )),
        ),
    ];
    parse_policy(&policy, ControlGate::assert_control_gated()).expect("policy parses")
}

fn session(now: i64) -> Session {
    Session {
        agent: iri(JESSE),
        now_unix_secs: now,
    }
}

fn status_entry(index: u64) -> StatusListEntry {
    StatusListEntry {
        status_list_credential: iri(STATUS_LIST),
        status_list_index: index,
        purpose: StatusPurpose::Revocation,
    }
}

/// base64url-no-pad encode (the encoder side of the crate's decoder).
fn base64url_encode(input: &[u8]) -> String {
    const ALPHA: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut acc: u32 = 0;
    let mut nbits: u32 = 0;
    for &b in input {
        acc = (acc << 8) | b as u32;
        nbits += 8;
        while nbits >= 6 {
            nbits -= 6;
            out.push(ALPHA[((acc >> nbits) & 0x3f) as usize] as char);
        }
    }
    if nbits > 0 {
        out.push(ALPHA[((acc << (6 - nbits)) & 0x3f) as usize] as char);
    }
    out
}

/// An in-memory status-list resolver: maps the one list IRI to its encoded bytes + as_of.
struct MapResolver {
    encoded: Option<(String, i64)>,
}
impl StatusListResolver for MapResolver {
    fn resolve(&self, _iri: &NamedNode) -> Option<(String, i64)> {
        self.encoded.clone()
    }
}

/// A status list (multibase `u` + identity codec) with `set_indices` marked revoked.
fn status_list(set_indices: &[u64], n_bytes: usize, as_of: i64) -> MapResolver {
    let mut bits = vec![0u8; n_bytes];
    for &i in set_indices {
        let byte = (i >> 3) as usize;
        let bit = (i & 7) as u8;
        bits[byte] |= 0x80 >> bit; // MSB-first
    }
    let encoded = format!("u{}", base64url_encode(&bits));
    MapResolver {
        encoded: Some((encoded, as_of)),
    }
}

fn read_granted(grants: &[Triple]) -> bool {
    grants.iter().any(|t| {
        t.predicate.as_str() == "https://sparq.dev/ns/auth#read"
            && matches!(&t.subject, NamedOrBlankNode::NamedNode(n) if n.as_str() == JESSE)
            && matches!(&t.object, Term::NamedNode(o) if o.as_str() == RESOURCE_X)
    })
}

#[test]
fn live_unset_status_grants_read_end_to_end() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(&pk);
    let cred = fresh_cred(&sk, age_graph(JESSE, "25"));
    // The credential is at index 5; only index 9 is revoked ⇒ index 5 is LIVE.
    let resolver = status_list(&[9], 4, NOW - 60); // fresh snapshot (1 min old)
    let check = LiveStatusCheck::new(resolver, IdentityGzipDecoder, 3600);

    let (admitted, status) = admit_with_status(
        &cred,
        &rules,
        &session(NOW),
        &iri(RESOURCE_X),
        &status_entry(5),
        &check,
    );
    assert_eq!(status, LiveStatus::Live, "unset, fresh, in-range ⇒ Live");
    assert!(status.admits());
    let grants = derive_grants(&admitted, ACR_ABAC_RULE).expect("derivation runs");
    assert!(
        read_granted(&grants),
        "a LIVE credential ⇒ the age>18 rule derives <Jesse> auth:read <resourceX>"
    );
}

#[test]
fn revoked_status_denies_with_minimal_justification() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(&pk);
    let cred = fresh_cred(&sk, age_graph(JESSE, "25"));
    // The credential is at index 5 AND index 5 is revoked ⇒ DENY.
    let resolver = status_list(&[5], 4, NOW - 60);
    let check = LiveStatusCheck::new(resolver, IdentityGzipDecoder, 3600);

    let (admitted, status) = admit_with_status(
        &cred,
        &rules,
        &session(NOW),
        &iri(RESOURCE_X),
        &status_entry(5),
        &check,
    );
    assert_eq!(status, LiveStatus::Set, "the bit is set ⇒ revoked");
    assert!(!status.admits(), "fail-closed: a set bit denies");
    assert!(admitted.is_empty(), "a revoked credential admits NOTHING");
    let grants = derive_grants(&admitted, ACR_ABAC_RULE).expect("derivation runs");
    assert!(
        !read_granted(&grants),
        "no admitted fact ⇒ no grant derived"
    );

    // The minimal DENIAL justification records WHY (the bead's "minimal denial justification").
    let would_be_grant = Triple::new(
        NamedOrBlankNode::NamedNode(iri(JESSE)),
        iri("https://sparq.dev/ns/auth#read"),
        Term::NamedNode(iri(RESOURCE_X)),
    );
    let j = justify_status_decision(&would_be_grant, &status_entry(5), status, Some(NOW));
    let joined: String = j
        .triples()
        .iter()
        .map(|t| format!("{} {} {}\n", t.subject, t.predicate, t.object))
        .collect();
    assert!(
        joined.contains("status-set"),
        "deny reason is recorded:\n{}",
        joined
    );
    assert!(joined.contains("statusAdmits") && joined.contains("false"));
    assert!(
        joined.contains("statusListIndex"),
        "binds the checked index"
    );
}

#[test]
fn stale_status_denies_fail_closed() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(&pk);
    let cred = fresh_cred(&sk, age_graph(JESSE, "25"));
    // The snapshot is 2 days old; max_age is 1 hour ⇒ STALE (deny even though unset).
    let resolver = status_list(&[], 4, NOW - 2 * 86_400);
    let check = LiveStatusCheck::new(resolver, IdentityGzipDecoder, 3600);

    let (admitted, status) = admit_with_status(
        &cred,
        &rules,
        &session(NOW),
        &iri(RESOURCE_X),
        &status_entry(5),
        &check,
    );
    assert_eq!(
        status,
        LiveStatus::Stale,
        "stale snapshot ⇒ fail-closed deny"
    );
    assert!(!status.admits());
    assert!(
        admitted.is_empty(),
        "a stale-status credential admits NOTHING"
    );
    let grants = derive_grants(&admitted, ACR_ABAC_RULE).unwrap();
    assert!(!read_granted(&grants));
}

#[test]
fn unknown_status_denies_fail_closed() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(&pk);
    let cred = fresh_cred(&sk, age_graph(JESSE, "25"));
    // The list does not resolve ⇒ UNKNOWN (deny — fail-closed, never fail-open).
    let resolver = MapResolver { encoded: None };
    let check = LiveStatusCheck::new(resolver, IdentityGzipDecoder, 3600);

    let (admitted, status) = admit_with_status(
        &cred,
        &rules,
        &session(NOW),
        &iri(RESOURCE_X),
        &status_entry(5),
        &check,
    );
    assert_eq!(
        status,
        LiveStatus::Unknown,
        "unresolvable list ⇒ fail-closed deny"
    );
    assert!(!status.admits());
    assert!(
        admitted.is_empty(),
        "an unknown-status credential admits NOTHING"
    );
    let grants = derive_grants(&admitted, ACR_ABAC_RULE).unwrap();
    assert!(!read_granted(&grants));
}

#[test]
fn out_of_range_index_denies_fail_closed() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(&pk);
    let cred = fresh_cred(&sk, age_graph(JESSE, "25"));
    // The list is 1 byte (indices 0..=7); the credential claims index 100 ⇒ not covered.
    let resolver = status_list(&[], 1, NOW - 60);
    let check = LiveStatusCheck::new(resolver, IdentityGzipDecoder, 3600);

    let (admitted, status) = admit_with_status(
        &cred,
        &rules,
        &session(NOW),
        &iri(RESOURCE_X),
        &status_entry(100),
        &check,
    );
    assert_eq!(
        status,
        LiveStatus::Unknown,
        "out-of-range index ⇒ fail-closed Unknown"
    );
    assert!(admitted.is_empty());
}
