use crate::expr_core::{
    DECIMAL, DOUBLE, exact, FLOAT, floating, INTEGER, Lane, lane, SCALE, StringLane,
};
use crate::expr_date::{DAY, HOURS, MINUTES, MONTH, SECONDS, YEAR};
use crate::expr_date as date_fn;
use crate::expr_numeric::{ABS, CEIL, FLOOR, ROUND};
use crate::expr_numeric as numeric_fn;
use crate::expr_term as term_fn;
use crate::expr_term_contract::{
    CheckedLexicalOpening, DT_BYTE, DT_DATE, DT_DATETIME, DT_DECIMAL, DT_DOUBLE, DT_FLOAT,
    DT_INTEGER, DT_LANGSTRING, DT_STRING, DT_UNSIGNEDLONG, KIND_BLANK_NODE, KIND_IRI, KIND_LITERAL,
    LexicalOpening, TermCell,
};
use std::collections::bounded_vec::BoundedVec;

// Static semantic fixtures only. The production opening has no implementation
// here; the suite-backed integration is exercised by the target-query harness.
struct Fixture {
    value: LexicalOpening<64, 128, 64>,
}
impl CheckedLexicalOpening<64, 128, 64> for Fixture {
    fn open_lexical(self) -> LexicalOpening<64, 128, 64> {
        self.value
    }
}
fn bytes<let L: u32, let N: u32>(s: [u8; N]) -> BoundedVec<u8, L> {
    let mut out = BoundedVec::new();
    for b in s {
        out.push(b);
    }
    out
}
fn fixture<let N: u32>(kind: u8, dt: Field, s: [u8; N]) -> Fixture {
    Fixture {
        value: LexicalOpening {
            cell: TermCell {
                bound: true,
                kind,
                enc: 1,
                key_class: 0,
                key: 0,
                dt,
                lang: 0,
                credential: 0,
            },
            lexical: bytes(s),
            datatype: BoundedVec::new(),
            language: BoundedVec::new(),
        },
    }
}
fn literal<let N: u32>(s: [u8; N], dt: Field) -> Fixture {
    fixture(KIND_LITERAL, dt, s)
}
fn plain<let N: u32>(s: [u8; N]) -> Fixture {
    literal(s, DT_STRING)
}
fn missing() -> Fixture {
    Fixture {
        value: LexicalOpening {
            cell: crate::expr_to_term::unbound(),
            lexical: BoundedVec::new(),
            datatype: BoundedVec::new(),
            language: BoundedVec::new(),
        },
    }
}
fn text<let N: u32>(s: [u8; N]) -> StringLane<64> {
    term_fn::text::<64, N>(BoundedVec::from_array(s)).value
}
fn check_bool(a: (Lane<bool>, bool), value: bool, error: bool) {
    let (v, e) = a;
    assert_eq(v.value, value);
    assert_eq(e, error);
    assert(!v.capacity);
}
fn check_text<let L: u32, let N: u32>(value: StringLane<L>, s: [u8; N]) {
    assert_eq(value.len, N);
    for i in 0..L {
        assert_eq(value.bytes[i], if i < N { s[i] } else { 0 });
    }
}
#[test]
fn term_kind_and_unbound() {
    check_bool(
        term_fn::is_iri((lane(fixture(KIND_IRI, 0, "urn:x".as_bytes())), false)),
        true,
        false,
    );
    check_bool(
        term_fn::is_blank((lane(fixture(KIND_BLANK_NODE, 0, "b".as_bytes())), false)),
        true,
        false,
    );
    check_bool(
        term_fn::is_literal((lane(plain("x".as_bytes())), false)),
        true,
        false,
    );
    check_bool(
        term_fn::is_iri((lane(plain("x".as_bytes())), false)),
        false,
        false,
    );
    check_bool(
        term_fn::is_blank((lane(plain("x".as_bytes())), false)),
        false,
        false,
    );
    check_bool(
        term_fn::is_literal((lane(fixture(KIND_IRI, 0, "urn:x".as_bytes())), false)),
        false,
        false,
    );
    check_bool(term_fn::is_iri((lane(missing()), false)), false, true);
}
#[test]
fn term_lexical_and_metadata() {
    let (v, e) = term_fn::str_value((lane(literal("+005".as_bytes(), DT_INTEGER)), false));
    assert(!e);
    check_text(v.value, "+005".as_bytes());
    let (v, e) = term_fn::str_value((lane(fixture(KIND_IRI, 0, "urn:x".as_bytes())), false));
    assert(!e);
    check_text(v.value, "urn:x".as_bytes());
    let (_, e) = term_fn::str_value((lane(fixture(KIND_BLANK_NODE, 0, "b".as_bytes())), false));
    assert(e);
    let (v, e) = term_fn::lang((lane(plain("chat".as_bytes())), false));
    assert(!e);
    check_text(v.value, "".as_bytes());
    let mut a = literal("chat".as_bytes(), DT_LANGSTRING);
    a.value.language = bytes("fr".as_bytes());
    let (v, e) = term_fn::lang((lane(a), false));
    assert(!e);
    check_text(v.value, "fr".as_bytes());
    let mut a = plain("x".as_bytes());
    a.value.datatype = bytes("http://www.w3.org/2001/XMLSchema#string".as_bytes());
    let (v, e) = term_fn::datatype((lane(a), false));
    assert(!e);
    assert_eq(v.value.kind, KIND_IRI);
    check_text(
        v.value.lexical,
        "http://www.w3.org/2001/XMLSchema#string".as_bytes(),
    );
    let (_, e) = term_fn::lang((lane(fixture(KIND_IRI, 0, "urn:x".as_bytes())), false));
    assert(e);
    let (_, e) = term_fn::datatype((lane(fixture(KIND_IRI, 0, "urn:x".as_bytes())), false));
    assert(e);
}
#[test]
fn numeric_classification_lexical_facets() {
    check_bool(
        term_fn::is_numeric((lane(literal("123".as_bytes(), DT_INTEGER)), false)),
        true,
        false,
    );
    check_bool(
        term_fn::is_numeric((
            lane(literal(
                "999999999999999999999999999999999999999999999999".as_bytes(),
                DT_INTEGER,
            )), false,
        )),
        true,
        false,
    );
    check_bool(
        term_fn::is_numeric((
            lane(
                literal(
                    "0.00000000000000000000000000000000001".as_bytes(),
                    DT_DECIMAL,
                ),
            ), false,
        )),
        true,
        false,
    );
    check_bool(
        term_fn::is_numeric((lane(literal(" \t+12\r\n".as_bytes(), DT_INTEGER)), false)),
        true,
        false,
    );
    check_bool(
        term_fn::is_numeric((lane(literal("bad".as_bytes(), DT_INTEGER)), false)),
        false,
        false,
    );
    check_bool(
        term_fn::is_numeric((lane(literal("5.0".as_bytes(), DT_INTEGER)), false)),
        false,
        false,
    );
    check_bool(
        term_fn::is_numeric((lane(literal("1200".as_bytes(), DT_BYTE)), false)),
        false,
        false,
    );
    check_bool(
        term_fn::is_numeric((lane(literal("-128".as_bytes(), DT_BYTE)), false)),
        true,
        false,
    );
    check_bool(
        term_fn::is_numeric((
            lane(literal("18446744073709551615".as_bytes(), DT_UNSIGNEDLONG)), false,
        )),
        true,
        false,
    );
    check_bool(
        term_fn::is_numeric((
            lane(literal("18446744073709551616".as_bytes(), DT_UNSIGNEDLONG)), false,
        )),
        false,
        false,
    );
    check_bool(
        term_fn::is_numeric((lane(literal("-0".as_bytes(), DT_UNSIGNEDLONG)), false)),
        true,
        false,
    );
    check_bool(
        term_fn::is_numeric((lane(literal("NaN".as_bytes(), DT_FLOAT)), false)),
        true,
        false,
    );
    check_bool(
        term_fn::is_numeric((lane(literal("-INF".as_bytes(), DT_DOUBLE)), false)),
        true,
        false,
    );
    check_bool(
        term_fn::is_numeric((lane(literal("1e+2".as_bytes(), DT_DOUBLE)), false)),
        true,
        false,
    );
    check_bool(
        term_fn::is_numeric((lane(literal("1e+".as_bytes(), DT_DOUBLE)), false)),
        false,
        false,
    );
    check_bool(
        term_fn::is_numeric((lane(plain("12".as_bytes())), false)),
        false,
        false,
    );
}
#[test]
fn language_matching_examples_and_nul() {
    for tag in [text("en".as_bytes()), text("en-US".as_bytes()), text("EN-us".as_bytes())] {
        check_bool(
            term_fn::lang_matches((lane(tag), false), (lane(text("en".as_bytes())), false)),
            true,
            false,
        );
    }
    check_bool(
        term_fn::lang_matches(
            (lane(text("eng".as_bytes())), false),
            (lane(text("en".as_bytes())), false),
        ),
        false,
        false,
    );
    check_bool(
        term_fn::lang_matches(
            (lane(text("".as_bytes())), false),
            (lane(text("*".as_bytes())), false),
        ),
        false,
        false,
    );
    check_bool(
        term_fn::lang_matches(
            (lane(text("en".as_bytes())), false),
            (lane(text("".as_bytes())), false),
        ),
        false,
        false,
    );
    check_bool(
        term_fn::lang_matches(
            (lane(text([101, 110, 0])), false),
            (lane(text("en".as_bytes())), false),
        ),
        false,
        false,
    );
    let mut tagged = text("en".as_bytes());
    tagged.lang = 1;
    check_bool(
        term_fn::lang_matches((lane(tagged), false), (lane(text("en".as_bytes())), false)),
        false,
        true,
    );
}
#[test]
fn term_constructor_examples_and_errors() {
    let (v, e) = term_fn::strdt(
        (lane(plain("123".as_bytes())), false),
        (
            lane(
                fixture(
                    KIND_IRI,
                    0,
                    "http://www.w3.org/2001/XMLSchema#integer".as_bytes(),
                ),
            ), false,
        ),
    );
    assert(!e);
    assert(!v.capacity);
    check_text(v.value.lexical, "123".as_bytes());
    check_text(
        v.value.datatype,
        "http://www.w3.org/2001/XMLSchema#integer".as_bytes(),
    );
    let (_, e) = term_fn::strdt(
        (lane(plain("123".as_bytes())), false),
        (lane(plain("urn:dt".as_bytes())), false),
    );
    assert(e);
    let (v, e) = term_fn::strlang(
        (lane(plain("chat".as_bytes())), false),
        (lane(plain("EN-us".as_bytes())), false),
    );
    assert(!e);
    assert(!v.capacity);
    check_text(v.value.language, "en-us".as_bytes());
    let (_, e) = term_fn::strlang(
        (lane(plain("chat".as_bytes())), false),
        (lane(plain("".as_bytes())), false),
    );
    assert(e);
    let (_, e) = term_fn::strlang(
        (lane(plain("chat".as_bytes())), false),
        (lane(plain("en--us".as_bytes())), false),
    );
    assert(e);
    let (_, e) = term_fn::strlang(
        (lane(plain("chat".as_bytes())), false),
        (lane(plain("-en".as_bytes())), false),
    );
    assert(e);
    let mut tagged = plain("chat".as_bytes());
    tagged.value.language = bytes("en".as_bytes());
    tagged.value.cell.dt = DT_LANGSTRING;
    let (_, e) = term_fn::strlang((lane(tagged), false), (lane(plain("en".as_bytes())), false));
    assert(e);
}
struct ShortFixture {
    value: LexicalOpening<32, 128, 64>,
}
impl CheckedLexicalOpening<32, 128, 64> for ShortFixture {
    fn open_lexical(self) -> LexicalOpening<32, 128, 64> {
        self.value
    }
}
fn short<let N: u32>(kind: u8, dt: Field, s: [u8; N]) -> ShortFixture {
    ShortFixture {
        value: LexicalOpening {
            cell: TermCell {
                bound: true,
                kind,
                enc: 1,
                key_class: 0,
                key: 0,
                dt,
                lang: 0,
                credential: 0,
            },
            lexical: bytes(s),
            datatype: BoundedVec::new(),
            language: BoundedVec::new(),
        },
    }
}
fn small_text<let N: u32>(s: [u8; N]) -> StringLane<32> {
    term_fn::text::<32, N>(BoundedVec::from_array(s)).value
}
#[test]
fn iri_resolution_examples_and_capacity() {
    let base = small_text("http://example.org/a/b".as_bytes());
    let (v, e) = term_fn::iri(
        (lane(short(KIND_LITERAL, DT_STRING, "../c".as_bytes())), false),
        base,
    );
    assert(!e);
    assert(!v.capacity);
    check_text(v.value.lexical, "http://example.org/c".as_bytes());
    let (v, e) = term_fn::iri(
        (lane(short(KIND_LITERAL, DT_STRING, "?q".as_bytes())), false),
        base,
    );
    assert(!e);
    check_text(v.value.lexical, "http://example.org/a/b?q".as_bytes());
    let (v, e) = term_fn::iri(
        (lane(short(KIND_IRI, 0, "http://example.org/a/../b".as_bytes())), false),
        small_text("".as_bytes()),
    );
    assert(!e);
    check_text(v.value.lexical, "http://example.org/a/../b".as_bytes());
    let (_, e) = term_fn::iri(
        (lane(short(KIND_LITERAL, DT_STRING, "bad space".as_bytes())), false),
        base,
    );
    assert(e);
    let (_, e) = term_fn::iri(
        (lane(short(KIND_LITERAL, DT_STRING, "bad%XZ".as_bytes())), false),
        base,
    );
    assert(e);
    let (_, e) = term_fn::iri(
        (lane(short(KIND_LITERAL, DT_STRING, "http://host:no".as_bytes())), false),
        base,
    );
    assert(e);
    let (v, e) = term_fn::iri(
        (lane(short(KIND_LITERAL, DT_STRING, "http://[::1]/".as_bytes())), false),
        base,
    );
    assert(!e);
    assert(v.capacity);
    let (v, e) = term_fn::iri(
        (lane(short(KIND_LITERAL, DT_STRING, "é".as_bytes())), false),
        base,
    );
    assert(!e);
    assert(v.capacity);
}
fn check_number(
    a: (Lane<crate::expr_core::Numeric>, bool),
    ty: u8,
    negative: bool,
    magnitude: Field,
    bits: u64,
) {
    let (v, e) = a;
    assert(!e);
    assert(!v.capacity);
    assert_eq(v.value.ty, ty);
    assert_eq(v.value.negative, negative);
    assert_eq(v.value.magnitude, magnitude);
    assert_eq(v.value.bits, bits);
}
#[test]
fn numeric_exact_functions_and_boundaries() {
    check_number(
        numeric_fn::numeric((lane(exact(INTEGER, true, 42 * SCALE)), false), ABS),
        INTEGER,
        false,
        42 * SCALE,
        0,
    );
    check_number(
        numeric_fn::numeric((lane(exact(INTEGER, true, 42 * SCALE)), false), ROUND),
        INTEGER,
        true,
        42 * SCALE,
        0,
    );
    check_number(
        numeric_fn::numeric((lane(exact(INTEGER, true, 42 * SCALE)), false), CEIL),
        INTEGER,
        true,
        42 * SCALE,
        0,
    );
    check_number(
        numeric_fn::numeric((lane(exact(INTEGER, true, 42 * SCALE)), false), FLOOR),
        INTEGER,
        true,
        42 * SCALE,
        0,
    );
    check_number(
        numeric_fn::numeric(
            (lane(exact(DECIMAL, true, 2500000000000000000)), false),
            ABS,
        ),
        DECIMAL,
        false,
        2500000000000000000,
        0,
    );
    check_number(
        numeric_fn::numeric(
            (lane(exact(DECIMAL, true, 2500000000000000000)), false),
            ROUND,
        ),
        DECIMAL,
        true,
        2 * SCALE,
        0,
    );
    check_number(
        numeric_fn::numeric(
            (lane(exact(DECIMAL, false, 2500000000000000000)), false),
            ROUND,
        ),
        DECIMAL,
        false,
        3 * SCALE,
        0,
    );
    check_number(
        numeric_fn::numeric(
            (lane(exact(DECIMAL, true, 2500000000000000000)), false),
            CEIL,
        ),
        DECIMAL,
        true,
        2 * SCALE,
        0,
    );
    check_number(
        numeric_fn::numeric(
            (lane(exact(DECIMAL, true, 2500000000000000000)), false),
            FLOOR,
        ),
        DECIMAL,
        true,
        3 * SCALE,
        0,
    );
    check_number(
        numeric_fn::numeric((lane(exact(DECIMAL, true, 1)), false), CEIL),
        DECIMAL,
        false,
        0,
        0,
    );
    check_number(
        numeric_fn::numeric((lane(exact(DECIMAL, true, 1)), false), FLOOR),
        DECIMAL,
        true,
        SCALE,
        0,
    );
    let (v, e) = numeric_fn::numeric(
        (lane(exact(DECIMAL, false, crate::expr_core::EXACT_LIMIT - 1)), false),
        CEIL,
    );
    assert(!e);
    assert(v.capacity);
    assert_eq(v.value.magnitude, 0);
    let v = term_fn::text::<1, 2>(BoundedVec::from_array("ab".as_bytes()));
    assert(v.capacity);
    assert_eq(v.value.len, 0);
    assert_eq(v.value.bytes[0], 0);
}
#[test]
fn numeric_ieee_functions_special_values() {
    for function in [ABS, ROUND, CEIL, FLOOR] {
        check_number(
            numeric_fn::numeric((lane(floating(FLOAT, 0x7fc00000)), false), function),
            FLOAT,
            false,
            0,
            0x7fc00000,
        );
        check_number(
            numeric_fn::numeric(
                (lane(floating(DOUBLE, 0x7ff8000000000000)), false),
                function,
            ),
            DOUBLE,
            false,
            0,
            0x7ff8000000000000,
        );
    }
    check_number(
        numeric_fn::numeric((lane(floating(FLOAT, 0xbe800000)), false), ABS),
        FLOAT,
        false,
        0,
        0x3e800000,
    );
    check_number(
        numeric_fn::numeric((lane(floating(FLOAT, 0xbe800000)), false), ROUND),
        FLOAT,
        false,
        0,
        0x80000000,
    );
    check_number(
        numeric_fn::numeric((lane(floating(FLOAT, 0xbe800000)), false), CEIL),
        FLOAT,
        false,
        0,
        0x80000000,
    );
    check_number(
        numeric_fn::numeric((lane(floating(FLOAT, 0xbe800000)), false), FLOOR),
        FLOAT,
        false,
        0,
        0xbf800000,
    );
    check_number(
        numeric_fn::numeric((lane(floating(DOUBLE, 0xbfd0000000000000)), false), ROUND),
        DOUBLE,
        false,
        0,
        0x8000000000000000,
    );
    check_number(
        numeric_fn::numeric((lane(floating(DOUBLE, 0xbfd0000000000000)), false), CEIL),
        DOUBLE,
        false,
        0,
        0x8000000000000000,
    );
    check_number(
        numeric_fn::numeric((lane(floating(DOUBLE, 0xbfd0000000000000)), false), FLOOR),
        DOUBLE,
        false,
        0,
        0xbff0000000000000,
    );
    check_number(
        numeric_fn::numeric((lane(floating(DOUBLE, 0xfff0000000000000)), false), ABS),
        DOUBLE,
        false,
        0,
        0x7ff0000000000000,
    );
    check_number(
        numeric_fn::numeric((lane(floating(DOUBLE, 0xfff0000000000000)), false), ROUND),
        DOUBLE,
        false,
        0,
        0xfff0000000000000,
    );
}
fn datetime<let N: u32>(s: [u8; N]) -> (Lane<date_fn::TemporalValue>, bool) {
    date_fn::read_date((lane(literal(s, DT_DATETIME)), false))
}
#[test]
fn date_component_recommendation_examples() {
    let a = datetime("2011-01-10T14:45:13.815-05:00".as_bytes());
    let cases: [(u8, u32); 5] = [(YEAR, 2011), (MONTH, 1), (DAY, 10), (HOURS, 14), (MINUTES, 45)];
    for pair in cases {
        let (function, expected) = pair;
        check_number(
            date_fn::component(a, function),
            INTEGER,
            false,
            expected as Field * SCALE,
            0,
        );
    }
    check_number(
        date_fn::component(a, SECONDS),
        DECIMAL,
        false,
        13815000000000000000,
        0,
    );
    let (v, e) = date_fn::timezone(a);
    assert(!e);
    assert(v.value.negative);
    assert_eq(v.value.microseconds, 18000000000);
    let (v, e) = date_fn::tz(a);
    assert(!e);
    check_text(v.value, "-05:00".as_bytes());
    let (v, e) = date_fn::tz(datetime("2011-01-10T14:45:13+00:00".as_bytes()));
    assert(!e);
    check_text(v.value, "+00:00".as_bytes());
    let (v, e) = date_fn::tz(datetime("2011-01-10T14:45:13Z".as_bytes()));
    assert(!e);
    check_text(v.value, "Z".as_bytes());
    let (v, e) = date_fn::tz(datetime("2011-01-10T14:45:13".as_bytes()));
    assert(!e);
    check_text(v.value, "".as_bytes());
    let (_, e) = date_fn::timezone(datetime("2011-01-10T14:45:13".as_bytes()));
    assert(e);
}
#[test]
fn date_boundaries_and_invalid_inputs() {
    let a = datetime("1999-12-31T24:00:00-05:00".as_bytes());
    check_number(date_fn::component(a, YEAR), INTEGER, false, 2000 * SCALE, 0);
    check_number(date_fn::component(a, MONTH), INTEGER, false, SCALE, 0);
    check_number(date_fn::component(a, DAY), INTEGER, false, SCALE, 0);
    check_number(date_fn::component(a, HOURS), INTEGER, false, 0, 0);
    let a = datetime("2024-02-29T24:00:00Z".as_bytes());
    check_number(date_fn::component(a, MONTH), INTEGER, false, 3 * SCALE, 0);
    check_number(date_fn::component(a, DAY), INTEGER, false, SCALE, 0);
    let a = datetime("2024-01-01T00:00:00.000000000000000001Z".as_bytes());
    check_number(date_fn::component(a, SECONDS), DECIMAL, false, 1, 0);
    let (_, e) = datetime("2023-02-29T00:00:00Z".as_bytes());
    assert(e);
    let (_, e) = datetime("2024-13-01T00:00:00Z".as_bytes());
    assert(e);
    let (_, e) = datetime("2024-01-01T25:00:00Z".as_bytes());
    assert(e);
    let (_, e) = datetime("2024-01-01T24:00:00.1Z".as_bytes());
    assert(e);
    let (_, e) = datetime("2024-01-01T00:00:00+14:01".as_bytes());
    assert(e);
    let (_, e) = datetime("0000-01-01T00:00:00Z".as_bytes());
    assert(e);
    let (v, e) = datetime("-0001-01-01T00:00:00Z".as_bytes());
    assert(!e);
    assert(v.capacity);
    let (v, e) = datetime("10000-01-01T00:00:00Z".as_bytes());
    assert(!e);
    assert(v.capacity);
    let (v, e) = datetime("2024-01-01T00:00:00.0000000000000000001Z".as_bytes());
    assert(!e);
    assert(v.capacity);
    let (_, e) = date_fn::read_date((lane(plain("2024-01-01T00:00:00Z".as_bytes())), false));
    assert(e);
    let (_, e) = date_fn::component(
        date_fn::read_date((lane(literal("2024-01-01".as_bytes(), DT_DATE)), false)),
        YEAR,
    );
    assert(e);
}
#[test]
fn date_comparison_fraction_and_timezone() {
    let a = datetime("2011-01-10T13:00:00.5+01:00".as_bytes());
    let b = datetime("2011-01-10T12:00:00.5Z".as_bytes());
    check_bool(date_fn::compare(a, b, 0), true, false);
    check_bool(
        date_fn::compare(
            datetime("2011-01-10T12:00:00.000000000000000001Z".as_bytes()),
            datetime("2011-01-10T12:00:00Z".as_bytes()),
            3,
        ),
        true,
        false,
    );
    let (v, e) = date_fn::compare(datetime("2011-01-10T12:00:00".as_bytes()), b, 0);
    assert(!e);
    assert(v.capacity);
}

