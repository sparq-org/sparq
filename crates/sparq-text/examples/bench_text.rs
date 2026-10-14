//! (sq-ustq) TextIndex benchmark runner — the G1 TSV-emitting runner for
//! `bench/fts/`, the full-text-search analogue of `sparq-cli bench` / `bench_shacl`.
//!
//! Two axes, one binary (design: research/capability-benchmark-program.md §3.4):
//!
//! 1. **Latency axis (engine surface):** N synthetic 8-word literals over a ~10k-term
//!    Zipf-skewed vocabulary (deterministic seed) loaded into a sparq Graph; a
//!    positions-enabled BM25 index; AND / OR / prefix / phrase / near query latency.
//! 2. **Deterministic IR-structure axis:** the TOTAL hit count over a FIXED query set
//!    (independent of the corpus-gen RNG draw order) and the index BYTES-PER-DOC are
//!    integer-exact at the pinned `(N, seed)` corpus — the FTS analogue of LUBM's count
//!    diff / `store_bytes_per_triple`. These are the hard gate columns `bench/fts/run.sh`
//!    asserts against `expected.tsv`.
//!
//! ```sh
//! # No args reproduces the PINNED per-commit corpus (N=100000, seed=0) that
//! # bench/fts/gen.sh + expected.tsv are derived from — so a casual `cargo run`
//! # reproduces the gated counts. The heavy/latency 1M tier is opt-in via the first arg.
//! cargo run -p sparq-text --release --example bench_text              # N=100000 seed=0 iters=3
//! cargo run -p sparq-text --release --example bench_text -- [N] [seed] [iters]
//! ```
//!
//! Output: one TSV line per workload (sorted by name), the same `name\tcount\tus`
//! 3-column contract the ci-bench hook consumes, PLUS the deterministic `bytes_per_doc`
//! row whose `count` is the integer index footprint per document:
//!
//! ```text
//! and_terms     <total_hits>   <min_us_per_query>
//! or_terms      <total_hits>   <min_us_per_query>
//! prefix4       <total_hits>   <min_us_per_query>
//! phrase        <total_hits>   <min_us_per_query>
//! near_slop2    <total_hits>   <min_us_per_query>
//! bytes_per_doc <bytes_per_doc> <build_us>
//! ```
//!
//! - `count` for the query workloads = the TOTAL hit count summed over the fixed query
//!   set (a deterministic integer at the pinned corpus — the gate).
//! - `count` for `bytes_per_doc` = `heap_bytes() / len()` truncated to an integer
//!   (deterministic, runner-noise-immune — like `store_bytes_per_triple`).
//! - `us` = the best-of-`iters` time (ADVISORY; this dev box is non-canonical).
//!   For the query rows it is microseconds-per-query; for `bytes_per_doc` it is the
//!   index BUILD time in microseconds (advisory `text_build`).
//!
//! `bench/fts/run.sh` parses this, asserts the `count` columns against
//! `bench/fts/expected.tsv` (exit 1 on drift), and forwards the same 3-column contract.
//! Correctness lives in `expected.tsv`, NOT in the binary — exactly like LUBM / SHACL.

use std::time::Instant;

// sq-8xug: `random_range` moved from `Rng` to `RngExt` in rand 0.10.
use rand::{rngs::StdRng, RngExt, SeedableRng};
use sparq_core::Graph;
use sparq_text::TextIndex;

/// The DEFAULT corpus RNG seed — pinned to match `bench/fts/gen.sh` (seed 0) so a no-arg
/// `cargo run --example bench_text` reproduces EXACTLY the committed `bench/fts/expected.tsv`
/// corpus. (Distinct from the query-set seed below so the two are independent — the query
/// set never shifts when corpus generation changes its RNG consumption, which is what makes
/// the hit counts a STABLE gate.)
const CORPUS_SEED: u64 = 0;

/// The DEFAULT corpus size — pinned to match `bench/fts/gen.sh` (N=100000, the per-commit
/// tier) so a no-arg run reproduces the gated `expected.tsv` counts. The heavy/latency
/// 1M-literal tier is opt-in via the first arg (`cargo run --example bench_text -- 1000000`).
const CORPUS_N: usize = 100_000;

