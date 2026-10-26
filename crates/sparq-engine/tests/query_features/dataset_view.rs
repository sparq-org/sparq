//! Integration tests for the L1 dataset view (`DatasetView` / `*_view` entry
//! points): named-graph visibility, default-graph modes, indistinguishability
//! of non-visible and absent graphs (the security property), dataset-clause
//! intersection, and composition with budgets / extension functions / the
//! rayon-parallel evaluation paths.

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::{
    ask_view, count_view, query, query_json_view, query_view, query_view_with_budget, with_functions, with_view,
    DatasetView, DefaultGraphMode, FunctionRegistry, FxHashSet, QueryBudget,
};
use std::sync::Arc;

/// Store: default graph d1-p->d2-p->d3 (a 2-edge chain), g1 (2 triples,
/// a-p->b-p->c), g2 (1 triple), g3 (1 triple, different predicate).
const DATA: &str = "\
<http://ex/d1> <http://ex/p> <http://ex/d2> .
<http://ex/d2> <http://ex/p> <http://ex/d3> .
<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g1> .
<http://ex/b> <http://ex/p> <http://ex/c> <http://ex/g1> .
<http://ex/x> <http://ex/p> <http://ex/y> <http://ex/g2> .
<http://ex/s> <http://ex/q> <http://ex/t> <http://ex/g3> .
";

fn store() -> Graph {
    Graph::load_dataset(DATA, "nquads").unwrap()
}

fn names(iris: &[&str]) -> Arc<FxHashSet<Term>> {
    Arc::new(iris.iter().map(|i| Term::NamedNode(NamedNode::new(*i).unwrap())).collect())
}

fn view<'g>(g: &'g Graph, visible: &[&str], default: DefaultGraphMode) -> DatasetView<'g> {
    DatasetView { base: g, named: names(visible), default }
}

#[test]
fn graph_var_enumerates_only_visible_graphs() {
    let g = store();
    let q = "SELECT ?g ?s WHERE { GRAPH ?g { ?s ?p ?o } }";
    // No view: all 4 named-graph triples.
    assert_eq!(query(&g, q).unwrap().len(), 4);
    // View {g1}: exactly g1's two rows, every ?g bound to g1.
    let v = view(&g, &["http://ex/g1"], DefaultGraphMode::StoreDefault);
    let r = query_view(&v, q).unwrap();
    assert_eq!(r.len(), 2);
    assert!(r.rows.iter().all(|row| row[0].as_ref().unwrap().to_string() == "<http://ex/g1>"));
    // View {g1,g2}: three rows.
    let v = view(&g, &["http://ex/g1", "http://ex/g2"], DefaultGraphMode::StoreDefault);
    assert_eq!(query_view(&v, q).unwrap().len(), 3);
    // Empty view: GRAPH ?g matches nothing.
    let v = view(&g, &[], DefaultGraphMode::StoreDefault);
    assert_eq!(query_view(&v, q).unwrap().len(), 0);
    // GRAPH ?g {} unit-row-per-graph semantics restrict the same way.
    let v = view(&g, &["http://ex/g1"], DefaultGraphMode::StoreDefault);
    assert_eq!(query_view(&v, "SELECT ?g WHERE { GRAPH ?g {} }").unwrap().len(), 1);
    // The view never leaks past the entry point: the plain API sees everything.
    assert_eq!(query(&g, q).unwrap().len(), 4);
}

