//! THE correctness test for multi-source UNION-per-leaf fan-out (bead sq-7yf0, epic
//! sq-3183): the federated result of a plan whose leaves are retained by **more than one
//! source** EQUALS local `sparq-engine` evaluation over the **union of every source's
//! graph**.
//!
//! Each "remote endpoint" is a faithful SPARQL endpoint over its OWN graph `G_i` (an
//! [`EngineTransport`] that runs each leaf sub-query through the real engine on an
//! in-process `sparq_core::Graph`, serialised to SPARQL-Results-JSON). When the planner
//! retains several sources for a leaf,
//! [`materialize_multi_source`](sparq_fedclient::materialize_multi_source) /
//! [`stream_multi_source`](sparq_fedclient::stream_multi_source) answer that leaf as the
//! **bag-union** of every retained source's solutions — exactly SPARQL UNION semantics over
//! the per-source solution sequences. The BGP answer is then the natural join of those
//! per-leaf unions.
//!
//! The canonical answer is `sparq_engine::query(&G_union, Q)` where `G_union` is the single
//! graph holding the union of every source's triples: for a federation over endpoints that
//! each hold a fragment of the data, the federated answer must equal evaluating the whole
//! query over the merged dataset. We assert
//! [`solutions_equal`](sparq_fedclient::solutions_equal) for BOTH the materialised and the
//! streaming multi-source interpreters (same multiset, any source-arrival interleaving).
//!
//! Gated on `fedclient`; the default build compiles this file to nothing.
//!
//! sq-7yf0 — flagged for Fable re-review when available.

#![cfg(feature = "fedclient")]

use sparq_core::Graph;
use sparq_engine::json::to_sparql_json;
use sparq_engine::query;
use sparq_fedclient::{
    materialize_multi_source, solutions_equal, stream_multi_source, Endpoint, FederatedSource,
    Relation, SourceResolver, StreamOptions, Transport,
};
use sparq_fedplan::{
    plan_bgp, select_sources, Bgp, PlanOptions, PredPartition, SourceDescriptor, SourceId, Term,
    TriplePattern, Var,
};
use std::sync::Arc;

/// A transport that answers a sub-query by evaluating it against ONE local engine `Graph` —
/// a faithful stand-in for a conformant SPARQL endpoint over that graph's fragment of the
/// federation. No network: the "remote" answer is that source's own engine evaluation.
struct EngineTransport {
    graph: Arc<Graph>,
}

impl Transport for EngineTransport {
    fn fetch(&self, _endpoint: &str, q: &str) -> Result<String, String> {
        let res = query(&self.graph, q)?;
        Ok(to_sparql_json(&res))
    }
}

fn iri(s: &str) -> Term {
    Term::Iri(s.to_string())
}
fn var(s: &str) -> Term {
    Term::Var(Var::new(s))
}

/// A per-source descriptor declaring `preds` so `select_sources` retains the source for
/// every pattern it covers. Identical predicate coverage across sources ⇒ a leaf retains
/// EVERY such source (the multi-source case). sq-7yf0.
fn descriptor(id: &str, preds: &[&str]) -> SourceDescriptor {
    let mut b = SourceDescriptor::builder(SourceId::new(id)).total_triples(1000);
    for p in preds {
        b = b.predicate(PredPartition {
            predicate: (*p).into(),
            triples: 100,
            distinct_subjects: 50,
            distinct_objects: 50,
        });
    }
    b.build()
}

/// Build the federation harness for `bgp` over the given per-source `(turtle, preds)` set:
/// returns the descriptors, the per-pattern selection, and the plan. The endpoints are kept
/// alive by the caller (they borrow their graphs). sq-7yf0.
fn plan(
    bgp: &Bgp,
    sources: &[(&str, &[&str])],
) -> (Vec<SourceDescriptor>, Vec<sparq_fedplan::PatternSources>, sparq_fedplan::JoinTree) {
    let descriptors: Vec<SourceDescriptor> = sources
        .iter()
        .enumerate()
        .map(|(i, (_g, preds))| descriptor(&format!("S{i}"), preds))
        .collect();
    let sel = select_sources(bgp, &descriptors);
    let tree = plan_bgp(bgp, &sel, &descriptors, &PlanOptions::default())
        .expect("non-empty BGP yields a plan");
    (descriptors, sel, tree)
}

