//! (sq-hmd7l.14) Serialization-throughput runner for `bench/serialize/`
//! — drives the PUBLIC writer matrix of this crate (buffered / streaming / pretty)
//! so the same-box comparison panel (vs serd / rapper / Jena riot / an oxrdfio
//! scratch shim) exercises exactly the surface sparq ships.
//!
//! ```sh
//! cargo build --release -p sparq-engine-serialize \
//!     --features serialize-rdf,streaming-serialization --example serialize_bench
//! target/release/examples/serialize_bench gen 2000 corpus.nt
//! target/release/examples/serialize_bench bench corpus.nt --formats nt,turtle --iters 3
//! target/release/examples/serialize_bench pipe corpus.nt turtle out.ttl
//! target/release/examples/serialize_bench check corpus.nt out.ttl turtle
//! ```
//!
//! Modes:
//! - `gen <n_subjects> <out.nt>` — deterministic synthetic N-Triples corpus
//!   (LCG-seeded; ~8 triples/subject; IRIs plus plain / lang-tagged / typed and
//!   escape-heavy literals; **no blank nodes**, so the round-trip gate can be an
//!   exact set comparison instead of graph isomorphism). Never committed.
//! - `bench <corpus.nt> [--formats <csv>] [--iters <k>] [--json <path>]` — the
//!   **in-process serialize-only regime**: load the corpus ONCE, then per format
//!   run the ROUND-TRIP GATE (serialize → re-parse → exact match vs the loaded
//!   store) and only if green time `k` iterations (min-of-k wall, MB/s over
//!   output bytes). Buffered vs streaming vs pretty rows are labelled. NOT
//!   comparable to process-wall CLI columns — see `bench/serialize/README.md`.
//! - `pipe <input.nt> <format> <out-file>` — the **pipeline regime** primitive:
//!   parse + serialize + write in one shot, for external process-wall timing by
//!   `run.sh` under the same measurement as the competitor CLIs.
//! - `check <original.nt> <emitted-file> <format>` — the cross-engine round-trip
//!   gate: parse both, compare triple count + exact sorted canonical N-Quads
//!   lines. Used to gate every competitor output before its MB/s row is trusted.
//!
//! Exit codes (the `bench/serialize/run.sh` contract):
//! - `0` — ok / gate green.
//! - `3` — input failed to parse.
//! - `4` — round-trip MISMATCH (the gate is red; no timing may be trusted).
//! - `5` — blank-node-bearing input refused fail-closed (the exact-match gate
//!   does not do bnode isomorphism; `gen` corpora never contain blank nodes).
//! - `1` — usage / IO error.
//!
//! Timings printed here are whatever this machine measured — NON-canonical
//! (work-box numbers are never transcribed into docs; see `bench/CATALOG.md`).

use sparq_core::Graph;
use sparq_engine_serialize::serialize::{
    default_prefixes, graph_to_nquads, graph_to_trig_streaming, graph_to_trig_with,
    graph_to_turtle_pretty_with, graph_to_turtle_streaming, graph_to_turtle_with, Prefixes,
    PrettyOptions,
};
use std::io::Write;
use std::time::Instant;

fn usage() -> ! {
    eprintln!(
        "usage: serialize_bench gen <n_subjects> <out.nt>\n\
         \x20      serialize_bench bench <corpus.nt> [--formats <csv>] [--iters <k>] [--json <path>]\n\
         \x20      serialize_bench pipe <input.nt> <format> <out-file>\n\
         \x20      serialize_bench check <original.nt> <emitted-file> <format>\n\
         formats: nt turtle turtle-pretty turtle-stream trig trig-stream\n\
         exit: 0 ok | 3 parse error | 4 round-trip mismatch | 5 blank-node input refused | 1 usage/IO"
    );
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen") => run_gen(&args[1..]),
        Some("bench") => run_bench(&args[1..]),
        Some("pipe") => run_pipe(&args[1..]),
        Some("check") => run_check(&args[1..]),
        _ => usage(),
    }
}

