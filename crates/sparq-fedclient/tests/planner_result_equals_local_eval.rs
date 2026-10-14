//! THE Phase-3 correctness test (bead sq-j27p, epic sq-dnko): the materialised
//! single-source federated result **EQUALS** local `sparq-engine` evaluation of the same
//! query.
//!
//! The setup makes the "remote endpoint" a faithful SPARQL endpoint over a graph `G`: a
//! [`Transport`] double answers every leaf sub-query by running it through the **real
//! engine** on an in-process `sparq_core::Graph` and serialising the solutions to
//! SPARQL-Results-JSON with `sparq_engine::to_sparql_json` — exactly the SRJ a conformant
//! endpoint over `G` would return. The Phase-3 interpreter
//! ([`materialize_single_source`](sparq_fedclient::materialize_single_source)) then:
//!
//!  1. lowers each `sparq-fedplan` plan leaf to a single-pattern `SELECT` sub-query,
//!  2. fetches its SRJ through the Phase-2 `Endpoint` adapter (egress-guarded transport),
//!  3. parses the SRJ back into `oxrdf::Term` solutions, and
//!  4. natural-joins the leaves in the plan's join order.
//!
//! We assert [`solutions_equal`](sparq_fedclient::solutions_equal) between that materialised
//! relation and `sparq_engine::query(&G, <the whole BGP>)`. Equal solution multisets ⇒ the
//! lowering + resolver + SRJ parse + join faithfully reproduce local evaluation. This is the
//! load-bearing invariant the bead asks for, exercised on the REAL path (no mock engine —
//! the canonical answer IS the engine).
//!
//! This test is gated on the `fedclient` feature; with the feature off the file compiles to
//! nothing (the whole module is `#[cfg]`-empty), so the default build is unaffected.
//!
//! sq-j27p — flagged for Fable re-review when available.

#![cfg(feature = "fedclient")]

use sparq_core::Graph;
use sparq_engine::json::to_sparql_json;
use sparq_engine::query;
use sparq_fedclient::{
    materialize_single_source, solutions_equal, FederatedSource, Relation, SourceResolver,
    Transport,
};
use sparq_fedplan::{
    plan_bgp, select_sources, Bgp, PlanOptions, PredPartition, SourceDescriptor, SourceId, Term,
    TriplePattern, Var,
};
use std::sync::Arc;

/// A transport that answers a sub-query by evaluating it against a local engine `Graph` and
/// serialising to SPARQL-Results-JSON — a faithful stand-in for a conformant SPARQL endpoint
/// over that graph. No network: the "remote" answer is the engine's own evaluation.
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

/// Build a one-source descriptor declaring each predicate in `preds` (so `select_sources`
/// retains the single source for every pattern). The stats only steer join *order*; the
/// result-equivalence holds for any order. sq-j27p.
fn descriptor(preds: &[&str]) -> SourceDescriptor {
    let mut b = SourceDescriptor::builder(SourceId::new("S")).total_triples(1000);
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

/// Lower + materialise `bgp` against the single engine-backed source, and assert the
/// materialised relation's solution multiset equals `sparq_engine::query(&G, whole_query)`.
fn assert_federated_equals_local(
    graph: Arc<Graph>,
    bgp: &Bgp,
    descriptor: &SourceDescriptor,
    whole_query: &str,
) {
    let descriptors = [descriptor.clone()];
    let sel = select_sources(bgp, &descriptors);
    let tree = plan_bgp(bgp, &sel, &descriptors, &PlanOptions::default())
        .expect("non-empty BGP yields a plan");

    let ep = sparq_fedclient::Endpoint::new(
        "http://example.org/sparql",
        Box::new(EngineTransport {
            graph: Arc::clone(&graph),
        }),
    );
    let adapters: Vec<&dyn FederatedSource> = vec![&ep];
    let resolver = SourceResolver::new(bgp, &adapters);

    let fed: Relation =
        materialize_single_source(&resolver, &sel, &ep, &tree).expect("interpreter succeeds");

    let local = query(&graph, whole_query).expect("local eval succeeds");
    let local_vars: Vec<String> = local.vars.iter().map(|v| v.as_str().to_string()).collect();

    assert!(
        solutions_equal(&fed.vars, &fed.rows, &local_vars, &local.rows),
        "federated result must equal local eval.\n  federated = {:?}\n  local vars = {:?}, rows = {:?}",
        fed,
        local_vars,
        local.rows,
    );
}

const DATA: &str = r#"
@prefix ex: <http://ex/> .
ex:alice ex:knows ex:bob .
ex:alice ex:knows ex:carol .
ex:bob   ex:knows ex:dave .
ex:carol ex:knows ex:dave .
ex:alice ex:name "Alice" .
ex:bob   ex:name "Bob" .
ex:carol ex:name "Carol" .
ex:dave  ex:name "Dave" .
ex:alice ex:age "30"^^<http://www.w3.org/2001/XMLSchema#integer> .
ex:bob   ex:age "25"^^<http://www.w3.org/2001/XMLSchema#integer> .
"#;

fn graph() -> Arc<Graph> {
    Arc::new(Graph::load_str(DATA, "turtle").unwrap())
}

#[test]
fn single_pattern_equals_local() {
    // ?s ex:knows ?o — one leaf, no join.
    let bgp = Bgp::new(vec![TriplePattern::new(
        var("s"),
        iri("http://ex/knows"),
        var("o"),
    )]);
    assert_federated_equals_local(
        graph(),
        &bgp,
        &descriptor(&["http://ex/knows"]),
        "SELECT ?s ?o WHERE { ?s <http://ex/knows> ?o }",
    );
}

#[test]
fn two_pattern_star_join_equals_local() {
    // ?s ex:knows ?o . ?s ex:name ?n  — a star on ?s.
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/knows"), var("o")),
        TriplePattern::new(var("s"), iri("http://ex/name"), var("n")),
    ]);
    assert_federated_equals_local(
        graph(),
        &bgp,
        &descriptor(&["http://ex/knows", "http://ex/name"]),
        "SELECT ?s ?o ?n WHERE { ?s <http://ex/knows> ?o . ?s <http://ex/name> ?n }",
    );
}

