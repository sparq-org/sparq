//! End-to-end: DID-bound issuer key (`trust:issuerDid`) drives the SAME admission gate
//! as the operator-asserted `trust:issuerKey` hex — the `sq-pfae.3` issuer-key binding.
//!
//! It exercises the REAL path: a Control-gated trust policy whose rule names its issuer
//! by `trust:issuerDid` (a `did:key` minted from the gov issuer's verifying key), is
//! resolved via [`resolve_rule_keys`] + a [`DidKeyResolver`] into a concrete `TrustRule`,
//! then run through the unchanged admission gate (RDFC-1.0 commit → CHECKED issuer
//! signature → SHACL statement-type scope → freshness → holder binding) and the N3 merge
//! with the `.acr` ABAC rule. The load-bearing invariant: **result-equivalence** — the
//! DID-bound policy admits and grants EXACTLY what the operator-asserted policy does.
//!
//! Fail-closed negatives (each MUST deny): a DID that resolves to the WRONG key, an
//! unresolvable / unsupported-method DID, and a `did:web` document with no usable key.
//! A `did:web` positive runs through an in-memory fetcher (no network).
//!
//! Only compiled with the opt-in `did` feature.
//!
//! sq-pfae.3 (issue #940 / gh-940). 🤖 SPARQ agent — trust-graph authz PoC.
#![cfg(feature = "did")]

use std::collections::HashMap;

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_trust::admit::{admit, PresentedCredential, Session};
use sparq_trust::did::{did_key_for, DidDocumentFetcher, DidKeyResolver, DidWebResolver};
use sparq_trust::policy::{parse_policy, resolve_rule_keys, ControlGate, TrustRule};
use sparq_trust::vocab;
use sparq_trust::wire::derive_grants;
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::sig::{public_key_to_hex, PublicKey, SecretKey};

const JESSE: &str = "https://jesse.ex/card#me";
const GOV_ISSUER: &str = "https://gov.example/issuer";
const SCHEMA_AGE: &str = "http://schema.org/age";
const RESOURCE_X: &str = "https://pod.ex/resourceX";
const SALT_BYTES: [u8; 32] = [7u8; 32];
const NOW: i64 = 1_700_000_000;
const ISSUED_FRESH: i64 = NOW - 86_400;

const ACR_ABAC_RULE: &str = r#"@prefix schema: <http://schema.org/> .
@prefix math:   <http://www.w3.org/2000/10/swap/math#> .
@prefix auth:   <https://sparq.dev/ns/auth#> .
{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <https://pod.ex/resourceX> } .
"#;

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

fn age_credential_graph(subject: &str, value: &str) -> Vec<Triple> {
    vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri(subject)),
        iri(SCHEMA_AGE),
        Term::Literal(Literal::new_typed_literal(
            value,
            iri("http://www.w3.org/2001/XMLSchema#integer"),
        )),
    )]
}

fn gov_key() -> (SecretKey, PublicKey) {
    let sk = SecretKey::from_seed(0xC0FFEE);
    let pk = sk.public_key();
    (sk, pk)
}

fn sign_graph(sk: &SecretKey, graph: &[Triple]) -> String {
    let salt = salt_from_bytes(&SALT_BYTES);
    let commitment = commit_triples(graph, salt).expect("credential graph commits");
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

fn session(agent: &str, now: i64) -> Session {
    Session {
        agent: iri(agent),
        now_unix_secs: now,
    }
}

/// Build a gov-trusts-age policy GRAPH whose rule binds the issuer via `binding`
/// (either a `trust:issuerKey` hex literal or a `trust:issuerDid` string literal).
fn gov_age_policy_graph(issuer: &str, binding: (&str, &str), scope: &str) -> Vec<Triple> {
    let (key_pred, key_lex) = binding;
    let rule = NamedNode::new("https://pod.ex/.acr#trustrule").unwrap();
    vec![
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::RDF_TYPE),
            Term::NamedNode(iri(vocab::TRUST_RULE)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::SOURCE),
            Term::NamedNode(iri(issuer)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(key_pred),
            Term::Literal(Literal::new_simple_literal(key_lex)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::FOR_PREDICATE),
            Term::NamedNode(iri(SCHEMA_AGE)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::SCOPE),
            Term::NamedNode(iri(scope)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule),
            iri(vocab::FRESH_WITHIN),
            Term::Literal(Literal::new_typed_literal(
                "P30D",
                iri("http://www.w3.org/2001/XMLSchema#duration"),
            )),
        ),
    ]
}

