//! (sq-7d3dj.30.7) End-to-end acceptance for the opt-in `value-join`
//! feature: the SP2Bench q05a star-intersection shape — two pattern components
//! glued only by `FILTER(?name = ?name2)` — must produce EXACTLY the rows of the
//! unoptimized evaluation.
//!
//! 🤖 SPARQ agent. Whole file gated on `value-join`: the default (feature-OFF)
//! build compiles it to nothing. The oracle here is INDEPENDENT of the pass's own
//! kill switch (that differential lives in the in-src `exec::eqjoin::tests`):
//! `FILTER(!(?a != ?b))` has the same three-valued keep-set as `FILTER(?a = ?b)`
//! (`=` TRUE ⇔ `!=` FALSE ⇔ `!(!=)` TRUE; type errors eliminate through both) but
//! is not a top-level equality conjunct, so it evaluates through the verbatim
//! cross-product path. The deep per-term-class differentials (numeric promotion,
//! sq-lr2ii high-precision decimals, language tags, temporals, booleans, declines)
//! are in `src/eqjoin.rs`'s unit tests, which this feature's matrix leg also runs.
#![cfg(feature = "value-join")]

use sparq_core::Graph;
use sparq_engine::{ask, query};

const PFX: &str = concat!(
    "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> ",
    "PREFIX bench: <http://ex/bench#> ",
    "PREFIX dc: <http://purl.org/dc/elements/1.1/> ",
    "PREFIX foaf: <http://xmlns.com/foaf/0.1/> ",
);

fn sp2b_graph() -> Graph {
    // Two Article authors and two Inproceedings authors; exactly ONE shared name
    // ("erdoes") across the two stars, via DIFFERENT person IRIs (so the join must
    // key on the literal VALUE, not on the person id).
    let nt = concat!(
        "<http://ex/article1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/bench#Article> .\n",
        "<http://ex/article1> <http://purl.org/dc/elements/1.1/creator> <http://ex/personA> .\n",
        "<http://ex/article2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/bench#Article> .\n",
        "<http://ex/article2> <http://purl.org/dc/elements/1.1/creator> <http://ex/personB> .\n",
        "<http://ex/personA> <http://xmlns.com/foaf/0.1/name> \"erdoes\" .\n",
        "<http://ex/personB> <http://xmlns.com/foaf/0.1/name> \"knuth\" .\n",
        "<http://ex/inproc1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/bench#Inproceedings> .\n",
        "<http://ex/inproc1> <http://purl.org/dc/elements/1.1/creator> <http://ex/personC> .\n",
        "<http://ex/inproc2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/bench#Inproceedings> .\n",
        "<http://ex/inproc2> <http://purl.org/dc/elements/1.1/creator> <http://ex/personD> .\n",
        "<http://ex/personC> <http://xmlns.com/foaf/0.1/name> \"erdoes\" .\n",
        "<http://ex/personD> <http://xmlns.com/foaf/0.1/name> \"lamport\" .\n",
    );
    Graph::load_str(nt, "ntriples").unwrap()
}

fn q05a(filter: &str) -> String {
    format!(
        "{PFX} SELECT DISTINCT ?person ?name WHERE {{ \
           ?article rdf:type bench:Article . \
           ?article dc:creator ?person . \
           ?inproc rdf:type bench:Inproceedings . \
           ?inproc dc:creator ?person2 . \
           ?person foaf:name ?name . \
           ?person2 foaf:name ?name2 . \
           {filter} }}"
    )
}

fn bag(g: &Graph, q: &str) -> Vec<Vec<String>> {
    let r = query(g, q).unwrap();
    let mut bag: Vec<Vec<String>> = r
        .rows
        .iter()
        .map(|row| {
            let mut cells: Vec<String> = r
                .vars
                .iter()
                .zip(row.iter())
                .map(|(v, c)| match c {
                    Some(t) => format!("{}={}", v, t),
                    None => format!("{}=UNBOUND", v),
                })
                .collect();
            cells.sort();
            cells
        })
        .collect();
    bag.sort();
    bag
}

#[test]
fn q05a_shape_identical_rows_to_unoptimized_oracle() {
    let g = sp2b_graph();
    let optimized = bag(&g, &q05a("FILTER(?name = ?name2)"));
    // The `!(?name != ?name2)` spelling is result-equivalent but shape-ineligible,
    // so it runs the verbatim cross-product-then-filter plan.
    let oracle = bag(&g, &q05a("FILTER(!(?name != ?name2))"));
    assert_eq!(optimized, oracle, "value-join must not change the q05a result");
    // And the literal expectation: exactly the one author name shared by both stars.
    assert_eq!(optimized.len(), 1, "bag: {:?}", optimized);
    assert_eq!(
        optimized[0],
        vec![
            "?name=\"erdoes\"".to_string(),
            "?person=<http://ex/personA>".to_string(),
        ],
        "bag: {:?}",
        optimized
    );
}

#[test]
fn q12a_ask_twin_agrees() {
    let g = sp2b_graph();
    // ASK twin of q05a (SP2Bench q12a is ASK over the q05a body).
    let run_ask = |filter: &str| -> bool {
        let q = q05a(filter).replace("SELECT DISTINCT ?person ?name", "ASK");
        ask(&g, &q).unwrap()
    };
    assert!(run_ask("FILTER(?name = ?name2)"));
    assert_eq!(run_ask("FILTER(?name = ?name2)"), run_ask("FILTER(!(?name != ?name2))"));
    // A filter with no possible intersection stays false through both plans.
    assert!(!run_ask("FILTER(?name = ?name2 && ?name != \"erdoes\")"));
}
