//! Parse-validation of the published machine-readable certification-scope vocabulary
//! `ontologies/trust/trust-framework.ttl` (sq-6syab.2; epic sq-6syab / issue #1592;
//! design record `research/trust-expression-spec.md` §3.4 / D4). The
//! `framework_vocab::tests` unit tests pin the IRIs against the Rust constants (the
//! *string* guard); this test is the *syntactic + structural* guard — the Turtle a
//! working group reads MUST stay valid Turtle, declare the `trustx:` terms, `owl:imports`
//! the `trust:` vocabulary it extends, and `rdfs:seeAlso` the vendored `sec-req:`
//! eIDAS/UK-DVS individuals rather than duplicating them (the design's D5).
//!
//! Behind the default-OFF `framework-vocab` feature, exactly like the module it guards.
//!
//! [FABLE-5] sq-6syab.2 (issue #1592). 🤖 SPARQ agent — trust-expression
//! certification-scope layer.
#![cfg(feature = "framework-vocab")]

use oxrdf::{NamedOrBlankNode, Term};
use oxttl::TurtleParser;

const TTL: &str = include_str!("../ontologies/trust/trust-framework.ttl");
/// The `trust:` core vocabulary this layer EXTENDS under the SAME base IRI — read here to
/// assert no local name collides (issue #3801).
const TRUST_TTL: &str = include_str!("../ontologies/trust/trust.ttl");

const TRUSTX_NS: &str = "https://sparq.dev/ns/trust#";
const SEC_REQ_NS: &str = "https://w3id.org/zkp-sparql/sec-req#";
const RDFS_SEE_ALSO: &str = "http://www.w3.org/2000/01/rdf-schema#seeAlso";
const RDFS_DOMAIN: &str = "http://www.w3.org/2000/01/rdf-schema#domain";
const OWL_IMPORTS: &str = "http://www.w3.org/2002/07/owl#imports";
const OWL_UNION_OF: &str = "http://www.w3.org/2002/07/owl#unionOf";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";

/// The published vocabulary is valid Turtle (parses without error) and is non-empty.
#[test]
fn trust_framework_ttl_is_valid_turtle() {
    let mut triples = 0usize;
    for result in TurtleParser::new().for_reader(TTL.as_bytes()) {
        result.expect("trust-framework.ttl must be valid Turtle");
        triples += 1;
    }
    assert!(
        triples > 0,
        "trust-framework.ttl parsed to zero triples — the published vocabulary is empty",
    );
}

/// Every `trustx:` term the layer mints is declared in the Turtle as a subject — a
/// coarse presence check independent of the unit-test constant pinning, so a refactor of
/// `framework_vocab.rs` cannot silently drop a published term from the gate.
#[test]
fn trust_framework_ttl_declares_all_trustx_terms() {
    let expected = [
        // trust requirements
        "TrustRequirements",
        "question",
        "trustsIssuer",
        "trustsFramework",
        "requiresScopeConformance",
        "requiresValidStatusAt",
        "methodPolicy",
        // certification-scope layer
        "Framework",
        "Certification",
        "certifies",
        "underFramework",
        "certificationScope",
        "validFrom",
        "validUntil",
        "AnyServiceScope",
        // status attestation
        "StatusAttestation",
        "coveredBy",
        // framework individuals
        "eIDAS2",
        "DIATF",
    ];

    let mut subjects = std::collections::HashSet::new();
    for result in TurtleParser::new().for_reader(TTL.as_bytes()) {
        let t = result.expect("valid Turtle");
        if let NamedOrBlankNode::NamedNode(s) = t.subject {
            subjects.insert(s.into_string());
        }
    }
    for local in expected {
        let iri = format!("{TRUSTX_NS}{local}");
        assert!(
            subjects.contains(&iri),
            "trust-framework.ttl does not declare `trustx:{local}` ({iri}) as a subject",
        );
    }
}