// ---------------------------------------------------------------------------
// Formats under test.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    /// `write_nquads` over the store; on a default-graph-only corpus the output
    /// is plain N-Triples (no graph column), so the row is labelled `nt`.
    Nt,
    Turtle,
    TurtlePretty,
    TurtleStream,
    Trig,
    TrigStream,
}

impl Format {
    fn parse(s: &str) -> Option<Format> {
        Some(match s {
            "nt" | "nquads" => Format::Nt,
            "turtle" => Format::Turtle,
            "turtle-pretty" => Format::TurtlePretty,
            "turtle-stream" => Format::TurtleStream,
            "trig" => Format::Trig,
            "trig-stream" => Format::TrigStream,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Format::Nt => "nt",
            Format::Turtle => "turtle",
            Format::TurtlePretty => "turtle-pretty",
            Format::TurtleStream => "turtle-stream",
            Format::Trig => "trig",
            Format::TrigStream => "trig-stream",
        }
    }

    /// buffered = the writer builds the whole output `String`; streaming = the
    /// writer flushes per subject block into an `io::Write` sink.
    fn regime(self) -> &'static str {
        match self {
            Format::TurtleStream | Format::TrigStream => "streaming",
            Format::TurtlePretty => "buffered-pretty",
            _ => "buffered",
        }
    }

    /// The sparq-core parser format string that re-reads this writer's output.
    fn reparse_as(self) -> &'static str {
        match self {
            Format::Nt => "nquads",
            Format::Turtle | Format::TurtlePretty | Format::TurtleStream => "turtle",
            Format::Trig | Format::TrigStream => "trig",
        }
    }

    fn all() -> &'static [Format] {
        &[
            Format::Nt,
            Format::Turtle,
            Format::TurtlePretty,
            Format::TurtleStream,
            Format::Trig,
            Format::TrigStream,
        ]
    }
}

/// Serializes `g` in `fmt`, returning the full output bytes (used once per
/// format for the round-trip gate and byte count — timing may instead use
/// [`serialize_counting`] so streaming rows do not pay a `Vec` append).
fn serialize_full(g: &Graph, fmt: Format, prefixes: &Prefixes) -> Vec<u8> {
    match fmt {
        Format::Nt => graph_to_nquads(g).into_bytes(),
        Format::Turtle => graph_to_turtle_with(g, prefixes).into_bytes(),
        Format::TurtlePretty => {
            graph_to_turtle_pretty_with(g, prefixes, &PrettyOptions::default()).into_bytes()
        }
        Format::TurtleStream => {
            let mut buf = Vec::new();
            graph_to_turtle_streaming(g, prefixes, &mut buf).expect("write to Vec cannot fail");
            buf
        }
        Format::Trig => graph_to_trig_with(g, prefixes).into_bytes(),
        Format::TrigStream => {
            let mut buf = Vec::new();
            graph_to_trig_streaming(g, prefixes, &mut buf).expect("write to Vec cannot fail");
            buf
        }
    }
}

/// A byte-counting `io::Write` sink: the streaming rows time real writer work
/// without accumulating the document (that is the point of the streaming regime).
struct CountSink(u64);

impl Write for CountSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// One timed serialization; returns the output byte count of this run.
fn serialize_counting(g: &Graph, fmt: Format, prefixes: &Prefixes) -> u64 {
    match fmt {
        Format::TurtleStream => {
            let mut sink = CountSink(0);
            graph_to_turtle_streaming(g, prefixes, &mut sink).expect("count sink cannot fail");
            sink.0
        }
        Format::TrigStream => {
            let mut sink = CountSink(0);
            graph_to_trig_streaming(g, prefixes, &mut sink).expect("count sink cannot fail");
            sink.0
        }
        // Buffered rows: build (and drop) the whole String, like a real caller.
        _ => serialize_full(g, fmt, prefixes).len() as u64,
    }
}

// ---------------------------------------------------------------------------
// Round-trip gate.
// ---------------------------------------------------------------------------

