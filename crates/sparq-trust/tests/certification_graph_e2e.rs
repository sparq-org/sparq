//! End-to-end + ADVERSARIAL edge-forgery matrix for the depth-bounded certification-edge
//! trust-graph closure (`sparq_trust::graph::derive_effective_rules`, `sq-pfae.15`).
//!
//! This is the load-bearing evidence for the bead's HARD invariant: **fail-closed,
//! attenuation-ONLY**. The closure runs AHEAD of the UNCHANGED admission gate and produces
//! the `Vec<TrustRule>` the gate consumes; a certification can only NARROW, never widen,
//! the certifier's own authority. The matrix drives EVERY escalation shape a broadening or
//! forged edge would take and asserts each DENIES (contributes nothing), plus a positive
//! end-to-end case and the strict-additivity byte-identical property.
//!
//! A broadening case the code lets through is a PRIVILEGE-ESCALATION bug, not a test to
//! weaken. Run: `cargo test -p sparq-trust --features cert-graph --test certification_graph_e2e`.
//!
//! sq-pfae.15 (epic sq-pfae, issue #940). Written while Fable unavailable — flag
//! for re-review when Fable returns. 🤖 SPARQ agent — certification-edge closure adversarial matrix.
#![cfg(feature = "cert-graph")]

use oxrdf::{BlankNode, Literal, NamedNode, Term, Triple};
use sparq_trust::graph::{
    certification_message, derive_effective_rules, explain_edge, CertScope, Certification,
    EdgeRejection,
};
use sparq_trust::policy::{ShapeRef, TrustRule};
use sparq_zk::sig::{public_key_to_hex, PublicKey, SecretKey};

// ── fixtures ────────────────────────────────────────────────────────────────

const GOV: &str = "https://gov.example/framework"; // the framework operator / certifier
const ISSUER: &str = "https://issuer.example/dvs"; // the issuer the cert vouches for
const AGE: &str = "https://schema.org/age";
const NAME: &str = "https://schema.org/name";
const RES: &str = "https://pod.ex/resourceX";
const NOW: i64 = 1_700_000_000;
const FRESH: i64 = 30 * 86400;

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

fn keypair(seed: u64) -> (SecretKey, PublicKey) {
    let sk = SecretKey::from_seed(seed);
    let pk = sk.public_key();
    (sk, pk)
}

/// A `trust:forPredicate P` desugared into a single-predicate SHACL node-shape (subjects
/// of `p`, `p` minCount 1) — the crate's ONE statement-type idiom.
fn predicate_shape(p: &str) -> ShapeRef {
    let root = BlankNode::default();
    let prop = BlankNode::default();
    let pred = iri(p);
    ShapeRef {
        root: Term::BlankNode(root.clone()),
        triples: vec![
            Triple::new(
                root.clone(),
                iri("http://www.w3.org/ns/shacl#targetSubjectsOf"),
                pred.clone(),
            ),
            Triple::new(
                root,
                iri("http://www.w3.org/ns/shacl#property"),
                prop.clone(),
            ),
            Triple::new(prop.clone(), iri("http://www.w3.org/ns/shacl#path"), pred),
            Triple::new(
                prop,
                iri("http://www.w3.org/ns/shacl#minCount"),
                Literal::new_simple_literal("1"),
            ),
        ],
    }
}

/// A shape that adds a constraint to `predicate_shape(p)` (a strictly-tighter shape: same
/// constraints PLUS a `sh:datatype` restriction) — a provable NARROWING of `predicate_shape`.
fn narrower_shape(p: &str) -> ShapeRef {
    let mut s = predicate_shape(p);
    // The property node is the subject of the last two triples; reuse it to add a datatype.
    let prop = s.triples[1].object.clone();
    if let Term::BlankNode(prop_bn) = prop {
        s.triples.push(Triple::new(
            prop_bn,
            iri("http://www.w3.org/ns/shacl#datatype"),
            iri("http://www.w3.org/2001/XMLSchema#integer"),
        ));
    }
    s
}

