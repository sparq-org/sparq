//! # `trust:` vocabulary — the minimal orthogonal ontology (10-term core)
//!
//! The `trust:` IRIs of the trust-graph design (`research/solid-trust-graph-authz-design.md`
//! §2.3.1) as constants, plus the **one** desugaring the design admits as sugar:
//! `trust:forPredicate P` → a single-predicate `trust:forShape` SHACL node-shape
//! (§2.3.3). `forPredicate` is **NOT a primitive** — it is defined wholly by the rewrite
//! below, so a working group standardises only `forShape`.
//!
//! All `trust:` IRIs are **NON-STANDARD, invented for this proposal** (a placeholder
//! namespace to make the semantics concrete); a WG would rename/rehome them.
//!
//! The canonical, machine-readable form of this vocabulary is
//! [`ontologies/trust/trust.ttl`](https://github.com/sparq-org/sparq/blob/main/crates/sparq-trust/ontologies/trust/trust.ttl)
//! (Turtle, with `rdfs:label`/`rdfs:comment`/`rdfs:domain`/`rdfs:range`), and its
//! two-stratum semantics — admission/derivation, the degenerate-`.acl` equivalence,
//! and the strict-additivity property (G6) — are pinned in the companion
//! `ontologies/trust/SEMANTICS.md`. The `ttl_pins_match_rust_constants` test below
//! keeps the published Turtle and these Rust constants from drifting.
//!
//! [OPUS-4.8] sq-pfae PoC (issue #940); vocab/semantics note pinned in sq-pfae.2.
//! Flag for re-review when Fable returns. 🤖 SPARQ agent — trust-graph authorisation.

use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Term, Triple};

/// The `trust:` namespace base (non-standard; a WG would rehome it).
pub const TRUST_NS: &str = "https://sparq.dev/ns/trust#";

/// The SHACL namespace (the `forShape` primitive is a `sh:NodeShape`; the
/// `forPredicate` sugar desugars into the genuine `sh:targetSubjectsOf` /
/// `sh:path` / `sh:minCount` idiom `sparq-shacl` already evaluates).
pub const SH_NS: &str = "http://www.w3.org/ns/shacl#";

/// `rdf:type`.
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

// --- the ten-term irreducible core (§2.3.1) --------------------------------

/// `trust:TrustPolicy` — the policy container: a set of admission rules scoping a
/// resource/server.
pub const TRUST_POLICY: &str = "https://sparq.dev/ns/trust#TrustPolicy";
/// `trust:TrustRule` — one reified admission rule grouping a
/// source/type/scope/freshness condition.
pub const TRUST_RULE: &str = "https://sparq.dev/ns/trust#TrustRule";
/// `trust:trustsSourceFor` — **the core relation:** admit statements of this type
/// (shape) from this source.
pub const TRUSTS_SOURCE_FOR: &str = "https://sparq.dev/ns/trust#trustsSourceFor";
/// `trust:source` — the attesting source a trust rule names.
pub const SOURCE: &str = "https://sparq.dev/ns/trust#source";
/// `trust:forShape` — statement-type as a SHACL shape (the *one* statement-type
/// primitive).
pub const FOR_SHAPE: &str = "https://sparq.dev/ns/trust#forShape";
/// `trust:Source` — an attesting authority, identified by an issuer key / DID.
pub const SOURCE_CLASS: &str = "https://sparq.dev/ns/trust#Source";
/// `trust:issuerKey` — verification key the source signs with (aligns with
/// `zk:issuerKey`).
pub const ISSUER_KEY: &str = "https://sparq.dev/ns/trust#issuerKey";
/// `trust:issuerDid` — the source's DID, resolved to its verifying key by a pluggable
/// DID resolver (`did:key` / `did:web`; the `crate::did` module, opt-in `did` feature).
/// A rule may name its issuer by `trust:issuerDid` INSTEAD of the operator-asserted
/// `trust:issuerKey` hex — the `sq-pfae.3` issuer-key-binding term that narrows forgery
/// vector D′.
pub const ISSUER_DID: &str = "https://sparq.dev/ns/trust#issuerDid";
/// `trust:scope` — where the trust rule applies (server-wide vs per-`.acr`).
pub const SCOPE: &str = "https://sparq.dev/ns/trust#scope";
/// `trust:freshWithin` — maximum staleness admitted (consulted against
/// `Session.now`).
pub const FRESH_WITHIN: &str = "https://sparq.dev/ns/trust#freshWithin";
/// `trust:admitted` — marks a fact that passed admission (analogous to the
/// `solidx:` internal vocab; the stratum-boundary marker).
pub const ADMITTED: &str = "https://sparq.dev/ns/trust#admitted";