#[test]
fn three_pattern_path_join_equals_local() {
    // ?s ex:knows ?o . ?o ex:knows ?z . ?z ex:name ?n  — a 2-hop path + a name lookup.
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/knows"), var("o")),
        TriplePattern::new(var("o"), iri("http://ex/knows"), var("z")),
        TriplePattern::new(var("z"), iri("http://ex/name"), var("n")),
    ]);
    assert_federated_equals_local(
        graph(),
        &bgp,
        &descriptor(&["http://ex/knows", "http://ex/name"]),
        "SELECT ?s ?o ?z ?n WHERE { \
            ?s <http://ex/knows> ?o . \
            ?o <http://ex/knows> ?z . \
            ?z <http://ex/name> ?n }",
    );
}

#[test]
fn bound_object_pattern_equals_local() {
    // ?s ex:knows ex:dave . ?s ex:name ?n  — a bound object narrows the first leaf.
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/knows"), iri("http://ex/dave")),
        TriplePattern::new(var("s"), iri("http://ex/name"), var("n")),
    ]);
    assert_federated_equals_local(
        graph(),
        &bgp,
        &descriptor(&["http://ex/knows", "http://ex/name"]),
        "SELECT ?s ?n WHERE { ?s <http://ex/knows> <http://ex/dave> . ?s <http://ex/name> ?n }",
    );
}

#[test]
fn typed_literal_join_equals_local() {
    // ?s ex:age ?a . ?s ex:name ?n  — the object is a typed (xsd:integer) literal; the SRJ
    // datatype round-trip must reproduce the engine's term exactly.
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/age"), var("a")),
        TriplePattern::new(var("s"), iri("http://ex/name"), var("n")),
    ]);
    assert_federated_equals_local(
        graph(),
        &bgp,
        &descriptor(&["http://ex/age", "http://ex/name"]),
        "SELECT ?s ?a ?n WHERE { ?s <http://ex/age> ?a . ?s <http://ex/name> ?n }",
    );
}

#[test]
fn empty_result_join_equals_local() {
    // A star whose two arms never co-occur on a subject in this graph ⇒ empty result; the
    // federated path must also be empty (not error, not a spurious row).
    let bgp = Bgp::new(vec![
        // ex:dave has a name but no outgoing ex:knows ⇒ the join on ?s is empty for daves,
        // but alice/bob/carol DO have both, so we use a predicate pair with no overlap:
        // ?s ex:age ?a . ?s ex:knows ?o — only alice/bob have an age, both also know someone,
        // so to force empty we bind the object to a non-existent node.
        TriplePattern::new(var("s"), iri("http://ex/knows"), iri("http://ex/nobody")),
        TriplePattern::new(var("s"), iri("http://ex/name"), var("n")),
    ]);
    assert_federated_equals_local(
        graph(),
        &bgp,
        &descriptor(&["http://ex/knows", "http://ex/name"]),
        "SELECT ?s ?n WHERE { ?s <http://ex/knows> <http://ex/nobody> . ?s <http://ex/name> ?n }",
    );
}
