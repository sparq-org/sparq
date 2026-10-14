//! RESEARCH SPIKE — result-cache hit-path ceiling (research/concurrent-serving.md §d.ii).
//!
//! Models the proposed fast path: hash the raw query string → pre-serialized response
//! bytes (`Arc<Vec<u8>>`). Measures, on this machine:
//!
//! * raw SPARQL parse cost (spargebra) for a point query — the cost every NON-cached
//!   request pays before anything else;
//! * single-thread cache-hit ops/s under a Zipfian repeat distribution (the optimistic
//!   in-process ceiling the Mreq/s claim must be calibrated against);
//! * multi-thread ops/s for (a) `RwLock<HashMap>` and (b) a sharded read-mostly map,
//!   showing what lock choice costs at this concurrency;
//! * the DEGENERATE case: all-distinct queries (100% miss) — the pure overhead the
//!   cache machinery adds to requests it cannot help (hash + lookup + insert).
//!
//! No HTTP here on purpose: the loadgen baseline tells us what the HTTP layer costs;
//! this isolates the cache data structure itself.
//!
//! Run: `cargo run --release --bin cache_spike [threads]` in bench/serve.
//!   cargo run --release --bin cache_spike -- [threads] --json /tmp/cache.json  # strictly-additive
//!
//! [OPUS-4.8] (sq-5vm.1) `--json <path>` writes the SAME parse / cache-hit / miss / insert
//! figures STDOUT prints as a stable, DEPENDENCY-FREE JSON document, mirroring writer_spike's
//! emit (sq-k5qq). No serde dep is added to the harness (serde_json is a TEST-only dev-dep that
//! parses the emit back). STDOUT is byte-for-byte unchanged whether or not the flag is present;
//! every number is best-effort, MEASURED on the running host — ADVISORY + NON-CANONICAL (stated
//! in the emitted `note`) — nothing is committed.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::Instant;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const KEYS: usize = 10_000;
const RESPONSE_BYTES: usize = 1024;
const ZIPF_S: f64 = 1.0;

/// query string → pre-serialized response bytes (the cache's value is a shared blob).
type ResponseCache = HashMap<String, Arc<Vec<u8>>>;
/// the sharded read-mostly map: one `RwLock`-guarded `ResponseCache` per shard.
/// [OPUS-4.8] sq-kujq — factored out to satisfy `clippy::type_complexity` under
/// `--all-targets -D warnings` (this standalone spike workspace is not gated by root CI).
type ShardedCache = Arc<Vec<RwLock<ResponseCache>>>;

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-5vm.1) --json <path> machine-readable results emit
// ---------------------------------------------------------------------------

/// One multi-thread `bench_threads` row: the lock-strategy label + its aggregate
/// throughput and per-thread ns/op.
struct ThreadsRow {
    label: String,
    threads: usize,
    total_m_ops_per_s: f64,
    ns_per_op_per_thread: f64,
}

/// All scalar figures this spike prints (the multi-thread rows live in their own Vec).
#[derive(Default)]
struct Results {
    threads: usize,
    parse_us_per_op: f64,
    parse_query_chars: usize,
    hit_1t_m_ops_per_s: f64,
    hit_1t_ns_per_op: f64,
    miss_1t_ns_per_op: f64,
    insert_ns_per_op: f64,
}

/// Extracts `--json <path>` from argv, returning argv WITHOUT the flag pair. A bare
/// `--json` is a usage error (exit 2), mirroring `sparq-cli` / `writer_spike`'s flag.
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

/// Minimal JSON string escaper (the dependency-free emit). Strategy labels are static
/// ASCII, so this covers the realistic input; anything else still yields valid `\uXXXX`.
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    sparq_core::json::escape_into(&mut out, raw);
    out.push('"');
    out
}

