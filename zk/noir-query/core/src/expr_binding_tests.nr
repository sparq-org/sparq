use crate::expr_core::{DECIMAL, exact, INTEGER, Lane, lane, SCALE, truth, zero_numeric};
use crate::expr_term_contract::TermCell;
use crate::expr_to_term::{boolean, ComputedTermEncoder, numeric, numeric_lexical};
use std::collections::bounded_vec::BoundedVec;

// Error-only control: any invocation of this constructor fails. Successful
// encodings are checked against lane A's real constructor in the live Rust test.
struct NeverEncode {}
impl ComputedTermEncoder for NeverEncode {
    fn literal(
        _lexical: BoundedVec<u8, 64>,
        _datatype: BoundedVec<u8, 64>,
        _key: [u8; 31],
    ) -> TermCell {
        assert(false, "error invoked encoder");
        crate::expr_to_term::unbound()
    }
}
fn text<let N: u32>(value: BoundedVec<u8, 64>, expected: [u8; N]) {
    assert_eq(value.len(), N);
    for i in 0..64 {
        assert_eq(value.get_unchecked(i), if i < N { expected[i] } else { 0 });
    }
}
#[test]
fn computed_numeric_canonical_lexical() {
    text(numeric_lexical(exact(INTEGER, false, 0)), "0".as_bytes());
    text(
        numeric_lexical(exact(INTEGER, true, 42 * SCALE)),
        "-42".as_bytes(),
    );
    text(
        numeric_lexical(exact(INTEGER, false, 281474976710656 * SCALE)),
        "281474976710656".as_bytes(),
    );
    text(numeric_lexical(exact(DECIMAL, false, 0)), "0.0".as_bytes());
    text(
        numeric_lexical(exact(DECIMAL, true, 300000000000000000)),
        "-0.3".as_bytes(),
    );
    text(
        numeric_lexical(exact(DECIMAL, false, SCALE + 1)),
        "1.000000000000000001".as_bytes(),
    );
    text(
        numeric_lexical(exact(DECIMAL, false, 1200000000000000000)),
        "1.2".as_bytes(),
    );
}
#[test]
fn bind_error_is_unbound() {
    let (a, e) = boolean::<NeverEncode>(truth(true, true));
    assert(e);
    assert(!a.value.bound());
    let (b, e) = numeric::<NeverEncode>((lane(zero_numeric()), true));
    assert(e);
    assert(!b.value.bound());
    let (c, e) = boolean::<NeverEncode>((Lane { value: true, capacity: true }, false));
    assert(!e);
    assert(c.capacity);
    assert(!c.value.bound());
}
#[test(should_fail_with = "forced error low")]
fn boolean_bind_error_low() {
    let (_, e) = boolean::<NeverEncode>(truth(true, true));
    assert(!e, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn numeric_bind_error_low() {
    let (_, e) = numeric::<NeverEncode>((lane(zero_numeric()), true));
    assert(!e, "forced error low");
}
#[test(should_fail_with = "integer scale")]
fn computed_integer_fraction_changed() {
    let n =
        crate::expr_core::Numeric { ty: INTEGER, negative: false, magnitude: SCALE + 1, bits: 0 };
    let _ = numeric_lexical(n);
}
#[test]
fn cell_binding_retains_identity() {
    let original = TermCell {
        bound: true,
        kind: 3,
        enc: 101,
        key_class: 1,
        key: 102,
        dt: 103,
        lang: 104,
        credential: 0,
    };
    let (value, e) = crate::expr_to_term::term((lane(original), false));
    assert(!e);
    assert_eq(value.value.enc(), original.enc());
    assert_eq(value.value.key(), original.key());
    let (value, e) = crate::expr_to_term::term((lane(original), true));
    assert(e);
    assert(!value.value.bound());
}
