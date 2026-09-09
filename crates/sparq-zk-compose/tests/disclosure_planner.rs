//! [GPT-6] Host-plan regressions; these tests do not run or verify ZK proofs.

use oxrdf::{BlankNode, Literal, NamedNode, Term, Triple};
use sparq_zk::commit::{commit_triples, GraphCommitment};
use sparq_zk::field::Fr;
use sparq_zk_compose::manifest::FilterOp;
use sparq_zk_compose::planner::{
    plan_disclosure, DisclosureQuery, FilterObligation, MembershipRef, PlanError, PlannerLimits,
    QueryKind, QuerySlot, SlotRef,
};
use std::collections::BTreeMap;

fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{local}")).unwrap()
}

fn integer(value: &str) -> Term {
    Literal::new_typed_literal(
        value,
        NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
    )
    .into()
}

fn triple(subject: &str, predicate: &str, object: Term) -> Triple {
    Triple::new(iri(subject), iri(predicate), object)
}

fn credential(triples: &[Triple], salt: u64) -> GraphCommitment {
    commit_triples(triples, Fr::from(salt)).unwrap()
}

fn query(body: &str) -> DisclosureQuery {
    DisclosureQuery::parse(&format!("PREFIX ex: <http://example.org/> {body}")).unwrap()
}

fn row(bindings: &[(&str, Term)]) -> BTreeMap<String, Term> {
    bindings
        .iter()
        .map(|(variable, term)| (variable.to_string(), term.clone()))
        .collect()
}

#[test]
fn public_triple_and_filter_retain_membership_and_authentication() {
    let q = query("SELECT DISTINCT ?s ?age WHERE { ?s ex:age ?age FILTER(?age >= 18) }");
    let graph = credential(&[triple("alice", "age", integer("25"))], 1);
    let released = row(&[("s", iri("alice").into()), ("age", integer("25"))]);
    let plan = plan_disclosure(&q, &[graph], &[released], PlannerLimits::default()).unwrap();
    assert_eq!(plan.metrics.disclosed_triples, 1);
    assert_eq!(plan.metrics.public_filter_checks, 1);
    assert_eq!(plan.metrics.hidden_filter_obligations, 0);
    assert_eq!(
        plan.memberships,
        vec![MembershipRef {
            credential: 0,
            leaf: 0
        }]
    );
    assert_eq!(plan.authentication, vec![0]);
    assert!(matches!(
        plan.rows[0].filters.as_slice(),
        [FilterObligation::PublicCheck { filter: 0 }]
    ));
    assert_eq!(
        plan.rows[0].disclosed_triples[0],
        Some(triple("alice", "age", integer("25")))
    );
}

#[test]
fn hidden_salary_is_never_grounded_from_projected_person() {
    let q = query("SELECT DISTINCT ?s WHERE { ?s ex:salary ?salary FILTER(?salary > 50) }");
    let graph = credential(&[triple("alice", "salary", integer("75"))], 1);
    let plan = plan_disclosure(
        &q,
        &[graph],
        &[row(&[("s", iri("alice").into())])],
        PlannerLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.rows[0].disclosed_triples, vec![None]);
    assert_eq!(plan.rows[0].public_slots[0][0], Some(iri("alice").into()));
    assert_eq!(plan.rows[0].public_slots[0][1], Some(iri("salary").into()));
    assert_eq!(plan.rows[0].public_slots[0][2], None);
    assert_eq!(
        plan.rows[0].filters,
        vec![FilterObligation::HiddenPredicate {
            filter: 0,
            operand: SlotRef {
                pattern: 0,
                slot: 2
            }
        }]
    );
    assert!(!plan.explanation().contains("75"));
    assert!(!plan.explanation().contains("alice"));
}

#[test]
fn selects_successful_witnesses_without_proving_failing_scan_rows() {
    let q = query("SELECT DISTINCT ?s WHERE { ?s ex:age ?age FILTER(?age >= 18) }");
    let graph = credential(
        &[
            triple("alice", "age", integer("12")),
            triple("alice", "age", integer("25")),
            triple("bob", "age", integer("9")),
        ],
        1,
    );
    let good_leaf = graph
        .leaf_index(&triple("alice", "age", integer("25")))
        .unwrap();
    let plan = plan_disclosure(
        &q,
        &[graph],
        &[row(&[("s", iri("alice").into())])],
        PlannerLimits::default(),
    )
    .unwrap();
    assert_eq!(
        plan.rows[0].witnesses,
        vec![MembershipRef {
            credential: 0,
            leaf: good_leaf
        }]
    );
    assert_eq!(plan.metrics.membership_occurrences, 1);
}

