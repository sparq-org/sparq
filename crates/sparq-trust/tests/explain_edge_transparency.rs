//! Decision-transparency fixture for `sparq_trust::graph::explain_edge` (`sq-qxcvc`).
//!
//! `explain_edge` is the LWS/Solid-WG-facing "why was this graph-driven decision made"
//! surface: it mirrors `derive_effective_rules`'s gate exactly and returns the FIRST
//! failing gate's [`EdgeRejection`] variant. This fixture pins TWO contracts a refactor
//! must not silently break:
//!
//! 1. **The right reason for the right adversary.** Each adversarial edge shape (self-cert
//!    cycle, over-depth, forged/absent/tampered signature, expired/not-yet-valid window,
//!    missing/stale positive status — modelled as a non-covering/inverted window — and a
//!    non-attenuating/broadening scope) yields its EXACT `EdgeRejection` variant, never
//!    `Ok`, and never a *plausible-but-wrong* variant (first-failing-gate ordering is
//!    asserted where an edge trips several gates at once).
//! 2. **Fast/explained agreement.** For a deterministic pseudo-random sample over the
//!    whole adversarial parameter cross-product, `explain_edge(...).is_ok()` equals
//!    "`derive_effective_rules` admitted that edge", and when both admit they derive the
//!    SAME rule. The two paths share `derive_one_explained` today; this property LOCKS the
//!    contract so a refactor cannot split them.
//!
//! **Non-vacuity (mutation-witnessed):** the agreement property FAILS if `explain_edge` is
//! stubbed to always-`Ok` (a rejected edge then explains as admitted while the fast path
//! derives nothing — the per-edge count assertion goes red), and each reason-matrix test
//! FAILS if its gate returns the wrong variant. Verified by mutating `explain_edge` to
//! return the anchor rule on rejection: this suite went red (see PR).
//!
//! NO src change: this file only exercises the public `graph` API (the `sq-6syab` boundary
//! — `graph.rs` / `framework_vocab.rs` untouched).
//!
//! Run: `cargo test -p sparq-trust --features cert-graph --test explain_edge_transparency`.
//!
//! sq-qxcvc (epic sq-6syab program context; design
//! `research/trust-expression-spec.md` §3.4). 🤖 SPARQ agent — explain_edge transparency
//! fixture, written by Claude Fable 5.
#![cfg(feature = "cert-graph")]

use oxrdf::{BlankNode, Literal, NamedNode, Term, Triple};
use sparq_trust::graph::{
    certification_message, derive_effective_rules, explain_edge, CertScope, Certification,
    EdgeRejection,
};
use sparq_trust::policy::{ShapeRef, TrustRule};
use sparq_zk::sig::{public_key_to_hex, PublicKey, SecretKey};

// ── fixtures (the crate's forPredicate shape idiom, as in certification_graph_e2e) ──────

const GOV: &str = "https://gov.example/framework"; // anchored certifier (framework operator)
const ROGUE: &str = "https://rogue.example/op"; // certifier the pod does NOT anchor
const ISSUER: &str = "https://issuer.example/dvs"; // the issuer being vouched for
const ANCHORED_ISSUER: &str = "https://anchored-issuer.example/reg"; // already a direct anchor
const AGE: &str = "https://schema.org/age";
const NAME: &str = "https://schema.org/name";
const EMAIL: &str = "https://schema.org/email";
const RES: &str = "https://pod.ex/resourceX";
const NOW: i64 = 1_700_000_000;
const FRESH: i64 = 30 * 86400;

const SEED_GOV: u64 = 1;
const SEED_ISSUER: u64 = 2;
const SEED_ROGUE: u64 = 5;
const SEED_ANCHORED_ISSUER: u64 = 6;
const SEED_ATTACKER: u64 = 9;

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

fn keypair(seed: u64) -> (SecretKey, PublicKey) {
    let sk = SecretKey::from_seed(seed);
    let pk = sk.public_key();
    (sk, pk)
}

/// `trust:forPredicate P` desugared into a single-predicate SHACL node-shape (subjects of
/// `p`, `p` minCount 1) — the crate's ONE statement-type idiom.
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

