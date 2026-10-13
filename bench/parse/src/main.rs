//! Baseline measurements for the custom-parsers design (one bin, subcommands).
//! Numbers land in research/custom-parsers-baseline.md.
//!
//! Subcommands (a trailing `--json <path>` on any subcommand also writes the same
//! measured rows as a dependency-free JSON document — sq-ghjc; STDOUT is unchanged):
//!
//! ```text
//!   gen <entities> <out.nt> <out.ttl>   deterministic synthetic dataset (same
//!                                       generator as crates/sparq-bench/src/dataset.rs)
//!   to-ttl <in.nt> <out.ttl>            convert N-Triples to Turtle via the oxttl
//!                                       serializer (predicate/object grouping, a few
//!                                       Wikidata prefixes registered)
//!   compress <file>                     write <file>.gz (flate2 -6) and <file>.zst (level 3)
//!   bench-nt <file.nt>                  parser/ingest measurements on N-Triples
//!   bench-ttl <file.ttl>                parser/ingest measurements on Turtle
//!   bench-zip <file.nt>                 decode-only + two-stage vs streaming ingest
//!                                       (expects <file.nt>.gz and <file.nt>.zst)
//!   gen-hdt <in.nt> <out.hdt>           build a real .hdt from N-Triples in-process
//!                                       (hdt crate writer; no external rdf2hdt)
//!   bench-hdt <file.hdt>                A/B HDT load: the sparq DIRECT decoder
//!                                       (graph_from_reader, skips the wavelet/OP
//!                                       index) vs the UPSTREAM wavelet-building
//!                                       path (Hdt::read + graph_from_hdt). Reports
//!                                       the speedup RATIO + a DIRECT per-stage split
//!                                       (dict/scan/build, with dict broken into the
//!                                       four PFC sections + merge and scan into
//!                                       read vs SPO-walk) + peak RSS + an NT-vs-HDT
//!                                       A/B row (loads <file>.nt via the fast
//!                                       parallel NT path) [sq-q6a1, sq-7ge0].
//!   bench-hdt-zip <file.hdt>            decompress+parse MB/s of <file.hdt>.{gz,zst,bz2}
//!                                       via the direct decoder (expects those files)
//! ```

// Match the production CLI: sparq-cli ingest runs under mimalloc.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use sparq_core::Graph;
use std::hint::black_box;
// [FABLE-5] (sq-tonhr.2) Pluggable CANDIDATE parser rows (rdf-shuttle generated
// parsers slot in here as competitor rows; empty in the default build).
mod candidate;

use std::io::{Read, Write};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const ITERS: usize = 3;

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-ghjc) --json <path> machine-readable results emit
// ---------------------------------------------------------------------------
//
// The table-printing subcommands (`bench-nt`, `bench-ttl`, `bench-zip`,
// `ab-ttl-intern`, `bench-hdt`, `bench-hdt-zip`) emit ad-hoc markdown so a doc
// can `run the harness` rather than restate frozen numbers (sq-5vm). This adds a
// strictly-additive `--json <path>` that mirrors the SAME measured rows as a
// stable, DEPENDENCY-FREE JSON document — the hand-built `format!` convention
// from `crates/sparq-mpc/examples/mpc_net_bench.rs::cell_json` and the
// `suite_results_json` sq-d7d added to `sparq-cli`. NO serde dep is added.
//
// STDOUT is byte-for-byte unchanged whether or not `--json` is present: the
// collector silently accumulates every `row(..)` call (and the `ab-ttl-intern`
// special rows) and `main` writes the document at the end only when a path was
// given. The numbers are whatever THIS machine measured — NON-CANONICAL — so the
// emitted `note` says so and nothing here is committed.

/// One captured measurement row: an ORDERED list of `(key, raw_json_value)` pairs.
/// Heterogeneous by design — `row()` pushes the 6-column table shape, while
/// `ab-ttl-intern` pushes its own `strategy`/`mtriples_s`/`ns_per_triple` shape —
/// so each subcommand serialises exactly the fields it measured.
type JsonRow = Vec<(&'static str, String)>;

/// Global, lazily-initialised JSON-emit state. `path` is the `--json <path>`
/// destination (None disables emit entirely); `subcommand`/`dataset`/`note` label
/// the run; `rows` accumulates every captured measurement. Behind a `Mutex` only so
/// the parallel benches (which call `row()` from the main thread, but via rayon
/// pools that could in principle move it) never race — emit is not on any hot path.
struct JsonCollector {
    path: Option<String>,
    subcommand: String,
    dataset: String,
    rows: Vec<JsonRow>,
}

static JSON: OnceLock<Mutex<JsonCollector>> = OnceLock::new();

fn json_collector() -> &'static Mutex<JsonCollector> {
    JSON.get_or_init(|| {
        Mutex::new(JsonCollector {
            path: None,
            subcommand: String::new(),
            dataset: String::new(),
            rows: Vec::new(),
        })
    })
}

/// Records the run-level labels (subcommand + dataset name) once, at dispatch.
fn json_init(subcommand: &str, dataset: &str) {
    let mut c = json_collector().lock().unwrap();
    c.subcommand = subcommand.to_string();
    c.dataset = dataset.to_string();
}

/// Records the `--json <path>` destination (only set when the flag was present).
fn json_set_path(path: &str) {
    json_collector().lock().unwrap().path = Some(path.to_string());
}

/// Appends one captured measurement row.
fn json_push(row: JsonRow) {
    json_collector().lock().unwrap().rows.push(row);
}

/// Extracts a `--json <path>` flag (and its value) from the raw argv, returning the
/// positional args WITHOUT the flag pair. The subcommands index their operands
/// positionally (`args[2]`, `args[3]`, ...), so the flag is removed BEFORE dispatch to
/// keep the historical positional contract intact whether the flag is present or not.
/// A bare `--json` with no following value is a usage error (exit 2), mirroring
/// `sparq-cli`'s identical flag.
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

/// Minimal JSON string escaper for the dependency-free emit — escapes the characters
/// JSON requires (`"`, `\`, the C0 control set incl. the named whitespace escapes).
/// Task labels are static ASCII and dataset names are file stems, so this covers the
/// realistic input; anything else still produces valid `\uXXXX` escapes.
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    sparq_core::json::escape_into(&mut out, raw);
    out.push('"');
    out
}

/// Serialises the accumulated rows to the `--json` path, if one was given. Called
/// once at the end of `main`. The shape mirrors `suite_results_json`: a top-level
/// object carrying the run labels + an honest non-canonical `note`, and a `rows`
/// array of one object per captured measurement (each row's keys are exactly the
/// fields that subcommand measured). Writes nothing (and is a no-op) when `--json`
/// was absent — STDOUT is the only output in that case.
fn json_flush() {
    let c = json_collector().lock().unwrap();
    let Some(path) = c.path.clone() else {
        return;
    };
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"harness\": \"bench/parse parse-baseline\",\n");
    s.push_str(&format!("  \"subcommand\": {},\n", json_str(&c.subcommand)));
    s.push_str(&format!("  \"dataset\": {},\n", json_str(&c.dataset)));
    s.push_str(&format!("  \"iters\": {ITERS},\n"));
    s.push_str(
        "  \"note\": \"median/best-of-iters wall-clock MEASURED on the running host; \
         NON-CANONICAL (whatever this machine measured) — do not bake into committed files\",\n",
    );
    s.push_str("  \"rows\": [\n");
    for (i, row) in c.rows.iter().enumerate() {
        s.push_str("    {");
        for (j, (k, v)) in row.iter().enumerate() {
            if j > 0 {
                s.push(',');
            }
            s.push_str(&format!(" {}: {}", json_str(k), v));
        }
        s.push_str(" }");
        if i + 1 < c.rows.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push_str("  ]\n");
    s.push_str("}\n");
    if let Err(e) = std::fs::write(&path, s) {
        eprintln!("error writing --json results to {path}: {e}");
        std::process::exit(1);
    }
    eprintln!("wrote {} result rows to {path}", c.rows.len());
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let (args, json_path) = take_json_flag(&argv);
    if let Some(p) = &json_path {
        json_set_path(p);
    }
    // Label the run (subcommand + dataset) for the JSON emit, where a dataset path
    // is the conventional second operand. Harmless when `--json` is absent.
    if let Some(sub) = args.get(1) {
        let dataset = args.get(2).map(|p| dataset_name(p)).unwrap_or("");
        json_init(sub, dataset);
    }
    match args.get(1).map(String::as_str) {
        Some("gen") => gen(args[2].parse().unwrap(), &args[3], &args[4]),
        Some("to-ttl") => to_ttl(&args[2], &args[3]),
        Some("compress") => compress(&args[2]),
        Some("bench-nt") => bench_nt(&args[2]),
        Some("bench-ttl") => bench_ttl(&args[2]),
        Some("ab-ttl-intern") => ab_ttl_intern(&args[2]),
        Some("bench-zip") => bench_zip(&args[2]),
        Some("probe-read") => probe_read(&args[2]),
        Some("gen-hdt") => gen_hdt(&args[2], &args[3]),
        Some("bench-hdt") => bench_hdt(&args[2]),
        Some("bench-hdt-zip") => bench_hdt_zip(&args[2]),
        // Internal: load ONE path once in a fresh process, print its peak RSS.
        Some("hdt-rss") => hdt_rss(&args[2], &args[3]),
        // [SONNET-4.6] (sq-hmd7l.6) external parser competitor columns.
        Some("bench-ext") => bench_ext(&args[2]),
        _ => {
            eprintln!("usage: parse-baseline gen|to-ttl|compress|bench-nt|bench-ttl|ab-ttl-intern|bench-zip|gen-hdt|bench-hdt|bench-hdt-zip|bench-ext ... [--json <results.json>]");
            std::process::exit(2);
        }
    }
    // Strictly-additive: writes the JSON document only when `--json <path>` was given.
    json_flush();
}

