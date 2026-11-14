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
        assert_raw_padding(raw);
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

// Keep the public, general table constructors unchanged. RDF-role specialization
// belongs only to complete credential openings, which enforce those same roles.
fn bounded<let L: u32>(raw: TermOpening<L>) -> BoundedTermOpening<L, DATATYPE_CAPACITY, LANGUAGE_CAPACITY> {
    BoundedTermOpening { bound: raw.bound, kind: raw.kind,
        lexical: raw.lexical, lexical_len: raw.lexical_len,
        datatype: raw.datatype, datatype_len: raw.datatype_len,
        language: raw.language, language_len: raw.language_len, key: raw.key }
}
fn open_nonliteral_cell<let L: u32, let D: u32, let G: u32, let P: u32>(
    raw: BoundedTermOpening<L, D, G>, credential: u32,
) -> TermCell {
    if raw.bound {
        let kind = if P == 1 { assert(raw.kind == 1, "predicate kind"); 1 } else { raw.kind };
        crate::term::open_nonliteral_term::<L, D, G, P>(kind,
            BoundedVec::from_parts_unchecked(raw.lexical, raw.lexical_len),
            BoundedVec::from_parts_unchecked(raw.datatype, raw.datatype_len),
            BoundedVec::from_parts_unchecked(raw.language, raw.language_len), raw.key, credential)
    } else {
        assert((raw.kind == 0) & (raw.lexical_len == 0) & (raw.datatype_len == 0) & (raw.language_len == 0), "non-zero padding");
        assert_raw_padding(raw);
        padding()
    }
}
pub(crate) fn open_credential_quads<let N: u32, let L: u32>(raw: [[TermOpening<L>; 4]; N], credential: u32) -> [[TermCell; 4]; N] {
    let mut quads = [[padding(); 4]; N];
    for i in 0..N {
        let row = raw[i];
        let quad = [
            open_nonliteral_cell::<L, DATATYPE_CAPACITY, LANGUAGE_CAPACITY, 0>(bounded(row[0]), credential),
            open_nonliteral_cell::<L, DATATYPE_CAPACITY, LANGUAGE_CAPACITY, 1>(bounded(row[1]), credential),
            open_cell(row[2], credential),
            open_nonliteral_cell::<L, DATATYPE_CAPACITY, LANGUAGE_CAPACITY, 3>(bounded(row[3]), credential),
        ];
        quads[i] = quad;
    }
    quads
}
pub(crate) fn open_credential_bounded_quads<let N: u32, let L: u32, let D: u32, let G: u32>(
    raw: [[BoundedTermOpening<L, D, G>; 4]; N], credential: u32,
) -> [[TermCell; 4]; N] {
    let mut quads = [[padding(); 4]; N];
    for i in 0..N {
        let row = raw[i];
        let quad = [
            open_nonliteral_cell::<L, D, G, 0>(row[0], credential),
            open_nonliteral_cell::<L, D, G, 1>(row[1], credential),
            open_bounded_cell(row[2], credential),
            open_nonliteral_cell::<L, D, G, 3>(row[3], credential),
        ];
        quads[i] = quad;
    }
    quads
}

/// Merge K long-bound rows into S short-bound rows in tree order. Long indices
/// strictly increase below N; short rows fill the other indices in their order.
/// That makes the placement a bijection onto the N leaves, so the root is unchanged.
pub(crate) fn merge_mixed_rows<let N: u32, let S: u32, let K: u32>(
    short: [[TermCell; 4]; S], long: [[TermCell; 4]; K], long_index: [u32; K],
) -> [[TermCell; 4]; N] {
    std::static_assert(S + K == N, "mixed row capacity");
    for k in 0..K {
        assert(long_index[k] < N, "long row index");
        if k > 0 { assert(long_index[k - 1] < long_index[k], "long rows out of order"); }
    }
    // Pad both ends so every static candidate index stays in range.
    let mut window = [[padding(); 4]; N + K];
    for s in 0..S { window[K + s] = short[s]; }
    let mut rows = [[padding(); 4]; N];
    for i in 0..N {
        let mut before: u32 = 0;
        for k in 0..K { before += (long_index[k] < i) as u32; }
        let mut is_long = false;
        let mut row = [padding(); 4];
        for k in 0..K { if long_index[k] == i { is_long = true; row = long[k]; } }
        // A short row at tree index i is short row i - before.
        for d in 0..K + 1 { if (!is_long) & (before == d) { row = window[i + K - d]; } }
        rows[i] = row;
    }
    rows
}

fn assert_raw_padding<let L: u32, let D: u32, let G: u32>(raw: BoundedTermOpening<L,D,G>) {
    // Byte ranges make this sum nonnegative and smaller than the field modulus.
    let sum = crate::term_bytes::byte_sum(raw.lexical) + crate::term_bytes::byte_sum(raw.datatype)
        + crate::term_bytes::byte_sum(raw.language) + crate::term_bytes::byte_sum(raw.key);
    assert(sum == 0, "non-zero padding");
}
