//! Behavioural tests through the public API. The float and zero cases reproduce the
//! datatype content of the W3C OWL 2 test cases `Datatype-Float-Discrete-001` and
//! `Plus and Minus Zero are Distinct`.

use super::*;
use crate::owl2dl::datatypes::{builtin, in_value_space, parse_value, DateTimeValue, Rational};
use DataRangeExpr as E;

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

fn iri(dt: &str) -> String {
    match dt.split_once(':') {
        Some(("owl", l)) => format!("http://www.w3.org/2002/07/owl#{l}"),
        Some(("rdf", l)) => format!("http://www.w3.org/1999/02/22-rdf-syntax-ns#{l}"),
        Some(("rdfs", l)) => format!("http://www.w3.org/2000/01/rdf-schema#{l}"),
        _ => format!("{XSD}{dt}"),
    }
}

fn b(dt: &str) -> Builtin {
    builtin(&iri(dt)).unwrap()
}

fn v(lex: &str, dt: &str) -> Value {
    parse_value(lex, &iri(dt), None).unwrap()
}

fn lang(s: &str, tag: &str) -> Value {
    parse_value(s, &iri("rdf:langString"), Some(tag)).unwrap()
}

fn s(x: &str) -> Value {
    Value::String(x.into())
}

fn dt(name: &str) -> E {
    E::Datatype(b(name))
}

fn restrict(name: &str, facets: &[(&str, Value)]) -> E {
    let f = |n: &str| match n {
        "minInclusive" => Facet::MinInclusive,
        "maxInclusive" => Facet::MaxInclusive,
        "minExclusive" => Facet::MinExclusive,
        "maxExclusive" => Facet::MaxExclusive,
        "length" => Facet::Length,
        "minLength" => Facet::MinLength,
        "maxLength" => Facet::MaxLength,
        "pattern" => Facet::Pattern,
        "langRange" => Facet::LangRange,
        _ => panic!("{n}"),
    };
    E::Restriction(
        b(name),
        facets.iter().map(|(n, x)| (f(n), x.clone())).collect(),
    )
}

fn one_of<const N: usize>(vs: [Value; N]) -> E {
    E::OneOf(vs.into())
}

fn and(a: E, b: E) -> E {
    E::And(vec![a, b])
}

fn or(a: E, b: E) -> E {
    E::Or(vec![a, b])
}

fn not(a: E) -> E {
    E::Not(Box::new(a))
}

fn subset(a: E, b: E) -> Tri {
    is_empty(&and(a, not(b)))
}

#[test]
fn float_ranges_are_discrete() {
    // Datatype-Float-Discrete-001: no float lies strictly between 0 and the least
    // positive subnormal.
    let gap = restrict(
        "float",
        &[
            ("minExclusive", v("0.0", "float")),
            ("maxExclusive", v("1.401298464324817e-45", "float")),
        ],
    );
    assert_eq!(is_empty(&gap), Tri::Yes);
    let closed = restrict(
        "float",
        &[
            ("minExclusive", v("0.0", "float")),
            ("maxInclusive", v("1.401298464324817e-45", "float")),
        ],
    );
    assert_eq!(is_empty(&closed), Tri::No);
    assert_eq!(has_at_least(&closed, 2), Tri::No);
    let dgap = restrict(
        "double",
        &[
            ("minExclusive", v("0", "double")),
            ("maxExclusive", v("4.9E-324", "double")),
        ],
    );
    assert_eq!(is_empty(&dgap), Tri::Yes);
    assert_eq!(
        is_empty(&restrict("float", &[("minInclusive", v("NaN", "float"))])),
        Tri::Yes
    );
    assert_eq!(
        contains(
            &not(restrict("float", &[("minInclusive", v("-INF", "float"))])),
            &v("NaN", "float")
        ),
        Tri::Yes
    );
    // A bound as large as the largest float still leaves +INF above it.
    let top = restrict("float", &[("minExclusive", v("3.4028235E38", "float"))]);
    assert_eq!(subset(top.clone(), one_of([v("INF", "float")])), Tri::Yes);
    assert_eq!(is_empty(&top), Tri::No);
    // The float value space: 2^32 bit patterns, one NaN.
    let all = (1u64 << 32) - 2 * ((1 << 23) - 1) + 1;
    assert_eq!(has_at_least(&dt("float"), all), Tri::Yes);
    assert_eq!(has_at_least(&dt("float"), all + 1), Tri::No);
}