// ---------------------------------------------------------------------------
// timing
// ---------------------------------------------------------------------------

/// Runs `f` ITERS times, returns the median wall seconds.
fn median<F: FnMut()>(mut f: F) -> f64 {
    let mut times: Vec<f64> = (0..ITERS)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64()
        })
        .collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times[times.len() / 2]
}

/// Prints one markdown row: task, threads, seconds, input MB/s, Mtriples/s.
///
/// [OPUS-4.8] (sq-ghjc) Also captures the SAME measured fields for the optional
/// `--json` emit. The push is unconditional and cheap; `json_flush` only writes a
/// file when a `--json <path>` was given, so STDOUT is unaffected either way.
fn row(dataset: &str, task: &str, threads: usize, secs: f64, bytes: usize, triples: usize) {
    let mbs = bytes as f64 / 1e6 / secs;
    let mts = triples as f64 / 1e6 / secs;
    println!("| {dataset} | {task} | {threads} | {secs:.3} | {mbs:.0} | {mts:.2} |");
    json_push(vec![
        ("dataset", json_str(dataset)),
        ("task", json_str(task)),
        ("threads", threads.to_string()),
        ("secs", format!("{secs:.6}")),
        ("bytes", bytes.to_string()),
        ("triples", triples.to_string()),
        ("mb_s", format!("{mbs:.3}")),
        ("mtriples_s", format!("{mts:.3}")),
    ]);
}

fn pool(n: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap()
}

fn dataset_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn read_to_string(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

// ---------------------------------------------------------------------------
// bench-nt
// ---------------------------------------------------------------------------

fn bench_nt(path: &str) {
    let text = read_to_string(path);
    let bytes = text.as_bytes();
    let name = dataset_name(path);
    let ncpu = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);

    // Triple count (for triples/s) via oxttl, also validates the slice parses.
    let triples = oxttl::NTriplesParser::new()
        .for_slice(bytes)
        // [OPUS-4.8] (sq-ghjc) fold-count keeps the parse validation (expect) while
        // satisfying clippy::suspicious_map (map+count would warn): count + panic on error.
        .fold(0usize, |acc, r| {
            r.expect("dataset must parse");
            acc + 1
        });
    eprintln!("{name}: {} bytes, {triples} triples", bytes.len());
    println!("| dataset | task | threads | s | MB/s | Mtriples/s |");
    println!("|---|---|---|---|---|---|");

    // Memory-traversal ceiling: sum the buffer as u64 words (what "the parser is
    // memory-bound" would look like).
    let secs = median(|| {
        let mut acc = 0u64;
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            acc = acc.wrapping_add(u64::from_le_bytes(c.try_into().unwrap()));
        }
        for &b in chunks.remainder() {
            acc = acc.wrapping_add(b as u64);
        }
        black_box(acc);
    });
    row(name, "memscan (sum bytes, 1 core)", 1, secs, bytes.len(), triples);

    // oxttl, parse only (discard triples): pure parser cost, no interning/index.
    let secs = median(|| {
        let n = oxttl::NTriplesParser::new().for_slice(bytes).fold(0usize, |acc, r| { r.unwrap(); acc + 1 });
        black_box(n);
    });
    row(name, "oxttl NT parse-only", 1, secs, bytes.len(), triples);

    // oxttl + intern into Dict (no index build): what Graph::load_reader does
    // before Graph::build.
    let secs = median(|| {
        let mut dict = sparq_core::dict::Dict::new();
        let mut out: Vec<[sparq_core::dict::Id; 3]> = Vec::new();
        for t in oxttl::NTriplesParser::new().for_slice(bytes) {
            let t = t.unwrap();
            let s = dict.intern(&subject_to_term(&t.subject));
            let p = dict.intern(&oxrdf::Term::NamedNode(t.predicate.clone()));
            let o = dict.intern(&t.object);
            out.push([s, p, o]);
        }
        black_box(out.len());
    });
    row(name, "oxttl NT parse+intern", 1, secs, bytes.len(), triples);

    // oxttl end-to-end into Graph (serial streaming loader = the non-NT real path).
    let secs = median(|| {
        let g = Graph::load_reader(std::io::Cursor::new(bytes), "ntriples").unwrap();
        black_box(g.len());
    });
    row(name, "oxttl NT -> Graph (load_reader)", 1, secs, bytes.len(), triples);

    // Incumbent custom byte-level parser (sparq_core::nt via parse_to_triples),
    // single-threaded and parallel; then the full Graph build on top.
    for &n in &[1usize, ncpu] {
        let p = pool(n);
        let secs = median(|| {
            let (d, t) = p.install(|| Graph::parse_to_triples(&text, "ntriples")).unwrap();
            black_box((d.len(), t.len()));
        });
        row(name, "custom NT parse+intern (incumbent)", n, secs, bytes.len(), triples);

        let secs = median(|| {
            let g = p.install(|| Graph::load_str(&text, "ntriples")).unwrap();
            black_box(g.len());
        });
        row(name, "custom NT -> Graph (load_str, incumbent)", n, secs, bytes.len(), triples);
    }

    candidate_rows(name, "ntriples", &text, bytes.len(), triples);
}

/// [FABLE-5] (sq-tonhr.2) Emit one competitor row per registered candidate parser
/// (`candidate::candidates_for`) — same table + `--json` shape as every other row.
/// The candidate's statement count is VERIFIED against the reference count before
/// timing (a mis-parsing candidate panics loudly; it never posts a bogus number).
fn candidate_rows(name: &str, format: &str, text: &str, bytes: usize, triples: usize) {
    for c in candidate::candidates_for(format) {
        let counted = (c.parse)(text).unwrap_or_else(|e| panic!("candidate {} failed to parse the corpus: {e}", c.name));
        assert_eq!(counted, triples, "candidate {} statement count {counted} != reference {triples}", c.name);
        let secs = median(|| {
            let n = (c.parse)(text).unwrap();
            black_box(n);
        });
        row(name, &format!("candidate {} parse ({format})", c.name), 1, secs, bytes, triples);
    }
}

