//! Differential oracle pinned to origin/main at 3685a49e.
//!
//! `main_parse.rs` is that revision's parse.rs, with only its two model/validation
//! imports redirected to the public crate. Each existing test's parse uses one
//! loaded graph for both parsers so blank-node identifiers agree. Only set-valued
//! constraint operands are normalized; rule, constraint and compound order remain
//! significant. Both errors count as the same refusal, since diagnostics may grow.

#[allow(dead_code)]
mod main_parse;
use sparq_core::Graph;
use sparq_policy::{Constraint, ConstraintNode, Operator, Policy, ValidatedPolicy, Value};

fn normalize_constraint(c: &mut Constraint) {
    if matches!(c.operator, Operator::IsPartOf | Operator::IsAnyOf | Operator::IsNoneOf) {
        if let Value::Str(s) = &mut c.right {
            let mut members: Vec<_> = s.split('|').collect();
            members.sort_unstable();
            members.dedup();
            *s = members.join("|");
        }
    }
}

fn normalize_nodes(nodes: &mut [ConstraintNode]) {
    for node in nodes {
        match node {
            ConstraintNode::Atomic(c) => normalize_constraint(c),
            ConstraintNode::Compound(c) => normalize_nodes(&mut c.operands),
        }
    }
}

fn normalize(p: ValidatedPolicy) -> Policy {
    let mut p = p.into_inner();
    for rule in p.permissions.iter_mut().chain(&mut p.prohibitions) {
        for c in &mut rule.constraints { normalize_constraint(c); }
        for c in &mut rule.logical_constraints { normalize_nodes(&mut c.operands); }
        for duty in &mut rule.duties {
            for c in &mut duty.constraints { normalize_constraint(c); }
        }
    }
    p
}

#[allow(dead_code)]
pub fn parse_policy_str(rdf: &str, format: &str) -> Result<ValidatedPolicy, String> {
    let graph = Graph::load_str(rdf, format)?;
    let expected = main_parse::parse_policy(&graph);
    let actual = sparq_policy::parse_policy(&graph);
    match (&expected, &actual) {
        (Ok(expected), Ok(actual)) => assert_eq!(normalize(actual.clone()), normalize(expected.clone()), "baseline parse differs: {rdf}"),
        (Err(_), Err(_)) => {},
        _ => panic!("baseline parse differs: expected {expected:?}, actual {actual:?}\n{rdf}"),
    }
    // Exercise the source-aware entry point too, with the same structural oracle.
    let source_actual = sparq_policy::parse_policy_str(rdf, format);
    assert_eq!(source_actual.is_ok(), actual.is_ok(), "source-aware admission differs: {rdf}");
    actual
}

#[allow(dead_code)]
pub fn parse_policy_str_with_memberships(rdf: &str, format: &str) -> Result<(ValidatedPolicy, std::collections::BTreeSet<(String, String)>), String> {
    let policy = parse_policy_str(rdf, format)?;
    let graph = Graph::load_str(rdf, format)?;
    let members = sparq_policy::target_memberships(&policy, &graph)?;
    let (_, source_members) = sparq_policy::parse_policy_str_with_memberships(rdf, format)?;
    assert_eq!(members, source_members);
    Ok((policy, members))
}
