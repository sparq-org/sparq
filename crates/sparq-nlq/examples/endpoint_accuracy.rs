//! Standalone exec-accuracy runner for the `nlq-endpoint` feature — bead sq-2m6zm.7.
//! [SONNET-4.6]
//!
//! Measures end-to-end NL→SPARQL→run exec-accuracy against any OpenAI-compatible
//! cheap-model endpoint (Ollama, llama.cpp server, vLLM, OpenRouter, OpenAI, etc.)
//! on the olympics benchmark dataset, reporting the four-cell comparison the design doc
//! requires (`research/genai-design.md` §4): grounded vs ungrounded end-to-end, and
//! oracle-linking (engine-side only, no model).
//!
//! ## Running
//!
//! ```sh
//! # Against a local Ollama instance serving llama3.1:
//! SPARQ_NLQ_ENDPOINT_URL=http://localhost:11434/v1 \
//! SPARQ_NLQ_ENDPOINT_MODEL=llama3.1 \
//! cargo run -p sparq-nlq --example endpoint_accuracy --features nlq-endpoint --release
//!
//! # Against OpenAI gpt-4o-mini (key required):
//! SPARQ_NLQ_ENDPOINT_URL=https://api.openai.com/v1 \
//! SPARQ_NLQ_ENDPOINT_MODEL=gpt-4o-mini \
//! SPARQ_NLQ_ENDPOINT_KEY=sk-...
//! cargo run -p sparq-nlq --example endpoint_accuracy --features nlq-endpoint --release
//! ```
//!
//! ## Dataset
//!
//! Defaults to `bench/qlever-olympics/olympics.nt` (the 1.78M-triple Olympics benchmark
//! fixture used by the sparq bench suite). Override with `SPARQ_OLYMPICS_NT`.
//!
//! ## Output
//!
//! Prints a four-cell summary table + per-question provenance (generated SPARQL,
//! whether it ran, F1 vs gold answer set) to stdout. Optionally writes a JSON results
//! document with `--json <path>` (machine-readable, non-canonical timing advisory).
//! Saves the recorded (prompt, completion) session pairs to
//! `tests/fixtures/endpoint_session_{i}.json` for replay/regression.
//!
//! Nothing is committed: quality numbers belong in `research/nlq-exec-accuracy-2026-07.md`,
//! not hardcoded in source.

fn main() {
    #[cfg(not(feature = "nlq-endpoint"))]
    {
        eprintln!(
            "error: this example requires the `nlq-endpoint` feature.\n\
             Run with: cargo run -p sparq-nlq --example endpoint_accuracy \
             --features nlq-endpoint --release"
        );
        std::process::exit(1);
    }
    #[cfg(feature = "nlq-endpoint")]
    inner::run();
}

#[cfg(feature = "nlq-endpoint")]
mod inner {
    use std::rc::Rc;

    use sparq_core::Graph;
    use sparq_nlq::endpoint::{EndpointConfig, EndpointLlm};
    use sparq_nlq::eval::{run_comparison, CaseResult, EvalCase};
    use sparq_nlq::{Llm, NlqConfig, RecordingLlm};

    // ---------------------------------------------------------------------------
    // Eval set: olympics questions with gold SPARQL
    // ---------------------------------------------------------------------------

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

    // ---------------------------------------------------------------------------
    // Per-question provenance printer
    // ---------------------------------------------------------------------------

    pub fn print_provenance(label: &str, cases: &[CaseResult]) {
        println!("\n--- {label} per-question provenance ---");
        for case in cases {
            match &case.outcome {
                Ok(score) => {
                    println!(
                        "  [ok   ] f1={:.3} EM={} repairs={} | {}",
                        score.f1.f1,
                        score.f1.is_exact(),
                        score.repairs,
                        case.question,
                    );
                    for line in score.sparql.lines() {
                        println!("           {line}");
                    }
                }
                Err(e) => {
                    println!(
                        "  [fail ] f1=0.000 | {} | err: {}",
                        case.question,
                        e.lines().next().unwrap_or(e),
                    );
                }
            }
        }
    }

    // ---------------------------------------------------------------------------
    // --json <path> output helpers (same pattern as record_olympics.rs)
    // ---------------------------------------------------------------------------

