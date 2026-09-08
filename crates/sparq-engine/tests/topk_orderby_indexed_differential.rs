//! Differential + correctness tests for `try_topk_orderby_indexed` (the
//! index-ordered top-k seed path added for the artifact-keeper claim-queue
//! throughput investigation).
//!
//! `try_topk_orderby_indexed` is a pure SHORTCUT: it either returns exactly the
//! valid rows the pre-existing `eval_modified`-then-`order_bindings` path would,
//! or declines (`Ok(None)`) and lets that path run. [GPT-6 Astra] Total-order
//! tests use an independent Rust sort or full ORDER BY oracle; exact ties assert
//! permitted membership and size. Execution traces pin admission/fallback guards.

use sparq_core::Graph;
use sparq_engine::query;

const PFX: &str = "PREFIX ak: <http://example.org/ak#>\n";

// [GPT-6 Astra] A repeated variable must bind one term in both positions.
// An IRI subject and an integer object can never satisfy this triple pattern.
#[test]
fn repeated_seed_variable_full_orderby_is_empty() {
    let graph = Graph::load_str("<urn:s> <urn:p> 1 .", "turtle").unwrap();
    let rows = query(&graph, "SELECT ?x WHERE { ?x <urn:p> ?x } ORDER BY ?x").unwrap();
    assert!(rows.rows.is_empty(), "full ORDER BY must enforce subject/object equality: {rows:?}");
}

// [GPT-6 Astra] The LIMIT path must preserve the full evaluator's equality guard.
#[test]
fn repeated_seed_variable_limit_is_empty() {
    let graph = Graph::load_str("<urn:s> <urn:p> 1 .", "turtle").unwrap();
    let rows = query(&graph, "SELECT ?x WHERE { ?x <urn:p> ?x } ORDER BY ?x LIMIT 1").unwrap();
    assert!(rows.rows.is_empty(), "LIMIT must enforce subject/object equality: {rows:?}");
}

// [GPT-6 Astra] LIMIT cannot hide an oversized intermediate working set.
#[test]
fn indexed_topk_preserves_intermediate_resource_limits() {
    let graph = build_graph("X", &(0..40).map(|i| (i, i)).collect::<Vec<_>>(), "");
    let query_text = format!("{PFX}{CLAIM_QUERY}1");
    for (budget, expected) in [
        (sparq_engine::QueryBudget { max_rows: Some(2), ..Default::default() }, "max-rows"),
        (sparq_engine::QueryBudget { max_bytes: Some(16), ..Default::default() }, "max-bytes"),
    ] {
        let result = sparq_engine::query_with_budget(&graph, &query_text, &budget);
        assert!(result.as_ref().is_err_and(|error| error.contains(expected)), "{expected}: {result:?}");
    }
    let generous = sparq_engine::QueryBudget { max_rows: Some(1000), ..Default::default() };
    let result = sparq_engine::query_with_budget(&graph, &query_text, &generous).unwrap();
    assert_eq!(task_seq(result.rows[0][0].as_ref().unwrap()), 39);
}

// [GPT-6 Astra] Keep the initial shortcut out of Cartesian fanout. Check both
// the ordered answers and the actual fallback execution, not static plan text.
#[test]
fn multivalued_probe_uses_fallback_and_preserves_cartesian_rows() {
    let graph = Graph::load_str(
        "<urn:s> <urn:p> 2 ; <urn:b> 10, 11 ; <urn:c> 20, 21 .", "turtle",
    ).unwrap();
    let query_text = "SELECT ?s ?b ?c WHERE { ?s <urn:p> ?p ; <urn:b> ?b ; <urn:c> ?c } ORDER BY DESC(?p) ?b ?c";
    let full = query(&graph, query_text).unwrap();
    assert_eq!(full.rows.len(), 4);
    let limited_query = format!("{query_text} LIMIT 3");
    let limited = query(&graph, &limited_query).unwrap();
    assert_eq!(limited.rows, full.rows[..3]);
    let trace = sparq_engine::explain_analyze(&graph, &limited_query).unwrap();
    assert!(trace.contains("BGP [binary GOO]"), "multivalued probe must execute the fallback: {trace}");
}

