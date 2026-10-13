//! [OPUS-4.8] sq-7d3dj.5 (epic sq-7d3dj) — CANONICAL loopback HTTP-throughput harness for
//! `sparq-server`.
//!
//! 🤖 SPARQ agent. Roadmap item 5 of `research/optimization-audit-2026-07.md`: the HTTP
//! surface had NO throughput metric anywhere in `bench/`, so every HTTP-lane optimisation
//! (sq-7d3dj.10 / .12 / .13) was unmeasurable. This is the PREREQUISITE that unblocks that
//! lane. It stands up the REAL in-process server (`sparq_server::serve` + `router` +
//! `AppState` — the exact stack the `sparq` binary runs) on an ephemeral `127.0.0.1:0`
//! loopback port over a small fixed synthetic corpus, drives concurrent SPARQL SELECT/ASK
//! queries against it with a dependency-free keep-alive HTTP/1.1 client, and reports
//! {req/s, p50/p99 latency (ms), peak RSS, p50/p99 TTFB (time-to-first-byte) ms}.
//!
//! It changes NO server behaviour — it is measurement infrastructure only (a detached
//! cargo project outside the root workspace, like `bench/serve` and `bench/memtier`).
//!
//! # Honesty — NON-CANONICAL on a shared box
//!
//! req/s and latency are WALL-CLOCK sensitive: on this work box (or any busy host) the
//! absolute numbers are directional brackets ONLY, never a claim. The CANONICAL numbers
//! come from a quiet EC2 runner — the same disposition the competitor-gather and the
//! perf-gate give every wall-clock metric. Nothing here is wired into the deterministic
//! perf-gate (`scripts/perf-gate.py`): req/s is a TREND / EC2 metric, not a gated ratchet.
//! Every report carries the NON-CANONICAL note: as a header comment line (`# {NOTE}`) on
//! the text summary, and as a top-level `"note"` field (plus `"canonical": false`) in the
//! `--json` document.
//!
//! # What it measures
//!
//! * throughput series — a fixed small SELECT (bound-subject point query) and an ASK, at
//!   concurrency 1/8/32 (the server's default `max_concurrent` is 32), closed-loop:
//!   req/s + p50/p99 ms.
//! * peak RSS — the process `VmHWM` high-water mark reached while serving one LARGE SELECT
//!   (a full-graph scan that materialises the whole result), capturing the whole-result
//!   materialisation cost the streaming beads (sq-7d3dj.12 / .13) target.
//!
//! Run: `cargo run --release --bin serve_throughput` in `bench/serve-throughput`, or via
//! `scripts/serve-throughput-bench.sh`. `--json <path>` writes a strictly-additive,
//! dependency-free results document (STDOUT is byte-for-byte unchanged with or without it).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use sparq_core::Graph;
use sparq_server::{router, serve, AppState};

// --------------------------------------------------------------------------- //
// Configuration
// --------------------------------------------------------------------------- //

/// Parsed CLI configuration. Defaults keep a plain `cargo run --release` a ~10s run;
/// `--smoke` shrinks everything so the gate can exercise the whole path in a second or two.
struct Config {
    /// Target total corpus triples (rounded to a whole number of subjects).
    triples: usize,
    /// Concurrency levels for the throughput series (client connections, closed-loop).
    conns: Vec<usize>,
    /// Closed-loop measurement window per (query, concurrency) cell.
    duration: Duration,
    /// Warm-up window before the first measured cell (JIT/page-in/connection-establish).
    warmup: Duration,
    /// Optional strictly-additive machine-readable results path.
    json: Option<String>,
    /// A label echoed into the summary + JSON (e.g. the host or a run tag).
    label: String,
}

