//! Full-path enumeration over an RDF predicate or materialized graph pattern.

use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, Term, Variable};
use spargebra::algebra::GraphPattern;
use spargebra::{Query, SparqlParser};
use sparq_core::{dict::Id, store::Perm, Graph};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Selects shortest-path or bounded all-path enumeration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathMode {
    /// Return every path at the minimum length for each selected endpoint pair.
    Shortest,
    /// Return every path up to `PathSpec::max_length`.
    All,
}

/// Defines the directed edge relation used for path enumeration.
///
/// [GPT-5.6] `sq-lsp7k.3.3`: pattern relations are materialized once, then
/// consumed by the same deterministic T1 enumerator as predicate relations.
#[derive(Clone, Debug)]
pub enum Via {
    /// Every triple using this predicate is one directed edge.
    Predicate(NamedNode),
    /// A SPARQL group graph pattern whose reserved `?from` and `?to` variables
    /// designate each directed edge's endpoints.
    Pattern(String),
}

/// Selects the candidate nodes at one end of a path.
///
/// [SONNET-4.6] `sq-lsp7k.3`: endpoint patterns are materialized once into a
/// deterministic id set, then drive the same enumerator as a fixed node.
#[derive(Clone, Debug)]
pub enum Endpoint {
    /// Exactly one fixed node.
    Node(Term),
    /// Every node the group graph pattern binds to `variable`. Solutions leaving
    /// `variable` unbound, or binding a term absent from the graph, contribute
    /// no candidate.
    Pattern {
        /// Group graph pattern body, without the enclosing braces.
        source: String,
        /// The pattern variable whose bindings are the candidate nodes.
        variable: Variable,
    },
}

/// Parameters for path enumeration over an edge relation.
#[derive(Clone, Debug)]
pub struct PathSpec {
    /// Enumeration mode.
    pub mode: PathMode,
    /// If true, return only non-empty paths whose first and last nodes are equal.
    pub cyclic: bool,
    /// Optional starting-node restriction; otherwise every edge subject is considered.
    pub start: Option<Endpoint>,
    /// Optional ending-node restriction; otherwise every edge object is considered.
    pub end: Option<Endpoint>,
    /// Predicate or graph pattern defining the directed edge relation.
    pub via: Via,
    /// Maximum edge count. Required for [`PathMode::All`].
    pub max_length: Option<usize>,
}

/// One materialized path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathSolution {
    /// Path nodes, including both endpoints.
    pub nodes: Vec<Term>,
    /// Edge terms, one fewer than `nodes`. Pattern edges use the pattern source
    /// as a canonical, blank-node-free `xsd:string` marker.
    pub edges: Vec<Term>,
}

/// Enumerates paths selected by `spec` in deterministic `(length, node ids)` order.
///
/// `All` paths are simple unless `cyclic` is true, in which case the only repeated
/// node is the final return to the start. An unbounded `All` request is rejected.
pub fn enumerate_paths(graph: &Graph, spec: &PathSpec) -> Result<Vec<PathSolution>, String> {
    if spec.mode == PathMode::All && spec.max_length.is_none() {
        return Err("PATHS ALL requires MAX LENGTH".to_owned());
    }

    let (edge_pairs, edge) = edge_relation(graph, &spec.via)?;
    let mut adjacency: BTreeMap<Id, Vec<Id>> = BTreeMap::new();
    let mut sinks = BTreeSet::new();
    for (from, to) in edge_pairs {
        adjacency.entry(from).or_default().push(to);
        sinks.insert(to);
    }
    for next in adjacency.values_mut() {
        next.sort_unstable();
        next.dedup();
    }

    let starts = endpoint_ids(graph, spec.start.as_ref(), adjacency.keys().copied())?;
    let ends = endpoint_ids(graph, spec.end.as_ref(), sinks.iter().copied())?;
    let mut paths = Vec::new();
    if spec.cyclic {
        for start in starts {
            if spec.end.as_ref().is_some() && !ends.contains(&start) {
                continue;
            }
            enumerate_pair(&adjacency, start, start, spec, &mut paths);
        }
    } else {
        for start in starts {
            for &end in &ends {
                if start != end {
                    enumerate_pair(&adjacency, start, end, spec, &mut paths);
                }
            }
        }
    }

    paths.sort_by(|left, right| left.len().cmp(&right.len()).then_with(|| left.cmp(right)));
    Ok(paths
        .into_iter()
        .map(|ids| PathSolution {
            edges: vec![edge.clone(); ids.len() - 1],
            nodes: ids.into_iter().map(|id| graph.dict.term(id)).collect(),
        })
        .collect())
}

