//! Validate packed temporal keys before delegating to the shared expression adapter.
//! The original cells and their authenticated encodings remain in operator tables.
use crate::table::{TermCell,Table};
use crate::ops_core::Boolean;
use crate::ops_core_expr::Step;
fn temporal(cell:TermCell)->TermCell {
    if cell.bound & (cell.kind==3) & ((cell.key_class==3)|(cell.key_class==4)|(cell.key_class==8)) {
        let bytes=cell.key.to_be_bytes::<31>();
        assert(bytes[0]==cell.key_class,"temporal key class");
        for i in 1..31 {
            if i>8 {assert(bytes[i]==0,"non-zero temporal key tail");}
        }
    }
    cell
}
pub fn evaluate_indexed<let R:u32,let V:u32,let P:u32,let I:u32,let E:u32>(
    input:Table<R,V>,parameters:[TermCell;P],indices:[u32;I],steps:[Step;E],
)->[Boolean;R] {
    crate::table::check_table(input);
    let mut columns=[false;V];let mut used=[false;P];
    for step in steps {
        for v in 0..V {columns[v]=columns[v] | ((step.op==0)&(step.parameter==v));}
        for p in 0..P {
            used[p]=used[p] | ((step.op==1)&(step.parameter==p))
                | ((step.op==8)&(p as u64>=step.parameter as u64)&((p as u64)<step.parameter as u64+16));
        }
    }
    let mut values=input;let mut params=parameters;
    for r in 0..R {for v in 0..V {if columns[v] {values.rows[r][v]=temporal(input.rows[r][v]);}}}
    for p in 0..P {crate::table::check_cell(parameters[p]);if used[p] {params[p]=temporal(parameters[p]);}}
    crate::ops_core_expr::evaluate_indexed(values,params,indices,steps)
}
fn date<let N:u32>(text:[u8;N],days:i64)->TermCell {
    let biased=(days as u64)^0x8000000000000000;
    let key=8*crate::ops_closed_numeric::PREFIX+(biased as Field)*0x100000000000000000000000000000000000000000000;
    let mut lexical=[0;64];for i in 0..N {lexical[i]=text[i];}
    let mut datatype=[0;41];let dt="http://www.w3.org/2001/XMLSchema#date".as_bytes();
    for i in 0..dt.len() {datatype[i]=dt[i];}
    let c=crate::suite_entry::open_bounded_cell(crate::suite_entry::BoundedTermOpening{bound:true,kind:3,lexical,lexical_len:N,datatype,datatype_len:dt.len(),language:[0;1],language_len:0,key:key.to_be_bytes::<31>()},0);
    TermCell{bound:c.bound(),kind:c.kind(),enc:c.identity(),key_class:c.key_class(),key:c.key(),dt:c.datatype(),lang:c.language(),scope:c.credential()}
}
fn program()->[Step;3]{[
    Step{op:0,a:0,b:0,parameter:0,comparison:0},
    Step{op:1,a:0,b:0,parameter:0,comparison:0},
    Step{op:2,a:0,b:1,parameter:0,comparison:2},
]}
#[test]
fn checked_dates_compare_through_the_expression_lane(){
    let a=date("2021-11-13".as_bytes(),18944);let b=date("2026-11-13".as_bytes(),20770);
    let input=crate::table::append(crate::table::empty::<2,1>(),[a],true);
    let result=evaluate_indexed(input,[b],[0],program());assert(result[0].value & !result[0].is_error);assert(result[1].is_error & !result[1].value);
    let result=evaluate_indexed(crate::table::append(crate::table::empty::<2,1>(),[b],true),[a],[0],program());assert(!result[0].value & !result[0].is_error);
}
#[test(should_fail_with="non-zero temporal key tail")]
fn temporal_payload_cannot_drop_a_low_byte(){
    let mut a=date("2021-11-13".as_bytes(),18944);a.key=a.key+1;
    let _out=evaluate_indexed(crate::table::append(crate::table::empty::<1,1>(),[a],true),[a],[0],program());
}
#[test(should_fail_with="temporal key class")]
fn temporal_payload_class_is_constrained(){
    let mut a=date("2021-11-13".as_bytes(),18944);a.key_class=4;let _out=temporal(a);
}
#[test]
fn datetime_classes_use_the_same_packed_word_contract(){
    for class in [3,4] {
        let a=TermCell{bound:true,kind:3,enc:1,key_class:class,key:(class as Field)*crate::ops_closed_numeric::PREFIX+2*0x100000000000000000000000000000000000000000000,dt:crate::suite_constants::DT_DATETIME,lang:0,scope:0};
        let mut b=a;b.enc=2;b.key=b.key+0x100000000000000000000000000000000000000000000;
        let result=evaluate_indexed(crate::table::append(crate::table::empty::<1,1>(),[a],true),[b],[0],program());assert(result[0].value & !result[0].is_error);
    }
}

