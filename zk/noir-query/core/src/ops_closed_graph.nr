//! One named graph per credential, named by its canonical credential-scoped blank node.
//! The authenticated profile constrains credential order through the suite adapter.
use crate::table::{TermCell, Table, empty, append, canonicalize, check_table, assert_table, same_term};
use crate::ops_closed_common::{get, project_row};
pub fn seed<let C: u32, let R: u32, let V: u32>(
    context: Table<C, V>, credential_count: u32, credential: u32,
    name: TermCell, capture: u32, constant: TermCell, out: Table<R, V>,
) {
    check_table(context); assert(context.len <= 1, "capture cardinality");
    crate::table::check_cell(name); crate::table::check_cell(constant);
    assert(name.bound & (name.kind == 2), "graph name");
    let captured = get(context.rows[0], capture);
    let selected = if captured.bound {captured} else {constant};
    let active = (context.len > 0) & (credential < credential_count) & (!selected.bound | same_term(selected, name));
    assert_table(append(empty(), context.rows[0], active), out);
}
pub fn graph<let A: u32, let C: u32, let R: u32, let VA: u32, let VC: u32, let V: u32>(
    input: Table<A, VA>, context: Table<C, VC>, columns: [u32; V], target: u32, capture: u32,
    name: TermCell, out: Table<R, V>,
) {
    check_table(input); check_table(context); crate::table::check_cell(name);
    assert(name.bound & (name.kind == 2), "graph name");
    assert(context.len <= 1, "capture cardinality"); assert(target <= V, "graph column");
    let mut expected = empty();
    let captured = get(context.rows[0], capture);
    for i in 0..A {
        let mut row = project_row(input.rows[i], columns);
        let mut matches = (i < input.len) & (context.len > 0);
        for v in 0..V {if target == v {
            matches = matches & (!row[v].bound | same_term(row[v], name));
            if !captured.bound {row[v] = name;}
        }}
        expected = append(expected, row, matches);
    }
    assert_table(canonicalize(expected), out);
}
