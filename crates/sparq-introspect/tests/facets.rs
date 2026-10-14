use oxrdf::Term;
use sparq_core::Graph;
use sparq_introspect::{facets, Counted, FacetRequest};
use std::collections::BTreeMap;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

fn fixture() -> Graph {
    let mut nt = String::new();
    for i in 0..6 {
        let class = if i < 4 { "A" } else { "B" };
        let color = if i % 3 == 0 { "red" } else { "blue" };
        nt.push_str(&format!(
            "<http://e/s{i}> <{RDF_TYPE}> <http://e/{class}> .\n"
        ));
        nt.push_str(&format!("<http://e/s{i}> <http://e/color> \"{color}\" .\n"));
        nt.push_str(&format!(
            "<http://e/s{i}> <http://e/size> \"{}\" .\n",
            i % 2
        ));
        nt.push_str(&format!("<http://e/s{i}> <http://e/tag> \"shared\" .\n"));
        nt.push_str(&format!("<http://e/s{i}> <http://e/tag> \"tag{i}\" .\n"));
    }
    Graph::load_str(&nt, "ntriples").unwrap()
}

fn find<'a>(items: &'a [Counted], value: &str) -> Option<&'a Counted> {
    items.iter().find(|item| item.iri == value)
}

#[test]
fn exact_whole_class_and_constrained_facets_with_elision() {
    let graph = fixture();
    let whole = facets(
        &graph,
        &FacetRequest {
            top_k: 10,
            ..Default::default()
        },
    );
    assert_eq!(whole.candidates, 6);
    assert_eq!(find(&whole.types, "http://e/A").unwrap().count, 4);
    assert_eq!(find(&whole.types, "http://e/B").unwrap().count, 2);
    assert_eq!(find(&whole.predicates, "http://e/tag").unwrap().count, 12);
    assert_eq!(find(&whole.predicates, RDF_TYPE).unwrap().count, 6);

    let class = facets(
        &graph,
        &FacetRequest {
            class: Some("http://e/A".into()),
            facet_predicates: Some(vec!["http://e/color".into()]),
            top_k: 1,
            ..Default::default()
        },
    );
    assert_eq!(class.candidates, 4);
    assert_eq!(
        class.types,
        vec![Counted {
            iri: "http://e/A".into(),
            count: 4
        }]
    );
    assert_eq!(
        class.values[0].values,
        vec![Counted {
            iri: "\"blue\"".into(),
            count: 2
        }]
    );
    assert_eq!(class.values[0].elided, 2);

    let constrained = facets(
        &graph,
        &FacetRequest {
            class: Some("http://e/A".into()),
            constraints: vec![("http://e/color".into(), "\"red\"".into())],
            top_k: 10,
            ..Default::default()
        },
    );
    assert_eq!(constrained.candidates, 2);
    assert_eq!(
        find(&constrained.predicates, "http://e/tag").unwrap().count,
        4
    );
}

fn aggregate(
    graph: &Graph,
    pattern: &str,
    group: &str,
    ntriples_terms: bool,
) -> BTreeMap<String, u64> {
    let query = format!("SELECT ?x (COUNT(*) AS ?c) WHERE {{ {pattern} }} GROUP BY {group}");
    sparq_engine::query(graph, &query)
        .unwrap()
        .rows
        .into_iter()
        .map(|row| {
            let key = row[0].as_ref().unwrap();
            let key = if ntriples_terms {
                key.to_string()
            } else {
                match key {
                    Term::NamedNode(node) => node.as_str().to_owned(),
                    other => other.to_string(),
                }
            };
            let Term::Literal(count) = row[1].as_ref().unwrap() else {
                panic!("COUNT is literal")
            };
            (key, count.value().parse().unwrap())
        })
        .collect()
}

fn assert_differential(graph: &Graph) {
    let response = facets(
        graph,
        &FacetRequest {
            top_k: usize::MAX,
            ..Default::default()
        },
    );
    let got_types: BTreeMap<_, _> = response
        .types
        .iter()
        .map(|v| (v.iri.clone(), v.count))
        .collect();
    let got_predicates: BTreeMap<_, _> = response
        .predicates
        .iter()
        .map(|v| (v.iri.clone(), v.count))
        .collect();
    assert_eq!(
        got_types,
        aggregate(graph, &format!("?s <{RDF_TYPE}> ?x"), "?x", false)
    );
    assert_eq!(got_predicates, aggregate(graph, "?s ?x ?o", "?x", false));
    for values in &response.values {
        let got: BTreeMap<_, _> = values
            .values
            .iter()
            .map(|v| (v.iri.clone(), v.count))
            .collect();
        let expected = aggregate(graph, &format!("?s <{}> ?x", values.predicate), "?x", true);
        assert_eq!(got, expected, "value distribution for {}", values.predicate);
    }
}

#[test]
fn distributions_match_sparql_group_by_on_fixture_and_seeded_graph() {
    assert_differential(&fixture());

    // Fixed LCG: deterministic coverage without adding rand to this opt-in crate.
    let mut state = 0x5eed_u64;
    let mut nt = String::new();
    for i in 0..50 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let bucket = state % 7;
        nt.push_str(&format!(
            "<http://r/s{i}> <{RDF_TYPE}> <http://r/C{}> .\n",
            bucket % 3
        ));
        nt.push_str(&format!("<http://r/s{i}> <http://r/p> \"v{}\" .\n", bucket));
        nt.push_str(&format!(
            "<http://r/s{i}> <http://r/q> <http://r/o{}> .\n",
            bucket % 5
        ));
        nt.push_str(&format!(
            "<http://r/s{i}> <http://r/n> \"{}\" .\n",
            state % 11
        ));
    }
    let random = Graph::load_str(&nt, "ntriples").unwrap();
    assert_eq!(random.len(), 200);
    assert_differential(&random);
}
