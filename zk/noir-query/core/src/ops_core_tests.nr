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

#[test(should_fail_with = "operator length")]
fn unbound_row_cannot_be_omitted() {
    ops_core::values([[unbound()]], empty::<2, 1>());
}
#[test(should_fail_with = "operator length")]
fn unbound_row_cannot_be_added() {
    ops_core::values::<0, 2, 1>([], Table {rows: [[unbound()]; 2], len: 1});
}
#[test(should_fail_with = "operator rows")]
fn equal_cardinality_does_not_allow_changed_values() {
    ops_core::values([[iri(1)], [iri(2)]], Table {rows: [[iri(1)], [iri(3)]], len: 2});
}
#[test(should_fail_with = "operator capacity")]
fn overflow_cannot_release_a_plausible_prefix() {
    ops_core::values([[iri(1)], [iri(2)], [iri(3)]], Table {rows: [[iri(1)], [iri(2)]], len: 2});
}
#[test(should_fail_with = "table padding")]
fn table_padding_is_checked_independently() {
    crate::table::check_table(Table {rows: [[iri(1)]], len: 0});
}
#[test(should_fail_with = "table order")]
fn unordered_input_cannot_be_hidden_by_projection_sorting() {
    ops_core::project(Table {rows: [[iri(2)], [iri(1)]], len: 2}, [0], Table {rows: [[iri(1)], [iri(2)]], len: 2});
}
#[test(should_fail_with = "table capacity")]
fn table_capacity_is_checked_independently() {
    crate::table::check_table(Table {rows: [[iri(1)]], len: 2});
}
#[test(should_fail_with = "unbound padding")]
fn unbound_cell_has_no_payload() {
    let mut cell = unbound(); cell.enc = 1;
    ops_core::values([[cell]], Table {rows: [[cell]], len: 1});
}
#[test(should_fail_with = "term kind")]
fn term_kind_is_constrained() {
    let mut cell = iri(1); cell.kind = 4;
    ops_core::values([[cell]], Table {rows: [[cell]], len: 1});
}
#[test(should_fail_with = "term scope")]
fn nonblank_scope_is_constrained() {
    let mut cell = iri(1); cell.scope = 1;
    ops_core::values([[cell]], Table {rows: [[cell]], len: 1});
}
#[test]
fn unit_uses_a_canonical_empty_schema_column() {
    ops_core::unit(Table {rows: [[unbound()]; 2], len: 1});
}
#[test(should_fail_with = "operator length")]
fn unit_cannot_be_missing() { ops_core::unit(empty::<2, 1>()); }
#[test(should_fail_with = "row request capacity")]
fn request_row_bound_applies_within_a_larger_bucket() { ops_core::check_bound(two(), 1); }
#[test(should_fail_with = "row request bound")]
fn request_row_bound_cannot_be_empty() { ops_core::check_bound(two(), 0); }
#[test(should_fail_with = "row request bound")]
fn request_row_bound_cannot_exceed_the_bucket() { ops_core::check_bound(two(), 5); }
#[test(should_fail_with = "parameter shape")]
fn parameter_count_cannot_exceed_its_shape() { ops_core::check_parameters([iri(1)], 2); }
#[test(should_fail_with = "public constant")]
fn parameter_cannot_be_unbound() { ops_core::check_parameters([unbound()], 1); }
#[test(should_fail_with = "public constant")]
fn parameter_cannot_carry_a_credential_scope() {
    let mut cell = iri(1); cell.kind = 2; cell.scope = 1;
    ops_core::check_parameters([cell], 1);
}
#[test(should_fail_with = "parameter padding")]
fn unused_parameter_is_canonical_padding() { ops_core::check_parameters([iri(1)], 0); }
#[test(should_fail_with = "index shape")]
fn index_count_cannot_exceed_its_shape() { ops_core::check_indices([0], 2); }
#[test(should_fail_with = "index padding")]
fn unused_index_is_zero() { ops_core::check_indices([1], 0); }

