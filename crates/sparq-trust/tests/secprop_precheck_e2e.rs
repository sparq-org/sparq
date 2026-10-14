//! GOLDEN end-to-end test for the Phase-5 property-admissibility PRE-CHECK
//! (`sq-dt5hv`, design §5b) — `sparq_trust::admit_with_precheck` over the REAL
//! admission path (RDFC-1.0 commit + CHECKED issuer Schnorr signature + SHACL
//! statement-type scoping + the clear-WebID holder binding) plus the Phase-2 secprop
//! admissibility reduction in front of it.
//!
//! The load-bearing invariants (design §5b / §4.3.3):
//!
//! - **OPT-IN strict additivity** — with `preference == None` the pre-check gate is
//!   byte-identical to the plain `admit` gate (same admitted facts), so an existing
//!   caller sees no behaviour change.
//! - **Satisfied preference admits + derives** — a method that satisfies the
//!   requester's relaxed `gteq` preference passes the pre-check, the real gate runs,
//!   and the age>18 grant still derives end-to-end.
//! - **Unsatisfied preference FAILS CLOSED** — Alice's strict `requiresAssurance gteq
//!   Proven` removes every (Claimed-only) sparq method while `sq-qhy4` is open, so the
//!   gate admits NOTHING and NO grant derives, even though the credential signature,
//!   freshness, and holder binding are all perfectly valid. The pre-check is a
//!   *principled refusal*, not a cryptographic failure.
//! - **BUNDLED, TAMPER-RESISTANT annotations** (`sq-nrwqs`, Phase 5.1) — the method's
//!   secprop annotation graph is resolved from the bundled sparq-zk
//!   `secprop-methods.ttl`, NOT from caller-supplied N3. The caller passes only the
//!   method IRI + the ODRL preference; a caller cannot widen the recorded posture and an
//!   unknown method fails closed. This hardens the input trust boundary; it makes NO new
//!   soundness claim (the estate stays externally UNAUDITED, `sq-qhy4`).
//!
//! Runs only with the `secprop-precheck` feature; the `feature-matrix.yml` /
//! `feature-matrix.d/sparq-trust.yml` leg EXECUTES it.
//!
//! 🤖 SPARQ agent — sq-dt5hv (epic sq-0dksu, Phase 5) + sq-nrwqs (Phase 5.1).
//! Flag for re-review when Fable returns.
#![cfg(feature = "secprop-precheck")]

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_trust::admit::{
    admit, admit_with_precheck, AdmissibilityConstraint, AdmissibilityPreference, PrecheckOutcome,
};
use sparq_trust::policy::{parse_policy, ControlGate, TrustRule};
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

/// The presented proof method IRI (the `zk:scheme` the registry records).
const METHOD: &str = "https://sparq.dev/ns/zk#poseidon2-rdfc10-v1";

