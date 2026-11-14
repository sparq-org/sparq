//! Transport the suite's packed temporal keys into the expression key channel.
//! The original cells and their authenticated encodings remain in operator tables.
use crate::table::{TermCell,Table};
use crate::ops_core::Boolean;
use crate::ops_core_expr::Step;
fn temporal(mut cell:TermCell)->TermCell {
    if cell.bound & (cell.kind==3) & ((cell.key_class==3)|(cell.key_class==4)|(cell.key_class==8)) {
        let bytes=cell.key.to_be_bytes::<31>();
        assert(bytes[0]==cell.key_class,"temporal key class");
        let mut value:Field=0;
        for i in 1..31 {
            if i<=8 {value=value*256+bytes[i] as Field;}
            else {assert(bytes[i]==0,"non-zero temporal key tail");}
        }
        cell.key=(cell.key_class as Field)*crate::ops_closed_numeric::PREFIX+value;
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
    let c=crate::ops_core_profile::open_object(crate::ops_core_profile::ObjectOpening{bound:true,kind:3,lexical,lexical_len:N,datatype:3,key:key.to_be_bytes::<31>()},0);
    TermCell{bound:c.bound(),kind:c.kind(),enc:c.enc(),key_class:c.key_class(),key:c.key(),dt:c.datatype(),lang:c.language(),scope:c.credential()}
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
