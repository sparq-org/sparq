//! The sequenced writer over the PRODUCTION store: `GraphApplier` applying real
//! SPARQL Updates to `sparq_core::Graph` through the structural fork +
//! delta-overlay (apply) + threshold-compaction (seal) paths. Proves the §6.5
//! batch semantics hold end-to-end on the real snapshot type, including
//! failed-SPARQL isolation and the bounded-pending-delta compaction policy, and
//! carries the `--ignored` fork-cost measurement quoted in `applier.rs`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sparq_core::Graph;
use sparq_serve::{GenerationRing, GraphApplier, PodId, WriteError, Writer, WriterConfig};

fn graph_with(n: usize) -> Graph {
    let mut nt = String::new();
    for i in 0..n {
        nt.push_str(&format!("<http://ex/s{i}> <http://ex/p> \"v{i}\" .\n"));
    }
    Graph::load_str(&nt, "ntriples").expect("load test graph")
}

fn insert(i: usize) -> String {
    format!("INSERT DATA {{ <http://ex/new{i}> <http://ex/p> \"w{i}\" }}")
}

/// A batch of real SPARQL updates → one generation, one folded graph.
#[test]
fn sparql_batch_publishes_one_generation() {
    let ring = Arc::new(GenerationRing::new(graph_with(10)));
    let writer = Writer::spawn(
        ring.clone(),
        GraphApplier::new(),
        WriterConfig {
            window: Duration::from_millis(200),
            max_batch: 64,
            ..WriterConfig::default()
        },
    );

    let before = ring.current(); // pinned across the commit
    for i in 0..3 {
        writer
            .submit_detached(insert(i), [PodId::from("pod:alice")])
            .unwrap();
    }
    let generation = writer
        .submit(
            "DELETE DATA { <http://ex/s0> <http://ex/p> \"v0\" }".to_string(),
            [PodId::from("pod:alice")],
        )
        .unwrap();

    assert_eq!(
        generation, 1,
        "3 inserts + 1 delete in one window = one generation"
    );
    let current = ring.current();
    assert_eq!(current.number(), 1);
    assert_eq!(current.snapshot().len(), 12, "10 + 3 inserts - 1 delete");
    assert_eq!(current.epochs().epoch(&PodId::from("pod:alice")), 1);
    // Snapshot isolation: the pinned pre-commit generation is untouched.
    assert_eq!(before.snapshot().len(), 10);
}

/// A non-SPARQL update fails alone: its submitter gets the parse error, the
/// rest of the batch lands, and a later window works on the published result.
#[test]
fn bad_sparql_is_isolated_from_its_batch() {
    let ring = Arc::new(GenerationRing::new(graph_with(5)));
    let writer = Arc::new(Writer::spawn(
        ring.clone(),
        GraphApplier::new(),
        WriterConfig {
            window: Duration::from_millis(300),
            max_batch: 64,
            ..WriterConfig::default()
        },
    ));

    writer
        .submit_detached(insert(0), [PodId::from("pod:a")])
        .unwrap();
    let bad = {
        let writer = writer.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            writer.submit("THIS IS NOT SPARQL".to_string(), [PodId::from("pod:bad")])
        })
    };
    std::thread::sleep(Duration::from_millis(120));
    let ok = writer.submit(insert(1), [PodId::from("pod:b")]).unwrap();

    match bad.join().unwrap() {
        Err(WriteError::Rejected(e)) => {
            assert!(!e.is_empty(), "parse error is forwarded to the submitter")
        }
        other => panic!("bad SPARQL must be rejected, got {other:?}"),
    }
    assert_eq!(ok, 1, "the surviving updates publish as one generation");
    let current = ring.current();
    assert_eq!(current.number(), 1);
    assert_eq!(current.snapshot().len(), 7, "5 + the 2 good inserts");
    assert_eq!(current.epochs().epoch(&PodId::from("pod:a")), 1);
    assert_eq!(current.epochs().epoch(&PodId::from("pod:b")), 1);
    assert_eq!(current.epochs().epoch(&PodId::from("pod:bad")), 0);

    // Next window builds on the committed state.
    assert_eq!(writer.submit(insert(2), [PodId::from("pod:a")]).unwrap(), 2);
    assert_eq!(ring.current().snapshot().len(), 8);
}

/// Consecutive windows produce consecutive generations over the real store.
#[test]
fn consecutive_windows_consecutive_generations() {
    let ring = Arc::new(GenerationRing::new(graph_with(2)));
    let writer = Writer::spawn(
        ring.clone(),
        GraphApplier::new(),
        WriterConfig {
            window: Duration::from_millis(1),
            max_batch: 256,
            ..WriterConfig::default()
        },
    );

    for expect in 1..=4u64 {
        let g = writer
            .submit(insert(expect as usize), [PodId::from("pod:a")])
            .unwrap();
        assert_eq!(g, expect);
    }
    assert_eq!(ring.current().snapshot().len(), 6);
    assert_eq!(ring.current().epochs().epoch(&PodId::from("pod:a")), 4);
}