fn subject_to_term(s: &oxrdf::NamedOrBlankNode) -> oxrdf::Term {
    match s {
        oxrdf::NamedOrBlankNode::NamedNode(n) => oxrdf::Term::NamedNode(n.clone()),
        oxrdf::NamedOrBlankNode::BlankNode(b) => oxrdf::Term::BlankNode(b.clone()),
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] ab-ttl-intern — ISOLATED A/B of the Turtle T1 lever
// ---------------------------------------------------------------------------
//
// Both `bench-ttl` rows mix parsing + interning; oxttl parse dominates (~63% of
// parse+intern), so the intern delta T1 targets is swamped by parse noise. This
// subcommand removes the parse: it pre-parses the document ONCE into a Vec of
// oxttl `Triple`s, then times ONLY the intern loop, two strategies back-to-back
// over the SAME term stream, many iterations — the cleanest read of the
// `oxrdf::Term`-materialization tax (T1). Median-of-N; report the RATIO (old/new),
// which is load-robust.
//
//   OLD (pre-T1): per S/P/O build an owned `oxrdf::Term` then `dict.intern(&Term)`.
//   NEW (T1):     dispatch on the borrowed oxttl components into
//                 `intern_iri`/`intern_blank`/`intern_lit` directly.
fn ab_ttl_intern(path: &str) {
    use oxrdf::Term;
    use sparq_core::dict::{Dict, Id};

    let text = read_to_string(path);
    let bytes = text.as_bytes();
    let name = dataset_name(path);

    // Parse once; hold the owned triples so the timed loops never touch the parser.
    let parsed: Vec<oxrdf::Triple> = oxttl::TurtleParser::new()
        .for_slice(bytes)
        .map(|r| r.expect("dataset must parse"))
        .collect();
    let n = parsed.len();
    eprintln!("{name}: {} bytes, {n} triples (parse excluded from timing)", bytes.len());

    // OLD: owned-Term materialization per component, then dict.intern(&Term).
    let old = |triples: &[oxrdf::Triple]| -> usize {
        let mut dict = Dict::new();
        let mut out: Vec<[Id; 3]> = Vec::with_capacity(triples.len());
        for t in triples {
            let s = dict.intern(&subject_to_term(&t.subject));
            let p = dict.intern(&Term::NamedNode(t.predicate.clone()));
            let o = dict.intern(&t.object);
            out.push([s, p, o]);
        }
        black_box(dict.len());
        out.len()
    };

    // NEW (T1): borrowed-slice dispatch into the component interners.
    let new = |triples: &[oxrdf::Triple]| -> usize {
        let mut dict = Dict::new();
        let mut out: Vec<[Id; 3]> = Vec::with_capacity(triples.len());
        for t in triples {
            let s = match &t.subject {
                oxrdf::NamedOrBlankNode::NamedNode(nn) => dict.intern_iri(nn.as_str()),
                oxrdf::NamedOrBlankNode::BlankNode(b) => dict.intern_blank(b.as_str()),
            };
            let p = dict.intern_iri(t.predicate.as_str());
            let o = match &t.object {
                Term::NamedNode(nn) => dict.intern_iri(nn.as_str()),
                Term::BlankNode(b) => dict.intern_blank(b.as_str()),
                // The bench datasets carry no base-direction literals; language() suffices here.
                Term::Literal(l) => dict.intern_lit(l.value(), l.datatype().as_str(), l.language()),
                Term::Triple(_) => dict.intern(&t.object),
            };
            out.push([s, p, o]);
        }
        black_box(dict.len());
        out.len()
    };

    // Sanity: both strategies must intern to the same dict size + triple count.
    assert_eq!(old(&parsed), new(&parsed));

    // Many iterations, interleaved old/new each round, median over rounds.
    const ROUNDS: usize = 15;
    let mut t_old = Vec::with_capacity(ROUNDS);
    let mut t_new = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        let s = Instant::now();
        black_box(old(&parsed));
        t_old.push(s.elapsed().as_secs_f64());
        let s = Instant::now();
        black_box(new(&parsed));
        t_new.push(s.elapsed().as_secs_f64());
    }
    t_old.sort_by(|a, b| a.partial_cmp(b).unwrap());
    t_new.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mo = t_old[t_old.len() / 2];
    let mn = t_new[t_new.len() / 2];
    println!("| dataset | strategy | median s ({ROUNDS} rounds) | Mtriples/s | ns/triple |");
    println!("|---|---|---|---|---|");
    println!("| {name} | OLD owned-Term intern | {mo:.4} | {:.2} | {:.1} |", n as f64 / 1e6 / mo, mo * 1e9 / n as f64);
    println!("| {name} | NEW (T1) borrowed intern | {mn:.4} | {:.2} | {:.1} |", n as f64 / 1e6 / mn, mn * 1e9 / n as f64);
    println!(
        "| {name} | **ratio old/new** | **{:.3}×** | | |",
        mo / mn
    );
    // [OPUS-4.8] (sq-ghjc) Capture the intern-A/B shape (distinct from the standard
    // `row()` columns) for the optional `--json` emit. Same measured numbers as the
    // markdown rows above; the ratio is recorded as a single labelled row.
    let intern_row = |strategy: &'static str, median_s: f64| -> JsonRow {
        vec![
            ("dataset", json_str(name)),
            ("strategy", json_str(strategy)),
            ("rounds", ROUNDS.to_string()),
            ("median_s", format!("{median_s:.6}")),
            ("mtriples_s", format!("{:.3}", n as f64 / 1e6 / median_s)),
            ("ns_per_triple", format!("{:.3}", median_s * 1e9 / n as f64)),
        ]
    };
    json_push(intern_row("OLD owned-Term intern", mo));
    json_push(intern_row("NEW (T1) borrowed intern", mn));
    json_push(vec![
        ("dataset", json_str(name)),
        ("strategy", json_str("ratio old/new")),
        ("ratio_old_new", format!("{:.3}", mo / mn)),
    ]);
    eprintln!(
        "intern-only A/B: OLD {mo:.4}s, NEW {mn:.4}s, speedup {:.3}× ({:.1}% of intern step removed)",
        mo / mn,
        100.0 * (mo - mn) / mo
    );
}

// ---------------------------------------------------------------------------
// bench-ttl
// ---------------------------------------------------------------------------

fn bench_ttl(path: &str) {
    let text = read_to_string(path);
    let bytes = text.as_bytes();
    let name = dataset_name(path);
    let ncpu = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);

    let triples = oxttl::TurtleParser::new()
        .for_slice(bytes)
        // [OPUS-4.8] (sq-ghjc) fold-count keeps the parse validation (expect) while
        // satisfying clippy::suspicious_map (map+count would warn): count + panic on error.
        .fold(0usize, |acc, r| {
            r.expect("dataset must parse");
            acc + 1
        });
    eprintln!("{name}: {} bytes, {triples} triples", bytes.len());
    println!("| dataset | task | threads | s | MB/s | Mtriples/s |");
    println!("|---|---|---|---|---|---|");

    // oxttl, parse only.
    let secs = median(|| {
        let n = oxttl::TurtleParser::new().for_slice(bytes).fold(0usize, |acc, r| { r.unwrap(); acc + 1 });
        black_box(n);
    });
    row(name, "oxttl Turtle parse-only", 1, secs, bytes.len(), triples);

    // oxttl + intern (no index).
    let secs = median(|| {
        let mut dict = sparq_core::dict::Dict::new();
        let mut out: Vec<[sparq_core::dict::Id; 3]> = Vec::new();
        for t in oxttl::TurtleParser::new().for_slice(bytes) {
            let t = t.unwrap();
            let s = dict.intern(&subject_to_term(&t.subject));
            let p = dict.intern(&oxrdf::Term::NamedNode(t.predicate.clone()));
            let o = dict.intern(&t.object);
            out.push([s, p, o]);
        }
        black_box(out.len());
    });
    row(name, "oxttl Turtle parse+intern", 1, secs, bytes.len(), triples);

    // Incumbent path (parse_turtle_parallel chunks the doc, oxttl parses chunks).
    for &n in &[1usize, ncpu] {
        let p = pool(n);
        let secs = median(|| {
            let (d, t) = p.install(|| Graph::parse_to_triples(&text, "turtle")).unwrap();
            black_box((d.len(), t.len()));
        });
        row(name, "Turtle parse+intern (incumbent chunked)", n, secs, bytes.len(), triples);

        let secs = median(|| {
            let g = p.install(|| Graph::load_str(&text, "turtle")).unwrap();
            black_box(g.len());
        });
        row(name, "Turtle -> Graph (load_str)", n, secs, bytes.len(), triples);
    }

    candidate_rows(name, "turtle", &text, bytes.len(), triples);
}

// ---------------------------------------------------------------------------
// bench-zip
// ---------------------------------------------------------------------------

