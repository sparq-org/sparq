//! Lexical key checks over the complete input of a value-consuming operator.
//! The authenticated term encoding binds each opening directly to its row;
//! there is no private source index or witness-selected subset of active rows.
use crate::table::{Table, equal_cell};
use crate::suite_entry::BoundedTermOpening;

pub fn check<let R:u32,let V:u32,let K:u32,let L:u32>(
    input:Table<R,V>,raw:[[BoundedTermOpening<L,41,1>;K];R],columns:[u32;K]) {
    check_with_parameters(input,raw,columns,[crate::table::unbound()]);
}

/// Public parameter cells are re-encoded by the verifier from its stored request.
/// Exact equality establishes their canonical key without a private lexical input.
pub fn check_with_parameters<let R:u32,let V:u32,let K:u32,let L:u32,let P:u32>(
    input:Table<R,V>,raw:[[BoundedTermOpening<L,41,1>;K];R],columns:[u32;K],parameters:[crate::table::TermCell;P]) {
    check_mode::<R,V,K,L,P,0>(input,raw,columns,parameters);
}

/// Equality and membership of plain strings use authenticated RDF identity.
/// Typed literals remain value consumers on every active input row.
pub fn check_identity_equalities<let R:u32,let V:u32,let K:u32,let L:u32,let P:u32>(
    input:Table<R,V>,raw:[[BoundedTermOpening<L,41,1>;K];R],columns:[u32;K],parameters:[crate::table::TermCell;P]) {
    check_mode::<R,V,K,L,P,1>(input,raw,columns,parameters);
}
/// Reads whose columns carry compile-time datatype profiles (`ops_core_suite::READ_*`).
pub fn check_profiled<let R:u32,let V:u32,let K:u32,let L:u32,let P:u32>(
    input:Table<R,V>,raw:[[BoundedTermOpening<L,41,1>;K];R],columns:[u32;K],profiles:[u32;K],parameters:[crate::table::TermCell;P]) {
    check_profiled_mode::<R,V,K,L,P,0>(input,raw,columns,profiles,parameters);
}
pub fn check_identity_profiled<let R:u32,let V:u32,let K:u32,let L:u32,let P:u32>(
    input:Table<R,V>,raw:[[BoundedTermOpening<L,41,1>;K];R],columns:[u32;K],profiles:[u32;K],parameters:[crate::table::TermCell;P]) {
    check_profiled_mode::<R,V,K,L,P,1>(input,raw,columns,profiles,parameters);
}
fn check_mode<let R:u32,let V:u32,let K:u32,let L:u32,let P:u32,let IDENTITY_STRINGS:u32>(
    input:Table<R,V>,raw:[[BoundedTermOpening<L,41,1>;K];R],columns:[u32;K],parameters:[crate::table::TermCell;P]) {
    check_profiled_mode::<R,V,K,L,P,IDENTITY_STRINGS>(input,raw,columns,[crate::ops_core_suite::READ_ALL;K],parameters);
}
fn check_profiled_mode<let R:u32,let V:u32,let K:u32,let L:u32,let P:u32,let IDENTITY_STRINGS:u32>(
    input:Table<R,V>,raw:[[BoundedTermOpening<L,41,1>;K];R],columns:[u32;K],profiles:[u32;K],parameters:[crate::table::TermCell;P]) {
    assert(input.len<=R,"read table capacity");
    for k in 0..K {
        assert(columns[k]<V,"read column");
        for r in 0..R {
            let cell=input.rows[r][columns[k]];
            let mut public=false;
            for p in 0..P {public=public | equal_cell(cell,parameters[p]);}
            let identity_only=(IDENTITY_STRINGS==1) & (cell.dt==crate::suite_constants::DT_STRING);
            if (r<input.len) & cell.bound & (cell.kind==3) & !public & !identity_only {
                let checked=if profiles[k]==crate::ops_core_suite::READ_ALL {
                    crate::ops_core_suite::checked_bounded_literal(cell,raw[r][k])
                } else {
                    crate::ops_core_suite::checked_profiled_literal(cell,raw[r][k],profiles[k])
                };
                assert(equal_cell(checked,cell),"read opening differs from input cell");
            } else {
                let opening=raw[r][k];
                assert(!opening.bound & (opening.kind==0) & (opening.lexical_len==0)
                    & (opening.datatype_len==0) & (opening.language_len==0)
                    & (opening.lexical==[0;L]) & (opening.datatype==[0;41])
                    & (opening.language==[0;1]) & (opening.key==[0;31]),"unused read opening padding");
            }
        }
    }
}