#[test]
fn zeros_are_equal_but_not_identical() {
    // Plus and Minus Zero are Distinct.
    let (pz, nz) = (v("0.0", "float"), v("-0.0", "float"));
    assert_eq!(
        is_empty(&and(one_of([pz.clone()]), one_of([nz.clone()]))),
        Tri::Yes
    );
    let zero = restrict(
        "float",
        &[("minInclusive", pz.clone()), ("maxInclusive", pz.clone())],
    );
    assert_eq!(contains(&zero, &nz), Tri::Yes);
    assert_eq!(has_at_least(&zero, 2), Tri::Yes);
    assert_eq!(has_at_least(&zero, 3), Tri::No);
    assert_eq!(
        subset(zero.clone(), one_of([pz.clone(), nz.clone()])),
        Tri::Yes
    );
    assert_eq!(subset(zero.clone(), one_of([pz.clone()])), Tri::No);
    let negative = restrict("float", &[("maxExclusive", pz.clone())]);
    assert_eq!(contains(&negative, &nz), Tri::No);
    // Integer zeros are one value.
    assert_eq!(
        contains(&one_of([v("0", "integer")]), &v("-0", "integer")),
        Tri::Yes
    );
}

#[test]
fn reals_are_ordered_exactly_across_layers() {
    let between =
        |d: &str, lo: Value, hi: Value| restrict(d, &[("minExclusive", lo), ("maxExclusive", hi)]);
    let (zero, one) = (v("0", "integer"), v("1", "integer"));
    assert_eq!(
        is_empty(&between("integer", zero.clone(), one.clone())),
        Tri::Yes
    );
    assert_eq!(
        is_empty(&between("decimal", zero.clone(), one.clone())),
        Tri::No
    );
    assert_eq!(
        has_at_least(&between("decimal", zero.clone(), one.clone()), u64::MAX),
        Tri::Yes
    );
    let third = v("1/3", "owl:rational");
    let point = |d: &str| {
        restrict(
            d,
            &[
                ("minInclusive", third.clone()),
                ("maxInclusive", third.clone()),
            ],
        )
    };
    assert_eq!(is_empty(&point("owl:rational")), Tri::No);
    assert_eq!(
        is_empty(&and(point("owl:rational"), dt("decimal"))),
        Tri::Yes,
        "1/3 is not a decimal"
    );
    // owl:real has irrationals in every interval, but not at a rational point.
    assert_eq!(
        is_empty(&and(
            between("owl:real", zero.clone(), one),
            not(dt("owl:rational"))
        )),
        Tri::No
    );
    assert_eq!(
        is_empty(&and(point("owl:real"), not(dt("owl:rational")))),
        Tri::Yes
    );
    // No rounding: a bound a hair above 0.1 still excludes 0.1.
    let tight = restrict("decimal", &[("minExclusive", v("0.1", "decimal"))]);
    assert_eq!(
        contains(
            &tight,
            &v("0.1000000000000000000000000000000001", "decimal")
        ),
        Tri::Yes
    );
    assert_eq!(contains(&tight, &v("0.1", "decimal")), Tri::No);
    assert_eq!(
        subset(
            and(dt("nonNegativeInteger"), dt("nonPositiveInteger")),
            one_of([zero])
        ),
        Tri::Yes
    );
    assert_eq!(
        is_empty(&and(dt("positiveInteger"), dt("negativeInteger"))),
        Tri::Yes
    );
    assert_eq!(
        contains(&one_of([v("1", "integer")]), &v("1.0", "decimal")),
        Tri::Yes
    );
    assert_eq!(
        contains(&one_of([v("1", "integer")]), &v("1", "float")),
        Tri::No
    );
    // Integer ranges count exactly.
    let small = restrict(
        "integer",
        &[
            ("minInclusive", v("-2", "integer")),
            ("maxExclusive", v("3", "integer")),
        ],
    );
    assert_eq!(has_at_least(&small, 5), Tri::Yes);
    assert_eq!(has_at_least(&small, 6), Tri::No);
    assert_eq!(has_at_least(&dt("byte"), 256), Tri::Yes);
    assert_eq!(has_at_least(&dt("byte"), 257), Tri::No);
    assert_eq!(has_at_least(&dt("unsignedLong"), u64::MAX), Tri::Yes);
}

