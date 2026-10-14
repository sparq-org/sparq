// sq-ddws7: mutation-witnessed coverage for canonical digest bytes.

use oxrdf::{BlankNode, GraphName, Literal, NamedNode, Quad};
use sha2::{Digest as _, Sha256, Sha384};
use sparq_canon::{canonicalize_quads, digest_quads_with};

fn dataset(first: &str, second: &str) -> Vec<Quad> {
    let first = BlankNode::new(first).unwrap();
    let second = BlankNode::new(second).unwrap();
    vec![
        Quad::new(
            first,
            NamedNode::new("http://example.org/p").unwrap(),
            second.clone(),
            GraphName::DefaultGraph,
        ),
        Quad::new(
            second,
            NamedNode::new("http://example.org/q").unwrap(),
            Literal::new_simple_literal("value"),
            GraphName::DefaultGraph,
        ),
    ]
}

#[test]
fn digest_lengths_follow_the_caller_selected_hasher() {
    let dataset = dataset("b0", "b1");
    let sha256 = digest_quads_with::<Sha256>(&dataset).unwrap();
    let sha384 = digest_quads_with::<Sha384>(&dataset).unwrap();

    assert_eq!(sha256.len(), 32);
    assert_eq!(sha384.len(), 48);
    assert_ne!(sha256, sha384);
}

#[test]
fn digest_is_invariant_under_blank_node_relabelling() {
    let original = dataset("b0", "b1");
    let relabelled = dataset("x", "y");

    assert_eq!(
        digest_quads_with::<Sha256>(&original).unwrap(),
        digest_quads_with::<Sha256>(&relabelled).unwrap()
    );
}

#[test]
fn digest_is_invariant_under_quad_order() {
    let original = dataset("b0", "b1");
    let mut reversed = original.clone();
    reversed.reverse();

    assert_eq!(
        digest_quads_with::<Sha256>(&original).unwrap(),
        digest_quads_with::<Sha256>(&reversed).unwrap()
    );
}

#[test]
fn digest_covers_the_exact_canonical_bytes() {
    let dataset = dataset("b0", "b1");
    let canonical = canonicalize_quads(&dataset).unwrap();
    let exact = Sha256::digest(canonical.as_bytes()).to_vec();

    assert!(canonical.ends_with('\n'));
    assert_eq!(digest_quads_with::<Sha256>(&dataset).unwrap(), exact);

    let without_final_newline = canonical.strip_suffix('\n').unwrap();
    assert_ne!(
        exact,
        Sha256::digest(without_final_newline.as_bytes()).to_vec()
    );
}
