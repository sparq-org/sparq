//! sq-xa15c (#1248 items 1+2) — REGRESSION TEST for the in-process
//! embedding seam.
//!
//! Proves that the extracted, runtime-agnostic [`GenerationRing`] core
//! (`sparq-serve`, factored out of `sparq-server`'s axum/tokio glue) preserves the
//! `fork → update → publish` + generation-pinning semantics an embedder depends on,
//! and that the [`embed`] facade routes through that same engine path:
//!
//! - a writer **forks** a base `Graph`, applies a real SPARQL Update, and
//!   **publishes** the new immutable generation (the production
//!   [`GraphApplier`]/`update_in_place` path — the same code `sparq-server` runs);
//! - a concurrent reader that **pinned** an older generation keeps a
//!   snapshot-consistent view: a SELECT against its pinned snapshot returns the OLD
//!   answer no matter how far the writer publishes past it;
//! - generations advance monotonically and a query against `ring.current()` sees the
//!   NEW answer.
//!
//! If a future refactor reintroduces a private `GenerationRing` copy in
//! `sparq-server`, or breaks fork/publish snapshot isolation, this test fails.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use sparq_core::Graph;
use sparq_serve::embed::{self, ApplyUpdates, GenerationRing, GraphApplier, PodId};

/// One named-graph-per-document resource graph, seeded with `n` triples on a single
/// subject (`<http://ex/r> <http://ex/p> "vK">`), so a `COUNT(*)` over it reads `n`.
fn resource_graph(n: usize) -> Graph {
    let mut nt = String::new();
    for i in 0..n {
        nt.push_str(&format!("<http://ex/r> <http://ex/p> \"v{i}\" .\n"));
    }
    Graph::load_str(&nt, "ntriples").expect("load resource graph")
}

fn count(graph: &Graph) -> usize {
    // Goes through the SAME facade read path an embedder uses.
    let json = embed::query_json(graph, "SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }")
        .expect("count query");
    // The single COUNT row carries the value as an xsd:integer literal; pull it out
    // of the SPARQL-JSON body without a JSON dependency.
    let needle = "\"value\":\"";
    let start = json.rfind(needle).expect("count value present") + needle.len();
    let end = json[start..].find('"').expect("close quote") + start;
    json[start..end].parse().expect("integer count")
}

/// fork → update → publish through the production applier, with a reader pinning an
/// old generation throughout: snapshot isolation + monotone generations.
#[test]
fn fork_update_publish_preserves_pinned_snapshot_and_advances_generations() {
    // Seed: a resource with 3 triples, generation 0.
    let ring = GenerationRing::new(resource_graph(3));
    let mut applier = GraphApplier::new();

    // A reader pins generation 0 (e.g. a long-lived stream / an in-flight request).
    let pinned_gen0 = ring.current();
    assert_eq!(pinned_gen0.number(), 0);
    assert_eq!(count(pinned_gen0.snapshot()), 3, "pinned view starts at 3");

    // Writer publishes 5 generations, each: fork the current snapshot, apply a real
    // SPARQL INSERT through the engine, publish the sealed graph. This is exactly the
    // sequenced writer's per-commit cycle (applier.rs), driven directly so the test
    // owns the timing.
    for g in 1..=5u64 {
        let base = ring.current();
        let mut working = applier.fork(base.snapshot()).expect("fork");
        embed::update_in_place(
            &mut working,
            &format!("INSERT DATA {{ <http://ex/r> <http://ex/q> \"add{g}\" }}"),
        )
        .expect("update_in_place");
        let sealed = applier.seal(working).expect("seal");
        let published = ring.publish(sealed, [PodId::from("http://ex/r")]);
        assert_eq!(published.number(), g, "generations advance by exactly 1");
    }

    // Generations advanced; the current snapshot reflects all 5 inserts.
    let current = ring.current();
    assert_eq!(current.number(), 5);
    assert_eq!(count(current.snapshot()), 8, "3 seed + 5 inserts");
    assert_eq!(
        current.epochs().epoch(&PodId::from("http://ex/r")),
        5,
        "the touched pod's epoch bumped once per publish"
    );

    // The load-bearing invariant: the reader's PINNED generation 0 is byte-stable —
    // a query against it still returns the OLD answer, untouched by 5 fork/publishes.
    assert_eq!(pinned_gen0.number(), 0);
    assert_eq!(
        count(pinned_gen0.snapshot()),
        3,
        "pinned snapshot unaffected by concurrent fork/update/publish"
    );

    // Read-your-writes: the generation that acked publish #5 satisfies token 5.
    assert!(ring.read_your_writes(5).is_some());
    assert!(
        ring.read_your_writes(6).is_none(),
        "a not-yet-applied commit token is not satisfiable"
    );
}

/// Concurrent: while a writer thread forks/updates/publishes, a reader thread holds
/// a pin and keeps querying it — it must ALWAYS see its own snapshot, never a torn
/// or advanced state. Mirrors a streaming reader racing the sequenced writer.
#[test]
fn concurrent_reader_pins_consistent_snapshot_across_publishes() {
    let ring = Arc::new(GenerationRing::new(resource_graph(4)));
    let start = Arc::new(Barrier::new(2));
    let max_seen = Arc::new(AtomicU64::new(0));

    let writer = {
        let ring = ring.clone();
        let start = start.clone();
        let max_seen = max_seen.clone();
        std::thread::spawn(move || {
            let mut applier = GraphApplier::new();
            start.wait();
            for g in 1..=30u64 {
                let base = ring.current();
                let mut working = applier.fork(base.snapshot()).expect("fork");
                embed::update_in_place(
                    &mut working,
                    &format!("INSERT DATA {{ <http://ex/r> <http://ex/q> \"c{g}\" }}"),
                )
                .expect("update");
                let sealed = applier.seal(working).expect("seal");
                let n = ring.publish(sealed, [PodId::from("http://ex/r")]).number();
                max_seen.store(n, Ordering::SeqCst);
            }
        })
    };

    let reader = {
        let ring = ring.clone();
        let start = start.clone();
        std::thread::spawn(move || {
            // Pin the generation that is current at the barrier; its count is fixed
            // for the pin's lifetime no matter what the writer does.
            start.wait();
            let pin = ring.current();
            let pinned_count = count(pin.snapshot());
            for _ in 0..200 {
                assert_eq!(
                    count(pin.snapshot()),
                    pinned_count,
                    "pinned snapshot must never change under concurrent publishes"
                );
            }
            (pin.number(), pinned_count)
        })
    };

    writer.join().expect("writer panicked");
    let (pin_num, pin_count) = reader.join().expect("reader panicked");

    // The writer ran to completion: 30 generations published.
    assert_eq!(max_seen.load(Ordering::SeqCst), 30);
    assert_eq!(ring.current().number(), 30);
    // The reader's pinned snapshot is internally consistent: count == seed + (its gen).
    assert_eq!(
        pin_count,
        4 + pin_num as usize,
        "pinned count is exactly the seed plus its own generation's inserts"
    );
    // And the current generation reflects every insert.
    assert_eq!(count(ring.current().snapshot()), 4 + 30);
}