/// The canonical comparison form: every triple/quad rendered as one N-Quads line
/// (oxrdf's canonical term `Display` — the same byte form the parsers accept),
/// sorted. Exact multiset equality on these lines is the round-trip oracle for
/// blank-node-free data.
fn canonical_lines(g: &Graph) -> Vec<String> {
    let mut lines: Vec<String> = graph_to_nquads(g).lines().map(str::to_string).collect();
    lines.sort_unstable();
    lines
}

/// Fail-closed guard: the exact-match gate cannot judge blank-node round-trips
/// (labels may legitimately differ — that needs isomorphism, which is the
/// `canon-bench` axis's business). Subject / object / graph tokens starting
/// `_:` trip it; literal objects start with `"` so they cannot false-positive.
fn contains_blank_nodes(lines: &[String]) -> bool {
    lines.iter().any(|l| {
        let mut toks = l.split_ascii_whitespace();
        let s = toks.next().unwrap_or("");
        let _p = toks.next();
        let o = toks.next().unwrap_or("");
        let g = toks.next().unwrap_or("");
        s.starts_with("_:") || o.starts_with("_:") || g.starts_with("_:")
    })
}

/// The gate: `emitted` (a document in `fmt`) must re-parse to exactly the same
/// store as `original`. Returns the first divergence on mismatch.
fn round_trip_gate(original_lines: &[String], emitted: &str, fmt: Format) -> Result<(), String> {
    let reparsed = parse_doc(emitted, fmt.reparse_as())
        .map_err(|e| format!("re-parse as {} failed: {}", fmt.reparse_as(), e))?;
    let got = canonical_lines(&reparsed);
    if got.len() != original_lines.len() {
        return Err(format!(
            "triple count mismatch: original {} vs re-parsed {}",
            original_lines.len(),
            got.len()
        ));
    }
    for (a, b) in original_lines.iter().zip(got.iter()) {
        if a != b {
            return Err(format!(
                "content mismatch; first divergence:\n  original:  {}\n  re-parsed: {}",
                a, b
            ));
        }
    }
    Ok(())
}

fn parse_doc(text: &str, format: &str) -> Result<Graph, String> {
    match format {
        "nquads" | "trig" => Graph::load_dataset(text, format),
        other => Graph::load_str(text, other),
    }
}

fn load_corpus(path: &str) -> Graph {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("serialize_bench: read {}: {}", path, e);
            std::process::exit(1);
        }
    };
    match Graph::load_str(&text, "ntriples") {
        Ok(g) => g,
        Err(e) => {
            eprintln!("serialize_bench: parse {}: {}", path, e);
            std::process::exit(3);
        }
    }
}

/// The corpus vocabulary prefixes (matching `gen`'s namespaces) layered over the
/// well-known defaults, so Turtle / TriG output exercises real prefix compaction.
fn bench_prefixes() -> Prefixes {
    let mut p = default_prefixes();
    p.insert("sb".to_string(), "http://sparq.dev/bench/serialize/".to_string());
    p.insert("sbv".to_string(), "http://sparq.dev/bench/serialize/vocab#".to_string());
    p
}

// ---------------------------------------------------------------------------
// gen — deterministic blank-node-free corpus.
// ---------------------------------------------------------------------------

