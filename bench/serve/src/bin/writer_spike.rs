// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! RESEARCH SPIKE — Wave A2 sequenced writer + group-commit
//! (research/concurrent-serving.md §6.5).
//!
//! Measures the A2 group-commit writer against the A1 baseline (publish-per-update
//! straight onto the generation ring, no batching):
//!
//!   1. WRITER THROUGHPUT (updates/s) at max_batch 1 / 16 / 256 — closed-loop
//!      drain: a single feeder submits N updates back-to-back via Writer::submit
//!      (sync), so the rate is bounded by commit cost, not arrival. Batch 1 IS the
//!      A2-shaped equivalent of the A1 baseline (one generation per update); 16 and
//!      256 show the group-commit win.
//!   2. READER LATENCY p50/p99 of ring.current() — OPEN-LOOP, coordinated-
//!      omission-safe: a sampler reads on a FIXED schedule (every `read_period`),
//!      and each sample's latency is measured from its INTENDED tick, not from when
//!      the thread actually got to issue it. A reader blocked behind the write path
//!      shows up as schedule slip, not as a hidden gap. Measured twice: with a
//!      concurrent writer hammering, and idle.
//!   3. A1 BASELINE: the same reader-latency measurement while a feeder calls
//!      ring.publish() once per update directly (the pre-A2 path).
//!
//! Honest reporting: if group-commit LOSES to A1 publish-per-update anywhere
//! (e.g. tiny single-update windows where the channel + window add latency), the
//! numbers say so — read them, do not assume the batched path always wins.
//!
//! Run: `cargo run --release --bin writer_spike` in bench/serve.
//!   cargo run --release --bin writer_spike -- --json /tmp/writer.json  # strictly-additive
//! Env: WRITER_UPDATES (default 30000), WRITER_SECS reader window (default 2).
//!
//! [OPUS-4.8] (sq-k5qq) `--json <path>` writes the SAME throughput + latency rows STDOUT
//! prints as a stable, DEPENDENCY-FREE JSON document, mirroring the `format!`-JSON
//! convention of `crates/sparq-mpc/examples/mpc_net_bench.rs::cell_json`. No serde dep is
//! added to the harness (serde_json is a TEST-only dev-dep used to parse the emit back).
//! STDOUT is byte-for-byte unchanged whether or not the flag is present; every number is
//! best-effort, MEASURED on the running host — ADVISORY + NON-CANONICAL (stated in the
//! emitted `note`) — nothing is committed.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sparq_core::Graph;
use sparq_serve::{
    ApplyUpdates, GenerationRing, GraphApplier, PodId, Writer, WriterConfig,
};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn base_graph(n: usize) -> Graph {
    let mut nt = String::with_capacity(n * 40);
    for i in 0..n {
        nt.push_str(&format!("<http://ex/s{i}> <http://ex/p> \"v{i}\" .\n"));
    }
    Graph::load_str(&nt, "ntriples").expect("base graph")
}

/// A small Solid-write-shaped INSERT DATA (one fresh resource triple).
fn update(i: usize) -> String {
    format!("INSERT DATA {{ <http://pod.ex/r{i}> <http://ex/p> \"w{i}\" }}")
}

fn pct(sorted_ns: &[u64], p: f64) -> u64 {
    if sorted_ns.is_empty() {
        return 0;
    }
    let idx = ((sorted_ns.len() as f64 * p) as usize).min(sorted_ns.len() - 1);
    sorted_ns[idx]
}

