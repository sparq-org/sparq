//! End-to-end PoC: the age>18 worked example (design §3.1 / §6.0), plus the full
//! adversarial negative-case matrix.
//!
//! It exercises the REAL admission path — RDFC-1.0 canonicalisation (sparq-canon),
//! a CHECKED issuer Schnorr signature over the RDFC-1.0 commitment (sparq-zk),
//! statement-type scoping via a real SHACL shape (sparq-shacl), freshness, and the
//! clear-WebID holder binding — then the N3 merge of the admitted fact with the
//! controller-authored `.acr` ABAC rule (sparq-reason) to derive the `auth:read`
//! grant.
//!
//! Positive: a trusted-government-issued VC `<Jesse> age 25` + a trust policy
//! trusting that issuer for age statements + the `.acr` rule ⇒ `<Jesse>` gets the
//! read grant. Negatives (each must DENY): untrusted issuer, tampered/absent
//! signature, stale credential, wrong holder, out-of-scope laundered predicate, and
//! age 16 (admitted but the rule denies).
//!
//! These are the §7.3-E forgery tests that did not exist — the ones that convert
//! statement-type-scoping / no-laundering / key-binding from *design intent* into
//! *verified property* (mirroring `acp_forged_*_in_acr_document_does_not_grant`).
//!
//! sq-pfae PoC. 🤖 SPARQ agent — trust-graph authorisation PoC.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_trust::admit::{admit, admit_static, PresentedCredential, Session};
use sparq_trust::policy::{parse_policy, ControlGate, ShapeRef, TrustRule};
use sparq_trust::vocab::{self, desugar_for_predicate};
use sparq_trust::wire::{derive_conditional_grants, derive_grants};
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::sig::{public_key_to_hex, PublicKey, SecretKey};

// --- fixtures ---------------------------------------------------------------

const JESSE: &str = "https://jesse.ex/card#me";
const MALLORY: &str = "https://mallory.ex/card#me";
const GOV_ISSUER: &str = "https://gov.example/issuer";
const UNTRUSTED_ISSUER: &str = "https://forge.example/issuer";
const SCHEMA_AGE: &str = "http://schema.org/age";
const RESOURCE_X: &str = "https://pod.ex/resourceX";
const ACL_AGENT: &str = "http://www.w3.org/ns/auth/acl#agent";
const SALT_BYTES: [u8; 32] = [7u8; 32];

/// The controller-authored ABAC rule carried in the `.acr` Control-gated channel.
const ACR_ABAC_RULE: &str = r#"@prefix schema: <http://schema.org/> .
@prefix math:   <http://www.w3.org/2000/10/swap/math#> .
@prefix auth:   <https://sparq.dev/ns/auth#> .
{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <https://pod.ex/resourceX> } .
"#;

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

/// A credential graph `<subject> schema:age <value>` (the gov VC claim).
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

/// The gov issuer key (deterministic for the test; production keys use OS entropy).
fn gov_key() -> (SecretKey, PublicKey) {
    let sk = SecretKey::from_seed(0xC0FFEE);
    let pk = sk.public_key();
    (sk, pk)
}

/// Sign a credential graph: RDFC-1.0 commit under SALT, sign the commitment, return
/// the hex signature exactly as the gate verifies it.
fn sign_graph(sk: &SecretKey, graph: &[Triple]) -> String {
    let salt = salt_from_bytes(&SALT_BYTES);
    let commitment = commit_triples(graph, salt).expect("credential graph commits");
    sk.sign_commitment(&commitment.commitment)
}

/// Build the gov-trusts-age trust policy graph (Control-gated), then parse it into
/// rules. Uses the `forPredicate schema:age` SUGAR (desugared to a single-predicate
/// `forShape` at parse). The issuer key is supplied DIRECTLY (operator-asserted; no
/// DID resolver — sq-pfae.3).
fn gov_age_policy(issuer: &str, key: &PublicKey, scope: &str) -> Vec<TrustRule> {
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
            Term::NamedNode(iri(issuer)),
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
    ];
    parse_policy(&policy, ControlGate::assert_control_gated()).expect("policy parses")
}

/// A session for `agent` at instant `now` (Unix seconds).
fn session(agent: &str, now: i64) -> Session {
    Session {
        agent: iri(agent),
        now_unix_secs: now,
    }
}

/// The full pipeline: admit + derive. Returns the derived `auth:*` grant triples.
fn run_pipeline(
    cred: &PresentedCredential,
    rules: &[TrustRule],
    sess: &Session,
    target: &str,
) -> Vec<Triple> {
    let admitted = admit(cred, rules, sess, &iri(target));
    derive_grants(&admitted, ACR_ABAC_RULE).expect("derivation runs")
}