fn credential_quad() -> [TermCell; 4] { [iri(1), iri(2), iri(3), unbound()] }
#[test(should_fail_with = "credential capacity")]
fn credential_opening_count_is_bounded() { crate::table::open_credential([credential_quad()], 2, 0); }
#[test(should_fail_with = "credential padding")]
fn credential_opening_cannot_hide_quads_in_padding() { crate::table::open_credential([credential_quad()], 0, 0); }
#[test(should_fail_with = "triple bound")]
fn credential_opening_requires_bound_triple_positions() {
    let mut quad = credential_quad(); quad[2] = unbound(); crate::table::open_credential([quad], 1, 0);
}
#[test(should_fail_with = "triple positions")]
fn credential_opening_requires_an_rdf_subject() {
    let mut quad = credential_quad(); quad[0].kind = 3; crate::table::open_credential([quad], 1, 0);
}
#[test(should_fail_with = "triple positions")]
fn credential_opening_requires_an_iri_predicate() {
    let mut quad = credential_quad(); quad[1].kind = 2; quad[1].scope = 1; crate::table::open_credential([quad], 1, 0);
}
#[test(should_fail_with = "credential named graph not yet supported")]
fn credential_opening_refuses_named_graphs() {
    let mut quad = credential_quad(); quad[3] = iri(4); crate::table::open_credential([quad], 1, 0);
}
#[test(should_fail_with = "credential blank-node scope")]
fn credential_opening_binds_blank_nodes_to_the_credential() {
    let mut quad = credential_quad(); quad[0].kind = 2; quad[0].scope = 2; crate::table::open_credential([quad], 1, 0);
}
#[test(should_fail_with = "credential request bound")]
fn dataset_credential_bound_is_positive() {
    let _dataset = ops_core::open_dataset::<1,1,1>([[[unbound();4]]], [0], 0, [0,1,2]);
}
#[test(should_fail_with = "credential request bound")]
fn dataset_credential_bound_fits_the_shape() {
    let _dataset = ops_core::open_dataset::<1,1,1>([[[unbound();4]]], [0], 0, [2,1,2]);
}
#[test(should_fail_with = "quad request bound")]
fn dataset_quad_bound_is_positive() {
    let _dataset = ops_core::open_dataset::<1,1,1>([[[unbound();4]]], [0], 0, [1,0,2]);
}
#[test(should_fail_with = "quad request bound")]
fn dataset_quad_bound_fits_the_shape() {
    let _dataset = ops_core::open_dataset::<1,1,1>([[[unbound();4]]], [0], 0, [1,2,2]);
}
#[test(should_fail_with = "credential count")]
fn dataset_cannot_claim_extra_credentials() {
    let _dataset = ops_core::open_dataset::<1,1,1>([[[unbound();4]]], [0], 2, [1,1,2]);
}
#[test(should_fail_with = "quad count")]
fn dataset_enforces_the_request_quad_bound() {
    let _dataset = ops_core::open_dataset::<1,2,2>([[credential_quad();2]], [2], 1, [1,1,2]);
}
#[test(should_fail_with = "credential padding count")]
fn dataset_cannot_activate_an_absent_credential() {
    let _dataset = ops_core::open_dataset::<1,1,1>([[credential_quad()]], [1], 0, [1,1,2]);
}
#[test]
fn filter_keeps_only_non_error_true_roots() {
    ops_core::filter(two(), [ops_core::Boolean {value:true,is_error:true};4], empty::<4,1>());
}
#[test]
fn slice_handles_an_offset_beyond_the_input() { ops_core::slice(two(), 100, false, 0, empty::<4,1>()); }