    pub fn take_json_flag(args: Vec<String>) -> (Vec<String>, Option<String>) {
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

    pub fn json_str(raw: &str) -> String {
        let mut out = String::with_capacity(raw.len() + 2);
        out.push('"');
        sparq_core::json::escape_into(&mut out, raw);
        out.push('"');
        out
    }

    pub fn results_json(
        model: &str,
        endpoint: &str,
        triples: usize,
        grounded_macro_f1: f64,
        ungrounded_macro_f1: f64,
        grounding_pays: bool,
        cases: &[CaseResult],
    ) -> String {
        let mut s = String::new();
        s.push_str("{\n");
        s.push_str("  \"harness\": \"sparq-nlq endpoint_accuracy\",\n");
        s.push_str(&format!("  \"model\": {},\n", json_str(model)));
        s.push_str(&format!("  \"endpoint\": {},\n", json_str(endpoint)));
        s.push_str(&format!("  \"dataset_triples\": {triples},\n"));
        s.push_str(&format!(
            "  \"grounded_macro_f1\": {grounded_macro_f1:.6},\n"
        ));
        s.push_str(&format!(
            "  \"ungrounded_macro_f1\": {ungrounded_macro_f1:.6},\n"
        ));
        s.push_str(&format!("  \"grounding_pays\": {grounding_pays},\n"));
        s.push_str(
            "  \"note\": \"exec-accuracy on the 8-question olympics eval set. \
             Quality is the user-chosen model's; timing is NON-CANONICAL (this box). \
             Do not bake these numbers into committed markdown.\",\n",
        );
        s.push_str("  \"per_question\": [\n");
        for (i, case) in cases.iter().enumerate() {
            let comma = if i + 1 < cases.len() { "," } else { "" };
            match &case.outcome {
                Ok(score) => {
                    s.push_str(&format!(
                        "    {{ \"question\": {}, \"f1\": {:.6}, \"exact\": {}, \
                         \"repairs\": {}, \"sparql\": {} }}{comma}\n",
                        json_str(&case.question),
                        score.f1.f1,
                        score.f1.is_exact(),
                        score.repairs,
                        json_str(&score.sparql),
                    ));
                }
                Err(e) => {
                    s.push_str(&format!(
                        "    {{ \"question\": {}, \"f1\": 0.0, \"exact\": false, \
                         \"repairs\": null, \"error\": {} }}{comma}\n",
                        json_str(&case.question),
                        json_str(e),
                    ));
                }
            }
        }
        s.push_str("  ]\n");
        s.push_str("}\n");
        s
    }

    // ---------------------------------------------------------------------------
    // run() — the actual measurement
    // ---------------------------------------------------------------------------

    pub fn run() {
        let (_args, json_path) = take_json_flag(std::env::args().collect());

        // 1. Endpoint config — actionable error if vars are missing.
        let cfg = EndpointConfig::from_env().unwrap_or_else(|e| {
            eprintln!("error: {e}");
            eprintln!();
            eprintln!("Set the following environment variables:");
            eprintln!(
                "  SPARQ_NLQ_ENDPOINT_URL   - base URL, e.g. http://localhost:11434/v1"
            );
            eprintln!("  SPARQ_NLQ_ENDPOINT_MODEL - model id, e.g. llama3.1 or gpt-4o-mini");
            eprintln!("  SPARQ_NLQ_ENDPOINT_KEY   - bearer token (optional; skip for local)");
            eprintln!();
            eprintln!("Example (Ollama):");
            eprintln!(
                "  SPARQ_NLQ_ENDPOINT_URL=http://localhost:11434/v1 \
                 SPARQ_NLQ_ENDPOINT_MODEL=llama3.1 \\"
            );
            eprintln!(
                "  cargo run -p sparq-nlq --example endpoint_accuracy \
                 --features nlq-endpoint --release"
            );
            std::process::exit(1);
        });
        let model = cfg.model.clone();
        let endpoint_url = cfg.base_url.clone();
        println!("endpoint: {endpoint_url}  model: {model}");

        // 2. Load the dataset.
        let dataset_path = std::env::var("SPARQ_OLYMPICS_NT").unwrap_or_else(|_| {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../bench/qlever-olympics/olympics.nt"
            )
            .to_string()
        });
        let Ok(text) = std::fs::read_to_string(&dataset_path) else {
            eprintln!(
                "olympics.nt not found at {dataset_path}\n\
                 Download it and set SPARQ_OLYMPICS_NT=/path/to/olympics.nt\n\
                 (QLever olympics benchmark fixture — 1.78M triples)"
            );
            std::process::exit(1);
        };
        println!("loading {dataset_path} ...");
        let graph = Graph::load_str(&text, "ntriples").expect("parse olympics.nt");
        drop(text);
        let triples = graph.len();
        println!("loaded {triples} triples");

        let cases = olympics_cases();
        let loop_cfg = NlqConfig {
            max_repair_rounds: 3,
            ..NlqConfig::default()
        };

        // 3. Build two independent RecordingLlm<EndpointLlm> instances.
        let recorders: Vec<Rc<RecordingLlm<EndpointLlm>>> = (0..2)
            .map(|_| {
                let llm = EndpointLlm::new(cfg.clone()).expect("build EndpointLlm");
                Rc::new(RecordingLlm::new(llm))
            })
            .collect();
        let rec0 = Rc::clone(&recorders[0]);
        let rec1 = Rc::clone(&recorders[1]);
        let mut slot = 0usize;

        // 4. Run the four-cell comparison.
        println!("\nrunning {}-question eval set ...", cases.len());
        let comparison = run_comparison(&graph, &cases, &loop_cfg, move |_ground| {
            let rec: Box<dyn Llm> = if slot == 0 {
                Box::new(Rc::clone(&rec0))
            } else {
                Box::new(Rc::clone(&rec1))
            };
            slot += 1;
            rec
        });

        // 5. Print results.
        println!("\n=== endpoint exec-accuracy (model={model}) ===");
        println!("{}", comparison.summary());
        print_provenance(
            "grounded end-to-end",
            &comparison.grounded_end_to_end.cases,
        );
        print_provenance(
            "ungrounded end-to-end",
            &comparison.ungrounded_end_to_end.cases,
        );

        // 6. Persist recorded sessions for replay/regression.
        let fixture_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        std::fs::create_dir_all(&fixture_dir).ok();
        for (i, rec) in recorders.iter().enumerate() {
            if !rec.exchanges().is_empty() {
                let out = fixture_dir.join(format!("endpoint_session_{i}.json"));
                match rec.save(&out) {
                    Ok(()) => println!(
                        "\nwrote {} exchanges to {}",
                        rec.exchanges().len(),
                        out.display()
                    ),
                    Err(e) => eprintln!("warning: could not save session: {e}"),
                }
            }
        }

        // 7. Optional --json emit.
        if let Some(path) = json_path {
            let doc = results_json(
                &model,
                &endpoint_url,
                triples,
                comparison.grounded_end_to_end.macro_f1,
                comparison.ungrounded_end_to_end.macro_f1,
                comparison.headline_grounding_pays(),
                &comparison.grounded_end_to_end.cases,
            );
            if let Err(e) = std::fs::write(&path, doc) {
                eprintln!("error writing --json results to {path}: {e}");
                std::process::exit(1);
            }
            println!("wrote JSON results to {path}");
        }

        // 8. Headline verdict.
        if comparison.headline_grounding_pays() {
            println!("\nok — grounding pays for itself on this endpoint/model.");
        } else {
            eprintln!(
                "\nWARNING: grounding did NOT pay for itself on this run — \
                 grounded macroF1={:.3} vs ungrounded macroF1={:.3}. \
                 Check the per-question provenance above.",
                comparison.grounded_end_to_end.macro_f1,
                comparison.ungrounded_end_to_end.macro_f1,
            );
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests for the JSON emit helpers (offline — no network)
// ---------------------------------------------------------------------------
#[cfg(test)]
#[cfg(feature = "nlq-endpoint")]
mod tests {
    use super::inner::{json_str, results_json, take_json_flag};

    #[test]
    fn json_str_escapes_correctly() {
        assert_eq!(json_str("hello"), "\"hello\"");
        assert_eq!(json_str("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_str("a\\b"), "\"a\\\\b\"");
        assert_eq!(json_str("a\nb"), "\"a\\nb\"");
    }

    #[test]
    fn take_json_flag_parses_flag_and_leaves_positionals() {
        let args: Vec<String> = ["endpoint_accuracy", "--json", "/tmp/r.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (pos, path) = take_json_flag(args);
        assert_eq!(pos, vec!["endpoint_accuracy"]);
        assert_eq!(path.as_deref(), Some("/tmp/r.json"));

        let plain: Vec<String> = ["endpoint_accuracy"].iter().map(|s| s.to_string()).collect();
        let (p2, none) = take_json_flag(plain.clone());
        assert_eq!(p2, plain);
        assert!(none.is_none());
    }

    #[test]
    fn results_json_is_valid_json_and_has_required_keys() {
        use sparq_nlq::eval::{CaseResult, CaseScore, F1};
        let cases = vec![
            CaseResult {
                question: "How many athletes?".into(),
                outcome: Ok(CaseScore {
                    f1: F1 {
                        precision: 1.0,
                        recall: 1.0,
                        f1: 1.0,
                        intersection: 1,
                        predicted: 1,
                        gold: 1,
                    },
                    sparql: "SELECT * WHERE { ?s ?p ?o }".into(),
                    repairs: 0,
                }),
            },
            CaseResult {
                question: "Unanswerable?".into(),
                outcome: Err("loop failure".into()),
            },
        ];

        let doc = results_json(
            "test-model",
            "http://localhost:11434/v1",
            42,
            1.0,
            0.5,
            true,
            &cases,
        );
        let v: serde_json::Value = serde_json::from_str(&doc).expect("must be valid JSON");
        assert_eq!(v["model"], "test-model");
        assert_eq!(v["dataset_triples"], 42);
        assert_eq!(v["grounding_pays"], true);
        assert!((v["grounded_macro_f1"].as_f64().unwrap() - 1.0).abs() < 1e-9);
        let arr = v["per_question"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert!((arr[0]["f1"].as_f64().unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(arr[0]["exact"], true);
        assert_eq!(arr[0]["repairs"], 0);
        assert!(arr[1].get("error").is_some());
    }
}
