use crate::expr_arith::{binary, MUL};
use crate::expr_compare::{EQ, GE, in_list, LE, numeric_compare, value_compare};
use crate::expr_core::{
    as_numeric, BOOLEAN, DATE, DECIMAL, exact, INTEGER, Lane, lane, logical_and, NUMERIC, Scalar,
    SCALE, STRING, StringLane, TemporalLane, TERM, truth, zero_numeric,
};
use crate::expr_term_contract::TermCell;

// Isolated semantic fixtures. The live target test replaces these helpers with
// lane A's actual open_term/open_lexical/recompute_key and expr_open::variable.
fn scalar(tag: u8, kind: u8, enc: Field) -> Scalar<128> {
    Scalar {
        tag,
        valid: true,
        boolean: false,
        number: zero_numeric(),
        string: StringLane { bytes: [0; 128], len: 0, lang: 0 },
        temporal: TemporalLane { key_class: 8, key: 0 },
        term: TermCell {
            bound: true,
            kind,
            enc,
            key_class: 0,
            key: 0,
            dt: 0,
            lang: 0,
            credential: 0,
        },
    }
}
fn number(magnitude: Field, decimal: bool) -> (Lane<Scalar<128>>, bool) {
    let mut s = scalar(NUMERIC, 3, 1);
    s.number = exact(if decimal { DECIMAL } else { INTEGER }, false, magnitude);
    (lane(s), false)
}
fn integer(value: Field) -> (Lane<Scalar<128>>, bool) {
    number(value * SCALE, false)
}
fn boolean(value: bool) -> (Lane<Scalar<128>>, bool) {
    let mut s = scalar(BOOLEAN, 3, 1);
    s.boolean = value;
    (lane(s), false)
}
fn text<let N: u32>(bytes: [u8; N]) -> (Lane<Scalar<128>>, bool) {
    let mut s = scalar(STRING, 3, 1);
    s.string.len = N;
    for i in 0..N {
        s.string.bytes[i] = bytes[i];
    }
    (lane(s), false)
}
fn iri(value: Field) -> (Lane<Scalar<128>>, bool) {
    (lane(scalar(TERM, 1, value)), false)
}
// Day keys are supplied by the live fixture's authenticated YYYY-MM-DD opening.
fn date(day: u64) -> (Lane<Scalar<128>>, bool) {
    let mut s = scalar(DATE, 3, 1);
    s.temporal.key = 0x8000000000000000 + day as Field;
    (lane(s), false)
}
fn missing() -> (Lane<Scalar<128>>, bool) {
    let mut s = scalar(TERM, 0, 0);
    s.term.bound = false;
    (lane(s), true)
}