/// Closed-loop writer throughput: one feeder submits `updates` sync, measuring
/// wall time. Returns (updates/s, final generation count).
fn writer_throughput(updates: usize, max_batch: usize) -> (f64, u64) {
    let ring = Arc::new(GenerationRing::new(base_graph(1_000)));
    let writer = Writer::spawn(
        ring.clone(),
        GraphApplier::new(),
        WriterConfig { window: Duration::from_millis(3), max_batch, ..WriterConfig::default() },
    );
    // Multiple concurrent submitters so the window actually fills (a single sync
    // submitter would serialise one-update-per-window and never batch). We use
    // `max_batch` feeders so the writer can collect up to a full batch per window.
    let n_feeders = max_batch.clamp(1, 64);
    let per = updates / n_feeders;
    let next = Arc::new(AtomicU64::new(0));
    let writer = Arc::new(writer);
    let start = Instant::now();
    let feeders: Vec<_> = (0..n_feeders)
        .map(|_| {
            let writer = writer.clone();
            let next = next.clone();
            std::thread::spawn(move || {
                for _ in 0..per {
                    let i = next.fetch_add(1, Ordering::Relaxed) as usize;
                    let _ = writer.submit(update(i), [PodId::from("pod:w")]);
                }
            })
        })
        .collect();
    for f in feeders {
        f.join().unwrap();
    }
    let elapsed = start.elapsed();
    let total = next.load(Ordering::Relaxed);
    let gens = ring.current().number();
    drop(writer);
    (total as f64 / elapsed.as_secs_f64(), gens)
}

/// Open-loop, coordinated-omission-safe reader latency. `sampler` reads
/// `ring.current()` on a fixed `read_period` schedule for `dur`; each sample's
/// latency is measured from its intended tick. Returns sorted latencies (ns).
fn reader_latencies(
    ring: Arc<GenerationRing<Graph>>,
    read_period: Duration,
    dur: Duration,
) -> Vec<u64> {
    let mut lat = Vec::with_capacity((dur.as_nanos() / read_period.as_nanos().max(1)) as usize + 16);
    let start = Instant::now();
    let mut tick = 0u64;
    loop {
        let intended = start + read_period * (tick as u32);
        let now = Instant::now();
        if now < intended {
            std::thread::sleep(intended - now);
        }
        // Latency from the INTENDED tick: if a prior read slipped us late, that
        // slip is charged here (coordinated-omission-safe).
        let g = ring.current();
        std::hint::black_box(g.number());
        let done = Instant::now();
        lat.push(done.saturating_duration_since(intended).as_nanos() as u64);
        tick += 1;
        if done.saturating_duration_since(start) >= dur {
            break;
        }
    }
    lat.sort_unstable();
    lat
}

/// Run a feeder that drives the A2 writer (or the A1 ring) flat-out for `dur`
/// while the calling thread samples reader latency. `mode` selects the path.
fn reader_under_writer(max_batch: usize, dur: Duration, mode: Mode) -> Vec<u64> {
    let ring = Arc::new(GenerationRing::new(base_graph(1_000)));
    let stop = Arc::new(AtomicBool::new(false));

    let feeder = match mode {
        Mode::A2 => {
            let writer = Arc::new(Writer::spawn(
                ring.clone(),
                GraphApplier::new(),
                WriterConfig { window: Duration::from_millis(3), max_batch, ..WriterConfig::default() },
            ));
            let stop = stop.clone();
            // Bounded in-flight: a few feeder threads each use SYNC submit, so the
            // number of outstanding (un-committed) updates is capped by the feeder
            // count — the writer is never buried under an unbounded backlog it must
            // drain on shutdown, and the load is paced to its commit rate
            // (open-loop with natural backpressure). One full batch can still fill
            // per window because there are at least `max_batch`-many… no — we cap
            // feeders at 64 (clamp below) so windows fill to min(max_batch, 64).
            let n_feeders = max_batch.clamp(1, 64);
            let next = Arc::new(AtomicU64::new(0));
            let handles: Vec<_> = (0..n_feeders)
                .map(|_| {
                    let writer = writer.clone();
                    let stop = stop.clone();
                    let next = next.clone();
                    std::thread::spawn(move || {
                        while !stop.load(Ordering::Relaxed) {
                            let i = next.fetch_add(1, Ordering::Relaxed) as usize;
                            if writer.submit(update(i), [PodId::from("pod:w")]).is_err() {
                                break;
                            }
                        }
                    })
                })
                .collect();
            FeederHandle::A2(writer, handles)
        }
        Mode::A1 => {
            // A1 baseline: publish-per-update straight onto the ring, no batching,
            // no group-commit window — the pre-A2 write path. We fork+apply+seal
            // inline (the same per-update work the A2 writer does, minus batching).
            let ring2 = ring.clone();
            let stop = stop.clone();
            FeederHandle::A1(std::thread::spawn(move || {
                let mut applier = GraphApplier::new();
                let mut i = 0usize;
                while !stop.load(Ordering::Relaxed) {
                    let base = ring2.current();
                    let mut w = applier.fork(base.snapshot()).unwrap();
                    if applier.apply(&mut w, &update(i)).is_ok() {
                        // seal now returns Result (the applier's compaction-fold can fail on a
                        // durable-backed store); this in-memory applier never errs, so a failure
                        // here is a real bug — surface it rather than skip the publish.
                        let s = applier.seal(w).expect("in-memory seal never fails");
                        ring2.publish(s, [PodId::from("pod:w")]);
                    }
                    i += 1;
                }
            }))
        }
    };

    let lat = reader_latencies(ring.clone(), Duration::from_micros(200), dur);
    stop.store(true, Ordering::Relaxed);
    feeder.join();
    lat
}

