//! Re-records the olympics replay fixture (`tests/fixtures/olympics_replay.json`).
//!
//! The "LLM" here is scripted: the completions are hand-written, realistic answers a
//! competent model would give for the grounding prompt this crate builds (they use
//! only vocabulary visible in the schema summary; one of them — Q8 — is deliberately
//! malformed on the first round so the fixture exercises the repair path). The
//! script is wrapped in [`RecordingLlm`], driven through the REAL `Nlq::ask` loop
//! against the REAL dataset, and the recorded (prompt, completion) pairs are written
//! out. Run this again whenever the prompt template, the default `NlqConfig`, or the
//! dataset changes:
//!
//! ```sh
//! cargo run -p sparq-nlq --example record_olympics --release
//! ```
//!
//! Needs `bench/qlever-olympics/olympics.nt` (downloaded benchmark fixture; override
//! the path with `SPARQ_OLYMPICS_NT`).
//!
//! (sq-k5qq) Strictly-additive machine-readable emit:
//!
//! ```sh
//! cargo run -p sparq-nlq --example record_olympics --release -- --json /tmp/nlq.json
//! ```
//!
//! `--json <path>` writes the SAME per-question rows STDOUT prints — `rows`/`repairs`
//! (DETERMINISTIC at the recorded fixture) plus the advisory `ms` timing — as a stable,
//! DEPENDENCY-FREE JSON document, mirroring the `format!`-JSON convention of
//! `crates/sparq-mpc/examples/mpc_net_bench.rs::cell_json`. No serde dep is added to the
//! harness (serde_json is a normal crate dep used by the round-trip test). STDOUT is
//! byte-for-byte unchanged whether or not the flag is present; `ms` is ADVISORY +
//! NON-CANONICAL (this dev box) and nothing is committed.

use std::rc::Rc;

use sparq_core::Graph;
use sparq_nlq::{Llm, Nlq, RecordingLlm};