fn read_granted(grants: &[Triple], subject: &str) -> bool {
    grants.iter().any(|t| {
        t.predicate.as_str() == "https://sparq.dev/ns/auth#read"
            && matches!(&t.subject, NamedOrBlankNode::NamedNode(n) if n.as_str() == subject)
            && matches!(&t.object, Term::NamedNode(o) if o.as_str() == RESOURCE_X)
    })
}

fn run(rules: &[TrustRule]) -> Vec<Triple> {
    let (sk, _) = gov_key();
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));
    let admitted = admit(&cred, rules, &session(JESSE, NOW), &iri(RESOURCE_X));
    derive_grants(&admitted, ACR_ABAC_RULE).expect("derivation runs")
}

// --- the load-bearing invariant: result-equivalence -------------------------

#[test]
fn did_key_bound_issuer_grants_read_identically_to_operator_asserted_key() {
    let (_sk, pk) = gov_key();

    // Path A: operator-asserted hex key (today's path).
    let policy_key = gov_age_policy_graph(
        GOV_ISSUER,
        (vocab::ISSUER_KEY, &public_key_to_hex(&pk)),
        RESOURCE_X,
    );
    let rules_key = parse_policy(&policy_key, ControlGate::assert_control_gated()).unwrap();

    // Path B: a did:key minted from the SAME key, resolved by the offline DidKeyResolver.
    let did = did_key_for(&pk);
    let policy_did = gov_age_policy_graph(GOV_ISSUER, (vocab::ISSUER_DID, &did), RESOURCE_X);
    let rules_did = resolve_rule_keys(
        &policy_did,
        &DidKeyResolver,
        ControlGate::assert_control_gated(),
    )
    .unwrap();

    // The resolved key MUST equal the operator-asserted key (binding correctness).
    assert_eq!(
        rules_did[0].issuer_key, rules_key[0].issuer_key,
        "the did:key resolves to the SAME issuer verifying key the operator would paste"
    );

    // And the end-to-end grant is IDENTICAL (result-equivalence).
    let grants_key = run(&rules_key);
    let grants_did = run(&rules_did);
    assert!(read_granted(&grants_key, JESSE), "operator-asserted path grants read");
    assert!(
        read_granted(&grants_did, JESSE),
        "did:key-bound path grants read end-to-end — identical to the operator path"
    );
}

// --- fail-closed negatives --------------------------------------------------

#[test]
fn did_key_bound_to_the_wrong_key_does_not_admit() {
    // The policy's DID resolves to a DIFFERENT key than the one that signed the
    // credential ⇒ the CHECKED signature fails ⇒ no admission ⇒ no access.
    let wrong = SecretKey::from_seed(0xDECAF).public_key();
    let did = did_key_for(&wrong);
    let policy = gov_age_policy_graph(GOV_ISSUER, (vocab::ISSUER_DID, &did), RESOURCE_X);
    let rules =
        resolve_rule_keys(&policy, &DidKeyResolver, ControlGate::assert_control_gated()).unwrap();
    let grants = run(&rules);
    assert!(
        !read_granted(&grants, JESSE),
        "a did:key bound to the wrong issuer key fails the signature check ⇒ no access"
    );
}

#[test]
fn unresolvable_did_rejects_the_policy_fail_closed() {
    // A syntactically valid but undecodable did:key id ⇒ resolve_rule_keys errors
    // (fail-closed): the policy is rejected, nothing is admitted.
    let policy =
        gov_age_policy_graph(GOV_ISSUER, (vocab::ISSUER_DID, "did:key:zNOTAKEY"), RESOURCE_X);
    let err = resolve_rule_keys(&policy, &DidKeyResolver, ControlGate::assert_control_gated());
    assert!(
        matches!(err, Err(sparq_trust::policy::PolicyError::BadIssuerDid { .. })),
        "an unresolvable issuer DID is a fail-closed BadIssuerDid, not a silent admit: {err:?}"
    );
}

#[test]
fn unsupported_did_method_rejects_the_policy() {
    // did:web through the did:key resolver is an unsupported method ⇒ fail-closed.
    let policy = gov_age_policy_graph(
        GOV_ISSUER,
        (vocab::ISSUER_DID, "did:web:gov.example"),
        RESOURCE_X,
    );
    let err = resolve_rule_keys(&policy, &DidKeyResolver, ControlGate::assert_control_gated());
    assert!(
        matches!(err, Err(sparq_trust::policy::PolicyError::BadIssuerDid { .. })),
        "an unsupported DID method is rejected fail-closed: {err:?}"
    );
}

// --- did:web through an in-memory fetcher (no network) ----------------------