enum Mode {
    A1,
    A2,
}

enum FeederHandle {
    A1(std::thread::JoinHandle<()>),
    A2(Arc<Writer<String>>, Vec<std::thread::JoinHandle<()>>),
}
impl FeederHandle {
    fn join(self) {
        match self {
            FeederHandle::A1(h) => h.join().unwrap(),
            FeederHandle::A2(w, hs) => {
                for h in hs {
                    h.join().unwrap();
                }
                drop(w);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-k5qq) --json <path> machine-readable results emit
// ---------------------------------------------------------------------------

/// One writer-throughput row (§1): updates/s + generation counts at a `max_batch`.
struct ThroughputRow {
    max_batch: usize,
    updates_per_s: f64,
    generations: u64,
    updates_per_gen: f64,
}

/// One reader-latency row (§2/§3): p50/p99/max in microseconds + sample count.
struct LatencyRow {
    scenario: String,
    p50_us: f64,
    p99_us: f64,
    max_us: f64,
    samples: usize,
}

/// Extracts `--json <path>` from argv, returning argv WITHOUT the flag pair. A bare
/// `--json` is a usage error (exit 2), mirroring `sparq-cli` / `bench_text`'s flag.
fn take_json_flag(args: Vec<String>) -> (Vec<String>, Option<String>) {
    let mut out = Vec::with_capacity(args.len());
    let mut json_path = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--json" {
            match args.get(i + 1) {
                Some(p) => {
                    json_path = Some(p.clone());
                    i += 2;
                    continue;
                }
                None => {
                    eprintln!("`--json` requires a path argument: --json <path>");
                    std::process::exit(2);
                }
            }
        }
        out.push(args[i].clone());
        i += 1;
    }
    (out, json_path)
}

/// Minimal JSON string escaper (the dependency-free emit). Scenario labels are static
/// ASCII, so this covers the realistic input; anything else still yields valid `\uXXXX`.
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    sparq_core::json::escape_into(&mut out, raw);
    out.push('"');
    out
}

/// Serialise the run to stable, dependency-free JSON: the workload params plus the two
/// row sets. Every metric is ADVISORY + NON-CANONICAL (stated in `note`).
fn results_json(
    updates: usize,
    reader_window_secs: u64,
    throughput: &[ThroughputRow],
    latency: &[LatencyRow],
) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"harness\": \"serve-spikes writer_spike\",\n");
    s.push_str(&format!("  \"updates\": {updates},\n"));
    s.push_str(&format!("  \"reader_window_secs\": {reader_window_secs},\n"));
    s.push_str(
        "  \"note\": \"Wave A2 group-commit vs A1 publish-per-update; every updates/s and \
         reader-latency figure is best-effort, MEASURED on the running host — ADVISORY, \
         NON-CANONICAL (this dev box) — do not bake into committed files\",\n",
    );
    s.push_str("  \"throughput\": [\n");
    for (i, r) in throughput.iter().enumerate() {
        let comma = if i + 1 < throughput.len() { "," } else { "" };
        s.push_str(&format!(
            "    {{ \"max_batch\": {}, \"updates_per_s\": {:.1}, \"generations\": {}, \
             \"updates_per_gen\": {:.3} }}{comma}\n",
            r.max_batch, r.updates_per_s, r.generations, r.updates_per_gen,
        ));
    }
    s.push_str("  ],\n");
    s.push_str("  \"reader_latency\": [\n");
    for (i, r) in latency.iter().enumerate() {
        let comma = if i + 1 < latency.len() { "," } else { "" };
        s.push_str(&format!(
            "    {{ \"scenario\": {}, \"p50_us\": {:.3}, \"p99_us\": {:.3}, \
             \"max_us\": {:.3}, \"samples\": {} }}{comma}\n",
            json_str(&r.scenario),
            r.p50_us,
            r.p99_us,
            r.max_us,
            r.samples,
        ));
    }
    s.push_str("  ]\n");
    s.push_str("}\n");
    s
}