#[test]
fn subsumption_follows_the_datatype_hierarchy() {
    let sub = |a: &str, c: &str| subset(dt(a), dt(c));
    assert_eq!(sub("integer", "decimal"), Tri::Yes);
    assert_eq!(sub("decimal", "owl:rational"), Tri::Yes);
    assert_eq!(sub("owl:rational", "owl:real"), Tri::Yes);
    assert_eq!(sub("owl:real", "owl:rational"), Tri::No);
    assert_eq!(sub("byte", "short"), Tri::Yes);
    assert_eq!(sub("unsignedByte", "byte"), Tri::No);
    assert_eq!(sub("float", "owl:real"), Tri::No);
    assert_eq!(sub("NCName", "Name"), Tri::Yes);
    assert_eq!(sub("Name", "NCName"), Tri::No);
    assert_eq!(sub("token", "normalizedString"), Tri::Yes);
    assert_eq!(sub("language", "token"), Tri::Yes);
    assert_eq!(sub("string", "rdf:PlainLiteral"), Tri::Yes);
    assert_eq!(sub("rdf:langString", "rdf:PlainLiteral"), Tri::Yes);
    assert_eq!(sub("dateTimeStamp", "dateTime"), Tri::Yes);
    assert_eq!(sub("dateTime", "dateTimeStamp"), Tri::No);
    assert_eq!(is_empty(&and(dt("anyURI"), dt("string"))), Tri::Yes);
    assert_eq!(
        is_empty(&and(dt("hexBinary"), dt("base64Binary"))),
        Tri::Yes
    );
    assert_eq!(is_empty(&not(dt("rdfs:Literal"))), Tri::Yes);
    assert_eq!(
        is_empty(&not(or(dt("boolean"), not(dt("boolean"))))),
        Tri::Yes
    );
    assert_eq!(
        is_empty(&and(
            dt("boolean"),
            not(one_of([v("true", "boolean"), v("0", "boolean")]))
        )),
        Tri::Yes
    );
    assert_eq!(has_at_least(&dt("boolean"), 2), Tri::Yes);
    assert_eq!(has_at_least(&dt("boolean"), 3), Tri::No);
}

