use crate::table::{TermCell, Table, unbound, empty};
use crate::ops_closed_group::{group, Aggregate, NumericTypes, Having};

global OFFSET: Field = 0x800000000000000000000000000000000000000000000000000000000000;
global SCALE: Field = 1000000000000000000;

// Test-only deterministic codec. Production wiring supplies the suite codec.
fn encode(key: Field, decimal: bool) -> TermCell {
    TermCell { bound: true, kind: 3, enc: key + decimal as Field,
        key_class: 1, key, dt: if decimal { 22 } else { 11 }, lang: 0, scope: 0 }
}
fn num(n: Field) -> TermCell { encode(OFFSET + n * SCALE, false) }
fn dec(n: Field) -> TermCell { encode(OFFSET + n * SCALE, true) }
fn iri(n: Field) -> TermCell {
    TermCell { bound: true, kind: 1, enc: n, key_class: 6, key: n, dt: 0, lang: 0, scope: 0 }
}
fn types() -> NumericTypes { NumericTypes { integer_dt: 11, decimal_dt: 22 } }
fn no_having() -> Having { Having { enabled: false, column: 0, comparison: 0, rhs: unbound() } }
fn spec(kind: u8) -> Aggregate { Aggregate { kind, column: if kind == 0 { 2 } else { 1 }, distinct: false } }
fn input() -> Table<4, 2> {
    Table { rows: [[iri(1), num(1)], [iri(1), num(1)], [iri(1), num(4)], [iri(2), num(2)]], len: 4 }
}
fn output(a: TermCell, b: TermCell) -> Table<4, 2> {
    Table { rows: [[iri(1), a], [iri(2), b], [unbound(); 2], [unbound(); 2]], len: 2 }
}
fn run(kind: u8, out: Table<4, 2>) {
    group(input(), out, [0], [spec(kind)], [[false]; 4], types(), no_having(), encode);
}

#[test]
fn count_star_honest() { run(0, output(num(3), num(1))); }
#[test(should_fail_with = "operator length")]
fn count_star_missing() { let mut out = output(num(3), num(1)); out.len = 1; out.rows[1] = [unbound(); 2]; run(0, out); }
#[test(should_fail_with = "operator length")]
fn count_star_extra() { let mut out = output(num(3), num(1)); out.len = 3; out.rows[2] = [iri(3), num(1)]; run(0, out); }
#[test(should_fail_with = "table order")]
fn count_star_reordered() { let mut out = output(num(3), num(1)); let row = out.rows[0]; out.rows[0] = out.rows[1]; out.rows[1] = row; run(0, out); }
#[test(should_fail_with = "table padding")]
fn count_star_padding() { let mut out = output(num(3), num(1)); out.rows[2] = [iri(3), num(1)]; run(0, out); }
#[test(should_fail_with = "operator capacity")]
fn count_star_overflow() { group(input(), empty::<1, 2>(), [0], [spec(0)], [[false]; 4], types(), no_having(), encode); }
#[test(should_fail_with = "operator rows")]
fn count_star_wrong_value() { run(0, output(num(99), num(1))); }

#[test]
fn count_honest() { run(1, output(num(3), num(1))); }
#[test(should_fail_with = "operator length")]
fn count_missing() { let mut out = output(num(3), num(1)); out.len = 1; out.rows[1] = [unbound(); 2]; run(1, out); }
#[test(should_fail_with = "operator length")]
fn count_extra() { let mut out = output(num(3), num(1)); out.len = 3; out.rows[2] = [iri(3), num(1)]; run(1, out); }
#[test(should_fail_with = "table order")]
fn count_reordered() { let mut out = output(num(3), num(1)); let row = out.rows[0]; out.rows[0] = out.rows[1]; out.rows[1] = row; run(1, out); }
#[test(should_fail_with = "table padding")]
fn count_padding() { let mut out = output(num(3), num(1)); out.rows[2] = [iri(3), num(1)]; run(1, out); }
#[test(should_fail_with = "operator capacity")]
fn count_overflow() { group(input(), empty::<1, 2>(), [0], [spec(1)], [[false]; 4], types(), no_having(), encode); }
#[test(should_fail_with = "operator rows")]
fn count_wrong_value() { run(1, output(num(99), num(1))); }