fn main() {
    let (_args, json_path) = take_json_flag(std::env::args().collect());
    let updates: usize = std::env::var("WRITER_UPDATES").ok().and_then(|v| v.parse().ok()).unwrap_or(30_000);
    let secs: u64 = std::env::var("WRITER_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    let dur = Duration::from_secs(secs);

    println!("# writer_spike — Wave A2 group-commit vs A1 publish-per-update");
    println!("# updates={updates}, reader window={secs}s, read_period=200µs (open-loop)\n");

    // [OPUS-4.8] (sq-k5qq) Capture the SAME rows STDOUT prints for the optional --json emit.
    let mut throughput_rows: Vec<ThroughputRow> = Vec::new();
    let mut latency_rows: Vec<LatencyRow> = Vec::new();
    // Build a LatencyRow from a scenario label + the sorted latencies (ns), exactly the
    // values the println! below derives — single source of truth.
    let lat_row = |scenario: &str, lat: &[u64]| LatencyRow {
        scenario: scenario.to_string(),
        p50_us: pct(lat, 0.50) as f64 / 1000.0,
        p99_us: pct(lat, 0.99) as f64 / 1000.0,
        max_us: *lat.last().unwrap() as f64 / 1000.0,
        samples: lat.len(),
    };

    println!("## 1. Writer throughput (closed-loop drain)");
    println!("| max_batch | updates/s | generations | updates/gen |");
    println!("|---|---:|---:|---:|");
    for &b in &[1usize, 16, 256] {
        let (ups, gens) = writer_throughput(updates, b);
        let upg = updates as f64 / gens.max(1) as f64;
        println!("| {b} | {ups:.0} | {gens} | {upg:.1} |");
        throughput_rows.push(ThroughputRow {
            max_batch: b,
            updates_per_s: ups,
            generations: gens,
            updates_per_gen: upg,
        });
    }

    println!("\n## 2. Reader latency p50/p99 (open-loop, coordinated-omission-safe)");
    println!("| scenario | p50 (µs) | p99 (µs) | max (µs) | samples |");
    println!("|---|---:|---:|---:|---:|");

    // Idle reader (no writer): the floor.
    {
        let ring = Arc::new(GenerationRing::new(base_graph(1_000)));
        let lat = reader_latencies(ring, Duration::from_micros(200), dur);
        println!(
            "| idle (no writer) | {:.2} | {:.2} | {:.2} | {} |",
            pct(&lat, 0.50) as f64 / 1000.0,
            pct(&lat, 0.99) as f64 / 1000.0,
            *lat.last().unwrap() as f64 / 1000.0,
            lat.len()
        );
        latency_rows.push(lat_row("idle (no writer)", &lat));
    }

    for &b in &[1usize, 16, 256] {
        let lat = reader_under_writer(b, dur, Mode::A2);
        println!(
            "| A2 writer max_batch={b} | {:.2} | {:.2} | {:.2} | {} |",
            pct(&lat, 0.50) as f64 / 1000.0,
            pct(&lat, 0.99) as f64 / 1000.0,
            *lat.last().unwrap() as f64 / 1000.0,
            lat.len()
        );
        latency_rows.push(lat_row(&format!("A2 writer max_batch={b}"), &lat));
    }
    {
        let lat = reader_under_writer(1, dur, Mode::A1);
        println!(
            "| A1 publish-per-update | {:.2} | {:.2} | {:.2} | {} |",
            pct(&lat, 0.50) as f64 / 1000.0,
            pct(&lat, 0.99) as f64 / 1000.0,
            *lat.last().unwrap() as f64 / 1000.0,
            lat.len()
        );
        latency_rows.push(lat_row("A1 publish-per-update", &lat));
    }

    println!("\n# Interpretation: readers never block the writer in EITHER mode (lock-free");
    println!("# current()); the win of group-commit is in WRITER throughput (§1) — more");
    println!("# updates per generation = fewer fork/seal/publish cycles. If batch-1 A2 is");
    println!("# slower than A1, that is the channel + window overhead with no batching gain.");

    // [OPUS-4.8] (sq-k5qq) Strictly-additive JSON emit: only when `--json <path>` was given.
    // STDOUT above (the markdown tables) is the unchanged human/research output.
    if let Some(path) = json_path {
        let doc = results_json(updates, secs, &throughput_rows, &latency_rows);
        if let Err(e) = std::fs::write(&path, doc) {
            eprintln!("error writing --json results to {path}: {e}");
            std::process::exit(1);
        }
        eprintln!(
            "wrote {} throughput + {} latency rows to {path}",
            throughput_rows.len(),
            latency_rows.len()
        );
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-k5qq) --json emit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_flag_extraction() {
        let argv: Vec<String> = ["writer_spike", "--json", "/tmp/o.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (positional, path) = take_json_flag(argv);
        assert_eq!(positional, vec!["writer_spike"]);
        assert_eq!(path.as_deref(), Some("/tmp/o.json"));
        let plain: Vec<String> = ["writer_spike"].iter().map(|s| s.to_string()).collect();
        let (p2, none) = take_json_flag(plain.clone());
        assert_eq!(p2, plain);
        assert!(none.is_none());
    }

    #[test]
    fn json_str_escapes() {
        assert_eq!(json_str("idle (no writer)"), "\"idle (no writer)\"");
        assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    #[test]
    fn results_json_shape_and_keys() {
        let throughput = vec![
            ThroughputRow { max_batch: 1, updates_per_s: 12000.0, generations: 11000, updates_per_gen: 1.09 },
            ThroughputRow { max_batch: 256, updates_per_s: 90000.0, generations: 200, updates_per_gen: 150.0 },
        ];
        let latency = vec![
            LatencyRow { scenario: "idle (no writer)".into(), p50_us: 0.21, p99_us: 0.5, max_us: 3.0, samples: 10000 },
            LatencyRow { scenario: "A2 writer max_batch=16".into(), p50_us: 0.3, p99_us: 1.2, max_us: 9.0, samples: 9800 },
        ];
        let doc = results_json(30_000, 2, &throughput, &latency);
        // The dependency-free emit must round-trip through a REAL serde_json parse.
        let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");
        assert_eq!(v["harness"], "serve-spikes writer_spike");
        assert_eq!(v["updates"], 30_000);
        assert_eq!(v["reader_window_secs"], 2);
        assert!(v["note"].as_str().unwrap().contains("NON-CANONICAL"));
        let t = v["throughput"].as_array().expect("throughput is an array");
        assert_eq!(t.len(), 2);
        assert_eq!(t[0]["max_batch"], 1);
        assert!(t[0]["updates_per_s"].is_number());
        let l = v["reader_latency"].as_array().expect("reader_latency is an array");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0]["scenario"], "idle (no writer)");
        assert!(l[0]["p99_us"].is_number());
        assert_eq!(l[0]["samples"], 10000);

        // Empty row sets must still be valid JSON (no trailing comma).
        let empty = results_json(0, 0, &[], &[]);
        let v2: serde_json::Value = serde_json::from_str(&empty).expect("empty is valid JSON");
        assert!(v2["throughput"].as_array().unwrap().is_empty());
        assert!(v2["reader_latency"].as_array().unwrap().is_empty());
    }
}
