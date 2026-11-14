//! Mixed-bound openings must authenticate exactly the uniform tree.
use crate::suite_entry::BoundedTermOpening;
use crate::suite_metadata::OpenedCredential;
use crate::term::{field_lt, same_term};
use compose::issuer::Point;

fn raw<let L: u32, let N: u32, let T: u32, let A: u32>(kind: u8, lex: [u8; N], dt: [u8; T], lang: [u8; A], key: Field) -> BoundedTermOpening<L, 64, 8> {
    let mut lexical = [0; L]; for i in 0..N { lexical[i] = lex[i]; }
    let mut datatype = [0; 64]; for i in 0..T { datatype[i] = dt[i]; }
    let mut language = [0; 8]; for i in 0..A { language[i] = lang[i]; }
    let bytes = key.to_be_bytes::<32>(); let mut k = [0; 31]; for i in 0..31 { k[i] = bytes[i + 1]; }
    BoundedTermOpening { bound: true, kind, lexical, lexical_len: N, datatype, datatype_len: T, language, language_len: A, key: k }
}
fn pad<let L: u32>() -> [BoundedTermOpening<L, 64, 8>; 4] {
    [BoundedTermOpening { bound: false, kind: 0, lexical: [0; L], lexical_len: 0, datatype: [0; 64], datatype_len: 0,
        language: [0; 8], language_len: 0, key: [0; 31] }; 4]
}
fn balance<let L: u32>() -> [BoundedTermOpening<L, 64, 8>; 4] {
    [raw(2, "c14n0".as_bytes(), [], [], 0), raw(1, "http://ex/balance".as_bytes(), [], [], 0),
     raw(3, "1250.50".as_bytes(), "http://www.w3.org/2001/XMLSchema#decimal".as_bytes(), [],
         0x000180000000000000000000000000000000000000000043ca2c749129fa0000),
     raw(0, [], [], [], 0)]
}
fn name<let L: u32>() -> [BoundedTermOpening<L, 64, 8>; 4] {
    [raw(1, "urn:vc:1".as_bytes(), [], [], 0), raw(1, "http://ex/name".as_bytes(), [], [], 0),
     raw(3, "Alice".as_bytes(), "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString".as_bytes(), "en".as_bytes(), 0),
     raw(0, [], [], [], 0)]
}
fn hash<let L: u32>(row: [BoundedTermOpening<L, 64, 8>; 4]) -> Field {
    let c = crate::suite_entry::open_bounded_quads([row], 1)[0];
    crate::suite::quad_hash([c[0].enc(), c[1].enc(), c[2].enc(), c[3].enc()])
}
// Tree order of the two facts, independent of their hash values.
fn ordered<let L: u32>() -> [[BoundedTermOpening<L, 64, 8>; 4]; 2] {
    if field_lt(hash(balance::<L>()), hash(name::<L>())) { [balance(), name()] } else { [name(), balance()] }
}
fn salts() -> [Field; 4] { [crate::suite::leaf_salt(7, 0), crate::suite::leaf_salt(7, 1), 0, 0] }
fn created<let L: u32>() -> BoundedTermOpening<L, 64, 8> {
    raw(3, "2026-01-01T00:00:00Z".as_bytes(), "http://www.w3.org/2001/XMLSchema#dateTime".as_bytes(), [],
        0x00038000019b76daa80000000000000000000000000000000000000000000000)
}
fn method<let L: u32>() -> BoundedTermOpening<L, 64, 8> { raw(1, "urn:key:19".as_bytes(), [], [], 0) }
fn pk() -> Point { Point { x: 0x1bb873421a895ca8c8d9fc0b44dd35be1d65592564942523f49590264f03d5a9, y: 0x0d9fd8a464e1fc3e79175f0729533a1b865b70848c2f90d9c037d4d6949c9b55 } }
fn uniform() -> OpenedCredential<4> {
    let o = ordered::<64>();
    crate::suite::open_bounded_credential_revealed([o[0], o[1], pad(), pad()], salts(), 7, 2,
        [0x09090909090909090909090909090909; 2], created::<32>(), method::<32>(), pk(), 0)
}
fn mixed<let S: u32, let K: u32>(short: [[BoundedTermOpening<32, 64, 8>; 4]; S], long: [[BoundedTermOpening<64, 64, 8>; 4]; K], index: [u32; K]) -> OpenedCredential<4> {
    crate::suite::open_bounded_credential_mixed_revealed(short, long, index, salts(), 7, 2,
        [0x09090909090909090909090909090909; 2], created::<32>(), method::<32>(), pk(), 0)
}
fn same(a: OpenedCredential<4>, b: OpenedCredential<4>) {
    assert((a.root() == b.root()) & (a.message() == b.message()) & (a.count() == b.count()), "opening differs");
    for i in 0..4 { for j in 0..4 {
        let (x, y) = (a.quads()[i][j], b.quads()[i][j]);
        assert(same_term(x, y) & (x.enc() == y.enc()) & (x.key() == y.key()), "quad table differs");
    } }
}
#[test]
fn mixed_long_row_after_a_short_row_matches_uniform() {
    let (s, l) = (ordered::<32>(), ordered::<64>());
    same(mixed([s[0], pad(), pad()], [l[1]], [1]), uniform());
}
#[test]
fn mixed_long_row_first_matches_uniform() {
    let (s, l) = (ordered::<32>(), ordered::<64>());
    same(mixed([s[1], pad(), pad()], [l[0]], [0]), uniform());
}
#[test]
fn mixed_absent_long_slot_comes_last() {
    let s = ordered::<32>();
    same(mixed([s[0], s[1], pad()], [pad()], [3]), uniform());
}
#[test]
fn mixed_two_long_slots_one_absent() {
    let (s, l) = (ordered::<32>(), ordered::<64>());
    same(mixed([s[1], pad()], [l[0], pad()], [0, 3]), uniform());
}
#[test(should_fail_with = "long rows out of order")]
fn mixed_long_indices_must_increase() {
    let (s, l) = (ordered::<32>(), ordered::<64>());
    let _o = mixed([s[1], pad()], [pad(), l[0]], [3, 0]);
}
#[test(should_fail_with = "long rows out of order")]
fn mixed_long_indices_must_be_distinct() {
    let (s, l) = (ordered::<32>(), ordered::<64>());
    let _o = mixed([s[1], pad()], [l[0], pad()], [0, 0]);
}
#[test(should_fail_with = "long row index")]
fn mixed_long_index_within_capacity() {
    let s = ordered::<32>();
    let _o = mixed([s[0], s[1], pad()], [pad()], [4]);
}
#[test(should_fail_with = "quads not strictly increasing")]
fn mixed_placement_cannot_reorder_the_tree() {
    let (s, l) = (ordered::<32>(), ordered::<64>());
    let _o = mixed([s[0], pad(), pad()], [l[1]], [0]);
}
#[test(should_fail_with = "wrong count")]
fn mixed_absent_long_slot_cannot_precede_counted_rows() {
    let s = ordered::<32>();
    let _o = mixed([s[0], s[1], pad()], [pad()], [0]);
}
