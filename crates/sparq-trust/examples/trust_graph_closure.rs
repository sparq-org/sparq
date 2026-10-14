//! sq-d9lzx — runnable END-TO-END demo of the certification-edge trust-graph
//! closure (`derive_effective_rules`, the default-OFF `cert-graph` feature): *framework
//! certifies issuer → attenuated effective rule → admissible fact class*, plus the two
//! headline FAIL-CLOSED rejects an integrator/WG reviewer should see deny with their own
//! eyes (a broadening cert contributes NOTHING; an unanchored cert is inert).
//!
//! Run:  cargo run -p sparq-trust --example trust_graph_closure --features cert-graph
//!
//! It walks four steps, printing each:
//!   1. ANCHOR   — parse a Control-gated trust policy into the pod's direct anchor rule
//!      (the framework operator GOV, trusted for `schema:age` statements).
//!   2. DERIVE   — GOV signs a `trustx:Certification` for a downstream issuer, scoped to
//!      the *integer-typed* `age` sub-shape; the closure derives an ATTENUATED rule and
//!      the demo PROVES it is narrower than its anchor on every axis (selection targets
//!      ⊆, conformance constraints ⊇, freshness ≤, resource scope unchanged).
//!   3. REJECT: Broadening — a cert whose scope EXCEEDS the anchor ceiling (an extra
//!      `sh:targetSubjectsOf schema:email` target — the privilege-escalation shape)
//!      contributes NOTHING.
//!   4. REJECT: NoAnchor  — a perfectly-signed cert from a certifier the pod does NOT
//!      anchor is inert (deny-by-absence).
//!
//! HONESTY (read first): this is a CLEAR-PATH research prototype. It demonstrates the
//! fail-closed attenuation *logic* only — a checked signature plus scope/time narrowing —
//! and makes **no ZK, anonymity, unlinkability, or production-soundness claim**. The
//! signature primitive it shares with the ZK estate has no external accredited-
//! cryptographer sign-off (open gate `sq-qhy4`), and the certifier's own key binding is
//! operator-asserted (the D′ vector). Demo keys are seeded, NOT production entropy.
//!
//! 🤖 SPARQ agent — written by Claude Fable 5.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_trust::graph::{
    certification_message, derive_effective_rules, explain_edge, CertScope, Certification,
    EdgeRejection,
};
use sparq_trust::policy::{parse_policy, ControlGate, ShapeRef, TrustRule};
use sparq_trust::vocab::{self, desugar_for_predicate};
use sparq_zk::sig::{public_key_to_hex, PublicKey, SecretKey};
use std::collections::BTreeSet;

// ── demo cast ────────────────────────────────────────────────────────────────
/// The trust-framework operator (e.g. a DIATF-style register) the pod anchors directly.
const GOV: &str = "https://gov.example/framework";
/// The downstream credential issuer GOV certifies (the pod never anchors it directly).
const ISSUER: &str = "https://issuer.example/dvs";
/// A certifier NOBODY anchors — its perfectly-signed cert must be inert.
const ROGUE: &str = "https://rogue.example/self-appointed";
const AGE: &str = "https://schema.org/age";
const EMAIL: &str = "https://schema.org/email";
/// The pod resource scope the anchor covers.
const RES: &str = "https://pod.example/credentials/";
/// A fixed demo evaluation instant (Unix seconds) — deterministic output, no wall clock.
const NOW: i64 = 1_700_000_000;

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).expect("demo IRIs are valid")
}

// ── shape helpers (the crate's ONE statement-type idiom) ─────────────────────

/// The `trust:forPredicate P` desugaring as a [`ShapeRef`] — identical structure to what
/// [`parse_policy`] derives for the anchor, so containment is decidable.
fn predicate_shape(p: &str) -> ShapeRef {
    let (root, triples) = desugar_for_predicate(&iri(p));
    ShapeRef {
        root: match root {
            NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b),
            NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n),
        },
        triples,
    }
}

/// [`predicate_shape`] PLUS an `sh:datatype xsd:integer` constraint on the property — a
/// provable NARROWING (same targets, strictly more conformance constraints).
fn narrower_shape(p: &str) -> ShapeRef {
    let mut s = predicate_shape(p);
    let prop = s
        .triples
        .iter()
        .find(|t| t.predicate.as_str() == vocab::SH_PROPERTY)
        .map(|t| t.object.clone())
        .expect("desugared shape has an sh:property node");
    if let Term::BlankNode(prop_bn) = prop {
        s.triples.push(Triple::new(
            prop_bn,
            iri("http://www.w3.org/ns/shacl#datatype"),
            iri("http://www.w3.org/2001/XMLSchema#integer"),
        ));
    }
    s
}

