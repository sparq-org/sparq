//! Bounded exhaustive model checks of multi-replica SPARQL-CRDT scenarios
//! (`sq-tag1q.7.4`). Every test explores ALL interleavings of origin
//! operations and delta deliveries within its bounds; the checker verifies
//! `CRDT-STATE-1` invariants at each configuration, the `CRDT-SEC-2`
//! convergence equalities plus duplicate tolerance at each terminal, and the
//! join laws over the reached deltas/states. Directed assertions on
//! `terminal_visible_sets` pin the *outcome space* the proposal describes
//! (add-wins, origin-snapshot pattern compilation). Bounded evidence only —
//! not a convergence proof.

use std::collections::BTreeSet;

use sparq_crdt::{check_convergence, GraphKey, Op, Quad, Scenario};

fn q(s: u8, p: u8) -> Quad {
    Quad::new(s, p, 0, GraphKey::Default)
}

#[test]
fn concurrent_add_vs_observed_remove_is_add_wins() {
    // Setup: q visible everywhere. Replica 0 deletes it; replica 1 concurrently
    // re-inserts it. Exhaustive outcome space: if the delete raced ahead of the
    // insert delta, the fresh dot survives (add-wins, CRDT-MUT-2); if the origin
    // observed the insert first, its delete covers both dots. Both branches
    // converge; nothing else is reachable.
    let report = check_convergence(&Scenario {
        setup: vec![Op::InsertData(vec![q(1, 1)])],
        scripts: vec![
            vec![Op::DeleteData(vec![q(1, 1)])],
            vec![Op::InsertData(vec![q(1, 1)])],
        ],
    })
    .unwrap();
    assert_eq!(
        report.terminal_visible_sets,
        BTreeSet::from([BTreeSet::new(), BTreeSet::from([q(1, 1)])])
    );
    assert!(report.terminal_configs >= 2);
    assert!(report.laws.associativity_triples > 0);
}

#[test]
fn pattern_update_compiles_against_origin_snapshot_only() {
    // The CRDT-UPD-WHERE-3 worked example: replica 0 runs a pattern delete over
    // predicate 5; replica 1 concurrently inserts another matching quad. When
    // the pattern op raced the insert, the unobserved concurrent add survives —
    // the delete is never broadened to newly matching quads at receivers.
    let report = check_convergence(&Scenario {
        setup: vec![Op::InsertData(vec![q(1, 5)])],
        scripts: vec![
            vec![Op::DeleteWherePredicate(5)],
            vec![Op::InsertData(vec![q(2, 5)])],
        ],
    })
    .unwrap();
    assert_eq!(
        report.terminal_visible_sets,
        BTreeSet::from([BTreeSet::new(), BTreeSet::from([q(2, 5)])])
    );
}

#[test]
fn named_graph_membership_is_part_of_quad_identity() {
    // The same triple lives in the default graph and a named graph; deleting
    // the named-graph element never affects the default-graph one
    // (CRDT-DATA-1), in any interleaving.
    let dg = Quad::new(1, 2, 3, GraphKey::Default);
    let ng = Quad::new(1, 2, 3, GraphKey::Named(1));
    let report = check_convergence(&Scenario {
        setup: vec![Op::InsertData(vec![dg, ng])],
        scripts: vec![vec![Op::DeleteData(vec![ng])], vec![]],
    })
    .unwrap();
    assert_eq!(
        report.terminal_visible_sets,
        BTreeSet::from([BTreeSet::from([dg])])
    );
}

#[test]
fn three_replicas_with_refresh_delete_and_unrelated_insert() {
    // Replica 0 refreshes q1 in one atomic op (CRDT-MUT-3: old dots removed,
    // fresh dot survives its own removal context); replica 1 deletes q1;
    // replica 2 inserts an unrelated quad. The unrelated insert must survive
    // in every terminal outcome; q1's fate depends on which dots each delete
    // observed, but every branch converges and every law holds.
    let report = check_convergence(&Scenario {
        setup: vec![Op::InsertData(vec![q(1, 1)])],
        scripts: vec![
            vec![Op::DeleteInsertWhere {
                delete: vec![q(1, 1)],
                insert: vec![q(1, 1)],
            }],
            vec![Op::DeleteData(vec![q(1, 1)])],
            vec![Op::InsertData(vec![q(2, 2)])],
        ],
    })
    .unwrap();
    assert!(report.terminal_configs >= 1);
    assert!(report.distinct_deltas >= 3);
    assert!(!report.terminal_visible_sets.is_empty());
    for visible in &report.terminal_visible_sets {
        assert!(
            visible.contains(&q(2, 2)),
            "unrelated concurrent insert must survive in every outcome"
        );
    }
}

#[test]
fn multi_operation_request_interleaves_with_a_concurrent_insert() {
    // Replica 0 runs a two-operation request (insert then delete of the same
    // quad — the later operation observes the earlier effect, CRDT-UPD-BATCH-1);
    // replica 1 concurrently inserts the same quad. Outcomes: only replica 1's
    // dot can remain visible, and only when replica 0's delete did not observe
    // it. All interleavings converge.
    let report = check_convergence(&Scenario {
        setup: vec![],
        scripts: vec![
            vec![Op::InsertData(vec![q(3, 3)]), Op::DeleteData(vec![q(3, 3)])],
            vec![Op::InsertData(vec![q(3, 3)])],
        ],
    })
    .unwrap();
    assert_eq!(
        report.terminal_visible_sets,
        BTreeSet::from([BTreeSet::new(), BTreeSet::from([q(3, 3)])])
    );
    // The exploration is genuinely bounded and exhausted (never capped).
    assert!(report.configs_explored > report.terminal_configs);
}
