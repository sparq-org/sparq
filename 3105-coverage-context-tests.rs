// [GPT-6 Astra] Actual public-query path and physical RHS-work witness for #3105.
#[cfg(test)]
mod capped_rhs_tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    thread_local! {
        static WORK: Cell<[usize; 4]> = const { Cell::new([0; 4]) };
        static STEPS: RefCell<Option<Vec<Step>>> = const { RefCell::new(None) };
    }

    // [GPT-6 Astra] Observe the actual branch, scan closure and returned metadata.
    #[derive(Debug, PartialEq, Eq)]
    struct Step {
        start: usize,
        pattern: usize,
        kernel: &'static str,
        requested: Option<usize>,
        scanned: bool,
        actual: Option<String>,
    }

    pub(super) fn step(
        start: usize,
        pattern: usize,
        kernel: &'static str,
        requested: Option<usize>,
        scanned: bool,
        actual: Option<&Variable>,
    ) {
        STEPS.with_borrow_mut(|steps| {
            if let Some(steps) = steps {
                steps.push(Step {
                    start,
                    pattern,
                    kernel,
                    requested,
                    scanned,
                    actual: actual.map(|v| v.as_str().to_owned()),
                });
            }
        });
    }

    fn trace<T>(f: impl FnOnce() -> T) -> (T, Vec<Step>) {
        STEPS.with_borrow_mut(|steps| *steps = Some(Vec::new()));
        let result = f();
        let steps = STEPS.with_borrow_mut(|steps| steps.take().unwrap());
        (result, steps)
    }

    fn bag(result: &crate::QueryResult) -> std::collections::BTreeMap<Vec<String>, usize> {
        let mut bag = std::collections::BTreeMap::new();
        for row in &result.rows {
            *bag.entry(
                row.iter()
                    .map(|v| v.as_ref().unwrap().to_string())
                    .collect(),
            )
            .or_default() += 1;
        }
        bag
    }

    #[test]
    fn capped_rhs_changing_sort_mixed_kernels_preserves_full_bag() {
        let mut ttl = String::from("@prefix : <http://ex/> .\n");
        for i in 0..70_000 {
            // Projection deliberately collapses pairs, making multiplicity observable.
            ttl.push_str(&format!(":s{i} :p {} ; :q {i} ; :r {i} .\n", i / 2));
        }
        ttl.push_str(":extra1 :q 70001 ; :r 70001 . :extra2 :r 70002 .");
        let graph = Graph::load_str(&ttl, "turtle").unwrap();
        let query = "PREFIX : <http://ex/> SELECT ?o WHERE { ?s :p ?o . ?s :q ?x . ?s :r ?x . FILTER(?o + 0 >= 0) }";
        let full = crate::query(&graph, query).unwrap();
        let (limited, steps) =
            trace(|| crate::query(&graph, &format!("{query} LIMIT 70001")).unwrap());
        println!("changing-sort actual steps: {steps:?}");
        assert_eq!(limited.rows.len(), 70_000);
        assert_eq!(bag(&limited), bag(&full));
        let expected: std::collections::BTreeMap<_, _> = (0..35_000)
            .map(|i| (vec![oxrdf::Literal::from(i).to_string()], 2))
            .collect();
        assert_eq!(bag(&limited), expected);
        let q: Vec<_> = steps
            .iter()
            .filter(|s| s.pattern == 1)
            .map(|s| {
                (
                    s.start,
                    s.kernel,
                    s.requested,
                    s.scanned,
                    s.actual.as_deref(),
                )
            })
            .collect();
        let r: Vec<_> = steps
            .iter()
            .filter(|s| s.pattern == 2)
            .map(|s| (s.start, s.requested, s.scanned, s.actual.as_deref()))
            .collect();
        if sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso) {
            assert_eq!(
                q,
                [
                    (0, "bind", None, false, None),
                    (1024, "merge", Some(0), true, Some("s")),
                    (65536, "bind", None, false, None)
                ]
            );
            assert_eq!(
                r,
                [
                    (0, None, true, Some("s")),
                    (1024, Some(0), true, Some("s")),
                    (65536, None, true, Some("s"))
                ]
            );
        } else {
            // Three permutations return object order: no false subject-order claim,
            // and the unchanged None request must reuse the actually unsorted-for-s RHS.
            assert_eq!(
                q,
                [
                    (0, "bind", None, false, None),
                    (1024, "hash", None, true, Some("x")),
                    (65536, "bind", None, false, None)
                ]
            );
            assert_eq!(
                r,
                [
                    (0, None, true, Some("x")),
                    (1024, None, false, Some("x")),
                    (65536, None, false, Some("x"))
                ]
            );
        }
    }

    #[test]
    fn capped_rhs_replacement_releases_old_slot_before_scan() {
        let mut slot = Some((
            None,
            Bindings::unsorted(
                vec![Variable::new("x").unwrap()],
                vec![Row::from_slice(&[1])],
            ),
        ));
        // A failed replacement leaves the actual slot empty only if old ownership
        // was released before invoking the scan. This is not a timing/heap estimate.
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            capped_rhs(&mut slot, Some(0), || panic!("replacement scan sentinel"));
        }));
        assert!(failed.is_err());
        assert!(
            slot.is_none(),
            "old RHS remained live through replacement scan"
        );
        let replacement = capped_rhs(&mut slot, Some(0), || {
            Bindings::unsorted(
                vec![Variable::new("x").unwrap()],
                vec![Row::from_slice(&[2])],
            )
        });
        assert_eq!(replacement.rows, [Row::from_slice(&[2])]);
    }

    #[test]
    fn capped_rhs_disconnected_cross_preserves_multiplicity() {
        let graph = Graph::load_str(
            "@prefix : <http://ex/> . :a :p 1 . :b :p 2 . :c :q 3, 4, 5 .",
            "turtle",
        )
        .unwrap();
        let query =
            "PREFIX : <http://ex/> SELECT ?s WHERE { ?s :p ?o . ?t :q ?x . FILTER(?o + 0 >= 0) }";
        let (limited, steps) = trace(|| crate::query(&graph, &format!("{query} LIMIT 7")).unwrap());
        assert_eq!(bag(&limited), bag(&crate::query(&graph, query).unwrap()));
        assert_eq!(limited.rows.len(), 6);
        assert_eq!(bag(&limited).values().copied().collect::<Vec<_>>(), [3, 3]);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].kernel, "cross");
        assert!(steps[0].scanned);
        println!("disconnected actual steps: {steps:?}");
    }

    #[test]
    fn capped_rhs_named_view_overlay_and_residual_exists() {
        let mut graph = Graph::load_dataset("@prefix : <http://ex/> . :g { :a :p 1, 2 ; :q 1, 2 ; :visible true . :b :p 3 ; :q 3 . }", "trig").unwrap();
        let query = "PREFIX : <http://ex/> SELECT ?s WHERE { GRAPH :g { SELECT ?s WHERE { ?s :p ?o . ?s :q ?o . FILTER(?o + 0 >= 0 && EXISTS { ?s :visible true }) } LIMIT 10 } }";
        let (base, steps) = trace(|| crate::query(&graph, query).unwrap());
        assert_eq!(base.rows.len(), 2);
        assert_eq!(base.rows[0], base.rows[1]);
        // EXISTS is deliberately non-conjunctive: prove the existing fallback remains.
        assert!(
            steps.is_empty(),
            "EXISTS must retain the scope-safe fallback"
        );
        let eligible = query.replace(" && EXISTS { ?s :visible true }", "");
        let (eligible_base, eligible_steps) = trace(|| crate::query(&graph, &eligible).unwrap());
        assert_eq!(eligible_base.rows.len(), 3);
        assert!(
            eligible_steps.iter().any(|s| s.scanned),
            "named subquery must reach capped RHS: {eligible_steps:?}"
        );
        let visible = crate::DatasetView {
            base: &graph,
            named: std::sync::Arc::new([graph.named[0].0.clone()].into_iter().collect()),
            default: crate::DefaultGraphMode::Empty,
        };
        assert_eq!(
            bag(&crate::query_view(&visible, query).unwrap()),
            bag(&base)
        );
        let (visible_rows, visible_steps) =
            trace(|| crate::query_view(&visible, &eligible).unwrap());
        assert_eq!(bag(&visible_rows), bag(&eligible_base));
        assert!(visible_steps.iter().any(|s| s.scanned));
        let hidden = crate::DatasetView {
            base: &graph,
            named: std::sync::Arc::new(Default::default()),
            default: crate::DefaultGraphMode::StoreDefault,
        };
        assert!(crate::query_view(&hidden, query).unwrap().rows.is_empty());
        let (hidden_rows, hidden_steps) = trace(|| crate::query_view(&hidden, &eligible).unwrap());
        assert!(hidden_rows.rows.is_empty());
        assert!(
            hidden_steps.is_empty(),
            "hidden graph must not enter RHS cache"
        );
        let mut fork = graph.named[0].1.fork();
        fork.apply_delta(
            &[],
            &[[
                oxrdf::NamedNode::new("http://ex/a").unwrap().into(),
                oxrdf::NamedNode::new("http://ex/q").unwrap().into(),
                oxrdf::Literal::from(2).into(),
            ]],
        )
        .unwrap();
        graph.named[0].1 = fork;
        let (changed, changed_steps) = trace(|| crate::query(&graph, query).unwrap());
        assert_eq!(changed.rows.len(), 1);
        assert_eq!(changed.rows[0], base.rows[0]);
        assert!(changed_steps.is_empty(), "overlay EXISTS retains fallback");
        let (eligible_overlay, overlay_steps) = trace(|| crate::query(&graph, &eligible).unwrap());
        assert_eq!(eligible_overlay.rows.len(), 2);
        assert!(overlay_steps.iter().any(|s| s.scanned));
        println!("named EXISTS fallback: {steps:?}, {changed_steps:?}; eligible base/overlay: {eligible_steps:?}, {overlay_steps:?}");
    }
    pub(super) fn observe(index: usize) {
        WORK.with(|cell| {
            let mut counts = cell.get();
            counts[index] += 1;
            cell.set(counts);
        });
    }

    fn take_work() -> [usize; 4] {
        WORK.with(|cell| cell.replace([0; 4]))
    }

    #[test]
    fn capped_rhs_three_block_miss() {
        let mut ttl = String::from("@prefix : <http://ex/> .\n");
        // More than the second block boundary; two shared variables prohibit bind join.
        for i in 0..70_000 {
            ttl.push_str(&format!(":s{i} :p {i} ; :q {} .\n", i + 1));
        }
        let graph = Graph::load_str(&ttl, "turtle").unwrap();
        // Arithmetic keeps this filter residual and defeats the count shortcut.
        let body = "?s :p ?o . ?s :q ?o . FILTER(?o + 0 >= 0)";
        take_work();
        assert!(!crate::ask(&graph, &format!("PREFIX : <http://ex/> ASK {{ {body} }}")).unwrap());
        let ask_work = take_work();
        assert_eq!(ask_work, [1, 3, 1, 0]);
        assert!(
            crate::query(
                &graph,
                &format!("PREFIX : <http://ex/> SELECT * WHERE {{ {body} }} LIMIT 1")
            )
            .unwrap()
            .rows
            .is_empty()
        );
        let limit_work = take_work();
        assert_eq!(limit_work, [1, 3, 1, 0]);
        println!(
            "ASK and LIMIT miss: entries/blocks/RHS scans/bind calls = {ask_work:?}, {limit_work:?}"
        );
        // The budget permits the old scan. Its presence must disable retention,
        // independently of whether it actually trips on this query.
        let generous = crate::QueryBudget {
            max_rows: Some(100_000),
            ..Default::default()
        };
        assert!(
            !crate::ask_with_budget(
                &graph,
                &format!("PREFIX : <http://ex/> ASK {{ {body} }}"),
                &generous
            )
            .unwrap()
        );
        assert_eq!(
            take_work(),
            [1, 3, 3, 0],
            "armed budget must retain original scan behavior"
        );
        let row_limited = crate::QueryBudget {
            max_rows: Some(10),
            ..Default::default()
        };
        // A nonempty intermediate, unlike the all-miss witness, exercises the row ceiling.
        let expansion =
            "PREFIX : <http://ex/> ASK { ?s :p ?o . ?s :q ?other . FILTER(?o + 0 >= 0) }";
        assert!(
            crate::ask_with_budget(&graph, expansion, &row_limited)
                .unwrap_err()
                .contains("max-rows")
        );
        take_work();
        let cancelled = crate::QueryBudget {
            cancel: Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                true,
            ))),
            ..Default::default()
        };
        assert!(
            crate::ask_with_budget(
                &graph,
                &format!("PREFIX : <http://ex/> ASK {{ {body} }}"),
                &cancelled
            )
            .unwrap_err()
            .contains("cancelled")
        );
        take_work();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let expired = crate::QueryBudget {
                deadline: Some(std::time::Instant::now()),
                ..Default::default()
            };
            assert!(
                crate::ask_with_budget(
                    &graph,
                    &format!("PREFIX : <http://ex/> ASK {{ {body} }}"),
                    &expired
                )
                .is_err()
            );
            take_work();
        }
    }
    #[test]
    fn capped_rhs_keeps_rows_and_actual_order_until_request_changes() {
        let mut slot = None;
        let variable = Variable::new("x").unwrap();
        let first = capped_rhs(&mut slot, Some(0), || Bindings {
            vars: vec![variable.clone()],
            rows: vec![Row::from_slice(&[1]), Row::from_slice(&[1])],
            // Requested and actual order need not agree on restricted permutations.
            sorted_by: None,
        });
        let pointer = first.rows.as_ptr();
        assert_eq!(first.rows.len(), 2);
        assert_eq!(first.sorted_by, None);
        let reused = capped_rhs(&mut slot, Some(0), || panic!("identical scan was repeated"));
        assert_eq!(
            reused.rows.as_ptr(),
            pointer,
            "reuse must not deep-clone rows"
        );
        assert_eq!(
            reused.rows[0], reused.rows[1],
            "bag multiplicity is retained"
        );
        let changed = capped_rhs(&mut slot, Some(2), || Bindings {
            vars: vec![variable.clone()],
            rows: vec![Row::from_slice(&[2])],
            sorted_by: Some(variable.clone()),
        });
        assert_eq!(changed.rows, vec![Row::from_slice(&[2])]);
        assert_eq!(changed.sorted_by, Some(variable));
    }

    #[test]
    fn capped_rhs_public_bags_repeated_variables_and_first_block_hit() {
        let graph = Graph::load_str(
            "@prefix : <http://ex/> . :a :p 1, 2 ; :q 1, 2 . :b :p :b ; :q :b . :c :p :b .",
            "turtle",
        )
        .unwrap();
        let body = "?s :p ?o . ?s :q ?o . FILTER(?o + 0 >= 0)";
        let full = crate::query(
            &graph,
            &format!("PREFIX : <http://ex/> SELECT ?s WHERE {{ {body} }}"),
        )
        .unwrap();
        take_work();
        let limited = crate::query(
            &graph,
            &format!("PREFIX : <http://ex/> SELECT ?s WHERE {{ {body} }} LIMIT 10"),
        )
        .unwrap();
        assert_eq!(take_work(), [1, 1, 1, 0]);
        assert_eq!(limited.rows, full.rows);
        assert_eq!(limited.rows.len(), 2);
        assert_eq!(
            limited.rows[0], limited.rows[1],
            "projection preserves multiplicity"
        );
        take_work();
        assert!(crate::ask(&graph, &format!("PREFIX : <http://ex/> ASK {{ {body} }}")).unwrap());
        assert_eq!(take_work(), [1, 1, 1, 0]);
        let repeated = "?s :p ?s . ?s :q ?o . FILTER(?s != <http://ex/missing>)";
        let result = crate::query(
            &graph,
            &format!("PREFIX : <http://ex/> SELECT ?s WHERE {{ {repeated} }} LIMIT 10"),
        )
        .unwrap();
        assert_eq!(
            result.rows.len(),
            1,
            "off-diagonal repeated-variable rows must not survive"
        );
    }
}
