//! End-to-end **exec-accuracy** harness for sparq-nlq (`research/genai-design.md` §4,
//! `sparq-nlq` row) — bead sq-05rv.
//!
//! Where `olympics_eval.rs` gates the engine-side plumbing (parse/execute/repair with
//! exact row counts), THIS file grades the design doc's headline accuracy claim the
//! QALD way: **answer-set F1** of the executed SPARQL, with the two axes the doc
//! requires reported separately —
//!
//!   * **oracle-linking vs end-to-end**, and
//!   * **grounded vs the same LLM ungrounded** ("the grounding must pay for itself").
//!
//! ## CI is fully offline — by construction
//!
//! Two layers, both network-free:
//!   1. `harness_demonstrates_grounding_pays_in_memory` — the **CI gate**. A tiny
//!      in-memory graph + scripted deterministic backends (a "grounded" model that
//!      answers from the schema deck, an "ungrounded" one that hallucinates a
//!      predicate). Always runs, no dataset, no network. This is what proves the
//!      harness itself works and what the `live` run must beat.
//!   2. `olympics_exec_accuracy_replay` — answer-F1 on the REAL 1.78M-triple olympics
//!      dataset using the committed `ReplayLlm` fixture. SKIPS (passes) if the dataset
//!      is absent (it is a downloaded benchmark fixture, not checked in).
//!
//! The LIVE measurement (a real model via `--features live` + `RecordingLlm`) is
//! `#[ignore]`'d AND `#[cfg(feature = "live")]` — it never runs in the default CI gate
//! and needs both the feature and an explicit `--ignored` to execute.

use sparq_core::Graph;
use sparq_nlq::eval::{run_comparison, run_config, EvalCase, Linking};
use sparq_nlq::{Llm, NlqConfig, RecordingLlm, ReplayLlm};

// ---------------------------------------------------------------------------
// Layer 1 — the offline CI gate: a tiny in-memory graph, scripted backends.
// ---------------------------------------------------------------------------

fn tiny_graph() -> Graph {
    // 3 people, 2 companies; people have ages, companies do not.
    let ttl = r#"
        @prefix ex: <http://example.org/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        ex:alice a ex:Person ; rdfs:label "Alice" ; ex:age 30 .
        ex:bob   a ex:Person ; rdfs:label "Bob"   ; ex:age 25 .
        ex:carol a ex:Person ; rdfs:label "Carol" ; ex:age 41 .
        ex:acme  a ex:Company ; rdfs:label "Acme" .
        ex:globex a ex:Company ; rdfs:label "Globex" .
    "#;
    Graph::load_str(ttl, "turtle").expect("tiny graph parses")
}

/// Closure-backed LLM.
struct FnLlm<F: Fn(&str) -> Result<String, String>>(F);
impl<F: Fn(&str) -> Result<String, String>> Llm for FnLlm<F> {
    fn complete(&self, prompt: &str) -> Result<String, String> {
        (self.0)(prompt)
    }
}

/// Gold queries for the tiny graph (the ground-truth answer sets).
fn cases() -> Vec<EvalCase> {
    vec![
        EvalCase::new(
            "List every person.",
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX ex: <http://example.org/>\n\
             SELECT ?p WHERE { ?p rdf:type ex:Person }",
        ),
        EvalCase::new(
            "List the label of every company.",
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\n\
             PREFIX ex: <http://example.org/>\n\
             SELECT ?l WHERE { ?c rdf:type ex:Company ; rdfs:label ?l }",
        ),
        EvalCase::new(
            "How many people are there?",
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX ex: <http://example.org/>\n\
             SELECT (COUNT(?p) AS ?n) WHERE { ?p rdf:type ex:Person }",
        ),
    ]
}

/// The "grounded" model: answers each question correctly (these are exactly the gold
/// queries — a competent model with the schema deck in front of it). Keyed by the
/// question line, which the loop always appends to the prompt.
fn grounded_completion(question: &str) -> Option<String> {
    cases()
        .into_iter()
        .find(|c| question.contains(&c.question))
        .map(|c| format!("```sparql\n{}\n```", c.gold_sparql))
}