/// The security property: a non-visible graph must be INDISTINGUISHABLE from an
/// absent one — byte-identical results for every query shape, with the
/// non-visible g2 substituted by the truly-absent nope.
#[test]
fn invisible_graph_is_indistinguishable_from_absent() {
    let g = store();
    let v = view(&g, &["http://ex/g1"], DefaultGraphMode::StoreDefault);
    let shapes = [
        // Concrete GRAPH: zero solutions.
        "SELECT * WHERE { GRAPH <G> { ?s ?p ?o } }",
        // GRAPH <g> {}: NOT the unit row (matches today's absent-graph semantics).
        "SELECT * WHERE { GRAPH <G> {} }",
        // ASK form.
        "ASK { GRAPH <G> { ?s ?p ?o } }",
        // OPTIONAL over the graph: left rows survive with unbound ?o.
        "SELECT * WHERE { ?s <http://ex/p> ?d OPTIONAL { GRAPH <G> { ?s2 ?p2 ?o } } }",
        // NOT EXISTS over the graph: all rows kept.
        "SELECT * WHERE { ?s <http://ex/p> ?d FILTER NOT EXISTS { GRAPH <G> { ?x ?y ?z } } }",
        // Property path inside the graph.
        "SELECT * WHERE { GRAPH <G> { ?s <http://ex/p>+ ?o } }",
        // Dataset clause naming the graph (active-dataset path).
        "SELECT * FROM NAMED <G> WHERE { GRAPH ?g { ?s ?p ?o } }",
        "SELECT * FROM NAMED <G> WHERE { GRAPH <G> {} }",
        "SELECT * FROM <G> WHERE { ?s ?p ?o }",
    ];
    for shape in shapes {
        let invisible = shape.replace("<G>", "<http://ex/g2>");
        let absent = shape.replace("<G>", "<http://ex/nope>");
        assert_eq!(
            query_json_view(&v, &invisible).unwrap(),
            query_json_view(&v, &absent).unwrap(),
            "distinguishable results for shape: {shape}"
        );
    }
}

#[test]
fn optional_absent_graph_preserves_schema_and_correlated_left_rows() {
    let g = store();
    let v = view(&g, &["http://ex/g1"], DefaultGraphMode::StoreDefault);
    for visible in [false, true] {
        let run = |q: &str| {
            if visible { query_view(&v, q).unwrap() } else { query(&g, q).unwrap() }
        };
        let pattern = "GRAPH <http://ex/nope> { ?s <http://ex/p> ?o BIND(?d AS ?inner) }";
        let absent = run(&format!("SELECT ?s ?o ?inner WHERE {{ {pattern} }}"));
        assert_eq!(absent.vars.iter().map(|v| v.as_str()).collect::<Vec<_>>(), ["s", "o", "inner"]);
        assert!(absent.rows.is_empty());
        let optional = run(&format!(
            "SELECT ?s ?d ?o ?inner WHERE {{ ?s <http://ex/p> ?d OPTIONAL {{ {pattern} }} }} ORDER BY ?s"
        ));
        assert_eq!(optional.vars.iter().map(|v| v.as_str()).collect::<Vec<_>>(), ["s", "d", "o", "inner"]);
        let iri = |name| Some(Term::NamedNode(NamedNode::new(format!("http://ex/{name}")).unwrap()));
        assert_eq!(optional.rows, vec![
            vec![iri("d1"), iri("d2"), None, None],
            vec![iri("d2"), iri("d3"), None, None],
        ]);
        let unit = run("SELECT * WHERE { GRAPH <http://ex/nope> {} }");
        assert!(unit.vars.is_empty());
        assert!(unit.rows.is_empty());
    }
}

