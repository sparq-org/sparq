//! THE Phase-5 streaming-correctness test (bead sq-vtba, epic sq-dnko): the **streamed**
//! federated result is **multiset-EQUAL** to the Phase-3 materialised result
//! ([`materialize_single_source`]) — for any source-arrival interleaving.
//!
//! The setup mirrors the Phase-3 test: each "remote endpoint" is a faithful SPARQL endpoint
//! over a graph `G`, answered by the **real engine** on an in-process `sparq_core::Graph`
//! serialised to SPARQL-Results-JSON. On top of that, a **`DelayTransport`** injects a
//! per-sub-query sleep so the leaf fetches complete in a controllable, *different* order each
//! run — exercising the non-blocking streaming join under genuinely interleaved arrivals.
//!
//! For every BGP and every delay schedule we assert:
//!
//!  * `stream_single_source(...)` (Phase 5) and `materialize_single_source(...)` (Phase 3)
//!    produce the **same solution multiset** (`solutions_equal`), and
//!  * both equal `sparq_engine::query(&G, <the whole BGP>)` (the ground truth).
//!
//! Equal multisets across every interleaving is exactly the load-bearing invariant the bead
//! asks for: the streaming operators emit before inputs are exhausted, yet lose/duplicate
//! nothing relative to the blocking materialised path. The streaming win (a fast leaf feeding
//! the join while a slow leaf is still in flight) is exercised by the delay schedules.
//!
//! Gated on `fedclient`; with the feature off the file compiles to nothing.
//!
//! sq-vtba — flagged for Fable re-review when available.

#![cfg(feature = "fedclient")]

use sparq_core::Graph;
use sparq_engine::json::to_sparql_json;
use sparq_engine::query;
use sparq_fedclient::{
    materialize_single_source, solutions_equal, FederatedSource, Solution, SourceResolver,
    StreamOptions, Transport,
};
use sparq_fedplan::{
    plan_bgp, select_sources, Bgp, PlanOptions, PredPartition, SourceDescriptor, SourceId,
    StreamJoinOptions, Term, TriplePattern, Var,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// A faithful endpoint-over-`G` transport that ALSO sleeps for a per-sub-query duration
/// before answering, so the leaf fetches complete in a controllable order. The "remote"
/// answer is the engine's own evaluation serialised to SRJ — no network. sq-vtba.
struct DelayTransport {
    graph: Arc<Graph>,
    /// sub-query SPARQL → artificial delay before answering.
    delays: HashMap<String, Duration>,
}

impl Transport for DelayTransport {
    fn fetch(&self, _endpoint: &str, q: &str) -> Result<String, String> {
        if let Some(d) = self.delays.get(q) {
            std::thread::sleep(*d);
        }
        let res = query(&self.graph, q)?;
        Ok(to_sparql_json(&res))
    }
}

/// An `Endpoint`-equivalent `FederatedSource` that is `Send + Sync` (the streaming
/// interpreter needs `Arc<dyn FederatedSource + Send + Sync>`). It delegates to a real
/// `sparq_fedclient::Endpoint` for the SSRF gate + transport, but the engine-backed transport
/// (`DelayTransport`) is the load-bearing part. sq-vtba.
struct EngineSource {
    endpoint: sparq_fedclient::Endpoint,
}

impl FederatedSource for EngineSource {
    fn source_type(&self) -> sparq_fedclient::SourceType<'_> {
        self.endpoint.source_type()
    }
    fn discover(
        &self,
    ) -> Result<(sparq_fedclient::Capability, Option<SourceDescriptor>), sparq_fedclient::FedError>
    {
        self.endpoint.discover()
    }
    fn execute(&self, sub: &sparq_fedclient::SubQuery) -> Result<String, sparq_fedclient::FedError> {
        self.endpoint.execute(sub)
    }
}

fn iri(s: &str) -> Term {
    Term::Iri(s.to_string())
}
fn var(s: &str) -> Term {
    Term::Var(Var::new(s))
}

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

