//! SPARQL-RL census. Unsupported language stages are failures, including negative tests.
#[path = "support/rules_suite.rs"]
mod suite;
mod support;
use oxrdf::Term;
use sparq_core::Graph;
use sparq_shacl::view::GraphView;
use suite::*;
use support::Outcome;

#[test]
fn w3c_sparql_rl_shacl12() {
    let root = root("sparql-rl");
    if !root.exists() {
        eprintln!("SKIP: suite not fetched: {}", root.display());
        return;
    }
    let mut score = Score::default();
    for group in [
        "eval",
        "eval2",
        "examples",
        "stratification",
        "syntax",
        "wellformed",
    ] {
        score.walk(&root.join(group).join("manifest.ttl"), group, run);
    }
    score.finish(342, 0, 342, 0);
}
fn run(_graph: &Graph, view: &GraphView, entry: &Term) -> Result<Outcome, String> {
    let types = view.objects(entry, "http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let kind = [
        "RulesEvalTest",
        "RulesPositiveSyntaxTest",
        "RulesNegativeSyntaxTest",
        "RulesPositiveWellFormednessTest",
        "RulesNegativeWellFormednessTest",
        "RulesPositiveStratificationTest",
        "RulesNegativeStratificationTest",
    ]
    .into_iter()
    .find(|kind| types.contains(&iri(&format!("{SRLT}{kind}"))))
    .ok_or("Unknown SPARQL-RL test type")?;
    let action = object(view, entry, MF, "action")?;
    if kind == "RulesEvalTest" {
        std::fs::read_to_string(path(&object(view, &action, SRLT, "ruleset")?)?)
            .map_err(|e| e.to_string())?;
        let _ = load(&path(&object(view, &action, SRLT, "data")?)?)?;
        let _ = expected_graph(view, &object(view, entry, MF, "result")?)?;
    } else {
        std::fs::read_to_string(path(&action)?).map_err(|e| e.to_string())?;
    }
    Err(format!("SPARQL-RL {kind}: language processor unavailable"))
}

#[test]
fn unsupported_negative_tests_are_not_successful_rejections() {
    for kind in [
        "RulesPositiveSyntaxTest",
        "RulesNegativeSyntaxTest",
        "RulesPositiveWellFormednessTest",
        "RulesNegativeWellFormednessTest",
        "RulesPositiveStratificationTest",
        "RulesNegativeStratificationTest",
    ] {
        let text = format!(
            "<urn:test> a <{SRLT}{kind}> ; <{MF}action> <file://{}/Cargo.toml> .",
            env!("CARGO_MANIFEST_DIR")
        );
        let graph = Graph::load_str(&text, "turtle").unwrap();
        assert!(run(&graph, &GraphView::new(&graph), &iri("urn:test")).is_err());
    }
}
