//! End-to-end: the claim-level `trust:trustsSourceFor` relational policy form drives the
//! SAME admission gate as the reified `trust:TrustRule` form — the `sq-pfae.4` core
//! deliverable that closes the `acp:vc` gap (per-(source, statement-type) trust replacing
//! ACP's type-only, unimplemented `acp:vc` matcher).
//!
//! It exercises the REAL admission path through a policy authored with the foundational
//! relation (`<gov> trust:trustsSourceFor schema:age`): RDFC-1.0 commit → CHECKED issuer
//! signature → SHACL statement-type scope → freshness → clear-WebID holder binding → N3
//! merge with the `.acr` ABAC rule. The load-bearing invariant: **result-equivalence** —
//! the `trustsSourceFor` policy admits and grants EXACTLY what the reified policy does.
//!
//! Adversarial forgery negatives (each MUST deny, mirroring sparq-solid's
//! `acp_forged_*_in_acr_document_does_not_grant`): an UNTRUSTED issuer key, a tampered
//! signature, a third-party (wrong-holder) credential, and an out-of-type laundered
//! `acl:agent` triple under a source trusted only for `schema:age`. The claim-level form
//! is NOT a weaker admission path: every soundness side-condition holds identically.
//!
//! sq-pfae.4 (issue #940 / gh-940). 🤖 SPARQ agent — trust-graph authz PoC.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_trust::admit::{admit, PresentedCredential, Session};
use sparq_trust::policy::{parse_policy, ControlGate, TrustRule};
use sparq_trust::vocab;
use sparq_trust::wire::derive_grants;
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::sig::{public_key_to_hex, PublicKey, SecretKey};

const JESSE: &str = "https://jesse.ex/card#me";
const MALLORY: &str = "https://mallory.ex/card#me";
const GOV_ISSUER: &str = "https://gov.example/issuer";
const SCHEMA_AGE: &str = "http://schema.org/age";
const ACL_AGENT: &str = "http://www.w3.org/ns/auth/acl#agent";
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

/// The REIFIED form: a `trust:TrustRule` node grouping source/key/forPredicate/scope/fresh.
fn reified_policy(issuer: &str, key: &PublicKey) -> Vec<Triple> {
    let rule = iri("https://pod.ex/.acr#trustrule");
    let s = || NamedOrBlankNode::NamedNode(rule.clone());
    vec![
        Triple::new(
            s(),
            iri(vocab::RDF_TYPE),
            Term::NamedNode(iri(vocab::TRUST_RULE)),
        ),
        Triple::new(s(), iri(vocab::SOURCE), Term::NamedNode(iri(issuer))),
        Triple::new(
            s(),
            iri(vocab::ISSUER_KEY),
            Term::Literal(Literal::new_simple_literal(public_key_to_hex(key))),
        ),
        Triple::new(
            s(),
            iri(vocab::FOR_PREDICATE),
            Term::NamedNode(iri(SCHEMA_AGE)),
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
    ]
}

/// The CLAIM-LEVEL form: the source node carries `trust:trustsSourceFor schema:age`
/// directly, alongside its key + scope + freshWithin. The source IRI is `issuer`.
fn claim_level_policy(issuer: &str, key: &PublicKey) -> Vec<Triple> {
    let s = || NamedOrBlankNode::NamedNode(iri(issuer));
    vec![
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
            iri(vocab::ISSUER_KEY),
            Term::Literal(Literal::new_simple_literal(public_key_to_hex(key))),
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
    ]
}

fn parse(policy: &[Triple]) -> Vec<TrustRule> {
    parse_policy(policy, ControlGate::assert_control_gated()).expect("policy parses")
}

fn read_granted(grants: &[Triple], subject: &str) -> bool {
    grants.iter().any(|t| {
        t.predicate.as_str() == "https://sparq.dev/ns/auth#read"
            && matches!(&t.subject, NamedOrBlankNode::NamedNode(n) if n.as_str() == subject)
            && matches!(&t.object, Term::NamedNode(o) if o.as_str() == RESOURCE_X)
    })
}

fn run(cred: &PresentedCredential, rules: &[TrustRule], sess: &Session) -> Vec<Triple> {
    let admitted = admit(cred, rules, sess, &iri(RESOURCE_X));
    derive_grants(&admitted, ACR_ABAC_RULE).expect("derivation runs")
}

// --- the load-bearing invariant: result-equivalence with the reified form ----

#[test]
fn claim_level_form_grants_read_identically_to_the_reified_form() {
    let (sk, pk) = gov_key();
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));

    let reified = parse(&reified_policy(GOV_ISSUER, &pk));
    let claim = parse(&claim_level_policy(GOV_ISSUER, &pk));

    // Both forms produce ONE rule with the SAME source/scope/freshness.
    assert_eq!(reified.len(), 1);
    assert_eq!(claim.len(), 1);
    assert_eq!(reified[0].source, claim[0].source);
    assert_eq!(reified[0].scope, claim[0].scope);
    assert_eq!(reified[0].fresh_within_secs, claim[0].fresh_within_secs);
    assert_eq!(reified[0].issuer_key, claim[0].issuer_key);

    // And the end-to-end grant is IDENTICAL (result-equivalence).
    assert!(
        read_granted(&run(&cred, &reified, &session(JESSE, NOW)), JESSE),
        "reified form grants read"
    );
    assert!(
        read_granted(&run(&cred, &claim, &session(JESSE, NOW)), JESSE),
        "claim-level trustsSourceFor form grants read — identical to the reified form"
    );
}