#[test]
fn string_ranges_combine_patterns_lengths_and_languages() {
    let aa = and(
        restrict("string", &[("length", v("2", "integer"))]),
        restrict("string", &[("pattern", s("a*"))]),
    );
    assert_eq!(subset(aa.clone(), one_of([s("aa")])), Tri::Yes);
    assert_eq!(is_empty(&aa), Tri::No);
    let empty_only = restrict("string", &[("maxLength", v("0", "integer"))]);
    assert_eq!(is_empty(&and(empty_only, not(one_of([s("")])))), Tri::Yes);
    let colon = restrict("string", &[("pattern", s(".*:.*"))]);
    assert_eq!(is_empty(&and(dt("NCName"), colon.clone())), Tri::Yes);
    assert_eq!(is_empty(&and(dt("Name"), colon)), Tri::No);
    let tagged = restrict("rdf:PlainLiteral", &[("langRange", s("*"))]);
    assert_eq!(is_empty(&and(tagged.clone(), dt("string"))), Tri::Yes);
    assert_eq!(
        subset(or(tagged, dt("string")), dt("rdf:PlainLiteral")),
        Tri::Yes
    );
    assert_eq!(subset(dt("rdf:PlainLiteral"), dt("string")), Tri::No);
    let en = restrict("rdf:PlainLiteral", &[("langRange", s("EN"))]);
    assert_eq!(contains(&en, &lang("x", "en-GB")), Tri::Yes);
    assert_eq!(contains(&en, &v("x@EN", "rdf:PlainLiteral")), Tri::Yes);
    assert_eq!(contains(&en, &lang("x", "eng")), Tri::No);
    assert_eq!(contains(&en, &s("x")), Tri::No);
    // Basic filtering (RFC 4647 §3.3.1): the range is a prefix of the tag ending at a
    // subtag boundary. Extended ranges are not language ranges here.
    let de = restrict("rdf:PlainLiteral", &[("langRange", s("de"))]);
    assert_eq!(contains(&de, &lang("x", "de-Latn-DE")), Tri::Yes);
    assert_eq!(contains(&de, &lang("x", "deu")), Tri::No);
    let de_de = restrict("rdf:PlainLiteral", &[("langRange", s("de-DE"))]);
    assert_eq!(contains(&de_de, &lang("x", "de-Latn-DE")), Tri::No);
    assert_eq!(
        is_empty(&restrict(
            "rdf:PlainLiteral",
            &[("langRange", s("de-*-DE"))]
        )),
        Tri::Unknown
    );
    // Tags are well-formed BCP 47 tags: `en-a` has a singleton with no extension subtag.
    assert_eq!(
        contains(
            &dt("rdf:PlainLiteral"),
            &Value::LangString("x".into(), "en-a".into())
        ),
        Tri::No
    );
    assert_eq!(
        contains(&dt("rdf:PlainLiteral"), &lang("x", "en-a-bbb")),
        Tri::Yes
    );
    assert_eq!(
        is_empty(&restrict("rdf:PlainLiteral", &[("langRange", s("a"))])),
        Tri::Yes,
        "no well-formed tag starts with a lone singleton"
    );
    // Length counts characters of the string part, whatever the tag.
    let two = restrict("rdf:PlainLiteral", &[("length", v("2", "integer"))]);
    assert_eq!(contains(&two, &lang("é€", "fr")), Tri::Yes);
    assert_eq!(
        is_empty(&restrict("string", &[("pattern", s(r"\p{IsBasicLatin}"))])),
        Tri::Unknown
    );
    let uri = restrict("anyURI", &[("pattern", s("http://.*"))]);
    assert_eq!(
        contains(&uri, &v("http://example.org/", "anyURI")),
        Tri::Yes
    );
    assert_eq!(contains(&uri, &s("http://example.org/")), Tri::No);
    // Counting walks the automata: two-character strings over {a, b}.
    let ab = restrict("string", &[("pattern", s("[ab]{2}"))]);
    assert_eq!(has_at_least(&ab, 4), Tri::Yes);
    assert_eq!(has_at_least(&ab, 5), Tri::No);
    assert_eq!(
        has_at_least(&restrict("string", &[("pattern", s("a+"))]), u64::MAX),
        Tri::Yes
    );
}

#[test]
fn string_subtypes_agree_with_the_value_spaces() {
    let corpus = [
        "",
        "a",
        "a b",
        "a  b",
        " a",
        "a\tb",
        "a\nb",
        "en",
        "en-GB",
        "en-",
        "toolongtag",
        "x-1",
        "été",
        "a:b",
        ":a",
        "1a",
        "-a",
        "a.b",
        "a·b",
        "_",
        "\u{37E}",
        "\u{2070}",
        "\u{10000}",
        "\u{EFFFF}",
        "\u{F0000}",
    ];
    for kind in [
        "normalizedString",
        "token",
        "language",
        "Name",
        "NCName",
        "NMTOKEN",
    ] {
        for x in corpus {
            let want = Tri::from(in_value_space(&s(x), b(kind)));
            assert_eq!(contains(&dt(kind), &s(x)), want, "{kind} {x:?}");
        }
    }
}

#[test]
fn binary_lengths_count_octets() {
    let empty_hex = restrict("hexBinary", &[("maxLength", v("0", "integer"))]);
    assert_eq!(subset(empty_hex, one_of([v("", "hexBinary")])), Tri::Yes);
    let one = restrict("base64Binary", &[("length", v("1", "integer"))]);
    assert_eq!(contains(&one, &v("QQ==", "base64Binary")), Tri::Yes);
    assert_eq!(contains(&one, &v("QUI=", "base64Binary")), Tri::No);
    assert_eq!(has_at_least(&one, 256), Tri::Yes);
    assert_eq!(has_at_least(&one, 257), Tri::No);
    assert_eq!(
        is_empty(&and(one, not(one_of([v("QQ==", "base64Binary")])))),
        Tri::No
    );
}

