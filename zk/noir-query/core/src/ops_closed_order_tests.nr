use crate::table::{TermCell, Table, unbound, empty};
use crate::ops_closed_order::{order_by, OrderKey};

fn num(enc: Field, n: Field) -> TermCell {
    TermCell { bound: true, kind: 3, enc, key_class: 1,
        key: 0x800000000000000000000000000000000000000000000000000000000000 + n * 1000000000000000000,
        dt: 11, lang: 0, scope: 0 }
}
fn key(descending: bool) -> OrderKey { OrderKey { column: 0, descending } }
fn input() -> Table<4, 1> { Table { rows: [[num(1, 4)], [num(2, 1)], [num(3, 3)], [unbound()]], len: 3 } }
fn output() -> Table<4, 1> { Table { rows: [[num(2, 1)], [num(3, 3)], [num(1, 4)], [unbound()]], len: 3 } }
#[test]
fn order_honest() { order_by(input(), output(), [key(false)]); }
#[test(should_fail_with = "operator length")]
fn order_missing() { let mut out = output(); out.len = 2; out.rows[2] = [unbound()]; order_by(input(), out, [key(false)]); }
#[test(should_fail_with = "operator length")]
fn order_extra() { let mut out = output(); out.len = 4; out.rows[3] = [num(4, 7)]; order_by(input(), out, [key(false)]); }
#[test(should_fail_with = "ORDER BY rows")]
fn order_reordered() { order_by(input(), input(), [key(false)]); }
#[test(should_fail_with = "table padding")]
fn order_padding() { let mut out = output(); out.rows[3] = [num(4, 7)]; order_by(input(), out, [key(false)]); }
#[test(should_fail_with = "operator capacity")]
fn order_overflow() { order_by(input(), empty::<2, 1>(), [key(false)]); }
#[test(should_fail_with = "ORDER BY rows")]
fn order_wrong_key() { let mut out = output(); out.rows[0][0].key = out.rows[0][0].key + 1; order_by(input(), out, [key(false)]); }
#[test(should_fail_with = "ORDER BY rows")]
fn order_duplicate_loses_an_input_row() { let mut out = output(); out.rows[1] = out.rows[0]; order_by(input(), out, [key(false)]); }
#[test]
fn order_descending() { order_by(input(), Table { rows: [[num(1, 4)], [num(3, 3)], [num(2, 1)], [unbound()]], len: 3 }, [key(true)]); }
#[test]
fn order_multiple_keys_and_canonical_ties() {
    let data = Table { rows: [[num(1, 2), num(8, 1)], [num(2, 2), num(7, 3)], [num(3, 1), num(6, 8)], [num(4, 2), num(7, 3)]], len: 4 };
    order_by(data, Table { rows: [data.rows[2], data.rows[1], data.rows[3], data.rows[0]], len: 4 },
        [key(false), OrderKey { column: 1, descending: true }]);
}
#[test]
fn order_bag_multiplicity_and_empty() {
    let data = Table { rows: [[num(1, 4)], [num(1, 4)], [num(2, 1)]], len: 3 };
    order_by(data, Table { rows: [data.rows[2], data.rows[0], data.rows[1]], len: 3 }, [key(false)]);
    order_by(empty::<2, 1>(), empty::<1, 1>(), [key(false)]);
}
#[test]
fn order_term_kinds_and_lexical_iris() {
    let blank = TermCell { bound: true, kind: 2, enc: 9, key_class: 0, key: 0, dt: 0, lang: 0, scope: 1 };
    let iri_z = TermCell { bound: true, kind: 1, enc: 1, key_class: 6, key: 122, dt: 0, lang: 0, scope: 0 };
    let iri_a = TermCell { bound: true, kind: 1, enc: 8, key_class: 6, key: 97, dt: 0, lang: 0, scope: 0 };
    let data = Table { rows: [[unbound()], [iri_z], [num(2, 0)], [iri_a], [blank]], len: 5 };
    order_by(data, Table { rows: [[unbound()], [blank], [iri_a], [iri_z], [num(2, 0)]], len: 5 }, [key(false)]);
}
#[test]
fn unordered_literals_extend_the_partial_order_without_cycles() {
    let mut unknown = num(2, 0); unknown.key_class = 0; unknown.dt = 99;
    let data = Table { rows: [[num(1, 4)], [unknown], [num(3, 1)]], len: 3 };
    order_by(data, Table { rows: [[unknown], [num(3, 1)], [num(1, 4)]], len: 3 }, [key(false)]);
}
#[test]
fn order_strings_booleans_temporals_and_language_ties() {
    for class in [3, 4, 5, 6] {
        let mut a = num(1, 2); a.key_class = class;
        let mut b = num(2, 1); b.key_class = class;
        order_by(Table { rows: [[a], [b]], len: 2 }, Table { rows: [[b], [a]], len: 2 }, [key(false)]);
        a.lang = 7; b.lang = 7;
        if class == 6 { order_by(Table { rows: [[a], [b]], len: 2 }, Table { rows: [[a], [b]], len: 2 }, [key(false)]); }
    }
}
#[test(should_fail_with = "ORDER BY IRI lexical key opening required")]
fn order_iri_without_key_refuses() {
    let mut iri = num(1, 0); iri.kind = 1; iri.key_class = 0;
    order_by(Table { rows: [[iri]], len: 1 }, Table { rows: [[iri]], len: 1 }, [key(false)]);
}
#[test(should_fail_with = "ORDER BY double arithmetic pending")]
fn order_double_refuses() { let mut c = num(1, 0); c.key_class = 2; order_by(Table { rows: [[c]], len: 1 }, Table { rows: [[c]], len: 1 }, [key(false)]); }
#[test(should_fail_with = "ORDER BY lexical opening required")]
fn order_long_string_refuses() { let mut c = num(1, 0); c.key_class = 7; order_by(Table { rows: [[c]], len: 1 }, Table { rows: [[c]], len: 1 }, [key(false)]); }
#[test(should_fail_with = "ORDER BY column")]
fn order_invalid_column() { order_by(input(), output(), [OrderKey { column: 1, descending: false }]); }

#[test(should_fail_with = "ORDER BY lexical key collision opening required")]
fn equal_lexical_prefix_keys_cannot_order_distinct_terms() {
    let mut a = num(1, 0); a.key_class = 6;
    let mut b = num(2, 0); b.key_class = 6;
    order_by(Table { rows: [[a], [b]], len: 2 }, Table { rows: [[a], [b]], len: 2 }, [key(false)]);
}
#[test(should_fail_with = "table padding")]
fn order_input_length_cannot_hide_a_row() { let mut data = input(); data.len = 2; order_by(data, output(), [key(false)]); }
#[test(should_fail_with = "table order")]
fn order_input_must_be_canonical() { order_by(output(), output(), [key(false)]); }

#[test(should_fail_with = "ORDER BY mixed timezone comparison pending")]
fn mixed_timezone_order_requires_temporal_comparator() {
    let mut a = num(1, 0); a.key_class = 3;
    let mut b = num(2, 1); b.key_class = 4;
    order_by(Table { rows: [[a], [b]], len: 2 }, Table { rows: [[a], [b]], len: 2 }, [key(false)]);
}