#[test]
fn sum_honest() { run(2, output(num(6), num(2))); }
#[test(should_fail_with = "operator length")]
fn sum_missing() { let mut out = output(num(6), num(2)); out.len = 1; out.rows[1] = [unbound(); 2]; run(2, out); }
#[test(should_fail_with = "operator length")]
fn sum_extra() { let mut out = output(num(6), num(2)); out.len = 3; out.rows[2] = [iri(3), num(1)]; run(2, out); }
#[test(should_fail_with = "table order")]
fn sum_reordered() { let mut out = output(num(6), num(2)); let row = out.rows[0]; out.rows[0] = out.rows[1]; out.rows[1] = row; run(2, out); }
#[test(should_fail_with = "table padding")]
fn sum_padding() { let mut out = output(num(6), num(2)); out.rows[2] = [iri(3), num(1)]; run(2, out); }
#[test(should_fail_with = "operator capacity")]
fn sum_overflow() { group(input(), empty::<1, 2>(), [0], [spec(2)], [[false]; 4], types(), no_having(), encode); }
#[test(should_fail_with = "operator rows")]
fn sum_wrong_value() { run(2, output(num(99), num(2))); }

#[test]
fn min_honest() { run(3, output(num(1), num(2))); }
#[test(should_fail_with = "operator length")]
fn min_missing() { let mut out = output(num(1), num(2)); out.len = 1; out.rows[1] = [unbound(); 2]; run(3, out); }
#[test(should_fail_with = "operator length")]
fn min_extra() { let mut out = output(num(1), num(2)); out.len = 3; out.rows[2] = [iri(3), num(1)]; run(3, out); }
#[test(should_fail_with = "table order")]
fn min_reordered() { let mut out = output(num(1), num(2)); let row = out.rows[0]; out.rows[0] = out.rows[1]; out.rows[1] = row; run(3, out); }
#[test(should_fail_with = "table padding")]
fn min_padding() { let mut out = output(num(1), num(2)); out.rows[2] = [iri(3), num(1)]; run(3, out); }
#[test(should_fail_with = "operator capacity")]
fn min_overflow() { group(input(), empty::<1, 2>(), [0], [spec(3)], [[false]; 4], types(), no_having(), encode); }
#[test(should_fail_with = "operator rows")]
fn min_wrong_value() { run(3, output(num(99), num(2))); }

#[test]
fn max_honest() { run(4, output(num(4), num(2))); }
#[test(should_fail_with = "operator length")]
fn max_missing() { let mut out = output(num(4), num(2)); out.len = 1; out.rows[1] = [unbound(); 2]; run(4, out); }
#[test(should_fail_with = "operator length")]
fn max_extra() { let mut out = output(num(4), num(2)); out.len = 3; out.rows[2] = [iri(3), num(1)]; run(4, out); }
#[test(should_fail_with = "table order")]
fn max_reordered() { let mut out = output(num(4), num(2)); let row = out.rows[0]; out.rows[0] = out.rows[1]; out.rows[1] = row; run(4, out); }
#[test(should_fail_with = "table padding")]
fn max_padding() { let mut out = output(num(4), num(2)); out.rows[2] = [iri(3), num(1)]; run(4, out); }
#[test(should_fail_with = "operator capacity")]
fn max_overflow() { group(input(), empty::<1, 2>(), [0], [spec(4)], [[false]; 4], types(), no_having(), encode); }
#[test(should_fail_with = "operator rows")]
fn max_wrong_value() { run(4, output(num(99), num(2))); }

#[test]
fn avg_honest() { run(5, output(dec(2), dec(2))); }
#[test(should_fail_with = "operator length")]
fn avg_missing() { let mut out = output(dec(2), dec(2)); out.len = 1; out.rows[1] = [unbound(); 2]; run(5, out); }
#[test(should_fail_with = "operator length")]
fn avg_extra() { let mut out = output(dec(2), dec(2)); out.len = 3; out.rows[2] = [iri(3), num(1)]; run(5, out); }
#[test(should_fail_with = "table order")]
fn avg_reordered() { let mut out = output(dec(2), dec(2)); let row = out.rows[0]; out.rows[0] = out.rows[1]; out.rows[1] = row; run(5, out); }
#[test(should_fail_with = "table padding")]
fn avg_padding() { let mut out = output(dec(2), dec(2)); out.rows[2] = [iri(3), num(1)]; run(5, out); }
#[test(should_fail_with = "operator capacity")]
fn avg_overflow() { group(input(), empty::<1, 2>(), [0], [spec(5)], [[false]; 4], types(), no_having(), encode); }
#[test(should_fail_with = "operator rows")]
fn avg_wrong_value() { run(5, output(num(99), dec(2))); }

