// eddsa-sha256-merkle-2026 encoding: value-lane order and fixed vectors.
#![cfg(feature = "authenticated-rdf")]

use oxrdf::{Literal, NamedNode, Quad};
use sparq_proved_evaluator_model::merkle_suite::{self as merkle, lane};

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

fn typed(lexical: &str, datatype: &str) -> [u8; merkle::LANE_BYTES] {
    merkle::value_lane(&Literal::new_typed_literal(lexical, NamedNode::new_unchecked(format!("{XSD}{datatype}"))))
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn increasing(class: u8, lanes: &[[u8; merkle::LANE_BYTES]]) {
    for lane in lanes {
        assert_eq!(lane[0], class, "{}", hex_string(lane));
    }
    for pair in lanes.windows(2) {
        assert!(pair[0] < pair[1], "{} !< {}", hex_string(&pair[0]), hex_string(&pair[1]));
    }
}

#[test]
fn numeric_lanes_follow_value_order_across_integer_and_decimal() {
    let values = [
        ("-123456789012345678901234567890", "integer"),
        ("-100.5", "decimal"),
        ("-100", "integer"),
        ("-1.000000000000000001", "decimal"),
        ("0", "integer"),
        ("0.000000000000000001", "decimal"),
        ("1", "decimal"),
        ("1.5", "decimal"),
        ("2", "integer"),
        ("10", "integer"),
        ("123456789012345678901234567890", "integer"),
    ];
    let lanes: Vec<_> = values.iter().map(|(v, t)| typed(v, t)).collect();
    increasing(lane::DECIMAL, &lanes);
    // Equal values share a lane whatever their lexical form or numeric datatype.
    for same in [("05", "integer"), ("+5", "integer"), ("5.000", "decimal"), ("5.", "decimal")] {
        assert_eq!(typed(same.0, same.1), typed("5", "integer"), "{same:?}");
    }
    assert_eq!(typed("-0", "integer"), typed("0", "decimal"));
    assert_eq!(typed("-.0", "decimal"), typed("0", "integer"));
    // Valid but outside precision or range; ill-typed literals have no lane.
    assert_eq!(typed("0.0000000000000000001", "decimal")[0], lane::UNREPRESENTABLE);
    assert_eq!(typed(&"9".repeat(60), "integer")[0], lane::UNREPRESENTABLE);
    for bad in [("1.5", "integer"), ("", "integer"), ("abc", "decimal"), (".", "decimal"), ("1e3", "decimal")] {
        assert_eq!(typed(bad.0, bad.1), [0; merkle::LANE_BYTES], "{bad:?}");
    }
}

#[test]
fn double_date_time_boolean_and_string_lanes_follow_sparql_order() {
    let doubles: Vec<_> = ["-INF", "-1e300", "-1.5", "0", "1e-300", "1", "1.5E2", "INF"]
        .iter()
        .map(|v| typed(v, "double"))
        .collect();
    increasing(lane::DOUBLE, &doubles);
    assert_eq!(typed("-0", "double"), typed("0.0", "double"));
    assert_eq!(typed("NaN", "double")[0], lane::UNREPRESENTABLE);
    assert_eq!(typed("inf", "double")[0], lane::NONE);
    assert_eq!(typed("0.1", "float"), typed(&f64::from(0.1f32).to_string(), "double"));

    let times: Vec<_> = [
        "0001-01-01T00:00:00Z",
        "1969-12-31T23:59:59.999Z",
        "1970-01-01T00:00:00Z",
        "2026-01-01T00:00:00.001Z",
        "2026-02-28T23:00:00-01:00",
        "9999-12-31T23:59:59Z",
    ]
    .iter()
    .map(|v| typed(v, "dateTime"))
    .collect();
    increasing(lane::DATE_TIME, &times);
    assert_eq!(typed("2026-01-01T01:00:00+01:00", "dateTime"), typed("2026-01-01T00:00:00.000Z", "dateTime"));
    assert_eq!(typed("2026-01-01T00:00:00", "dateTime")[0], lane::LOCAL_DATE_TIME);
    assert_eq!(typed("2026-01-01T00:00:00.0001Z", "dateTime")[0], lane::UNREPRESENTABLE);
    assert_eq!(typed("12026-01-01T00:00:00Z", "dateTime")[0], lane::UNREPRESENTABLE);
    for bad in ["2026-02-29T00:00:00Z", "2026-01-01T24:00:01Z", "2026-01-01", "2026-01-01T00:00:00+15:00"] {
        assert_ne!(typed(bad, "dateTime")[0], lane::DATE_TIME, "{bad}");
    }

    increasing(lane::BOOLEAN, &[typed("false", "boolean"), typed("true", "boolean")]);
    assert_eq!(typed("1", "boolean"), typed("true", "boolean"));
    assert_eq!(typed("yes", "boolean")[0], lane::NONE);

    let strings: Vec<_> = ["", "A", "Ab", "a", "é"].iter().map(|v| typed(v, "string")).collect();
    increasing(lane::STRING, &strings);
    let lang = merkle::value_lane(&Literal::new_language_tagged_literal_unchecked("chat", "fr"));
    assert_eq!(lang, typed("chat", "string"));
    let long = typed(&"x".repeat(31), "string");
    assert_eq!(long[0], lane::STRING_PREFIX);
    assert_eq!(&long[1..], "x".repeat(30).as_bytes());
    assert_eq!(typed("a\u{0}", "string")[0], lane::STRING_PREFIX);
    assert_eq!(merkle::value_lane(&Literal::new_typed_literal("x", NamedNode::new_unchecked("http://ex/t"))), [0; 31]);
}

#[test]
fn leaf_root_and_message_match_the_fixed_vectors() {
    let quad: Quad = oxttl::NQuadsParser::new()
        .for_slice(br#"_:c14n0 <http://ex/balance> "1250.50"^^<http://www.w3.org/2001/XMLSchema#decimal> ."#)
        .next()
        .unwrap()
        .unwrap();
    let leaf = merkle::leaf(&quad).unwrap();
    assert_eq!(hex_string(&leaf), LEAF);
    let other: Quad = oxttl::NQuadsParser::new()
        .for_slice(b"<urn:vc:1> <http://ex/name> \"Alice\"@en .")
        .next()
        .unwrap()
        .unwrap();
    let mut leaves = [leaf, merkle::leaf(&other).unwrap()];
    leaves.sort();
    let root = merkle::root(&leaves).unwrap();
    assert_eq!(hex_string(&root), ROOT);
    assert_eq!(merkle::root(&[leaf]), Some(leaf));
    assert_eq!(merkle::root(&[]), None);
    assert_eq!(merkle::root(&[leaves[1], leaves[0]]), None);
    assert_eq!(merkle::root(&[leaf, leaf]), None);
    let message = merkle::signed_message(&[7; 32], 2, &root, &[9; 32]);
    assert_eq!(hex_string(&message), MESSAGE);
}

const LEAF: &str = "8c4a491021d82e07c77c748df8c123d9721163cb8da614c7d21a4aef901c1146";
const ROOT: &str = "173a57085f798200738c519bcc71b7af4cb16bf67edcf1b2e1add2b516f6be2b";
const MESSAGE: &str = "f2c7bec9161e40bf8054356fa12905665d7c2d4187504d31aa529aeee4e601a3";
