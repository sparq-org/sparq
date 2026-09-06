#![doc = include_str!("../README.md")]
// [OPUS-4.8] MS-G2 (sq-8wbn): make `// SAFETY:` mandatory on every first-party unsafe block.
#![warn(clippy::undocumented_unsafe_blocks)]

use std::io::Read;
use std::time::Instant;

// [FABLE-5] (sq-lsp7k.8) Materializing tabular→RDF import (CSV direct mapping + the R2RML
// materializing subset over CSV logical tables). The whole module exists only under the
// opt-in `tabular` feature; the default CLI build carries none of it.
#[cfg(feature = "tabular")]
mod tabular;

// T1.0 scaling lever: replace the system allocator (whose per-thread arena locks contend under
// rayon's many-worker per-row allocation) with mimalloc (sharded, lock-light). Compile-time;
// `--no-default-features --features mmap` builds with the system allocator for A/B.
#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("query") => cmd_query(&args),
        Some("reason") => cmd_reason(&args),
        // [OPUS-5] (sq-2ch27, Phase E6) `classify` materializes the OWL 2 EL subsumption lattice
        // (`sparq-reason-el`, `rbox` on / `cdomain` off — complete for the E1+E2 fragment, with
        // concrete-domain axioms deferred into `skipped_axioms`) — the class hierarchy OWL 2 RL is
        // sound but silently incomplete for. Gated behind the opt-in `el` feature: the default CLI
        // build carries no EL code, so the subcommand is only present under `--features el`.
        #[cfg(feature = "el")]
        Some("classify") => cmd_classify(&args),
        Some("bench") => cmd_bench(&args),
        Some("memstat") => cmd_memstat(&args),
        Some("bench-mmap") => cmd_bench_mmap(&args),
        Some("ingest") => cmd_ingest(&args),
        Some("save") => cmd_save(&args),
        Some("recompress") => cmd_recompress(&args),
        Some("compact") => cmd_compact(&args),
        // [GPT-5.6] (sq-lsp7k.28) Deterministic RDF triple-set diff. The entire command
        // is opt-in, so feature-OFF builds retain the previous dispatch and binary surface.
        #[cfg(feature = "diff")]
        Some("diff") => cmd_diff(&args),
        Some("build") => cmd_build(&args),
        Some("query-mmap") => cmd_query_mmap(&args),
        Some("probe-compress") => cmd_probe_compress(&args),
        Some("compare-compress") => cmd_compare_compress(&args),
        Some("bench-remap") => cmd_bench_remap(&args),
        Some("scaling") => cmd_scaling(&args),
        // [OPUS-4.8] (sq-678h) `dump` re-serializes a loaded RDF document into the writer
        // matrix (turtle / trig / nquads). Gated behind the opt-in `serialize-rdf` feature:
        // the default CLI build carries no serializer code, so the subcommand is only present
        // when built with `--features serialize-rdf`.
        #[cfg(feature = "serialize-rdf")]
        Some("dump") => cmd_dump(&args),
        // [OPUS-4.8] (sq-vczh2) `terse` transpiles a terse query (the `K:<name>` keyword layer
        // over canonical SPARQL) into the canonical SPARQL it expands to, printing the verifiable
        // JSON contract. Gated behind the opt-in `terse` feature: the default CLI build carries no
        // terse code, so the subcommand is only present when built with `--features terse`.
        #[cfg(feature = "terse")]
        Some("terse") => cmd_terse(&args),
        // [FABLE-5] (sq-8ju74) `to-hdt` EXPORTS a loaded RDF document as a standard-layout
        // HDT v1.0 archive via sparq-hdt's direct in-memory encoder. Gated behind the opt-in
        // `hdt-write` cargo feature (which implies `hdt`, the loader): the default CLI build
        // carries no HDT code, so the subcommand is only present when built with
        // `--features hdt-write`.
        #[cfg(feature = "hdt-write")]
        Some("to-hdt") => cmd_to_hdt(&args),
        // [FABLE-5] (sq-lsp7k.8) `tabular` materializes RDF from CSV — direct mapping by
        // default, R2RML (CSV logical tables) with `--mapping` — then loads the graph (and
        // optionally queries it) or streams N-Triples to `--out`. Gated behind the opt-in
        // `tabular` feature: the default CLI build carries no CSV/R2RML code and the
        // subcommand is absent.
        #[cfg(feature = "tabular")]
        Some("tabular") => tabular::cmd_tabular(&args),
        _ => {
            eprintln!("usage:\n  sparq-cli query <data-file> <format> <sparql> [--format <table|tsv|csv|xml|json|ntriples>] [--count]\n  sparq-cli bench <data-file> <format> <queries-dir> [iters]\n  sparq-cli memstat <data-file> <format> [compressed]  # deterministic memory-composition breakdown (B/triple) + RSS\n  sparq-cli ingest <file[.gz|.bz2]> [parse|intern|full] [max_millions]\n  sparq-cli save <data-file> <format> <dir> [compressed]  # build + persist indexes to disk\n  sparq-cli recompress <src-dir> <dst-dir>          # re-persist with block-compressed indexes\n  sparq-cli compact <persist-dir>                   # WAL compact/vacuum: physically purge erased data (offline)\n  sparq-cli query-mmap <dir> <sparql> [--format <table|tsv|csv|xml|json|ntriples>] [--count]  # query with indexes MEMORY-MAPPED (out-of-core)\n\n  env SPARQ_STORE_PROFILE=raw|compressed selects the in-RAM store profile on the shared load path (query/bench/reason/scaling); unset=raw; unknown value is a hard error");
            std::process::exit(2);
        }
    }
}

/// Isolated micro-benchmark of the latency-bound dict-remap gather, to measure the per-ISA
/// software prefetch in isolation (undiluted by parsing) on each hardware target.
///   sparq-cli bench-remap n_triples dict_size iters
/// Run twice — once normally, once with SPARQ_NO_PREFETCH=1 — to get the prefetch delta.
fn cmd_bench_remap(args: &[String]) {
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20_000_000);
    let dict: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(50_000_000);
    let iters: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(5);
    let pf = std::env::var("SPARQ_NO_PREFETCH").as_deref() != Ok("1");
    let ms = sparq_core::bench_remap(n, dict, iters);
    let mtps = (n as f64) / (ms / 1e3) / 1e6;
    println!("remap\tn={n}\tdict={dict}\tprefetch={pf}\tbest_ms={ms:.2}\tMtriples_s={mtps:.2}");
}