/// Serialise the run to stable, dependency-free JSON. Every metric is ADVISORY +
/// NON-CANONICAL (stated in `note`).
fn results_json(r: &Results, multi: &[ThreadsRow]) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"harness\": \"serve-spikes cache_spike\",\n");
    s.push_str(
        "  \"note\": \"result-cache hit-path ceiling (no HTTP); every parse-µs, M-ops/s, ns/op \
         figure is best-effort, MEASURED on the running host — ADVISORY, NON-CANONICAL (this dev \
         box) — do not bake into committed files\",\n",
    );
    s.push_str(&format!("  \"threads\": {},\n", r.threads));
    s.push_str(&format!("  \"parse_us_per_op\": {:.3},\n", r.parse_us_per_op));
    s.push_str(&format!("  \"parse_query_chars\": {},\n", r.parse_query_chars));
    s.push_str(&format!("  \"hit_1t_m_ops_per_s\": {:.3},\n", r.hit_1t_m_ops_per_s));
    s.push_str(&format!("  \"hit_1t_ns_per_op\": {:.1},\n", r.hit_1t_ns_per_op));
    s.push_str(&format!("  \"miss_1t_ns_per_op\": {:.1},\n", r.miss_1t_ns_per_op));
    s.push_str(&format!("  \"insert_ns_per_op\": {:.1},\n", r.insert_ns_per_op));
    s.push_str("  \"multi_thread_hit\": [\n");
    for (i, row) in multi.iter().enumerate() {
        let comma = if i + 1 < multi.len() { "," } else { "" };
        s.push_str(&format!(
            "    {{ \"label\": {}, \"threads\": {}, \"total_m_ops_per_s\": {:.3}, \
             \"ns_per_op_per_thread\": {:.1} }}{comma}\n",
            json_str(&row.label),
            row.threads,
            row.total_m_ops_per_s,
            row.ns_per_op_per_thread,
        ));
    }
    s.push_str("  ]\n");
    s.push_str("}\n");
    s
}

