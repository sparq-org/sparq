use crate::expr_arith::{ADD, binary, DIV, exact_to_bits, MUL, SUB, unary, widen_float};
use crate::expr_compare::{
    EQ, GE, GT, in_list, LE, LT, not_in_list, numeric_compare, same_term, value_compare,
};
use crate::expr_core::{
    as_numeric, BOOLEAN, bound, choose, coalesce, DATE, DATETIME, DECIMAL, DOUBLE, ebv, exact,
    EXACT_LIMIT, filter_accept, FLOAT, floating, INTEGER, Lane, lane, logical_and, logical_not,
    logical_or, NUMERIC, Numeric, Scalar, SCALE, STRING, StringLane, TemporalLane, TERM, truth,
    variable, zero_numeric,
};
use crate::expr_term_contract::{CheckedOpening, TermCell};

// Test-only fixture. Production has no CheckedOpening implementation until the
// credential lane supplies its lexical/key constraints.
struct Fixture {
    scalar: Scalar<4>,
}
impl CheckedOpening<4> for Fixture {
    fn open(self) -> Scalar<4> {
        self.scalar
    }
}
fn cell(bound: bool, kind: u8, enc: Field) -> TermCell {
    TermCell { bound, kind, enc, key_class: 0, key: 0, dt: 0, lang: 0, credential: 0 }
}
fn scalar(tag: u8) -> Scalar<4> {
    Scalar {
        tag,
        valid: true,
        boolean: false,
        number: zero_numeric(),
        string: StringLane { bytes: [0; 4], len: 0, lang: 0 },
        temporal: TemporalLane { key_class: 3, key: 0 },
        term: cell(true, 3, 1),
    }
}
fn number(n: Numeric) -> (Lane<Scalar<4>>, bool) {
    let mut v = scalar(NUMERIC);
    v.number = n;
    (lane(v), false)
}
fn integer(n: Field) -> (Lane<Numeric>, bool) {
    (lane(exact(INTEGER, false, n * SCALE)), false)
}
fn check_bool(a: (Lane<bool>, bool), value: bool, error: bool) {
    let (v, e) = a;
    assert_eq(e, error, "error lane");
    assert_eq(v.value, value, "boolean value");
    assert(!v.capacity);
}
#[test]
fn boolean_truth_tables() {
    // Published section 17.2 table, including error payloads that are not false.
    let inputs = [truth(true, false), truth(false, false), truth(true, true)];
    let ors = [true, true, true, true, false, false, true, false, false];
    let ands = [true, false, false, false, false, false, false, false, false];
    let or_errors = [false, false, false, false, false, true, false, true, true];
    let and_errors = [false, false, true, false, false, false, true, false, true];
    for i in 0..3 {
        for j in 0..3 {
            check_bool(
                logical_or(inputs[i], inputs[j]),
                ors[i * 3 + j],
                or_errors[i * 3 + j],
            );
            check_bool(
                logical_and(inputs[i], inputs[j]),
                ands[i * 3 + j],
                and_errors[i * 3 + j],
            );
        }
    }
    check_bool(logical_not(truth(true, false)), false, false);
    check_bool(logical_not(truth(false, false)), true, false);
    check_bool(logical_not(truth(true, true)), false, true);
}
#[test]
fn effective_boolean_values() {
    let mut v = scalar(BOOLEAN);
    v.boolean = true;
    check_bool(ebv((lane(v), false)), true, false);
    v.valid = false;
    check_bool(ebv((lane(v), false)), false, false);
    v.tag = NUMERIC;
    check_bool(ebv((lane(v), false)), false, false);
    check_bool(ebv(number(exact(INTEGER, true, SCALE))), true, false);
    check_bool(ebv(number(exact(DECIMAL, false, 0))), false, false);
    for bits in [0x7ff8000000000000, 0x8000000000000000, 0] {
        check_bool(ebv(number(floating(DOUBLE, bits))), false, false);
    }
    check_bool(ebv(number(floating(FLOAT, 0x7fc00000))), false, false);
    check_bool(
        ebv(number(floating(DOUBLE, 0x7ff0000000000000))),
        true,
        false,
    );
    v = scalar(STRING);
    v.string.bytes = [97, 0, 0, 0];
    v.string.len = 1;
    check_bool(ebv((lane(v), false)), true, false);
    v.string.len = 0;
    v.string.bytes = [0; 4];
    check_bool(ebv((lane(v), false)), false, false);
    for tag in [TERM, DATE, DATETIME] {
        v = scalar(tag);
        check_bool(ebv((lane(v), false)), false, true);
    }
    v = scalar(BOOLEAN);
    v.term.bound = false;
    check_bool(ebv(variable(Fixture { scalar: v })), false, true);
    check_bool(bound(v.term), false, false);
    v.term.bound = true;
    check_bool(bound(v.term), true, false);
}
#[test]
fn conditional_and_coalesce() {
    let yes = (lane(7 as Field), false);
    let no = (lane(0 as Field), true);
    let (v, e) = choose(truth(true, false), yes, no);
    assert(!e);
    assert_eq(v.value, 7);
    let (_, e) = choose(truth(false, false), yes, no);
    assert(e);
    let (_, e) = choose(truth(false, true), yes, yes);
    assert(e);
    let (v, e) = coalesce([no, yes, (lane(9), false)], 0);
    assert(!e);
    assert_eq(v.value, 7);
    let (_, e) = coalesce([no], 0);
    assert(e);
    let (_, e) = coalesce::<Field, 0>([], 0);
    assert(e);
    let capped = (Lane { value: 8 as Field, capacity: true }, false);
    let (v, e) = coalesce([capped, yes], 0);
    assert(!e & v.capacity);
    assert_eq(v.value, 8);
    let (v, e) = choose(truth(true, false), yes, capped);
    assert(!e & !v.capacity);
}
#[test]
fn numeric_comparison_and_promotion() {
    check_bool(
        numeric_compare(
            integer(2),
            (lane(exact(DECIMAL, false, 2 * SCALE)), false),
            EQ,
        ),
        true,
        false,
    );
    check_bool(
        numeric_compare((lane(exact(INTEGER, true, SCALE)), false), integer(0), LT),
        true,
        false,
    );
    for op in [EQ, LE, GE] {
        check_bool(numeric_compare(integer(2), integer(2), op), true, false);
    }
    for op in [LT, GT] {
        check_bool(numeric_compare(integer(2), integer(2), op), false, false);
    }
    for op in [EQ, LT, LE, GT, GE] {
        let nan = (lane(floating(DOUBLE, 0x7ff8000000000000)), false);
        check_bool(numeric_compare(nan, nan, op), false, false);
    }
    check_bool(
        numeric_compare((lane(floating(FLOAT, 0x80000000)), false), integer(0), EQ),
        true,
        false,
    );
    check_bool(
        numeric_compare(
            integer(16777217),
            (lane(floating(FLOAT, 0x4b800000)), false),
            EQ,
        ),
        true,
        false,
    );
    check_bool(
        numeric_compare(
            integer(16777217),
            (lane(floating(DOUBLE, 0x4170000000000000)), false),
            EQ,
        ),
        false,
        false,
    );
    assert_eq(exact_to_bits::<23, 127>(exact(DECIMAL, false, 100000000000000000)), 0x3dcccccd);
    assert_eq(
        exact_to_bits::<52, 1023>(exact(DECIMAL, false, 100000000000000000)),
        0x3fb999999999999a,
    );
    assert_eq(widen_float(1), 0x36a0000000000000);
    assert_eq(widen_float(0x80000000), 0x8000000000000000);
    assert_eq(widen_float(0x7f800000), 0x7ff0000000000000);
}
#[test]
fn term_equality_and_membership() {
    let a = number(exact(INTEGER, false, 2 * SCALE));
    let b = number(exact(DECIMAL, false, 2 * SCALE));
    let err = (lane(scalar(TERM)), true);
    check_bool(in_list(a, [err, b]), true, false);
    check_bool(not_in_list(a, [b, err]), false, false);
    check_bool(
        in_list(a, [number(exact(INTEGER, false, 3 * SCALE)), err]),
        false,
        true,
    );
    check_bool(in_list::<4, 0>(err, []), false, false);
    check_bool(not_in_list::<4, 0>(err, []), true, false);
    let mut opaque = scalar(TERM);
    opaque.term.enc = 8;
    let mut other = opaque;
    other.term.enc = 9;
    check_bool(
        value_compare((lane(opaque), false), (lane(other), false), EQ),
        false,
        true,
    );
    check_bool(
        same_term((lane(opaque.term), false), (lane(other.term), false)),
        false,
        false,
    );
    check_bool(
        value_compare((lane(opaque), false), (lane(opaque), false), EQ),
        true,
        false,
    );
    opaque.term.kind = 1; // IRI versus literal is false, without a type error.
    check_bool(
        value_compare((lane(opaque), false), (lane(other), false), EQ),
        false,
        false,
    );
    other.term.bound = false;
    check_bool(
        same_term((lane(opaque.term), false), (lane(other.term), false)),
        false,
        true,
    );
    let mut invalid = scalar(NUMERIC);
    invalid.valid = false;
    check_bool(
        value_compare((lane(invalid), false), (lane(invalid), false), EQ),
        false,
        true,
    );
}
#[test]
fn strings_booleans_and_dates() {
    let mut a = scalar(STRING);
    let mut b = a;
    a.string.bytes = [97, 0, 0, 0];
    a.string.len = 1;
    b.string.bytes = [97, 0, 0, 0];
    b.string.len = 2;
    check_bool(
        value_compare((lane(a), false), (lane(b), false), LT),
        true,
        false,
    );
    check_bool(
        value_compare((lane(a), false), (lane(b), false), EQ),
        false,
        false,
    );
    b = a;
    b.string.lang = 6;
    check_bool(
        value_compare((lane(a), false), (lane(b), false), EQ),
        false,
        false,
    );
    check_bool(
        value_compare((lane(a), false), (lane(b), false), LT),
        false,
        true,
    );
    a = scalar(BOOLEAN);
    b = a;
    b.boolean = true;
    check_bool(
        value_compare((lane(a), false), (lane(b), false), LT),
        true,
        false,
    );
    a = scalar(DATETIME);
    b = a;
    b.temporal.key = 1;
    check_bool(
        value_compare((lane(a), false), (lane(b), false), LT),
        true,
        false,
    );
    b.temporal.key_class = 4;
    let (v, e) = value_compare((lane(a), false), (lane(b), false), LT);
    assert(!e & v.capacity);
}
#[test]
fn arithmetic_examples_and_boundaries() {
    for op in [ADD, SUB, MUL, DIV] {
        let (v, e) = binary(integer(6), integer(2), op);
        let expected = if op == ADD {
            8
        } else if op == SUB {
            4
        } else if op == MUL {
            12
        } else {
            3
        };
        assert(!e & !v.capacity);
        assert_eq(v.value.magnitude, expected * SCALE);
        assert_eq(v.value.ty, if op == DIV { DECIMAL } else { INTEGER });
    }
    let (v, e) = binary(integer(1), integer(2), DIV);
    assert(!e & !v.capacity);
    assert_eq(v.value.magnitude, SCALE / 2);
    let (v, e) = binary(integer(1), integer(3), DIV);
    assert(!e & v.capacity);
    let (_, e) = binary(integer(1), integer(0), DIV);
    assert(e);
    let (v, e) = binary(integer(0), integer(1), SUB);
    assert(!e & !v.capacity);
    assert(v.value.negative);
    let (v, e) = unary(integer(2), true);
    assert(!e);
    assert(v.value.negative);
    let (v, e) = unary((lane(floating(FLOAT, 0)), false), true);
    assert(!e);
    assert_eq(v.value.bits, 0x80000000);
    let (v, e) = unary((lane(floating(DOUBLE, 0x8000000000000000)), false), true);
    assert(!e);
    assert_eq(v.value.bits, 0);
    let big = (lane(exact(DECIMAL, false, EXACT_LIMIT - 1)), false);
    let (v, e) = binary(big, big, ADD);
    assert(!e & v.capacity);
    let (v, e) = binary(big, big, MUL);
    assert(!e & v.capacity);
    let tiny = (lane(exact(DECIMAL, false, 1)), false);
    let (v, e) = binary(tiny, tiny, MUL);
    assert(!e & v.capacity);
    let (v, e) = binary(
        (lane(floating(DOUBLE, 0x3ff0000000000000)), false),
        integer(0),
        DIV,
    );
    assert(!e & !v.capacity);
    assert_eq(v.value.bits, 0x7ff0000000000000);
    let (v, e) = binary((lane(floating(FLOAT, 0)), false), integer(0), DIV);
    assert(!e & !v.capacity);
    assert_eq(v.value.bits, 0x7fc00000);
    let mut invalid = scalar(NUMERIC);
    invalid.valid = false;
    let (_, e) = unary(as_numeric((lane(invalid), false)), false);
    assert(e);
}

