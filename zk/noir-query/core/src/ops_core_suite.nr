//! Suite authentication and conversion into the operator table vocabulary.
use std::collections::bounded_vec::BoundedVec;
use compose::issuer::{Point, key_set_membership};
use crate::suite_entry::{TermOpening, BoundedTermOpening, open_quads, open_bounded_quads};
use crate::suite::open_credential_hidden;
use crate::suite_constants::{DT_INTEGER, DT_DECIMAL, DT_DATE, DT_DATETIME, DT_BOOLEAN, DT_STRING};
use crate::table::{TermCell, unbound};
use crate::ops_core::Dataset;
use crate::ops_core_profile::{ResourceOpening, ObjectOpening, QuadOpening};

pub global VERIFIED_ISSUER_ENCODING: Field = 0x09eec572ceda200a642295e68e09118d7e1d26ea1a082cc560e303164907f5e9;
fn check_input_predicate(enc:Field) { assert(enc != VERIFIED_ISSUER_ENCODING, "credential verifiedIssuer assertion"); }
fn issuer_predicate() -> TermCell {
    TermCell {bound:true,kind:1,enc:VERIFIED_ISSUER_ENCODING,key_class:0,key:0,dt:0,lang:0,scope:0}
}

pub struct CredentialProof<let N: u32> {
    pub salt: Field,
    pub salts: [Field; N],
    pub config: [u128; 2],
    pub created: TermOpening<32>,
    pub method: TermOpening<128>,
    pub pk: Point,
    pub r: Point,
    pub s: Field,
    pub e: Field,
    pub e_k: Field,
    pub issuer_index: Field,
    pub issuer_siblings: [Field; 4],
}

fn checked_literal<let L: u32>(cell: crate::term::TermCell, raw: TermOpening<L>) {
    let lexical = BoundedVec::from_parts_unchecked(raw.lexical, raw.lexical_len);
    let datatype = BoundedVec::from_parts_unchecked(raw.datatype, raw.datatype_len);
    let language = BoundedVec::from_parts_unchecked(raw.language, raw.language_len);
    if cell.datatype() == DT_INTEGER {
        let _checked = crate::term::recompute_key_for(cell, lexical, datatype, language, DT_INTEGER);
    } else if cell.datatype() == DT_DECIMAL {
        let _checked = crate::term::recompute_key_for(cell, lexical, datatype, language, DT_DECIMAL);
    } else if cell.datatype() == DT_DATE {
        let _checked = crate::term::recompute_key_for(cell, lexical, datatype, language, DT_DATE);
    } else if cell.datatype() == DT_DATETIME {
        let _checked = crate::term::recompute_key_for(cell, lexical, datatype, language, DT_DATETIME);
    } else if cell.datatype() == DT_BOOLEAN {
        let _checked = crate::term::recompute_key_for(cell, lexical, datatype, language, DT_BOOLEAN);
    } else {
        assert(cell.datatype() == DT_STRING, "N0 credential literal datatype");
        let _checked = crate::term::recompute_key_for(cell, lexical, datatype, language, DT_STRING);
    }
}

fn table_cell(cell: crate::term::TermCell) -> TermCell {
    TermCell {bound: cell.bound(), kind: cell.kind(), enc: cell.enc(),
        key_class: cell.key_class(), key: cell.key(), dt: cell.datatype(),
        lang: cell.language(), scope: cell.credential()}
}

pub struct OpenedCredential<let N: u32> {
    pub rows: [[TermCell; 3]; N],
    pub active: [bool; N],
    pub message: Field,
    pub graph: TermCell,
}

pub fn credential_flags<let C: u32>(count: u32, bound: u32, signature_mode: u8) -> [bool; C] {
    assert(signature_mode == 1, "hidden suite signature mode");
    assert((bound > 0) & (bound <= C) & (count <= bound), "credential count");
    let mut flags = [false; C];
    for c in 0..C { flags[c] = c < count; }
    flags
}

pub fn credential_mode_flags<let C:u32>(count:u32,bound:u32,signature_mode:u8,
    revealed_count:u32,revealed_messages:[Field;C],revealed_keys:[Field;C]) -> [bool;C] {
    assert(signature_mode<=1,"suite signature mode");
    assert((bound>0)&(bound<=C)&(count<=bound),"credential count");
    assert(revealed_count==if signature_mode==0 {count}else{0},"revealed credential count");
    let mut flags=[false;C];
    for c in 0..C {
        flags[c]=c<count;
        if (signature_mode==1)|(c>=count) {
            assert((revealed_messages[c]==0)&(revealed_keys[c]==0),"revealed input padding");
        }
    }
    flags
}

pub fn open_one<let N: u32, let L: u32>(
    credentials: [[TermOpening<L>; 4]; N], proof: CredentialProof<N>,
    count: u32, active: bool, quad_bound: u32, credential_index: u32, issuer_root: Field,
    request_digest: [u8;32],
) -> OpenedCredential<N> {
    check_limits::<N>(count, quad_bound);
    let mut opened = OpenedCredential {rows: [[unbound(); 3]; N], active: [false; N], message: 0, graph: unbound()};
    if active {
        let authenticated = authenticate(credentials,proof,count,quad_bound,credential_index);
        key_set_membership(proof.pk, issuer_root, proof.issuer_index, proof.issuer_siblings);
        opened=from_authenticated(authenticated,credentials,credential_index,request_digest);
    } else {
        check_inactive(credentials, proof, count, credential_index);
    }
    opened
}