fn bench_zip(path: &str) {
    let raw = std::fs::read(path).unwrap();
    let gz = std::fs::read(format!("{path}.gz")).unwrap();
    let zst = std::fs::read(format!("{path}.zst")).unwrap();
    let name = dataset_name(path);
    let ncpu = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
    let p = pool(ncpu);

    let triples = oxttl::NTriplesParser::new()
        .for_slice(&raw)
        // [OPUS-4.8] (sq-ghjc) fold-count keeps the parse validation (expect) while
        // satisfying clippy::suspicious_map (map+count would warn): count + panic on error.
        .fold(0usize, |acc, r| {
            r.expect("dataset must parse");
            acc + 1
        });
    eprintln!(
        "{name}: raw {} B, gz {} B ({:.2}x), zst {} B ({:.2}x), {triples} triples",
        raw.len(),
        gz.len(),
        raw.len() as f64 / gz.len() as f64,
        zst.len(),
        raw.len() as f64 / zst.len() as f64
    );
    println!("| dataset | task | threads | s | MB/s (decompressed) | Mtriples/s |");
    println!("|---|---|---|---|---|---|");

    // Decode only, discard output: bounds what any fused unzip+parse could save.
    let mut sink = vec![0u8; 1 << 20];
    let secs = median(|| {
        let mut d = flate2::read::MultiGzDecoder::new(&gz[..]);
        let mut n = 0usize;
        loop {
            let k = d.read(&mut sink).unwrap();
            if k == 0 {
                break;
            }
            n += k;
        }
        assert_eq!(n, raw.len());
        black_box(n);
    });
    row(name, "gzip decode-only (flate2)", 1, secs, raw.len(), triples);

    let secs = median(|| {
        let mut d = zstd::stream::read::Decoder::new(&zst[..]).unwrap();
        let mut n = 0usize;
        loop {
            let k = d.read(&mut sink).unwrap();
            if k == 0 {
                break;
            }
            n += k;
        }
        assert_eq!(n, raw.len());
        black_box(n);
    });
    row(name, "zstd decode-only", 1, secs, raw.len(), triples);

    // Two-stage: decompress fully to a String, then parallel load_str.
    for (codec, comp) in [("gzip", &gz), ("zstd", &zst)] {
        let secs = median(|| {
            let mut buf = Vec::with_capacity(raw.len());
            match codec {
                "gzip" => {
                    flate2::read::MultiGzDecoder::new(&comp[..]).read_to_end(&mut buf).unwrap()
                }
                _ => zstd::stream::read::Decoder::new(&comp[..])
                    .unwrap()
                    .read_to_end(&mut buf)
                    .unwrap(),
            };
            let text = String::from_utf8(buf).unwrap();
            let g = p.install(|| Graph::load_str(&text, "ntriples")).unwrap();
            black_box(g.len());
        });
        row(name, &format!("{codec} two-stage decode->load_str"), ncpu, secs, raw.len(), triples);

        // Streaming: decompressor feeds load_reader_parallel's 32 MiB block loop
        // (= what `sparq-cli ingest` does today).
        let secs = median(|| {
            let g = p
                .install(|| match codec {
                    "gzip" => Graph::load_reader_parallel(
                        flate2::read::MultiGzDecoder::new(&comp[..]),
                        "ntriples",
                    ),
                    _ => Graph::load_reader_parallel(
                        zstd::stream::read::Decoder::new(&comp[..]).unwrap(),
                        "ntriples",
                    ),
                })
                .unwrap();
            black_box(g.len());
        });
        row(name, &format!("{codec} streaming decode->load_reader_parallel"), ncpu, secs, raw.len(), triples);
    }
}

/// Diagnostic: what does one `read()` into a 32 MiB buffer actually return for each
/// decompressor? Graph::load_reader_parallel used to flush a parse block per read()
/// call, so small reads meant tiny parse blocks (the confirmed streaming-slowdown root
/// cause — fixed by the fill-loop + pipelined rewrite, see the baseline doc's post-fix
/// section).
fn probe_read(path: &str) {
    let gz = std::fs::read(format!("{path}.gz")).unwrap();
    let zst = std::fs::read(format!("{path}.zst")).unwrap();
    let mut buf = vec![0u8; 32 << 20];

    let mut stats = |name: &str, mut r: Box<dyn Read>| {
        let (mut reads, mut total, mut min, mut max) = (0usize, 0usize, usize::MAX, 0usize);
        loop {
            let n = r.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            reads += 1;
            total += n;
            min = min.min(n);
            max = max.max(n);
        }
        eprintln!(
            "{name}: {reads} reads, avg {} B, min {min} B, max {max} B (into a 32 MiB buffer)",
            total / reads.max(1)
        );
    };
    stats("gzip (MultiGzDecoder)", Box::new(flate2::read::MultiGzDecoder::new(std::io::Cursor::new(gz))));
    stats("zstd (Decoder)", Box::new(zstd::stream::read::Decoder::new(std::io::Cursor::new(zst)).unwrap()));
}

// ---------------------------------------------------------------------------
// dataset prep
// ---------------------------------------------------------------------------

/// Same deterministic generator as crates/sparq-bench/src/dataset.rs (write_nt /
/// generate), copied so this standalone project doesn't pull the bench crate
/// (which links oxigraph).
fn gen(n: u32, nt_path: &str, ttl_path: &str) {
    let n = n.max(1);
    let follows_per = 4u32;
    let n_cities = (n / 10).max(1);

    let mut seed = 0x9E3779B97F4A7C15u64;
    let mut rng = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    let mut nt = std::io::BufWriter::new(std::fs::File::create(nt_path).unwrap());
    let mut triples = 0usize;
    for i in 0..n {
        writeln!(nt, "<http://ex/n{i}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Person> .").unwrap();
        writeln!(nt, "<http://ex/n{i}> <http://ex/name> \"name{i}\" .").unwrap();
        writeln!(
            nt,
            "<http://ex/n{i}> <http://ex/age> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            20 + i % 80
        )
        .unwrap();
        writeln!(nt, "<http://ex/n{i}> <http://ex/city> <http://ex/c{}> .", i % n_cities).unwrap();
        triples += 4;
        for _ in 0..follows_per {
            writeln!(nt, "<http://ex/n{i}> <http://ex/follows> <http://ex/n{}> .", rng() % n).unwrap();
            triples += 1;
        }
    }
    nt.flush().unwrap();

    // Turtle variant: same data, prefixed + predicate-grouped (the realistic
    // human-authored shape, same as sparq-bench's generate()).
    let mut seed = 0x9E3779B97F4A7C15u64;
    let mut rng = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    let mut ttl = std::io::BufWriter::new(std::fs::File::create(ttl_path).unwrap());
    writeln!(ttl, "@prefix ex: <http://ex/> .").unwrap();
    for i in 0..n {
        write!(
            ttl,
            "ex:n{i} a ex:Person ;\n  ex:name \"name{i}\" ;\n  ex:age {} ;\n  ex:city ex:c{}",
            20 + i % 80,
            i % n_cities
        )
        .unwrap();
        for _ in 0..follows_per {
            write!(ttl, " ;\n  ex:follows ex:n{}", rng() % n).unwrap();
        }
        writeln!(ttl, " .").unwrap();
    }
    ttl.flush().unwrap();
    eprintln!("gen: {n} entities, {triples} triples -> {nt_path}, {ttl_path}");
}

fn to_ttl(nt_path: &str, ttl_path: &str) {
    let bytes = std::fs::read(nt_path).unwrap();
    let mut ser = oxttl::TurtleSerializer::new()
        .with_prefix("wd", "http://www.wikidata.org/entity/")
        .unwrap()
        .with_prefix("wdt", "http://www.wikidata.org/prop/direct/")
        .unwrap()
        .with_prefix("rdfs", "http://www.w3.org/2000/01/rdf-schema#")
        .unwrap()
        .with_prefix("schema", "http://schema.org/")
        .unwrap()
        .with_prefix("skos", "http://www.w3.org/2004/02/skos/core#")
        .unwrap()
        .for_writer(std::io::BufWriter::new(std::fs::File::create(ttl_path).unwrap()));
    let mut n = 0usize;
    for t in oxttl::NTriplesParser::new().for_slice(&bytes) {
        ser.serialize_triple(&t.unwrap()).unwrap();
        n += 1;
    }
    ser.finish().unwrap().flush().unwrap();
    eprintln!("to-ttl: {n} triples -> {ttl_path}");
}