/// The "ungrounded" model: WITHOUT the schema deck it does not know the `ex:` IRIs and
/// hallucinates plausible-but-wrong vocabulary. The queries parse and execute (so the
/// loop succeeds) but return the WRONG answer set — exactly the failure grounding is
/// supposed to fix. F1 against the (non-empty) gold is therefore 0.
fn ungrounded_completion(_question: &str) -> String {
    // Valid SPARQL over a guessed schema (schema.org-ish) that matches nothing here.
    "```sparql\nPREFIX schema: <http://schema.org/>\n\
     SELECT ?p WHERE { ?p a schema:Person }\n```"
        .to_string()
}

#[test]
fn harness_demonstrates_grounding_pays_in_memory() {
    let g = tiny_graph();
    let cases = cases();

    let comparison = run_comparison(&g, &cases, &NlqConfig::default(), |ground| {
        if ground {
            Box::new(FnLlm(|prompt| {
                grounded_completion(prompt)
                    .ok_or_else(|| format!("no scripted grounded answer for {prompt:?}"))
            }))
        } else {
            Box::new(FnLlm(|prompt| Ok(ungrounded_completion(prompt))))
        }
    });

    eprintln!("{}", comparison.summary());

    // The headline claim: grounded end-to-end strictly beats ungrounded end-to-end.
    assert!(
        comparison.headline_grounding_pays(),
        "grounding must pay for itself: grounded macroF1 {} vs ungrounded {}",
        comparison.grounded_end_to_end.macro_f1,
        comparison.ungrounded_end_to_end.macro_f1
    );
    // Grounded model answers all three correctly.
    assert_eq!(comparison.grounded_end_to_end.macro_f1, 1.0);
    assert_eq!(comparison.grounded_end_to_end.exact_match, 1.0);
    // Ungrounded model produces valid queries (validity 1.0) but the wrong answers.
    assert_eq!(comparison.ungrounded_end_to_end.validity, 1.0);
    assert_eq!(comparison.ungrounded_end_to_end.macro_f1, 0.0);
    // Oracle (linking given) is perfect under both grounding settings: this is the
    // engine-side loop, isolated from the model.
    assert_eq!(comparison.grounded_oracle.macro_f1, 1.0);
    assert_eq!(comparison.ungrounded_oracle.macro_f1, 1.0);
}

