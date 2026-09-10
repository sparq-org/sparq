//! Benchmark + differential harness: runs the same dataset and queries through
//! `sparq` and through Oxigraph (a mature Rust SPARQL engine), checks that they
//! return the same number of solutions (a correctness cross-check against an
//! independent implementation), and reports load/query timings and memory.
//!
//! Usage:
//!   sparq-bench [--scale N] [--iters K]      # compare both engines (default)
//!   sparq-bench --mem sparq|oxigraph --scale N
//!       # internal: build one engine, print peak RSS (used by the orchestrator)

// [OPUS-4.8] MS-G2 (sq-8wbn): make `// SAFETY:` mandatory on every first-party unsafe block.
#![warn(clippy::undocumented_unsafe_blocks)]

use std::process::Command;
use std::time::{Duration, Instant};

mod dataset;
mod fuzz;
// [OPUS-5] sq-qcnn.5: the oxrdf → sparq-difftest neutral-term bridge the query differential's
// value-level comparators run on (see its module docs for the independence constraint).
mod neutral;
mod update_fuzz;

/// The query workload. Each exercises a different plan shape.
const QUERIES: &[(&str, &str)] = &[
    ("scan-type", "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s a ex:Person }"),
    (
        "star-3",
        "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:name ?n . ?s ex:age ?ag . ?s ex:city ?c }",
    ),
    (
        "chain-2",
        "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:follows ?b . ?b ex:follows ?c }",
    ),
    (
        "triangle",
        "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:follows ?b . ?b ex:follows ?c . ?c ex:follows ?a }",
    ),
    ("filter-age", "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . FILTER(?a > 70) }"),
    ("count-edges", "PREFIX ex: <http://ex/> SELECT (COUNT(*) AS ?c) WHERE { ?s ex:follows ?o }"),
    (
        "optional",
        "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:name ?n OPTIONAL { ?s ex:age ?a } }",
    ),
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scale: u32 = arg_val(&args, "--scale").and_then(|s| s.parse().ok()).unwrap_or(20_000);
    let iters: usize = arg_val(&args, "--iters").and_then(|s| s.parse().ok()).unwrap_or(5);

    // `sparq-bench dump <scale> <out.nt>` — write the synthetic dataset for the
    // larger-scale QLever comparison.
    if args.get(1).map(String::as_str) == Some("dump") {
        let scale: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(scale);
        let out = args.get(3).map(String::as_str).unwrap_or("synthetic.nt");
        let t = Instant::now();
        let n = dataset::write_nt(scale, out).expect("write dataset");
        eprintln!("wrote {n} triples ({scale} entities) to {out} in {:.1}s", t.elapsed().as_secs_f64());
        return;
    }

    // `sparq-bench fuzz <seed_start> <count> [category]` — differential fuzzer vs
    // Oxigraph (correctness audit of the optimizations).
    if args.get(1).map(String::as_str) == Some("fuzz") {
        let seed_start: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        let count: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1000);
        let category = args.get(4).map(String::as_str).unwrap_or("all");
        fuzz::run(seed_start, count, category);
        return;
    }

    // `sparq-bench update-fuzz --seed-start N --seed-count M` — SPARQL UPDATE
    // differential vs Oxigraph (sq-3dyje.4): random ground-term update sequences
    // through both sparq update paths + Oxigraph, canonical per-step compare.
    if args.get(1).map(String::as_str) == Some("update-fuzz") {
        let seed_start: u64 =
            arg_val(&args, "--seed-start").and_then(|s| s.parse().ok()).unwrap_or(0);
        let count: u64 =
            arg_val(&args, "--seed-count").and_then(|s| s.parse().ok()).unwrap_or(1000);
        update_fuzz::run(seed_start, count);
        return;
    }

    // `sparq-bench diff <data.ttl> <query.rq|inline-sparql>` — run ONE hand-crafted
    // (graph, query) through sparq and Oxigraph and print both solution counts.
    // Lets an adversarial agent probe a specific edge case. Exit 1 on mismatch.
    if args.get(1).map(String::as_str) == Some("diff") {
        let data_path = args.get(2).expect("usage: diff <data.ttl> <query>");
        let q_arg = args.get(3).expect("usage: diff <data.ttl> <query>");
        let ttl = std::fs::read_to_string(data_path).expect("read data file");
        let q = std::fs::read_to_string(q_arg).unwrap_or_else(|_| q_arg.clone());
        let g = sparq_core::Graph::load_str(&ttl, "turtle").expect("sparq load");
        let store = oxigraph::store::Store::new().unwrap();
        store.load_from_reader(oxigraph::io::RdfFormat::Turtle, ttl.as_bytes()).expect("oxi load");
        let oxi = oxi_count(&store, &q);
        let sparq = match sparq_engine::query(&g, &q) {
            Ok(r) => r.len() as i64,
            Err(e) => {
                println!("sparq=ERROR({e}) oxi={oxi}");
                return;
            }
        };
        let cnt = sparq_engine::count(&g, &q).map(|c| c as i64).unwrap_or(-1);
        let ok = sparq == oxi as i64;
        println!("sparq={sparq} sparq_count={cnt} oxi={oxi} match={ok}");
        if !ok {
            std::process::exit(1);
        }
        return;
    }

    // [OPUS-4.8] sq-ays7 — `sparq-bench bench-corpus <corpus> <format> <queries-dir> [iters]`:
    // load a REAL featured-suite corpus (the same file `sparq-cli bench` consumes) into
    // Oxigraph and time each `.rq` query, min-of-N, COUNT semantics — emitting the BYTE-FOR-BYTE
    // same `<name>\t<rows>\t<best_us>` TSV contract as crates/sparq-cli run_query_suite (count
    // mode). This is the missing same-box Oxigraph path for the featured SPARQL suites
    // (SP2Bench/WatDiv/LUBM/BSBM): run sparq via `sparq-cli bench … count` and Oxigraph via
    // this, on the SAME box + SAME generated corpus + SAME query files, so the two TSVs are a
    // self-consistent same-box comparison (NOT the cross-machine gh-runner CI band).
    if args.get(1).map(String::as_str) == Some("bench-corpus") {
        let corpus = args.get(2).expect("usage: bench-corpus <corpus> <format> <queries-dir> [iters]");
        let format = args.get(3).expect("usage: bench-corpus <corpus> <format> <queries-dir> [iters]");
        let dir = args.get(4).expect("usage: bench-corpus <corpus> <format> <queries-dir> [iters]");
        let q_iters: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(5);
        bench_corpus_oxigraph(corpus, format, dir, q_iters);
        return;
    }

    if let Some(engine) = arg_val(&args, "--mem") {
        mem_subprocess(&engine, scale);
        return;
    }
    compare(scale, iters);
}