/// `predicate_shape(p)` + a `sh:datatype` conformance constraint — a provable NARROWING.
fn narrower_shape(p: &str) -> ShapeRef {
    let mut s = predicate_shape(p);
    if let Term::BlankNode(prop_bn) = s.triples[1].object.clone() {
        s.triples.push(Triple::new(
            prop_bn,
            iri("http://www.w3.org/ns/shacl#datatype"),
            iri("http://www.w3.org/2001/XMLSchema#integer"),
        ));
    }
    s
}

/// `predicate_shape(p)` + an extra root `sh:targetSubjectsOf q` — a target-set BROADENING
/// (SHACL selects by the UNION of targets; the `sq-pfae.15` escalation shape).
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

/// A PARTIAL shape keeping only the target triple (drops the property constraint) — admits
/// MORE nodes than `predicate_shape(p)` ⇒ a conformance-dropping broadening.
fn partial_shape(p: &str) -> ShapeRef {
    let root = BlankNode::default();
    ShapeRef {
        root: Term::BlankNode(root.clone()),
        triples: vec![Triple::new(
            root,
            iri("http://www.w3.org/ns/shacl#targetSubjectsOf"),
            iri(p),
        )],
    }
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

/// The fixed anchor set every case runs against: GOV (for `age`) plus an already-anchored
/// issuer (for `name`) so the anchored-issuer cycle arm is reachable.
fn anchors() -> Vec<TrustRule> {
    let (_g, gov_pk) = keypair(SEED_GOV);
    let (_a, anch_pk) = keypair(SEED_ANCHORED_ISSUER);
    vec![
        anchor(GOV, gov_pk, predicate_shape(AGE), RES),
        anchor(ANCHORED_ISSUER, anch_pk, predicate_shape(NAME), RES),
    ]
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

/// The standard well-formed GOV → ISSUER edge (AnyService, in-window, honestly signed).
fn good_cert() -> Certification {
    let (gov_sk, _gov_pk) = keypair(SEED_GOV);
    let (_i, iss_pk) = keypair(SEED_ISSUER);
    signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::AnyService,
        NOW - 86400,
        NOW + 86400,
    )
}

/// Assert `cert` explains as EXACTLY `expected` AND the fast path agrees it contributes
/// nothing (fail-closed transparency: right reason + no divergence).
fn assert_rejects(cert: &Certification, expected: EdgeRejection) {
    let rules = anchors();
    assert_eq!(
        explain_edge(&rules, cert, NOW, 1).err(),
        Some(expected),
        "explain_edge must return the exact first-failing-gate reason"
    );
    let out = derive_effective_rules(&rules, std::slice::from_ref(cert), NOW, 1);
    assert_eq!(
        out.len(),
        rules.len(),
        "a rejected edge must contribute NOTHING on the fast path"
    );
}

/// Field-wise equality for a derived rule (TrustRule has no PartialEq).
fn assert_same_rule(a: &TrustRule, b: &TrustRule) {
    assert_eq!(a.source.as_str(), b.source.as_str());
    assert_eq!(
        public_key_to_hex(&a.issuer_key),
        public_key_to_hex(&b.issuer_key)
    );
    assert_eq!(a.scope.as_str(), b.scope.as_str());
    assert_eq!(a.fresh_within_secs, b.fresh_within_secs);
    assert_eq!(
        a.shape.root, b.shape.root,
        "derived shapes must be identical"
    );
    assert_eq!(
        a.shape.triples, b.shape.triples,
        "derived shapes must be identical"
    );
}

// ── (a) the adversarial reason matrix: exact variant per adversarial input ──────────────

#[test]
fn self_certification_explains_cyclic() {
    let (gov_sk, _) = keypair(SEED_GOV);
    let (_a, atk_pk) = keypair(SEED_ATTACKER);
    let cert = signed_cert(
        GOV,
        &gov_sk,
        GOV,
        atk_pk,
        CertScope::AnyService,
        NOW - 10,
        NOW + 10,
    );
    assert_rejects(&cert, EdgeRejection::Cyclic);
}

#[test]
fn already_anchored_certified_issuer_explains_cyclic() {
    // GOV certifies an issuer the pod ALREADY anchors directly (same key) — the mutual-
    // certification loop shape. Must be Cyclic, not admitted (no amplification).
    let (gov_sk, _) = keypair(SEED_GOV);
    let (_a, anch_pk) = keypair(SEED_ANCHORED_ISSUER);
    let cert = signed_cert(
        GOV,
        &gov_sk,
        ANCHORED_ISSUER,
        anch_pk,
        CertScope::AnyService,
        NOW - 10,
        NOW + 10,
    );
    assert_rejects(&cert, EdgeRejection::Cyclic);
}

#[test]
fn forged_self_certification_still_explains_cyclic_first_gate_wins() {
    // An edge that trips BOTH the cycle gate and the signature gate (forged self-cert):
    // the FIRST failing gate (Cyclic) is the reason — the transparency surface never
    // reports a later gate for an earlier failure.
    let (gov_sk, _) = keypair(SEED_GOV);
    let (_a, atk_pk) = keypair(SEED_ATTACKER);
    let mut cert = signed_cert(
        GOV,
        &gov_sk,
        GOV,
        atk_pk,
        CertScope::AnyService,
        NOW - 10,
        NOW + 10,
    );
    cert.signature_hex =
        SecretKey::from_seed(SEED_ATTACKER).sign_commitment(&certification_message(&cert));
    assert_rejects(&cert, EdgeRejection::Cyclic);
}

#[test]
fn depth_zero_explains_over_depth_even_for_a_perfect_edge() {
    let cert = good_cert();
    let rules = anchors();
    assert_eq!(
        explain_edge(&rules, &cert, NOW, 0).err(),
        Some(EdgeRejection::OverDepth),
        "depth_bound 0 denies EVERY edge as OverDepth, even a well-formed one"
    );
    let out = derive_effective_rules(&rules, std::slice::from_ref(&cert), NOW, 0);
    assert_eq!(
        out.len(),
        rules.len(),
        "fast path agrees: depth 0 derives nothing"
    );
}

#[test]
fn unanchored_certifier_explains_no_anchor() {
    // Perfectly signed by ROGUE — but the pod anchors no such certifier.
    let (rogue_sk, _) = keypair(SEED_ROGUE);
    let (_i, iss_pk) = keypair(SEED_ISSUER);
    let cert = signed_cert(
        ROGUE,
        &rogue_sk,
        ISSUER,
        iss_pk,
        CertScope::AnyService,
        NOW - 10,
        NOW + 10,
    );
    assert_rejects(&cert, EdgeRejection::NoAnchor);
}

#[test]
fn anchored_iri_with_wrong_key_explains_no_anchor() {
    // The certifier IRI matches the GOV anchor but under a DIFFERENT key (self-consistent
    // signature under that key). The anchor binds IRI + key; a key mismatch is NoAnchor.
    let (wrong_sk, _) = keypair(SEED_ROGUE);
    let (_i, iss_pk) = keypair(SEED_ISSUER);
    let cert = signed_cert(
        GOV,
        &wrong_sk,
        ISSUER,
        iss_pk,
        CertScope::AnyService,
        NOW - 10,
        NOW + 10,
    );
    assert_rejects(&cert, EdgeRejection::NoAnchor);
}

#[test]
fn forged_signature_explains_signature_invalid() {
    // The genuine edge re-signed by an attacker while certifier_key still names GOV's key.
    let mut cert = good_cert();
    cert.signature_hex =
        SecretKey::from_seed(SEED_ATTACKER).sign_commitment(&certification_message(&cert));
    assert_rejects(&cert, EdgeRejection::SignatureInvalid);
}

#[test]
fn absent_signature_explains_signature_invalid() {
    let mut cert = good_cert();
    cert.signature_hex = String::new();
    assert_rejects(&cert, EdgeRejection::SignatureInvalid);
}

#[test]
fn certified_key_substitution_explains_signature_invalid() {
    // A forger lifts the genuine edge and swaps in its own key as the certified issuer's —
    // the certified_key is in the signed preimage, so GOV's signature no longer verifies.
    let mut cert = good_cert();
    let (_a, atk_pk) = keypair(SEED_ATTACKER);
    cert.certified_key = atk_pk;
    assert_rejects(&cert, EdgeRejection::SignatureInvalid);
}

#[test]
fn window_tampered_after_signing_explains_signature_invalid_not_out_of_window() {
    // A forger extends an EXPIRED edge's window without re-signing. The window is in the
    // signed preimage, so the sig gate (earlier) catches the tamper — the reason must be
    // SignatureInvalid, NOT OutOfWindow: transparency reports the forgery, not the
    // symptom the forger tried to fake away.
    let (gov_sk, _) = keypair(SEED_GOV);
    let (_i, iss_pk) = keypair(SEED_ISSUER);
    let mut cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::AnyService,
        NOW - 100,
        NOW - 50,
    );
    cert.valid_until_unix_secs = NOW + 1_000_000; // tampered, not re-signed
    assert_rejects(&cert, EdgeRejection::SignatureInvalid);
}