#[test]
fn default_graph_modes() {
    let g = store();
    // StoreDefault: today's behaviour — the store's default graph is visible.
    let v = view(&g, &["http://ex/g1"], DefaultGraphMode::StoreDefault);
    assert_eq!(query_view(&v, "SELECT * WHERE { ?s ?p ?o }").unwrap().len(), 2);
    assert_eq!(count_view(&v, "SELECT * WHERE { ?s <http://ex/p> ?o }").unwrap(), 2);
    // Property path over the default chain d1->d2->d3: 3 pairs under p+.
    assert_eq!(query_view(&v, "SELECT * WHERE { ?s <http://ex/p>+ ?o }").unwrap().len(), 3);

    // Empty: the default graph has no data at top-level scope…
    let v = view(&g, &["http://ex/g1"], DefaultGraphMode::Empty);
    assert_eq!(query_view(&v, "SELECT * WHERE { ?s ?p ?o }").unwrap().len(), 0);
    assert_eq!(query_view(&v, "SELECT * WHERE { ?s <http://ex/p>+ ?o }").unwrap().len(), 0);
    // …including the index fast paths (count / ASK / streaming JSON).
    assert_eq!(count_view(&v, "SELECT * WHERE { ?s <http://ex/p> ?o }").unwrap(), 0);
    assert!(!ask_view(&v, "ASK { ?s <http://ex/p> ?o }").unwrap());
    let json = query_json_view(&v, "SELECT ?s WHERE { ?s <http://ex/p> ?o }").unwrap();
    assert!(json.contains("\"bindings\":[]"), "got: {json}");
    // …but the empty group pattern keeps its unit row,
    assert_eq!(query_view(&v, "SELECT (1 AS ?x) WHERE {}").unwrap().len(), 1);
    assert!(ask_view(&v, "ASK {}").unwrap());
    // …and GRAPH evaluation is untouched (the suspend flag): BGPs, paths and
    // EXISTS inside a visible graph still see that graph's data.
    assert_eq!(query_view(&v, "SELECT * WHERE { GRAPH <http://ex/g1> { ?s ?p ?o } }").unwrap().len(), 2);
    assert_eq!(
        query_view(&v, "SELECT * WHERE { GRAPH <http://ex/g1> { ?s <http://ex/p>+ ?o } }").unwrap().len(),
        3 // a->b, b->c, a->c
    );
    assert_eq!(
        query_view(
            &v,
            "SELECT * WHERE { GRAPH <http://ex/g1> { ?s <http://ex/p> ?o FILTER EXISTS { ?o <http://ex/p> ?n } } }"
        )
        .unwrap()
        .len(),
        1 // only a->b (b has an outgoing edge inside g1)
    );
}

#[test]
fn graph_unit_row_semantics_preserved() {
    let g = store();
    let v = view(&g, &["http://ex/g1"], DefaultGraphMode::StoreDefault);
    // A visible graph keeps the unit-row semantics of `GRAPH <g> {}`.
    let r = query_view(&v, "SELECT * WHERE { GRAPH <http://ex/g1> {} }").unwrap();
    assert_eq!((r.vars.len(), r.rows.len()), (0, 1));
    // A non-visible one behaves like an absent one: zero rows.
    assert_eq!(query_view(&v, "SELECT * WHERE { GRAPH <http://ex/g2> {} }").unwrap().len(), 0);
    assert_eq!(query_view(&v, "SELECT * WHERE { GRAPH <http://ex/nope> {} }").unwrap().len(), 0);
}

#[test]
fn dataset_clause_intersects_with_view() {
    let g = store();
    let v = view(&g, &["http://ex/g1"], DefaultGraphMode::StoreDefault);
    let n = |q: &str| query_view(&v, q).unwrap().len();
    // FROM NAMED of a visible graph: normal active-dataset behaviour.
    assert_eq!(n("SELECT * FROM NAMED <http://ex/g1> WHERE { GRAPH ?g { ?s ?p ?o } }"), 2);
    // FROM NAMED of a non-visible graph contributes the EMPTY graph.
    assert_eq!(n("SELECT * FROM NAMED <http://ex/g2> WHERE { GRAPH ?g { ?s ?p ?o } }"), 0);
    // Listing extra graphs cannot WIDEN the view: only g1 data appears.
    assert_eq!(
        n("SELECT * FROM NAMED <http://ex/g1> FROM NAMED <http://ex/g2> FROM NAMED <http://ex/g3> \
           WHERE { GRAPH ?g { ?s ?p ?o } }"),
        2
    );
    // FROM merges only visible graphs into the active default graph.
    assert_eq!(n("SELECT * FROM <http://ex/g1> WHERE { ?s ?p ?o }"), 2);
    assert_eq!(n("SELECT * FROM <http://ex/g2> WHERE { ?s ?p ?o }"), 0);
    assert_eq!(n("SELECT * FROM <http://ex/g1> FROM <http://ex/g2> WHERE { ?s ?p ?o }"), 2);
    // In the ACTIVE dataset a named (even non-visible ≡ absent) FROM NAMED graph
    // exists as the empty graph, so `GRAPH <g> {}` has the unit row — identically
    // for non-visible and absent (covered byte-for-byte in the
    // indistinguishability test).
    assert_eq!(n("SELECT * FROM NAMED <http://ex/g2> WHERE { GRAPH <http://ex/g2> {} }"), 1);
    assert_eq!(n("SELECT * FROM NAMED <http://ex/nope> WHERE { GRAPH <http://ex/nope> {} }"), 1);
    // A dataset clause under DefaultGraphMode::Empty: FROM re-scopes the default
    // graph to (visible) named-graph data, which the view authorises — Empty
    // governs the STORE default graph only.
    let v_empty = view(&g, &["http://ex/g1"], DefaultGraphMode::Empty);
    assert_eq!(query_view(&v_empty, "SELECT * FROM <http://ex/g1> WHERE { ?s ?p ?o }").unwrap().len(), 2);
    assert_eq!(query_view(&v_empty, "SELECT * FROM <http://ex/g2> WHERE { ?s ?p ?o }").unwrap().len(), 0);
}