// [GPT-6 Astra] These two keys give a total order, so every OFFSET/LIMIT window
// must equal the same full-order prefix/window, including complete tie boundaries.
#[test]
fn total_order_desc_offset_windows_match_full_sort() {
    let graph = build_graph("X", &(0..96).map(|i| (i, i % 12)).collect::<Vec<_>>(), "");
    let text = format!("{PFX}SELECT ?t WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s }} ORDER BY DESC(?p) DESC(?s)");
    let full = query(&graph, &text).unwrap();
    assert_eq!(full.rows.len(), 96);
    for (offset, limit) in [(0, 0), (0, 1), (7, 2), (8, 9), (15, 3), (31, 17), (95, 4), (96, 1)] {
        let got = query(&graph, &format!("{text} OFFSET {offset} LIMIT {limit}")).unwrap();
        let expected: Vec<_> = full.rows.iter().skip(offset).take(limit).cloned().collect();
        assert_eq!(got.rows, expected, "offset={offset}, limit={limit}");
    }
}

// [GPT-6 Astra] With no secondary key, SPARQL does not specify which tied
// subjects survive. Assert valid membership, multiplicity and size, not stability.
#[test]
fn exact_desc_ties_preserve_valid_membership_and_window_size() {
    let graph = build_graph("X", &(0..32).map(|i| (i, 7)).collect::<Vec<_>>(), "");
    let text = format!("{PFX}SELECT ?t ?p WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p }} ORDER BY DESC(?p)");
    for (offset, limit) in [(0usize, 5usize), (3, 9), (28, 10), (40, 2), (0, 0)] {
        let got = query(&graph, &format!("{text} OFFSET {offset} LIMIT {limit}")).unwrap();
        assert_eq!(got.rows.len(), 32usize.saturating_sub(offset).min(limit));
        let ids: std::collections::HashSet<_> = got.rows.iter()
            .map(|row| task_seq(row[0].as_ref().unwrap())).collect();
        assert_eq!(ids.len(), got.rows.len(), "no duplicate solution may be manufactured");
        assert!(ids.iter().all(|id| (0..32).contains(id)));
        assert!(got.rows.iter().all(|row| as_int(row[1].as_ref().unwrap()) == 7));
    }
}

// [GPT-6 Astra] Each fixture has distinct numeric values, avoiding unspecified ties.
#[test]
fn negative_mixed_and_typed_lexical_values_use_the_fallback() {
    for values in [
        ["-5", "-1", "-3"],
        ["\"02\"^^xsd:integer", "\"4\"^^xsd:int", "1.5"],
        ["1", "-2", "3.5"],
    ] {
        let mut ttl = String::from("@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n");
        for (i, value) in values.iter().enumerate() {
            ttl.push_str(&format!("<urn:s{i}> <urn:p> {value} .\n"));
        }
        let graph = Graph::load_str(&ttl, "turtle").unwrap();
        let text = "SELECT ?s ?p WHERE { ?s <urn:p> ?p } ORDER BY DESC(?p) ?s";
        let full = query(&graph, text).unwrap();
        let limited = format!("{text} LIMIT 2");
        assert_eq!(query(&graph, &limited).unwrap().rows, full.rows[..2]);
        let trace = sparq_engine::explain_analyze(&graph, &limited).unwrap();
        assert!(trace.contains("BGP [binary GOO]"), "non-inline fixture must decline: {values:?}, {trace}");
    }
}

// [GPT-6 Astra] Exercise an indexed subquery inside GRAPH, where the empty-default
// view is suspended for a visible named graph. Hidden data must remain inaccessible.
#[test]
fn ordered_subqueries_preserve_default_and_named_view_boundaries() {
    use sparq_engine::{DatasetView, DefaultGraphMode, query_view};
    let graph = Graph::load_dataset(
        "<urn:d> <urn:p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n\
         <urn:v> <urn:p> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> <urn:visible> .\n\
         <urn:h> <urn:p> \"99\"^^<http://www.w3.org/2001/XMLSchema#integer> <urn:hidden> .\n",
        "nquads",
    ).unwrap();
    let named = std::sync::Arc::new([oxrdf::Term::NamedNode(oxrdf::NamedNode::new("urn:visible").unwrap())].into_iter().collect());
    let default_query = "SELECT ?s WHERE { ?s <urn:p> ?p } ORDER BY DESC(?p) LIMIT 1";
    let named_query = |name: &str| format!("SELECT ?s WHERE {{ GRAPH <{name}> {{ SELECT ?s WHERE {{ ?s <urn:p> ?p }} ORDER BY DESC(?p) LIMIT 1 }} }}");
    for default in [DefaultGraphMode::StoreDefault, DefaultGraphMode::Empty] {
        let view = DatasetView { base: &graph, named: std::sync::Arc::clone(&named), default };
        let rows = query_view(&view, default_query).unwrap().rows;
        assert_eq!(rows.len(), usize::from(default == DefaultGraphMode::StoreDefault));
        if let Some(row) = rows.first() { assert_eq!(row[0].as_ref().unwrap().to_string(), "<urn:d>"); }
        let visible = query_view(&view, &named_query("urn:visible")).unwrap();
        assert_eq!(visible.rows[0][0].as_ref().unwrap().to_string(), "<urn:v>");
        assert_eq!(visible.rows.len(), 1);
        for hidden in ["urn:hidden", "urn:absent"] {
            assert!(query_view(&view, &named_query(hidden)).unwrap().rows.is_empty());
            let from = format!("SELECT ?s FROM <{hidden}> WHERE {{ ?s <urn:p> ?p }} ORDER BY DESC(?p) LIMIT 1");
            assert!(query_view(&view, &from).unwrap().rows.is_empty());
        }
    }
    assert_eq!(query(&graph, &named_query("urn:hidden")).unwrap().rows[0][0].as_ref().unwrap().to_string(), "<urn:h>");
}

