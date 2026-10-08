//! [OPUS-4.8] (sq-0z43i, gh #909) Dispatch-overhead measurement for `sparq-mcp`:
//! the cost the MCP layer ADDS around the engine — JSON-RPC parse, tool dispatch,
//! and result wrapping — isolated from engine compute by running over a small
//! in-memory graph where the underlying query is trivial.
//!
//! Reported: the structural facts (graph triples, tools advertised) — deterministic
//! and load-robust — plus ADVISORY, NON-CANONICAL per-call wall times for an
//! `initialize` / `tools/list` / `query` / `stats` round-trip on THIS dev box
//! (do not bake these into docs or gates). Run:
//!
//! ```text
//! cargo run -p sparq-mcp --example mcp_roundtrip_bench --release
//! ```
//!
//! `--json <path>` writes the deterministic structural counts as a dependency-free
//! JSON document (mirroring the `format!`-JSON convention of the other harnesses);
//! STDOUT is byte-for-byte identical whether or not the flag is present.

use std::time::Instant;

use sparq_core::Graph;
use sparq_mcp::{tools, McpServer};

const TTL: &str = r#"@prefix ex: <http://ex/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
ex:a rdf:type ex:T ; ex:p ex:b .
ex:b rdf:type ex:T ; ex:p ex:c .
ex:c rdf:type ex:T .
"#;

fn time_call(server: &mut McpServer, raw: &str, iters: u32) -> f64 {
    // Warm.
    let _ = server.handle_message(raw);
    let start = Instant::now();
    for _ in 0..iters {
        let _ = server.handle_message(raw);
    }
    start.elapsed().as_secs_f64() / f64::from(iters) * 1e6 // µs/call
}

fn main() {
    let json_path = std::env::args()
        .collect::<Vec<_>>()
        .windows(2)
        .find_map(|w| {
            // Positional format args (CodeQL rust/unused-variable false-positive guard).
            if w[0] == "--json" {
                Some(w[1].clone())
            } else {
                None
            }
        });

    let graph = Graph::load_str(TTL, "turtle").expect("load");
    let triples = graph.len();
    let mut server = McpServer::new(graph);
    let advertised = tools::advertised(&server).len();

    const ITERS: u32 = 2000;
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
    let query = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"query","arguments":{"sparql":"SELECT ?s WHERE { ?s ?p ?o }"}}}"#;
    let stats = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"stats","arguments":{}}}"#;

    let t_init = time_call(&mut server, init, ITERS);
    let t_list = time_call(&mut server, list, ITERS);
    let t_query = time_call(&mut server, query, ITERS);
    let t_stats = time_call(&mut server, stats, ITERS);

    // Positional format args throughout (CodeQL rust/unused-variable guard).
    println!("sparq-mcp dispatch-overhead measurement (NON-CANONICAL, advisory)");
    println!("  graph triples       : {}", triples);
    println!("  tools advertised    : {}", advertised);
    println!("  initialize  µs/call : {:.2}", t_init);
    println!("  tools/list  µs/call : {:.2}", t_list);
    println!("  query       µs/call : {:.2}", t_query);
    println!("  stats       µs/call : {:.2}", t_stats);

    if let Some(path) = json_path {
        // Only the DETERMINISTIC structural facts are persisted (timings are
        // box-sensitive and intentionally omitted from the stable JSON).
        let doc = format!(
            "{{\"crate\":\"sparq-mcp\",\"graph_triples\":{},\"tools_advertised\":{}}}\n",
            triples, advertised
        );
        if let Err(e) = std::fs::write(&path, doc) {
            eprintln!("could not write {}: {}", path, e);
        }
    }
}