fn compress(path: &str) {
    let raw = std::fs::read(path).unwrap();

    let f = std::fs::File::create(format!("{path}.gz")).unwrap();
    let mut enc = flate2::write::GzEncoder::new(std::io::BufWriter::new(f), flate2::Compression::new(6));
    enc.write_all(&raw).unwrap();
    enc.finish().unwrap().flush().unwrap();

    let f = std::fs::File::create(format!("{path}.zst")).unwrap();
    let mut enc = zstd::stream::write::Encoder::new(std::io::BufWriter::new(f), 3).unwrap();
    enc.write_all(&raw).unwrap();
    enc.finish().unwrap().flush().unwrap();

    let gz = std::fs::metadata(format!("{path}.gz")).unwrap().len();
    let zst = std::fs::metadata(format!("{path}.zst")).unwrap().len();
    eprintln!(
        "compress: {} B -> gz {} B ({:.2}x), zst {} B ({:.2}x)",
        raw.len(),
        gz,
        raw.len() as f64 / gz as f64,
        zst,
        raw.len() as f64 / zst as f64
    );
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] HDT: gen-hdt / bench-hdt / bench-hdt-zip
//
// The HEADLINE measurement for plan lever H1 (research/parsing-optimization-plan.md
// §1, §4, §5 Wave A): the sparq DIRECT decoder (`sparq_hdt::graph_from_reader`,
// which reads bitmaps/sequences directly and SKIPS the upstream `TriplesBitmap::new`
// wavelet-matrix + OP-index + rank/select build) vs the UPSTREAM wavelet-building
// path (`hdt::Hdt::read` + `sparq_hdt::graph_from_hdt`). The durable, load-robust
// claim is the speedup RATIO; absolute MB/s is noisy on a shared box (§4 NOTE).
// ---------------------------------------------------------------------------

/// Peak resident set size (VmHWM, the high-water mark) in bytes, from
/// /proc/self/status. Returns 0 if unavailable (non-Linux / no procfs).
fn peak_rss_bytes() -> u64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else { return 0 };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            // "VmHWM:    123456 kB"
            if let Some(kb) = rest.split_whitespace().next().and_then(|s| s.parse::<u64>().ok()) {
                return kb * 1024;
            }
        }
    }
    0
}

/// Builds a real `.hdt` archive from an N-Triples file, in-process, via the `hdt`
/// crate's writer (FourSectDict PFC + BitmapTriples, SPO order) — the same archive
/// shape hdt-cpp/hdt-java emit. No external `rdf2hdt` binary needed.
fn gen_hdt(nt_path: &str, hdt_path: &str) {
    let t = Instant::now();
    let hdt = hdt::Hdt::read_nt(std::path::Path::new(nt_path))
        .unwrap_or_else(|e| panic!("building HDT from {nt_path}: {e}"));
    let mut out = std::io::BufWriter::new(std::fs::File::create(hdt_path).unwrap());
    hdt.write(&mut out).unwrap_or_else(|e| panic!("writing {hdt_path}: {e}"));
    out.flush().unwrap();
    drop(out);
    let nt_bytes = std::fs::metadata(nt_path).unwrap().len();
    let hdt_bytes = std::fs::metadata(hdt_path).unwrap().len();
    eprintln!(
        "gen-hdt: {nt_path} ({nt_bytes} B) -> {hdt_path} ({hdt_bytes} B, {:.2}x smaller) in {:.1}s",
        nt_bytes as f64 / hdt_bytes as f64,
        t.elapsed().as_secs_f64()
    );
}