#[test]
fn time_instants_compare_on_the_time_line() {
    let midnight = v("2000-01-01T00:00:00Z", "dateTime");
    let from = restrict("dateTime", &[("minInclusive", midnight.clone())]);
    // A value without an offset is above only past the ±14:00 window.
    assert_eq!(
        contains(&from, &v("2000-01-01T10:00:00", "dateTime")),
        Tri::No
    );
    assert_eq!(
        contains(&from, &v("2000-01-01T14:00:00", "dateTime")),
        Tri::No
    );
    assert_eq!(
        contains(&from, &v("2000-01-01T14:00:01", "dateTime")),
        Tri::Yes
    );
    assert_eq!(
        contains(&from, &v("2000-01-01T01:00:00+01:00", "dateTime")),
        Tri::Yes
    );
    // Same instant, different offset: equal, so inside the facet; not identical, so
    // outside the enumeration.
    let same = v("2000-01-01T01:00:00+01:00", "dateTimeStamp");
    assert_eq!(contains(&one_of([midnight.clone()]), &same), Tri::No);
    let instant = restrict(
        "dateTimeStamp",
        &[
            ("minInclusive", midnight.clone()),
            ("maxInclusive", midnight.clone()),
        ],
    );
    assert_eq!(contains(&instant, &same), Tri::Yes);
    assert_eq!(
        is_empty(&and(instant.clone(), not(one_of([midnight, same])))),
        Tri::No
    );
    // One instant has a value for each offset in -14:00..=+14:00.
    assert_eq!(has_at_least(&instant, 1681), Tri::Yes);
    assert_eq!(has_at_least(&instant, 1682), Tri::No);
    let before = restrict(
        "dateTime",
        &[("maxExclusive", v("2000-01-01T00:00:00", "dateTime"))],
    );
    assert_eq!(
        contains(&before, &v("1999-12-31T23:59:59.999", "dateTime")),
        Tri::Yes
    );
    assert_eq!(
        contains(&before, &v("1999-12-31T23:00:00Z", "dateTime")),
        Tri::No
    );
    assert_eq!(
        contains(&before, &v("1999-12-31T09:59:59Z", "dateTime")),
        Tri::Yes
    );
}

#[test]
fn conjunction_counts_distinct_fresh_values() {
    let digit = restrict(
        "integer",
        &[
            ("minInclusive", v("0", "integer")),
            ("maxInclusive", v("9", "integer")),
        ],
    );
    let odd = one_of([
        v("1", "integer"),
        v("3", "integer"),
        v("5", "integer"),
        v("7", "integer"),
        v("9", "integer"),
    ]);
    let taken = [v("0", "integer"), v("2", "integer")];
    // Digits, not odd, not 0 or 2: {4, 6, 8}.
    assert_eq!(
        conjunction_satisfiable(
            std::slice::from_ref(&digit),
            std::slice::from_ref(&odd),
            3,
            &taken
        ),
        Tri::Yes
    );
    assert_eq!(
        conjunction_satisfiable(
            std::slice::from_ref(&digit),
            std::slice::from_ref(&odd),
            4,
            &taken
        ),
        Tri::No
    );
    assert_eq!(
        conjunction_satisfiable(&[digit.clone(), dt("string")], &[], 1, &[]),
        Tri::No
    );
    assert_eq!(conjunction_satisfiable(&[], &[], 0, &[]), Tri::Yes);
    assert_eq!(
        conjunction_satisfiable(&[], &[dt("rdfs:Literal")], 1, &[]),
        Tri::No
    );
    // No positive ranges means rdfs:Literal: the data domain is infinite.
    assert_eq!(conjunction_satisfiable(&[], &[], usize::MAX, &[]), Tri::Yes);
    let many: Vec<Value> = (0..100).map(|i| v(&i.to_string(), "integer")).collect();
    assert_eq!(
        conjunction_satisfiable(&[], &[dt("string")], usize::MAX, &many),
        Tri::Yes
    );
    // An Unknown anywhere makes the whole check Unknown.
    let unsupported = restrict("string", &[("pattern", s(r"\p{IsBasicLatin}"))]);
    assert_eq!(
        conjunction_satisfiable(&[digit], &[unsupported], 1, &[]),
        Tri::Unknown
    );
}

