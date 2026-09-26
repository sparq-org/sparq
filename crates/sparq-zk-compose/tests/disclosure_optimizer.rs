//! [GPT-6] Structural objective tests; no prover latency or security claim.

use oxrdf::{BlankNode, Literal, NamedNode, Term, Triple};
use sparq_zk::commit::{commit_triples, GraphCommitment};
use sparq_zk::field::Fr;
use sparq_zk_compose::planner::{
    optimize_disclosure, optimize_disclosure_admitted, plan_disclosure, DisclosureQuery,
    MembershipRef, OptimizationCompletion, OptimizationLimits, PlanError, PlanObjective,
    PlannerLimits,
};
use std::collections::{BTreeMap, BTreeSet};

fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{local}")).unwrap()
}

fn one() -> Term {
    Literal::new_typed_literal(
        "1",
        NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
    )
    .into()
}

fn link(subject: &str, company: &str) -> Triple {
    Triple::new(iri(subject), iri("link"), iri(company))
}

fn approved(company: &str) -> Triple {
    Triple::new(iri(company), iri("ok"), one())
}

fn graph(triples: &[Triple], salt: u64) -> GraphCommitment {
    commit_triples(triples, Fr::from(salt)).unwrap()
}

fn query() -> DisclosureQuery {
    DisclosureQuery::parse(
        "PREFIX ex: <http://example.org/> SELECT DISTINCT ?s WHERE { ?s ex:link ?x . ?x ex:ok 1 }",
    )
    .unwrap()
}

fn released() -> Vec<BTreeMap<String, Term>> {
    ["alice", "bob"]
        .iter()
        .map(|s| BTreeMap::from([("s".into(), iri(s).into())]))
        .collect()
}

#[test]
fn joint_search_finds_one_shared_credential_where_first_success_uses_two() {
    let q = query();
    let graphs = [
        graph(&[link("alice", "a"), approved("a")], 1),
        graph(&[link("bob", "b"), approved("b")], 2),
        graph(&[link("alice", "c"), link("bob", "c"), approved("c")], 3),
    ];
    let rows = released();
    let baseline = plan_disclosure(&q, &graphs, &rows, PlannerLimits::default()).unwrap();
    assert_eq!(baseline.authentication, vec![0, 1]);
    let optimized = optimize_disclosure(&q, &graphs, &rows, OptimizationLimits::default()).unwrap();
    assert_eq!(optimized.completion, OptimizationCompletion::Optimal);
    assert_eq!(
        optimized.objective(),
        Some(PlanObjective {
            authentications: 1,
            memberships: 3
        })
    );
    let plan = optimized.plan.unwrap();
    assert_eq!(plan.authentication, vec![2]);
    assert_eq!(
        plan.rows
            .iter()
            .map(|row| row.released.clone())
            .collect::<Vec<_>>(),
        rows
    );
    assert_eq!(plan.rows[0].public_slots, baseline.rows[0].public_slots);
    assert_eq!(plan.rows[0].identities, baseline.rows[0].identities);
    assert_eq!(optimized.stats.input_credentials, 3);
    assert_eq!(optimized.stats.input_patterns, 2);
    assert_eq!(optimized.stats.released_rows, 2);
    assert_eq!(optimized.stats.pattern_occurrences, 4);
}

#[test]
fn authentication_has_priority_over_membership_count() {
    let graphs = [
        graph(&[link("alice", "common"), approved("common")], 1),
        graph(&[link("bob", "common")], 2),
        graph(
            &[
                link("alice", "a"),
                approved("a"),
                link("bob", "b"),
                approved("b"),
            ],
            3,
        ),
    ];
    let baseline =
        plan_disclosure(&query(), &graphs, &released(), PlannerLimits::default()).unwrap();
    assert_eq!(baseline.metrics.authentication_obligations, 2);
    assert_eq!(baseline.metrics.unique_memberships, 3);
    let optimized = optimize_disclosure(
        &query(),
        &graphs,
        &released(),
        OptimizationLimits::default(),
    )
    .unwrap();
    assert_eq!(
        optimized.objective(),
        Some(PlanObjective {
            authentications: 1,
            memberships: 4
        })
    );
    assert_eq!(optimized.plan.unwrap().authentication, vec![2]);
}