#[test]
fn expired_certification_explains_out_of_window() {
    let (gov_sk, _) = keypair(SEED_GOV);
    let (_i, iss_pk) = keypair(SEED_ISSUER);
    let cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::AnyService,
        NOW - 100,
        NOW - 50,
    );
    assert_rejects(&cert, EdgeRejection::OutOfWindow);
}

#[test]
fn not_yet_valid_certification_explains_out_of_window() {
    let (gov_sk, _) = keypair(SEED_GOV);
    let (_i, iss_pk) = keypair(SEED_ISSUER);
    let cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::AnyService,
        NOW + 50,
        NOW + 100,
    );
    assert_rejects(&cert, EdgeRejection::OutOfWindow);
}

#[test]
fn missing_positive_status_inverted_window_explains_out_of_window() {
    // `valid_until < valid_from`: the shape a MISSING/STALE positive status presents as —
    // no covering validity window exists. Fail-closed on time/status (§3.3 D3: positive
    // existence-of-a-covering-window semantics, never evidence-of-absence).
    let (gov_sk, _) = keypair(SEED_GOV);
    let (_i, iss_pk) = keypair(SEED_ISSUER);
    let cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::AnyService,
        NOW + 100,
        NOW - 100,
    );
    assert_rejects(&cert, EdgeRejection::OutOfWindow);
}

