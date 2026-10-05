//! Parse-validation of the published `sec-prop:` extension vocabulary
//! `ontologies/secprop-ext.ttl` (sq-5oru9). The Turtle a working group / reasoner
//! reads MUST stay valid Turtle and MUST declare exactly the term set the Rust
//! constants name. The `tests::secprop_ext_iris_match_rust_constants` unit test in
//! `src/lib.rs` pins the IRIs against the constants as plain strings; this test is
//! the *parsed*/syntactic guard (the `sparq-trust` `vocab_ttl.rs` discipline applied
//! to the extension).
//!
//! Unconditional: this crate is the dependency-free leaf that OWNS the vocabulary, so
//! there is no feature to gate on — its consumers keep their own default-OFF gates.
//!
//! [OPUS-5] sq-3705 (moved here from `sparq-trust/tests/secprop_ttl.rs` with the
//! Turtle it validates). Originally sq-5oru9 (epic sq-0dksu; design PR #972).
//! 🤖 SPARQ agent — security-properties ontology.

use oxttl::TurtleParser;

const TTL: &str = include_str!("../ontologies/secprop-ext.ttl");

/// The published extension vocabulary is valid Turtle and non-empty.
#[test]
fn secprop_ext_ttl_is_valid_turtle() {
    let mut triples = 0usize;
    for result in TurtleParser::new().for_reader(TTL.as_bytes()) {
        result.expect("secprop-ext.ttl must be valid Turtle");
        triples += 1;
    }
    assert!(
        triples > 0,
        "secprop-ext.ttl parsed to zero triples — the published vocabulary is empty",
    );
}

/// Every constant in `ALL_SECPROP_IRIS` appears as a SUBJECT in the parsed
/// Turtle (a coarse presence check independent of the unit-test constant pinning, so
/// a refactor of `src/lib.rs` cannot silently drop a published term). The two reused
/// vendored class IRIs (`sec-prop:Unlinkability`, `sec-prop:SecurityProperty`) are
/// referenced (range / owl:imports) but not re-declared as subjects in the extension
/// file, so they are exempt — exactly the split the unit test makes.
#[test]
fn secprop_ext_ttl_declares_every_minted_term() {
    use oxrdf::NamedOrBlankNode;
    use sparq_secprop_vocab::{
        ALL_SECPROP_IRIS, SEC_PROP_SECURITY_PROPERTY, SEC_PROP_UNLINKABILITY,
    };

    let reused_only = [SEC_PROP_UNLINKABILITY, SEC_PROP_SECURITY_PROPERTY];

    let mut subjects = std::collections::HashSet::new();
    for result in TurtleParser::new().for_reader(TTL.as_bytes()) {
        let t = result.expect("valid Turtle");
        if let NamedOrBlankNode::NamedNode(s) = t.subject {
            subjects.insert(s.into_string());
        }
    }

    for &iri in ALL_SECPROP_IRIS {
        if reused_only.contains(&iri) {
            continue;
        }
        assert!(
            subjects.contains(iri),
            "secprop-ext.ttl does not declare `{}` as a subject",
            iri,
        );
    }
}

/// The three VENDORED `sec-prop:` dimension IRIs that the estate uses as `secx:property`
/// values — `PostQuantumForgery`, `PostQuantumSnooping`, `SignatureTypeLeakage` — are
/// declared here as `sec-prop:SecurityProperty` subjects (issue #3441). Previously only
/// their LEVELS were in this file, so the cross-crate dimension-IRI drift guards
/// (sq-mgxz8, `sparq_policy::secprop`) had to exempt them from the subject-presence
/// check via a hard-coded list — those exemption entries are now redundant for these
/// three, but retiring them is follow-up work in `sparq-policy`. The declarations
/// re-assert the vendored IRI, label and `sec-prop:SecurityProperty` type; nothing is
/// minted or refined (design §4.1, extend-do-not-fork).
#[test]
fn secprop_ext_ttl_declares_the_vendored_dimension_subjects() {
    use oxrdf::{NamedNode, NamedOrBlankNode, Term};
    use sparq_secprop_vocab::{
        SEC_PROP_POST_QUANTUM_FORGERY, SEC_PROP_POST_QUANTUM_SNOOPING,
        SEC_PROP_SECURITY_PROPERTY, SEC_PROP_SIGNATURE_TYPE_LEAKAGE,
    };

    const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    let rdf_type = NamedNode::new(RDF_TYPE).unwrap();

    let mut typed: Vec<(String, String)> = Vec::new();
    for result in TurtleParser::new().for_reader(TTL.as_bytes()) {
        let t = result.expect("valid Turtle");
        if t.predicate == rdf_type {
            if let (NamedOrBlankNode::NamedNode(s), Term::NamedNode(o)) = (t.subject, t.object) {
                typed.push((s.into_string(), o.into_string()));
            }
        }
    }

    for dim in [
        SEC_PROP_POST_QUANTUM_FORGERY,
        SEC_PROP_POST_QUANTUM_SNOOPING,
        SEC_PROP_SIGNATURE_TYPE_LEAKAGE,
    ] {
        assert!(
            typed
                .iter()
                .any(|(s, ty)| s == dim && ty == SEC_PROP_SECURITY_PROPERTY),
            "the vendored dimension `{}` must be declared in secprop-ext.ttl as a \
             sec-prop:SecurityProperty subject, so the sq-mgxz8 drift guards can check \
             subject presence without a vendored-dimension exemption list (#3441)",
            dim,
        );
    }
}

/// The assurance axis carries exactly the three ordered levels, the audit-status set
/// includes the live `ExternalSignOffPending` state, and the file references the open
/// `sq-qhy4` audit gate — the load-bearing honesty invariants of the vocabulary.
#[test]
fn secprop_ext_ttl_carries_the_assurance_axis_and_audit_gate() {
    use oxrdf::{NamedNode, NamedOrBlankNode, Term};
    use sparq_secprop_vocab::{
        SECX_ASSURANCE_LEVEL, SECX_AUDIT_STATUS_CLASS, SECX_CLAIMED, SECX_CONJECTURED,
        SECX_EXTERNAL_SIGN_OFF_PENDING, SECX_PROVEN,
    };

    const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

    // Collect (subject, type) pairs.
    let mut typed: Vec<(String, String)> = Vec::new();
    for result in TurtleParser::new().for_reader(TTL.as_bytes()) {
        let t = result.expect("valid Turtle");
        if t.predicate == NamedNode::new(RDF_TYPE).unwrap() {
            if let (NamedOrBlankNode::NamedNode(s), Term::NamedNode(o)) = (t.subject, t.object) {
                typed.push((s.into_string(), o.into_string()));
            }
        }
    }
    let is_a = |s: &str, class: &str| typed.iter().any(|(sub, ty)| sub == s && ty == class);

    for level in [SECX_PROVEN, SECX_CLAIMED, SECX_CONJECTURED] {
        assert!(
            is_a(level, SECX_ASSURANCE_LEVEL),
            "`{}` must be a secx:AssuranceLevel",
            level,
        );
    }
    assert!(
        is_a(SECX_EXTERNAL_SIGN_OFF_PENDING, SECX_AUDIT_STATUS_CLASS),
        "the live sq-qhy4 state ExternalSignOffPending must be a secx:AuditStatus",
    );

    assert!(
        TTL.contains("sq-qhy4"),
        "secprop-ext.ttl must reference the open external-audit gate (sq-qhy4)",
    );
}