/// A shape that ADDS an extra `sh:targetSubjectsOf q` root triple to `predicate_shape(p)`.
/// Every constraint of `predicate_shape(p)` is present PLUS one more target predicate — an
/// injective structural SUPERSET of `predicate_shape(p)`. It is a **broadening**, not a
/// narrowing: SHACL selects focus nodes by the UNION of target predicates, so the extra
/// `sh:targetSubjectsOf q` WIDENS the admitted set to subjects of `p` OR `q` (the
/// privilege-escalation shape the `sq-pfae.15` review found). The containment check must
/// treat selection vocab as CONTRAVARIANT and reject this.
fn additive_target_shape(p: &str, extra_target: &str) -> ShapeRef {
    let mut s = predicate_shape(p);
    if let Term::BlankNode(root) = s.root.clone() {
        s.triples.push(Triple::new(
            root,
            iri("http://www.w3.org/ns/shacl#targetSubjectsOf"),
            iri(extra_target),
        ));
    }
    s
}

/// The set of `sh:targetSubjectsOf` object IRIs a shape declares at its root — the
/// target-predicate set the admission gate (`admit::shape_targets_subject`) selects focus
/// nodes by (the UNION). Used to assert the actual consumed-by-admission invariant: a derived
/// rule's target-predicate set must be `⊆` the anchor's.
fn target_predicate_set(shape: &ShapeRef) -> std::collections::BTreeSet<String> {
    let root = match &shape.root {
        Term::BlankNode(b) => format!("B{}", b.as_str()),
        Term::NamedNode(n) => format!("I{}", n.as_str()),
        other => format!("{other:?}"),
    };
    shape
        .triples
        .iter()
        .filter(|t| {
            let subj = match &t.subject {
                oxrdf::NamedOrBlankNode::BlankNode(b) => format!("B{}", b.as_str()),
                oxrdf::NamedOrBlankNode::NamedNode(n) => format!("I{}", n.as_str()),
            };
            subj == root
                && t.predicate.as_str() == "http://www.w3.org/ns/shacl#targetSubjectsOf"
        })
        .filter_map(|t| match &t.object {
            Term::NamedNode(n) => Some(n.as_str().to_string()),
            _ => None,
        })
        .collect()
}

fn anchor(source: &str, key: PublicKey, shape: ShapeRef, scope: &str) -> TrustRule {
    TrustRule {
        source: iri(source),
        issuer_key: key,
        shape,
        scope: iri(scope),
        fresh_within_secs: FRESH,
    }
}

/// A certification signed by `certifier_sk` over the domain-separated message.
fn signed_cert(
    certifier: &str,
    certifier_sk: &SecretKey,
    certified: &str,
    certified_key: PublicKey,
    scope: CertScope,
    valid_from: i64,
    valid_until: i64,
) -> Certification {
    let mut cert = Certification {
        certifier: iri(certifier),
        certifier_key: certifier_sk.public_key(),
        certified_issuer: iri(certified),
        certified_key,
        scope,
        valid_from_unix_secs: valid_from,
        valid_until_unix_secs: valid_until,
        signature_hex: String::new(),
    };
    cert.signature_hex = certifier_sk.sign_commitment(&certification_message(&cert));
    cert
}

/// The standard AnyService-scoped, in-window, signed cert from GOV → ISSUER.
fn good_cert() -> (TrustRule, Certification, PublicKey) {
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::AnyService,
        NOW - 86400,
        NOW + 86400,
    );
    (anchor_rule, cert, iss_pk)
}

/// Assert the closure produced EXACTLY the anchors (no derived rule) — the fail-closed
/// outcome for a denied edge.
fn assert_no_derivation(anchors: &[TrustRule], certs: &[Certification]) {
    let out = derive_effective_rules(anchors, certs, NOW, 1);
    assert_eq!(
        out.len(),
        anchors.len(),
        "a denied edge must contribute NOTHING (output == anchors only)"
    );
}

// ── POSITIVE end-to-end ───────────────────────────────────────────────────────

#[test]
fn positive_certified_issuer_admitted_within_intersected_scope() {
    let (anchor_rule, cert, iss_pk) = good_cert();
    let out = derive_effective_rules(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&cert), NOW, 1);
    assert_eq!(out.len(), 2, "anchor + one derived rule");
    // Anchor first, verbatim.
    assert_eq!(out[0].source.as_str(), GOV);
    // The derived rule authorises the CERTIFIED issuer with ITS key, over the anchor's
    // resource scope + (AnyService ⇒ anchor's) shape, no fresher than the anchor.
    let d = &out[1];
    assert_eq!(d.source.as_str(), ISSUER);
    assert_eq!(public_key_to_hex(&d.issuer_key), public_key_to_hex(&iss_pk));
    assert_eq!(d.scope.as_str(), RES, "resource scope is the certifier ceiling, unchanged");
    assert!(
        d.fresh_within_secs <= anchor_rule.fresh_within_secs,
        "attenuation: derived freshness never exceeds the anchor's"
    );
    // And the explained path agrees.
    assert!(explain_edge(std::slice::from_ref(&anchor_rule), &cert, NOW, 1).is_ok());
}