#[test]
fn broader_scope_than_anchor_explains_broadening() {
    // Anchor GOV only for the tighter integer-typed `age` shape; cert tries plain `age`
    // (drops the datatype constraint) — a widening of what conforms.
    let (gov_sk, _) = keypair(SEED_GOV);
    let (_a, anch_pk) = keypair(SEED_ANCHORED_ISSUER);
    let (_i, iss_pk) = keypair(SEED_ISSUER);
    let rules = vec![
        anchor(GOV, gov_sk.public_key(), narrower_shape(AGE), RES),
        anchor(ANCHORED_ISSUER, anch_pk, predicate_shape(NAME), RES),
    ];
    let cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::Shape(predicate_shape(AGE)),
        NOW - 10,
        NOW + 10,
    );
    assert_eq!(
        explain_edge(&rules, &cert, NOW, 1).err(),
        Some(EdgeRejection::Broadening)
    );
    let out = derive_effective_rules(&rules, std::slice::from_ref(&cert), NOW, 1);
    assert_eq!(out.len(), rules.len(), "fast path agrees: nothing derived");
}

#[test]
fn disjoint_predicate_scope_explains_broadening() {
    // GOV is anchored for `age`; the cert scopes to `email` — disjoint, not a narrowing.
    let (gov_sk, _) = keypair(SEED_GOV);
    let (_i, iss_pk) = keypair(SEED_ISSUER);
    let cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::Shape(predicate_shape(EMAIL)),
        NOW - 10,
        NOW + 10,
    );
    assert_rejects(&cert, EdgeRejection::Broadening);
}

