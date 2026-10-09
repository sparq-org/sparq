//! A prohibition whose constraint cannot be represented must not silently stop gating.
//! On a permission the unsatisfiable guard fails closed (the rule never grants); on a
//! prohibition it would fail OPEN (the rule never fires, so a sibling permission grants).
//! So a prohibition carrying a degraded constraint, atomic or inside a compound, refuses
//! the whole policy, like a malformed collection operand.

use sparq_policy::{evaluate, parse_policy_str, Request};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

fn policy(rule: &str, constraint: &str) -> String {
    format!(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/p> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:{rule} [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint {constraint} ] .
"#
    )
}

/// Each shape degrades to the unsatisfiable guard at parse time.
const DEGRADED: [(&str, &str); 5] = [
    (
        "two dateTimes under lteq",
        r#"[ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
             odrl:rightOperand "2026-12-31T00:00:00Z"^^xsd:dateTime , "2027-12-31T00:00:00Z"^^xsd:dateTime ]"#,
    ),
    (
        "two purposes under eq",
        "[ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ; odrl:rightOperand <urn:p/a> , <urn:p/b> ]",
    ),
    (
        "separator-carrying set member",
        r#"[ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isAnyOf ; odrl:rightOperand ( "two words" <urn:p/b> ) ]"#,
    ),
    ("missing rightOperand", "[ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ]"),
    (
        "unknown operator",
        "[ odrl:leftOperand odrl:purpose ; odrl:operator <urn:op/unknown> ; odrl:rightOperand <urn:p/a> ]",
    ),
];

#[test]
fn a_degraded_prohibition_constraint_refuses_the_policy() {
    for (name, c) in DEGRADED {
        let err = parse_policy_str(&policy("prohibition", c), "turtle").unwrap_err();
        assert!(err.contains("prohibition"), "{name}: {err}");
    }
}

#[test]
fn a_degraded_operand_inside_a_prohibition_compound_refuses_the_policy() {
    let (_, bad) = DEGRADED[0];
    let compound = format!(
        "[ a odrl:LogicalConstraint ; odrl:or ( {bad} \
           [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ; odrl:rightOperand <urn:p/a> ] ) ]"
    );
    assert!(parse_policy_str(&policy("prohibition", &compound), "turtle").is_err());
}

/// On a permission the guard already fails closed, so the policy still parses and the
/// degraded permission grants nothing beyond its unconditional sibling.
#[test]
fn a_degraded_permission_constraint_still_fails_closed() {
    for (name, c) in DEGRADED {
        let ttl = format!(
            r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/p> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ; odrl:constraint {c} ] .
"#
        );
        let p = parse_policy_str(&ttl, "turtle").unwrap_or_else(|e| panic!("{name}: {e}"));
        let req = Request::new(format!("{ODRL}read")).on("urn:asset/x").at("2026-06-01T00:00:00Z");
        assert!(!evaluate(&p, &req).allow, "{name}");
    }
}