/// Per-subsystem parallel SCALING harness (roadmap T8). Builds fixed-size rayon thread pools
/// (1,2,4,8,…) and runs each subsystem inside `pool.install()`, so the engine's `par_iter`/
/// `par_chunks` (which size off `rayon::current_num_threads()`) use exactly that many threads —
/// sweeping the thread count in ONE process. Reports best time, speedup vs the smallest pool, and
/// parallel EFFICIENCY (speedup ÷ thread-ratio; 1.0 = perfectly linear) per subsystem, so you can
/// see precisely where each part plateaus. On a many-core box pass e.g. `1,2,4,8,16,32,64,128,192`.
///   sparq-cli scaling `<data-file>` `<format>` `<queries-dir>` [threads=auto] [iters=3]
fn cmd_scaling(args: &[String]) {
    let (path, format, qdir) = match (args.get(2), args.get(3), args.get(4)) {
        (Some(p), Some(f), Some(d)) => (p.clone(), f.clone(), d.clone()),
        _ => {
            eprintln!("usage: sparq-cli scaling <data-file> <format> <queries-dir> [threads=1,2,4,8,…] [iters=3]");
            std::process::exit(2);
        }
    };
    let threads: Vec<usize> = match args.get(5) {
        Some(s) => {
            let mut v: Vec<usize> = s.split(',').filter_map(|t| t.trim().parse().ok()).filter(|&n| n >= 1).collect();
            v.dedup();
            v
        }
        None => {
            let max = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
            let mut v = vec![1usize];
            let mut n = 2;
            while n < max {
                v.push(n);
                n *= 2;
            }
            if *v.last().unwrap() != max {
                v.push(max);
            }
            v
        }
    };
    let iters: usize = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(3);
    if threads.is_empty() {
        eprintln!("no valid thread counts");
        std::process::exit(2);
    }
    let base = threads[0];
    eprintln!(
        "scaling sweep: threads={threads:?} iters={iters}  (efficiency = speedup ÷ (threads/{base}); 1.0 = linear)"
    );
    let pool = |n: usize| rayon::ThreadPoolBuilder::new().num_threads(n).build().expect("rayon pool");

    println!("subsystem\tthreads\tbest_ms\tspeedup\tefficiency");

    // LOAD subsystem — re-load the file inside each pool size (parse + dict-merge + 6 perms).
    {
        let mut t_base = 0.0f64;
        for (i, &n) in threads.iter().enumerate() {
            let p = pool(n);
            let mut best = f64::INFINITY;
            for _ in 0..iters {
                let t = Instant::now();
                let g = p.install(|| load_quiet(&path, &format));
                best = best.min(t.elapsed().as_secs_f64() * 1e3);
                std::hint::black_box(&g);
            }
            if i == 0 {
                t_base = best;
            }
            let sp = t_base / best;
            let eff = sp / (n as f64 / base as f64);
            println!("load\t{n}\t{best:.1}\t{sp:.2}\t{eff:.2}");
        }
    }

    // QUERY subsystems — load once, run each query at each pool size in `materialize` form (compute
    // all bindings, no serialization): that is the parallel compute we want to scale.
    let g = load(&path, &format);
    let mut queries: Vec<(String, String)> = std::fs::read_dir(&qdir)
        .unwrap_or_else(|e| {
            eprintln!("read {qdir}: {e}");
            std::process::exit(1);
        })
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "rq").unwrap_or(false))
        .filter_map(|p| {
            let name = p.file_stem()?.to_string_lossy().into_owned();
            Some((name, std::fs::read_to_string(&p).ok()?))
        })
        .collect();
    queries.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, sparql) in &queries {
        let mut t_base = 0.0f64;
        let mut ok = true;
        for (i, &n) in threads.iter().enumerate() {
            let p = pool(n);
            let mut best = f64::INFINITY;
            for _ in 0..iters {
                let t = Instant::now();
                match p.install(|| sparq_engine::query(&g, sparql).map(|r| r.len())) {
                    Ok(rows) => {
                        best = best.min(t.elapsed().as_secs_f64() * 1e3);
                        std::hint::black_box(rows);
                    }
                    Err(e) => {
                        eprintln!("{name}: query error: {e}");
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                break;
            }
            if i == 0 {
                t_base = best;
            }
            let sp = t_base / best;
            let eff = sp / (n as f64 / base as f64);
            println!("{name}\t{n}\t{best:.1}\t{sp:.2}\t{eff:.2}");
        }
    }
}

/// Streaming-ingest throughput experiment (for the Wikidata-vs-RDFox comparison).
/// Decompresses (.gz/.bz2) and parses N-Triples from a stream, never holding the
/// whole document on disk or in memory:
///   parse  — decompress + parse + count only (constant memory, unbounded): the
///            raw front-end ceiling.
///   intern — + dictionary interning (memory grows with distinct terms); capped.
///   full   — + collect triples and build the six permutation indexes; capped.
/// Reports triples/s and extrapolates to full Wikidata truthy (~8.0B triples).
fn cmd_ingest(args: &[String]) {
    let path = match args.get(2) {
        Some(p) => p.clone(),
        None => {
            eprintln!("usage: sparq-cli ingest <file[.gz|.bz2]> [parse|intern|full] [max_millions]");
            std::process::exit(2);
        }
    };
    let mode = args.get(3).map(String::as_str).unwrap_or("parse");
    let cap: u64 = args.get(4).and_then(|s| s.parse::<u64>().ok()).map(|m| m * 1_000_000).unwrap_or(u64::MAX);

    let file = std::fs::File::open(&path).unwrap_or_else(|e| {
        eprintln!("open {path}: {e}");
        std::process::exit(1);
    });
    // `+ Send` so the build can run decompression on its own (overlapping) thread.
    let decoded: Box<dyn Read + Send> = if path.ends_with(".bz2") {
        Box::new(bzip2::read::MultiBzDecoder::new(file))
    } else if path.ends_with(".gz") {
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else if path.ends_with(".zst") || path.ends_with(".zstd") {
        // zstd decompresses ~12x faster than bzip2, so it stops being the ingestion
        // bottleneck; recompress a .bz2 source once with `zstd -9 -T0` to use it.
        // [GPT-5.6] (sq-fo528) Accept frames produced with zstd's large-window mode.
        let mut decoder = zstd::stream::read::Decoder::new(file).unwrap_or_else(|e| {
            eprintln!("zstd decode {path}: {e}");
            std::process::exit(1);
        });
        decoder.window_log_max(31).unwrap_or_else(|e| {
            eprintln!("zstd decode {path}: {e}");
            std::process::exit(1);
        });
        Box::new(decoder)
    } else {
        Box::new(file)
    };
    let reader = std::io::BufReader::with_capacity(1 << 22, decoded);

    let mut dict = sparq_core::dict::Dict::new();
    let mut triples: Vec<[u32; 3]> = Vec::new();
    let mut n: u64 = 0u64;
    let t0 = Instant::now();
    let mut last = 0u64;
    let mut last_t = Instant::now();

    eprintln!("ingest mode={mode} cap={} from {path}", if cap == u64::MAX { "none".into() } else { format!("{}M", cap / 1_000_000) });

    for triple in oxttl::NTriplesParser::new().for_reader(reader) {
        let t = match triple {
            Ok(t) => t,
            Err(e) => {
                // A truncated prefix (we only downloaded part of the dump) ends in
                // a decode/parse error — expected; stop cleanly.
                eprintln!("(stream ended after {n} triples: {e})");
                break;
            }
        };
        if mode != "parse" {
            let s = dict.intern(&subject_to_term(&t.subject));
            let p = dict.intern(&oxrdf::Term::NamedNode(t.predicate.clone()));
            let o = dict.intern(&t.object);
            if mode == "full" {
                triples.push([s, p, o]);
            }
        }
        n += 1;
        if n - last >= 5_000_000 {
            let dt = last_t.elapsed().as_secs_f64();
            eprintln!(
                "  {n:>12} triples  |  {:.2} M/s (window)  |  {:.2} M/s (avg)  |  {} distinct terms",
                (n - last) as f64 / 1e6 / dt,
                n as f64 / 1e6 / t0.elapsed().as_secs_f64(),
                dict.len()
            );
            last = n;
            last_t = Instant::now();
        }
        if n >= cap {
            eprintln!("(reached cap of {} triples)", cap);
            break;
        }
    }

    let parse_secs = t0.elapsed().as_secs_f64();
    let mut build_secs = 0.0;
    if mode == "full" && !triples.is_empty() {
        let tb = Instant::now();
        let store = sparq_core::store::TripleStore::from_triples(triples);
        build_secs = tb.elapsed().as_secs_f64();
        std::hint::black_box(&store);
    }

    let total = parse_secs + build_secs;
    let rate = n as f64 / total.max(1e-9);
    println!("\n=== ingest summary ({mode}) ===");
    println!("triples ingested : {n}");
    if mode != "parse" {
        println!("distinct terms   : {}", dict.len());
    }
    println!("decompress+parse : {parse_secs:.1}s");
    if mode == "full" {
        println!("index build      : {build_secs:.1}s (6 permutations)");
    }
    println!("total            : {total:.1}s");
    println!("throughput       : {:.2} M triples/s", rate / 1e6);
    // Extrapolate to full Wikidata truthy (~8.0B triples).
    let wikidata = 8.0e9;
    println!("extrapolated full Wikidata truthy (~8.0B triples): {:.0} min ({:.1} h) at this rate", wikidata / rate / 60.0, wikidata / rate / 3600.0);
}

fn subject_to_term(s: &oxrdf::NamedOrBlankNode) -> oxrdf::Term {
    match s {
        oxrdf::NamedOrBlankNode::NamedNode(n) => oxrdf::Term::NamedNode(n.clone()),
        oxrdf::NamedOrBlankNode::BlankNode(b) => oxrdf::Term::BlankNode(b.clone()),
    }
}

// ===== [SONNET-4.6] sq-kmve2 — the SPQCPRM2 V2 emit FLAG =====
//
// `save … compressed --format-v2` and `recompress … --v2` select the `SPQCPRM2` on-disk block
// stream for THIS invocation, so asking for V2 no longer means exporting `SPARQ_EMIT_FORMAT=v2`
// into the process environment (which leaks into every child process and is easy to leave set).
// The flag maps onto `compress::with_emit_format` — the PER-THREAD override, which takes
// precedence over the env var and is read on the writing thread that `save_compressed` encodes
// on. Passing no flag leaves the env-var path exactly as it was.
//
// FAIL-CLOSED in three places, because silently writing the OTHER on-disk format is the one
// outcome worse than an error: `--format-v2` without the `compressed` positional is rejected
// (raw perms carry no block-stream version); a binary built WITHOUT the opt-in `spqcprm2`
// feature rejects the flag instead of quietly emitting `SPQCPRM1`; and an unrecognised
// `--flag` on either subcommand is rejected rather than ignored. All three run BEFORE the dataset
// is loaded/opened, so a bad invocation fails in milliseconds rather than after a long ingest.

/// Usage line for `save` (also the too-few-arguments error).
const SAVE_USAGE: &str = "sparq-cli save <data-file> <format> <dir> [compressed] [--format-v2]";
/// Usage line for `recompress`.
const RECOMPRESS_USAGE: &str = "sparq-cli recompress <src-dir> <dst-dir> [--v2]   (dirs must differ)";

/// [SONNET-4.6] (sq-kmve2) Splits `args` into positionals and the V2-emit flag. `--format-v2`
/// (the `save` spelling) and `--v2` (the `recompress` spelling) are accepted interchangeably;
/// any OTHER `--`-prefixed argument is a hard usage error, so a typo'd flag can never be
/// silently dropped and leave the caller believing it wrote V2.
fn take_emit_v2(args: &[String], usage: &str) -> (Vec<String>, bool) {
    let mut positional = Vec::with_capacity(args.len());
    let mut v2 = false;
    for a in args {
        match a.as_str() {
            "--format-v2" | "--v2" => v2 = true,
            other if other.starts_with("--") => {
                eprintln!("unknown flag {}\nusage: {}", other, usage);
                std::process::exit(2);
            }
            _ => positional.push(a.clone()),
        }
    }
    (positional, v2)
}

/// [SONNET-4.6] (sq-kmve2) V2 emission is compiled in — nothing to reject.
#[cfg(feature = "spqcprm2")]
fn check_emit_v2(_v2: bool) {}

/// [SONNET-4.6] (sq-kmve2) This binary cannot emit `SPQCPRM2` (the `spqcprm2` feature is off),
/// so the flag is a loud error rather than a silent `SPQCPRM1` write.
#[cfg(not(feature = "spqcprm2"))]
fn check_emit_v2(v2: bool) {
    if v2 {
        eprintln!(
            "error: --format-v2/--v2 needs a build with the opt-in `spqcprm2` cargo feature; this binary emits SPQCPRM1 only.\n       rebuild with: cargo build --release -p sparq-cli --features spqcprm2"
        );
        std::process::exit(2);
    }
}

/// [SONNET-4.6] (sq-kmve2) Runs `f` (a compressed save) with this thread's emit format forced to
/// `SPQCPRM2` when `v2`; otherwise runs it untouched, so the default path stays bit-identical.
#[cfg(feature = "spqcprm2")]
fn with_emit_v2<R>(v2: bool, f: impl FnOnce() -> R) -> R {
    if v2 { sparq_core::compress::with_emit_format(sparq_core::compress::EmitFormat::V2, f) } else { f() }
}

/// [SONNET-4.6] (sq-kmve2) Feature-OFF twin: `check_emit_v2` has already exited on `v2`, so this
/// only ever runs the closure unchanged.
#[cfg(not(feature = "spqcprm2"))]
fn with_emit_v2<R>(_v2: bool, f: impl FnOnce() -> R) -> R {
    f()
}

/// `save <data> <format> <dir> [compressed] [--format-v2]` — build the store and persist its
/// indexes to disk; `compressed` writes the block-compressed permutation format (auto-detected
/// by `query-mmap`/`bench-mmap`), and `--format-v2` writes it as `SPQCPRM2` instead of
/// `SPQCPRM1` (opt-in `spqcprm2` builds only; see the emit-flag notes above).
fn cmd_save(args: &[String]) {
    let (args, v2) = take_emit_v2(args, SAVE_USAGE);
    let (path, format, dir) = match (args.get(2), args.get(3), args.get(4)) {
        (Some(p), Some(f), Some(d)) => (p.as_str(), f.as_str(), d.as_str()),
        _ => {
            eprintln!("usage: {}", SAVE_USAGE);
            std::process::exit(2);
        }
    };
    let compressed = args.get(5).map(String::as_str) == Some("compressed");
    if v2 && !compressed {
        eprintln!("error: --format-v2 selects the SPQCPRM2 *compressed*-permutation format; add the `compressed` positional\nusage: {}", SAVE_USAGE);
        std::process::exit(2);
    }
    check_emit_v2(v2);
    let g = load(path, format);
    let t = Instant::now();
    let res = if compressed {
        with_emit_v2(v2, || g.save_compressed(std::path::Path::new(dir)))
    } else {
        g.save(std::path::Path::new(dir))
    };
    res.unwrap_or_else(|e| {
        eprintln!("save error: {e}");
        std::process::exit(1);
    });
    eprintln!(
        "saved {} triples to {dir}{} in {:.3}s",
        g.len(),
        match (compressed, v2) {
            (true, true) => " (compressed perms, SPQCPRM2)",
            (true, false) => " (compressed perms)",
            _ => "",
        },
        t.elapsed().as_secs_f64()
    );
}

/// `recompress <src-dir> <dst-dir> [--v2]` — re-persist a saved dataset with block-compressed
/// permutation indexes (the dictionary/numerics are rewritten unchanged). Lets a raw
/// (e.g. external-memory `build`) directory be compacted without re-parsing the source.
/// `--v2` writes `SPQCPRM2` instead of `SPQCPRM1` (opt-in `spqcprm2` builds only).
fn cmd_recompress(args: &[String]) {
    let (args, v2) = take_emit_v2(args, RECOMPRESS_USAGE);
    let (src, dst) = match (args.get(2), args.get(3)) {
        (Some(s), Some(d)) if s != d => (s.as_str(), d.as_str()),
        _ => {
            eprintln!("usage: {}", RECOMPRESS_USAGE);
            std::process::exit(2);
        }
    };
    check_emit_v2(v2);
    let g = sparq_core::Graph::open(std::path::Path::new(src)).unwrap_or_else(|e| {
        eprintln!("open error: {e}");
        std::process::exit(1);
    });
    let t = Instant::now();
    with_emit_v2(v2, || g.save_compressed(std::path::Path::new(dst))).unwrap_or_else(|e| {
        eprintln!("save error: {e}");
        std::process::exit(1);
    });
    // Only claim a format when the FLAG chose it: an `SPARQ_EMIT_FORMAT=v2` process writes V2
    // without the flag, so an unconditional "(SPQCPRM1)" here would be a false report.
    eprintln!(
        "recompressed {} triples {src} -> {dst}{} in {:.3}s",
        g.len(),
        if v2 { " (SPQCPRM2)" } else { "" },
        t.elapsed().as_secs_f64()
    );
}

/// [OPUS-4.8] (sq-x32t) `compact <persist-dir>` — WAL COMPACTION / VACUUM for ERASURE-
/// COMPLETENESS (epic sq-toze.33). An OFFLINE operator command: stop the server, run this on its
/// `--persist <dir>`, restart. A logical SPARQL `DELETE` / `DROP GRAPH` retracts data from the
/// live view but leaves the superseded bytes in earlier WAL segments until a compaction folds the
/// live state into a fresh base; this command does exactly that — physically rewriting the store
/// to contain ONLY the current live triples, so erased data is gone from the on-disk history.
///
/// `Graph::open` replays the WAL into the live overlay, `Graph::vacuum` re-interns the live
/// triples into a fresh dictionary (so orphaned term VALUES are purged too) and ATOMICALLY
/// (rollback-safe two-rename swap, parent dir fsync'd between renames, WAL truncated) replaces the
/// on-disk store; an interrupted swap is healed deterministically on the next open. The live
/// triple set is preserved exactly (round-trip).
///
/// Online equivalent: `POST /admin/compact` on a running `--persist` server (gated by the write
/// auth token). The complement on the server side runs on the writer thread between batches.
///
/// PHYSICAL-ERASURE CAVEAT (honest scope): this scrubs the engine's own on-disk segments; it
/// cannot reach bytes already copied off-box — filesystem snapshots, block-level COW history, or
/// external backups — which the storage/backup tier must handle per the retention-erasure runbook.
fn cmd_compact(args: &[String]) {
    let dir = match args.get(2) {
        Some(d) => d.as_str(),
        None => {
            eprintln!("usage: sparq-cli compact <persist-dir>   # offline WAL compact/vacuum for erasure-completeness");
            std::process::exit(2);
        }
    };
    let path = std::path::Path::new(dir);
    // `open` replays the WAL into the live overlay (and heals any interrupted prior compaction).
    let mut g = sparq_core::Graph::open(path).unwrap_or_else(|e| {
        eprintln!("open error ({dir}): {e}");
        std::process::exit(1);
    });
    let before = g.len();
    let t = Instant::now();
    // [OPUS-4.8] (sq-x32t) ERASURE-GRADE vacuum: atomic, crash-safe rewrite of the on-disk store
    // to only the live triples + a fresh dictionary (so orphaned term VALUES are purged too) +
    // WAL truncate. `vacuum` re-interns; the lighter `Graph::compact` keeps the dict and would
    // leave a deleted literal's bytes on disk, which is not erasure-complete.
    g.vacuum().unwrap_or_else(|e| {
        eprintln!("compact error ({dir}): {e}");
        std::process::exit(1);
    });
    let after = g.len();
    // `len` is invariant across compaction (the live set is preserved exactly); print it as a
    // round-trip sanity signal, not as a deletion count (the purged data was already retracted).
    eprintln!(
        "compacted {dir}: {} live triples (was {before}) in {:.3}s — superseded/erased data physically removed from the on-disk WAL history",
        after,
        t.elapsed().as_secs_f64()
    );
}

/// Infers an RDF input format from a path, after removing one compression extension.
#[cfg(feature = "diff")]
fn diff_format(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    let uncompressed = [".gz", ".bz2", ".zst", ".zstd"]
        .into_iter()
        .find_map(|suffix| lower.strip_suffix(suffix))
        .unwrap_or(&lower);
    match std::path::Path::new(uncompressed)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
    {
        Some("nt" | "ntriples") => "ntriples",
        Some("ttl" | "turtle") => "turtle",
        Some("nq" | "nquads") => "nquads",
        Some("trig") => "trig",
        Some("jsonld") => "jsonld",
        _ => {
            eprintln!(
                "cannot infer RDF format from `{path}`; expected .nt, .ttl, .nq, .trig, or .jsonld (optionally compressed)"
            );
            std::process::exit(2);
        }
    }
}

/// Renders every triple in a loaded RDF document as a canonical set of N-Triples lines.
/// Named-graph names are intentionally discarded: this command compares triple sets, not quads.
#[cfg(feature = "diff")]
fn diff_triples(graph: &sparq_core::Graph) -> std::collections::BTreeSet<String> {
    fn extend(graph: &sparq_core::Graph, triples: &mut std::collections::BTreeSet<String>) {
        for [s, p, o] in graph.iter_ids() {
            triples.insert(format!(
                "{} {} {} .",
                graph.dict.term(s),
                graph.dict.term(p),
                graph.dict.term(o)
            ));
        }
        for (_, named) in &graph.named {
            extend(named, triples);
        }
    }

    let mut triples = std::collections::BTreeSet::new();
    extend(graph, &mut triples);
    triples
}

/// `diff <file-a> <file-b> [--exact]` — emit removed lines, then added lines, with
/// each block sorted lexicographically. `--exact` is currently a compatibility alias:
/// both modes compare rendered triple sets, including blank-node labels exactly as loaded.
#[cfg(feature = "diff")]
fn cmd_diff(args: &[String]) {
    let (left_path, right_path) = match args {
        [_, _, left, right] => (left.as_str(), right.as_str()),
        [_, _, left, right, exact] if exact == "--exact" => (left.as_str(), right.as_str()),
        _ => {
            eprintln!("usage: sparq-cli diff <file-a> <file-b> [--exact]");
            std::process::exit(2);
        }
    };

    let left = diff_triples(&load_quiet(left_path, diff_format(left_path)));
    let right = diff_triples(&load_quiet(right_path, diff_format(right_path)));
    let different = left != right;

    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let write_result = (|| -> std::io::Result<()> {
        for triple in left.difference(&right) {
            writeln!(out, "- {triple}")?;
        }
        for triple in right.difference(&left) {
            writeln!(out, "+ {triple}")?;
        }
        out.flush()
    })();
    if let Err(error) = write_result {
        eprintln!("error writing diff: {error}");
        std::process::exit(1);
    }
    if different {
        std::process::exit(1);
    }
}

/// `build <file[.gz|.bz2]> <format> <dir> [chunk_millions]` — EXTERNAL-MEMORY build:
/// stream the (optionally compressed) document straight to on-disk, memory-mapped indexes
/// via disk-backed sort/merge, so a dataset whose indexes exceed RAM can be constructed on
/// a small machine. `chunk_millions` (default 16) sets the in-memory run size (16M triples
/// ≈ 192 MB of ids); lower it to cap memory further. Query the result with `query-mmap`.
///
/// SPARQ_DICT_SPILL=1 additionally SPILLS the term dictionary (N-Triples only): peak RSS
/// is bounded by SPARQ_DICT_SPILL_BUDGET_MB (default: 1/4 of RAM) instead of growing with
/// distinct terms; SPARQ_DICT_SPILL_DISK_FLOOR_MB (default 1024) aborts before filling the
/// disk. Output is byte-identical. See research/external-dictionary.md.
fn cmd_build(args: &[String]) {
    let (path, format, dir) = match (args.get(2), args.get(3), args.get(4)) {
        (Some(p), Some(f), Some(d)) => (p.as_str(), f.as_str(), d.as_str()),
        _ => {
            // [OPUS-4.8] sq-vkz7: `SPARQ_BUILD_COMPRESSED=1` makes the external build emit
            // block-compressed (SPQCPRM1) perms in ONE pass — skipping a later `recompress`.
            eprintln!(
                "usage: sparq-cli build <file[.gz|.bz2]> <format> <dir> [chunk_millions]\n  \
                 (set SPARQ_BUILD_COMPRESSED=1 to write block-compressed perms directly, no recompress pass)"
            );
            std::process::exit(2);
        }
    };
    // [OPUS-4.8] `build` calls Graph::build_external directly (not load_quiet), which shares
    // the same Turtle catch-all — so reject an unknown format here too (bug sq-q50l). HDT is
    // not a build target (build streams text formats), so it is excluded from the accepted set.
    if !is_known_format(format) || format == "hdt" {
        die_unknown_format(format);
    }
    let chunk = args.get(5).and_then(|s| s.parse::<usize>().ok()).unwrap_or(16) * 1_000_000;

    let file = std::fs::File::open(path).unwrap_or_else(|e| {
        eprintln!("open {path}: {e}");
        std::process::exit(1);
    });
    // `+ Send` so the build can run decompression on its own (overlapping) thread.
    let decoded: Box<dyn Read + Send> = if path.ends_with(".bz2") {
        Box::new(bzip2::read::MultiBzDecoder::new(file))
    } else if path.ends_with(".gz") {
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else if path.ends_with(".zst") || path.ends_with(".zstd") {
        // zstd decompresses ~12x faster than bzip2, so it stops being the ingestion
        // bottleneck; recompress a .bz2 source once with `zstd -9 -T0` to use it.
        // [GPT-5.6] (sq-fo528) Accept frames produced with zstd's large-window mode.
        let mut decoder = zstd::stream::read::Decoder::new(file).unwrap_or_else(|e| {
            eprintln!("zstd decode {path}: {e}");
            std::process::exit(1);
        });
        decoder.window_log_max(31).unwrap_or_else(|e| {
            eprintln!("zstd decode {path}: {e}");
            std::process::exit(1);
        });
        Box::new(decoder)
    } else {
        Box::new(file)
    };
    let reader = std::io::BufReader::with_capacity(1 << 22, decoded);

    let t = Instant::now();
    // [OPUS-4.8] (sq-5atq) Quad formats (N-Quads / TriG) build OUT-OF-CORE *with* their named
    // graphs via `build_external_quads` (partition-by-graph + per-graph external sort, same
    // on-disk layout `save_named` emits) — a default-graph-only `build_external` would silently
    // FLATTEN every quad into the default graph, losing the dataset's named graphs (the PSS
    // shape). Triple formats keep the existing single-graph external path unchanged.
    let build_result = match format {
        "nquads" | "n-quads" | "trig" | "application/trig" => {
            sparq_core::Graph::build_external_quads(reader, format, std::path::Path::new(dir), chunk)
        }
        _ => sparq_core::Graph::build_external(reader, format, std::path::Path::new(dir), chunk),
    };
    build_result.unwrap_or_else(|e| {
        eprintln!("build error: {e}");
        std::process::exit(1);
    });
    eprintln!(
        "built on-disk indexes in {dir} in {:.1}s (external-memory, {}M-triple runs)",
        t.elapsed().as_secs_f64(),
        chunk / 1_000_000,
    );
}

/// `probe-compress <perm-file>` — MEASURE-FIRST probe (no engine change): how small can a
/// sorted permutation index get? Reports raw vs a lexicographic delta + LEB128-varint
/// encoding (the natural columnar scheme for sorted triples) vs gzip, in bytes/triple, so
/// we can decide whether a block-compressed backend is worth building before building it.
fn cmd_probe_compress(args: &[String]) {
    let path = match args.get(2) {
        Some(p) => p.as_str(),
        None => {
            eprintln!("usage: sparq-cli probe-compress <perm-file>");
            std::process::exit(2);
        }
    };
    let file = std::fs::File::open(path).unwrap_or_else(|e| {
        eprintln!("open {path}: {e}");
        std::process::exit(1);
    });
    // SAFETY: read-only mmap of a file held open for the call.
    let map = unsafe { memmap2::Mmap::map(&file) }.unwrap();
    let n = map.len() / 12;
    // SAFETY: a permutation file is a whole number of [u32;3] rows.
    let rows: &[[u32; 3]] = unsafe { std::slice::from_raw_parts(map.as_ptr().cast::<[u32; 3]>(), n) };
    if n == 0 {
        eprintln!("empty permutation");
        return;
    }

    // LEB128 byte length of a u64.
    let vlen = |mut x: u64| -> usize {
        let mut b = 1;
        while x >= 0x80 {
            x >>= 7;
            b += 1;
        }
        b
    };

    // Lexicographic delta: per row emit 3 varints (d0, x1, x2). When the higher column
    // changes the lower one is absolute; otherwise it is a (non-negative) delta. Exactly
    // decodable from the previous row — the standard sorted-triple columnar encoding.
    let t = Instant::now();
    let mut delta_bytes = 0usize;
    let mut prev = [0u32; 3];
    for &r in rows {
        if r[0] != prev[0] {
            delta_bytes += vlen((r[0] - prev[0]) as u64) + vlen(r[1] as u64) + vlen(r[2] as u64);
        } else if r[1] != prev[1] {
            delta_bytes += 1 + vlen((r[1] - prev[1]) as u64) + vlen(r[2] as u64);
        } else {
            delta_bytes += 1 + 1 + vlen(r[2].wrapping_sub(prev[2]) as u64);
        }
        prev = r;
    }
    let dscan = t.elapsed().as_secs_f64();

    // Block-directory overhead for random access: one (12-byte key + 8-byte offset) entry
    // per block of `B` triples.
    let block_dir = |b: usize| -> f64 { (n.div_ceil(b) * 20) as f64 / n as f64 };

    println!("permutation: {path}");
    println!("  triples           : {n}");
    println!("  raw               : {:>6.2} B/triple ({:.2} GB)", 12.0, map.len() as f64 / 1e9);
    println!(
        "  delta+varint      : {:>6.2} B/triple ({:.2} GB, {:.0}% of raw)  [{:.2} M/s decode-cost scan]",
        delta_bytes as f64 / n as f64,
        delta_bytes as f64 / 1e9,
        100.0 * delta_bytes as f64 / map.len() as f64,
        n as f64 / 1e6 / dscan,
    );
    println!(
        "  + block dir (B=128): +{:.2} B/triple  => {:.2} B/triple usable random-access",
        block_dir(128),
        delta_bytes as f64 / n as f64 + block_dir(128),
    );

    // gzip the raw bytes as a general-purpose ceiling — only for smaller files (it is slow).
    if map.len() <= 200_000_000 {
        use std::io::Write;
        let tg = Instant::now();
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&map).unwrap();
        let gz = enc.finish().unwrap().len();
        println!(
            "  gzip(raw)         : {:>6.2} B/triple ({:.0}% of raw)  [{:.2} M/s]",
            gz as f64 / n as f64,
            100.0 * gz as f64 / map.len() as f64,
            n as f64 / 1e6 / tg.elapsed().as_secs_f64(),
        );
    } else {
        println!("  gzip(raw)         : skipped (>200 MB; delta+varint is the fast, random-accessible scheme anyway)");
    }
}

/// `compare-compress <data-file> <format> [<sparql>]` — load a dataset both raw and
/// block-compressed, report the in-RAM index footprint of each, and (if a query is given)
/// the query latency of each — the memory-vs-decode tradeoff that decides the browser
/// storage mode.
fn cmd_compare_compress(args: &[String]) {
    let (path, format) = match (args.get(2), args.get(3)) {
        (Some(p), Some(f)) => (p.as_str(), f.as_str()),
        _ => {
            eprintln!("usage: sparq-cli compare-compress <data-file> <format> [<sparql>]");
            std::process::exit(2);
        }
    };
    let sparql = args.get(4).map(String::as_str);

    let raw = load(path, format);
    let raw_heap = raw.heap_bytes();
    let raw_store = raw.store.heap_bytes();
    // Re-encode the same store compressed (keeps the dict + numeric cache).
    let t = Instant::now();
    let cmp = raw.into_compressed();
    let enc_s = t.elapsed().as_secs_f64();
    let cmp_heap = cmp.heap_bytes();
    let cmp_store = cmp.store.heap_bytes();

    println!("=== index footprint (in-RAM) ===");
    println!("  triples            : {}", cmp.len());
    println!("  raw   perms        : {:>7.2} MB ({:.1} B/triple)", raw_store as f64 / 1e6, raw_store as f64 / cmp.len().max(1) as f64);
    println!(
        "  compressed perms   : {:>7.2} MB ({:.1} B/triple, {:.0}% of raw)  [encoded in {:.2}s]",
        cmp_store as f64 / 1e6,
        cmp_store as f64 / cmp.len().max(1) as f64,
        100.0 * cmp_store as f64 / raw_store.max(1) as f64,
        enc_s,
    );
    println!(
        "  total graph (perms+dict+numerics): raw {:.2} GB -> compressed {:.2} GB",
        raw_heap as f64 / 1e9,
        cmp_heap as f64 / 1e9,
    );

    if let Some(q) = sparql {
        // Materialise to SPARQL JSON — the heaviest path, which actually scans + (for the
        // compressed store) decodes the blocks the query touches.
        println!("\n=== query latency, MATERIALISED to JSON (min of 5) ===");
        let bench = |g: &sparq_core::Graph| -> (usize, f64) {
            let mut best = f64::MAX;
            let mut len = 0;
            for _ in 0..5 {
                let t = Instant::now();
                len = sparq_engine::query_json(g, q).map(|s| s.len()).unwrap_or(0);
                best = best.min(t.elapsed().as_secs_f64() * 1e3);
            }
            (len, best)
        };
        let (rn, rt) = bench(&raw_reload(path, format));
        let (cn, ct) = bench(&cmp);
        println!("  raw        : {rn} bytes of JSON in {rt:.3} ms");
        println!("  compressed : {cn} bytes of JSON in {ct:.3} ms  ({:.2}x raw)", ct / rt);
        assert_eq!(rn, cn, "compressed returned different JSON length!");
    }
}

/// Reloads a fresh raw graph (the original was consumed by `into_compressed`).
fn raw_reload(path: &str, format: &str) -> sparq_core::Graph {
    let text = std::fs::read_to_string(path).unwrap();
    sparq_core::Graph::load_str(&text, format).unwrap()
}

/// Parses `path`, optionally materializes the entailed closure for `profile` (opt-in
/// reasoning), then builds the graph. The reasoning step runs between parse and index-build
/// (the `parse_to_triples` → `from_parts` seam), so the default no-`--reason` path is
/// untouched. Reports the closure expansion.
fn load_reasoned(path: &str, format: &str, profile: sparq_reason::Profile) -> sparq_core::Graph {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error reading {path}: {e}");
        std::process::exit(1);
    });
    let (mut dict, mut triples) = sparq_core::Graph::parse_to_triples(&text, format).unwrap_or_else(|e| {
        eprintln!("parse error: {e}");
        std::process::exit(1);
    });
    let base = triples.len();
    let t = Instant::now();
    let added = sparq_reason::materialize(profile, &mut dict, &mut triples);
    eprintln!(
        "reasoned [{:?}]: {base} -> {} triples (+{added} entailed) in {:.3}s",
        profile,
        triples.len(),
        t.elapsed().as_secs_f64()
    );
    // OWL: report any detected inconsistency (cax-dw/cls-com/eq-diff/cls-nothing clashes).
    if profile == sparq_reason::Profile::OwlRl {
        let clashes = sparq_reason::inconsistencies(&dict, &triples);
        if !clashes.is_empty() {
            eprintln!("INCONSISTENT — {} clash(es) detected:", clashes.len());
            for c in &clashes {
                eprintln!("  - {c}");
            }
        }
    }
    // [FABLE-5] (sq-7d3dj.32.2.1) The reasoned load path also honours `SPARQ_STORE_PROFILE`, so
    // `reason` / `query --reason` inherit the store profile uniformly with the non-reasoned path.
    apply_store_profile(sparq_core::Graph::from_parts(dict, triples), store_profile_from_env())
}