#[test]
fn additive_target_scope_explains_broadening() {
    // The `sq-pfae.15` escalation shape: anchor's `age` shape PLUS an extra
    // `sh:targetSubjectsOf email` — a structural superset that WIDENS the target set.
    let (gov_sk, _) = keypair(SEED_GOV);
    let (_i, iss_pk) = keypair(SEED_ISSUER);
    let cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::Shape(additive_target_shape(AGE, EMAIL)),
        NOW - 10,
        NOW + 10,
    );
    assert_rejects(&cert, EdgeRejection::Broadening);
}

#[test]
fn constraint_dropping_partial_scope_explains_broadening() {
    // Keeps only the target triple, drops the property/minCount conformance constraint —
    // admits MORE nodes than the anchor.
    let (gov_sk, _) = keypair(SEED_GOV);
    let (_i, iss_pk) = keypair(SEED_ISSUER);
    let cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        iss_pk,
        CertScope::Shape(partial_shape(AGE)),
        NOW - 10,
        NOW + 10,
    );
    assert_rejects(&cert, EdgeRejection::Broadening);
}

#[test]
fn well_formed_edge_explains_ok_with_the_same_rule_the_fast_path_derives() {
    // The positive control: a good edge is Ok on BOTH paths and the derived rules agree
    // field-for-field.
    let cert = good_cert();
    let rules = anchors();
    let explained = explain_edge(&rules, &cert, NOW, 1).expect("a well-formed edge is admitted");
    let out = derive_effective_rules(&rules, std::slice::from_ref(&cert), NOW, 1);
    assert_eq!(out.len(), rules.len() + 1, "fast path admits the same edge");
    assert_same_rule(&explained, &out[rules.len()]);
    assert_eq!(explained.source.as_str(), ISSUER);
}

// ── (b) fast/explained agreement over a deterministic pseudo-random sample ──────────────

/// Deterministic splitmix64-style generator — reproducible sample, no RNG dependency.
struct Prng(u64);
impl Prng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// The adversarial parameter cross-product one sampled edge is drawn from. Six dimensions,
/// each with an HONEST value (index 0) and adversarial values. `mode` shapes the draw so
/// the sample actually crosses the admit/reject boundary:
/// - mode 0: ALL dimensions honest ⇒ an admitted edge (the fully-independent product
///   almost never lands here — p ≈ 0.001 — so it is drawn explicitly);
/// - modes 1–2: EXACTLY ONE adversarial dimension ⇒ every single-gate rejection reason;
/// - mode 3: fully independent mix ⇒ multi-gate edges (first-failing-gate ordering).
fn sample_edge(rng: &mut Prng) -> (Certification, u32) {
    let (gov_sk, _) = keypair(SEED_GOV);
    let (rogue_sk, _) = keypair(SEED_ROGUE);
    let (_i, iss_pk) = keypair(SEED_ISSUER);
    let (_a, anch_pk) = keypair(SEED_ANCHORED_ISSUER);
    let (_k, atk_pk) = keypair(SEED_ATTACKER);

    // Per-dimension choice index: 0 is always the honest value.
    let mode = rng.pick(4);
    let adv = rng.pick(6); // which dimension is adversarial in modes 1–2
    fn dim(
        rng: &mut Prng,
        mode: usize,
        n_choices: usize,
        d: usize,
        adversarial_dim: usize,
    ) -> usize {
        match mode {
            0 => 0,
            1 | 2 => {
                if d == adversarial_dim {
                    1 + rng.pick(n_choices - 1)
                } else {
                    0
                }
            }
            _ => rng.pick(n_choices),
        }
    }

    // dim 0 — certifier: GOV (anchored) / unanchored ROGUE / GOV's IRI under the wrong key.
    let (certifier, sk): (&str, &SecretKey) = match dim(rng, mode, 3, 0, adv) {
        0 => (GOV, &gov_sk),
        1 => (ROGUE, &rogue_sk),
        _ => (GOV, &rogue_sk),
    };
    // dim 1 — certified issuer: fresh / self (cycle) / already-anchored (cycle).
    let (certified, certified_key) = match dim(rng, mode, 3, 1, adv) {
        0 => (ISSUER, iss_pk),
        1 => (certifier, atk_pk),
        _ => (ANCHORED_ISSUER, anch_pk),
    };
    // dim 2 — scope: identity or narrowing (honest) / disjoint / additive-target / partial.
    let scope = match dim(rng, mode, 5, 2, adv) {
        0 => {
            // Both honest scopes, still deterministic: AnyService or a provable narrowing.
            if rng.pick(2) == 0 {
                CertScope::AnyService
            } else {
                CertScope::Shape(narrower_shape(AGE))
            }
        }
        1 => CertScope::Shape(predicate_shape(NAME)),
        2 => CertScope::Shape(additive_target_shape(AGE, EMAIL)),
        3 => CertScope::Shape(partial_shape(AGE)),
        _ => CertScope::Shape(predicate_shape(EMAIL)),
    };
    // dim 3 — window: covering / expired / future / inverted (missing positive status).
    let (from, until) = match dim(rng, mode, 4, 3, adv) {
        0 => (NOW - 100, NOW + 100),
        1 => (NOW - 200, NOW - 100),
        2 => (NOW + 100, NOW + 200),
        _ => (NOW + 100, NOW - 100),
    };
    let mut cert = signed_cert(certifier, sk, certified, certified_key, scope, from, until);
    // dim 4 — signature: honest / forged / absent / post-sign key-substitution tamper.
    match dim(rng, mode, 4, 4, adv) {
        0 => {}
        1 => {
            cert.signature_hex =
                SecretKey::from_seed(SEED_ATTACKER).sign_commitment(&certification_message(&cert));
        }
        2 => cert.signature_hex = String::new(),
        _ => cert.certified_key = atk_pk,
    }
    // dim 5 — depth: 1 (honest) or 0 (OverDepth).
    let depth = match dim(rng, mode, 2, 5, adv) {
        0 => 1u32,
        _ => 0u32,
    };
    (cert, depth)
}

