//! Bounded credential adapter for the closed graph profile.
//! Lane A owns term hashing/key relations; lane E owns identity and the envelope.
//! Datatype bytes are reconstructed for integer, local date and boolean literals.
use crate::ops_core_suite::{CredentialProof,OpenedCredential};
use crate::suite_constants::{DT_INTEGER,DT_DATE,DT_BOOLEAN};
use crate::table::unbound;
use std::collections::bounded_vec::BoundedVec;
pub struct Opening<let L:u32> {pub bound:bool,pub kind:u8,pub lexical:[u8;L],pub lexical_len:u32,pub datatype_tag:u8,pub key:[u8;31]}
fn datatype<let L:u32>(raw:Opening<L>)->BoundedVec<u8,40>{
    assert(raw.datatype_tag<=3,"closed datatype tag");
    assert((raw.kind==3)==(raw.datatype_tag>0),"closed datatype kind");
    let mut dt:BoundedVec<u8,40> = BoundedVec::new();
    if raw.datatype_tag==1 {for b in "http://www.w3.org/2001/XMLSchema#integer".as_bytes(){dt.push(b);}}
    if raw.datatype_tag==2 {for b in "http://www.w3.org/2001/XMLSchema#boolean".as_bytes(){dt.push(b);}}
    if raw.datatype_tag==3 {for b in "http://www.w3.org/2001/XMLSchema#date".as_bytes(){dt.push(b);}}
    dt
}
fn open_cell<let L:u32>(raw:Opening<L>,credential:u32)->crate::term::TermCell{
    let dt=datatype(raw);
    let mut out=crate::term::padding();
    if raw.bound {out=crate::term::open_term(raw.kind,BoundedVec::from_parts_unchecked(raw.lexical,raw.lexical_len),dt,BoundedVec::<u8,0>::new(),raw.key,credential);}
    else {assert((raw.kind==0)&(raw.lexical_len==0)&(raw.lexical==[0;L])&(raw.datatype_tag==0)&(raw.key==[0;31]),"non-zero padding");}
    out
}
fn open_nonliteral<let L:u32>(raw:Opening<L>,credential:u32)->crate::term::TermCell{
    assert(raw.datatype_tag==0,"non-literal datatype");
    let mut out=crate::term::padding();
    if raw.bound {
        assert(raw.kind<=2,"non-literal kind");
        out=crate::term::open_term(raw.kind,BoundedVec::from_parts_unchecked(raw.lexical,raw.lexical_len),BoundedVec::<u8,0>::new(),BoundedVec::<u8,0>::new(),raw.key,credential);
    } else {assert((raw.kind==0)&(raw.lexical_len==0)&(raw.lexical==[0;L])&(raw.key==[0;31]),"non-zero padding");}
    out
}
fn open_default<let L:u32>(raw:Opening<L>)->crate::term::TermCell{
    assert(raw.kind==0,"credential named graph not yet supported");
    assert((raw.datatype_tag==0)&(raw.lexical_len==0)&(raw.lexical==[0;L])&(raw.key==[0;31]),"default graph opening");
    if raw.bound {crate::term::open_term(0,BoundedVec::<u8,0>::new(),BoundedVec::<u8,0>::new(),BoundedVec::<u8,0>::new(),[0;31],0)} else {crate::term::padding()}
}
fn checked_for<let L:u32,let TYPES:u32,let KEYL:u32>(cell:crate::term::TermCell,raw:Opening<L>){
    assert(raw.lexical_len<=KEYL,"closed credential literal capacity");
    let mut bytes=[0;KEYL];for i in 0..L {if i<KEYL {bytes[i]=raw.lexical[i];}}
    let lexical=BoundedVec::from_parts_unchecked(bytes,raw.lexical_len);
    let dt=datatype(raw);let language=BoundedVec::<u8,0>::new();let mut supported=false;
    if (TYPES & 1)!=0 {if cell.datatype()==DT_INTEGER {let _key=crate::term::recompute_key_for(cell,lexical,dt,language,DT_INTEGER);supported=true;}}
    if (TYPES & 2)!=0 {if cell.datatype()==DT_BOOLEAN {let _key=crate::term::recompute_key_for(cell,lexical,dt,language,DT_BOOLEAN);supported=true;}}
    if (TYPES & 4)!=0 {if cell.datatype()==DT_DATE {let _key=crate::term::recompute_key_for(cell,lexical,dt,language,DT_DATE);supported=true;}}
    assert(supported,"closed credential literal profile");
}
fn checked<let L:u32>(cell:crate::term::TermCell,raw:Opening<L>){checked_for::<L,7,L>(cell,raw);}
/// The plan pins these families; the check covers every active input row.
pub fn literal_types<let D:u32>(dataset:crate::ops_core::Dataset<D>,types:u32){
    assert((types==1)|(types==6)|(types==7),"closed credential literal profile");
    for i in 0..D {
        let c=dataset.rows[i][2];
        let allowed=(((types & 1)!=0)&(c.dt==DT_INTEGER))|(((types & 2)!=0)&(c.dt==DT_BOOLEAN))|(((types & 4)!=0)&(c.dt==DT_DATE));
        assert(!dataset.active[i]|(c.kind!=3)|allowed,"closed credential literal profile");
    }
}
fn open_typed<let N:u32,let L:u32,let TYPES:u32,let KEYL:u32>(raw:[[Opening<L>;4];N],proof:CredentialProof<N>,count:u32,active:bool,quad_bound:u32,credential_index:u32,issuer_root:Field)->OpenedCredential<N>{
    assert((quad_bound<=N)&(count<=quad_bound),"per-credential quad count");
    let mut quads=[[crate::term::padding();4];N];
    for i in 0..N {
        if raw[i][0].bound {assert((raw[i][0].kind==1)|(raw[i][0].kind==2),"subject kind");}
        if raw[i][1].bound {assert(raw[i][1].kind==1,"predicate kind");}
        quads[i][0]=open_nonliteral(raw[i][0],credential_index+1);
        quads[i][1]=open_nonliteral(raw[i][1],credential_index+1);
        quads[i][2]=open_cell(raw[i][2],credential_index+1);
        quads[i][3]=open_default(raw[i][3]);
    }
    let mut out=OpenedCredential{rows:[[unbound();3];N],active:[false;N],message:0};
    if active {
        out.message=crate::suite::open_credential_revealed(quads,proof.salts,proof.salt,count,proof.config);
        compose::issuer::schnorr_verify(proof.pk,proof.r,proof.s,proof.e,proof.e_k,out.message);
        compose::issuer::key_set_membership(proof.pk,issuer_root,proof.issuer_index,proof.issuer_siblings);
        for i in 0..N {
            // The suite root constrains subject and predicate kinds. Only objects can be literals.
            if quads[i][2].bound() & (quads[i][2].kind()==3) {checked_for::<L,TYPES,KEYL>(quads[i][2],raw[i][2]);}
            for p in 0..3 {if quads[i][p].bound(){
                let c=quads[i][p];
                out.rows[i][p]=crate::table::TermCell{bound:c.bound(),kind:c.kind(),enc:c.enc(),key_class:c.key_class(),key:c.key(),dt:c.datatype(),lang:c.language(),scope:c.credential()};
            }}
            out.active[i]=i<count;
            if i<count {assert(quads[i][3].kind()==0,"credential named graph not yet supported");}
        }
    }else{
        assert(count==0,"credential padding count");
        for i in 0..N {for p in 0..4 {quads[i][p].assert_padding();}}
        assert((proof.salt==0)&(proof.salts==[0;N])&(proof.config==[0;2]),"credential proof padding");
        assert((proof.pk.x==0)&(proof.pk.y==0)&(proof.r.x==0)&(proof.r.y==0)&(proof.s==0)&(proof.e==0)&(proof.e_k==0)&(proof.issuer_index==0)&(proof.issuer_siblings==[0;4]),"credential signature padding");
    }
    out
}
pub fn open_one<let N:u32,let L:u32>(raw:[[Opening<L>;4];N],proof:CredentialProof<N>,count:u32,active:bool,quad_bound:u32,credential_index:u32,issuer_root:Field)->OpenedCredential<N>{open_typed::<N,L,7,L>(raw,proof,count,active,quad_bound,credential_index,issuer_root)}
pub fn open_integer<let N:u32,let L:u32>(raw:[[Opening<L>;4];N],proof:CredentialProof<N>,count:u32,active:bool,quad_bound:u32,credential_index:u32,issuer_root:Field)->OpenedCredential<N>{open_typed::<N,L,1,16>(raw,proof,count,active,quad_bound,credential_index,issuer_root)}
pub fn open_date_boolean<let N:u32,let L:u32>(raw:[[Opening<L>;4];N],proof:CredentialProof<N>,count:u32,active:bool,quad_bound:u32,credential_index:u32,issuer_root:Field)->OpenedCredential<N>{open_typed::<N,L,6,16>(raw,proof,count,active,quad_bound,credential_index,issuer_root)}
fn empty_opening()->Opening<1>{Opening{bound:false,kind:0,lexical:[0],lexical_len:0,datatype_tag:0,key:[0;31]}}
fn empty_proof()->CredentialProof<1>{CredentialProof{salt:0,salts:[0],config:[0;2],pk:compose::issuer::Point{x:0,y:0},r:compose::issuer::Point{x:0,y:0},s:0,e:0,e_k:0,issuer_index:0,issuer_siblings:[0;4]}}
#[test] fn compact_padding_is_checked_by_the_suite(){let out=open_one([[empty_opening();4]],empty_proof(),0,false,1,0,0);assert((out.active==[false])&(out.message==0));}
#[test(should_fail_with="closed datatype tag")]
fn compact_opening_rejects_unknown_datatype(){let mut raw=empty_opening();raw.datatype_tag=4;let _dt=datatype(raw);}
#[test(should_fail_with="closed datatype kind")]
fn compact_opening_rejects_iri_with_literal_datatype(){let mut raw=empty_opening();raw.kind=1;raw.datatype_tag=1;let _dt=datatype(raw);}
#[test(should_fail_with="non-zero padding")]
fn compact_opening_rejects_nonzero_padding(){let mut raw=empty_opening();raw.lexical=[1];let _out=open_one([[raw;4]],empty_proof(),0,false,1,0,0);}
#[test(should_fail_with="key disagrees with lexical form")]
fn compact_integer_key_is_recomputed(){
    let mut raw=empty_opening();raw.bound=true;raw.kind=3;raw.lexical=[50];raw.lexical_len=1;raw.datatype_tag=1;
    let cell=crate::ops_closed_numeric_standin::encode(crate::ops_closed_numeric_standin::OFFSET+1000000000000000000,false);
    let bytes=(cell.key+crate::ops_closed_numeric_standin::PREFIX).to_be_bytes::<32>();for i in 0..31 {raw.key[i]=bytes[i+1];}
    checked(open_cell(raw,1),raw);
}
fn bound_iri()->Opening<1>{let mut raw=empty_opening();raw.bound=true;raw.kind=1;raw.lexical=[97];raw.lexical_len=1;raw}
fn bound_literal()->Opening<1>{let mut raw=empty_opening();raw.bound=true;raw.kind=3;raw.lexical=[49];raw.lexical_len=1;raw.datatype_tag=1;raw}
fn bound_default()->Opening<1>{let mut raw=empty_opening();raw.bound=true;raw}
#[test(should_fail_with="subject kind")]
fn compact_subject_cannot_bypass_the_suite_kind_constraint(){let _out=open_one([[bound_literal(),bound_iri(),bound_iri(),bound_default()]],empty_proof(),1,true,1,0,0);}
#[test(should_fail_with="predicate kind")]
fn compact_predicate_cannot_bypass_the_suite_kind_constraint(){let _out=open_one([[bound_iri(),bound_literal(),bound_iri(),bound_default()]],empty_proof(),1,true,1,0,0);}
#[test(should_fail_with="closed credential literal profile")]
fn integer_profile_rejects_a_boolean_in_the_last_active_row(){
    let c=crate::table::TermCell{bound:true,kind:3,enc:1,key_class:5,key:0,dt:DT_BOOLEAN,lang:0,scope:0};
    literal_types(crate::ops_core::Dataset{rows:[[unbound();3],[unbound();3],[unbound(),unbound(),c]],active:[false,false,true]},1);
}
#[test]
fn date_boolean_profile_accepts_both_families(){
    let c=crate::table::TermCell{bound:true,kind:3,enc:1,key_class:5,key:0,dt:DT_BOOLEAN,lang:0,scope:0};let mut d=c;d.dt=DT_DATE;
    literal_types(crate::ops_core::Dataset{rows:[[unbound(),unbound(),c],[unbound(),unbound(),d]],active:[true,true]},6);
}
#[test(should_fail_with="closed credential literal profile")]
fn date_boolean_profile_rejects_an_integer(){
    let c=crate::table::TermCell{bound:true,kind:3,enc:1,key_class:1,key:0,dt:DT_INTEGER,lang:0,scope:0};
    literal_types(crate::ops_core::Dataset{rows:[[unbound(),unbound(),c]],active:[true]},6);
}
#[test(should_fail_with="closed credential literal capacity")]
fn typed_literal_profile_rejects_an_oversized_lexical_value(){
    let raw=Opening{bound:true,kind:3,lexical:[49;17],lexical_len:17,datatype_tag:1,key:[0;31]};
    checked_for::<17,1,16>(open_cell(raw,1),raw);
}
#[test]
fn compact_key_opening_matches_larger_term_opening(){
    let mut lexical=[0;64];lexical[0]=49;
    let bytes=(crate::ops_closed_numeric_standin::PREFIX+crate::ops_closed_numeric_standin::OFFSET+1000000000000000000).to_be_bytes::<32>();
    let mut key=[0;31];for i in 0..31 {key[i]=bytes[i+1];}
    let raw=Opening{bound:true,kind:3,lexical,lexical_len:1,datatype_tag:1,key};
    checked_for::<64,1,16>(open_cell(raw,1),raw);
}
#[test]
fn nonliteral_openings_match_the_general_suite_constructor(){
    let raw=bound_iri();let a=open_nonliteral(raw,1);let b=open_cell(raw,1);assert((a.enc()==b.enc())&(a.kind()==b.kind())&(a.bound()==b.bound()));
    let mut blank=raw;blank.kind=2;let a=open_nonliteral(blank,1);let b=open_cell(blank,1);assert((a.enc()==b.enc())&(a.credential()==b.credential()));
}
#[test]
fn constant_default_opening_matches_the_general_suite_constructor(){
    let raw=bound_default();let a=open_default(raw);let b=open_cell(raw,1);assert(a.bound()&(a.kind()==0)&(a.enc()==b.enc()));
}
