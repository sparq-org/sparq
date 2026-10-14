//! GeoIndex benchmark: build + query latency on ~100k points, PLUS a
//! deterministic TSV-emitting bench mode (the G1 runner for `bench/geo/`).
//!
//! Modes:
//!
//! ```sh
//! # 1. Human-readable latency report (the original mode; numbers land in the README):
//! cargo run --release -p sparq-geo --example bench_geo
//!
//! # 2. Deterministic bench TSV (consumed by bench/geo/run.sh's self-asserting gate):
//! cargo run --release -p sparq-geo --example bench_geo -- bench [corpus.nt] [iters]
//!
//! # 3. Untimed nearest-neighbour entity oracle (one IRI per line):
//! cargo run --release -p sparq-geo --example bench_geo -- nearest-ids <corpus.nt> <k>
//! ```
//!
//! ## Mode 2 contract — `name\tcount\tus`
//!
//! The bench mode loads a FIXED CRS84 point corpus (generated deterministically
//! by `bench/geo/gen.sh`, or regenerated in-process when no corpus path is
//! given), builds the [`GeoIndex`], and emits one TSV line per workload:
//!
//! ```text
//! <name>\t<count>\t<us>
//! ```
//!
//! - `<name>`  — the workload (`within10km`, `within50km`, `nearest_k10`,
//!   `nearest_k100`, `geof_within`, `geo_compliance_pass`).
//! - `<count>` — the DETERMINISTIC result-set SIZE (or, for `geo_compliance_pass`,
//!   the number of OGC topology fixtures passing). Counts are bit-stable; the
//!   floating-point COORDINATES are NOT, so only the COUNT is gated, never a value.
//! - `<us>`    — best-of-`iters` wall-clock microseconds (ADVISORY timing only).
//!
//! `bench/geo/run.sh` parses this, asserts every `<count>` against
//! `bench/geo/expected.tsv` (exit 1 on drift), and forwards the same three-column
//! contract to the ci-bench hook (which harvests the `_us` columns as advisory
//! `geo_<name>_us` trend metrics). Correctness lives in `expected.tsv`, NOT in
//! the binary — exactly like LUBM / SHACL. (sq-tf8n)
//!
//! ## Mode 3 — `nearest-ids <corpus.nt> <k>`
//!
//! This untimed correctness oracle loads the corpus, builds the same index as
//! bench mode, replays `index.nearest(QUERY_CENTER, k)`, and emits one entity
//! IRI per line for exact competitor-result comparison.
//!
//! ## Mode 4 — `query <corpus.nt> <query.rq> [iters]` (needs the `engine` feature)
//!
//! The generic SPARQL runner for the Geographica real-world family
//! (`bench/geo/queries-geographica/`, driven by `scripts/bench/geo-same-box.sh`
//! under `GEO_GEOGRAPHICA=1`): load the corpus, run the pinned query file
//! through `sparq_engine::query_with_functions` with the full `geof:` registry,
//! and emit the same `name\tcount\tus` line (name = the query-file stem;
//! `count` = the COUNT scalar of a COUNT-wrapped query, else the row count —
//! the identical comparable-size convention as
//! `scripts/bench-adapters/http_sparql_adapter.py`, so the result-set-size
//! cross-check against an HTTP competitor is like-for-like).
//! (sq-hmd7l.29)

use std::time::Instant;

use geo_types::Point;
// sq-8xug: `random_range` moved from `Rng` to `RngExt` in rand 0.10.
use rand::{rngs::StdRng, RngExt, SeedableRng};
use sparq_core::Graph;
use sparq_geo::geof::lex;
use sparq_geo::{parse_wkt_literal, GeoIndex};

// ---- the FIXED corpus pin (shared by bench/geo/gen.sh and the in-process fallback) ----
//
// These MUST match bench/geo/gen.sh exactly so the in-process fallback corpus is
// byte-identical to the generated one (and therefore the asserted counts agree).
/// Deterministic RNG seed for the point corpus.
const CORPUS_SEED: u64 = 20260615;
/// Default corpus size (points). ~100k per the design (§3.5).
const CORPUS_N: usize = 100_000;
/// Corpus window (CRS84 long/lat degrees): a ~8°×8° France-ish box.
const LON: (f64, f64) = (-4.0, 4.0);
const LAT: (f64, f64) = (47.0, 55.0);

