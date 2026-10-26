// sq-ywe8p — self-relative facet fast-path benchmark with a full-scan oracle.
use oxrdf::Term;
use sparq_core::{dict::Id, Graph};
use sparq_introspect::{facets, Counted, FacetRequest, FacetResponse, PredicateValues};
use std::collections::{BTreeMap, BTreeSet};
use std::hint::black_box;
use std::time::Instant;

const EX: &str = "http://example.com/";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

fn generated_graph(subjects: usize) -> Graph {
    let mut nt = String::with_capacity(subjects * 300);
    for i in 0..subjects {
        let class = if i % 3 == 0 { "Featured" } else { "Item" };
        let color = ["red", "green", "blue", "amber"][i % 4];
        let region = ["north", "south", "east"][i % 3];
        let subject = format!("<{EX}s{i}>");
        nt.push_str(&format!("{subject} <{RDF_TYPE}> <{EX}{class}> .\n"));
        nt.push_str(&format!("{subject} <{EX}color> \"{color}\" .\n"));
        nt.push_str(&format!("{subject} <{EX}region> \"{region}\" .\n"));
        nt.push_str(&format!("{subject} <{EX}bucket> \"{}\" .\n", i % 17));
        if i % 5 == 0 {
            nt.push_str(&format!("{subject} <{EX}tag> \"promoted\" .\n"));
        }
    }
    Graph::load_str(&nt, "ntriples").expect("generated N-Triples is valid")
}

fn scenarios() -> Vec<(&'static str, FacetRequest)> {
    vec![
        (
            "all",
            FacetRequest {
                top_k: 20,
                ..Default::default()
            },
        ),
        (
            "class",
            FacetRequest {
                class: Some(format!("{EX}Featured")),
                top_k: 20,
                ..Default::default()
            },
        ),
        (
            "filter",
            FacetRequest {
                constraints: vec![(format!("{EX}color"), "\"red\"".into())],
                top_k: 20,
                ..Default::default()
            },
        ),
        (
            "class_filter",
            FacetRequest {
                class: Some(format!("{EX}Item")),
                constraints: vec![(format!("{EX}region"), "\"south\"".into())],
                top_k: 20,
                ..Default::default()
            },
        ),
        (
            "selected_top_k",
            FacetRequest {
                facet_predicates: Some(vec![
                    format!("{EX}color"),
                    format!("{EX}bucket"),
                    format!("{EX}missing"),
                ]),
                top_k: 3,
                ..Default::default()
            },
        ),
    ]
}

fn ranked(map: BTreeMap<String, u64>, top_k: usize) -> (Vec<Counted>, u64) {
    let mut values: Vec<_> = map
        .into_iter()
        .map(|(iri, count)| Counted { iri, count })
        .collect();
    values.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.iri.cmp(&b.iri)));
    let elided = values.iter().skip(top_k).map(|v| v.count).sum();
    values.truncate(top_k);
    (values, elided)
}

/// Deliberately naive oracle: every filter and every aggregate traverses the full graph.
fn naive_facets(graph: &Graph, req: &FacetRequest) -> FacetResponse {
    let rows: Vec<[Id; 3]> = graph.iter_ids().collect();
    let mut candidates: BTreeSet<Id> = rows.iter().map(|row| row[0]).collect();
    if let Some(class) = &req.class {
        candidates.retain(|subject| {
            rows.iter().any(|row| {
                row[0] == *subject
                    && named_iri(&graph.dict().term(row[1])) == Some(RDF_TYPE)
                    && named_iri(&graph.dict().term(row[2])) == Some(class.as_str())
            })
        });
    }
    for (predicate, object) in &req.constraints {
        candidates.retain(|subject| {
            rows.iter().any(|row| {
                row[0] == *subject
                    && named_iri(&graph.dict().term(row[1])) == Some(predicate.as_str())
                    && graph.dict().term(row[2]).to_string() == *object
            })
        });
    }

    let requested = req
        .facet_predicates
        .as_ref()
        .map(|items| items.iter().cloned().collect::<BTreeSet<_>>());
    let mut type_counts = BTreeMap::new();
    let mut predicate_counts = BTreeMap::new();
    let mut value_counts: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    for row in &rows {
        if !candidates.contains(&row[0]) {
            continue;
        }
        let predicate_term = graph.dict().term(row[1]);
        let Some(predicate) = named_iri(&predicate_term) else {
            continue;
        };
        *predicate_counts.entry(predicate.to_owned()).or_insert(0) += 1;
        let object = graph.dict().term(row[2]);
        if predicate == RDF_TYPE {
            let value = named_iri(&object).map_or_else(|| object.to_string(), str::to_owned);
            *type_counts.entry(value).or_insert(0) += 1;
        }
        if requested.as_ref().is_none_or(|set| set.contains(predicate)) {
            *value_counts
                .entry(predicate.to_owned())
                .or_default()
                .entry(object.to_string())
                .or_insert(0) += 1;
        }
    }
    if let Some(predicates) = &req.facet_predicates {
        for predicate in predicates {
            value_counts.entry(predicate.clone()).or_default();
        }
    }
    let (types, _) = ranked(type_counts, req.top_k);
    let (predicates, _) = ranked(predicate_counts, req.top_k);
    let values = value_counts
        .into_iter()
        .map(|(predicate, counts)| {
            let (values, elided) = ranked(counts, req.top_k);
            PredicateValues {
                predicate,
                values,
                elided,
            }
        })
        .collect();
    FacetResponse {
        candidates: candidates.len() as u64,
        types,
        predicates,
        values,
        #[cfg(feature = "numeric-facets")]
        numeric: Vec::new(),
    }
}

fn named_iri(term: &Term) -> Option<&str> {
    match term {
        Term::NamedNode(node) => Some(node.as_str()),
        _ => None,
    }
}

fn timed<F: FnMut() -> FacetResponse>(mut f: F) -> (FacetResponse, u128) {
    let start = Instant::now();
    let result = black_box(f());
    (result, start.elapsed().as_micros())
}

fn main() {
    let smoke = std::env::args().skip(1).any(|arg| arg == "--smoke");
    let graph = generated_graph(if smoke { 120 } else { 20_000 });
    println!("# scenario\timplementation\tcount\tus");
    for (name, request) in scenarios() {
        // Equality is checked before either timing row can be emitted.
        let expected = naive_facets(&graph, &request);
        let actual = facets(&graph, &request);
        assert_eq!(actual, expected, "facet distribution mismatch in {name}");
        let (fast, fast_us) = timed(|| facets(&graph, &request));
        let (naive, naive_us) = timed(|| naive_facets(&graph, &request));
        assert_eq!(fast, naive, "timed facet distribution mismatch in {name}");
        println!("{name}\tfast\t{}\t{fast_us}", fast.candidates);
        println!("{name}\tnaive\t{}\t{naive_us}", naive.candidates);
    }
    println!("FACET_BENCH_ENVELOPE {{\"canonical\":false,\"dataset\":\"deterministic-generated\",\"subjects\":{},\"equality_gate\":\"count-and-distribution\",\"smoke\":{smoke}}}", if smoke { 120 } else { 20_000 });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oracle_witnesses_filter_and_distribution_changes() {
        let graph = generated_graph(24);
        for (name, request) in scenarios() {
            assert_eq!(
                facets(&graph, &request),
                naive_facets(&graph, &request),
                "{name}"
            );
        }
        let mut changed = naive_facets(&graph, &scenarios()[2].1);
        changed.candidates += 1;
        assert_ne!(facets(&graph, &scenarios()[2].1), changed);
    }
}
