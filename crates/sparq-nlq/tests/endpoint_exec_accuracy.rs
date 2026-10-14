//! Exec-accuracy measurement wired to [`EndpointLlm`] — bead sq-2m6zm.7.
//!
//! Mirrors `exec_accuracy.rs` / `live_exec_accuracy` but drives the loop through the
//! **`nlq-endpoint` feature**'s [`EndpointLlm`] (a provider-agnostic OpenAI-compatible
//! chat-completions client) instead of the Anthropic `live` client. This lets operators
//! measure end-to-end NL→SPARQL→run exec-accuracy against ANY cheap-model endpoint
//! (local Ollama/vLLM/llama.cpp server, OpenRouter, direct OpenAI, etc.).
//!
//! ## CI / gate behaviour
//!
//! **NEVER runs in CI**: the test is both `#[cfg(feature = "nlq-endpoint")]`-gated AND
//! `#[ignore]`'d. It requires:
//!   - `SPARQ_NLQ_ENDPOINT_URL` — the endpoint base URL (e.g. `http://localhost:11434/v1`)
//!   - `SPARQ_NLQ_ENDPOINT_MODEL` — the model id the endpoint expects
//!   - `SPARQ_NLQ_ENDPOINT_KEY` — bearer token (optional; omit for local endpoints)
//!   - `SPARQ_OLYMPICS_NT` (optional) — path to `olympics.nt`; defaults to the standard
//!     bench fixture path; the test SKIPS if the file is absent.
//!
//! ## How to run
//!
//! ```sh
//! # Against a local Ollama instance with llama3.1:
//! SPARQ_NLQ_ENDPOINT_URL=http://localhost:11434/v1 \
//! SPARQ_NLQ_ENDPOINT_MODEL=llama3.1 \
//! cargo test -p sparq-nlq --features nlq-endpoint --test endpoint_exec_accuracy \
//!   -- --ignored endpoint_exec_accuracy
//!
//! # Against OpenAI gpt-4o-mini:
//! SPARQ_NLQ_ENDPOINT_URL=https://api.openai.com/v1 \
//! SPARQ_NLQ_ENDPOINT_MODEL=gpt-4o-mini \
//! SPARQ_NLQ_ENDPOINT_KEY=sk-... \
//! cargo test -p sparq-nlq --features nlq-endpoint --test endpoint_exec_accuracy \
//!   -- --ignored endpoint_exec_accuracy
//! ```
//!
//! ## What is measured
//!
//! The **four-cell comparison** the design doc (`research/genai-design.md` §4) requires:
//! grounded vs ungrounded end-to-end, and oracle-linking (engine-side only). Per-question
//! provenance is printed to stderr: generated SPARQL, whether it ran, and F1 vs gold.
//! The headline claim — grounded end-to-end macro-F1 > ungrounded — is asserted.
//!
//! Quality is entirely the user-chosen model's, not sparq's. Results are printed at run
//! time only; nothing is baked into committed markdown.
#![cfg(feature = "nlq-endpoint")]

use sparq_core::Graph;
use sparq_nlq::endpoint::{EndpointConfig, EndpointLlm};
use sparq_nlq::eval::{run_comparison, EvalCase};
use sparq_nlq::{Llm, NlqConfig, RecordingLlm};
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn olympics_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("SPARQ_OLYMPICS_NT") {
        return Some(p.into());
    }
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../bench/qlever-olympics/olympics.nt");
    p.exists().then_some(p)
}

/// (question, gold SPARQL) — same set as `exec_accuracy.rs::olympics_cases()`. The gold
/// is executed on the live graph at score time, so there is no checked-in answer blob.
fn olympics_cases() -> Vec<EvalCase> {
    vec![
        EvalCase::new(
            "How many athletes are in the dataset?",
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
             SELECT (COUNT(?athlete) AS ?count)\n\
             WHERE { ?athlete rdf:type foaf:Person }",
        ),
        EvalCase::new(
            "Which team has the most athletes?",
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
             PREFIX dbo: <http://dbpedia.org/ontology/>\n\
             SELECT ?team (COUNT(?athlete) AS ?count)\n\
             WHERE { ?athlete rdf:type foaf:Person ; dbo:team ?team }\n\
             GROUP BY ?team\n\
             ORDER BY DESC(?count)\n\
             LIMIT 1",
        ),
        EvalCase::new(
            "Are there any athletes taller than 200 centimetres?",
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
             PREFIX dbo: <http://dbpedia.org/ontology/>\n\
             ASK { ?athlete rdf:type foaf:Person ; dbo:height ?height . FILTER(?height > 200) }",
        ),
        EvalCase::new(
            "List the year and host city of every Olympic games.",
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX dbo: <http://dbpedia.org/ontology/>\n\
             PREFIX dbp: <http://dbpedia.org/property/>\n\
             SELECT ?games ?year ?city\n\
             WHERE { ?games rdf:type dbo:Olympics ; dbp:year ?year ; dbp:location ?city }\n\
             ORDER BY ?year",
        ),
        EvalCase::new(
            "How many medals of each type were awarded?",
            "PREFIX ns6: <http://wallscope.co.uk/ontology/olympics/>\n\
             PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\n\
             SELECT ?medal (COUNT(?participation) AS ?count)\n\
             WHERE { ?participation ns6:medal ?m . ?m rdfs:label ?medal }\n\
             GROUP BY ?medal\n\
             ORDER BY DESC(?count)",
        ),
        EvalCase::new(
            "What is the average height of the athletes?",
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
             PREFIX dbo: <http://dbpedia.org/ontology/>\n\
             SELECT (AVG(?height) AS ?avgHeight)\n\
             WHERE { ?athlete rdf:type foaf:Person ; dbo:height ?height }",
        ),
        EvalCase::new(
            "How many athletes are on each team?",
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
             PREFIX dbo: <http://dbpedia.org/ontology/>\n\
             SELECT ?team (COUNT(?athlete) AS ?count)\n\
             WHERE { ?athlete rdf:type foaf:Person ; dbo:team ?team }\n\
             GROUP BY ?team\n\
             ORDER BY DESC(?count)",
        ),
        EvalCase::new(
            "List all sports with their labels.",
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\n\
             PREFIX dbo: <http://dbpedia.org/ontology/>\n\
             SELECT ?sport ?label\n\
             WHERE { ?sport rdf:type dbo:Sport ; rdfs:label ?label }\n\
             ORDER BY ?label",
        ),
    ]
}