fn arg_val(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

// ---- comparison --------------------------------------------------------------

fn compare(scale: u32, iters: usize) {
    eprintln!("generating dataset (scale={scale} entities)…");
    let ttl = dataset::generate(scale);
    let n_triples = ttl.lines().filter(|l| l.trim_end().ends_with('.')).count();
    println!(
        "# sparq vs oxigraph — {scale} entities, ~{:.1}M triples, {} bytes turtle, {iters} iters (min time)\n",
        n_triples as f64 / 1e6,
        ttl.len()
    );

    // ---- load ----
    let t = Instant::now();
    let g = sparq_core::Graph::load_str(&ttl, "turtle").expect("sparq load");
    let sparq_load = t.elapsed();

    let t = Instant::now();
    let store = oxigraph::store::Store::new().expect("oxi store");
    store
        .load_from_reader(oxigraph::io::RdfFormat::Turtle, ttl.as_bytes())
        .expect("oxi load");
    let oxi_load = t.elapsed();

    println!("{:<14} {:>12} {:>12} {:>9}", "stage", "sparq", "oxigraph", "speedup");
    println!("{}", "-".repeat(50));
    println!(
        "{:<14} {:>12} {:>12} {:>8.2}x",
        "load",
        fmt_dur(sparq_load),
        fmt_dur(oxi_load),
        oxi_load.as_secs_f64() / sparq_load.as_secs_f64()
    );
    println!("(triples loaded: sparq={}, oxi={})\n", g.len(), store.len().unwrap());

    // ---- queries ----
    println!(
        "{:<14} {:>9} {:>9} {:>6} {:>12} {:>12} {:>9}",
        "query", "sparq#", "oxi#", "match", "sparq", "oxigraph", "speedup"
    );
    println!("{}", "-".repeat(76));
    let mut mismatches = 0;
    for (name, q) in QUERIES {
        let (sd, srows) = time_min(iters, || sparq_engine::query(&g, q).expect("sparq query").len());
        let (od, orows) = time_min(iters, || oxi_count(&store, q));
        let ok = srows == orows;
        if !ok {
            mismatches += 1;
        }
        println!(
            "{:<14} {:>9} {:>9} {:>6} {:>12} {:>12} {:>8.2}x",
            name,
            srows,
            orows,
            if ok { "ok" } else { "DIFF" },
            fmt_dur(sd),
            fmt_dur(od),
            od.as_secs_f64() / sd.as_secs_f64()
        );
    }

    // ---- memory (separate subprocesses for clean peak RSS) ----
    println!("\nmemory (peak process RSS, includes the parse buffer — approximate):");
    let sparq_mem = measure_mem_subprocess("sparq", scale);
    let oxi_mem = measure_mem_subprocess("oxigraph", scale);
    println!(
        "  sparq    {:>10}   (store self-estimate {})",
        fmt_bytes(sparq_mem),
        fmt_bytes(g.heap_bytes())
    );
    println!("  oxigraph {:>10}", fmt_bytes(oxi_mem));
    if sparq_mem > 0 && oxi_mem > 0 {
        println!("  ratio    {:.2}x (oxi/sparq)", oxi_mem as f64 / sparq_mem as f64);
    }

    if mismatches > 0 {
        eprintln!("\nWARNING: {mismatches} query result-count mismatch(es) vs oxigraph");
        std::process::exit(1);
    } else {
        println!("\nall {} queries agree with oxigraph ✓", QUERIES.len());
    }
}

/// [OPUS-4.8] sq-ays7 — load a featured-suite corpus into Oxigraph and time each `.rq`
/// query (min-of-N, COUNT semantics), emitting the SAME `<name>\t<rows>\t<best_us>` TSV
/// contract as `sparq-cli bench … count`. The query loop mirrors run_query_suite exactly:
/// sort the `.rq` files, read each, run `iters` times keeping the min wall-µs, report the
/// solution count as `rows`. ERROR lines mirror the sparq path so a query Oxigraph cannot
/// run is recorded (not silently dropped) and stays out of the comparison.
fn bench_corpus_oxigraph(corpus: &str, format: &str, dir: &str, iters: usize) {
    let fmt = match format.to_ascii_lowercase().as_str() {
        "ntriples" | "nt" | "n-triples" => oxigraph::io::RdfFormat::NTriples,
        "turtle" | "ttl" => oxigraph::io::RdfFormat::Turtle,
        "nquads" | "nq" => oxigraph::io::RdfFormat::NQuads,
        "trig" => oxigraph::io::RdfFormat::TriG,
        other => {
            eprintln!("bench-corpus: unsupported format `{other}` (ntriples|turtle|nquads|trig)");
            std::process::exit(2);
        }
    };
    let f = std::fs::File::open(corpus).unwrap_or_else(|e| {
        eprintln!("bench-corpus: cannot open corpus {corpus}: {e}");
        std::process::exit(1);
    });
    let t = Instant::now();
    let store = oxigraph::store::Store::new().expect("oxi store");
    store
        .load_from_reader(fmt, std::io::BufReader::new(f))
        .unwrap_or_else(|e| {
            eprintln!("bench-corpus: oxigraph load failed: {e}");
            std::process::exit(1);
        });
    eprintln!(
        "# oxigraph bench-corpus: loaded {} triples from {corpus} in {:.3}s ({iters} iters/query, min)",
        store.len().unwrap_or(0),
        t.elapsed().as_secs_f64()
    );

    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| {
            eprintln!("bench-corpus: error reading {dir}: {e}");
            std::process::exit(1);
        })
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "rq").unwrap_or(false))
        .collect();
    entries.sort();

    for path in entries {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let sparql = std::fs::read_to_string(&path).unwrap();
        let mut best = f64::INFINITY;
        let mut rows = 0usize;
        let mut err: Option<String> = None;
        for _ in 0..iters.max(1) {
            let t = Instant::now();
            match oxi_count_checked(&store, &sparql) {
                Ok(n) => {
                    rows = n;
                    best = best.min(t.elapsed().as_secs_f64() * 1e6);
                }
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }
        match err {
            None => println!("{name}\t{rows}\t{best:.1}"),
            Some(e) => println!("{name}\tERROR\t{e}"),
        }
    }
}

