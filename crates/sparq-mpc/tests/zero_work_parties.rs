use oxrdf::Variable;
use sparq_mpc::{
    distinct_hidden_pairs, distinct_hidden_pairs_oblivious,
    distinct_hidden_pairs_oblivious_with_widths, distinct_hidden_pairs_slots,
    eval_bounded_path_hidden, eval_bounded_path_hidden_slots, eval_bounded_path_hidden_with_widths,
    eval_exact_k_chain_hidden, eval_exact_k_chain_hidden_slots,
    eval_exact_k_chain_hidden_with_widths, oblivious_join_output, oblivious_set_output,
    oblivious_set_output_hidden_keys, oblivious_set_output_to_recipient, plan_hidden_bounded_path,
    BatchedHiddenInput, HiddenBoundedPath, HiddenEdges, HiddenPathRequest, HiddenValueJoin,
    HolderId, PayloadWidths, PredicatedEdges, RowBinding, ShamirBackend,
};

#[test]
fn empty_output_entries_accept_4097_parties_without_slots() {
    for parties in [1025, 4097] {
        let backend = ShamirBackend::new(parties).unwrap();
        for widths in [
            PayloadWidths::derived(1),
            PayloadWidths::declare(1, 64).unwrap(),
        ] {
            let (slots, cost) = oblivious_set_output(&backend, &[], widths, 0).unwrap();
            assert!(slots.is_empty());
            assert_eq!(cost.opens, 0);
            assert_eq!(cost.select_mults, 0);
            let (shares, cost) = oblivious_set_output_to_recipient(
                &backend,
                &[],
                widths,
                0,
                HolderId::new("recipient"),
            )
            .unwrap();
            assert_eq!(shares.bound(), 0);
            assert_eq!(cost.opens, 0);
        }
        let (result, _) =
            oblivious_join_output(&backend, &[], vec![Variable::new_unchecked("value")], 0)
                .unwrap();
        assert!(result.rows.is_empty());
        assert!(oblivious_set_output_hidden_keys(&backend, &[], &[], 0)
            .unwrap()
            .0
            .is_empty());
    }
}

#[test]
fn empty_batched_join_entries_accept_4097_parties_without_slots() {
    let join = HiddenValueJoin::new(ShamirBackend::new(4097).unwrap());
    for binding in [RowBinding::Positional, RowBinding::Keyed(vec![])] {
        let input = |holder| {
            BatchedHiddenInput::new(HolderId::new(holder), vec![], vec![], binding.clone()).unwrap()
        };
        let left = input("left");
        let right = input("right");
        for (result, cost) in [
            join.batched_join(&left, &right, 0).unwrap(),
            join.fully_oblivious_batched_join(&left, &right, 0).unwrap(),
        ] {
            assert!(result.rows.is_empty());
            assert_eq!(cost.opens, 0);
            assert_eq!(cost.select_mults, 0);
        }
    }
}

#[test]
fn empty_hidden_path_entries_accept_4097_parties_without_slots() {
    let backend = ShamirBackend::new(4097).unwrap();
    let edges = HiddenEdges::new(vec![]);
    for hops in [1, 2] {
        assert!(eval_exact_k_chain_hidden(&backend, &edges, hops, 0)
            .unwrap()
            .rows
            .is_empty());
        assert!(eval_exact_k_chain_hidden_slots(&backend, &edges, hops, 0)
            .unwrap()
            .0
            .is_empty());
        assert!(
            eval_exact_k_chain_hidden_with_widths(&backend, &edges, hops, 64, 0)
                .unwrap()
                .rows
                .is_empty()
        );
    }
    let edges = PredicatedEdges::new(vec![]).with_node_domain([]).unwrap();
    for form in [
        HiddenBoundedPath::range("p", 1, 2),
        HiddenBoundedPath::optional("p"),
    ] {
        assert!(eval_bounded_path_hidden(&backend, &edges, &form, 0)
            .unwrap()
            .rows
            .is_empty());
        assert!(eval_bounded_path_hidden_slots(&backend, &edges, &form, 0)
            .unwrap()
            .0
            .is_empty());
        assert!(
            eval_bounded_path_hidden_with_widths(&backend, &edges, &form, 64, 0)
                .unwrap()
                .rows
                .is_empty()
        );
    }
}

#[test]
fn empty_distinct_entries_accept_4097_parties_without_slots() {
    let backend = ShamirBackend::new(4097).unwrap();
    assert!(distinct_hidden_pairs(&backend, &[], 0)
        .unwrap()
        .rows
        .is_empty());
    assert!(distinct_hidden_pairs_slots(&backend, &[], 0)
        .unwrap()
        .0
        .is_empty());
    assert!(distinct_hidden_pairs_oblivious(&backend, &[], 0)
        .unwrap()
        .0
        .is_empty());
    assert!(
        distinct_hidden_pairs_oblivious_with_widths(&backend, &[], 64, 0)
            .unwrap()
            .0
            .is_empty()
    );
}

#[test]
fn empty_planner_accepts_5000_parties_without_slots() {
    let plan = plan_hidden_bounded_path(&HiddenPathRequest::range("p", 1, 3), 0).unwrap();
    let counter = plan.comm_counter(5000, 0).unwrap();
    assert_eq!(counter.parties, 5000);
    assert_eq!(counter.multiplications, 0);
    assert_eq!(counter.field_elements_shared, 0);
    assert_eq!(counter.field_elements_opened, 0);
}
