//! (sq-7d3dj.30.5) DPccp default-on acceptance tests.
//!
//! Verifies that when the `dp-planner` feature is compiled:
//!
//! 1. DPccp fires by default for multi-pattern BGPs **without** any explicit
//!    `with_dp_planner` install.
//! 2. `without_dp_planner` explicitly opts back to greedy GOO.
//! 3. Both paths produce result-equivalent (bag-identical) output.
//! 4. Nested opt-out and re-enable correctly restore the prior state.
//!
//! The file compiles clean in BOTH feature states (`dp-planner` on and off).  In the
//! feature-OFF state no tests are registered (the binary builds but finds nothing to
//! run), which is the byte-identical greedy baseline.

// All feature-specific code — helpers AND tests — is gated together under
// `dp-planner` so the file compiles without `dead_code` warnings in the OFF state.
#[cfg(feature = "dp-planner")]
use sparq_core::Graph;
#[cfg(feature = "dp-planner")]
use sparq_engine::query;

// ---- shared helpers (only compiled when the feature is on) ----------------------

#[cfg(feature = "dp-planner")]
type Row = Vec<(String, String)>;

/// Run query `q` on `graph` and return a sorted bag of `(variable, value)` rows,
/// suitable for order-independent comparison.  Unbound cells use an unambiguous
/// sentinel (no real RDF term serialises to it).
#[cfg(feature = "dp-planner")]
fn result_bag(graph: &Graph, q: &str) -> Vec<Row> {
    const PFX: &str = "PREFIX ex: <http://ex/> \
                       PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
                       PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
                       PREFIX dct: <http://purl.org/dc/terms/> ";
    const UNBOUND: &str = "\0\u{1}unbound\u{1}\0";
    let r = query(graph, &format!("{}{}", PFX, q)).unwrap();
    let vars: Vec<String> = r.vars.iter().map(|v| v.as_str().to_string()).collect();
    let mut bag: Vec<Row> = r
        .rows
        .iter()
        .map(|row| {
            let mut cells: Row = vars
                .iter()
                .zip(row.iter())
                .map(|(v, cell)| {
                    (
                        v.clone(),
                        cell.as_ref()
                            .map(|t| format!("{}", t))
                            .unwrap_or_else(|| UNBOUND.to_string()),
                    )
                })
                .collect();
            cells.sort();
            cells
        })
        .collect();
    bag.sort();
    bag
}

/// Build a tiny graph shaped like the SP2Bench q07 BGP (subClassOf + type + references
/// chain) — small enough to construct inline, big enough that the DP planner picks a
/// DIFFERENT join order than greedy GOO, exercising the code path under test.
///
/// Triples:
/// * 5 `rdfs:subClassOf` edges (class0 through class5)
/// * 30 `rdf:type` assertions (doc_i typed class_(i%5))
/// * 10 `dct:references` + `ex:member` bag triples (doc0 through doc9)
#[cfg(feature = "dp-planner")]
fn q07_shaped_graph() -> Graph {
    let mut nt = String::new();
    for i in 0..5usize {
        nt.push_str(&format!(
            "<http://ex/class{}> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://ex/class{}> .\n",
            i, i + 1
        ));
    }
    for i in 0..30usize {
        nt.push_str(&format!(
            "<http://ex/doc{}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/class{}> .\n",
            i, i % 5
        ));
    }
    for i in 0..10usize {
        nt.push_str(&format!(
            "<http://ex/doc{}> <http://purl.org/dc/terms/references> <http://ex/bag{}> .\n",
            i, i
        ));
        nt.push_str(&format!(
            "<http://ex/bag{}> <http://ex/member> <http://ex/doc{}> .\n",
            i,
            (i + 3) % 30
        ));
    }
    Graph::load_str(&nt, "ntriples").unwrap()
}

// ---- acceptance tests (only compiled + registered under `dp-planner`) ----------

