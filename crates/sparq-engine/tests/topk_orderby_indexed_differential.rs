//! Differential + correctness tests for `try_topk_orderby_indexed` (the
//! index-ordered top-k seed path added for the artifact-keeper claim-queue
//! throughput investigation).
//!
//! `try_topk_orderby_indexed` is a pure SHORTCUT: it either returns exactly the
//! same rows the pre-existing `eval_modified`-then-`order_bindings` path would,
//! or declines (`Ok(None)`) and lets that path run. These tests never assume
//! which branch fired — they only assert the OBSERVABLE result is correct,
//! computed independently by fetching the full unordered relation (a query
//! shape the new path never touches) and sorting it in the test itself.

use sparq_core::Graph;
use sparq_engine::query;

const PFX: &str = "PREFIX ak: <http://example.org/ak#>\n";

/// One synthetic pending task: `(seq, priority)`. `seq` doubles as a stable,
/// unique task identifier so results can be checked by task number.
fn build_graph(peer: &str, tasks: &[(i64, i64)], extra_ttl: &str) -> Graph {
    let mut ttl = String::from("@prefix ak: <http://example.org/ak#> .\n");
    for &(seq, priority) in tasks {
        ttl.push_str(&format!(
            "<urn:task:{peer}:{seq}> ak:peer <urn:peer:{peer}> ; ak:status \"pending\" ; ak:priority {priority} ; ak:seq {seq} .\n"
        ));
    }
    ttl.push_str(extra_ttl);
    Graph::load_str(&ttl, "turtle").expect("load turtle")
}

const CLAIM_QUERY: &str = "
SELECT ?t WHERE {
  ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s .
}
ORDER BY DESC(?p) ASC(?s)
LIMIT ";

fn task_seq(t: &oxrdf::Term) -> i64 {
    match t {
        oxrdf::Term::NamedNode(n) => n
            .as_str()
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .expect("task IRI ends in :<seq>"),
        other => panic!("expected a task IRI, got {other:?}"),
    }
}

/// Ground truth: fetch every pending task's (seq, priority) via a query the new
/// path never activates on (no ORDER BY/LIMIT at all), sort in test code by the
/// exact same key (DESC priority, ASC seq), and return the expected task-seq
/// order for the first `k`.
fn expected_top_k(graph: &Graph, k: usize) -> Vec<i64> {
    let r = query(
        graph,
        &format!("{PFX} SELECT ?s ?p WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s . }}"),
    )
    .unwrap();
    let mut rows: Vec<(i64, i64)> = r
        .rows
        .iter()
        .map(|row| {
            let s = as_int(row[0].as_ref().unwrap());
            let p = as_int(row[1].as_ref().unwrap());
            (s, p)
        })
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0))); // DESC priority, ASC seq
    rows.into_iter().take(k).map(|(s, _)| s).collect()
}

fn as_int(t: &oxrdf::Term) -> i64 {
    match t {
        oxrdf::Term::Literal(l) => l.value().parse().unwrap(),
        other => panic!("expected an integer literal, got {other:?}"),
    }
}

fn actual_top_k(graph: &Graph, k: usize) -> Vec<i64> {
    let r = query(graph, &format!("{PFX}{CLAIM_QUERY}{k}")).unwrap();
    r.rows.iter().map(|row| task_seq(row[0].as_ref().unwrap())).collect()
}

fn check(tasks: &[(i64, i64)], ks: &[usize]) {
    let graph = build_graph("X", tasks, "");
    for &k in ks {
        let expected = expected_top_k(&graph, k);
        let actual = actual_top_k(&graph, k);
        assert_eq!(actual, expected, "k={k}, n={}", tasks.len());
    }
}

#[test]
fn unique_priorities_small() {
    // Deliberately unsorted insertion order.
    let tasks: Vec<(i64, i64)> = vec![(0, 5), (1, 20), (2, 1), (3, 15), (4, 9), (5, 30), (6, 0)];
    check(&tasks, &[1, 2, 3, 7, 100]);
}

#[test]
fn unique_priorities_pseudo_random_medium() {
    let n = 300;
    let tasks: Vec<(i64, i64)> = (0..n)
        .map(|i| (i as i64, ((i as i64) * 2654435761i64) % (n as i64 * 7)))
        .collect();
    check(&tasks, &[1, 5, 50, 299, 300, 400]);
}

#[test]
fn ties_broken_by_seq_ascending() {
    // Three groups of tied priorities; within a group, seq must break the tie
    // ascending regardless of insertion order.
    let tasks: Vec<(i64, i64)> = vec![
        (5, 10), (2, 10), (8, 10), // priority 10: expect order 2,5,8
        (1, 20), (0, 20),          // priority 20: expect order 0,1
        (9, 5),                    // priority 5: alone
    ];
    check(&tasks, &[1, 2, 3, 4, 5, 6, 100]);
}