// CASES: also run with real term openings by the live Rust integration test.
fn all<let N: u32>(values: [(Lane<bool>, bool); N]) -> (Lane<bool>, bool) {
    let mut result = truth(true, false);
    for value in values {
        result = logical_and(result, value);
    }
    result
}
fn check(result: (Lane<bool>, bool), expected: bool, error: bool) {
    let (v, e) = result;
    assert_eq(e, error, "target error");
    assert_eq(v.value, expected, "target value");
    assert(!v.capacity, "target capacity");
}
fn stated(
    v0: (Lane<Scalar<128>>, bool),
    v1: (Lane<Scalar<128>>, bool),
    v2: (Lane<Scalar<128>>, bool),
    v3: (Lane<Scalar<128>>, bool),
) -> (Lane<bool>, bool) {
    all([
        value_compare(v0, integer(5), GE),
        value_compare(v1, integer(3), LE),
        value_compare(v2, integer(0), EQ),
        value_compare(v3, boolean(true), EQ),
    ])
}
fn closed(
    v0: (Lane<Scalar<128>>, bool),
    v1: (Lane<Scalar<128>>, bool),
    from: (Lane<Scalar<128>>, bool),
    to: (Lane<Scalar<128>>, bool),
    absent: (Lane<bool>, bool),
) -> (Lane<bool>, bool) {
    all([
        value_compare(v0, integer(5), GE),
        value_compare(v1, integer(3), LE),
        value_compare(from, date(18944), LE),
        value_compare(to, date(20770), GE),
        absent,
    ])
}
fn trial(
    age: (Lane<Scalar<128>>, bool),
    dx: (Lane<Scalar<128>>, bool),
    from: (Lane<Scalar<128>>, bool),
    to: (Lane<Scalar<128>>, bool),
    absent: (Lane<bool>, bool),
) -> (Lane<bool>, bool) {
    all([
        value_compare(age, integer(18), GE),
        value_compare(age, integer(65), LE),
        value_compare(dx, text("condition-a".as_bytes()), EQ),
        value_compare(from, date(20758), LE),
        value_compare(to, date(20770), GE),
        absent,
    ])
}
fn recycled(
    value: (Lane<Scalar<128>>, bool),
    issuers: (Lane<Scalar<128>>, bool),
    count: (Lane<Scalar<128>>, bool),
) -> (Lane<bool>, bool) {
    all([
        value_compare(value, number(300000000000000000, true), GE),
        value_compare(issuers, integer(3), EQ),
        value_compare(count, integer(3), EQ),
    ])
}
fn totals(total: (Lane<Scalar<128>>, bool), n: (Lane<Scalar<128>>, bool)) -> (Lane<bool>, bool) {
    all(
        [value_compare(total, integer(500), GE), value_compare(n, integer(3), EQ)],
    )
}
#[test]
fn target_1a_insurance_stated() {
    check(
        stated(integer(12), integer(0), integer(0), boolean(true)),
        true,
        false,
    );
    check(
        stated(integer(4), integer(0), integer(0), boolean(true)),
        false,
        false,
    );
    check(
        stated(integer(12), integer(4), integer(0), boolean(true)),
        false,
        false,
    );
    check(
        stated(integer(12), integer(0), integer(1), boolean(true)),
        false,
        false,
    );
    check(
        stated(integer(12), integer(0), integer(0), boolean(false)),
        false,
        false,
    );
    check(
        stated(missing(), integer(0), integer(0), boolean(true)),
        false,
        true,
    );
    check(
        stated(
            integer(281474976710656),
            integer(0),
            integer(0),
            boolean(true),
        ),
        true,
        false,
    );
}
#[test]
fn target_1b_insurance_closed() {
    check(
        closed(
            integer(12),
            integer(0),
            date(18944),
            date(20770),
            truth(true, false),
        ),
        true,
        false,
    );
    check(
        closed(
            integer(4),
            integer(0),
            date(18944),
            date(20770),
            truth(true, false),
        ),
        false,
        false,
    );
    check(
        closed(
            integer(12),
            integer(4),
            date(18944),
            date(20770),
            truth(true, false),
        ),
        false,
        false,
    );
    check(
        closed(
            integer(12),
            integer(0),
            date(18945),
            date(20770),
            truth(true, false),
        ),
        false,
        false,
    );
    check(
        closed(
            integer(12),
            integer(0),
            date(18944),
            date(20769),
            truth(true, false),
        ),
        false,
        false,
    );
    check(
        closed(
            integer(12),
            integer(0),
            date(18944),
            date(20770),
            truth(false, false),
        ),
        false,
        false,
    );
    check(
        closed(
            integer(12),
            integer(0),
            missing(),
            date(20770),
            truth(true, false),
        ),
        false,
        true,
    );
    // E && false absorbs E; E && true retains E, including absence inputs.
    check(
        closed(
            missing(),
            integer(0),
            date(18944),
            date(20770),
            truth(false, false),
        ),
        false,
        false,
    );
    check(value_compare(date(18944), date(18944), GE), true, false);
    check(value_compare(date(18943), date(18944), GE), false, false);
}
#[test]
fn target_2_trial() {
    check(
        trial(
            integer(45),
            text("condition-a".as_bytes()),
            date(20758),
            date(20770),
            truth(true, false),
        ),
        true,
        false,
    );
    check(
        trial(
            integer(17),
            text("condition-a".as_bytes()),
            date(20758),
            date(20770),
            truth(true, false),
        ),
        false,
        false,
    );
    check(
        trial(
            integer(66),
            text("condition-a".as_bytes()),
            date(20758),
            date(20770),
            truth(true, false),
        ),
        false,
        false,
    );
    check(
        trial(
            integer(45),
            text("condition-b".as_bytes()),
            date(20758),
            date(20770),
            truth(true, false),
        ),
        false,
        false,
    );
    check(
        trial(
            integer(45),
            text("condition-a".as_bytes()),
            date(20759),
            date(20770),
            truth(true, false),
        ),
        false,
        false,
    );
    check(
        trial(
            integer(45),
            text("condition-a".as_bytes()),
            date(20758),
            date(20769),
            truth(true, false),
        ),
        false,
        false,
    );
    let excluded = [text("excluded-x".as_bytes()), text("excluded-y".as_bytes())];
    check(
        in_list(text("excluded-x".as_bytes()), excluded),
        true,
        false,
    );
    check(
        in_list(text("medicine-a".as_bytes()), excluded),
        false,
        false,
    );
    check(
        trial(
            integer(45),
            text("condition-a".as_bytes()),
            date(20758),
            date(20770),
            truth(false, false),
        ),
        false,
        false,
    );
    check(
        trial(
            integer(45),
            missing(),
            date(20758),
            date(20770),
            truth(true, false),
        ),
        false,
        true,
    );
    let mut entries = [text("excluded-x".as_bytes()); 32];
    entries[31] = text("medicine-a".as_bytes());
    check(in_list(text("medicine-a".as_bytes()), entries), true, false);
    let full = text([97; 128]);
    let mut changed = [97; 128];
    changed[127] = 98;
    check(value_compare(full, full, EQ), true, false);
    check(value_compare(full, text(changed), EQ), false, false);
    let long = text(
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!!".as_bytes(),
    );
    check(value_compare(long, long, EQ), true, false);
    check(
        value_compare(text([97, 0, 98]), text([97, 0, 99]), EQ),
        false,
        false,
    );
}
#[test]
fn target_3_recycled() {
    check(
        recycled(number(350000000000000000, true), integer(3), integer(3)),
        true,
        false,
    );
    check(
        recycled(number(290000000000000000, true), integer(3), integer(3)),
        false,
        false,
    );
    check(
        recycled(number(350000000000000000, true), integer(2), integer(3)),
        false,
        false,
    );
    check(
        recycled(number(350000000000000000, true), integer(3), integer(4)),
        false,
        false,
    );
    check(recycled(missing(), integer(3), integer(3)), false, true);
    check(in_list(iri(2), [iri(1), iri(2), iri(3)]), true, false);
    check(in_list(iri(4), [iri(1), iri(2), iri(3)]), false, false);
    check(in_list(missing(), [iri(1), iri(2), iri(3)]), false, true);
}
#[test]
fn target_sum_having() {
    check(totals(integer(500), integer(3)), true, false);
    check(totals(integer(499), integer(3)), false, false);
    check(totals(integer(500), integer(2)), false, false);
    check(totals(missing(), integer(3)), false, true);
    let ratio = |r, t| numeric_compare(
        binary(as_numeric(r), as_numeric(integer(10)), MUL),
        binary(as_numeric(t), as_numeric(integer(3)), MUL),
        GE,
    );
    check(ratio(integer(150), integer(500)), true, false);
    check(ratio(integer(149), integer(500)), false, false);
    check(ratio(missing(), integer(500)), false, true);
    check(
        ratio(integer(281474976710656), integer(281474976710656)),
        true,
        false,
    );
}
#[test(should_fail_with = "forced error low")]
fn target_1a_error_low() {
    let (_, e) = stated(missing(), integer(0), integer(0), boolean(true));
    assert(!e, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn target_1b_error_low() {
    let (_, e) = closed(
        integer(12),
        integer(0),
        missing(),
        date(20770),
        truth(true, false),
    );
    assert(!e, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn target_2_error_low() {
    let (_, e) = trial(
        integer(45),
        missing(),
        date(20758),
        date(20770),
        truth(true, false),
    );
    assert(!e, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn target_3_error_low() {
    let (_, e) = recycled(missing(), integer(3), integer(3));
    assert(!e, "forced error low");
}
#[test(should_fail_with = "forced error low")]
fn target_sum_error_low() {
    let (_, e) = totals(missing(), integer(3));
    assert(!e, "forced error low");
}