/// Pull an optional `--reason <profile>` flag (`rdfs` | `owl` | `n3` | `el` |
/// `datalog:<rules.dlog>`) out of the argument list.
fn reason_flag(args: &[String]) -> Option<String> {
    let i = args.iter().position(|a| a == "--reason")?;
    Some(
        args.get(i + 1)
            .unwrap_or_else(|| {
                eprintln!("--reason needs a profile (rdfs | owl | n3 | el | datalog:<rules.dlog>)");
                std::process::exit(2);
            })
            .clone(),
    )
}

/// Load a graph applying the named reasoning profile. `rdfs`/`owl` materialize over the
/// parsed triples; `n3` parses the file as Notation3 (rules + facts) and forward-chains;
/// `el` runs the OWL 2 EL classifier (opt-in `el` feature — see `load_el_classified`, a code
/// span rather than an intra-doc link so this always-compiled comment stays clean in a build
/// without the feature); `datalog:<rules.dlog>` runs a stratified-Datalog program over the
/// parsed triples (opt-in `datalog` feature — see `load_datalog`, a code span for the same
/// reason).
fn load_with_reasoning(path: &str, format: &str, profile: &str) -> sparq_core::Graph {
    if profile.eq_ignore_ascii_case("n3") {
        return load_n3(path);
    }
    // [SONNET-4.6] (sq-p4zci, design record research/stratified-datalog-rules.md §6 item 6)
    // `datalog:<rules.dlog>` is the ONLY `--reason` value that carries an argument: a Datalog
    // program is user-supplied rules, not a fixed profile, so the rules file is part of the
    // flag. Intercepted before the RL/RDFS profile parse for the same reason as `el` — it is a
    // different rule language in a different module, and there is no profile to fall back to.
    if let Some(rules) = datalog_rules_arg(profile) {
        #[cfg(feature = "datalog")]
        return load_datalog(path, format, rules);
        #[cfg(not(feature = "datalog"))]
        {
            eprintln!(
                "cannot run the datalog rules in {}: reasoning profile 'datalog' needs the \
                 opt-in `datalog` cargo feature (cargo run -p sparq-cli --features datalog -- …); \
                 there is no fall-back profile — RDFS/OWL-RL are monotone and cannot express \
                 negation as failure or aggregation",
                rules
            );
            std::process::exit(2);
        }
    }
    // [OPUS-5] (sq-2ch27) `el` is NOT a `sparq_reason::Profile` — EL classification is a
    // different algorithm family in a different crate, so it is intercepted before the RL/RDFS
    // profile parse. Without the opt-in feature this is a hard exit-2 error naming the feature:
    // falling back to `owl` would silently hand back an INCOMPLETE class hierarchy.
    if profile.eq_ignore_ascii_case("el") {
        #[cfg(feature = "el")]
        return load_el_classified(path, format);
        #[cfg(not(feature = "el"))]
        {
            eprintln!(
                "reasoning profile 'el' needs the opt-in `el` cargo feature \
                 (cargo run -p sparq-cli --features el -- …); refusing to fall back to \
                 'owl', which is sound but INCOMPLETE for class classification"
            );
            std::process::exit(2);
        }
    }
    let prof = sparq_reason::Profile::parse(profile).unwrap_or_else(|| {
        eprintln!(
            "unknown reasoning profile '{profile}' (known: rdfs | owl | n3 | el | datalog:<rules.dlog>)"
        );
        std::process::exit(2);
    });
    load_reasoned(path, format, prof)
}

