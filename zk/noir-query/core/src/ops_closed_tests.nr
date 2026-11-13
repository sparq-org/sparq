//! Adversarial contracts use the production relation gadgets.
use crate::table::{TermCell, Table, empty, append, unbound};
use crate::{ops_closed_negation as neg, ops_closed_union as bag, ops_closed_extend as bind};
fn cell(n: Field) -> TermCell {TermCell {bound: true, kind: 1, enc: n, key_class: 0, key: 0, dt: 0, lang: 0, scope: 0}}
fn two() -> Table<4, 1> {append(append(empty(), [cell(1)], true), [cell(3)], true)}
fn right() -> Table<4, 1> {append(empty(), [cell(2)], true)}
fn optional(out: Table<4, 1>) {
    neg::left_join(two(), right(), [0], [0], empty::<1,1>(), [0], [1], [unbound()],
        [crate::ops_core_expr::Step {op: 7, a: 0, b: 0, parameter: 0, comparison: 0}], false, out);
}
fn minus(out: Table<4, 1>) {neg::minus(two(), right(), [0], out);}
fn exists(out: Table<4, 1>) {neg::exists(two(), [right(), right(), empty(), empty()], false, out);}
fn not_exists(out: Table<4, 1>) {neg::exists(two(), [empty::<4,1>(); 4], true, out);}
fn union(out: Table<4, 1>) {bag::union(two(), empty::<4,1>(), [0], [0], out);}
fn extend(out: Table<4, 1>) {bind::extend(two(), [1], 0, 0, unbound(), false, out);}

#[test]
fn optional_honest() {
    let mut out = two();
    optional(out);
}

#[test(should_fail_with = "operator length")]
fn optional_missing() {
    let mut out = two();
    out.len = 1; out.rows[1] = [unbound()];
    optional(out);
}

#[test(should_fail_with = "operator length")]
fn optional_extra() {
    let mut out = two();
    out = append(out, [cell(4)], true);
    optional(out);
}

#[test(should_fail_with = "table order")]
fn optional_reordered() {
    let mut out = two();
    out.rows[0] = [cell(3)]; out.rows[1] = [cell(1)];
    optional(out);
}

#[test(should_fail_with = "table padding")]
fn optional_padding() {
    let mut out = two();
    out.rows[3] = [cell(4)];
    optional(out);
}

#[test]
fn minus_honest() {
    let mut out = two();
    minus(out);
}

#[test(should_fail_with = "operator length")]
fn minus_missing() {
    let mut out = two();
    out.len = 1; out.rows[1] = [unbound()];
    minus(out);
}

#[test(should_fail_with = "operator length")]
fn minus_extra() {
    let mut out = two();
    out = append(out, [cell(4)], true);
    minus(out);
}

#[test(should_fail_with = "table order")]
fn minus_reordered() {
    let mut out = two();
    out.rows[0] = [cell(3)]; out.rows[1] = [cell(1)];
    minus(out);
}

#[test(should_fail_with = "table padding")]
fn minus_padding() {
    let mut out = two();
    out.rows[3] = [cell(4)];
    minus(out);
}

#[test]
fn exists_honest() {
    let mut out = two();
    exists(out);
}

#[test(should_fail_with = "operator length")]
fn exists_missing() {
    let mut out = two();
    out.len = 1; out.rows[1] = [unbound()];
    exists(out);
}

#[test(should_fail_with = "operator length")]
fn exists_extra() {
    let mut out = two();
    out = append(out, [cell(4)], true);
    exists(out);
}

#[test(should_fail_with = "table order")]
fn exists_reordered() {
    let mut out = two();
    out.rows[0] = [cell(3)]; out.rows[1] = [cell(1)];
    exists(out);
}

#[test(should_fail_with = "table padding")]
fn exists_padding() {
    let mut out = two();
    out.rows[3] = [cell(4)];
    exists(out);
}

#[test]
fn not_exists_honest() {
    let mut out = two();
    not_exists(out);
}

#[test(should_fail_with = "operator length")]
fn not_exists_missing() {
    let mut out = two();
    out.len = 1; out.rows[1] = [unbound()];
    not_exists(out);
}

