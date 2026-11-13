//! Raw ABI openings. Keep this witness type private at a query entry point.
use crate::suite_constants::{DATATYPE_CAPACITY, LANGUAGE_CAPACITY, KEY_BYTES};
use std::collections::bounded_vec::BoundedVec;
use crate::term::{TermCell, open_term, padding};

pub struct TermOpening<let L: u32> {
    pub bound: bool,
    pub kind: u8,
    pub lexical: [u8; L],
    pub lexical_len: u32,
    pub datatype: [u8; DATATYPE_CAPACITY],
    pub datatype_len: u32,
    pub language: [u8; LANGUAGE_CAPACITY],
    pub language_len: u32,
    pub key: [u8; KEY_BYTES],
}

/// Raw opening with independently chosen lexical, datatype and language bounds.
/// Lengths remain witnesses; bytes beyond each length must be canonical zero.
pub struct BoundedTermOpening<let L: u32, let D: u32, let G: u32> {
    pub bound: bool,
    pub kind: u8,
    pub lexical: [u8; L],
    pub lexical_len: u32,
    pub datatype: [u8; D],
    pub datatype_len: u32,
    pub language: [u8; G],
    pub language_len: u32,
    pub key: [u8; KEY_BYTES],
}

pub fn open_cell<let L: u32>(raw: TermOpening<L>, credential: u32) -> TermCell {
    open_bounded_cell(BoundedTermOpening {
        bound: raw.bound, kind: raw.kind, lexical: raw.lexical, lexical_len: raw.lexical_len,
        datatype: raw.datatype, datatype_len: raw.datatype_len,
        language: raw.language, language_len: raw.language_len, key: raw.key,
    }, credential)
}

/// Construct every cell component through the existing typed term relation.
pub fn open_bounded_cell<let L: u32, let D: u32, let G: u32>(
    raw: BoundedTermOpening<L, D, G>, credential: u32,
) -> TermCell {
    if raw.bound {
        open_term(raw.kind, BoundedVec::from_parts_unchecked(raw.lexical, raw.lexical_len),
            BoundedVec::from_parts_unchecked(raw.datatype, raw.datatype_len),
            BoundedVec::from_parts_unchecked(raw.language, raw.language_len), raw.key, credential)
    } else {
        assert((raw.kind == 0) & (raw.lexical_len == 0) & (raw.datatype_len == 0) & (raw.language_len == 0), "non-zero padding");
        assert((raw.lexical == [0; L]) & (raw.datatype == [0; D]) & (raw.language == [0; G]) & (raw.key == [0; KEY_BYTES]), "non-zero padding");
        padding()
    }
}

pub fn open_quads<let N: u32, let L: u32>(raw: [[TermOpening<L>; 4]; N], credential: u32) -> [[TermCell; 4]; N] {
    let mut quads = [[padding(); 4]; N];
    for i in 0..N { for j in 0..4 { quads[i][j] = open_cell(raw[i][j], credential); } }
    quads
}

pub fn open_bounded_quads<let N: u32, let L: u32, let D: u32, let G: u32>(
    raw: [[BoundedTermOpening<L, D, G>; 4]; N], credential: u32,
) -> [[TermCell; 4]; N] {
    let mut quads = [[padding(); 4]; N];
    for i in 0..N { for j in 0..4 { quads[i][j] = open_bounded_cell(raw[i][j], credential); } }
    quads
}
