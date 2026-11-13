use crate::table::{TermCell, Table, unbound, empty};
use crate::ops_core;

fn iri(n: Field) -> TermCell {
    TermCell { bound: true, kind: 1, enc: n, key_class: 0, key: 0, dt: 0, lang: 0, scope: 0 }
}

#[test]
fn values_honest() { ops_core::values([[iri(2)], [iri(1)]], Table { rows: [[iri(1)], [iri(2)]], len: 2 }); }

#[test(should_fail_with = "operator length")]
fn values_missing() { ops_core::values([[iri(1)], [iri(2)]], Table { rows: [[iri(1)], [unbound()]], len: 1 }); }

#[test(should_fail_with = "operator length")]
fn values_extra() { ops_core::values([[iri(1)]], Table { rows: [[iri(1)], [iri(2)]], len: 2 }); }

#[test(should_fail_with = "table order")]
fn values_reordered() { ops_core::values([[iri(1)], [iri(2)]], Table { rows: [[iri(2)], [iri(1)]], len: 2 }); }

#[test(should_fail_with = "table padding")]
fn values_padding() { ops_core::values([[iri(1)]], Table { rows: [[iri(1)], [iri(2)]], len: 1 }); }

#[test(should_fail_with = "operator capacity")]
fn values_overflow() { ops_core::values([[iri(1)], [iri(2)], [iri(3)]], empty::<2,1>()); }

fn two() -> Table<4, 1> { Table { rows: [[iri(1)], [iri(2)], [unbound()], [unbound()]], len: 2 } }
fn one() -> Table<4, 1> { Table { rows: [[iri(1)], [unbound()], [unbound()], [unbound()]], len: 1 } }

#[test]
fn scan_honest() { ops_core::scan([[iri(1),iri(9),iri(10)],[iri(2),iri(9),iri(10)],[iri(1),iri(9),iri(10)]],[true,true,true],[0,1,1],[unbound(),iri(9),iri(10)],two()); }

#[test(should_fail_with = "operator length")]
fn scan_missing() { ops_core::scan([[iri(1),iri(9),iri(10)],[iri(2),iri(9),iri(10)],[iri(1),iri(9),iri(10)]],[true,true,true],[0,1,1],[unbound(),iri(9),iri(10)],one()); }

#[test(should_fail_with = "operator length")]
fn scan_extra() { ops_core::scan([[iri(1),iri(9),iri(10)],[iri(2),iri(9),iri(10)],[iri(1),iri(9),iri(10)]],[true,true,true],[0,1,1],[unbound(),iri(9),iri(10)],Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:3}); }

#[test(should_fail_with = "table order")]
fn scan_reordered() { ops_core::scan([[iri(1),iri(9),iri(10)],[iri(2),iri(9),iri(10)],[iri(1),iri(9),iri(10)]],[true,true,true],[0,1,1],[unbound(),iri(9),iri(10)],Table {rows:[[iri(2)],[iri(1)],[unbound()],[unbound()]],len:2}); }

#[test(should_fail_with = "table padding")]
fn scan_padding() { ops_core::scan([[iri(1),iri(9),iri(10)],[iri(2),iri(9),iri(10)],[iri(1),iri(9),iri(10)]],[true,true,true],[0,1,1],[unbound(),iri(9),iri(10)],Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:2}); }

#[test]
fn join_honest() { ops_core::join(two(),Table {rows:[[]],len:1},[0],[0],two()); }

#[test(should_fail_with = "operator length")]
fn join_missing() { ops_core::join(two(),Table {rows:[[]],len:1},[0],[0],one()); }

#[test(should_fail_with = "operator length")]
fn join_extra() { ops_core::join(two(),Table {rows:[[]],len:1},[0],[0],Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:3}); }

#[test(should_fail_with = "table order")]
fn join_reordered() { ops_core::join(two(),Table {rows:[[]],len:1},[0],[0],Table {rows:[[iri(2)],[iri(1)],[unbound()],[unbound()]],len:2}); }

#[test(should_fail_with = "table padding")]
fn join_padding() { ops_core::join(two(),Table {rows:[[]],len:1},[0],[0],Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:2}); }

#[test]
fn filter_honest() { ops_core::filter(two(),[ops_core::Boolean {value:true,is_error:false};4],two()); }

#[test(should_fail_with = "operator length")]
fn filter_missing() { ops_core::filter(two(),[ops_core::Boolean {value:true,is_error:false};4],one()); }

#[test(should_fail_with = "operator length")]
fn filter_extra() { ops_core::filter(two(),[ops_core::Boolean {value:true,is_error:false};4],Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:3}); }

#[test(should_fail_with = "table order")]
fn filter_reordered() { ops_core::filter(two(),[ops_core::Boolean {value:true,is_error:false};4],Table {rows:[[iri(2)],[iri(1)],[unbound()],[unbound()]],len:2}); }

#[test(should_fail_with = "table padding")]
fn filter_padding() { ops_core::filter(two(),[ops_core::Boolean {value:true,is_error:false};4],Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:2}); }

#[test]
fn project_honest() { ops_core::project(two(),[0],two()); }

#[test(should_fail_with = "operator length")]
fn project_missing() { ops_core::project(two(),[0],one()); }

#[test(should_fail_with = "operator length")]
fn project_extra() { ops_core::project(two(),[0],Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:3}); }

#[test(should_fail_with = "table order")]
fn project_reordered() { ops_core::project(two(),[0],Table {rows:[[iri(2)],[iri(1)],[unbound()],[unbound()]],len:2}); }

#[test(should_fail_with = "table padding")]
fn project_padding() { ops_core::project(two(),[0],Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:2}); }

#[test]
fn distinct_honest() { ops_core::distinct(Table {rows:[[iri(1)],[iri(1)],[iri(2)],[unbound()]],len:3},two()); }