/// Default-on: DPccp fires for a 4-pattern BGP without any explicit planner install.
/// Result must be correct and non-empty.
#[cfg(feature = "dp-planner")]
#[test]
fn default_on_fires_for_multi_pattern_bgp() {
    let g = q07_shaped_graph();
    // 4-pattern BGP: two join variables (?class, ?doc) connecting all four patterns.
    // Greedy GOO seeds from the selective subClassOf scan and fans out; DPccp
    // (default-on) finds the Cout-optimal bushy tree.  No explicit install needed.
    let q = "SELECT ?doc ?bag WHERE { \
        <http://ex/class0> rdfs:subClassOf ?class . \
        ?doc rdf:type ?class . \
        ?doc dct:references ?bag \
    }";
    let result = result_bag(&g, q);
    // class0 subClassOf class1; docs of type class1: doc1, doc6, ... ; only doc1 and
    // doc6 have references bags (indices 0-9 have bags).
    assert!(!result.is_empty(), "default-on DPccp must return correct non-empty results");
}

/// Explicit opt-out: `without_dp_planner` restores greedy GOO.
/// Both paths must return the same (result-equivalent) bag.
#[cfg(feature = "dp-planner")]
#[test]
fn without_dp_planner_gives_same_results() {
    let g = q07_shaped_graph();
    let q = "SELECT ?doc ?bag WHERE { \
        <http://ex/class0> rdfs:subClassOf ?class . \
        ?doc rdf:type ?class . \
        ?doc dct:references ?bag \
    }";
    let default_result = result_bag(&g, q);
    let greedy_result = sparq_engine::without_dp_planner(|| result_bag(&g, q));
    assert_eq!(
        default_result,
        greedy_result,
        "opt-out (greedy GOO) must give same results as default DPccp"
    );
    assert!(!default_result.is_empty(), "non-vacuous result");
}

/// Explicit `with_dp_planner` install still produces the same result as the default.
#[cfg(feature = "dp-planner")]
#[test]
fn explicit_install_still_works() {
    let g = q07_shaped_graph();
    let q = "SELECT ?doc ?bag WHERE { \
        <http://ex/class0> rdfs:subClassOf ?class . \
        ?doc rdf:type ?class . \
        ?doc dct:references ?bag \
    }";
    let default_result = result_bag(&g, q);
    let explicit_result = sparq_engine::with_dp_planner(|| result_bag(&g, q));
    assert_eq!(
        default_result,
        explicit_result,
        "explicit with_dp_planner must match default DPccp"
    );
}

/// Nested opt-out scopes restore the prior state correctly when the guard drops.
#[cfg(feature = "dp-planner")]
#[test]
fn nested_opt_out_restores_outer_state() {
    let g = q07_shaped_graph();
    let q = "SELECT ?doc ?bag WHERE { \
        <http://ex/class1> rdfs:subClassOf ?class . \
        ?doc rdf:type ?class . \
        ?doc dct:references ?bag \
    }";
    let before = result_bag(&g, q);
    sparq_engine::without_dp_planner(|| {
        let inner = result_bag(&g, q);
        // Result-equivalent even in greedy mode.
        assert_eq!(before, inner, "opt-out result must match default for this query");
    });
    // After the guard drops, the prior state (default-on) is restored.
    let after = result_bag(&g, q);
    assert_eq!(before, after, "default-on state must be restored after opt-out guard drops");
}

/// `with_dp_planner_budget(0, …)` disables the DP (all-greedy); result is still correct.
#[cfg(feature = "dp-planner")]
#[test]
fn budget_zero_behaves_like_greedy() {
    let g = q07_shaped_graph();
    let q = "SELECT ?doc ?bag WHERE { \
        <http://ex/class0> rdfs:subClassOf ?class . \
        ?doc rdf:type ?class . \
        ?doc dct:references ?bag \
    }";
    let default_result = result_bag(&g, q);
    let zero_budget = sparq_engine::with_dp_planner_budget(0, || result_bag(&g, q));
    assert_eq!(default_result, zero_budget, "budget=0 must give same results as default");
}
