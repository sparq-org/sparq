//! [GPT-6] Signed host semantics and unsigned isolation; no guest or proof evidence.

use oxrdf::{Literal, NamedNode, Term, Triple};
use sparq_zk::commit::commit_triples;
use sparq_zk::field::Fr;
use sparq_zk_compose::manifest::FilterOp;
use sparq_zk_compose::planner::signed::{
    canonical_signed_integer, optimize_signed_disclosure, plan_signed_disclosure,
    plan_signed_disclosure_admitted, SignedDisclosureQuery,
};
use sparq_zk_compose::planner::{
    canonical_integer, plan_disclosure, DisclosureQuery, OptimizationCompletion,
    OptimizationLimits, PlanError, PlannerLimits,
};
use std::collections::BTreeMap;

fn integer(value: &str) -> Term {
    Literal::new_typed_literal(
        value,
        NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
    )
    .into()
}

fn query(filter: &str) -> String {
    format!("SELECT DISTINCT ?s WHERE {{ ?s <urn:value> ?v FILTER({filter}) }}")
}

fn release() -> BTreeMap<String, Term> {
    BTreeMap::from([("s".into(), NamedNode::new("urn:subject").unwrap().into())])
}

fn graph(value: &str) -> sparq_zk::commit::GraphCommitment {
    commit_triples(
        &[Triple::new(
            NamedNode::new("urn:subject").unwrap(),
            NamedNode::new("urn:value").unwrap(),
            integer(value),
        )],
        Fr::from(3_u64),
    )
    .unwrap()
}

#[test]
fn canonical_signed_integer_admits_exact_boundaries_and_rejects_other_tokens() {
    for value in [i64::MIN, -1, 0, 1, i64::MAX] {
        assert_eq!(
            canonical_signed_integer(&integer(&value.to_string())),
            Some(value)
        );
    }
    for lexical in [
        "-9223372036854775809",
        "9223372036854775808",
        "-0",
        "+0",
        "+1",
        "00",
        "01",
        "-01",
        " 1",
        "1 ",
        "",
        "-",
        "1.0",
    ] {
        assert_eq!(
            canonical_signed_integer(&integer(lexical)),
            None,
            "{lexical}"
        );
    }
    for datatype in [
        "http://www.w3.org/2001/XMLSchema#decimal",
        "http://www.w3.org/2001/XMLSchema#int",
        "http://www.w3.org/2001/XMLSchema#string",
    ] {
        let term = Literal::new_typed_literal("-1", NamedNode::new(datatype).unwrap()).into();
        assert_eq!(canonical_signed_integer(&term), None);
    }
    assert_eq!(
        canonical_signed_integer(
            &Literal::new_language_tagged_literal("-1", "en")
                .unwrap()
                .into()
        ),
        None
    );
}

#[test]
fn signed_query_normalizes_both_operand_orders_without_changing_unsigned_admission() {
    for bound in [i64::MIN, -1, 0, 1, i64::MAX] {
        let q = SignedDisclosureQuery::parse(&query(&format!("?v >= {bound} && {bound} <= ?v")))
            .unwrap();
        assert_eq!(q.filters().len(), 2);
        assert!(q
            .filters()
            .iter()
            .all(|f| f.bound == bound && f.op == FilterOp::Ge));
    }
    for bound in [
        "-0",
        "+1",
        "01",
        "-01",
        "-9223372036854775809",
        "9223372036854775808",
    ] {
        let typed = format!("?v >= \"{bound}\"^^<http://www.w3.org/2001/XMLSchema#integer>");
        assert!(
            SignedDisclosureQuery::parse(&query(&typed)).is_err(),
            "{bound}"
        );
    }
    assert!(DisclosureQuery::parse(&query("?v >= -1")).is_err());
    assert!(DisclosureQuery::parse(&query("?v <= 18446744073709551615")).is_ok());
    assert_eq!(
        canonical_integer(&integer("18446744073709551615")),
        Some(u64::MAX)
    );
    assert_eq!(canonical_integer(&integer("-1")), None);
}

#[test]
fn signed_planners_match_signed_order_across_zero_and_extremes() {
    let values = [i64::MIN, -1, 0, 1, i64::MAX];
    for value in values {
        let credential = graph(&value.to_string());
        for bound in values {
            let q = SignedDisclosureQuery::parse(&query(&format!("?v < {bound}"))).unwrap();
            let first = plan_signed_disclosure(
                &q,
                std::slice::from_ref(&credential),
                &[release()],
                PlannerLimits::default(),
            );
            let optimized = optimize_signed_disclosure(
                &q,
                std::slice::from_ref(&credential),
                &[release()],
                OptimizationLimits::default(),
            )
            .unwrap();
            assert_eq!(first.is_ok(), value < bound, "{value} < {bound}");
            assert_eq!(
                optimized.plan.is_some(),
                value < bound,
                "optimized {value} < {bound}"
            );
            assert_eq!(
                optimized.completion,
                if value < bound {
                    OptimizationCompletion::Optimal
                } else {
                    OptimizationCompletion::Infeasible
                }
            );
        }
    }
}

#[test]
fn signed_planning_retains_exact_leaf_and_backend_admission() {
    let credential = graph("-1");
    let q = SignedDisclosureQuery::parse(&query("?v = -1")).unwrap();
    let plan = plan_signed_disclosure(
        &q,
        std::slice::from_ref(&credential),
        &[release()],
        PlannerLimits::default(),
    )
    .unwrap();
    let witness = plan.rows[0].witnesses[0];
    assert_eq!(
        credential.canonical.triples[witness.leaf].object,
        integer("-1")
    );
    assert!(matches!(
        plan_signed_disclosure_admitted(
            &q,
            &[credential],
            &[release()],
            PlannerLimits::default(),
            |_, _, _| false
        ),
        Err(PlanError::NoWitness { .. })
    ));
    for value in ["-0", "-01", "+1", "9223372036854775808"] {
        let q = SignedDisclosureQuery::parse(&query("?v >= -9223372036854775808")).unwrap();
        assert!(
            matches!(
                plan_signed_disclosure(&q, &[graph(value)], &[release()], PlannerLimits::default()),
                Err(PlanError::NoWitness { .. })
            ),
            "{value}"
        );
    }
    let unsigned = DisclosureQuery::parse(&query("?v < 10")).unwrap();
    assert!(matches!(
        plan_disclosure(
            &unsigned,
            &[graph("-1")],
            &[release()],
            PlannerLimits::default()
        ),
        Err(PlanError::NoWitness { .. })
    ));
}

#[test]
fn signed_admission_rejects_scope_arithmetic_and_other_numeric_types() {
    for filter in [
        "?v < 1.0",
        "?v < ?s",
        "?v + 1 < 0",
        "?v < -1 || ?v > 1",
        "!(?v < 1)",
        "?v < STRLEN('x')",
    ] {
        assert!(
            SignedDisclosureQuery::parse(&query(filter)).is_err(),
            "{filter}"
        );
    }
    assert!(SignedDisclosureQuery::parse(
        "SELECT DISTINCT ?s WHERE { { ?s <urn:p> ?x FILTER(?v < 0) } ?s <urn:value> ?v }"
    )
    .is_err());
}