/// Shared signature/metadata opening. The envelope consumes this object once.
pub fn authenticate<let N:u32, let L:u32>(credentials:[[TermOpening<L>;4];N],
    proof:CredentialProof<N>,count:u32,quad_bound:u32,credential_index:u32) -> crate::suite_metadata::OpenedCredential<N> {
    check_limits::<N>(count,quad_bound);
    open_credential_hidden(credentials,proof.salts,proof.salt,count,proof.config,
        proof.created,proof.method,proof.pk,credential_index,proof.r,proof.s,proof.e,proof.e_k)
}

/// Convert after the envelope has consumed the authenticated metadata/properties.
pub fn from_authenticated<let N:u32, let L:u32>(authenticated:crate::suite_metadata::OpenedCredential<N>,
    credentials:[[TermOpening<L>;4];N],credential_index:u32,request_digest:[u8;32]) -> OpenedCredential<N> {
    let quads=authenticated.quads();
    for n in 0..N {if (n < authenticated.count()) & (quads[n][2].kind()==3) {checked_literal(quads[n][2],credentials[n][2]);}}
    convert_authenticated(authenticated,credential_index,request_digest)
}
fn convert_authenticated<let N:u32>(authenticated:crate::suite_metadata::OpenedCredential<N>,
    credential_index:u32,request_digest:[u8;32]) -> OpenedCredential<N> {
        let quads = authenticated.quads();
        let count=authenticated.count();
        let mut opened=OpenedCredential {rows:[[unbound();3];N],active:[false;N],message:authenticated.message(),graph:unbound()};
        let mut request: [u128;2] = [0;2];
        for i in 0..32 { request[i/16] = request[i/16] * 256 + request_digest[i] as u128; }
        opened.graph = table_cell(crate::term::credential_graph(request, credential_index));
        for n in 0..N {
            for p in 0..3 {
                if n < count { check_dataset_scope(quads[n][p],credential_index); }
                opened.rows[n][p] = table_cell(quads[n][p]);
            }
            opened.active[n] = n < count;
            if n < count {
                assert(quads[n][3].kind() == 0, "credential named graph not yet supported");
                check_input_predicate(quads[n][1].enc());
            }
        }
    opened
}
fn check_dataset_scope(cell:crate::term::TermCell,credential_index:u32) {
    assert(credential_index < 0xffffffff,"credential scope capacity");
    assert(!cell.bound() | (cell.kind()!=2) | (cell.credential()==credential_index+1),
        "dataset credential index");
}

/// Canonical inactive slot, shared by the suite and envelope dispatchers.
pub fn inactive_credential<let N:u32, let L:u32>(credentials:[[TermOpening<L>;4];N],proof:CredentialProof<N>,
    count:u32,quad_bound:u32,credential_index:u32) -> OpenedCredential<N> {
    check_limits::<N>(count,quad_bound);
    check_inactive(credentials,proof,count,credential_index);
    OpenedCredential {rows:[[unbound();3];N],active:[false;N],message:0,graph:unbound()}
}

fn check_limits<let N:u32>(count:u32, quad_bound:u32) {
    assert((quad_bound <= N) & (count <= quad_bound), "per-credential quad count");
}
fn check_inactive<let N:u32, let L:u32>(credentials:[[TermOpening<L>;4];N],
    proof:CredentialProof<N>, count:u32, credential_index:u32) {
        assert(count == 0, "credential padding count");
        let quads = open_quads(credentials, credential_index + 1);
        crate::suite_entry::open_cell(proof.created, 0).assert_padding();
        crate::suite_entry::open_cell(proof.method, 0).assert_padding();
        for n in 0..N { for p in 0..4 { quads[n][p].assert_padding(); } }
        check_proof_padding(proof.salt,proof.salts,proof.config,proof.pk,proof.r,proof.s,proof.e,proof.e_k,
            proof.issuer_index,proof.issuer_siblings);

}

pub fn empty_dataset<let D: u32>() -> Dataset<D> {
    Dataset {rows: [[unbound(); 3]; D], active: [false; D], graphs:[unbound();D]}
}

pub fn append_credential<let D: u32, let N: u32>(mut dataset: Dataset<D>, opened: OpenedCredential<N>, offset: u32) -> Dataset<D> {
    assert((offset as u64 + N as u64) <= D as u64, "dataset assembly capacity");
    for i in 0..N {
        dataset.rows[offset + i] = opened.rows[i];
        dataset.active[offset + i] = opened.active[i];
        dataset.graphs[offset + i] = if opened.active[i] { opened.graph } else { unbound() };
    }
    dataset
}

/// The envelope supplies the issuer cell after checking its trust membership.
pub fn append_verified_issuer<let D:u32, let N:u32>(mut dataset:Dataset<D>, opened:OpenedCredential<N>,
    issuer:crate::term::TermCell, active:bool, offset:u32) -> Dataset<D> {
    assert(offset < D, "issuer assertion capacity");
    if active {
        assert(opened.graph.bound & (opened.graph.kind == 2), "credential graph identity");
        assert(issuer.bound() & (issuer.kind() == 1), "verified issuer is an IRI");
    }
    let row = if active { [opened.graph,issuer_predicate(),table_cell(issuer)] } else { [unbound();3] };
    dataset.rows[offset]=row;
    dataset.active[offset]=active;
    dataset.graphs[offset]=unbound();
    dataset
}


fn check_proof_padding<let N:u32>(salt:Field,salts:[Field;N],config:[u128;2],pk:Point,r:Point,
    s:Field,e:Field,e_k:Field,issuer_index:Field,issuer_siblings:[Field;4]) {
        assert((salt == 0) & (salts == [0; N]) & (config == [0; 2]), "credential proof padding");
        assert((pk.x == 0) & (pk.y == 0) & (r.x == 0) & (r.y == 0)
            & (s == 0) & (e == 0) & (e_k == 0)
            & (issuer_index == 0) & (issuer_siblings == [0; 4]), "credential signature padding");
}

