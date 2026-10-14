//! RESEARCH SPIKE — streaming seam assessment (research/concurrent-serving.md §d.iv).
//!
//! `sparq_engine::query_json_chunks_with_budget` is the server's existing "streamed"
//! SELECT path. This spike measures whether it actually streams in TIME (incremental
//! delivery) or only in SPACE (memory): time-to-first-chunk vs total time, against
//! `query_json` (single string) and `count` (pure compute, no serialisation).
//!
//! Hypothesis to test: the chunks Vec is fully evaluated before the function returns,
//! so time-to-first-chunk == full evaluation time — i.e. today's seam removes the
//! second result copy but does NOT give a pull-streaming executor; true streaming
//! needs an iterator/callback seam in the engine (an engine-hooks ask).
//!
//! Run: `cargo run --release --bin stream_spike [data.nt]` in bench/serve.
//!   cargo run --release --bin stream_spike -- [data.nt] --json /tmp/stream.json  # strictly-additive
//!
//! [OPUS-4.8] (sq-5vm.1) `--json <path>` writes the SAME per-query timing rows STDOUT prints
//! as a stable, DEPENDENCY-FREE JSON document, mirroring writer_spike's emit (sq-k5qq). No serde
//! dep is added to the harness (serde_json is a TEST-only dev-dep that parses the emit back).
//! STDOUT is byte-for-byte unchanged whether or not the flag is present; every number is
//! best-effort, MEASURED on the running host — ADVISORY + NON-CANONICAL (stated in the emitted
//! `note`) — nothing is committed.

use std::time::Instant;

use sparq_core::Graph;
use sparq_engine::QueryBudget;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-5vm.1) --json <path> machine-readable results emit
// ---------------------------------------------------------------------------

/// One query's streaming-seam timings: rows/chunks/bytes plus the four phase times (ms)
/// and the derived first-chunk share of full evaluation.
struct QueryRow {
    query: String,
    rows: usize,
    chunks: usize,
    bytes: usize,
    count_compute_ms: f64,
    chunks_time_to_first_ms: f64,
    chunks_drain_ms: f64,
    query_json_ms: f64,
    first_chunk_pct_of_full: f64,
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

/// Minimal JSON string escaper (the dependency-free emit). Query strings are ASCII
/// SPARQL, so this covers the realistic input; anything else still yields valid `\uXXXX`.
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    sparq_core::json::escape_into(&mut out, raw);
    out.push('"');
    out
}

/// Serialise the run to stable, dependency-free JSON. Every metric is ADVISORY +
/// NON-CANONICAL (stated in `note`).
fn results_json(triples: usize, rows: &[QueryRow]) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"harness\": \"serve-spikes stream_spike\",\n");
    s.push_str(
        "  \"note\": \"streaming-seam assessment (time-to-first-chunk vs full evaluation); every \
         millisecond figure is best-effort, MEASURED on the running host — ADVISORY, NON-CANONICAL \
         (this dev box) — do not bake into committed files\",\n",
    );
    s.push_str(&format!("  \"triples\": {},\n", triples));
    s.push_str("  \"queries\": [\n");
    for (i, r) in rows.iter().enumerate() {
        let comma = if i + 1 < rows.len() { "," } else { "" };
        s.push_str(&format!(
            "    {{ \"query\": {}, \"rows\": {}, \"chunks\": {}, \"bytes\": {}, \
             \"count_compute_ms\": {:.3}, \"chunks_time_to_first_ms\": {:.3}, \
             \"chunks_drain_ms\": {:.3}, \"query_json_ms\": {:.3}, \
             \"first_chunk_pct_of_full\": {:.1} }}{comma}\n",
            json_str(&r.query),
            r.rows,
            r.chunks,
            r.bytes,
            r.count_compute_ms,
            r.chunks_time_to_first_ms,
            r.chunks_drain_ms,
            r.query_json_ms,
            r.first_chunk_pct_of_full,
        ));
    }
    s.push_str("  ]\n");
    s.push_str("}\n");
    s
}

