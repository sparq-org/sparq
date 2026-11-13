//! Compact raw openings for the supported credential profile.
//! Resource metadata is absent by construction. Literal datatype dispatch uses
//! fixed identifiers, so each checked constructor specializes its value parser.

use crate::suite_entry::{BoundedTermOpening, open_bounded_quads};
use crate::term::{TermCell, open_key_checked};
use crate::suite_constants::{DT_INTEGER, DT_DECIMAL, DT_DATE, DT_DATETIME, DT_BOOLEAN, DT_STRING};

pub struct ResourceOpening<let L:u32> {
    pub bound:bool,
    pub kind:u8,
    pub lexical:[u8;L],
    pub lexical_len:u32,
}

pub struct ObjectOpening<let L:u32> {
    pub bound:bool,
    pub kind:u8,
    pub lexical:[u8;L],
    pub lexical_len:u32,
    // Canonical profile tag: absent/resource, integer, decimal, date,
    // dateTime, boolean, string. This is private data, not circuit structure.
    pub datatype:u8,
    pub key:[u8;31],
}

pub struct QuadOpening<let R:u32,let V:u32> {
    pub subject:ResourceOpening<R>,
    pub predicate:ResourceOpening<R>,
    pub object:ObjectOpening<V>,
}

fn resource<let L:u32>(raw:ResourceOpening<L>) -> BoundedTermOpening<L,0,0> {
    assert(!raw.bound | ((raw.kind == 1) | (raw.kind == 2)), "profile resource kind");
    BoundedTermOpening {bound:raw.bound,kind:raw.kind,lexical:raw.lexical,lexical_len:raw.lexical_len,
        datatype:[],datatype_len:0,language:[],language_len:0,key:[0;31]}
}

fn empty<let L:u32>() -> BoundedTermOpening<L,0,0> {
    BoundedTermOpening {bound:false,kind:0,lexical:[0;L],lexical_len:0,
        datatype:[],datatype_len:0,language:[],language_len:0,key:[0;31]}
}

fn predicate<let L:u32>(raw:ResourceOpening<L>) -> TermCell {
    let mut cell=crate::term::padding();
    if raw.bound {
        assert(raw.kind == 1, "profile predicate kind");
        cell=crate::suite_entry::open_bounded_cell(BoundedTermOpening {
            bound:true,kind:1,lexical:raw.lexical,lexical_len:raw.lexical_len,
            datatype:[],datatype_len:0,language:[],language_len:0,key:[0;31]},0);
    } else {
        cell=crate::suite_entry::open_bounded_cell(BoundedTermOpening {
            bound:false,kind:raw.kind,lexical:raw.lexical,lexical_len:raw.lexical_len,
            datatype:[],datatype_len:0,language:[],language_len:0,key:[0;31]},0);
    }
    cell
}

fn literal<let TAG:u32,let CHECK:u32,let L:u32,let D:u32>(raw:ObjectOpening<L>, datatype:[u8;D]) -> TermCell {
    std::static_assert((TAG >= 1) & (TAG <= 6), "profile datatype specialization");
    let expected=if TAG == 1 { DT_INTEGER } else if TAG == 2 { DT_DECIMAL }
        else if TAG == 3 { DT_DATE } else if TAG == 4 { DT_DATETIME }
        else if TAG == 5 { DT_BOOLEAN } else { DT_STRING };
    let opening=BoundedTermOpening {bound:true,kind:3,lexical:raw.lexical,lexical_len:raw.lexical_len,
        datatype,datatype_len:D,language:[],language_len:0,key:raw.key};
    if CHECK == 1 { open_key_checked(opening,0,expected).cell() }
    else { crate::suite_entry::open_bounded_cell(opening,0) }
}