/// Bounded production proof fields; resource metadata is constructed as zero.
pub struct BoundedCredentialProof<let N:u32> {
    pub salt:Field, pub salts:[Field;N], pub config:[u128;2],
    pub created:BoundedTermOpening<32,41,1>,
    pub method:crate::ops_core_profile::ResourceOpening<128>,
    pub pk:Point, pub r:Point, pub s:Field, pub e:Field, pub e_k:Field,
    pub issuer_index:Field, pub issuer_siblings:[Field;4],
}
fn scalar_opening<let L:u32>(raw:BoundedTermOpening<L,41,1>) -> BoundedTermOpening<32,41,1> {
    std::static_assert(L >= 32,"scalar source backing capacity");
    assert(raw.lexical_len <= 32,"credential scalar lexical capacity");
    let mut lexical=[0;32];for i in 0..32 {lexical[i]=raw.lexical[i];}
    BoundedTermOpening {bound:raw.bound,kind:raw.kind,lexical,lexical_len:raw.lexical_len,
        datatype:raw.datatype,datatype_len:raw.datatype_len,language:raw.language,language_len:raw.language_len,key:raw.key}
}
fn literal_profile<let L:u32>(cell:crate::term::TermCell,raw:BoundedTermOpening<L,41,1>) {
    let dt=cell.datatype();
    assert((dt==DT_INTEGER)|(dt==DT_DECIMAL)|(dt==DT_DATE)|(dt==DT_DATETIME)|(dt==DT_BOOLEAN)|(dt==DT_STRING),
        "N0 credential literal datatype");
    assert((dt==DT_STRING)|(raw.lexical_len <= 32),"credential scalar lexical capacity");
}
/// Reopen a literal through the suite's fixed-datatype constructors.
/// The caller binds the returned cell to its authenticated input table.
pub fn checked_bounded_literal<let L:u32>(cell:TermCell,raw:BoundedTermOpening<L,41,1>) -> TermCell {
    let checked=if cell.dt==DT_STRING {
        crate::term::open_key_checked(raw,0,DT_STRING).cell()
    } else {
        let scalar=scalar_opening(raw);
        let mut tail:Field=0;
        for i in 32..L {tail=tail+raw.lexical[i] as Field;}
        assert(tail==0,"read scalar backing padding");
        if cell.dt==DT_INTEGER {crate::term::open_key_checked(scalar,0,DT_INTEGER).cell()}
        else if cell.dt==DT_DECIMAL {crate::term::open_key_checked(scalar,0,DT_DECIMAL).cell()}
        else if cell.dt==DT_DATE {crate::term::open_key_checked(scalar,0,DT_DATE).cell()}
        else if cell.dt==DT_DATETIME {crate::term::open_key_checked(scalar,0,DT_DATETIME).cell()}
        else {assert(cell.dt==DT_BOOLEAN,"N0 credential literal datatype");crate::term::open_key_checked(scalar,0,DT_BOOLEAN).cell()}
    };
    table_cell(checked)
}
/// The complete bounded suite opening is shared with the envelope consumer.
pub fn authenticate_bounded<let N:u32,let R:u32,let L:u32>(
    credentials:[crate::ops_core_profile::QuadOpening<R,L>;N],proof:BoundedCredentialProof<N>,
    count:u32,quad_bound:u32,credential_index:u32) -> crate::suite_metadata::OpenedCredential<N> {
    authenticate_profile_bounded::<N,R,L,128>(credentials,proof,count,quad_bound,credential_index)
}
pub fn authenticate_profile_bounded<let N:u32,let R:u32,let L:u32,let M:u32>(
    credentials:[crate::ops_core_profile::QuadOpening<R,L>;N],proof:BoundedCredentialProof<N>,
    count:u32,quad_bound:u32,credential_index:u32) -> crate::suite_metadata::OpenedCredential<N> {
    check_limits::<N>(count,quad_bound);
    let raw=crate::ops_core_profile::normalize(credentials,count);
    crate::suite::open_bounded_credential_hidden(raw,proof.salts,proof.salt,count,proof.config,
        proof.created,crate::ops_core_profile::method::<M>(proof.method),proof.pk,credential_index,
        proof.r,proof.s,proof.e,proof.e_k)
}
pub fn from_authenticated_bounded<let N:u32,let R:u32,let L:u32,let CHECK:u32>(
    authenticated:crate::suite_metadata::OpenedCredential<N>,
    credentials:[crate::ops_core_profile::QuadOpening<R,L>;N],credential_index:u32,request_digest:[u8;32]) -> OpenedCredential<N> {
    std::static_assert(CHECK <= 1,"value key consumption mode");
    let raw=crate::ops_core_profile::normalize(credentials,authenticated.count());
    let quads=authenticated.quads();
    for n in 0..N {if (n < authenticated.count()) & (quads[n][2].kind()==3) {
        literal_profile(quads[n][2],raw[n][2]);
        if CHECK==1 {let _checked=checked_bounded_literal(table_cell(quads[n][2]),raw[n][2]);}
    }}
    convert_authenticated(authenticated,credential_index,request_digest)
}
fn check_inactive_bounded<let N:u32,let R:u32,let L:u32>(
    credentials:[crate::ops_core_profile::QuadOpening<R,L>;N],proof:BoundedCredentialProof<N>,count:u32,credential_index:u32) {
    assert(count==0,"credential padding count");
    assert(credential_index < 0xffffffff,"credential scope capacity");
    let quads=open_bounded_quads(crate::ops_core_profile::normalize(credentials,0),credential_index+1);
    for n in 0..N {for p in 0..4 {quads[n][p].assert_padding();}}
    crate::suite_entry::open_bounded_cell(proof.created,0).assert_padding();
    crate::suite_entry::open_bounded_cell(crate::ops_core_profile::optional_resource(proof.method),0).assert_padding();
    check_proof_padding(proof.salt,proof.salts,proof.config,proof.pk,proof.r,proof.s,proof.e,proof.e_k,
        proof.issuer_index,proof.issuer_siblings);
}
/// Thin lifecycle dispatch; the suite owns the only credential opener.
pub fn open_bounded_one<let N:u32,let R:u32,let L:u32,let CHECK:u32>(
    credentials:[crate::ops_core_profile::QuadOpening<R,L>;N],proof:BoundedCredentialProof<N>,
    count:u32,active:bool,quad_bound:u32,credential_index:u32,issuer_root:Field,request_digest:[u8;32]) -> OpenedCredential<N> {
    open_profile_one::<N,R,L,CHECK,128>(credentials,proof,count,active,quad_bound,credential_index,issuer_root,request_digest)
}
pub fn open_profile_one<let N:u32,let R:u32,let L:u32,let CHECK:u32,let M:u32>(
    credentials:[crate::ops_core_profile::QuadOpening<R,L>;N],proof:BoundedCredentialProof<N>,
    count:u32,active:bool,quad_bound:u32,credential_index:u32,issuer_root:Field,request_digest:[u8;32]) -> OpenedCredential<N> {
    check_limits::<N>(count,quad_bound);
    let mut opened=OpenedCredential {rows:[[unbound();3];N],active:[false;N],message:0,graph:unbound()};
    if active {
        let authenticated=authenticate_profile_bounded::<N,R,L,M>(credentials,proof,count,quad_bound,credential_index);
        key_set_membership(proof.pk,issuer_root,proof.issuer_index,proof.issuer_siblings);
        opened=from_authenticated_bounded::<N,R,L,CHECK>(authenticated,credentials,credential_index,request_digest);
    } else {check_inactive_bounded(credentials,proof,count,credential_index);}
    opened
}

