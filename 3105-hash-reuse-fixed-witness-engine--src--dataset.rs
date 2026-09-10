//! Dataset (re)construction shared by the query and update sides.
//!
//! A query's dataset clause (`FROM` / `FROM NAMED`, SPARQL 1.1 §13.2) and an
//! update's `USING (NAMED)` re-scoping describe the same thing: an ACTIVE
//! dataset assembled from the store's named graphs — default graph = the merge
//! of the listed default-graph sources, named graphs = exactly the listed ones.
//! This module holds the term-triple-set primitives (decode a graph to ground
//! term triples, rebuild a fresh `Graph` from a set) and the query-side
//! [`build_active`]; the update side's `Dataset::build_using` in `update.rs`
//! reuses the primitives (it works over already-decoded sets because updates
//! mutate them).

use oxrdf::{NamedNode, Term};
use rustc_hash::FxHashSet;
use sparq_core::dict::Dict;
use sparq_core::store::Pattern as IdPattern;
use sparq_core::Graph;
use spargebra::algebra::QueryDataset;

pub(crate) type TripleTerms = [Term; 3];
pub(crate) type TripleSet = FxHashSet<TripleTerms>;

/// The triples of one graph as ground term-triples (decoded from its dictionary).
pub(crate) fn decode_triples(g: &Graph) -> TripleSet {
    let pat: IdPattern = [None, None, None];
    let scan = g.store.scan(&pat);
    scan.rows
        .iter()
        .map(|r| {
            let t = scan.to_spo(r);
            [g.dict.term(t[0]), g.dict.term(t[1]), g.dict.term(t[2])]
        })
        .collect()
}

/// Rebuild an immutable graph from a term-triple set (fresh dictionary + permutation indexes).
pub(crate) fn build(triples: &TripleSet) -> Graph {
    let mut dict = Dict::new();
    let mut ids = Vec::with_capacity(triples.len());
    for [s, p, o] in triples {
        ids.push([dict.intern(s), dict.intern(p), dict.intern(o)]);
    }
    Graph::from_parts(dict, ids)
}

pub(crate) fn empty_graph() -> Graph {
    Graph::from_parts(Dict::new(), Vec::new())
}

/// The store's named graph called `name`, if it holds one.
fn find_named<'a>(graph: &'a Graph, name: &NamedNode) -> Option<&'a Graph> {
    graph.named.iter().find_map(|(g, sub)| match g {
        Term::NamedNode(n) if n == name => Some(sub),
        _ => None,
    })
}

/// The ACTIVE dataset of a query that carries a dataset clause: the default
/// graph is the merge of the `FROM` graphs and the named graphs are exactly the
/// `FROM NAMED` ones — the store's own default/named graphs do NOT leak in. A
/// graph name the store does not hold denotes the EMPTY graph (it still exists
/// in the active dataset, so `GRAPH <absent> {}` keeps its unit-row semantics).
///
/// Under an installed dataset view (L1) the clause first INTERSECTS with the
/// view: a non-visible graph is treated exactly like an absent one (it
/// contributes nothing to `FROM` and denotes the empty graph for `FROM NAMED`),
/// so the restriction composes and never widens — and a non-visible graph stays
/// indistinguishable from an absent one.
///
/// Only called when the query has a dataset clause; the common no-clause path
/// never pays for this.
pub(crate) fn build_active(graph: &Graph, ds: &QueryDataset) -> Graph {
    let visible = |n: &NamedNode| crate::exec::view::allows(&Term::NamedNode(n.clone()));
    let mut default = TripleSet::default();
    for n in &ds.default {
        if !visible(n) {
            continue; // view: non-visible ≡ absent
        }
        if let Some(g) = find_named(graph, n) {
            default.extend(decode_triples(g));
        }
    }
    let mut out = build(&default);
    for n in ds.named.as_deref().unwrap_or_default() {
        let name = Term::NamedNode(n.clone());
        if out.named.iter().any(|(g, _)| *g == name) {
            continue; // a repeated FROM NAMED still names ONE graph
        }
        let g = match find_named(graph, n).filter(|_| visible(n)) {
            Some(g) => build(&decode_triples(g)),
            None => empty_graph(),
        };
        out.named.push((name, g));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Graph {
        // default: 1 triple; g1: 2 triples; g2: 1 triple (shares :x with g1).
        let src = "<http://ex/d> <http://ex/p> <http://ex/e> .\n\
                   <http://ex/x> <http://ex/p> <http://ex/y> <http://ex/g1> .\n\
                   <http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g1> .\n\
                   <http://ex/x> <http://ex/q> <http://ex/z> <http://ex/g2> .";
        Graph::load_dataset(src, "nquads").unwrap()
    }

    fn ds(default: &[&str], named: &[&str]) -> QueryDataset {
        QueryDataset {
            default: default.iter().map(|i| NamedNode::new(*i).unwrap()).collect(),
            named: Some(named.iter().map(|i| NamedNode::new(*i).unwrap()).collect()),
        }
    }

    #[test]
    fn from_merges_into_default_and_drops_store_graphs() {
        let g = store();
        let active = build_active(&g, &ds(&["http://ex/g1", "http://ex/g2"], &[]));
        assert_eq!(active.len(), 3); // g1 ∪ g2; the store's default graph is gone
        assert!(active.named.is_empty()); // no FROM NAMED -> no named graphs
    }

    #[test]
    fn from_named_selects_exactly_those_graphs() {
        let g = store();
        let active = build_active(&g, &ds(&[], &["http://ex/g2"]));
        assert_eq!(active.len(), 0); // no FROM -> empty default graph
        assert_eq!(active.named.len(), 1);
        assert_eq!(active.named[0].1.len(), 1);
    }

    #[test]
    fn absent_graph_is_empty_and_repeats_dedup() {
        let g = store();
        let active =
            build_active(&g, &ds(&["http://ex/nope"], &["http://ex/g1", "http://ex/g1", "http://ex/nope"]));
        assert_eq!(active.len(), 0); // FROM of an absent graph contributes nothing
        assert_eq!(active.named.len(), 2); // g1 once, plus the absent one as EMPTY
        assert_eq!(active.named[0].1.len(), 2);
        assert_eq!(active.named[1].1.len(), 0);
    }
}
