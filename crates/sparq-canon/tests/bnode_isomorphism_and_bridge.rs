//! sq-bif.9: focused isolated cases for `sparq-canon`'s PUBLIC API.
//!
//! The manifest-driven `tests/rdf_canon_suite.rs` already drives the FULL 86-entry W3C
//! RDFC-1.0 suite (eval + issued-id map + negative poison-graph), so this crate's test
//! BREADTH is strong. This file adds the two pin-point cases the bead calls out, which the
//! suite exercises only implicitly:
//!
//! 1. **Blank-node-isomorphism equivalence** — two graphs that differ ONLY in their
//!    (lexical) blank-node labels and triple order must canonicalize to BYTE-IDENTICAL
//!    output; a graph that is NOT isomorphic must canonicalize differently. The suite
//!    proves isomorphism transitively across its vectors; this pins the core property
//!    directly with a hand-built pair.
//! 2. **oxrdf-bridge error / edge paths** — the single oxrdf-0.3 -> text -> oxrdf-0.2
//!    bridge is the seam every sparq consumer shares. We pin (a) the RDF-1.2 triple-term
//!    rejection on BOTH the quad (`canonicalize`) and the single-graph
//!    (`canonicalize_triples`) entry points (they reject via different code paths), (b)
//!    that the `CanonError` `Display` is non-empty and distinct per variant, and (c) that
//!    datatyped / language-tagged literals survive the text round-trip losslessly (the
//!    bridge's most fragile case).

use oxrdf::{BlankNode, GraphName, Literal, NamedNode, NamedOrBlankNode, Quad, Term, Triple};
use sparq_canon::{
    canonicalize, canonicalize_triples, issued_identifiers, CanonError, CanonicalGraph,
};

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

fn bnode(label: &str) -> NamedOrBlankNode {
    NamedOrBlankNode::BlankNode(BlankNode::new(label).unwrap())
}

// ---------------------------------------------------------------------------
// 1. Blank-node-isomorphism equivalence.
// ---------------------------------------------------------------------------

/// Builds a 2-triple, 2-blank-node "diamond" graph parameterized by the input blank-node
/// labels and triple order. Shape (independent of labels):
///   `_:x  <p>  _:y`
///   `_:y  <q>  "v"`
fn linked_bnode_graph(x: &str, y: &str, swap_order: bool) -> CanonicalGraph {
    let t1 = Triple::new(
        bnode(x),
        iri("http://ex/p"),
        Term::BlankNode(BlankNode::new(y).unwrap()),
    );
    let t2 = Triple::new(
        bnode(y),
        iri("http://ex/q"),
        Term::Literal(Literal::new_simple_literal("v")),
    );
    let triples = if swap_order {
        vec![t2, t1]
    } else {
        vec![t1, t2]
    };
    canonicalize_triples(&triples).expect("canonicalize")
}

/// Two graphs that differ ONLY in blank-node labels (`zzz/aaa` vs `p/q`) and triple order
/// must produce byte-identical canonical N-Quads — that IS the RDFC-1.0 guarantee. Distinct
/// from the inline `src` unit test in that it also asserts the canonical labels are the
/// fully-relabelled `c14n` form and the line count, so a regression that, e.g., stopped
/// relabelling is caught here too.
#[test]
fn isomorphic_graphs_under_different_bnode_labels_canonicalize_identically() {
    let a = linked_bnode_graph("zzz", "aaa", false);
    let b = linked_bnode_graph("p", "q", true);

    assert_eq!(a.lines.len(), 2, "two triples -> two canonical lines");
    assert_eq!(
        a.lines, b.lines,
        "isomorphic graphs (relabelled bnodes + permuted order) must be byte-identical"
    );
    // Every blank node was relabelled to the canonical c14n form; NO input label leaks.
    let joined = a.to_nquads();
    assert!(
        joined.contains("_:c14n"),
        "canonical form must use c14n labels: {joined}"
    );
    for leaked in ["zzz", "aaa", "_:p", "_:q"] {
        assert!(
            !joined.contains(leaked),
            "input bnode label {leaked} leaked into canonical form: {joined}"
        );
    }
}

/// The negative side: a graph that is NOT isomorphic to the above (the literal differs)
/// must canonicalize DIFFERENTLY. Without this, a degenerate "always return the same
/// string" implementation would pass the positive test; this pins that canonicalization
/// actually discriminates non-isomorphic graphs.
#[test]
fn non_isomorphic_graphs_canonicalize_differently() {
    let a = linked_bnode_graph("zzz", "aaa", false);
    // Same shape but a different literal value -> not isomorphic.
    let t1 = Triple::new(
        bnode("m"),
        iri("http://ex/p"),
        Term::BlankNode(BlankNode::new("n").unwrap()),
    );
    let t2 = Triple::new(
        bnode("n"),
        iri("http://ex/q"),
        Term::Literal(Literal::new_simple_literal("DIFFERENT")),
    );
    let c = canonicalize_triples(&[t1, t2]).unwrap();
    assert_ne!(
        a.lines, c.lines,
        "graphs differing in a literal must NOT share a canonical form"
    );
}