#[test(should_fail_with = "operator length")]
fn not_exists_extra() {
    let mut out = two();
    out = append(out, [cell(4)], true);
    not_exists(out);
}

#[test(should_fail_with = "table order")]
fn not_exists_reordered() {
    let mut out = two();
    out.rows[0] = [cell(3)]; out.rows[1] = [cell(1)];
    not_exists(out);
}

#[test(should_fail_with = "table padding")]
fn not_exists_padding() {
    let mut out = two();
    out.rows[3] = [cell(4)];
    not_exists(out);
}

#[test]
fn union_honest() {
    let mut out = two();
    union(out);
}

#[test(should_fail_with = "operator length")]
fn union_missing() {
    let mut out = two();
    out.len = 1; out.rows[1] = [unbound()];
    union(out);
}

#[test(should_fail_with = "operator length")]
fn union_extra() {
    let mut out = two();
    out = append(out, [cell(4)], true);
    union(out);
}

#[test(should_fail_with = "table order")]
fn union_reordered() {
    let mut out = two();
    out.rows[0] = [cell(3)]; out.rows[1] = [cell(1)];
    union(out);
}

#[test(should_fail_with = "table padding")]
fn union_padding() {
    let mut out = two();
    out.rows[3] = [cell(4)];
    union(out);
}

#[test]
fn extend_honest() {
    let mut out = two();
    extend(out);
}

#[test(should_fail_with = "operator length")]
fn extend_missing() {
    let mut out = two();
    out.len = 1; out.rows[1] = [unbound()];
    extend(out);
}

#[test(should_fail_with = "operator length")]
fn extend_extra() {
    let mut out = two();
    out = append(out, [cell(4)], true);
    extend(out);
}

#[test(should_fail_with = "table order")]
fn extend_reordered() {
    let mut out = two();
    out.rows[0] = [cell(3)]; out.rows[1] = [cell(1)];
    extend(out);
}

#[test(should_fail_with = "table padding")]
fn extend_padding() {
    let mut out = two();
    out.rows[3] = [cell(4)];
    extend(out);
}

#[test(should_fail_with = "operator capacity")]
fn optional_overflow() {
    neg::left_join(two(), right(), [0], [0], empty::<1,1>(), [0], [1], [unbound()],
        [crate::ops_core_expr::Step {op: 7, a: 0, b: 0, parameter: 0, comparison: 0}], false, empty::<1,1>());
}
#[test(should_fail_with = "operator capacity")]
fn minus_overflow() {neg::minus(two(), right(), [0], empty::<1,1>());}
#[test(should_fail_with = "operator capacity")]
fn exists_overflow() {neg::exists(two(), [right(), right(), empty(), empty()], false, empty::<1,1>());}
#[test(should_fail_with = "operator capacity")]
fn not_exists_overflow() {neg::exists(two(), [empty::<4,1>(); 4], true, empty::<1,1>());}
#[test(should_fail_with = "operator capacity")]
fn union_overflow() {bag::union(two(), empty::<4,1>(), [0], [0], empty::<1,1>());}
#[test(should_fail_with = "operator capacity")]
fn extend_overflow() {bind::extend(two(), [1], 0, 0, unbound(), false, empty::<1,1>());}

// Hiding a matching row must fail at the exact upstream table relation.
fn full_right(hidden: Table<4,1>) {crate::ops_core::project(two(), [0], hidden);}
#[test(should_fail_with = "operator length")]
fn minus_hidden_matching_row() {
    let hidden = append(empty::<4,1>(), [cell(3)], true);
    full_right(hidden); neg::minus(two(), hidden, [0], empty::<4,1>());
}
#[test(should_fail_with = "operator length")]
fn optional_hidden_matching_row() {full_right(empty::<4,1>()); optional(two());}
#[test(should_fail_with = "operator length")]
fn not_exists_hidden_matching_row() {full_right(empty::<4,1>()); not_exists(two());}
#[test(should_fail_with = "operator length")]
fn exists_hidden_matching_row() {full_right(empty::<4,1>()); exists(two());}
#[test]
fn minus_disjoint_domains_keep_left() {neg::minus(two(), right(), [1], two());}
#[test]
fn minus_unbound_shared_column_is_disjoint() {
    neg::minus(two(), append(empty::<4,1>(), [unbound()], true), [0], two());
}
#[test]
fn union_keeps_duplicate_mappings() {
    let mut out = append(append(empty::<4,1>(), [cell(1)], true), [cell(1)], true);
    out = append(out, [cell(3)], true); out = append(out, [cell(3)], true);
    bag::union(two(), two(), [0], [0], out);
}
#[test]
fn extend_unbound_keeps_mapping() {
    bind::extend(two(), [0,1], 1, 1, unbound(), false,
        append(append(empty::<4,2>(), [cell(1),unbound()], true), [cell(3),unbound()], true));
}

