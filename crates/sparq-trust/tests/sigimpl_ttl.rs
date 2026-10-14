//! Parse and subject-presence validation for the sparq-authored `sig-impl:`
//! extension graph.
//!
//! Issue #2832, PR #4198 review round 2.
#![cfg(feature = "secprop-vocab")]

use oxrdf::NamedOrBlankNode;
use oxttl::TurtleParser;

const TTL: &str = include_str!("../ontologies/zkp-sparql/sigimpl-ext.ttl");
const SIG_IMPL: &str = "https://w3id.org/zkp-sparql/sig-impl#";

#[test]
fn sigimpl_ext_ttl_is_valid_and_declares_mldsa_terms() {
    let mut subjects = std::collections::HashSet::new();
    let mut triples = 0usize;

    for result in TurtleParser::new().for_reader(TTL.as_bytes()) {
        let triple = result.expect("sigimpl-ext.ttl must be valid Turtle");
        triples += 1;
        if let NamedOrBlankNode::NamedNode(subject) = triple.subject {
            subjects.insert(subject.into_string());
        }
    }

    assert!(
        triples > 0,
        "sigimpl-ext.ttl parsed to zero triples — the extension graph is empty",
    );

    for local_name in [
        "MlDsa",
        "lattice",
        "assert-mldsa-pq-forgery",
        "assert-mldsa-pq-snooping",
        "assert-mldsa-unlinkability",
        "assert-mldsa-sigtype-leakage",
    ] {
        let iri = format!("{}{}", SIG_IMPL, local_name);
        assert!(
            subjects.contains(&iri),
            "sigimpl-ext.ttl does not declare `{}` as a subject",
            iri,
        );
    }
}
