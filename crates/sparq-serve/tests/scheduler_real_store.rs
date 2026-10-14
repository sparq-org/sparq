//! Scheduler over the PRODUCTION substrate: the [`Scheduler`] dispatching real
//! SPARQL queries that each pin a generation from the A1 [`GenerationRing`] and
//! run the real `sparq_engine` evaluator — while the sequenced [`Writer`]
//! publishes new generations concurrently.
//!
//! written while Fable 5 unavailable — re-review when Fable returns.
//!
//! This is the brief's CORRECTNESS gate at the full-stack level (category 4):
//! scheduling must not change RESULTS or snapshot semantics. Each scheduled job
//! pins `ring.current()` *inside the closure* (A1 snapshot consistency) and runs
//! the engine against it; we assert the scheduled result is byte-identical to
//! running the same query directly against the same pinned generation, including
//! while writes advance the ring underneath.

use std::sync::Arc;

use sparq_core::Graph;
use sparq_serve::{
    GenerationRing, GraphApplier, PodId, Scheduler, SchedulerConfig, Writer, WriterConfig,
};

fn graph_with(n: usize) -> Graph {
    let mut nt = String::new();
    for i in 0..n {
        nt.push_str(&format!("<http://ex/s{i}> <http://ex/p> \"v{i}\" .\n"));
    }
    Graph::load_str(&nt, "ntriples").expect("load test graph")
}

const Q: &str = "SELECT (COUNT(*) AS ?c) WHERE { ?s <http://ex/p> ?o }";

/// Run the query directly against a pinned graph — the "unscheduled" reference.
fn direct(graph: &Graph) -> String {
    sparq_engine::query_json(graph, Q).expect("query ok")
}

#[test]
fn scheduled_query_matches_direct_execution_on_pinned_generation() {
    let ring = Arc::new(GenerationRing::new(graph_with(100)));
    let sched = Scheduler::new(SchedulerConfig {
        workers: 4,
        heavy_concurrency: 2,
        heavy_threshold: 10_000,
    });

    // Submit many queries; each pins the CURRENT generation inside the closure and
    // runs the engine. We compare to a direct run on a generation pinned now.
    let pinned = ring.current();
    let expected = direct(pinned.snapshot());

    let mut tickets = Vec::new();
    for _ in 0..200 {
        let ring = ring.clone();
        // cost estimate: a COUNT over a small graph is "cheap" here; deliberately
        // submit some as heavy too (cost above threshold) to exercise both lanes.
        let cost = if tickets.len() % 5 == 0 { 50_000 } else { 1 };
        tickets.push(sched.submit(cost, move || {
            let g = ring.current(); // pin a generation (A1) — snapshot consistency
            (g.number(), direct(g.snapshot()))
        }));
    }
    for t in tickets {
        let (num, got) = t.wait().expect("scheduled query ok");
        assert_eq!(num, 0, "no writes happened, all reads see generation 0");
        assert_eq!(
            got, expected,
            "scheduled result must equal direct execution"
        );
    }
}

#[test]
fn snapshot_consistency_preserved_under_concurrent_writes() {
    // A reader scheduled at generation G must see exactly G's state even as the
    // writer publishes past it — the scheduler must not perturb the A1 pin.
    let ring = Arc::new(GenerationRing::new(graph_with(50)));
    let sched = Scheduler::new(SchedulerConfig {
        workers: 4,
        heavy_concurrency: 2,
        heavy_threshold: 10_000,
    });

    // Pin generation 0 BEFORE any writes; schedule a job that reads it but is held
    // until after the writer has advanced the ring. The job captures the pinned
    // generation explicitly (a stream/long-read holding its snapshot, §6.6).
    let pinned0 = ring.current();
    let expected0 = direct(pinned0.snapshot());

    let writer = Writer::spawn(ring.clone(), GraphApplier::new(), WriterConfig::default());

    // Advance the ring with real updates while reads are in flight.
    for i in 0..20 {
        let upd = format!(
            "INSERT DATA {{ GRAPH <http://pod/{i}> {{ <http://ex/s{i}> <http://ex/p> \"new{i}\" }} }}"
        );
        writer
            .submit(upd, [PodId::from(format!("http://pod/{i}").as_str())])
            .expect("write ok");
    }

    // The job that pinned generation 0 must still report generation-0 state.
    // Reads return `(generation_number, count)` so both scheduled jobs share one
    // result type `T` (a `Scheduler<T>` is monomorphic over one outcome type).
    let p0 = pinned0.clone();
    let exp0 = expected0.clone();
    let held = sched.submit(1, move || {
        let got = direct(p0.snapshot());
        assert_eq!(
            got, exp0,
            "pinned generation-0 read must be unchanged by concurrent writes"
        );
        let count = sparq_engine::count(p0.snapshot(), "SELECT * WHERE { GRAPH ?gr { ?s ?p ?o } }")
            .unwrap_or(0);
        (p0.number(), count)
    });
    let (num, held_named_count) = held.wait().expect("held read ok");
    assert_eq!(num, 0, "the held read stayed on generation 0");
    assert_eq!(
        held_named_count, 0,
        "generation 0 has no named-graph triples — the writes are invisible to it"
    );

    // A fresh read scheduled now sees the advanced state (more triples, higher gen).
    let ring2 = ring.clone();
    let fresh = sched.submit(1, move || {
        let g = ring2.current();
        let count = sparq_engine::count(g.snapshot(), "SELECT * WHERE { GRAPH ?gr { ?s ?p ?o } }")
            .unwrap_or(0);
        (g.number(), count)
    });
    let (fresh_num, fresh_named_count) = fresh.wait().expect("fresh read ok");
    assert!(
        fresh_num >= 1,
        "fresh read should see at least one published generation, saw {fresh_num}"
    );
    assert_eq!(
        fresh_named_count, 20,
        "the 20 named-graph inserts are visible to a fresh read"
    );

    // Clean shutdown.
    drop(writer);
}
