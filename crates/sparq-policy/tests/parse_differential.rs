//! Run the existing policy tests through both parsers, including their generated
//! inputs and the fetched conformance policies. `support/main_parse.rs` is the
//! unchanged parsing algorithm from origin/main at the revision named there.
//! Only set-operator right operands are normalized as sets; all other model fields,
//! rule order, compound structure and accepted/refused outcomes are compared.
extern crate self as sparq_policy;
extern crate sparq_policy as current;
pub use current::*;

#[path = "support/main_parse.rs"]
mod main_parse;

fn normalize(mut p: Policy) -> Policy {
    fn atom(c: &mut Constraint) {
        if matches!(
            c.operator,
            Operator::IsAnyOf | Operator::IsPartOf | Operator::IsNoneOf
        ) {
            if let Value::Str(s) = &mut c.right {
                let parts: std::collections::BTreeSet<_> = s.split('|').collect();
                *s = parts.into_iter().collect::<Vec<_>>().join("|");
            }
        }
    }
    fn compound(c: &mut LogicalConstraint) {
        for n in &mut c.operands {
            match n {
                ConstraintNode::Atomic(c) => atom(c),
                ConstraintNode::Compound(c) => compound(c),
            }
        }
    }
    for r in p.permissions.iter_mut().chain(p.prohibitions.iter_mut()) {
        for c in &mut r.constraints {
            atom(c);
        }
        for c in &mut r.logical_constraints {
            compound(c);
        }
        for d in &mut r.duties {
            for c in &mut d.constraints {
                atom(c);
            }
        }
    }
    p
}

pub fn parse_policy(graph: &sparq_core::Graph) -> Result<ValidatedPolicy, String> {
    let actual = current::parse_policy(graph);
    let expected = main_parse::parse_policy(graph);
    match (&actual, &expected) {
        (Ok(a), Ok(b)) => assert_eq!(normalize(a.policy().clone()), normalize(b.policy().clone())),
        (Err(_), Err(_)) => {}
        _ => panic!("parse parity differs: current={actual:?}; main={expected:?}"),
    }
    actual
}
pub fn parse_policy_str(rdf: &str, format: &str) -> Result<ValidatedPolicy, String> {
    // Load once, so randomly allocated blank-node labels are identical.
    let graph = sparq_core::Graph::load_str(rdf, format)?;
    let actual = current::parse_policy_str(rdf, format);
    let compared = parse_policy(&graph);
    assert_eq!(
        actual.is_ok(),
        compared.is_ok(),
        "string and graph wrappers disagree: {actual:?}; {compared:?}"
    );
    compared
}
pub fn parse_policy_str_with_memberships(
    rdf: &str,
    format: &str,
) -> Result<
    (
        ValidatedPolicy,
        std::collections::BTreeSet<(String, String)>,
    ),
    String,
> {
    let graph = sparq_core::Graph::load_str(rdf, format)?;
    let policy = parse_policy(&graph)?;
    let members = current::target_memberships(&policy, &graph)?;
    let old_members = main_parse::target_memberships(&policy, &graph)?;
    assert_eq!(members, old_members);
    let actual = current::parse_policy_str_with_memberships(rdf, format);
    assert!(actual.is_ok(), "membership wrapper refused {actual:?}");
    Ok((policy, members))
}

#[path = "decision_report.rs"]
mod decision_report;

#[path = "eval_metamorph.rs"]
mod eval_metamorph;

#[path = "odrl_compare.rs"]
mod odrl_compare;

#[path = "odrl_compare_compound.rs"]
mod odrl_compare_compound;

#[path = "odrl_compare_witness.rs"]
mod odrl_compare_witness;

#[path = "odrl_conflict_strategy.rs"]
mod odrl_conflict_strategy;

#[path = "odrl_constraint_budget.rs"]
mod odrl_constraint_budget;

#[path = "odrl_constraint_matching.rs"]
mod odrl_constraint_matching;

#[path = "odrl_count.rs"]
mod odrl_count;

#[path = "odrl_count_backend.rs"]
mod odrl_count_backend;

#[path = "odrl_count_file.rs"]
mod odrl_count_file;