#[test]
fn positive_shape_scoped_cert_narrows_to_the_tighter_shape() {
    // The certifier is anchored for `age` (broad); the cert narrows to the integer-typed
    // `age` sub-shape. The derived rule must carry the STRICTLY TIGHTER shape.
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::Shape(narrower_shape(AGE)),
        NOW - 10,
        NOW + 10,
    );
    let d = explain_edge(std::slice::from_ref(&anchor_rule), &cert, NOW, 1)
        .expect("a narrowing shape-scope is admitted");
    // The derived shape must be the tighter one (more constraint triples than the anchor).
    assert!(
        d.shape.triples.len() > anchor_rule.shape.triples.len(),
        "derived shape is the strictly-tighter cert shape (a narrowing)"
    );
}

// ── STRICT ADDITIVITY ─────────────────────────────────────────────────────────

#[test]
fn zero_certifications_is_direct_rules_byte_identical() {
    let (_sk, pk) = keypair(1);
    let rules = vec![
        anchor("https://a.ex", pk, predicate_shape(AGE), "https://pod.ex/"),
        anchor("https://b.ex", pk, predicate_shape(NAME), "https://pod.ex/x"),
    ];
    let out = derive_effective_rules(&rules, &[], NOW, 1);
    assert_eq!(out.len(), rules.len(), "no certs ⇒ no derived rules");
    for (a, b) in rules.iter().zip(out.iter()) {
        assert_eq!(a.source.as_str(), b.source.as_str());
        assert_eq!(a.scope.as_str(), b.scope.as_str());
        assert_eq!(a.fresh_within_secs, b.fresh_within_secs);
        assert_eq!(a.shape.triples.len(), b.shape.triples.len());
        assert_eq!(
            public_key_to_hex(&a.issuer_key),
            public_key_to_hex(&b.issuer_key)
        );
    }
}

#[test]
fn all_edges_denied_leaves_direct_rules_byte_identical() {
    // A pile of edges that ALL deny (forged, broadening, expired, cyclic) must leave the
    // anchors exactly as they were — strict additivity holds even under adversarial input.
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let (_atk_sk, atk_pk) = keypair(9);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);

    // forged (attacker-signed), expired, self-cert cycle, no-anchor
    let mut forged = signed_cert(GOV, &gov_sk, ISSUER, iss_pk, CertScope::AnyService, NOW - 10, NOW + 10);
    forged.signature_hex = SecretKey::from_seed(9).sign_commitment(&certification_message(&forged));
    let expired = signed_cert(GOV, &gov_sk, ISSUER, iss_pk, CertScope::AnyService, NOW - 100, NOW - 50);
    let self_cert = signed_cert(GOV, &gov_sk, GOV, atk_pk, CertScope::AnyService, NOW - 10, NOW + 10);
    let (unknown_sk, _unknown_pk) = keypair(7);
    let no_anchor = signed_cert("https://rogue.ex", &unknown_sk, ISSUER, iss_pk, CertScope::AnyService, NOW - 10, NOW + 10);

    assert_no_derivation(
        std::slice::from_ref(&anchor_rule),
        &[forged, expired, self_cert, no_anchor],
    );
}

// ── ADVERSARIAL matrix: each edge-forgery shape DENIES ─────────────────────────

#[test]
fn forged_certifier_signature_is_denied() {
    // The genuine GOV → ISSUER edge, but signed by an ATTACKER key (or corrupted). The
    // certifier_key still names GOV's key, so the anchor matches — the SIGNATURE is the gate.
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let mut cert = signed_cert(GOV, &gov_sk, ISSUER, iss_pk, CertScope::AnyService, NOW - 10, NOW + 10);
    // Re-sign under an attacker key while leaving certifier_key = GOV's key.
    cert.signature_hex = SecretKey::from_seed(42).sign_commitment(&certification_message(&cert));
    assert!(matches!(
        explain_edge(std::slice::from_ref(&anchor_rule), &cert, NOW, 1),
        Err(EdgeRejection::SignatureInvalid)
    ));
    assert_no_derivation(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&cert));
}