/// [`predicate_shape`] PLUS an extra `sh:targetSubjectsOf extra_target` at the root — a
/// BROADENING: SHACL selects focus nodes by the UNION of targets, so the extra target
/// predicate WIDENS the admitted set beyond the anchor ceiling (the escalation shape).
fn broadening_shape(p: &str, extra_target: &str) -> ShapeRef {
    let mut s = predicate_shape(p);
    if let Term::BlankNode(root) = s.root.clone() {
        s.triples.push(Triple::new(
            root,
            iri(vocab::SH_TARGET_SUBJECTS_OF),
            iri(extra_target),
        ));
    }
    s
}

/// The `sh:targetSubjectsOf` object IRIs a shape declares at its root — the SELECTION
/// (target-predicate) set the admission gate picks focus nodes by. Used to PROVE the
/// derived rule's targets are a subset of the anchor's.
fn target_set(shape: &ShapeRef) -> BTreeSet<String> {
    let root_id = node_id(&shape.root);
    shape
        .triples
        .iter()
        .filter(|t| {
            t.predicate.as_str() == vocab::SH_TARGET_SUBJECTS_OF
                && subject_id(&t.subject) == root_id
        })
        .filter_map(|t| match &t.object {
            Term::NamedNode(n) => Some(n.as_str().to_string()),
            _ => None,
        })
        .collect()
}

fn node_id(t: &Term) -> String {
    match t {
        Term::BlankNode(b) => format!("B{}", b.as_str()),
        Term::NamedNode(n) => format!("I{}", n.as_str()),
        other => format!("{other:?}"),
    }
}

fn subject_id(s: &NamedOrBlankNode) -> String {
    match s {
        NamedOrBlankNode::BlankNode(b) => format!("B{}", b.as_str()),
        NamedOrBlankNode::NamedNode(n) => format!("I{}", n.as_str()),
    }
}

// ── policy + certification builders ──────────────────────────────────────────

