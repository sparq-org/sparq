#![cfg(feature = "engine")]

// [GPT-6] Structural integration rewrites preserve the announced EBV rule.
#[test]
fn rewrite_preserves_version_ebv_and_conflicts() {
    let graph = sparq_core::Graph::load_str("", "ntriples").unwrap();
    let index = sparq_text::TextIndex::build(&graph);
    for (label, expected) in [("1.1", true), ("1.2", false)] {
        let query = format!("VERSION '{label}' ASK {{FILTER(!\"z\"^^<http://www.w3.org/2001/XMLSchema#boolean>)}}");
        let prepared = sparq_text::prepare_text(&graph, &query, &index).unwrap();
        assert_eq!(prepared.versions(), [label]);
        assert_eq!(sparq_engine::ask_prepared(&graph, &prepared).unwrap(), expected);
        if label == "1.2" {
            let budget = sparq_engine::QueryBudget { ebv_semantics: Some(sparq_engine::EbvSemantics::Rec2013), ..Default::default() };
            assert!(sparq_engine::ask_prepared_with_budget(&graph, &prepared, &budget).is_err());
        }
    }
    for query in ["VERSION 'bogus' ASK {}", "VERSION '1.1' VERSION '1.2' ASK {}"] {
        assert!(sparq_text::prepare_text(&graph, &query, &index).is_err());
    }
}