// Non-panicking variant of oxi_count: returns the solution/row count or the engine error
// string, so bench-corpus can record an ERROR line instead of aborting the whole suite.
#[allow(deprecated)]
fn oxi_count_checked(store: &oxigraph::store::Store, q: &str) -> Result<usize, String> {
    match store.query(q).map_err(|e| e.to_string())? {
        oxigraph::sparql::QueryResults::Solutions(s) => {
            // count() consumes the iterator; a row that fails to evaluate is an error.
            let mut n = 0usize;
            for r in s {
                r.map_err(|e| e.to_string())?;
                n += 1;
            }
            Ok(n)
        }
        oxigraph::sparql::QueryResults::Boolean(_) => Ok(1),
        oxigraph::sparql::QueryResults::Graph(g) => {
            let mut n = 0usize;
            for r in g {
                r.map_err(|e| e.to_string())?;
                n += 1;
            }
            Ok(n)
        }
    }
}

fn time_min(iters: usize, mut f: impl FnMut() -> usize) -> (Duration, usize) {
    let mut best = Duration::MAX;
    let mut rows = 0;
    for _ in 0..iters.max(1) {
        let t = Instant::now();
        rows = f();
        best = best.min(t.elapsed());
    }
    (best, rows)
}

// clippy: differential oracle pins oxigraph's legacy Store::query semantics
#[allow(deprecated)]
fn oxi_count(store: &oxigraph::store::Store, q: &str) -> usize {
    match store.query(q).expect("oxi query") {
        oxigraph::sparql::QueryResults::Solutions(s) => s.count(),
        oxigraph::sparql::QueryResults::Boolean(_) => 1,
        oxigraph::sparql::QueryResults::Graph(g) => g.count(),
    }
}