/// [SONNET-4.6] (sq-p4zci) Recognise the `datalog:<rules.dlog>` reasoning profile and return the
/// rules-file path. A bare `datalog` (no rules file) is a hard exit-2 error naming the syntax —
/// there is no default program to guess, and falling through to the profile parser would report
/// the far less useful "unknown reasoning profile".
///
/// Split on the FIRST `:` only, so a rules path may itself contain colons.
fn datalog_rules_arg(profile: &str) -> Option<&str> {
    if profile.eq_ignore_ascii_case("datalog") {
        eprintln!("reasoning profile 'datalog' needs a rules file: --reason datalog:<rules.dlog>");
        std::process::exit(2);
    }
    let (head, rules) = profile.split_once(':')?;
    if !head.eq_ignore_ascii_case("datalog") {
        return None;
    }
    if rules.is_empty() {
        eprintln!("reasoning profile 'datalog' needs a rules file: --reason datalog:<rules.dlog>");
        std::process::exit(2);
    }
    Some(rules)
}

/// [SONNET-4.6] (sq-p4zci, design record `research/stratified-datalog-rules.md` §6 item 6) The
/// `--reason datalog:<rules.dlog>` load path: parse the data file, parse the rule document,
/// check stratifiability, run the per-stratum fixpoint, and build the graph from the closure.
///
/// The closure is ordinary triples, so the subsequent query is plain BGP evaluation — the same
/// shape as the RDFS/OWL-RL materialization path, except the rules come from the user.
///
/// Reasoning runs at the `parse_to_triples` → `from_parts` seam (exactly like `load_reasoned`),
/// so the default no-`--reason` path is untouched. Both failure modes are LOUD (exit 1): a syntax
/// error or a construct outside the documented fragment is a parse error naming the offending
/// construct, and a program with a recursion cycle through `NOT`/`AGGREGATE` is rejected by the
/// stratification checker naming a predicate on the cycle — never a silently mis-evaluated
/// program. Both messages carry the RULES path, which the data-file path would not identify.
#[cfg(feature = "datalog")]
fn load_datalog(path: &str, format: &str, rules_path: &str) -> sparq_core::Graph {
    use sparq_reason::datalog;
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error reading {}: {}", path, e);
        std::process::exit(1);
    });
    let rules_src = std::fs::read_to_string(rules_path).unwrap_or_else(|e| {
        eprintln!("error reading datalog rules {}: {}", rules_path, e);
        std::process::exit(1);
    });
    let (mut dict, triples) = sparq_core::Graph::parse_to_triples(&text, format).unwrap_or_else(|e| {
        eprintln!("parse error: {}", e);
        std::process::exit(1);
    });
    let program = datalog::parse_program(&mut dict, &rules_src).unwrap_or_else(|e| {
        eprintln!("datalog rule error in {}: {}", rules_path, e);
        std::process::exit(1);
    });
    // Check stratifiability BEFORE evaluating so a non-stratifiable program is rejected by the
    // checker rather than after the data has been walked; `n_strata` is also what we report.
    // (`eval` re-checks — the duplicate is over the RULES, not the data, so it is negligible.)
    let strat = datalog::stratify(&dict, &program).unwrap_or_else(|e| {
        eprintln!("datalog stratification error in {}: {}", rules_path, e);
        std::process::exit(1);
    });
    // `eval` seeds its fact store with every input fact and only ever inserts, so the closure is
    // a de-duplicated SUPERSET of the parsed facts. The derived count is therefore exact against
    // the DISTINCT input (and the subtraction below cannot underflow) — `triples` may carry
    // duplicates the closure collapses, and subtracting the raw parsed length would under-report
    // the derivations by that many.
    let distinct_in: std::collections::HashSet<[sparq_core::dict::Id; 3]> = triples.iter().copied().collect();
    let base = distinct_in.len();
    let t = Instant::now();
    let closure = datalog::eval(&mut dict, &triples, &program).unwrap_or_else(|e| {
        eprintln!("datalog reasoning error: {}", e);
        std::process::exit(1);
    });
    eprintln!(
        "reasoned [datalog {}]: {} rule(s) in {} stratum/strata; {} -> {} distinct triples (+{} derived) in {:.3}s",
        rules_path,
        program.n_rules(),
        strat.n_strata(),
        base,
        closure.len(),
        closure.len() - base,
        t.elapsed().as_secs_f64()
    );
    // Honour `SPARQ_STORE_PROFILE` on the datalog path too (see `load_reasoned`).
    apply_store_profile(sparq_core::Graph::from_parts(dict, closure), store_profile_from_env())
}

/// Parse a Notation3 document (facts + `{…}=>{…}` rules), run the rule closure, build a graph.
fn load_n3(path: &str) -> sparq_core::Graph {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error reading {path}: {e}");
        std::process::exit(1);
    });
    let mut dict = sparq_core::dict::Dict::new();
    let t = Instant::now();
    let triples = sparq_reason::reason_n3(&mut dict, &text).unwrap_or_else(|e| {
        eprintln!("n3 reasoning error: {e}");
        std::process::exit(1);
    });
    eprintln!("reasoned [N3]: {} ground triples in closure in {:.3}s", triples.len(), t.elapsed().as_secs_f64());
    // [FABLE-5] (sq-7d3dj.32.2.1) Honour `SPARQ_STORE_PROFILE` on the N3 path too (see `load_reasoned`).
    apply_store_profile(sparq_core::Graph::from_parts(dict, triples), store_profile_from_env())
}

/// [OPUS-5] (sq-2ch27, Phase E6) Parse `path` and materialize the OWL 2 EL subsumption lattice
/// into the parsed triples via `sparq_reason_el::classify_graph`, returning the `(Dict, triples)`
/// pair plus the classifier's [`sparq_reason_el::Report`]. Shared by the `--reason el` load path
/// and the `classify` subcommand so the two cannot drift.
///
/// SCOPE: the CLI pulls `sparq-reason-el` with `rbox` but WITHOUT `cdomain`, so the lattice is
/// complete for the E1+E2 fragment (CR1–CR6 class saturation + the CR10/CR11 role automaton),
/// NOT for OWL 2 EL as a whole — concrete-domain axioms are deferred, not applied.
///
/// Everything the classifier could NOT reason over is reported on stderr rather than swallowed:
/// EL is SOUND but complete only for the fragment it recognises, so a non-zero `skipped_axioms`
/// (or a non-regular RBox) means the emitted lattice may be incomplete for those axioms.
#[cfg(feature = "el")]
fn el_classify_parts(
    path: &str,
    format: &str,
) -> (sparq_core::dict::Dict, Vec<[sparq_core::dict::Id; 3]>, sparq_reason_el::Report) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error reading {path}: {e}");
        std::process::exit(1);
    });
    let (mut dict, mut triples) = sparq_core::Graph::parse_to_triples(&text, format).unwrap_or_else(|e| {
        eprintln!("parse error: {e}");
        std::process::exit(1);
    });
    let base = triples.len();
    let t = Instant::now();
    let report = sparq_reason_el::classify_graph(&mut dict, &mut triples);
    eprintln!(
        "classified [OWL 2 EL]: {base} -> {} triples (+{} rdfs:subClassOf, +{} rdfs:subPropertyOf) in {:.3}s",
        triples.len(),
        report.emitted_subsumptions,
        report.emitted_role_subsumptions,
        t.elapsed().as_secs_f64()
    );
    if report.skipped_axioms > 0 {
        eprintln!(
            "  NOTE: {} class axiom(s) used a construct this build does NOT reason over and were NOT applied\n\
             \x20       — either outside EL entirely (union / complement / allValuesFrom / cardinality /\n\
             \x20         multi-individual oneOf, which need a different calculus), or IN OWL 2 EL but\n\
             \x20         deferred here: this CLI enables `rbox` and NOT `cdomain`, so every concrete-domain\n\
             \x20         axiom (faceted owl:onDatatype/owl:withRestrictions, literal owl:hasValue/owl:oneOf)\n\
             \x20         is skipped. The lattice may be INCOMPLETE for those axioms.",
            report.skipped_axioms
        );
    }
    if report.rbox_non_regular {
        eprintln!(
            "  NOTE: the told RBox is NOT regular (a property-chain cycle, which OWL 2's global\n\
             \x20       restrictions forbid). Every derived subsumption is still sound, but the EL+\n\
             \x20       completeness argument assumes regularity — this classification may be INCOMPLETE."
        );
    }
    if report.unsatisfiable_classes > 0 {
        eprintln!(
            "  {} named class(es) are UNSATISFIABLE (⊑ owl:Nothing).",
            report.unsatisfiable_classes
        );
    }
    if report.thing_unsatisfiable {
        eprintln!("  owl:Thing ⊑ owl:Nothing — the TBox forces EVERY class to be empty.");
    }
    (dict, triples, report)
}

