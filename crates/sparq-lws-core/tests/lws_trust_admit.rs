// AUTHORED-BY Claude Opus 5
//! The trust-graph admission seam (`authz::trust_admit`, sq-hed3q) end-to-end over REAL signed
//! credentials.
//!
//! Proves the three things the seam claims:
//!   1. a validly-signed credential + a Control-gated trust rule + the `.acr` ABAC rule yields
//!      EXACTLY the expected additive read grant, and unioning it onto a WAC decision only adds;
//!   2. a forged signature / a wrong holder / a stale credential / an out-of-scope resource each
//!      admit NOTHING (fail-closed, `None` — never an error that grants);
//!   3. the ADVERSARIAL case: the seam cannot flip a WAC-denied resource to allow via the trust
//!      path — neither for a resource the credential does not cover, nor for an anonymous
//!      requester, nor when the controller's rule derives a denial, nor by REPLAYING an already
//!      issued grant onto another resource's or another requester's denial.
//!
//! Compiled only under `--features trust-graph`.
//!
//! [OPUS-5] sq-hed3q. 🤖 SPARQ agent — trust-graph LIBRARY admission seam.
#![cfg(feature = "trust-graph")]

use std::collections::BTreeSet;

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_lws_core::authz::trust_admit::{
    trust_admit_verdict, union_trust_grants, PresentedCredential, TrustRule, TrustSession,
};
use sparq_lws_core::authz::{AccessMode, Decision};
use sparq_trust::policy::{parse_policy, ControlGate};
use sparq_trust::vocab;
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::sig::{public_key_to_hex, PublicKey, SecretKey};

const JESSE: &str = "https://jesse.ex/card#me";
const MALLORY: &str = "https://mallory.ex/card#me";
const GOV_ISSUER: &str = "https://gov.example/issuer";
const SCHEMA_AGE: &str = "http://schema.org/age";
const RESOURCE_X: &str = "https://pod.ex/resourceX";
const RESOURCE_Y: &str = "https://pod.ex/resourceY";
const SALT: [u8; 32] = [9u8; 32];
const NOW: i64 = 1_700_000_000;

/// The controller-authored ABAC rule from the Control-gated `.acr` channel: an adult, as attested
/// by a trusted issuer, may READ `resourceX`.
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
    let sk = SecretKey::from_seed(0xABCDEF);
    (sk.clone(), sk.public_key())
}

/// A credential the government issuer really signed, over the RDFC-1.0 commitment of its claim
/// graph — issued `issued_days_ago` days before [`NOW`].
fn signed_cred(
    sk: &SecretKey,
    subject: &str,
    value: &str,
    issued_days_ago: i64,
) -> PresentedCredential {
    let graph = age_credential_graph(subject, value);
    let salt = salt_from_bytes(&SALT);
    let commitment = commit_triples(&graph, salt).expect("the claim graph must commit");
    PresentedCredential {
        graph,
        issuer_signature_hex: sk.sign_commitment(&commitment.commitment),
        salt: SALT,
        issued_at_unix_secs: NOW - issued_days_ago * 86_400,
        revoked: false,
    }
}

/// The Control-gated trust policy: `issuer` is trusted for `schema:age` statements scoped to
/// `scope`, freshness 30 days.
fn gov_age_rules(issuer: &str, key: &PublicKey, scope: &str) -> Vec<TrustRule> {
    let rule = iri("https://pod.ex/.acr#trustrule");
    let policy = vec![
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
            Term::Literal(Literal::new_simple_literal("P30D")),
        ),
    ];
    parse_policy(&policy, ControlGate::assert_control_gated()).expect("the policy must parse")
}

fn session(agent: &str, now: i64) -> TrustSession {
    TrustSession {
        agent: iri(agent),
        now_unix_secs: now,
    }
}

// ---------------------------------------------------------------------------------------------
// (1) The positive path: a valid credential yields EXACTLY the expected additive read grant.
// ---------------------------------------------------------------------------------------------