fn read_granted(grants: &[Triple], subject: &str) -> bool {
    grants.iter().any(|t| {
        t.predicate.as_str() == "https://sparq.dev/ns/auth#read"
            && matches!(&t.subject, NamedOrBlankNode::NamedNode(n) if n.as_str() == subject)
            && matches!(&t.object, Term::NamedNode(o) if o.as_str() == RESOURCE_X)
    })
}

const NOW: i64 = 1_700_000_000; // a fixed "now"
const ISSUED_FRESH: i64 = NOW - 86_400; // 1 day ago — within P30D

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

// --- the positive case ------------------------------------------------------

#[test]
fn age_25_trusted_gov_issuer_grants_read_end_to_end() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));
    let grants = run_pipeline(&cred, &rules, &session(JESSE, NOW), RESOURCE_X);
    assert!(
        read_granted(&grants, JESSE),
        "age 25 from the trusted gov issuer, fresh, holder-bound, in scope ⇒ \
         <Jesse> auth:read <resourceX> is derived end-to-end"
    );
}

// --- the negative matrix (each MUST deny) -----------------------------------

#[test]
fn untrusted_issuer_is_not_admitted_no_access() {
    // The credential is validly signed by an UNTRUSTED key; the trust policy trusts
    // a DIFFERENT issuer. No matching trustsSourceFor ⇒ not admitted ⇒ no access.
    let (_gov_sk, gov_pk) = gov_key();
    let forge_sk = SecretKey::from_seed(0xBADBAD);
    // Policy trusts the GOV key, but the credential is signed by the FORGE key.
    let rules = gov_age_policy(UNTRUSTED_ISSUER, &gov_pk, RESOURCE_X);
    let cred = fresh_cred(&forge_sk, age_credential_graph(JESSE, "25"));
    let grants = run_pipeline(&cred, &rules, &session(JESSE, NOW), RESOURCE_X);
    assert!(
        !read_granted(&grants, JESSE),
        "credential signed by an untrusted key whose signature does not verify under \
         the trusted issuerKey is NOT admitted ⇒ no access"
    );
}

#[test]
fn tampered_signature_is_rejected() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let graph = age_credential_graph(JESSE, "25");
    let mut cred = fresh_cred(&sk, graph);
    // Flip the last hex nibble — a tampered signature must fail the CHECKED verify.
    let mut sig: Vec<char> = cred.issuer_signature_hex.chars().collect();
    let last = sig.len() - 1;
    sig[last] = if sig[last] == '0' { '1' } else { '0' };
    cred.issuer_signature_hex = sig.into_iter().collect();
    let grants = run_pipeline(&cred, &rules, &session(JESSE, NOW), RESOURCE_X);
    assert!(
        !read_granted(&grants, JESSE),
        "a tampered issuer signature is rejected (fail-closed) ⇒ no access"
    );
}

#[test]
fn absent_signature_is_rejected() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let graph = age_credential_graph(JESSE, "25");
    let mut cred = fresh_cred(&sk, graph);
    cred.issuer_signature_hex = String::new(); // no signature at all
    let grants = run_pipeline(&cred, &rules, &session(JESSE, NOW), RESOURCE_X);
    assert!(
        !read_granted(&grants, JESSE),
        "an absent issuer signature is rejected (a self-asserted 'I am signed' triple \
         proves nothing — §3.3 item 1) ⇒ no access"
    );
}

#[test]
fn stale_credential_is_rejected() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let graph = age_credential_graph(JESSE, "25");
    let sig = sign_graph(&sk, &graph);
    let cred = PresentedCredential {
        graph,
        issuer_signature_hex: sig,
        salt: SALT_BYTES,
        // Issued 60 days ago — past the rule's P30D freshWithin window.
        issued_at_unix_secs: NOW - 60 * 86_400,
        revoked: false,
    };
    let grants = run_pipeline(&cred, &rules, &session(JESSE, NOW), RESOURCE_X);
    assert!(
        !read_granted(&grants, JESSE),
        "a credential past trust:freshWithin (a per-request Rust check, not in-reasoner) \
         is rejected ⇒ no access"
    );
}