/// The fork-cost measurement quoted in `applier.rs` (run manually):
/// `cargo test -p sparq-serve --release fork_cost_measurement -- --ignored --nocapture`
///
/// Measures, per base size: the FIRST fork of a flat graph (one-time dict/cache
/// freeze), the steady-state fork off a frozen lineage, and the full per-commit
/// cycle (fork + 10-triple batch apply + seal) averaged over enough commits to
/// include the amortised threshold compactions.
#[test]
#[ignore = "measurement, not an assertion — run with --ignored --nocapture in release"]
fn fork_cost_measurement() {
    use sparq_serve::ApplyUpdates;
    let threshold: usize = std::env::var("SPARQ_COMPACT_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(sparq_serve::DEFAULT_COMPACT_THRESHOLD);
    for &n in &[100_000usize, 1_000_000] {
        let g = graph_with(n);
        let mut applier = GraphApplier::with_compact_threshold(threshold);

        // First fork of the flat base: the one-time freeze.
        let t = Instant::now();
        let first = applier.fork(&g).unwrap();
        let first_cost = t.elapsed();
        assert_eq!(first.len(), n);
        let frozen = applier.seal(first).unwrap(); // (sq-vpx4) fallible seal; in-memory never Err

        // Steady-state fork off the frozen lineage (no pending delta).
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let t = Instant::now();
            let f = applier.fork(&frozen).unwrap();
            best = best.min(t.elapsed());
            assert_eq!(f.len(), n);
        }
        println!(
            "fork of {n} triples: first (freeze) {first_cost:?}, steady-state {best:?} (best of 5)"
        );

        // Full commit cycle: fork + 10-triple INSERT DATA + seal, chained like the
        // writer (each commit forks the previous sealed generation). 2000 commits
        // x ~20 delta entries comfortably crosses the compaction threshold, so the
        // average includes the amortised O(graph) folds.
        let commits = 2000u32;
        let mut base = frozen;
        let t = Instant::now();
        for c in 0..commits {
            let mut w = applier.fork(&base).unwrap();
            let upd = format!(
                "INSERT DATA {{ {} }}",
                (0..10)
                    .map(|j| format!("<http://ex/c{c}_{j}> <http://ex/q> \"x{c}_{j}\" ."))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            applier.apply(&mut w, &upd).unwrap();
            base = applier.seal(w).unwrap();
        }
        let total = t.elapsed();
        assert_eq!(base.len(), n + commits as usize * 10);
        println!(
            "commit cycle (fork + 10-triple apply + seal) on {n}: {:?}/commit avg over {commits} (total {total:?})",
            total / commits
        );
    }
}

/// The compaction policy keeps the pending delta BOUNDED under sustained updates:
/// with a small threshold, the published generation's delta never exceeds
/// threshold + one batch's growth, and compaction actually runs (the delta resets).
#[test]
fn pending_delta_stays_bounded_under_sustained_updates() {
    use sparq_serve::ApplyUpdates;
    let threshold = 64usize;
    let mut applier = GraphApplier::with_compact_threshold(threshold);
    let mut base = graph_with(200);
    let mut compactions = 0usize;
    let mut max_pending = 0usize;
    for c in 0..200 {
        let mut w = applier.fork(&base).unwrap();
        let upd = format!(
            "INSERT DATA {{ <http://ex/b{c}> <http://ex/q> \"y{c}\" . \
             <http://ex/b{c}> <http://ex/r> <http://ex/t{c}> }}"
        );
        applier.apply(&mut w, &upd).unwrap();
        let sealed = applier.seal(w).unwrap();
        let pending = sealed.pending_delta_len();
        max_pending = max_pending.max(pending);
        if pending == 0 && c > 0 {
            compactions += 1;
        }
        base = sealed;
    }
    // Each commit adds 2 overlay triples + up to 3 new terms = ≤5 delta entries.
    assert!(
        max_pending < threshold + 5,
        "pending delta must stay bounded by threshold + one batch (saw {max_pending})"
    );
    assert!(
        compactions >= 5,
        "compaction must keep firing (saw {compactions})"
    );
    assert_eq!(base.len(), 200 + 200 * 2);
    // And the final generation still answers correctly after many fold cycles.
    assert_eq!(
        sparq_engine::count(&base, "SELECT * WHERE { ?s <http://ex/r> ?o }").unwrap(),
        200
    );
}
