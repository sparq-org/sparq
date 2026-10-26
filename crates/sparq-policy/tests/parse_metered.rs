use sparq_core::{
    meter::{Dim, Ledger, Limits},
    Graph,
};
use sparq_policy::{
    constraint_budget_refusal, default_policy_limits, parse_policy_metered, parse_policy_str,
    parse_policy_str_metered, parse_policy_str_with_memberships_metered, ConstraintBudget,
};

const PREFIX: &str = "@prefix o: <http://www.w3.org/ns/odrl/2/> .\n";
fn ledger(nodes: usize) -> Ledger {
    let mut l = Limits::default();
    l.nodes = nodes;
    Ledger::new(l)
}

#[test]
fn multiplying_fields_use_linear_query_rows() {
    let mut rdf = format!("{PREFIX}<urn:p> o:permission <urn:r> . <urn:r> o:constraint <urn:c> . <urn:c> o:leftOperand o:purpose .\n");
    for i in 0..200 {
        rdf.push_str(&format!("<urn:c> o:rightOperand <urn:v{i}> .\n"));
    }
    for i in 0..50 {
        rdf.push_str(&format!(
            "<urn:c> o:unit <urn:u{i}> ; o:operator <urn:op{i}> .\n"
        ));
    }
    let nested = rdf.replace("o:constraint <urn:c>", "o:constraint <urn:root>")
        + "<urn:root> o:and <urn:c> .";
    for rdf in [&rdf, &nested] {
        let graph = Graph::load_str(rdf, "turtle").unwrap();
        // Leave the caller's row allowance open so a restored product query
        // fails the spent-row assertion rather than being hidden by a refusal.
        let mut l = Ledger::new(Limits::default());
        let start = std::time::Instant::now();
        let err = parse_policy_metered(&graph, &mut l).unwrap_err();
        assert!(err.contains("several distinct odrl:operator"), "{err}");
        assert_eq!(constraint_budget_refusal(&err), None);
        assert!(l.spent(Dim::QueryRows) <= 8 * graph.len());
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
    }
}

fn repeated_compounds(rules: usize, width: usize) -> String {
    let mut rdf = format!("{PREFIX}<urn:p> a o:Set .\n");
    for i in 0..rules {
        rdf.push_str(&format!(
            "<urn:p> o:permission <urn:r{i}> . <urn:r{i}> o:constraint <urn:lc> .\n"
        ));
    }
    // Unknown atomic nodes retain main's unsatisfiable permission guards.
    for i in 0..width {
        rdf.push_str(&format!("<urn:lc> o:and <urn:a{i}> .\n"));
    }
    rdf
}

#[test]
fn policy_wide_nodes_accept_legacy_compounds_and_refuse_total_before_assembly() {
    let rdf = repeated_compounds(64, 1000);
    let mut l = ledger(64 * 1024);
    let p = parse_policy_str_metered(&rdf, "turtle", &mut l).unwrap();
    assert_eq!(p.permissions.len(), 64);
    assert_eq!(l.spent(Dim::Nodes), 64_000);
    assert!(parse_policy_str(&rdf, "turtle").is_ok());
    let ceiling = default_policy_limits().nodes;
    let rdf = repeated_compounds(ceiling / 1000 + 1, 1000);
    let mut l = ledger(ceiling);
    let err = parse_policy_str_metered(&rdf, "turtle", &mut l).unwrap_err();
    assert_eq!(
        constraint_budget_refusal(&err),
        Some(ConstraintBudget::Nodes)
    );
    assert_eq!(
        l.spent(Dim::Nodes),
        0,
        "refuse the total before constructing any expanded operand vector"
    );
}

#[test]
fn rows_bytes_and_depth_are_typed_and_share_one_ledger() {
    let rdf = format!("{PREFIX}<urn:p> o:permission [ o:target <urn:collection>; o:constraint [ o:leftOperand o:purpose; o:operator o:eq; o:rightOperand <urn:v> ] ] . <urn:m> o:partOf <urn:collection> .");
    let mut limits = Limits::default();
    limits.query_rows = 0;
    let err = parse_policy_str_metered(&rdf, "turtle", &mut Ledger::new(limits)).unwrap_err();
    assert_eq!(
        constraint_budget_refusal(&err),
        Some(ConstraintBudget::Rows)
    );
    limits = Limits::default();
    limits.bytes = 0;
    let mut l = Ledger::new(limits);
    let err = parse_policy_str_metered(&rdf, "turtle", &mut l).unwrap_err();
    assert_eq!(
        constraint_budget_refusal(&err),
        Some(ConstraintBudget::Bytes)
    );
    assert_eq!(l.spent(Dim::Bytes), 0);
    let mut l = ledger(1024);
    let (_, members) = parse_policy_str_with_memberships_metered(&rdf, "turtle", &mut l).unwrap();
    assert!(members.contains(&("urn:m".into(), "urn:collection".into())));
    let rows = l.spent(Dim::QueryRows);
    limits = Limits::default();
    limits.query_rows = rows - 1;
    let err = parse_policy_str_with_memberships_metered(&rdf, "turtle", &mut Ledger::new(limits))
        .unwrap_err();
    assert_eq!(
        constraint_budget_refusal(&err),
        Some(ConstraintBudget::Rows)
    );
}

#[test]
fn permission_and_prohibition_totals_are_checked_together() {
    let rdf = repeated_compounds(2, 600).replace(
        "<urn:p> o:permission <urn:r1>",
        "<urn:p> o:prohibition <urn:r1>",
    );
    let mut l = ledger(1000);
    let err = parse_policy_str_metered(&rdf, "turtle", &mut l).unwrap_err();
    assert_eq!(
        constraint_budget_refusal(&err),
        Some(ConstraintBudget::Nodes)
    );
    assert_eq!(l.spent(Dim::Nodes), 0);
}