#[test]
fn backtracks_across_join_candidates_instead_of_stitching_incompatible_rows() {
    let q = query("SELECT DISTINCT ?s WHERE { ?s ex:employer ?company . ?company ex:approved ?flag FILTER(?flag = 1) }");
    let graph = credential(
        &[
            triple("alice", "employer", iri("a-unapproved").into()),
            triple("alice", "employer", iri("z-approved").into()),
            triple("a-unapproved", "approved", integer("0")),
            triple("z-approved", "approved", integer("1")),
        ],
        1,
    );
    let expected = graph
        .leaf_index(&triple("alice", "employer", iri("z-approved").into()))
        .unwrap();
    let plan = plan_disclosure(
        &q,
        &[graph],
        &[row(&[("s", iri("alice").into())])],
        PlannerLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.rows[0].witnesses[0].leaf, expected);
    assert_eq!(plan.rows[0].identities.len(), 1);
    assert_eq!(plan.rows[0].identities[0].variable, "company");
}

#[test]
fn repeated_variable_within_triple_has_identity_obligation() {
    let q = query("SELECT DISTINCT ?p WHERE { ?x ?p ?x }");
    let bad = credential(&[triple("alice", "knows", iri("bob").into())], 1);
    let released = row(&[("p", iri("knows").into())]);
    assert!(matches!(
        plan_disclosure(
            &q,
            &[bad],
            std::slice::from_ref(&released),
            PlannerLimits::default()
        ),
        Err(PlanError::NoWitness { row: 0 })
    ));
    let good = credential(&[triple("alice", "knows", iri("alice").into())], 1);
    let plan = plan_disclosure(&q, &[good], &[released], PlannerLimits::default()).unwrap();
    assert_eq!(
        plan.rows[0].identities[0].from,
        SlotRef {
            pattern: 0,
            slot: 0
        }
    );
    assert_eq!(
        plan.rows[0].identities[0].to,
        SlotRef {
            pattern: 0,
            slot: 2
        }
    );
}

#[test]
fn repeated_memberships_share_once_and_irrelevant_credentials_are_pruned() {
    let q =
        query("SELECT DISTINCT ?s WHERE { ?s ex:age ?age . ?s ex:age ?age FILTER(?age >= 18) }");
    let irrelevant = credential(&[triple("noise", "other", integer("10"))], 1);
    let used = credential(
        &[
            triple("alice", "age", integer("25")),
            triple("bob", "age", integer("30")),
        ],
        2,
    );
    let plan = plan_disclosure(
        &q,
        &[irrelevant, used],
        &[
            row(&[("s", iri("alice").into())]),
            row(&[("s", iri("bob").into())]),
        ],
        PlannerLimits::default(),
    )
    .unwrap();
    // A parser may retain duplicate triple patterns; sharing remains explicit.
    assert_eq!(q.patterns.len(), 2);
    assert_eq!(plan.metrics.membership_occurrences, 4);
    assert_eq!(plan.metrics.unique_memberships, 2);
    assert_eq!(plan.authentication, vec![1]);
    assert_eq!(plan.metrics.unused_credentials, 1);
    assert_eq!(plan.rows[0].identities.len(), 1);
}

#[test]
fn a_common_membership_is_shared_across_different_released_results() {
    let q = query("SELECT DISTINCT ?s WHERE { ?s ex:employer ?c . ?c ex:approved 1 }");
    let graph = credential(
        &[
            triple("alice", "employer", iri("company").into()),
            triple("bob", "employer", iri("company").into()),
            triple("company", "approved", integer("1")),
        ],
        1,
    );
    let plan = plan_disclosure(
        &q,
        &[graph],
        &[
            row(&[("s", iri("alice").into())]),
            row(&[("s", iri("bob").into())]),
        ],
        PlannerLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.metrics.membership_occurrences, 4);
    assert_eq!(plan.metrics.unique_memberships, 3);
    assert_eq!(plan.rows[0].witnesses[1], plan.rows[1].witnesses[1]);
}