/// Open once and enforce the selected signature mode before the envelope consumes metadata.
pub fn authenticate_profile_bounded_mode<let N:u32,let R:u32,let L:u32,let M:u32>(
    credentials:[crate::ops_core_profile::QuadOpening<R,L>;N],proof:BoundedCredentialProof<N>,
    count:u32,quad_bound:u32,credential_index:u32,signature_mode:u8,
    revealed_message:Field,revealed_key:Field) -> crate::suite_metadata::OpenedCredential<N> {
    assert(signature_mode<=1,"suite signature mode");
    check_limits::<N>(count,quad_bound);
    let raw=crate::ops_core_profile::normalize(credentials,count);
    let authenticated=crate::suite::open_bounded_credential_revealed(raw,proof.salts,proof.salt,count,proof.config,
        proof.created,crate::ops_core_profile::method::<M>(proof.method),proof.pk,credential_index);
    if signature_mode==1 {
        assert((revealed_message==0)&(revealed_key==0),"revealed input padding");
        compose::issuer::schnorr_verify(proof.pk,proof.r,proof.s,proof.e,proof.e_k,authenticated.message());
    } else {
        assert(authenticated.message()==revealed_message,"revealed credential message");
        assert(authenticated.verification_key().enc()==revealed_key,"revealed credential key");
        assert((proof.r.x==0)&(proof.r.y==0)&(proof.s==0)&(proof.e==0)&(proof.e_k==0),"revealed signature witness padding");
    }
    authenticated
}