/// The framework individuals `rdfs:seeAlso` the vendored `sec-req:` eIDAS/UK-DVS
/// individuals (no duplication — design D5), and the ontology node `owl:imports` the
/// `trust:` vocabulary it extends. Both are verified against the PARSED graph, not a
/// substring — the real invariant the design pins.
#[test]
fn extends_trust_and_references_vendored_sec_req_in_the_parsed_graph() {
    let mut see_also_objects: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut import_objects: std::collections::HashSet<String> = std::collections::HashSet::new();
    // eIDAS2 → sec-req:Eidas20, DIATF → sec-req:UkDvs, both by subject+object.
    let mut eidas_sees_vendored = false;
    let mut diatf_sees_vendored = false;

    for result in TurtleParser::new().for_reader(TTL.as_bytes()) {
        let t = result.expect("valid Turtle");
        let pred = t.predicate.as_str().to_owned();
        if pred == RDFS_SEE_ALSO {
            if let Term::NamedNode(o) = &t.object {
                see_also_objects.insert(o.as_str().to_owned());
                if let NamedOrBlankNode::NamedNode(s) = &t.subject {
                    if s.as_str() == format!("{TRUSTX_NS}eIDAS2")
                        && o.as_str() == format!("{SEC_REQ_NS}Eidas20")
                    {
                        eidas_sees_vendored = true;
                    }
                    if s.as_str() == format!("{TRUSTX_NS}DIATF")
                        && o.as_str() == format!("{SEC_REQ_NS}UkDvs")
                    {
                        diatf_sees_vendored = true;
                    }
                }
            }
        }
        if pred == OWL_IMPORTS {
            if let Term::NamedNode(o) = &t.object {
                import_objects.insert(o.as_str().to_owned());
            }
        }
    }

    assert!(
        eidas_sees_vendored,
        "trustx:eIDAS2 must rdfs:seeAlso the vendored sec-req:Eidas20 (no duplication — D5)",
    );
    assert!(
        diatf_sees_vendored,
        "trustx:DIATF must rdfs:seeAlso the vendored sec-req:UkDvs (no duplication — D5)",
    );
    assert!(
        import_objects.contains("https://sparq.dev/ns/trust"),
        "the framework ontology node must owl:imports the trust: vocabulary it extends",
    );
}

/// The validity-window properties are carried by BOTH `trustx:Certification` and
/// `trustx:StatusAttestation`, so their `rdfs:domain` must not name `Certification`
/// alone: under RDFS entailment `?a trustx:validFrom ?t` would then type every status
/// attestation as a `trustx:Certification` — a class this model expects to carry
/// `trustx:certifies`, `trustx:underFramework` and `trustx:certificationScope`, none of
/// which a status attestation has (issue #3801). Verified on the PARSED graph — the
/// domain must be the union class containing both.
#[test]
fn validity_window_domain_admits_status_attestations() {
    let triples: Vec<_> = TurtleParser::new()
        .for_reader(TTL.as_bytes())
        .map(|r| r.expect("valid Turtle"))
        .collect();

    // Blank-node subject → the classes its `owl:unionOf` list enumerates.
    let union_members = |head: &NamedOrBlankNode| -> std::collections::HashSet<String> {
        let mut members = std::collections::HashSet::new();
        // Follow `head owl:unionOf ?list`, then the rdf:first/rdf:rest chain.
        let mut cursor = triples.iter().find_map(|t| {
            (&t.subject == head && t.predicate.as_str() == OWL_UNION_OF)
                .then(|| NamedOrBlankNode::try_from(t.object.clone()).ok())
                .flatten()
        });
        while let Some(node) = cursor {
            for t in &triples {
                if t.subject != node {
                    continue;
                }
                if t.predicate.as_str() == RDF_FIRST {
                    if let Term::NamedNode(o) = &t.object {
                        members.insert(o.as_str().to_owned());
                    }
                }
            }
            cursor = triples.iter().find_map(|t| {
                (t.subject == node && t.predicate.as_str() == RDF_REST)
                    .then(|| NamedOrBlankNode::try_from(t.object.clone()).ok())
                    .flatten()
            });
        }
        members
    };

    for local in ["validFrom", "validUntil"] {
        let prop = format!("{TRUSTX_NS}{local}");
        let domains: Vec<_> = triples
            .iter()
            .filter(|t| {
                matches!(&t.subject, NamedOrBlankNode::NamedNode(s) if s.as_str() == prop)
                    && t.predicate.as_str() == RDFS_DOMAIN
            })
            .map(|t| t.object.clone())
            .collect();
        assert_eq!(
            domains.len(),
            1,
            "trustx:{local} must declare exactly one rdfs:domain (found {})",
            domains.len(),
        );
        let head = NamedOrBlankNode::try_from(domains[0].clone()).unwrap_or_else(|_| {
            panic!("trustx:{local}'s rdfs:domain must be a class node, not a literal")
        });
        let members = union_members(&head);
        assert!(
            !members.is_empty(),
            "trustx:{local}'s rdfs:domain must be an owl:unionOf class node (found the bare \
             domain {head}) — a single named domain class excludes the other window-carrying \
             class (issue #3801)",
        );
        for required in ["Certification", "StatusAttestation"] {
            let iri = format!("{TRUSTX_NS}{required}");
            assert!(
                members.contains(&iri),
                "trustx:{local}'s rdfs:domain must be a union including trustx:{required} \
                 ({iri}) — a narrower domain entails that every status attestation is a \
                 certification (issue #3801); found {members:?}",
            );
        }
    }
}