#[test]
fn graph_scoped_blank_nodes_never_join_across_credentials() {
    let q = query("SELECT DISTINCT ?s WHERE { ?s ex:link ?x . ?x ex:age ?age FILTER(?age >= 18) }");
    let bnode = BlankNode::new("same-input-label").unwrap();
    let first = credential(&[triple("alice", "link", bnode.clone().into())], 1);
    let second = credential(&[Triple::new(bnode.clone(), iri("age"), integer("25"))], 2);
    let released = row(&[("s", iri("alice").into())]);
    assert!(matches!(
        plan_disclosure(
            &q,
            &[first, second],
            std::slice::from_ref(&released),
            PlannerLimits::default()
        ),
        Err(PlanError::NoWitness { .. })
    ));
    let together = credential(
        &[
            triple("alice", "link", bnode.clone().into()),
            Triple::new(bnode, iri("age"), integer("25")),
        ],
        1,
    );
    let plan = plan_disclosure(&q, &[together], &[released], PlannerLimits::default()).unwrap();
    assert_eq!(plan.rows[0].identities.len(), 1);
    assert!(plan.rows[0].disclosed_triples.iter().all(Option::is_none));
}

#[test]
fn same_iri_can_join_across_independent_credentials() {
    let q = query("SELECT DISTINCT ?s WHERE { ?s ex:employer ?x . ?x ex:approved 1 }");
    let first = credential(&[triple("alice", "employer", iri("company").into())], 1);
    let second = credential(&[triple("company", "approved", integer("1"))], 2);
    let plan = plan_disclosure(
        &q,
        &[first, second],
        &[row(&[("s", iri("alice").into())])],
        PlannerLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.authentication, vec![0, 1]);
    assert_eq!(plan.rows[0].identities.len(), 1);
}

#[test]
fn release_domain_duplicates_and_unwitnessed_rows_fail_closed() {
    let q = query("SELECT DISTINCT ?s WHERE { ?s ex:age ?age }");
    let graphs = [credential(&[triple("alice", "age", integer("25"))], 1)];
    let alice = row(&[("s", iri("alice").into())]);
    for malformed in [
        row(&[]),
        row(&[("s", iri("alice").into()), ("age", integer("25"))]),
    ] {
        assert!(matches!(
            plan_disclosure(&q, &graphs, &[malformed], PlannerLimits::default()),
            Err(PlanError::InvalidRelease { .. })
        ));
    }
    assert!(matches!(
        plan_disclosure(
            &q,
            &graphs,
            &[alice.clone(), alice],
            PlannerLimits::default()
        ),
        Err(PlanError::DuplicateRelease {
            first: 0,
            second: 1
        })
    ));
    assert!(matches!(
        plan_disclosure(
            &q,
            &graphs,
            &[row(&[("s", iri("bob").into())])],
            PlannerLimits::default()
        ),
        Err(PlanError::NoWitness { row: 0 })
    ));
}

#[test]
fn selected_set_is_exact_without_claiming_wallet_answer_completeness() {
    let q = query("SELECT DISTINCT ?s WHERE { ?s ex:age ?age }");
    let graphs = [credential(
        &[
            triple("alice", "age", integer("25")),
            triple("bob", "age", integer("30")),
        ],
        1,
    )];
    let plan = plan_disclosure(
        &q,
        &graphs,
        &[row(&[("s", iri("alice").into())])],
        PlannerLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.rows.len(), 1);
    assert_eq!(plan.metrics.unique_memberships, 1);
    let empty = plan_disclosure(&q, &graphs, &[], PlannerLimits::default()).unwrap();
    assert!(empty.rows.is_empty());
    assert!(empty.authentication.is_empty());
}

