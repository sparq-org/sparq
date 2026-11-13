//! Raw ABI openings. Keep this witness type private at a query entry point.
use std::collections::bounded_vec::BoundedVec;
use crate::term::{TermCell, open_term, padding};

pub struct TermOpening<let L: u32> {
    pub bound: bool,
    pub kind: u8,
    pub lexical: [u8; L],
    pub lexical_len: u32,
    pub datatype: [u8; 128],
    pub datatype_len: u32,
    pub language: [u8; 64],
    pub language_len: u32,
    pub key: [u8; 31],
}

pub fn open_cell<let L: u32>(raw: TermOpening<L>, credential: u32) -> TermCell {
    if raw.bound {
        open_term(raw.kind, BoundedVec::from_parts_unchecked(raw.lexical, raw.lexical_len),
            BoundedVec::from_parts_unchecked(raw.datatype, raw.datatype_len),
            BoundedVec::from_parts_unchecked(raw.language, raw.language_len), raw.key, credential)
    } else {
        assert((raw.kind == 0) & (raw.lexical_len == 0) & (raw.datatype_len == 0) & (raw.language_len == 0), "non-zero padding");
        assert((raw.lexical == [0; L]) & (raw.datatype == [0; 128]) & (raw.language == [0; 64]) & (raw.key == [0; 31]), "non-zero padding");
        padding()
    }
}

pub fn open_quads<let N: u32, let L: u32>(raw: [[TermOpening<L>; 4]; N], credential: u32) -> [[TermCell; 4]; N] {
    let mut quads = [[padding(); 4]; N];
    for i in 0..N { for j in 0..4 { quads[i][j] = open_cell(raw[i][j], credential); } }
    quads
}