#[test]
fn sharing_memberships_is_secondary_to_authentication() {
    let graphs = [graph(
        &[
            link("alice", "a"),
            approved("a"),
            link("bob", "b"),
            approved("b"),
            link("alice", "z-shared"),
            link("bob", "z-shared"),
            approved("z-shared"),
        ],
        1,
    )];
    let baseline =
        plan_disclosure(&query(), &graphs, &released(), PlannerLimits::default()).unwrap();
    assert_eq!(baseline.metrics.unique_memberships, 4);
    let optimized = optimize_disclosure(
        &query(),
        &graphs,
        &released(),
        OptimizationLimits::default(),
    )
    .unwrap();
    assert_eq!(
        optimized.objective(),
        Some(PlanObjective {
            authentications: 1,
            memberships: 3
        })
    );
    let plan = optimized.plan.unwrap();
    assert_eq!(plan.rows[0].witnesses[1], plan.rows[1].witnesses[1]);
}

#[test]
fn original_backend_admission_cannot_be_bypassed_by_optimization() {
    let graphs = [
        graph(&[link("alice", "a"), approved("a")], 1),
        graph(&[link("bob", "b"), approved("b")], 2),
        graph(&[link("alice", "c"), link("bob", "c"), approved("c")], 3),
    ];
    let optimized = optimize_disclosure_admitted(
        &query(),
        &graphs,
        &released(),
        OptimizationLimits::default(),
        |pattern, witness, triple| {
            assert_eq!(
                triple,
                &graphs[witness.credential].canonical.triples[witness.leaf]
            );
            assert!(pattern < 2);
            witness.credential != 2
        },
    )
    .unwrap();
    assert_eq!(optimized.completion, OptimizationCompletion::Optimal);
    assert_eq!(optimized.plan.unwrap().authentication, vec![0, 1]);
    let impossible = optimize_disclosure_admitted(
        &query(),
        &graphs,
        &released(),
        OptimizationLimits::default(),
        |_, _, _| false,
    )
    .unwrap();
    assert_eq!(impossible.completion, OptimizationCompletion::Infeasible);
    assert!(impossible.plan.is_none());
    assert!(impossible.stats.candidate_steps > 0);
}

#[test]
fn budget_exhaustion_distinguishes_no_solution_yet_from_best_feasible() {
    let q = DisclosureQuery::parse(
        "PREFIX ex: <http://example.org/> SELECT DISTINCT ?s WHERE { ?s ex:ok 1 }",
    )
    .unwrap();
    let graphs = [graph(&[approved("alice"), approved("bob")], 1)];
    let rows = vec![BTreeMap::from([("s".into(), iri("alice").into())])];
    let limits = |steps| OptimizationLimits {
        planner: PlannerLimits {
            max_search_steps: steps,
            ..Default::default()
        },
        ..Default::default()
    };
    let none = optimize_disclosure(&q, &graphs, &rows, limits(0)).unwrap();
    assert_eq!(none.completion, OptimizationCompletion::BudgetExhausted);
    assert!(none.plan.is_none());
    assert_eq!(none.stats.candidate_steps, 0);
    let some = optimize_disclosure(&q, &graphs, &rows, limits(1)).unwrap();
    assert_eq!(some.completion, OptimizationCompletion::BudgetExhausted);
    assert_eq!(
        some.objective(),
        Some(PlanObjective {
            authentications: 1,
            memberships: 1
        })
    );
    assert_eq!(some.plan.unwrap().rows[0].released, rows[0]);
    assert_eq!(some.stats.candidate_steps, 1);
    let exact = optimize_disclosure(&q, &graphs, &rows, limits(2)).unwrap();
    assert_eq!(exact.completion, OptimizationCompletion::Optimal);
    assert_eq!(exact.stats.candidate_steps, 2);
}

