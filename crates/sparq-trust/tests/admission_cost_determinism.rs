//! Mutation witness for deterministic admission-cost evidence (`sq-r78pf`).

#![cfg(feature = "cert-graph")]

#[allow(dead_code)]
#[path = "../examples/admission_cost.rs"]
mod admission_cost;

#[test]
fn identical_fixture_runs_are_byte_identical_and_have_no_environment_metrics() {
    let first = admission_cost::render_fixture_suite();
    let second = admission_cost::render_fixture_suite();

    assert_eq!(first.as_bytes(), second.as_bytes());
    assert!(first.contains("\"certification_edges_considered\""));
    assert!(first.contains("\"derived_rule_count\""));
    for forbidden in ["timing", "duration", "elapsed", "hostname", "platform"] {
        assert!(
            !first.contains(forbidden),
            "forbidden metric key: {forbidden}"
        );
    }
}
