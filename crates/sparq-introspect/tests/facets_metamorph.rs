use sparq_core::Graph;
use sparq_introspect::{facets, Counted, FacetRequest, FacetResponse};
use std::collections::{BTreeMap, BTreeSet};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const PREDICATES: [&str; 4] = ["http://e/color", "http://e/size", "http://e/tag", RDF_TYPE];

// A small deterministic generator exercises the algebraic laws without
// adding a random-number dependency to this opt-in crate.
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

fn generated_graph(seed: u64) -> Graph {
    let mut rng = Lcg(seed);
    let mut nt = String::new();
    for subject in 0..12 {
        let class = rng.next() % 3;
        let color = rng.next() % 3;
        let size = rng.next() % 2;
        let tag = rng.next() % 4;
        nt.push_str(&format!(
            "<http://e/s{subject}> <{RDF_TYPE}> <http://e/C{class}> .\n"
        ));
        nt.push_str(&format!(
            "<http://e/s{subject}> <http://e/color> \"c{color}\" .\n"
        ));
        nt.push_str(&format!(
            "<http://e/s{subject}> <http://e/size> \"{size}\" .\n"
        ));
        nt.push_str(&format!(
            "<http://e/s{subject}> <http://e/tag> <http://e/t{tag}> .\n"
        ));
    }
    Graph::load_str(&nt, "ntriples").unwrap()
}

fn counts(items: &[Counted]) -> BTreeMap<&str, u64> {
    items
        .iter()
        .map(|item| (item.iri.as_str(), item.count))
        .collect()
}

fn value_counts(response: &FacetResponse) -> BTreeMap<(&str, &str), u64> {
    response
        .values
        .iter()
        .flat_map(|distribution| {
            distribution.values.iter().map(|value| {
                (
                    (distribution.predicate.as_str(), value.iri.as_str()),
                    value.count,
                )
            })
        })
        .collect()
}

fn assert_counts_do_not_grow(less: &FacetResponse, more: &FacetResponse) {
    assert!(more.candidates <= less.candidates);
    let less_types = counts(&less.types);
    for (key, count) in counts(&more.types) {
        assert!(count <= less_types.get(key).copied().unwrap_or(0));
    }
    let less_predicates = counts(&less.predicates);
    for (key, count) in counts(&more.predicates) {
        assert!(count <= less_predicates.get(key).copied().unwrap_or(0));
    }
    let less_values = value_counts(less);
    for (key, count) in value_counts(more) {
        assert!(count <= less_values.get(&key).copied().unwrap_or(0));
    }
}

#[test]
fn adding_a_constraint_never_increases_facet_counts() {
    for seed in 0..24 {
        let graph = generated_graph(seed);
        for color in 0..3 {
            for size in 0..2 {
                let first = ("http://e/color".into(), format!("\"c{color}\""));
                let second = ("http://e/size".into(), format!("\"{size}\""));
                let less = facets(
                    &graph,
                    &FacetRequest {
                        constraints: vec![first.clone()],
                        top_k: usize::MAX,
                        ..Default::default()
                    },
                );
                let more = facets(
                    &graph,
                    &FacetRequest {
                        constraints: vec![first, second],
                        top_k: usize::MAX,
                        ..Default::default()
                    },
                );
                assert_counts_do_not_grow(&less, &more);
            }
        }
    }
}

#[test]
fn constraint_order_is_commutative() {
    for seed in 24..48 {
        let graph = generated_graph(seed);
        for color in 0..3 {
            for size in 0..2 {
                let a = ("http://e/color".into(), format!("\"c{color}\""));
                let b = ("http://e/size".into(), format!("\"{size}\""));
                let request = |constraints| FacetRequest {
                    constraints,
                    top_k: usize::MAX,
                    ..Default::default()
                };
                assert_eq!(
                    facets(&graph, &request(vec![a.clone(), b.clone()])).to_json(),
                    facets(&graph, &request(vec![b, a])).to_json()
                );
            }
        }
    }
}

#[test]
fn facet_value_predicates_respect_requested_or_graph_predicate_set() {
    for seed in 48..72 {
        let graph = generated_graph(seed);
        let requested: BTreeSet<_> = ["http://e/color", "http://e/tag"].into_iter().collect();
        let restricted = facets(
            &graph,
            &FacetRequest {
                facet_predicates: Some(requested.iter().map(ToString::to_string).collect()),
                top_k: usize::MAX,
                ..Default::default()
            },
        );
        assert!(restricted
            .values
            .iter()
            .all(|values| requested.contains(values.predicate.as_str())));

        let unrestricted = facets(
            &graph,
            &FacetRequest {
                top_k: usize::MAX,
                ..Default::default()
            },
        );
        let graph_predicates: BTreeSet<_> = PREDICATES.into_iter().collect();
        assert!(unrestricted
            .values
            .iter()
            .all(|values| graph_predicates.contains(values.predicate.as_str())));
    }
}

#[test]
fn absent_constraint_fails_closed_to_empty_response() {
    for seed in 72..96 {
        let graph = generated_graph(seed);
        let response = facets(
            &graph,
            &FacetRequest {
                constraints: vec![(
                    "http://e/absent-predicate".into(),
                    "<http://e/absent-object>".into(),
                )],
                top_k: usize::MAX,
                ..Default::default()
            },
        );
        assert_eq!(
            response,
            FacetResponse {
                candidates: 0,
                types: Vec::new(),
                predicates: Vec::new(),
                values: Vec::new(),
                #[cfg(feature = "numeric-facets")]
                numeric: Vec::new(),
            }
        );
    }
}