const STEMS: [&str; 20] = [
    "data", "graph", "query", "index", "store", "term", "literal", "token", "search", "join",
    "plan", "merge", "scan", "parse", "shard", "cache", "delta", "score", "match", "prefix",
];

/// A ~10k-word vocabulary: 20 stems x ~500 numbered suffixes; a Zipf-ish skew comes
/// from drawing the suffix index quadratically (low numbers common, high numbers rare).
fn word(rng: &mut StdRng) -> String {
    let stem = STEMS[rng.random_range(0..STEMS.len())];
    let r: f64 = rng.random_range(0.0..1.0);
    let suffix = (r * r * 500.0) as u32; // quadratic skew: ~common head, long tail
    format!("{stem}{suffix}")
}

/// A FIXED query set, derived ONLY from a dedicated seed (NOT from the post-corpus RNG
/// state). This is the load-bearing determinism trick: the same `(a, b)` term pairs are
/// drawn no matter how the corpus generator evolves, so the asserted hit counts only
/// change when search SEMANTICS change (the regression we want to catch), never as an
/// artefact of how many RNG draws the corpus consumed. Returns `QUERIES` term pairs.
fn fixed_queries(n: usize) -> Vec<(String, String)> {
    let mut q = StdRng::seed_from_u64(0xF7501); // "FTS-01" — independent query seed
    (0..n).map(|_| (word(&mut q), word(&mut q))).collect()
}

/// Number of queries in the fixed set (kept modest so the per-commit corpus stays
/// sub-second; the hit-count SUM over this set is the deterministic gate).
const QUERIES: usize = 200;

// ---------------------------------------------------------------------------
// (sq-ghjc) --json <path> machine-readable results emit
// ---------------------------------------------------------------------------
//
// Strictly-additive: a trailing `--json <path>` writes the SAME `name/count/us`
// rows (plus the run parameters) as a stable, DEPENDENCY-FREE JSON document — the
// hand-built `format!` convention from `crates/sparq-mpc/examples/mpc_net_bench.rs`
// (no serde dep). STDOUT stays byte-for-byte the pure TSV the `bench/fts/run.sh`
// gate parses whether or not the flag is present; the flag is stripped from argv
// BEFORE the positional `[N] [seed] [iters]` parse so the gate's positional call
// (`bench_text <N> <seed> <iters>`, no flag) is wholly unaffected.

/// Extracts `--json <path>` (and its value) from argv, returning argv WITHOUT the
/// flag pair so the positional `[N] [seed] [iters]` indexing is unchanged. A bare
/// `--json` is a usage error (exit 2), mirroring `sparq-cli`'s identical flag.
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

/// Minimal JSON string escaper (the dependency-free emit). Workload names are static
/// ASCII identifiers, so this covers the realistic input; anything else still yields
/// valid `\uXXXX` escapes.
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    sparq_core::json::escape_into(&mut out, raw);
    out.push('"');
    out
}

/// Serialises the run to stable, dependency-free JSON. The `rows` array carries one
/// object per workload with the SAME `name`/`count`/`us` fields the TSV prints; the
/// `count` column is the DETERMINISTIC gate value (corpus-stable), while `us`/`build_us`
/// are best-of-iters timings — ADVISORY and NON-CANONICAL (this dev box), stated in
/// `note`. `n`/`seed`/`iters` record the corpus the document describes.
fn results_json(
    n: usize,
    seed: u64,
    iters: usize,
    rows: &[(String, usize, f64)],
    bytes_per_doc: usize,
    build_us: f64,
) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"harness\": \"sparq-text bench_text\",\n");
    s.push_str(&format!("  \"n\": {n},\n"));
    s.push_str(&format!("  \"seed\": {seed},\n"));
    s.push_str(&format!("  \"iters\": {iters},\n"));
    s.push_str(
        "  \"note\": \"`count` columns are DETERMINISTIC (the bench/fts/expected.tsv gate); \
         `us`/`build_us` are best-of-iters wall-clock MEASURED on the running host — ADVISORY, \
         NON-CANONICAL (this dev box) — do not bake into committed files\",\n",
    );
    s.push_str("  \"rows\": [\n");
    for (name, count, us) in rows.iter() {
        // Every workload row is ALWAYS followed by the bytes_per_doc gate row below,
        // so each gets a trailing comma; only the gate row (last) omits it.
        s.push_str(&format!(
            "    {{ \"name\": {}, \"count\": {count}, \"us\": {us:.1} }},\n",
            json_str(name)
        ));
    }
    // The deterministic index-footprint gate row (count = integer B/doc; us = advisory build µs).
    // This is the LAST element of `rows`, hence no trailing comma.
    s.push_str(&format!(
        "    {{ \"name\": \"bytes_per_doc\", \"count\": {bytes_per_doc}, \"us\": {build_us:.1} }}\n"
    ));
    s.push_str("  ]\n");
    s.push_str("}\n");
    s
}

