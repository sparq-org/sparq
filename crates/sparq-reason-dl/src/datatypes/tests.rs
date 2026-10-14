//! Behavioural tests of the datatype map through its public API. The float and zero
//! cases reproduce the datatype content of the W3C OWL 2 test cases
//! `Datatype-Float-Discrete-001` and `Plus and Minus Zero are Distinct`.

use super::*;

fn xsd(local: &str) -> String {
    format!("{XSD}{local}")
}

fn lit(lexical: &str, dt: &str) -> Literal {
    let iri = match dt.split_once(':') {
        Some(("owl", l)) => format!("{OWL}{l}"),
        Some(("rdf", l)) => format!("{RDF}{l}"),
        _ => xsd(dt),
    };
    Literal::typed(lexical, iri)
}

fn dt(local: &str) -> DataRange {
    DataRange::Datatype(lit("", local).datatype)
}

fn restrict(dt_local: &str, facets: &[(&str, Literal)]) -> DataRange {
    let facets = facets
        .iter()
        .map(|(f, v)| {
            (
                if *f == "langRange" {
                    format!("{RDF}langRange")
                } else {
                    xsd(f)
                },
                v.clone(),
            )
        })
        .collect();
    DataRange::Restriction(lit("", dt_local).datatype, facets)
}

fn one_of<const N: usize>(lits: [Literal; N]) -> DataRange {
    DataRange::OneOf(lits.into())
}

fn and(a: DataRange, b: DataRange) -> DataRange {
    DataRange::Intersection(vec![a, b])
}

fn not(a: DataRange) -> DataRange {
    DataRange::Complement(Box::new(a))
}

#[test]
fn lexical_forms_map_into_exact_value_spaces() {
    let ok = |l: &str, d: &str| in_value_space(&lit(l, d));
    assert_eq!(ok("127", "byte"), Tri::True);
    assert_eq!(ok("128", "byte"), Tri::False);
    assert_eq!(ok("18446744073709551615", "unsignedLong"), Tri::True);
    assert_eq!(ok("-0", "nonNegativeInteger"), Tri::True);
    assert_eq!(ok("1.", "decimal"), Tri::True);
    assert_eq!(ok("1e3", "decimal"), Tri::False);
    assert_eq!(ok("-6/4", "owl:rational"), Tri::True);
    assert_eq!(ok("1/0", "owl:rational"), Tri::False);
    assert_eq!(ok("1.5", "owl:rational"), Tri::False);
    assert_eq!(
        ok("1", "owl:real"),
        Tri::False,
        "owl:real has no lexical forms"
    );
    assert_eq!(ok("-INF", "float"), Tri::True);
    assert_eq!(ok("inf", "double"), Tri::False);
    assert_eq!(ok("QQ==", "base64Binary"), Tri::True);
    assert_eq!(
        ok("QR==", "base64Binary"),
        Tri::False,
        "non-zero trailing bits"
    );
    assert_eq!(ok("QQ= =", "base64Binary"), Tri::True);
    assert_eq!(ok(" QQ==", "base64Binary"), Tri::False);
    assert_eq!(ok("0aFF", "hexBinary"), Tri::True);
    assert_eq!(ok("0aF", "hexBinary"), Tri::False);
    assert_eq!(ok("2000-01-01T00:00:00", "dateTime"), Tri::True);
    assert_eq!(ok("2000-01-01T00:00:00", "dateTimeStamp"), Tri::False);
    assert_eq!(ok("2000-01-01T00:00:00Z", "dateTimeStamp"), Tri::True);
    assert_eq!(ok("a b", "token"), Tri::True);
    assert_eq!(ok("a  b", "token"), Tri::False);
    assert_eq!(ok("été", "NCName"), Tri::True);
    assert_eq!(ok("a:b", "NCName"), Tri::False);
    assert_eq!(ok("abc@EN", "rdf:PlainLiteral"), Tri::True);
    assert_eq!(ok("abc", "rdf:PlainLiteral"), Tri::False);
    assert_eq!(ok("1", "boolean"), Tri::True);
    assert_eq!(in_value_space(&Literal::lang("x", "en-GB")), Tri::True);
    assert_eq!(in_value_space(&Literal::lang("x", "e_n")), Tri::False);
    assert_eq!(ok("<a/>", "rdf:XMLLiteral"), Tri::Unknown);
    assert_eq!(
        ok("2000-01-01", "date"),
        Tri::Unknown,
        "xsd:date is not in the map"
    );
}

