//! Authenticated RDF term cells. Construct cells through `open_term`; never use
//! TermCell itself as an entry-point witness type. Lexical hashing dominates the
//! constructor, and is shared when the same opening is used by key recomputation.
use std::collections::bounded_vec::BoundedVec;
use poseidon::poseidon2::Poseidon2;
use crate::suite_constants::{TERM_DOMAIN, DATATYPE_DOMAIN, LANGUAGE_DOMAIN, PACK_BYTES, DT_LANGSTRING, KIND_DEFAULT_GRAPH, KIND_BLANK_NODE, KIND_LITERAL};

pub struct TermCell {
    bound: bool,
    kind: u8,
    enc: Field,
    key_class: u8,
    key: Field,
    dt: Field,
    lang: Field,
    credential: u32,
}

impl TermCell {
    pub fn bound(self) -> bool { self.bound }
    pub fn kind(self) -> u8 { self.kind }
    pub fn enc(self) -> Field { self.enc }
    pub fn key_class(self) -> u8 { self.key_class }
    pub fn key(self) -> Field { self.key }
    pub fn datatype(self) -> Field { self.dt }
    pub fn language(self) -> Field { self.lang }
    pub fn credential(self) -> u32 { self.credential }
    pub fn assert_padding(self) {
        assert(!self.bound & (self.kind == 0) & (self.enc == 0) & (self.key_class == 0)
            & (self.key == 0) & (self.dt == 0) & (self.lang == 0) & (self.credential == 0), "non-zero padding");
    }
}

/// Canonical table padding. A bound default-graph cell is different.
pub fn padding() -> TermCell {
    TermCell { bound: false, kind: 0, enc: 0, key_class: 0, key: 0, dt: 0, lang: 0, credential: 0 }
}

/// The byte hash has the same value in every capacity bucket. Zero padding is
/// checked explicitly because BoundedVec's private backing storage is still ABI data.
pub fn hash_bytes<let L: u32>(domain: Field, bytes: BoundedVec<u8, L>) -> Field {
    assert(bytes.len() <= L, "byte capacity");
    let mut packed: [Field; (L + 30) / 31 + 2] = [0; (L + 30) / 31 + 2];
    packed[0] = domain;
    packed[1] = bytes.len() as Field;
    for i in 0..L {
        let byte = bytes.get_unchecked(i);
        if i < bytes.len() { packed[2 + i / PACK_BYTES] = packed[2 + i / PACK_BYTES] * 256 + byte as Field; }
        else { assert(byte == 0, "non-zero byte padding"); }
    }
    Poseidon2::hash(packed, 2 + (bytes.len() + 30) / 31)
}

fn append<let N: u32, let L: u32>(out: &mut BoundedVec<u8, N>, bytes: BoundedVec<u8, L>) {
    assert(bytes.len() <= L, "byte capacity");
    let length = (bytes.len() as Field).to_be_bytes::<4>();
    for i in 0..4 { out.push(length[i]); }
    for i in 0..L {
        if i < bytes.len() { out.push(bytes.get_unchecked(i)); }
        else { assert(bytes.get_unchecked(i) == 0, "non-zero byte padding"); }
    }
}