/// The Control-gated trust policy: GOV is a `trust:Source` trusted for `schema:age`
/// statements over `RES`, fresh within 30 days — the claim-level `trustsSourceFor` form.
fn control_gated_policy(gov_key_hex: &str) -> Vec<Triple> {
    let s = || NamedOrBlankNode::NamedNode(iri(GOV));
    vec![
        Triple::new(
            s(),
            iri(vocab::RDF_TYPE),
            Term::NamedNode(iri(vocab::SOURCE_CLASS)),
        ),
        Triple::new(
            s(),
            iri(vocab::TRUSTS_SOURCE_FOR),
            Term::NamedNode(iri(AGE)),
        ),
        Triple::new(
            s(),
            iri(vocab::ISSUER_KEY),
            Term::Literal(Literal::new_simple_literal(gov_key_hex)),
        ),
        Triple::new(s(), iri(vocab::SCOPE), Term::NamedNode(iri(RES))),
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

/// A certification EDGE signed by `certifier_sk` over the domain-separated
/// [`certification_message`] — the checked-signature discipline (a graph merely
/// *claiming* `trustx:certifies` proves nothing).
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

fn print_rule(label: &str, r: &TrustRule) {
    println!(
        "  {label}: source=<{}>  scope=<{}>  fresh_within={}s  shape: {} triple(s), targets {:?}",
        r.source.as_str(),
        r.scope.as_str(),
        r.fresh_within_secs,
        r.shape.triples.len(),
        target_set(&r.shape)
    );
}

fn main() {
    println!("=== trust_graph_closure — certification-edge closure demo (cert-graph) ===");
    println!(
        "(clear-path research prototype; NO ZK/privacy/production-soundness claim — sq-qhy4)\n"
    );

    // Demo-only seeded keys (deterministic output; NOT production key generation).
    let gov_sk = SecretKey::from_seed(1);
    let issuer_key = SecretKey::from_seed(2).public_key();
    let rogue_sk = SecretKey::from_seed(9);

    // ── 1. ANCHOR — the Control-gated direct rule ────────────────────────────
    // The pod's trust ROOT: parsed from a Control-gated policy document (the gate token
    // asserts the channel; in production the WAC/ACP materialiser decides "behind Control").
    let policy = control_gated_policy(&public_key_to_hex(&gov_sk.public_key()));
    let anchors = parse_policy(&policy, ControlGate::assert_control_gated())
        .expect("the demo policy is complete");
    println!("STEP 1 — ANCHOR: the pod Control-gates ONE direct rule (the framework operator):");
    print_rule("anchor", &anchors[0]);
    println!();

    // ── 2. DERIVE — framework certifies issuer → attenuated effective rule ──
    // GOV signs a certification for ISSUER, scoped to the *integer-typed* `age` sub-shape
    // (a narrowing of GOV's own `age` anchor), valid for one day around NOW.
    let narrowing_cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        issuer_key,
        CertScope::Shape(narrower_shape(AGE)),
        NOW - 3_600,
        NOW + 86_400,
    );
    let effective = derive_effective_rules(&anchors, std::slice::from_ref(&narrowing_cert), NOW, 1);
    assert_eq!(effective.len(), 2, "anchor + exactly one derived rule");
    let (anchor, derived) = (&effective[0], &effective[1]);
    println!(
        "STEP 2 — DERIVE: GOV signs `trustx:certifies <{ISSUER}>` scoped to integer-typed `age`;"
    );
    println!("the closure appends ONE derived (attenuated) effective rule:");
    print_rule("derived (attenuated) rule", derived);

    // PROVE the attenuation-only invariant, axis by axis (a false claim here panics):
    assert_eq!(derived.source.as_str(), ISSUER);
    assert_eq!(
        public_key_to_hex(&derived.issuer_key),
        public_key_to_hex(&issuer_key),
        "the derived rule binds the CERTIFIED issuer's key (signed into the edge)"
    );
    let (anchor_targets, derived_targets) = (target_set(&anchor.shape), target_set(&derived.shape));
    assert!(
        derived_targets.is_subset(&anchor_targets),
        "selection targets may only SHRINK"
    );
    assert!(
        derived.shape.triples.len() > anchor.shape.triples.len(),
        "conformance constraints may only GROW (the extra sh:datatype)"
    );
    assert!(
        derived.fresh_within_secs < anchor.fresh_within_secs,
        "freshness capped by the certification window (< the anchor's 30 days)"
    );
    assert_eq!(
        derived.scope.as_str(),
        anchor.scope.as_str(),
        "resource scope is the certifier ceiling, unchanged"
    );
    println!("  PROVEN narrower than its anchor on every axis:");
    println!(
        "    targets {derived_targets:?} ⊆ anchor {anchor_targets:?}; constraints {} > {} triples \
(+ sh:datatype xsd:integer); fresh {}s < {}s; resource scope unchanged",
        derived.shape.triples.len(),
        anchor.shape.triples.len(),
        derived.fresh_within_secs,
        anchor.fresh_within_secs
    );
    println!(
        "  → statements ISSUER signs that match the DERIVED shape inside <{RES}> are now\n    \
admissible; everything outside it stays denied (the gate itself is UNCHANGED).\n"
    );

    // ── 3. REJECT: Broadening — scope exceeds the anchor ceiling ────────────
    // GOV (or a compromised register) certifies ISSUER for `age` PLUS an extra
    // `sh:targetSubjectsOf schema:email` target. SHACL selects by the UNION of targets, so
    // this WIDENS beyond GOV's own authority — the closure must derive NOTHING from it.
    let broadening_cert = signed_cert(
        GOV,
        &gov_sk,
        ISSUER,
        issuer_key,
        CertScope::Shape(broadening_shape(AGE, EMAIL)),
        NOW - 3_600,
        NOW + 86_400,
    );
    let verdict = explain_edge(&anchors, &broadening_cert, NOW, 1);
    assert_eq!(verdict.unwrap_err(), EdgeRejection::Broadening);
    let out = derive_effective_rules(&anchors, std::slice::from_ref(&broadening_cert), NOW, 1);
    assert_eq!(
        out.len(),
        anchors.len(),
        "a broadening edge contributes NOTHING"
    );
    println!("STEP 3 — REJECT: Broadening — a validly-signed cert whose scope EXCEEDS the anchor");
    println!("ceiling (extra `sh:targetSubjectsOf <{EMAIL}>` target) is denied:");
    println!("  explain_edge → Err(Broadening); derive_effective_rules output = anchors only");
    println!(
        "  ({} rule(s) in, {} out — the edge contributed NOTHING, fail-closed).\n",
        anchors.len(),
        out.len()
    );

    // ── 4. REJECT: NoAnchor — an unanchored certifier is inert ──────────────
    // ROGUE self-appoints as a framework and (validly!) signs a cert for ISSUER. The pod
    // anchors no rule for ROGUE, so the edge has no authority to narrow: deny-by-absence.
    let unanchored_cert = signed_cert(
        ROGUE,
        &rogue_sk,
        ISSUER,
        issuer_key,
        CertScope::AnyService,
        NOW - 3_600,
        NOW + 86_400,
    );
    let verdict = explain_edge(&anchors, &unanchored_cert, NOW, 1);
    assert_eq!(verdict.unwrap_err(), EdgeRejection::NoAnchor);
    let out = derive_effective_rules(&anchors, std::slice::from_ref(&unanchored_cert), NOW, 1);
    assert_eq!(out.len(), anchors.len(), "an unanchored edge is inert");
    println!("STEP 4 — REJECT: NoAnchor — a perfectly-signed cert from <{ROGUE}>,");
    println!("which the pod does NOT anchor, is inert (deny-by-absence):");
    println!("  explain_edge → Err(NoAnchor); derive_effective_rules output = anchors only");
    println!(
        "  ({} rule(s) in, {} out — no anchor, no authority, NOTHING derived).\n",
        anchors.len(),
        out.len()
    );

    println!("=== done: attenuation-only closure demonstrated; both rejects fail-closed ===");
}