/// Assert the multi-source federated result (materialised AND streaming) equals local
/// evaluation of `whole_query` over the union of every source graph. sq-7yf0.
fn assert_multi_source_equals_union(
    bgp: &Bgp,
    sources: &[(&str, &[&str])],
    whole_query: &str,
) {
    // Per-source engine graphs + endpoint adapters (index i == descriptor i == source i).
    let graphs: Vec<Arc<Graph>> = sources
        .iter()
        .map(|(g, _)| Arc::new(Graph::load_str(g, "turtle").unwrap()))
        .collect();
    let endpoints: Vec<Endpoint> = graphs
        .iter()
        .map(|g| {
            Endpoint::new(
                "http://example.org/sparql",
                Box::new(EngineTransport {
                    graph: Arc::clone(g),
                }),
            )
        })
        .collect();

    let (_descriptors, sel, tree) = plan(bgp, sources);

    // The borrowed adapter slice for the resolver (range-checked index → adapter).
    let adapters: Vec<&dyn FederatedSource> = endpoints.iter().map(|e| e as &dyn FederatedSource).collect();
    let resolver = SourceResolver::new(bgp, &adapters);

    // Materialised multi-source fan-out.
    let fed: Relation = materialize_multi_source(&resolver, &sel, &tree)
        .expect("multi-source interpreter succeeds");

    // The canonical answer: query the UNION graph (all sources' triples merged).
    let union_graph = {
        let mut all = String::new();
        for (g, _) in sources {
            all.push_str(g);
            all.push('\n');
        }
        Arc::new(Graph::load_str(&all, "turtle").unwrap())
    };
    let local = query(&union_graph, whole_query).expect("local eval succeeds");
    let local_vars: Vec<String> = local.vars.iter().map(|v| v.as_str().to_string()).collect();

    assert!(
        solutions_equal(&fed.vars, &fed.rows, &local_vars, &local.rows),
        "materialised multi-source result must equal local eval over the union graph.\n  fed = {:?}\n  local vars = {:?} rows = {:?}",
        fed,
        local_vars,
        local.rows,
    );

    // Streaming multi-source fan-out (owned Arc adapters, indexed the same way).
    let arc_adapters: Vec<Arc<dyn FederatedSource + Send + Sync>> = graphs
        .iter()
        .map(|g| {
            let ep: Arc<dyn FederatedSource + Send + Sync> = Arc::new(Endpoint::new(
                "http://example.org/sparql",
                Box::new(EngineTransport {
                    graph: Arc::clone(g),
                }),
            ));
            ep
        })
        .collect();
    let stream =
        stream_multi_source(&resolver, &sel, &arc_adapters, &tree, &StreamOptions::default())
            .expect("streaming multi-source interpreter builds");
    let got = stream.collect_solutions().unwrap();
    let got_rows: Vec<Vec<Option<oxrdf::Term>>> = got.iter().map(|s| s.cells.clone()).collect();
    let got_vars = got
        .first()
        .map(|s| s.vars.clone())
        .unwrap_or_else(|| fed.vars.clone());
    assert!(
        solutions_equal(&got_vars, &got_rows, &local_vars, &local.rows),
        "streamed multi-source result must equal local eval over the union graph.\n  streamed = {:?}\n  local vars = {:?} rows = {:?}",
        got_rows,
        local_vars,
        local.rows,
    );
}

// Two endpoints, each holding a DISJOINT fragment of the same predicate's triples — the
// canonical multi-source case the bead names. A single leaf `?s ex:knows ?o` retains both
// sources; the union of their solutions must equal the merged graph's.
const SRC_A: &str = r#"
@prefix ex: <http://ex/> .
ex:alice ex:knows ex:bob .
ex:alice ex:knows ex:carol .
ex:alice ex:name "Alice" .
ex:bob   ex:name "Bob" .
"#;
const SRC_B: &str = r#"
@prefix ex: <http://ex/> .
ex:carol ex:knows ex:dave .
ex:bob   ex:knows ex:dave .
ex:carol ex:name "Carol" .
ex:dave  ex:name "Dave" .
"#;