#[test]
fn wrong_holder_third_party_credential_is_rejected() {
    // Mallory presents JESSE's validly-signed credential. credentialSubject (Jesse)
    // != Session.agent (Mallory) ⇒ holder binding fails ⇒ not admitted.
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));
    let grants = run_pipeline(&cred, &rules, &session(MALLORY, NOW), RESOURCE_X);
    assert!(
        !read_granted(&grants, MALLORY),
        "presenting a third party's credential without holder binding \
         (credentialSubject != Session.agent, §3.4) must NOT admit ⇒ no access"
    );
    // And it does not silently grant JESSE either (the requester is Mallory).
    assert!(!read_granted(&grants, JESSE));
}

#[test]
fn out_of_scope_laundered_predicate_is_not_admitted() {
    // The gov issuer is trusted for schema:age. It signs a graph that ALSO carries a
    // smuggled `<Jesse> acl:agent <Jesse>` triple (an attempt to launder an
    // access-control predicate in). The shape targets only schema:age subjects AND the
    // reserved-predicate guard blocks acl: predicates, so the smuggled triple is never
    // admitted — and crucially does not poison the legitimate age admission.
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let mut graph = age_credential_graph(JESSE, "25");
    graph.push(Triple::new(
        NamedOrBlankNode::NamedNode(iri(JESSE)),
        iri(ACL_AGENT),
        Term::NamedNode(iri(JESSE)),
    ));
    let cred = fresh_cred(&sk, graph);
    let admitted = admit(&cred, &rules, &session(JESSE, NOW), &iri(RESOURCE_X));
    // The acl:agent triple is NOT among the admitted facts (statement-type scoping +
    // reserved-predicate guard).
    assert!(
        admitted
            .iter()
            .all(|f| f.triple.predicate.as_str() == SCHEMA_AGE),
        "a source trusted for schema:age cannot launder an acl:agent triple in \
         (§3.3 item 2): only the age fact is admitted"
    );
    // The legitimate age fact still flows through to the grant.
    let grants = derive_grants(&admitted, ACR_ABAC_RULE).unwrap();
    assert!(
        read_granted(&grants, JESSE),
        "the smuggled predicate does not block the legitimate age admission"
    );
}

#[test]
fn age_16_admitted_but_rule_denies() {
    // age 16 IS a trusted age statement (admitted), but the > 18 rule does not fire ⇒
    // no grant. This proves the N3 MERGE is the decision point, not admission.
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "16"));
    let admitted = admit(&cred, &rules, &session(JESSE, NOW), &iri(RESOURCE_X));
    assert_eq!(
        admitted.len(),
        1,
        "age 16 is admitted (it is a trusted age statement)"
    );
    let grants = derive_grants(&admitted, ACR_ABAC_RULE).unwrap();
    assert!(
        !read_granted(&grants, JESSE),
        "age 16 is admitted but the >18 rule denies ⇒ no access (the merge decides)"
    );
}

#[test]
fn revoked_credential_is_rejected() {
    // A previously-admittable credential marked revoked (the input-only guard, sq-tu4e)
    // is rejected ⇒ no access.
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let graph = age_credential_graph(JESSE, "25");
    let sig = sign_graph(&sk, &graph);
    let cred = PresentedCredential {
        graph,
        issuer_signature_hex: sig,
        salt: SALT_BYTES,
        issued_at_unix_secs: ISSUED_FRESH,
        revoked: true,
    };
    let grants = run_pipeline(&cred, &rules, &session(JESSE, NOW), RESOURCE_X);
    assert!(
        !read_granted(&grants, JESSE),
        "a revoked credential (input-only guard) is rejected ⇒ no access"
    );
}

#[test]
fn out_of_scope_resource_is_not_covered() {
    // The trust rule is scoped to resourceX; a request for resourceY is not covered.
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));
    let resource_y = "https://pod.ex/resourceY";
    let admitted = admit(&cred, &rules, &session(JESSE, NOW), &iri(resource_y));
    assert!(
        admitted.is_empty(),
        "a rule scoped to resourceX does not cover a request for resourceY (§3.2 scope)"
    );
}

// --- the static/dynamic admission split (sq-xc4y) ---------------------------

#[test]
fn static_admission_is_session_independent_and_carries_dynamic_conditions() {
    // admit_static() takes NO session: it decides only the STATIC class (signature,
    // type-scope, reserved-predicate guard, scope) and carries the DYNAMIC conditions
    // (the bound holder + the freshness deadline) for a per-request check.
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));
    let statics = admit_static(&cred, &rules, &iri(RESOURCE_X));
    assert_eq!(statics.len(), 1, "the age fact passes static admission");
    let f = &statics[0];
    assert_eq!(
        f.holder.as_str(),
        JESSE,
        "the bound holder is the credential subject"
    );
    assert_eq!(
        f.not_after_unix_secs,
        ISSUED_FRESH + 30 * 86_400,
        "the freshness deadline is issued_at + freshWithin (P30D), carried for the \
         per-request check — NOT evaluated at static-admission time"
    );
}