/// The eval set: (question, scripted first completion, scripted repair completion).
/// Kept in sync by hand with `tests/olympics_eval.rs` (which hardcodes the same
/// questions + expectations).
pub fn script() -> Vec<(&'static str, &'static str, Option<&'static str>)> {
    vec![
        (
            "How many athletes are on each team?",
            "```sparql\n\
             PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
             PREFIX dbo: <http://dbpedia.org/ontology/>\n\
             SELECT ?team (COUNT(?athlete) AS ?count)\n\
             WHERE {\n  ?athlete rdf:type foaf:Person ;\n           dbo:team ?team .\n}\n\
             GROUP BY ?team\n\
             ORDER BY DESC(?count)\n\
             ```",
            None,
        ),
        (
            "How many athletes are in the dataset?",
            "```sparql\n\
             PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
             SELECT (COUNT(?athlete) AS ?count)\n\
             WHERE { ?athlete rdf:type foaf:Person }\n\
             ```",
            None,
        ),
        (
            "List the year and host city of every Olympic games.",
            "```sparql\n\
             PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX dbo: <http://dbpedia.org/ontology/>\n\
             PREFIX dbp: <http://dbpedia.org/property/>\n\
             SELECT ?games ?year ?city\n\
             WHERE {\n  ?games rdf:type dbo:Olympics ;\n         dbp:year ?year ;\n         dbp:location ?city .\n}\n\
             ORDER BY ?year\n\
             ```",
            None,
        ),
        (
            // The participation records are untyped — the LLM has to read them off
            // the characteristic-set section of the summary ({ns6:athlete, ns6:event,
            // ns6:games, ns6:medal}) and join medals to their labels.
            "How many medals of each type were awarded?",
            "```sparql\n\
             PREFIX ns6: <http://wallscope.co.uk/ontology/olympics/>\n\
             PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\n\
             SELECT ?medal (COUNT(?participation) AS ?count)\n\
             WHERE {\n  ?participation ns6:medal ?m .\n  ?m rdfs:label ?medal .\n}\n\
             GROUP BY ?medal\n\
             ORDER BY DESC(?count)\n\
             ```",
            None,
        ),
        (
            "What is the average height of the athletes?",
            "```sparql\n\
             PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
             PREFIX dbo: <http://dbpedia.org/ontology/>\n\
             SELECT (AVG(?height) AS ?avgHeight)\n\
             WHERE { ?athlete rdf:type foaf:Person ; dbo:height ?height }\n\
             ```",
            None,
        ),
        (
            "Which team has the most athletes?",
            "```sparql\n\
             PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
             PREFIX dbo: <http://dbpedia.org/ontology/>\n\
             SELECT ?team (COUNT(?athlete) AS ?count)\n\
             WHERE { ?athlete rdf:type foaf:Person ; dbo:team ?team }\n\
             GROUP BY ?team\n\
             ORDER BY DESC(?count)\n\
             LIMIT 1\n\
             ```",
            None,
        ),
        (
            "List all sports with their labels.",
            "```sparql\n\
             PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\n\
             PREFIX dbo: <http://dbpedia.org/ontology/>\n\
             SELECT ?sport ?label\n\
             WHERE { ?sport rdf:type dbo:Sport ; rdfs:label ?label }\n\
             ORDER BY ?label\n\
             ```",
            None,
        ),
        (
            // THE REPAIR CASE: the first completion drops the closing parenthesis of
            // the aggregate alias (a classic LLM slip) — spargebra rejects it, the
            // loop sends the parse error back, the second completion is fixed.
            "How many events does each sport have?",
            "```sparql\n\
             PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\n\
             PREFIX dbo: <http://dbpedia.org/ontology/>\n\
             SELECT ?sport (COUNT(?event) AS ?count\n\
             WHERE { ?event rdf:type dbo:SportsEvent ; rdfs:subClassOf ?sport }\n\
             GROUP BY ?sport\n\
             ```",
            Some(
                "```sparql\n\
                 PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
                 PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\n\
                 PREFIX dbo: <http://dbpedia.org/ontology/>\n\
                 SELECT ?sport (COUNT(?event) AS ?count)\n\
                 WHERE { ?event rdf:type dbo:SportsEvent ; rdfs:subClassOf ?sport }\n\
                 GROUP BY ?sport\n\
                 ORDER BY DESC(?count)\n\
                 ```",
            ),
        ),
        (
            "Are there any athletes taller than 200 centimetres?",
            "```sparql\n\
             PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
             PREFIX dbo: <http://dbpedia.org/ontology/>\n\
             ASK {\n  ?athlete rdf:type foaf:Person ;\n           dbo:height ?height .\n  FILTER(?height > 200)\n}\n\
             ```",
            None,
        ),
    ]
}

/// Answers by matching the question line in the prompt; serves the repair completion
/// when the prompt carries the loop's failure marker.
struct ScriptedLlm {
    script: Vec<(&'static str, &'static str, Option<&'static str>)>,
}

impl Llm for ScriptedLlm {
    fn complete(&self, prompt: &str) -> Result<String, String> {
        let repair = prompt.contains("Your previous query failed");
        for (q, first, fix) in &self.script {
            if prompt.contains(&format!("Question: {q}")) {
                return if repair {
                    fix.map(|f| f.to_string())
                        .ok_or_else(|| format!("no scripted repair for {q:?}"))
                } else {
                    Ok(first.to_string())
                };
            }
        }
        Err(format!(
            "no scripted answer for prompt ending {:?}",
            prompt.lines().last()
        ))
    }
}

// ---------------------------------------------------------------------------
// (sq-k5qq) --json <path> machine-readable results emit
// ---------------------------------------------------------------------------

/// One captured per-question row. `rows`/`repairs` are DETERMINISTIC at the recorded
/// fixture (a pure function of the dataset + scripted completions); `ms` is best-effort
/// (ADVISORY, NON-CANONICAL on this box).
struct Row {
    question: String,
    rows: usize,
    repairs: usize,
    ms: f64,
}

/// Extracts `--json <path>` from argv, returning argv WITHOUT the flag pair (so any
/// positional parsing is unchanged). A bare `--json` is a usage error (exit 2),
/// mirroring `sparq-cli` / `bench_text`'s identical flag.
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

/// Minimal JSON string escaper (the dependency-free emit). Questions can carry any
/// Unicode; this covers the realistic input and yields valid `\uXXXX` for control chars.
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    sparq_core::json::escape_into(&mut out, raw);
    out.push('"');
    out
}

/// Serialise the recorded run to stable, dependency-free JSON. Each `rows` object carries
/// the SAME fields the STDOUT line prints; `rows`/`repairs` are DETERMINISTIC, `ms` is
/// ADVISORY + NON-CANONICAL (stated in `note`).
fn results_json(triples: usize, rows: &[Row]) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"harness\": \"sparq-nlq record_olympics\",\n");
    s.push_str(&format!("  \"triples\": {triples},\n"));
    s.push_str(
        "  \"note\": \"`rows`/`repairs` are DETERMINISTIC at the recorded fixture (a pure \
         function of the dataset + scripted completions); `ms` is best-effort per-question \
         latency MEASURED on the running host — ADVISORY, NON-CANONICAL (this dev box) — do \
         not bake into committed files\",\n",
    );
    s.push_str("  \"questions\": [\n");
    for (i, r) in rows.iter().enumerate() {
        let comma = if i + 1 < rows.len() { "," } else { "" };
        s.push_str(&format!(
            "    {{ \"question\": {}, \"rows\": {}, \"repairs\": {}, \"ms\": {:.3} }}{comma}\n",
            json_str(&r.question),
            r.rows,
            r.repairs,
            r.ms,
        ));
    }
    s.push_str("  ]\n");
    s.push_str("}\n");
    s
}

fn main() {
    let (_args, json_path) = take_json_flag(std::env::args().collect());

    let path = std::env::var("SPARQ_OLYMPICS_NT").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../bench/qlever-olympics/olympics.nt"
        )
        .into()
    });
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("olympics.nt not found at {path} (set SPARQ_OLYMPICS_NT) — nothing recorded");
        return;
    };
    let graph = Graph::load_str(&text, "ntriples").expect("parse olympics.nt");
    drop(text);
    println!("loaded {} triples", graph.len());

    let recorder = Rc::new(RecordingLlm::new(ScriptedLlm { script: script() }));
    let nlq = Nlq::new(&graph, Box::new(Rc::clone(&recorder)));

    let mut measured: Vec<Row> = Vec::new();
    for (question, _, _) in script() {
        let t0 = std::time::Instant::now();
        let a = nlq
            .ask(question)
            .unwrap_or_else(|e| panic!("{question:?} failed: {e}\n{:#?}", e.transcript));
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        println!(
            "ok ({} rows, {} repair(s), {ms:.1} ms): {question}",
            a.result.len(),
            a.repairs,
        );
        measured.push(Row {
            question: question.to_string(),
            rows: a.result.len(),
            repairs: a.repairs,
            ms,
        });
        // Show the head of the result so a human can sanity-check the recording.
        for row in a.result.rows.iter().take(3) {
            let cells: Vec<String> = row
                .iter()
                .map(|c| {
                    c.as_ref()
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "∅".into())
                })
                .collect();
            println!("    {}", cells.join(" | "));
        }
    }

    let out = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/olympics_replay.json"
    );
    std::fs::create_dir_all(std::path::Path::new(out).parent().unwrap()).unwrap();
    recorder.save(out).expect("write fixture");
    println!("wrote {} exchanges to {out}", recorder.exchanges().len());

    // (sq-k5qq) Strictly-additive JSON emit: only when `--json <path>` was given.
    // STDOUT above (the per-question lines + fixture write) is the unchanged human output.
    if let Some(path) = json_path {
        let doc = results_json(graph.len(), &measured);
        if let Err(e) = std::fs::write(&path, doc) {
            eprintln!("error writing --json results to {path}: {e}");
            std::process::exit(1);
        }
        eprintln!("wrote {} question rows to {path}", measured.len());
    }
}

// ---------------------------------------------------------------------------
// (sq-k5qq) --json emit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_flag_extraction() {
        let argv: Vec<String> = ["record_olympics", "--json", "/tmp/o.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (positional, path) = take_json_flag(argv);
        assert_eq!(positional, vec!["record_olympics"]);
        assert_eq!(path.as_deref(), Some("/tmp/o.json"));
        // Absent flag -> argv unchanged, no path.
        let plain: Vec<String> = ["record_olympics"].iter().map(|s| s.to_string()).collect();
        let (p2, none) = take_json_flag(plain.clone());
        assert_eq!(p2, plain);
        assert!(none.is_none());
    }

    #[test]
    fn json_str_escapes() {
        assert_eq!(json_str("rows"), "\"rows\"");
        assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    #[test]
    fn results_json_shape_and_keys() {
        let rows = vec![
            Row {
                question: "How many athletes are in the dataset?".into(),
                rows: 1,
                repairs: 0,
                ms: 1.5,
            },
            Row {
                question: "How many events does each sport have?".into(),
                rows: 12,
                repairs: 1,
                ms: 3.25,
            },
        ];
        let doc = results_json(123_456, &rows);
        // The dependency-free emit must round-trip through a REAL serde_json parse —
        // this catches a malformed document (e.g. a missing inter-row comma).
        let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");
        assert_eq!(v["harness"], "sparq-nlq record_olympics");
        assert_eq!(v["triples"], 123_456);
        assert!(v["note"].as_str().unwrap().contains("NON-CANONICAL"));
        let arr = v["questions"].as_array().expect("questions is an array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["question"], "How many athletes are in the dataset?");
        assert_eq!(arr[0]["rows"], 1);
        assert_eq!(arr[0]["repairs"], 0);
        assert!(arr[0]["ms"].is_number());
        assert_eq!(arr[1]["repairs"], 1);

        // An empty run must still be valid JSON (no trailing comma).
        let empty = results_json(0, &[]);
        let v2: serde_json::Value = serde_json::from_str(&empty).expect("empty run is valid JSON");
        assert!(v2["questions"].as_array().unwrap().is_empty());
    }
}
