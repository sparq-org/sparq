//! BIND of a term slot preserves each mapping, including an unbound source.
use crate::table::{TermCell, Table, empty, append, canonicalize, check_table, assert_table};
use crate::ops_closed_common::{get, project_row};
pub fn extend<let A: u32, let R: u32, let VA: u32, let V: u32>(
    input: Table<A, VA>, columns: [u32; V], target: u32,
    source: u32, constant: TermCell, is_constant: bool, out: Table<R, V>,
) {
    extend_context(input, columns, target, source, constant, is_constant, empty::<1,1>(), 1, out);
}
pub fn extend_context<let A: u32, let R: u32, let VA: u32, let V: u32, let C: u32, let VC: u32>(
    input: Table<A, VA>, columns: [u32; V], target: u32,
    source: u32, constant: TermCell, is_constant: bool, context: Table<C, VC>, capture: u32, out: Table<R, V>,
) {
    check_table(input); check_table(context); crate::table::check_cell(constant);
    assert(context.len <= 1, "capture cardinality");
    let captured = get(context.rows[0], capture);
    assert(target < V, "BIND target");
    let mut expected = empty();
    for i in 0..A {
        let mut row = project_row(input.rows[i], columns);
        assert((i >= input.len) | !row[target].bound, "BIND target already bound");
        row[target] = if is_constant {constant} else if captured.bound {captured} else {get(input.rows[i], source)};
        expected = append(expected, row, i < input.len);
    }
    assert_table(canonicalize(expected), out);
}