#[test]
fn explain_edge_never_diverges_from_derive_effective_rules_on_admission() {
    // THE agreement property (the bead's invariant): for every sampled edge, the explained
    // path admits iff the fast path derives a rule — and when both admit, the SAME rule.
    //
    // NON-VACUOUS by construction: if explain_edge were stubbed to always-Ok, every
    // rejected sample (the majority of the cross-product) would explain as admitted while
    // the fast path derives nothing ⇒ the per-edge count assertion below goes red.
    // (Mutation-witnessed — see the module doc.)
    let rules = anchors();
    let mut rng = Prng(0x5152_5843_5643_0001); // fixed seed: reproducible sample
    let mut admitted = 0usize;
    let mut rejected = 0usize;
    let mut variants_seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for i in 0..160 {
        let (cert, depth) = sample_edge(&mut rng);
        let explained = explain_edge(&rules, &cert, NOW, depth);
        let out = derive_effective_rules(&rules, std::slice::from_ref(&cert), NOW, depth);
        let fast_admitted = out.len() - rules.len();
        assert_eq!(
            usize::from(explained.is_ok()),
            fast_admitted,
            "sample {i}: explain_edge and derive_effective_rules DIVERGED on admission \
             (cert {:?}, depth {depth})",
            cert.certified_issuer
        );
        match explained {
            Ok(rule) => {
                admitted += 1;
                assert_same_rule(&rule, &out[rules.len()]);
            }
            Err(reason) => {
                rejected += 1;
                variants_seen.insert(format!("{reason:?}"));
            }
        }
    }
    // The sample is only meaningful if it actually crossed the admit/reject boundary AND
    // exercised every rejection variant — guard against a silently-degenerate generator.
    assert!(admitted > 0, "degenerate sample: no edge admitted");
    assert!(rejected > 0, "degenerate sample: no edge rejected");
    for variant in [
        "NoAnchor",
        "SignatureInvalid",
        "OutOfWindow",
        "Cyclic",
        "Broadening",
        "OverDepth",
    ] {
        assert!(
            variants_seen.contains(variant),
            "degenerate sample: rejection variant {variant} never exercised (saw {variants_seen:?})"
        );
    }
}
