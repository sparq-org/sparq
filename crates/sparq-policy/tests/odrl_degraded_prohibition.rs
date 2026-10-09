//! Every constraint the evaluator cannot decide is Unknown, and Unknown is fail-closed
//! on both rule kinds: a permission grants only on a definite yes, and a prohibition
//! fires unless it definitely does not apply. On a prohibition an unsatisfiable guard
//! would fail OPEN (the rule never fires, so a sibling permission grants).
//!
//! The enumeration crosses every unsupported, malformed or incomparable constraint shape
//! with both rule kinds and every wrapper (atomic, `and`, `or`, `xone`, an action
//! refinement). Each combination must either refuse the policy at parse time or deny.

use sparq_policy::{evaluate, parse_policy_str, Request, Value};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

/// A constraint the request below definitely satisfies.
const TRUE_C: &str = "[ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ; odrl:rightOperand <urn:p/a> ]";
/// A constraint the request below definitely fails.
const FALSE_C: &str = "[ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ; odrl:rightOperand <urn:p/z> ]";

/// Shapes the evaluator cannot decide for the request built by [`request`].
const SHAPES: [(&str, &str); 10] = [
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
        r#"[ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isAnyOf ; odrl:rightOperand ( "two words" <urn:p/a> ) ]"#,
    ),
    ("missing rightOperand", "[ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ]"),
    (
        "unknown operator",
        "[ odrl:leftOperand odrl:purpose ; odrl:operator <urn:op/unknown> ; odrl:rightOperand <urn:p/a> ]",
    ),
    (
        "typed set membership",
        "[ odrl:leftOperand odrl:count ; odrl:operator odrl:isAnyOf ; odrl:rightOperand ( 1 2 ) ]",
    ),
    (
        "unsupported unit",
        "[ odrl:leftOperand odrl:count ; odrl:operator odrl:lteq ; odrl:rightOperand 5 ; odrl:unit <urn:unit/kb> ]",
    ),
    (
        "incomparable operand types",
        "[ odrl:leftOperand odrl:count ; odrl:operator odrl:eq ; odrl:rightOperand <urn:p/a> ]",
    ),
    (
        "unparseable dateTime bound",
        r#"[ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ; odrl:rightOperand "soon"^^xsd:dateTime ]"#,
    ),
    (
        "no evidence for the dimension",
        "[ odrl:leftOperand odrl:spatial ; odrl:operator odrl:eq ; odrl:rightOperand <urn:region/eu> ]",
    ),
];

fn request() -> Request {
    Request::new(format!("{ODRL}read"))
        .on("urn:asset/x")
        .at("2026-06-01T00:00:00Z")
        .for_purpose(Value::Iri("urn:p/a".into()))
        .with(format!("{ODRL}count"), Value::Num(2.0))
}

/// The constraint object for `shape` under each wrapper.
fn wrapped(shape: &str) -> [(&'static str, String); 4] {
    let lc = |op: &str, other: &str| {
        format!("[ a odrl:LogicalConstraint ; odrl:{op} ( {shape} {other} ) ]")
    };
    [
        ("atomic", shape.to_owned()),
        ("and", lc("and", TRUE_C)),
        // `or` with a definitely-false sibling: Unknown, not a grant.
        ("or", lc("or", FALSE_C)),
        // `xone` with a definitely-true sibling: the Unknown operand might be a second
        // true, so the exactly-one count is Unknown.
        ("xone", lc("xone", TRUE_C)),
    ]
}

const PREFIXES: &str = "@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n\
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n";

/// Parse `ttl` and evaluate the request: `None` when the policy is refused.
fn allows(ttl: &str) -> Option<bool> {
    parse_policy_str(ttl, "turtle").ok().map(|p| evaluate(&p, &request()).allow)
}

#[test]
fn every_undecidable_shape_fails_closed_on_both_rule_kinds() {
    for (name, shape) in SHAPES {
        for (wrapper, c) in wrapped(shape) {
            let permission = format!(
                "{PREFIXES}<urn:pol/p> a odrl:Set ; odrl:permission [ odrl:action odrl:read ; \
                 odrl:target <urn:asset/x> ; odrl:constraint {c} ] ."
            );
            assert_ne!(allows(&permission), Some(true), "permission, {wrapper}, {name}");
            let prohibition = format!(
                "{PREFIXES}<urn:pol/p> a odrl:Set ; \
                 odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ; \
                 odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ; \
                 odrl:constraint {c} ] ."
            );
            assert_ne!(allows(&prohibition), Some(true), "prohibition, {wrapper}, {name}");
        }
        // An action refinement is not supported, so either rule kind refuses the policy.
        for rule in ["permission", "prohibition"] {
            let refined = format!(
                "{PREFIXES}<urn:pol/p> a odrl:Set ; \
                 odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ; \
                   odrl:constraint {FALSE_C} ] ; \
                 odrl:{rule} [ odrl:target <urn:asset/x> ; \
                   odrl:action [ rdf:value odrl:read ; odrl:refinement {shape} ] ] ."
            );
            assert_eq!(allows(&refined), None, "{rule}, refinement, {name}");
        }
    }
}

/// The control: the same wrappers around decidable constraints still decide. A true
/// permission grants and a false prohibition does not block.
#[test]
fn decidable_constraints_still_decide() {
    for (wrapper, c) in wrapped(TRUE_C) {
        let permission = format!(
            "{PREFIXES}<urn:pol/p> a odrl:Set ; odrl:permission [ odrl:action odrl:read ; \
             odrl:target <urn:asset/x> ; odrl:constraint {c} ] ."
        );
        // `or (true, false)` and `and (true, true)` hold; `xone (true, true)` does not.
        assert_eq!(allows(&permission), Some(wrapper != "xone"), "permission, {wrapper}");
    }
    let prohibition = format!(
        "{PREFIXES}<urn:pol/p> a odrl:Set ; \
         odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ; \
         odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ; \
         odrl:constraint {FALSE_C} ] ."
    );
    assert_eq!(allows(&prohibition), Some(true));
}

/// A constrained duty is never provably discharged, so its permission does not grant.
#[test]
fn a_constrained_duty_is_not_discharged_by_its_action_alone() {
    let ttl = format!(
        "{PREFIXES}<urn:pol/p> a odrl:Set ; odrl:permission [ odrl:action odrl:read ; \
         odrl:target <urn:asset/x> ; odrl:duty [ odrl:action odrl:compensate ; \
         odrl:constraint [ odrl:leftOperand odrl:payAmount ; odrl:operator odrl:eq ; \
         odrl:rightOperand 5 ] ] ] ."
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let req = request().discharge(format!("{ODRL}compensate"));
    assert!(!evaluate(&p, &req).allow);
}