#[test]
fn composes_with_budget_and_functions() {
    let g = store();
    let v = view(&g, &["http://ex/g1"], DefaultGraphMode::StoreDefault);
    // Budget: refusal (not truncation) fires through the view entry point.
    let tight = QueryBudget { max_rows: Some(1), ..QueryBudget::unlimited() };
    let e = query_view_with_budget(&v, "SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }", &tight).map(|r| r.len()).unwrap_err();
    assert!(e.contains("query budget exceeded"), "got: {e}");
    let ok = QueryBudget { max_rows: Some(100), ..QueryBudget::unlimited() };
    assert_eq!(query_view_with_budget(&v, "SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }", &ok).unwrap().len(), 2);

    // Extension functions: `ex:one(?x)` = 1, used inside a GRAPH pattern, with
    // the view installed — both nesting orders work.
    let mut reg = FunctionRegistry::new();
    reg.register("http://ex/fn#one", |_args: &[Term]| Ok(Term::Literal(oxrdf::Literal::from(1))));
    let q = "PREFIX fn: <http://ex/fn#> SELECT ?s WHERE { GRAPH ?g { ?s ?p ?o FILTER(fn:one(?s) = 1) } }";
    let r = with_functions(&reg, || query_view(&v, q)).unwrap();
    assert_eq!(r.len(), 2); // only g1 is visible
    let r = with_view(&v, || with_functions(&reg, || query(v.base, q))).unwrap();
    assert_eq!(r.len(), 2);
}

/// The view must reach pattern evaluation INSIDE rayon workers: the parallel
/// FILTER branch engages at PAR_THRESHOLD (50k) rows and re-enters the
/// evaluator through EXISTS. If a worker lost the view, `NOT EXISTS { GRAPH
/// <g2> … }` would see the (non-visible) g2 triple and drop every row it
/// processed.
#[test]
#[cfg(feature = "parallel")]
fn view_holds_under_parallel_evaluation() {
    let n: usize = 50_001;
    let mut nq = String::with_capacity(n * 40);
    for i in 0..n {
        nq.push_str(&format!("<http://ex/n{i}> <http://ex/v> <http://ex/w{i}> .\n"));
    }
    nq.push_str("<http://ex/a> <http://ex/m> <http://ex/b> <http://ex/g1> .\n");
    nq.push_str("<http://ex/x> <http://ex/m> <http://ex/y> <http://ex/g2> .\n");
    let g = Graph::load_dataset(&nq, "nquads").unwrap();
    let v = view(&g, &["http://ex/g1"], DefaultGraphMode::StoreDefault);
    let q = "SELECT ?s WHERE { ?s <http://ex/v> ?o FILTER NOT EXISTS { GRAPH <http://ex/g2> { ?x ?y ?z } } }";
    // Without the view the EXISTS sees g2 and the FILTER drops everything…
    assert_eq!(query(&g, q).unwrap().len(), 0);
    // …with the view, g2 is invisible on EVERY worker: all rows survive.
    assert_eq!(query_view(&v, q).unwrap().len(), n);
    // And the positive direction: a visible graph is seen by every worker.
    let q1 = "SELECT ?s WHERE { ?s <http://ex/v> ?o FILTER EXISTS { GRAPH <http://ex/g1> { ?x ?y ?z } } }";
    assert_eq!(query_view(&v, q1).unwrap().len(), n);
}