fn expression_cell(class: u8, key: Field, enc: Field) -> TermCell {
    TermCell {bound:true, kind:3, enc, key_class:class, key, dt:0, lang:0, scope:0}
}
fn expr_step(op: u8, a: u32, b: u32, parameter: u32, comparison: u8) -> crate::ops_core_expr::Step {
    crate::ops_core_expr::Step {op, a, b, parameter, comparison}
}
#[test]
fn expression_boolean_connectives_absorb_only_the_specified_errors() {
    let input = Table {rows:[[unbound()]],len:1};
    let truth = expression_cell(5,1,7);
    let falsity = expression_cell(5,0,8);
    let steps_or = [expr_step(0,0,0,0,0),expr_step(1,0,0,0,0),expr_step(4,0,1,0,0)];
    let result_or = crate::ops_core_expr::evaluate(input,[truth],steps_or);
    assert(result_or[0].value & !result_or[0].is_error);
    let result_error = crate::ops_core_expr::evaluate(input,[falsity],steps_or);
    assert(result_error[0].is_error);
    let steps_and = [expr_step(0,0,0,0,0),expr_step(1,0,0,0,0),expr_step(3,0,1,0,0)];
    let result_and = crate::ops_core_expr::evaluate(input,[falsity],steps_and);
    assert(!result_and[0].value & !result_and[0].is_error);
    let result_error_and = crate::ops_core_expr::evaluate(input,[truth],steps_and);
    assert(result_error_and[0].is_error);
    let negated = crate::ops_core_expr::evaluate(input,[truth],[expr_step(0,0,0,0,0),expr_step(5,0,0,0,0)]);
    assert(negated[0].is_error);
}
#[test(should_fail_with = "expression error lane")]
fn expression_error_lane_cannot_be_forced_low() {
    let result = crate::ops_core_expr::evaluate(Table {rows:[[unbound()]],len:1},[unbound()],[expr_step(0,0,0,0,0)]);
    assert(!result[0].is_error,"expression error lane");
}
#[test]
fn expression_key_comparison_includes_boundaries() {
    for class in [1,3,4,5,6] {
        let a=expression_cell(class,2,11); let b=expression_cell(class,3,12);
        for op in 0..5 {
            let unequal=crate::ops_core_expr::evaluate(Table {rows:[[a]],len:1},[b],
                [expr_step(0,0,0,0,0),expr_step(1,0,0,0,0),expr_step(2,0,1,0,op as u8)]);
            assert(unequal[0].value == ((op==1) | (op==2)));
            assert(!unequal[0].is_error);
            let equal=crate::ops_core_expr::evaluate(Table {rows:[[a]],len:1},[a],
                [expr_step(0,0,0,0,0),expr_step(1,0,0,0,0),expr_step(2,0,1,0,op as u8)]);
            assert(equal[0].value == ((op==0) | (op==2) | (op==4)));
            assert(!equal[0].is_error);
        }
    }
}
fn expression_compare_case(a: TermCell, b: TermCell, comparison: u8) {
    let _result=crate::ops_core_expr::evaluate(Table {rows:[[a]],len:1},[b],
        [expr_step(0,0,0,0,0),expr_step(1,0,0,0,0),expr_step(2,0,1,0,comparison)]);
}
#[test(should_fail_with = "literal comparison range")]
fn expression_comparison_refuses_unrepresentable_keys() { expression_compare_case(expression_cell(0x7f,0,1),expression_cell(1,2,2),0); }
#[test(should_fail_with = "floating-point comparison not yet supported")]
fn expression_comparison_refuses_the_pending_float_lane() { expression_compare_case(expression_cell(2,0,1),expression_cell(1,2,2),0); }
#[test(should_fail_with = "mixed timezone comparison not yet supported")]
fn expression_comparison_refuses_mixed_timezone_classes() { expression_compare_case(expression_cell(3,0,1),expression_cell(4,2,2),1); }
#[test(should_fail_with = "literal equality without a supported value key")]
fn expression_comparison_refuses_pending_literal_equality() { expression_compare_case(expression_cell(0,0,1),expression_cell(0,0,1),0); }
#[test(should_fail_with = "string comparison capacity")]
fn expression_comparison_refuses_prefix_only_ordering() { expression_compare_case(expression_cell(7,1,1),expression_cell(7,2,2),1); }
#[test]
fn expression_string_prefix_class_has_exact_identity_equality() {
    expression_compare_case(expression_cell(7,1,1),expression_cell(7,1,1),0);
    let result=crate::ops_core_expr::evaluate(Table {rows:[[expression_cell(7,0,1)]],len:1},[unbound()],[expr_step(0,0,0,0,0)]);
    assert(result[0].value & !result[0].is_error);
}
#[test(should_fail_with = "literal value range")]
fn expression_ebv_refuses_unrepresentable_values() {
    let _result=crate::ops_core_expr::evaluate(Table {rows:[[expression_cell(0x7f,0,1)]],len:1},[unbound()],[expr_step(0,0,0,0,0)]);
}
#[test(should_fail_with = "floating-point EBV not yet supported")]
fn expression_ebv_refuses_the_pending_float_lane() {
    let _result=crate::ops_core_expr::evaluate(Table {rows:[[expression_cell(2,0,1)]],len:1},[unbound()],[expr_step(0,0,0,0,0)]);
}
#[test(should_fail_with = "expression parameter")]
fn expression_parameter_reference_is_bounded() {
    let _result=crate::ops_core_expr::evaluate(Table {rows:[[unbound()]],len:1},[unbound()],[expr_step(1,0,0,1,0)]);
}
#[test(should_fail_with = "expression topology")]
fn expression_cannot_reference_a_future_step() {
    let _result=crate::ops_core_expr::evaluate(Table {rows:[[unbound()]],len:1},[unbound()],[expr_step(5,0,0,0,0)]);
}
#[test(should_fail_with = "expression opcode")]
fn expression_refuses_unknown_instructions() {
    let _result=crate::ops_core_expr::evaluate(Table {rows:[[unbound()]],len:1},[unbound()],
        [expr_step(7,0,0,0,0),expr_step(6,0,0,0,0)]);
}

