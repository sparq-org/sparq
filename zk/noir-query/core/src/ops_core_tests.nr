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

#[test(should_fail_with = "scan completeness or capacity")]
fn scan_missing() { ops_core::scan([[iri(1),iri(9),iri(10)],[iri(2),iri(9),iri(10)],[iri(1),iri(9),iri(10)]],[true,true,true],[0,1,1],[unbound(),iri(9),iri(10)],one()); }

#[test(should_fail_with = "scan extra binding")]
fn scan_extra() { ops_core::scan([[iri(1),iri(9),iri(10)],[iri(2),iri(9),iri(10)],[iri(1),iri(9),iri(10)]],[true,true,true],[0,1,1],[unbound(),iri(9),iri(10)],Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:3}); }

#[test(should_fail_with = "table order")]
fn scan_reordered() { ops_core::scan([[iri(1),iri(9),iri(10)],[iri(2),iri(9),iri(10)],[iri(1),iri(9),iri(10)]],[true,true,true],[0,1,1],[unbound(),iri(9),iri(10)],Table {rows:[[iri(2)],[iri(1)],[unbound()],[unbound()]],len:2}); }

#[test(should_fail_with = "table padding")]
fn scan_padding() { ops_core::scan([[iri(1),iri(9),iri(10)],[iri(2),iri(9),iri(10)],[iri(1),iri(9),iri(10)]],[true,true,true],[0,1,1],[unbound(),iri(9),iri(10)],Table {rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:2}); }

#[test]
fn join_honest() { ops_core::join(two(),Table {rows:[[]],len:1},[0],[0],two()); }

#[test(should_fail_with = "join completeness or capacity")]
fn join_missing() { ops_core::join(two(),Table {rows:[[]],len:1},[0],[0],one()); }

#[test(should_fail_with = "join extra binding")]
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

#[test(should_fail_with = "scan completeness or capacity")]
fn scan_overflow() { ops_core::scan([[iri(1),iri(9),iri(10)],[iri(2),iri(9),iri(10)],[iri(3),iri(9),iri(10)]],[true;3],[0,1,1],[unbound(),iri(9),iri(10)],empty::<2,1>()); }

#[test(should_fail_with = "join completeness or capacity")]
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

