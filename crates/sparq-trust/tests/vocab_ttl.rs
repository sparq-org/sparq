//! Parse-validation of the published machine-readable vocabulary
//! `ontologies/trust/trust.ttl` (sq-pfae.2, extended for the `trust:issuerDid`
//! issuer-key-binding term in sq-pfae.3). The Turtle a working group reads MUST
//! stay valid Turtle and MUST declare the core terms + the one sugar term. The
//! `vocab::tests::ttl_pins_match_rust_constants` unit test additionally pins the IRIs
//! against the Rust constants; this test is the *syntactic* guard.
//!
//! sq-pfae.2 (issue #940). 🤖 SPARQ agent — trust-graph authorisation.

use oxttl::TurtleParser;

const TTL: &str = include_str!("../ontologies/trust/trust.ttl");

/// The published vocabulary is valid Turtle (it parses without error) and is
/// non-empty.
#[test]
fn trust_ttl_is_valid_turtle() {
    let mut triples = 0usize;
    for result in TurtleParser::new().for_reader(TTL.as_bytes()) {
        result.expect("trust.ttl must be valid Turtle");
        triples += 1;
    }
    assert!(
        triples > 0,
        "trust.ttl parsed to zero triples — the published vocabulary is empty",
    );
}

/// Every term IRI (ten core + the `trust:issuerDid` binding term + the one sugar) is
/// declared in the Turtle as a subject. This is a coarse presence check independent of
/// the unit-test constant pinning, so a refactor of `vocab.rs` cannot silently drop a
/// published term from the gate.
#[test]
fn trust_ttl_declares_all_core_terms() {
    use oxrdf::NamedOrBlankNode;

    const NS: &str = "https://sparq.dev/ns/trust#";
    let expected = [
        "TrustPolicy",
        "TrustRule",
        "Source",
        "trustsSourceFor",
        "source",
        "forShape",
        "issuerKey",
        "issuerDid",
        "scope",
        "freshWithin",
        "admitted",
        "forPredicate",
    ];

    let mut subjects = std::collections::HashSet::new();
    for result in TurtleParser::new().for_reader(TTL.as_bytes()) {
        let t = result.expect("valid Turtle");
        if let NamedOrBlankNode::NamedNode(s) = t.subject {
            subjects.insert(s.into_string());
        }
    }

    for local in expected {
        let iri = format!("{NS}{local}");
        assert!(
            subjects.contains(&iri),
            "trust.ttl does not declare `trust:{local}` ({iri}) as a subject",
        );
    }
}