fn integer(value:u8) -> BoundedTermOpening<65,41,1> {
    let mut raw=crate::ops_core_profile::empty::<65>();raw.bound=true;raw.kind=3;
    raw.lexical[0]=48+value;raw.lexical_len=1;
    let dt="http://www.w3.org/2001/XMLSchema#integer".as_bytes();
    raw.datatype_len=dt.len();for i in 0..dt.len() {raw.datatype[i]=dt[i];}
    let key:Field=0x1800000000000000000000000000000000000000000000000000000000000
        + value as Field*1000000000000000000;
    let bytes=key.to_be_bytes::<32>();for i in 0..31 {raw.key[i]=bytes[i+1];}
    raw
}
fn cell(raw:BoundedTermOpening<65,41,1>) -> crate::table::TermCell {
    let opened=crate::suite_entry::open_bounded_cell(raw,0);
    crate::table::TermCell {bound:opened.bound(),kind:opened.kind(),enc:opened.identity(),
        key_class:opened.key_class(),key:opened.key(),dt:opened.datatype(),lang:opened.language(),scope:opened.credential()}
}
fn input() -> Table<2,1> {
    // Both rows are checked even when a later consumer keeps only one of them.
    Table {rows:[[cell(integer(1))],[cell(integer(2))]],len:2}
}
#[test]
fn reads_check_every_active_input_row() {check(input(),[[integer(1)],[integer(2)]],[0]);}
#[test]
fn reads_accept_empty_unbound_and_identity_only_cells() {
    let raw=[[crate::ops_core_profile::empty::<65>()];2];
    check(crate::table::empty::<2,1>(),raw,[0]);
    let mut table=crate::table::empty::<2,1>();table.len=2;
    table.rows[1][0]=crate::table::TermCell {bound:true,kind:1,enc:7,key_class:0,key:0,dt:0,lang:0,scope:0};
    check(table,raw,[0]);
}
#[test]
fn reads_accept_exact_verifier_parameter_cells() {
    let raw=[[crate::ops_core_profile::empty::<65>()];2];
    check_with_parameters(input(),raw,[0],[cell(integer(1)),cell(integer(2))]);
}
#[test(should_fail_with="key requires literal")]
fn reads_cannot_reuse_a_parameter_for_a_different_cell() {
    let raw=[[crate::ops_core_profile::empty::<65>()];2];
    check_with_parameters(input(),raw,[0],[cell(integer(2))]);
}
#[test(should_fail_with="key requires literal")]
fn reads_public_match_includes_value_metadata() {
    let raw=[[crate::ops_core_profile::empty::<65>()];2];
    let mut parameter=cell(integer(1));parameter.key=parameter.key+1;
    check_with_parameters(input(),raw,[0],[parameter,cell(integer(2))]);
}
#[test(should_fail_with="key disagrees with lexical form")]
fn reads_refuse_swapped_lexical() {
    let mut swapped=integer(1);swapped.lexical[0]=50;check(input(),[[swapped],[integer(2)]],[0]);
}
#[test(should_fail_with="datatype specialization differs")]
fn reads_refuse_swapped_datatype() {
    let mut swapped=integer(1);
    swapped.datatype=[0;41];let dt="http://www.w3.org/2001/XMLSchema#decimal".as_bytes();
    swapped.datatype_len=dt.len();for i in 0..dt.len() {swapped.datatype[i]=dt[i];}
    check(input(),[[swapped],[integer(2)]],[0]);
}
#[test(should_fail_with="read opening differs from input cell")]
fn reads_refuse_a_different_quad_opening() {check(input(),[[integer(2)],[integer(2)]],[0]);}
#[test(should_fail_with="key disagrees with lexical form")]
fn reads_check_non_minimum_input_keys() {
    let mut raw=integer(2);raw.key[30]=raw.key[30]+1;
    let mut table=input();table.rows[1][0]=cell(raw);check(table,[[integer(1)],[raw]],[0]);
}
#[test(should_fail_with="key disagrees with lexical form")]
fn reads_check_rows_a_later_filter_rejects() {
    let mut raw=integer(1);raw.key[30]=raw.key[30]+1;
    let mut table=input();table.rows[0][0]=cell(raw);check(table,[[raw],[integer(2)]],[0]);
}
#[test(should_fail_with="unused read opening padding")]
fn reads_have_canonical_inactive_padding() {
    let mut table=input();table.len=1;table.rows[1]=[crate::table::unbound()];
    check(table,[[integer(1)],[integer(2)]],[0]);
}
#[test(should_fail_with="read scalar backing padding")]
fn reads_have_canonical_scalar_backing() {
    let mut raw=integer(1);raw.lexical[64]=1;
    check(input(),[[raw],[integer(2)]],[0]);
}
#[test(should_fail_with="read table capacity")]
fn reads_cannot_truncate_an_overflowing_input() {
    let mut table=input();table.len=3;check(table,[[integer(1)],[integer(2)]],[0]);
}
#[test(should_fail_with="read column")]
fn reads_cannot_select_another_column() {check(input(),[[integer(1)],[integer(2)]],[1]);}