#[test(should_fail_with = "operator length")]
fn distinct_missing() { ops_core::distinct(Table {rows:[[iri(1)],[iri(1)],[iri(2)],[unbound()]],len:3},one()); }

#[test(should_fail_with = "operator length")]
fn distinct_extra() { ops_core::distinct(Table {rows:[[iri(1)],[iri(1)],[iri(2)],[unbound()]],len:3},Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:3}); }

#[test(should_fail_with = "table order")]
fn distinct_reordered() { ops_core::distinct(Table {rows:[[iri(1)],[iri(1)],[iri(2)],[unbound()]],len:3},Table {rows:[[iri(2)],[iri(1)],[unbound()],[unbound()]],len:2}); }

#[test(should_fail_with = "table padding")]
fn distinct_padding() { ops_core::distinct(Table {rows:[[iri(1)],[iri(1)],[iri(2)],[unbound()]],len:3},Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:2}); }

#[test]
fn slice_honest() { ops_core::slice(two(),0,true,2,two()); }

#[test(should_fail_with = "operator length")]
fn slice_missing() { ops_core::slice(two(),0,true,2,one()); }

#[test(should_fail_with = "operator length")]
fn slice_extra() { ops_core::slice(two(),0,true,2,Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:3}); }

#[test(should_fail_with = "table order")]
fn slice_reordered() { ops_core::slice(two(),0,true,2,Table {rows:[[iri(2)],[iri(1)],[unbound()],[unbound()]],len:2}); }

#[test(should_fail_with = "table padding")]
fn slice_padding() { ops_core::slice(two(),0,true,2,Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:2}); }

#[test]
fn select_honest() { ops_core::select(two(),two()); }

#[test(should_fail_with = "operator length")]
fn select_missing() { ops_core::select(two(),one()); }

#[test(should_fail_with = "operator length")]
fn select_extra() { ops_core::select(two(),Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:3}); }

#[test(should_fail_with = "table order")]
fn select_reordered() { ops_core::select(two(),Table {rows:[[iri(2)],[iri(1)],[unbound()],[unbound()]],len:2}); }

#[test(should_fail_with = "table padding")]
fn select_padding() { ops_core::select(two(),Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:2}); }

#[test(should_fail_with = "operator capacity")]
fn scan_overflow() { ops_core::scan([[iri(1),iri(9),iri(10)],[iri(2),iri(9),iri(10)],[iri(3),iri(9),iri(10)]],[true;3],[0,1,1],[unbound(),iri(9),iri(10)],empty::<2,1>()); }

#[test(should_fail_with = "operator capacity")]
fn join_overflow() { ops_core::join(two(),two(),[0,1],[1,0],empty::<2,2>()); }

#[test(should_fail_with = "operator capacity")]
fn filter_overflow() { ops_core::filter(two(),[ops_core::Boolean {value:true,is_error:false};4],empty::<1,1>()); }

#[test(should_fail_with = "operator capacity")]
fn project_overflow() { ops_core::project(two(),[0],empty::<1,1>()); }

#[test(should_fail_with = "operator capacity")]
fn distinct_overflow() { ops_core::distinct(two(),empty::<1,1>()); }

#[test(should_fail_with = "operator capacity")]
fn slice_overflow() { ops_core::slice(two(),0,false,0,empty::<1,1>()); }

#[test(should_fail_with = "table capacity")]
fn select_overflow() { ops_core::select(two(),Table {rows:[[iri(1)],[iri(2)],[unbound()],[unbound()]],len:5}); }

#[test]
fn ask_honest() { ops_core::ask(two(), true); ops_core::ask(empty::<4,1>(), false); }
#[test(should_fail_with = "ASK result")]
fn ask_missing() { ops_core::ask(two(), false); }
#[test(should_fail_with = "ASK result")]
fn ask_extra() { ops_core::ask(empty::<4,1>(), true); }
#[test(should_fail_with = "table order")]
fn ask_reordered() { ops_core::ask(Table {rows:[[iri(2)],[iri(1)]],len:2},true); }
#[test(should_fail_with = "table padding")]
fn ask_padding() { ops_core::ask(Table {rows:[[iri(1)],[iri(2)]],len:1},true); }
#[test(should_fail_with = "table capacity")]
fn ask_overflow() { ops_core::ask(Table {rows:[[iri(1)],[iri(2)]],len:3},true); }
#[test]
fn repeated_pattern_variable_and_blank_scope() {
    ops_core::scan([[iri(1),iri(9),iri(2)]],[true],[0,1,0],[unbound(),iri(9),unbound()],empty::<2,1>());
    let mut a = iri(1); a.kind = 2; a.scope = 1;
    let mut b = a; b.scope = 2;
    ops_core::join(Table{rows:[[a]],len:1},Table{rows:[[b]],len:1},[0],[0],empty::<2,1>());
}
#[test]
fn values_unbound_join_compatibility_preserves_multiplicity() {
    ops_core::join(Table{rows:[[unbound()],[iri(1)]],len:2},Table{rows:[[iri(1)]],len:1},[0],[0],Table{rows:[[iri(1)],[iri(1)]],len:2});
}

#[test]
fn select_blank_nodes_are_result_local() {
    let mut private = iri(42); private.kind = 2; private.scope = 1;
    let mut public = iri(1); public.kind = 2;
    ops_core::select(Table{rows:[[private]],len:1},Table{rows:[[public]],len:1});
}

#[test(should_fail_with = "operator rows")]
fn select_rejects_credential_scoped_result_cells() {
    let mut private = iri(42); private.kind = 2; private.scope = 1;
    ops_core::select(Table{rows:[[private]],len:1},Table{rows:[[private]],len:1});
}