#[test]
fn sample_honest() { run(6, output(num(1), num(2))); }
#[test(should_fail_with = "operator length")]
fn sample_missing() { let mut out = output(num(1), num(2)); out.len = 1; out.rows[1] = [unbound(); 2]; run(6, out); }
#[test(should_fail_with = "operator length")]
fn sample_extra() { let mut out = output(num(1), num(2)); out.len = 3; out.rows[2] = [iri(3), num(1)]; run(6, out); }
#[test(should_fail_with = "table order")]
fn sample_reordered() { let mut out = output(num(1), num(2)); let row = out.rows[0]; out.rows[0] = out.rows[1]; out.rows[1] = row; run(6, out); }
#[test(should_fail_with = "table padding")]
fn sample_padding() { let mut out = output(num(1), num(2)); out.rows[2] = [iri(3), num(1)]; run(6, out); }
#[test(should_fail_with = "operator capacity")]
fn sample_overflow() { group(input(), empty::<1, 2>(), [0], [spec(6)], [[false]; 4], types(), no_having(), encode); }
#[test(should_fail_with = "operator rows")]
fn sample_wrong_value() { run(6, output(num(99), num(2))); }

#[test]
fn distinct_aggregates_and_full_mapping_count() {
    for kind in [0, 1, 2, 3, 4, 5, 6] {
        let a = if (kind == 0) | (kind == 1) { num(2) }
            else if kind == 2 { num(5) } else if kind == 4 { num(4) }
            else if kind == 5 { dec(5 / 2) } else { num(1) };
        group(input(), output(a, if kind == 5 { dec(2) } else if (kind == 0) | (kind == 1) { num(1) } else { num(2) }),
            [0], [Aggregate { kind, column: if kind == 0 { 2 } else { 1 }, distinct: true }],
            [[false]; 4], types(), no_having(), encode);
    }
}