/// [OPUS-5] (sq-2ch27, Phase E6) The `--reason el` load path: classify, then build the graph
/// from the lattice-augmented triples. The derived `rdfs:subClassOf` / `rdfs:subPropertyOf`
/// edges are ordinary triples, so the subsequent query is plain BGP evaluation — exactly like
/// the RL `scm-*` output, just complete for the E1+E2 fragment (see `el_classify_parts` for the
/// `cdomain` deferral that keeps this short of full OWL 2 EL).
#[cfg(feature = "el")]
fn load_el_classified(path: &str, format: &str) -> sparq_core::Graph {
    let (dict, triples, _) = el_classify_parts(path, format);
    // Honour `SPARQ_STORE_PROFILE` on the EL path too (see `load_reasoned`).
    apply_store_profile(sparq_core::Graph::from_parts(dict, triples), store_profile_from_env())
}

/// [OPUS-5] (sq-2ch27, Phase E6) `classify <data-file> <format> [out.nt]` — run the OWL 2 EL
/// classifier and print the classification REPORT as `name<TAB>value` lines on stdout (the
/// `memstat` convention); with `out.nt`, also write the lattice-augmented graph as N-Triples.
///
/// This is the surface for the capability OWL 2 RL cannot reach. RL's completeness theorem is
/// scoped to ASSERTIONAL conclusions, so `--reason owl` silently omits derivable class
/// subsumptions — e.g. `B ⊑ D` from `B ⊑ C`, `B ⊑ E`, `C ⊓ E ⊑ D` (RL has no TBox
/// conjunction-composition rule), pinned by `tests/el_cli.rs::el_derives_what_rl_cannot`.
#[cfg(feature = "el")]
fn cmd_classify(args: &[String]) {
    let (path, format) = match (args.get(2), args.get(3)) {
        (Some(p), Some(f)) => (p.as_str(), f.as_str()),
        _ => {
            eprintln!("usage: sparq-cli classify <data-file> <format> [out.nt]");
            std::process::exit(2);
        }
    };
    let (dict, triples, report) = el_classify_parts(path, format);
    println!("triples\t{}", triples.len());
    println!("named_classes\t{}", report.named_classes);
    println!("emitted_subclassof\t{}", report.emitted_subsumptions);
    println!("emitted_subpropertyof\t{}", report.emitted_role_subsumptions);
    println!("skipped_axioms\t{}", report.skipped_axioms);
    println!("unsatisfiable_classes\t{}", report.unsatisfiable_classes);
    println!("thing_unsatisfiable\t{}", report.thing_unsatisfiable);
    println!("rbox_non_regular\t{}", report.rbox_non_regular);
    if let Some(out) = args.get(4) {
        use std::io::Write;
        let mut w = std::io::BufWriter::new(std::fs::File::create(out).unwrap_or_else(|e| {
            eprintln!("create {out}: {e}");
            std::process::exit(1);
        }));
        for t in &triples {
            writeln!(w, "{} {} {} .", dict.term(t[0]), dict.term(t[1]), dict.term(t[2])).unwrap();
        }
        w.flush().unwrap();
        eprintln!("wrote classified graph to {out}");
    }
}

/// `reason <data-file> <format> <profile> [out.nt]` — materialize the entailed closure and
/// report the expansion; with `out.nt`, write the full closure as N-Triples.
fn cmd_reason(args: &[String]) {
    let (path, format, profile) = match (args.get(2), args.get(3), args.get(4)) {
        (Some(p), Some(f), Some(pr)) => (p.as_str(), f.as_str(), pr.as_str()),
        _ => {
            eprintln!(
                "usage: sparq-cli reason <data-file> <format> <rdfs|owl|n3|el|datalog:<rules.dlog>> [out.nt]"
            );
            std::process::exit(2);
        }
    };
    // N3 proof output (EYE --proof analogue): print each derivation step.
    if profile.eq_ignore_ascii_case("n3") && args.iter().any(|a| a == "--proof") {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("error reading {path}: {e}");
            std::process::exit(1);
        });
        let mut dict = sparq_core::dict::Dict::new();
        let (closure, proof) = sparq_reason::reason_n3_proof(&mut dict, &text).unwrap_or_else(|e| {
            eprintln!("n3 reasoning error: {e}");
            std::process::exit(1);
        });
        let t = |id| dict.term(id).to_string();
        println!("{} triples in closure; {} derivation step(s):", closure.len(), proof.len());
        for (i, step) in proof.iter().enumerate() {
            let c = step.conclusion;
            println!("  [{}] {} {} {} .", i + 1, t(c[0]), t(c[1]), t(c[2]));
            println!("      ⊢ by rule #{} from:", step.rule);
            for p in &step.premises {
                println!("        {} {} {} .", t(p[0]), t(p[1]), t(p[2]));
            }
        }
        return;
    }
    let g = load_with_reasoning(path, format, profile);
    println!("{} triples after {profile} reasoning", g.len());
    if let Some(out) = args.get(5) {
        use std::io::Write;
        let mut w = std::io::BufWriter::new(std::fs::File::create(out).unwrap_or_else(|e| {
            eprintln!("create {out}: {e}");
            std::process::exit(1);
        }));
        let scan = g.store.scan(&[None, None, None]);
        for r in scan.rows.iter() {
            let spo = scan.to_spo(r);
            writeln!(w, "{} {} {} .", g.dict.term(spo[0]), g.dict.term(spo[1]), g.dict.term(spo[2])).unwrap();
        }
        w.flush().unwrap();
        eprintln!("wrote closure to {out}");
    }
}

/// `query-mmap <dir> <sparql> [--format <out>] [--count]` — open a saved dataset with
/// memory-mapped indexes and run a query, printing its RESULTS. Reports load time + the
/// store self-estimate to stderr (≈0 GB heap for the mmap'd permutations — they live in the
/// OS page cache, not the process heap).
///
/// [OPUS-4.8] (sq-iwyy) Output is at PARITY with `query`: it shares the exact same
/// `emit_query_results` emission core, so SELECT prints bindings (default a readable table),
/// ASK prints a boolean, CONSTRUCT/DESCRIBE print N-Triples, `--format <table|tsv|csv|xml|
/// json|ntriples>` selects the SELECT/ASK serialisation, and `--count` restores the old
/// count-only line (`<n> solutions/triples in <ms>ms`). The only difference from `query` is
/// the data source: an mmap-backed `Graph::open` instead of an in-RAM `load`.
fn cmd_query_mmap(args: &[String]) {
    let (dir, sparql) = match (args.get(2), args.get(3)) {
        (Some(d), Some(q)) => (d.as_str(), q.as_str()),
        _ => {
            eprintln!("usage: sparq-cli query-mmap <dir> <sparql> [--format <table|tsv|csv|xml|json|ntriples>] [--count]");
            std::process::exit(2);
        }
    };
    // [OPUS-4.8] (sq-iwyy) Same flag surface as `query`: `--count` for the legacy count-only
    // line, `--format` for the SELECT/ASK serialisation (default table).
    let count_only = args.iter().any(|a| a == "--count");
    let out_fmt = out_format_flag(args);

    let t = Instant::now();
    let g = sparq_core::Graph::open(std::path::Path::new(dir)).unwrap_or_else(|e| {
        eprintln!("open error: {e}");
        std::process::exit(1);
    });
    eprintln!(
        "opened {} triples (indexes MEMORY-MAPPED) in {:.3}s | store-heap ~{:.2} GB (mmap'd perms not counted), dict ~{:.2} GB",
        g.len(),
        t.elapsed().as_secs_f64(),
        g.heap_bytes() as f64 / 1e9,
        g.dict.heap_bytes() as f64 / 1e9,
    );
    // The mmap-backed Graph borrows its on-disk permutations for the whole of `g`'s scope;
    // `emit_query_results` takes `&g` and finishes before `g` drops, so the borrow is sound.
    emit_query_results(&g, sparql, count_only, out_fmt);
}

/// [OPUS-4.8] Whether `format` is an RDF serialization this CLI accepts. Kept in lock-step
/// with the format arms in `load_quiet` / `cmd_build` and with `sparq_core::parse_to_triples`.
/// `parse_to_triples` falls back to Turtle for ANY unrecognised string, so the CLI must
/// gate on this set itself to honour the "unsupported format → non-zero exit" contract.
fn is_known_format(format: &str) -> bool {
    // [OPUS-4.8] (sq-oy1f.4) JSON-LD input is recognised in the DEFAULT build (the `jsonld`
    // feature is in the CLI default set — a maintainer-directed exception). Without the feature
    // (`--no-default-features`) the `oxjsonld` parser is not linked, so the JSON-LD tokens are
    // NOT "known" and a `jsonld` input format errors (exit 2) rather than mis-parsing as Turtle.
    #[cfg(feature = "jsonld")]
    if matches!(format, "jsonld" | "json-ld" | "application/ld+json") {
        return true;
    }
    matches!(
        format,
        "hdt"
            | "ntriples"
            | "n-triples"
            | "nt"
            | "application/n-triples"
            | "nquads"
            | "n-quads"
            | "nq"
            | "application/n-quads"
            | "trig"
            | "application/trig"
            | "turtle"
            | "ttl"
            | "text/turtle"
            | "application/turtle"
    )
}

/// [OPUS-4.8] Report an unknown `--format` value and exit 2 (usage error).
fn die_unknown_format(format: &str) -> ! {
    // [OPUS-4.8] (sq-oy1f.4) `jsonld` is named in the default build (the `jsonld` feature is in
    // the CLI default set); a `--no-default-features` build omits it from the list.
    eprintln!(
        "unknown format '{}' (known: turtle | ntriples | nquads | trig{}{})",
        format,
        if cfg!(feature = "jsonld") { " | jsonld" } else { "" },
        if cfg!(feature = "hdt") { " | hdt" } else { "" }
    );
    std::process::exit(2);
}

/// Opens a (possibly compressed) RDF document as a streaming reader. `.gz`/`.bz2`/`.zst[d]` are
/// decompressed transparently on the fly — the decompressed bytes are never all held at once.
fn open_reader(path: &str) -> std::io::Result<Box<dyn std::io::Read + Send>> {
    let file = std::fs::File::open(path)?;
    Ok(if path.ends_with(".gz") {
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else if path.ends_with(".bz2") {
        Box::new(bzip2::read::MultiBzDecoder::new(file))
    } else if path.ends_with(".zst") || path.ends_with(".zstd") {
        // [GPT-5.6] (sq-fo528) Accept frames produced with zstd's large-window mode.
        let mut decoder = zstd::stream::read::Decoder::new(file)?;
        decoder.window_log_max(31)?;
        Box::new(decoder)
    } else {
        Box::new(file)
    })
}

#[cfg(test)]
mod open_reader_tests {
    use super::open_reader;
    use std::io::{Read, Write};

    /// [GPT-5.6] (sq-fo528) A streaming `--long=28` encoder advertises a 256 MiB
    /// window even for this tiny fixture. The default decoder ceiling rejects the
    /// frame, so this witnesses the raised ceiling without a large test artifact.
    #[test]
    fn large_window_zstd_round_trips_byte_identically() {
        let source = b"<http://ex/s> <http://ex/p> <http://ex/o> .\n";
        let path = std::env::temp_dir().join(format!(
            "sparq-cli-zstd-long-window-{}.nt.zst",
            std::process::id()
        ));
        {
            let mut encoder =
                zstd::stream::write::Encoder::new(std::fs::File::create(&path).unwrap(), 3)
                    .unwrap();
            encoder.window_log(28).unwrap();
            encoder.write_all(source).unwrap();
            encoder.finish().unwrap();
        }

        let mut decoded = Vec::new();
        open_reader(path.to_str().unwrap())
            .unwrap()
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, source);
        let _ = std::fs::remove_file(path);
    }
}

/// [FABLE-5] (sq-7d3dj.32.2.1) The in-RAM store profile selected by `SPARQ_STORE_PROFILE`.
/// `Raw` is the default six-permutation layout; `Compressed` re-encodes into the
/// block-compressed in-RAM mode (`Graph::into_compressed`). Read once per load from the env.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StoreProfile {
    Raw,
    Compressed,
}

/// [FABLE-5] (sq-7d3dj.32.2.1) Resolve the `SPARQ_STORE_PROFILE` env var into a [`StoreProfile`].
/// Contract: unset or `raw` → [`StoreProfile::Raw`] (byte-identical current behaviour);
/// `compressed` → [`StoreProfile::Compressed`]; ANY other value → hard error, exit 2 —
/// fail-closed, no silent typo fall-through (a mistyped profile must never quietly select raw).
/// The value is matched case-insensitively and after trimming surrounding whitespace.
fn store_profile_from_env() -> StoreProfile {
    match std::env::var("SPARQ_STORE_PROFILE") {
        Err(_) => StoreProfile::Raw,
        Ok(v) => {
            let norm = v.trim().to_ascii_lowercase();
            match norm.as_str() {
                "" | "raw" => StoreProfile::Raw,
                "compressed" => StoreProfile::Compressed,
                _ => {
                    eprintln!(
                        "error: SPARQ_STORE_PROFILE={:?} is not a valid store profile (known: raw | compressed)",
                        v
                    );
                    std::process::exit(2);
                }
            }
        }
    }
}

/// [FABLE-5] (sq-7d3dj.32.2.1) Apply a resolved store [`StoreProfile`] to a freshly-loaded graph.
/// `Raw` returns `g` untouched (the default path is byte-identical to before this hook existed);
/// `Compressed` re-encodes it via `Graph::into_compressed()`. Honest scope: this reduces
/// steady-state resident footprint only — the load-time peak RSS still includes the raw perm
/// build `into_compressed` encodes from (see `research/compressed-memory-profile.md` §2).
fn apply_store_profile(g: sparq_core::Graph, profile: StoreProfile) -> sparq_core::Graph {
    match profile {
        StoreProfile::Raw => g,
        StoreProfile::Compressed => g.into_compressed(),
    }
}

