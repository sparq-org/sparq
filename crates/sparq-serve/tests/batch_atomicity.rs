// written while Fable 5 unavailable — re-review when Fable returns.
//! Batch atomicity (§6.5): a reader sees EXACTLY 0-or-all rows of a bulk update,
//! never a partial prefix. Mirrors sparq-server's
//! `readers_are_not_blocked_during_a_slow_update` but asserts the stronger
//! generation-ring guarantee — atomicity, not just non-blocking: a 50k-triple
//! update lands as one generation, and a concurrent sampler reading
//! `ring.current()` only ever observes the base size or the fully-applied size.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sparq_core::Graph;
use sparq_serve::{GenerationRing, GraphApplier, PodId, Writer, WriterConfig};

const BASE: usize = 1_000;
const BULK: usize = 50_000;

fn graph_with(n: usize) -> Graph {
    let mut nt = String::with_capacity(n * 40);
    for i in 0..n {
        nt.push_str(&format!("<http://ex/s{i}> <http://ex/p> \"v{i}\" .\n"));
    }
    Graph::load_str(&nt, "ntriples").expect("load base graph")
}

/// One INSERT DATA of `BULK` fresh triples — a single bulk update.
fn bulk_insert() -> String {
    let mut s = String::with_capacity(BULK * 50);
    s.push_str("INSERT DATA {");
    for i in 0..BULK {
        s.push_str(&format!(" <http://ex/new{i}> <http://ex/p> \"w{i}\" ."));
    }
    s.push('}');
    s
}

/// A concurrent sampler hammering `ring.current().snapshot().len()` while a 50k
/// bulk update commits sees ONLY the base size or the fully-applied size — never
/// an in-between (a torn / partially-applied generation would show an
/// intermediate count), and never blocks (every sample completes promptly).
#[test]
fn reader_sees_zero_or_all_of_a_bulk_update() {
    let ring = Arc::new(GenerationRing::new(graph_with(BASE)));
    let writer = Arc::new(Writer::spawn(
        ring.clone(),
        GraphApplier::new(),
        WriterConfig {
            window: Duration::from_millis(1),
            max_batch: 256,
            ..WriterConfig::default()
        },
    ));

    let stop = Arc::new(AtomicBool::new(false));
    let torn = Arc::new(AtomicUsize::new(0)); // count of intermediate observations
    let samples = Arc::new(AtomicUsize::new(0));
    let max_sample_ns = Arc::new(AtomicUsize::new(0));

    // Sampler thread: only BASE or BASE+BULK are legal sizes.
    let sampler = {
        let ring = ring.clone();
        let stop = stop.clone();
        let torn = torn.clone();
        let samples = samples.clone();
        let max_sample_ns = max_sample_ns.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let t = Instant::now();
                let len = ring.current().snapshot().len();
                let dt = t.elapsed().as_nanos() as usize;
                max_sample_ns.fetch_max(dt, Ordering::Relaxed);
                samples.fetch_add(1, Ordering::Relaxed);
                if len != BASE && len != BASE + BULK {
                    torn.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
    };

    // Let the sampler warm up on the base, then fire the bulk update.
    std::thread::sleep(Duration::from_millis(20));
    let before = ring.current();
    assert_eq!(before.snapshot().len(), BASE);

    let gen = writer
        .submit(bulk_insert(), [PodId::from("pod:bulk")])
        .unwrap();
    assert_eq!(gen, 1, "the bulk update is one generation");
    assert_eq!(
        ring.current().snapshot().len(),
        BASE + BULK,
        "all 50k applied atomically"
    );

    // Keep sampling briefly after the commit so the sampler observes the new gen.
    std::thread::sleep(Duration::from_millis(20));
    stop.store(true, Ordering::Relaxed);
    sampler.join().unwrap();

    assert_eq!(
        torn.load(Ordering::Relaxed),
        0,
        "a reader must NEVER observe a partial bulk update (atomicity)"
    );
    assert!(samples.load(Ordering::Relaxed) > 1_000, "sampler ran hot");
    // The pinned pre-commit generation is unchanged (snapshot isolation).
    assert_eq!(before.snapshot().len(), BASE);
    // Non-blocking: no single `current()` read stalled behind the 50k apply.
    let worst = max_sample_ns.load(Ordering::Relaxed);
    assert!(
        worst < 50_000_000,
        "a read blocked on the write path (worst sample {worst} ns)"
    );
}

/// Retention / pinning under sustained writes: a reader that pins a generation
/// keeps seeing exactly its snapshot while the writer commits many bulk-ish
/// generations past it, and the ring's retention bound K forgets only its own
/// references (the pin stays alive). Mirrors the §6.4 invariant under A2 commits.
#[test]
fn pinned_generation_survives_sustained_writes() {
    let ring = Arc::new(GenerationRing::new(graph_with(BASE)));
    let writer = Writer::spawn(
        ring.clone(),
        GraphApplier::new(),
        WriterConfig {
            window: Duration::from_millis(1),
            max_batch: 64,
            ..WriterConfig::default()
        },
    );

    let pinned = ring.current(); // generation 0, BASE triples
    assert_eq!(pinned.number(), 0);

    // Sustained writes: 40 generations, each a modest INSERT DATA.
    for i in 0..40 {
        let upd = format!("INSERT DATA {{ <http://ex/x{i}> <http://ex/q> \"y{i}\" }}");
        writer.submit(upd, [PodId::from("pod:w")]).unwrap();
    }
    let current = ring.current();
    assert_eq!(current.number(), 40);
    assert_eq!(current.snapshot().len(), BASE + 40);

    // The pin still answers at its own state despite the ring forgetting it
    // (retention K << 40): the reader's Arc alone keeps generation 0 alive.
    assert_eq!(pinned.snapshot().len(), BASE, "pinned generation unchanged");
    assert!(
        ring.oldest_retained() > 0,
        "ring retention moved past generation 0 under sustained writes"
    );
    // Generation 0 lives only through the pin now — not the ring's references.
    assert_eq!(pinned.number(), 0);
}