/// Small deterministic LCG (Knuth MMIX constants) — no rand dependency.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn run_gen(args: &[String]) -> ! {
    let (n, out) = match args {
        [n, out] => (n, out),
        _ => usage(),
    };
    let n: u64 = match n.parse() {
        Ok(v) => v,
        Err(_) => usage(),
    };
    let file = match std::fs::File::create(out) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("serialize_bench: create {}: {}", out, e);
            std::process::exit(1);
        }
    };
    let mut w = std::io::BufWriter::new(file);
    let mut rng = Lcg(0x5eed_5eed_5eed_5eed);
    let mut triples: u64 = 0;
    const V: &str = "http://sparq.dev/bench/serialize/vocab#";
    const E: &str = "http://sparq.dev/bench/serialize/";
    let types = ["Person", "Org", "Place", "Event"];
    let langs = ["en", "de", "fr", "ja"];
    for i in 0..n {
        let s = format!("<{}s/{}>", E, i);
        let ty = types[(rng.next() % 4) as usize];
        let mut emit = |p: &str, o: String| {
            writeln!(w, "{} <{}{}> {} .", s, V, p, o).expect("corpus write failed");
            triples += 1;
        };
        emit("type-of", format!("<{}{}>", V, ty)); // plain IRI object
        emit("name", format!("\"entity {} of kind {}\"", i, ty));
        emit(
            "label",
            format!("\"labelled {}\"@{}", i, langs[(rng.next() % 4) as usize]),
        );
        emit(
            "rank",
            format!(
                "\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                rng.next() % 100_000
            ),
        );
        emit(
            "score",
            format!(
                "\"{}.{:04}\"^^<http://www.w3.org/2001/XMLSchema#double>",
                rng.next() % 1000,
                rng.next() % 10_000
            ),
        );
        emit(
            "stamp",
            format!(
                "\"20{:02}-{:02}-{:02}T{:02}:{:02}:{:02}Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>",
                rng.next() % 30,
                1 + rng.next() % 12,
                1 + rng.next() % 28,
                rng.next() % 24,
                rng.next() % 60,
                rng.next() % 60
            ),
        );
        // Escape-heavy literal: quotes, backslash, newline, tab (as NT escapes).
        emit(
            "note",
            format!("\"say \\\"{}\\\" \\\\ line\\nbreak\\ttab\"", rng.next() % 997),
        );
        emit("knows", format!("<{}s/{}>", E, rng.next() % n.max(1))); // IRI link
    }
    w.flush().expect("corpus flush failed");
    println!("wrote {} triples ({} subjects) to {}", triples, n, out);
    std::process::exit(0);
}

// ---------------------------------------------------------------------------
// bench — in-process serialize-only regime (gate, then min-of-k).
// ---------------------------------------------------------------------------

fn run_bench(args: &[String]) -> ! {
    let mut corpus: Option<&String> = None;
    let mut formats: Vec<Format> = Format::all().to_vec();
    let mut iters: u32 = 5;
    let mut json_out: Option<&String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--formats" => {
                let list = it.next().unwrap_or_else(|| usage());
                formats = list
                    .split(',')
                    .map(|s| Format::parse(s.trim()).unwrap_or_else(|| usage()))
                    .collect();
            }
            "--iters" => {
                iters = it
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| usage());
            }
            "--json" => json_out = Some(it.next().unwrap_or_else(|| usage())),
            _ if corpus.is_none() => corpus = Some(a),
            _ => usage(),
        }
    }
    let corpus = corpus.unwrap_or_else(|| usage());
    if iters == 0 {
        usage();
    }

    let t = Instant::now();
    let g = load_corpus(corpus);
    let load_us = t.elapsed().as_micros();
    let original = canonical_lines(&g);
    if contains_blank_nodes(&original) {
        eprintln!(
            "serialize_bench: {} contains blank nodes — the exact-match round-trip gate \
             refuses fail-closed (use a blank-node-free corpus, e.g. `gen`)",
            corpus
        );
        std::process::exit(5);
    }
    let prefixes = bench_prefixes();
    eprintln!(
        "loaded {} triples from {} in {}us (load time is NOT part of any row below)",
        original.len(),
        corpus,
        load_us
    );

    println!("| format | regime | round-trip | out MB | min wall (us) x{} | MB/s |", iters);
    println!("|---|---|---|---|---|---|");
    let mut rows: Vec<String> = Vec::new();
    for &fmt in &formats {
        // 1. GATE before any timing.
        let emitted = serialize_full(&g, fmt, &prefixes);
        let text = String::from_utf8(emitted).expect("writers emit UTF-8");
        if let Err(e) = round_trip_gate(&original, &text, fmt) {
            eprintln!("serialize_bench: ROUND-TRIP GATE RED for {}: {}", fmt.label(), e);
            std::process::exit(4);
        }
        let bytes = text.len() as u64;
        drop(text);
        // 2. min-of-k timing.
        let mut min_us = u128::MAX;
        for _ in 0..iters {
            let t = Instant::now();
            let n = serialize_counting(&g, fmt, &prefixes);
            let us = t.elapsed().as_micros();
            assert_eq!(n, bytes, "output byte count changed between runs");
            min_us = min_us.min(us);
        }
        let mb = bytes as f64 / 1e6;
        let mbps = if min_us > 0 { mb / (min_us as f64 / 1e6) } else { f64::INFINITY };
        println!(
            "| {} | {} | green | {:.2} | {} | {:.1} |",
            fmt.label(),
            fmt.regime(),
            mb,
            min_us,
            mbps
        );
        rows.push(format!(
            "{{\"format\":\"{}\",\"regime\":\"{}\",\"round_trip\":\"green\",\"out_bytes\":{},\"min_wall_us\":{},\"mb_per_s\":{:.1}}}",
            fmt.label(),
            fmt.regime(),
            bytes,
            min_us,
            mbps
        ));
    }
    if let Some(path) = json_out {
        // Dependency-free JSON emit (mirrors bench/parse's --json): the machine-
        // readable twin of the table above. Numbers are this box's — non-canonical.
        let doc = format!(
            "{{\n  \"suite\": \"serialize-bench\",\n  \"regime\": \"in-process serialize-only (store pre-loaded; min-of-{} wall)\",\n  \"corpus_triples\": {},\n  \"note\": \"work-box numbers; NON-canonical\",\n  \"rows\": [\n    {}\n  ]\n}}\n",
            iters,
            original.len(),
            rows.join(",\n    ")
        );
        if let Err(e) = std::fs::write(path, doc) {
            eprintln!("serialize_bench: write {}: {}", path, e);
            std::process::exit(1);
        }
        eprintln!("json written to {}", path);
    }
    std::process::exit(0);
}

