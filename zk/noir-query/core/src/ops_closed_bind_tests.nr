//! Boolean BIND contract, including exact existence and SPARQL error behavior.
use crate::table::{Table,TermCell,empty,append,unbound};
use crate::ops_core::Boolean;
use std::collections::bounded_vec::BoundedVec;
fn iri(n:Field)->TermCell{TermCell{bound:true,kind:1,enc:n,key_class:0,key:0,dt:0,lang:0,scope:0}}
fn input()->Table<2,1>{append(append(empty(),[iri(1)],true),[iri(2)],true)}
fn boolean(v:bool)->TermCell{
    let mut lexical:BoundedVec<u8,5> = BoundedVec::new();
    if v {for b in "true".as_bytes(){lexical.push(b);}}else{for b in "false".as_bytes(){lexical.push(b);}}
    let mut dt:BoundedVec<u8,40> = BoundedVec::new();for b in "http://www.w3.org/2001/XMLSchema#boolean".as_bytes(){dt.push(b);}
    let key:Field=0x5000000000000000000000000000000000000000000000000000000000000+if v{0x10000000000000000000000000000000000000000000000000000000000}else{0};
    let c=crate::term::open_term(3,lexical,dt,BoundedVec::<u8,0>::new(),key.to_be_bytes::<31>(),0);
    TermCell{bound:c.bound(),kind:c.kind(),enc:c.identity(),key_class:c.key_class(),key:c.key(),dt:c.datatype(),lang:c.language(),scope:0}
}
fn output()->Table<2,2>{append(append(empty(),[iri(1),boolean(true)],true),[iri(2),boolean(false)],true)}
fn run(out:Table<2,2>){crate::ops_closed_extend::boolean(input(),[0,1],1,[Boolean{value:true,is_error:false},Boolean{value:false,is_error:false}],out);}
#[test]fn bind_boolean_honest(){run(output());}
#[test(should_fail_with="operator length")]fn bind_boolean_missing(){let mut out=output();out.len=1;out.rows[1]=[unbound();2];run(out);}
#[test(should_fail_with="operator length")]fn bind_boolean_extra(){crate::ops_closed_extend::boolean(append(empty::<2,1>(),[iri(1)],true),[0,1],1,[Boolean{value:true,is_error:false};2],output());}
#[test(should_fail_with="table order")]fn bind_boolean_reordered(){let mut out=output();let a=out.rows[0];out.rows[0]=out.rows[1];out.rows[1]=a;run(out);}
#[test(should_fail_with="table padding")]fn bind_boolean_padding(){let mut out=append(empty::<2,2>(),[iri(1),boolean(true)],true);out.rows[1]=[iri(2),boolean(false)];crate::ops_closed_extend::boolean(append(empty::<2,1>(),[iri(1)],true),[0,1],1,[Boolean{value:true,is_error:false};2],out);}
#[test(should_fail_with="operator capacity")]fn bind_boolean_overflow(){crate::ops_closed_extend::boolean(input(),[0,1],1,[Boolean{value:true,is_error:false};2],empty::<1,2>());}
#[test]fn bind_error_keeps_an_unbound_solution(){crate::ops_closed_extend::boolean(input(),[0,1],1,[Boolean{value:false,is_error:true};2],append(append(empty::<2,2>(),[iri(1),unbound()],true),[iri(2),unbound()],true));}
#[test(should_fail_with="operator rows")]fn bind_boolean_wrong_result(){let mut out=output();out.rows[0][1]=boolean(false);run(out);}
#[test]fn existence_can_feed_boolean_bind(){
    let right=append(empty::<2,1>(),[iri(3)],true);
    let values=crate::ops_closed_negation::existence([right,empty()]);
    crate::ops_closed_extend::boolean(input(),[0,1],1,values,output());
}
#[test(should_fail_with="operator length")]fn existence_hidden_matching_right_fails_before_binding(){
    let complete=append(empty::<2,1>(),[iri(3)],true);let hidden=empty::<2,1>();
    crate::ops_core::project(complete,[0],hidden);
    let values=crate::ops_closed_negation::existence([hidden,empty()]);
    crate::ops_closed_extend::boolean(input(),[0,1],1,values,output());
}
#[test]fn empty_min_error_and_false_releases_false(){
    let values=crate::ops_closed_common::combine([Boolean{value:false,is_error:true};2],[Boolean{value:false,is_error:false};2],1);
    crate::ops_closed_extend::boolean(input(),[0,1],1,values,append(append(empty::<2,2>(),[iri(1),boolean(false)],true),[iri(2),boolean(false)],true));
}
#[test]fn existence_last_left_row_can_feed_bind(){
    let right=append(empty::<2,1>(),[iri(3)],true);
    let values=crate::ops_closed_negation::existence([empty(),right]);
    crate::ops_closed_extend::boolean(input(),[0,1],1,values,
        append(append(empty::<2,2>(),[iri(1),boolean(false)],true),[iri(2),boolean(true)],true));
}

#[test(should_fail_with="BIND target already bound")]
fn bind_boolean_cannot_overwrite_an_existing_variable(){
    crate::ops_closed_extend::boolean(input(),[0],0,[Boolean{value:true,is_error:false};2],empty::<2,1>());
}