#[test]
fn exhaustion_never_returns_a_plan_with_only_some_requested_rows() {
    let q = DisclosureQuery::parse(
        "PREFIX ex: <http://example.org/> SELECT DISTINCT ?s WHERE { ?s ex:ok 1 }",
    )
    .unwrap();
    let graphs = [graph(&[approved("alice"), approved("bob")], 1)];
    let report = optimize_disclosure(
        &q,
        &graphs,
        &released(),
        OptimizationLimits {
            planner: PlannerLimits {
                max_search_steps: 1,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.completion, OptimizationCompletion::BudgetExhausted);
    assert!(report.plan.is_none());
}

#[test]
fn equal_objectives_choose_original_credential_then_canonical_leaf_order() {
    let triples = [
        link("alice", "a"),
        link("bob", "a"),
        approved("a"),
        link("alice", "b"),
        link("bob", "b"),
        approved("b"),
    ];
    let graphs = [graph(&triples, 1), graph(&triples, 2)];
    let a = optimize_disclosure(
        &query(),
        &graphs,
        &released(),
        OptimizationLimits::default(),
    )
    .unwrap();
    let b = optimize_disclosure(
        &query(),
        &graphs,
        &released(),
        OptimizationLimits::default(),
    )
    .unwrap();
    assert_eq!(a, b);
    let plan = a.plan.unwrap();
    assert_eq!(plan.authentication, vec![0]);
    let leaf = graphs[0].leaf_index(&link("alice", "a")).unwrap();
    assert_eq!(
        plan.rows[0].witnesses[0],
        MembershipRef {
            credential: 0,
            leaf
        }
    );
}

#[test]
fn preserved_filter_and_blank_node_scope_checks_reject_false_witnesses() {
    let q = DisclosureQuery::parse("PREFIX ex: <http://example.org/> SELECT DISTINCT ?s WHERE { ?s ex:link ?x . ?x ex:age ?age FILTER(?age > 1) }").unwrap();
    let blank = BlankNode::new("same-label").unwrap();
    let graphs = [
        graph(&[Triple::new(iri("alice"), iri("link"), blank.clone())], 1),
        graph(
            &[Triple::new(
                blank,
                iri("age"),
                Literal::new_typed_literal(
                    "5",
                    NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
                ),
            )],
            2,
        ),
        graph(
            &[
                link("alice", "iri-company"),
                Triple::new(iri("iri-company"), iri("age"), one()),
            ],
            3,
        ),
    ];
    let report =
        optimize_disclosure(&q, &graphs, &released()[..1], OptimizationLimits::default()).unwrap();
    assert_eq!(report.completion, OptimizationCompletion::Infeasible);
    assert!(report.plan.is_none());
}

#[test]
fn input_caps_precede_programmatic_shape_validation_and_bound_joint_depth() {
    let mut oversized = query();
    oversized.patterns.clear();
    for _ in 0..129 {
        oversized.patterns.push(query().patterns[0].clone());
    }
    let errors = [
        optimize_disclosure(
            &oversized,
            &[],
            &released()[..1],
            OptimizationLimits::default(),
        ),
        optimize_disclosure(
            &query(),
            &[],
            &released(),
            OptimizationLimits {
                max_pattern_occurrences: 3,
                ..Default::default()
            },
        ),
        optimize_disclosure(
            &query(),
            &[],
            &released(),
            OptimizationLimits {
                planner: PlannerLimits {
                    max_results: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
        ),
        optimize_disclosure(
            &oversized,
            &[],
            &released()[..1],
            OptimizationLimits {
                max_pattern_occurrences: usize::MAX,
                planner: PlannerLimits {
                    max_patterns: usize::MAX,
                    ..Default::default()
                },
                ..Default::default()
            },
        ),
    ];
    for error in errors {
        assert!(matches!(error, Err(PlanError::LimitExceeded(_))));
    }
}

#[test]
fn joint_backend_credential_capacity_prunes_infeasible_first_fit_assignments() {
    let graphs = [
        graph(&[link("alice", "a"), approved("a")], 1),
        graph(&[link("bob", "b"), approved("b")], 2),
        graph(&[link("alice", "c"), link("bob", "c"), approved("c")], 3),
    ];
    let result = optimize_disclosure(
        &query(),
        &graphs,
        &released(),
        OptimizationLimits {
            max_authentications: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.completion, OptimizationCompletion::Optimal);
    assert_eq!(result.stats.authentication_limit, 1);
    assert_eq!(result.plan.unwrap().authentication, vec![2]);
    let unavailable = optimize_disclosure_admitted(
        &query(),
        &graphs,
        &released(),
        OptimizationLimits {
            max_authentications: 1,
            ..Default::default()
        },
        |_, witness, _| witness.credential < 2,
    )
    .unwrap();
    assert_eq!(unavailable.completion, OptimizationCompletion::Infeasible);
    assert!(unavailable.plan.is_none());
}

#[test]
fn invalid_releases_are_errors_instead_of_search_infeasibility() {
    let q = query();
    let graphs = [graph(&[link("alice", "a"), approved("a")], 1)];
    assert!(matches!(
        optimize_disclosure(
            &q,
            &graphs,
            &[BTreeMap::new()],
            OptimizationLimits::default()
        ),
        Err(PlanError::InvalidRelease { .. })
    ));
    let row = released()[0].clone();
    assert!(matches!(
        optimize_disclosure(
            &q,
            &graphs,
            &[row.clone(), row],
            OptimizationLimits::default()
        ),
        Err(PlanError::DuplicateRelease { .. })
    ));
}

#[test]
fn empty_select_and_true_ask_keep_their_exact_contracts() {
    let empty = optimize_disclosure(&query(), &[], &[], OptimizationLimits::default()).unwrap();
    assert_eq!(empty.completion, OptimizationCompletion::Optimal);
    assert_eq!(
        empty.objective(),
        Some(PlanObjective {
            authentications: 0,
            memberships: 0
        })
    );
    let ask = DisclosureQuery::parse("PREFIX ex: <http://example.org/> ASK { ex:alice ex:ok 1 }")
        .unwrap();
    let graphs = [graph(&[approved("alice")], 1)];
    let yes = optimize_disclosure(
        &ask,
        &graphs,
        &[BTreeMap::new()],
        OptimizationLimits::default(),
    )
    .unwrap();
    assert_eq!(yes.completion, OptimizationCompletion::Optimal);
    assert_eq!(yes.plan.unwrap().rows.len(), 1);
    assert!(matches!(
        optimize_disclosure(&ask, &graphs, &[], OptimizationLimits::default()),
        Err(PlanError::InvalidRelease { .. })
    ));
}

#[test]
fn empty_credential_prefixes_have_an_input_cap_independent_of_candidate_fuel() {
    use sparq_zk_compose::planner::{plan_disclosure_admitted, MAX_DISCLOSURE_CREDENTIALS};
    let q = DisclosureQuery::parse(
        "PREFIX ex: <http://example.org/> SELECT DISTINCT ?s WHERE { ?s ex:ok 1 }",
    )
    .unwrap();
    let empty = graph(&[], 1);
    let mut graphs = vec![empty.clone(); MAX_DISCLOSURE_CREDENTIALS - 1];
    graphs.push(graph(&[approved("alice")], 2));
    let rows = &released()[..1];
    let planner_limits = PlannerLimits {
        max_search_steps: 1,
        ..Default::default()
    };
    let optimization_limits = OptimizationLimits {
        planner: planner_limits,
        ..Default::default()
    };
    let baseline = plan_disclosure(&q, &graphs, rows, planner_limits).unwrap();
    assert_eq!(
        baseline.authentication,
        vec![MAX_DISCLOSURE_CREDENTIALS - 1]
    );
    let optimized = optimize_disclosure(&q, &graphs, rows, optimization_limits).unwrap();
    assert_eq!(optimized.completion, OptimizationCompletion::Optimal);
    assert_eq!(optimized.stats.candidate_steps, 1);
    assert_eq!(
        optimized.stats.input_credentials,
        MAX_DISCLOSURE_CREDENTIALS
    );
    assert_eq!(optimized.stats.input_triples, 1);

    graphs.insert(0, empty);
    let no_search = PlannerLimits {
        max_search_steps: 0,
        ..Default::default()
    };
    let expected = PlanError::LimitExceeded("input credentials");
    assert_eq!(
        plan_disclosure_admitted(&q, &graphs, rows, no_search, |_, _, _| panic!(
            "input cap must precede admission"
        )),
        Err(expected.clone())
    );
    assert_eq!(
        optimize_disclosure_admitted(
            &q,
            &graphs,
            rows,
            OptimizationLimits {
                planner: no_search,
                ..Default::default()
            },
            |_, _, _| panic!("input cap must precede admission")
        ),
        Err(expected.clone())
    );
    // Even an empty release cannot bypass the optimizer statistics prepass cap.
    assert_eq!(
        optimize_disclosure(&q, &graphs, &[], OptimizationLimits::default()),
        Err(expected)
    );
}

// Independent exhaustive oracle for the fixed two-pattern fixture query. It
// enumerates every tuple of canonical triples and only then evaluates the full
// row relation; it shares no planner matching, pruning, or assembly helpers.
fn exhaustive_oracle(graphs: &[GraphCommitment]) -> Option<(PlanObjective, Vec<MembershipRef>)> {
    let all: Vec<_> = graphs
        .iter()
        .enumerate()
        .flat_map(|(credential, graph)| {
            (0..graph.canonical.triples.len()).map(move |leaf| MembershipRef { credential, leaf })
        })
        .collect();
    let triple = |r: MembershipRef| &graphs[r.credential].canonical.triples[r.leaf];
    let matches_row = |name: &str, a: MembershipRef, b: MembershipRef| {
        let (first, second) = (triple(a), triple(b));
        first.subject == iri(name).into()
            && first.predicate == iri("link")
            && first.object == Term::from(second.subject.clone())
            && second.predicate == iri("ok")
            && second.object == one()
    };
    let mut best = None;
    for &a in &all {
        for &b in &all {
            for &c in &all {
                for &d in &all {
                    if !matches_row("alice", a, b) || !matches_row("bob", c, d) {
                        continue;
                    }
                    let assignment = vec![a, b, c, d];
                    let objective = PlanObjective {
                        authentications: assignment
                            .iter()
                            .map(|w| w.credential)
                            .collect::<BTreeSet<_>>()
                            .len(),
                        memberships: assignment.iter().collect::<BTreeSet<_>>().len(),
                    };
                    if best.as_ref().is_none_or(
                        |(cost, selected): &(PlanObjective, Vec<MembershipRef>)| {
                            (objective, &assignment) < (*cost, selected)
                        },
                    ) {
                        best = Some((objective, assignment));
                    }
                }
            }
        }
    }
    best
}

#[test]
fn optimized_objective_and_ties_match_exhaustive_oracle_across_small_datasets() {
    let choices = [
        link("alice", "a"),
        link("bob", "a"),
        approved("a"),
        link("alice", "b"),
        link("bob", "b"),
        approved("b"),
    ];
    // Every subset of this six-triple fixture is tested against its rotated
    // counterpart. This includes both feasible and infeasible complete spaces.
    for mask in 0_u8..64 {
        let first: Vec<_> = choices
            .iter()
            .enumerate()
            .filter(|(bit, _)| mask & (1 << bit) != 0)
            .map(|(_, t)| t.clone())
            .collect();
        let second: Vec<_> = choices
            .iter()
            .enumerate()
            .filter(|(bit, _)| mask.rotate_left(1) & (1 << bit) != 0)
            .map(|(_, t)| t.clone())
            .collect();
        let graphs = [graph(&first, 1), graph(&second, 2)];
        let oracle = exhaustive_oracle(&graphs);
        let report = optimize_disclosure(
            &query(),
            &graphs,
            &released(),
            OptimizationLimits::default(),
        )
        .unwrap();
        assert_ne!(
            report.completion,
            OptimizationCompletion::BudgetExhausted,
            "mask {mask}"
        );
        assert_eq!(
            report.objective(),
            oracle.as_ref().map(|(objective, _)| *objective),
            "mask {mask}"
        );
        match (report.plan, oracle) {
            (Some(plan), Some((_, assignment))) => {
                let actual: Vec<_> = plan
                    .rows
                    .iter()
                    .flat_map(|row| row.witnesses.clone())
                    .collect();
                assert_eq!(actual, assignment, "tie order differs for mask {mask}");
                assert_eq!(report.completion, OptimizationCompletion::Optimal);
            }
            (None, None) => assert_eq!(report.completion, OptimizationCompletion::Infeasible),
            _ => panic!("feasibility differs for mask {mask}"),
        }
    }
}