// Scalar lexical capacity is checked independently from resource/string capacity.
// The complete backing array is checked before narrowing, including unused bytes.
fn scalar<let L:u32>(raw:ObjectOpening<L>) -> ObjectOpening<32> {
    std::static_assert(L >= 32, "profile scalar source capacity");
    assert(raw.lexical_len <= 32, "profile scalar lexical capacity");
    let mut lexical=[0;32];
    for i in 0..L {
        if i < 32 { lexical[i] = raw.lexical[i]; }
        else { assert(raw.lexical[i] == 0, "profile scalar backing padding"); }
    }
    ObjectOpening {bound:raw.bound,kind:raw.kind,lexical,lexical_len:raw.lexical_len,datatype:raw.datatype,key:raw.key}
}

// Root authentication shares a scalar term hash across the admitted datatypes.
// Value consumers still enter their fixed datatype checked constructor.
fn identity_scalar(raw:ObjectOpening<32>) -> TermCell {
    assert((raw.datatype >= 1) & (raw.datatype <= 5), "profile scalar datatype");
    let (suffix,length)=if raw.datatype == 1 { ([105,110,116,101,103,101,114,0],7) }
        else if raw.datatype == 2 { ([100,101,99,105,109,97,108,0],7) }
        else if raw.datatype == 3 { ([100,97,116,101,0,0,0,0],4) }
        else if raw.datatype == 4 { ([100,97,116,101,84,105,109,101],8) }
        else { ([98,111,111,108,101,97,110,0],7) };
    let prefix="http://www.w3.org/2001/XMLSchema#".as_bytes();
    let mut datatype:[u8;41]=[0;41];
    for i in 0..33 { datatype[i]=prefix[i]; }
    for i in 0..8 { datatype[33+i]=suffix[i]; }
    crate::suite_entry::open_bounded_cell(BoundedTermOpening {
        bound:true,kind:3,lexical:raw.lexical,lexical_len:raw.lexical_len,
        datatype,datatype_len:33+length,language:[],language_len:0,key:raw.key},0)
}

pub fn open_object<let L:u32>(raw:ObjectOpening<L>, scope:u32) -> TermCell {
    object::<1,L>(raw,scope)
}

fn object<let CHECK:u32,let L:u32>(raw:ObjectOpening<L>, scope:u32) -> TermCell {
    std::static_assert(CHECK <= 1, "profile key consumption mode");
    let mut value=crate::term::padding();
    if (CHECK == 0) & raw.bound & (raw.kind == 3) & (raw.datatype != 6) {
        value=identity_scalar(scalar(raw));
    } else if raw.bound & (raw.kind == 3) {
        if raw.datatype == 1 {
            value=literal::<1,CHECK,32,40>(scalar(raw),"http://www.w3.org/2001/XMLSchema#integer".as_bytes());
        } else if raw.datatype == 2 {
            value=literal::<2,CHECK,32,40>(scalar(raw),"http://www.w3.org/2001/XMLSchema#decimal".as_bytes());
        } else if raw.datatype == 3 {
            value=literal::<3,CHECK,32,37>(scalar(raw),"http://www.w3.org/2001/XMLSchema#date".as_bytes());
        } else if raw.datatype == 4 {
            value=literal::<4,CHECK,32,41>(scalar(raw),"http://www.w3.org/2001/XMLSchema#dateTime".as_bytes());
        } else if raw.datatype == 5 {
            value=literal::<5,CHECK,32,40>(scalar(raw),"http://www.w3.org/2001/XMLSchema#boolean".as_bytes());
        } else {
            assert(raw.datatype == 6, "profile literal datatype");
            value=literal::<6,CHECK,L,39>(raw,"http://www.w3.org/2001/XMLSchema#string".as_bytes());
        }
    } else {
        assert(raw.datatype == 0, "profile resource datatype");
        assert(!raw.bound | ((raw.kind == 1) | (raw.kind == 2)), "profile object kind");
        value=crate::suite_entry::open_bounded_cell(BoundedTermOpening {
            bound:raw.bound,kind:raw.kind,lexical:raw.lexical,lexical_len:raw.lexical_len,
            datatype:[],datatype_len:0,language:[],language_len:0,key:raw.key},scope);
    }
    value
}

