//! Suite authentication and conversion into the operator table vocabulary.
use std::collections::bounded_vec::BoundedVec;
use compose::issuer::{Point, key_set_membership};
use crate::suite_entry::{TermOpening, open_quads};
use crate::suite::open_credential_hidden;
use crate::suite_constants::{DT_INTEGER, DT_DECIMAL, DT_DATE, DT_DATETIME, DT_BOOLEAN, DT_STRING};
use crate::table::{TermCell, unbound};
use crate::ops_core::Dataset;

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
        let quads = authenticated.quads();
        let count=authenticated.count();
        let mut opened=OpenedCredential {rows:[[unbound();3];N],active:[false;N],message:authenticated.message(),graph:unbound()};
        let mut request: [u128;2] = [0;2];
        for i in 0..32 { request[i/16] = request[i/16] * 256 + request_digest[i] as u128; }
        opened.graph = table_cell(crate::term::credential_graph(request, credential_index));
        for n in 0..N {
            for p in 0..3 {
                if (n < count) & (p == 2) & (quads[n][p].kind() == 3) {
                    checked_literal(quads[n][p], credentials[n][p]);
                }
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
        assert((proof.salt == 0) & (proof.salts == [0; N]) & (proof.config == [0; 2]), "credential proof padding");
        assert((proof.pk.x == 0) & (proof.pk.y == 0) & (proof.r.x == 0) & (proof.r.y == 0)
            & (proof.s == 0) & (proof.e == 0) & (proof.e_k == 0)
            & (proof.issuer_index == 0) & (proof.issuer_siblings == [0; 4]), "credential signature padding");

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