/// Load without the timing/size summary line — used by the scaling harness, which loads many
/// times and prints its own table. Honours `SPARQ_STORE_PROFILE` (see [`store_profile_from_env`])
/// so `query` / `bench` / `scaling` inherit the profile uniformly. The raw-format routing lives
/// in `load_quiet_raw`; this wrapper adds only the post-load profile encoding.
fn load_quiet(path: &str, format: &str) -> sparq_core::Graph {
    apply_store_profile(load_quiet_raw(path, format), store_profile_from_env())
}

/// [FABLE-5] (sq-7d3dj.32.2.1) The unconditional raw load — format routing + parse + index build,
/// with NO store-profile applied. Byte-identical to the pre-profile `load_quiet` body, so callers
/// that manage their own profile decision (e.g. `memstat`, which has an explicit positional) use
/// this to avoid a double application.
fn load_quiet_raw(path: &str, format: &str) -> sparq_core::Graph {
    let die = |e: String| -> ! {
        eprintln!("error loading {path}: {e}");
        std::process::exit(1);
    };
    // [OPUS-4.8] Reject an unknown format argument up-front (exit 2 = usage error).
    // sparq-core's `parse_to_triples` treats ANY unrecognised format string as Turtle
    // (a catch-all `_ => Turtle` arm), so without this guard a typo'd / unsupported
    // format value would SILENTLY parse the input as Turtle and exit 0 — the opposite
    // of the "unsupported format → non-zero exit" CLI contract (bug sq-q50l).
    let ext_routed = path.ends_with(".hdt") || path.ends_with(".hdt.gz");
    if !is_known_format(format) && !ext_routed {
        die_unknown_format(format);
    }
    // HDT archives — `hdt` as the format argument, or a `.hdt`/`.hdt.gz` file
    // extension — route through sparq-hdt (gzip is sniffed by magic bytes
    // there, so a mislabelled extension still loads). Opt-in cargo feature:
    // the decode stack stays out of the default build (see Cargo.toml).
    if format == "hdt" || path.ends_with(".hdt") || path.ends_with(".hdt.gz") {
        #[cfg(feature = "hdt")]
        return sparq_hdt::load(path).unwrap_or_else(|e| die(e.to_string()));
        #[cfg(not(feature = "hdt"))]
        die("HDT support is not compiled into this binary: rebuild with `cargo build -p sparq-cli --features hdt`".to_string());
    }
    // N-Triples streams block-by-block (parallel parse, no full decompressed copy in RAM); other
    // formats need the whole document buffered for the parallel statement-splitter.
    if matches!(format, "ntriples" | "n-triples") {
        let reader = open_reader(path).unwrap_or_else(|e| die(e.to_string()));
        sparq_core::Graph::load_reader_parallel(reader, format).unwrap_or_else(|e| die(e))
    } else {
        use std::io::Read;
        let mut text = String::new();
        open_reader(path).and_then(|mut r| r.read_to_string(&mut text)).unwrap_or_else(|e| die(e.to_string()));
        // N-Quads / TriG (and — [OPUS-4.8] sq-oy1f.4 — JSON-LD, whose `@graph` carries named
        // graphs) load as a DATASET so GRAPH queries and full-dataset re-serialisation (`dump …
        // jsonld`) see the named graphs instead of folding them into the default graph.
        #[cfg(feature = "jsonld")]
        let dataset = matches!(
            format,
            "nquads" | "n-quads" | "trig" | "application/trig"
                | "jsonld" | "json-ld" | "application/ld+json"
        );
        #[cfg(not(feature = "jsonld"))]
        let dataset = matches!(format, "nquads" | "n-quads" | "trig" | "application/trig");
        if dataset {
            sparq_core::Graph::load_dataset(&text, format).unwrap_or_else(|e| die(e))
        } else {
            sparq_core::Graph::load_str(&text, format).unwrap_or_else(|e| die(e))
        }
    }
}

fn load(path: &str, format: &str) -> sparq_core::Graph {
    let t = Instant::now();
    let g = load_quiet(path, format);
    let secs = t.elapsed().as_secs_f64();
    let heap = g.heap_bytes();
    let dict = g.dict.heap_bytes();
    eprintln!(
        "loaded {} triples in {:.3}s ({:.2} M/s) | store ~{:.2} GB ({:.0} B/triple), dict ~{:.2} GB ({} terms, {:.0} B/term)",
        g.len(),
        secs,
        g.len() as f64 / 1e6 / secs,
        heap as f64 / 1e9,
        heap as f64 / g.len().max(1) as f64,
        dict as f64 / 1e9,
        g.dict.len(),
        dict as f64 / g.dict.len().max(1) as f64,
    );
    g
}

/// [FABLE-5] (sq-7d3dj.32) `memstat <data-file> <format> [compressed]` — load a document and print a
/// DETERMINISTIC memory-composition breakdown as `name<TAB>value` lines on stdout, plus the
/// process RSS counters. This is the at-scale extension of the CI `store_bytes_per_triple`
/// metric (scripts/ci-bench.sh greps the same `heap_bytes()` self-accounting off the load
/// summary line): here the total is DECOMPOSED into its components — dictionary vs the six
/// permutation indexes vs the numeric/temporal caches — so a bytes-per-triple figure comes
/// with its explanation, and the kernel's `VmRSS`/`VmHWM` (post-load resident + peak during
/// load, Linux `/proc/self/status`; 0 elsewhere) sit next to the self-accounted heap so
/// allocator/parse-transient overhead is visible rather than assumed. Consumed by
/// `scripts/bench/bytes-per-triple.sh` (bench id `bytes-per-triple`).
///
/// The trailing literal `compressed` (or `SPARQ_STORE_PROFILE=compressed`) re-encodes into the
/// memory-bound in-RAM mode (block-compressed permutations + blob dictionary,
/// `Graph::into_compressed`) before reporting — the same graph, the other end of the in-memory
/// footprint/latency trade, so the two framings come from one instrument. The reported `mode`
/// line says which. Because `memstat` reads the raw load (`load_quiet_raw`) and applies the
/// compression decision itself, the positional flag and the env are OR'd and applied exactly
/// once (no double `into_compressed`).
fn cmd_memstat(args: &[String]) {
    let (path, format) = match (args.get(2), args.get(3)) {
        (Some(p), Some(f)) => (p.as_str(), f.as_str()),
        _ => {
            eprintln!("usage: sparq-cli memstat <data-file> <format> [compressed]");
            std::process::exit(2);
        }
    };
    // The explicit positional `compressed` OR `SPARQ_STORE_PROFILE=compressed` (an unknown
    // profile value still fails closed via `store_profile_from_env`). Applied exactly once
    // over the raw load — `load_quiet_raw` never applies a profile itself.
    let compressed = args.get(4).map(String::as_str) == Some("compressed")
        || store_profile_from_env() == StoreProfile::Compressed;
    let t = Instant::now();
    let mut g = load_quiet_raw(path, format);
    if compressed {
        g = g.into_compressed();
    }
    let load_s = t.elapsed().as_secs_f64();

    let triples = g.len().max(1);
    let terms = g.dict.len().max(1);
    let heap_total = g.heap_bytes();
    let heap_dict = g.dict.heap_bytes();
    let heap_store = g.store.heap_bytes();
    // The remainder is the numerics + temporals literal-value caches (their accessors are
    // crate-private; the subtraction is exact because Graph::heap_bytes is the plain sum).
    let heap_caches = heap_total.saturating_sub(heap_dict + heap_store);
    let (vm_rss, vm_hwm) = proc_vm_bytes();

    println!("memstat_version\t1");
    println!("mode\t{}", if compressed { "compressed" } else { "raw" });
    println!("triples\t{}", g.len());
    println!("dict_terms\t{}", g.dict.len());
    println!("load_s\t{:.3}", load_s);
    println!("heap_total_bytes\t{}", heap_total);
    println!("heap_dict_bytes\t{}", heap_dict);
    println!("heap_store_bytes\t{}", heap_store);
    println!("heap_caches_bytes\t{}", heap_caches);
    println!("heap_b_per_triple\t{:.2}", heap_total as f64 / triples as f64);
    println!("store_b_per_triple\t{:.2}", heap_store as f64 / triples as f64);
    println!("dict_b_per_triple\t{:.2}", heap_dict as f64 / triples as f64);
    println!("caches_b_per_triple\t{:.2}", heap_caches as f64 / triples as f64);
    println!("dict_b_per_term\t{:.2}", heap_dict as f64 / terms as f64);
    println!("vm_rss_bytes\t{}", vm_rss);
    println!("vm_hwm_bytes\t{}", vm_hwm);
    println!("rss_b_per_triple\t{:.2}", vm_rss as f64 / triples as f64);
    println!("hwm_b_per_triple\t{:.2}", vm_hwm as f64 / triples as f64);
}

/// [FABLE-5] (sq-7d3dj.32) Read the process `VmRSS` / `VmHWM` (resident set + high-water mark) in
/// bytes from Linux `/proc/self/status`. Returns `(0, 0)` where the file is unavailable
/// (non-Linux), so `memstat`'s deterministic heap lines still work there.
fn proc_vm_bytes() -> (u64, u64) {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return (0, 0);
    };
    let field = |key: &str| -> u64 {
        status
            .lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(0, |kib| kib * 1024)
    };
    (field("VmRSS:"), field("VmHWM:"))
}

/// [OPUS-4.8] (sq-678h, sq-e3pj) `dump <file[.gz|.bz2|.zst]> <in-format> <out-format>` — load an
/// RDF document and re-serialize the whole graph (default + named graphs) into one of the writer
/// matrix formats and print it to stdout. `out-format` ∈ {turtle, trig, nquads, ntriples,
/// jsonld[-expanded|-flattened|-compacted]}. Turtle emits only the default graph (named graphs
/// need a dataset format); trig/nquads/jsonld emit the full dataset. JSON-LD defaults to the
/// expanded form; `jsonld-flattened` / `jsonld-compacted` select the other 1.1 document forms.
/// [OPUS-4.8] (sq-ixc3.3) `jsonld-pretty[-expanded|-flattened|-compacted]` emit the same
/// JSON-LD documents in an indented, multi-line shape (whitespace-only over the minified writer).
/// [OPUS-4.8] (sq-oy1f.5) `jsonld-compact` (+ `jsonld-compact-pretty`) emit the FULL W3C
/// JSON-LD 1.1 Compaction Algorithm against a caller `@context` supplied via `--context <file>`
/// (term definitions / `@vocab` / type-language-`@container` coercion / `@reverse` / aliasing),
/// not the prefix-only `jsonld-compacted` form. Behind the opt-in `serialize-rdf` cargo feature.
#[cfg(feature = "serialize-rdf")]
fn cmd_dump(args: &[String]) {
    let (path, in_fmt, out_fmt) = match (args.get(2), args.get(3), args.get(4)) {
        (Some(p), Some(i), Some(o)) => (p.as_str(), i.as_str(), o.as_str()),
        _ => {
            eprintln!("usage: sparq-cli dump <file[.gz|.bz2|.zst]> <in-format> <out-format> [--context <ctx.jsonld>]\n  out-format: turtle | turtle-pretty | trig | trig-pretty | nquads | ntriples | jsonld[-expanded|-flattened|-compacted] | jsonld-pretty[-expanded|-flattened|-compacted] | jsonld-compact[-pretty] (needs --context)");
            std::process::exit(2);
        }
    };
    // [OPUS-4.8] (sq-oy1f.5) `--context <file>`: the JSON-LD `@context` for full 1.1 Compaction.
    // Scanned from the args (the CLI is positional, not clap); only the `jsonld-compact` out-
    // formats consult it. A present `--context` with no value is a usage error.
    let context_path: Option<&str> = match args.iter().position(|a| a == "--context") {
        None => None,
        Some(i) => Some(args.get(i + 1).map(String::as_str).unwrap_or_else(|| {
            eprintln!("--context needs a value (a JSON-LD @context file)");
            std::process::exit(2);
        })),
    };
    let g = load_quiet(path, in_fmt);
    use sparq_engine::serialize::JsonLdForm;
    let serialized = match out_fmt {
        // [OPUS-4.8] (sq-oy1f.5) FULL W3C JSON-LD 1.1 Compaction against the `--context` file.
        "jsonld-compact" | "json-ld-compact" | "jsonld-compact-pretty" | "json-ld-compact-pretty" => {
            let ctx_path = context_path.unwrap_or_else(|| {
                eprintln!("out-format '{out_fmt}' needs a `@context`: pass --context <file.jsonld>");
                std::process::exit(2);
            });
            let ctx_text = std::fs::read_to_string(ctx_path).unwrap_or_else(|e| {
                eprintln!("error reading context file {ctx_path}: {e}");
                std::process::exit(1);
            });
            let ctx = sparq_engine::serialize::parse_context_json(&ctx_text).unwrap_or_else(|| {
                eprintln!("context file {ctx_path} is not a JSON object (a JSON-LD @context)");
                std::process::exit(1);
            });
            if out_fmt.ends_with("-pretty") {
                let opts = sparq_engine::serialize::JsonLdPrettyOptions::default();
                sparq_engine::serialize::graph_to_jsonld_compact_pretty(&g, &ctx, &opts)
            } else {
                sparq_engine::serialize::graph_to_jsonld_compact(&g, &ctx)
            }
        }
        "turtle" | "ttl" => sparq_engine::serialize::graph_to_turtle(&g),
        // [OPUS-4.8] (sq-ixc3.2) idiomatic, deterministic pretty Turtle / TriG.
        "turtle-pretty" | "ttl-pretty" => sparq_engine::serialize::graph_to_turtle_pretty(&g),
        "trig-pretty" => sparq_engine::serialize::graph_to_trig_pretty(&g),
        "trig" => sparq_engine::serialize::graph_to_trig(&g),
        "nquads" | "n-quads" => sparq_engine::serialize::graph_to_nquads(&g),
        "jsonld" | "json-ld" | "jsonld-expanded" => {
            sparq_engine::serialize::graph_to_jsonld(&g, JsonLdForm::Expanded)
        }
        "jsonld-flattened" => sparq_engine::serialize::graph_to_jsonld(&g, JsonLdForm::Flattened),
        "jsonld-compacted" => sparq_engine::serialize::graph_to_jsonld(&g, JsonLdForm::Compacted),
        // [OPUS-4.8] (sq-ixc3.3) pretty (indented) JSON-LD — whitespace-only over the minified
        // forms above. `jsonld-pretty` defaults to expanded, matching `jsonld`.
        "jsonld-pretty" | "json-ld-pretty" | "jsonld-pretty-expanded" => {
            sparq_engine::serialize::graph_to_jsonld_pretty(&g, JsonLdForm::Expanded)
        }
        "jsonld-pretty-flattened" => {
            sparq_engine::serialize::graph_to_jsonld_pretty(&g, JsonLdForm::Flattened)
        }
        "jsonld-pretty-compacted" => {
            sparq_engine::serialize::graph_to_jsonld_pretty(&g, JsonLdForm::Compacted)
        }
        // N-Triples stays on the always-on writer (the default graph as `s p o .` lines).
        "ntriples" | "n-triples" => {
            let triples: Vec<oxrdf::Triple> = g
                .iter_ids()
                .map(|[s, p, o]| {
                    let subject = match g.dict.term(s) {
                        oxrdf::Term::NamedNode(n) => oxrdf::NamedOrBlankNode::NamedNode(n),
                        oxrdf::Term::BlankNode(b) => oxrdf::NamedOrBlankNode::BlankNode(b),
                        other => {
                            eprintln!("corrupt store: non-IRI/blank subject {other}");
                            std::process::exit(1);
                        }
                    };
                    let predicate = match g.dict.term(p) {
                        oxrdf::Term::NamedNode(n) => n,
                        other => {
                            eprintln!("corrupt store: non-IRI predicate {other}");
                            std::process::exit(1);
                        }
                    };
                    oxrdf::Triple { subject, predicate, object: g.dict.term(o) }
                })
                .collect();
            sparq_engine::triples_to_ntriples(&triples)
        }
        other => {
            eprintln!("unknown out-format '{other}' (known: turtle | turtle-pretty | trig | trig-pretty | nquads | ntriples | jsonld[-expanded|-flattened|-compacted] | jsonld-pretty[-expanded|-flattened|-compacted] | jsonld-compact[-pretty] (needs --context))");
            std::process::exit(2);
        }
    };
    print!("{serialized}");
}