fn edge_relation(graph: &Graph, via: &Via) -> Result<(Vec<(Id, Id)>, Term), String> {
    match via {
        Via::Predicate(predicate) => {
            let edge = Term::NamedNode(predicate.clone());
            let Some(via_id) = graph.id_of(&edge) else {
                return Ok((Vec::new(), edge));
            };
            let scan = graph
                .store
                .scan_perm(&[None, Some(via_id), None], Perm::Pso)
                .or_else(|| {
                    graph
                        .store
                        .scan_perm(&[None, Some(via_id), None], Perm::Pos)
                })
                .expect("a predicate-leading permutation is always built");
            let pairs = scan
                .rows
                .iter()
                .map(|row| {
                    let [subject, _, object] = scan.to_spo(row);
                    (subject, object)
                })
                .collect();
            Ok((pairs, edge))
        }
        Via::Pattern(pattern) => {
            let source = format!("SELECT ?from ?to WHERE {{ {pattern} }}");
            let parsed = SparqlParser::new()
                .parse_query(&source)
                .map_err(|error| format!("invalid PATHS VIA pattern: {error}"))?;
            let Query::Select {
                pattern: GraphPattern::Project { inner, .. },
                ..
            } = parsed
            else {
                return Err("invalid SELECT projection for PATHS VIA pattern".to_owned());
            };
            let mut binds_from = false;
            let mut binds_to = false;
            inner.on_in_scope_variable(|variable| match variable.as_str() {
                "from" => binds_from = true,
                "to" => binds_to = true,
                _ => {}
            });
            if !binds_from {
                return Err("PATHS VIA pattern must bind ?from".to_owned());
            }
            if !binds_to {
                return Err("PATHS VIA pattern must bind ?to".to_owned());
            }
            let result = crate::query(graph, &source)?;
            let from = result
                .vars
                .iter()
                .position(|var| var.as_str() == "from")
                .ok_or_else(|| "PATHS VIA pattern must bind ?from".to_owned())?;
            let to = result
                .vars
                .iter()
                .position(|var| var.as_str() == "to")
                .ok_or_else(|| "PATHS VIA pattern must bind ?to".to_owned())?;
            let mut pairs = Vec::with_capacity(result.rows.len());
            for row in result.rows {
                let from_term = row[from].as_ref().ok_or_else(|| {
                    "PATHS VIA pattern must bind ?from in every solution".to_owned()
                })?;
                let to_term = row[to].as_ref().ok_or_else(|| {
                    "PATHS VIA pattern must bind ?to in every solution".to_owned()
                })?;
                let Some(from_id) = graph.id_of(from_term) else {
                    continue;
                };
                let Some(to_id) = graph.id_of(to_term) else {
                    continue;
                };
                pairs.push((from_id, to_id));
            }
            let marker = Term::Literal(Literal::new_typed_literal(pattern.clone(), xsd::STRING));
            Ok((pairs, marker))
        }
    }
}

fn endpoint_ids(
    graph: &Graph,
    endpoint: Option<&Endpoint>,
    all: impl Iterator<Item = Id>,
) -> Result<Vec<Id>, String> {
    match endpoint {
        None => Ok(all.collect()),
        Some(Endpoint::Node(term)) => Ok(graph.id_of(term).into_iter().collect()),
        Some(Endpoint::Pattern { source, variable }) => {
            endpoint_pattern_ids(graph, source, variable)
        }
    }
}

/// Materializes an endpoint pattern into the sorted, deduplicated id set it selects.
fn endpoint_pattern_ids(
    graph: &Graph,
    source: &str,
    variable: &Variable,
) -> Result<Vec<Id>, String> {
    let name = variable.as_str();
    let select = format!("SELECT ?{} WHERE {{ {} }}", name, source);
    let parsed = SparqlParser::new()
        .parse_query(&select)
        .map_err(|error| format!("invalid PATHS endpoint pattern: {}", error))?;
    let Query::Select {
        pattern: GraphPattern::Project { inner, .. },
        ..
    } = parsed
    else {
        return Err("invalid SELECT projection for PATHS endpoint pattern".to_owned());
    };
    let mut binds = false;
    inner.on_in_scope_variable(|in_scope| {
        if in_scope.as_str() == name {
            binds = true;
        }
    });
    if !binds {
        return Err(format!("PATHS endpoint pattern must bind ?{}", name));
    }
    let result = crate::query(graph, &select)?;
    let column = result
        .vars
        .iter()
        .position(|var| var.as_str() == name)
        .ok_or_else(|| format!("PATHS endpoint pattern must bind ?{}", name))?;
    let mut ids = BTreeSet::new();
    for row in result.rows {
        let Some(term) = row[column].as_ref() else {
            continue;
        };
        if let Some(id) = graph.id_of(term) {
            ids.insert(id);
        }
    }
    Ok(ids.into_iter().collect())
}

fn enumerate_pair(
    adjacency: &BTreeMap<Id, Vec<Id>>,
    start: Id,
    end: Id,
    spec: &PathSpec,
    output: &mut Vec<Vec<Id>>,
) {
    match spec.mode {
        PathMode::Shortest => shortest(adjacency, start, end, output),
        PathMode::All => {
            let mut path = vec![start];
            all_paths(
                adjacency,
                end,
                spec.max_length.expect("validated above"),
                &mut path,
                output,
            );
        }
    }
}

fn shortest(adjacency: &BTreeMap<Id, Vec<Id>>, start: Id, end: Id, output: &mut Vec<Vec<Id>>) {
    let mut queue = VecDeque::from([vec![start]]);
    let mut found_length = None;
    while let Some(path) = queue.pop_front() {
        let edges = path.len() - 1;
        if found_length.is_some_and(|minimum| edges >= minimum) {
            continue;
        }
        let current = *path.last().expect("paths are non-empty");
        let Some(nexts) = adjacency.get(&current) else {
            continue;
        };
        for &next in nexts {
            if next == end {
                let mut found = path.clone();
                found.push(next);
                found_length = Some(edges + 1);
                output.push(found);
            } else if !path.contains(&next) {
                let mut extended = path.clone();
                extended.push(next);
                queue.push_back(extended);
            }
        }
    }
}

fn all_paths(
    adjacency: &BTreeMap<Id, Vec<Id>>,
    end: Id,
    maximum: usize,
    path: &mut Vec<Id>,
    output: &mut Vec<Vec<Id>>,
) {
    if path.len() > maximum {
        return;
    }
    let current = *path.last().expect("paths are non-empty");
    let Some(nexts) = adjacency.get(&current) else {
        return;
    };
    for &next in nexts {
        if next == end {
            path.push(next);
            output.push(path.clone());
            path.pop();
        } else if !path.contains(&next) {
            path.push(next);
            all_paths(adjacency, end, maximum, path, output);
            path.pop();
        }
    }
}