// ---------------------------------------------------------------------------
// pipe — parse + serialize + write, one shot (externally timed).
// ---------------------------------------------------------------------------

fn run_pipe(args: &[String]) -> ! {
    let (input, fmt, out) = match args {
        [i, f, o] => (i, f, o),
        _ => usage(),
    };
    let fmt = Format::parse(fmt).unwrap_or_else(|| usage());
    let t = Instant::now();
    let g = load_corpus(input);
    let parse_us = t.elapsed().as_micros();
    let t = Instant::now();
    let emitted = serialize_full(&g, fmt, &bench_prefixes());
    let ser_us = t.elapsed().as_micros();
    if let Err(e) = std::fs::write(out, &emitted) {
        eprintln!("serialize_bench: write {}: {}", out, e);
        std::process::exit(1);
    }
    // Advisory in-process split on stderr; run.sh measures the process wall.
    eprintln!("parse_us={}", parse_us);
    eprintln!("ser_us={}", ser_us);
    eprintln!("out_bytes={}", emitted.len());
    std::process::exit(0);
}

// ---------------------------------------------------------------------------
// check — the cross-engine round-trip gate.
// ---------------------------------------------------------------------------

fn run_check(args: &[String]) -> ! {
    let (orig_path, emitted_path, fmt) = match args {
        [a, b, f] => (a, b, f),
        _ => usage(),
    };
    let fmt = Format::parse(fmt).unwrap_or_else(|| usage());
    let g = load_corpus(orig_path);
    let original = canonical_lines(&g);
    if contains_blank_nodes(&original) {
        eprintln!("serialize_bench: original contains blank nodes — gate refuses fail-closed");
        std::process::exit(5);
    }
    let emitted = match std::fs::read_to_string(emitted_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("serialize_bench: read {}: {}", emitted_path, e);
            std::process::exit(1);
        }
    };
    match round_trip_gate(&original, &emitted, fmt) {
        Ok(()) => {
            eprintln!(
                "round-trip green: {} triples, {} as {}",
                original.len(),
                emitted_path,
                fmt.label()
            );
            std::process::exit(0);
        }
        Err(e) => {
            let code = if e.starts_with("re-parse") { 3 } else { 4 };
            eprintln!("serialize_bench: ROUND-TRIP GATE RED: {}", e);
            std::process::exit(code);
        }
    }
}
