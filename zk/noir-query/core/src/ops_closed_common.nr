//! Shared closed-relation wiring. All data access is bounded by full table capacity.
use crate::table::{TermCell, Table, unbound, same_term, check_table};
use crate::ops_core::Boolean;

pub fn get<let V: u32>(row: [TermCell; V], column: u32) -> TermCell {
    assert(column <= V, "closed column");
    let mut cell = unbound();
    for v in 0..V { if column == v { cell = row[v]; } }
    cell
}
pub fn project_row<let VA: u32, let V: u32>(row: [TermCell; VA], columns: [u32; V]) -> [TermCell; V] {
    let mut out = [unbound(); V];
    for v in 0..V { out[v] = get(row, columns[v]); }
    out
}
pub fn compatible<let VA: u32, let VB: u32, let V: u32>(
    a: [TermCell; VA], b: [TermCell; VB], ac: [u32; V], bc: [u32; V],
) -> (bool, bool, [TermCell; V]) {
    let mut matches = true; let mut shared = false; let mut row = [unbound(); V];
    for v in 0..V {
        let l = get(a, ac[v]); let r = get(b, bc[v]);
        matches = matches & (!l.bound | !r.bound | same_term(l, r));
        shared = shared | (l.bound & r.bound);
        row[v] = if l.bound { l } else { r };
    }
    (matches, shared, row)
}
pub fn overlay<let A: u32, let VA: u32, let C: u32, let VC: u32, let V: u32>(
    input: Table<A, VA>, context: Table<C, VC>, columns: [u32; V], context_cols: [u32; V],
) -> Table<A, V> {
    check_table(input); check_table(context);
    assert(context.len <= 1, "capture cardinality");
    let mut out = crate::table::empty();
    out.len = input.len;
    for i in 0..A { for v in 0..V {
        if i < input.len {
            let outer = get(context.rows[0], context_cols[v]);
            out.rows[i][v] = if outer.bound {outer} else {get(input.rows[i], columns[v])};
        }
    } }
    out
}
pub fn combine<let R: u32>(a: [Boolean; R], b: [Boolean; R], op: u8) -> [Boolean; R] {
    assert(op <= 2, "closed boolean opcode");
    let mut out = a;
    for i in 0..R {
        if op == 0 {out[i] = Boolean {value: !a[i].value, is_error: a[i].is_error};}
        else if op == 1 {out[i] = Boolean {value: a[i].value & b[i].value,
            is_error: (a[i].is_error | b[i].is_error) & !((!a[i].value & !a[i].is_error) | (!b[i].value & !b[i].is_error))};}
        else {out[i] = Boolean {value: a[i].value | b[i].value,
            is_error: (a[i].is_error | b[i].is_error) & !((a[i].value & !a[i].is_error) | (b[i].value & !b[i].is_error))};}
    }
    out
}