/// Micro-benchmark (ignored; run with `cargo test --release -p sparq-engine
/// --test dataset_view -- --ignored --nocapture`): the zero-copy view vs the
/// v1 baseline of listing the visible graphs as FROM NAMED (which decodes and
/// rebuilds an active dataset per query). ~1000 named graphs in the store,
/// querying a ~100-graph subset.
#[test]
#[ignore]
fn bench_view_vs_from_named_copy() {
    let (graphs, visible, per_graph) = (1000usize, 100usize, 10usize);
    let mut nq = String::new();
    for gi in 0..graphs {
        for i in 0..per_graph {
            nq.push_str(&format!("<http://ex/s{i}> <http://ex/p> <http://ex/o{gi}x{i}> <http://ex/g{gi}> .\n"));
        }
    }
    let g = Graph::load_dataset(&nq, "nquads").unwrap();
    let named: FxHashSet<Term> =
        (0..visible).map(|gi| Term::NamedNode(NamedNode::new(format!("http://ex/g{gi}")).unwrap())).collect();
    let v = DatasetView { base: &g, named: Arc::new(named), default: DefaultGraphMode::Empty };
    let q = "SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s <http://ex/p> ?o } }";
    let mut q_from = String::from("SELECT ?g ?s ?o\n");
    for gi in 0..visible {
        q_from.push_str(&format!("FROM NAMED <http://ex/g{gi}>\n"));
    }
    q_from.push_str("WHERE { GRAPH ?g { ?s <http://ex/p> ?o } }");

    // Same answers first.
    let a = query_view(&v, q).unwrap().len();
    let b = query(&g, &q_from).unwrap().len();
    assert_eq!((a, b), (visible * per_graph, visible * per_graph));

    let iters = 30u32;
    let t0 = std::time::Instant::now();
    for _ in 0..iters {
        assert_eq!(query_view(&v, q).unwrap().len(), visible * per_graph);
    }
    let view_per_query = t0.elapsed() / iters;
    let t0 = std::time::Instant::now();
    for _ in 0..iters {
        assert_eq!(query(&g, &q_from).unwrap().len(), visible * per_graph);
    }
    let copy_per_query = t0.elapsed() / iters;
    println!(
        "dataset-view bench ({graphs} graphs, {visible}-graph subset, {per_graph} triples/graph): \
         query_view {view_per_query:?}/query vs FROM-NAMED copy {copy_per_query:?}/query \
         ({:.1}x)",
        copy_per_query.as_secs_f64() / view_per_query.as_secs_f64()
    );

    // Concrete-GRAPH variant: the query touches ONE visible graph, but the copy
    // path still rebuilds the whole authorized subset per query — this is the
    // common "read one document" shape.
    let q1 = "SELECT ?s ?o WHERE { GRAPH <http://ex/g7> { ?s <http://ex/p> ?o } }";
    let mut q1_from = String::from("SELECT ?s ?o\n");
    for gi in 0..visible {
        q1_from.push_str(&format!("FROM NAMED <http://ex/g{gi}>\n"));
    }
    q1_from.push_str("WHERE { GRAPH <http://ex/g7> { ?s <http://ex/p> ?o } }");
    assert_eq!(query_view(&v, q1).unwrap().len(), per_graph);
    assert_eq!(query(&g, &q1_from).unwrap().len(), per_graph);
    let t0 = std::time::Instant::now();
    for _ in 0..iters {
        assert_eq!(query_view(&v, q1).unwrap().len(), per_graph);
    }
    let view_one = t0.elapsed() / iters;
    let t0 = std::time::Instant::now();
    for _ in 0..iters {
        assert_eq!(query(&g, &q1_from).unwrap().len(), per_graph);
    }
    let copy_one = t0.elapsed() / iters;
    println!(
        "dataset-view bench, single concrete GRAPH: query_view {view_one:?}/query vs \
         FROM-NAMED copy {copy_one:?}/query ({:.0}x)",
        copy_one.as_secs_f64() / view_one.as_secs_f64()
    );
}