#[test]
fn facet_spaces_are_per_datatype() {
    let f = |d: &str, facet: &str, v: Literal| facet_applies(&lit("", d).datatype, &xsd(facet), &v);
    assert_eq!(
        f("integer", "minInclusive", lit("1.5", "decimal")),
        Tri::True
    );
    assert_eq!(
        f("integer", "minInclusive", lit("1/3", "owl:rational")),
        Tri::True
    );
    assert_eq!(f("integer", "minInclusive", lit("1", "float")), Tri::False);
    assert_eq!(f("float", "minInclusive", lit("1", "integer")), Tri::False);
    assert_eq!(f("float", "maxExclusive", lit("NaN", "float")), Tri::True);
    assert_eq!(
        f("boolean", "minInclusive", lit("true", "boolean")),
        Tri::False
    );
    assert_eq!(f("integer", "totalDigits", lit("3", "integer")), Tri::False);
    assert_eq!(f("string", "length", lit("-1", "integer")), Tri::False);
    assert_eq!(
        f("hexBinary", "maxLength", lit("2", "nonNegativeInteger")),
        Tri::True
    );
    assert_eq!(f("hexBinary", "pattern", lit("0+", "string")), Tri::False);
    assert_eq!(f("anyURI", "pattern", lit("http:.*", "string")), Tri::True);
    assert_eq!(f("string", "pattern", lit("[a-", "string")), Tri::False);
    assert_eq!(
        f("string", "pattern", lit(r"\p{IsBasicLatin}+", "string")),
        Tri::Unknown
    );
    assert_eq!(
        f(
            "dateTime",
            "maxExclusive",
            lit("2000-01-01T00:00:00", "dateTime")
        ),
        Tri::True
    );
    let lang = |d: &str, r: &str| {
        facet_applies(
            &lit("", d).datatype,
            &format!("{RDF}langRange"),
            &lit(r, "string"),
        )
    };
    assert_eq!(lang("rdf:PlainLiteral", "en-GB"), Tri::True);
    assert_eq!(lang("rdf:PlainLiteral", "*"), Tri::True);
    assert_eq!(
        lang("rdf:PlainLiteral", "de-*-DE"),
        Tri::False,
        "only basic ranges"
    );
    assert_eq!(lang("string", "en"), Tri::False);
}

#[test]
fn float_ranges_are_discrete() {
    // Datatype-Float-Discrete-001: no float lies strictly between 0 and the least
    // positive subnormal.
    let gap = restrict(
        "float",
        &[
            ("minExclusive", lit("0.0", "float")),
            ("maxExclusive", lit("1.401298464324817e-45", "float")),
        ],
    );
    assert_eq!(gap.is_empty(), Tri::True);
    let closed = restrict(
        "float",
        &[
            ("minExclusive", lit("0.0", "float")),
            ("maxInclusive", lit("1.401298464324817e-45", "float")),
        ],
    );
    assert_eq!(closed.is_empty(), Tri::False);
    let dgap = restrict(
        "double",
        &[
            ("minExclusive", lit("0", "double")),
            ("maxExclusive", lit("4.9E-324", "double")),
        ],
    );
    assert_eq!(dgap.is_empty(), Tri::True);
    assert_eq!(
        restrict("float", &[("minInclusive", lit("NaN", "float"))]).is_empty(),
        Tri::True
    );
    assert_eq!(
        not(restrict("float", &[("minInclusive", lit("-INF", "float"))]))
            .contains(&lit("NaN", "float")),
        Tri::True
    );
    // A bound as large as the largest float still leaves +INF above it.
    let top = restrict("float", &[("minExclusive", lit("3.4028235E38", "float"))]);
    assert_eq!(top.is_subset_of(&one_of([lit("INF", "float")])), Tri::True);
}

#[test]
fn zeros_are_equal_but_not_identical() {
    // Plus and Minus Zero are Distinct.
    let (pz, nz) = (lit("0.0", "float"), lit("-0.0", "float"));
    assert_eq!(
        and(one_of([pz.clone()]), one_of([nz.clone()])).is_empty(),
        Tri::True
    );
    let zero = restrict(
        "float",
        &[("minInclusive", pz.clone()), ("maxInclusive", pz.clone())],
    );
    assert_eq!(zero.contains(&nz), Tri::True);
    assert_eq!(
        zero.is_subset_of(&one_of([pz.clone(), nz.clone()])),
        Tri::True
    );
    assert_eq!(zero.is_subset_of(&one_of([pz])), Tri::False);
    // Integer zeros are one value.
    assert_eq!(
        one_of([lit("0", "integer")]).contains(&lit("-0", "integer")),
        Tri::True
    );
}