impl Config {
    fn parse() -> Self {
        let mut cfg = Config {
            triples: 40_000,
            conns: vec![1, 8, 32],
            duration: Duration::from_secs(3),
            warmup: Duration::from_millis(500),
            json: None,
            label: "local".to_string(),
        };
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            let mut next =
                |flag: &str| args.next().unwrap_or_else(|| panic!("{} needs a value", flag));
            match a.as_str() {
                "--triples" => cfg.triples = next("--triples").parse().expect("triples int"),
                "--conns" => {
                    cfg.conns = next("--conns")
                        .split(',')
                        .map(|s| s.trim().parse().expect("conns int"))
                        .collect();
                }
                "--duration" => {
                    cfg.duration =
                        Duration::from_secs_f64(next("--duration").parse().expect("duration secs"));
                }
                "--warmup" => {
                    cfg.warmup =
                        Duration::from_secs_f64(next("--warmup").parse().expect("warmup secs"));
                }
                "--json" => cfg.json = Some(next("--json")),
                "--label" => cfg.label = next("--label"),
                "--smoke" => {
                    // A fast, deterministic path for the CI gate: tiny corpus, low
                    // concurrency, sub-second windows — enough to prove the whole
                    // server + client + measurement path runs, not to produce a number.
                    cfg.triples = 4_000;
                    cfg.conns = vec![1, 4];
                    cfg.duration = Duration::from_millis(300);
                    cfg.warmup = Duration::from_millis(100);
                }
                "--help" | "-h" => {
                    eprintln!(
                        "serve_throughput — canonical loopback HTTP throughput harness (NON-CANONICAL on a shared box)\n\
                         \n\
                         --triples N      target corpus triples (default 40000)\n\
                         --conns a,b,c    concurrency levels (default 1,8,32)\n\
                         --duration S     seconds per measured cell (default 3)\n\
                         --warmup S       warm-up seconds before measuring (default 0.5)\n\
                         --json PATH      also write a dependency-free results document\n\
                         --label TAG      run label echoed into the summary + JSON\n\
                         --smoke          fast tiny run for the CI gate"
                    );
                    std::process::exit(0);
                }
                other => panic!("unknown arg {} (try --help)", other),
            }
        }
        cfg
    }
}

// --------------------------------------------------------------------------- //
// A fixed workload — a small SELECT + an ASK (concurrency series) and one large SELECT
// (peak-RSS probe). Bound-subject queries so the point SELECT/ASK stay small + stable.
// --------------------------------------------------------------------------- //

const SUBJECT0: &str = "http://sparq.example/s0";

fn point_select() -> String {
    format!("SELECT ?p ?o WHERE {{ <{}> ?p ?o }}", SUBJECT0)
}
fn point_ask() -> String {
    format!("ASK {{ <{}> ?p ?o }}", SUBJECT0)
}
fn large_select() -> String {
    "SELECT ?s ?p ?o WHERE { ?s ?p ?o }".to_string()
}

/// A small, deterministic synthetic corpus: `entities` subjects, four triples each
/// (`rdf:type`, a name literal, an integer age, a `knows` edge to the next subject). So a
/// bound-subject point query returns exactly four rows and the full scan returns every
/// triple. `triples` is rounded down to a whole number of subjects.
fn build_corpus(triples: usize) -> (Graph, usize) {
    let entities = (triples / 4).max(1);
    let mut nt = String::with_capacity(entities * 200);
    for i in 0..entities {
        let s = format!("http://sparq.example/s{}", i);
        nt.push_str(&format!(
            "<{}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://sparq.example/Person> .\n",
            s
        ));
        nt.push_str(&format!(
            "<{}> <http://xmlns.com/foaf/0.1/name> \"Person {}\" .\n",
            s, i
        ));
        nt.push_str(&format!(
            "<{}> <http://sparq.example/age> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            s,
            20 + (i % 60)
        ));
        nt.push_str(&format!(
            "<{}> <http://sparq.example/knows> <http://sparq.example/s{}> .\n",
            s,
            (i + 1) % entities
        ));
    }
    let graph = Graph::load_str(&nt, "ntriples").expect("load synthetic corpus");
    (graph, entities * 4)
}

// --------------------------------------------------------------------------- //
// In-process loopback server (mirrors crates/sparq-conformance/src/service_loopback.rs):
// a REAL sparq_server::serve endpoint on 127.0.0.1:0, torn down cleanly on drop.
// --------------------------------------------------------------------------- //