#[test(should_fail_with = "forced error low")]
fn is_iri_error_low() {
    let (_, e) = term_fn::is_iri((lane(missing()), false));
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn is_blank_error_low() {
    let (_, e) = term_fn::is_blank((lane(missing()), false));
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn is_literal_error_low() {
    let (_, e) = term_fn::is_literal((lane(missing()), false));
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn is_numeric_error_low() {
    let (_, e) = term_fn::is_numeric((lane(missing()), false));
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn str_error_low() {
    let (_, e) = term_fn::str_value((lane(missing()), false));
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn lang_error_low() {
    let (_, e) = term_fn::lang((lane(missing()), false));
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn datatype_error_low() {
    let (_, e) = term_fn::datatype((lane(missing()), false));
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn lang_matches_error_low() {
    let (_, e) = term_fn::lang_matches(
        (lane(text("en".as_bytes())), true),
        (lane(text("en".as_bytes())), false),
    );
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn iri_error_low() {
    let (_, e) = term_fn::iri(
        (lane(plain("bad space".as_bytes())), false),
        text("http://example.org/".as_bytes()),
    );
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn strdt_error_low() {
    let (_, e) = term_fn::strdt(
        (lane(missing()), false),
        (lane(fixture(KIND_IRI, 0, "urn:dt".as_bytes())), false),
    );
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn strlang_error_low() {
    let (_, e) = term_fn::strlang(
        (lane(missing()), false),
        (lane(plain("en".as_bytes())), false),
    );
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn year_error_low() {
    let (_, e) = date_fn::component(datetime("2023-02-29T00:00:00Z".as_bytes()), YEAR);
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn month_error_low() {
    let (_, e) = date_fn::component(datetime("2023-02-29T00:00:00Z".as_bytes()), MONTH);
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn day_error_low() {
    let (_, e) = date_fn::component(datetime("2023-02-29T00:00:00Z".as_bytes()), DAY);
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn hours_error_low() {
    let (_, e) = date_fn::component(datetime("2023-02-29T00:00:00Z".as_bytes()), HOURS);
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn minutes_error_low() {
    let (_, e) = date_fn::component(datetime("2023-02-29T00:00:00Z".as_bytes()), MINUTES);
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn seconds_error_low() {
    let (_, e) = date_fn::component(datetime("2023-02-29T00:00:00Z".as_bytes()), SECONDS);
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn timezone_error_low() {
    let (_, e) = date_fn::timezone(datetime("2024-01-01T00:00:00".as_bytes()));
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn tz_error_low() {
    let (_, e) = date_fn::tz(date_fn::read_date((lane(missing()), false)));
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn date_compare_error_low() {
    let (_, e) = date_fn::compare(
        datetime("bad".as_bytes()),
        datetime("2024-01-01T00:00:00Z".as_bytes()),
        0,
    );
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn abs_error_low() {
    let (_, e) = numeric_fn::numeric((lane(exact(INTEGER, false, 0)), true), ABS);
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn round_error_low() {
    let (_, e) = numeric_fn::numeric((lane(exact(INTEGER, false, 0)), true), ROUND);
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn ceil_error_low() {
    let (_, e) = numeric_fn::numeric((lane(exact(INTEGER, false, 0)), true), CEIL);
    assert(!e, "forced error low");
}

#[test(should_fail_with = "forced error low")]
fn floor_error_low() {
    let (_, e) = numeric_fn::numeric((lane(exact(INTEGER, false, 0)), true), FLOOR);
    assert(!e, "forced error low");
}

#[test]
fn date_xml_whitespace_and_language_well_formedness() {
    check_number(
        date_fn::component(datetime(" \t2024-01-01T00:00:00Z\r\n".as_bytes()), YEAR),
        INTEGER,
        false,
        2024 * SCALE,
        0,
    );
    for tag in [
        text("en-US".as_bytes()),
        text("zh-cmn-Hans-CN".as_bytes()),
        text("x-private".as_bytes()),
        text("i-klingon".as_bytes()),
        text("en-a-foo-x-bar".as_bytes()),
    ] {
        let mut raw: BoundedVec<u8, 64> = BoundedVec::new();
        for i in 0..64 {
            if i < tag.len {
                raw.push(tag.bytes[i]);
            }
        }
        assert(crate::expr_language::valid(raw));
    }
    for tag in [
        text("abcd-USA".as_bytes()),
        text("en-a".as_bytes()),
        text("en-a-foo-a-bar".as_bytes()),
        text("de-1901-1901".as_bytes()),
        text("en-123456789".as_bytes()),
    ] {
        let mut raw: BoundedVec<u8, 64> = BoundedVec::new();
        for i in 0..64 {
            if i < tag.len {
                raw.push(tag.bytes[i]);
            }
        }
        assert(!crate::expr_language::valid(raw));
    }
}
#[test]
fn capacity_survives_metadata_and_temporal_errors_stay_errors() {
    let input = (Lane { value: plain("x".as_bytes()), capacity: true }, false);
    let (v, e) = term_fn::str_value(input);
    assert(!e);
    assert(v.capacity);
    let (v, e) = term_fn::is_numeric(input);
    assert(!e);
    assert(v.capacity);
    let (v, e) = term_fn::lang(input);
    assert(!e);
    assert(v.capacity);
    let (v, e) = term_fn::datatype(input);
    assert(!e);
    assert(v.capacity);
    let (v, e) = term_fn::strlang(input, (lane(plain("en".as_bytes())), false));
    assert(!e);
    assert(v.capacity);
    check_bool(
        date_fn::compare(
            datetime("bad".as_bytes()),
            datetime("2024-01-01T00:00:00Z".as_bytes()),
            0,
        ),
        false,
        true,
    );
}
#[test]
fn computed_numeric_datatype() {
    let (v, e) = term_fn::numeric_datatype((lane(floating(FLOAT, 0x3f800000)), false));
    assert(!e);
    check_text(
        v.value.lexical,
        "http://www.w3.org/2001/XMLSchema#float".as_bytes(),
    );
    let (v, e) = term_fn::numeric_datatype((lane(exact(DECIMAL, false, 3 * SCALE)), false));
    assert(!e);
    check_text(
        v.value.lexical,
        "http://www.w3.org/2001/XMLSchema#decimal".as_bytes(),
    );
}
#[test]
fn opened_language_literal_remains_a_string() {
    let mut a = literal("chat".as_bytes(), DT_LANGSTRING);
    a.value.language = bytes("fr".as_bytes());
    a.value.cell.lang = 77;
    let (v, e) = crate::expr_open::variable(a);
    assert(!e);
    assert(!v.capacity);
    assert_eq(v.value.tag, crate::expr_core::STRING);
    assert_eq(v.value.string.lang, 77);
    check_text(v.value.string, "chat".as_bytes());
    check_bool(crate::expr_core::ebv((v, false)), true, false);
}