#[test]
fn empty_global_group_all_aggregates() {
    for kind in [0, 1, 2, 3, 4, 5, 6] {
        let value = if (kind <= 2) | (kind == 5) { num(0) } else { unbound() };
        group(empty::<2, 1>(), Table { rows: [[value], [unbound()]], len: 1 }, [],
            [Aggregate { kind, column: if kind == 0 { 1 } else { 0 }, distinct: false }],
            [[false]; 2], types(), no_having(), encode);
    }
}
#[test]
fn explicit_group_over_empty_input_has_no_groups() {
    group(empty::<2, 1>(), empty::<2, 2>(), [0],
        [Aggregate { kind: 0, column: 1, distinct: false }], [[false]; 2], types(), no_having(), encode);
}
#[test(should_fail_with = "operator length")]
fn empty_global_group_cannot_be_hidden() {
    group(empty::<2, 1>(), empty::<2, 1>(), [],
        [Aggregate { kind: 0, column: 1, distinct: false }], [[false]; 2], types(), no_having(), encode);
}
#[test(should_fail_with = "operator rows")]
fn nonempty_input_cannot_claim_empty_count() {
    group(Table { rows: [[num(9)], [unbound()]], len: 1 }, Table { rows: [[num(0)], [unbound()]], len: 1 }, [],
        [Aggregate { kind: 0, column: 1, distinct: false }], [[false]; 2], types(), no_having(), encode);
}
#[test(should_fail_with = "operator length")]
fn hidden_row_cannot_reach_empty_group_through_checked_input() {
    let hidden = empty::<2, 1>();
    crate::ops_core::values([[num(9)]], hidden);
    group(hidden, Table { rows: [[num(0)], [unbound()]], len: 1 }, [],
        [Aggregate { kind: 0, column: 1, distinct: false }], [[false]; 2], types(), no_having(), encode);
}
#[test(should_fail_with = "table padding")]
fn hidden_input_length_rejects_nonpadding_row() {
    let mut data = input(); data.len = 3;
    group(data, output(num(3), num(1)), [0], [spec(0)], [[false]; 4], types(), no_having(), encode);
}
#[test(should_fail_with = "operator rows")]
fn wrong_group_membership() { run(2, output(num(5), num(3))); }
#[test]
fn unbound_and_expression_errors() {
    let data = Table { rows: [[unbound()], [num(2)], [num(4)]], len: 3 };
    for kind in [1, 2, 3, 4, 5, 6] {
        let value = if (kind == 1) | (kind == 6) { num(2) } else { unbound() };
        group(data, Table { rows: [[value]], len: 1 }, [],
            [Aggregate { kind, column: 0, distinct: false }], [[false]; 3], types(), no_having(), encode);
        let value_error = if (kind == 2) | (kind == 5) { unbound() } else { value };
        group(data, Table { rows: [[value_error]], len: 1 }, [],
            [Aggregate { kind, column: 0, distinct: false }], [[true], [false], [false]], types(), no_having(), encode);
    }
}
#[test]
fn nonnumeric_sum_and_avg_are_unbound() {
    for kind in [2, 5] {
        group(Table { rows: [[iri(1)], [num(2)]], len: 2 }, Table { rows: [[unbound()]], len: 1 }, [],
            [Aggregate { kind, column: 0, distinct: false }], [[false]; 2], types(), no_having(), encode);
    }
}
#[test]
fn integer_negative_sum_and_decimal_average() {
    let data = Table { rows: [[num(0 - 4)], [num(1)]], len: 2 };
    group(data, Table { rows: [[num(0 - 3), dec(0 - 3 / 2)]], len: 1 }, [],
        [Aggregate { kind: 2, column: 0, distinct: false }, Aggregate { kind: 5, column: 0, distinct: false }],
        [[false; 2]; 2], types(), no_having(), encode);
}
#[test(should_fail)]
fn sum_numeric_range_overflow() {
    let large: Field = 883423532389192164791648750371459257913741948437809479;
    let data = Table { rows: [[num(large)], [num(large)]], len: 2 };
    group(data, Table { rows: [[encode(OFFSET + large * 2 * SCALE, false)]], len: 1 }, [], [Aggregate { kind: 2, column: 0, distinct: false }], [[false]; 2], types(), no_having(), encode);
}
#[test(should_fail)]
fn avg_does_not_round_nonterminating_decimal() {
    group(Table { rows: [[num(0)], [num(0)], [num(1)]], len: 3 }, Table { rows: [[encode(OFFSET + SCALE / 3, true)]], len: 1 }, [],
        [Aggregate { kind: 5, column: 0, distinct: false }], [[false]; 3], types(), no_having(), encode);
}
#[test(should_fail_with = "integer aggregate arithmetic required")]
fn decimal_sum_refused_by_name() {
    group(Table { rows: [[dec(1)]], len: 1 }, empty::<1, 1>(), [],
        [Aggregate { kind: 2, column: 0, distinct: false }], [[false]], types(), no_having(), encode);
}
#[test(should_fail_with = "GROUP_CONCAT lexical opening required")]
fn group_concat_refused_by_name() {
    group(input(), empty::<4, 2>(), [0], [spec(7)], [[false]; 4], types(), no_having(), encode);
}
#[test]
fn multiple_group_keys_and_aggregates() {
    group(input(), Table { rows: [[iri(1), num(1), num(2), num(2)], [iri(1), num(4), num(1), num(4)],
        [iri(2), num(2), num(1), num(2)]], len: 3 }, [0, 1], [spec(0), spec(2)], [[false; 2]; 4], types(), no_having(), encode);
}
#[test]
fn unbound_group_key_and_blank_node_scope() {
    let mut a = iri(1); a.kind = 2; a.scope = 1;
    let mut b = a; b.scope = 2;
    group(Table { rows: [[unbound()], [a], [b]], len: 3 },
        Table { rows: [[unbound(), num(1)], [a, num(1)], [b, num(1)]], len: 3 }, [0],
        [Aggregate { kind: 0, column: 1, distinct: false }], [[false]; 3], types(), no_having(), encode);
}
fn having() -> Having { Having { enabled: true, column: 1, comparison: 4, rhs: num(1) } }
fn run_having(out: Table<4, 2>) { group(input(), out, [0], [spec(0)], [[false]; 4], types(), having(), encode); }
#[test]
fn having_honest() { run_having(output(num(3), num(1))); }
#[test(should_fail_with = "operator length")]
fn having_missing() { let mut out = output(num(3), num(1)); out.len = 1; out.rows[1] = [unbound(); 2]; run_having(out); }
#[test(should_fail_with = "operator length")]
fn having_extra() { let mut out = output(num(3), num(1)); out.len = 3; out.rows[2] = [iri(3), num(1)]; run_having(out); }
#[test(should_fail_with = "table order")]
fn having_reordered() { let mut out = output(num(3), num(1)); out.rows[0] = [iri(2), num(1)]; out.rows[1] = [iri(1), num(3)]; run_having(out); }
#[test(should_fail_with = "table padding")]
fn having_padding() { let mut out = output(num(3), num(1)); out.rows[2] = [iri(3), num(1)]; run_having(out); }
#[test(should_fail_with = "operator capacity")]
fn having_overflow() { group(input(), empty::<1, 2>(), [0], [spec(0)], [[false]; 4], types(), having(), encode); }
#[test]
fn having_filters_recomputed_aggregate_and_errors() {
    group(input(), Table { rows: [[iri(1), num(6)], [unbound(); 2]], len: 1 }, [0], [spec(2)], [[false]; 4], types(),
        Having { enabled: true, column: 1, comparison: 3, rhs: num(3) }, encode);
    group(empty::<2, 1>(), empty::<2, 1>(), [], [Aggregate { kind: 3, column: 0, distinct: false }], [[false]; 2], types(),
        Having { enabled: true, column: 0, comparison: 3, rhs: num(0) }, encode);
}
#[test(should_fail_with = "operator rows")]
fn having_wrong_aggregate_cannot_change_membership() { run_having(output(num(1), num(3))); }

