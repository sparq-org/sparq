// [GPT-6] String-producing authorization rewrites preserve the query dialect.
use sparq_core::Graph;
use sparq_engine::{EbvSemantics, PreparedQuery, QueryBudget};

#[test]
fn every_authorization_rewrite_retains_version_contract() {
    let graph = Graph::load_str("", "ntriples").unwrap();
    for label in ["1.1", "1.2", "1.2-basic"] {
        let query = format!("VERSION '{label}' ASK {{ FILTER(!\"z\"^^<http://www.w3.org/2001/XMLSchema#boolean>) }}");
        for text in [
            sparq_solid::rewrite_for(&query, &[]).unwrap(),
            sparq_solid::wrap_for_view(&query).unwrap(),
            sparq_solid::wrap_for_view_opt_in(&query).unwrap(),
        ] {
            let prepared = PreparedQuery::parse(&text).unwrap();
            assert_eq!(prepared.versions(), [label]);
            assert_eq!(sparq_engine::ask_prepared(&graph, &prepared).unwrap(), label == "1.1");
            let budget = QueryBudget { ebv_semantics: Some(EbvSemantics::Rec2013), ..Default::default() };
            assert_eq!(sparq_engine::ask_prepared_with_budget(&graph, &prepared, &budget).is_ok(), label == "1.1");
        }
    }
    for query in ["VERSION 'unknown' ASK {}", "VERSION '1.1' VERSION '1.2' ASK {}"] {
        assert!(sparq_solid::rewrite_for(query, &[]).is_err());
        assert!(sparq_solid::wrap_for_view(query).is_err());
        assert!(sparq_solid::wrap_for_view_opt_in(query).is_err());
    }
    let query = "VERSION '1.2' VERSION '1.2-basic' VERSION '1.2' ASK {}";
    for text in [sparq_solid::rewrite_for(query, &[]).unwrap(), sparq_solid::wrap_for_view(query).unwrap(), sparq_solid::wrap_for_view_opt_in(query).unwrap()] {
        assert_eq!(PreparedQuery::parse(&text).unwrap().versions(), ["1.2", "1.2-basic", "1.2"]);
    }
}
