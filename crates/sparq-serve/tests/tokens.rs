// written while Fable 5 unavailable — re-review when Fable returns.
//! Read-your-writes token tests (Wave A3, adr-horizontal-scaling §4.3 collapsed
//! to one node). `Writer::submit` returns the generation number that contains the
//! update; that number is the session token a client round-trips.
//! `GenerationRing::read_your_writes(token)` pins the current generation iff it
//! already reflects the commit the token names.
//!
//! Two invariants:
//!  1. On the node that acked the write, the token is satisfied the moment
//!     `submit` returns — the publish happens-before the ack (publish-then-ack in
//!     `writer::commit`), so RYW never returns `None` on the acking ring.
//!  2. A lagging ring (a replica that has applied fewer generations) does NOT
//!     satisfy a higher token — it must wait or redirect, never serve a stale read
//!     as if it were fresh.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sparq_serve::{ApplyUpdates, GenerationRing, PodId, Writer, WriterConfig};

/// Snapshot = applied-update log (mirrors tests/writer.rs).
type Log = Vec<String>;

struct LogApplier;

impl ApplyUpdates for LogApplier {
    type Snapshot = Log;
    type Working = Log;
    type Update = String;

    fn fork(&mut self, base: &Log) -> Result<Log, String> {
        Ok(base.clone())
    }
    fn apply(&mut self, working: &mut Log, update: &String) -> Result<(), String> {
        working.push(update.clone());
        Ok(())
    }
    fn seal(&mut self, working: Log) -> Result<Log, String> {
        Ok(working) // (sq-vpx4) no durable side: seal is infallible
    }
}

fn pod(s: &str) -> Vec<PodId> {
    vec![PodId::from(s)]
}

/// Invariant 1: the token returned by `submit` is satisfied on the acking ring
/// the instant `submit` returns — `read_your_writes(token)` is `Some` with no
/// wait, and pins a generation whose number is `>= token`.
#[test]
fn token_is_satisfied_immediately_on_the_acking_ring() {
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        LogApplier,
        WriterConfig {
            window: Duration::from_millis(1),
            max_batch: 256,
            ..WriterConfig::default()
        },
    );

    for expect in 1..=5u64 {
        let token = writer.submit(format!("u{expect}"), pod("p")).unwrap();
        assert_eq!(token, expect, "token is the containing generation number");
        // No wait, no poll: the publish happened-before this ack.
        let g = ring
            .read_your_writes(token)
            .expect("the acking ring always satisfies its own write token");
        assert!(g.number() >= token, "pinned generation reflects the commit");
        assert_eq!(*g.snapshot().last().unwrap(), format!("u{expect}"));
    }
}

/// `satisfies` / `read_your_writes` boundary: a generation satisfies tokens
/// `<= its number` and not strictly-greater ones.
#[test]
fn read_your_writes_boundary() {
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Writer::spawn(
        ring.clone(),
        LogApplier,
        WriterConfig {
            window: Duration::from_millis(1),
            max_batch: 256,
            ..WriterConfig::default()
        },
    );
    let token = writer.submit("only".into(), pod("p")).unwrap();
    assert_eq!(token, 1);

    let g = ring.current();
    assert!(g.satisfies(0), "gen 1 reflects the genesis token 0");
    assert!(g.satisfies(1), "gen 1 reflects its own token");
    assert!(!g.satisfies(2), "gen 1 does NOT reflect a future token");

    assert!(ring.read_your_writes(0).is_some());
    assert!(ring.read_your_writes(1).is_some());
    assert!(
        ring.read_your_writes(2).is_none(),
        "a not-yet-applied token yields None (caller must wait/redirect)"
    );
}

/// Invariant 2: a lagging replica ring does NOT satisfy a token it has not yet
/// applied. We model two rings fed the same update stream, with the replica
/// deliberately held behind, and assert the replica refuses the leader's token
/// until it catches up — then accepts it.
#[test]
fn lagging_replica_refuses_then_accepts_token() {
    let leader = Arc::new(GenerationRing::new(Log::new()));
    let replica = Arc::new(GenerationRing::new(Log::new()));

    let lw = Writer::spawn(
        leader.clone(),
        LogApplier,
        WriterConfig {
            window: Duration::from_millis(1),
            max_batch: 256,
            ..WriterConfig::default()
        },
    );

    // Leader applies three updates; replica is still at generation 0.
    let token = {
        let mut t = 0;
        for i in 0..3 {
            t = lw.submit(format!("u{i}"), pod("p")).unwrap();
        }
        t
    };
    assert_eq!(token, 3);
    assert_eq!(replica.current().number(), 0);

    // The replica cannot satisfy the leader's token yet — read-your-writes must
    // route elsewhere or wait, never serve the stale generation 0.
    assert!(
        replica.read_your_writes(token).is_none(),
        "a replica behind the token must refuse it"
    );

    // Replica catches up (replays the same stream via its own writer).
    let rw = Writer::spawn(
        replica.clone(),
        LogApplier,
        WriterConfig {
            window: Duration::from_millis(1),
            max_batch: 256,
            ..WriterConfig::default()
        },
    );
    for i in 0..3 {
        rw.submit(format!("u{i}"), pod("p")).unwrap();
    }
    assert_eq!(replica.current().number(), 3);

    // Now the replica satisfies the token and can serve the read.
    let g = replica
        .read_your_writes(token)
        .expect("a caught-up replica satisfies the token");
    assert!(g.number() >= token);
}

/// Concurrent readers polling `read_your_writes(token)` against a live writer:
/// once a reader observes `Some`, the pinned generation NEVER regresses below the
/// token even as the writer publishes far past it (monotonic, snapshot-pinned).
#[test]
fn satisfied_token_pin_never_regresses_under_concurrent_writes() {
    let ring = Arc::new(GenerationRing::new(Log::new()));
    let writer = Arc::new(Writer::spawn(
        ring.clone(),
        LogApplier,
        WriterConfig {
            window: Duration::from_millis(1),
            max_batch: 16,
            ..WriterConfig::default()
        },
    ));

    // Establish a token at generation 1.
    let token = writer.submit("anchor".into(), pod("p")).unwrap();
    assert_eq!(token, 1);

    let stop = Arc::new(AtomicBool::new(false));
    let feeder = {
        let writer = writer.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut i = 0u64;
            while !stop.load(Ordering::Relaxed) {
                if writer.submit_detached(format!("x{i}"), pod("p")).is_err() {
                    break;
                }
                i += 1;
            }
        })
    };

    // Reader: every satisfied pin reflects at least the token.
    for _ in 0..50_000 {
        if let Some(g) = ring.read_your_writes(token) {
            assert!(
                g.number() >= token,
                "a satisfied RYW pin never regresses below its token"
            );
        }
    }
    // The feeder ran concurrently throughout the reader spin, but on a contended CPU
    // (e.g. CI under load) the background writer may not have published past the anchor
    // by the time the tight 50k-iteration spin ends — the reader thread can monopolise
    // its core and starve the writer. Wait (bounded) for the advance we know is coming
    // rather than racing it; the monotonicity invariant above is what this test really
    // guards, and it held on every iteration.
    let deadline = Instant::now() + Duration::from_secs(10);
    while ring.current().number() <= 1 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    stop.store(true, Ordering::Relaxed);
    feeder.join().unwrap();
    assert!(
        ring.current().number() > 1,
        "writer advanced past the anchor"
    );
}