fn graph(out: Table<4,2>) {
    crate::ops_closed_graph::graph(two(), append(empty::<1,1>(), [unbound()], true), [0,1], 1, 1, cell(9), out);
}
fn graph_expected() -> Table<4,2> {append(append(empty(), [cell(1),cell(9)], true), [cell(3),cell(9)], true)}
fn scan(out: Table<4,1>) {
    let credentials = [[[cell(8),cell(9),cell(1),unbound()], [cell(8),cell(9),cell(3),unbound()]]];
    crate::ops_closed_substitute::scan::<1,2,2,1,1,4,1>(credentials, [2], 1, [1,2,4],
        append(empty(), [unbound()], true), [1,1,0], [1,1,0], [cell(8),cell(9),unbound()], false, 0, out);
}
fn substitute(out: Table<4,1>) {
    crate::ops_closed_substitute::values(two(), append(empty::<1,1>(), [unbound()], true), [1], out);
}

#[test]
fn graph_honest() {
    let mut out = graph_expected();
    graph(out);
}

#[test(should_fail_with = "operator length")]
fn graph_missing() {
    let mut out = graph_expected();
    out.len = 1; out.rows[1] = [unbound(); 2];
    graph(out);
}

#[test(should_fail_with = "operator length")]
fn graph_extra() {
    let mut out = graph_expected();
    out = append(out, [cell(4); 2], true);
    graph(out);
}

#[test(should_fail_with = "table order")]
fn graph_reordered() {
    let mut out = graph_expected();
    let first = out.rows[0]; out.rows[0] = out.rows[1]; out.rows[1] = first;
    graph(out);
}

#[test(should_fail_with = "table padding")]
fn graph_padding() {
    let mut out = graph_expected();
    out.rows[3] = [cell(4); 2];
    graph(out);
}

#[test]
fn scan_honest() {
    let mut out = two();
    scan(out);
}

#[test(should_fail_with = "operator length")]
fn scan_missing() {
    let mut out = two();
    out.len = 1; out.rows[1] = [unbound(); 1];
    scan(out);
}

#[test(should_fail_with = "operator length")]
fn scan_extra() {
    let mut out = two();
    out = append(out, [cell(4); 1], true);
    scan(out);
}

#[test(should_fail_with = "table order")]
fn scan_reordered() {
    let mut out = two();
    let first = out.rows[0]; out.rows[0] = out.rows[1]; out.rows[1] = first;
    scan(out);
}

#[test(should_fail_with = "table padding")]
fn scan_padding() {
    let mut out = two();
    out.rows[3] = [cell(4); 1];
    scan(out);
}

#[test]
fn substitute_honest() {
    let mut out = two();
    substitute(out);
}

#[test(should_fail_with = "operator length")]
fn substitute_missing() {
    let mut out = two();
    out.len = 1; out.rows[1] = [unbound(); 1];
    substitute(out);
}

#[test(should_fail_with = "operator length")]
fn substitute_extra() {
    let mut out = two();
    out = append(out, [cell(4); 1], true);
    substitute(out);
}

#[test(should_fail_with = "table order")]
fn substitute_reordered() {
    let mut out = two();
    let first = out.rows[0]; out.rows[0] = out.rows[1]; out.rows[1] = first;
    substitute(out);
}

#[test(should_fail_with = "table padding")]
fn substitute_padding() {
    let mut out = two();
    out.rows[3] = [cell(4); 1];
    substitute(out);
}