#[test]
fn valid_credential_yields_exactly_the_expected_additive_read_grant() {
    let (sk, pk) = gov_key();
    let verdict = trust_admit_verdict(
        &signed_cred(&sk, JESSE, "25", 1),
        &gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X),
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        ACR_ABAC_RULE,
    )
    .expect("a validly-signed in-scope credential must admit");

    // EXACTLY read — the `.acr` rule grants nothing else, and the seam must not widen it.
    assert_eq!(verdict.modes(), &BTreeSet::from([AccessMode::Read]));
    assert!(verdict.grants(AccessMode::Read));
    assert!(!verdict.grants(AccessMode::Write));
    assert!(!verdict.grants(AccessMode::Control));
    assert_eq!(verdict.holder().as_str(), JESSE);
    assert_eq!(verdict.target().as_str(), RESOURCE_X);
    assert_eq!(verdict.issuers(), &[iri(GOV_ISSUER)]);

    // ADDITIVE onto a WAC allow: the existing modes survive untouched, read is added.
    let wac = Decision::Allow(BTreeSet::from([AccessMode::Append]));
    assert_eq!(
        verdict.union_onto(wac, Some(&iri(JESSE)), &iri(RESOURCE_X)),
        Decision::Allow(BTreeSet::from([AccessMode::Append, AccessMode::Read]))
    );

    // ...and it lifts the authenticated-but-unauthorized denial to exactly the granted modes.
    assert_eq!(
        verdict.union_onto(Decision::Forbidden, Some(&iri(JESSE)), &iri(RESOURCE_X)),
        Decision::Allow(BTreeSet::from([AccessMode::Read]))
    );
}

#[test]
fn a_trust_grant_never_drops_a_wac_allow() {
    let (sk, pk) = gov_key();
    let verdict = trust_admit_verdict(
        &signed_cred(&sk, JESSE, "25", 1),
        &gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X),
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        ACR_ABAC_RULE,
    )
    .expect("a validly-signed in-scope credential must admit");

    // Every mode WAC already granted is still granted afterwards, for every starting mode set.
    let all = [
        AccessMode::Read,
        AccessMode::Write,
        AccessMode::Append,
        AccessMode::Control,
    ];
    for start in [
        BTreeSet::new(),
        BTreeSet::from([AccessMode::Write]),
        BTreeSet::from([AccessMode::Append, AccessMode::Control]),
        BTreeSet::from(all),
    ] {
        let Decision::Allow(after) = verdict.union_onto(
            Decision::Allow(start.clone()),
            Some(&iri(JESSE)),
            &iri(RESOURCE_X),
        ) else {
            panic!("an allow must stay an allow");
        };
        assert!(
            start.is_subset(&after),
            "the trust union dropped a WAC allow: {start:?} -> {after:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// (2) The fail-closed cases: bad signature / wrong holder / stale each admit NOTHING.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_forged_signature_admits_nothing() {
    let (sk, pk) = gov_key();
    let mut cred = signed_cred(&sk, JESSE, "25", 1);
    // Flip one hex digit of the issuer signature: the commitment no longer verifies.
    let mut sig: Vec<char> = cred.issuer_signature_hex.chars().collect();
    sig[0] = if sig[0] == 'a' { 'b' } else { 'a' };
    cred.issuer_signature_hex = sig.into_iter().collect();

    assert!(trust_admit_verdict(
        &cred,
        &gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X),
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        ACR_ABAC_RULE,
    )
    .is_none());
}

#[test]
fn a_credential_signed_by_an_untrusted_issuer_admits_nothing() {
    let other_sk = SecretKey::from_seed(0x1234);
    let (_, trusted_pk) = gov_key();
    // Really signed — just not by the key the Control-gated rule names.
    let cred = signed_cred(&other_sk, JESSE, "25", 1);

    assert!(trust_admit_verdict(
        &cred,
        &gov_age_rules(GOV_ISSUER, &trusted_pk, RESOURCE_X),
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        ACR_ABAC_RULE,
    )
    .is_none());
}

#[test]
fn a_wrong_holder_credential_admits_nothing() {
    let (sk, pk) = gov_key();
    // A perfectly valid credential about Jesse, presented by Mallory. The holder binding
    // (credential subject == authenticated WebID) must refuse it.
    assert!(trust_admit_verdict(
        &signed_cred(&sk, JESSE, "25", 1),
        &gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X),
        &session(MALLORY, NOW),
        &iri(RESOURCE_X),
        ACR_ABAC_RULE,
    )
    .is_none());
}

#[test]
fn a_stale_credential_admits_nothing() {
    let (sk, pk) = gov_key();
    // Issued 90 days ago against a 30-day `trust:freshWithin` window.
    assert!(trust_admit_verdict(
        &signed_cred(&sk, JESSE, "25", 90),
        &gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X),
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        ACR_ABAC_RULE,
    )
    .is_none());
}

#[test]
fn a_revoked_credential_admits_nothing() {
    let (sk, pk) = gov_key();
    let mut cred = signed_cred(&sk, JESSE, "25", 1);
    cred.revoked = true;

    assert!(trust_admit_verdict(
        &cred,
        &gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X),
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        ACR_ABAC_RULE,
    )
    .is_none());
}

