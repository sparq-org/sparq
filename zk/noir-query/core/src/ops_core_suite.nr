//! Suite authentication and conversion into the operator table vocabulary.
use std::collections::bounded_vec::BoundedVec;
use compose::issuer::{Point, key_set_membership};
use crate::suite_entry::{TermOpening, open_quads};
use crate::suite::{open_credential_hidden, open_credential_revealed};
use crate::suite_constants::{DT_INTEGER, DT_DECIMAL, DT_DATE, DT_DATETIME, DT_BOOLEAN, DT_STRING};
use crate::table::{TermCell, unbound};
use crate::ops_core::Dataset;

pub struct CredentialProof<let N: u32> {
    pub salt: Field,
    pub salts: [Field; N],
    pub config: [u128; 2],
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
) -> OpenedCredential<N> {
    assert((quad_bound <= N) & (count <= quad_bound), "per-credential quad count");
    let quads = open_quads(credentials, credential_index + 1);
    let mut opened = OpenedCredential {rows: [[unbound(); 3]; N], active: [false; N], message: 0};
    if active {
        open_credential_hidden(quads, proof.salts, proof.salt, count, proof.config,
            proof.pk, proof.r, proof.s, proof.e, proof.e_k);
        key_set_membership(proof.pk, issuer_root, proof.issuer_index, proof.issuer_siblings);
        opened.message = open_credential_revealed(quads, proof.salts, proof.salt, count, proof.config);
        for n in 0..N {
            for p in 0..3 {
                if quads[n][p].bound() {
                    if quads[n][p].kind() == 3 { checked_literal(quads[n][p], credentials[n][p]); }
                    opened.rows[n][p] = table_cell(quads[n][p]);
                }
            }
            opened.active[n] = n < count;
            if n < count { assert(quads[n][3].kind() == 0, "credential named graph not yet supported"); }
        }
    } else {
        assert(count == 0, "credential padding count");
        for n in 0..N { for p in 0..4 { quads[n][p].assert_padding(); } }
        assert((proof.salt == 0) & (proof.salts == [0; N]) & (proof.config == [0; 2]), "credential proof padding");
        assert((proof.pk.x == 0) & (proof.pk.y == 0) & (proof.r.x == 0) & (proof.r.y == 0)
            & (proof.s == 0) & (proof.e == 0) & (proof.e_k == 0)
            & (proof.issuer_index == 0) & (proof.issuer_siblings == [0; 4]), "credential signature padding");
    }
    opened
}

pub fn empty_dataset<let D: u32>() -> Dataset<D> {
    Dataset {rows: [[unbound(); 3]; D], active: [false; D]}
}

pub fn append_credential<let D: u32, let N: u32>(mut dataset: Dataset<D>, opened: OpenedCredential<N>, offset: u32) -> Dataset<D> {
    assert((offset as u64 + N as u64) <= D as u64, "dataset assembly capacity");
    for i in 0..N {
        dataset.rows[offset + i] = opened.rows[i];
        dataset.active[offset + i] = opened.active[i];
    }
    dataset
}

fn empty_proof<let N: u32>() -> CredentialProof<N> {
    CredentialProof {salt:0,salts:[0;N],config:[0;2],pk:Point{x:0,y:0},r:Point{x:0,y:0},s:0,e:0,e_k:0,issuer_index:0,issuer_siblings:[0;4]}
}
fn empty_opening() -> TermOpening<1> {
    TermOpening {bound:false,kind:0,lexical:[0],lexical_len:0,datatype:[0;128],datatype_len:0,language:[0;64],language_len:0,key:[0;31]}
}
fn inactive(proof:CredentialProof<1>,count:u32,bound:u32) -> OpenedCredential<1> {
    open_one([[empty_opening();4]],proof,count,false,bound,0,0)
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
    let opened=inactive(empty_proof(),0,0);
    assert(crate::table::equal_row(opened.rows[0], [unbound();3]) & (opened.active == [false]) & (opened.message == 0));
}
#[test(should_fail_with = "per-credential quad count")]
fn quad_bound_cannot_exceed_the_credential_bucket() { let _opened=inactive(empty_proof(),0,2); }
#[test(should_fail_with = "per-credential quad count")]
fn quad_count_cannot_exceed_its_public_bound() { let _opened=inactive(empty_proof(),1,0); }
#[test(should_fail_with = "credential padding count")]
fn inactive_credential_cannot_claim_a_quad() { let _opened=inactive(empty_proof(),1,1); }
#[test(should_fail_with = "credential proof padding")]
fn inactive_credential_salt_is_zero() { let mut proof=empty_proof();proof.salt=1;let _opened=inactive(proof,0,0); }
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
    let opened=OpenedCredential {rows:[[cell,cell,cell]],active:[true],message:0};
    let dataset=append_credential(empty_dataset::<2>(),opened,1);
    assert(crate::table::equal_row(dataset.rows[1], [cell,cell,cell]));assert(dataset.active == [false,true]);
}
