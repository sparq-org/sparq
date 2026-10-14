//! sq-lsp7k.9.2: mutation-witnessed incremental completion oracle.

use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_text::CompletionIndex;

const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const SKOS_PREF_LABEL: &str = "http://www.w3.org/2004/02/skos/core#prefLabel";

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.0
    }
}

fn iri(value: impl Into<String>) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(value.into()))
}

fn random_batch(rng: &mut Lcg, round: usize) -> Vec<[Term; 3]> {
    (0..8)
        .map(|item| {
            let n = rng.next();
            let subject = iri(format!("http://example.com/entity/{round}/{item}/{n}"));
            if n.is_multiple_of(3) {
                let predicate = if n.is_multiple_of(2) {
                    RDFS_LABEL
                } else {
                    SKOS_PREF_LABEL
                };
                [
                    subject,
                    iri(predicate),
                    Term::Literal(Literal::new_simple_literal(format!("Label {round} {n}"))),
                ]
            } else {
                [
                    subject,
                    iri(format!("http://example.com/property/{}", n % 7)),
                    iri(format!("http://example.com/object/{n}")),
                ]
            }
        })
        .collect()
}

#[test]
fn apply_delta_equals_fresh_build_after_every_seeded_batch() {
    let mut graph = Graph::load_str("", "turtle").unwrap();
    let mut index = CompletionIndex::build(&graph);
    let mut rng = Lcg(0xc0de_cafe_5eed);

    for round in 0..5 {
        let inserts = random_batch(&mut rng, round);
        graph.apply_delta(&inserts, &[]).unwrap();
        index.apply_delta(&graph, &inserts, &[]);

        let rebuilt = CompletionIndex::build(&graph);
        assert_eq!(
            index, rebuilt,
            "incremental index diverged after batch {round}"
        );
        assert!(index.is_consistent_with(&graph));
    }
}

#[test]
fn reconcile_growth_equals_fresh_build_and_reports_work() {
    let mut graph = Graph::load_str(
        "@prefix ex: <http://example.com/> . ex:seed ex:p ex:value .",
        "turtle",
    )
    .unwrap();
    let mut index = CompletionIndex::build(&graph);

    let additions = [[
        iri("http://example.com/appended"),
        iri(RDFS_LABEL),
        Term::Literal(Literal::new_simple_literal("Appended label")),
    ]];
    graph.apply_delta(&additions, &[]).unwrap();
    assert!(!index.is_consistent_with(&graph));

    let added = index.reconcile(&graph);
    assert!(added > 0);
    assert_eq!(index, CompletionIndex::build(&graph));
    assert!(index.is_consistent_with(&graph));
    assert_eq!(index.reconcile(&graph), 0);
}

#[test]
fn unrelated_shorter_graph_honestly_needs_rebuild() {
    let indexed_graph = Graph::load_str(
        "@prefix ex: <http://example.com/> . ex:a ex:p ex:b . ex:c ex:q ex:d .",
        "turtle",
    )
    .unwrap();
    let index = CompletionIndex::build(&indexed_graph);
    let unrelated = Graph::load_str(
        "<http://other.example/a> <http://other.example/p> \"v\" .",
        "turtle",
    )
    .unwrap();

    assert!(unrelated.dict.len() < index.indexed_dict_len());
    assert!(index.needs_rebuild(&unrelated));
    assert!(!index.is_consistent_with(&unrelated));
}
