//! Captures are derived from complete upstream tables, never independently supplied.
use crate::table::{TermCell, Table, unbound, empty, append, canonicalize, check_table, assert_table, same_term, same_row};
use crate::ops_closed_common::{get};
pub fn capture<let A: u32, let PA: u32, let R: u32, let VA: u32, let VP: u32, let V: u32>(
    input: Table<A, VA>, index: u32, parent: Table<PA, VP>, has_parent: bool,
    columns: [u32; V], parent_cols: [u32; V], out: Table<R, V>,
) {
    check_table(input); check_table(parent);
    assert(index < A, "capture row"); assert(parent.len <= 1, "capture cardinality");
    let mut row = [unbound(); V];
    for v in 0..V {
        let outer = get(parent.rows[0], parent_cols[v]);
        row[v] = if has_parent & outer.bound {outer} else {get(input.rows[index], columns[v])};
    }
    let expected = append(empty(), row, (index < input.len) & (!has_parent | (parent.len > 0)));
    assert_table(expected, out);
}
pub fn unit<let C: u32, let R: u32, let VC: u32, let V: u32>(context: Table<C, VC>, out: Table<R, V>) {
    check_table(context); assert(context.len <= 1, "capture cardinality");
    assert_table(append(empty(), [unbound(); V], context.len > 0), out);
}
pub fn scan<let C: u32, let N: u32, let D: u32, let CR: u32, let VC: u32, let R: u32, let V: u32>(
    credentials: [[[TermCell; 4]; N]; C], counts: [u32; C], credential_count: u32, bounds: [u32; 3],
    context: Table<CR, VC>, slots: [u32; 3], captures: [u32; 3], constants: [TermCell; 3], named: bool, credential: u32, out: Table<R, V>,
) {
    let dataset = crate::ops_core::open_dataset::<C, N, D>(credentials, counts, credential_count, bounds);
    check_table(context); assert(context.len <= 1, "capture cardinality");
    let mut expected = empty();
    for i in 0..D {
        let mut duplicate = false;
        for j in 0..D {duplicate = duplicate | ((j < i) & (!named | (j / N == credential)) & dataset.active[j] & same_row(dataset.rows[i], dataset.rows[j]));}
        let mut matches = dataset.active[i] & (!named | (i / N == credential)) & !duplicate & (context.len > 0);
        let mut row = [unbound(); V];
        for p in 0..3 {
            assert(slots[p] <= V, "scan column");
            let outer = get(context.rows[0], captures[p]);
            if outer.bound {matches = matches & same_term(dataset.rows[i][p], outer);}
            else if slots[p] == V {matches = matches & same_term(dataset.rows[i][p], constants[p]);}
            else {for v in 0..V {if slots[p] == v {
                matches = matches & (!row[v].bound | same_term(row[v], dataset.rows[i][p]));
                row[v] = dataset.rows[i][p];
            }}}
        }
        expected = append(expected, row, matches);
    }
    assert_table(canonicalize(expected), out);
}

pub fn values<let A: u32, let C: u32, let R: u32, let V: u32, let VC: u32>(
    input: Table<A, V>, context: Table<C, VC>, captures: [u32; V], out: Table<R, V>,
) {
    check_table(input); check_table(context); assert(context.len <= 1, "capture cardinality");
    let mut expected = empty();
    for i in 0..A {
        let mut row = input.rows[i]; let mut matches = (i < input.len) & (context.len > 0);
        for v in 0..V {
            let outer = get(context.rows[0], captures[v]);
            matches = matches & (!outer.bound | !row[v].bound | same_term(outer, row[v]));
            if outer.bound {row[v] = unbound();}
        }
        expected = append(expected, row, matches);
    }
    assert_table(canonicalize(expected), out);
}