// --- delegation principal kinds (§4.2 on-behalf-of; sq-pfae.6) -------------
//
// "Human vs AI is just an attested attribute / caveat on the delegate" (design §4.2).
// These classify a delegation principal in the PROV-O audit the `delegation_prov`
// module emits (opt-in `delegation-prov` feature). They are an ATTESTED ATTRIBUTE on
// the delegate — they confer NO authority of their own (authority is the attenuated
// capability the signed chain carries); they only let a policy / audit reader tell a
// human principal from an AI agent acting on a human's behalf.

/// `trust:HumanPrincipal` — a delegation principal that is a human (the human root /
/// any human delegate). An attested attribute on the principal, not an authority.
pub const HUMAN_PRINCIPAL: &str = "https://sparq.dev/ns/trust#HumanPrincipal";
/// `trust:AiAgent` — a delegation principal that is an AI agent: a **distinct
/// principal** (its own DID/key) holding an **attenuated child** of its human
/// principal's authority (§4.2). An attested attribute on the delegate, not an
/// authority — the AI agent's authority is the attenuated capability the signed chain
/// carries, never this type.
pub const AI_AGENT: &str = "https://sparq.dev/ns/trust#AiAgent";

// --- the one convenience (sugar, NOT a primitive — §2.3.3) -----------------

/// `trust:forPredicate` — **SUGAR** for a single-predicate `trust:forShape`. It is
/// kept for ergonomics but defined *wholly* by [`desugar_for_predicate`]; it adds
/// no expressivity `forShape` lacks.
pub const FOR_PREDICATE: &str = "https://sparq.dev/ns/trust#forPredicate";

// --- SHACL terms used by the desugaring ------------------------------------

/// `sh:NodeShape`.
pub const SH_NODE_SHAPE: &str = "http://www.w3.org/ns/shacl#NodeShape";
/// `sh:targetSubjectsOf` — target = the subjects asserting a predicate.
pub const SH_TARGET_SUBJECTS_OF: &str = "http://www.w3.org/ns/shacl#targetSubjectsOf";
/// `sh:property`.
pub const SH_PROPERTY: &str = "http://www.w3.org/ns/shacl#property";
/// `sh:path`.
pub const SH_PATH: &str = "http://www.w3.org/ns/shacl#path";
/// `sh:minCount`.
pub const SH_MIN_COUNT: &str = "http://www.w3.org/ns/shacl#minCount";

/// `xsd:integer` (the `sh:minCount 1` literal datatype).
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

fn iri(s: &str) -> NamedNode {
    // Every constant above is a syntactically valid IRI, so `new_unchecked` is safe.
    NamedNode::new_unchecked(s)
}