const ACR_ABAC_RULE: &str = r#"@prefix schema: <http://schema.org/> .
@prefix math:   <http://www.w3.org/2000/10/swap/math#> .
@prefix auth:   <https://sparq.dev/ns/auth#> .
{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <https://pod.ex/resourceX> } .
"#;

// sq-nrwqs (Phase 5.1): the method's secprop annotation graph is NO LONGER
// caller-supplied — the pre-check resolves it from the bundled sparq-zk
// `secprop-methods.ttl`, and the requester's ODRL preference is STRUCTURED (validated
// `AdmissibilityConstraint` IRIs), not raw N3. The string-canonical method's bundled block
// records `UnlinkabilityScope PerPresentation` and every positive property `Claimed` (so
// its derived AssuranceLevel is `Claimed`; `Proven` is barred while `sq-qhy4` is open),
// which is exactly the §4.3.3 worked-example posture these tests pin — now tamper-resistant.

// The `secx:requires…` leftOperands + level IRIs the requester's `gteq` constraints name.
const REQ_UNLINK: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresUnlinkabilityScope";
const REQ_ASSUR: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresAssurance";
const REQ_SOUND: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresSoundness";
const LVL_PER_PRESENTATION: &str = "https://w3id.org/zkp-sparql/sec-prop#PerPresentation";
const LVL_CLAIMED: &str = "https://w3id.org/zkp-sparql/sec-prop#Claimed";
const LVL_PROVEN: &str = "https://w3id.org/zkp-sparql/sec-prop#Proven";
const LVL_KNOWLEDGE_SOUND: &str = "https://w3id.org/zkp-sparql/sec-prop#KnowledgeSound";

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

/// A single `gteq` admissibility constraint from two IRI strings.
fn constraint(left: &str, right: &str) -> AdmissibilityConstraint {
    AdmissibilityConstraint {
        left_operand: iri(left),
        right_operand: iri(right),
    }
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

fn session(agent: &str, now: i64) -> sparq_trust::admit::Session {
    sparq_trust::admit::Session {
        agent: iri(agent),
        now_unix_secs: now,
    }
}

fn fresh_cred(sk: &SecretKey, graph: Vec<Triple>) -> sparq_trust::admit::PresentedCredential {
    let sig = sign_graph(sk, &graph);
    sparq_trust::admit::PresentedCredential {
        graph,
        issuer_signature_hex: sig,
        salt: SALT_BYTES,
        issued_at_unix_secs: ISSUED_FRESH,
        revoked: false,
    }
}

fn read_granted(grants: &[Triple], subject: &str) -> bool {
    grants.iter().any(|t| {
        t.predicate.as_str() == "https://sparq.dev/ns/auth#read"
            && matches!(&t.subject, NamedOrBlankNode::NamedNode(n) if n.as_str() == subject)
            && matches!(&t.object, Term::NamedNode(o) if o.as_str() == RESOURCE_X)
    })
}

fn pref(constraints: Vec<AdmissibilityConstraint>) -> AdmissibilityPreference {
    AdmissibilityPreference {
        method_iri: iri(METHOD),
        constraints,
    }
}

/// OPT-IN strict additivity: `preference == None` is byte-identical to plain `admit`.
#[test]
fn no_preference_is_byte_identical_to_plain_admit() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));

    let plain = admit(&cred, &rules, &session(JESSE, NOW), &iri(RESOURCE_X));
    let (with_pre, outcome) =
        admit_with_precheck(&cred, &rules, &session(JESSE, NOW), &iri(RESOURCE_X), None);

    assert_eq!(
        outcome,
        PrecheckOutcome::Admitted,
        "no preference => Admitted"
    );
    assert_eq!(
        with_pre, plain,
        "with NO preference, admit_with_precheck must admit EXACTLY the same facts as admit"
    );
    assert_eq!(with_pre.len(), 1, "the age fact is admitted");
}

/// A satisfiable preference passes the pre-check, the real gate runs, and the age>18
/// grant still derives end-to-end.
#[test]
fn satisfiable_preference_admits_and_derives_the_grant() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));

    // Relaxed: unlinkability >= PerPresentation (held), assurance >= Claimed (held).
    let p = pref(vec![
        constraint(REQ_UNLINK, LVL_PER_PRESENTATION),
        constraint(REQ_ASSUR, LVL_CLAIMED),
    ]);

    let (admitted, outcome) = admit_with_precheck(
        &cred,
        &rules,
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        Some(&p),
    );
    assert_eq!(
        outcome,
        PrecheckOutcome::Admitted,
        "the relaxed preference is satisfied"
    );
    assert_eq!(
        admitted.len(),
        1,
        "the age fact passes the gate after the pre-check"
    );

    let grants = derive_grants(&admitted, ACR_ABAC_RULE).expect("derivation runs");
    assert!(
        read_granted(&grants, JESSE),
        "with a satisfied pre-check the age>18 grant derives end-to-end"
    );
}