/// `trustx:` and `trust:` share ONE base IRI, so a repeated local name is the SAME
/// property carrying two `rdfs:domain` axioms — not a homonym. No term declared in this
/// extension may already be declared by `trust.ttl` (issue #3801: `trustx:scope` WAS
/// `trust:scope`, with domains `trustx:Certification` and `trust:TrustRule` at once).
#[test]
fn no_trustx_term_collides_with_a_trust_core_term() {
    // The IRIs each document DECLARES (gives an `rdf:type` to) in the shared base.
    let declared = |ttl: &str| -> std::collections::HashSet<String> {
        TurtleParser::new()
            .for_reader(ttl.as_bytes())
            .map(|r| r.expect("valid Turtle"))
            .filter(|t| t.predicate.as_str() == RDF_TYPE)
            .filter_map(|t| match t.subject {
                NamedOrBlankNode::NamedNode(s) if s.as_str().starts_with(TRUSTX_NS) => {
                    Some(s.into_string())
                }
                _ => None,
            })
            .collect()
    };

    let core = declared(TRUST_TTL);
    let ext = declared(TTL);
    assert!(
        core.contains(&format!("{TRUSTX_NS}scope")),
        "sanity: trust.ttl declares trust:scope",
    );
    let collisions: Vec<_> = ext.intersection(&core).cloned().collect();
    assert!(
        collisions.is_empty(),
        "trust-framework.ttl redeclares {} term(s) already declared by trust.ttl under the \
         SAME base IRI — same IRI, conflicting rdfs:domain axioms (issue #3801): {collisions:?}",
        collisions.len(),
    );
}

/// No vendored `sec-req:` individual is REDECLARED (given an `rdf:type`) in this
/// extension file — they are referenced only. Verified on the parsed graph: no triple
/// has a `sec-req:` subject with predicate `rdf:type` (the D5 no-duplication invariant).
#[test]
fn does_not_redeclare_vendored_sec_req_individuals() {
    for result in TurtleParser::new().for_reader(TTL.as_bytes()) {
        let t = result.expect("valid Turtle");
        if let NamedOrBlankNode::NamedNode(s) = &t.subject {
            if s.as_str().starts_with(SEC_REQ_NS) {
                assert_ne!(
                    t.predicate.as_str(),
                    RDF_TYPE,
                    "trust-framework.ttl redeclares (types) the vendored `{}` — the D5 \
                     no-duplication rule forbids editing/duplicating vendored sec-req:",
                    s.as_str(),
                );
            }
        }
    }
}
