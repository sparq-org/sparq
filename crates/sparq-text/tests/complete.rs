// sq-lsp7k.9.1: non-vacuous completion API coverage.
use rustc_hash::FxHashMap;
use sparq_core::Graph;
use sparq_text::{Candidate, CandidateKind, CompletionIndex};

fn fixture() -> Graph {
    Graph::load_str(
        r#"
        @prefix ex: <http://example.com/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        @prefix skos: <http://www.w3.org/2004/02/skos/core#> .
        ex:AlphaOne ex:related ex:AlphaTwo .
        ex:labelledOne rdfs:label "Alpine"@en .
        ex:labelledTwo skos:prefLabel "Albatross" .
        "#,
        "turtle",
    )
    .unwrap()
}

fn id(graph: &Graph, iri: &str) -> u32 {
    graph
        .id_of(&oxrdf::NamedNode::new_unchecked(iri).into())
        .unwrap()
}

#[test]
fn builds_and_completes_with_casefolded_deterministic_candidates() {
    let graph = fixture();
    let index = CompletionIndex::build(&graph);
    let alpha_one = id(&graph, "http://example.com/AlphaOne");
    let alpha_two = id(&graph, "http://example.com/AlphaTwo");
    let labelled_one = id(&graph, "http://example.com/labelledOne");
    let labelled_two = id(&graph, "http://example.com/labelledTwo");
    let rdfs_label = id(&graph, "http://www.w3.org/2000/01/rdf-schema#label");
    let skos_pref = id(&graph, "http://www.w3.org/2004/02/skos/core#prefLabel");

    let expected = vec![
        Candidate {
            id: labelled_two,
            key: "albatross".into(),
            kind: CandidateKind::Label(skos_pref),
            score: 0.0,
        },
        Candidate {
            id: alpha_one,
            key: "alphaone".into(),
            kind: CandidateKind::LocalName,
            score: 0.0,
        },
        Candidate {
            id: alpha_two,
            key: "alphatwo".into(),
            kind: CandidateKind::LocalName,
            score: 0.0,
        },
        Candidate {
            id: labelled_one,
            key: "alpine".into(),
            kind: CandidateKind::Label(rdfs_label),
            score: 0.0,
        },
    ];
    assert_eq!(index.complete("al", 10, None), expected);
    assert_eq!(index.complete("AL", 10, None), expected);
    assert_eq!(index.indexed_dict_len(), graph.dict.len());
    assert_eq!(index.len(), 16);
    assert!(!index.is_empty());
    assert!(index.heap_bytes() > 0);

    // The label result identifies its subject, never the label literal.
    let alpine = index.complete("alpine", 1, None).pop().unwrap();
    assert_eq!(alpine.id, labelled_one);
    assert_ne!(graph.dict.term(alpine.id).to_string(), "\"Alpine\"@en");
}

#[test]
fn injected_scores_reorder_and_k_truncates() {
    let graph = fixture();
    let index = CompletionIndex::build(&graph);
    let alpha_two = id(&graph, "http://example.com/AlphaTwo");
    let mut scores = FxHashMap::default();
    scores.insert(alpha_two, 2.5);

    let hits = index.complete("al", 2, Some(&scores));
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, alpha_two);
    assert_eq!(hits[0].score, 2.5);
    assert!(index.complete("al", 0, Some(&scores)).is_empty());
}

#[test]
fn default_index_introspection_is_empty() {
    let index = CompletionIndex::default();
    assert!(index.is_empty());
    assert_eq!(index.len(), 0);
    assert_eq!(index.indexed_dict_len(), 0);
    assert_eq!(index.heap_bytes(), 0);
    assert!(index.complete("anything", 10, None).is_empty());
}