/// A/B HDT load benchmark: direct decoder vs upstream wavelet path, with a
/// per-stage split for the upstream path and the speedup ratio.
///
/// [OPUS-4.8] (sq-q6a1 / sq-7ge0) Also emits the plan §4 measurement gaps: a per-stage
/// split of the DIRECT path (dict decode / triple+id scan / `Graph::build`, via the
/// measurement-only `graph_from_reader_timed`) and an NT-vs-HDT A/B row that loads
/// the same dataset's `.nt` companion via `Graph::load_reader_parallel`. sq-7ge0 refines
/// the split further — the `dict` stage into its four PFC sections + merge, and the `scan`
/// stage into the triples-section READ vs the SPO id-translation WALK — to localise where
/// direct-decode time goes. Both are measurement tooling only — the decoder is unchanged.
fn bench_hdt(path: &str) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let name = dataset_name(path);

    // Triple/term counts + correctness gate: the two paths MUST agree before any
    // perf number is meaningful (empirical-honesty rule).
    let direct = sparq_hdt::graph_from_reader(std::io::Cursor::new(&bytes[..])).expect("direct decode");
    let upstream_hdt = hdt::Hdt::read(std::io::BufReader::new(std::io::Cursor::new(&bytes[..]))).expect("Hdt::read");
    let upstream = sparq_hdt::graph_from_hdt(&upstream_hdt).expect("graph_from_hdt");
    let triples = direct.store.len();
    assert_eq!(triples, upstream.store.len(), "A/B triple count mismatch — refusing to report perf");
    assert_eq!(direct.dict.len(), upstream.dict.len(), "A/B distinct-term mismatch — refusing to report perf");
    drop((direct, upstream, upstream_hdt));

    eprintln!("{name}: {} HDT bytes, {triples} triples", bytes.len());
    println!("| dataset | task | s | MB/s (.hdt bytes) | Mtriples/s |");
    println!("|---|---|---|---|---|");

    // --- DIRECT decoder (the new default path): one end-to-end stage. ---
    let secs_direct = median(|| {
        let g = sparq_hdt::graph_from_reader(std::io::Cursor::new(&bytes[..])).unwrap();
        black_box(g.store.len());
    });
    row(name, "DIRECT decoder (graph_from_reader)", 1, secs_direct, bytes.len(), triples);

    // [OPUS-4.8] (sq-q6a1) 3-way per-stage split of the DIRECT path. The plan's §4
    // asks for dict-decode vs id-translation(scan) vs Graph::build separated; today
    // the upstream split above only fuses translate+build. `graph_from_reader_timed`
    // runs the IDENTICAL decode but records each stage's wall via `Instant::now()` at
    // the existing boundaries (no decoder behaviour change). Median over ITERS, each
    // stage independently, so a noisy run never reorders the stages.
    //
    // [OPUS-4.8] (sq-7ge0) FINER split: the coarse `dict` blob is further broken into the
    // four per-PFC-section decodes (shared/subjects/objects/predicates) + the section-order
    // merge, and the coarse `scan` is split into the triples-section READ vs the SPO
    // id-translation WALK — so the bench localises where direct-decode time goes. Same
    // `StageTimings` measurement-only pattern; the decoder behaviour is unchanged.
    let mut cols: std::collections::BTreeMap<&str, Vec<f64>> = std::collections::BTreeMap::new();
    for _ in 0..ITERS {
        let mut st = sparq_hdt::StageTimings::default();
        let g = sparq_hdt::graph_from_reader_timed(std::io::Cursor::new(&bytes[..]), &mut st).unwrap();
        black_box(g.store.len());
        // Coarse three (back-compat) + the finer sub-stages.
        cols.entry("dict").or_default().push(st.dict.as_secs_f64());
        cols.entry("scan").or_default().push(st.scan.as_secs_f64());
        cols.entry("build").or_default().push(st.build.as_secs_f64());
        cols.entry("dict_shared").or_default().push(st.dict_shared.as_secs_f64());
        cols.entry("dict_subjects").or_default().push(st.dict_subjects.as_secs_f64());
        cols.entry("dict_objects").or_default().push(st.dict_objects.as_secs_f64());
        cols.entry("dict_predicates").or_default().push(st.dict_predicates.as_secs_f64());
        cols.entry("dict_merge").or_default().push(st.dict_merge.as_secs_f64());
        cols.entry("scan_read").or_default().push(st.scan_read.as_secs_f64());
        cols.entry("scan_walk").or_default().push(st.scan_walk.as_secs_f64());
    }
    let med = |mut v: Vec<f64>| -> f64 {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let mut med_of = |k: &str| med(cols.remove(k).unwrap());
    let secs_dict = med_of("dict");
    // [OPUS-4.8] sq-7ge0 finer dict sub-stages. In a multi-threaded pool the four section
    // decodes run CONCURRENTLY, so these per-section rows overlap each other and do NOT sum
    // to `dict` — they show the relative cost of each section, not a wall partition.
    row(name, "  direct stage (a) dict decode", 1, secs_dict, bytes.len(), triples);
    row(name, "  direct stage (a1) dict: shared section decode", 1, med_of("dict_shared"), bytes.len(), triples);
    row(name, "  direct stage (a2) dict: subjects section decode", 1, med_of("dict_subjects"), bytes.len(), triples);
    row(name, "  direct stage (a3) dict: objects section decode", 1, med_of("dict_objects"), bytes.len(), triples);
    row(name, "  direct stage (a4) dict: predicates section decode", 1, med_of("dict_predicates"), bytes.len(), triples);
    row(name, "  direct stage (a5) dict: section-order merge", 1, med_of("dict_merge"), bytes.len(), triples);
    // [OPUS-4.8] sq-7ge0 finer scan sub-stages: (b1) + (b2) sum EXACTLY to (b) `scan`.
    row(name, "  direct stage (b) triple/id scan (id-translation)", 1, med_of("scan"), bytes.len(), triples);
    row(name, "  direct stage (b1) scan: triples-section read", 1, med_of("scan_read"), bytes.len(), triples);
    row(name, "  direct stage (b2) scan: SPO id-translation walk", 1, med_of("scan_walk"), bytes.len(), triples);
    row(name, "  direct stage (c) Graph::build", 1, med_of("build"), bytes.len(), triples);

    // --- UPSTREAM wavelet path, split into its two public stages. ---
    // Stage 1: Hdt::read = decode dict + BUILD the wavelet matrix / OP-index /
    // rank-select that the direct decoder never builds (the eliminated work).
    let secs_up_read = median(|| {
        let h = hdt::Hdt::read(std::io::BufReader::new(std::io::Cursor::new(&bytes[..]))).unwrap();
        black_box(h.triples.adjlist_z.sequence.entries);
    });
    row(name, "  upstream stage Hdt::read (decode+index build)", 1, secs_up_read, bytes.len(), triples);

    // Stage 2: graph_from_hdt = id-translation + Graph build (shared with direct).
    let read_once = hdt::Hdt::read(std::io::BufReader::new(std::io::Cursor::new(&bytes[..]))).unwrap();
    let secs_up_translate = median(|| {
        let g = sparq_hdt::graph_from_hdt(&read_once).unwrap();
        black_box(g.store.len());
    });
    drop(read_once);
    row(name, "  upstream stage graph_from_hdt (translate+build)", 1, secs_up_translate, bytes.len(), triples);

    // End-to-end upstream = read + translate (matches load_reader_via_upstream).
    let secs_upstream = median(|| {
        let h = hdt::Hdt::read(std::io::BufReader::new(std::io::Cursor::new(&bytes[..]))).unwrap();
        let g = sparq_hdt::graph_from_hdt(&h).unwrap();
        black_box(g.store.len());
    });
    row(name, "UPSTREAM total (Hdt::read + graph_from_hdt)", 1, secs_upstream, bytes.len(), triples);

    eprintln!("------------------------------------------------------------");
    eprintln!(
        "A/B SPEEDUP (load-robust): upstream {:.3}s / direct {:.3}s = {:.2}x faster",
        secs_upstream, secs_direct, secs_upstream / secs_direct
    );
    eprintln!(
        "  upstream split: Hdt::read(decode+index)={:.3}s + graph_from_hdt(translate)={:.3}s",
        secs_up_read, secs_up_translate
    );
    eprintln!(
        "  the direct decoder ELIMINATES the index-build portion of Hdt::read and shares the translate stage."
    );
    // VmHWM is a process-lifetime high-water mark, so an in-process pair would be
    // identical (both paths ran). For an HONEST per-path peak, re-load each path
    // ALONE in a fresh subprocess and read its VmHWM.
    match (rss_one_path(path, "direct"), rss_one_path(path, "upstream")) {
        (Some(d), Some(u)) => eprintln!(
            "  peak RSS (per-path, fresh process each): direct {:.0} MB, upstream {:.0} MB ({:.2}x)",
            d as f64 / 1e6, u as f64 / 1e6, u as f64 / d as f64
        ),
        _ => eprintln!("  peak RSS (per-path): unavailable (subprocess failed)"),
    }

    // [OPUS-4.8] (sq-q6a1) NT-vs-HDT A/B: the gap the sq-4wo plan headlines ("HDT
    // direct decode vs the fast parallel N-Triples loader"). Loads the SAME dataset's
    // `.nt` form via `Graph::load_reader_parallel` (the production fast NT path) and
    // emits an A/B row, so the comparison is measurable on a canonical runner. The
    // companion `.nt` is `<file.hdt>` with the `.hdt` suffix replaced by `.nt` (the
    // same NT used by `gen-hdt` to build the archive); absent => skip (not an error).
    let nt_path = match path.strip_suffix(".hdt") {
        Some(stem) => format!("{stem}.nt"),
        None => format!("{path}.nt"),
    };
    match std::fs::read(&nt_path) {
        Ok(nt_bytes) => {
            // Correctness gate: the NT load MUST yield the same triple count as the
            // HDT decode of the same data, else the A/B is comparing two datasets.
            let nt_g = sparq_core::Graph::load_reader_parallel(
                std::io::Cursor::new(&nt_bytes[..]),
                "ntriples",
            )
            .expect("NT parallel load");
            assert_eq!(
                nt_g.store.len(),
                triples,
                "NT-vs-HDT A/B triple-count mismatch ({} NT vs {} HDT) — refusing to report perf",
                nt_g.store.len(),
                triples
            );
            drop(nt_g);
            eprintln!("------------------------------------------------------------");
            eprintln!("NT-vs-HDT A/B: {} ({} NT bytes) via load_reader_parallel", nt_path, nt_bytes.len());
            // The NT row's MB/s is over the .nt byte count (its own input size); the
            // HDT direct row above is over .hdt bytes — the formats differ in size, so
            // compare Mtriples/s (or wall s) across the two, NOT raw MB/s.
            let secs_nt = median(|| {
                let g = sparq_core::Graph::load_reader_parallel(
                    std::io::Cursor::new(&nt_bytes[..]),
                    "ntriples",
                )
                .unwrap();
                black_box(g.store.len());
            });
            let ncpu = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
            row(name, "NT load_reader_parallel (fast NT path)", ncpu, secs_nt, nt_bytes.len(), triples);
            eprintln!(
                "  HDT direct {:.3}s vs NT parallel {:.3}s on {triples} triples = {:.2}x ({})",
                secs_direct,
                secs_nt,
                secs_nt / secs_direct,
                if secs_nt > secs_direct { "HDT direct faster" } else { "NT parallel faster" }
            );
        }
        Err(_) => eprintln!(
            "NT-vs-HDT A/B: skipped — companion {nt_path} absent (point bench-hdt at <data.hdt> alongside its <data.nt>)"
        ),
    }
}

