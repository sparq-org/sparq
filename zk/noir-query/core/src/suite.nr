//! Complete, salted credential openings. No unsalted digest is released.
use poseidon::poseidon2::Poseidon2;
use compose::issuer::{Point, schnorr_verify};
use crate::term::{TermCell, field_lt};
use crate::suite_constants::{QUAD_DOMAIN, SALT_DOMAIN, LEAF_DOMAIN, NODE_DOMAIN, MESSAGE_DOMAIN, SUITE_ID, KIND_IRI, KIND_BLANK_NODE, KIND_LITERAL};

pub fn quad_hash(enc: [Field; 4]) -> Field {
    Poseidon2::hash([QUAD_DOMAIN, enc[0], enc[1], enc[2], enc[3]], 5)
}
pub fn leaf_salt(salt: Field, index: u32) -> Field { Poseidon2::hash([SALT_DOMAIN, salt, index as Field], 3) }
pub fn leaf(q: Field, salt: Field, index: u32) -> Field { Poseidon2::hash([LEAF_DOMAIN, q, leaf_salt(salt, index)], 3) }

/// Minimal tree: a larger compiled capacity never changes an issued root.
/// Padding encodings and supplied salts are canonical zero outside `count`.
pub fn credential_root<let N: u32>(quads: [[TermCell; 4]; N], salts: [Field; N], salt: Field, count: u32) -> Field {
    std::static_assert((N > 0) & ((N & (N - 1)) == 0), "power-of-two credential capacity");
    assert((count > 0) & (count <= N), "wrong count");
    let mut row: [Field; N] = [0; N];
    let mut previous: Field = 0;
    for i in 0..N {
        let mut value: Field = 0;
        if i < count {
            let quad = quads[i];
            for j in 0..4 { assert(quad[j].bound(), "wrong count"); }
            assert((quad[0].kind() == KIND_IRI) | (quad[0].kind() == KIND_BLANK_NODE), "subject kind");
            assert(quad[1].kind() == KIND_IRI, "predicate kind");
            assert(quad[2].kind() > 0, "object kind");
            assert(quad[3].kind() < KIND_LITERAL, "graph kind");
            let q = quad_hash([quad[0].enc(), quad[1].enc(), quad[2].enc(), quad[3].enc()]);
            if i > 0 { assert(field_lt(previous, q), "quads not strictly increasing"); }
            previous = q;
            assert(salts[i] == leaf_salt(salt, i), "leaf salt not derived from tree salt");
            value = leaf(q, salt, i);
        } else {
            for j in 0..4 { quads[i][j].assert_padding(); }
            assert(salts[i] == 0, "non-zero padding");
        }
        row[i] = value;
    }
    let mut result = row[0];
    let mut depth: u32 = 0;
    for bit in 0..32 { if (N >> bit) > 1 { depth = depth + 1; } }
    for level in 0..depth {
        let width = N >> level;
        for i in 0..width / 2 {
            row[i] = Poseidon2::hash([NODE_DOMAIN, row[2 * i], row[2 * i + 1]], 3);
        }
        let covered = 1 << (level + 1);
        if (count > covered / 2) & (count <= covered) { result = row[0]; }
    }
    result
}

/// Both configuration halves are range-constrained by their u128 types.
pub fn signed_message(salt: Field, count: u32, root: Field, config: [u128; 2], metadata: Field) -> Field {
    Poseidon2::hash([MESSAGE_DOMAIN, SUITE_ID, salt, count as Field, root, config[0] as Field, config[1] as Field, metadata], 8)
}

/// Revealed mode releases only the salted signed message. Configuration is an
/// agreed public input; salt, terms, count, root and signature remain private.
pub(crate) fn tree_message<let N: u32>(quads: [[TermCell; 4]; N], salts: [Field; N], salt: Field, count: u32, config: [u128; 2], metadata: Field) -> Field {
    signed_message(salt, count, credential_root(quads, salts, salt, count), config, metadata)
}

/// Hidden mode returns nothing. The caller exposes the issuer public key and
/// agreed configuration, and keeps message and all credential material private.
pub(crate) fn check_credential_signature<let N: u32>(quads: [[TermCell; 4]; N], salts: [Field; N], salt: Field, count: u32, config: [u128; 2],
    pk: Point, r: Point, s: Field, e: Field, e_k: Field, metadata: Field) {
    let message = tree_message(quads, salts, salt, count, config, metadata);
    schnorr_verify(pk, r, s, e, e_k, message);
}

// Public credential entry points authenticate the envelope metadata as well.
pub use crate::suite_metadata::{open_credential_hidden, open_credential_revealed};