#[test(should_fail_with="IN list capacity")]
fn empty_input_keeps_the_expression_parameter_capacity_guard(){
    let _out=evaluate_indexed(crate::table::empty::<1,1>(),[crate::table::unbound();16],[17],[
        Step{op:7,a:0,b:0,parameter:0,comparison:0},
        Step{op:8,a:0,b:0,parameter:0,comparison:0},
    ]);
}

fn read_date<let N:u32>(text:[u8;N],days:u64)->crate::suite_entry::BoundedTermOpening<65,41,1> {
    let mut raw=crate::ops_core_profile::empty::<65>();raw.bound=true;raw.kind=3;
    for i in 0..N {raw.lexical[i]=text[i];}raw.lexical_len=N;
    let dt="http://www.w3.org/2001/XMLSchema#date".as_bytes();
    for i in 0..dt.len() {raw.datatype[i]=dt[i];}raw.datatype_len=dt.len();
    let key=8*crate::ops_closed_numeric::PREFIX+((days^0x8000000000000000) as Field)*0x100000000000000000000000000000000000000000000;
    raw.key=key.to_be_bytes::<31>();raw
}
fn read_cell(raw:crate::suite_entry::BoundedTermOpening<65,41,1>)->TermCell {
    let c=crate::suite_entry::open_bounded_cell(raw,0);
    TermCell{bound:c.bound(),kind:c.kind(),enc:c.identity(),key_class:c.key_class(),key:c.key(),dt:c.datatype(),lang:c.language(),scope:c.credential()}
}
#[test]
fn closed_reads_check_honest_claim_dates_before_not_exists() {
    let raw=read_date("2026-11-13".as_bytes(),20770);
    let input=crate::table::append(crate::table::empty::<4,1>(),[read_cell(raw)],true);
    let mut reads=[[crate::ops_core_profile::empty::<65>()];4];reads[0]=[raw];
    crate::ops_core_reads::check(input,reads,[0]);
    let threshold=date("2021-11-13".as_bytes(),18944);
    let predicates=evaluate_indexed(input,[threshold],[0],[
        Step{op:0,a:0,b:0,parameter:0,comparison:0},Step{op:1,a:0,b:0,parameter:0,comparison:0},
        Step{op:2,a:0,b:1,parameter:0,comparison:3}]);
    let right=input;
    crate::ops_core::filter(input,predicates,right);
    let result=crate::ops_closed_negation::existence([right]);assert(result[0].value);
}
#[test(should_fail_with="key disagrees with lexical form")]
fn closed_reads_refuse_forged_date_on_nonoutput_not_exists_claim() {
    // The newer claim's forged old date would be rejected by the inner FILTER.
    let raw=read_date("2026-11-13".as_bytes(),0);
    let input=crate::table::append(crate::table::empty::<4,1>(),[read_cell(raw)],true);
    let mut reads=[[crate::ops_core_profile::empty::<65>()];4];reads[0]=[raw];
    crate::ops_core_reads::check(input,reads,[0]);
}
fn fraction(digit:u8,key_digit:u8)->crate::suite_entry::BoundedTermOpening<65,41,1> {
    let mut raw=crate::ops_core_profile::empty::<65>();raw.bound=true;raw.kind=3;
    raw.lexical[0]=48;raw.lexical[1]=46;raw.lexical[2]=digit+48;raw.lexical_len=3;
    let dt="http://www.w3.org/2001/XMLSchema#decimal".as_bytes();
    for i in 0..dt.len() {raw.datatype[i]=dt[i];}raw.datatype_len=dt.len();
    raw.key=(crate::ops_closed_numeric::PREFIX+crate::ops_closed_numeric::OFFSET+key_digit as Field*100000000000000000).to_be_bytes::<31>();raw
}
fn minimum_reads(forged:bool) {
    let a=fraction(3,3);let b=fraction(4,if forged {5}else{4});
    let input=crate::table::canonicalize(crate::table::append(crate::table::append(crate::table::empty::<4,1>(),[read_cell(a)],true),[read_cell(b)],true));
    let mut reads=[[crate::ops_core_profile::empty::<65>()];4];
    for i in 0..2 {reads[i]=[if crate::table::same_term(input.rows[i][0],read_cell(a)) {a}else{b}];}
    crate::ops_core_reads::check(input,reads,[0]);
    let out=crate::table::append(crate::table::empty::<4,1>(),[read_cell(a)],true);
    crate::ops_closed_group_plan::group(input,out,[],[crate::ops_closed_group::Aggregate{kind:3,column:0,distinct:false}]);
}
#[test]
fn closed_reads_check_honest_recycled_minimum_inputs(){minimum_reads(false);}
#[test(should_fail_with="key disagrees with lexical form")]
fn closed_reads_refuse_forged_nonminimum_recycled_figure(){minimum_reads(true);}