#[test(should_fail_with = "canonical permutation")]
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
fn credential_opening_count_is_bounded() { crate::table::check_credential_cells([credential_quad()], 2, 0); }
#[test(should_fail_with = "credential padding")]
fn credential_opening_cannot_hide_quads_in_padding() { crate::table::check_credential_cells([credential_quad()], 0, 0); }
#[test(should_fail_with = "triple bound")]
fn credential_opening_requires_bound_triple_positions() {
    let mut quad = credential_quad(); quad[2] = unbound(); crate::table::check_credential_cells([quad], 1, 0);
}
#[test(should_fail_with = "triple positions")]
fn credential_opening_requires_an_rdf_subject() {
    let mut quad = credential_quad(); quad[0].kind = 3; crate::table::check_credential_cells([quad], 1, 0);
}
#[test(should_fail_with = "triple positions")]
fn credential_opening_requires_an_iri_predicate() {
    let mut quad = credential_quad(); quad[1].kind = 2; quad[1].scope = 1; crate::table::check_credential_cells([quad], 1, 0);
}
#[test(should_fail_with = "credential named graph not yet supported")]
fn credential_opening_refuses_named_graphs() {
    let mut quad = credential_quad(); quad[3] = iri(4); crate::table::check_credential_cells([quad], 1, 0);
}
#[test(should_fail_with = "credential blank-node scope")]
fn credential_opening_binds_blank_nodes_to_the_credential() {
    let mut quad = credential_quad(); quad[0].kind = 2; quad[0].scope = 2; crate::table::check_credential_cells([quad], 1, 0);
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
    let payload = if class == 1 {
        0x800000000000000000000000000000000000000000000000000000000000 + key * 1000000000000000000
    } else if (class == 5) | (class == 6) {
        key * 0x10000000000000000000000000000000000000000000000000000000000
    } else if (class == 3) | (class == 4) | (class == 8) {
        key * 0x100000000000000000000000000000000000000000000
    } else { key };
    TermCell {bound:true, kind:3, enc, key_class:class, key: payload + class as Field * 0x1000000000000000000000000000000000000000000000000000000000000, dt:0, lang:0, scope:0}
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
    for class in [1,3,4,5,6,8] {
        let a=expression_cell(class,if class == 5 {0} else {2},11); let b=expression_cell(class,if class == 5 {1} else {3},12);
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
        [expr_step(7,0,0,0,0),expr_step(9,0,0,0,0)]);
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

#[test(should_fail_with = "row request bound")]
fn request_row_bound_is_positive_even_for_an_empty_table() {
    ops_core::check_bound(empty::<1,1>(),0);
}

#[test]
fn expression_effective_boolean_value_uses_the_suite_key_prefix() {
    let zero: Field = 0x1800000000000000000000000000000000000000000000000000000000000;
    let mut numeric=expression_cell(1,0,11);numeric.key=zero;
    let result=crate::ops_core_expr::evaluate(Table{rows:[[numeric]],len:1},[unbound()],[expr_step(0,0,0,0,0)]);
    assert(!result[0].value & !result[0].is_error);
    numeric.key=zero+1;
    let result=crate::ops_core_expr::evaluate(Table{rows:[[numeric]],len:1},[unbound()],[expr_step(0,0,0,0,0)]);
    assert(result[0].value & !result[0].is_error);
    for class in [5,6] {
        let cell=expression_cell(class,0,11);
        let result=crate::ops_core_expr::evaluate(Table{rows:[[cell]],len:1},[unbound()],[expr_step(0,0,0,0,0)]);
        assert(!result[0].value & !result[0].is_error);
    }
}

#[test(should_fail_with = "language-tagged effective boolean value not yet supported")]
fn expression_refuses_pending_language_tagged_ebv() {
    let mut cell=expression_cell(0,0,11);cell.dt=crate::suite_constants::DT_LANGSTRING;
    let _result=crate::ops_core_expr::evaluate(Table{rows:[[cell]],len:1},[unbound()],[expr_step(0,0,0,0,0)]);
}

#[test(should_fail_with = "membership comparison instruction")]
fn expression_membership_cannot_select_a_different_relation() {
    let _ = crate::ops_core_expr::evaluate(Table {rows:[[expression_cell(1,0,1)]],len:1},
        [expression_cell(1,1,2)], [expr_step(0,0,0,0,0),expr_step(1,0,0,0,0),expr_step(6,0,1,0,1)]);
}
#[test]
fn expression_unbound_comparison_keeps_its_error() {
    let result = crate::ops_core_expr::evaluate(Table {rows:[[unbound()]],len:1},
        [expression_cell(0x7f,0,1)], [expr_step(0,0,0,0,0),expr_step(1,0,0,0,0),expr_step(2,0,1,0,0)]);
    assert(!result[0].value & result[0].is_error);
}

fn bounded_membership(value:TermCell, parameters:[TermCell;16], count:u32) -> ops_core::Boolean {
    crate::ops_core_expr::evaluate_indexed(Table {rows:[[value]],len:1},parameters,[count],
        [expr_step(0,0,0,0,0),expr_step(8,0,0,0,0)])[0]
}
#[test]
fn membership_public_length_selects_exactly_the_active_prefix() {
    let zero=expression_cell(1,0,11);let one=expression_cell(1,1,12);
    let mut parameters=[zero;16];parameters[0]=one;
    let empty=bounded_membership(zero,parameters,0);
    assert(!empty.value & !empty.is_error);
    let absent=bounded_membership(zero,parameters,1);
    assert(!absent.value & !absent.is_error);
    let present=bounded_membership(zero,parameters,2);
    assert(present.value & !present.is_error);
    let first=bounded_membership(one,parameters,1);
    assert(first.value & !first.is_error);
}
#[test]
fn membership_uses_the_final_list_slot() {
    let mut parameters=[expression_cell(1,0,11);16];
    parameters[15]=expression_cell(1,7,12);
    let result=bounded_membership(parameters[15],parameters,16);
    assert(result.value & !result.is_error);
}
#[test]
fn membership_empty_and_unbound_error_channels_are_distinct() {
    let parameters=[expression_cell(1,0,11);16];
    let empty=bounded_membership(unbound(),parameters,0);
    assert(!empty.value & !empty.is_error);
    let nonempty=bounded_membership(unbound(),parameters,1);
    assert(!nonempty.value & nonempty.is_error);
}
#[test(should_fail_with = "IN list capacity")]
fn membership_cannot_claim_more_than_its_list_bucket() {
    let _=bounded_membership(expression_cell(1,0,11),[expression_cell(1,0,11);16],17);
}
#[test(should_fail_with = "IN length parameter")]
fn membership_length_index_is_checked() {
    let _=crate::ops_core_expr::evaluate_indexed(Table {rows:[[unbound()]],len:1},[unbound();16],[0],
        [expr_step(0,0,0,0,0),expr_step(8,0,1,0,0)]);
}
#[test(should_fail_with = "IN term parameters")]
fn membership_term_range_is_checked() {
    let _=crate::ops_core_expr::evaluate_indexed(Table {rows:[[unbound()]],len:1},[unbound();16],[0],
        [expr_step(0,0,0,0,0),expr_step(8,0,0,1,0)]);
}
#[test(should_fail_with = "IN exact result")]
fn membership_cannot_release_a_match_from_inactive_padding() {
    let mut parameters=[expression_cell(1,0,11);16];parameters[0]=expression_cell(1,1,12);
    let result=bounded_membership(parameters[1],parameters,1);
    assert(result.value,"IN exact result");
}

fn mixed_string_equality(a:TermCell,b:TermCell) -> ops_core::Boolean {
    crate::ops_core_expr::evaluate(Table {rows:[[a]],len:1},[b],
        [expr_step(0,0,0,0,0),expr_step(1,0,0,0,0),expr_step(2,0,1,0,0)])[0]
}
fn plain_string_cell(class:u8) -> TermCell {
    let mut cell=expression_cell(class,1,11);cell.dt=crate::suite_constants::DT_STRING;cell
}
#[test]
fn expression_same_type_strings_compare_authenticated_identity() {
    for class in [6,7] {
        let a=plain_string_cell(class);let mut b=a;b.enc=12;
        let equal=mixed_string_equality(a,a);let different=mixed_string_equality(a,b);
        assert(equal.value & !equal.is_error,"same-type string identity equality");
        assert(!different.value & !different.is_error,"same-type string identity inequality");
    }
}
#[test]
fn expression_plain_and_language_string_equality_uses_its_type_class() {
    let mut language=expression_cell(0,0,12);
    language.dt=crate::suite_constants::DT_LANGSTRING;language.lang=7;
    for plain in [plain_string_cell(6),plain_string_cell(7)] {
        for pair in [[plain,language],[language,plain]] {
            let result=mixed_string_equality(pair[0],pair[1]);
            assert(!result.value & result.is_error);
        }
    }
}
#[test]
fn expression_string_numeric_equality_preserves_type_error() {
    for plain in [plain_string_cell(6),plain_string_cell(7)] {
        for pair in [[plain,expression_cell(1,1,12)],[expression_cell(1,1,12),plain]] {
            let result=mixed_string_equality(pair[0],pair[1]);
            assert(!result.value & result.is_error);
        }
    }
}
#[test]
fn expression_string_date_equality_preserves_type_error() {
    for plain in [plain_string_cell(6),plain_string_cell(7)] {
        let result=mixed_string_equality(plain,expression_cell(8,1,12));
        assert(!result.value & result.is_error);
    }
}
#[test]
fn expression_string_boolean_equality_preserves_type_error() {
    let result=mixed_string_equality(plain_string_cell(7),expression_cell(5,1,12));
    assert(!result.value & result.is_error);
}
#[test]
fn expression_string_iri_equality_is_false_without_error() {
    let result=mixed_string_equality(plain_string_cell(7),iri(12));
    assert(!result.value & !result.is_error);
}
#[test]
fn expression_language_string_equality_preserves_term_identity() {
    let mut a=expression_cell(0,0,11);a.dt=crate::suite_constants::DT_LANGSTRING;a.lang=7;
    let mut b=a;b.enc=12;
    let equal=mixed_string_equality(a,a);let different=mixed_string_equality(a,b);
    assert(equal.value & !equal.is_error);assert(!different.value & different.is_error);
}

#[test]
fn expression_padding_does_not_evaluate_values() {
    let huge=expression_cell(1,1000000000000000000,11);
    let result=crate::ops_core_expr::evaluate(empty::<2,1>(),[huge],[expr_step(1,0,0,0,0)]);
    assert(!result[0].value & result[0].is_error);
    assert(!result[1].value & result[1].is_error);
}
#[test(should_fail_with="expression capacity")]
fn expression_active_rows_keep_their_value_capacity() {
    let huge=expression_cell(1,1000000000000000000,11);
    let _result=crate::ops_core_expr::evaluate(Table {rows:[[unbound()]],len:1},[huge],[expr_step(1,0,0,0,0)]);
}
#[test(should_fail_with="expression opcode")]
fn expression_empty_input_still_checks_its_instruction_structure() {
    let _result=crate::ops_core_expr::evaluate(empty::<2,1>(),[unbound()],
        [expr_step(7,0,0,0,0),expr_step(9,0,0,0,0)]);
}


#[test]
fn expression_invalid_numeric_and_boolean_ebv_is_false_without_error() {
    for dt in [crate::suite_constants::DT_INTEGER,crate::suite_constants::DT_DECIMAL,
        crate::suite_constants::DT_BOOLEAN,crate::suite_constants::DT_DOUBLE,crate::suite_constants::DT_FLOAT] {
        let mut invalid=expression_cell(0,0,11);invalid.dt=dt;
        let result=crate::ops_core_expr::evaluate(Table {rows:[[invalid]],len:1},[unbound()],
            [expr_step(0,0,0,0,0),expr_step(5,0,0,0,0)])[0];
        assert(result.value & !result.is_error);
    }
}

#[test]
fn expression_temporal_key_normalizes_the_suite_payload() {
    for class in [3,4,8] {
        let a=expression_cell(class,100,11);let b=expression_cell(class,101,12);
        let result=crate::ops_core_expr::evaluate(Table {rows:[[a]],len:1},[b],
            [expr_step(0,0,0,0,0),expr_step(1,0,0,0,0),expr_step(2,0,1,0,1)]);
        assert(result[0].value & !result[0].is_error);
    }
}
#[test(should_fail_with="call to assert_max_bit_size")]
fn expression_temporal_key_tail_is_exact() {
    let mut a=expression_cell(8,100,11);a.key=a.key+1;
    let _result=mixed_string_equality(a,a);
}

#[test]
fn expression_string_prefix_with_zero_payload_is_nonempty() {
    let mut cell=expression_cell(7,0,11);cell.dt=crate::suite_constants::DT_STRING;
    let result=crate::ops_core_expr::evaluate(Table {rows:[[cell]],len:1},[unbound()],[expr_step(0,0,0,0,0)]);
    assert(result[0].value & !result[0].is_error);
}

#[test]
fn select_releases_literal_identity_without_value_metadata() {
    let mut source=iri(7);source.kind=3;source.dt=11;source.lang=13;source.key_class=1;source.key=17;
    let mut released=source;released.key_class=0;released.key=0;
    ops_core::select(Table{rows:[[source]],len:1},Table{rows:[[released]],len:1});
}
#[test(should_fail_with = "canonical payload")]
fn select_rejects_unreleased_value_metadata() {
    let mut source=iri(7);source.kind=3;source.key_class=1;source.key=17;
    ops_core::select(Table{rows:[[source]],len:1},Table{rows:[[source]],len:1});
}

#[test]
fn identity_keeps_kind_and_credential_scope() {
    let iri=iri(7);let mut blank=iri;blank.kind=2;
    assert(!crate::table::same_term(iri,blank));
    blank.scope=1;let mut other=blank;other.scope=2;assert(!crate::table::same_term(blank,other));
    other=blank;other.key=123;assert(crate::table::same_term(blank,other));
}

fn rows_literal(enc:Field,key:Field) -> TermCell {
    TermCell {bound:true,kind:3,enc,key_class:1,key,dt:5,lang:7,scope:0}
}
fn rows_project_input() -> Table<4,2> {
    Table {rows:[[iri(1),rows_literal(9,11)],[iri(2),rows_literal(9,22)],
        [iri(3),rows_literal(8,33)],[unbound();2]],len:3}
}
fn rows_project_output() -> Table<4,1> {
    Table {rows:[[rows_literal(8,33)],[rows_literal(9,11)],[rows_literal(9,22)],[unbound()]],len:3}
}
#[test]
fn rows_projection_preserves_stable_duplicate_payloads() {
    ops_core::project(rows_project_input(),[1],rows_project_output());
}
#[test(should_fail_with="canonical payload")]
fn rows_projection_refuses_swapped_duplicate_payloads() {
    let mut out=rows_project_output();let row=out.rows[1];out.rows[1]=out.rows[2];out.rows[2]=row;
    ops_core::project(rows_project_input(),[1],out);
}
#[test]
fn rows_projection_sorts_the_complete_bucket() {
    let mut input=crate::table::empty::<32,2>();let mut out=crate::table::empty::<32,1>();
    input.len=32;out.len=32;
    for i in 0..32 {input.rows[i]=[iri(i as Field+1),iri(32-i as Field)];out.rows[i]=[iri(i as Field+1)];}
    ops_core::project(input,[1],out);
}
#[test(should_fail_with="canonical permutation")]
fn rows_permutation_preserves_multiplicity() {
    crate::table::assert_canonicalized(Table{rows:[[iri(1)],[iri(1)],[iri(2)]],len:3},
        Table{rows:[[iri(1)],[iri(2)],[iri(2)]],len:3});
}
#[test(should_fail_with="canonical permutation")]
fn rows_permutation_refuses_a_changed_identity() {
    crate::table::assert_canonicalized(two(),Table{rows:[[iri(1)],[iri(3)],[unbound()],[unbound()]],len:2});
}
#[test(should_fail_with="operator length")]
fn rows_permutation_refuses_missing_rows() {crate::table::assert_canonicalized(two(),one());}
#[test(should_fail_with="operator length")]
fn rows_permutation_refuses_extra_rows() {crate::table::assert_canonicalized(one(),two());}
#[test(should_fail_with="table order")]
fn rows_permutation_checks_output_order() {
    crate::table::assert_canonicalized(two(),Table{rows:[[iri(2)],[iri(1)],[unbound()],[unbound()]],len:2});
}
#[test(should_fail_with="table padding")]
fn rows_permutation_checks_input_padding() {
    crate::table::assert_canonicalized(Table{rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:2},two());
}
#[test(should_fail_with="table padding")]
fn rows_permutation_checks_output_padding() {
    crate::table::assert_canonicalized(two(),Table{rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:2});
}
#[test]
fn rows_permutation_accepts_distinct_capacity_buckets() {
    crate::table::assert_canonicalized(Table{rows:[[iri(2)],[iri(1)]],len:2},two());
}
fn rows_scan(out:TermCell) {
    ops_core::scan([[iri(1),iri(9),rows_literal(10,11)],[iri(1),iri(9),rows_literal(10,22)]],
        [true,true],[1,1,0],[iri(1),iri(9),unbound()],Table{rows:[[out],[unbound()],[unbound()],[unbound()]],len:1});
}
#[test]
fn rows_scan_keeps_the_first_duplicate_payload() {rows_scan(rows_literal(10,11));}
#[test(should_fail_with="scan first payload")]
fn rows_scan_refuses_a_later_duplicate_payload() {rows_scan(rows_literal(10,22));}
#[test(should_fail_with="scan first payload")]
fn rows_scan_authenticates_the_key_class() {let mut out=rows_literal(10,11);out.key_class=2;rows_scan(out);}
#[test(should_fail_with="scan first payload")]
fn rows_scan_authenticates_the_key() {let mut out=rows_literal(10,11);out.key=12;rows_scan(out);}
#[test(should_fail_with="scan first payload")]
fn rows_scan_authenticates_the_datatype() {let mut out=rows_literal(10,11);out.dt=6;rows_scan(out);}
#[test(should_fail_with="scan first payload")]
fn rows_scan_authenticates_the_language() {let mut out=rows_literal(10,11);out.lang=8;rows_scan(out);}
#[test(should_fail_with="scan distinct bindings")]
fn rows_scan_refuses_duplicate_output_bindings() {
    ops_core::scan([[iri(1),iri(9),iri(10)]],[true],[0,1,1],[unbound(),iri(9),iri(10)],
        Table{rows:[[iri(1)],[iri(1)],[unbound()],[unbound()]],len:2});
}
#[test]
fn rows_scan_ignores_nonmatching_source_payloads() {
    ops_core::scan([[iri(2),iri(9),rows_literal(10,11)],[iri(1),iri(9),rows_literal(10,22)]],
        [true,true],[1,1,0],[iri(1),iri(9),unbound()],
        Table{rows:[[rows_literal(10,22)],[unbound()],[unbound()],[unbound()]],len:1});
}
#[test(should_fail_with="scan completeness or capacity")]
fn rows_scan_cannot_truncate_a_full_bucket() {
    let mut source=[[iri(1),iri(9),iri(10)];33];let mut out=crate::table::empty::<32,1>();out.len=32;
    for i in 0..33 {source[i][0]=iri(i as Field+1);}
    for i in 0..32 {out.rows[i]=[iri(i as Field+1)];}
    ops_core::scan(source,[true;33],[0,1,1],[unbound(),iri(9),iri(10)],out);
}

#[test]
fn rows_temporal_scalar_matches_the_existing_byte_extraction() {
    for class in [3u8,4,8] {for instant in [0 as Field,100,0xffffffffffffffff] {
        let key=class as Field*0x1000000000000000000000000000000000000000000000000000000000000+instant*0x100000000000000000000000000000000000000000000;
        assert(crate::ops_core_expr::temporal_payload_for_class(key,class)==instant);
        assert(crate::ops_core_expr::temporal_payload(key)==instant);
    }}
}
#[test(should_fail_with="call to assert_max_bit_size")]
fn rows_temporal_scalar_refuses_nonzero_tail() {
    let _word=crate::ops_core_expr::temporal_payload_for_class(8*0x1000000000000000000000000000000000000000000000000000000000000+1,8);
}
#[test(should_fail_with="call to assert_max_bit_size")]
fn rows_temporal_scalar_refuses_a_different_header() {
    let _word=crate::ops_core_expr::temporal_payload_for_class(8*0x1000000000000000000000000000000000000000000000000000000000000,3);
}
#[test(should_fail_with="call to assert_max_bit_size")]
fn rows_temporal_scalar_refuses_an_overflowing_word() {
    let _word=crate::ops_core_expr::temporal_payload_for_class(9*0x1000000000000000000000000000000000000000000000000000000000000,8);
}

#[test]
fn rows_filter_preserves_selected_duplicate_payloads() {
    let input=Table{rows:[[rows_literal(9,11)],[rows_literal(9,22)],[unbound()],[unbound()]],len:2};
    let out=Table{rows:[[rows_literal(9,22)],[unbound()],[unbound()],[unbound()]],len:1};
    ops_core::filter(input,[ops_core::Boolean{value:false,is_error:false},
        ops_core::Boolean{value:true,is_error:false},ops_core::Boolean{value:true,is_error:false},
        ops_core::Boolean{value:true,is_error:false}],out);
}
#[test(should_fail_with="operator rows")]
fn rows_filter_refuses_a_different_duplicate_payload() {
    let input=Table{rows:[[rows_literal(9,11)],[rows_literal(9,22)],[unbound()],[unbound()]],len:2};
    ops_core::filter(input,[ops_core::Boolean{value:false,is_error:false},
        ops_core::Boolean{value:true,is_error:false},ops_core::Boolean{value:false,is_error:false},
        ops_core::Boolean{value:false,is_error:false}],one_literal());
}
fn one_literal() -> Table<4,1> {
    Table{rows:[[rows_literal(9,11)],[unbound()],[unbound()],[unbound()]],len:1}
}
#[test]
fn rows_filter_preserves_a_full_input_with_a_smaller_output_bucket() {
    let mut input=crate::table::empty::<32,1>();input.len=32;
    let mut out=crate::table::empty::<16,1>();out.len=16;
    let mut predicates=[ops_core::Boolean{value:false,is_error:false};32];
    for i in 0..32 {input.rows[i]=[iri(i as Field+1)];predicates[i].value=(i%2)==1;}
    for i in 0..16 {out.rows[i]=[iri(2*i as Field+2)];}
    ops_core::filter(input,predicates,out);
}
fn rows_filter_payload(out:TermCell) {
    ops_core::filter(one_literal(),[ops_core::Boolean{value:true,is_error:false};4],
        Table{rows:[[out],[unbound()],[unbound()],[unbound()]],len:1});
}
#[test(should_fail_with="operator rows")]
fn rows_filter_authenticates_the_key_class() {let mut out=rows_literal(9,11);out.key_class=2;rows_filter_payload(out);}
#[test(should_fail_with="operator rows")]
fn rows_filter_authenticates_the_key() {let mut out=rows_literal(9,11);out.key=12;rows_filter_payload(out);}
#[test(should_fail_with="operator rows")]
fn rows_filter_authenticates_the_datatype() {let mut out=rows_literal(9,11);out.dt=8;rows_filter_payload(out);}
#[test(should_fail_with="operator rows")]
fn rows_filter_authenticates_the_language() {let mut out=rows_literal(9,11);out.lang=8;rows_filter_payload(out);}
#[test(should_fail_with="operator rows")]
fn rows_filter_authenticates_the_identity() {let mut out=rows_literal(9,11);out.enc=10;rows_filter_payload(out);}
#[test(should_fail_with="operator rows")]
fn rows_filter_authenticates_the_kind() {let mut out=rows_literal(9,11);out.kind=1;rows_filter_payload(out);}
#[test(should_fail_with="operator rows")]
fn rows_filter_authenticates_the_scope() {
    let mut cell=iri(9);cell.kind=2;cell.scope=1;let mut changed=cell;changed.scope=2;
    ops_core::filter(Table{rows:[[cell]],len:1},[ops_core::Boolean{value:true,is_error:false}],Table{rows:[[changed]],len:1});
}
#[test(should_fail_with="operator rows")]
fn rows_filter_authenticates_the_bound_flag() {
    let mut cell=unbound();cell.bound=true;cell.kind=1;
    ops_core::filter(Table{rows:[[cell]],len:1},[ops_core::Boolean{value:true,is_error:false}],Table{rows:[[unbound()]],len:1});
}
#[test(should_fail_with="table padding")]
fn rows_filter_checks_input_padding() {
    ops_core::filter(Table{rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:2},
        [ops_core::Boolean{value:true,is_error:false};4],two());
}

#[test]
fn rows_ordered_projection_preserves_duplicate_payload_order() {
    let input=Table{rows:[[rows_literal(9,11),iri(1)],[rows_literal(9,22),iri(2)]],len:2};
    let out=Table{rows:[[rows_literal(9,11)],[rows_literal(9,22)]],len:2};
    ops_core::project(input,[0],out);
}
#[test(should_fail_with="canonical payload")]
fn rows_ordered_projection_refuses_swapped_payloads() {
    let input=Table{rows:[[rows_literal(9,11),iri(1)],[rows_literal(9,22),iri(2)]],len:2};
    let out=Table{rows:[[rows_literal(9,22)],[rows_literal(9,11)]],len:2};
    ops_core::project(input,[0],out);
}
#[test(should_fail_with="canonical permutation")]
fn rows_ordered_projection_refuses_a_changed_identity() {
    crate::table::assert_ordered(two(),Table{rows:[[iri(1)],[iri(3)],[unbound()],[unbound()]],len:2});
}
#[test(should_fail_with="table order")]
fn rows_ordered_projection_requires_canonical_order() {
    let reversed=Table{rows:[[iri(2)],[iri(1)]],len:2};
    crate::table::assert_ordered(reversed,reversed);
}
#[test(should_fail_with="table padding")]
fn rows_ordered_projection_checks_input_padding() {
    crate::table::assert_ordered(Table{rows:[[iri(1)],[iri(2)],[iri(3)],[unbound()]],len:2},Table{rows:[[iri(1)],[iri(2)]],len:2});
}
#[test(should_fail_with="operator length")]
fn rows_ordered_projection_refuses_missing_rows() {
    crate::table::assert_ordered(Table{rows:[[unbound()]],len:1},crate::table::empty::<4,1>());
}
#[test(should_fail_with="operator length")]
fn rows_ordered_projection_refuses_extra_rows() {
    crate::table::assert_ordered(Table{rows:[[iri(1)]],len:1},two());
}

#[test(should_fail_with="operator capacity")]
fn rows_permutation_refuses_output_capacity() {crate::table::assert_canonicalized(two(),empty::<1,1>());}
#[test(should_fail_with="operator capacity")]
fn rows_ordered_projection_refuses_output_capacity() {crate::table::assert_ordered(two(),empty::<1,1>());}


fn rows_join_payload(out:TermCell) {
    ops_core::join(Table{rows:[[rows_literal(9,11)]],len:1},Table{rows:[[]],len:1},
        [0],[0],Table{rows:[[out]],len:1});
}
#[test]
fn rows_join_preserves_bag_multiplicity() {
    let left=Table{rows:[[iri(1)],[iri(1)]],len:2};
    ops_core::join(left,Table{rows:[[]],len:1},[0],[0],left);
}
#[test(should_fail_with="join completeness or capacity")]
fn rows_join_cannot_reuse_an_output_occurrence() {
    ops_core::join(Table{rows:[[iri(1)],[iri(1)]],len:2},Table{rows:[[]],len:1},[0],[0],
        Table{rows:[[iri(1)]],len:1});
}
#[test(should_fail_with="join extra binding")]
fn rows_join_cannot_emit_a_pair_twice() {
    ops_core::join(Table{rows:[[iri(1)]],len:1},Table{rows:[[]],len:1},[0],[0],
        Table{rows:[[iri(1)],[iri(1)]],len:2});
}
#[test]
fn rows_join_preserves_duplicate_payload_order() {
    let left=Table{rows:[[rows_literal(9,11)],[rows_literal(9,22)]],len:2};
    ops_core::join(left,Table{rows:[[]],len:1},[0],[0],left);
}
#[test(should_fail_with="join source payload")]
fn rows_join_refuses_swapped_duplicate_payloads() {
    ops_core::join(Table{rows:[[rows_literal(9,11)],[rows_literal(9,22)]],len:2},
        Table{rows:[[]],len:1},[0],[0],Table{rows:[[rows_literal(9,22)],[rows_literal(9,11)]],len:2});
}
#[test]
fn rows_join_shared_bindings_keep_the_left_payload() {
    let left=Table{rows:[[rows_literal(9,11)]],len:1};
    ops_core::join(left,Table{rows:[[rows_literal(9,22)]],len:1},[0],[0],left);
}
#[test]
fn rows_join_unbound_compatibility_retains_the_bound_cell() {
    ops_core::join(Table{rows:[[unbound()]],len:1},Table{rows:[[iri(2)]],len:1},
        [0],[0],Table{rows:[[iri(2)]],len:1});
}
#[test]
fn rows_join_skips_an_incompatible_pair_before_a_match() {
    ops_core::join(Table{rows:[[iri(1)]],len:1},Table{rows:[[iri(0)],[iri(1)]],len:2},
        [0],[0],Table{rows:[[iri(1)]],len:1});
}
#[test(should_fail_with="join extra binding")]
fn rows_join_incompatible_pairs_cannot_contribute() {
    ops_core::join(Table{rows:[[iri(1)]],len:1},Table{rows:[[iri(2)]],len:1},
        [0],[0],Table{rows:[[iri(1)]],len:1});
}
#[test(should_fail_with="join completeness or capacity")]
fn rows_join_authenticates_the_output_identity() {
    ops_core::join(Table{rows:[[iri(1)]],len:1},Table{rows:[[]],len:1},
        [0],[0],Table{rows:[[iri(2)]],len:1});
}
#[test(should_fail_with="join source payload")]
fn rows_join_authenticates_the_key_class() {let mut out=rows_literal(9,11);out.key_class=2;rows_join_payload(out);}
#[test(should_fail_with="join source payload")]
fn rows_join_authenticates_the_key() {let mut out=rows_literal(9,11);out.key=12;rows_join_payload(out);}
#[test(should_fail_with="join source payload")]
fn rows_join_authenticates_the_datatype() {let mut out=rows_literal(9,11);out.dt=8;rows_join_payload(out);}
#[test(should_fail_with="join source payload")]
fn rows_join_authenticates_the_language() {let mut out=rows_literal(9,11);out.lang=8;rows_join_payload(out);}
#[test]
fn rows_join_covers_a_full_left_input_bucket() {
    let mut left=crate::table::empty::<32,1>();left.len=32;
    for i in 0..32 {left.rows[i]=[iri(i as Field+1)];}
    ops_core::join(left,Table{rows:[[]],len:1},[0],[0],left);
}

#[test]
fn rows_join_excludes_inactive_source_pairs() {
    let input=Table{rows:[[iri(1)],[unbound()]],len:1};
    ops_core::join(input,input,[0],[0],Table{rows:[[iri(1)]],len:1});
}

#[test]
fn rows_plain_string_identity_ignores_unused_key_classes() {
    for class in [2,5] {
        let mut a=plain_string_cell(7);a.key_class=class;a.key=0;
        let mut b=plain_string_cell(8);b.enc=a.enc+1;b.key_class=class;b.key=1;
        let equal=mixed_string_equality(a,a);assert(equal.value&!equal.is_error);
        let different=mixed_string_equality(a,b);assert(!different.value&!different.is_error);
        let iri_pair=mixed_string_equality(a,iri(7));assert(!iri_pair.value&!iri_pair.is_error);
        let not_in=crate::ops_core_expr::evaluate(Table{rows:[[a]],len:1},[b],[
            crate::ops_core_expr::Step{op:0,a:0,b:0,parameter:0,comparison:0},
            crate::ops_core_expr::Step{op:1,a:0,b:0,parameter:0,comparison:0},
            crate::ops_core_expr::Step{op:6,a:0,b:1,parameter:0,comparison:0},
            crate::ops_core_expr::Step{op:5,a:2,b:2,parameter:0,comparison:0}]);
        assert(not_in[0].value&!not_in[0].is_error);
        let mixed=mixed_string_equality(a,expression_cell(1,1,12));assert(!mixed.value&mixed.is_error);
    }
}

#[test]
fn expression_string_ordering_keeps_checked_payloads() {
    let mut a=expression_cell(6,3,11);a.dt=crate::suite_constants::DT_STRING;
    let mut b=expression_cell(6,2,12);b.dt=crate::suite_constants::DT_STRING;
    let result=crate::ops_core_expr::evaluate(Table{rows:[[a]],len:1},[b],
        [expr_step(0,0,0,0,0),expr_step(1,0,0,0,0),expr_step(2,0,1,0,4)])[0];
    assert(result.value&!result.is_error);
}


#[test]
fn rows_plain_string_numeric_class_is_not_a_numeric_value() {
    let number=expression_cell(1,1,12);let mut string=number;
    string.dt=crate::suite_constants::DT_STRING;string.enc=11;
    let result=mixed_string_equality(string,number);
    assert(!result.value&result.is_error);
}
#[test]
fn rows_plain_string_range_class_still_compares_identity() {
    let mut string=plain_string_cell(6);string.key_class=127;string.key=0;
    let result=mixed_string_equality(string,string);
    assert(result.value&!result.is_error);
}