/// Each resource enters the bounded table constructor once, and each object
/// enters either its fixed datatype checked constructor or the resource opener.
pub fn open_quads<let N:u32,let R:u32,let V:u32>(raw:[QuadOpening<R,V>;N],count:u32,index:u32) -> [[TermCell;4];N] {
    construct_quads::<1,N,R,V>(raw,count,index)
}

/// Identity-only input for root authentication. Every value-key consumer must
/// separately use its checked lexical opening before comparisons or functions.
pub fn open_identity_quads<let N:u32,let R:u32,let V:u32>(raw:[QuadOpening<R,V>;N],count:u32,index:u32) -> [[TermCell;4];N] {
    construct_quads::<0,N,R,V>(raw,count,index)
}

fn construct_quads<let CHECK:u32,let N:u32,let R:u32,let V:u32>(raw:[QuadOpening<R,V>;N],count:u32,index:u32) -> [[TermCell;4];N] {
    assert(index < 0xffffffff, "credential scope capacity");
    assert(count <= N, "profile quad count");
    let mut resources=[[empty();4];N];
    for i in 0..N {
        resources[i][0]=resource(raw[i].subject);
        resources[i][3].bound=i < count;
    }
    let mut quads=open_bounded_quads(resources,index+1);
    for i in 0..N {
        quads[i][1]=predicate(raw[i].predicate);
        quads[i][2]=object::<CHECK,V>(raw[i].object,index+1);
    }
    quads
}

fn fixture<let L:u32,let T:u32>(text:[u8;T],datatype:u8,key:[u8;31]) -> ObjectOpening<L> {
    let mut lexical=[0;L];
    for i in 0..T { lexical[i]=text[i]; }
    ObjectOpening {bound:true,kind:3,lexical,lexical_len:T,datatype,key}
}
fn integer() -> ObjectOpening<64> {
    let bytes: [u8;32]=(0x01800000000000000000000000000000000000000000006124fee993bc0000 as Field).to_be_bytes();
    let mut key=[0;31];for i in 0..31 {key[i]=bytes[i+1];}
    fixture("7".as_bytes(),1,key)
}
fn matches<let L:u32,let D:u32>(raw:ObjectOpening<L>,dt:[u8;D]) {
    let expected=crate::suite_entry::open_bounded_cell(BoundedTermOpening {
        bound:true,kind:3,lexical:raw.lexical,lexical_len:raw.lexical_len,
        datatype:dt,datatype_len:D,language:[],language_len:0,key:raw.key},0);
    let actual=open_object(raw,9);
    assert(crate::term::same_term(expected,actual));
    assert((expected.key()==actual.key()) & (expected.datatype()==actual.datatype()));
    let identity=object::<0,L>(raw,9);
    assert(crate::term::same_term(actual,identity));
    assert((actual.key()==identity.key()) & (actual.datatype()==identity.datatype())
        & (actual.language()==identity.language()));
}