struct LoopbackServer {
    addr: SocketAddr,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl LoopbackServer {
    fn serve(graph: Graph) -> Self {
        let (addr_tx, addr_rx) = mpsc::channel::<SocketAddr>();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let handle = std::thread::Builder::new()
            .name("serve-throughput-loopback".into())
            .spawn(move || {
                // The default multi-thread runtime (worker_threads = num CPUs) so the
                // server can actually serve the 8/32 concurrency levels — this is a
                // throughput harness, not the tiny fixture the conformance lane uses.
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("build loopback tokio runtime");
                rt.block_on(async move {
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                        .await
                        .expect("bind 127.0.0.1:0 loopback");
                    let addr = listener.local_addr().expect("listener local_addr");
                    addr_tx.send(addr).expect("report bound loopback addr");
                    // Stock AppState: 30s deadline-only budget, max_concurrent = 32 —
                    // the exact production default the HTTP opt lane will be measured
                    // against. No slow-client deadlines needed on a trusted loopback.
                    let app = router(AppState::new(graph));
                    let shutdown = async move {
                        let _ = shutdown_rx.await;
                    };
                    serve(listener, app, None, None, shutdown)
                        .await
                        .expect("loopback serve loop");
                });
            })
            .expect("spawn loopback server thread");
        // recv_timeout (not recv) so a server thread that panics before binding fails the
        // harness fast + deterministically instead of hanging forever. [OPUS-4.8]
        let addr = addr_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("loopback reported bound address");
        LoopbackServer {
            addr,
            shutdown: Some(shutdown_tx),
            handle: Some(handle),
        }
    }
}

impl Drop for LoopbackServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// --------------------------------------------------------------------------- //
// Dependency-free keep-alive HTTP/1.1 client (adapted from bench/serve/src/bin/loadgen.rs).
// sparq-server always sets Content-Length (even on the streamed JSON path — see
// http.rs::chunked_response), so a Content-Length reader frames every response and keeps
// the keep-alive connection aligned.
// --------------------------------------------------------------------------- //

fn request_bytes(addr: &SocketAddr, query: &str) -> Vec<u8> {
    let mut enc = String::new();
    for b in query.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                enc.push(b as char)
            }
            b' ' => enc.push('+'),
            _ => enc.push_str(&format!("%{:02X}", b)),
        }
    }
    format!(
        "GET /sparql?query={} HTTP/1.1\r\nHost: {}\r\nAccept: application/sparql-results+json\r\nConnection: keep-alive\r\n\r\n",
        enc, addr
    )
    .into_bytes()
}

/// Writes one request and reads exactly one response (Content-Length framed). Returns the
/// HTTP status and time-to-first-byte in microseconds, or `None` on a socket error (the
/// caller reconnects). TTFB is measured from request write to first read from socket.
fn do_request(conn: &mut TcpStream, req: &[u8]) -> Option<(u16, u64)> {
    conn.write_all(req).ok()?;
    let ttfb_start = Instant::now();
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 16384];
    let mut ttfb_us = 0u64;
    let header_end = loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        let n = conn.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        // Record TTFB on first successful read [OPUS-4.8]
        if ttfb_us == 0 {
            ttfb_us = ttfb_start.elapsed().as_micros() as u64;
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let headers = std::str::from_utf8(&buf[..header_end]).ok()?;
    let status: u16 = headers.split_whitespace().nth(1)?.parse().ok()?;
    let clen: usize = headers
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())?
        })
        .unwrap_or(0);
    let mut have = buf.len() - header_end;
    while have < clen {
        let n = conn.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        have += n;
    }
    Some((status, ttfb_us))
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

// --------------------------------------------------------------------------- //
// Closed-loop measurement
// --------------------------------------------------------------------------- //

