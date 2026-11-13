//! SPARQL-RL syntax and census. Unsupported stages fail, including negative tests.
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
    #[cfg(feature = "sparql-rl")]
    score.finish(342, 212, 130, 0);
    #[cfg(not(feature = "sparql-rl"))]
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
    #[cfg(feature = "sparql-rl")]
    if matches!(kind, "RulesPositiveSyntaxTest" | "RulesNegativeSyntaxTest") {
        let file = path(&action)?;
        let source = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        let base = format!("file://{}", file.display());
        return syntax_outcome(&source, &base, kind == "RulesPositiveSyntaxTest");
    }
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

#[cfg(feature = "sparql-rl")]
fn syntax_outcome(source: &str, base: &str, positive: bool) -> Result<Outcome, String> {
    use sparq_shacl::sparql_rl::{parse, ErrorKind};
    match (positive, parse(source, base)) {
        (true, Ok(_)) => Ok(Outcome::Pass),
        (false, Err(e)) if e.kind == ErrorKind::Syntax => Ok(Outcome::Pass),
        (true, Err(e)) | (false, Err(e)) => Err(e.to_string()),
        (false, Ok(_)) => Err("Accepted negative syntax test".into()),
    }
}

#[test]
fn unsupported_negative_tests_are_not_successful_rejections() {
    for kind in [
        #[cfg(not(feature = "sparql-rl"))]
        "RulesPositiveSyntaxTest",
        #[cfg(not(feature = "sparql-rl"))]
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

#[cfg(feature = "sparql-rl")]
#[test]
fn syntax_runner_checks_both_polarities_and_does_not_count_limits_as_rejections() {
    let valid = "RULE {} WHERE {}";
    let invalid = "RULE {} WHERE";
    let base = "http://example.org/";
    assert!(matches!(
        syntax_outcome(valid, base, true),
        Ok(Outcome::Pass)
    ));
    assert!(matches!(
        syntax_outcome(invalid, base, false),
        Ok(Outcome::Pass)
    ));
    assert!(syntax_outcome(valid, base, false).is_err());
    assert!(syntax_outcome(invalid, base, true).is_err());
    let deep = format!(
        "RULE {{}} WHERE {{ FILTER({}1{}) }}",
        "(".repeat(100),
        ")".repeat(100)
    );
    assert!(syntax_outcome(&deep, base, false).is_err());
}