fn main() {
    // Strictly-additive: pull `--json <path>` out before reading the optional thread count,
    // so the positional `[threads]` argument is unaffected when the flag is absent.
    let (args, json_path) = take_json_flag(std::env::args().collect());
    let threads: usize = args.into_iter().nth(1).and_then(|s| s.parse().ok()).unwrap_or(8);
    let mut res = Results { threads, ..Results::default() };
    let mut multi: Vec<ThreadsRow> = Vec::new();

    // --- parse cost calibration -------------------------------------------------
    let q = "PREFIX foaf: <http://xmlns.com/foaf/0.1/> SELECT ?n WHERE { <http://example.org/person/123> foaf:name ?n }";
    let t = Instant::now();
    let iters = 20_000;
    for _ in 0..iters {
        let p = spargebra::SparqlParser::new().parse_query(q).unwrap();
        std::hint::black_box(&p);
    }
    let per = t.elapsed().as_nanos() as f64 / iters as f64;
    res.parse_query_chars = q.len();
    res.parse_us_per_op = per / 1000.0;
    println!("spargebra parse (point query, {} chars): {:.2} us/parse", q.len(), per / 1000.0);

    // --- build the cache ----------------------------------------------------------
    let queries: Vec<String> = (0..KEYS)
        .map(|i| format!("SELECT ?n WHERE {{ <http://example.org/person/{i}> <http://xmlns.com/foaf/0.1/name> ?n }}"))
        .collect();
    let blob: Arc<Vec<u8>> = Arc::new(vec![b'x'; RESPONSE_BYTES]);
    let mut map: ResponseCache = HashMap::with_capacity(KEYS * 2);
    for s in &queries {
        map.insert(s.clone(), blob.clone());
    }
    let cache = Arc::new(RwLock::new(map));

    // Zipfian sampler over KEYS ranks (inverse-CDF on a precomputed table).
    let cdf = zipf_cdf(KEYS, ZIPF_S);

    // --- single-thread hit path -----------------------------------------------------
    let ops = 5_000_000u64;
    let t = Instant::now();
    let mut rng = 0xdeadbeefu64;
    let mut sink = 0usize;
    for _ in 0..ops {
        let k = &queries[zipf_sample(&cdf, &mut rng)];
        let guard = cache.read().unwrap();
        if let Some(v) = guard.get(k) {
            sink = sink.wrapping_add(v.len());
        }
    }
    let el = t.elapsed().as_secs_f64();
    res.hit_1t_m_ops_per_s = ops as f64 / el / 1e6;
    res.hit_1t_ns_per_op = el / ops as f64 * 1e9;
    println!(
        "cache hit, 1 thread (RwLock<HashMap>, SipHash): {:.2} M ops/s ({:.0} ns/op) [{sink}]",
        res.hit_1t_m_ops_per_s,
        res.hit_1t_ns_per_op
    );

    // --- multi-thread hit path: RwLock<HashMap> ----------------------------------
    multi.push(bench_threads("RwLock<HashMap>", threads, &queries, &cdf, {
        let cache = cache.clone();
        move |k| cache.read().unwrap().get(k).map(|v| v.len()).unwrap_or(0)
    }));

    // --- multi-thread hit path: sharded maps (no writer contention modelled) ------
    const SHARDS: usize = 64;
    let mut shards: Vec<ResponseCache> = (0..SHARDS).map(|_| HashMap::new()).collect();
    for s in &queries {
        shards[shard_of(s, SHARDS)].insert(s.clone(), blob.clone());
    }
    let shards: ShardedCache = Arc::new(shards.into_iter().map(RwLock::new).collect());
    multi.push(bench_threads("sharded(64) RwLock", threads, &queries, &cdf, {
        let shards = shards.clone();
        move |k| {
            let g = shards[shard_of(k, SHARDS)].read().unwrap();
            g.get(k).map(|v| v.len()).unwrap_or(0)
        }
    }));

    // --- multi-thread: lock-free reads via an immutable Arc-swapped map ----------
    // Models the "generation cache": readers clone an Arc to the whole map (as the
    // server already does for the graph) and look up with zero locking.
    let frozen: Arc<ResponseCache> = Arc::new(cache.read().unwrap().clone());
    multi.push(bench_threads("Arc-swapped immutable map", threads, &queries, &cdf, {
        let frozen = frozen.clone();
        move |k| frozen.get(k).map(|v| v.len()).unwrap_or(0)
    }));

    // --- degenerate: all-distinct queries, 100% miss ------------------------------
    // The overhead the cache adds to a workload it cannot help: hash + miss lookup
    // (we do NOT model insert-on-miss here; a real cache would also pay an insert,
    // measured separately below).
    let distinct: Vec<String> = (0..1_000_000)
        .map(|i| format!("SELECT ?n WHERE {{ <http://example.org/distinct/{i}> <http://xmlns.com/foaf/0.1/name> ?n }}"))
        .collect();
    let t = Instant::now();
    let mut sink = 0usize;
    for k in &distinct {
        let g = cache.read().unwrap();
        if let Some(v) = g.get(k) {
            sink = sink.wrapping_add(v.len());
        }
    }
    let el = t.elapsed().as_secs_f64();
    res.miss_1t_ns_per_op = el / distinct.len() as f64 * 1e9;
    println!(
        "cache MISS lookup (all-distinct, 1 thread): {:.0} ns/op — pure per-request overhead when the cache cannot help [{sink}]",
        res.miss_1t_ns_per_op
    );

    // Insert-on-miss cost (what an all-distinct workload would also pay, with the
    // eviction policy ignored — i.e. a lower bound).
    let t = Instant::now();
    {
        let mut g = cache.write().unwrap();
        for k in distinct.iter().take(200_000) {
            g.insert(k.clone(), blob.clone());
        }
    }
    res.insert_ns_per_op = t.elapsed().as_secs_f64() / 200_000.0 * 1e9;
    println!(
        "cache insert (lower bound, lock held once): {:.0} ns/op",
        res.insert_ns_per_op
    );

    // [OPUS-4.8] (sq-5vm.1) Strictly-additive JSON emit: only when `--json <path>` was given.
    // STDOUT above (the free-text metric lines) is the unchanged human/research output.
    if let Some(path) = json_path {
        let doc = results_json(&res, &multi);
        if let Err(e) = std::fs::write(&path, doc) {
            eprintln!("error writing --json results to {path}: {e}");
            std::process::exit(1);
        }
        eprintln!("wrote cache_spike results ({} multi-thread rows) to {path}", multi.len());
    }
}