#[test]
fn static_admission_admits_a_stale_credential_but_defers_the_freshness_check() {
    // A credential past freshWithin is STILL statically admitted (freshness is a
    // DYNAMIC, per-request condition, not a static one) — but the carried deadline is
    // in the past, so the per-request conditional check would deny it. The point of the
    // split: the static decision is session-independent; the freshness verdict is
    // re-evaluated per request, never frozen.
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let graph = age_credential_graph(JESSE, "25");
    let sig = sign_graph(&sk, &graph);
    let issued = NOW - 60 * 86_400; // 60 days ago — past P30D
    let cred = PresentedCredential {
        graph,
        issuer_signature_hex: sig,
        salt: SALT_BYTES,
        issued_at_unix_secs: issued,
        revoked: false,
    };
    let statics = admit_static(&cred, &rules, &iri(RESOURCE_X));
    assert_eq!(
        statics.len(),
        1,
        "a stale credential STILL passes static admission (signature + type-scope hold)"
    );
    let deadline = statics[0].not_after_unix_secs;
    assert!(
        deadline < NOW,
        "but the carried freshness deadline ({deadline}) is in the past, so the \
         per-request check denies it at query time — NOT frozen into the view"
    );
}

#[test]
fn static_admission_then_conditional_grant_carries_holder_and_deadline() {
    // The materialise-time half end-to-end: static admission → derive_conditional_grants
    // → a conditional grant tagged with the holder + deadline for the per-request check.
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));
    let statics = admit_static(&cred, &rules, &iri(RESOURCE_X));
    let cgs = derive_conditional_grants(&statics, ACR_ABAC_RULE).expect("derivation runs");
    assert_eq!(
        cgs.len(),
        1,
        "age 25 > 18 derives one conditional read grant"
    );
    let cg = &cgs[0];
    assert_eq!(
        cg.grant.predicate.as_str(),
        "https://sparq.dev/ns/auth#read"
    );
    assert_eq!(cg.holder.as_str(), JESSE);
    assert_eq!(cg.not_after_unix_secs, ISSUED_FRESH + 30 * 86_400);
}

#[test]
fn static_admission_drops_a_blank_node_subject() {
    // A credential triple whose subject is a blank node has no stable WebID to bind the
    // holder to, so it cannot be holder-bound for a credential-gated grant: fail closed.
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let graph = vec![Triple::new(
        NamedOrBlankNode::BlankNode(oxrdf::BlankNode::new("b0").unwrap()),
        iri(SCHEMA_AGE),
        Term::Literal(Literal::new_typed_literal(
            "25",
            iri("http://www.w3.org/2001/XMLSchema#integer"),
        )),
    )];
    let sig = sign_graph(&sk, &graph);
    let cred = PresentedCredential {
        graph,
        issuer_signature_hex: sig,
        salt: SALT_BYTES,
        issued_at_unix_secs: ISSUED_FRESH,
        revoked: false,
    };
    let statics = admit_static(&cred, &rules, &iri(RESOURCE_X));
    assert!(
        statics.is_empty(),
        "a blank-node-subject triple cannot be holder-bound ⇒ not statically admitted"
    );
}

#[test]
fn for_shape_primitive_equivalent_to_for_predicate_sugar() {
    // CLAIM 1 (design §7.6): the forPredicate→forShape desugaring is lossless in the
    // predicate-only direction. A rule built with the EXPLICIT desugared forShape
    // admits the SAME fact as the forPredicate sugar. We assert the desugaring is what
    // the sugar produces by admitting through a hand-built forShape rule.
    let (sk, pk) = gov_key();
    let (shape_root, shape_triples) = desugar_for_predicate(&iri(SCHEMA_AGE));
    let rule = TrustRule {
        source: iri(GOV_ISSUER),
        issuer_key: pk,
        shape: ShapeRef {
            root: match shape_root {
                NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n),
                NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b),
            },
            triples: shape_triples,
        },
        scope: iri(RESOURCE_X),
        fresh_within_secs: 30 * 86_400,
    };
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));
    let admitted = admit(&cred, &[rule], &session(JESSE, NOW), &iri(RESOURCE_X));
    assert_eq!(
        admitted.len(),
        1,
        "the explicit forShape admits the same age fact as the forPredicate sugar"
    );
}