/// [FABLE-5] (sq-8ju74) `to-hdt <data-file[.gz|.bz2|.zst]> <format> <out.hdt[.gz|.zst|.bz2]>` —
/// load an RDF document through the shared load path (any ingestible format, `.hdt` itself
/// included) and EXPORT it as a standard-layout HDT v1.0 archive (FourSectionDictionary +
/// BitmapTriples, SPO) via `sparq_hdt::save` — the direct in-memory encoder, no temporary
/// N-Triples round-trip. The output container (`.hdt.gz` / `.hdt.zst` / `.hdt.bz2`, or a bare
/// `.hdt`) is chosen by the OUTPUT path's extension (the write side cannot sniff content that
/// does not exist yet). HDT carries a SINGLE default graph: when the loaded input has named
/// graphs (TriG / N-Quads / JSON-LD `@graph`) they are DROPPED from the archive, and a loud
/// warning with the dropped graph/triple counts goes to stderr — never silently. An RDF 1.2
/// quoted-triple term cannot be represented in standard HDT; `save` fails with a term error
/// (exit 1) rather than emitting a lossy archive. Behind the opt-in `hdt-write` cargo feature.
#[cfg(feature = "hdt-write")]
fn cmd_to_hdt(args: &[String]) {
    let (path, format, out) = match (args.get(2), args.get(3), args.get(4)) {
        (Some(p), Some(f), Some(o)) => (p.as_str(), f.as_str(), o.as_str()),
        _ => {
            eprintln!(
                "usage: sparq-cli to-hdt <data-file[.gz|.bz2|.zst]> <format> <out.hdt[.gz|.zst|.bz2]>\n  \
                 exports the loaded document as a standard HDT v1.0 archive (default graph only; \
                 output compression chosen by the output extension)"
            );
            std::process::exit(2);
        }
    };
    let g = load_quiet(path, format);
    // Honesty over silence: HDT has no place for named graphs, so a dataset input (TriG /
    // N-Quads / JSON-LD `@graph`) loses them in the archive. Count what is dropped and say so.
    let (mut named_graphs, mut named_triples) = (0usize, 0usize);
    g.for_named_graphs_with_prefix("", |_, sub| {
        named_graphs += 1;
        named_triples += sub.len();
    });
    if named_graphs > 0 {
        eprintln!(
            "warning: HDT carries a single default graph — dropping {named_graphs} named graph(s) \
             ({named_triples} triple(s)); only the {} default-graph triple(s) are written",
            g.len()
        );
    }
    let t = Instant::now();
    sparq_hdt::save(&g, out).unwrap_or_else(|e| {
        eprintln!("error writing {out}: {e}");
        std::process::exit(1);
    });
    eprintln!("wrote {} triples to {out} in {:.3}s", g.len(), t.elapsed().as_secs_f64());
}

/// [OPUS-4.8] (sq-vczh2, epic sq-2m6zm) `terse <terse-query>` — transpile a *terse* query into
/// the canonical, conformant SPARQL it expands to, printing the verifiable JSON contract
/// (`{ canonical_sparql, keywords, resolutions, warnings, legendVersion }`) — the SAME shape the
/// server's `POST /terse/transpile` endpoint returns.
///
/// The terse query is read from the positional argument, or — when that argument is `-` — from
/// stdin (so a query with shell-hostile characters can be piped). It is the LEAN `sparq-terse`
/// build: the `K:<name>` keyword layer is fully on; a `V("phrase")` construct loud-FAILS (exit 2)
/// rather than being guessed (concept resolution is a future `vectors`-gated extension; the V()
/// ambiguity caveat is tracked by sq-26fdp). It NEVER executes the query — pipe the printed
/// `canonical_sparql` into `query`/`query-mmap` to run it. Behind the opt-in `terse` cargo feature.
#[cfg(feature = "terse")]
fn cmd_terse(args: &[String]) {
    let src: String = match args.get(2).map(String::as_str) {
        Some("-") => {
            let mut buf = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
                eprintln!("error reading terse query from stdin: {e}");
                std::process::exit(1);
            }
            buf
        }
        Some(q) => q.to_string(),
        None => {
            eprintln!(
                "usage: sparq-cli terse <terse-query | ->\n  \
                 Transpiles a terse query (the `K:<name>` keyword layer over canonical SPARQL) and\n  \
                 prints the canonical SPARQL it expands to, as JSON. Pass `-` to read the query from\n  \
                 stdin. It does NOT execute the query — pipe the canonical_sparql into `query`."
            );
            std::process::exit(2);
        }
    };
    match sparq_terse::terse_to_sparql(&src) {
        Ok(expansion) => println!("{}", terse_expansion_to_json(&expansion)),
        // Every terse failure is a loud, deterministic input error (unknown keyword, `PREFIX K:`
        // collision, an un-resolvable `V(...)` in this lean build, or the conformance canary): it
        // goes to stderr with the transpiler's own message and a non-zero exit, never a guess.
        Err(e) => {
            eprintln!("terse: {e}");
            std::process::exit(2);
        }
    }
}

/// [OPUS-4.8] (sq-vczh2) Hand-build the verifiable terse-expansion JSON (the CLI runtime binary
/// carries no `serde_json` — it hand-builds JSON like `bench`'s `--json`). The shape mirrors the
/// server's `POST /terse/transpile` body so a caller gets the SAME contract from either surface.
#[cfg(feature = "terse")]
fn terse_expansion_to_json(expansion: &sparq_terse::Expansion) -> String {
    // Minimal JSON-string escaper (the same control-char + quote/backslash discipline the server's
    // hand-rolled JSON uses); positional `format!` args avoid the CodeQL rust/unused-variable
    // false positive on inline-captured identifiers.
    fn esc(out: &mut String, s: &str) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
    }

    let mut out = String::from("{\"canonical_sparql\":");
    esc(&mut out, &expansion.canonical_sparql);
    out.push_str(",\"keywords\":[");
    for (i, k) in expansion.keywords.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"keyword\":");
        esc(&mut out, &k.keyword);
        out.push_str(",\"iri\":");
        esc(&mut out, &k.iri);
        out.push_str(",\"legendVersion\":");
        esc(&mut out, &k.legend_version);
        out.push('}');
    }
    // `resolutions` is always empty in this lean (no-`vectors`) CLI build, but the field is in the
    // contract so a future `vectors`-enabled build can populate it without a shape change.
    out.push_str("],\"resolutions\":[");
    for (i, r) in expansion.resolutions.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"phrase\":");
        esc(&mut out, &r.phrase);
        out.push_str(",\"iri\":");
        esc(&mut out, &r.iri);
        out.push_str(&format!(",\"score\":{}", r.score));
        match &r.runner_up {
            Some(ru) => {
                out.push_str(",\"runnerUp\":");
                esc(&mut out, ru);
            }
            None => out.push_str(",\"runnerUp\":null"),
        }
        match r.runner_up_score {
            Some(s) => out.push_str(&format!(",\"runnerUpScore\":{}", s)),
            None => out.push_str(",\"runnerUpScore\":null"),
        }
        out.push_str(&format!(",\"confidence\":{}", r.confidence));
        out.push_str(",\"method\":");
        esc(&mut out, r.method.as_str());
        out.push('}');
    }
    out.push_str("],\"warnings\":[");
    for (i, w) in expansion.warnings.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        esc(&mut out, w);
    }
    out.push_str("],\"legendVersion\":");
    esc(&mut out, sparq_terse::LEGEND_VERSION);
    out.push('}');
    out
}

/// [OPUS-4.8] (sq-l4ki) Output serialisation chosen by `query --format`. `Table` (the
/// human-readable default) and the four W3C SELECT result formats apply to SELECT (and,
/// where meaningful, ASK); CONSTRUCT/DESCRIBE always emit N-Triples regardless of this.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OutFormat {
    Table,
    Tsv,
    Csv,
    Xml,
    Json,
    NTriples,
}

impl OutFormat {
    /// Parses the `--format` value; `None` for an unknown name (caller reports + exits 2).
    fn parse(s: &str) -> Option<OutFormat> {
        Some(match s {
            "table" => OutFormat::Table,
            "tsv" => OutFormat::Tsv,
            "csv" => OutFormat::Csv,
            "xml" => OutFormat::Xml,
            "json" => OutFormat::Json,
            "ntriples" | "n-triples" | "nt" => OutFormat::NTriples,
            _ => return None,
        })
    }
}

/// [OPUS-4.8] (sq-l4ki) Pull an optional `--format <name>` flag out of the argument list,
/// defaulting to a readable `Table`. Exits 2 (usage error) on a missing/unknown value —
/// matching the rest of the CLI's flag-validation contract.
fn out_format_flag(args: &[String]) -> OutFormat {
    match args.iter().position(|a| a == "--format") {
        None => OutFormat::Table,
        Some(i) => {
            let val = args.get(i + 1).unwrap_or_else(|| {
                eprintln!("--format needs a value (table | tsv | csv | xml | json | ntriples)");
                std::process::exit(2);
            });
            OutFormat::parse(val).unwrap_or_else(|| {
                eprintln!("unknown --format '{val}' (known: table | tsv | csv | xml | json | ntriples)");
                std::process::exit(2);
            })
        }
    }
}