#[test]
fn projected_blank_node_is_rejected_instead_of_assigning_global_identity() {
    let q = query("SELECT DISTINCT ?s WHERE { ?s ex:age ?age }");
    let blank = BlankNode::new("c14n0").unwrap();
    let graphs = [credential(
        &[Triple::new(blank.clone(), iri("age"), integer("25"))],
        1,
    )];
    assert!(matches!(
        plan_disclosure(
            &q,
            &graphs,
            &[row(&[("s", blank.into())])],
            PlannerLimits::default()
        ),
        Err(PlanError::InvalidRelease { .. })
    ));
}

#[test]
fn term_identity_is_not_numeric_value_equality() {
    let q = query("SELECT DISTINCT ?age WHERE { ex:alice ex:age ?age }");
    let graphs = [credential(
        &[
            triple("alice", "age", integer("1")),
            triple("alice", "age", integer("01")),
        ],
        1,
    )];
    let plan = plan_disclosure(
        &q,
        &graphs,
        &[
            row(&[("age", integer("1"))]),
            row(&[("age", integer("01"))]),
        ],
        PlannerLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.rows.len(), 2);
    assert_eq!(plan.metrics.unique_memberships, 2);
    let filtered = query("SELECT DISTINCT ?age WHERE { ex:alice ex:age ?age FILTER(?age = 1) }");
    assert!(matches!(
        plan_disclosure(
            &filtered,
            &graphs,
            &[row(&[("age", integer("01"))])],
            PlannerLimits::default()
        ),
        Err(PlanError::NoWitness { .. })
    ));
}

#[test]
fn accepts_true_ask_but_never_false_ask() {
    let q = query("ASK { ex:alice ex:age ?age FILTER(?age >= 18) }");
    assert_eq!(q.kind, QueryKind::Ask);
    let graphs = [credential(&[triple("alice", "age", integer("25"))], 1)];
    let plan = plan_disclosure(&q, &graphs, &[row(&[])], PlannerLimits::default()).unwrap();
    assert_eq!(plan.metrics.hidden_filter_obligations, 1);
    assert!(matches!(
        plan_disclosure(&q, &graphs, &[], PlannerLimits::default()),
        Err(PlanError::InvalidRelease { .. })
    ));
}

#[test]
fn parses_prefixes_comments_reversed_bounds_and_variable_ids() {
    let q = query("SELECT DISTINCT ?s ?p WHERE { # LIMIT UNION words in comments are harmless\n ?s ?p ?age FILTER(18 <= ?age && ?age != 99) }");
    assert_eq!(q.variables(), vec!["s", "p", "age"]);
    assert_eq!(q.filters[0].op, FilterOp::Ge);
    assert_eq!(q.filters[1].op, FilterOp::Ne);
    assert_eq!(q.projection, vec!["s", "p"]);
    assert_eq!(q.patterns[0][1], QuerySlot::Variable("p".into()));
}

