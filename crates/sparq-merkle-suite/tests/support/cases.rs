use oxrdf::{Literal, NamedNode};
use sparq_merkle_suite::{comparison_key, key_class};

pub struct Case {
    pub datatype: &'static str,
    pub datatype_b: String,
    pub a: Literal,
    pub b: Literal,
    pub key_a: [u8; 31],
    pub key_b: [u8; 31],
    pub order: Option<std::cmp::Ordering>,
}
fn typed(text: impl Into<String>, dt: &str) -> Literal {
    Literal::new_typed_literal(
        text,
        NamedNode::new_unchecked(format!("http://www.w3.org/2001/XMLSchema#{dt}")),
    )
}
pub fn cases() -> Vec<Case> {
    let mut seed = 0x5a17_1243_6b9d_08fe_u64;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut literals = Vec::new();
    for _ in 0..12 {
        let a = random();
        let b = random();
        literals.push((
            "integer",
            typed((a % 100000).to_string(), "integer"),
            typed(format!("{}.5", b % 100000), "decimal"),
        ));
        literals.push((
            "double",
            typed(format!("{}.{}", a % 100000, a % 997), "double"),
            typed(format!("{}.{}", b % 100000, b % 997), "float"),
        ));

        for (dt, left, right) in [
            ("integer", (a as i64).to_string(), (b as i64).to_string()),
            (
                "decimal",
                format!("{}.{}", a as i64 / 100, a % 100),
                format!("{}.{}", b as i64 / 100, b % 100),
            ),
            (
                "double",
                format!("{}.{}e{}", a % 100000, a % 997, a as i32 % 300),
                format!("{}.{}e{}", b % 100000, b % 997, b as i32 % 300),
            ),
            (
                "float",
                format!("{}.{}e{}", a % 10000, a % 997, a as i32 % 40),
                format!("{}.{}e{}", b % 10000, b % 997, b as i32 % 40),
            ),
            (
                "dateTime",
                format!(
                    "{:04}-{:02}-{:02}T00:00:00Z",
                    1000 + a % 8000,
                    1 + a % 12,
                    1 + a % 28
                ),
                format!(
                    "{:04}-{:02}-{:02}T00:00:00Z",
                    1000 + b % 8000,
                    1 + b % 12,
                    1 + b % 28
                ),
            ),
            (
                "dateTime",
                format!(
                    "{:04}-{:02}-{:02}T00:00:00",
                    1000 + a % 8000,
                    1 + a % 12,
                    1 + a % 28
                ),
                format!(
                    "{:04}-{:02}-{:02}T00:00:00",
                    1000 + b % 8000,
                    1 + b % 12,
                    1 + b % 28
                ),
            ),
            (
                "date",
                format!("{:04}-{:02}-{:02}", 1000 + a % 8000, 1 + a % 12, 1 + a % 28),
                format!("{:04}-{:02}-{:02}", 1000 + b % 8000, 1 + b % 12, 1 + b % 28),
            ),
            (
                "boolean",
                if a % 2 == 0 { "false" } else { "true" }.to_owned(),
                if b % 2 == 0 { "false" } else { "true" }.to_owned(),
            ),
            ("string", format!("é{a:016x}"), format!("é{b:016x}")),
            (
                "string",
                format!("{a:016x}{}", "z".repeat(20)),
                format!("{b:016x}{}", "z".repeat(20)),
            ),
            ("decimal", format!("0.{a:019}"), format!("0.{b:019}")),
            ("other", format!("value-{a}"), format!("value-{b}")),
        ] {
            literals.push((dt, typed(left, dt), typed(right, dt)));
        }
    }
    for dt in [
        "integer", "decimal", "double", "float", "dateTime", "date", "boolean",
    ] {
        literals.push((dt, typed("not-a-value", dt), typed("bad", dt)));
    }
    for (dt, left, right) in [
        ("decimal", "0.0000000000000000001", "0.0000000000000000002"),
        ("decimal", "-0", "0.000"),
        ("decimal", "-.0", "+.5"),
        ("double", "-0", "0"),
        ("double", "5e-324", "2.2250738585072014e-308"),
        ("double", "1.7976931348623157e308", "INF"),
        ("double", "9007199254740993", "9007199254740995"),
        (
            "float",
            "1.000000059604644775390625",
            "1.000000059604644775390626",
        ),
        (
            "double",
            "1.00000000000000011102230246251565404236316680908203125",
            "1.00000000000000011102230246251565404236316680908203126",
        ),
        ("double", "1e-99999", "1e99999"),
        ("float", "1.401298464324817e-45", "1.1754943508222875e-38"),
        ("float", "3.4028234663852886e38", "INF"),
        ("double", "NaN", "NaN"),
        (
            "dateTime",
            "2026-01-01T00:00:00.0001Z",
            "2026-01-01T24:00:00Z",
        ),
        ("date", "2026-01-01Z", "2026-01-01-01:00"),
        ("date", "-0044-03-15", "10000-01-01"),
        ("dateTime", "2000-02-29T00:00:00Z", "1900-02-29T00:00:00Z"),
        ("string", "abc\0", "abd\0"),
    ] {
        literals.push((dt, typed(left, dt), typed(right, dt)));
    }
    for dt in [
        "nonPositiveInteger",
        "negativeInteger",
        "long",
        "int",
        "short",
        "byte",
        "nonNegativeInteger",
        "unsignedLong",
        "unsignedInt",
        "unsignedShort",
        "unsignedByte",
        "positiveInteger",
    ] {
        for _ in 0..4 {
            let a = random() % 100 + 1;
            let b = random() % 100 + 1;
            let negative = matches!(dt, "nonPositiveInteger" | "negativeInteger");
            literals.push((
                dt,
                typed(format!("{}{a}", if negative { "-" } else { "+" }), dt),
                typed(format!("{}{b}", if negative { "-" } else { "" }), dt),
            ));
        }
    }
    for (dt, left, right) in [
        ("byte", "127", "128"),
        ("byte", "-128", "-129"),
        ("short", "32767", "32768"),
        ("short", "-32768", "-32769"),
        ("int", "2147483647", "2147483648"),
        ("int", "-2147483648", "-2147483649"),
        ("long", "9223372036854775807", "9223372036854775808"),
        ("long", "-9223372036854775808", "-9223372036854775809"),
        ("unsignedByte", "255", "256"),
        ("unsignedShort", "65535", "65536"),
        ("unsignedInt", "4294967295", "4294967296"),
        (
            "unsignedLong",
            "18446744073709551615",
            "18446744073709551616",
        ),
        ("nonPositiveInteger", "0", "1"),
        ("negativeInteger", "-1", "0"),
        ("nonNegativeInteger", "0", "-1"),
        ("positiveInteger", "1", "0"),
        (
            "dateTime",
            "2026-01-01T01:00:00+01:00",
            "2026-01-01T00:00:00Z",
        ),
        (
            "dateTime",
            "2026-01-01T01:00:00-01:00",
            "2026-01-01T00:00:00Z",
        ),
        (
            "dateTime",
            "2026-01-01T00:00:00.001Z",
            "2026-01-01T00:00:00.002Z",
        ),
    ] {
        literals.push((dt, typed(left, dt), typed(right, dt)));
    }
    let graph = sparq_core::Graph::new();
    literals
        .into_iter()
        .map(|(datatype, a, b)| {
            let key_a = comparison_key(&a);
            let key_b = comparison_key(&b);
            let order = if key_a[0] == key_b[0]
                && !matches!(key_a[0], key_class::NONE | key_class::UNREPRESENTABLE)
                && !(key_a[0] == key_class::STRING_PREFIX && key_a == key_b)
            {
                let order = key_a.cmp(&key_b);
                let lt =
                    sparq_engine::ask(&graph, &format!("ASK {{ FILTER({a} < {b}) }}")).unwrap();
                let gt =
                    sparq_engine::ask(&graph, &format!("ASK {{ FILTER({a} > {b}) }}")).unwrap();
                assert_eq!(
                    (lt, gt),
                    (order.is_lt(), order.is_gt()),
                    "engine/key ordering differs: {a}, {b}"
                );
                Some(order)
            } else {
                None
            };
            let datatype_b = b
                .datatype()
                .as_str()
                .strip_prefix("http://www.w3.org/2001/XMLSchema#")
                .unwrap()
                .to_owned();
            Case {
                datatype,
                datatype_b,
                a,
                b,
                key_a,
                key_b,
                order,
            }
        })
        .collect()
}