// ---- the FIXED query pins (deterministic; counts derived by running) ----
/// Centre of the within / nearest queries (CRS84 long/lat, well inside the window).
const QUERY_CENTER: (f64, f64) = (0.0, 51.0);

/// Generate the deterministic CRS84 point corpus as `geo:asWKT` N-Triples.
/// Byte-identical to what `bench/geo/gen.sh` writes (same seed, window, order).
fn generate_corpus(n: usize) -> String {
    let mut rng = StdRng::seed_from_u64(CORPUS_SEED);
    let mut nt = String::with_capacity(n * 140);
    for i in 0..n {
        let x = rng.random_range(LON.0..LON.1);
        let y = rng.random_range(LAT.0..LAT.1);
        nt.push_str(&format!(
            "<http://example.org/e{i}> <http://www.opengis.net/ont/geosparql#asWKT> \
             \"POINT({x} {y})\"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .\n"
        ));
    }
    nt
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // `gen [N]` — write the deterministic corpus N-Triples to stdout. bench/geo/gen.sh
        // shells out to THIS so the committed corpus is byte-identical to the in-process
        // fallback (no risk of f64-formatting drift between Rust and shell).
        Some("gen") => {
            let n = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(CORPUS_N);
            print!("{}", generate_corpus(n));
        }
        Some("bench") => bench_mode(&args[1..]),
        Some("nearest-ids") => nearest_ids_mode(&args[1..]),
        Some("query") => query_mode(&args[1..]),
        _ => report_mode(args.first().and_then(|a| a.parse().ok()).unwrap_or(CORPUS_N)),
    }
}

/// Emit the exact nearest-neighbour entity set for the same-box crosscheck.
///
/// This path is deliberately untimed: benchmark timings continue to come from
/// `bench`, while the harness uses these sorted IDs as a correctness oracle.
/// (sq-6jl8z)
fn nearest_ids_mode(args: &[String]) {
    let usage = "usage: bench_geo nearest-ids <corpus.nt> <k>";
    let (corpus_path, k) = match (args.first(), args.get(1).and_then(|s| s.parse().ok())) {
        (Some(path), Some(k)) => (path, k),
        _ => {
            eprintln!("{}", usage);
            std::process::exit(2);
        }
    };
    let nt = std::fs::read_to_string(corpus_path)
        .unwrap_or_else(|e| panic!("read corpus {}: {}", corpus_path, e));
    let graph =
        Graph::load_str(&nt, "ntriples").unwrap_or_else(|e| panic!("parse corpus: {}", e));
    let index = GeoIndex::build(&graph);
    let center = Point::new(QUERY_CENTER.0, QUERY_CENTER.1);
    let mut entities: Vec<String> = index
        .nearest(center, k)
        .into_iter()
        .map(|(entity, _)| entity.to_string())
        .collect();
    entities.sort_unstable();
    for entity in entities {
        println!("{}", entity);
    }
}