#[test]
fn a_credential_whose_attested_fact_fails_the_acr_rule_admits_nothing() {
    let (sk, pk) = gov_key();
    // Signed, fresh, correctly held — but 16 does not satisfy `math:greaterThan 18`.
    assert!(trust_admit_verdict(
        &signed_cred(&sk, JESSE, "16", 1),
        &gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X),
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        ACR_ABAC_RULE,
    )
    .is_none());
}

#[test]
fn a_malformed_acr_rule_admits_nothing_rather_than_erroring() {
    let (sk, pk) = gov_key();
    let verdict = trust_admit_verdict(
        &signed_cred(&sk, JESSE, "25", 1),
        &gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X),
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        "this is not N3 {{{",
    );
    assert!(verdict.is_none());
}

// ---------------------------------------------------------------------------------------------
// (3) ADVERSARIAL: the seam never flips a WAC-denied resource to allow via the trust path.
// ---------------------------------------------------------------------------------------------

#[test]
fn adversarial_a_wac_denied_resource_is_not_flipped_by_a_credential_that_does_not_cover_it() {
    let (sk, pk) = gov_key();
    let cred = signed_cred(&sk, JESSE, "25", 1);
    let rules = gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X);

    // resourceY is WAC-denied. Jesse holds a perfectly valid credential — but the `.acr` rule
    // grants read on resourceX, and the trust rule's `trust:scope` is resourceX too.
    for wac in [Decision::Forbidden, Decision::Unauthenticated] {
        let verdict = trust_admit_verdict(
            &cred,
            &rules,
            &session(JESSE, NOW),
            &iri(RESOURCE_Y),
            ACR_ABAC_RULE,
        );
        assert!(
            verdict.is_none(),
            "an out-of-scope resource must admit nothing"
        );
        assert_eq!(
            union_trust_grants(
                wac.clone(),
                verdict.as_ref(),
                Some(&iri(JESSE)),
                &iri(RESOURCE_Y)
            ),
            wac,
            "the WAC denial on resourceY must stand"
        );
    }
}

#[test]
fn adversarial_every_failed_admission_leaves_a_wac_denial_intact() {
    let (sk, pk) = gov_key();
    let rules = gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X);
    let mut forged = signed_cred(&sk, JESSE, "25", 1);
    forged.issuer_signature_hex = "00".repeat(64);

    // Each of these is a distinct route by which an attacker might hope to lift the denial.
    let attempts: Vec<(&str, PresentedCredential, TrustSession)> = vec![
        ("forged signature", forged, session(JESSE, NOW)),
        (
            "third party's credential",
            signed_cred(&sk, JESSE, "25", 1),
            session(MALLORY, NOW),
        ),
        (
            "stale credential",
            signed_cred(&sk, JESSE, "25", 400),
            session(JESSE, NOW),
        ),
        (
            "self-asserted credential",
            signed_cred(&SecretKey::from_seed(0x5151), MALLORY, "25", 1),
            session(MALLORY, NOW),
        ),
    ];

    for (why, cred, sess) in attempts {
        let verdict = trust_admit_verdict(&cred, &rules, &sess, &iri(RESOURCE_X), ACR_ABAC_RULE);
        assert!(verdict.is_none(), "{why}: must admit nothing");
        assert_eq!(
            union_trust_grants(
                Decision::Forbidden,
                verdict.as_ref(),
                Some(&sess.agent),
                &iri(RESOURCE_X)
            ),
            Decision::Forbidden,
            "{why}: the 403 must stand"
        );
    }
}

#[test]
fn adversarial_an_anonymous_requester_is_never_upgraded_even_with_a_valid_grant() {
    let (sk, pk) = gov_key();
    let verdict = trust_admit_verdict(
        &signed_cred(&sk, JESSE, "25", 1),
        &gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X),
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        ACR_ABAC_RULE,
    )
    .expect("a validly-signed in-scope credential must admit");

    // `Unauthenticated` means no verified WebID reached the server, so there is no authenticated
    // agent the holder binding could have bound against: the 401 must stand.
    assert_eq!(
        union_trust_grants(
            Decision::Unauthenticated,
            Some(&verdict),
            Some(&iri(JESSE)),
            &iri(RESOURCE_X)
        ),
        Decision::Unauthenticated
    );
    // ...and with no authenticated WebID to pass at all, the binding check refuses it too.
    assert_eq!(
        union_trust_grants(
            Decision::Unauthenticated,
            Some(&verdict),
            None,
            &iri(RESOURCE_X)
        ),
        Decision::Unauthenticated
    );
}