/// One measured cell: {req/s, p50/p99 ms, p50/p99 TTFB ms} over `conns` closed-loop connections.
struct Sample {
    query: String,
    conns: usize,
    total: u64,
    errors: u64,
    non200: u64,
    elapsed_s: f64,
    reqs_per_s: f64,
    p50_ms: f64,
    p99_ms: f64,
    p50_ttfb_ms: f64,
    p99_ttfb_ms: f64,
}

/// Drive `query` closed-loop from `conns` keep-alive connections for `duration`, collecting
/// per-request latencies and TTFB. Closed-loop: each connection fires back-to-back, so this
/// measures the server's saturation throughput at that concurrency (latency is interpreted
/// relative to the level — the classic closed-loop caveat).
fn run_cell(addr: SocketAddr, query: &str, conns: usize, duration: Duration) -> Sample {
    let req = Arc::new(request_bytes(&addr, query));
    // Anchor the deadline AND the elapsed timer to one `start` instant so req/s and latency
    // aren't skewed by requests completed during worker spawn-up. [OPUS-4.8]
    let start = Instant::now();
    let deadline = start + duration;
    let errors = Arc::new(AtomicU64::new(0));
    let non200 = Arc::new(AtomicU64::new(0));

    let workers: Vec<_> = (0..conns)
        .map(|_| {
            let req = req.clone();
            let errors = errors.clone();
            let non200 = non200.clone();
            std::thread::spawn(move || {
                let mut lat: Vec<u64> = Vec::with_capacity(1 << 16);
                let mut ttfb: Vec<u64> = Vec::with_capacity(1 << 16);
                let mut conn = TcpStream::connect(addr).expect("client connect");
                conn.set_nodelay(true).ok();
                while Instant::now() < deadline {
                    let t = Instant::now();
                    match do_request(&mut conn, &req) {
                        Some((status, ttfb_us)) => {
                            lat.push(t.elapsed().as_micros() as u64);
                            ttfb.push(ttfb_us);
                            if status != 200 {
                                non200.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        None => {
                            errors.fetch_add(1, Ordering::Relaxed);
                            match TcpStream::connect(addr) {
                                Ok(c) => {
                                    conn = c;
                                    conn.set_nodelay(true).ok();
                                }
                                Err(_) => break,
                            }
                        }
                    }
                }
                (lat, ttfb)
            })
        })
        .collect();

    let mut all: Vec<u64> = Vec::new();
    let mut all_ttfb: Vec<u64> = Vec::new();
    for w in workers {
        let (lat, ttfb) = w.join().expect("client worker join");
        all.extend(lat);
        all_ttfb.extend(ttfb);
    }
    let elapsed_s = start.elapsed().as_secs_f64().max(1e-9);
    all.sort_unstable();
    all_ttfb.sort_unstable();
    Sample {
        query: query.to_string(),
        conns,
        total: all.len() as u64,
        errors: errors.load(Ordering::Relaxed),
        non200: non200.load(Ordering::Relaxed),
        elapsed_s,
        reqs_per_s: all.len() as f64 / elapsed_s,
        p50_ms: percentile_ms(&all, 0.50),
        p99_ms: percentile_ms(&all, 0.99),
        p50_ttfb_ms: percentile_ms(&all_ttfb, 0.50),
        p99_ttfb_ms: percentile_ms(&all_ttfb, 0.99),
    }
}

/// Percentile of a sorted µs latency slice, in milliseconds, using the standard
/// NEAREST-RANK definition: the value at 1-based ordinal rank `⌈p·N⌉` (clamped to
/// `[1, N]`), i.e. index `⌈p·N⌉ − 1`. So `p=0` is the min, `p=1` the max, and for a
/// 4-sample slice `p=0.5` is the 2nd sample (not the 3rd, as a `⌊p·N⌋` index would give).
/// Empty slice => 0.0. [OPUS-4.8]
fn percentile_ms(sorted_us: &[u64], p: f64) -> f64 {
    if sorted_us.is_empty() {
        return 0.0;
    }
    let n = sorted_us.len();
    let rank = (p * n as f64).ceil() as usize; // 1-based ordinal rank ⌈p·N⌉
    let idx = rank.max(1).min(n) - 1; // clamp to [1, N], then to a 0-based index
    sorted_us[idx] as f64 / 1000.0
}

/// Peak resident set size (VmHWM high-water mark) in bytes, from /proc/self/status.
/// Returns 0 if unavailable (non-Linux / no procfs). Mirrors bench/parse's helper.
fn peak_rss_bytes() -> u64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            if let Some(kb) = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok())
            {
                return kb * 1024;
            }
        }
    }
    0
}

