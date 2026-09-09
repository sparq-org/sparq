//! The zk-trace consumer (plan §4.E, module (i) — skeleton).
//!
//! Drives a SELECT/ASK over the **merge of a set of named graphs** with the
//! engine's `zk` trace seam armed, then resolves the recorded per-pattern
//! input sets to **leaf indices into the per-graph commitment orderings**
//! (the witness references the circuits consume), and enforces the Q6
//! prover-side guard: plans that join bnode-valued variables across graph
//! boundaries are rejected, failing closed at witness-build time.
//!
//! Fragment honesty (skeleton): supported queries are SELECT/ASK over
//! BGP + FILTER (the stage-1 derived-credential fragment). `GRAPH ?g` is
//! contradictory with graph-set privacy and excluded by design (plan §2.4);
//! non-monotone operators and property paths are later deliverables.

use crate::commit::{commit_triples, CommitError, GraphCommitment};
use crate::field::Fr;
use oxrdf::{BlankNode, NamedNode, Term, Triple};
use sparq_core::dict::Dict;
use sparq_core::Graph;
use sparq_engine::zk::{PatternKey, SlotPattern, ZkTrace};
use sparq_engine::QueryResult;
use std::collections::{HashMap, HashSet};

/// Errors of the traced-execution / resolution pipeline.
#[derive(Debug)]
pub enum TraceError {
    /// The named graph was not found in the store.
    MissingGraph(String),
    /// Commitment pipeline failure for a graph.
    Commit(String, CommitError),
    /// Query evaluation failed.
    Query(String),
    /// Soundness violation: a traced triple is not attributable to any
    /// queried graph (must be impossible for scans over the union).
    UnattributableTriple(String),
    /// Q6 guard: the executed plan joins a bnode-valued variable across
    /// graph boundaries (plan §2.4, layer 2). Fail closed.
    CrossGraphBnodeJoin {
        variable: String,
        bnode: String,
        graphs: (String, String),
    },
}

impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TraceError::MissingGraph(g) => write!(f, "named graph not in store: {g}"),
            TraceError::Commit(g, e) => write!(f, "commitment failed for {g}: {e}"),
            TraceError::Query(e) => write!(f, "query failed: {e}"),
            TraceError::UnattributableTriple(t) => {
                write!(f, "traced triple not in any queried graph: {t}")
            }
            TraceError::CrossGraphBnodeJoin { variable, bnode, graphs } => write!(
                f,
                "rejected: variable ?{variable} joins blank node _:{bnode} across graph boundaries ({} vs {}) — \
                 cross-graph bnode identity violates RDF merge semantics (Q6); skolemize at issuance if intended",
                graphs.0, graphs.1
            ),
        }
    }
}

impl std::error::Error for TraceError {}

/// One committed named graph of the queried union.
pub struct CommittedNamedGraph {
    pub name: NamedNode,
    /// The graph's content triples as stored (input bnode labels).
    pub triples: Vec<Triple>,
    /// The commitment (canonical order = leaf order).
    pub commitment: GraphCommitment,
    /// RDFC10 issued-identifier map: input bnode label → canonical label.
    label_map: HashMap<String, String>,
    /// Canonical triple → leaf index.
    leaf_of: HashMap<Triple, usize>,
}

impl CommittedNamedGraph {
    fn build(name: NamedNode, triples: Vec<Triple>, salt: Fr) -> Result<Self, TraceError> {
        let commitment = commit_triples(&triples, salt)
            .map_err(|e| TraceError::Commit(name.as_str().to_string(), e))?;
        let label_map = issue_label_map(&triples)
            .map_err(|e| TraceError::Commit(name.as_str().to_string(), CommitError::Canon(e)))?;
        let leaf_of = commitment
            .canonical
            .triples
            .iter()
            .enumerate()
            .map(|(i, t)| (t.clone(), i))
            .collect();
        Ok(CommittedNamedGraph { name, triples, commitment, label_map, leaf_of })
    }

    /// Leaf index of a store-form triple (input labels), via the issued
    /// identifier map. `None` when the triple is not in this graph.
    pub fn leaf_index(&self, t: &Triple) -> Option<usize> {
        let canonical = relabel_triple(t, &self.label_map)?;
        self.leaf_of.get(&canonical).copied()
    }
}

/// RDFC-1.0 issued-identifier map for a graph's content (input label →
/// canonical `c14nN` label). Single-sourced via the [`sparq_canon`] public API
/// (same bridge as [`crate::canon`]). [OPUS-4.8] sq-0qip.
fn issue_label_map(triples: &[Triple]) -> Result<HashMap<String, String>, crate::canon::CanonError> {
    sparq_canon::issue_triples(triples)
}

