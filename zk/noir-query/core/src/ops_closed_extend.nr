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

// Result encoding is delegated to the shared expression library and suite.
pub struct SuiteEncoder {}
impl crate::expr_to_term::ComputedTermEncoder for SuiteEncoder {
    fn literal(lexical:std::collections::bounded_vec::BoundedVec<u8,64>,datatype:std::collections::bounded_vec::BoundedVec<u8,64>,key:[u8;31])->crate::expr_term_contract::TermCell {
        let c=crate::term::open_term(3,lexical,datatype,std::collections::bounded_vec::BoundedVec::<u8,0>::new(),key,0);
        crate::expr_term_contract::TermCell{bound:c.bound(),kind:c.kind(),enc:c.enc(),key_class:c.key_class(),key:c.key(),dt:c.datatype(),lang:c.language(),credential:c.credential()}
    }
}
pub fn table_cell(c:crate::expr_term_contract::TermCell)->TermCell {
    TermCell{bound:c.bound,kind:c.kind,enc:c.enc,key_class:c.key_class,key:c.key,dt:c.dt,lang:c.lang,scope:c.credential}
}
pub fn boolean<let A:u32,let VA:u32,let R:u32,let V:u32>(input:Table<A,VA>,columns:[u32;V],target:u32,predicates:[crate::ops_core::Boolean;A],out:Table<R,V>){
    check_table(input);assert(target<V,"BIND column");let mut expected=empty();
    for i in 0..A {if i<input.len {
        let (value,_error)=crate::expr_to_term::boolean::<SuiteEncoder>(crate::expr_core::truth(predicates[i].value,predicates[i].is_error));
        assert(!value.capacity,"BIND capacity");let mut row=project_row(input.rows[i],columns);assert(!row[target].bound,"BIND target already bound");row[target]=table_cell(value.value);
        expected=append(expected,row,true);
    }}
    assert_table(canonicalize(expected),out);
}
