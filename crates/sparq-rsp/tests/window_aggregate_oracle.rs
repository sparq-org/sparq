#![cfg(feature = "window-aggregate")]

// sq-lsp7k.27: hand-derived deterministic aggregate oracle.

use oxrdf::{Literal, NamedNode, Term, Variable};
use sparq_rsp::{window_aggregate, Agg, WindowResult};

fn result(vars: &[&str], rows: Vec<Vec<Option<Term>>>) -> WindowResult {
    WindowResult {
        start: 0,
        end: 10,
        vars: vars
            .iter()
            .map(|name| Variable::new(*name).expect("test variable is valid"))
            .collect(),
        rows,
    }
}

fn number(value: i32) -> Option<Term> {
    Some(Literal::from(value).into())
}

#[test]
fn exact_count_sum_min_and_max_oracle() {
    let window = result(
        &["x"],
        vec![vec![number(1)], vec![number(2)], vec![number(3)]],
    );

    assert_eq!(window_aggregate(&window, "x", Agg::Count), Some(3.0));
    assert_eq!(window_aggregate(&window, "x", Agg::Sum), Some(6.0));
    assert_eq!(window_aggregate(&window, "x", Agg::Avg), Some(2.0));
    assert_eq!(window_aggregate(&window, "x", Agg::Min), Some(1.0));
    assert_eq!(window_aggregate(&window, "x", Agg::Median), Some(2.0));
    assert_eq!(window_aggregate(&window, "x", Agg::Max), Some(3.0));
}

#[test]
fn numeric_fold_skips_unbound_and_non_numeric_while_count_keeps_rows() {
    let window = result(
        &["x"],
        vec![
            vec![number(4)],
            vec![None],
            vec![Some(Literal::new_simple_literal("not numeric").into())],
            vec![Some(
                NamedNode::new_unchecked("http://example/value").into(),
            )],
            Vec::new(),
            vec![number(-2)],
        ],
    );

    assert_eq!(window_aggregate(&window, "x", Agg::Count), Some(6.0));
    assert_eq!(window_aggregate(&window, "x", Agg::Sum), Some(2.0));
    assert_eq!(window_aggregate(&window, "x", Agg::Avg), Some(1.0));
    assert_eq!(window_aggregate(&window, "x", Agg::Min), Some(-2.0));
    assert_eq!(window_aggregate(&window, "x", Agg::Median), Some(1.0));
    assert_eq!(window_aggregate(&window, "x", Agg::Max), Some(4.0));
}

#[test]
fn absent_variable_returns_none_for_every_aggregate() {
    let window = result(&["x"], vec![vec![number(1)]]);

    for aggregate in [
        Agg::Count,
        Agg::Sum,
        Agg::Avg,
        Agg::Min,
        Agg::Median,
        Agg::Max,
    ] {
        assert_eq!(window_aggregate(&window, "notavar", aggregate), None);
    }
}

#[test]
fn empty_window_has_zero_count_and_sum_but_no_extrema() {
    let window = result(&["x"], Vec::new());

    assert_eq!(window_aggregate(&window, "x", Agg::Count), Some(0.0));
    assert_eq!(window_aggregate(&window, "x", Agg::Sum), Some(0.0));
    assert_eq!(window_aggregate(&window, "x", Agg::Avg), None);
    assert_eq!(window_aggregate(&window, "x", Agg::Min), None);
    assert_eq!(window_aggregate(&window, "x", Agg::Median), None);
    assert_eq!(window_aggregate(&window, "x", Agg::Max), None);
}

#[test]
fn column_selection_and_row_order_do_not_change_the_oracle() {
    let first = result(
        &["ignored", "x"],
        vec![
            vec![number(100), Some(Literal::from(1.0e16).into())],
            vec![number(100), Some(Literal::from(-1.0e16).into())],
            vec![number(100), number(1)],
        ],
    );
    let reversed = result(
        &["ignored", "x"],
        vec![
            vec![number(100), number(1)],
            vec![number(100), Some(Literal::from(-1.0e16).into())],
            vec![number(100), Some(Literal::from(1.0e16).into())],
        ],
    );

    for aggregate in [
        Agg::Count,
        Agg::Sum,
        Agg::Avg,
        Agg::Min,
        Agg::Median,
        Agg::Max,
    ] {
        assert_eq!(
            window_aggregate(&first, "x", aggregate),
            window_aggregate(&reversed, "x", aggregate)
        );
    }
}

#[test]
fn average_uses_only_numeric_bindings() {
    // sq-xf3b8: value-pinned oracle witnesses the divisor and unbound skip.
    let numeric = result(
        &["x"],
        vec![vec![number(2)], vec![number(4)], vec![number(6)]],
    );
    let with_unbound = result(&["x"], vec![vec![number(10)], vec![None], vec![number(20)]]);

    assert_eq!(window_aggregate(&numeric, "x", Agg::Avg), Some(4.0));
    assert_eq!(window_aggregate(&with_unbound, "x", Agg::Avg), Some(15.0));
}

#[test]
fn median_handles_odd_even_empty_and_unbound_numeric_sets() {
    // sq-sfle1: pinned values witness both middle-selection branches.
    let odd = result(
        &["x"],
        vec![vec![number(10)], vec![number(20)], vec![number(30)]],
    );
    let even = result(
        &["x"],
        vec![
            vec![number(10)],
            vec![number(20)],
            vec![number(30)],
            vec![number(40)],
        ],
    );
    let empty = result(&["x"], Vec::new());
    let with_unbound = result(&["x"], vec![vec![number(10)], vec![None], vec![number(30)]]);

    assert_eq!(window_aggregate(&odd, "x", Agg::Median), Some(20.0));
    assert_eq!(window_aggregate(&even, "x", Agg::Median), Some(25.0));
    assert_eq!(window_aggregate(&empty, "x", Agg::Median), None);
    assert_eq!(
        window_aggregate(&with_unbound, "x", Agg::Median),
        Some(20.0)
    );
}