#[test]
fn facets_outside_the_facet_space_answer_unknown() {
    assert_eq!(
        is_empty(&restrict("integer", &[("minInclusive", v("1", "float"))])),
        Tri::Unknown
    );
    assert_eq!(
        is_empty(&restrict("float", &[("minInclusive", v("1", "integer"))])),
        Tri::Unknown
    );
    assert_eq!(
        is_empty(&restrict(
            "boolean",
            &[("minInclusive", v("true", "boolean"))]
        )),
        Tri::Unknown
    );
    assert_eq!(
        is_empty(&restrict("hexBinary", &[("pattern", s("0+"))])),
        Tri::Unknown
    );
    assert_eq!(
        is_empty(&restrict("string", &[("pattern", s("[a-"))])),
        Tri::Unknown
    );
    assert_eq!(
        is_empty(&restrict("string", &[("length", v("-1", "integer"))])),
        Tri::Unknown
    );
    assert_eq!(
        is_empty(&restrict("string", &[("langRange", s("en"))])),
        Tri::Unknown
    );
    assert_eq!(
        is_empty(&restrict("rdf:PlainLiteral", &[("langRange", s("en_GB"))])),
        Tri::Unknown,
        "not a language range"
    );
    // An order bound must be in the restricted datatype's value space (XML Schema 1.1
    // §4.3.7-§4.3.10); owl:real and owl:rational take any real.
    let bounded = |d: &str, x: Value| is_empty(&restrict(d, &[("minInclusive", x)]));
    assert_eq!(bounded("integer", v("1/3", "owl:rational")), Tri::Unknown);
    assert_eq!(bounded("decimal", v("1/3", "owl:rational")), Tri::Unknown);
    assert_eq!(bounded("integer", v("0.9", "decimal")), Tri::Unknown);
    assert_eq!(bounded("byte", v("1000", "integer")), Tri::Unknown);
    assert_eq!(
        bounded("dateTimeStamp", v("2000-01-01T00:00:00", "dateTime")),
        Tri::Unknown
    );
    assert_eq!(
        bounded("decimal", v("1", "integer")),
        Tri::No,
        "1 is a decimal"
    );
    assert_eq!(bounded("owl:real", v("1/3", "owl:rational")), Tri::No);
    assert_eq!(bounded("owl:rational", v("1/3", "owl:rational")), Tri::No);
    assert_eq!(is_empty(&dt("rdf:XMLLiteral")), Tri::No);
    assert_eq!(
        contains(
            &one_of([Value::XmlLiteral("<a/>".into())]),
            &Value::XmlLiteral("<a/>".into())
        ),
        Tri::Yes
    );
}

#[test]
fn strings_are_sequences_of_xml_characters() {
    // OWL 2 §2.3: every character matches XML `Char`.
    let one = restrict("string", &[("length", v("1", "integer"))]);
    let xml_chars = 3 + (0xD800 - 0x20) + (0xFFFE - 0xE000) + 0x10_0000;
    assert_eq!(has_at_least(&one, xml_chars), Tri::Yes);
    assert_eq!(has_at_least(&one, xml_chars + 1), Tri::No);
    let controls = restrict("string", &[("pattern", s("[\t-\r]"))]);
    assert_eq!(has_at_least(&controls, 3), Tri::Yes);
    assert_eq!(has_at_least(&controls, 4), Tri::No);
    assert_eq!(
        is_empty(&restrict("string", &[("pattern", s("\u{FFFE}"))])),
        Tri::Yes
    );
    assert_eq!(contains(&dt("string"), &s("\u{0}")), Tri::No);
    assert_eq!(contains(&not(dt("string")), &s("\u{0}")), Tri::No);
    assert_eq!(
        contains(&dt("anyURI"), &Value::AnyUri("\u{B}".into())),
        Tri::No
    );
    assert_eq!(is_empty(&one_of([s("a\u{1F}")])), Tri::Yes);
}