// [GPT-6 Astra] A scan must observe both overlay tombstones and newly inserted ids,
// while a retained snapshot and the subsequently compacted result stay equivalent.
#[test]
fn forked_overlay_and_compaction_preserve_ordered_answers() {
    let base = build_graph("X", &(0..40).map(|i| (i, i)).collect::<Vec<_>>(), "");
    let mut changed = base.fork();
    sparq_engine::update_in_place(&mut changed, &format!("{PFX}DELETE DATA {{ <urn:task:X:39> ak:priority 39 }}; INSERT DATA {{ <urn:task:X:90> ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority 90 ; ak:seq 90 }}")).unwrap();
    assert!(changed.pending_delta_len() > 0);
    for k in [1, 7, 40, 50] { assert_eq!(actual_top_k(&changed, k), expected_top_k(&changed, k)); }
    assert_eq!(actual_top_k(&changed, 1), vec![90]);
    assert_eq!(actual_top_k(&base, 1), vec![39]);
    let before = actual_top_k(&changed, 50);
    changed.compact().unwrap();
    assert_eq!(changed.pending_delta_len(), 0);
    assert_eq!(actual_top_k(&changed, 50), before);
}

// [GPT-6 Astra] A live cancellation flag arms the fallback even while false.
// No sleeps or concurrent timing race is needed to prove cancellation and cleanup.
#[test]
fn armed_cancellation_uses_fallback_and_does_not_leak() {
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
    let graph = build_graph("X", &(0..40).map(|i| (i, i)).collect::<Vec<_>>(), "");
    let text = format!("{PFX}{CLAIM_QUERY}1");
    let flag = Arc::new(AtomicBool::new(false));
    let budget = sparq_engine::QueryBudget::cancelled_by(Arc::clone(&flag));
    let trace = sparq_engine::explain_analyze_with_budget(&graph, &text, &budget).unwrap();
    assert!(trace.contains("BGP [binary GOO]"), "armed cancellation must decline: {trace}");
    flag.store(true, Ordering::Relaxed);
    let error = sparq_engine::query_with_budget(&graph, &text, &budget).unwrap_err();
    assert!(error.contains("query budget exceeded (cancelled)"), "{error}");
    assert_eq!(actual_top_k(&graph, 1), vec![39]);
}

// [GPT-6 Astra] Expiration is established before execution, never by a timed race.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn armed_deadline_uses_fallback_and_expired_deadline_errors() {
    use std::time::{Duration, Instant};
    let graph = build_graph("X", &(0..40).map(|i| (i, i)).collect::<Vec<_>>(), "");
    let text = format!("{PFX}{CLAIM_QUERY}1");
    let future = sparq_engine::QueryBudget { deadline: Some(Instant::now() + Duration::from_secs(3600)), ..Default::default() };
    let trace = sparq_engine::explain_analyze_with_budget(&graph, &text, &future).unwrap();
    assert!(trace.contains("BGP [binary GOO]"), "armed deadline must decline: {trace}");
    let expired = sparq_engine::QueryBudget { deadline: Some(Instant::now()), ..Default::default() };
    let error = sparq_engine::query_with_budget(&graph, &text, &expired).unwrap_err();
    assert!(error.contains("query budget exceeded (timeout)"), "{error}");
    assert_eq!(actual_top_k(&graph, 1), vec![39]);
}

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
    // [GPT-6 Astra] compact-index lacks the PSO scan needed by variable-object
    // probes. Exercise its real decline instead of a zero-test feature pass.
    let supported = sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso);
    assert_eq!(!explained.contains("BGP [binary GOO]"), supported,
               "indexed engagement must match the built permutations:\n{explained}");
}