/// Wiring-only diagnostic entry point; all lexical constraints remain in core.
#[allow(dead_code)]
pub fn program(datatype: &str, bucket: usize) -> String {
    let identifier = sparq_zk::field::field_to_hex(&sparq_merkle_suite::poseidon::hash_bytes(
        sparq_merkle_suite::poseidon::DATATYPE_DOMAIN,
        format!("http://www.w3.org/2001/XMLSchema#{datatype}").as_bytes(),
    ));
    format!(
        r#"
use std::collections::bounded_vec::BoundedVec;
fn main(bytes: [u8; {bucket}], len: u32, key: [u8;31], other: Field, expected: u8) {{
 let lexical = BoundedVec::from_parts_unchecked(bytes, len);
 let dt: BoundedVec<u8,64> = BoundedVec::from_array("http://www.w3.org/2001/XMLSchema#{datatype}".as_bytes());
 let lang: BoundedVec<u8,1> = BoundedVec::new();
 let cell = core::term::open_term(3, lexical, dt, lang, key, 0);
 let value = core::term::recompute_key_for(cell, lexical, dt, lang, {identifier}).cell().key();
 if expected == 1 {{ assert(core::term::field_lt(value,other), "engine less-than differs"); }}
 if expected == 2 {{ assert(value == other, "engine equality differs"); }}
 if expected == 3 {{ assert(core::term::field_lt(other,value), "engine greater-than differs"); }}
}}
"#
    )
}

