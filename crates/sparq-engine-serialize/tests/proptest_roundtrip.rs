// (sq-wruhj) Mutation-witnessed RDF writer/parser round-trip properties.
#![cfg(all(feature = "serialize-rdf", feature = "streaming-serialization"))]

use std::collections::{BTreeSet, HashMap};

use oxrdf::{NamedOrBlankNode, Term, Triple};
use proptest::prelude::*;
use sparq_core::Graph;
use sparq_engine_serialize::serialize::{
    default_prefixes, graph_to_nquads, graph_to_trig, graph_to_trig_streaming, graph_to_turtle,
    graph_to_turtle_streaming, write_nquads, write_trig, write_trig_streaming, write_turtle,
    write_turtle_streaming, NamedGraph,
};

#[derive(Clone, Debug)]
enum Subject {
    Iri(u8),
    Blank(u8),
}

#[derive(Clone, Debug)]
enum Object {
    Iri(u8),
    Blank(u8),
    Plain(u8),
    Lang(u8),
    Typed(i16),
}

#[derive(Clone, Debug)]
struct Statement {
    subject: Subject,
    predicate: u8,
    object: Object,
    graph: Option<Subject>,
}

fn statement_strategy() -> impl Strategy<Value = Statement> {
    (
        prop_oneof![
            (0u8..5).prop_map(Subject::Iri),
            (0u8..3).prop_map(Subject::Blank)
        ],
        0u8..5,
        prop_oneof![
            (0u8..5).prop_map(Object::Iri),
            (0u8..3).prop_map(Object::Blank),
            (0u8..5).prop_map(Object::Plain),
            (0u8..5).prop_map(Object::Lang),
            any::<i16>().prop_map(Object::Typed),
        ],
        prop::option::of(prop_oneof![
            (0u8..3).prop_map(Subject::Iri),
            (0u8..2).prop_map(Subject::Blank),
        ]),
    )
        .prop_map(|(subject, predicate, object, graph)| Statement {
            subject,
            predicate,
            object,
            graph,
        })
}

fn subject(value: &Subject) -> String {
    match value {
        Subject::Iri(n) => format!("<http://example.test/s{n}>"),
        Subject::Blank(n) => format!("_:b{n}"),
    }
}

fn object(value: &Object) -> String {
    match value {
        Object::Iri(n) => format!("<http://example.test/o{n}>"),
        Object::Blank(n) => format!("_:b{n}"),
        Object::Plain(n) => {
            // Quotes, backslashes and newlines make escaping changes observable.
            const VALUES: [&str; 5] = ["plain", "quote\\\"", "slash\\\\", "line\\nfeed", "é"];
            format!("\"{}\"", VALUES[*n as usize])
        }
        Object::Lang(n) => format!("\"language-{n}\"@en"),
        Object::Typed(n) => format!("\"{n}\"^^<http://www.w3.org/2001/XMLSchema#integer>"),
    }
}

fn source_dataset(statements: &[Statement]) -> String {
    let mut out = String::new();
    for statement in statements {
        out.push_str(&subject(&statement.subject));
        out.push_str(&format!(
            " <http://example.test/p{}> {}",
            statement.predicate,
            object(&statement.object)
        ));
        if let Some(graph) = &statement.graph {
            out.push(' ');
            out.push_str(&subject(graph));
        }
        out.push_str(" .\n");
    }
    out
}

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
        for [s, p, o] in part.iter_ids() {
            rows.insert([
                term_key(part.dict.term(s), blank_map),
                term_key(part.dict.term(p), blank_map),
                term_key(part.dict.term(o), blank_map),
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
    let rows = dataset_rows(graph, &HashMap::new());
    rows.into_iter()
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

fn isomorphic(expected: &Graph, actual: &Graph) -> bool {
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

fn default_only(graph: &Graph) -> Graph {
    Graph::load_str(&graph_to_turtle(graph), "turtle").expect("writer emitted invalid Turtle")
}

// (sq-zd6hq) Exercise the slice-level writers in addition to their graph wrappers.
fn graph_triples(graph: &Graph) -> Vec<Triple> {
    graph
        .iter_ids()
        .map(|[s, p, o]| Triple {
            subject: match graph.dict.term(s) {
                Term::NamedNode(node) => NamedOrBlankNode::NamedNode(node),
                Term::BlankNode(node) => NamedOrBlankNode::BlankNode(node),
                other => panic!("invalid RDF subject: {other}"),
            },
            predicate: match graph.dict.term(p) {
                Term::NamedNode(node) => node,
                other => panic!("invalid RDF predicate: {other}"),
            },
            object: graph.dict.term(o),
        })
        .collect()
}

fn dataset_parts(graph: &Graph) -> Vec<(Option<Term>, Vec<Triple>)> {
    std::iter::once((None, graph_triples(graph)))
        .chain(
            graph
                .named
                .iter()
                .map(|(name, part)| (Some(name.clone()), graph_triples(part))),
        )
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn rdf_writers_round_trip_and_streaming_matches(
        mut statements in prop::collection::vec(statement_strategy(), 0..24)
    ) {
        // Always exercise the term-escaping seam, including in the default graph.
        statements.push(Statement {
            subject: Subject::Blank(2),
            predicate: 4,
            object: Object::Plain(1),
            graph: None,
        });
        let source = source_dataset(&statements);
        let graph = Graph::load_dataset(&source, "nquads").expect("generator emitted invalid RDF");

        let turtle = graph_to_turtle(&graph);
        let parsed_turtle = Graph::load_str(&turtle, "turtle").expect("invalid Turtle output");
        prop_assert!(isomorphic(&default_only(&graph), &parsed_turtle));

        let trig = graph_to_trig(&graph);
        let parsed_trig = Graph::load_dataset(&trig, "trig").expect("invalid TriG output");
        prop_assert!(isomorphic(&graph, &parsed_trig));

        let nquads = graph_to_nquads(&graph);
        let parsed_nquads = Graph::load_dataset(&nquads, "nquads").expect("invalid N-Quads output");
        prop_assert!(isomorphic(&graph, &parsed_nquads));

        let parts = dataset_parts(&graph);
        let view: Vec<NamedGraph<'_>> = parts
            .iter()
            .map(|(name, triples)| (name.as_ref(), triples.as_slice()))
            .collect();
        let prefixes = default_prefixes();
        prop_assert_eq!(write_turtle(&parts[0].1, &prefixes), turtle.clone());
        prop_assert_eq!(write_trig(&view, &prefixes), trig.clone());
        prop_assert_eq!(write_nquads(&view), nquads);

        let mut streamed_turtle = Vec::new();
        graph_to_turtle_streaming(&graph, &prefixes, &mut streamed_turtle).unwrap();
        prop_assert_eq!(turtle.as_bytes(), streamed_turtle.as_slice());

        let mut streamed_trig = Vec::new();
        graph_to_trig_streaming(&graph, &prefixes, &mut streamed_trig).unwrap();
        prop_assert_eq!(trig.as_bytes(), streamed_trig.as_slice());

        streamed_turtle.clear();
        write_turtle_streaming(&parts[0].1, &prefixes, &mut streamed_turtle).unwrap();
        prop_assert_eq!(turtle.as_bytes(), streamed_turtle.as_slice());

        streamed_trig.clear();
        write_trig_streaming(&view, &prefixes, &mut streamed_trig).unwrap();
        prop_assert_eq!(trig.as_bytes(), streamed_trig.as_slice());
    }
}