#[test]
fn crosses_block_escalation_boundary() {
    // CAPPED_SEED_BLOCK is 1024; exercise a queue depth comfortably on both
    // sides of that boundary, including k values that force multiple blocks.
    let n = 2500;
    let tasks: Vec<(i64, i64)> = (0..n)
        .map(|i| (i as i64, ((i as i64) * 2654435761i64) % (n as i64 * 3)))
        .collect();
    check(&tasks, &[1, 1000, 1023, 1024, 1025, 2000, 2500, 3000]);
}

#[test]
fn multi_peer_isolation() {
    // Peer Y's tasks (higher priorities than any of X's) must never appear in
    // an X-scoped claim, and must not affect X's ordering.
    let mut ttl = String::new();
    for (seq, priority) in [(0i64, 999i64), (1, 998), (2, 997)] {
        ttl.push_str(&format!(
            "<urn:task:Y:{seq}> ak:peer <urn:peer:Y> ; ak:status \"pending\" ; ak:priority {priority} ; ak:seq {seq} .\n"
        ));
    }
    let x_tasks: Vec<(i64, i64)> = vec![(10, 1), (11, 5), (12, 3)];
    let graph = build_graph("X", &x_tasks, &ttl);
    let expected = expected_top_k(&graph, 10);
    let actual = actual_top_k(&graph, 10);
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 3, "peer Y's tasks must not leak into an X-scoped claim");
}

#[test]
fn status_filter_excludes_done_tasks() {
    // A `done` task with the highest priority must never be claimed — exercises
    // the fully-bound (`obj_const`) branch of the other-pattern join.
    let mut ttl = String::from(
        "<urn:task:X:999> ak:peer <urn:peer:X> ; ak:status \"done\" ; ak:priority 9999 ; ak:seq 999 .\n",
    );
    ttl.push_str("");
    let tasks: Vec<(i64, i64)> = vec![(0, 1), (1, 2), (2, 3)];
    let graph = build_graph("X", &tasks, &ttl);
    let actual = actual_top_k(&graph, 10);
    assert_eq!(actual, vec![2, 1, 0]); // DESC priority among the PENDING tasks only
}

#[test]
fn fewer_pending_than_k_returns_all_sorted() {
    let tasks: Vec<(i64, i64)> = vec![(0, 5), (1, 1), (2, 3)];
    check(&tasks, &[3, 4, 100, 1024]);
}

#[test]
fn empty_pending_queue() {
    let graph = build_graph("X", &[], "");
    let actual = actual_top_k(&graph, 5);
    assert!(actual.is_empty());
}

#[test]
fn ascending_order_direction() {
    let tasks: Vec<(i64, i64)> = vec![(0, 5), (1, 20), (2, 1), (3, 15)];
    let graph = build_graph("X", &tasks, "");
    let r = query(
        &graph,
        &format!("{PFX} SELECT ?t WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s . }} ORDER BY ASC(?p) LIMIT 2"),
    )
    .unwrap();
    let actual: Vec<i64> = r.rows.iter().map(|row| task_seq(row[0].as_ref().unwrap())).collect();
    assert_eq!(actual, vec![2, 0]); // priorities 1, 5 ascending
}

#[test]
fn non_inline_priority_values_still_correct() {
    // Priorities far outside the small-inline-integer range must still produce
    // correct results — this must exercise the DECLINE guard (dict::is_inline)
    // and fall back to the general path, not silently mis-sort.
    let tasks: Vec<(i64, i64)> = vec![
        (0, 9_000_000_000_000_000),
        (1, 1_000_000_000_000_000),
        (2, 5_000_000_000_000_000),
    ];
    check(&tasks, &[1, 2, 3]);
}

#[test]
fn large_tie_group_still_correct() {
    // Low priority cardinality (a handful of tiers over many tasks) forces a
    // large tie-group — the MAX_INDEXED_GROUP cap should decline the fast path
    // here and defer to the fallback. This test only asserts correctness (the
    // regression this cap fixes was a PERFORMANCE regression, not a correctness
    // one — verified separately via profiling, not asserted here to avoid a
    // flaky timing-based test).
    for tiers in [1usize, 2, 5, 16] {
        let n = 600;
        let tasks: Vec<(i64, i64)> = (0..n).map(|i| (i as i64, (i as i64) % tiers as i64)).collect();
        check(&tasks, &[1, 2, 10, 100, n as usize]);
    }
}