/// Mode 4: run one pinned SPARQL query file over a corpus with the `geof:`
/// registry installed; emit `stem\tcount\tus`. (sq-hmd7l.29)
#[cfg(feature = "engine")]
fn query_mode(args: &[String]) {
    let usage = "usage: bench_geo query <corpus.nt> <query.rq> [iters]";
    let (corpus_path, query_path) = match (args.first(), args.get(1)) {
        (Some(c), Some(q)) => (c, q),
        _ => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    if iters == 0 {
        eprintln!("error: iters must be >= 1 (got 0); need at least one sample for finite timings");
        std::process::exit(2);
    }
    let nt = std::fs::read_to_string(corpus_path)
        .unwrap_or_else(|e| panic!("read corpus {corpus_path}: {e}"));
    let sparql = std::fs::read_to_string(query_path)
        .unwrap_or_else(|e| panic!("read query {query_path}: {e}"));
    let graph = Graph::load_str(&nt, "ntriples").unwrap_or_else(|e| panic!("parse corpus: {e}"));
    let reg = sparq_geo::geof_registry();

    // Best-of-iters over the FULL query (parse + plan + eval); the count must be
    // identical across iters (a drifting count would poison the size oracle).
    let mut count: Option<usize> = None;
    let mut us = f64::INFINITY;
    for _ in 0..iters {
        let t = Instant::now();
        let r = sparq_engine::query_with_functions(&graph, &sparql, &reg)
            .unwrap_or_else(|e| panic!("query {query_path}: {e}"));
        us = us.min(t.elapsed().as_secs_f64() * 1e6);
        let c = comparable_count(&r);
        if let Some(prev) = count {
            assert_eq!(prev, c, "nondeterministic count across iters for {}", query_path);
        }
        count = Some(c);
    }

    let stem = std::path::Path::new(query_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("query");
    emit(stem, count.expect("iters >= 1"), us);
}

/// The comparable result-set size of a query result: a single-row single-var
/// result whose only binding is an integer literal is a COUNT-wrapped query and
/// yields the SCALAR; anything else yields the row count. Mirrors
/// `parse_sparql_json` in `scripts/bench-adapters/http_sparql_adapter.py` so
/// sparq's number and an HTTP competitor's number measure the same thing.
/// (sq-hmd7l.29)
#[cfg(feature = "engine")]
fn comparable_count(r: &sparq_engine::QueryResult) -> usize {
    if r.vars.len() == 1 && r.rows.len() == 1 {
        if let Some(Some(oxrdf::Term::Literal(l))) = r.rows[0].first() {
            if let Ok(v) = l.value().parse::<usize>() {
                return v;
            }
        }
    }
    r.rows.len()
}

/// Without the `engine` feature there is no SPARQL path; fail loudly rather
/// than silently degrading (the corpus/bench modes stay available).
#[cfg(not(feature = "engine"))]
fn query_mode(_args: &[String]) {
    eprintln!(
        "error: `bench_geo query` needs the sparq-geo `engine` feature \
         (on by default; rebuild without --no-default-features)"
    );
    std::process::exit(2);
}

/// Mode 2: the deterministic `name\tcount\tus` runner (the G1 contract).
fn bench_mode(args: &[String]) {
    // Optional corpus path (the gen.sh artifact); else regenerate in-process.
    let corpus_path = args.first().filter(|p| p.as_str() != "-");
    let iters: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    if iters == 0 {
        eprintln!("error: iters must be >= 1 (got 0); need at least one sample for finite timings");
        std::process::exit(2);
    }

    let nt = match corpus_path {
        Some(p) => std::fs::read_to_string(p).unwrap_or_else(|e| panic!("read corpus {p}: {e}")),
        None => generate_corpus(CORPUS_N),
    };
    let graph = Graph::load_str(&nt, "ntriples").unwrap_or_else(|e| panic!("parse corpus: {e}"));
    let index = GeoIndex::build(&graph);

    let center = Point::new(QUERY_CENTER.0, QUERY_CENTER.1);

    // ---- within_distance workloads: count = #entities within the radius -------------
    // (the `geof:`/within result-set size; deterministic given the fixed corpus).
    for (name, radius) in [("within10km", 10_000.0_f64), ("within50km", 50_000.0)] {
        let (count, us) = best_of(iters, || index.within_distance(center, radius, None).len());
        emit(name, count, us);
    }

    // ---- nearest-k workloads: count = k (a fixed-corpus invariant; gates that the
    // expanding-radius search returns exactly k from a corpus larger than k) ----------
    for (name, k) in [("nearest_k10", 10usize), ("nearest_k100", 100)] {
        let (count, us) = best_of(iters, || index.nearest(center, k).len());
        emit(name, count, us);
    }

    // ---- a `geof:` filter-function workload over the corpus: count the entities whose
    // point literal satisfies geof:sfWithin(point, query-box). This drives the lexical
    // geof: relation (the SPARQL filter-function path) over every corpus literal, so the
    // count is the geof:sfWithin result-set size — deterministic, COUNT-only. -----------
    {
        // A 1°×1° box centred on the query point (CRS84 WKT).
        let qbox = format!(
            "POLYGON(({xlo} {ylo}, {xhi} {ylo}, {xhi} {yhi}, {xlo} {yhi}, {xlo} {ylo}))",
            xlo = QUERY_CENTER.0 - 0.5,
            xhi = QUERY_CENTER.0 + 0.5,
            ylo = QUERY_CENTER.1 - 0.5,
            yhi = QUERY_CENTER.1 + 0.5,
        );
        let points: Vec<String> = corpus_point_literals(&nt);
        let (count, us) = best_of(iters, || {
            points.iter().filter(|wkt| lex::sf_within(wkt, &qbox).unwrap_or(false)).count()
        });
        emit("geof_within", count, us);
    }

    // ---- compliance: number of OGC topology fixtures passing (the deficit-ratchet
    // numerator). COUNT/PASS-FAIL only, never coordinates. -----------------------------
    {
        let (count, us) = best_of(iters, ogc_compliance_passes);
        emit("geo_compliance_pass", count, us);
    }
}

/// Mode 1: the original human-readable latency report (numbers land in the README).
fn report_mode(n: usize) {
    let nt = generate_corpus(n);
    let t = Instant::now();
    let graph = Graph::load_str(&nt, "ntriples").unwrap();
    let load = t.elapsed();
    println!("graph load     : {n} asWKT triples in {load:.2?}");

    let t = Instant::now();
    let index = GeoIndex::build(&graph);
    let build = t.elapsed();
    println!(
        "index build    : {} entries in {build:.2?} ({:.2} Mentries/s)",
        index.len(),
        index.len() as f64 / build.as_secs_f64() / 1e6
    );

    let mut rng = StdRng::seed_from_u64(CORPUS_SEED ^ 0xC0FFEE);
    let mut query_points: Vec<Point<f64>> = Vec::new();
    for _ in 0..1000 {
        query_points
            .push(Point::new(rng.random_range(LON.0..LON.1), rng.random_range(LAT.0..LAT.1)));
    }

    for radius in [1_000.0, 10_000.0, 50_000.0] {
        let t = Instant::now();
        let mut total = 0usize;
        for &p in &query_points {
            total += index.within_distance(p, radius, None).len();
        }
        let el = t.elapsed();
        println!(
            "within {:>5}km : {:>8.1} µs/query ({:.1} avg hits)",
            radius / 1000.0,
            el.as_micros() as f64 / query_points.len() as f64,
            total as f64 / query_points.len() as f64
        );
    }

    for k in [1, 10, 100] {
        let t = Instant::now();
        let mut total = 0usize;
        for &p in &query_points {
            total += index.nearest(p, k).len();
        }
        let el = t.elapsed();
        assert_eq!(total, k * query_points.len());
        println!(
            "nearest k={k:<4}  : {:>8.1} µs/query",
            el.as_micros() as f64 / query_points.len() as f64
        );
    }

    let mut polygons = Vec::new();
    for _ in 0..100 {
        let x = rng.random_range(LON.0..LON.1 - 0.5);
        let y = rng.random_range(LAT.0..LAT.1 - 0.5);
        let (w, h) = (0.5, 0.5);
        polygons.push(
            parse_wkt_literal(&format!(
                "POLYGON(({x} {y}, {x2} {y}, {x2} {y2}, {x} {y2}, {x} {y}))",
                x2 = x + w,
                y2 = y + h
            ))
            .unwrap(),
        );
    }
    let t = Instant::now();
    let mut total = 0usize;
    for poly in &polygons {
        total += index.intersects(poly).len();
    }
    let el = t.elapsed();
    println!(
        "intersects box : {:>8.1} µs/query ({:.1} avg hits, 0.5°x0.5° boxes)",
        el.as_micros() as f64 / polygons.len() as f64,
        total as f64 / polygons.len() as f64
    );
}

/// Extract the WKT literal lexical forms (`POINT(x y)`) from the corpus N-Triples,
/// in file order. Used by the `geof_within` workload to drive the lexical relation.
fn corpus_point_literals(nt: &str) -> Vec<String> {
    nt.lines()
        .filter_map(|line| {
            let start = line.find("\"POINT(")? + 1;
            let end = line[start..].find('"')? + start;
            Some(line[start..end].to_string())
        })
        .collect()
}

/// Run `f` `iters` times; return its (count, best-of-iters microseconds).
fn best_of(iters: usize, mut f: impl FnMut() -> usize) -> (usize, f64) {
    let mut count = 0usize;
    let mut us = f64::INFINITY;
    for _ in 0..iters {
        let t = Instant::now();
        count = f();
        us = us.min(t.elapsed().as_secs_f64() * 1e6);
    }
    (count, us)
}

fn emit(name: &str, count: usize, us: f64) {
    println!("{name}\t{count}\t{us:.1}");
}

// ----------------------------------------------------------------------------------------
// OGC GeoSPARQL topology compliance fixtures (the `geo_compliance_pass` numerator).
//
// A compact, hand-derived set of `geof:` topology assertions (sf / eh / rcc8 families)
// over WKT operands, mirroring crates/sparq-geo/tests/ogc_compliance_ratchet.rs. The
// example reports the PASS COUNT (a deterministic integer); bench/geo/run.sh asserts it
// and the perf-baseline ratchet gates the DEFICIT (max - passed), so coverage may only
// tighten. PASS/FAIL only — never a coordinate value.
// ----------------------------------------------------------------------------------------

type Rel = fn(&str, &str) -> Result<bool, sparq_geo::GeoError>;

const BIG: &str = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
const SMALL_INSIDE: &str = "POLYGON((1 1, 2 1, 2 2, 1 2, 1 1))";
const EDGE_ADJACENT: &str = "POLYGON((4 0, 6 0, 6 2, 4 2, 4 0))";
const FAR: &str = "POLYGON((9 9, 10 9, 10 10, 9 10, 9 9))";
const OVERLAP: &str = "POLYGON((3 3, 6 3, 6 6, 3 6, 3 3))";

/// `(relation, a, b, expected)` topology fixtures. Each `expected` is hand-derived
/// from the relation's DE-9IM matrix per the OGC GeoSPARQL specification.
#[rustfmt::skip]
const FIXTURES: &[(Rel, &str, &str, bool)] = &[
    // Simple Features (Req 22).
    (lex::sf_equals as Rel, BIG, BIG, true),
    (lex::sf_disjoint, BIG, FAR, true),
    (lex::sf_intersects, BIG, OVERLAP, true),
    (lex::sf_touches, BIG, EDGE_ADJACENT, true),
    (lex::sf_within, SMALL_INSIDE, BIG, true),
    (lex::sf_contains, BIG, SMALL_INSIDE, true),
    (lex::sf_overlaps, BIG, OVERLAP, true),
    (lex::sf_within, BIG, FAR, false),
    (lex::sf_contains, SMALL_INSIDE, BIG, false),
    (lex::sf_disjoint, BIG, OVERLAP, false),
    (lex::sf_within, "POINT(1 1)", "POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))", true),
    (lex::sf_intersects, "LINESTRING(-1 1, 3 1)", "POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))", true),
    (lex::sf_crosses, "LINESTRING(0 0, 2 2)", "LINESTRING(0 2, 2 0)", true),
    // Egenhofer (Req 25).
    (lex::eh_equals, BIG, BIG, true),
    (lex::eh_disjoint, BIG, FAR, true),
    (lex::eh_meet, BIG, EDGE_ADJACENT, true),
    (lex::eh_overlap, BIG, OVERLAP, true),
    (lex::eh_inside, SMALL_INSIDE, BIG, true),
    (lex::eh_contains, BIG, SMALL_INSIDE, true),
    // RCC8 (Req 26).
    (lex::rcc8_eq, BIG, BIG, true),
    (lex::rcc8_dc, BIG, FAR, true),
    (lex::rcc8_ec, BIG, EDGE_ADJACENT, true),
    (lex::rcc8_po, BIG, OVERLAP, true),
    (lex::rcc8_ntpp, SMALL_INSIDE, BIG, true),
    (lex::rcc8_ntppi, BIG, SMALL_INSIDE, true),
];

/// Number of OGC topology fixtures whose `geof:` relation matches the spec truth value.
fn ogc_compliance_passes() -> usize {
    FIXTURES
        .iter()
        .filter(|(rel, a, b, expected)| rel(a, b).map(|got| got == *expected).unwrap_or(false))
        .count()
}
