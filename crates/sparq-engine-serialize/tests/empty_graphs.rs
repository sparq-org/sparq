// (sq-lzaw6) Mutation-witnessed empty graph and empty-default-graph round trips.
#![cfg(feature = "serialize-rdf")]

use std::collections::{BTreeSet, HashMap};

use oxrdf::Term;
use sparq_core::Graph;
use sparq_engine_serialize::serialize::{
    graph_to_jsonld, graph_to_nquads, graph_to_trig, graph_to_turtle, JsonLdForm,
};

fn term_key(term: Term, blank_map: &HashMap<String, String>) -> String {
    match term {
        Term::BlankNode(node) => blank_map
            .get(node.as_str())
            .cloned()
            .unwrap_or_else(|| format!("_:{}", node.as_str())),
        other => other.to_string(),
    }
}

fn dataset_rows(graph: &Graph, blank_map: &HashMap<String, String>) -> BTreeSet<[String; 4]> {
    let mut rows = BTreeSet::new();
    let mut collect = |name: Option<Term>, part: &Graph| {
        for [subject, predicate, object] in part.iter_ids() {
            rows.insert([
                term_key(part.dict().term(subject), blank_map),
                term_key(part.dict().term(predicate), blank_map),
                term_key(part.dict().term(object), blank_map),
                name.clone()
                    .map(|term| term_key(term, blank_map))
                    .unwrap_or_default(),
            ]);
        }
    };

    collect(None, graph);
    for (name, part) in &graph.named {
        collect(Some(name.clone()), part);
    }
    rows
}

fn blank_labels(graph: &Graph) -> Vec<String> {
    dataset_rows(graph, &HashMap::new())
        .into_iter()
        .flatten()
        .filter_map(|value| value.strip_prefix("_:").map(str::to_owned))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn permutations<T: Clone>(values: &mut [T], at: usize, output: &mut Vec<Vec<T>>) {
    if at == values.len() {
        output.push(values.to_vec());
        return;
    }
    for index in at..values.len() {
        values.swap(at, index);
        permutations(values, at + 1, output);
        values.swap(at, index);
    }
}

fn datasets_are_isomorphic(expected: &Graph, actual: &Graph) -> bool {
    let expected_blanks = blank_labels(expected);
    let mut actual_blanks = blank_labels(actual);
    if expected_blanks.len() != actual_blanks.len() {
        return false;
    }

    let expected_rows = dataset_rows(expected, &HashMap::new());
    let mut candidates = Vec::new();
    permutations(&mut actual_blanks, 0, &mut candidates);
    candidates.into_iter().any(|candidate| {
        let mapping: HashMap<_, _> = candidate
            .into_iter()
            .zip(expected_blanks.iter().map(|label| format!("_:{label}")))
            .collect();
        dataset_rows(actual, &mapping) == expected_rows
    })
}

fn assert_isomorphic(expected: &Graph, actual: &Graph) {
    assert!(
        datasets_are_isomorphic(expected, actual),
        "datasets are not isomorphic"
    );
}

fn assert_empty_round_trip(output: &str, format: &str) {
    let parsed = Graph::load_dataset(output, format).expect("writer output must parse");
    assert_eq!(parsed.len(), 0, "default graph must stay empty");
    assert!(parsed.named.is_empty(), "no named graph may be introduced");
    assert_isomorphic(&Graph::new(), &parsed);
}

fn named_only_dataset() -> Graph {
    Graph::load_dataset(
        concat!(
            "<http://example.test/s1> <http://example.test/p> \"one\" <http://example.test/g1> .\n",
            "<http://example.test/s2> <http://example.test/p> <http://example.test/o2> <http://example.test/g2> .\n",
        ),
        "nquads",
    )
    .expect("test fixture must parse")
}

#[test]
fn empty_turtle_round_trips() {
    assert_empty_round_trip(&graph_to_turtle(&Graph::new()), "turtle");
}

#[test]
fn empty_trig_round_trips() {
    assert_empty_round_trip(&graph_to_trig(&Graph::new()), "trig");
}

#[test]
fn empty_nquads_round_trips() {
    assert_empty_round_trip(&graph_to_nquads(&Graph::new()), "nquads");
}

#[test]
fn empty_jsonld_round_trips() {
    assert_empty_round_trip(
        &graph_to_jsonld(&Graph::new(), JsonLdForm::Expanded),
        "jsonld",
    );
}

#[test]
#[should_panic(expected = "datasets are not isomorphic")]
fn isomorphism_assertion_rejects_a_stray_expected_triple() {
    let expected = Graph::load_str(
        "<http://example.test/stray> <http://example.test/p> <http://example.test/o> .",
        "turtle",
    )
    .expect("test fixture must parse");
    assert_isomorphic(&expected, &Graph::new());
}

#[test]
fn trig_preserves_named_graphs_when_default_graph_is_empty() {
    let expected = named_only_dataset();
    assert_eq!(expected.len(), 0, "fixture default graph must be empty");

    let output = graph_to_trig(&expected);
    let actual = Graph::load_dataset(&output, "trig").expect("TriG output must parse");

    assert_eq!(actual.len(), 0, "default graph must stay empty");
    assert_isomorphic(&expected, &actual);
}

#[test]
fn nquads_preserves_named_graphs_when_default_graph_is_empty() {
    let expected = named_only_dataset();
    assert_eq!(expected.len(), 0, "fixture default graph must be empty");

    let output = graph_to_nquads(&expected);
    let actual = Graph::load_dataset(&output, "nquads").expect("N-Quads output must parse");

    assert_eq!(actual.len(), 0, "default graph must stay empty");
    assert_isomorphic(&expected, &actual);
}
