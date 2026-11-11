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

fn shared_compound(rules: usize) -> String {
    let mut rdf = format!("{PREFIX}<urn:p> a o:Set . <urn:atom> o:leftOperand o:purpose; o:operator o:isAnyOf; o:rightOperand <urn:list> . <urn:list> <http://www.w3.org/1999/02/22-rdf-syntax-ns#first> <urn:value0>; <http://www.w3.org/1999/02/22-rdf-syntax-ns#rest> (");
    for i in 1..2000 {
        rdf.push_str(&format!(" <urn:value{i}>"));
    }
    rdf.push_str(") .\n");
    for i in 0..500 {
        rdf.push_str(&format!(
            "<urn:root> o:and <urn:sub{i}> . <urn:sub{i}> o:and <urn:atom> .\n"
        ));
    }
    for i in 0..rules {
        rdf.push_str(&format!(
            "<urn:p> o:permission <urn:r{i}> . <urn:r{i}> o:constraint <urn:root> .\n"
        ));
    }
    rdf
}

#[test]
fn shared_atom_expansions_retain_one_allocation_and_linear_charges() {
    use sparq_policy::{default_policy_limits_for, ConstraintNode};
    let mut bytes = None;
    for rules in [1, 200, 2000] {
        let rdf = shared_compound(rules);
        let graph = Graph::load_str(&rdf, "turtle").unwrap();
        let limits = default_policy_limits_for(&graph, Some(rdf.len()));
        assert_eq!(limits.bytes, default_policy_limits_for(&graph, None).bytes);
        let mut l = Ledger::new(limits);
        let result = parse_policy_metered(&graph, &mut l);
        if rules == 2000 {
            let err = result.unwrap_err();
            assert_eq!(
                constraint_budget_refusal(&err),
                Some(ConstraintBudget::Nodes)
            );
            assert_eq!(
                l.spent(Dim::Nodes),
                0,
                "total refused before expansion or list resolution"
            );
            continue;
        }
        let policy = result.unwrap();
        assert_eq!(policy.permissions.len(), rules);
        assert_eq!(l.spent(Dim::Nodes), rules * 1000 + 2000);
        assert!(l.spent(Dim::Bytes) <= l.limits().bytes);
        if let Some(expected) = bytes {
            assert_eq!(l.spent(Dim::Bytes), expected);
        } else {
            bytes = Some(l.spent(Dim::Bytes));
        }
        let mut first = None;
        for rule in &policy.permissions {
            for sub in &rule.logical_constraints[0].operands {
                let ConstraintNode::Compound(sub) = sub else {
                    panic!("expected compound")
                };
                let ConstraintNode::SharedAtomic(atom) = &sub.operands[0] else {
                    panic!("expected shared leaf")
                };
                if let Some(first) = first {
                    assert!(
                        std::sync::Arc::ptr_eq(first, atom),
                        "expansions must retain the atom allocation"
                    );
                } else {
                    first = Some(atom);
                }
            }
        }
        assert!(parse_policy_str(&rdf, "turtle").is_ok());
    }
}

#[test]
fn shared_list_references_retain_one_folded_allocation() {
    use sparq_policy::{default_policy_limits_for, Value};
    let mut rdf = format!("{PREFIX}<urn:p> o:permission <urn:r> . <urn:holder> o:rightOperand (");
    for i in 1..4000 {
        rdf.push_str(&format!(" <urn:v{i}>"));
    }
    rdf.push_str(") .\n");
    // A named head lets every attached constraint use the same collection.
    rdf = rdf.replace("<urn:holder> o:rightOperand (", "<urn:list> <http://www.w3.org/1999/02/22-rdf-syntax-ns#first> <urn:v0>; <http://www.w3.org/1999/02/22-rdf-syntax-ns#rest> (");
    for i in 0..4000 {
        rdf.push_str(&format!("<urn:r> o:constraint <urn:c{i}> . <urn:c{i}> o:leftOperand o:purpose; o:operator o:isAnyOf; o:rightOperand <urn:list> .\n"));
    }
    let graph = Graph::load_str(&rdf, "turtle").unwrap();
    let mut l = Ledger::new(default_policy_limits_for(&graph, Some(rdf.len())));
    let p = parse_policy_metered(&graph, &mut l).unwrap();
    assert_eq!(p.permissions[0].constraints.len(), 4000);
    assert!(
        l.spent(Dim::Nodes) <= 4000 + 4000,
        "cells plus references: {}",
        l.spent(Dim::Nodes)
    );
    assert!(l.spent(Dim::Bytes) <= l.limits().bytes);
    let Value::SharedStr(first) = &p.permissions[0].constraints[0].right else {
        panic!("expected shared folded string")
    };
    for c in &p.permissions[0].constraints {
        let Value::SharedStr(value) = &c.right else {
            panic!("expected shared folded string")
        };
        assert!(
            std::sync::Arc::ptr_eq(first, value),
            "one folded allocation per list"
        );
    }
    // Unattached fields are scanned, without constructing their atoms or resolving the list.
    let unattached = rdf.replace("<urn:r> o:constraint", "<urn:unused> o:constraint");
    let graph = Graph::load_str(&unattached, "turtle").unwrap();
    let mut l = Ledger::new(default_policy_limits_for(&graph, Some(unattached.len())));
    assert!(parse_policy_metered(&graph, &mut l).is_ok());
    assert_eq!(l.spent(Dim::Bytes), 0);
    assert_eq!(l.spent(Dim::Nodes), 0);
}