// --------------------------------------------------------------------------- //
// Reporting
// --------------------------------------------------------------------------- //

const NOTE: &str = "NON-CANONICAL on a shared box: req/s + latency are wall-clock sensitive \
directional brackets only. Canonical numbers come from a quiet EC2 runner; NOT wired into \
the deterministic perf-gate (req/s is a trend/EC2 metric).";

fn print_summary(cfg: &Config, corpus_triples: usize, samples: &[Sample], peak_rss: u64) {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    println!("# sparq-server loopback HTTP throughput  [label={}]", cfg.label);
    println!("# {}", NOTE);
    println!(
        "# corpus={} triples  server max_concurrent=32 (default)  host_cpus={}",
        corpus_triples, cpus
    );
    println!("# {:<40} {:>5} {:>12} {:>10} {:>10} {:>7} {:>7} {:>10} {:>10}", "query", "conns", "req/s", "p50_ms", "p99_ms", "errs", "non200", "p50_ttfb", "p99_ttfb");
    for s in samples {
        let q = if s.query.len() > 40 {
            format!("{}…", &s.query[..39])
        } else {
            s.query.clone()
        };
        println!(
            "  {:<40} {:>5} {:>12.0} {:>10.3} {:>10.3} {:>7} {:>7} {:>10.3} {:>10.3}",
            q, s.conns, s.reqs_per_s, s.p50_ms, s.p99_ms, s.errors, s.non200, s.p50_ttfb_ms, s.p99_ttfb_ms
        );
    }
    println!(
        "# peak RSS while serving one large SELECT (full scan): {:.1} MiB",
        peak_rss as f64 / (1024.0 * 1024.0)
    );
}

/// Strictly-additive, dependency-free JSON emit (mirrors bench/serve's `--json` spikes — no
/// serde in the harness). STDOUT is unchanged whether or not `--json` is passed.
fn write_json(
    path: &str,
    cfg: &Config,
    corpus_triples: usize,
    samples: &[Sample],
    peak_rss: u64,
) {
    let mut s = String::with_capacity(1024);
    s.push_str("{\n");
    s.push_str(&format!("  \"label\": {},\n", json_str(&cfg.label)));
    s.push_str(&format!("  \"note\": {},\n", json_str(NOTE)));
    s.push_str("  \"canonical\": false,\n");
    s.push_str(&format!("  \"corpus_triples\": {},\n", corpus_triples));
    s.push_str("  \"server_max_concurrent\": 32,\n");
    s.push_str(&format!("  \"peak_rss_bytes\": {},\n", peak_rss));
    s.push_str("  \"samples\": [\n");
    for (i, sm) in samples.iter().enumerate() {
        s.push_str("    {\n");
        s.push_str(&format!("      \"query\": {},\n", json_str(&sm.query)));
        s.push_str(&format!("      \"conns\": {},\n", sm.conns));
        s.push_str(&format!("      \"total\": {},\n", sm.total));
        s.push_str(&format!("      \"errors\": {},\n", sm.errors));
        s.push_str(&format!("      \"non200\": {},\n", sm.non200));
        s.push_str(&format!("      \"elapsed_s\": {:.6},\n", sm.elapsed_s));
        s.push_str(&format!("      \"reqs_per_s\": {:.3},\n", sm.reqs_per_s));
        s.push_str(&format!("      \"p50_ms\": {:.6},\n", sm.p50_ms));
        s.push_str(&format!("      \"p99_ms\": {:.6},\n", sm.p99_ms));
        s.push_str(&format!("      \"p50_ttfb_ms\": {:.6},\n", sm.p50_ttfb_ms));
        s.push_str(&format!("      \"p99_ttfb_ms\": {:.6}\n", sm.p99_ttfb_ms));
        s.push_str(if i + 1 == samples.len() { "    }\n" } else { "    },\n" });
    }
    s.push_str("  ]\n}\n");
    std::fs::write(path, s).unwrap_or_else(|e| panic!("write --json {}: {}", path, e));
    // stderr, not stdout, so the promised "STDOUT byte-for-byte unchanged" holds. [OPUS-4.8]
    eprintln!("# wrote {}", path);
}