fn main() {
    let (args, json_path) = take_json_flag(std::env::args().collect());
    let n: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(CORPUS_N);
    let seed: u64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(CORPUS_SEED);
    let iters: usize = args.get(3).and_then(|a| a.parse().ok()).unwrap_or(3);
    let iters = iters.max(1);

    // ---- data generation + graph load ------------------------------------------------
    let mut rng = StdRng::seed_from_u64(seed);
    let mut nt = String::with_capacity(n * 120);
    for i in 0..n {
        let sentence: Vec<String> = (0..8).map(|_| word(&mut rng)).collect();
        nt.push_str(&format!(
            "<http://example.org/e{i}> <http://example.org/comment> \"{}\" .\n",
            sentence.join(" ")
        ));
    }
    let graph = Graph::load_str(&nt, "ntriples").unwrap();

    // ---- index build (positions enabled so phrase/near are exercised) -----------------
    // Best-of-iters build for the advisory `text_build` timing; the resulting index is
    // identical every iteration (build is a pure function of the graph), so heap_bytes()
    // and every hit count are deterministic.
    let mut build_us = f64::INFINITY;
    let mut index = TextIndex::default();
    for _ in 0..iters {
        let t = Instant::now();
        index = TextIndex::build_with_positions(&graph);
        build_us = build_us.min(t.elapsed().as_secs_f64() * 1e6);
    }
    let docs = index.len();
    assert!(docs > 0, "empty index — corpus generation produced no literals");
    // Integer bytes-per-doc (truncating div) — runner-noise-immune, like store_bytes_per_triple.
    let bytes_per_doc = index.heap_bytes() / docs;

    // ---- the fixed query set ----------------------------------------------------------
    let queries = fixed_queries(QUERIES);

    // Each workload: (name, total-hit-count over the query set, best-of-iters µs/query).
    // The count is summed deterministically; the timing is best-of-iters / |queries|.
    let mut rows: Vec<(String, usize, f64)> = Vec::new();

    // AND of two terms (`text:matches`): docs containing BOTH terms.
    {
        let (mut count, mut us) = (0usize, f64::INFINITY);
        for _ in 0..iters {
            let t = Instant::now();
            let mut c = 0usize;
            for (a, b) in &queries {
                c += index.search(&format!("{a} {b}")).len();
            }
            us = us.min(t.elapsed().as_secs_f64() * 1e6 / queries.len() as f64);
            count = c;
        }
        rows.push(("and_terms".into(), count, us));
    }

    // OR of two terms (`text:matchesAny`): docs containing AT LEAST ONE term.
    {
        let (mut count, mut us) = (0usize, f64::INFINITY);
        for _ in 0..iters {
            let t = Instant::now();
            let mut c = 0usize;
            for (a, b) in &queries {
                c += index.search_any(&format!("{a} {b}")).len();
            }
            us = us.min(t.elapsed().as_secs_f64() * 1e6 / queries.len() as f64);
            count = c;
        }
        rows.push(("or_terms".into(), count, us));
    }

    // Prefix (4 chars) of the first term — autocomplete shape (`text:matches "foo*"`).
    {
        let (mut count, mut us) = (0usize, f64::INFINITY);
        for _ in 0..iters {
            let t = Instant::now();
            let mut c = 0usize;
            for (a, _) in &queries {
                let p = &a[..a.len().min(4)];
                c += index.search(&format!("{p}*")).len();
            }
            us = us.min(t.elapsed().as_secs_f64() * 1e6 / queries.len() as f64);
            count = c;
        }
        rows.push(("prefix4".into(), count, us));
    }

    // Phrase (two adjacent, in-order terms — `text:phrase "foo bar"`).
    {
        let (mut count, mut us) = (0usize, f64::INFINITY);
        for _ in 0..iters {
            let t = Instant::now();
            let mut c = 0usize;
            for (a, b) in &queries {
                c += index.phrase(&format!("{a} {b}")).len();
            }
            us = us.min(t.elapsed().as_secs_f64() * 1e6 / queries.len() as f64);
            count = c;
        }
        rows.push(("phrase".into(), count, us));
    }

    // Proximity / slop (two in-order terms within gap 2 — `text:near "foo bar"` slop 2).
    {
        let (mut count, mut us) = (0usize, f64::INFINITY);
        for _ in 0..iters {
            let t = Instant::now();
            let mut c = 0usize;
            for (a, b) in &queries {
                c += index.phrase_near(&format!("{a} {b}"), 2).len();
            }
            us = us.min(t.elapsed().as_secs_f64() * 1e6 / queries.len() as f64);
            count = c;
        }
        rows.push(("near_slop2".into(), count, us));
    }

    rows.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, count, us) in &rows {
        println!("{name}\t{count}\t{us:.1}");
    }
    // The deterministic index-footprint gate (count = integer B/doc; us = advisory build time).
    println!("bytes_per_doc\t{bytes_per_doc}\t{build_us:.1}");

    // (sq-ghjc) Strictly-additive JSON emit: only when `--json <path>` was
    // given. STDOUT above is the unchanged TSV the bench/fts gate parses.
    if let Some(path) = json_path {
        let doc = results_json(n, seed, iters, &rows, bytes_per_doc, build_us);
        if let Err(e) = std::fs::write(&path, doc) {
            eprintln!("error writing --json results to {path}: {e}");
            std::process::exit(1);
        }
        eprintln!("wrote {} result rows to {path}", rows.len() + 1);
    }
}