// --- adversarial forgery negatives (each MUST deny) --------------------------

#[test]
fn claim_level_untrusted_issuer_key_does_not_grant() {
    // The policy trusts the GOV key; the credential is signed by a DIFFERENT key whose
    // signature does not verify ⇒ no admission ⇒ no access.
    let (_gov_sk, gov_pk) = gov_key();
    let forge_sk = SecretKey::from_seed(0xBADBAD);
    let rules = parse(&claim_level_policy(GOV_ISSUER, &gov_pk));
    let cred = fresh_cred(&forge_sk, age_credential_graph(JESSE, "25"));
    assert!(
        !read_granted(&run(&cred, &rules, &session(JESSE, NOW)), JESSE),
        "a credential signed by an untrusted key fails the CHECKED signature ⇒ no access"
    );
}

#[test]
fn claim_level_tampered_signature_does_not_grant() {
    let (sk, pk) = gov_key();
    let rules = parse(&claim_level_policy(GOV_ISSUER, &pk));
    let mut cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));
    let mut sig: Vec<char> = cred.issuer_signature_hex.chars().collect();
    let last = sig.len() - 1;
    sig[last] = if sig[last] == '0' { '1' } else { '0' };
    cred.issuer_signature_hex = sig.into_iter().collect();
    assert!(
        !read_granted(&run(&cred, &rules, &session(JESSE, NOW)), JESSE),
        "a tampered issuer signature is rejected fail-closed ⇒ no access"
    );
}

#[test]
fn claim_level_wrong_holder_third_party_does_not_grant() {
    // Mallory presents Jesse's validly-signed credential: credentialSubject (Jesse) !=
    // Session.agent (Mallory) ⇒ holder binding fails ⇒ not admitted.
    let (sk, pk) = gov_key();
    let rules = parse(&claim_level_policy(GOV_ISSUER, &pk));
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));
    let grants = run(&cred, &rules, &session(MALLORY, NOW));
    assert!(
        !read_granted(&grants, MALLORY),
        "wrong-holder presentation denies"
    );
    assert!(
        !read_granted(&grants, JESSE),
        "and does not silently grant Jesse"
    );
}

#[test]
fn claim_level_out_of_type_laundered_predicate_is_not_admitted() {
    // The source is trusted (trustsSourceFor) ONLY for schema:age. It signs a graph that
    // also smuggles an `acl:agent` triple. Statement-type scoping + the reserved-predicate
    // guard reject the smuggled triple — and it does not poison the legit age admission.
    let (sk, pk) = gov_key();
    let rules = parse(&claim_level_policy(GOV_ISSUER, &pk));
    let mut graph = age_credential_graph(JESSE, "25");
    graph.push(Triple::new(
        NamedOrBlankNode::NamedNode(iri(JESSE)),
        iri(ACL_AGENT),
        Term::NamedNode(iri(JESSE)),
    ));
    let cred = fresh_cred(&sk, graph);
    let admitted = admit(&cred, &rules, &session(JESSE, NOW), &iri(RESOURCE_X));
    assert!(
        admitted
            .iter()
            .all(|f| f.triple.predicate.as_str() == SCHEMA_AGE),
        "a source trusted only for schema:age cannot launder an acl:agent triple in"
    );
    assert!(
        read_granted(&derive_grants(&admitted, ACR_ABAC_RULE).unwrap(), JESSE),
        "the smuggled predicate does not block the legitimate age admission"
    );
}

#[test]
fn claim_level_out_of_scope_resource_is_not_covered() {
    let (sk, pk) = gov_key();
    let rules = parse(&claim_level_policy(GOV_ISSUER, &pk));
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));
    let admitted = admit(
        &cred,
        &rules,
        &session(JESSE, NOW),
        &iri("https://pod.ex/resourceY"),
    );
    assert!(
        admitted.is_empty(),
        "a source scoped to resourceX does not cover a request for resourceY"
    );
}