#[test(should_fail_with = "operator capacity")]
fn graph_overflow() {
    crate::ops_closed_graph::graph(two(), append(empty::<1,1>(), [unbound()], true), [0,1], 1, 1, cell(9), empty::<1,2>());
}
#[test(should_fail_with = "operator capacity")]
fn scan_overflow() {
    let credentials = [[[cell(8),cell(9),cell(1),unbound()], [cell(8),cell(9),cell(3),unbound()]]];
    crate::ops_closed_substitute::scan::<1,2,2,1,1,1,1>(credentials, [2], 1, [1,2,1],
        append(empty(), [unbound()], true), [1,1,0], [1,1,0], [cell(8),cell(9),unbound()], false, 0, empty());
}
#[test(should_fail_with = "operator capacity")]
fn substitute_overflow() {crate::ops_closed_substitute::values(two(), append(empty::<1,1>(), [unbound()], true), [1], empty::<1,1>());}
#[test]
fn optional_keeps_all_matches_and_one_unmatched_mapping() {
    let right = append(append(empty::<4,2>(), [cell(1),cell(2)], true), [cell(1),cell(4)], true);
    let out = append(append(append(empty::<4,2>(), [cell(1),cell(2)], true), [cell(1),cell(4)], true), [cell(3),unbound()], true);
    neg::left_join(two(), right, [0,1], [0,1], empty::<1,1>(), [0,1], [1,1], [unbound()],
        [crate::ops_core_expr::Step {op:7,a:0,b:0,parameter:0,comparison:0}], false, out);
}
#[test]
fn minus_removes_compatible_bound_mapping() {
    neg::minus(two(), append(empty::<4,1>(), [cell(1)], true), [0], append(empty::<4,1>(), [cell(3)], true));
}
#[test]
fn exists_and_not_exists_choose_different_outer_rows() {
    let rights = [right(), empty(), right(), empty()];
    neg::exists(two(), rights, false, append(empty::<4,1>(), [cell(1)], true));
    neg::exists(two(), rights, true, append(empty::<4,1>(), [cell(3)], true));
}
#[test]
fn graph_absent_credential_is_empty_even_for_empty_graph_pattern() {
    crate::ops_closed_graph::seed(append(empty::<1,1>(), [unbound()], true), 0, 0, cell(9), 1, unbound(), empty::<4,1>());
}
#[test]
fn captured_term_disappears_before_minus_observes_domains() {
    let context = append(empty::<1,1>(), [cell(1)], true);
    let output = append(empty::<4,1>(), [unbound()], true);
    crate::ops_closed_substitute::values(append(empty::<4,1>(), [cell(1)], true), context, [0], output);
    neg::minus(output, output, [0], output);
}

fn capture(out: Table<4,1>) {
    crate::ops_closed_substitute::capture(two(), 0, empty::<1,1>(), false, [0], [1], out);
}
fn graph_seed(out: Table<4,1>) {
    crate::ops_closed_graph::seed(append(empty::<1,1>(), [cell(1)], true), 1, 0, cell(9), 1, unbound(), out);
}
fn context_unit(out: Table<4,1>) {
    crate::ops_closed_substitute::unit(append(empty::<1,1>(), [unbound()], true), out);
}

#[test]
fn capture_honest() {
    let mut out = append(empty::<4,1>(), [cell(1)], true);
    capture(out);
}

#[test(should_fail_with = "operator length")]
fn capture_missing() {
    let mut out = append(empty::<4,1>(), [cell(1)], true);
    out = empty();
    capture(out);
}

#[test(should_fail_with = "operator length")]
fn capture_extra() {
    let mut out = append(empty::<4,1>(), [cell(1)], true);
    out = append(out, [cell(4)], true);
    capture(out);
}

#[test(should_fail_with = "table order")]
fn capture_reordered() {
    let mut out = append(empty::<4,1>(), [cell(1)], true);
    out.len = 2; out.rows[0] = [cell(3)]; out.rows[1] = [cell(1)];
    capture(out);
}

#[test(should_fail_with = "table padding")]
fn capture_padding() {
    let mut out = append(empty::<4,1>(), [cell(1)], true);
    out.rows[3] = [cell(4)];
    capture(out);
}