/// BUNDLED-RESOLUTION end-to-end: a `requiresSoundness gteq secx:KnowledgeSound`
/// preference is satisfied purely from the shipped `secprop-methods.ttl` (which records
/// the string-canonical method's `Soundness KnowledgeSound`) — a dimension NO caller
/// supplies. The real gate then runs and the age>18 grant derives end-to-end. This pins
/// that admissibility is decided by the bundled ontology, not caller-supplied N3
/// (`sq-nrwqs`).
#[test]
fn bundled_soundness_preference_admits_and_derives_the_grant() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));

    let p = pref(vec![constraint(REQ_SOUND, LVL_KNOWLEDGE_SOUND)]);

    let (admitted, outcome) = admit_with_precheck(
        &cred,
        &rules,
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        Some(&p),
    );
    assert_eq!(
        outcome,
        PrecheckOutcome::Admitted,
        "the bundled Soundness KnowledgeSound satisfies the requester's gteq constraint"
    );
    assert_eq!(admitted.len(), 1, "the age fact passes the gate");
    let grants = derive_grants(&admitted, ACR_ABAC_RULE).expect("derivation runs");
    assert!(
        read_granted(&grants, JESSE),
        "the grant derives end-to-end from a bundled-resolved admissibility verdict"
    );
}

/// Alice's STRICT `requiresAssurance gteq Proven` removes every Claimed-only sparq
/// method while `sq-qhy4` is open: the gate admits NOTHING and NO grant derives, even
/// though the signature, freshness, and holder binding are all valid. A principled
/// refusal, not a cryptographic failure.
#[test]
fn strict_assurance_preference_fails_closed_despite_a_valid_credential() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));

    let p = pref(vec![constraint(REQ_ASSUR, LVL_PROVEN)]);

    // Sanity: the SAME credential admits without the pre-check (so the denial below is
    // the pre-check's doing, not a broken credential).
    let baseline = admit(&cred, &rules, &session(JESSE, NOW), &iri(RESOURCE_X));
    assert_eq!(
        baseline.len(),
        1,
        "the credential is otherwise perfectly admittable"
    );

    let (admitted, outcome) = admit_with_precheck(
        &cred,
        &rules,
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        Some(&p),
    );
    assert_eq!(
        outcome,
        PrecheckOutcome::Denied {
            unsatisfied: vec![REQ_ASSUR.to_owned()]
        },
        "requiresAssurance gteq Proven names the unsatisfied dimension (the honest refusal)"
    );
    assert!(
        admitted.is_empty(),
        "fail-closed: a method that fails the property pre-check admits NOTHING"
    );

    let grants = derive_grants(&admitted, ACR_ABAC_RULE).expect("derivation runs");
    assert!(
        !read_granted(&grants, JESSE),
        "with the pre-check denied, NO grant derives — the principled 'no admissible proof' refusal"
    );
}

/// A preference whose method IRI has no BUNDLED annotation block is an unknown method:
/// it fails closed with `UnknownMethod` (the bundled graph is the source of truth; a
/// wrong/unknown method admits nothing) — the `sq-nrwqs` hardening.
#[test]
fn unknown_method_iri_fails_closed() {
    let (sk, pk) = gov_key();
    let rules = gov_age_policy(GOV_ISSUER, &pk, RESOURCE_X);
    let cred = fresh_cred(&sk, age_credential_graph(JESSE, "25"));

    let p = AdmissibilityPreference {
        method_iri: iri("https://sparq.dev/ns/zk#no-such-method"),
        constraints: vec![constraint(REQ_ASSUR, LVL_CLAIMED)],
    };

    let (admitted, outcome) = admit_with_precheck(
        &cred,
        &rules,
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        Some(&p),
    );
    assert!(
        matches!(outcome, PrecheckOutcome::UnknownMethod { .. }),
        "an unknown method (no bundled block) denies with UnknownMethod"
    );
    assert!(
        admitted.is_empty(),
        "a method with no bundled annotation block admits nothing"
    );
}