fn main() {
    // Strictly-additive: pull `--json <path>` out before reading the optional data path,
    // so the positional `.nt` argument is unaffected when the flag is absent.
    let (args, json_path) = take_json_flag(std::env::args().collect());
    let path = args.into_iter().nth(1);
    let graph = match &path {
        Some(p) => {
            eprintln!("loading {p} ...");
            Graph::load_str(&std::fs::read_to_string(p).expect("read"), "ntriples").expect("load")
        }
        None => {
            let mut nt = String::new();
            for i in 0..1_000_000 {
                nt.push_str(&format!("<http://ex/s{i}> <http://ex/p> \"value number {i}\" .\n"));
            }
            Graph::load_str(&nt, "ntriples").expect("load synthetic")
        }
    };
    let triples = graph.len();
    println!("loaded {triples} triples");

    let mut rows: Vec<QueryRow> = Vec::new();
    for q in [
        "SELECT ?s ?p ?o WHERE { ?s ?p ?o }",                 // full scan, graph-sized result
        "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 100000",    // large but bounded
        "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10",        // the cheap case
    ] {
        let b = QueryBudget::unlimited();

        // Pure compute (no term materialisation, no serialisation).
        let t = Instant::now();
        let n = sparq_engine::count(&graph, q).expect("count");
        let t_count = t.elapsed();

        // Chunked path: time-to-return == time-to-first-chunk (the Vec is complete
        // before we can see chunk 0), then "drain" the chunks like hyper would.
        let t = Instant::now();
        let chunks = sparq_engine::query_json_chunks_with_budget(&graph, q, &b).expect("chunks");
        let t_first = t.elapsed();
        let total_bytes: usize = chunks.iter().map(String::len).sum();
        let n_chunks = chunks.len();
        let t = Instant::now();
        let mut sink = 0usize;
        for c in &chunks {
            sink = sink.wrapping_add(std::hint::black_box(c.len()));
        }
        let t_drain = t.elapsed();

        // Single-string path for comparison.
        let t = Instant::now();
        let s = sparq_engine::query_json(&graph, q).expect("json");
        let t_single = t.elapsed();

        let first_chunk_pct = t_first.as_secs_f64() / t_single.as_secs_f64() * 100.0;
        println!(
            "{q}\n  rows={n} chunks={n_chunks} bytes={total_bytes}\n  count(compute only)      {:>10.1} ms\n  chunks: time-to-first    {:>10.1} ms   drain {:.3} ms  [{sink}]\n  query_json (one string)  {:>10.1} ms\n  => first-chunk latency is {:.0}% of full evaluation (1.0 == no time streaming)",
            t_count.as_secs_f64() * 1e3,
            t_first.as_secs_f64() * 1e3,
            t_drain.as_secs_f64() * 1e3,
            t_single.as_secs_f64() * 1e3,
            first_chunk_pct,
        );
        let _ = s;
        rows.push(QueryRow {
            query: q.to_string(),
            rows: n,
            chunks: n_chunks,
            bytes: total_bytes,
            count_compute_ms: t_count.as_secs_f64() * 1e3,
            chunks_time_to_first_ms: t_first.as_secs_f64() * 1e3,
            chunks_drain_ms: t_drain.as_secs_f64() * 1e3,
            query_json_ms: t_single.as_secs_f64() * 1e3,
            first_chunk_pct_of_full: first_chunk_pct,
        });
    }

    // [OPUS-4.8] (sq-5vm.1) Strictly-additive JSON emit: only when `--json <path>` was given.
    // STDOUT above (the per-query text blocks) is the unchanged human/research output.
    if let Some(path) = json_path {
        let doc = results_json(triples, &rows);
        if let Err(e) = std::fs::write(&path, doc) {
            eprintln!("error writing --json results to {path}: {e}");
            std::process::exit(1);
        }
        eprintln!("wrote {} query rows to {path}", rows.len());
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-5vm.1) --json emit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_flag_extraction() {
        let argv: Vec<String> = ["stream_spike", "data.nt", "--json", "/tmp/o.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (positional, path) = take_json_flag(argv);
        assert_eq!(positional, vec!["stream_spike", "data.nt"]);
        assert_eq!(path.as_deref(), Some("/tmp/o.json"));
        let plain: Vec<String> = ["stream_spike"].iter().map(|s| s.to_string()).collect();
        let (p2, none) = take_json_flag(plain.clone());
        assert_eq!(p2, plain);
        assert!(none.is_none());
    }

    #[test]
    fn json_str_escapes() {
        assert_eq!(
            json_str("SELECT ?s WHERE { ?s ?p ?o }"),
            "\"SELECT ?s WHERE { ?s ?p ?o }\""
        );
        assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    #[test]
    fn results_json_round_trips() {
        let rows = vec![
            QueryRow {
                query: "SELECT ?s ?p ?o WHERE { ?s ?p ?o }".into(),
                rows: 1_000_000,
                chunks: 100,
                bytes: 50_000_000,
                count_compute_ms: 12.0,
                chunks_time_to_first_ms: 90.0,
                chunks_drain_ms: 0.5,
                query_json_ms: 95.0,
                first_chunk_pct_of_full: 94.7,
            },
            QueryRow {
                query: "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10".into(),
                rows: 10,
                chunks: 1,
                bytes: 600,
                count_compute_ms: 0.1,
                chunks_time_to_first_ms: 0.2,
                chunks_drain_ms: 0.001,
                query_json_ms: 0.2,
                first_chunk_pct_of_full: 100.0,
            },
        ];
        let doc = results_json(1_000_000, &rows);
        // The dependency-free emit must round-trip through a REAL serde_json parse.
        let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");
        assert_eq!(v["harness"], "serve-spikes stream_spike");
        assert!(v["note"].as_str().unwrap().contains("NON-CANONICAL"));
        assert_eq!(v["triples"], 1_000_000);
        let qs = v["queries"].as_array().expect("queries is an array");
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0]["query"], "SELECT ?s ?p ?o WHERE { ?s ?p ?o }");
        assert_eq!(qs[0]["rows"], 1_000_000);
        assert!(qs[0]["chunks_time_to_first_ms"].is_number());
        assert!(qs[0]["first_chunk_pct_of_full"].is_number());

        // Empty query set must still be valid JSON (no trailing comma).
        let empty = results_json(0, &[]);
        let v2: serde_json::Value = serde_json::from_str(&empty).expect("empty is valid JSON");
        assert!(v2["queries"].as_array().unwrap().is_empty());
    }
}