#[test(should_fail_with = "GROUP_CONCAT lexical opening required")]
fn group_concat_empty_explicit_group_refuses() {
    group(empty::<2, 1>(), empty::<2, 2>(), [0], [Aggregate { kind: 7, column: 0, distinct: false }], [[false]; 2], types(), no_having(), encode);
}
#[test(should_fail_with = "aggregate error cell")]
fn aggregate_bound_value_cannot_claim_expression_error() {
    group(input(), output(num(0), num(0)), [0], [spec(1)], [[true]; 4], types(), no_having(), encode);
}
#[test(should_fail_with = "aggregate error padding")]
fn aggregate_padding_cannot_hide_error_lane() {
    group(empty::<2, 1>(), Table { rows: [[num(0)]], len: 1 }, [], [Aggregate { kind: 1, column: 0, distinct: false }], [[false], [true]], types(), no_having(), encode);
}
#[test(should_fail_with = "numeric encoding contract")]
fn numeric_codec_cannot_change_comparison_key() {
    let bad = |key: Field, decimal: bool| encode(key + SCALE, decimal);
    group(input(), output(num(3), num(1)), [0], [spec(0)], [[false]; 4], types(), no_having(), bad);
}
#[test]
fn numeric_sum_large_cancellation_is_exact() {
    let n: Field = 883423532389192164791648750371459257913741948437809479;
    group(Table { rows: [[num(0 - n)], [num(n)], [num(n)]], len: 3 }, Table { rows: [[num(n)]], len: 1 }, [],
        [Aggregate { kind: 2, column: 0, distinct: false }], [[false]; 3], types(), no_having(), encode);
}
#[test(should_fail_with = "HAVING numeric comparison required")]
fn having_nonnumeric_refused_by_name() {
    group(input(), empty::<4, 2>(), [0], [spec(2)], [[false]; 4], types(),
        Having { enabled: true, column: 1, comparison: 0, rhs: iri(3) }, encode);
}
#[test]
fn having_all_comparisons_are_checked() {
    for comparison in [0, 1, 2, 3, 4] {
        let mut out = empty::<2, 2>();
        if comparison == 3 { out.rows[0] = [iri(1), num(6)]; out.len = 1; }
        if (comparison == 0) | (comparison == 2) { out.rows[0] = [iri(2), num(2)]; out.len = 1; }
        if comparison == 4 { out = Table { rows: [[iri(1), num(6)], [iri(2), num(2)]], len: 2 }; }
        group(input(), out, [0], [spec(2)], [[false]; 4], types(),
            Having { enabled: true, column: 1, comparison, rhs: num(2) }, encode);
    }
}

#[test]
fn distinct_preserves_numeric_lexical_term_identity() {
    let a = num(1); let mut b = a; b.enc = a.enc + 2;
    group(Table { rows: [[a], [b]], len: 2 }, Table { rows: [[num(2)]], len: 1 }, [],
        [Aggregate { kind: 1, column: 0, distinct: true }], [[false]; 2], types(), no_having(), encode);
}
#[test]
fn min_max_ties_choose_canonical_term_encoding() {
    let a = num(1); let mut b = a; b.enc = a.enc + 2;
    for kind in [3, 4] {
        group(Table { rows: [[a], [b]], len: 2 }, Table { rows: [[a]], len: 1 }, [],
            [Aggregate { kind, column: 0, distinct: false }], [[false]; 2], types(), no_having(), encode);
    }
}
#[test(should_fail)]
fn integer_datatype_with_fractional_key_refuses() {
    let mut fractional = num(1); fractional.key = fractional.key + 1;
    group(Table { rows: [[fractional]], len: 1 }, empty::<1, 1>(), [],
        [Aggregate { kind: 2, column: 0, distinct: false }], [[false]], types(), no_having(), encode);
}

