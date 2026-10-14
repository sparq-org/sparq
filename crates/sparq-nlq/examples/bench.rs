//! sparq-nlq offline NL→SPARQL micro-benchmark. sq-ncvq.12. Run:
//!     cargo run -p sparq-nlq --example bench --release
//!
//! Measures the crate's deterministic, OFFLINE core operations — NO network, NO
//! `live` feature, no Anthropic call. The LLM is a tiny in-example stub that
//! returns a fixed valid SPARQL completion, so the loop exercises the real
//! retrieve→generate→validate(spargebra)→execute path against a synthetic graph:
//!   1. grounding — `Nlq::with_config` (the sparq-introspect schema scan +
//!      token-budgeted summary render: the once-per-graph cost);
//!   2. prompt — `prompt_for(question)` (schema-grounded prompt construction);
//!   3. ask loop — one full `ask` (extract SPARQL → spargebra validate →
//!      budgeted execute), with the stub LLM, best-of-K.
//!
//! NON-CANONICAL timing (this box is not a quiet bench instance) — the numbers are
//! a sanity-scale signal, not a published baseline.

use sparq_core::Graph;
use sparq_nlq::{Llm, Nlq};
use std::time::Instant;

/// Deterministic offline stub: returns a fixed, schema-valid SPARQL query in a
/// fenced block (the shape `extract_sparql` expects), independent of the prompt —
/// so the bench measures the loop, never an LLM. NOT a `ReplayLlm` (that needs an
/// exact prompt match); this is the minimal happy-path `Llm` for benchmarking.
struct FixedLlm(String);

impl Llm for FixedLlm {
    fn complete(&self, _prompt: &str) -> Result<String, String> {
        Ok(format!("Here is the query:\n```sparql\n{}\n```", self.0))
    }
}

/// Synthetic typed graph: `n` people of one of two types with a label + an age, so
/// the introspection scan sees real predicate/type structure to summarise and the
/// generated SELECT returns a non-empty result.
fn synth_graph(n: usize) -> Graph {
    let mut ttl = String::from(
        "@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
         @prefix ex: <http://ex/> .\n",
    );
    for i in 0..n {
        let ty = if i % 2 == 0 { "Person" } else { "Org" };
        ttl.push_str(&format!(
            "ex:e{i} rdf:type ex:{ty} ; rdfs:label \"entity {i}\" ; ex:age {} .\n",
            20 + (i % 50)
        ));
    }
    Graph::load_str(&ttl, "turtle").expect("synthetic graph parses")
}

fn timed<T>(label: &str, iters: u32, mut f: impl FnMut() -> T) -> T {
    let mut best = f64::MAX;
    let mut out = None;
    for _ in 0..iters {
        let t = Instant::now();
        out = Some(f());
        best = best.min(t.elapsed().as_secs_f64() * 1e6);
    }
    println!("{label:48} {best:12.2} us");
    out.unwrap()
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(2000);
    let graph = synth_graph(n);
    println!("synthetic graph: {n} entities, {} triples\n", graph.len());

    // The query the stub LLM "generates" — valid against the synthetic schema.
    let sparql = "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
                  SELECT ?type (COUNT(?s) AS ?count)\n\
                  WHERE { ?s rdf:type ?type }\n\
                  GROUP BY ?type";

    // 1. grounding: the introspection scan + summary render, once per graph.
    let nlq = timed("ground (introspect scan + summary)", 5, || {
        Nlq::new(&graph, Box::new(FixedLlm(sparql.to_string())))
    });

    // 2. prompt construction (deterministic, grounding-free of further index work).
    let prompt = timed("prompt_for (build grounded prompt)", 100, || {
        nlq.prompt_for("How many entities of each type are there?")
    });
    println!("  prompt: {} chars\n", prompt.len());

    // 3. one full ask loop: extract → spargebra validate → budgeted execute.
    let answer = timed("ask (extract + validate + execute)", 50, || {
        nlq.ask("How many entities of each type are there?")
            .expect("offline ask succeeds with the fixed stub")
    });
    println!(
        "  answered in {} repair round(s); result rows = {}",
        answer.repairs,
        answer.result.len()
    );

    println!("\nok");
}