/// Shared complete opening; the selected signature mode consumes it once.
pub fn open_profile_mode_one<let N:u32,let R:u32,let L:u32,let CHECK:u32,let M:u32>(
    credentials:[crate::ops_core_profile::QuadOpening<R,L>;N],proof:BoundedCredentialProof<N>,
    count:u32,active:bool,quad_bound:u32,credential_index:u32,issuer_root:Field,request_digest:[u8;32],
    signature_mode:u8,revealed_message:Field,revealed_key:Field) -> OpenedCredential<N> {
    assert(signature_mode<=1,"suite signature mode");
    check_limits::<N>(count,quad_bound);
    let mut opened=OpenedCredential {rows:[[unbound();3];N],active:[false;N],message:0,graph:unbound()};
    if active {
        let authenticated=authenticate_profile_bounded_mode::<N,R,L,M>(credentials,proof,count,quad_bound,credential_index,
            signature_mode,revealed_message,revealed_key);
        key_set_membership(proof.pk,issuer_root,proof.issuer_index,proof.issuer_siblings);
        opened=from_authenticated_bounded::<N,R,L,CHECK>(authenticated,credentials,credential_index,request_digest);
    } else {
        assert((revealed_message==0)&(revealed_key==0),"revealed input padding");
        check_inactive_bounded(credentials,proof,count,credential_index);
    }
    opened
}
fn empty_proof<let N: u32>() -> CredentialProof<N> {
    CredentialProof {salt:0,salts:[0;N],config:[0;2],created:empty_opening::<32>(),method:empty_opening::<128>(),pk:Point{x:0,y:0},r:Point{x:0,y:0},s:0,e:0,e_k:0,issuer_index:0,issuer_siblings:[0;4]}
}
fn empty_opening<let L: u32>() -> TermOpening<L> {
    TermOpening {bound:false,kind:0,lexical:[0;L],lexical_len:0,datatype:[0;128],datatype_len:0,language:[0;64],language_len:0,key:[0;31]}
}
fn inactive(proof:CredentialProof<1>,count:u32,bound:u32) -> OpenedCredential<1> {
    check_limits::<1>(count,bound);
    check_inactive([[empty_opening::<1>();4]],proof,count,0);
    OpenedCredential {rows:[[unbound();3]],active:[false],message:0,graph:unbound()}
}
#[test]
fn credential_flags_are_exact() { assert(credential_flags::<4>(2,4,1) == [true,true,false,false]); }
#[test(should_fail_with = "hidden suite signature mode")]
fn suite_mode_is_required() { let _flags=credential_flags::<1>(0,1,0); }
#[test(should_fail_with = "credential count")]
fn credential_count_cannot_exceed_the_request() { let _flags=credential_flags::<4>(3,2,1); }
#[test(should_fail_with = "credential count")]
fn credential_bound_cannot_exceed_the_shape() { let _flags=credential_flags::<1>(0,2,1); }
#[test(should_fail_with = "credential count")]
fn credential_bound_must_be_positive() { let _flags=credential_flags::<1>(0,0,1); }
#[test]
fn inactive_credential_has_no_rows_or_message() {
    let opened=open_one([[empty_opening::<1>();4]],empty_proof(),0,false,0,0,0,[0;32]);
    assert(crate::table::equal_row(opened.rows[0], [unbound();3]) & (opened.active == [false]) & (opened.message == 0));
}
#[test(should_fail_with = "per-credential quad count")]
fn quad_bound_cannot_exceed_the_credential_bucket() { let _opened=inactive(empty_proof(),0,2); }
#[test(should_fail_with = "per-credential quad count")]
fn quad_count_cannot_exceed_its_public_bound() { let _opened=inactive(empty_proof(),1,0); }
#[test(should_fail_with = "credential padding count")]
fn inactive_credential_cannot_claim_a_quad() { let _opened=inactive(empty_proof(),1,1); }
#[test(should_fail_with = "credential proof padding")]
fn inactive_credential_salt_is_zero() { let mut proof=empty_proof();proof.salt=1;
    let _opened=open_one([[empty_opening::<1>();4]],proof,0,false,0,0,0,[0;32]); }
#[test(should_fail_with = "credential proof padding")]
fn inactive_credential_leaf_salts_are_zero() { let mut proof=empty_proof();proof.salts[0]=1;let _opened=inactive(proof,0,0); }
#[test(should_fail_with = "credential proof padding")]
fn inactive_credential_configuration_is_zero() { let mut proof=empty_proof();proof.config[1]=1;let _opened=inactive(proof,0,0); }
#[test(should_fail_with = "credential signature padding")]
fn inactive_credential_key_is_zero() { let mut proof=empty_proof();proof.pk.x=1;let _opened=inactive(proof,0,0); }
#[test(should_fail_with = "credential signature padding")]
fn inactive_credential_signature_point_is_zero() { let mut proof=empty_proof();proof.r.y=1;let _opened=inactive(proof,0,0); }
#[test(should_fail_with = "credential signature padding")]
fn inactive_credential_signature_scalar_is_zero() { let mut proof=empty_proof();proof.s=1;let _opened=inactive(proof,0,0); }
#[test(should_fail_with = "credential signature padding")]
fn inactive_credential_challenge_is_zero() { let mut proof=empty_proof();proof.e=1;let _opened=inactive(proof,0,0); }
#[test(should_fail_with = "credential signature padding")]
fn inactive_credential_reduction_is_zero() { let mut proof=empty_proof();proof.e_k=1;let _opened=inactive(proof,0,0); }
#[test(should_fail_with = "credential signature padding")]
fn inactive_credential_membership_index_is_zero() { let mut proof=empty_proof();proof.issuer_index=1;let _opened=inactive(proof,0,0); }
#[test(should_fail_with = "credential signature padding")]
fn inactive_credential_membership_path_is_zero() { let mut proof=empty_proof();proof.issuer_siblings[0]=1;let _opened=inactive(proof,0,0); }
#[test(should_fail_with = "dataset assembly capacity")]
fn dataset_assembly_cannot_hide_a_credential() {
    let _dataset=append_credential(empty_dataset::<1>(),inactive(empty_proof(),0,0),1);
}
#[test]
fn dataset_assembly_copies_every_row_and_flag() {
    let cell=TermCell {bound:true,kind:1,enc:7,key_class:0,key:0,dt:0,lang:0,scope:0};
    let opened=OpenedCredential {rows:[[cell,cell,cell]],active:[true],message:0,graph:unbound()};
    let dataset=append_credential(empty_dataset::<2>(),opened,1);
    assert(crate::table::equal_row(dataset.rows[1], [cell,cell,cell]));assert(dataset.active == [false,true]);
}