/// Re-runs THIS binary's `hdt-rss <path> <which>` in a fresh process so VmHWM
/// reflects only that single load path; parses the printed byte count.
fn rss_one_path(path: &str, which: &str) -> Option<u64> {
    let exe = std::env::current_exe().ok()?;
    let out = std::process::Command::new(exe).args(["hdt-rss", path, which]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse::<u64>().ok()
}

/// Loads exactly one path once, prints its peak-RSS byte count to stdout.
fn hdt_rss(path: &str, which: &str) {
    let bytes = std::fs::read(path).unwrap();
    match which {
        "direct" => {
            let g = sparq_hdt::graph_from_reader(std::io::Cursor::new(&bytes[..])).unwrap();
            black_box(g.store.len());
        }
        _ => {
            let h = hdt::Hdt::read(std::io::BufReader::new(std::io::Cursor::new(&bytes[..]))).unwrap();
            let g = sparq_hdt::graph_from_hdt(&h).unwrap();
            black_box(g.store.len());
        }
    }
    println!("{}", peak_rss_bytes());
}

/// Decompress+parse MB/s of the compressed HDT containers via the direct decoder
/// (the H5 streaming path). Expects `<file.hdt>.{gz,zst,bz2}` alongside the plain
/// `.hdt`; MB/s is over the DECOMPRESSED `.hdt` byte count.
fn bench_hdt_zip(path: &str) {
    let plain = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let name = dataset_name(path);
    let triples = sparq_hdt::graph_from_reader(std::io::Cursor::new(&plain[..]))
        .expect("plain decode")
        .store
        .len();

    println!("| dataset | task | s | MB/s (decompressed .hdt) | Mtriples/s |");
    println!("|---|---|---|---|---|");

    for ext in ["gz", "zst", "bz2"] {
        let cpath = format!("{path}.{ext}");
        let Ok(comp) = std::fs::read(&cpath) else {
            eprintln!("skipping {ext}: {cpath} absent (run `compress-hdt` or gzip/zstd/bzip2 the .hdt)");
            continue;
        };
        let ratio = plain.len() as f64 / comp.len() as f64;
        let secs = median(|| {
            // load_reader sniffs the container by magic bytes and streams the
            // decode through the direct decoder — exactly the production path.
            let g = sparq_hdt::load_reader(std::io::Cursor::new(&comp[..])).unwrap();
            black_box(g.store.len());
        });
        row(name, &format!("{ext} decode+parse (direct, {ratio:.2}x compressed)"), 1, secs, plain.len(), triples);
    }
}

// ---------------------------------------------------------------------------
// [SONNET-4.6] bench-ext — external parser competitor columns (sq-hmd7l.6)
// ---------------------------------------------------------------------------
//
// Invokes serdi (serd), rapper (raptor2), and Jena riot as subprocesses over
// the SAME corpus file used by bench-nt / bench-ttl.  Key design invariants:
//
//  REGIME LABEL  Every output row carries the substring "subprocess" in the
//                task column so the reader knows timing includes process spawn,
//                kernel file-I/O, parse, and process exit — NOT comparable to
//                in-process "parse-only" rows that exclude all three costs.
//
//  COUNT GUARD   Before printing any MB/s row, the tool's reported or
//                observable triple count is cross-checked against the oxttl
//                authoritative count for the same file.  A mismatch is printed
//                to stderr and the row is suppressed — no fabricated numbers.
//
//  ABSENT TOOL   `tool_available()` probes PATH with `which`; a missing tool
//                prints a diagnostic to stderr and the column is absent.  The
//                suite exits 0 with no rows when all three are absent.
//
//  NON-CANONICAL These numbers come from the shared work box, not a quiet EC2
//                instance.  The first-read gap record (research/gap-parse-2026-07.md)
//                is flagged NON-canonical; canonical rows ride sq-hmd7l.26.

/// Returns `true` if `tool` is found on PATH (via `which`).
fn tool_available(tool: &str) -> bool {
    std::process::Command::new("which")
        .arg(tool)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Runs `argv[0]` with `argv[1..]` ITERS times, capturing all output (stdout
/// + stderr concatenated).
///
/// Returns `(min_wall_secs, combined_output_of_best_run)`,
/// or `None` on launch failure or non-zero exit.
fn subprocess_time_and_output(argv: &[&str]) -> Option<(f64, String)> {
    if argv.is_empty() {
        return None;
    }
    let mut min_secs = f64::MAX;
    let mut best_output = String::new();
    for _ in 0..ITERS {
        let t = Instant::now();
        let out = std::process::Command::new(argv[0])
            .args(&argv[1..])
            .output()
            .ok()?;
        let secs = t.elapsed().as_secs_f64();
        if !out.status.success() {
            eprintln!("warn: {} exited {:?}", argv[0], out.status);
            return None;
        }
        if secs < min_secs {
            min_secs = secs;
            best_output = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
            );
        }
    }
    (min_secs < f64::MAX).then_some((min_secs, best_output))
}

/// Runs `argv[0]` with `argv[1..]` ITERS times, discarding stdout + stderr.
/// Returns the minimum wall-clock seconds, or `None` on failure.
fn subprocess_time_sink(argv: &[&str]) -> Option<f64> {
    if argv.is_empty() {
        return None;
    }
    let mut min_secs = f64::MAX;
    for _ in 0..ITERS {
        let t = Instant::now();
        let status = std::process::Command::new(argv[0])
            .args(&argv[1..])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?;
        let secs = t.elapsed().as_secs_f64();
        if !status.success() {
            eprintln!("warn: {} exited {:?}", argv[0], status);
            return None;
        }
        min_secs = min_secs.min(secs);
    }
    (min_secs < f64::MAX).then_some(min_secs)
}

/// Prints one external-tool row.
///
/// Pre-condition: caller has already verified count agreement (INVARIANT: no
/// MB/s row without triple-count agreement vs the corpus's known count).
/// The `count-ok` column always shows `yes` because this function is only
/// reachable after the count guard passes.
fn ext_row(dataset: &str, task: &str, secs: f64, bytes: usize, triples: usize) {
    let mbs = bytes as f64 / 1e6 / secs;
    let mts = triples as f64 / 1e6 / secs;
    println!("| {dataset} | {task} | 1 (subprocess) | {secs:.3} | {mbs:.0} | {mts:.2} | yes |");
    json_push(vec![
        ("dataset", json_str(dataset)),
        ("task", json_str(task)),
        ("regime", json_str("subprocess")),
        ("secs", format!("{secs:.6}")),
        ("bytes", bytes.to_string()),
        ("triples", triples.to_string()),
        ("mb_s", format!("{mbs:.3}")),
        ("mtriples_s", format!("{mts:.3}")),
        ("count_ok", "true".to_string()),
    ]);
}

/// Extract the triple count from `riot --count` output: `"<file> : Triples = N"`
/// (field name padding varies by version, and riot 5.x prints the count WITH
/// grouping separators — `"Triples = 2,560,000"` observed from riot 5.4.0 on the
/// wave-1 canonical box, which wrongly suppressed the row until the commas were
/// stripped).
fn parse_riot_count(output: &str) -> Option<usize> {
    output.lines().find_map(|line| {
        let pos = line.find("Triples")?;
        let after = &line[pos..];
        let eq = after.find('=')?;
        after[eq + 1..]
            .split_whitespace()
            .next()
            .and_then(|s| s.replace(',', "").parse::<usize>().ok())
    })
}

/// External parser competitor measurements (`bench-ext <file>`).
///
/// Detects format from the file extension (`.nt` → N-Triples, `.ttl` /
/// `.turtle` → Turtle) and invokes serdi, rapper, and Jena riot as
/// subprocesses.  Each tool is timed min-of-ITERS; triple count is
/// cross-checked before any MB/s row is printed.
///
/// The table header has an extra `count-ok` column vs bench-nt / bench-ttl
/// to make the count-guard contract visible in the output.
fn bench_ext(path: &str) {
    let is_nt = path.ends_with(".nt");
    let is_ttl = path.ends_with(".ttl") || path.ends_with(".turtle");
    if !is_nt && !is_ttl {
        eprintln!(
            "bench-ext: unrecognised extension in {path} — expected .nt or .ttl"
        );
        std::process::exit(2);
    }
    let format_name = if is_nt { "N-Triples" } else { "Turtle" };
    let rapper_fmt = if is_nt { "ntriples" } else { "turtle" };

    // Raw bytes drive the MB/s denominator.
    let text = read_to_string(path);
    let bytes_slice = text.as_bytes();
    let name = dataset_name(path);

    // Authoritative triple count via oxttl — same oracle as bench-nt / bench-ttl.
    let expected: usize = if is_nt {
        oxttl::NTriplesParser::new()
            .for_slice(bytes_slice)
            .fold(0usize, |acc, r| {
                r.expect("dataset must parse");
                acc + 1
            })
    } else {
        oxttl::TurtleParser::new()
            .for_slice(bytes_slice)
            .fold(0usize, |acc, r| {
                r.expect("dataset must parse");
                acc + 1
            })
    };
    let nbytes = bytes_slice.len();
    eprintln!(
        "{name}: {nbytes} bytes, {expected} triples ({format_name}, authoritative oxttl count)"
    );

    // Table for external tools: same columns as bench-nt plus `count-ok`.
    println!(
        "| dataset | task | threads | s (min-of-{ITERS}) | MB/s | Mtriples/s | count-ok |"
    );
    println!("|---|---|---|---|---|---|---|");

    // Absolute path so tools that require a URI or absolute path work correctly.
    let abs = std::fs::canonicalize(path)
        .unwrap_or_else(|_| std::path::PathBuf::from(path));
    let abs_str = abs.to_str().unwrap();

    // ── rapper / raptor2 ──────────────────────────────────────────────────────
    //
    // Mode: `rapper -c -i <fmt> <file>`
    //   `-c`  count-only; parses without serializing output.  Closest available
    //         equivalent to "parse-only" for an external tool.
    //   Without `-q`  so the count line appears on stderr:
    //         "rapper: Parsing returned N triples"
    // Regime: subprocess (spawn + file-I/O + parse), count-only (no serialization,
    //         no store build).
    // Note: `-q` (quiet) suppresses the count line entirely — do NOT add it.
    if tool_available("rapper") {
        let task = format!(
            "rapper/raptor2 {format_name} (subprocess, count-only, no store)"
        );
        match subprocess_time_and_output(&["rapper", "-c", "-i", rapper_fmt, abs_str]) {
            Some((secs, output)) => {
                // Count is on stderr: "rapper: Parsing returned N triples"
                let parsed_count = output.lines().find_map(|line| {
                    let rest = line.strip_prefix("rapper: Parsing returned ")?;
                    rest.strip_suffix(" triples")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                });
                match parsed_count {
                    Some(c) if c == expected => {
                        ext_row(name, &task, secs, nbytes, expected);
                    }
                    Some(c) => {
                        eprintln!(
                            "WARN: rapper reported {c} triples, expected {expected} \
                             — suppressing MB/s row (INVARIANT: no row without count agreement)"
                        );
                    }
                    None => {
                        eprintln!(
                            "WARN: could not parse triple count from rapper output \
                             — suppressing MB/s row; raw output: {:?}",
                            &output[..output.len().min(200)]
                        );
                    }
                }
            }
            None => eprintln!("warn: rapper invocation failed — skipping column"),
        }
    } else {
        eprintln!("rapper: not found on PATH — column absent");
    }

    // ── serdi / serd ──────────────────────────────────────────────────────────
    //
    // No built-in count mode: we pipe stdout (NT output) through `wc -l` via
    // a shell pipeline for count verification, then time ITERS sink runs.
    // Regime: subprocess (spawn + file-I/O + parse + NT serialization to sink).
    // The count-check run uses a separate shell pipeline; the timing runs
    // redirect stdout to /dev/null to avoid the serialization cost distorting
    // the timed measurement (the task label says "serialize-to-sink" to be
    // honest about what the timing includes).
    if tool_available("serdi") {
        let task = format!(
            "serdi/serd {format_name} (subprocess, parse+serialize-to-sink)"
        );
        // Count check via `serdi <file> | wc -l`.
        let count_result: Option<usize> = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("serdi '{}' | wc -l", abs_str))
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8_lossy(&o.stdout)
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<usize>().ok())
                } else {
                    None
                }
            });
        match count_result {
            None => {
                eprintln!("warn: could not verify serdi triple count — skipping column");
            }
            Some(c) if c != expected => {
                eprintln!(
                    "WARN: serdi line-count {c} != expected {expected} \
                     — suppressing MB/s row (INVARIANT: no row without count agreement)"
                );
            }
            Some(_) => {
                // Count verified; time ITERS runs with stdout discarded.
                match subprocess_time_sink(&["serdi", abs_str]) {
                    Some(secs) => ext_row(name, &task, secs, nbytes, expected),
                    None => eprintln!("warn: serdi timing run failed — skipping column"),
                }
            }
        }
    } else {
        eprintln!("serdi: not found on PATH — column absent");
    }

    // ── Jena riot ─────────────────────────────────────────────────────────────
    //
    // Mode: `riot --count <file>`
    //   `--count`  parse and count without writing output.
    //   Count appears in combined output: "Triples = N" or
    //   "INFO  riot  :: Triples      = N" depending on riot version.
    // Regime: subprocess (spawn + file-I/O + parse), count-mode (no serialization,
    //         no store build).
    // Fallback: if `--count` is unavailable (older riot), the column is absent;
    //           we do NOT fall back to a sink run because count cannot then be
    //           verified (INVARIANT).
    if tool_available("riot") {
        let task = format!(
            "Jena riot {format_name} (subprocess, --count, no store)"
        );
        match subprocess_time_and_output(&["riot", "--count", abs_str]) {
            Some((secs, output)) => {
                let parsed_count = parse_riot_count(&output);
                match parsed_count {
                    Some(c) if c == expected => {
                        ext_row(name, &task, secs, nbytes, expected);
                    }
                    Some(c) => {
                        eprintln!(
                            "WARN: riot reported {c} triples, expected {expected} \
                             — suppressing MB/s row (INVARIANT: no row without count agreement)"
                        );
                    }
                    None => {
                        eprintln!(
                            "WARN: could not parse triple count from riot --count output \
                             — suppressing MB/s row; raw output: {:?}",
                            &output[..output.len().min(200)]
                        );
                    }
                }
            }
            None => {
                // --count failed (old riot or unsupported flag).  Count cannot be
                // verified from a sink run, so per the INVARIANT we skip entirely.
                eprintln!(
                    "warn: riot --count failed (unsupported flag or error) \
                     — column absent (count verification not possible in fallback mode)"
                );
            }
        }
    } else {
        eprintln!("riot: not found on PATH — column absent");
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-ghjc) --json emit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: riot 5.4.0 prints grouping separators ("Triples = 2,560,000",
    /// verbatim wave-1 canonical-box output) — the count must parse, or the riot
    /// row is wrongly suppressed. Plain and padded forms must keep working.
    #[test]
    fn riot_count_parses_grouping_separators() {
        assert_eq!(
            parse_riot_count("/root/sparq/bench/parse/data/synthetic.nt : Triples = 2,560,000\n"),
            Some(2_560_000)
        );
        assert_eq!(parse_riot_count("Triples = 400000"), Some(400_000));
        assert_eq!(
            parse_riot_count("INFO  riot  :: Triples      = 328"),
            Some(328)
        );
        assert_eq!(parse_riot_count("no count here"), None);
    }

    /// `--json <path>` is stripped from the positional argv (so the subcommands'
    /// positional indexing is unaffected) and its value is returned; the escaper
    /// produces RFC-valid JSON strings; and the full emit round-trips through a
    /// real `serde_json` parse with the documented shape + keys. One test only,
    /// because the collector is a process-global `OnceLock` (parallel tests would
    /// race its `rows`); this drives the whole emit path end-to-end in isolation.
    #[test]
    fn json_flag_and_emit_round_trip() {
        // --- flag extraction keeps positional args intact -------------------------
        let argv: Vec<String> = ["parse-baseline", "bench-nt", "data.nt", "--json", "/tmp/out.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (positional, path) = take_json_flag(&argv);
        assert_eq!(positional, vec!["parse-baseline", "bench-nt", "data.nt"]);
        assert_eq!(path.as_deref(), Some("/tmp/out.json"));
        // Absent flag -> args unchanged, no path.
        let (p2, none) = take_json_flag(&argv[..3]);
        assert_eq!(p2, argv[..3]);
        assert!(none.is_none());

        // --- string escaper produces valid JSON strings ---------------------------
        assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
        assert_eq!(json_str("plain"), "\"plain\"");

        // --- full emit round-trips through serde_json -----------------------------
        json_init("bench-nt", "data.nt");
        let tmp = std::env::temp_dir().join(format!("sq-ghjc-emit-{}.json", std::process::id()));
        json_set_path(tmp.to_str().unwrap());
        // A standard table row (the `row()` shape) ...
        json_push(vec![
            ("dataset", json_str("data.nt")),
            ("task", json_str("oxttl NT parse-only")),
            ("threads", 1.to_string()),
            ("secs", format!("{:.6}", 0.5_f64)),
            ("bytes", 100usize.to_string()),
            ("triples", 4usize.to_string()),
            ("mb_s", format!("{:.3}", 0.2_f64)),
            ("mtriples_s", format!("{:.3}", 0.008_f64)),
        ]);
        // ... and a heterogeneous (intern-A/B) row to prove the shape is per-row.
        json_push(vec![
            ("dataset", json_str("data.nt")),
            ("strategy", json_str("ratio old/new")),
            ("ratio_old_new", format!("{:.3}", 1.25_f64)),
        ]);
        json_flush();

        let doc = std::fs::read_to_string(&tmp).expect("emit file written");
        let _ = std::fs::remove_file(&tmp);
        let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");

        assert_eq!(v["harness"], "bench/parse parse-baseline");
        assert_eq!(v["subcommand"], "bench-nt");
        assert_eq!(v["dataset"], "data.nt");
        assert_eq!(v["iters"], ITERS);
        assert!(v["note"].as_str().unwrap().contains("NON-CANONICAL"));

        let rows = v["rows"].as_array().expect("rows is an array");
        assert_eq!(rows.len(), 2);
        // Standard row keys.
        let r0 = &rows[0];
        assert_eq!(r0["task"], "oxttl NT parse-only");
        assert_eq!(r0["threads"], 1);
        assert_eq!(r0["bytes"], 100);
        assert_eq!(r0["triples"], 4);
        assert!(r0["secs"].is_number());
        assert!(r0["mb_s"].is_number());
        assert!(r0["mtriples_s"].is_number());
        // Heterogeneous row keeps its own keys (no leakage from the table shape).
        let r1 = &rows[1];
        assert_eq!(r1["strategy"], "ratio old/new");
        assert!(r1["ratio_old_new"].is_number());
        assert!(r1.get("threads").is_none());
    }
}