/// Drive BOTH the Phase-5 streaming interpreter and the Phase-3 materialised interpreter over
/// the same plan + engine-backed source (with the given delay schedule for the streaming run)
/// and assert all three (streamed, materialised, ground-truth local eval) carry the same
/// solution multiset. sq-vtba.
fn assert_stream_equals_phase3_and_local(
    bgp: &Bgp,
    descriptor: &SourceDescriptor,
    whole_query: &str,
    delays: HashMap<String, Duration>,
    opts: &StreamOptions,
) {
    let g = graph();
    let descriptors = [descriptor.clone()];
    let sel = select_sources(bgp, &descriptors);
    let tree =
        plan_bgp(bgp, &sel, &descriptors, &PlanOptions::default()).expect("non-empty BGP plans");

    // ── Phase 3 reference (blocking, no delays needed) ──
    let ep_ref = sparq_fedclient::Endpoint::new(
        "http://example.org/sparql",
        Box::new(DelayTransport {
            graph: Arc::clone(&g),
            delays: HashMap::new(),
        }),
    );
    let adapters: Vec<&dyn FederatedSource> = vec![&ep_ref];
    let resolver_ref = SourceResolver::new(bgp, &adapters);
    let reference =
        materialize_single_source(&resolver_ref, &sel, &ep_ref, &tree).expect("phase-3 succeeds");

    // ── Phase 5 streamed (with the interleaving delay schedule) ──
    let ep_stream = sparq_fedclient::Endpoint::new(
        "http://example.org/sparql",
        Box::new(DelayTransport {
            graph: Arc::clone(&g),
            delays,
        }),
    );
    let arc_src: Arc<dyn FederatedSource + Send + Sync> = Arc::new(EngineSource {
        endpoint: ep_stream,
    });
    // The resolver needs an adapter slice for index typing/count; the streaming interpreter
    // answers every leaf through `arc_src`, so this stub is never `execute`d.
    let stub = sparq_fedclient::Endpoint::new(
        "http://example.org/sparql",
        Box::new(DelayTransport {
            graph: Arc::clone(&g),
            delays: HashMap::new(),
        }),
    );
    let stub_adapters: Vec<&dyn FederatedSource> = vec![&stub];
    let resolver = SourceResolver::new(bgp, &stub_adapters);
    let streamed: Vec<Solution> =
        sparq_fedclient::stream_single_source(&resolver, &sel, arc_src, &tree, opts)
            .expect("phase-5 builds")
            .collect_solutions()
            .expect("phase-5 streams without error");
    let s_rows: Vec<Vec<Option<oxrdf::Term>>> = streamed.iter().map(|s| s.cells.clone()).collect();
    let s_vars = streamed
        .first()
        .map(|s| s.vars.clone())
        .unwrap_or_else(|| reference.vars.clone());

    // ── Ground truth: local engine eval of the whole BGP ──
    let local = query(&g, whole_query).expect("local eval succeeds");
    let local_vars: Vec<String> = local.vars.iter().map(|v| v.as_str().to_string()).collect();

    // Streamed == Phase-3 materialised.
    assert!(
        solutions_equal(&s_vars, &s_rows, &reference.vars, &reference.rows),
        "STREAMED must equal Phase-3 materialised.\n streamed = {:?}\n phase3   = {:?}",
        s_rows,
        reference.rows,
    );
    // And both equal local eval (the ground truth).
    assert!(
        solutions_equal(&s_vars, &s_rows, &local_vars, &local.rows),
        "STREAMED must equal local eval.\n streamed = {:?}\n local    = {:?}",
        s_rows,
        local.rows,
    );
}

/// A two-pattern star with FOUR delay schedules: no delay, first-leaf-slow, second-leaf-slow,
/// and both-slow. Each forces a different arrival order into the streaming join; the result
/// must be identical across all of them. sq-vtba.
#[test]
fn two_pattern_star_equals_across_interleavings() {
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/knows"), var("o")),
        TriplePattern::new(var("s"), iri("http://ex/name"), var("n")),
    ]);
    let q = "SELECT ?s ?o ?n WHERE { ?s <http://ex/knows> ?o . ?s <http://ex/name> ?n }";
    let q_knows = "SELECT ?s ?o WHERE { ?s <http://ex/knows> ?o }".to_string();
    let q_name = "SELECT ?s ?n WHERE { ?s <http://ex/name> ?n }".to_string();
    let d = Duration::from_millis(40);
    let schedules: Vec<HashMap<String, Duration>> = vec![
        HashMap::new(),
        HashMap::from([(q_knows.clone(), d)]),
        HashMap::from([(q_name.clone(), d)]),
        HashMap::from([(q_knows.clone(), d), (q_name.clone(), d / 2)]),
    ];
    let opts = StreamOptions {
        workers: 4,
        channel_cap: 8,
        join_opts: StreamJoinOptions::default(),
    };
    for sched in schedules {
        assert_stream_equals_phase3_and_local(
            &bgp,
            &descriptor(&["http://ex/knows", "http://ex/name"]),
            q,
            sched,
            &opts,
        );
    }
}

