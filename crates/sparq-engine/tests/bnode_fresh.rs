//! `BNODE()` / `BNODE(str)` mint blank nodes distinct from every blank node in the
//! dataset (SPARQL 1.1 §17.4.2.9), including data built from labels this process minted
//! earlier, blank-node graph names, and graphs a dataset view hides.

use std::collections::HashSet;
use std::sync::Arc;

use oxrdf::Term;
use sparq_core::Graph;
use sparq_engine::{query, query_view, DatasetView, DefaultGraphMode, FxHashSet, QueryResult};

const ROWS: usize = 200;
const MINT: &str = "PREFIX ex: <http://example.org/>
    SELECT ?b ?k ?k2 WHERE { ?s ex:rank ?r
      BIND(BNODE() AS ?b) BIND(BNODE(\"k\") AS ?k) BIND(BNODE(\"k\") AS ?k2) }";

fn ranked(n: usize) -> String {
    (0..n).map(|i| format!("<http://example.org/s{i}> <http://example.org/rank> <http://example.org/one> .\n")).collect()
}

/// The minted labels of one [`MINT`] result, checking the per-solution rules on the way:
/// `BNODE()` differs on every row, `BNODE("k")` is shared within a row only.
fn minted(r: &QueryResult) -> Vec<String> {
    let label = |t: &Option<Term>| match t {
        Some(Term::BlankNode(b)) => b.as_str().to_owned(),
        other => panic!("expected a blank node, got {other:?}"),
    };
    let mut out = Vec::new();
    for row in &r.rows {
        assert_eq!(label(&row[1]), label(&row[2]), "equal arguments in one solution share a node");
        out.push(label(&row[0]));
        out.push(label(&row[1]));
    }
    let distinct: HashSet<_> = out.iter().collect();
    assert_eq!(distinct.len(), out.len(), "every solution gets its own nodes");
    out
}

#[test]
fn minted_blank_nodes_never_equal_stored_ones() {
    let seed = Graph::load_dataset(&ranked(ROWS), "nquads").expect("graph");
    let earlier = minted(&query(&seed, MINT).expect("query"));

    // Store every earlier label, and the counter-shaped labels older builds minted, as a
    // subject in the default graph, as a graph name, and inside that graph.
    let mut nq = ranked(ROWS);
    let old_shape = (0..2 * ROWS).map(|i| format!("fnb{i}"));
    let stored: Vec<String> = earlier.iter().cloned().chain(old_shape).collect();
    for l in &stored {
        nq.push_str(&format!("_:{l} <http://example.org/seen> <http://example.org/one> .\n"));
        nq.push_str(&format!("_:{l} <http://example.org/seen> <http://example.org/one> _:{l} .\n"));
    }
    let g = Graph::load_dataset(&nq, "nquads").expect("graph");
    let stored: HashSet<String> = stored.into_iter().collect();
    for l in minted(&query(&g, MINT).expect("query")) {
        assert!(!stored.contains(&l), "minted {l} equals a stored blank node");
    }
    let joined = "PREFIX ex: <http://example.org/>
        SELECT ?s WHERE { ?s ex:rank ?r BIND(BNODE() AS ?b) BIND(BNODE(\"k\") AS ?k)
          FILTER NOT EXISTS { ?x ex:seen ex:one FILTER(sameTerm(?x, ?b) || sameTerm(?x, ?k)) }
          FILTER NOT EXISTS { GRAPH ?n { } FILTER(sameTerm(?n, ?b) || sameTerm(?n, ?k)) } }";
    assert_eq!(query(&g, joined).expect("query").len(), ROWS, "no minted node joins a stored one");

    // A view that hides every named graph mints the same kind of labels, unaffected by
    // what the hidden graphs hold.
    let view = DatasetView {
        base: &g,
        named: Arc::new(FxHashSet::default()),
        default: DefaultGraphMode::StoreDefault,
    };
    for l in minted(&query_view(&view, MINT).expect("query")) {
        assert!(!stored.contains(&l), "minted {l} equals a stored blank node");
        assert_eq!(l.len(), earlier[0].len(), "labels have one shape");
    }
}