fn raw_literal<let T: u32>(lexical:u8, datatype:[u8;T]) -> TermOpening<1> {
    let mut bytes=[0;128];for i in 0..T {bytes[i]=datatype[i];}
    TermOpening {bound:true,kind:3,lexical:[lexical],lexical_len:1,datatype:bytes,datatype_len:T,language:[0;64],language_len:0,key:[0;31]}
}
fn check_raw_literal(raw:TermOpening<1>) {checked_literal(crate::suite_entry::open_cell(raw,1),raw);}
#[test(should_fail_with = "key disagrees with lexical form")]
fn integer_key_is_recomputed() {check_raw_literal(raw_literal(49,"http://www.w3.org/2001/XMLSchema#integer".as_bytes()));}
#[test(should_fail_with = "key disagrees with lexical form")]
fn decimal_key_is_recomputed() {check_raw_literal(raw_literal(49,"http://www.w3.org/2001/XMLSchema#decimal".as_bytes()));}
#[test(should_fail_with = "key disagrees with lexical form")]
fn date_key_is_recomputed() {
    let mut raw=raw_literal(49,"http://www.w3.org/2001/XMLSchema#date".as_bytes());
    raw.key[0]=8;
    check_raw_literal(raw);
}
#[test(should_fail_with = "key disagrees with lexical form")]
fn datetime_key_is_recomputed() {
    let mut raw=raw_literal(49,"http://www.w3.org/2001/XMLSchema#dateTime".as_bytes());
    raw.key[0]=3;
    check_raw_literal(raw);
}
#[test(should_fail_with = "key disagrees with lexical form")]
fn boolean_key_is_recomputed() {check_raw_literal(raw_literal(49,"http://www.w3.org/2001/XMLSchema#boolean".as_bytes()));}
#[test(should_fail_with = "key disagrees with lexical form")]
fn string_key_is_recomputed() {
    let mut raw=raw_literal(97,"http://www.w3.org/2001/XMLSchema#string".as_bytes());
    raw.key[0]=6;
    check_raw_literal(raw);
}

#[test(should_fail_with = "credential signature padding")]
fn inactive_credential_key_y_is_zero() { let mut proof=empty_proof();proof.pk.y=1;let _opened=inactive(proof,0,0); }
#[test(should_fail_with = "credential signature padding")]
fn inactive_credential_signature_x_is_zero() { let mut proof=empty_proof();proof.r.x=1;let _opened=inactive(proof,0,0); }

#[test(should_fail_with = "non-zero padding")]
fn inactive_creation_opening_is_zero() {
    let mut proof=empty_proof();proof.created.lexical[0]=1;
    let _opened=inactive(proof,0,0);
}
#[test(should_fail_with = "non-zero padding")]
fn inactive_method_opening_is_zero() {
    let mut proof=empty_proof();proof.method.kind=1;
    let _opened=inactive(proof,0,0);
}

#[test(should_fail_with = "credential verifiedIssuer assertion")]
fn input_cannot_supply_a_verified_issuer_assertion() { check_input_predicate(VERIFIED_ISSUER_ENCODING); }
#[test]
fn named_view_keeps_a_graph_per_active_row() {
    let graph=TermCell {bound:true,kind:2,enc:7,key_class:0,key:0,dt:0,lang:0,scope:1};
    let opened=OpenedCredential {rows:[[unbound();3];2],active:[true,false],message:0,graph};
    let dataset=append_credential(empty_dataset::<2>(),opened,0);
    assert(crate::table::equal_cell(dataset.graphs[0],graph));
    assert(crate::table::equal_cell(dataset.graphs[1],unbound()));
}
#[test]
fn verified_issuer_assertion_is_in_the_default_view() {
    let graph=TermCell {bound:true,kind:2,enc:7,key_class:0,key:0,dt:0,lang:0,scope:1};
    let opened=OpenedCredential {rows:[[unbound();3]],active:[false],message:0,graph};
    let issuer=crate::term::iri_vc_type();
    let dataset=append_verified_issuer(empty_dataset::<1>(),opened,issuer,true,0);
    assert(dataset.active[0]);assert(!dataset.graphs[0].bound);
    assert(crate::table::equal_cell(dataset.rows[0][0],graph));
    assert(dataset.rows[0][1].enc == VERIFIED_ISSUER_ENCODING);
    assert(dataset.rows[0][2].enc == issuer.enc());
}
#[test(should_fail_with = "issuer assertion capacity")]
fn issuer_assertion_cannot_escape_the_dataset() {
    let opened=inactive(empty_proof(),0,0);
    let _=append_verified_issuer(empty_dataset::<1>(),opened,crate::term::padding(),false,1);
}
#[test(should_fail_with = "credential graph identity")]
fn issuer_assertion_requires_its_credential_graph() {
    let opened=inactive(empty_proof(),0,0);
    let _=append_verified_issuer(empty_dataset::<1>(),opened,crate::term::iri_vc_type(),true,0);
}
#[test(should_fail_with = "verified issuer is an IRI")]
fn issuer_assertion_requires_an_issuer_iri() {
    let mut opened=inactive(empty_proof(),0,0);
    opened.graph=TermCell {bound:true,kind:2,enc:7,key_class:0,key:0,dt:0,lang:0,scope:1};
    let _=append_verified_issuer(empty_dataset::<1>(),opened,crate::term::padding(),true,0);
}