#[test]
fn single_leaf_two_sources_unions() {
    // ?s ex:knows ?o — one leaf, BOTH sources hold ex:knows ⇒ a per-source UNION.
    let bgp = Bgp::new(vec![TriplePattern::new(
        var("s"),
        iri("http://ex/knows"),
        var("o"),
    )]);
    assert_multi_source_equals_union(
        &bgp,
        &[(SRC_A, &["http://ex/knows"]), (SRC_B, &["http://ex/knows"])],
        "SELECT ?s ?o WHERE { ?s <http://ex/knows> ?o }",
    );
}

#[test]
fn star_join_with_multi_source_leaves_unions() {
    // ?s ex:knows ?o . ?s ex:name ?n — a star on ?s where BOTH leaves are multi-source.
    // The join is over the per-leaf unions: a ?s/?o pair from one source can join a ?s/?n
    // pair from the OTHER (cross-source join), which only the union-per-leaf semantics
    // capture (e.g. bob's ex:knows lives in B but his ex:name in A).
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/knows"), var("o")),
        TriplePattern::new(var("s"), iri("http://ex/name"), var("n")),
    ]);
    assert_multi_source_equals_union(
        &bgp,
        &[
            (SRC_A, &["http://ex/knows", "http://ex/name"]),
            (SRC_B, &["http://ex/knows", "http://ex/name"]),
        ],
        "SELECT ?s ?o ?n WHERE { ?s <http://ex/knows> ?o . ?s <http://ex/name> ?n }",
    );
}

#[test]
fn three_sources_path_join_unions() {
    // Three sources, a 2-hop path + name lookup; every leaf retains all three sources whose
    // descriptor covers the predicate. Verifies the fan-out generalises past two sources.
    const SRC_C: &str = r#"
@prefix ex: <http://ex/> .
ex:dave  ex:knows ex:erin .
ex:erin  ex:name "Erin" .
"#;
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/knows"), var("o")),
        TriplePattern::new(var("o"), iri("http://ex/knows"), var("z")),
        TriplePattern::new(var("z"), iri("http://ex/name"), var("n")),
    ]);
    assert_multi_source_equals_union(
        &bgp,
        &[
            (SRC_A, &["http://ex/knows", "http://ex/name"]),
            (SRC_B, &["http://ex/knows", "http://ex/name"]),
            (SRC_C, &["http://ex/knows", "http://ex/name"]),
        ],
        "SELECT ?s ?o ?z ?n WHERE { \
            ?s <http://ex/knows> ?o . \
            ?o <http://ex/knows> ?z . \
            ?z <http://ex/name> ?n }",
    );
}

#[test]
fn mixed_single_and_multi_source_leaves_unions() {
    // One leaf is single-source (only A declares ex:age), the other multi-source (both
    // declare ex:name) — the interpreter must NOT reject the single-source leaf and must
    // union the multi-source one.
    const SRC_AGE: &str = r#"
@prefix ex: <http://ex/> .
ex:alice ex:age "30"^^<http://www.w3.org/2001/XMLSchema#integer> .
ex:alice ex:name "Alice" .
"#;
    const SRC_NAME: &str = r#"
@prefix ex: <http://ex/> .
ex:alice ex:name "Alice-dup" .
ex:bob   ex:name "Bob" .
"#;
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/age"), var("a")),
        TriplePattern::new(var("s"), iri("http://ex/name"), var("n")),
    ]);
    assert_multi_source_equals_union(
        &bgp,
        &[
            (SRC_AGE, &["http://ex/age", "http://ex/name"]),
            (SRC_NAME, &["http://ex/name"]),
        ],
        "SELECT ?s ?a ?n WHERE { ?s <http://ex/age> ?a . ?s <http://ex/name> ?n }",
    );
}
