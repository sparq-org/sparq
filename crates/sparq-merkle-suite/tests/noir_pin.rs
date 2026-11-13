use sparq_merkle_suite::{key_class as k, poseidon as p, Fr};
use sparq_zk::field::field_from_hex_str_strict;
use std::collections::BTreeMap;
const SOURCE: &str = include_str!("../../../zk/noir-query/core/src/suite_constants.nr");
#[test]
fn every_noir_constant_equals_its_rust_source() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../zk/noir-query/core/src");
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "nr") && path.file_name().unwrap() != "suite_constants.nr" {
            let source = std::fs::read_to_string(&path).unwrap();
            assert!(!source.lines().map(|line| line.split("//").next().unwrap()).flat_map(str::split_whitespace).any(|word| word == "global"), "declare every protocol constant in suite_constants: {}",path.display());
        }
    }
    let mut actual = BTreeMap::new();
    assert!(!SOURCE.contains("/*"));
    for declaration in SOURCE.split("pub global ").skip(1) {
        let (name, rest) = declaration.split_once(':').unwrap();
        let (_, rest) = rest.split_once('=').unwrap();
        let value = rest.split_once(';').unwrap().0.trim();
        let value = if value.starts_with("0x") {
            field_from_hex_str_strict(value).unwrap()
        } else {
            Fr::from(value.parse::<u64>().unwrap())
        };
        assert!(actual.insert(name.to_owned(), value).is_none());
    }
    assert_eq!(
        SOURCE
            .lines()
            .map(|l| l.split("//").next().unwrap())
            .flat_map(str::split_whitespace)
            .filter(|w| *w == "global")
            .count(),
        actual.len()
    );
    let mut expected = BTreeMap::new();
    for (name, value) in [
        ("TERM_DOMAIN", p::TERM_DOMAIN),
        ("QUAD_DOMAIN", p::QUAD_DOMAIN),
        ("SALT_DOMAIN", p::SALT_DOMAIN),
        ("LEAF_DOMAIN", p::LEAF_DOMAIN),
        ("NODE_DOMAIN", p::NODE_DOMAIN),
        ("MESSAGE_DOMAIN", p::MESSAGE_DOMAIN),
        ("DATATYPE_DOMAIN", p::DATATYPE_DOMAIN),
        ("LANGUAGE_DOMAIN", p::LANGUAGE_DOMAIN),
        ("SUITE_DOMAIN", p::SUITE_DOMAIN),
        ("PACK_BYTES", p::PACK_BYTES as u64),
        ("KEY_BYTES", sparq_merkle_suite::KEY_BYTES as u64),
    ] {
        expected.insert(name.to_owned(), Fr::from(value));
    }
    for (name, value) in [
        ("NONE", k::NONE),
        ("DECIMAL", k::DECIMAL),
        ("DOUBLE", k::DOUBLE),
        ("DATE_TIME", k::DATE_TIME),
        ("LOCAL_DATE_TIME", k::LOCAL_DATE_TIME),
        ("BOOLEAN", k::BOOLEAN),
        ("STRING", k::STRING),
        ("STRING_PREFIX", k::STRING_PREFIX),
        ("DATE", k::DATE),
        ("UNREPRESENTABLE", k::UNREPRESENTABLE),
    ] {
        expected.insert(format!("CLASS_{name}"), Fr::from(value));
    }
    for dt in [
        "integer",
        "decimal",
        "double",
        "float",
        "dateTime",
        "boolean",
        "string",
        "date",
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
        expected.insert(
            format!("DT_{}", dt.to_ascii_uppercase()),
            p::hash_bytes(
                p::DATATYPE_DOMAIN,
                format!("http://www.w3.org/2001/XMLSchema#{dt}").as_bytes(),
            ),
        );
    }
    for (name, value) in [
        (
            "KIND_DEFAULT_GRAPH",
            sparq_merkle_suite::term_kind::DEFAULT_GRAPH,
        ),
        ("KIND_IRI", sparq_merkle_suite::term_kind::IRI),
        ("KIND_BLANK_NODE", sparq_merkle_suite::term_kind::BLANK_NODE),
        ("KIND_LITERAL", sparq_merkle_suite::term_kind::LITERAL),
    ] {
        expected.insert(name.to_owned(), Fr::from(value));
    }
    expected.insert(
        "DT_LANGSTRING".to_owned(),
        p::hash_bytes(
            p::DATATYPE_DOMAIN,
            b"http://www.w3.org/1999/02/22-rdf-syntax-ns#langString",
        ),
    );
    for (name, value) in [
        ("METADATA_DOMAIN", sparq_merkle_suite::metadata::METADATA_DOMAIN),
        ("GRAPH_DOMAIN", sparq_merkle_suite::metadata::GRAPH_DOMAIN),
        ("MULTIKEY_CODEC", sparq_merkle_suite::metadata::MULTIKEY_CODEC as u64),
        ("MULTIKEY_LENGTH", sparq_merkle_suite::metadata::MULTIKEY_LENGTH as u64),
        ("LEXICAL_SHORT", sparq_merkle_suite::metadata::LEXICAL_BUCKETS[0] as u64),
        ("LEXICAL_MEDIUM", sparq_merkle_suite::metadata::LEXICAL_BUCKETS[1] as u64),
        ("LEXICAL_LONG", sparq_merkle_suite::metadata::LEXICAL_BUCKETS[2] as u64),
        ("DATATYPE_CAPACITY", sparq_merkle_suite::metadata::DATATYPE_CAPACITY as u64),
        ("LANGUAGE_CAPACITY", sparq_merkle_suite::metadata::LANGUAGE_CAPACITY as u64),
    ] { expected.insert(name.to_owned(), Fr::from(value)); }
    for (i, b) in sparq_merkle_suite::metadata::MULTIKEY_PREFIX.iter().enumerate() {
        expected.insert(format!("MULTIKEY_PREFIX_{i}"), Fr::from(*b));
    }
    for (name, value) in [
        ("LANG_EMPTY", p::hash_bytes(p::LANGUAGE_DOMAIN, b"")),
        ("LANG_EN", p::hash_bytes(p::LANGUAGE_DOMAIN, b"en")),
        ("DEFAULT_GRAPH_ENC", p::default_graph().enc),
        ("DT_OTHER", p::hash_bytes(p::DATATYPE_DOMAIN, b"http://www.w3.org/2001/XMLSchema#other")),
    ] { expected.insert(name.to_owned(), value); }
    for (name, iri) in sparq_merkle_suite::metadata::VOCABULARY {
        expected.insert(format!("IRI_{name}"), p::term_cell(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(*iri)), 0).unwrap().enc);
    }
    expected.insert("GRAPH_VECTOR".to_owned(), sparq_merkle_suite::metadata::graph_name([11,13],1).unwrap().enc);
    expected.insert("SUITE_ID".to_owned(), p::suite_id());
    assert_eq!(actual, expected);
    let tags = [
        p::TERM_DOMAIN,
        p::QUAD_DOMAIN,
        p::SALT_DOMAIN,
        p::LEAF_DOMAIN,
        p::NODE_DOMAIN,
        p::MESSAGE_DOMAIN,
        p::DATATYPE_DOMAIN,
        p::LANGUAGE_DOMAIN,
        p::SUITE_DOMAIN,
        sparq_merkle_suite::metadata::METADATA_DOMAIN,
        sparq_merkle_suite::metadata::GRAPH_DOMAIN,
    ];
    for (i, a) in tags.iter().enumerate() {
        for b in &tags[i + 1..] {
            assert_ne!(a, b);
        }
    }
}