#[test]
fn randomized_sweep() {
    // A cheap xorshift so this test has no extra dependency and is fully
    // deterministic (fixed seed) across runs, while still covering a wide
    // spread of n / tie-density / k combinations in one pass.
    fn xorshift(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }
    let mut state: u64 = 0x9E3779B97F4A7C15;
    for trial in 0..40 {
        let n = 1 + (xorshift(&mut state) % 3000) as usize;
        // Tie density: smaller modulus => more ties among priorities.
        let priority_modulus = 1 + (xorshift(&mut state) % (n as u64 * 4 + 1));
        let tasks: Vec<(i64, i64)> = (0..n)
            .map(|i| (i as i64, (xorshift(&mut state) % priority_modulus) as i64))
            .collect();
        let graph = build_graph("X", &tasks, "");
        let ks = [
            1,
            1 + (xorshift(&mut state) % (n as u64 + 3)) as usize,
            n,
            n + 5,
        ];
        for k in ks {
            let expected = expected_top_k(&graph, k);
            let actual = actual_top_k(&graph, k);
            assert_eq!(actual, expected, "trial={trial} n={n} k={k} priority_modulus={priority_modulus}");
        }
    }
}

#[test]
fn extra_pattern_beyond_star_shape_still_correct() {
    // A second, unrelated variable-object pattern on the hub (not just the
    // seed + status) — still a valid star shape, exercises multi-pattern join.
    let mut ttl = String::from("@prefix ak: <http://example.org/ak#> .\n");
    for (seq, priority, region) in [(0i64, 5i64, "us"), (1, 20, "eu"), (2, 1, "us")] {
        ttl.push_str(&format!(
            "<urn:task:X:{seq}> ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority {priority} ; ak:seq {seq} ; ak:region \"{region}\" .\n"
        ));
    }
    let graph = Graph::load_str(&ttl, "turtle").unwrap();
    let r = query(
        &graph,
        &format!("{PFX} SELECT ?t ?r WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s ; ak:region ?r . }} ORDER BY DESC(?p) LIMIT 3"),
    )
    .unwrap();
    let actual: Vec<i64> = r.rows.iter().map(|row| task_seq(row[0].as_ref().unwrap())).collect();
    assert_eq!(actual, vec![1, 0, 2]);
    // Region for the top result (task 1) must be "eu", proving the extra
    // pattern's binding survived the join correctly.
    if let oxrdf::Term::Literal(l) = r.rows[0][1].as_ref().unwrap() {
        assert_eq!(l.value(), "eu");
    } else {
        panic!("expected a literal region");
    }
}

#[test]
fn large_already_claimed_prefix_still_correct() {
    // A queue drained strictly in priority order leaves a large CONTIGUOUS
    // prefix of already-claimed (status != "pending") tasks at the head of
    // the priority-sorted scan -- this must still return correct results
    // (regardless of the upfront-selectivity / cumulative-failure guards'
    // exact thresholds) across a range of already-claimed prefix sizes.
    let n = 1600;
    for already_claimed in [0usize, 50, 100, 400, 800, 1200, 1599] {
        let mut ttl = String::from("@prefix ak: <http://example.org/ak#> .\n");
        for i in 0..n {
            // priority = i, so "top `already_claimed`" = highest-numbered tasks.
            let status = if i >= n - already_claimed { "in_progress" } else { "pending" };
            ttl.push_str(&format!(
                "<urn:task:{i}> ak:peer <urn:peer:X> ; ak:status \"{status}\" ; ak:priority {i} ; ak:seq {i} .\n"
            ));
        }
        let graph = Graph::load_str(&ttl, "turtle").unwrap();
        let expected = expected_top_k(&graph, 5);
        let actual = actual_top_k(&graph, 5);
        assert_eq!(actual, expected, "already_claimed={already_claimed}");
    }
}

#[test]
fn fast_path_actually_engages_not_just_correct() {
    // Every other test in this file checks CORRECTNESS, which the fallback
    // path (eval_modified + order_bindings) also satisfies on its own -- none
    // of them would fail if try_topk_orderby_indexed were deleted entirely.
    // This test checks that the fast path actually FIRES for the shape it
    // exists for, using the engine's own EXPLAIN ANALYZE instrumentation.
    //
    // The "Plan:" section is a STATIC description of the general planner's
    // choice and always names a "BGP [...]" step regardless of which path
    // actually runs -- checking for its absence would be a vacuous assertion
    // (confirmed by first writing this test with exactly that check: it
    // failed even with the fast path correctly firing, because the static
    // plan text matched). The real signal is the "Execution trace" section,
    // which only reports a "BGP [binary GOO] (... patterns ...) rows=N" line
    // when the general BGP evaluator was ACTUALLY EXECUTED (it materializes
    // and reports the touched row count) -- the indexed fast path bypasses
    // that evaluator entirely, so this line is present iff the fallback ran.
    let n = 800;
    let graph = build_graph("X", &(0..n as i64).map(|i| (i, i)).collect::<Vec<_>>(), "");
    let explained = sparq_engine::explain_analyze(
        &graph,
        &format!("{PFX}{CLAIM_QUERY}1"),
    )
    .unwrap();
    assert!(
        !explained.contains("BGP [binary GOO]"),
        "expected the indexed fast path to fire (no BGP execution-trace line), got:\n{explained}"
    );
}
