//! Contract controls for the suite-key bridges and typed integer stand-in.
use crate::ops_closed_arith_standin::{integer,add,subtract,multiply,compare};
use crate::table::{TermCell,Table,unbound,empty,append};
use crate::ops_closed_numeric_standin::{encode,OFFSET,PREFIX};
use crate::ops_closed_group::Aggregate;
fn int(n:u64)->TermCell {
    let mut c=encode(OFFSET+n as Field*1000000000000000000,false);c.key=c.key+PREFIX;
    c.lang=crate::term::hash_bytes(crate::suite_constants::LANGUAGE_DOMAIN,std::collections::bounded_vec::BoundedVec::<u8,0>::new());c
}
fn input()->Table<4,1>{crate::table::canonicalize(append(append(empty(),[int(1)],true),[int(2)],true))}
fn sum(out:Table<4,1>){crate::ops_closed_group_plan::group(input(),out,[],[Aggregate{kind:2,column:0,distinct:false}]);}
#[test] fn group_bridge_honest(){sum(append(empty(),[int(3)],true));}
#[test(should_fail_with="operator length")] fn group_bridge_missing(){sum(empty());}
#[test(should_fail_with="operator length")] fn group_bridge_extra(){sum(append(append(empty(),[int(3)],true),[int(3)],true));}
#[test(should_fail_with="table order")] fn group_bridge_reordered(){
    let a=input();let mut out=crate::table::canonicalize(a);let t=out.rows[0];out.rows[0]=out.rows[1];out.rows[1]=t;
    crate::ops_closed_group_plan::group(a,out,[0],[]);
}
#[test(should_fail_with="table padding")] fn group_bridge_padding(){let mut out=append(empty(),[int(3)],true);out.rows[3]=[int(9)];sum(out);}
#[test(should_fail_with="operator capacity")] fn group_bridge_overflow(){crate::ops_closed_group_plan::group(input(),empty::<0,1>(),[],[Aggregate{kind:2,column:0,distinct:false}]);}
#[test(should_fail_with="operator rows")] fn group_bridge_hidden_sum_operand(){sum(append(empty(),[int(1)],true));}
#[test(should_fail_with="operator rows")] fn group_bridge_wrong_encoding(){let mut c=int(3);c.enc=c.enc+1;sum(append(empty(),[c],true));}
#[test(should_fail_with="numeric language")] fn group_bridge_wrong_language(){let mut c=int(3);c.lang=0;sum(append(empty(),[c],true));}
#[test] fn group_bridge_empty_sum_is_zero(){crate::ops_closed_group_plan::group(empty::<4,1>(),append(empty::<4,1>(),[int(0)],true),[],[Aggregate{kind:2,column:0,distinct:false}]);}
fn iri(n:Field)->TermCell{TermCell{bound:true,kind:1,enc:n,key_class:0,key:0,dt:0,lang:0,scope:0}}
fn dataset()->crate::ops_core::Dataset<4>{crate::ops_core::Dataset{rows:[[iri(1),iri(2),iri(3)],[unbound();3],[iri(1),iri(2),iri(3)],[unbound();3]],active:[true,false,true,false]}}
fn graph_out()->Table<4,2>{append(append(empty(),[iri(3),crate::ops_closed_dataset::graph_name(0)],true),[iri(3),crate::ops_closed_dataset::graph_name(1)],true)}
fn graph(out:Table<4,2>){crate::ops_closed_dataset::graph_scan(dataset(),[0,2],[2,2],crate::ops_closed_dataset::unit_context(),[2,2,0,1],[1;4],[iri(1),iri(2),unbound(),unbound()],out);}
#[test] fn graph_scan_honest(){graph(graph_out());}
#[test(should_fail_with="operator length")] fn graph_scan_missing(){let mut out=graph_out();out.len=1;out.rows[1]=[unbound();2];graph(out);}
#[test(should_fail_with="operator length")] fn graph_scan_extra(){let mut out=graph_out();out=append(out,out.rows[1],true);graph(out);}
#[test(should_fail_with="table order")] fn graph_scan_reordered(){let mut out=graph_out();let row=out.rows[0];out.rows[0]=out.rows[1];out.rows[1]=row;graph(out);}
#[test(should_fail_with="table padding")] fn graph_scan_padding(){let mut out=graph_out();out.rows[3]=out.rows[0];graph(out);}
#[test(should_fail_with="operator capacity")] fn graph_scan_overflow(){crate::ops_closed_dataset::graph_scan(dataset(),[0,2],[2,2],crate::ops_closed_dataset::unit_context(),[2,2,0,1],[1;4],[iri(1),iri(2),unbound(),unbound()],empty::<1,2>());}
#[test(should_fail_with="operator length")] fn graph_scan_hidden_matching_graph(){let mut out=graph_out();out.len=0;out.rows=[[unbound();2];4];graph(out);}
#[test] fn default_scan_honest(){crate::ops_closed_dataset::scan(dataset(),crate::ops_closed_dataset::unit_context(),[1,1,0],[1;3],[iri(1),iri(2),unbound()],append(empty::<4,1>(),[iri(3)],true));}
#[test(should_fail_with="operator length")] fn default_scan_missing(){crate::ops_closed_dataset::scan(dataset(),crate::ops_closed_dataset::unit_context(),[1,1,0],[1;3],[iri(1),iri(2),unbound()],empty::<4,1>());}
#[test(should_fail_with="operator length")] fn default_scan_extra(){crate::ops_closed_dataset::scan(dataset(),crate::ops_closed_dataset::unit_context(),[1,1,0],[1;3],[iri(1),iri(2),unbound()],append(append(empty::<4,1>(),[iri(3)],true),[iri(3)],true));}
#[test(should_fail_with="table order")] fn default_scan_reordered(){let mut d=dataset();d.rows[2][2]=iri(4);let out=append(append(empty::<4,1>(),[iri(4)],true),[iri(3)],true);crate::ops_closed_dataset::scan(d,crate::ops_closed_dataset::unit_context(),[1,1,0],[1;3],[iri(1),iri(2),unbound()],out);}
#[test(should_fail_with="table padding")] fn default_scan_padding(){let mut out=append(empty::<4,1>(),[iri(3)],true);out.rows[3]=[iri(3)];crate::ops_closed_dataset::scan(dataset(),crate::ops_closed_dataset::unit_context(),[1,1,0],[1;3],[iri(1),iri(2),unbound()],out);}
#[test(should_fail_with="operator capacity")] fn default_scan_overflow(){crate::ops_closed_dataset::scan(dataset(),crate::ops_closed_dataset::unit_context(),[1,1,0],[1;3],[iri(1),iri(2),unbound()],empty::<0,1>());}
#[test] fn integer_arithmetic_signed_and_error_lanes(){
    let a=integer(int(30));let b=integer(int(10));assert(compare(multiply(a,b),integer(int(300)),0).value);
    let negative=subtract(b,a);assert(compare(add(negative,a),b,0).value);
    assert(multiply(integer(unbound()),a).1);
}
#[test(should_fail)] fn integer_multiplication_overflow_is_unsatisfiable(){let _value=crate::ops_closed_arith_standin::multiply((0x10000000000000000000000000,false),(0x10000000000000000000000000,false));}
#[test(should_fail_with="canonical credential order")] fn credential_order_is_constrained(){crate::ops_closed_dataset::canonical_credentials([2,1],2);}
#[test]
fn signed_blank_node_cannot_alias_a_credential_graph(){
    let raw=TermCell{bound:true,kind:2,enc:0x09db4003a33bf2be14a433948944754a19c4f505bfdcc56fdbbf8d9402cf35c1,key_class:0,key:0,dt:0,lang:0,scope:1};
    let dataset=crate::ops_core::Dataset{rows:[[raw,iri(2),iri(3)]],active:[true]};
    crate::ops_closed_dataset::graph_scan(dataset,[0],[1],crate::ops_closed_dataset::unit_context(),[0,1,1,0],[1;4],[unbound(),iri(2),iri(3),unbound()],empty::<2,1>());
}
#[test]
fn graph_name_domain_is_pinned(){assert(crate::ops_closed_dataset::graph_name(0).enc==poseidon::poseidon2::Poseidon2::hash([0x4e512f4752415048,0,0],3));}

#[test(should_fail)]
fn integer_multiplication_rejects_modular_wrap(){let _value=crate::ops_closed_arith_standin::multiply((761693500506309222409094138289133375,false),(958703690306430087654759117285840627,false));}
#[test(should_fail_with="operator rows")]
fn group_bridge_empty_group_cannot_claim_a_nonzero_sum(){crate::ops_closed_group_plan::group(empty::<4,1>(),append(empty::<4,1>(),[int(1)],true),[],[Aggregate{kind:2,column:0,distinct:false}]);}
