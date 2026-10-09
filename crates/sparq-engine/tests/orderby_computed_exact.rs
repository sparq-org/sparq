//! #3198 — a COMPUTED numeric ORDER BY key is ordered by its exact value, like a plain
//! numeric variable key, `<` and MIN/MAX. An `f64` key collapsed integers beyond 2^53
//! and high-precision decimals into ties that kept their input order.

use sparq_core::Graph;
use sparq_engine::query;

fn subjects(g: &Graph, q: &str) -> Vec<String> {
    query(g, q)
        .unwrap()
        .rows
        .iter()
        .map(|r| r[0].as_ref().map(|t| t.to_string()).unwrap_or_default())
        .collect()
}

fn graph(values: &[(&str, &str)]) -> Graph {
    let mut ttl = String::from("@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n");
    for (s, v) in values {
        ttl.push_str(&format!("<http://ex/{s}> <http://ex/v> {v} .\n"));
    }
    Graph::load_str(&ttl, "turtle").unwrap()
}

const EXPECT_ASC: [&str; 3] = ["<http://ex/b>", "<http://ex/a>", "<http://ex/c>"];

fn check(g: &Graph) {
    for key in ["?v", "(?v + 0)", "(?v * 1)", "(-(-?v))", "?k"] {
        let q = format!(
            "SELECT ?s WHERE {{ ?s <http://ex/v> ?v BIND(?v + 0 AS ?k) }} ORDER BY {key}"
        );
        assert_eq!(subjects(g, &q), EXPECT_ASC, "ascending by {key}");
        let q = format!(
            "SELECT ?s WHERE {{ ?s <http://ex/v> ?v BIND(?v + 0 AS ?k) }} ORDER BY DESC({key})"
        );
        let mut desc = EXPECT_ASC.to_vec();
        desc.reverse();
        assert_eq!(subjects(g, &q), desc, "descending by {key}");
        let q = format!(
            "SELECT ?s WHERE {{ ?s <http://ex/v> ?v BIND(?v + 0 AS ?k) }} ORDER BY {key} LIMIT 1"
        );
        assert_eq!(subjects(g, &q), EXPECT_ASC[..1], "top-1 by {key}");
    }
}

#[test]
fn high_precision_decimals_order_exactly_under_a_computed_key() {
    // All three share one f64. Input order a, b, c differs from value order b, a, c.
    check(&graph(&[
        ("a", "\"0.100000000000000000002\"^^xsd:decimal"),
        ("b", "\"0.100000000000000000001\"^^xsd:decimal"),
        ("c", "\"0.100000000000000000003\"^^xsd:decimal"),
    ]));
}

#[test]
fn integers_beyond_2_pow_53_order_exactly_under_a_computed_key() {
    check(&graph(&[("a", "9007199254740994"), ("b", "9007199254740993"), ("c", "9007199254740995")]));
}

#[test]
fn integer_division_by_zero_is_an_unbound_key_not_infinity() {
    // Unbound sorts first; an `f64` key made 1/0 positive infinity, which sorts last.
    let g = graph(&[("a", "1"), ("b", "0"), ("c", "2")]);
    let q = "SELECT ?s WHERE { ?s <http://ex/v> ?v } ORDER BY (1 / ?v)";
    assert_eq!(subjects(&g, q)[0], "<http://ex/b>");
}

// One ORDER BY column holding both a stored numeric (passed through IF) and a computed
// one must still compare exactly: the stored key may not fall back to its cached f64.
#[test]
fn mixed_stored_and_computed_keys_order_exactly() {
    let g = graph(&[("a", "0.100000000000000000001"), ("b", "0.100000000000000000002")]);
    let base = "SELECT ?s WHERE { ?s <http://ex/v> ?v \
        BIND(IF(?s = <http://ex/a>, ?v, ?v + 0.000000000000000000001) AS ?k) } ORDER BY ?k";
    assert_eq!(subjects(&g, base), ["<http://ex/a>", "<http://ex/b>"]);
    assert_eq!(subjects(&g, &format!("{base} LIMIT 1")), ["<http://ex/a>"]);
    let desc = base.replace("ORDER BY ?k", "ORDER BY DESC(?k)");
    assert_eq!(subjects(&g, &desc), ["<http://ex/b>", "<http://ex/a>"]);
}

// Two computed decimals whose two-rounding f64 images were in REVERSE order: the smaller
// value's image came out above the larger's, so the exact tie recheck never ran.
#[test]
fn computed_decimals_whose_f64_images_crossed_order_by_value() {
    let g = Graph::load_str("", "turtle").unwrap();
    let base = "SELECT ?v WHERE { VALUES ?v { 0.947 0.9469999999999999999999 } }";
    let small = subjects(&g, &format!("{base} ORDER BY (?v + 0) LIMIT 1"));
    assert!(small[0].contains("0.9469999999999999999999"), "{small:?}");
    let large = subjects(&g, &format!("{base} ORDER BY DESC(?v + 0) LIMIT 1"));
    assert!(!large[0].contains("0.94699"), "{large:?}");
}

/// Like [`check`], for the plain variable key only: arithmetic on a value beyond the i128
/// tower is an error (an unbound key), so only the stored term's order is pinned there.
fn check_plain(g: &Graph) {
    let q = |m: &str| subjects(g, &format!("SELECT ?s WHERE {{ ?s <http://ex/v> ?v }} ORDER BY {m}"));
    assert_eq!(q("?v"), EXPECT_ASC, "ascending");
    let mut desc = EXPECT_ASC.to_vec();
    desc.reverse();
    assert_eq!(q("DESC(?v)"), desc, "descending");
    assert_eq!(q("?v LIMIT 1"), EXPECT_ASC[..1], "top-1");
}