fn bounded_raw<let L:u32>(raw:TermOpening<L>) -> BoundedTermOpening<L,41,1> {
    let mut datatype=[0;41];for i in 0..41 {datatype[i]=raw.datatype[i];}
    BoundedTermOpening {bound:raw.bound,kind:raw.kind,lexical:raw.lexical,lexical_len:raw.lexical_len,
        datatype,datatype_len:raw.datatype_len,language:[0;1],language_len:0,key:raw.key}
}
fn bounded_empty_proof<let N:u32>() -> BoundedCredentialProof<N> {
    BoundedCredentialProof {salt:0,salts:[0;N],config:[0;2],created:crate::ops_core_profile::empty(),
        method:crate::ops_core_profile::ResourceOpening {kind:0,lexical:[0;128],lexical_len:0},
        pk:Point{x:0,y:0},r:Point{x:0,y:0},s:0,e:0,e_k:0,issuer_index:0,issuer_siblings:[0;4]}
}
fn bounded_empty_quad() -> crate::ops_core_profile::QuadOpening<4,32> {
    crate::ops_core_profile::QuadOpening {
        subject:crate::ops_core_profile::ResourceOpening {kind:0,lexical:[0;4],lexical_len:0},
        predicate:crate::ops_core_profile::ResourceOpening {kind:0,lexical:[0;4],lexical_len:0},
        object:crate::ops_core_profile::ObjectOpening {kind:0,lexical:[0;32],lexical_len:0,
            datatype:[0;41],datatype_len:0,language:[0;1],language_len:0,key:[0;31]}}
}
#[test]
fn bounded_inactive_slot_is_canonical() {
    let opened=open_bounded_one::<1,4,32,1>([bounded_empty_quad()],bounded_empty_proof(),0,false,0,0,0,[0;32]);
    assert(crate::table::equal_row(opened.rows[0],[unbound();3]));
    assert(!opened.active[0] & (opened.message==0) & !opened.graph.bound);
}
#[test(should_fail_with="credential padding count")]
fn bounded_inactive_count_is_zero() {
    let _opened=open_bounded_one::<1,4,32,1>([bounded_empty_quad()],bounded_empty_proof(),1,false,1,0,0,[0;32]);
}
#[test(should_fail_with="credential signature padding")]
fn bounded_inactive_signature_is_zero() {
    let mut proof=bounded_empty_proof();proof.e=1;
    let _opened=open_bounded_one::<1,4,32,1>([bounded_empty_quad()],proof,0,false,0,0,0,[0;32]);
}
#[test(should_fail_with="non-zero padding")]
fn bounded_inactive_resource_bytes_are_zero() {
    let mut raw=bounded_empty_quad();raw.subject.lexical[0]=1;
    let _opened=open_bounded_one::<1,4,32,1>([raw],bounded_empty_proof(),0,false,0,0,0,[0;32]);
}
fn checked_bounded_test<let T:u32>(byte:u8,datatype:[u8;T],key_class:u8) {
    let mut raw=empty_opening::<32>();raw.bound=true;raw.kind=3;
    raw.lexical[0]=byte;raw.lexical_len=1;raw.datatype_len=T;raw.key[0]=key_class;
    for i in 0..T {raw.datatype[i]=datatype[i];}
    let bounded=bounded_raw(raw);let cell=crate::suite_entry::open_bounded_cell(bounded,0);
    literal_profile(cell,bounded);let _checked=checked_bounded_literal(table_cell(cell),bounded);
}
#[test]
fn bounded_invalid_integer_keeps_its_exact_none_key() {
    checked_bounded_test(113,"http://www.w3.org/2001/XMLSchema#integer".as_bytes(),0);
}
#[test(should_fail_with="key disagrees with lexical form")]
fn bounded_integer_key_is_recomputed() {checked_bounded_test(49,"http://www.w3.org/2001/XMLSchema#integer".as_bytes(),0);}
#[test(should_fail_with="key disagrees with lexical form")]
fn bounded_decimal_key_is_recomputed() {checked_bounded_test(49,"http://www.w3.org/2001/XMLSchema#decimal".as_bytes(),0);}
#[test(should_fail_with="key disagrees with lexical form")]
fn bounded_boolean_key_is_recomputed() {checked_bounded_test(49,"http://www.w3.org/2001/XMLSchema#boolean".as_bytes(),0);}
#[test(should_fail_with="key disagrees with lexical form")]
fn bounded_string_key_is_recomputed() {checked_bounded_test(97,"http://www.w3.org/2001/XMLSchema#string".as_bytes(),6);}
#[test(should_fail_with="N0 credential literal datatype")]
fn bounded_profile_refuses_unadmitted_datatypes() {
    let raw=bounded_raw(raw_literal(49,"http://www.w3.org/2001/XMLSchema#double".as_bytes()));
    literal_profile(crate::suite_entry::open_bounded_cell(raw,0),raw);
}
#[test(should_fail_with="credential scalar lexical capacity")]
fn bounded_scalar_range_is_checked_before_narrowing() {
    let mut raw=crate::ops_core_profile::empty::<33>();raw.bound=true;raw.kind=3;raw.lexical_len=33;
    let dt="http://www.w3.org/2001/XMLSchema#integer".as_bytes();
    raw.datatype_len=dt.len();for i in 0..dt.len() {raw.datatype[i]=dt[i];}
    literal_profile(crate::suite_entry::open_bounded_cell(raw,0),raw);
}

#[test(should_fail_with="key disagrees with lexical form")]
fn bounded_date_key_is_recomputed() {checked_bounded_test(49,"http://www.w3.org/2001/XMLSchema#date".as_bytes(),8);}
#[test(should_fail_with="key disagrees with lexical form")]
fn bounded_datetime_key_is_recomputed() {checked_bounded_test(49,"http://www.w3.org/2001/XMLSchema#dateTime".as_bytes(),3);}