#[test]
fn values_no_literal_has_answer_unknown() {
    let odd_nan = Value::Float(f32::NAN.to_bits() | 1);
    assert_eq!(
        has_at_least(&one_of([v("NaN", "float"), odd_nan.clone()]), 2),
        Tri::No
    );
    assert_eq!(contains(&one_of([v("NaN", "float")]), &odd_nan), Tri::Yes);
    let instant = match v("2000-01-01T00:00:00Z", "dateTime") {
        Value::DateTime(d) => d,
        _ => unreachable!(),
    };
    let far = Value::DateTime(DateTimeValue {
        tz: Some(2000),
        ..instant
    });
    let point = restrict(
        "dateTime",
        &[
            ("minInclusive", Value::DateTime(instant)),
            ("maxInclusive", Value::DateTime(instant)),
        ],
    );
    assert_eq!(
        has_at_least(&and(point.clone(), not(one_of([far.clone()]))), 1),
        Tri::Unknown
    );
    assert_eq!(contains(&point, &far), Tri::Unknown);
    let third = Value::DateTime(DateTimeValue {
        seconds: RationalKey(Rational { num: 1, den: 3 }),
        tz: None,
    });
    assert_eq!(
        contains(&dt("dateTime"), &third),
        Tri::Unknown,
        "seconds are decimal"
    );
    assert_eq!(has_at_least(&one_of([third]), 1), Tri::Unknown);
    let min = Value::Real(RationalKey(Rational {
        num: i128::MIN,
        den: 1,
    }));
    assert_eq!(
        contains(&dt("integer"), &min),
        Tri::Unknown,
        "outside `Rational`'s range"
    );
    let max = Value::Real(RationalKey(Rational::int(i128::MAX).unwrap()));
    assert_eq!(contains(&dt("integer"), &max), Tri::Yes);
    assert_eq!(contains(&dt("long"), &max), Tri::No);
    // Extreme negative non-integers compare and count exactly.
    let edge = Value::Real(RationalKey(Rational::new(i128::MIN + 1, 3).unwrap()));
    let near = Value::Real(RationalKey(Rational::new(i128::MIN + 2, 3).unwrap()));
    let both = one_of([edge.clone(), near.clone(), v("1/3", "owl:rational")]);
    assert_eq!(has_at_least(&both, 3), Tri::Yes);
    assert_eq!(has_at_least(&both, 4), Tri::No);
    assert_eq!(contains(&dt("owl:rational"), &edge), Tri::Yes);
    let above = restrict("owl:rational", &[("minExclusive", edge.clone())]);
    assert_eq!(contains(&above, &near), Tri::Yes);
    assert_eq!(contains(&above, &edge), Tri::No);
    let negative_den = Value::Real(RationalKey(Rational { num: 1, den: -1 }));
    assert_eq!(contains(&dt("integer"), &negative_den), Tri::Unknown);
    let unreduced = Value::Real(RationalKey(Rational { num: 2, den: 4 }));
    assert_eq!(contains(&dt("decimal"), &unreduced), Tri::Unknown);
    assert_eq!(
        is_empty(&one_of([Value::Real(RationalKey(Rational {
            num: 1,
            den: 0
        }))])),
        Tri::Unknown
    );
    assert_eq!(
        contains(
            &dt("rdf:PlainLiteral"),
            &Value::LangString("x".into(), "EN-GB".into())
        ),
        Tri::Yes
    );
}

#[test]
fn deep_ranges_answer_unknown() {
    let nest = |n: usize| (0..n).fold(dt("integer"), |r, _| not(not(r)));
    assert_eq!(is_empty(&nest(30)), Tri::No);
    assert_eq!(is_empty(&nest(100)), Tri::Unknown);
}

#[test]
fn large_enumerations_count_quickly() {
    let words: Vec<Value> = (0..3000)
        .map(|i| s(&format!("value-number-{i:08}")))
        .collect();
    let r = E::OneOf(words.clone());
    assert_eq!(has_at_least(&r, 3000), Tri::Yes);
    assert_eq!(has_at_least(&r, 3001), Tri::No);
    assert_eq!(
        conjunction_satisfiable(std::slice::from_ref(&r), &[], 1, &words),
        Tri::No
    );
}

#[test]
fn long_patterns_answer_unknown() {
    let p = "a".repeat(5000);
    assert_eq!(
        is_empty(&restrict("string", &[("pattern", s(&p))])),
        Tri::Unknown
    );
    assert_eq!(
        is_empty(&restrict("string", &[("pattern", s(&"a".repeat(4000)))])),
        Tri::No
    );
}

#[test]
fn enumerations_past_the_byte_budget_answer_unknown() {
    let long: Vec<Value> = (0..100)
        .map(|i| s(&format!("{i:05}{}", "x".repeat(20_000))))
        .collect();
    assert_eq!(has_at_least(&E::OneOf(long), 2), Tri::Unknown);
}