/// Desugar `trust:forPredicate P` into the **normative primitive** — a
/// single-predicate `trust:forShape` SHACL node-shape (§2.3.3):
///
/// ```n3
/// [] a sh:NodeShape ;
///    sh:targetSubjectsOf P ;
///    sh:property [ sh:path P ; sh:minCount 1 ] .
/// ```
///
/// Returns `(shape_node, triples)`: the blank node naming the shape and the triples
/// that define it (fresh blank nodes, so two desugarings never collide). The
/// rewrite is **lossless in the predicate-only direction** (every `forPredicate P`
/// assertion maps to exactly this shape) and is the SOLE redundancy the de-dup found
/// — see CLAIM 1 of the design's §7.6.
///
/// `sh:targetSubjectsOf` + a `sh:minCount 1` property shape is the genuine, shipped
/// SHACL idiom for "constrain to a single predicate": there is **no**
/// `sh:targetPredicate` term in SHACL, so the desugaring uses the real target/path
/// mechanism `sparq-shacl` already evaluates.
pub fn desugar_for_predicate(predicate: &NamedNode) -> (NamedOrBlankNode, Vec<Triple>) {
    let shape = BlankNode::default();
    let prop = BlankNode::default();
    let shape_node = NamedOrBlankNode::BlankNode(shape.clone());
    let prop_node = NamedOrBlankNode::BlankNode(prop.clone());
    let one = Literal::new_typed_literal("1", iri(XSD_INTEGER));
    let triples = vec![
        Triple::new(
            shape_node.clone(),
            iri(RDF_TYPE),
            Term::NamedNode(iri(SH_NODE_SHAPE)),
        ),
        Triple::new(
            shape_node.clone(),
            iri(SH_TARGET_SUBJECTS_OF),
            Term::NamedNode(predicate.clone()),
        ),
        Triple::new(
            shape_node.clone(),
            iri(SH_PROPERTY),
            Term::BlankNode(prop.clone()),
        ),
        Triple::new(
            prop_node.clone(),
            iri(SH_PATH),
            Term::NamedNode(predicate.clone()),
        ),
        Triple::new(prop_node, iri(SH_MIN_COUNT), Term::Literal(one)),
    ];
    (shape_node, triples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desugar_produces_the_normative_single_predicate_shape() {
        let p = NamedNode::new("http://schema.org/age").unwrap();
        let (shape, triples) = desugar_for_predicate(&p);
        // 5 triples: type + targetSubjectsOf + property + path + minCount.
        assert_eq!(triples.len(), 5);
        // The shape node is a fresh blank node (lossless, collision-free).
        assert!(matches!(shape, NamedOrBlankNode::BlankNode(_)));
        // targetSubjectsOf names exactly the desugared predicate.
        assert!(triples.iter().any(
            |t| matches!(&t.predicate, p2 if p2.as_str() == SH_TARGET_SUBJECTS_OF)
                && matches!(&t.object, Term::NamedNode(o) if o.as_str() == p.as_str())
        ));
        // sh:path names exactly the desugared predicate too.
        assert!(triples
            .iter()
            .any(|t| matches!(&t.predicate, p2 if p2.as_str() == SH_PATH)
                && matches!(&t.object, Term::NamedNode(o) if o.as_str() == p.as_str())));
        // sh:minCount 1 is present.
        assert!(triples.iter().any(
            |t| matches!(&t.predicate, p2 if p2.as_str() == SH_MIN_COUNT)
                && matches!(&t.object, Term::Literal(l) if l.value() == "1")
        ));
    }

    #[test]
    fn two_desugarings_use_distinct_fresh_shape_nodes() {
        let p = NamedNode::new("http://schema.org/age").unwrap();
        let (a, _) = desugar_for_predicate(&p);
        let (b, _) = desugar_for_predicate(&p);
        assert_ne!(a, b, "fresh blank nodes — desugarings never collide");
    }

    /// No-drift guard for sq-pfae.2: every `trust:` IRI these Rust constants name
    /// MUST appear (as a subject) in the published machine-readable vocabulary
    /// `ontologies/trust/trust.ttl`, and vice-versa. If a term is renamed/added in
    /// one and not the other this test fails — the published Turtle a working group
    /// reads and the constants the admission gate uses cannot silently diverge.
    #[test]
    fn ttl_pins_match_rust_constants() {
        // The published Turtle vocabulary (Doc + vocab artifact of sq-pfae.2).
        const TTL: &str = include_str!("../ontologies/trust/trust.ttl");

        // The full set of `trust:` IRIs the Rust core + the one sugar name.
        let constants: &[&str] = &[
            TRUST_POLICY,
            TRUST_RULE,
            TRUSTS_SOURCE_FOR,
            SOURCE,
            FOR_SHAPE,
            SOURCE_CLASS,
            ISSUER_KEY,
            ISSUER_DID,
            SCOPE,
            FRESH_WITHIN,
            ADMITTED,
            FOR_PREDICATE,
        ];

        // Every constant must be declared in the Turtle (as `trust:LocalName a …`).
        for &iri in constants {
            let local = iri
                .strip_prefix(TRUST_NS)
                .expect("every constant is in the trust: namespace");
            let decl = format!("trust:{local} a ");
            assert!(
                TTL.contains(&decl),
                "trust.ttl is missing a declaration for `trust:{local}` ({iri}) — \
                 the published vocabulary drifted from the Rust constants (sq-pfae.2)",
            );
        }

        // And every `trust:LocalName a` declaration in the Turtle must be backed by a
        // Rust constant — no published term without a constant. Scan declaration lines.
        let declared_locals: Vec<&str> = TTL
            .lines()
            .filter_map(|line| {
                let t = line.trim_start();
                let rest = t.strip_prefix("trust:")?;
                // `trust: a owl:Ontology` is the ontology-node line (bare namespace IRI):
                // the char right after the colon is whitespace, so it has no local name —
                // it is metadata, not a term. Skip it.
                if rest.starts_with(char::is_whitespace) {
                    return None;
                }
                // A term declaration line looks like `trust:Foo a …`.
                let name = rest.split_whitespace().next()?;
                if rest[name.len()..].trim_start().starts_with("a ") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        assert!(
            !declared_locals.is_empty(),
            "no trust: term declarations found in trust.ttl — parser/format drift",
        );
        for local in declared_locals {
            let full = format!("{TRUST_NS}{local}");
            assert!(
                constants.contains(&full.as_str()),
                "trust.ttl declares `trust:{local}` but no Rust constant names it — \
                 add the constant or remove the term (sq-pfae.2)",
            );
        }
    }
}