fn bounded_resource_fixture<let T:u32>(text:[u8;T]) -> crate::ops_core_profile::ResourceOpening<64> {
    std::static_assert(T<=64,"resource fixture capacity");
    let mut lexical=[0;64];for i in 0..T {lexical[i]=text[i];}
    crate::ops_core_profile::ResourceOpening {kind:1,lexical,lexical_len:T}
}
fn bounded_converted_fixture<let T:u32>(predicate:[u8;T]) -> OpenedCredential<1> {
    let mut object=crate::ops_core_profile::ObjectOpening {kind:1,lexical:[0;64],lexical_len:5,
        datatype:[0;41],datatype_len:0,language:[0;1],language_len:0,key:[0;31]};
    let bytes="urn:o".as_bytes();for i in 0..5 {object.lexical[i]=bytes[i];}
    let raw=[crate::ops_core_profile::QuadOpening {
        subject:bounded_resource_fixture("urn:s".as_bytes()),predicate:bounded_resource_fixture(predicate),object}];
    let authenticated=crate::suite::open_bounded_credential_revealed(
        crate::ops_core_profile::normalize(raw,1),[crate::suite::leaf_salt(7,0)],7,1,[0;2],
        crate::ops_core_profile::empty::<32>(),crate::ops_core_profile::empty::<128>(),
        Point {x:0x1bb873421a895ca8c8d9fc0b44dd35be1d65592564942523f49590264f03d5a9,
            y:0x0d9fd8a464e1fc3e79175f0729533a1b865b70848c2f90d9c037d4d6949c9b55},0);
    from_authenticated_bounded::<1,64,64,0>(authenticated,raw,0,[7;32])
}
#[test]
fn bounded_suite_conversion_retains_rows_message_and_request_graph() {
    let opened=bounded_converted_fixture("urn:p".as_bytes());
    assert(opened.active[0] & (opened.message != 0));
    assert(opened.rows[0][0].bound & opened.rows[0][1].bound & opened.rows[0][2].bound);
    assert(opened.graph.bound & (opened.graph.kind==2) & (opened.graph.scope==1));
}
#[test(should_fail_with="credential verifiedIssuer assertion")]
fn bounded_suite_conversion_refuses_the_reserved_input_predicate() {
    let _opened=bounded_converted_fixture("https://w3id.org/sparq/vcq#verifiedIssuer".as_bytes());
}

fn index_fixture() -> ([crate::ops_core_profile::QuadOpening<8,32>;1],crate::suite_metadata::OpenedCredential<1>) {
    let raw=[QuadOpening {
        subject:ResourceOpening {kind:2,lexical:[98,0,0,0,0,0,0,0],lexical_len:1},
        predicate:ResourceOpening {kind:1,lexical:[117,114,110,58,112,0,0,0],lexical_len:5},
        object:ObjectOpening {kind:1,lexical:[117,114,110,58,111,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],lexical_len:5,
            datatype:[0;41],datatype_len:0,language:[0;1],language_len:0,key:[0;31]},
    }];
    let opened=crate::suite::open_bounded_credential_revealed(crate::ops_core_profile::normalize(raw,1),
        [crate::suite::leaf_salt(7,0)],7,1,[0;2],crate::ops_core_profile::empty::<32>(),
        crate::ops_core_profile::empty::<32>(),Point{x:compose::issuer::BJJ_GX,y:compose::issuer::BJJ_GY},0);
    (raw,opened)
}
#[test]
fn dataset_conversion_reuses_the_opening_index_and_request_scope() {
    let (raw,opened)=index_fixture();
    let converted=from_authenticated_bounded::<1,8,32,0>(opened,raw,0,
        [0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31]);
    assert(converted.rows[0][0].scope==1);assert(converted.graph.scope==1);
    assert(crate::table::equal_cell(converted.graph,table_cell(crate::term::credential_graph([
        0x000102030405060708090a0b0c0d0e0f,0x101112131415161718191a1b1c1d1e1f],0))));
}
#[test(should_fail_with="dataset credential index")]
fn dataset_conversion_refuses_a_different_credential_index() {
    let (raw,opened)=index_fixture();
    let _converted=from_authenticated_bounded::<1,8,32,0>(opened,raw,1,[7;32]);
}

#[test]
fn credential_modes_have_exact_active_slots() {
    assert(credential_mode_flags::<4>(2,3,0,2,[7,8,0,0],[9,10,0,0])==[true,true,false,false]);
    assert(credential_mode_flags::<4>(2,3,1,0,[0;4],[0;4])==[true,true,false,false]);
    assert(credential_mode_flags::<1>(0,1,0,0,[0],[0])==[false]);
}
#[test(should_fail_with="suite signature mode")]
fn credential_modes_refuse_unknown_modes() {let _flags=credential_mode_flags::<1>(0,1,2,0,[0],[0]);}
#[test(should_fail_with="credential count")]
fn credential_modes_refuse_count_over_request() {let _flags=credential_mode_flags::<4>(3,2,0,3,[1,2,3,0],[1,2,3,0]);}
#[test(should_fail_with="credential count")]
fn credential_modes_refuse_bound_over_shape() {let _flags=credential_mode_flags::<1>(0,2,0,0,[0],[0]);}
#[test(should_fail_with="credential count")]
fn credential_modes_refuse_zero_bound() {let _flags=credential_mode_flags::<1>(0,0,0,0,[0],[0]);}
#[test(should_fail_with="revealed credential count")]
fn credential_modes_refuse_missing_evidence() {let _flags=credential_mode_flags::<1>(1,1,0,0,[0],[0]);}
#[test(should_fail_with="revealed credential count")]
fn credential_modes_refuse_extra_evidence() {let _flags=credential_mode_flags::<1>(0,1,0,1,[0],[0]);}
#[test(should_fail_with="revealed credential count")]
fn credential_modes_refuse_hidden_evidence() {let _flags=credential_mode_flags::<1>(1,1,1,1,[0],[0]);}
#[test(should_fail_with="revealed input padding")]
fn credential_modes_refuse_hidden_message() {let _flags=credential_mode_flags::<1>(1,1,1,0,[7],[0]);}
#[test(should_fail_with="revealed input padding")]
fn credential_modes_refuse_hidden_key() {let _flags=credential_mode_flags::<1>(1,1,1,0,[0],[7]);}
#[test(should_fail_with="revealed input padding")]
fn credential_modes_refuse_inactive_message() {let _flags=credential_mode_flags::<2>(1,2,0,1,[7,8],[9,0]);}
#[test(should_fail_with="revealed input padding")]
fn credential_modes_refuse_inactive_key() {let _flags=credential_mode_flags::<2>(1,2,0,1,[7,0],[9,8]);}