#[test]
fn reads_plain_string_identity_needs_no_value_key() {
    let mut string=crate::table::unbound();string.bound=true;string.kind=3;
    string.dt=crate::suite_constants::DT_STRING;string.enc=7;string.key_class=6;string.key=17;
    check_identity_equalities(Table{rows:[[string]],len:1},[[crate::ops_core_profile::empty::<65>()]],[0],[crate::table::unbound()]);
}
#[test(should_fail_with="key requires literal")]
fn reads_typed_membership_still_needs_a_value_key() {
    check_identity_equalities(input(),[[crate::ops_core_profile::empty::<65>()];2],[0],[crate::table::unbound()]);
}
fn string_raw() -> BoundedTermOpening<65,41,1> {
    let mut raw=crate::ops_core_profile::empty::<65>();raw.bound=true;raw.kind=3;
    raw.lexical[0]=55;raw.lexical_len=1;
    let dt="http://www.w3.org/2001/XMLSchema#string".as_bytes();
    raw.datatype_len=dt.len();for i in 0..dt.len() {raw.datatype[i]=dt[i];}
    raw
}
#[test]
fn profiled_reads_check_values_inside_the_column_profile() {
    let numeric=crate::ops_core_suite::READ_INTEGER+crate::ops_core_suite::READ_DECIMAL;
    check_profiled(input(),[[integer(1)],[integer(2)]],[0],[numeric],[crate::table::unbound()]);
}
#[test(should_fail_with="key disagrees with lexical form")]
fn profiled_reads_still_check_keys() {
    let numeric=crate::ops_core_suite::READ_INTEGER;
    let mut swapped=integer(1);swapped.lexical[0]=50;
    check_profiled(input(),[[swapped],[integer(2)]],[0],[numeric],[crate::table::unbound()]);
}
// A supported datatype outside the profile is not key-checked: the comparison with the
// column's constants is a type error, so the row's result is exact without its key.
#[test]
fn profiled_reads_leave_a_supported_datatype_outside_the_profile_unchecked() {
    let mut forged=integer(1);forged.lexical[0]=57;
    check_profiled(input(),[[forged],[integer(2)]],[0],[crate::ops_core_suite::READ_DATE],[crate::table::unbound()]);
}
#[test]
fn profiled_reads_leave_a_string_in_a_numeric_column_unchecked() {
    let numeric=crate::ops_core_suite::READ_INTEGER+crate::ops_core_suite::READ_DECIMAL;
    let mut table=input();
    table.rows[1][0]=crate::table::TermCell {bound:true,kind:3,enc:7,key_class:6,key:0,dt:crate::suite_constants::DT_STRING,lang:0,scope:0};
    check_profiled(table,[[integer(1)],[string_raw()]],[0],[numeric],[crate::table::unbound()]);
}
#[test(should_fail_with="N0 credential literal datatype")]
fn profiled_reads_refuse_an_unsupported_datatype() {
    let numeric=crate::ops_core_suite::READ_INTEGER+crate::ops_core_suite::READ_DECIMAL;
    let mut table=input();
    table.rows[1][0].dt=crate::suite_constants::DT_LONG;
    check_profiled(table,[[integer(1)],[integer(2)]],[0],[numeric],[crate::table::unbound()]);
}
// F1 exactness: a supported datatype outside the profile compares with the column's
// constants as a type error, whatever its unchecked key says. FILTER drops the row,
// `!` keeps the error, and a BIND would leave its variable unbound.
#[test]
fn unchecked_out_of_profile_cells_compare_as_type_errors_whatever_their_key() {
    let step=|op:u8,a:u32,b:u32,parameter:u32,comparison:u8| crate::ops_core_expr::Step {op,a,b,parameter,comparison};
    for key in [6*0x1000000000000000000000000000000000000000000000000000000000000,
        6*0x1000000000000000000000000000000000000000000000000000000000000+0x3100000000000000000000000000000000000000000000000000000000] {
        // A plain string in a column compared only with an integer constant.
        let mut table=input();
        table.rows[0][0]=crate::table::TermCell {bound:true,kind:3,enc:7,key_class:6,key,dt:crate::suite_constants::DT_STRING,lang:0,scope:0};
        let integer_constant=[cell(integer(1))];
        for comparison in [0,1,2,3,4] {
            let compared=crate::ops_core_expr::evaluate(table,integer_constant,[step(0,0,0,0,0),step(1,0,0,0,0),step(2,0,1,0,comparison)]);
            assert(!compared[0].value & compared[0].is_error);
            let negated=crate::ops_core_expr::evaluate(table,integer_constant,[step(0,0,0,0,0),step(1,0,0,0,0),step(2,0,1,0,comparison),step(5,2,2,0,0)]);
            assert(!negated[0].value & negated[0].is_error);
        }
    }
    for forged in [1,2] {
        // An integer with an unchecked key in a column compared only with a plain string.
        let mut table=input();
        table.rows[0][0].key=table.rows[0][0].key+forged;
        let string_constant=[crate::table::TermCell {bound:true,kind:3,enc:9,key_class:6,
            key:6*0x1000000000000000000000000000000000000000000000000000000000000,dt:crate::suite_constants::DT_STRING,lang:0,scope:0}];
        for comparison in [0,1,2,3,4] {
            let compared=crate::ops_core_expr::evaluate(table,string_constant,[step(0,0,0,0,0),step(1,0,0,0,0),step(2,0,1,0,comparison)]);
            assert(!compared[0].value & compared[0].is_error);
        }
    }
}
fn date_constant() -> crate::table::TermCell {
    crate::table::TermCell {bound:true,kind:3,enc:11,key_class:8,
        key:8*0x1000000000000000000000000000000000000000000000000000000000000+0x100000000000000000000000000000000000000000000,
        dt:crate::suite_constants::DT_DATE,lang:0,scope:0}
}
fn date_time_cell(key_class:u8) -> crate::table::TermCell {
    crate::table::TermCell {bound:true,kind:3,enc:13,key_class,
        key:key_class as Field*0x1000000000000000000000000000000000000000000000000000000000000,
        dt:crate::suite_constants::DT_DATETIME,lang:0,scope:0}
}
// Date-compared columns admit only xsd:date, so a dateTime cell there keeps an unchecked
// key. Exact only while the comparator never orders date against dateTime: with no key
// class an ordering is a type error (equality is a refusal), and with a temporal key
// class every comparison is a refusal.
#[test]
fn unchecked_other_temporal_cells_never_compare_with_a_date() {
    let step=|op:u8,a:u32,b:u32,parameter:u32,comparison:u8| crate::ops_core_expr::Step {op,a,b,parameter,comparison};
    let mut table=input();
    table.rows[0][0]=date_time_cell(0);
    for comparison in [1,2,3,4] {
        let compared=crate::ops_core_expr::evaluate(table,[date_constant()],[step(0,0,0,0,0),step(1,0,0,0,0),step(2,0,1,0,comparison)]);
        assert(!compared[0].value & compared[0].is_error);
    }
}
#[test(should_fail_with="mixed temporal datatypes not yet supported")]
fn unchecked_zoned_date_times_are_refused_against_a_date() {
    let step=|op:u8,a:u32,b:u32,parameter:u32,comparison:u8| crate::ops_core_expr::Step {op,a,b,parameter,comparison};
    let mut table=input();
    table.rows[0][0]=date_time_cell(3);
    let _=crate::ops_core_expr::evaluate(table,[date_constant()],[step(0,0,0,0,0),step(1,0,0,0,0),step(2,0,1,0,4)]);
}