#[path = "odrl_count_stateless.rs"]
mod odrl_count_stateless;

#[path = "odrl_datetimestamp.rs"]
mod odrl_datetimestamp;

#[path = "odrl_degraded_prohibition.rs"]
mod odrl_degraded_prohibition;

#[path = "odrl_eval.rs"]
mod odrl_eval;

#[path = "odrl_hierarchy.rs"]
mod odrl_hierarchy;

#[path = "odrl_right_operand_sets.rs"]
mod odrl_right_operand_sets;

#[path = "odrl_set_operators.rs"]
mod odrl_set_operators;

#[path = "odrl_test_suite.rs"]
mod odrl_test_suite;

#[path = "secprop_leftoperands.rs"]
mod secprop_leftoperands;

#[path = "target_memberships.rs"]
mod target_memberships;

#[test]
fn repeated_real_atoms_retain_main_results_with_default_limits() {
    let mut rdf = String::from("@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> a o:Set .\n");
    for i in 0..64 {
        rdf.push_str(&format!(
            "<urn:p> o:permission <urn:r{i}> . <urn:r{i}> o:constraint <urn:lc> .\n"
        ));
    }
    for i in 0..1000 {
        rdf.push_str(&format!("<urn:lc> o:and <urn:a{i}> . <urn:a{i}> o:leftOperand o:purpose ; o:operator o:eq ; o:rightOperand <urn:v> .\n"));
    }
    let p = parse_policy_str(&rdf, "turtle").unwrap();
    assert_eq!(p.permissions.len(), 64);
    assert!(p
        .permissions
        .iter()
        .all(|r| r.logical_constraints[0].operands.len() == 1000));
}

#[test]
fn per_property_extraction_preserves_references_missing_fields_and_sets() {
    for left in ["o:leftOperand o:purpose;", ""] {
        for direct in ["o:rightOperand <urn:a>, <urn:b>;", ""] {
            for reference in ["o:rightOperandReference <urn:b>, <urn:c>;", ""] {
                for op in ["o:operator o:isAnyOf;", "o:operator o:eq;", ""] {
                    let rdf = format!("@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> o:permission [ o:constraint <urn:c> ] . <urn:c> {left} {direct} {reference} {op} a o:Constraint .");
                    parse_policy_str(&rdf, "turtle").unwrap();
                }
            }
        }
    }
}

// Reuse the vendored and generated example fixtures through the same proxy.
#[allow(dead_code)]
#[path = "../examples/bench.rs"]
mod bench;
#[allow(dead_code)]
#[path = "../examples/policy_bench.rs"]
mod policy_bench;

#[test]
fn list_cells_and_several_compounds_on_one_rule_retain_main_results() {
    let mut rdf = String::from("@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> o:permission <urn:r> . <urn:r> o:constraint <urn:lc1>, <urn:lc2> . <urn:lc1> o:and (");
    for i in 0..1000 {
        rdf.push_str(&format!(" <urn:a{i}>"));
    }
    rdf.push_str(") . <urn:lc2> o:and <urn:lc1> .");
    parse_policy_str(&rdf, "turtle").unwrap();
}

#[test]
fn shared_folded_set_atoms_retain_main_results() {
    let mut rdf = String::from("@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> o:permission <urn:r> . <urn:r> o:constraint <urn:root> . <urn:atom> o:leftOperand o:purpose ; o:operator o:isAnyOf ; o:rightOperand (");
    for i in 0..2000 {
        rdf.push_str(&format!(" <urn:value{i}>"));
    }
    rdf.push_str(") .\n");
    for i in 0..500 {
        rdf.push_str(&format!(
            "<urn:root> o:and <urn:sub{i}> . <urn:sub{i}> o:and <urn:atom> .\n"
        ));
    }
    let graph = sparq_core::Graph::load_str(&rdf, "turtle").unwrap();
    let expected = main_parse::parse_policy(&graph).expect("main accepts this shared folded atom");
    let actual = current::parse_policy(&graph).expect("defaults cover folded atom copies");
    assert_eq!(
        normalize(actual.into_inner()),
        normalize(expected.into_inner())
    );
    assert!(current::parse_policy_str(&rdf, "turtle").is_ok());
}