#[test]
fn per_compound_cap_is_independent_of_padding_rules() {
    let mut rdf = repeated_compounds(1, 2000);
    for i in 0..2000 {
        rdf.push_str(&format!(
            "<urn:a{i}> o:leftOperand o:purpose; o:operator o:eq; o:rightOperand <urn:value> .\n"
        ));
    }

    for i in 0..10 {
        rdf.push_str(&format!("<urn:p> o:permission <urn:pad{i}> .\n"));
    }
    let err = parse_policy_str(&rdf, "turtle").unwrap_err();
    assert_eq!(
        constraint_budget_refusal(&err),
        Some(ConstraintBudget::Expansion)
    );
}

#[test]
fn field_scans_charge_every_visited_triple() {
    let mut rdf = format!("{PREFIX}<urn:p> o:permission <urn:r> . <urn:r> o:constraint <urn:c> . <urn:c> o:leftOperand o:purpose; o:operator o:isAnyOf .\n");
    for i in 0..200 {
        rdf.push_str(&format!("<urn:c> o:rightOperand <urn:v{i}> .\n"));
    }
    let graph = Graph::load_str(&rdf, "turtle").unwrap();
    let mut l = Ledger::new(Limits::default());
    parse_policy_metered(&graph, &mut l).unwrap();
    assert!(
        l.spent(Dim::QueryRows) >= graph.len(),
        "all field candidates must be charged"
    );
}

#[test]
fn defaults_follow_parsed_graph_and_rule_counts() {
    use sparq_policy::default_policy_limits_for;
    let rdf =
        format!("{PREFIX}<urn:p> o:permission <urn:r1>, <urn:r2> . <urn:r1> o:action o:read .");
    let graph = Graph::load_str(&rdf, "turtle").unwrap();
    let source = default_policy_limits_for(&graph, Some(rdf.len()));
    assert_eq!(source.nodes, 2 * 1024);
    let mut sizing = Ledger::new(Limits::default());
    let scan = graph.store().scan(&[None, None, None]);
    for row in scan.rows.iter() {
        for id in scan.to_spo(row) {
            sizing.stored_term(graph.dict(), id).unwrap();
        }
    }
    let limits = default_policy_limits_for(&graph, None);
    assert_eq!(limits.bytes, sizing.spent(Dim::Bytes) * 4);
    assert_eq!(limits.bytes, source.bytes);
    for source_bytes in [0, 1, usize::MAX] {
        assert_eq!(
            default_policy_limits_for(&graph, Some(source_bytes)).bytes,
            limits.bytes
        );
    }
    assert_eq!(limits.nodes, source.nodes);
    assert_eq!(limits.query_rows, source.query_rows);
    assert!(parse_policy_metered(&graph, &mut Ledger::new(limits)).is_ok());
}