/// Rewrites a triple's bnode labels through the issued-identifier map
/// (`None` when a label is unknown to the map — the triple cannot belong to
/// the graph the map was issued for).
fn relabel_triple(t: &Triple, map: &HashMap<String, String>) -> Option<Triple> {
    use oxrdf::NamedOrBlankNode;
    let relabel_bnode = |b: &BlankNode| -> Option<BlankNode> {
        map.get(b.as_str()).map(|l| BlankNode::new_unchecked(l.clone()))
    };
    let subject = match &t.subject {
        NamedOrBlankNode::BlankNode(b) => NamedOrBlankNode::BlankNode(relabel_bnode(b)?),
        s => s.clone(),
    };
    let object = match &t.object {
        Term::BlankNode(b) => Term::BlankNode(relabel_bnode(b)?),
        o => o.clone(),
    };
    Some(Triple::new(subject, t.predicate.clone(), object))
}

/// A leaf reference: (graph index into [`ZkDataset::graphs`], leaf index in
/// that graph's canonical commitment ordering).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LeafRef {
    pub graph: usize,
    pub leaf: usize,
}

/// The resolved input set of one BGP triple pattern.
#[derive(Debug, Clone)]
pub struct ResolvedPattern {
    pub pattern: PatternKey,
    /// For each matched triple (same order as the trace's `triples`): every
    /// (graph, leaf) the triple is attributable to. Multiple entries mean
    /// the triple occurs in several queried graphs; the prover may use any.
    pub leaves: Vec<Vec<LeafRef>>,
}

/// A queried union of committed named graphs, ready for traced execution.
pub struct ZkDataset {
    pub graphs: Vec<CommittedNamedGraph>,
    /// The RDF merge of the content graphs, evaluated as the default graph
    /// (plan §2.4: the merge is presented to the query as the default graph).
    union: Graph,
}

/// The output of one traced execution: the result, the raw engine trace, and
/// the per-obligation input sets resolved to commitment leaf references.
#[derive(Debug)]
pub struct TracedExecution {
    pub result: QueryResult,
    pub trace: ZkTrace,
    pub patterns: Vec<ResolvedPattern>,
}

impl TracedExecution {
    /// The public per-pattern graph attributions for the manifest, in trace
    /// pattern order: for each traced pattern, the union of graph indices
    /// its matched triples are attributable to. This is the prover-side
    /// input to [`crate::verify::cross_graph_join_obligations`] — the
    /// verifier recomputes the required obligations from the query text and
    /// these (public) sets, independent of the trace itself.
    pub fn attributions(&self) -> Vec<(PatternKey, std::collections::BTreeSet<usize>)> {
        self.patterns
            .iter()
            .map(|rp| {
                let set = rp
                    .leaves
                    .iter()
                    .flat_map(|refs| refs.iter().map(|r| r.graph))
                    .collect();
                (rp.pattern.clone(), set)
            })
            .collect()
    }
}

impl ZkDataset {
    /// Builds the queried union from named graphs of `store`, committing
    /// each under its salt. `names` and `salts` are parallel slices.
    pub fn from_store(store: &Graph, names: &[NamedNode], salts: &[Fr]) -> Result<Self, TraceError> {
        assert_eq!(names.len(), salts.len(), "names and salts must be parallel");
        let mut graphs = Vec::with_capacity(names.len());
        for (name, salt) in names.iter().zip(salts) {
            let gname = Term::NamedNode(name.clone());
            let Some((_, g)) = store.named.iter().find(|(n, _)| *n == gname) else {
                return Err(TraceError::MissingGraph(name.as_str().to_string()));
            };
            let triples = crate::canon::graph_triples(g)
                .map_err(|e| TraceError::Commit(name.as_str().to_string(), CommitError::Canon(e)))?;
            graphs.push(CommittedNamedGraph::build(name.clone(), triples, *salt)?);
        }
        let union = build_union(&graphs);
        Ok(ZkDataset { graphs, union })
    }

    /// Convenience: load a TriG/N-Quads dataset and commit the named graphs
    /// listed in `names`.
    pub fn from_dataset_str(
        text: &str,
        format: &str,
        names: &[NamedNode],
        salts: &[Fr],
    ) -> Result<Self, TraceError> {
        let store = Graph::load_dataset(text, format).map_err(|e| TraceError::Query(e.to_string()))?;
        Self::from_store(&store, names, salts)
    }

    /// Evaluates a SELECT/ASK over the union with the zk trace armed,
    /// resolves leaf references, and applies the Q6 bnode guard.
    pub fn query_traced(&self, sparql: &str) -> Result<TracedExecution, TraceError> {
        let _guard = sparq_engine::zk::install();
        let result =
            sparq_engine::query(&self.union, sparql).map_err(TraceError::Query)?;
        let trace = sparq_engine::zk::take();
        let patterns = self.resolve(&trace)?;
        bnode_guard(&trace, &patterns, &self.graphs)?;
        Ok(TracedExecution { result, trace, patterns })
    }

    /// Resolves each traced triple to its (graph, leaf) attributions.
    fn resolve(&self, trace: &ZkTrace) -> Result<Vec<ResolvedPattern>, TraceError> {
        let mut out = Vec::with_capacity(trace.patterns.len());
        for pm in &trace.patterns {
            let mut leaves = Vec::with_capacity(pm.triples.len());
            for t in &pm.triples {
                let triple = terms_to_triple(t)
                    .map_err(TraceError::UnattributableTriple)?;
                let mut refs = Vec::new();
                for (gi, g) in self.graphs.iter().enumerate() {
                    if let Some(leaf) = g.leaf_index(&triple) {
                        refs.push(LeafRef { graph: gi, leaf });
                    }
                }
                if refs.is_empty() {
                    return Err(TraceError::UnattributableTriple(triple.to_string()));
                }
                leaves.push(refs);
            }
            out.push(ResolvedPattern { pattern: pm.pattern.clone(), leaves });
        }
        Ok(out)
    }
}