#[test(should_fail_with = "forced error low")]
fn ebv_error_low() {
    assert(!ebv((lane(scalar(TERM)), false)).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn and_error_low() {
    assert(!logical_and(truth(true, false), truth(false, true)).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn or_error_low() {
    assert(!logical_or(truth(false, false), truth(false, true)).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn not_error_low() {
    assert(!logical_not(truth(false, true)).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn if_error_low() {
    assert(!choose(truth(false, true), (lane(1), false), (lane(2), false)).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn coalesce_error_low() {
    assert(!coalesce([(lane(1), true)], 0).1, "forced error low");
}
#[test(should_fail_with = "bound value")]
fn bound_value_changed() {
    assert(bound(cell(false, 0, 0)).0.value, "bound value");
}
#[test(should_fail_with = "forced error low")]
fn variable_error_low() {
    let mut v = scalar(NUMERIC);
    v.term.bound = false;
    assert(!variable(Fixture { scalar: v }).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn compare_error_low() {
    assert(!numeric_compare((integer(1).0, true), integer(2), EQ).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn equality_error_low() {
    let a = scalar(NUMERIC);
    let mut b = scalar(STRING);
    b.term.enc = 2;
    assert(!value_compare((lane(a), false), (lane(b), false), EQ).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn same_term_error_low() {
    assert(
        !same_term(
            (lane(cell(false, 0, 0)), false),
            (lane(cell(true, 0, 0)), false),
        )
            .1,
        "forced error low",
    );
}
#[test(should_fail_with = "forced error low")]
fn in_error_low() {
    let a = number(zero_numeric());
    assert(!in_list(a, [(a.0, true)]).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn not_in_error_low() {
    let a = number(zero_numeric());
    assert(!not_in_list(a, [(a.0, true)]).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn add_error_low() {
    assert(!binary((integer(1).0, true), integer(2), ADD).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn sub_error_low() {
    assert(!binary((integer(1).0, true), integer(2), SUB).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn mul_error_low() {
    assert(!binary((integer(1).0, true), integer(2), MUL).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn div_error_low() {
    assert(!binary(integer(1), integer(0), DIV).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn unary_plus_error_low() {
    assert(!unary((integer(1).0, true), false).1, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn unary_minus_error_low() {
    assert(!unary((integer(1).0, true), true).1, "forced error low");
}
#[test(should_fail_with = "expression capacity")]
fn capacity_forced_low() {
    let (v, e) = binary(
        (lane(exact(DECIMAL, false, EXACT_LIMIT - 1)), false),
        integer(1),
        ADD,
    );
    let _ = filter_accept((Lane { value: false, capacity: v.capacity }, e));
}
#[test(should_fail_with = "string padding")]
fn string_padding_changed() {
    let mut a = scalar(STRING);
    a.string.bytes = [1, 0, 0, 0];
    let _ = ebv((lane(a), false));
}
#[test(should_fail_with = "integer scale")]
fn integer_fraction_changed() {
    let _ = exact(INTEGER, false, 1);
}

#[test]
fn unary_plus_keeps_identity() {
    let mut v = scalar(NUMERIC);
    v.number = exact(INTEGER, false, 5 * SCALE);
    v.term.enc = 42;
    v.term.dt = 11;
    let (out, e) = crate::expr_arith::unary_plus_term((lane(v), false));
    assert(!e);
    assert_eq(out.value.term.enc, 42);
    assert_eq(out.value.term.dt, 11);
}
#[test(should_fail_with = "forced error low")]
fn unary_plus_term_error_low() {
    assert(
        !crate::expr_arith::unary_plus_term((lane(scalar(STRING)), false)).1,
        "forced error low",
    );
}
#[test]
fn all_comparison_errors() {
    for op in [EQ, LT, LE, GT, GE] {
        check_bool(
            numeric_compare((integer(1).0, true), integer(2), op),
            false,
            true,
        );
        let mut v = scalar(DATETIME);
        v.valid = false;
        check_bool(
            value_compare((lane(v), false), (lane(v), false), op),
            false,
            true,
        );
    }
    let cap = (Lane { value: exact(INTEGER, false, SCALE), capacity: true }, false);
    let (v, e) = binary(cap, (integer(0).0, true), ADD);
    assert(e & v.capacity);
    let (v, e) = coalesce([(v, true), integer(2)], zero_numeric());
    assert(!e & v.capacity);
    let (v, e) = unary(integer(0), true);
    assert(!e & !v.value.negative);
    assert_eq(exact_to_bits::<52, 1023>(exact(DECIMAL, false, 1)), 0x3c32725dd1d243ac);
    assert_eq(widen_float(0x7fffff), 0x380fffffc0000000);
}
#[test(should_fail)]
fn exact_input_outside_bucket() {
    let _ = exact(DECIMAL, false, EXACT_LIMIT);
}
#[test(should_fail)]
fn float_input_upper_bits() {
    let _ = floating(FLOAT, 0x100000000);
}

#[test]
fn floating_boundaries() {
    let float_max = (lane(floating(FLOAT, 0x7f7fffff)), false);
    let (v, e) = binary(float_max, float_max, ADD);
    assert(!e & !v.capacity);
    assert_eq(v.value.bits, 0x7f800000);
    let (v, e) = binary(
        (lane(floating(FLOAT, 1)), false),
        (lane(floating(FLOAT, 0x40000000)), false),
        DIV,
    );
    assert(!e & !v.capacity);
    assert_eq(v.value.bits, 0);
    let double_max = (lane(floating(DOUBLE, 0x7fefffffffffffff)), false);
    let (v, e) = binary(double_max, double_max, ADD);
    assert(!e & !v.capacity);
    assert_eq(v.value.bits, 0x7ff0000000000000);
    let (v, e) = binary(
        (lane(floating(DOUBLE, 1)), false),
        (lane(floating(DOUBLE, 0x4000000000000000)), false),
        DIV,
    );
    assert(!e & !v.capacity);
    assert_eq(v.value.bits, 0);
    let (v, e) = binary(
        (lane(floating(DOUBLE, 0x8000000000000000)), false),
        integer(1),
        DIV,
    );
    assert(!e & !v.capacity);
    assert_eq(v.value.bits, 0x8000000000000000);
    // Exact halfway values: binary64 nearest/even must round only once.
    assert_eq(
        exact_to_bits::<52, 1023>(exact(DECIMAL, false, 34359738368000003814697265625)),
        0x4220000000000000,
    );
    assert_eq(
        exact_to_bits::<52, 1023>(exact(DECIMAL, false, 34359738368000011444091796875)),
        0x4220000000000002,
    );
}