// #3157: integers and decimals beyond the i128 tower are numbers in the ORDER BY total
// order (compared exactly by lexical), not opaque strings ordered lexically.
#[test]
fn integers_beyond_i128_order_by_value() {
    // 40-digit integers: past i128 (~1.7e38). Lexical order would put c ("1000…") first.
    check_plain(&graph(&[
        ("a", "\"9999999999999999999999999999999999999999\"^^xsd:integer"),
        ("b", "\"9999999999999999999999999999999999999998\"^^xsd:integer"),
        ("c", "\"10000000000000000000000000000000000000000\"^^xsd:integer"),
    ]));
}

#[test]
fn integers_beyond_i128_order_against_in_range_numbers() {
    // An in-range integer, a double and a beyond-i128 decimal in one column.
    check_plain(&graph(&[
        ("a", "\"1.0e39\"^^xsd:double"),
        ("b", "\"5\"^^xsd:integer"),
        ("c", "\"1000000000000000000000000000000000000000.5\"^^xsd:decimal"),
    ]));
    // Beyond f64's range (both images are +INF): still ordered by exact value.
    let big = "1".to_string() + &"0".repeat(400);
    let bigger = "2".to_string() + &"0".repeat(400);
    check_plain(&graph(&[
        ("a", &format!("\"{bigger}\"^^xsd:integer")),
        ("b", &format!("\"{big}\"^^xsd:integer")),
        ("c", "\"INF\"^^xsd:double"),
    ]));
}

#[test]
fn min_max_over_integers_beyond_i128_use_value_order() {
    let g = graph(&[
        ("a", "\"9999999999999999999999999999999999999999\"^^xsd:integer"),
        ("b", "\"10000000000000000000000000000000000000000\"^^xsd:integer"),
    ]);
    let q = |agg: &str| subjects(&g, &format!("SELECT ({agg}(?v) AS ?m) WHERE {{ ?s <http://ex/v> ?v }}"));
    assert!(q("MAX")[0].starts_with("\"10000000000000000000000000000000000000000\""), "{:?}", q("MAX"));
    assert!(q("MIN")[0].starts_with("\"9999999999999999999999999999999999999999\""), "{:?}", q("MIN"));
}

// Unbounded integer subtypes beyond i128 are numbers too, so a negativeInteger sorts
// below a positive integer; a subtype whose sign facet the lexical breaks stays opaque.
#[test]
fn integer_subtypes_beyond_i128_order_by_value() {
    let big = "10000000000000000000000000000000000000000";
    check_plain(&graph(&[
        ("a", &format!("\"{big}\"^^xsd:positiveInteger")),
        ("b", &format!("\"-{big}\"^^xsd:negativeInteger")),
        ("c", &format!("\"{big}0\"^^xsd:nonNegativeInteger")),
    ]));
    let g = graph(&[
        ("a", &format!("\"-{big}\"^^xsd:nonPositiveInteger")),
        ("b", &format!("\"{big}\"^^xsd:integer")),
    ]);
    let q = |agg: &str| subjects(&g, &format!("SELECT ({agg}(?v) AS ?m) WHERE {{ ?s <http://ex/v> ?v }}"));
    assert!(q("MIN")[0].starts_with(&format!("\"-{big}\"")), "{:?}", q("MIN"));
    assert!(q("MAX")[0].starts_with(&format!("\"{big}\"")), "{:?}", q("MAX"));
}

// A whitespace-padded raw lexical is not a well-formed beyond-tower number, so it is not
// ordered as one: a padded huge NEGATIVE integer does not sort below the in-range numbers.
#[test]
fn padded_beyond_i128_lexical_is_not_a_number() {
    let big = "10000000000000000000000000000000000000000";
    let g = graph(&[("a", &format!("\" -{big} \"^^xsd:integer")), ("b", "5"), ("c", "7")]);
    let order = subjects(&g, "SELECT ?s WHERE { ?s <http://ex/v> ?v } ORDER BY ?v");
    assert_ne!(order[0], "<http://ex/a>", "{order:?}");
}

// A Unicode-whitespace-padded lexical (U+00A0) is not a number either. Admitting it as one
// while its `f64` image failed made the comparator cyclic (2 < 10 < "11\u{a0}" < 2), so
// the sorted order depended on input order.
#[test]
fn unicode_padded_lexical_is_not_a_number_and_order_is_input_independent() {
    let vals = ["\"11\u{a0}\"^^xsd:integer", "2", "10"];
    let perms = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    let mut seen = None;
    for p in perms {
        let names = ["a", "b", "c"];
        let g = graph(&p.map(|i| (names[i], vals[i])));
        let order = subjects(&g, "SELECT ?s WHERE { ?s <http://ex/v> ?v } ORDER BY ?v");
        let min = subjects(&g, "SELECT (MIN(?v) AS ?m) WHERE { ?s <http://ex/v> ?v }");
        let max = subjects(&g, "SELECT (MAX(?v) AS ?m) WHERE { ?s <http://ex/v> ?v }");
        let got = (order, min, max);
        match &seen {
            None => seen = Some(got),
            Some(first) => assert_eq!(&got, first, "input order {p:?}"),
        }
    }
    // The two real numbers stay in numeric order.
    let (order, ..) = seen.unwrap();
    let pos = |s: &str| order.iter().position(|o| o == s).unwrap();
    assert!(pos("<http://ex/b>") < pos("<http://ex/c>"), "{order:?}");
}