/// Two blank nodes that are structurally INDISTINGUISHABLE (both are subjects of the same
/// triple shape pointing at the same IRI) must each still receive a distinct, stable c14n
/// label, and the issued-identifier map must be a bijection onto `{c14n0, c14n1}`. This is
/// the "twins" case RDFC-1.0's n-degree hashing exists to disambiguate.
#[test]
fn structurally_symmetric_bnodes_get_distinct_stable_canonical_labels() {
    let q1 = Quad::new(
        bnode("aaa"),
        iri("http://ex/p"),
        Term::NamedNode(iri("http://ex/o")),
        GraphName::DefaultGraph,
    );
    let q2 = Quad::new(
        bnode("bbb"),
        iri("http://ex/p"),
        Term::NamedNode(iri("http://ex/o")),
        GraphName::DefaultGraph,
    );
    let map = issued_identifiers(&[q1, q2]).unwrap();
    assert_eq!(map.len(), 2, "two distinct input bnodes -> two issued ids");
    let mut issued: Vec<&str> = map.values().map(String::as_str).collect();
    issued.sort_unstable();
    assert_eq!(
        issued,
        vec!["c14n0", "c14n1"],
        "issued ids must be the canonical c14n0/c14n1 set: {map:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. oxrdf-bridge error / edge paths.
// ---------------------------------------------------------------------------

/// The RDF-1.2 triple-term rejection on the QUAD entry point (`canonicalize`). This is a
/// DISTINCT code path from the single-graph `canonicalize_triples` rejection (validated by
/// the inline unit test): `bridge_to_02` inspects `q.object` directly, whereas
/// `canonicalize_triples` pre-scans with `contains_triple_term`. Both must fail closed.
#[test]
fn quad_with_triple_term_object_is_rejected_by_bridge() {
    let inner = Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/p"),
        Term::Literal(Literal::new_simple_literal("v")),
    );
    let q = Quad::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/p"),
        Term::Triple(Box::new(inner)),
        GraphName::DefaultGraph,
    );
    assert!(
        matches!(canonicalize(&[q]), Err(CanonError::TripleTerm)),
        "a quad with a triple-term object must be rejected (RDFC-1.0 data model)"
    );
}

/// `CanonError`'s `Display` must be non-empty and DISTINCT per variant (so a fail-closed
/// caller surfaces a meaningful message, and the variants are not accidentally collapsed
/// to one string). The `Bridge` variant is otherwise only reachable internally, so this is
/// the focused coverage of its formatting path the suite never triggers.
#[test]
fn canon_error_display_is_distinct_and_nonempty_per_variant() {
    let tt = CanonError::TripleTerm.to_string();
    let br = CanonError::Bridge("parse boom".into()).to_string();
    let cz = CanonError::Canonicalization("hndq limit".into()).to_string();
    let nb = CanonError::NestedBlankNode.to_string();
    let td = CanonError::TripleTermDepthExceeded.to_string();

    for (label, msg) in [
        ("TripleTerm", &tt),
        ("Bridge", &br),
        ("Canonicalization", &cz),
        ("NestedBlankNode", &nb),
        ("TripleTermDepthExceeded", &td),
    ] {
        assert!(!msg.is_empty(), "{label} Display must be non-empty");
    }
    assert_ne!(nb, td, "the two ungated profile-guard variants must differ");
    assert_ne!(tt, td);
    // The wrapped detail must be carried through, and the three messages must differ.
    assert!(
        br.contains("parse boom"),
        "Bridge must carry its detail: {br}"
    );
    assert!(
        cz.contains("hndq limit"),
        "Canon must carry its detail: {cz}"
    );
    assert_ne!(tt, br);
    assert_ne!(tt, cz);
    assert_ne!(br, cz);
}

/// Bridge round-trip fidelity for the FRAGILE literal forms: a language-tagged literal and
/// a datatyped literal must survive the oxrdf-0.3 -> canonical N-Quads text -> oxrdf-0.2
/// seam losslessly (these are exactly where a naive `Display`/parse bridge silently drops
/// the tag/datatype). Pin that the canonical output still carries both the language tag and
/// the datatype IRI, and that the literal object is preserved byte-for-byte in the
/// re-parsed canonical triple.
#[test]
fn bridge_preserves_language_tag_and_datatype_through_text_roundtrip() {
    let lang = Literal::new_language_tagged_literal("chat", "fr").unwrap();
    let typed = Literal::new_typed_literal("42", iri("http://www.w3.org/2001/XMLSchema#integer"));
    let t_lang = Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/says"),
        Term::Literal(lang.clone()),
    );
    let t_typed = Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/count"),
        Term::Literal(typed.clone()),
    );

    let c = canonicalize_triples(&[t_lang, t_typed]).unwrap();
    let doc = c.to_nquads();
    assert!(
        doc.contains("@fr"),
        "language tag must survive the bridge: {doc}"
    );
    assert!(
        doc.contains("XMLSchema#integer"),
        "datatype IRI must survive the bridge: {doc}"
    );
    // The re-parsed canonical triples must carry back the EXACT literal terms.
    let objects: Vec<Term> = c.triples.iter().map(|t| t.object.clone()).collect();
    assert!(
        objects.contains(&Term::Literal(lang)),
        "language-tagged literal not preserved in canonical triple: {objects:?}"
    );
    assert!(
        objects.contains(&Term::Literal(typed)),
        "datatyped literal not preserved in canonical triple: {objects:?}"
    );
}

/// Edge case: the empty dataset canonicalizes to the empty document (no error, no spurious
/// lines). A trivial-but-real boundary the suite's non-empty vectors never hit.
#[test]
fn empty_dataset_canonicalizes_to_empty_document() {
    assert_eq!(canonicalize(&[]).unwrap(), "");
    let c = canonicalize_triples(&[]).unwrap();
    assert!(c.lines.is_empty());
    assert!(c.triples.is_empty());
    assert_eq!(c.to_nquads(), "");
}
