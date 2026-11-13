// Static result-construction seam for term functions. This is separate from
// serialization of newly computed boolean/numeric lanes.
use crate::expr_core::{check_string, Lane, StringLane};
use crate::expr_term::TermParts;
use crate::expr_term_contract::TermCell;
use std::collections::bounded_vec::BoundedVec;
pub trait CheckedTermConstructor<let L: u32, let D: u32, let G: u32> {
    // Implement by A's open_term and, for literals, recompute_key. The candidate
    // key is only a constrained hint; it cannot change any computed character.
    fn construct(
        kind: u8,
        lexical: BoundedVec<u8, L>,
        datatype: BoundedVec<u8, D>,
        language: BoundedVec<u8, G>,
        key: [u8; 31],
    ) -> TermCell;
}
fn bytes<let L: u32>(value: StringLane<L>) -> BoundedVec<u8, L> {
    check_string(value);
    assert_eq(value.lang, 0, "term part metadata");
    let mut out = BoundedVec::new();
    for i in 0..L {
        if i < value.len {
            out.push(value.bytes[i]);
        }
    }
    out
}
pub fn term<E, let L: u32, let D: u32, let G: u32>(
    input: (Lane<TermParts<L, D, G>>, bool),
    key: [u8; 31],
) -> (Lane<TermCell>, bool)
where
    E: CheckedTermConstructor<L, D, G>,
{
    let (a, e) = input;
    let value = if e | a.capacity {
        crate::expr_to_term::unbound()
    } else {
        let p = a.value;
        let c = E::construct(
            p.kind,
            bytes(p.lexical),
            bytes(p.datatype),
            bytes(p.language),
            key,
        );
        let mut expected: Field = 0;
        for b in key {
            expected = expected * 256 + b as Field;
        }
        assert(
            c.bound()
                & (c.kind() == p.kind)
                & (c.key() == expected)
                & (c.key_class() == key[0])
                & (c.credential() == 0),
            "term construction contract",
        );
        c
    };
    (Lane { value, capacity: a.capacity }, e)
}
