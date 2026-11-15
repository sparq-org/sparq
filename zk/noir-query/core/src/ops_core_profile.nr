//! Witness normalization for the credential suite's bounded entry points.
//! Term construction, key checking, byte packing and authentication belong to the suite.
use crate::suite_entry::{BoundedTermOpening, DigestQuadOpening, ResourceDigest};

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
/// A subject or predicate as its framed term digest. The signed root binds the
/// encoding P(kind, framed, 0), so no byte opening is needed.
pub struct ResourceDigestOpening {
    pub kind:u8,
    pub framed:Field,
}
/// Credential quad witness: resources by digest, the object by bytes (its datatype
/// dispatches value reads). `R` is the opening profile's resource limit, which the
/// digest opening no longer reads; it keeps the profile in the program's types.
pub struct QuadOpening<let R:u32,let L:u32> {
    pub subject:ResourceDigestOpening,
    pub predicate:ResourceDigestOpening,
    pub object:ObjectOpening<L>,
}
/// The digest of a byte resource opening, for fixtures and harnesses.
pub fn digest<let R:u32>(raw:ResourceOpening<R>) -> ResourceDigestOpening {
    assert(raw.lexical_len <= R,"credential resource lexical capacity");
    let framed=if raw.kind==0 {0} else {
        crate::term_bytes::nonliteral_hash(raw.kind,std::collections::bounded_vec::BoundedVec::from_parts_unchecked(raw.lexical,raw.lexical_len))
    };
    ResourceDigestOpening {kind:raw.kind,framed}
}
fn resource_digest(raw:ResourceDigestOpening,bound:bool) -> ResourceDigest {
    ResourceDigest {bound,kind:raw.kind,framed:raw.framed}
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
/// Retain the proof field's backing while enforcing the selected method bound.
pub fn method<let M:u32>(raw:ResourceOpening<128>) -> BoundedTermOpening<M,41,1> {
    std::static_assert((M > 0) & (M <= 128),"method profile backing");
    assert(raw.lexical_len <= M,"credential method lexical capacity");
    let mut lexical=[0;M];let mut tail:Field=0;
    for i in 0..128 {
        if i < M {lexical[i]=raw.lexical[i];} else {tail=tail+raw.lexical[i] as Field;}
    }
    assert(tail==0,"credential method backing padding");
    BoundedTermOpening {bound:raw.kind != 0,kind:raw.kind,lexical,lexical_len:raw.lexical_len,
        datatype:[0;41],datatype_len:0,language:[0;1],language_len:0,key:[0;31]}
}
pub fn empty<let L:u32>() -> BoundedTermOpening<L,41,1> {
    BoundedTermOpening {bound:false,kind:0,lexical:[0;L],lexical_len:0,
        datatype:[0;41],datatype_len:0,language:[0;1],language_len:0,key:[0;31]}
}
/// Position-derived bounds over the digest witness; every quad is in the default graph.
pub fn normalize<let N:u32,let R:u32,let L:u32>(raw:[QuadOpening<R,L>;N],count:u32) -> [DigestQuadOpening<L,41,1>;N] {
    assert(count <= N,"credential normalization count");
    let padding=ResourceDigest {bound:false,kind:0,framed:0};
    let mut quads=[DigestQuadOpening {subject:padding,predicate:padding,object:empty(),graph:padding};N];
    for i in 0..N {
        let bound=i < count;
        let object=raw[i].object;
        quads[i]=DigestQuadOpening {subject:resource_digest(raw[i].subject,bound),
            predicate:resource_digest(raw[i].predicate,bound),
            object:BoundedTermOpening {bound,kind:object.kind,lexical:object.lexical,lexical_len:object.lexical_len,
                datatype:object.datatype,datatype_len:object.datatype_len,
                language:object.language,language_len:object.language_len,key:object.key},
            graph:ResourceDigest {bound,kind:0,framed:0}};
    }
    quads
}
fn fixture() -> QuadOpening<4,32> {
    QuadOpening {subject:digest(ResourceOpening{kind:1,lexical:[117,114,110,58],lexical_len:4}),
        predicate:digest(ResourceOpening{kind:1,lexical:[117,114,110,58],lexical_len:4}),
        object:ObjectOpening{kind:1,lexical:[0;32],lexical_len:0,datatype:[0;41],datatype_len:0,
            language:[0;1],language_len:0,key:[0;31]}}
}
#[test]
fn normalized_roles_have_derived_bounds_and_zero_resource_metadata() {
    let raw=normalize([fixture()],1);
    assert(raw[0].subject.bound & raw[0].predicate.bound & raw[0].object.bound & raw[0].graph.bound);
    assert(raw[0].subject.framed==fixture().subject.framed);assert(raw[0].subject.framed!=0);
    assert((raw[0].graph.kind==0) & (raw[0].graph.framed==0));
    let inactive=normalize([fixture()],0);
    assert(!inactive[0].subject.bound & !inactive[0].predicate.bound & !inactive[0].object.bound & !inactive[0].graph.bound);
}
#[test(should_fail_with="credential resource lexical capacity")]
fn resource_digest_length_matches_its_backing_array() {
    let _digest=digest(ResourceOpening{kind:1,lexical:[117,114,110,58],lexical_len:5});
}
#[test(should_fail_with="credential normalization count")]
fn normalized_count_cannot_exceed_its_shape() {let _quads=normalize([fixture()],2);}
#[test]
fn selected_method_profile_keeps_all_admitted_bytes() {
    let mut raw=ResourceOpening {kind:1,lexical:[0;128],lexical_len:32};
    raw.lexical[31]=97;
    let opened=method::<32>(raw);assert(opened.lexical[31]==97);assert(opened.lexical_len==32);
    raw.lexical[100]=98;raw.lexical_len=101;
    let wide=method::<128>(raw);assert(wide.lexical[100]==98);assert(wide.lexical_len==101);
}
#[test(should_fail_with="credential method lexical capacity")]
fn selected_method_profile_refuses_longer_lexical_forms() {
    let raw=ResourceOpening {kind:1,lexical:[0;128],lexical_len:33};let _opened=method::<32>(raw);
}
#[test(should_fail_with="credential method backing padding")]
fn selected_method_profile_refuses_hidden_backing_bytes() {
    let mut raw=ResourceOpening {kind:1,lexical:[0;128],lexical_len:1};raw.lexical[90]=1;
    let _opened=method::<32>(raw);
}