/// Minimal JSON string escaping (double-quote, backslash, control chars).
fn json_str(v: &str) -> String {
    let mut out = String::with_capacity(v.len() + 2);
    out.push('"');
    sparq_core::json::escape_into(&mut out, v);
    out.push('"');
    out
}

// --------------------------------------------------------------------------- //

fn main() {
    let cfg = Config::parse();
    let (graph, corpus_triples) = build_corpus(cfg.triples);
    let server = LoopbackServer::serve(graph);
    let addr = server.addr;

    // Warm up: page-in, connection establish, first-query plan/JIT. Not measured.
    if !cfg.warmup.is_zero() {
        let _ = run_cell(addr, &point_select(), 1, cfg.warmup);
    }

    // Throughput series: small SELECT + ASK across the concurrency levels.
    let mut samples = Vec::new();
    for query in [point_select(), point_ask()] {
        for &c in &cfg.conns {
            samples.push(run_cell(addr, &query, c, cfg.duration));
        }
    }

    // Peak-RSS probe LAST: serve one large full-scan SELECT so the VmHWM high-water mark
    // captures the whole-result materialisation cost. A modest concurrency is enough to
    // force the biggest allocation; VmHWM only rises, so we read it after this phase.
    let rss_conns = *cfg.conns.iter().max().unwrap_or(&8).min(&8);
    let large = run_cell(addr, &large_select(), rss_conns, cfg.duration);
    samples.push(large);
    let peak_rss = peak_rss_bytes();

    print_summary(&cfg, corpus_triples, &samples, peak_rss);
    if let Some(path) = &cfg.json {
        write_json(path, &cfg, corpus_triples, &samples, peak_rss);
    }
    // `server` drops here → graceful shutdown + thread join (port released).
}

// --------------------------------------------------------------------------- //
// Tests — the load-bearing invariant is that the WHOLE real path runs and answers 200s.
// --------------------------------------------------------------------------- //

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_shape_is_four_triples_per_subject() {
        let (_g, n) = build_corpus(4_000);
        assert_eq!(n % 4, 0);
        assert!(n >= 4);
    }

    #[test]
    fn percentile_bounds() {
        assert_eq!(percentile_ms(&[], 0.5), 0.0);
        let xs = vec![1_000u64, 2_000, 3_000, 4_000]; // µs
        assert!((percentile_ms(&xs, 0.0) - 1.0).abs() < 1e-9);
        assert!(percentile_ms(&xs, 0.99) <= 4.0);
    }

    #[test]
    fn smoke_real_loopback_answers_200() {
        // The REAL path: build a tiny corpus, stand up sparq_server::serve on loopback,
        // and drive the small SELECT + ASK closed-loop. The invariant this pins is
        // answer-safety: every framed response is HTTP 200 with zero io-errors, so the
        // harness is measuring the real server, not a broken/mock seam.
        let (graph, _n) = build_corpus(2_000);
        let server = LoopbackServer::serve(graph);
        let addr = server.addr;
        for q in [point_select(), point_ask(), large_select()] {
            let s = run_cell(addr, &q, 2, Duration::from_millis(200));
            assert!(s.total > 0, "no requests completed for {}", q);
            assert_eq!(s.errors, 0, "io errors for {}", q);
            assert_eq!(s.non200, 0, "non-200 responses for {}", q);
            assert!(s.reqs_per_s > 0.0);
        }
    }
}