#[test]
fn graph_seed_honest() {
    let mut out = append(empty::<4,1>(), [cell(1)], true);
    graph_seed(out);
}

#[test(should_fail_with = "operator length")]
fn graph_seed_missing() {
    let mut out = append(empty::<4,1>(), [cell(1)], true);
    out = empty();
    graph_seed(out);
}

#[test(should_fail_with = "operator length")]
fn graph_seed_extra() {
    let mut out = append(empty::<4,1>(), [cell(1)], true);
    out = append(out, [cell(4)], true);
    graph_seed(out);
}

#[test(should_fail_with = "table order")]
fn graph_seed_reordered() {
    let mut out = append(empty::<4,1>(), [cell(1)], true);
    out.len = 2; out.rows[0] = [cell(3)]; out.rows[1] = [cell(1)];
    graph_seed(out);
}

#[test(should_fail_with = "table padding")]
fn graph_seed_padding() {
    let mut out = append(empty::<4,1>(), [cell(1)], true);
    out.rows[3] = [cell(4)];
    graph_seed(out);
}

#[test]
fn context_unit_honest() {
    let mut out = append(empty::<4,1>(), [unbound()], true);
    context_unit(out);
}

#[test(should_fail_with = "operator length")]
fn context_unit_missing() {
    let mut out = append(empty::<4,1>(), [unbound()], true);
    out = empty();
    context_unit(out);
}

#[test(should_fail_with = "operator length")]
fn context_unit_extra() {
    let mut out = append(empty::<4,1>(), [unbound()], true);
    out = append(out, [cell(4)], true);
    context_unit(out);
}

#[test(should_fail_with = "table order")]
fn context_unit_reordered() {
    let mut out = append(empty::<4,1>(), [unbound()], true);
    out.len = 2; out.rows[0] = [cell(3)]; out.rows[1] = [cell(1)];
    context_unit(out);
}

#[test(should_fail_with = "table padding")]
fn context_unit_padding() {
    let mut out = append(empty::<4,1>(), [unbound()], true);
    out.rows[3] = [cell(4)];
    context_unit(out);
}

#[test(should_fail_with = "operator capacity")]
fn capture_overflow() {crate::ops_closed_substitute::capture(two(), 0, empty::<1,1>(), false, [0], [1], empty::<0,1>());}
#[test(should_fail_with = "operator capacity")]
fn graph_seed_overflow() {crate::ops_closed_graph::seed(append(empty::<1,1>(), [unbound()], true), 1, 0, cell(9), 1, unbound(), empty::<0,1>());}
#[test(should_fail_with = "operator capacity")]
fn context_unit_overflow() {crate::ops_closed_substitute::unit(append(empty::<1,1>(), [unbound()], true), empty::<0,1>());}

#[test(should_fail_with = "BIND target already bound")]
fn extend_cannot_overwrite_an_existing_binding() {bind::extend(two(), [0], 0, 0, cell(9), true, two());}
#[test(should_fail_with = "graph name")]
fn graph_rejects_a_literal_graph_identifier() {
    let mut name = cell(9); name.kind = 3;
    crate::ops_closed_graph::graph(two(), append(empty::<1,1>(), [unbound()], true), [0], 1, 1, name, two());
}

#[test]
fn minus_matching_last_right_row_must_remove_left() {
    let right = append(append(empty::<4,1>(), [cell(2)], true), [cell(3)], true);
    neg::minus(two(), right, [0], append(empty::<4,1>(), [cell(1)], true));
}
#[test]
fn optional_matching_last_right_row_suppresses_unmatched_branch() {
    let right = append(append(empty::<4,2>(), [cell(2),cell(9)], true), [cell(3),cell(8)], true);
    let out = append(append(empty::<4,2>(), [cell(1),unbound()], true), [cell(3),cell(8)], true);
    neg::left_join(two(), right, [0,1], [0,1], empty::<1,1>(), [0,1], [1,1], [unbound()],
        [crate::ops_core_expr::Step {op:7,a:0,b:0,parameter:0,comparison:0}], false, out);
}
