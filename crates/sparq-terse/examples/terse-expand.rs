//! sq-bzign (epic sq-2m6zm, Phase 5 A/B). 🤖 SPARQ agent — the deterministic
//! transpile-and-echo tool the Phase-5 A/B harness (`bench/terse/`) and a sub-agent both call.
//!
//! It takes a terse query (arm B `K:<name>` keywords, or arm C `K:` + `V("phrase")`), loads the
//! PKG graph, runs the *real* `terse_to_sparql_with` transpiler (lexical-first `V()` resolution,
//! confidence-gated + staleness-guarded, silent-rewrite canary), and prints a single JSON object:
//!
//! ```json
//! { "ok": true, "canonical_sparql": "...", "resolutions": [ {"phrase","iri","score",
//!   "confidence","runner_up","method"} ], "keywords": [ {"keyword","iri"} ] }
//! ```
//!
//! On a transpile error (unknown keyword, unresolved/ambiguous `V()`, canary failure) it prints
//! `{ "ok": false, "error": "<message>" }` and exits non-zero — the loud-fail the soundness
//! envelope demands, surfaced to the caller as data. A plain-SPARQL (arm A) string passes through
//! byte-identical (the identity transpile), so the same tool grades all three arms.
//!
//! This carries ZERO extra runtime cost into the lean default build: it is an example gated on the
//! `vectors` feature (it needs `terse_to_sparql_with` + the `Graph`), built only when that feature
//! is on. The embedder is a no-op `|_| None` — the no-model, deterministic, lexical-only path
//! (design §6.4) — so the A/B never depends on a network or an embedding model.
//!
//! Usage:
//!   terse-expand <graph.ttl> <format>            # terse query on stdin
//!   terse-expand <graph.ttl> <format> "<query>"  # terse query as the 3rd arg

use std::io::Read;

use sparq_core::Graph;
use sparq_terse::{terse_to_sparql_with, ResolveCtx};

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    sparq_core::json::escape_into(&mut out, s);
    out
}

fn fail(msg: &str) -> ! {
    println!("{{\"ok\":false,\"error\":\"{}\"}}", json_escape(msg));
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| fail("missing <graph.ttl>"));
    let format = args.get(2).cloned().unwrap_or_else(|| "turtle".to_string());

    // The terse query: 3rd arg if present, else stdin.
    let query = match args.get(3) {
        Some(q) => q.clone(),
        None => {
            let mut buf = String::new();
            if std::io::stdin().read_to_string(&mut buf).is_err() {
                fail("could not read the terse query from stdin");
            }
            buf
        }
    };
    let query = query.trim();
    if query.is_empty() {
        fail("empty terse query");
    }

    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => fail(&format!("could not read graph {path}: {e}")),
    };
    let graph = match Graph::load_str(&text, &format) {
        Ok(g) => g,
        Err(e) => fail(&format!("could not load graph: {e}")),
    };

    // Lexical-only, no-model resolution context (the deterministic default the design mandates
    // for the A/B). The embedder is a no-op, so the vector fallback never fires.
    let ctx = ResolveCtx::lexical(&graph);
    let exp = match terse_to_sparql_with(query, &ctx, |_phrase| None) {
        Ok(e) => e,
        Err(e) => fail(&format!("{e}")),
    };

    // Emit the verifiable expansion as JSON: the canonical SPARQL the engine will run, plus every
    // keyword + V() bind echoed (the soundness envelope's "show your work").
    let mut out = String::new();
    out.push_str("{\"ok\":true,\"canonical_sparql\":\"");
    out.push_str(&json_escape(&exp.canonical_sparql));
    out.push_str("\",\"keywords\":[");
    for (i, k) in exp.keywords.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"keyword\":\"{}\",\"iri\":\"{}\"}}",
            json_escape(&k.keyword),
            json_escape(&k.iri)
        ));
    }
    out.push_str("],\"resolutions\":[");
    for (i, r) in exp.resolutions.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let runner = r
            .runner_up
            .as_deref()
            .map(|s| format!("\"{}\"", json_escape(s)))
            .unwrap_or_else(|| "null".to_string());
        out.push_str(&format!(
            "{{\"phrase\":\"{}\",\"iri\":\"{}\",\"score\":{:.4},\"confidence\":{:.4},\
             \"runner_up\":{},\"method\":\"{}\"}}",
            json_escape(&r.phrase),
            json_escape(&r.iri),
            r.score,
            r.confidence,
            runner,
            r.method.as_str()
        ));
    }
    out.push_str("]}");
    println!("{out}");
}