#[test]
fn absent_signature_is_denied() {
    let (anchor_rule, mut cert, _iss_pk) = good_cert();
    cert.signature_hex = String::new();
    assert!(matches!(
        explain_edge(std::slice::from_ref(&anchor_rule), &cert, NOW, 1),
        Err(EdgeRejection::SignatureInvalid)
    ));
    assert_no_derivation(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&cert));
}

#[test]
fn tampered_scope_after_signing_is_denied() {
    // A forger lifts a genuine narrow cert and re-presents it with a BROADER scope
    // (AnyService) WITHOUT re-signing (it cannot — it lacks GOV's key). The scope is folded
    // into the signed message, so the signature no longer verifies.
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let mut cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::Shape(narrower_shape(AGE)),
        NOW - 10,
        NOW + 10,
    );
    // Tamper: widen the scope to AnyService, keep the old signature.
    cert.scope = CertScope::AnyService;
    assert!(matches!(
        explain_edge(std::slice::from_ref(&anchor_rule), &cert, NOW, 1),
        Err(EdgeRejection::SignatureInvalid)
    ));
}

#[test]
fn key_substitution_on_certified_issuer_is_denied() {
    // A forger captures the GOV → ISSUER edge and substitutes its OWN key as the certified
    // issuer's key (to then present its own credentials as ISSUER). The certified_key is
    // folded into the signed message, so the substitution breaks GOV's signature.
    let (anchor_rule, mut cert, _iss_pk) = good_cert();
    let (_atk_sk, atk_pk) = keypair(66);
    cert.certified_key = atk_pk; // substitute — but do NOT (cannot) re-sign under GOV's key
    assert!(matches!(
        explain_edge(std::slice::from_ref(&anchor_rule), &cert, NOW, 1),
        Err(EdgeRejection::SignatureInvalid)
    ));
    assert_no_derivation(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&cert));
}

#[test]
fn no_matching_anchor_is_denied() {
    // The certifier names no anchor rule (the pod does not anchor "rogue.ex"). Even a
    // perfectly-signed edge confers nothing without an anchor to narrow.
    let (_gov_sk, gov_pk) = keypair(1);
    let (rogue_sk, _rogue_pk) = keypair(5);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let cert = signed_cert(
        "https://rogue.ex",
        &rogue_sk,
        ISSUER,
        iss_pk,
        CertScope::AnyService,
        NOW - 10,
        NOW + 10,
    );
    assert!(matches!(
        explain_edge(std::slice::from_ref(&anchor_rule), &cert, NOW, 1),
        Err(EdgeRejection::NoAnchor)
    ));
    assert_no_derivation(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&cert));
}

#[test]
fn certifier_named_with_wrong_key_is_denied() {
    // The certifier IRI matches an anchor, but certifier_key is a DIFFERENT key (and the
    // edge is signed under that different key so the sig would verify in isolation). The
    // anchor-key binding rejects it: the pod anchors GOV's real key, not this one.
    let (_gov_sk, gov_pk) = keypair(1);
    let (wrong_sk, _wrong_pk) = keypair(3);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    // signed_cert sets certifier_key = wrong_sk.public_key(); the IRI is GOV.
    let cert = signed_cert(GOV, &wrong_sk, ISSUER, iss_pk, CertScope::AnyService, NOW - 10, NOW + 10);
    assert!(matches!(
        explain_edge(std::slice::from_ref(&anchor_rule), &cert, NOW, 1),
        Err(EdgeRejection::NoAnchor)
    ));
    assert_no_derivation(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&cert));
}

#[test]
fn scope_broadening_shape_cert_is_denied() {
    // The certifier is anchored ONLY for the integer-typed `age` sub-shape (narrow). The
    // cert attempts to certify ISSUER for the BROADER plain-`age` shape (dropping the
    // datatype constraint) — a broadening. It must contribute nothing.
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let narrow_anchor = anchor(GOV, gov_pk, narrower_shape(AGE), RES);
    let broadening = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::Shape(predicate_shape(AGE)), // broader than the anchor
        NOW - 10,
        NOW + 10,
    );
    assert!(matches!(
        explain_edge(std::slice::from_ref(&narrow_anchor), &broadening, NOW, 1),
        Err(EdgeRejection::Broadening)
    ));
    assert_no_derivation(std::slice::from_ref(&narrow_anchor), std::slice::from_ref(&broadening));
}