#[test]
fn rejects_unsupported_query_operators_and_modifiers_via_ast() {
    let queries = [
        "SELECT ?s WHERE { ?s ex:p ?o }",
        "SELECT REDUCED ?s WHERE { ?s ex:p ?o }",
        "SELECT DISTINCT ?s WHERE { ?s ex:p ?o } LIMIT 1",
        "SELECT DISTINCT ?s WHERE { ?s ex:p ?o } OFFSET 0",
        "SELECT DISTINCT ?s WHERE { ?s ex:p ?o } ORDER BY ?s",
        "SELECT DISTINCT ?s FROM ex:g WHERE { ?s ex:p ?o }",
        "SELECT DISTINCT ?s WHERE { ?s ex:p ?o OPTIONAL { ?s ex:q ?v } }",
        "SELECT DISTINCT ?s WHERE { { ?s ex:p ?o } UNION { ?s ex:q ?o } }",
        "SELECT DISTINCT ?s WHERE { ?s ex:p ?o MINUS { ?s ex:q ?o } }",
        "SELECT DISTINCT ?s WHERE { GRAPH ex:g { ?s ex:p ?o } }",
        "SELECT DISTINCT ?s WHERE { SERVICE ex:g { ?s ex:p ?o } }",
        "SELECT DISTINCT ?s WHERE { ?s ex:p+ ?o }",
        "SELECT DISTINCT ?s WHERE { ?s ex:p ?o VALUES ?o { 1 } }",
        "SELECT DISTINCT ?s WHERE { ?s ex:p ?o BIND(?o AS ?x) }",
        "SELECT DISTINCT (?s AS ?x) WHERE { ?s ex:p ?o }",
        "SELECT DISTINCT (COUNT(?s) AS ?c) WHERE { ?s ex:p ?o }",
        "SELECT DISTINCT ?s WHERE { { SELECT ?s WHERE { ?s ex:p ?o } } }",
        "SELECT DISTINCT ?s WHERE { ?s ex:p ?o FILTER(EXISTS { ?s ex:q 1 }) }",
        "SELECT DISTINCT ?s WHERE { ?s ex:p ?o FILTER(?o > 1 || ?o < 5) }",
        "SELECT DISTINCT ?s WHERE { ?s ex:p ?o FILTER(?o + 1 > 5) }",
        "SELECT DISTINCT ?s WHERE { ?s ex:p ?o FILTER(?o > -1) }",
        "SELECT DISTINCT ?s WHERE { ?s ex:p ?o FILTER(?o = \"1\") }",
        "SELECT DISTINCT ?s WHERE { ?s ex:p ?o FILTER(?o = 1.0) }",
        "SELECT DISTINCT ?missing WHERE { ?s ex:p ?o }",
        "SELECT DISTINCT ?s WHERE { ?s ex:p _:hidden }",
        "SELECT DISTINCT * WHERE {}",
        "ASK { ?s ex:p ?o } LIMIT 1",
    ];
    for text in queries {
        assert!(
            DisclosureQuery::parse(&format!("PREFIX ex: <http://example.org/> {text}")).is_err(),
            "unexpectedly admitted {text}"
        );
    }
}

#[test]
fn rejects_filter_variable_bound_only_in_sibling_join() {
    let text = "PREFIX ex: <http://example.org/> SELECT DISTINCT ?s WHERE { { ?s ex:name ?n FILTER(?age > 18) } ?s ex:age ?age }";
    assert!(
        matches!(DisclosureQuery::parse(text), Err(PlanError::Unsupported(reason)) if reason.contains("scope"))
    );
}

#[test]
fn bounded_search_rejects_without_returning_partial_plan() {
    let q = query("SELECT DISTINCT ?s WHERE { ?s ex:age ?age }");
    let graphs = [credential(&[triple("alice", "age", integer("25"))], 1)];
    let released = [row(&[("s", iri("alice").into())])];
    for limits in [
        PlannerLimits {
            max_search_steps: 0,
            ..Default::default()
        },
        PlannerLimits {
            max_patterns: 0,
            ..Default::default()
        },
        PlannerLimits {
            max_results: 0,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            plan_disclosure(&q, &graphs, &released, limits),
            Err(PlanError::LimitExceeded(_))
        ));
    }
}

#[test]
fn local_plan_is_deterministic_for_identical_inputs() {
    let q = query("SELECT DISTINCT ?s WHERE { ?s ex:age ?age FILTER(?age >= 18) }");
    let graphs = [credential(
        &[
            triple("alice", "age", integer("25")),
            triple("alice", "age", integer("35")),
        ],
        1,
    )];
    let released = [row(&[("s", iri("alice").into())])];
    let a = plan_disclosure(&q, &graphs, &released, PlannerLimits::default()).unwrap();
    let b = plan_disclosure(&q, &graphs, &released, PlannerLimits::default()).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.explanation(), b.explanation());
}

#[test]
fn admitted_candidates_preserve_wallet_indices_and_query_checks() {
    let q = query("SELECT DISTINCT ?s WHERE { ?s ex:age ?age FILTER(?age >= 18) }");
    let graph = credential(&[triple("alice", "age", integer("42"))], 1);
    let graphs = [graph.clone(), graph];
    let rows = [row(&[("s", iri("alice").into())])];
    let plan = sparq_zk_compose::planner::plan_disclosure_admitted(
        &q, &graphs, &rows, PlannerLimits::default(), |_, witness, _| witness.credential == 1,
    ).unwrap();
    assert_eq!(plan.authentication, vec![1]);
    assert!(sparq_zk_compose::planner::plan_disclosure_admitted(
        &q, &graphs, &rows, PlannerLimits::default(), |_, _, _| false,
    ).is_err());
}
