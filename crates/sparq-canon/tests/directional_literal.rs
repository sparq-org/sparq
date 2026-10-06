//! GitHub #5359: RDF 1.2 directional-language literals (`"…"@en--ltr`) are
//! outside RDFC-1.0's RDF-1.1 data model and cannot cross the standard path's
//! oxrdf-0.2 bridge. Every standard entry point must fail closed with the typed
//! `CanonError::DirectionalLiteral`, not a generic `CanonError::Bridge`.

use oxrdf::{BaseDirection, BlankNode, GraphName, Literal, NamedNode, Quad, Triple};
use sparq_canon::CanonError;

fn directional_triple() -> Triple {
    Triple::new(
        BlankNode::new("b0").unwrap(),
        NamedNode::new("http://ex/p").unwrap(),
        Literal::new_directional_language_tagged_literal("hello", "en", BaseDirection::Ltr)
            .unwrap(),
    )
}

fn directional_quad() -> Quad {
    let t = directional_triple();
    Quad::new(t.subject, t.predicate, t.object, GraphName::DefaultGraph)
}

fn assert_directional<T: std::fmt::Debug>(r: Result<T, CanonError>) {
    match r {
        Err(CanonError::DirectionalLiteral) => {}
        other => panic!("expected CanonError::DirectionalLiteral, got {other:?}"),
    }
}

#[test]
fn standard_dataset_paths_reject_directional_literal_with_typed_error() {
    let ds = [directional_quad()];
    assert_directional(sparq_canon::canonicalize(&ds));
    assert_directional(sparq_canon::canonicalize_quads(&ds));
    assert_directional(sparq_canon::issue_quads(&ds));
    assert_directional(sparq_canon::issued_identifiers(&ds));
}

#[test]
fn standard_single_graph_paths_reject_directional_literal_with_typed_error() {
    let ts = [directional_triple()];
    assert_directional(sparq_canon::canonicalize_triples(&ts));
    assert_directional(sparq_canon::issue_triples(&ts));
}

#[test]
fn nquads_text_entry_point_rejects_directional_literal_with_typed_error() {
    let doc = "_:b0 <http://ex/p> \"hello\"@en--rtl .\n";
    assert_directional(sparq_canon::canonicalize_nquads(doc));
}

#[test]
fn directional_literal_error_names_the_rdf12_profile() {
    let msg = CanonError::DirectionalLiteral.to_string();
    assert!(msg.contains("rdf12-triple-terms"), "{msg}");
}

#[test]
fn plain_language_literal_still_canonicalizes() {
    let doc = "_:b0 <http://ex/p> \"hello\"@en .\n";
    let canon = sparq_canon::canonicalize_nquads(doc).unwrap();
    assert_eq!(canon, "_:c14n0 <http://ex/p> \"hello\"@en .\n");
}

/// The opt-in non-standard profile canonicalizes directional literals natively.
#[cfg(feature = "rdf12-triple-terms")]
#[test]
fn rdf12_profile_canonicalizes_directional_literal() {
    let canon = sparq_canon::rdf12::canonicalize_rdf12(&[directional_quad()]).unwrap();
    assert_eq!(canon, "_:c14n0 <http://ex/p> \"hello\"@en--ltr .\n");
}
