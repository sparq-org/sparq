//! STAND-IN: lane E owns the verified signing-key to issuer-IRI policy.
//! The verifier supplies the registry as ordinary bound public term parameters.
use crate::table::{TermCell,unbound,same_term};
use crate::ops_core::Dataset;
use compose::issuer::Point;
use std::collections::bounded_vec::BoundedVec;
pub fn key_name(key:Point)->TermCell{
    let mut lexical:BoundedVec<u8,149> = BoundedVec::new();
    for b in "urn:sparq:issuer-key:".as_bytes(){lexical.push(b);}
    let bytes=[key.x.to_be_bytes::<32>(),key.y.to_be_bytes::<32>()];
    for coordinate in bytes {for b in coordinate {let high=b/16;let low=b%16;lexical.push(if high<10 {48+high}else{87+high});lexical.push(if low<10 {48+low}else{87+low});}}
    let cell=crate::term::open_term(1,lexical,BoundedVec::<u8,0>::new(),BoundedVec::<u8,0>::new(),[0;31],0);
    TermCell{bound:true,kind:1,enc:cell.enc(),key_class:0,key:0,dt:0,lang:0,scope:0}
}
pub fn issuer_identity<let D:u32,let K:u32>(dataset:Dataset<D>,offset:u32,length:u32,key:Point,active:bool,registry:[[TermCell;2];K]){
    assert((offset as u64+length as u64)<=D as u64,"issuer credential capacity");
    let mut issuer=unbound();
    for i in 0..D {if dataset.active[i] & (i>=offset) & (i<offset+length) & (dataset.rows[i][1].enc==crate::suite_constants::IRI_ISSUER){
        let value=dataset.rows[i][2];assert((value.kind==1)&(!issuer.bound|same_term(issuer,value)),"credential issuer field");issuer=value;
    }}
    if active {
        let encoded=key_name(key);let mut matched=false;
        for pair in registry {crate::table::check_cell(pair[0]);crate::table::check_cell(pair[1]);matched=matched|(same_term(issuer,pair[0])&same_term(encoded,pair[1]));}
        assert(issuer.bound&matched,"verified issuer registry");
    }
}
fn fixture_key()->Point {Point{x:0x1bb873421a895ca8c8d9fc0b44dd35be1d65592564942523f49590264f03d5a9,y:0x0d9fd8a464e1fc3e79175f0729533a1b865b70848c2f90d9c037d4d6949c9b55}}
fn fixture_name()->TermCell {TermCell{bound:true,kind:1,enc:0x30392bb676562edce489071472b71d1e88ea73213891d91d5c63df7b9bafb099,key_class:0,key:0,dt:0,lang:0,scope:0}}
fn fixture_issuer()->TermCell {TermCell{bound:true,kind:1,enc:7,key_class:0,key:0,dt:0,lang:0,scope:0}}
fn fixture_dataset()->Dataset<1>{Dataset{rows:[[fixture_issuer(),TermCell{bound:true,kind:1,enc:crate::suite_constants::IRI_ISSUER,key_class:0,key:0,dt:0,lang:0,scope:0},fixture_issuer()]],active:[true],graphs:[crate::table::unbound();1]}}
#[test] fn issuer_key_name_matches_suite_vector(){assert(same_term(key_name(fixture_key()),fixture_name()));}
#[test] fn issuer_registry_honest(){issuer_identity(fixture_dataset(),0,1,fixture_key(),true,[[fixture_issuer(),fixture_name()]]);}
#[test(should_fail_with="verified issuer registry")]
fn issuer_registry_rejects_wrong_signing_key(){let mut key=fixture_key();key.x=key.x+1;issuer_identity(fixture_dataset(),0,1,key,true,[[fixture_issuer(),fixture_name()]]);}
#[test(should_fail_with="verified issuer registry")]
fn issuer_registry_rejects_wrong_issuer_iri(){let mut issuer=fixture_issuer();issuer.enc=8;issuer_identity(fixture_dataset(),0,1,fixture_key(),true,[[issuer,fixture_name()]]);}
#[test(should_fail_with="verified issuer registry")]
fn issuer_registry_rejects_a_hidden_issuer_field(){let mut dataset=fixture_dataset();dataset.active=[false];issuer_identity(dataset,0,1,fixture_key(),true,[[fixture_issuer(),fixture_name()]]);}
