// sq-bif.23: direct coverage for provenance configuration and accessors.

use std::time::{Duration, SystemTime};

use oxrdf::NamedNode;
use sparq_core::Graph;
use sparq_prov::{derive_construct, ProvConfig};

const CONSTRUCT: &str =
    "CONSTRUCT { ?s <http://example.test/derived> ?o } WHERE { ?s <http://example.test/source> ?o }";
const RECENT_BOUND: Duration = Duration::from_secs(60);

fn tiny_graph() -> Graph {
    Graph::load_str(
        "<http://example.test/subject> <http://example.test/source> <http://example.test/object> .",
        "ntriples",
    )
    .expect("the test fixture must be valid N-Triples")
}

fn distance(left: SystemTime, right: SystemTime) -> Duration {
    left.duration_since(right).unwrap_or_else(|_| {
        right
            .duration_since(left)
            .expect("system times must be ordered")
    })
}

#[test]
fn with_inputs_preserves_configured_order() {
    let inputs = [
        NamedNode::new_unchecked("http://example.test/input/first"),
        NamedNode::new_unchecked("http://example.test/input/second"),
    ];
    let derivation = derive_construct(
        &tiny_graph(),
        CONSTRUCT,
        ProvConfig::with_inputs(inputs.clone()),
    )
    .expect("the CONSTRUCT query must derive a graph");

    assert_eq!(derivation.used_inputs(), inputs.as_slice());
}

#[test]
fn default_config_records_no_used_inputs() {
    let derivation = derive_construct(&tiny_graph(), CONSTRUCT, ProvConfig::default())
        .expect("the CONSTRUCT query must derive a graph");

    assert!(derivation.used_inputs().is_empty());
}

#[test]
fn select_query_returns_a_nonempty_graph_form_error() {
    let error = derive_construct(
        &tiny_graph(),
        "SELECT ?s WHERE { ?s ?p ?o }",
        ProvConfig::default(),
    )
    .expect_err("SELECT is not a graph-valued query");

    assert!(!error.trim().is_empty());
    assert!(
        error.contains("CONSTRUCT") || error.contains("DESCRIBE"),
        "the error should identify the accepted graph query forms: {error}"
    );
}

#[test]
fn successful_derivation_reports_recent_ordered_timing() {
    let derivation = derive_construct(&tiny_graph(), CONSTRUCT, ProvConfig::default())
        .expect("the CONSTRUCT query must derive a graph");
    let now = SystemTime::now();
    let (started, ended) = derivation.timing();

    assert!(ended.duration_since(started).is_ok());
    assert!(distance(started, now) <= RECENT_BOUND);
    assert!(distance(ended, now) <= RECENT_BOUND);
}
