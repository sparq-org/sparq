// sq-bif.22: direct coverage for public introspection read-side failures
// and budgeted schema retrieval, without enabling optional crate features.

use sparq_core::Graph;
use sparq_introspect::{Introspection, characteristic_set_ids};

const FIXTURE: &str = r#"<http://ex.org/alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Person> .
<http://ex.org/alice> <http://xmlns.com/foaf/0.1/name> "Alice" .
"#;

fn fixture_graph() -> Graph {
    Graph::load_str(FIXTURE, "ntriples").expect("parse introspection fixture")
}

#[test]
fn from_json_rejects_syntax_and_numeric_type_errors() {
    let syntax_error = Introspection::from_json(r#"{"triples":"unterminated"#)
        .expect_err("an unterminated JSON string must be rejected");
    assert!(!syntax_error.to_string().is_empty());

    let introspection = Introspection::build(&fixture_graph());
    let mut wrong_type: serde_json::Value =
        serde_json::from_str(&introspection.to_json()).expect("parse generated JSON");
    wrong_type["triples"] = serde_json::Value::String("not-a-number".to_owned());
    let wrong_type = serde_json::to_string(&wrong_type).expect("serialize wrong-type JSON");
    let type_error = Introspection::from_json(&wrong_type)
        .expect_err("a string-valued numeric field must be rejected");
    assert!(!type_error.to_string().is_empty());
}

#[test]
fn load_maps_valid_json_with_the_wrong_schema_to_invalid_data() {
    let path = std::env::temp_dir().join(format!(
        "sparq-introspect-wrong-schema-{}.introspect",
        std::process::id()
    ));
    let wrong_schema = r#"{"not":"an introspection"}"#;
    serde_json::from_str::<serde_json::Value>(wrong_schema).expect("fixture is valid JSON");
    std::fs::write(&path, wrong_schema).expect("write wrong-schema sidecar");

    let error = Introspection::load(&path).expect_err("the wrong sidecar schema must fail");
    std::fs::remove_file(&path).expect("remove wrong-schema sidecar");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn schema_summary_for_scopes_to_a_seed_and_honors_tiny_budgets() {
    let introspection = Introspection::build(&fixture_graph());
    let person = "http://xmlns.com/foaf/0.1/Person";
    let summary = introspection.schema_summary_for(&[person], 1_000);
    assert!(
        summary.contains("foaf:Person"),
        "summary must mention the requested class seed:\n{summary}"
    );

    let tiny_budget = 8;
    let tiny = introspection.schema_summary_for(&[], tiny_budget);
    // The schema_summary_for rustdoc promises that the rendered slice is under
    // budget_chars. Mutation witness: lowering the expected bound below the emitted
    // elision marker's character count makes this assertion fail.
    assert!(
        tiny.chars().count() <= tiny_budget,
        "empty-seed summary exceeded {tiny_budget} characters: {tiny:?}"
    );
}

#[test]
fn empty_graph_has_no_characteristic_set_ids() {
    let graph = Graph::load_str("", "ntriples").expect("parse empty graph");
    assert!(characteristic_set_ids(&graph).is_empty());
}