#[test]
fn reals_are_ordered_exactly_across_layers() {
    let between = |d: &str, lo: Literal, hi: Literal| {
        restrict(d, &[("minExclusive", lo), ("maxExclusive", hi)])
    };
    let (zero, one) = (lit("0", "integer"), lit("1", "integer"));
    assert_eq!(
        between("integer", zero.clone(), one.clone()).is_empty(),
        Tri::True
    );
    assert_eq!(
        between("decimal", zero.clone(), one.clone()).is_empty(),
        Tri::False
    );
    let third = lit("1/3", "owl:rational");
    let point = |d: &str| {
        restrict(
            d,
            &[
                ("minInclusive", third.clone()),
                ("maxInclusive", third.clone()),
            ],
        )
    };
    assert_eq!(point("owl:rational").is_empty(), Tri::False);
    assert_eq!(
        point("decimal").is_empty(),
        Tri::True,
        "1/3 is not a decimal"
    );
    // owl:real has irrationals in every interval, but not at a rational point.
    assert_eq!(
        and(
            between("owl:real", zero.clone(), one),
            not(dt("owl:rational"))
        )
        .is_empty(),
        Tri::False
    );
    assert_eq!(
        and(point("owl:real"), not(dt("owl:rational"))).is_empty(),
        Tri::True
    );
    // No rounding: a bound a hair above 0.1 still excludes 0.1.
    let tight = restrict("decimal", &[("minExclusive", lit("0.1", "decimal"))]);
    assert_eq!(
        tight.contains(&lit("0.1000000000000000000000000000000001", "decimal")),
        Tri::True
    );
    assert_eq!(tight.contains(&lit("0.1", "decimal")), Tri::False);
    assert_eq!(
        and(dt("nonNegativeInteger"), dt("nonPositiveInteger")).is_subset_of(&one_of([zero])),
        Tri::True
    );
    assert_eq!(
        and(dt("positiveInteger"), dt("negativeInteger")).is_empty(),
        Tri::True
    );
    assert_eq!(
        one_of([lit("1", "integer")]).contains(&lit("1.0", "decimal")),
        Tri::True
    );
    assert_eq!(
        one_of([lit("1", "integer")]).contains(&lit("1", "float")),
        Tri::False
    );
}

#[test]
fn subsumption_follows_the_datatype_hierarchy() {
    let sub = |a: &str, b: &str| dt(a).is_subset_of(&dt(b));
    assert_eq!(sub("integer", "decimal"), Tri::True);
    assert_eq!(sub("decimal", "owl:rational"), Tri::True);
    assert_eq!(sub("owl:rational", "owl:real"), Tri::True);
    assert_eq!(sub("owl:real", "owl:rational"), Tri::False);
    assert_eq!(sub("byte", "short"), Tri::True);
    assert_eq!(sub("unsignedByte", "byte"), Tri::False);
    assert_eq!(sub("float", "owl:real"), Tri::False);
    assert_eq!(sub("NCName", "Name"), Tri::True);
    assert_eq!(sub("Name", "NCName"), Tri::False);
    assert_eq!(sub("token", "normalizedString"), Tri::True);
    assert_eq!(sub("language", "token"), Tri::True);
    assert_eq!(sub("string", "rdf:PlainLiteral"), Tri::True);
    assert_eq!(sub("dateTimeStamp", "dateTime"), Tri::True);
    assert_eq!(sub("dateTime", "dateTimeStamp"), Tri::False);
    assert_eq!(and(dt("anyURI"), dt("string")).is_empty(), Tri::True);
    assert_eq!(
        and(dt("hexBinary"), dt("base64Binary")).is_empty(),
        Tri::True
    );
    let literal = DataRange::Datatype(RDFS_LITERAL.into());
    assert_eq!(not(literal.clone()).is_empty(), Tri::True);
    assert_eq!(
        not(DataRange::Union(vec![dt("boolean"), not(dt("boolean"))])).is_empty(),
        Tri::True
    );
    assert_eq!(
        and(
            dt("boolean"),
            not(one_of([lit("true", "boolean"), lit("0", "boolean")]))
        )
        .is_empty(),
        Tri::True
    );
}