/// A three-pattern path `?s knows ?o . ?o knows ?z . ?z name ?n` — chained streaming joins.
/// Run under a "middle leaf slow" schedule to interleave the chain. sq-vtba.
#[test]
fn three_pattern_path_equals_with_middle_slow() {
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/knows"), var("o")),
        TriplePattern::new(var("o"), iri("http://ex/knows"), var("z")),
        TriplePattern::new(var("z"), iri("http://ex/name"), var("n")),
    ]);
    let q = "SELECT ?s ?o ?z ?n WHERE { \
        ?s <http://ex/knows> ?o . ?o <http://ex/knows> ?z . ?z <http://ex/name> ?n }";
    let middle = "SELECT ?o ?z WHERE { ?o <http://ex/knows> ?z }".to_string();
    let delays = HashMap::from([(middle, Duration::from_millis(30))]);
    let opts = StreamOptions {
        workers: 3,
        channel_cap: 4,
        join_opts: StreamJoinOptions::default(),
    };
    assert_stream_equals_phase3_and_local(
        &bgp,
        &descriptor(&["http://ex/knows", "http://ex/name"]),
        q,
        delays,
        &opts,
    );
}

/// A typed-literal join under a tiny `StreamJoin` budget so the **spill** path is exercised in
/// the streaming interpreter — the spilled streamed result must still equal Phase 3 / local.
/// sq-vtba.
#[test]
fn typed_literal_join_equals_under_spill() {
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/age"), var("a")),
        TriplePattern::new(var("s"), iri("http://ex/name"), var("n")),
    ]);
    let q = "SELECT ?s ?a ?n WHERE { ?s <http://ex/age> ?a . ?s <http://ex/name> ?n }";
    let opts = StreamOptions {
        workers: 2,
        channel_cap: 2,
        join_opts: StreamJoinOptions {
            mem_budget_tuples: 1, // force spill on the second tuple.
            spill_store: sparq_fedplan::SpillStore::Memory,
        },
    };
    assert_stream_equals_phase3_and_local(
        &bgp,
        &descriptor(&["http://ex/age", "http://ex/name"]),
        q,
        HashMap::new(),
        &opts,
    );
}

/// An empty-result join: the bound object matches nothing, so the streamed path must also be
/// empty (not error, not a spurious row). sq-vtba.
#[test]
fn empty_result_join_streams_empty() {
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/knows"), iri("http://ex/nobody")),
        TriplePattern::new(var("s"), iri("http://ex/name"), var("n")),
    ]);
    let q = "SELECT ?s ?n WHERE { ?s <http://ex/knows> <http://ex/nobody> . ?s <http://ex/name> ?n }";
    assert_stream_equals_phase3_and_local(
        &bgp,
        &descriptor(&["http://ex/knows", "http://ex/name"]),
        q,
        HashMap::new(),
        &StreamOptions::default(),
    );
}

/// Single-pattern (no join): the streamed path is just the fanned-out leaf, which must equal
/// Phase 3 / local. sq-vtba.
#[test]
fn single_pattern_streams_equal() {
    let bgp = Bgp::new(vec![TriplePattern::new(
        var("s"),
        iri("http://ex/knows"),
        var("o"),
    )]);
    assert_stream_equals_phase3_and_local(
        &bgp,
        &descriptor(&["http://ex/knows"]),
        "SELECT ?s ?o WHERE { ?s <http://ex/knows> ?o }",
        HashMap::new(),
        &StreamOptions::default(),
    );
}