/// Builds the union store (RDF merge presented as the default graph).
///
/// NOTE on merge fidelity: the engine identifies equal bnode *labels* across
/// the input graphs, which RDF merge semantics forbid. Queries whose results
/// could depend on such an identification are exactly the ones the Q6 guard
/// rejects after execution; everything else is label-insensitive.
fn build_union(graphs: &[CommittedNamedGraph]) -> Graph {
    let mut dict = Dict::new();
    let mut ids = Vec::new();
    for g in graphs {
        for t in &g.triples {
            let s = match &t.subject {
                oxrdf::NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n.clone()),
                oxrdf::NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b.clone()),
            };
            let p = Term::NamedNode(t.predicate.clone());
            let o = t.object.clone();
            ids.push([dict.intern(&s), dict.intern(&p), dict.intern(&o)]);
        }
    }
    Graph::from_parts(dict, ids)
}

fn terms_to_triple(t: &[Term; 3]) -> Result<Triple, String> {
    use oxrdf::NamedOrBlankNode;
    let subject = match &t[0] {
        Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n.clone()),
        Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b.clone()),
        other => return Err(format!("invalid traced subject: {other}")),
    };
    let predicate = match &t[1] {
        Term::NamedNode(n) => n.clone(),
        other => return Err(format!("invalid traced predicate: {other}")),
    };
    Ok(Triple::new(subject, predicate, t[2].clone()))
}

/// Q6 prover-side guard (plan §2.4, layer 2): reject executed plans that
/// join a bnode-valued variable across graph boundaries.
///
/// For every variable shared between two traced patterns, and every blank
/// node bound to it by both patterns' matched triples: if some pair of
/// supporting triples has **disjoint** graph attributions, the join equates
/// a bnode across graphs — which RDF merge semantics make distinct-by-
/// construction (salted leaf encodings, §2.2) — so the plan is rejected
/// rather than proving a join that matches nothing (or leaking which graphs
/// were tried).
fn bnode_guard(
    trace: &ZkTrace,
    resolved: &[ResolvedPattern],
    graphs: &[CommittedNamedGraph],
) -> Result<(), TraceError> {
    // Per pattern, per variable: bnode value → set of attributable graphs
    // over all supporting triples (graph sets unioned per bnode, but we keep
    // per-triple sets to detect disjoint pairs precisely).
    type Support = HashMap<String, Vec<HashSet<usize>>>; // bnode label → per-triple graph sets
    let mut per_pattern: Vec<HashMap<String, Support>> = Vec::with_capacity(trace.patterns.len());

    for (pm, rp) in trace.patterns.iter().zip(resolved) {
        let mut vars: HashMap<String, Support> = HashMap::new();
        for (slot_i, slot) in pm.pattern.slots.iter().enumerate() {
            let SlotPattern::Var(name) = slot else { continue };
            for (t, refs) in pm.triples.iter().zip(&rp.leaves) {
                let Term::BlankNode(b) = &t[slot_i] else { continue };
                let gset: HashSet<usize> = refs.iter().map(|r| r.graph).collect();
                vars.entry(name.clone())
                    .or_default()
                    .entry(b.as_str().to_string())
                    .or_default()
                    .push(gset);
            }
        }
        per_pattern.push(vars);
    }

    for i in 0..per_pattern.len() {
        for j in (i + 1)..per_pattern.len() {
            for (var, support_i) in &per_pattern[i] {
                let Some(support_j) = per_pattern[j].get(var) else { continue };
                for (bnode, sets_i) in support_i {
                    let Some(sets_j) = support_j.get(bnode) else { continue };
                    for si in sets_i {
                        for sj in sets_j {
                            if si.is_disjoint(sj) {
                                let gi = si.iter().next().copied().unwrap_or(0);
                                let gj = sj.iter().next().copied().unwrap_or(0);
                                return Err(TraceError::CrossGraphBnodeJoin {
                                    variable: var.clone(),
                                    bnode: bnode.clone(),
                                    graphs: (
                                        graphs[gi].name.as_str().to_string(),
                                        graphs[gj].name.as_str().to_string(),
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Differential-test helper: the union of all traced triples as a fresh
/// store (used to assert sufficiency: re-evaluating the query over ONLY the
/// traced triples must reproduce the result).
pub fn traced_triples_store(trace: &ZkTrace) -> Graph {
    let mut dict = Dict::new();
    let mut ids = Vec::new();
    for pm in &trace.patterns {
        for t in &pm.triples {
            ids.push([dict.intern(&t[0]), dict.intern(&t[1]), dict.intern(&t[2])]);
        }
    }
    Graph::from_parts(dict, ids)
}