#[test]
fn string_ranges_combine_patterns_lengths_and_languages() {
    let aa = and(
        restrict("string", &[("length", lit("2", "integer"))]),
        restrict("string", &[("pattern", lit("a*", "string"))]),
    );
    assert_eq!(aa.is_subset_of(&one_of([lit("aa", "string")])), Tri::True);
    assert_eq!(aa.is_empty(), Tri::False);
    let empty_only = restrict("string", &[("maxLength", lit("0", "integer"))]);
    assert_eq!(
        and(empty_only, not(one_of([lit("", "string")]))).is_empty(),
        Tri::True
    );
    let colon = restrict("string", &[("pattern", lit(".*:.*", "string"))]);
    assert_eq!(and(dt("NCName"), colon.clone()).is_empty(), Tri::True);
    assert_eq!(and(dt("Name"), colon).is_empty(), Tri::False);
    let tagged = restrict("rdf:PlainLiteral", &[("langRange", lit("*", "string"))]);
    assert_eq!(and(tagged.clone(), dt("string")).is_empty(), Tri::True);
    assert_eq!(
        DataRange::Union(vec![tagged, dt("string")]).is_subset_of(&dt("rdf:PlainLiteral")),
        Tri::True
    );
    assert_eq!(
        dt("rdf:PlainLiteral").is_subset_of(&dt("string")),
        Tri::False
    );
    let en = restrict("rdf:PlainLiteral", &[("langRange", lit("EN", "string"))]);
    assert_eq!(en.contains(&Literal::lang("x", "en-GB")), Tri::True);
    assert_eq!(en.contains(&lit("x@EN", "rdf:PlainLiteral")), Tri::True);
    assert_eq!(en.contains(&Literal::lang("x", "eng")), Tri::False);
    assert_eq!(en.contains(&lit("x", "string")), Tri::False);
    // Length counts characters of the string part, whatever the tag.
    let two = restrict("rdf:PlainLiteral", &[("length", lit("2", "integer"))]);
    assert_eq!(two.contains(&Literal::lang("é€", "fr")), Tri::True);
    assert_eq!(
        restrict("string", &[("pattern", lit(r"\p{IsBasicLatin}", "string"))]).is_empty(),
        Tri::Unknown
    );
    let uri = restrict("anyURI", &[("pattern", lit("http://.*", "string"))]);
    assert_eq!(
        uri.contains(&lit("http://example.org/", "anyURI")),
        Tri::True
    );
    assert_eq!(
        uri.contains(&lit("http://example.org/", "string")),
        Tri::False
    );
}

#[test]
fn binary_lengths_count_octets() {
    let empty_hex = restrict("hexBinary", &[("maxLength", lit("0", "integer"))]);
    assert_eq!(
        empty_hex.is_subset_of(&one_of([lit("", "hexBinary")])),
        Tri::True
    );
    let one = restrict("base64Binary", &[("length", lit("1", "integer"))]);
    assert_eq!(one.contains(&lit("QQ==", "base64Binary")), Tri::True);
    assert_eq!(one.contains(&lit("QUI=", "base64Binary")), Tri::False);
    assert_eq!(
        and(one, not(one_of([lit("QQ==", "base64Binary")]))).is_empty(),
        Tri::False
    );
}

#[test]
fn time_instants_compare_on_the_time_line() {
    let noon = lit("2000-01-01T00:00:00Z", "dateTime");
    let from = restrict("dateTime", &[("minInclusive", noon.clone())]);
    // A value without an offset is above only past the ±14:00 window.
    assert_eq!(
        from.contains(&lit("2000-01-01T10:00:00", "dateTime")),
        Tri::False
    );
    assert_eq!(
        from.contains(&lit("2000-01-01T14:00:01", "dateTime")),
        Tri::True
    );
    assert_eq!(
        from.contains(&lit("2000-01-01T01:00:00+01:00", "dateTime")),
        Tri::True
    );
    // Same instant, different offset: equal, so inside the facet; not identical, so
    // outside the enumeration.
    let same = lit("2000-01-01T01:00:00+01:00", "dateTimeStamp");
    assert_eq!(one_of([noon.clone()]).contains(&same), Tri::False);
    let instant = restrict(
        "dateTimeStamp",
        &[
            ("minInclusive", noon.clone()),
            ("maxInclusive", noon.clone()),
        ],
    );
    assert_eq!(instant.contains(&same), Tri::True);
    assert_eq!(
        and(instant, not(one_of([noon, same]))).is_empty(),
        Tri::False
    );
    let before = restrict(
        "dateTime",
        &[("maxExclusive", lit("2000-01-01T00:00:00", "dateTime"))],
    );
    assert_eq!(
        before.contains(&lit("1999-12-31T23:59:59.999", "dateTime")),
        Tri::True
    );
    assert_eq!(
        before.contains(&lit("1999-12-31T23:00:00Z", "dateTime")),
        Tri::False
    );
    assert_eq!(
        before.contains(&lit("1999-12-31T09:59:59Z", "dateTime")),
        Tri::True
    );
}

#[test]
fn unimplemented_inputs_answer_unknown() {
    assert_eq!(dt("date").is_empty(), Tri::Unknown);
    assert_eq!(
        DataRange::Datatype("http://example.org/custom".into()).is_subset_of(&dt("string")),
        Tri::Unknown
    );
    assert_eq!(
        one_of([lit("<a/>", "rdf:XMLLiteral")]).is_empty(),
        Tri::Unknown
    );
    assert_eq!(dt("rdf:XMLLiteral").is_empty(), Tri::False);
    assert_eq!(
        dt("integer").contains(&lit("x", "integer")),
        Tri::Unknown,
        "an ill-typed literal has no value"
    );
    assert_eq!(
        restrict("integer", &[("minInclusive", lit("1", "float"))]).is_empty(),
        Tri::Unknown
    );
}