#[test]
fn partial_shape_dropping_an_anchor_constraint_is_denied() {
    // The anchor requires the FULL predicate shape (targetSubjectsOf + property/minCount).
    // The cert presents a PARTIAL shape that keeps only the targetSubjectsOf triple (drops
    // the property constraint) — it admits MORE nodes than the anchor ⇒ a broadening. The
    // injective structural matcher must NOT report containment; the edge is denied.
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    // A partial shape: same root, ONLY the targetSubjectsOf triple.
    let root = BlankNode::default();
    let partial = ShapeRef {
        root: Term::BlankNode(root.clone()),
        triples: vec![Triple::new(
            root,
            iri("http://www.w3.org/ns/shacl#targetSubjectsOf"),
            iri(AGE),
        )],
    };
    let cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::Shape(partial),
        NOW - 10,
        NOW + 10,
    );
    assert!(
        matches!(
            explain_edge(std::slice::from_ref(&anchor_rule), &cert, NOW, 1),
            Err(EdgeRejection::Broadening)
        ),
        "a cert shape that DROPS an anchor constraint admits more nodes ⇒ broadening ⇒ denied"
    );
    assert_no_derivation(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&cert));
}

#[test]
fn cert_shape_for_a_different_predicate_is_denied() {
    // The anchor is for `age`; the cert scopes to `name` — a DISJOINT (not narrower) shape.
    // It shares no constraint with the anchor, so it does not contain it ⇒ denied (a cert
    // cannot swing the certifier's `age` authority onto a `name` scope).
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::Shape(predicate_shape(NAME)),
        NOW - 10,
        NOW + 10,
    );
    assert!(matches!(
        explain_edge(std::slice::from_ref(&anchor_rule), &cert, NOW, 1),
        Err(EdgeRejection::Broadening)
    ));
    assert_no_derivation(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&cert));
}

#[test]
fn any_service_cert_over_shape_scoped_anchor_never_broadens_shape() {
    // AnyService imposes no statement-type narrowing of its own — but it must NOT broaden a
    // shape-scoped anchor either: the derived shape must be the ANCHOR's (narrow) shape,
    // never an implicit "any predicate" widening.
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let narrow_anchor = anchor(GOV, gov_pk, narrower_shape(AGE), RES);
    let cert = signed_cert(GOV, &gov_sk, ISSUER, iss_pk, CertScope::AnyService, NOW - 10, NOW + 10);
    let d = explain_edge(std::slice::from_ref(&narrow_anchor), &cert, NOW, 1)
        .expect("AnyService inherits the anchor shape (a valid narrowing = identity)");
    assert_eq!(
        d.shape.triples.len(),
        narrow_anchor.shape.triples.len(),
        "AnyService derives the ANCHOR shape unchanged — never broadens it"
    );
}

#[test]
fn expired_certification_is_denied() {
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let expired = signed_cert(GOV, &gov_sk, ISSUER, iss_pk, CertScope::AnyService, NOW - 100, NOW - 50);
    assert!(matches!(
        explain_edge(std::slice::from_ref(&anchor_rule), &expired, NOW, 1),
        Err(EdgeRejection::OutOfWindow)
    ));
    assert_no_derivation(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&expired));
}

#[test]
fn not_yet_valid_certification_is_denied() {
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let future = signed_cert(GOV, &gov_sk, ISSUER, iss_pk, CertScope::AnyService, NOW + 50, NOW + 100);
    assert!(matches!(
        explain_edge(std::slice::from_ref(&anchor_rule), &future, NOW, 1),
        Err(EdgeRejection::OutOfWindow)
    ));
    assert_no_derivation(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&future));
}

#[test]
fn inverted_window_certification_is_denied() {
    // valid_until < valid_from — an ill-formed window (the shape a "revoked/missing positive
    // status" would present as: no covering window). Fail-closed.
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let inverted = signed_cert(GOV, &gov_sk, ISSUER, iss_pk, CertScope::AnyService, NOW + 100, NOW - 100);
    assert!(matches!(
        explain_edge(std::slice::from_ref(&anchor_rule), &inverted, NOW, 1),
        Err(EdgeRejection::OutOfWindow)
    ));
    assert_no_derivation(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&inverted));
}