#[test]
fn generated_constraint_guards_consume_bytes() {
    for body in [
        "<urn:r> o:constraint <urn:missing> .",
        "<urn:r> o:constraint <urn:root> . <urn:root> o:and <urn:missing> .",
        "<urn:r> o:constraint <urn:root> . <urn:root> o:and <urn:root> .",
        "<urn:r> o:constraint <urn:c> . <urn:c> o:unit <urn:unit> .",
        "<urn:r> o:duty <urn:d> . <urn:d> o:constraint <urn:missing> .",
    ] {
        let rdf = format!("{PREFIX}<urn:p> o:permission <urn:r> . {body}");
        let mut limits = Limits::default();
        limits.bytes = 0;
        let mut l = Ledger::new(limits);
        let err = parse_policy_str_metered(&rdf, "turtle", &mut l).unwrap_err();
        assert_eq!(
            constraint_budget_refusal(&err),
            Some(ConstraintBudget::Bytes)
        );
        assert_eq!(l.spent(Dim::Bytes), 0);
        let mut l = Ledger::new(Limits::default());
        assert!(parse_policy_str_metered(&rdf, "turtle", &mut l).is_ok());
        assert!(l.spent(Dim::Bytes) > 0);
        assert!(parse_policy_str(&rdf, "turtle").is_ok());
    }
}

#[test]
fn singleton_lists_share_text_while_retaining_value_kinds() {
    use sparq_policy::{default_policy_limits_for, Value};
    let long = "v".repeat(16 * 1024);
    for operand in [
        format!("<urn:{long}>"),
        format!("\"{long}\""),
        "\"2026-10-26T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>".into(),
    ] {
        let mut rdf = format!("{PREFIX}<urn:p> o:permission <urn:r> . <urn:list> <http://www.w3.org/1999/02/22-rdf-syntax-ns#first> {operand}; <http://www.w3.org/1999/02/22-rdf-syntax-ns#rest> <http://www.w3.org/1999/02/22-rdf-syntax-ns#nil> .\n");
        for i in 0..128 {
            rdf.push_str(&format!("<urn:r> o:constraint <urn:c{i}> . <urn:c{i}> o:leftOperand o:purpose; o:operator o:eq; o:rightOperand <urn:list> .\n"));
        }
        let graph = Graph::load_str(&rdf, "turtle").unwrap();
        let mut l = Ledger::new(default_policy_limits_for(&graph, Some(rdf.len())));
        let policy = parse_policy_metered(&graph, &mut l).unwrap();
        let mut first = None;
        for constraint in &policy.permissions[0].constraints {
            let value = match &constraint.right {
                Value::SharedIri(value) if operand.starts_with('<') => value,
                Value::SharedDateTime(value) if operand.contains("^^") => value,
                Value::SharedStr(value) if !operand.contains("^^") => value,
                value => panic!("wrong shared value kind: {value:?}"),
            };
            if let Some(first) = first {
                assert!(std::sync::Arc::ptr_eq(first, value));
            } else {
                first = Some(value);
            }
        }
        assert_eq!(l.spent(Dim::Nodes), 129);
        assert!(l.spent(Dim::Bytes) <= l.limits().bytes);
    }
}

#[test]
fn shared_right_values_do_not_charge_operand_references_twice() {
    use sparq_policy::default_policy_limits_for;
    for list in [false, true] {
        let mut rdf =
            format!("{PREFIX}<urn:p> o:permission <urn:r> . <urn:r> o:constraint <urn:root> .\n");
        if list {
            rdf.push_str("<urn:list> <http://www.w3.org/1999/02/22-rdf-syntax-ns#first> <urn:value>; <http://www.w3.org/1999/02/22-rdf-syntax-ns#rest> <http://www.w3.org/1999/02/22-rdf-syntax-ns#nil> .\n");
        }
        let right = if list { "<urn:list>" } else { "<urn:value>" };
        for i in 0..1000 {
            rdf.push_str(&format!("<urn:root> o:and <urn:c{i}> . <urn:c{i}> o:leftOperand o:purpose; o:operator o:eq; o:rightOperand {right} .\n"));
        }
        let graph = Graph::load_str(&rdf, "turtle").unwrap();
        let mut l = Ledger::new(default_policy_limits_for(&graph, Some(rdf.len())));
        let p = parse_policy_metered(&graph, &mut l).unwrap();
        assert_eq!(p.permissions[0].logical_constraints[0].operands.len(), 1000);
        assert_eq!(l.spent(Dim::Nodes), 1000 + usize::from(list));
        assert!(parse_policy_str(&rdf, "turtle").is_ok());
    }
}
