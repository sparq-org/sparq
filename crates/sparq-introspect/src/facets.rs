//! Facet distributions over a filtered set of RDF subjects.

use crate::{Counted, RDF_TYPE};
use oxrdf::{NamedNode, Term};
use serde::{Deserialize, Serialize};
use sparq_core::dict::Id;
use sparq_core::store::Perm;
use sparq_core::Graph;
use std::collections::{BTreeMap, BTreeSet};

/// Filters and output bounds for [`facets`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FacetRequest {
    /// Optional `rdf:type` class IRI restricting candidate subjects.
    pub class: Option<String>,
    /// `(predicate IRI, object N-Triples term)` filters, AND-combined.
    pub constraints: Vec<(String, String)>,
    /// Predicates whose value distributions are requested; `None` means every
    /// predicate occurring on a candidate subject.
    pub facet_predicates: Option<Vec<String>>,
    /// Maximum entries retained in each distribution.
    pub top_k: usize,
}

/// The retained values for one predicate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PredicateValues {
    pub predicate: String,
    pub values: Vec<Counted>,
    /// Total occurrences represented by values omitted by `top_k`.
    pub elided: u64,
}

/// One equal-width interval in a numeric facet histogram.
///
/// Buckets are half-open (`[lo, hi)`) except for the final bucket, which includes
/// `hi`. A constant-valued facet has ten zero-width buckets and places every value
/// in the final, closed bucket.
#[cfg(feature = "numeric-facets")]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NumericBucket {
    pub lo: f64,
    pub hi: f64,
    pub count: u64,
}

/// Numeric range and fixed ten-bucket histogram for one predicate.
///
/// `count` includes finite, well-formed XSD numeric literal occurrences only.
/// Non-numeric, ill-formed, NaN, and infinite objects remain in the ordinary value
/// distribution but are excluded here so `min`, `max`, and every bucket edge stay
/// finite and ordered.
#[cfg(feature = "numeric-facets")]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NumericFacet {
    pub predicate: String,
    pub min: f64,
    pub max: f64,
    pub count: u64,
    pub buckets: Vec<NumericBucket>,
}

/// Facet distributions for the request's candidate subjects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(not(feature = "numeric-facets"), derive(Eq))]
pub struct FacetResponse {
    pub candidates: u64,
    pub types: Vec<Counted>,
    pub predicates: Vec<Counted>,
    pub values: Vec<PredicateValues>,
    /// Numeric summaries in predicate-IRI order. Present only with the
    /// `numeric-facets` feature; predicates without finite numeric values are omitted.
    #[cfg(feature = "numeric-facets")]
    #[serde(default)]
    pub numeric: Vec<NumericFacet>,
}

impl FacetResponse {
    /// Serialises this response as pretty JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("facet response serialises to JSON")
    }
}

fn iri_id(graph: &Graph, iri: &str) -> Option<Id> {
    let node = NamedNode::new(iri).ok()?;
    graph.id_of(&Term::NamedNode(node))
}

fn object_id(graph: &Graph, nt: &str) -> Option<Id> {
    // Parse through sparq-core so constraint syntax has exactly the same
    // RDF 1.2/N-Triples semantics as data loading, including escaped literals.
    let line = format!("<urn:sparq:facet:s> <urn:sparq:facet:p> {nt} .");
    let parsed = Graph::load_str(&line, "ntriples").ok()?;
    if parsed.len() != 1 {
        return None;
    }
    let object = parsed
        .iter_ids()
        .next()
        .map(|row| parsed.dict().term(row[2]))?;
    graph.id_of(&object)
}

fn matching_subjects(graph: &Graph, predicate: Id, object: Id) -> Vec<Id> {
    let pattern = [None, Some(predicate), Some(object)];
    let Some(scan) = graph.store().scan_perm(&pattern, Perm::Pos) else {
        return Vec::new();
    };
    scan.rows.iter().map(|row| scan.to_spo(row)[0]).collect()
}