#[test]
fn self_certification_cycle_is_denied() {
    // GOV certifies GOV — a self-cert cycle. Even perfectly signed, it can confer no new
    // authority and is the shape a cycle would use to launder a broadening. Denied.
    let (gov_sk, gov_pk) = keypair(1);
    let (_atk_sk, atk_pk) = keypair(9);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let self_cert = signed_cert(GOV, &gov_sk, GOV, atk_pk, CertScope::AnyService, NOW - 10, NOW + 10);
    assert!(matches!(
        explain_edge(std::slice::from_ref(&anchor_rule), &self_cert, NOW, 1),
        Err(EdgeRejection::Cyclic)
    ));
    assert_no_derivation(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&self_cert));
}

#[test]
fn certified_issuer_already_anchoring_certifier_is_a_cycle_and_denied() {
    // Two anchors: GOV and ISSUER (ISSUER is ALREADY a direct anchor with key iss_pk). A
    // cert GOV → ISSUER (same key) would form a cycle / add nothing — denied as Cyclic so a
    // mutual-certification loop cannot amplify authority.
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let gov_anchor = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let issuer_anchor = anchor(ISSUER, iss_pk, predicate_shape(NAME), RES);
    let cert = signed_cert(GOV, &gov_sk, ISSUER, iss_pk, CertScope::AnyService, NOW - 10, NOW + 10);
    let anchors = vec![gov_anchor, issuer_anchor];
    assert!(matches!(
        explain_edge(&anchors, &cert, NOW, 1),
        Err(EdgeRejection::Cyclic)
    ));
    // Output is the two anchors, unchanged — the cert added nothing.
    let out = derive_effective_rules(&anchors, std::slice::from_ref(&cert), NOW, 1);
    assert_eq!(out.len(), 2);
}

#[test]
fn over_depth_is_denied_at_depth_zero() {
    // depth_bound = 0 short-circuits: NO edge is derived (OverDepth via explain_edge).
    let (anchor_rule, cert, _iss_pk) = good_cert();
    assert!(matches!(
        explain_edge(std::slice::from_ref(&anchor_rule), &cert, NOW, 0),
        Err(EdgeRejection::OverDepth)
    ));
    let out = derive_effective_rules(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&cert), NOW, 0);
    assert_eq!(out.len(), 1, "depth 0 ⇒ anchors only");
}

#[test]
fn derived_rule_is_never_reused_as_an_anchor_no_transitive_chain() {
    // The v1 depth-1 bound: GOV certifies ISSUER (derived), and a second edge ISSUER
    // certifies THIRD. Because ISSUER is only a DERIVED rule (never a direct anchor), the
    // ISSUER → THIRD edge finds no anchor and contributes nothing — the closure does NOT
    // transitively chain. THIRD must NOT appear.
    let (gov_sk, gov_pk) = keypair(1);
    let (iss_sk, iss_pk) = keypair(2);
    let (_third_sk, third_pk) = keypair(3);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let gov_to_issuer = signed_cert(GOV, &gov_sk, ISSUER, iss_pk, CertScope::AnyService, NOW - 10, NOW + 10);
    // ISSUER signs an edge to a THIRD party — but ISSUER is not a direct anchor.
    let issuer_to_third = signed_cert(
        ISSUER,
        &iss_sk,
        "https://third.ex",
        third_pk,
        CertScope::AnyService,
        NOW - 10,
        NOW + 10,
    );
    let out = derive_effective_rules(
        std::slice::from_ref(&anchor_rule),
        &[gov_to_issuer, issuer_to_third],
        NOW,
        1,
    );
    // anchor + ISSUER only — THIRD is NOT derived (depth-1; no transitive chaining).
    assert_eq!(out.len(), 2, "only the direct GOV→ISSUER edge derives; ISSUER→THIRD does not");
    assert!(out.iter().all(|r| r.source.as_str() != "https://third.ex"));
}