/// Runs the N-thread hit-path benchmark for one lock strategy, PRINTS the same line it
/// always did, and RETURNS the row so the optional `--json` emit shares one source of truth.
fn bench_threads(
    label: &str,
    threads: usize,
    queries: &[String],
    cdf: &Arc<Vec<f64>>,
    f: impl Fn(&str) -> usize + Send + Sync + Clone + 'static,
) -> ThreadsRow {
    let ops_per_thread = 3_000_000u64;
    let queries = Arc::new(queries.to_vec());
    let t = Instant::now();
    let handles: Vec<_> = (0..threads)
        .map(|i| {
            let q = queries.clone();
            let cdf = cdf.clone();
            let f = f.clone();
            std::thread::spawn(move || {
                let mut rng = 0x12345678u64.wrapping_add(i as u64 * 0x9e3779b9);
                let mut sink = 0usize;
                for _ in 0..ops_per_thread {
                    let k = &q[zipf_sample(&cdf, &mut rng)];
                    sink = sink.wrapping_add(f(k));
                }
                sink
            })
        })
        .collect();
    let mut sink = 0usize;
    for h in handles {
        sink = sink.wrapping_add(h.join().unwrap());
    }
    let el = t.elapsed().as_secs_f64();
    let total = ops_per_thread * threads as u64;
    let total_m_ops_per_s = total as f64 / el / 1e6;
    let ns_per_op_per_thread = el / (total as f64 / threads as f64) * 1e9;
    println!(
        "cache hit, {threads} threads ({label}): {:.2} M ops/s total ({:.0} ns/op/thread) [{sink}]",
        total_m_ops_per_s,
        ns_per_op_per_thread
    );
    ThreadsRow {
        label: label.to_string(),
        threads,
        total_m_ops_per_s,
        ns_per_op_per_thread,
    }
}

fn zipf_cdf(n: usize, s: f64) -> Arc<Vec<f64>> {
    let mut weights: Vec<f64> = (1..=n).map(|k| 1.0 / (k as f64).powf(s)).collect();
    let sum: f64 = weights.iter().sum();
    let mut acc = 0.0;
    for w in weights.iter_mut() {
        acc += *w / sum;
        *w = acc;
    }
    Arc::new(weights)
}

fn zipf_sample(cdf: &[f64], rng: &mut u64) -> usize {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    let u = (*rng >> 11) as f64 / (1u64 << 53) as f64;
    cdf.partition_point(|&c| c < u).min(cdf.len() - 1)
}

fn shard_of(s: &str, shards: usize) -> usize {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    (h.finish() as usize) % shards
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-5vm.1) --json emit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_flag_extraction() {
        let argv: Vec<String> = ["cache_spike", "16", "--json", "/tmp/o.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (positional, path) = take_json_flag(argv);
        assert_eq!(positional, vec!["cache_spike", "16"]);
        assert_eq!(path.as_deref(), Some("/tmp/o.json"));
        let plain: Vec<String> = ["cache_spike", "16"].iter().map(|s| s.to_string()).collect();
        let (p2, none) = take_json_flag(plain.clone());
        assert_eq!(p2, plain);
        assert!(none.is_none());
    }

    #[test]
    fn json_str_escapes() {
        assert_eq!(json_str("sharded(64) RwLock"), "\"sharded(64) RwLock\"");
        assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    #[test]
    fn results_json_round_trips() {
        let r = Results {
            threads: 8,
            parse_us_per_op: 1.23,
            parse_query_chars: 110,
            hit_1t_m_ops_per_s: 30.5,
            hit_1t_ns_per_op: 33.0,
            miss_1t_ns_per_op: 45.0,
            insert_ns_per_op: 120.0,
        };
        let multi = vec![
            ThreadsRow {
                label: "RwLock<HashMap>".into(),
                threads: 8,
                total_m_ops_per_s: 80.0,
                ns_per_op_per_thread: 100.0,
            },
            ThreadsRow {
                label: "Arc-swapped immutable map".into(),
                threads: 8,
                total_m_ops_per_s: 200.0,
                ns_per_op_per_thread: 40.0,
            },
        ];
        let doc = results_json(&r, &multi);
        // The dependency-free emit must round-trip through a REAL serde_json parse.
        let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");
        assert_eq!(v["harness"], "serve-spikes cache_spike");
        assert!(v["note"].as_str().unwrap().contains("NON-CANONICAL"));
        assert_eq!(v["threads"], 8);
        assert!(v["parse_us_per_op"].is_number());
        assert!(v["hit_1t_m_ops_per_s"].is_number());
        assert_eq!(v["parse_query_chars"], 110);
        let m = v["multi_thread_hit"].as_array().expect("multi_thread_hit is an array");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0]["label"], "RwLock<HashMap>");
        assert_eq!(m[1]["label"], "Arc-swapped immutable map");
        assert!(m[0]["total_m_ops_per_s"].is_number());
        assert_eq!(m[0]["threads"], 8);

        // Empty multi-thread set must still be valid JSON (no trailing comma).
        let empty = results_json(&Results::default(), &[]);
        let v2: serde_json::Value = serde_json::from_str(&empty).expect("empty is valid JSON");
        assert!(v2["multi_thread_hit"].as_array().unwrap().is_empty());
    }
}