#[test]
fn empty_gold_is_a_true_negative() {
    // A question whose correct answer is "nothing" (no Robots in the graph). A model
    // that produces a valid, genuinely-empty query is CORRECT (F1 = 1) — not a miss.
    let g = tiny_graph();
    let cases = vec![EvalCase::new(
        "List every robot.",
        "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
         PREFIX ex: <http://example.org/>\n\
         SELECT ?r WHERE { ?r rdf:type ex:Robot }",
    )];
    let report = run_config(
        &g,
        &cases,
        Box::new(FnLlm(|_| {
            Ok(
                "```sparql\nPREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
                PREFIX ex: <http://example.org/>\n\
                SELECT ?r WHERE { ?r rdf:type ex:Robot }\n```"
                    .into(),
            )
        })),
        NlqConfig::default(),
        Linking::EndToEnd,
    );
    assert_eq!(report.macro_f1, 1.0, "empty==empty is a true negative");
    assert_eq!(report.exact_match, 1.0);
}

/// The **regression-set contract** the live run depends on (bead sq-g0lw, follow-up
/// to sq-05rv): a recorded session, once written to disk in the on-disk fixture format
/// `live_exec_accuracy` uses (`RecordingLlm::save` → `tests/fixtures/live_session_
/// {i}.json`), must reload via `ReplayLlm::from_file` and re-score **identically**
/// through the very same exec-accuracy harness (`run_config`). That round-trip is what
/// makes "commit the recorded session as the regression set" honest — without it, a
/// live run could write a fixture the offline gate cannot faithfully replay.
///
/// Proven OFFLINE. The live measurement itself needs `ANTHROPIC_API_KEY` + network +
/// the 1.78M-triple olympics dump (none of which CI has), and per project policy a
/// live run from a non-canonical box must NOT bake numbers into committed fixtures. So
/// the live API is substituted by the same deterministic grounded backend the CI gate
/// already uses; what is exercised is the *mechanism* (record → save-to-disk → reload
/// → identical harness scores), not a real model's accuracy.
#[test]
fn recorded_session_saves_and_replays_as_regression_set() {
    use std::rc::Rc;

    let g = tiny_graph();
    let cases = cases();
    let cfg = NlqConfig::default();

    // 1. Record a grounded session through the SAME `run_config` harness the live run
    //    drives — the recorder wraps the deterministic backend exactly as
    //    `RecordingLlm<AnthropicLlm>` wraps the live client. `Rc` keeps a handle to the
    //    recorder after `run_config` takes ownership of its `Box<dyn Llm>` (an `Rc<L>`
    //    is itself `Llm`), mirroring `live_exec_accuracy`'s recorder bookkeeping.
    let recorder = Rc::new(RecordingLlm::new(FnLlm(|prompt: &str| {
        grounded_completion(prompt)
            .ok_or_else(|| format!("no scripted grounded answer for {prompt:?}"))
    })));
    let recorded_report = run_config(
        &g,
        &cases,
        Box::new(Rc::clone(&recorder)),
        cfg.clone(),
        Linking::EndToEnd,
    );
    assert_eq!(
        recorded_report.macro_f1, 1.0,
        "the recorded grounded session answers all tiny-graph cases"
    );
    assert!(
        !recorder.exchanges().is_empty(),
        "the session recorded at least one (prompt, completion) pair"
    );

    // 2. Persist it exactly as `live_exec_accuracy` does — to disk, in the committed
    //    `Exchange[]` JSON format — then reload through the public file API.
    let path = std::env::temp_dir().join(format!(
        "sparq_nlq_live_session_{}_{}.json",
        std::process::id(),
        // distinguish concurrent test binaries sharing the temp dir.
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    recorder.save(&path).expect("save the recorded session");

    // The on-disk shape IS the committed fixture format (a JSON array of
    // {prompt, completion}) — same contract as `olympics_replay.json`.
    let on_disk = std::fs::read_to_string(&path).expect("read back the saved session");
    let parsed: serde_json::Value =
        serde_json::from_str(&on_disk).expect("saved session is valid JSON");
    assert!(parsed.is_array(), "fixture is a JSON array of exchanges");
    assert!(
        parsed[0].get("prompt").is_some() && parsed[0].get("completion").is_some(),
        "each exchange carries a prompt and a completion"
    );

    let reloaded = ReplayLlm::from_file(&path).expect("saved session reloads as a fixture");
    assert_eq!(
        reloaded.len(),
        recorder.exchanges().len(),
        "every recorded exchange survives the disk round-trip"
    );

    // 3. Re-score the SAME cases through the SAME `run_config` harness, now driven by
    //    the reloaded fixture. The regression set is faithful iff the scores match.
    let replay_report = run_config(
        &g,
        &cases,
        Box::new(reloaded),
        cfg.clone(),
        Linking::EndToEnd,
    );
    assert_eq!(
        replay_report.macro_f1, recorded_report.macro_f1,
        "replayed regression set must reproduce the recorded macro-F1"
    );
    assert_eq!(
        replay_report.exact_match, recorded_report.exact_match,
        "replayed regression set must reproduce the recorded exact-match"
    );
    assert_eq!(
        replay_report.validity, recorded_report.validity,
        "replayed regression set must reproduce the recorded validity"
    );

    let _ = std::fs::remove_file(&path); // hygiene: leave no scratch fixture behind.
}

// ---------------------------------------------------------------------------
// Layer 2 — answer-F1 on the REAL olympics dataset, offline replay. Skip-if-absent.
// ---------------------------------------------------------------------------

fn olympics_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("SPARQ_OLYMPICS_NT") {
        return Some(p.into());
    }
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../bench/qlever-olympics/olympics.nt");
    p.exists().then_some(p)
}

const OLYMPICS_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/olympics_replay.json"
);

/// (question, gold SPARQL) for the olympics set. The gold queries are the SAME ones
/// the committed grounded fixture records as completions — so the grounded fixture
/// replay scores answer-F1 1.0 here, while the gold is recomputed live against the
/// dataset (`research/genai-nl-to-sparql.md` §4.3: no checked-in answer blob).
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
    ]
}

#[test]
fn olympics_exec_accuracy_replay() {
    let Some(path) = olympics_path() else {
        eprintln!("skipping: olympics.nt not present (downloaded benchmark fixture)");
        return;
    };
    let text = std::fs::read_to_string(&path).expect("read olympics.nt");
    let graph = Graph::load_str(&text, "ntriples").expect("parse olympics.nt");
    drop(text);
    assert!(
        graph.len() > 1_700_000,
        "expected the 1.78M-triple olympics dataset"
    );

    let cases = olympics_cases();

    // Grounded end-to-end: replay the committed fixture (the recorded completions are
    // exactly the gold queries), so answer-F1 must be 1.0 on these three.
    let replay = sparq_nlq::ReplayLlm::from_file(OLYMPICS_FIXTURE)
        .expect("committed olympics replay fixture loads");
    let report = run_config(
        &graph,
        &cases,
        Box::new(replay),
        NlqConfig::default(),
        Linking::EndToEnd,
    );
    eprintln!(
        "olympics grounded end-to-end: macroF1={:.3} EM={:.3} validity={:.3}",
        report.macro_f1, report.exact_match, report.validity
    );
    assert_eq!(
        report.macro_f1, 1.0,
        "grounded fixture replay should answer all olympics cases exactly"
    );
    assert_eq!(report.exact_match, 1.0);

    // Oracle linking on the real dataset: the engine-side validate→execute path over
    // 1.78M triples, isolated from the model. Also perfect.
    let oracle = run_config(
        &graph,
        &cases,
        Box::new(NoLlm),
        NlqConfig::default(),
        Linking::Oracle,
    );
    assert_eq!(oracle.macro_f1, 1.0, "oracle linking on olympics");
}

/// Never consulted on the oracle path.
struct NoLlm;
impl Llm for NoLlm {
    fn complete(&self, _prompt: &str) -> Result<String, String> {
        Err("oracle run must not consult the LLM".into())
    }
}

// ---------------------------------------------------------------------------
// Layer 3 — the LIVE measurement. NEVER runs in CI: feature-gated AND #[ignore]'d.
// ---------------------------------------------------------------------------
//
// Run it explicitly (network + API key + the dataset):
//   ANTHROPIC_API_KEY=sk-ant-... SPARQ_OLYMPICS_NT=/path/olympics.nt \
//     cargo test -p sparq-nlq --features live --release -- --ignored live_exec_accuracy
//
// It drives the SAME harness against a real model, records the exchanges (so the run
// is replayable afterwards), and prints the four-cell comparison. It asserts the
// design-doc inequality (grounding pays) — the honest, measured headline claim, made
// OUT of the CI gate so a network/model outage never reds the build.
#[cfg(feature = "live")]
#[test]
#[ignore = "live: needs ANTHROPIC_API_KEY + network + the olympics dataset"]
fn live_exec_accuracy() {
    use sparq_nlq::live::AnthropicLlm;
    use sparq_nlq::RecordingLlm;
    use std::rc::Rc;

    let Some(path) = olympics_path() else {
        panic!("live run needs the olympics dataset (set SPARQ_OLYMPICS_NT)");
    };
    let text = std::fs::read_to_string(&path).expect("read olympics.nt");
    let graph = Graph::load_str(&text, "ntriples").expect("parse olympics.nt");
    drop(text);

    let cases = olympics_cases();
    let cfg = NlqConfig {
        max_repair_rounds: 3, // design-doc cap for live runs
        ..NlqConfig::default()
    };

    // Record while we go, so the live run yields a replay fixture for later.
    let recorders: Rc<std::cell::RefCell<Vec<Rc<RecordingLlm<AnthropicLlm>>>>> =
        Rc::new(std::cell::RefCell::new(Vec::new()));
    let recorders_for_closure = Rc::clone(&recorders);
    let comparison = run_comparison(&graph, &cases, &cfg, move |_ground| {
        let rec = Rc::new(RecordingLlm::new(
            AnthropicLlm::from_env().expect("ANTHROPIC_API_KEY set"),
        ));
        recorders_for_closure.borrow_mut().push(Rc::clone(&rec));
        Box::new(rec)
    });

    eprintln!("{}", comparison.summary());
    // Persist every recorded session next to the fixture for replay/regression.
    for (i, rec) in recorders.borrow().iter().enumerate() {
        let out = format!(
            "{}/tests/fixtures/live_session_{i}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        rec.save(&out).expect("save live session");
        eprintln!("wrote {out}");
    }

    assert!(
        comparison.headline_grounding_pays(),
        "LIVE: grounding must pay for itself — grounded {} vs ungrounded {}",
        comparison.grounded_end_to_end.macro_f1,
        comparison.ungrounded_end_to_end.macro_f1
    );
}