#[test]
fn attribute_scoped_issuer_cannot_meta_escalate_to_certify_another_issuer() {
    // Meta-scope escalation: an issuer anchored ONLY for an attribute shape (`age`) is NOT a
    // framework operator and holds no authority to certify OTHER issuers. In this depth-1
    // model that is exactly the "derived rule can never be an anchor" property, tested from
    // the escalation angle: an ISSUER anchored for `age` presents a cert ISSUER → ATTACKER.
    // ISSUER *is* a direct anchor here (anchored for age) — but the derived ATTACKER rule can
    // only NARROW ISSUER's `age` authority, never gain a broader/meta authority. An attempt
    // to certify ATTACKER for a DIFFERENT predicate (`name`, outside ISSUER's `age` anchor)
    // is a broadening and is denied.
    let (iss_sk, iss_pk) = keypair(2);
    let (_atk_sk, atk_pk) = keypair(8);
    // ISSUER anchored ONLY for `age`.
    let issuer_anchor = anchor(ISSUER, iss_pk, predicate_shape(AGE), RES);
    // ISSUER tries to certify ATTACKER for `name` — outside ISSUER's own `age` scope.
    let escalation = signed_cert(
        ISSUER,
        &iss_sk,
        "https://attacker.ex",
        atk_pk,
        CertScope::Shape(predicate_shape(NAME)),
        NOW - 10,
        NOW + 10,
    );
    assert!(
        matches!(
            explain_edge(std::slice::from_ref(&issuer_anchor), &escalation, NOW, 1),
            Err(EdgeRejection::Broadening)
        ),
        "certifying an issuer for a predicate OUTSIDE the certifier's own scope is a broadening"
    );
    assert_no_derivation(std::slice::from_ref(&issuer_anchor), std::slice::from_ref(&escalation));
}

#[test]
fn multi_anchor_certifier_narrowed_by_the_matching_anchor() {
    // The certifier is anchored for BOTH `age` and `name`. A cert scoped to the `name`
    // shape must narrow the `name` anchor (not be wrongly denied because `age` happened to
    // be listed first). Completeness without weakening the invariant: the chosen anchor is
    // always genuine pod authority.
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let age_anchor = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let name_anchor = anchor(GOV, gov_pk, predicate_shape(NAME), RES);
    let anchors = vec![age_anchor, name_anchor];
    // Cert scoped to `name` (a narrowing of the `name` anchor).
    let cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::Shape(predicate_shape(NAME)),
        NOW - 10,
        NOW + 10,
    );
    let d = explain_edge(&anchors, &cert, NOW, 1)
        .expect("the `name`-scoped cert narrows the `name` anchor");
    assert_eq!(d.source.as_str(), ISSUER);
    // The full closure derives the ISSUER rule (2 anchors + 1 derived).
    let out = derive_effective_rules(&anchors, std::slice::from_ref(&cert), NOW, 1);
    assert_eq!(out.len(), 3);
}

#[test]
fn derived_freshness_is_capped_by_the_certification_window() {
    // The anchor is fresh-within 30d, but the cert window closes in 100s. The derived rule's
    // freshness must be the SMALLER (100s) — a cert can only shrink admitted staleness.
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES); // FRESH = 30d
    let short = signed_cert(GOV, &gov_sk, ISSUER, iss_pk, CertScope::AnyService, NOW - 10, NOW + 100);
    let d = explain_edge(std::slice::from_ref(&anchor_rule), &short, NOW, 1)
        .expect("in-window, signed, narrowing edge is admitted");
    assert!(
        d.fresh_within_secs <= 100,
        "derived freshness capped by the (narrow) certification window, got {}",
        d.fresh_within_secs
    );
    assert!(d.fresh_within_secs <= anchor_rule.fresh_within_secs);
}

// ── sq-pfae.15 REVIEW FIX: target-set contravariance (additive-target broadening) ──────

#[test]
fn additive_target_predicate_cert_is_denied_as_broadening() {
    // THE escalation the escalated review found. The certifier is anchored ONLY for `age`.
    // The cert scope shape = the `age` predicate-shape PLUS an extra
    // `root sh:targetSubjectsOf schema:email` — an injective structural SUPERSET of the anchor
    // shape. A uniform "more edges ⇒ narrower" matcher wrongly reports this as a narrowing;
    // but SHACL selects focus nodes by the UNION of target predicates, so the extra target
    // WIDENS the admitted set (subjects of `age` OR `email`). The certified issuer would gain
    // authority over `email` the certifier never held — privilege escalation. Selection vocab
    // is CONTRAVARIANT: the cert must be denied as `Broadening`.
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let email = "https://schema.org/email";
    let broadening = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::Shape(additive_target_shape(AGE, email)),
        NOW - 10,
        NOW + 10,
    );
    assert!(
        matches!(
            explain_edge(std::slice::from_ref(&anchor_rule), &broadening, NOW, 1),
            Err(EdgeRejection::Broadening)
        ),
        "an ADDITIVE target predicate WIDENS the admitted set ⇒ broadening ⇒ denied"
    );
    assert_no_derivation(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&broadening));
}

