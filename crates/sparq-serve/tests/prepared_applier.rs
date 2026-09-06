//! [SONNET-5] (sq-artifact-keeper-topk) `PreparedGraphApplier` over the real sequenced
//! writer: proves a parsed-once, bound-per-submission `PreparedUpdate` applies correctly
//! through the SAME structural-fork + delta-overlay + threshold-compaction path
//! `GraphApplier` uses for raw text, and that repeated submissions of the SAME template
//! (only the bound value changing) accumulate correctly across many generations.
//!
//! Only compiled under the opt-in `params` feature — see `applier.rs`'s module docs.
#![cfg(feature = "params")]

use std::sync::Arc;
use std::time::Duration;

use oxrdf::{Literal, Term};
use sparq_core::Graph;
use sparq_engine::PreparedUpdate;
use sparq_serve::{GenerationRing, PodId, PreparedGraphApplier, Writer, WriterConfig};

fn graph_with(n: usize) -> Graph {
    let mut ttl = String::from("@prefix ak: <http://example.org/ak#> .\n");
    for i in 0..n {
        ttl.push_str(&format!(
            "<urn:task:{i}> ak:status \"pending\" ; ak:seq {i} .\n"
        ));
    }
    Graph::load_str(&ttl, "turtle").expect("load test graph")
}

fn claim_template() -> PreparedUpdate {
    PreparedUpdate::parse(
        "PREFIX ak: <http://example.org/ak#>
DELETE { ?t ak:status \"pending\" }
INSERT { ?t ak:status \"in_progress\" ; ak:claimedBy ?worker }
WHERE {
  SELECT ?t WHERE { ?t ak:status \"pending\" ; ak:seq ?s }
  ORDER BY ASC(?s) LIMIT 1
}",
    )
    .expect("parse claim template")
}

/// A single bound-and-submitted `PreparedUpdate` applies exactly like the
/// equivalent raw-text update would — one generation, correct delta.
#[test]
fn prepared_update_applies_through_the_real_writer() {
    let ring = Arc::new(GenerationRing::new(graph_with(10)));
    let writer = Writer::spawn(
        ring.clone(),
        PreparedGraphApplier::new(),
        WriterConfig {
            window: Duration::from_millis(200),
            max_batch: 64,
            ..WriterConfig::default()
        },
    );

    let template = claim_template();
    let bound = template
        .bind("worker", Term::Literal(Literal::new_simple_literal("worker-0")))
        .expect("bind worker");

    let generation = writer.submit(bound, [PodId::from("pod:alice")]).unwrap();
    assert_eq!(generation, 1);

    let current = ring.current();
    let graph = current.snapshot();
    let r = sparq_engine::query(
        graph,
        "PREFIX ak: <http://example.org/ak#> SELECT ?t ?w WHERE { ?t ak:claimedBy ?w }",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1, "exactly one task claimed");
    match r.rows[0][0].as_ref().unwrap() {
        oxrdf::Term::NamedNode(n) => assert_eq!(n.as_str(), "urn:task:0", "lowest seq claimed first"),
        other => panic!("expected an IRI, got {other:?}"),
    }
    match r.rows[0][1].as_ref().unwrap() {
        oxrdf::Term::Literal(l) => assert_eq!(l.value(), "worker-0"),
        other => panic!("expected a literal, got {other:?}"),
    }
}

/// Repeated submissions of the SAME parsed-once template, each bound to a
/// distinct value, drain the whole queue correctly across many generations —
/// the shape `PreparedGraphApplier` exists to speed up (a job-queue claim
/// loop), verified for correctness here (the speed claim is measured
/// separately, not asserted in a test).
#[test]
fn repeated_prepared_submissions_drain_the_queue() {
    let n = 50;
    let ring = Arc::new(GenerationRing::new(graph_with(n)));
    let writer = Writer::spawn(
        ring.clone(),
        PreparedGraphApplier::new(),
        WriterConfig {
            window: Duration::from_millis(50),
            adaptive_commit: true,
            ..WriterConfig::default()
        },
    );

    let template = claim_template();
    for i in 0..n {
        let bound = template
            .bind("worker", Term::Literal(Literal::new_simple_literal(format!("worker-{i}"))))
            .expect("bind worker");
        writer.submit(bound, [PodId::from("pod:alice")]).unwrap();
    }

    let current = ring.current();
    let graph = current.snapshot();
    let remaining = sparq_engine::query(
        graph,
        "PREFIX ak: <http://example.org/ak#> SELECT ?t WHERE { ?t ak:status \"pending\" }",
    )
    .unwrap();
    assert_eq!(remaining.rows.len(), 0, "every task claimed exactly once");

    let claimed = sparq_engine::query(
        graph,
        "PREFIX ak: <http://example.org/ak#> SELECT ?t WHERE { ?t ak:status \"in_progress\" }",
    )
    .unwrap();
    assert_eq!(claimed.rows.len(), n, "no duplicate/lost claims");
}
