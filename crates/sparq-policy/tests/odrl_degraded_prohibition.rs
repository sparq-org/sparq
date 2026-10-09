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
const TRUE_C: &str =
    "[ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ; odrl:rightOperand <urn:p/a> ]";
/// A constraint the request below definitely fails.
const FALSE_C: &str =
    "[ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ; odrl:rightOperand <urn:p/z> ]";

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
    parse_policy_str(ttl, "turtle")
        .ok()
        .map(|p| evaluate(&p, &request()).allow)
}

#[test]
fn every_undecidable_shape_fails_closed_on_both_rule_kinds() {
    for (name, shape) in SHAPES {
        for (wrapper, c) in wrapped(shape) {
            let permission = format!(
                "{PREFIXES}<urn:pol/p> a odrl:Set ; odrl:permission [ odrl:action odrl:read ; \
                 odrl:target <urn:asset/x> ; odrl:constraint {c} ] ."
            );
            assert_ne!(
                allows(&permission),
                Some(true),
                "permission, {wrapper}, {name}"
            );
            let prohibition = format!(
                "{PREFIXES}<urn:pol/p> a odrl:Set ; \
                 odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ; \
                 odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ; \
                 odrl:constraint {c} ] ."
            );
            assert_ne!(
                allows(&prohibition),
                Some(true),
                "prohibition, {wrapper}, {name}"
            );
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
        assert_eq!(
            allows(&permission),
            Some(wrapper != "xone"),
            "permission, {wrapper}"
        );
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

// --- The validation boundary: typed construction gets the same admission checks. ---

use sparq_policy::{
    contains, decide, Action, Constraint, ConstraintNode, Containment, LogicalConstraint,
    LogicalOperator, Operator, Policy, Rule,
};

fn rule(id: &str, action: &str) -> Rule {
    Rule {
        id: id.into(),
        action: Action(action.into()),
        target: Some("urn:asset/x".into()),
        assignee: None,
        assigner: None,
        constraints: Vec::new(),
        logical_constraints: Vec::new(),
        duties: Vec::new(),
    }
}

fn atom(left: &str, operator: Operator, right: Value) -> Constraint {
    Constraint {
        left: left.into(),
        operator,
        right,
    }
}

fn compound(operator: LogicalOperator, operands: Vec<Constraint>) -> LogicalConstraint {
    LogicalConstraint {
        id: "_:lc".into(),
        operator,
        operands: operands.into_iter().map(ConstraintNode::Atomic).collect(),
    }
}

fn purpose(iri: &str) -> Constraint {
    atom(
        &format!("{ODRL}purpose"),
        Operator::Eq,
        Value::Iri(iri.into()),
    )
}

#[test]
fn validate_refuses_a_blank_prohibition_head_built_by_hand() {
    let read = format!("{ODRL}read");
    for (field, prohibition) in [
        ("action", rule("urn:r/no", "_:refined")),
        (
            "target",
            Rule {
                target: Some("_:anon".into()),
                ..rule("urn:r/no", &read)
            },
        ),
        (
            "assignee",
            Rule {
                assignee: Some("_:anon".into()),
                ..rule("urn:r/no", &read)
            },
        ),
    ] {
        let policy = Policy {
            permissions: vec![rule("urn:r/yes", &read)],
            prohibitions: vec![prohibition],
            ..Policy::default()
        };
        assert!(policy.validate().is_err(), "blank prohibition {field}");
    }
}

#[test]
fn validate_refuses_empty_compounds_on_either_rule_kind() {
    let read = format!("{ODRL}read");
    for op in [
        LogicalOperator::And,
        LogicalOperator::Or,
        LogicalOperator::Xone,
    ] {
        let empty = Rule {
            logical_constraints: vec![compound(op, Vec::new())],
            ..rule("urn:r", &read)
        };
        let as_permission = Policy {
            permissions: vec![empty.clone()],
            ..Policy::default()
        };
        assert!(as_permission.validate().is_err(), "empty {op:?} permission");
        let as_prohibition = Policy {
            permissions: vec![rule("urn:r/yes", &read)],
            prohibitions: vec![empty],
            ..Policy::default()
        };
        assert!(
            as_prohibition.validate().is_err(),
            "empty {op:?} prohibition"
        );
    }
}

/// `xone` is "exactly one True": two definite Trues decide False whatever the rest.
#[test]
fn xone_with_two_trues_is_false_even_with_an_unknown_operand() {
    let read = format!("{ODRL}read");
    let unknown = atom(
        &format!("{ODRL}spatial"),
        Operator::Eq,
        Value::Iri("urn:region/eu".into()),
    );
    let xone = compound(
        LogicalOperator::Xone,
        vec![purpose("urn:p/a"), purpose("urn:p/a"), unknown],
    );
    // As a prohibition it definitely does not apply, so the sibling permission grants.
    let policy = Policy {
        permissions: vec![rule("urn:r/yes", &read)],
        prohibitions: vec![Rule {
            logical_constraints: vec![xone.clone()],
            ..rule("urn:r/no", &read)
        }],
        ..Policy::default()
    }
    .validate()
    .unwrap();
    assert!(decide(&policy, &request()).allow);
    // As a permission it definitely does not hold.
    let policy = Policy {
        permissions: vec![Rule {
            logical_constraints: vec![xone],
            ..rule("urn:r/yes", &read)
        }],
        ..Policy::default()
    }
    .validate()
    .unwrap();
    assert!(!decide(&policy, &request()).allow);
}

/// A numeric set flattens to a string, so it degrades to Unknown: textual evidence that
/// spells a member does not grant, and textual evidence outside it does not withdraw a
/// prohibition.
#[test]
fn a_typed_set_never_matches_textual_evidence() {
    let set =
        "[ odrl:leftOperand odrl:count ; odrl:operator odrl:isAnyOf ; odrl:rightOperand ( 1 2 ) ]";
    let textual = |v: &str| {
        Request::new(format!("{ODRL}read"))
            .on("urn:asset/x")
            .with(format!("{ODRL}count"), Value::Str(v.into()))
    };
    let permission = format!(
        "{PREFIXES}<urn:pol/p> a odrl:Set ; odrl:permission [ odrl:action odrl:read ; \
         odrl:target <urn:asset/x> ; odrl:constraint {set} ] ."
    );
    let p = parse_policy_str(&permission, "turtle").unwrap();
    assert!(!decide(&p, &textual("1")).allow);
    let prohibition = format!(
        "{PREFIXES}<urn:pol/p> a odrl:Set ; \
         odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ; \
         odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ; \
         odrl:constraint {set} ] ."
    );
    assert!(parse_policy_str(&prohibition, "turtle").is_err());
}

/// Static containment claims an implication only where the evaluator decides: an outer
/// `neq <urn:x>` does not admit an inner `eq 1`, because the evaluator reads the
/// number-against-IRI pair as Unknown and the outer permission does not grant.
#[test]
fn containment_does_not_claim_an_incomparable_implication() {
    let read = format!("{ODRL}read");
    let with = |c: Constraint| Policy {
        permissions: vec![Rule {
            constraints: vec![c],
            ..rule("urn:r", &read)
        }],
        ..Policy::default()
    };
    let inner = with(atom("urn:dimension", Operator::Eq, Value::Num(1.0)));
    let outer = with(atom(
        "urn:dimension",
        Operator::Neq,
        Value::Iri("urn:x".into()),
    ));
    assert_ne!(contains(&outer, &inner), Containment::Contains);
    let req = Request::new(read.clone())
        .on("urn:asset/x")
        .with("urn:dimension", Value::Num(1.0));
    assert!(decide(&inner.validate().unwrap(), &req).allow);
    assert!(!decide(&outer.validate().unwrap(), &req).allow);
}

/// A declared conflict strategy the engine cannot honour denies at the decision point.
#[test]
fn decide_denies_under_an_unhonourable_conflict_strategy() {
    let ttl = format!(
        "{PREFIXES}<urn:pol/p> a odrl:Set ; odrl:conflict odrl:perm ; \
         odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ."
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let d = decide(&p, &request());
    assert!(!d.allow && d.permit.is_none());
}

/// A grant carries a permit naming what it covers; a deny carries none.
#[test]
fn only_a_grant_carries_a_permit() {
    let ttl = format!(
        "{PREFIXES}<urn:pol/p> a odrl:Set ; odrl:permission <urn:r/yes> . \
         <urn:r/yes> odrl:action odrl:read ; odrl:target <urn:asset/x> ."
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let d = decide(&p, &request());
    let permit = d.permit.expect("granted");
    assert_eq!(
        (permit.rule(), permit.target()),
        ("urn:r/yes", Some("urn:asset/x"))
    );
    let d = decide(&p, &Request::new(format!("{ODRL}write")).on("urn:asset/x"));
    assert!(!d.allow && d.permit.is_none());
}

/// Containment uses the evaluator's own comparison: an inner `eq` instant spelled in
/// another offset is the excluded instant of an outer `neq`, so it is not contained.
#[test]
fn containment_compares_instants_like_the_evaluator() {
    let read = format!("{ODRL}read");
    let dt = format!("{ODRL}dateTime");
    let with = |c: Constraint| Policy {
        permissions: vec![Rule {
            constraints: vec![c],
            ..rule("urn:r", &read)
        }],
        ..Policy::default()
    };
    let inner = with(atom(
        &dt,
        Operator::Eq,
        Value::DateTime("2026-06-16T12:00:00Z".into()),
    ));
    let outer = with(atom(
        &dt,
        Operator::Neq,
        Value::DateTime("2026-06-16T14:00:00+02:00".into()),
    ));
    assert_ne!(contains(&outer, &inner), Containment::Contains);
    let req = Request::new(read.clone())
        .on("urn:asset/x")
        .at("2026-06-16T12:00:00Z");
    assert!(decide(&inner.validate().unwrap(), &req).allow);
    assert!(!decide(&outer.validate().unwrap(), &req).allow);
}

/// An inclusive inner bound does not imply a strict outer bound at the same value.
#[test]
fn containment_does_not_read_lteq_as_lt() {
    let read = format!("{ODRL}read");
    let n = "urn:dimension";
    let with = |op: Operator| Policy {
        permissions: vec![Rule {
            constraints: vec![atom(n, op, Value::Num(5.0))],
            ..rule("urn:r", &read)
        }],
        ..Policy::default()
    };
    assert_ne!(
        contains(&with(Operator::Lt), &with(Operator::Lteq)),
        Containment::Contains
    );
    assert_ne!(
        contains(&with(Operator::Gt), &with(Operator::Gteq)),
        Containment::Contains
    );
    assert_eq!(
        contains(&with(Operator::Lteq), &with(Operator::Lt)),
        Containment::Contains
    );
}