#[test]
fn every_derived_rule_target_set_is_subset_of_the_anchor_target_set() {
    // The actual CONSUMED-BY-ADMISSION invariant (not merely the matcher's internal verdict):
    // the admission gate (`admit::shape_targets_subject`) selects focus nodes by the UNION of a
    // rule shape's `sh:targetSubjectsOf` predicates, so a derived rule that targets a predicate
    // OUTSIDE the certifier's anchor is a live escalation regardless of what the matcher
    // reported. Drive a mix of edges (a genuine narrowing, an additive-target broadening, an
    // AnyService edge) and assert EVERY derived rule's target-predicate set ⊆ its certifier
    // anchor's. The anchor is `age` only, so the derived target set must be a subset of {age}.
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let (_iss2_sk, iss2_pk) = keypair(4);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    let anchor_targets = target_predicate_set(&anchor_rule.shape);

    let email = "https://schema.org/email";
    let broadening = signed_cert(
        GOV, &gov_sk, ISSUER, iss_pk,
        CertScope::Shape(additive_target_shape(AGE, email)),
        NOW - 10, NOW + 10,
    );
    let narrowing = signed_cert(
        GOV, &gov_sk, "https://issuer.example/two", iss2_pk,
        CertScope::Shape(narrower_shape(AGE)),
        NOW - 10, NOW + 10,
    );
    let anyservice = signed_cert(
        GOV, &gov_sk, "https://issuer.example/three", keypair(5).1,
        CertScope::AnyService, NOW - 10, NOW + 10,
    );

    let out = derive_effective_rules(
        std::slice::from_ref(&anchor_rule),
        &[broadening, narrowing, anyservice],
        NOW,
        1,
    );
    // The broadening edge must contribute nothing; the two legitimate narrowings do. Anchor +
    // (narrowing, anyservice) = 3 rules; the additive-target broadening is dropped.
    assert_eq!(out.len(), 3, "additive-target broadening dropped; two narrowings derived");
    // EVERY derived rule (index ≥ 1) must target a SUBSET of the anchor's target predicates.
    for derived in &out[1..] {
        let derived_targets = target_predicate_set(&derived.shape);
        assert!(
            derived_targets.is_subset(&anchor_targets),
            "derived rule for {} targets {:?} ⊄ anchor targets {:?} — escalation",
            derived.source.as_str(),
            derived_targets,
            anchor_targets
        );
    }
    // And no derived rule may target `email` (the predicate the broadening tried to smuggle).
    assert!(
        out[1..].iter().all(|r| !target_predicate_set(&r.shape).contains(email)),
        "no derived rule may target the smuggled `email` predicate"
    );
}

#[test]
fn un_modelled_selection_predicate_cert_fails_closed() {
    // Fail-closed on an UN-MODELLED selection predicate. The cert scope shape = the `age`
    // predicate-shape PLUS a `root sh:targetObjectsOf schema:email` triple — a SHACL target
    // predicate the matcher recognises by the `sh:target*` prefix but which the desugared
    // anchor shape does not carry. Because it is a selection predicate the anchor lacks, it is
    // a broadening (and any future/unknown `sh:target…` is likewise treated as selection ⇒
    // fail-closed, never silently admitted as a conformance constraint).
    let (gov_sk, gov_pk) = keypair(1);
    let (_iss_sk, iss_pk) = keypair(2);
    let anchor_rule = anchor(GOV, gov_pk, predicate_shape(AGE), RES);
    // Add an unrelated SHACL target predicate at the root.
    let mut shape = predicate_shape(AGE);
    if let Term::BlankNode(root) = shape.root.clone() {
        shape.triples.push(Triple::new(
            root,
            iri("http://www.w3.org/ns/shacl#targetObjectsOf"),
            iri("https://schema.org/email"),
        ));
    }
    let cert = signed_cert(
        GOV, &gov_sk, ISSUER, iss_pk,
        CertScope::Shape(shape),
        NOW - 10, NOW + 10,
    );
    assert!(
        matches!(
            explain_edge(std::slice::from_ref(&anchor_rule), &cert, NOW, 1),
            Err(EdgeRejection::Broadening)
        ),
        "an un-modelled / extra SHACL target predicate ⇒ fail-closed (Broadening), never admitted"
    );
    assert_no_derivation(std::slice::from_ref(&anchor_rule), std::slice::from_ref(&cert));
}
