#[cfg(test)]
mod capped_rhs_tests {
    use super::*;
    use std::cell::Cell;

    thread_local! {
        static WORK: Cell<[usize; 4]> = const { Cell::new([0; 4]) };
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
