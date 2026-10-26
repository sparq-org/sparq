//! Resource guards through the public parser. The baseline oracle is shared with
//! the existing corpus tests; explicit byte headroom isolates node-limit behavior.
use sparq_core::{meter::{Dim, Ledger, Limits}, Graph};
use sparq_policy::{constraint_budget_refusal, default_policy_limits, parse_policy_metered,
    parse_policy_str_metered, parse_policy_str_with_memberships_metered, ConstraintBudget};

const PREFIX: &str = "@prefix o: <http://www.w3.org/ns/odrl/2/> .\n";

fn shared_compound(rules: usize, width: usize) -> String {
    let mut rdf = format!("{PREFIX}<urn:p> a o:Set .\n");
    for r in 0..rules {
        rdf.push_str(&format!("<urn:p> o:permission <urn:r{r}> . <urn:r{r}> o:constraint <urn:root> .\n"));
    }
    for c in 0..width {
        rdf.push_str(&format!("<urn:root> o:and <urn:c{c}> . <urn:c{c}> o:leftOperand <urn:l> ; o:operator o:eq ; o:rightOperand <urn:v> .\n"));
    }
    rdf
}

#[test]
fn node_allowance_is_policy_wide_and_forecast_before_output() {
    let graph = Graph::load_str(&shared_compound(64, 1000), "turtle").unwrap();
    let mut limits = default_policy_limits();
    limits.bytes = usize::MAX;
    limits.nodes = 64 * 1024;
    let mut ledger = Ledger::new(limits);
    let p = parse_policy_metered(&graph, &mut ledger).unwrap();
    assert_eq!(p.permissions.len(), 64);
    assert!(p.permissions.iter().all(|r| r.logical_constraints[0].operands.len() == 1000));
    assert_eq!(ledger.spent(Dim::Nodes), 64_000);

    limits.nodes = 63_999;
    let mut refused = Ledger::new(limits);
    let err = parse_policy_metered(&graph, &mut refused).unwrap_err();
    assert_eq!(constraint_budget_refusal(&err), Some(ConstraintBudget::Nodes), "{err}");
    assert_eq!(refused.spent(Dim::Nodes), 0, "forecast must refuse before tree allocation");

    // Every individual compound fits the legacy allowance, but the policy exceeds
    // the default ceiling. No output-tree operand has been reserved at refusal.
    let rules = default_policy_limits().nodes / 1000 + 1;
    let graph = Graph::load_str(&shared_compound(rules, 1000), "turtle").unwrap();
    let mut limits = default_policy_limits();
    limits.bytes = usize::MAX;
    let mut refused = Ledger::new(limits);
    let err = parse_policy_metered(&graph, &mut refused).unwrap_err();
    assert_eq!(constraint_budget_refusal(&err), Some(ConstraintBudget::Nodes), "{err}");
    assert_eq!(refused.spent(Dim::Nodes), 0);
}

#[test]
fn multiplying_properties_stay_linear_and_fail_as_the_baseline() {
    for logical in [false, true] {
        let mut rdf = format!("{PREFIX}<urn:p> a o:Set ; o:permission <urn:r> .\n");
        if logical {
            rdf.push_str("<urn:r> o:constraint [ o:and <urn:c> ] .\n");
        } else {
            rdf.push_str("<urn:r> o:constraint <urn:c> .\n");
        }
        rdf.push_str("<urn:c> o:leftOperand o:purpose .\n");
        for n in 0..200 { rdf.push_str(&format!("<urn:c> o:rightOperand <urn:v{n}> .\n")); }
        for n in 0..50 {
            rdf.push_str(&format!("<urn:c> o:unit <urn:u{n}> ; o:operator <urn:op{n}> .\n"));
        }
        let graph = Graph::load_str(&rdf, "turtle").unwrap();
        let mut limits = default_policy_limits();
        limits.query_rows = 8 * graph.len();
        let mut ledger = Ledger::new(limits);
        let start = std::time::Instant::now();
        let err = parse_policy_metered(&graph, &mut ledger).unwrap_err();
        assert!(err.contains("several distinct odrl:operator"), "baseline's ambiguity refusal: {err}");
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        assert!(ledger.spent(Dim::QueryRows) <= 8 * graph.len());
    }
}

#[test]
fn rows_bytes_source_and_memberships_use_one_cumulative_ledger() {
    let rdf = format!("{PREFIX}<urn:p> a o:Set ; o:permission [ o:target <urn:collection> ; o:constraint [ o:leftOperand o:purpose ; o:operator o:eq ; o:rightOperand <urn:value> ] ] . <urn:member> o:partOf <urn:collection> .");
    let graph = Graph::load_str(&rdf, "turtle").unwrap();
    let mut limits = Limits::default();
    limits.query_rows = 0;
    let mut rows = Ledger::new(limits);
    let err = parse_policy_metered(&graph, &mut rows).unwrap_err();
    assert_eq!(constraint_budget_refusal(&err), Some(ConstraintBudget::Rows));
    assert_eq!(rows.spent(Dim::QueryRows), 0);

    let mut limits = Limits::default();
    limits.bytes = 0;
    let mut bytes = Ledger::new(limits);
    let err = parse_policy_metered(&graph, &mut bytes).unwrap_err();
    assert_eq!(constraint_budget_refusal(&err), Some(ConstraintBudget::Bytes));
    assert_eq!(bytes.spent(Dim::Bytes), 0);

    let mut ledger = Ledger::new(Limits::default());
    parse_policy_str_metered(&rdf, "turtle", &mut ledger).unwrap();
    let rows = ledger.spent(Dim::QueryRows);
    let bytes = ledger.spent(Dim::Bytes);
    let (_, memberships) = parse_policy_str_with_memberships_metered(&rdf, "turtle", &mut ledger).unwrap();
    assert_eq!(memberships.len(), 1);
    assert!(ledger.spent(Dim::QueryRows) > 2 * rows);
    assert_eq!(ledger.spent(Dim::Bytes), 2 * bytes);
    assert_eq!(ledger.spent(Dim::SourceBytes), 2 * rdf.len());
}
