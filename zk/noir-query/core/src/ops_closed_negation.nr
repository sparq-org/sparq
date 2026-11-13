//! Exact OPTIONAL, MINUS and correlated existence over complete right tables.
//! Pairwise compatibility and canonical sorting dominate these relations.
use crate::table::{TermCell, Table, empty, append, canonicalize, check_table, assert_table};
use crate::ops_core::Boolean;
use crate::ops_core_expr::{Step, evaluate};
use crate::ops_closed_common::{compatible, project_row, overlay};

pub fn left_join<let A: u32, let B: u32, let R: u32, let VA: u32, let VB: u32, let V: u32,
    let C: u32, let VC: u32, let EV: u32, let P: u32, let E: u32>(
    left: Table<A, VA>, right: Table<B, VB>, ac: [u32; V], bc: [u32; V],
    context: Table<C, VC>, eval_cols: [u32; EV], context_cols: [u32; EV],
    parameters: [TermCell; P], steps: [Step; E], has_condition: bool, out: Table<R, V>,
) {
    check_table(left); check_table(right); check_table(context);
    let mut expected = empty();
    for i in 0..A {
        let mut matched = false;
        for j in 0..B {
            let (compatible, _, row) = compatible(left.rows[i], right.rows[j], ac, bc);
            let mut passes = true;
            if has_condition & compatible & (i < left.len) & (j < right.len) {
                let one: Table<1, V> = Table {rows: [row], len: 1};
                let evaluated = overlay(one, context, eval_cols, context_cols);
                let predicate = evaluate(evaluated, parameters, steps)[0];
                passes = predicate.value & !predicate.is_error;
            }
            let active = (i < left.len) & (j < right.len) & compatible & passes;
            matched = matched | active;
            expected = append(expected, row, active);
        }
        expected = append(expected, project_row(left.rows[i], ac), (i < left.len) & !matched);
    }
    assert_table(canonicalize(expected), out);
}
pub fn minus<let A: u32, let B: u32, let R: u32, let VA: u32, let VB: u32>(
    left: Table<A, VA>, right: Table<B, VB>, right_cols: [u32; VA], out: Table<R, VA>,
) {
    check_table(left); check_table(right);
    let mut expected = empty();
    let mut ac = [0; VA]; for v in 0..VA {ac[v] = v;}
    for i in 0..A {
        let mut remove = false;
        for j in 0..B {
            let (matches, shared, _) = compatible(left.rows[i], right.rows[j], ac, right_cols);
            remove = remove | ((j < right.len) & matches & shared);
        }
        expected = append(expected, left.rows[i], (i < left.len) & !remove);
    }
    assert_table(expected, out);
}
pub fn existence<let A: u32, let B: u32, let V: u32>(rights: [Table<B, V>; A]) -> [Boolean; A] {
    let mut predicates = [Boolean {value: false, is_error: false}; A];
    for i in 0..A {check_table(rights[i]); predicates[i].value = rights[i].len > 0;}
    predicates
}
pub fn exists<let A: u32, let B: u32, let R: u32, let V: u32, let VB: u32>(
    left: Table<A, V>, rights: [Table<B, VB>; A], negated: bool, out: Table<R, V>,
) {
    let mut predicates = existence(rights);
    for i in 0..A {predicates[i].value = predicates[i].value != negated;}
    crate::ops_core::filter(left, predicates, out);
}