#[test]
fn adversarial_a_derived_denial_refuses_the_whole_verdict() {
    let (sk, pk) = gov_key();
    // The controller's rule grants read but ALSO denies write on the same resource. An allow-only
    // union cannot express the denial, so honouring the read alone would misrepresent the policy:
    // the seam must refuse the verdict outright.
    const RULE_WITH_DENY: &str = r#"@prefix schema: <http://schema.org/> .
@prefix math:   <http://www.w3.org/2000/10/swap/math#> .
@prefix auth:   <https://sparq.dev/ns/auth#> .
{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <https://pod.ex/resourceX> } .
{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:denyWrite <https://pod.ex/resourceX> } .
"#;

    let verdict = trust_admit_verdict(
        &signed_cred(&sk, JESSE, "25", 1),
        &gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X),
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        RULE_WITH_DENY,
    );
    assert!(verdict.is_none(), "a derived denial must refuse the verdict");
    assert_eq!(
        union_trust_grants(
            Decision::Forbidden,
            verdict.as_ref(),
            Some(&iri(JESSE)),
            &iri(RESOURCE_X)
        ),
        Decision::Forbidden
    );
}

#[test]
fn adversarial_a_grant_derived_for_a_third_party_is_not_usable_by_the_requester() {
    let (sk, pk) = gov_key();
    // The `.acr` rule hands the grant to a FIXED third party rather than to the credential
    // subject. Jesse's credential fires it, but the grant is not his to use.
    const RULE_GRANTING_A_THIRD_PARTY: &str = r#"@prefix schema: <http://schema.org/> .
@prefix math:   <http://www.w3.org/2000/10/swap/math#> .
@prefix auth:   <https://sparq.dev/ns/auth#> .
{ ?x schema:age ?y . ?y math:greaterThan 18 } => { <https://mallory.ex/card#me> auth:read <https://pod.ex/resourceX> } .
"#;

    let verdict = trust_admit_verdict(
        &signed_cred(&sk, JESSE, "25", 1),
        &gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X),
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        RULE_GRANTING_A_THIRD_PARTY,
    );
    assert!(verdict.is_none(), "a third party's grant must not admit");
}

#[test]
fn adversarial_an_issued_grant_cannot_be_replayed_onto_another_resource_or_requester() {
    let (sk, pk) = gov_key();
    // Issue ONE genuine grant for Jesse/resourceX...
    let verdict = trust_admit_verdict(
        &signed_cred(&sk, JESSE, "25", 1),
        &gov_age_rules(GOV_ISSUER, &pk, RESOURCE_X),
        &session(JESSE, NOW),
        &iri(RESOURCE_X),
        ACR_ABAC_RULE,
    )
    .expect("a validly-signed in-scope credential must admit");

    // ...then try to apply THAT EXACT grant (it is `Clone`, and a caller may hold it across
    // requests) to denials it was never issued for. Each must stand.
    for (why, agent, target) in [
        ("another resource", Some(iri(JESSE)), iri(RESOURCE_Y)),
        ("another requester", Some(iri(MALLORY)), iri(RESOURCE_X)),
        ("both", Some(iri(MALLORY)), iri(RESOURCE_Y)),
        ("no authenticated requester", None, iri(RESOURCE_X)),
    ] {
        let replayed = verdict.clone();
        assert_eq!(
            union_trust_grants(Decision::Forbidden, Some(&replayed), agent.as_ref(), &target),
            Decision::Forbidden,
            "{}: the 403 must stand",
            why
        );
        assert_eq!(
            replayed.union_onto(Decision::Forbidden, agent.as_ref(), &target),
            Decision::Forbidden,
            "{}: the 403 must stand for the direct union too",
            why
        );
        // ...and it must not widen an unrelated allow either.
        let allow = Decision::Allow(BTreeSet::from([AccessMode::Append]));
        assert_eq!(
            replayed.union_onto(allow.clone(), agent.as_ref(), &target),
            allow,
            "{}: an unrelated allow must not gain the granted modes",
            why
        );
    }
}

#[test]
fn adversarial_a_grant_derived_for_another_resource_is_not_usable_here() {
    let (sk, pk) = gov_key();
    // The trust rule's scope covers resourceY (so admission succeeds for a resourceY request),
    // but the `.acr` rule only ever grants read on resourceX.
    let verdict = trust_admit_verdict(
        &signed_cred(&sk, JESSE, "25", 1),
        &gov_age_rules(GOV_ISSUER, &pk, RESOURCE_Y),
        &session(JESSE, NOW),
        &iri(RESOURCE_Y),
        ACR_ABAC_RULE,
    );
    assert!(
        verdict.is_none(),
        "a grant scoped to another resource must not admit here"
    );
}