#[test]
fn profile_checked_integer_matches_the_suite() { matches(integer(),"http://www.w3.org/2001/XMLSchema#integer".as_bytes()); }
#[test]
fn profile_checked_decimal_matches_the_suite() {
    let mut raw=integer();raw.datatype=2;
    matches(raw,"http://www.w3.org/2001/XMLSchema#decimal".as_bytes());
}
#[test]
fn profile_checked_date_matches_the_suite() {
    let raw=fixture::<64,10>("6743-12-20".as_bytes(),3,[8,128,0,0,0,0,26,155,39,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
    matches(raw,"http://www.w3.org/2001/XMLSchema#date".as_bytes());
}
#[test]
fn profile_checked_datetime_matches_the_suite() {
    let raw=fixture::<64,20>("6743-12-20T00:00:00Z".as_bytes(),4,[3,128,0,137,4,90,140,4,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
    matches(raw,"http://www.w3.org/2001/XMLSchema#dateTime".as_bytes());
}
#[test]
fn profile_checked_boolean_matches_the_suite() {
    let raw=fixture::<64,4>("true".as_bytes(),5,[5,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
    matches(raw,"http://www.w3.org/2001/XMLSchema#boolean".as_bytes());
}
#[test]
fn profile_checked_string_matches_the_suite() {
    let mut key=[0;31];key[0]=6;key[1]=97;
    matches(fixture::<64,1>("a".as_bytes(),6,key),"http://www.w3.org/2001/XMLSchema#string".as_bytes());
}
#[test(should_fail_with="key disagrees with lexical form")]
fn profile_checks_the_literal_key() {let mut raw=integer();raw.lexical[0]=56;let _cell=open_object(raw,1);}
#[test(should_fail_with="profile literal datatype")]
fn profile_refuses_an_unknown_literal_datatype() {
    let mut key=[0;31];key[0]=6;key[1]=97;
    let _cell=open_object(fixture::<64,1>("a".as_bytes(),7,key),1);
}
#[test(should_fail_with="profile scalar backing padding")]
fn profile_checks_scalar_bytes_after_narrowing() {let mut raw=integer();raw.lexical[63]=1;let _cell=open_object(raw,1);}
#[test(should_fail_with="profile scalar lexical capacity")]
fn profile_refuses_an_overlong_scalar() {let mut raw=integer();raw.lexical_len=33;let _cell=open_object(raw,1);}
#[test(should_fail_with="profile resource datatype")]
fn profile_requires_zero_absent_datatype() {
    let raw=ObjectOpening {bound:false,kind:0,lexical:[0;64],lexical_len:0,datatype:1,key:[0;31]};
    let _cell=open_object(raw,1);
}
#[test(should_fail_with="profile object kind")]
fn profile_objects_cannot_be_default_graphs() {
    let raw=ObjectOpening {bound:true,kind:0,lexical:[0;64],lexical_len:0,datatype:0,key:[0;31]};
    let _cell=open_object(raw,1);
}
#[test(should_fail_with="profile resource kind")]
fn profile_resources_cannot_be_default_graphs() {
    let _raw=resource(ResourceOpening{bound:true,kind:0,lexical:[0;32],lexical_len:0});
}

fn resource_fixture<let L:u32,let T:u32>(text:[u8;T],kind:u8) -> ResourceOpening<L> {
    let mut lexical=[0;L];for i in 0..T {lexical[i]=text[i];}
    ResourceOpening {bound:true,kind,lexical,lexical_len:T}
}
fn quad() -> QuadOpening<32,64> {
    QuadOpening {subject:resource_fixture("b0".as_bytes(),2),
        predicate:resource_fixture("urn:p".as_bytes(),1),object:integer()}
}
#[test]
fn profile_quad_positions_and_scope_are_constructed_once() {
    let cells=open_quads([quad()],1,8);
    let identities=open_identity_quads([quad()],1,8);
    for p in 0..4 { assert(crate::term::same_term(cells[0][p],identities[0][p])); }
    assert(cells[0][0].kind()==2);assert(cells[0][0].credential()==9);
    assert(cells[0][1].kind()==1);assert(cells[0][1].credential()==0);
    assert(cells[0][2].datatype()==DT_INTEGER);
    assert(cells[0][3].bound());assert(cells[0][3].kind()==0);
}
#[test(should_fail_with="credential scope capacity")]
fn profile_checks_the_credential_scope_range() {let _cells=open_quads([quad()],1,0xffffffff);}
#[test(should_fail_with="profile quad count")]
fn profile_checks_its_public_quad_capacity() {let _cells=open_quads([quad()],2,0);}
#[test(should_fail_with="profile scalar datatype")]
fn profile_identity_scalar_tag_has_an_upper_bound() {
    let raw=fixture::<64,4>("true".as_bytes(),7,[5,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
    let _cell=object::<0,64>(raw,1);
}
#[test(should_fail_with="profile scalar datatype")]
fn profile_identity_scalar_tag_is_present_for_literals() {
    let raw=fixture::<64,4>("true".as_bytes(),0,[5,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
    let _cell=object::<0,64>(raw,1);
}
#[test(should_fail_with="profile predicate kind")]
fn profile_predicates_require_the_iri_kind() {
    let _cell=predicate(resource_fixture::<32,5>("urn:p".as_bytes(),2));
}