fn intersect(left: Vec<Id>, right: Vec<Id>) -> Vec<Id> {
    let mut out = Vec::with_capacity(left.len().min(right.len()));
    let (mut a, mut b) = (0, 0);
    while a < left.len() && b < right.len() {
        match left[a].cmp(&right[b]) {
            std::cmp::Ordering::Less => a += 1,
            std::cmp::Ordering::Greater => b += 1,
            std::cmp::Ordering::Equal => {
                out.push(left[a]);
                a += 1;
                b += 1;
            }
        }
    }
    out
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

#[cfg(feature = "numeric-facets")]
const NUMERIC_BUCKET_COUNT: usize = 10;

#[cfg(feature = "numeric-facets")]
fn numeric_facet(predicate: String, values: Vec<f64>) -> NumericFacet {
    debug_assert!(!values.is_empty());
    debug_assert!(values.iter().all(|value| value.is_finite()));

    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = max - min;
    let mut edges = Vec::with_capacity(NUMERIC_BUCKET_COUNT + 1);
    for index in 0..=NUMERIC_BUCKET_COUNT {
        let fraction = index as f64 / NUMERIC_BUCKET_COUNT as f64;
        let edge = if span.is_finite() {
            min + span * fraction
        } else {
            // Opposite-sign finite extrema can overflow `max - min`.
            // Convex interpolation keeps every interior edge finite and ordered.
            min * (1.0 - fraction) + max * fraction
        };
        edges.push(edge);
    }
    // Avoid arithmetic drift at the two closed range endpoints.
    edges[0] = min;
    edges[NUMERIC_BUCKET_COUNT] = max;

    let mut counts = [0_u64; NUMERIC_BUCKET_COUNT];
    if min == max {
        counts[NUMERIC_BUCKET_COUNT - 1] = values.len() as u64;
    } else {
        for value in &values {
            let bucket = if *value == max {
                NUMERIC_BUCKET_COUNT - 1
            } else {
                edges
                    .partition_point(|edge| *edge <= *value)
                    .saturating_sub(1)
            };
            counts[bucket.min(NUMERIC_BUCKET_COUNT - 1)] += 1;
        }
    }

    let buckets = counts
        .into_iter()
        .enumerate()
        .map(|(index, count)| NumericBucket {
            lo: edges[index],
            hi: edges[index + 1],
            count,
        })
        .collect();
    NumericFacet {
        predicate,
        min,
        max,
        count: values.len() as u64,
        buckets,
    }
}

/// Computes type, predicate, and object-value distributions for subjects matching
/// all request filters. Invalid or absent filter terms produce an empty candidate set.
pub fn facets(graph: &Graph, req: &FacetRequest) -> FacetResponse {
    let rdf_type = iri_id(graph, RDF_TYPE);
    let mut filters = Vec::with_capacity(req.constraints.len() + usize::from(req.class.is_some()));
    if let Some(class) = &req.class {
        filters.push(rdf_type.zip(iri_id(graph, class)));
    }
    for (predicate, object) in &req.constraints {
        filters.push(iri_id(graph, predicate).zip(object_id(graph, object)));
    }

    let mut candidates = if filters.is_empty() {
        let mut subjects = Vec::new();
        let mut previous = None;
        for [subject, _, _] in graph.iter_ids() {
            if previous != Some(subject) {
                subjects.push(subject);
                previous = Some(subject);
            }
        }
        subjects
    } else {
        let Some((predicate, object)) = filters[0] else {
            return empty_response();
        };
        matching_subjects(graph, predicate, object)
    };
    for filter in filters.into_iter().skip(1) {
        let Some((predicate, object)) = filter else {
            return empty_response();
        };
        candidates = intersect(candidates, matching_subjects(graph, predicate, object));
    }

    let requested: Option<BTreeSet<Id>> = req
        .facet_predicates
        .as_ref()
        .map(|predicates| predicates.iter().filter_map(|p| iri_id(graph, p)).collect());
    let mut type_counts = BTreeMap::new();
    let mut predicate_counts = BTreeMap::new();
    let mut value_counts: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    #[cfg(feature = "numeric-facets")]
    let mut numeric_values: BTreeMap<String, Vec<f64>> = BTreeMap::new();

    for subject in &candidates {
        let scan = graph.store().scan(&[Some(*subject), None, None]);
        for row in scan.rows.iter() {
            let [_, predicate, object] = scan.to_spo(row);
            let predicate_term = graph.dict().term(predicate);
            let Term::NamedNode(predicate_node) = predicate_term else {
                continue;
            };
            let predicate_iri = predicate_node.as_str().to_owned();
            *predicate_counts.entry(predicate_iri.clone()).or_insert(0) += 1;
            if Some(predicate) == rdf_type {
                let term = graph.dict().term(object);
                let value = match term {
                    Term::NamedNode(node) => node.as_str().to_owned(),
                    other => other.to_string(),
                };
                *type_counts.entry(value).or_insert(0) += 1;
            }
            if requested
                .as_ref()
                .is_none_or(|set| set.contains(&predicate))
            {
                #[cfg(feature = "numeric-facets")]
                let numeric_predicate_iri = predicate_iri.clone();
                let value = graph.dict().term(object).to_string();
                *value_counts
                    .entry(predicate_iri)
                    .or_default()
                    .entry(value)
                    .or_insert(0) += 1;
                #[cfg(feature = "numeric-facets")]
                if let Some(value) = graph
                    .numeric_value(object)
                    .filter(|value| value.is_finite())
                {
                    numeric_values
                        .entry(numeric_predicate_iri)
                        .or_default()
                        .push(value);
                }
            }
        }
    }

    // Preserve explicitly requested, valid predicates even when their distribution is empty.
    if let Some(predicates) = &req.facet_predicates {
        for predicate in predicates {
            if NamedNode::new(predicate).is_ok() {
                value_counts.entry(predicate.clone()).or_default();
            }
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
    #[cfg(feature = "numeric-facets")]
    let numeric = numeric_values
        .into_iter()
        .map(|(predicate, values)| numeric_facet(predicate, values))
        .collect();
    FacetResponse {
        candidates: candidates.len() as u64,
        types,
        predicates,
        values,
        #[cfg(feature = "numeric-facets")]
        numeric,
    }
}

fn empty_response() -> FacetResponse {
    FacetResponse {
        candidates: 0,
        types: Vec::new(),
        predicates: Vec::new(),
        values: Vec::new(),
        #[cfg(feature = "numeric-facets")]
        numeric: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facets_counts_a_filtered_subject_directly() {
        let graph = Graph::load_str(
            "<http://e/a> <http://e/p> \"x\" .\n<http://e/b> <http://e/p> \"y\" .\n",
            "ntriples",
        )
        .unwrap();
        let response = facets(
            &graph,
            &FacetRequest {
                constraints: vec![("http://e/p".into(), "\"x\"".into())],
                top_k: 10,
                ..FacetRequest::default()
            },
        );
        assert_eq!(response.candidates, 1);
        assert_eq!(
            response.predicates,
            vec![Counted {
                iri: "http://e/p".into(),
                count: 1
            }]
        );
        assert!(response.to_json().contains("candidates"));
        #[cfg(not(feature = "numeric-facets"))]
        assert!(!response.to_json().contains("\"numeric\""));
    }
}