#[test(should_fail_with = "operator length")]
fn unit_cannot_be_added() { ops_core::unit(Table {rows: [[unbound()]; 2],len:2}); }
#[test]
fn numeric_value_equality_is_independent_of_lexical_identity() {
    for class in [1,3,4,5] {
        let result=crate::ops_core_expr::evaluate(Table{rows:[[expression_cell(class,1,11)]],len:1},[expression_cell(class,1,12)],
            [expr_step(0,0,0,0,0),expr_step(1,0,0,0,0),expr_step(2,0,1,0,0)]);
        assert(result[0].value & !result[0].is_error);
    }
}
#[test(should_fail_with = "comparison instruction")]
fn expression_refuses_unknown_comparison_instructions() { expression_compare_case(expression_cell(1,1,1),expression_cell(1,1,2),5); }

#[test(should_fail)]
fn statement_select_cannot_omit_the_binding() {
    crate::ops_core_statement::bind_select([7;32],[0;32],0,[1,4,2],[unbound()],[0],empty::<2,1>(),0);
}
#[test(should_fail)]
fn statement_ask_cannot_omit_the_binding() {
    crate::ops_core_statement::bind_ask([7;32],[0;32],0,[1,4,2],[unbound()],[0],true,0);
}
#[test(should_fail)]
fn statement_dataset_commitment_cannot_be_changed() {
    crate::ops_core_statement::check_dataset([[[unbound();4];4];1],[0],0,[3;32],[0;32],0);
}

#[test]
fn statement_empty_dataset_matches_the_independent_sha_vector() {
    crate::ops_core_statement::check_dataset([[[unbound();4];4];1],[0],0,[3;32],[58,190,209,124,208,8,157,55,127,255,87,201,64,246,64,65,149,26,209,178,202,85,130,158,238,152,142,98,146,119,20,96],0);
}
#[test(should_fail)]
fn statement_supplied_cells_cannot_claim_a_signature_mode() {
    crate::ops_core_statement::check_dataset([[[unbound();4];4];1],[0],0,[3;32],[58,190,209,124,208,8,157,55,127,255,87,201,64,246,64,65,149,26,209,178,202,85,130,158,238,152,142,98,146,119,20,96],1);
}

#[test(should_fail)]
fn scan_column_must_belong_to_the_schema() {
    ops_core::scan([[unbound();3]],[false],[2,1,1],[unbound();3],empty::<2,1>());
}
#[test(should_fail)]
fn join_column_must_belong_to_the_schema() {
    ops_core::join(empty::<2,1>(),empty::<2,1>(),[2],[0],empty::<2,1>());
}
#[test(should_fail)]
fn projection_column_must_belong_to_the_schema() {
    ops_core::project(empty::<2,1>(),[2],empty::<2,1>());
}
#[test(should_fail)]
fn public_blank_node_constant_cannot_have_a_credential_scope() {
    let cell=TermCell{bound:true,kind:2,enc:1,key_class:0,key:0,dt:0,lang:0,scope:1};
    ops_core::check_parameters([cell],1);
}
#[test(should_fail)]
fn term_kind_cannot_be_the_padding_kind_when_bound() {
    crate::table::check_cell(TermCell{bound:true,kind:0,enc:1,key_class:0,key:0,dt:0,lang:0,scope:0});
}

#[test(should_fail)]
fn expression_refuses_mixed_temporal_datatypes() {
    let a=expression_cell(3,1,1);
    let b=TermCell{bound:true,kind:3,enc:2,key_class:3,key:1,dt:2,lang:0,scope:0};
    expression_compare_case(a,b,0);
}
#[test(should_fail)]
fn expression_refuses_language_tagged_string_ordering() {
    let a=expression_cell(6,1,1);
    let b=TermCell{bound:true,kind:3,enc:2,key_class:6,key:2,dt:0,lang:1,scope:0};
    expression_compare_case(a,b,1);
}

#[test(should_fail)]
fn overflow_cannot_be_hidden_by_a_full_truncated_output() {
    ops_core::values([[iri(1)],[iri(2)],[iri(3)]],Table {rows:[[iri(1)],[iri(2)]],len:2});
}
#[test]
fn slice_public_indices_do_not_wrap() {
    ops_core::slice(two(),0xffffffff,true,0xffffffff,empty::<4,1>());
    ops_core::slice(two(),0,true,0xffffffff,two());
}