/// Each pair is executed in both directions. The candidate key is constrained
/// to its own lexical form, and the peer key is checked in the other execution.
#[allow(dead_code)]
pub fn fixtures() -> std::collections::BTreeMap<(String, usize), Vec<serde_json::Value>> {
    use ark_ff::PrimeField as _;
    let mut groups = std::collections::BTreeMap::<(String, usize), Vec<serde_json::Value>>::new();
    for c in cases() {
        for (dt, literal, key, peer, order) in [
            (c.datatype, &c.a, c.key_a, c.key_b, c.order),
            (
                c.datatype_b.as_str(),
                &c.b,
                c.key_b,
                c.key_a,
                c.order.map(std::cmp::Ordering::reverse),
            ),
        ] {
            let len = literal.value().len();
            let [short, medium, long] = sparq_merkle_suite::metadata::LEXICAL_BUCKETS;
            assert!(len <= long);
            let bucket = if len <= short {
                short
            } else if len <= medium {
                medium
            } else {
                long
            };
            let mut bytes = literal.value().as_bytes().to_vec();
            bytes.resize(bucket, 0);
            let other = sparq_zk::field::field_to_hex(
                &sparq_merkle_suite::Fr::from_be_bytes_mod_order(&peer),
            );
            let input = serde_json::json!({"bytes":bytes,"len":len,"key":key,"other":other,
                "expected":order.map_or(0,|o| if o.is_lt(){1}else if o.is_eq(){2}else{3})});
            groups
                .entry((dt.to_owned(), bucket))
                .or_default()
                .push(input);
        }
    }
    groups
}
