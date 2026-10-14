//! Metamorphic laws for the deterministic, model-free lexical entity linker.
//!
//! sq-588cl

use std::collections::BTreeSet;

use sparq_core::Graph;
use sparq_nlq::link::EntityLinker;

const ALICE: &str = "http://example.org/alice";
const BOB: &str = "http://example.org/bob";
const ACME: &str = "http://example.org/acme";

fn fixture_graph() -> Graph {
    let ntriples = r#"
<http://example.org/alice> <http://www.w3.org/2000/01/rdf-schema#label> "Alice Smith" .
<http://example.org/bob> <http://www.w3.org/2000/01/rdf-schema#label> "Bob Jones" .
<http://example.org/acme> <http://schema.org/name> "Acme Company" .
<http://example.org/alice> <http://example.org/worksFor> <http://example.org/acme> .
<http://example.org/bob> <http://example.org/worksFor> <http://example.org/acme> .
"#;
    Graph::load_str(ntriples, "ntriples").expect("fixture N-Triples parses")
}

fn generated_questions() -> Vec<String> {
    let words = [
        "alice", "smith", "bob", "jones", "acme", "company", "works", "for", "unknown", "galaxy",
    ];
    let mut questions = vec![String::new()];
    questions.extend(words.iter().map(|word| (*word).to_owned()));
    for left in words {
        for right in words {
            questions.push(format!("{left} {right}"));
        }
    }
    questions
}

#[test]
fn repeated_links_are_identical_and_fixture_grounded() {
    let graph = fixture_graph();
    let linker = EntityLinker::build(&graph, 2, 3);
    let fixture_entities = BTreeSet::from([ALICE, BOB, ACME]);
    let fixture_labels = BTreeSet::from(["Alice Smith", "Bob Jones", "Acme Company"]);

    for question in generated_questions() {
        let first = linker.link(&question);
        assert_eq!(first, linker.link(&question), "question: {question:?}");
        assert_eq!(first, linker.link(&question), "question: {question:?}");

        for entity in &first.entities {
            assert!(
                fixture_entities.contains(entity.iri.as_str()),
                "linked IRI was not in fixture for {question:?}: {}",
                entity.iri
            );
            assert!(
                fixture_labels.contains(entity.label.as_str()),
                "linked label was not in fixture for {question:?}: {}",
                entity.label
            );
        }
    }
}

#[test]
fn max_links_caps_entities_for_generated_questions() {
    let graph = fixture_graph();
    for max_links in 0..=3 {
        let linker = EntityLinker::build(&graph, 0, max_links);
        for question in generated_questions() {
            let linked = linker.link(&question);
            assert!(
                linked.entities.len() <= max_links,
                "{} entities exceeded cap {max_links} for {question:?}",
                linked.entities.len()
            );
        }
    }
}

#[test]
fn empty_input_and_empty_graph_fail_closed() {
    let graph = fixture_graph();
    assert!(EntityLinker::build(&graph, 2, 3).link("").is_empty());

    let empty = Graph::load_str("", "ntriples").expect("empty N-Triples parses");
    let linker = EntityLinker::build(&empty, 2, 3);
    for question in generated_questions() {
        assert!(
            linker.link(&question).is_empty(),
            "empty graph fabricated a link for {question:?}"
        );
    }
}