/// Build every component from an opening. Non-literals carry no key or metadata.
/// `kind` uses the suite byte tag: default graph, IRI, blank node, literal.
pub fn open_term<let L: u32, let D: u32, let G: u32>(
    kind: u8, lexical: BoundedVec<u8, L>, datatype: BoundedVec<u8, D>,
    language: BoundedVec<u8, G>, key: [u8; 31], credential: u32,
) -> TermCell {
    assert(kind <= KIND_LITERAL, "term kind");
    let mut encoded: BoundedVec<u8, L + D + G + 13> = BoundedVec::new();
    encoded.push(kind);
    if kind == KIND_DEFAULT_GRAPH {
        assert(lexical.len() == 0, "default graph lexical");
    } else { append(&mut encoded, lexical); }
    let mut key_field: Field = 0;
    for i in 0..31 { key_field = key_field * 256 + key[i] as Field; }
    let mut dt: Field = 0;
    let mut lang: Field = 0;
    if kind == KIND_LITERAL {
        assert(datatype.len() > 0, "literal datatype");
        append(&mut encoded, datatype);
        append(&mut encoded, language);
        dt = hash_bytes(DATATYPE_DOMAIN, datatype);
        assert((language.len() > 0) == (dt == DT_LANGSTRING), "language datatype");
        lang = hash_bytes(LANGUAGE_DOMAIN, language);
    } else {
        assert((datatype.len() == 0) & (language.len() == 0) & (key_field == 0), "non-literal metadata");
        // The helpers check backing storage even for an empty opening.
        let _dt = hash_bytes(DATATYPE_DOMAIN, datatype);
        assert((language.len() > 0) == (dt == DT_LANGSTRING), "language datatype");
        let _lang = hash_bytes(LANGUAGE_DOMAIN, language);
    }
    // Hashing also canonicalizes storage for default-graph lexical input.
    let _lexical = hash_bytes(TERM_DOMAIN, lexical);
    let enc = Poseidon2::hash([kind as Field, hash_bytes(TERM_DOMAIN, encoded), key_field], 3);
    TermCell { bound: true, kind, enc, key_class: key[0], key: key_field, dt, lang,
        credential: if kind == KIND_BLANK_NODE { credential } else { 0 } }
}

/// Bind optional lexical bytes and metadata to a cell's term identity.
pub fn open_lexical<let L: u32, let D: u32, let G: u32>(
    cell: TermCell, lexical: BoundedVec<u8, L>, datatype: BoundedVec<u8, D>, language: BoundedVec<u8, G>,
) {
    assert(cell.bound, "unbound lexical opening");
    let key = cell.key.to_be_bytes::<32>();
    assert(key[0] == 0, "key width");
    let mut bytes = [0; 31];
    for i in 0..31 { bytes[i] = key[i + 1]; }
    let rebuilt = open_term(cell.kind, lexical, datatype, language, bytes, cell.credential);
    assert((rebuilt.enc == cell.enc) & (rebuilt.dt == cell.dt) & (rebuilt.lang == cell.lang)
        & (rebuilt.key_class == cell.key_class), "lexical opening differs");
}

/// Its private payload can only be minted by `recompute_key`.
pub struct KeyCheckedCell { cell: TermCell }
impl KeyCheckedCell { pub fn cell(self) -> TermCell { self.cell } }

/// Opening and recomputation are inseparable: a caller cannot mark an arbitrary
/// cell checked by presenting a key alone.
pub fn recompute_key<let L: u32, let D: u32, let G: u32>(
    cell: TermCell, lexical: BoundedVec<u8, L>, datatype: BoundedVec<u8, D>, language: BoundedVec<u8, G>,
) -> KeyCheckedCell {
    recompute_key_for(cell, lexical, datatype, language, cell.dt)
}

/// Datatype-specialized form. The expected identifier is still constrained to
/// the opened datatype; a compiled query can fold unrelated lexical parsers.
pub fn recompute_key_for<let L: u32, let D: u32, let G: u32>(
    cell: TermCell, lexical: BoundedVec<u8, L>, datatype: BoundedVec<u8, D>, language: BoundedVec<u8, G>, expected_datatype: Field,
) -> KeyCheckedCell {
    open_lexical(cell, lexical, datatype, language);
    assert(cell.kind == KIND_LITERAL, "key requires literal");
    assert(cell.dt == expected_datatype, "datatype specialization differs");
    let (class, key) = crate::term_key::recompute(lexical, expected_datatype);
    assert(class == cell.key_class, "class disagrees with datatype");
    assert(key == cell.key, "key disagrees with lexical form");
    KeyCheckedCell { cell }
}

/// Canonical unsigned field order, including values above the signed bit range.
pub fn field_lt(a: Field, b: Field) -> bool {
    let a = a.to_be_bytes::<32>();
    let b = b.to_be_bytes::<32>();
    let mut equal = true;
    let mut lt = false;
    for i in 0..32 { lt = lt | (equal & (a[i] < b[i])); equal = equal & (a[i] == b[i]); }
    lt
}
