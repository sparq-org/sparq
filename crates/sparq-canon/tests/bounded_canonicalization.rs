// [GPT-6] Native resource/identity regressions; these are not guest proof evidence.
use oxrdf::{BlankNode, GraphName, NamedNode, Quad, Term};
use sha2::Sha256;
use sparq_canon::{
    CanonicalizationLimits, canonicalize_quads_bounded_with, issue_quads_bounded_with,
};

fn limits() -> CanonicalizationLimits {
    CanonicalizationLimits {
        max_quads: 64,
        max_input_bytes: 16_384,
        max_output_bytes: 20_480,
        max_hndq_calls: 128,
        max_permutation_steps: 20_000,
    }
}

fn edge(from: &str, predicate: &str, to: &str) -> Quad {
    Quad::new(
        BlankNode::new(from).unwrap(),
        NamedNode::new(format!("http://ex/{predicate}")).unwrap(),
        BlankNode::new(to).unwrap(),
        GraphName::DefaultGraph,
    )
}

#[test]
fn bounded_canonicalization_and_issuer_match_standard_output_under_relabeling() {
    let graph = vec![edge("a", "p", "b"), edge("b", "p", "a")];
    let renamed = vec![edge("second", "p", "first"), edge("first", "p", "second")];
    let expected = sparq_canon::canonicalize_quads_with::<Sha256>(&graph).unwrap();
    for input in [graph, renamed] {
        assert_eq!(
            canonicalize_quads_bounded_with::<Sha256>(&input, &limits()).unwrap(),
            expected
        );
        assert_eq!(
            issue_quads_bounded_with::<Sha256>(&input, &limits()).unwrap(),
            sparq_canon::issue_quads_with::<Sha256>(&input).unwrap()
        );
    }
}

#[test]
fn each_capacity_rejects_and_hndq_limit_reaches_the_library() {
    let graph = vec![edge("a", "p", "b"), edge("b", "p", "a")];
    for (field, expected) in [
        (0, "input quad capacity"),
        (1, "input byte capacity"),
        (2, "output capacity"),
        (3, "permutation capacity"),
    ] {
        let mut cap = limits();
        match field {
            0 => cap.max_quads = 1,
            1 => cap.max_input_bytes = 1,
            2 => cap.max_output_bytes = 1,
            _ => cap.max_permutation_steps = 1,
        }
        assert!(
            canonicalize_quads_bounded_with::<Sha256>(&graph, &cap)
                .unwrap_err()
                .to_string()
                .contains(expected)
        );
        assert!(issue_quads_bounded_with::<Sha256>(&graph, &cap).is_err());
    }
    let mut cap = limits();
    cap.max_hndq_calls = 0;
    let error = canonicalize_quads_bounded_with::<Sha256>(&graph, &cap).unwrap_err();
    assert!(error.to_string().contains("Hash N-degree Quads"), "{error}");
}

#[test]
fn repeated_related_occurrences_are_counted_before_permutations() {
    // Same neighbouring node through eight different predicates: counting only
    // distinct neighbours would admit degree one and fail this regression.
    let graph: Vec<_> = (0..8).map(|i| edge("a", &format!("p{i}"), "b")).collect();
    let error = canonicalize_quads_bounded_with::<Sha256>(&graph, &limits()).unwrap_err();
    assert!(error.to_string().contains("permutation capacity"));
    // A node used as both subject and object contributes its quad twice to
    // the library's mention map; blank graph-name occurrences also count.
    let graph: Vec<_> = (0..4)
        .map(|i| {
            Quad::new(
                BlankNode::new("a").unwrap(),
                NamedNode::new(format!("http://ex/p{i}")).unwrap(),
                BlankNode::new("a").unwrap(),
                BlankNode::new("b").unwrap(),
            )
        })
        .collect();
    assert!(canonicalize_quads_bounded_with::<Sha256>(&graph, &limits()).is_err());
    let mut many = limits();
    many.max_hndq_calls = usize::MAX;
    assert!(canonicalize_quads_bounded_with::<Sha256>(&[edge("a", "p", "b")], &many).is_err());
    // Exercise checked factorial overflow, not only the configured small cap.
    let graph: Vec<_> = (0..30).map(|i| edge("a", &format!("p{i}"), "b")).collect();
    many.max_hndq_calls = 1;
    many.max_permutation_steps = usize::MAX;
    assert!(canonicalize_quads_bounded_with::<Sha256>(&graph, &many).is_err());
}

#[test]
fn input_and_output_bounds_apply_without_any_blank_nodes() {
    let graph = vec![Quad::new(
        NamedNode::new("http://ex/s").unwrap(),
        NamedNode::new("http://ex/p").unwrap(),
        Term::Literal(oxrdf::Literal::new_simple_literal("x".repeat(200))),
        GraphName::DefaultGraph,
    )];
    let mut cap = limits();
    cap.max_input_bytes = 100;
    assert!(canonicalize_quads_bounded_with::<Sha256>(&graph, &cap).is_err());
    cap.max_input_bytes = 1000;
    cap.max_output_bytes = 100;
    assert!(canonicalize_quads_bounded_with::<Sha256>(&graph, &cap).is_err());
    cap.max_output_bytes = 1000;
    cap.max_hndq_calls = 0;
    cap.max_permutation_steps = 0;
    assert!(canonicalize_quads_bounded_with::<Sha256>(&graph, &cap).is_ok());
}