// ---------------------------------------------------------------------------
// (sq-ghjc) --json emit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_flag_extraction() {
        let argv: Vec<String> = ["bench_text", "100", "0", "3", "--json", "/tmp/o.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (positional, path) = take_json_flag(argv);
        // The positional `[N] [seed] [iters]` contract the fts gate relies on is intact.
        assert_eq!(positional, vec!["bench_text", "100", "0", "3"]);
        assert_eq!(path.as_deref(), Some("/tmp/o.json"));
        // Absent flag -> argv unchanged, no path.
        let plain: Vec<String> = ["bench_text", "100"].iter().map(|s| s.to_string()).collect();
        let (p2, none) = take_json_flag(plain.clone());
        assert_eq!(p2, plain);
        assert!(none.is_none());
    }

    #[test]
    fn json_str_escapes() {
        assert_eq!(json_str("and_terms"), "\"and_terms\"");
        assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    #[test]
    fn results_json_shape_and_keys() {
        let rows = vec![
            ("and_terms".to_string(), 42usize, 1.5f64),
            ("or_terms".to_string(), 99usize, 2.25f64),
        ];
        let doc = results_json(100, 0, 3, &rows, 64, 12.5);
        // The dependency-free emit must round-trip through a REAL serde_json parse — this
        // is what catches a malformed document (e.g. a missing inter-row comma).
        let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");
        assert_eq!(v["harness"], "sparq-text bench_text");
        assert_eq!(v["n"], 100);
        assert_eq!(v["seed"], 0);
        assert_eq!(v["iters"], 3);
        assert!(v["note"].as_str().unwrap().contains("NON-CANONICAL"));
        let arr = v["rows"].as_array().expect("rows is an array");
        // 2 workloads + the deterministic bytes_per_doc gate row.
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["name"], "and_terms");
        assert_eq!(arr[0]["count"], 42);
        assert!(arr[0]["us"].is_number());
        assert_eq!(arr[1]["name"], "or_terms");
        assert_eq!(arr[1]["count"], 99);
        // The gate row is last and carries the integer footprint + advisory build µs.
        assert_eq!(arr[2]["name"], "bytes_per_doc");
        assert_eq!(arr[2]["count"], 64);
        assert!((arr[2]["us"].as_f64().unwrap() - 12.5).abs() < 1e-9);
    }
}
