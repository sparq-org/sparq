use crate::expr_core::{BOOLEAN, DATE, NUMERIC, SCALE, STRING};
use crate::expr_open::variable;
use crate::expr_term_contract::{
    CheckedLexicalOpening, DT_BOOLEAN, DT_DATE, DT_INTEGER, DT_STRING, LexicalOpening, TermCell,
};
use std::collections::bounded_vec::BoundedVec;
struct Fixture {
    cell: TermCell,
    bytes: [u8; 64],
    len: u32,
}
impl CheckedLexicalOpening<64, 0, 0> for Fixture {
    fn open_lexical(self) -> LexicalOpening<64, 0, 0> {
        let mut lexical = BoundedVec::new();
        for i in 0..64 {
            if i < self.len {
                lexical.push(self.bytes[i]);
            }
        }
        LexicalOpening {
            cell: self.cell,
            lexical,
            datatype: BoundedVec::new(),
            language: BoundedVec::new(),
        }
    }
}
fn fixture(dt: Field, class: u8, payload: Field) -> Fixture {
    Fixture {
        cell: TermCell {
            bound: true,
            kind: 3,
            enc: 1,
            key_class: class,
            key: class as Field * 0x1000000000000000000000000000000000000000000000000000000000000
                + payload,
            dt,
            lang: 1,
            credential: 0,
        },
        bytes: [0; 64],
        len: 0,
    }
}
#[test]
fn checked_key_lane_derivation() {
    let bias: Field = 0x800000000000000000000000000000000000000000000000000000000000;
    let (v, e) = variable(fixture(DT_INTEGER, 1, bias + 281474976710656 * SCALE));
    assert(!e);
    assert(!v.capacity);
    assert_eq(v.value.tag, NUMERIC);
    assert_eq(v.value.number.magnitude, 281474976710656 * SCALE);
    let (v, e) = variable(fixture(DT_INTEGER, 1, bias - 5 * SCALE));
    assert(!e);
    assert(v.value.number.negative);
    assert_eq(v.value.number.magnitude, 5 * SCALE);
    let (v, e) = variable(fixture(
        DT_BOOLEAN,
        5,
        0x10000000000000000000000000000000000000000000000000000000000,
    ));
    assert(!e);
    assert_eq(v.value.tag, BOOLEAN);
    assert(v.value.boolean);
    let (v, e) = variable(fixture(
        DT_DATE,
        8,
        (0x8000000000000000 + 20770) * 0x100000000000000000000000000000000000000000000,
    ));
    assert(!e);
    assert_eq(v.value.tag, DATE);
    assert_eq(v.value.temporal.key, 0x8000000000000000 + 20770);
    let mut input = fixture(DT_STRING, 6, 0);
    input.len = 3;
    input.bytes[0] = 97;
    input.bytes[1] = 0;
    input.bytes[2] = 98;
    let (v, e) = variable(input);
    assert(!e);
    assert_eq(v.value.tag, STRING);
    assert_eq(v.value.string.len, 3);
    assert_eq(v.value.string.bytes[2], 98);
}
#[test]
fn checked_key_capacity_and_invalid() {
    let bias: Field = 0x800000000000000000000000000000000000000000000000000000000000;
    let (v, e) = variable(fixture(DT_INTEGER, 1, bias + crate::expr_core::EXACT_LIMIT));
    assert(!e);
    assert(v.capacity);
    let (v, e) = variable(fixture(DT_INTEGER, 0, 0));
    assert(!e);
    assert(!v.value.valid);
    assert(!v.capacity);
    let (v, e) = variable(fixture(DT_DATE, 127, 0));
    assert(!e);
    assert(v.capacity);
}
#[test(should_fail_with = "opening UTF-8")]
fn malformed_opening_utf8() {
    let mut input = fixture(DT_STRING, 7, 0);
    input.len = 1;
    input.bytes[0] = 255;
    let _ = variable(input);
}
#[test(should_fail_with = "key class byte")]
fn changed_class_byte() {
    let mut input = fixture(DT_INTEGER, 1, 0);
    input.cell.key = 0;
    let _ = variable(input);
}
#[test(should_fail_with = "temporal key padding")]
fn changed_temporal_padding() {
    let _ = variable(fixture(DT_DATE, 8, 1));
}
#[test(should_fail_with = "boolean key payload")]
fn changed_boolean_payload() {
    let _ = variable(fixture(DT_BOOLEAN, 5, 1));
}