// ---- memory subprocess -------------------------------------------------------

fn mem_subprocess(engine: &str, scale: u32) {
    let ttl = dataset::generate(scale);
    // Measure peak RSS right after load — before any result-materialising query —
    // so the figure reflects store + parse, not a large intermediate result.
    let footprint;
    match engine {
        "sparq" => {
            let g = sparq_core::Graph::load_str(&ttl, "turtle").unwrap();
            footprint = g.heap_bytes();
            let rss = peak_rss_bytes();
            std::hint::black_box(&g);
            println!("{rss} {footprint}");
        }
        "oxigraph" => {
            let store = oxigraph::store::Store::new().unwrap();
            store.load_from_reader(oxigraph::io::RdfFormat::Turtle, ttl.as_bytes()).unwrap();
            let rss = peak_rss_bytes();
            std::hint::black_box(&store);
            println!("{rss} 0");
        }
        other => panic!("unknown engine {other}"),
    }
}

fn measure_mem_subprocess(engine: &str, scale: u32) -> usize {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let out = Command::new(exe)
        .args(["--mem", engine, "--scale", &scale.to_string()])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .split_whitespace()
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        _ => 0,
    }
}

fn peak_rss_bytes() -> usize {
    // SAFETY: `rusage` is a plain C struct that is sound to zero-initialise, and
    // `getrusage(RUSAGE_SELF, &mut ru)` writes only into the valid `&mut ru`
    // out-param; the return is checked before any field is read. [OPUS-4.8]
    unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut ru) != 0 {
            return 0;
        }
        let m = ru.ru_maxrss as usize;
        // macOS reports bytes; Linux/BSD report kilobytes.
        if cfg!(target_os = "macos") {
            m
        } else {
            m * 1024
        }
    }
}

// ---- formatting --------------------------------------------------------------

fn fmt_dur(d: Duration) -> String {
    let us = d.as_secs_f64() * 1e6;
    if us < 1000.0 {
        format!("{us:.1}µs")
    } else if us < 1e6 {
        format!("{:.2}ms", us / 1e3)
    } else {
        format!("{:.3}s", us / 1e6)
    }
}

fn fmt_bytes(b: usize) -> String {
    let b = b as f64;
    if b < 1024.0 {
        format!("{b:.0}B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1}KiB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1}MiB", b / 1024.0 / 1024.0)
    } else {
        format!("{:.2}GiB", b / 1024.0 / 1024.0 / 1024.0)
    }
}