struct MapFetcher(HashMap<String, String>);
impl DidDocumentFetcher for MapFetcher {
    fn fetch(&self, url: &str) -> Result<String, String> {
        self.0
            .get(url)
            .cloned()
            .ok_or_else(|| format!("no document at {}", url))
    }
}

#[test]
fn did_web_bound_issuer_grants_read_through_an_in_memory_document() {
    let (_sk, pk) = gov_key();
    // The DID document carries the SAME key as publicKeyMultibase (the did:key encoding).
    let mb = did_key_for(&pk).strip_prefix("did:key:").unwrap().to_string();
    let did = "did:web:gov.example";
    let url = DidWebResolver::<MapFetcher>::document_url("gov.example").unwrap();
    let doc = format!(
        r#"{{
          "id": "{did}",
          "verificationMethod": [
            {{ "id": "{did}#k1", "type": "Multikey", "publicKeyMultibase": "{mb}" }}
          ]
        }}"#
    );
    let mut map = HashMap::new();
    map.insert(url, doc);
    let resolver = DidWebResolver::new(MapFetcher(map));

    let policy = gov_age_policy_graph(GOV_ISSUER, (vocab::ISSUER_DID, did), RESOURCE_X);
    let rules = resolve_rule_keys(&policy, &resolver, ControlGate::assert_control_gated()).unwrap();
    assert_eq!(
        rules[0].issuer_key, pk,
        "the did:web document's verification method binds the gov issuer key"
    );
    let grants = run(&rules);
    assert!(
        read_granted(&grants, JESSE),
        "a did:web-bound issuer grants read end-to-end (in-memory document; no network)"
    );
}

#[test]
fn claim_level_trusts_source_for_composes_with_a_did_key_bound_issuer() {
    // The `sq-pfae.4` claim-level relational form (`<gov> trust:trustsSourceFor schema:age`)
    // names its issuer by `trust:issuerDid` on the SAME source node — so the foundational
    // primitive and the DID-resolved key binding (`sq-pfae.3`) compose: resolve_rule_keys
    // binds the key, and the unchanged gate grants read end-to-end.
    let (_sk, pk) = gov_key();
    let did = did_key_for(&pk);
    let s = || NamedOrBlankNode::NamedNode(iri(GOV_ISSUER));
    let policy = vec![
        Triple::new(
            s(),
            iri(vocab::RDF_TYPE),
            Term::NamedNode(iri(vocab::SOURCE_CLASS)),
        ),
        Triple::new(
            s(),
            iri(vocab::TRUSTS_SOURCE_FOR),
            Term::NamedNode(iri(SCHEMA_AGE)),
        ),
        Triple::new(
            s(),
            iri(vocab::ISSUER_DID),
            Term::Literal(Literal::new_simple_literal(&did)),
        ),
        Triple::new(s(), iri(vocab::SCOPE), Term::NamedNode(iri(RESOURCE_X))),
        Triple::new(
            s(),
            iri(vocab::FRESH_WITHIN),
            Term::Literal(Literal::new_typed_literal(
                "P30D",
                iri("http://www.w3.org/2001/XMLSchema#duration"),
            )),
        ),
    ];
    let rules = resolve_rule_keys(
        &policy,
        &DidKeyResolver,
        ControlGate::assert_control_gated(),
    )
    .unwrap();
    assert_eq!(rules.len(), 1, "one trustsSourceFor statement ⇒ one rule");
    assert_eq!(
        rules[0].issuer_key, pk,
        "the source's trust:issuerDid resolves to the gov verifying key"
    );
    let grants = run(&rules);
    assert!(
        read_granted(&grants, JESSE),
        "the claim-level trustsSourceFor form + a did:key-bound issuer grants read end-to-end"
    );
}

#[test]
fn did_web_with_no_usable_key_rejects_the_policy() {
    let did = "did:web:gov.example";
    let url = DidWebResolver::<MapFetcher>::document_url("gov.example").unwrap();
    let mut map = HashMap::new();
    map.insert(url, format!(r#"{{"id":"{did}"}}"#)); // keyless document
    let resolver = DidWebResolver::new(MapFetcher(map));
    let policy = gov_age_policy_graph(GOV_ISSUER, (vocab::ISSUER_DID, did), RESOURCE_X);
    let err = resolve_rule_keys(&policy, &resolver, ControlGate::assert_control_gated());
    assert!(
        matches!(err, Err(sparq_trust::policy::PolicyError::BadIssuerDid { .. })),
        "a did:web document with no usable issuer key rejects the policy fail-closed: {err:?}"
    );
}
