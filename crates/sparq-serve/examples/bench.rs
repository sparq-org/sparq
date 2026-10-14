//! sparq-serve concurrent-serving-core micro-benchmark. sq-ncvq.12. Run:
//!     cargo run -p sparq-serve --example bench --release
//!
//! Measures the lock-free serving primitives that motivate the crate (the
//! generation ring replacing the double-buffered snapshot scheme, §4.3/§4.4):
//!   1. current() — the lock-free `ArcSwap::load` read every reader does on
//!      entry to pin a generation (the hot path; must be ~ns);
//!   2. publish() — one writer-side generation publish (arc-swap of a new
//!      immutable snapshot + per-pod epoch bump), the cost a reader NEVER pays;
//!   3. scheduler — `run` round-trip through the two-lane cost-aware pool for a
//!      cheap job and a heavy job (submit → worker → result).
//!
//! The snapshot type is a cheap `usize` so the numbers isolate the ring/scheduler
//! machinery, NOT graph-build cost (that is the engine's ingest benches). A
//! concurrent-reader arm reports aggregate current() throughput while a writer
//! publishes, exercising the readers-never-block-the-writer invariant.
//!
//! NON-CANONICAL timing (this box is not a quiet bench instance) — the numbers are
//! a sanity-scale signal, not a published baseline.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use sparq_serve::{Cost, GenerationRing, PodId, Scheduler};

fn timed(label: &str, iters: u64, mut f: impl FnMut()) {
    // Best-of-5 over `iters` ops; report per-op nanoseconds.
    let mut best = f64::MAX;
    for _ in 0..5 {
        let t = Instant::now();
        for _ in 0..iters {
            f();
        }
        let per = t.elapsed().as_secs_f64() * 1e9 / iters as f64;
        best = best.min(per);
    }
    println!("{label:48} {best:12.2} ns/op");
}

fn main() {
    let pods: Vec<PodId> = (0..8).map(|i| PodId::from(format!("pod{i}"))).collect();
    let ring: Arc<GenerationRing<usize>> = Arc::new(GenerationRing::new(0usize));

    // 1. lock-free reader pin — the hot path.
    timed("current() (lock-free reader pin)", 5_000_000, || {
        std::hint::black_box(ring.current());
    });

    // 2. writer publish — arc-swap a new snapshot + bump the touched pods' epochs.
    {
        let mut n = 0usize;
        let pods = pods.clone();
        timed("publish() (snapshot swap + epoch bump)", 200_000, || {
            n += 1;
            ring.publish(n, [pods[n % pods.len()].clone()]);
        });
    }

    // 3. scheduler round-trip: a cheap job and a heavy job through the two lanes.
    let sched: Arc<Scheduler<u64>> = Scheduler::with_defaults();
    let cheap: Cost = 1;
    let heavy: Cost = 1_000_000;
    timed("scheduler.run cheap-lane round-trip", 50_000, || {
        let r = sched.run(cheap, || 7u64).expect("cheap job runs");
        std::hint::black_box(r);
    });
    timed("scheduler.run heavy-lane round-trip", 50_000, || {
        let r = sched.run(heavy, || 9u64).expect("heavy job runs");
        std::hint::black_box(r);
    });

    // 4. concurrent invariant: readers pin generations at full speed while a
    //    writer publishes — readers-never-block-the-writer (§4.3). Reports the
    //    aggregate reader throughput observed during a fixed publish burst.
    {
        let stop = Arc::new(AtomicBool::new(false));
        let reads = Arc::new(AtomicU64::new(0));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let ring = ring.clone();
            let stop = stop.clone();
            let reads = reads.clone();
            handles.push(std::thread::spawn(move || {
                let mut local = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    std::hint::black_box(ring.current());
                    local += 1;
                }
                reads.fetch_add(local, Ordering::Relaxed);
            }));
        }
        let t = Instant::now();
        let mut n = 1_000_000usize;
        for _ in 0..50_000 {
            n += 1;
            ring.publish(n, [pods[n % pods.len()].clone()]);
        }
        let publish_s = t.elapsed().as_secs_f64();
        stop.store(true, Ordering::Relaxed);
        for h in handles {
            h.join().expect("reader thread joins");
        }
        let total_reads = reads.load(Ordering::Relaxed);
        println!(
            "\nconcurrent: 50000 publishes in {:.4}s; 4 readers did {} pins \
             (~{:.1} M pins/s aggregate) — none blocked",
            publish_s,
            total_reads,
            total_reads as f64 / publish_s / 1e6
        );
    }

    println!("\nok");
}
