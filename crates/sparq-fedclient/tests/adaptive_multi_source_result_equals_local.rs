//! End-to-end correctness for the **multi-source (union-arm) ADAPTIVE loop** (bead
//! `sq-xw8zz`): the adaptive federated result of a plan whose leaves are retained by more
//! than one source EQUALS local `sparq-engine` evaluation over the **union of every
//! source's graph** — the same oracle `tests/multi_source_union_result_equals_local.rs`
//! holds the static/streaming multi-source interpreters to (`sq-7yf0`) — AND the live run
//! records a DISTINCT per-arm latency for every source, which the opt-in
//! [`LatencyAggregation::CardinalityWeighted`] policy (`sq-s5kd`) consumes at the operator
//! boundaries.
//!
//! Each "remote endpoint" is a faithful in-process SPARQL endpoint over its OWN graph
//! fragment (an [`EngineTransport`] running each leaf sub-query through the real engine,
//! serialised to SPARQL-Results-JSON — the `sq-my8wd` mock-endpoint harness style); one arm
//! is additionally wrapped in a [`SleepTransport`] so the two arms have a REAL, measurable
//! latency gap. The fragments are triple-disjoint, so the federated bag-union multiset
//! equals the merged-graph answer.
//!
//! Gated on `fedclient-adaptive`; the default build compiles this file to nothing.
//!
//! sq-xw8zz.

#![cfg(feature = "fedclient-adaptive")]

use sparq_core::Graph;
use sparq_engine::json::to_sparql_json;
use sparq_engine::query;
use sparq_fedclient::{
    execute_adaptive_multi_source, solutions_equal, Endpoint, FederatedSource, SourceResolver,
    Transport,
};
use sparq_fedplan::{
    plan_bgp, select_sources, Bgp, LatencyAggregation, PlanOptions, PredPartition, ReplanPolicy,
    SourceDescriptor, SourceId, Term, TriplePattern, Var,
};
use std::sync::Arc;
use std::time::Duration;

/// A transport that answers a sub-query by evaluating it against ONE local engine `Graph` —
/// a faithful stand-in for a conformant SPARQL endpoint over that graph's fragment of the
/// federation (the `sq-7yf0`/`sq-my8wd` harness). sq-xw8zz.
struct EngineTransport {
    graph: Arc<Graph>,
}

impl Transport for EngineTransport {
    fn fetch(&self, _endpoint: &str, q: &str) -> Result<String, String> {
        let res = query(&self.graph, q)?;
        Ok(to_sparql_json(&res))
    }
}

/// [`EngineTransport`] plus a fixed pre-answer sleep — gives one arm a REAL, measurable
/// wall-clock latency so the per-arm observations are provably distinct. sq-xw8zz.
struct SleepTransport {
    inner: EngineTransport,
    delay: Duration,
}

impl Transport for SleepTransport {
    fn fetch(&self, endpoint: &str, q: &str) -> Result<String, String> {
        std::thread::sleep(self.delay);
        self.inner.fetch(endpoint, q)
    }
}

fn iri(s: &str) -> Term {
    Term::Iri(s.to_string())
}
fn var(s: &str) -> Term {
    Term::Var(Var::new(s))
}

/// A per-source descriptor declaring `preds` so `select_sources` retains the source for
/// every pattern it covers; identical coverage across sources ⇒ every leaf is a genuine
/// multi-arm union leaf. The estimates are deliberately far from the observed handful of
/// rows so the re-planner has real divergence to consider. sq-xw8zz.
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

// Two endpoints, each holding a TRIPLE-DISJOINT fragment: cross-source joins are required
// (e.g. `alice knows carol` lives on A while `carol name "Carol"` lives on B).
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

/// THE sq-xw8zz end-to-end assertion on the REAL engine path: adaptive union-arm execution
/// under the `CardinalityWeighted` policy returns the merged-graph answer, with two distinct
/// live per-arm latencies recorded and the re-planner consulted on them.
#[test]
fn adaptive_multi_source_equals_local_union_eval_with_live_latencies() {
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/knows"), var("o")),
        TriplePattern::new(var("o"), iri("http://ex/name"), var("n")),
    ]);
    let preds: &[&str] = &["http://ex/knows", "http://ex/name"];
    let descriptors = [descriptor("A", preds), descriptor("B", preds)];
    let sel = select_sources(&bgp, &descriptors);
    for ps in &sel {
        assert_eq!(
            ps.candidates.len(),
            2,
            "every leaf must be a 2-arm union leaf (pattern {})",
            ps.pattern
        );
    }
    let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default())
        .expect("non-empty BGP yields a plan");

    let g_a = Arc::new(Graph::load_str(SRC_A, "turtle").unwrap());
    let g_b = Arc::new(Graph::load_str(SRC_B, "turtle").unwrap());
    // Arm A answers at engine speed; arm B sleeps 60ms per fetch — a real latency gap.
    let ep_a = Endpoint::new(
        "http://example.org/sparql",
        Box::new(EngineTransport {
            graph: Arc::clone(&g_a),
        }),
    );
    let ep_b = Endpoint::new(
        "http://example.org/sparql",
        Box::new(SleepTransport {
            inner: EngineTransport {
                graph: Arc::clone(&g_b),
            },
            delay: Duration::from_millis(60),
        }),
    );
    let adapters: Vec<&dyn FederatedSource> = vec![&ep_a, &ep_b];
    let resolver = SourceResolver::new(&bgp, &adapters);

    let policy = ReplanPolicy {
        latency_aggregation: LatencyAggregation::CardinalityWeighted,
        ..ReplanPolicy::default()
    };
    let out = execute_adaptive_multi_source(
        &bgp,
        &descriptors,
        &resolver,
        &sel,
        &tree,
        PlanOptions::default(),
        policy,
    )
    .expect("adaptive multi-source execution succeeds");

    // The canonical answer: local engine eval over the UNION graph.
    let union_graph = Arc::new(Graph::load_str(&format!("{SRC_A}\n{SRC_B}"), "turtle").unwrap());
    let local = query(
        &union_graph,
        "SELECT ?s ?o ?n WHERE { ?s <http://ex/knows> ?o . ?o <http://ex/name> ?n }",
    )
    .expect("local eval succeeds");
    let local_vars: Vec<String> = local.vars.iter().map(|v| v.as_str().to_string()).collect();
    assert!(
        !local.rows.is_empty(),
        "oracle must be non-vacuous (cross-source joins exist)"
    );
    assert!(
        solutions_equal(&out.relation.vars, &out.relation.rows, &local_vars, &local.rows),
        "adaptive multi-source result must equal local eval over the union graph.\n  fed = {:?}\n  local vars = {:?} rows = {:?}",
        out.relation,
        local_vars,
        local.rows,
    );

    // LIVE per-arm observations: both arms recorded, and the slept arm is measurably slower
    // (`thread::sleep` guarantees AT LEAST the requested 60ms; the EWMA of >=1 such fetches
    // cannot fall below it).
    let fast = out
        .stats
        .observed_latency_of(0)
        .expect("arm A latency recorded");
    let slow = out
        .stats
        .observed_latency_of(1)
        .expect("arm B latency recorded");
    assert!(slow >= 59.0, "slept arm >= its sleep floor, got {slow}ms");
    assert!(
        slow > fast,
        "per-arm latencies must be DISTINCT (slow {slow}ms > fast {fast}ms)"
    );

    // The CardinalityWeighted policy was consulted at the operator boundary with those stats
    // (a 2-pattern plan has exactly one considered boundary).
    assert_eq!(out.replans.len(), 1, "one operator boundary considered");
}
