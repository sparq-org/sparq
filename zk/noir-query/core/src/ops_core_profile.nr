//! Witness normalization for the credential suite's bounded entry points.
//! Term construction, key checking, byte packing and authentication belong to the suite.
use crate::suite_entry::BoundedTermOpening;

pub struct ResourceOpening<let R:u32> {
    pub kind:u8,
    pub lexical:[u8;R],
    pub lexical_len:u32,
}
pub struct ObjectOpening<let L:u32> {
    pub kind:u8,
    pub lexical:[u8;L],
    pub lexical_len:u32,
    pub datatype:[u8;41],
    pub datatype_len:u32,
    pub language:[u8;1],
    pub language_len:u32,
    pub key:[u8;31],
}
pub struct QuadOpening<let R:u32,let L:u32> {
    pub subject:ResourceOpening<R>,
    pub predicate:ResourceOpening<R>,
    pub object:ObjectOpening<L>,
}

pub fn resource<let R:u32,let L:u32>(raw:ResourceOpening<R>,bound:bool) -> BoundedTermOpening<L,41,1> {
    std::static_assert(R <= L,"resource backing capacity");
    assert(raw.lexical_len <= R,"credential resource lexical capacity");
    let mut lexical=[0;L];
    for i in 0..R {lexical[i]=raw.lexical[i];}
    BoundedTermOpening {bound,kind:raw.kind,lexical,lexical_len:raw.lexical_len,
        datatype:[0;41],datatype_len:0,language:[0;1],language_len:0,key:[0;31]}
}
pub fn optional_resource<let R:u32>(raw:ResourceOpening<R>) -> BoundedTermOpening<R,41,1> {
    resource(raw,raw.kind != 0)
}
pub fn empty<let L:u32>() -> BoundedTermOpening<L,41,1> {
    BoundedTermOpening {bound:false,kind:0,lexical:[0;L],lexical_len:0,
        datatype:[0;41],datatype_len:0,language:[0;1],language_len:0,key:[0;31]}
}
pub fn normalize<let N:u32,let R:u32,let L:u32>(raw:[QuadOpening<R,L>;N],count:u32) -> [[BoundedTermOpening<L,41,1>;4];N] {
    assert(count <= N,"credential normalization count");
    let mut quads=[[empty();4];N];
    for i in 0..N {
        let bound=i < count;
        let object=raw[i].object;
        let mut graph=empty();graph.bound=bound;
        quads[i]=[resource(raw[i].subject,bound),resource(raw[i].predicate,bound),
            BoundedTermOpening {bound,kind:object.kind,lexical:object.lexical,lexical_len:object.lexical_len,
                datatype:object.datatype,datatype_len:object.datatype_len,
                language:object.language,language_len:object.language_len,key:object.key},graph];
    }
    quads
}
fn fixture() -> QuadOpening<4,32> {
    QuadOpening {subject:ResourceOpening{kind:1,lexical:[117,114,110,58],lexical_len:4},
        predicate:ResourceOpening{kind:1,lexical:[117,114,110,58],lexical_len:4},
        object:ObjectOpening{kind:1,lexical:[0;32],lexical_len:0,datatype:[0;41],datatype_len:0,
            language:[0;1],language_len:0,key:[0;31]}}
}
#[test]
fn normalized_roles_have_derived_bounds_and_zero_resource_metadata() {
    let raw=normalize([fixture()],1);
    for p in [0,1,3] {
        assert(raw[0][p].bound);assert(raw[0][p].datatype==[0;41]);
        assert(raw[0][p].language==[0;1]);assert(raw[0][p].key==[0;31]);
    }
    assert(raw[0][0].lexical[0]==117);assert(raw[0][0].lexical[4]==0);
    assert(raw[0][3].kind==0);assert(raw[0][3].lexical==[0;32]);
    let inactive=normalize([fixture()],0);
    for p in 0..4 {assert(!inactive[0][p].bound);}
}
#[test(should_fail_with="credential resource lexical capacity")]
fn normalized_resource_length_matches_its_backing_array() {
    let mut raw=fixture();raw.subject.lexical_len=5;let _quads=normalize([raw],1);
}
#[test(should_fail_with="credential normalization count")]
fn normalized_count_cannot_exceed_its_shape() {let _quads=normalize([fixture()],2);}