/// Print per-question provenance: the generated SPARQL, whether it ran, and the F1 score.
fn print_case_provenance(case: &sparq_nlq::eval::CaseResult) {
    match &case.outcome {
        Ok(score) => {
            eprintln!(
                "  [ok   ] f1={:.3} EM={} repairs={} q={}",
                score.f1.f1,
                score.f1.is_exact(),
                score.repairs,
                case.question.lines().next().unwrap_or(""),
            );
            // Print the generated SPARQL indented so a human can sanity-check linking.
            for line in score.sparql.lines() {
                eprintln!("           {line}");
            }
        }
        Err(e) => {
            eprintln!(
                "  [fail ] f1=0.000 EM=false q={} err={}",
                case.question.lines().next().unwrap_or(""),
                e.lines().next().unwrap_or(e),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The live endpoint measurement
// ---------------------------------------------------------------------------

/// End-to-end exec-accuracy against a real cheap-model endpoint — bead sq-2m6zm.7.
///
/// Requires:
///   - `SPARQ_NLQ_ENDPOINT_URL` (e.g. `http://localhost:11434/v1`)
///   - `SPARQ_NLQ_ENDPOINT_MODEL` (e.g. `llama3.1`)
///   - `SPARQ_NLQ_ENDPOINT_KEY` (optional bearer token)
///   - The olympics.nt dataset (defaults to `bench/qlever-olympics/olympics.nt` relative
///     to the workspace root; override with `SPARQ_OLYMPICS_NT`).
///
/// Skips cleanly if `olympics.nt` is absent. Fails with an actionable message if the
/// endpoint env vars are missing.
///
/// Results are printed to stderr as a four-cell comparison table (grounded vs ungrounded,
/// end-to-end vs oracle) plus per-question provenance. Nothing is written to committed
/// markdown — quality is the user-chosen model's.
#[test]
#[ignore = "live endpoint: needs SPARQ_NLQ_ENDPOINT_URL + SPARQ_NLQ_ENDPOINT_MODEL + \
            SPARQ_NLQ_ENDPOINT_KEY (optional) + the olympics.nt dataset"]
fn endpoint_exec_accuracy() {
    let Some(dataset_path) = olympics_path() else {
        eprintln!("skipping: olympics.nt not present (set SPARQ_OLYMPICS_NT or place at \
                   bench/qlever-olympics/olympics.nt)");
        return;
    };

    // Build EndpointLlm from env — fails loudly if the required vars are absent.
    let cfg = EndpointConfig::from_env().unwrap_or_else(|e| {
        panic!(
            "endpoint not configured: {e}\n\
             Set SPARQ_NLQ_ENDPOINT_URL and SPARQ_NLQ_ENDPOINT_MODEL (SPARQ_NLQ_ENDPOINT_KEY \
             is optional for local endpoints)."
        )
    });
    let model = cfg.model.clone();
    eprintln!("endpoint: {} model={}", cfg.base_url, model);

    let text = std::fs::read_to_string(&dataset_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", dataset_path.display()));
    eprintln!("loading {} ...", dataset_path.display());
    let graph = Graph::load_str(&text, "ntriples").expect("parse olympics.nt");
    drop(text);
    eprintln!("loaded {} triples", graph.len());

    let cases = olympics_cases();
    let loop_cfg = NlqConfig {
        max_repair_rounds: 3,
        ..NlqConfig::default()
    };

    // Wrap each EndpointLlm in a RecordingLlm so the session is replayable. Two
    // independent instances (grounded vs ungrounded prompts are distinct strings).
    let recorders: Vec<Rc<RecordingLlm<EndpointLlm>>> = (0..2)
        .map(|_| {
            let llm = EndpointLlm::new(cfg.clone()).expect("build EndpointLlm");
            Rc::new(RecordingLlm::new(llm))
        })
        .collect();
    let rec0 = Rc::clone(&recorders[0]);
    let rec1 = Rc::clone(&recorders[1]);
    let mut slot = 0usize;
    let comparison = run_comparison(&graph, &cases, &loop_cfg, move |_ground| {
        let rec = if slot == 0 {
            Rc::clone(&rec0)
        } else {
            Rc::clone(&rec1)
        };
        slot += 1;
        Box::new(rec) as Box<dyn Llm>
    });

    eprintln!("\n=== endpoint exec-accuracy (model={model}) ===");
    eprintln!("{}", comparison.summary());

    // Per-question provenance — grounded end-to-end only (the primary axis).
    eprintln!("\n--- grounded end-to-end per-question ---");
    for case in &comparison.grounded_end_to_end.cases {
        print_case_provenance(case);
    }
    eprintln!("\n--- ungrounded end-to-end per-question ---");
    for case in &comparison.ungrounded_end_to_end.cases {
        print_case_provenance(case);
    }

    // Persist the recorded sessions alongside the committed fixture directory.
    let fixture_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::create_dir_all(&fixture_dir).ok();
    for (i, rec) in recorders.iter().enumerate() {
        if !rec.exchanges().is_empty() {
            let out = fixture_dir.join(format!("endpoint_session_{i}.json"));
            match rec.save(&out) {
                Ok(()) => eprintln!("wrote {} exchanges to {}", rec.exchanges().len(), out.display()),
                Err(e) => eprintln!("warning: could not write session fixture: {e}"),
            }
        }
    }

    // Honest headline check: grounding must pay for itself.
    assert!(
        comparison.headline_grounding_pays(),
        "ENDPOINT: grounding must pay for itself — \
         grounded macroF1={:.3} vs ungrounded macroF1={:.3} (model={})",
        comparison.grounded_end_to_end.macro_f1,
        comparison.ungrounded_end_to_end.macro_f1,
        model,
    );
}

// ---------------------------------------------------------------------------
// Offline gate: EndpointLlm API wiring compiles and the config reads from env.
// This test ALWAYS runs in CI (no #[ignore]) and needs no network.
// ---------------------------------------------------------------------------

/// Confirms that [`EndpointConfig::from_env`] returns a descriptive error when the
/// required env vars are absent, and that [`EndpointLlm::new`] succeeds from a valid
/// config. Exercises the public API used by `endpoint_exec_accuracy` without a live call.
#[test]
fn endpoint_config_missing_vars_gives_actionable_error() {
    // Serialize env access against the process-wide lock (same pattern as endpoint.rs
    // unit tests — `set_var`/`remove_var` are not thread-safe across test threads).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    // Stash and clear the endpoint vars so this test is self-contained.
    let saved_url = std::env::var("SPARQ_NLQ_ENDPOINT_URL").ok();
    let saved_model = std::env::var("SPARQ_NLQ_ENDPOINT_MODEL").ok();
    let saved_key = std::env::var("SPARQ_NLQ_ENDPOINT_KEY").ok();
    std::env::remove_var("SPARQ_NLQ_ENDPOINT_URL");
    std::env::remove_var("SPARQ_NLQ_ENDPOINT_MODEL");
    std::env::remove_var("SPARQ_NLQ_ENDPOINT_KEY");

    // Missing URL: error must name the variable.
    let e = EndpointConfig::from_env().unwrap_err();
    assert!(
        e.contains("SPARQ_NLQ_ENDPOINT_URL"),
        "missing-URL error must name the variable: {e}"
    );

    // URL set but model missing.
    std::env::set_var("SPARQ_NLQ_ENDPOINT_URL", "http://localhost:11434/v1");
    let e2 = EndpointConfig::from_env().unwrap_err();
    assert!(
        e2.contains("SPARQ_NLQ_ENDPOINT_MODEL"),
        "missing-model error must name the variable: {e2}"
    );

    // Both set, key absent (optional) → succeeds; EndpointLlm builds from it.
    std::env::set_var("SPARQ_NLQ_ENDPOINT_MODEL", "test-model");
    let cfg = EndpointConfig::from_env().expect("URL + model sufficient");
    assert!(cfg.api_key.is_none(), "absent key is None");
    let llm = EndpointLlm::new(cfg).expect("build EndpointLlm from valid config");
    assert_eq!(llm.model(), "test-model");

    // Restore.
    std::env::remove_var("SPARQ_NLQ_ENDPOINT_URL");
    std::env::remove_var("SPARQ_NLQ_ENDPOINT_MODEL");
    if let Some(v) = saved_url {
        std::env::set_var("SPARQ_NLQ_ENDPOINT_URL", v);
    }
    if let Some(v) = saved_model {
        std::env::set_var("SPARQ_NLQ_ENDPOINT_MODEL", v);
    }
    if let Some(v) = saved_key {
        std::env::set_var("SPARQ_NLQ_ENDPOINT_KEY", v);
    }
}