#[test(should_fail_with = "duplicate output group")]
fn equal_cardinality_cannot_duplicate_one_group_and_omit_another() {
    group(input(), Table { rows: [[iri(1), num(3)], [iri(1), num(3)]], len: 2 },
        [0], [spec(0)], [[false]; 4], types(), no_having(), encode);
}
#[test(should_fail_with = "operator rows")]
fn equal_cardinality_cannot_invent_a_group() {
    group(input(), Table { rows: [[iri(1), num(3)], [iri(3), num(1)]], len: 2 },
        [0], [spec(0)], [[false]; 4], types(), no_having(), encode);
}

fn group_only(out: Table<4, 1>) { group(input(), out, [0], [], [[false; 0]; 4], types(), no_having(), encode); }
fn group_only_output() -> Table<4, 1> { Table { rows: [[iri(1)], [iri(2)], [unbound()], [unbound()]], len: 2 } }
#[test]
fn group_honest() { group_only(group_only_output()); }
#[test(should_fail_with = "operator length")]
fn group_missing() { let mut out = group_only_output(); out.len = 1; out.rows[1] = [unbound()]; group_only(out); }
#[test(should_fail_with = "operator length")]
fn group_extra() { let mut out = group_only_output(); out.len = 3; out.rows[2] = [iri(3)]; group_only(out); }
#[test(should_fail_with = "table order")]
fn group_reordered() { let mut out = group_only_output(); out.rows[0] = [iri(2)]; out.rows[1] = [iri(1)]; group_only(out); }
#[test(should_fail_with = "table padding")]
fn group_padding() { let mut out = group_only_output(); out.rows[2] = [iri(3)]; group_only(out); }
#[test(should_fail_with = "operator capacity")]
fn group_overflow() { group(input(), empty::<1, 1>(), [0], [], [[false; 0]; 4], types(), no_having(), encode); }
#[test]
fn global_group_without_aggregates_has_one_empty_mapping() {
    group(empty::<2, 1>(), Table { rows: [[], []], len: 1 }, [], [], [[false; 0]; 2], types(), no_having(), encode);
}

#[test(should_fail_with = "operator rows")]
fn distinct_count_cannot_claim_bag_count() {
    group(input(), output(num(3), num(1)), [0], [Aggregate { kind: 1, column: 1, distinct: true }],
        [[false]; 4], types(), no_having(), encode);
}

#[test]
fn avg_count_zero_clause_includes_only_error_operands() {
    for explicit_error in [false, true] {
        group(Table { rows: [[unbound()]], len: 1 }, Table { rows: [[num(0)]], len: 1 }, [],
            [Aggregate { kind: 5, column: 0, distinct: false }], [[explicit_error]], types(), no_having(), encode);
    }
}
#[test(should_fail_with = "operator rows")]
fn sum_cannot_silently_omit_an_unbound_operand() {
    group(Table { rows: [[unbound()], [num(2)]], len: 2 }, Table { rows: [[num(2)]], len: 1 }, [],
        [Aggregate { kind: 2, column: 0, distinct: false }], [[false]; 2], types(), no_having(), encode);
}

#[test]
fn aggregate_output_is_the_checked_order_input() {
    let grouped = output(num(6), num(2));
    group(input(), grouped, [0], [spec(2)], [[false]; 4], types(), no_having(), encode);
    crate::ops_closed_order::order_by(grouped,
        Table { rows: [[iri(2), num(2)], [iri(1), num(6)], [unbound(); 2], [unbound(); 2]], len: 2 },
        [crate::ops_closed_order::OrderKey { column: 1, descending: false }]);
}

#[test]
fn count_star_distinct_uses_all_mapping_columns() {
    group(Table { rows: [[iri(1), num(1)], [iri(2), num(1)]], len: 2 },
        Table { rows: [[num(2), num(1)]], len: 1 }, [],
        [Aggregate { kind: 0, column: 2, distinct: true }, Aggregate { kind: 1, column: 1, distinct: true }],
        [[false; 2]; 2], types(), no_having(), encode);
}