/// [OPUS-4.8] (sq-l4ki) Renders a SELECT `QueryResult` as a fixed-width ASCII table —
/// the default human-readable `query` output. Unbound cells render empty; each term uses
/// its SPARQL/Turtle term syntax (oxrdf's `Display`). Column widths are sized to the
/// widest cell so columns line up; for a zero-variable result (which only ASK produces,
/// not SELECT) it prints the row count.
fn select_to_table(r: &sparq_engine::QueryResult) -> String {
    use std::fmt::Write;
    if r.vars.is_empty() {
        return format!("({} row(s))\n", r.rows.len());
    }
    let headers: Vec<String> = r.vars.iter().map(|v| format!("?{}", v.as_str())).collect();
    let cells: Vec<Vec<String>> = r
        .rows
        .iter()
        .map(|row| row.iter().map(|c| c.as_ref().map(|t| t.to_string()).unwrap_or_default()).collect())
        .collect();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in &cells {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let sep = |out: &mut String| {
        out.push('+');
        for w in &widths {
            for _ in 0..w + 2 {
                out.push('-');
            }
            out.push('+');
        }
        out.push('\n');
    };
    let row_line = |out: &mut String, fields: &[String]| {
        out.push('|');
        for (i, f) in fields.iter().enumerate() {
            let pad = widths[i] - f.chars().count();
            let _ = write!(out, " {f}{} |", " ".repeat(pad));
        }
        out.push('\n');
    };
    let mut out = String::new();
    sep(&mut out);
    row_line(&mut out, &headers);
    sep(&mut out);
    for row in &cells {
        row_line(&mut out, row);
    }
    sep(&mut out);
    let _ = writeln!(out, "({} row(s))", r.rows.len());
    out
}

fn cmd_query(args: &[String]) {
    let (path, format, sparql) = match (args.get(2), args.get(3), args.get(4)) {
        (Some(p), Some(f), Some(q)) => (p, f, q),
        _ => {
            eprintln!(
                "usage: sparq-cli query <data-file> <format> <sparql> [--format <table|tsv|csv|xml|json|ntriples>] [--count]"
            );
            std::process::exit(2);
        }
    };
    // [OPUS-4.8] (sq-l4ki) `--count` preserves the historical count-only output; otherwise
    // we emit real results. `--format` selects the SELECT/ASK serialisation (default table).
    let count_only = args.iter().any(|a| a == "--count");
    let out_fmt = out_format_flag(args);

    let g = match reason_flag(args) {
        Some(profile) => load_with_reasoning(path, format, &profile),
        None => load(path, format),
    };

    emit_query_results(&g, sparql, count_only, out_fmt);
}

/// [OPUS-4.8] (sq-iwyy) The shared result-emission core used by BOTH `query` and
/// `query-mmap`, so the two stay at output PARITY by construction. Classifies the parsed
/// query FORM (SELECT / ASK / CONSTRUCT / DESCRIBE) once and routes each to its executor —
/// the engine's `query`/`query_json` run SELECT/ASK; the graph-valued forms go through
/// `construct_or_describe`/`construct_ntriples` (the same path the `bench` runner uses).
///
/// `count_only` restores the historical count-only line; `out_fmt` selects the SELECT/ASK
/// serialisation (CONSTRUCT/DESCRIBE always emit N-Triples). Takes `&Graph` by reference, so
/// it is agnostic to whether the graph is in-RAM (`query`) or mmap-backed (`query-mmap`).
fn emit_query_results(g: &sparq_core::Graph, sparql: &str, count_only: bool, out_fmt: OutFormat) {
    // Parse once so we can classify the query FORM (SELECT / ASK / CONSTRUCT / DESCRIBE)
    // and route it to the matching executor — the engine's `query`/`query_json` only run
    // SELECT/ASK; the graph-valued forms go through `construct_or_describe` (the same path
    // the `bench` runner already uses). [OPUS-4.8]
    let prepared = sparq_engine::PreparedQuery::parse(sparql).unwrap_or_else(|e| {
        eprintln!("query error: {e}");
        std::process::exit(1);
    });

    let t = Instant::now();

    // Backward-friendly count-only path (the pre-sq-l4ki behaviour).
    if count_only {
        if prepared.is_graph_form() {
            match sparq_engine::construct_or_describe(g, sparql) {
                Ok(ts) => println!("{} triples in {:.3}ms", ts.len(), t.elapsed().as_secs_f64() * 1e3),
                Err(e) => die_query(e),
            }
        } else {
            match sparq_engine::query(g, sparql) {
                Ok(r) => println!("{} solutions in {:.3}ms", r.len(), t.elapsed().as_secs_f64() * 1e3),
                Err(e) => die_query(e),
            }
        }
        return;
    }

    // CONSTRUCT / DESCRIBE -> the resulting triples as N-Triples (always; `--format` is a
    // SELECT/ASK results-format selector and does not apply to the graph forms).
    if prepared.is_graph_form() {
        match sparq_engine::construct_ntriples(g, sparql) {
            Ok(nt) => print!("{nt}"),
            Err(e) => die_query(e),
        }
        return;
    }

    // ASK -> a boolean. (`--format json`/`xml` emit the W3C boolean documents; the other
    // formats fall back to the bare `true`/`false` token.)
    if matches!(prepared.query(), spargebra::Query::Ask { .. }) {
        let value = match sparq_engine::ask(g, sparql) {
            Ok(b) => b,
            Err(e) => die_query(e),
        };
        match out_fmt {
            OutFormat::Json => println!("{}", sparq_server::results::ask_to_json(value)),
            OutFormat::Xml => print!("{}", sparq_server::results::ask_to_xml(value)),
            _ => println!("{value}"),
        }
        return;
    }

    // SELECT -> the solution bindings. JSON reuses the engine's fast direct serialiser; the
    // other formats reuse sparq-server's W3C SELECT serialisers over the QueryResult.
    if out_fmt == OutFormat::Json {
        match sparq_engine::query_json(g, sparql) {
            Ok(s) => println!("{s}"),
            Err(e) => die_query(e),
        }
        return;
    }
    let r = match sparq_engine::query(g, sparql) {
        Ok(r) => r,
        Err(e) => die_query(e),
    };
    match out_fmt {
        OutFormat::Table => print!("{}", select_to_table(&r)),
        OutFormat::Tsv => print!("{}", sparq_server::results::select_to_tsv(&r)),
        OutFormat::Csv => print!("{}", sparq_server::results::select_to_csv(&r)),
        OutFormat::Xml => print!("{}", sparq_server::results::select_to_xml(&r)),
        // A SELECT has no graph form, so N-Triples is meaningless for bindings — fall back
        // to TSV (the closest line-oriented bindings format) rather than error.
        OutFormat::NTriples => print!("{}", sparq_server::results::select_to_tsv(&r)),
        OutFormat::Json => unreachable!("json handled above"),
    }
}

/// [OPUS-4.8] (sq-l4ki) Reports a query execution error to stderr and exits 1 (runtime
/// error) — the shared failure tail of every `query` dispatch arm.
fn die_query(e: String) -> ! {
    eprintln!("query error: {e}");
    std::process::exit(1);
}

/// [OPUS-4.8] (sq-d7d) Extract a `--json <path>` results-emit flag (and its value) from the
/// positional argument vector, returning `(positional_args_without_the_flag, Option<path>)`.
/// `bench` / `bench-mmap` index their other arguments positionally (`args.get(N)`), so the
/// flag+value pair is removed BEFORE positional parsing rather than handled inline — this keeps
/// the historical positional contract intact whether the flag is present or absent. A bare
/// `--json` with no following value is a usage error (exit 2), mirroring the rest of the CLI.
fn take_json_flag(args: &[String]) -> (Vec<String>, Option<String>) {
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

fn cmd_bench(args: &[String]) {
    let (args, json_path) = take_json_flag(args);
    let (path, format, dir) = match (args.get(2), args.get(3), args.get(4)) {
        (Some(p), Some(f), Some(d)) => (p, f, d),
        _ => {
            eprintln!("usage: sparq-cli bench <data-file> <format> <queries-dir> [iters] [count|materialize|json] [--json <results.json>]");
            std::process::exit(2);
        }
    };
    let iters: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(5);
    let mode = args.get(6).map(String::as_str).unwrap_or("materialize");
    if !matches!(mode, "count" | "materialize" | "json") {
        eprintln!("unknown mode `{mode}`; expected count | materialize | json");
        std::process::exit(2);
    }
    let g = load(path, format);
    run_query_suite(&g, dir, iters, mode, json_path.as_deref());
}

/// `bench-mmap <dir> <queries-dir> [iters] [count|materialize|json]` — same as `bench`
/// but OPENS the dataset out-of-core (memory-mapped), so the compute of a >RAM index can
/// be measured without loading it into RAM. Used to compare sparq's compute against the
/// stored QLever baselines at 100M+ on a 16 GB machine.
fn cmd_bench_mmap(args: &[String]) {
    let (args, json_path) = take_json_flag(args);
    let (dir, qdir) = match (args.get(2), args.get(3)) {
        (Some(d), Some(q)) => (d.as_str(), q.as_str()),
        _ => {
            eprintln!("usage: sparq-cli bench-mmap <index-dir> <queries-dir> [iters] [count|materialize|json] [decompress] [--json <results.json>]");
            std::process::exit(2);
        }
    };
    let iters: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(5);
    let mode = args.get(5).map(String::as_str).unwrap_or("count");
    let t = Instant::now();
    let mut g = sparq_core::Graph::open(std::path::Path::new(dir)).unwrap_or_else(|e| {
        eprintln!("open error: {e}");
        std::process::exit(1);
    });
    eprintln!("opened {} triples (mmap) in {:.3}s | committed heap ~{:.3} GB", g.len(), t.elapsed().as_secs_f64(), g.heap_bytes() as f64 / 1e9);
    // Load-time decompression mode: decode compressed perms to raw RAM before querying.
    if args.get(6).map(String::as_str) == Some("decompress") {
        let t = Instant::now();
        g.decompress_indexes();
        eprintln!("decompressed indexes to RAM in {:.3}s | committed heap ~{:.3} GB", t.elapsed().as_secs_f64(), g.heap_bytes() as f64 / 1e9);
    }
    run_query_suite(&g, qdir, iters, mode, json_path.as_deref());
}

/// One measured row of the query suite — the same fields the TSV reports
/// (`name`, `rows`, `min_micros`), captured so they can be serialised to JSON.
/// [OPUS-4.8] (sq-d7d)
struct SuiteRow {
    name: String,
    /// `Ok(min_micros)` for a successful query (with `rows`), or `Err(message)` if it failed.
    outcome: Result<(usize, f64), String>,
}

/// Runs every `*.rq` in `dir` (sorted) `iters` times in `mode`, printing one TSV line
/// per query: `<name>\t<rows>\t<min_micros>`.
///
/// [OPUS-4.8] (sq-d7d) When `json_path` is `Some`, the SAME measured fields are ALSO written
/// to that path as a machine-readable JSON document (the structured-benchmark-catalog pattern,
/// mirroring `bench/memtier` / `mpc_net_bench`'s dependency-free emit). STDOUT is byte-for-byte
/// unchanged whether or not the flag is present — JSON is strictly additive.
fn run_query_suite(g: &sparq_core::Graph, dir: &str, iters: usize, mode: &str, json_path: Option<&str>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| {
            eprintln!("error reading {dir}: {e}");
            std::process::exit(1);
        })
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "rq").unwrap_or(false))
        .collect();
    entries.sort();

    let mut results: Vec<SuiteRow> = Vec::with_capacity(entries.len());
    for path in entries {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let sparql = std::fs::read_to_string(&path).unwrap();
        // [OPUS-4.8] Route the graph-valued forms (CONSTRUCT / DESCRIBE) through the
        // construct/describe executor — count()/query()/query_json() only handle SELECT/ASK.
        // This lets the operator-coverage suite (bench/operators/queries) exercise every
        // SPARQL query form through the same `bench` runner; "rows" = produced triples.
        let graph_form = sparq_engine::PreparedQuery::parse(&sparql).map(|p| p.is_graph_form()).unwrap_or(false);
        let mut best = f64::INFINITY;
        let mut rows = 0;
        let mut err = None;
        for _ in 0..iters {
            let t = Instant::now();
            let r: Result<usize, String> = if graph_form {
                sparq_engine::construct_or_describe(g, &sparql).map(|ts| ts.len())
            } else {
                match mode {
                    "count" => sparq_engine::count(g, &sparql),
                    "json" => sparq_engine::query_json(g, &sparql).map(|s| {
                        let n = s.len();
                        std::hint::black_box(s);
                        n
                    }),
                    _ => sparq_engine::query(g, &sparql).map(|r| r.len()),
                }
            };
            match r {
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
        match &err {
            None => println!("{name}\t{rows}\t{best:.1}"),
            Some(e) => println!("{name}\tERROR\t{e}"),
        }
        let outcome = match err {
            None => Ok((rows, best)),
            Some(e) => Err(e),
        };
        results.push(SuiteRow { name, outcome });
    }

    if let Some(p) = json_path {
        let doc = suite_results_json(mode, iters, &results);
        if let Err(e) = std::fs::write(p, doc) {
            eprintln!("error writing --json results to {p}: {e}");
            std::process::exit(1);
        }
        eprintln!("wrote {} query results to {p}", results.len());
    }
}

/// [OPUS-4.8] (sq-d7d) Serialise a query-suite run to stable, dependency-free JSON — the same
/// hand-built `format!` convention as `mpc_net_bench::cell_json` (no serde_json dep added to the
/// CLI). The shape mirrors the catalog pattern: a top-level object carrying the run parameters +
/// an honest `note` (these are the numbers THIS machine measured — non-canonical), and a
/// `queries` array of one object per `*.rq`, each with the SAME fields the TSV prints
/// (`name`, `rows`, `min_micros`) or an `error` string when the query failed.
fn suite_results_json(mode: &str, iters: usize, rows: &[SuiteRow]) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"harness\": \"sparq-cli bench\",\n");
    s.push_str(&format!("  \"mode\": {},\n", json_str(mode)));
    s.push_str(&format!("  \"iters\": {iters},\n"));
    s.push_str(
        "  \"note\": \"min-of-iters wall-clock MEASURED on the running host; \
         NON-CANONICAL (whatever this machine measured) — do not bake into committed files\",\n",
    );
    s.push_str("  \"queries\": [\n");
    for (i, r) in rows.iter().enumerate() {
        s.push_str("    {\n");
        s.push_str(&format!("      \"name\": {}", json_str(&r.name)));
        match &r.outcome {
            Ok((rows, micros)) => {
                s.push_str(&format!(",\n      \"rows\": {rows}"));
                s.push_str(&format!(",\n      \"min_micros\": {micros:.1}\n"));
            }
            Err(e) => {
                s.push_str(&format!(",\n      \"error\": {}\n", json_str(e)));
            }
        }
        s.push_str("    }");
        if i + 1 < rows.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push_str("  ]\n");
    s.push_str("}\n");
    s
}

/// [OPUS-4.8] (sq-d7d) Minimal JSON string escaper for the dependency-free emit — escapes the
/// characters JSON requires (`"`, `\`, and the C0 control set incl. the named whitespace
/// escapes). Query names are file stems and error strings are engine messages, so this covers
/// the realistic input; anything else still produces valid `\uXXXX` escapes.
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for c in raw.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
