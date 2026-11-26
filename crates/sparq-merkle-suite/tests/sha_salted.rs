#![cfg(feature = "sha256-salted")]

use ed25519_dalek::{Signer, SigningKey};
use oxrdf::{GraphName, Literal, NamedNode, Quad, Term};
use sha2::{Digest, Sha256};
use sparq_merkle_suite::{comparison_key, sha256, sha256_salted as salted, SignedCredential};

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const SUITE: &str = "eddsa-sha256-merkle-salted-2026";
const DATE: &str = "<urn:s> <urn:p> \"2000-02-29\"^^<http://www.w3.org/2001/XMLSchema#date> .\n";
const LANG: &str = "<urn:s> <urn:p> \"café\"@fr .\n";
const INTEGER: &str =
    "<urn:s> <urn:p> \"+0255\"^^<http://www.w3.org/2001/XMLSchema#unsignedByte> <urn:g> .\n";

fn typed(text: &str, datatype: &str) -> Literal {
    Literal::new_typed_literal(text, NamedNode::new(format!("{XSD}{datatype}")).unwrap())
}

fn assert_committed_key(literal: &Literal, expected_key: [u8; 31]) {
    fn frame(bytes: &mut Vec<u8>, text: &str) {
        bytes.extend_from_slice(&(text.len() as u32).to_be_bytes());
        bytes.extend_from_slice(text.as_bytes());
    }

    let quad = Quad::new(
        NamedNode::new("urn:s").unwrap(),
        NamedNode::new("urn:p").unwrap(),
        literal.clone(),
        GraphName::DefaultGraph,
    );
    let mut encoded =
        b"eddsa-sha256-merkle-salted-2026/quad\0\x01\0\0\0\x05urn:s\x01\0\0\0\x05urn:p\x03"
            .to_vec();
    frame(&mut encoded, literal.value());
    frame(&mut encoded, literal.datatype().as_str());
    frame(&mut encoded, literal.language().unwrap_or(""));
    encoded.extend_from_slice(&expected_key);
    encoded.push(0);
    assert_eq!(comparison_key(literal), expected_key, "{literal}");
    let expected: [u8; 32] = Sha256::digest(encoded).into();
    assert_eq!(salted::quad_hash(&quad), Some(expected), "{literal}");
}

#[test]
fn successor_commits_checked_calendar_keys() {
    for (lexical, day) in [
        ("0001-01-01", -719_162_i64),
        ("1900-02-28", -25_509),
        ("1969-12-31", -1),
        ("1970-01-01", 0),
        ("2000-02-29", 11_016),
        ("2100-03-01", 47_541),
        ("2400-02-29", 157_113),
        ("9999-12-31", 2_932_896),
    ] {
        let mut expected = [0; 31];
        expected[0] = 0x08;
        expected[1..9].copy_from_slice(&((i128::from(day) + (1 << 63)) as u64).to_be_bytes());
        assert_committed_key(&typed(lexical, "date"), expected);
    }
    let mut unrepresentable = [0; 31];
    unrepresentable[0] = 0x7f;
    for lexical in [
        "2000-02-29Z",
        "2000-02-29+00:00",
        "2000-02-29-00:00",
        "2000-02-29+14:00",
        "2000-02-29-14:00",
        "0000-02-29",
        "-0400-02-29",
        "10400-02-29",
    ] {
        assert_committed_key(&typed(lexical, "date"), unrepresentable);
    }
    for lexical in [
        "1900-02-29",
        "2100-02-29Z",
        "10401-02-29",
        "02000-02-29",
        "2000-00-29",
        "2000-02-30",
        "2000-2-29",
        "2000-02-29+14:01",
        "2000-02-29-14:01",
        "2000-02-29+01:60",
        "2000-02-29+1:00",
        "2000-02-29z",
        "2000-02-29T00:00:00Z",
    ] {
        assert_committed_key(&typed(lexical, "date"), [0; 31]);
    }
}

fn decimal_key(value: i128) -> [u8; 31] {
    let mut key = [0; 31];
    key[0] = 1;
    if value < 0 {
        key[1] = 0x7f;
        key[2..15].fill(0xff);
    } else {
        key[1] = 0x80;
    }
    key[15..].copy_from_slice(&(value * 10_i128.pow(18)).to_be_bytes());
    key
}

#[test]
fn successor_commits_range_checked_integer_subtypes() {
    for (datatype, min, max) in [
        ("byte", i128::from(i8::MIN), i128::from(i8::MAX)),
        ("short", i128::from(i16::MIN), i128::from(i16::MAX)),
        ("int", i128::from(i32::MIN), i128::from(i32::MAX)),
        ("long", i128::from(i64::MIN), i128::from(i64::MAX)),
        ("unsignedByte", 0, i128::from(u8::MAX)),
        ("unsignedShort", 0, i128::from(u16::MAX)),
        ("unsignedInt", 0, i128::from(u32::MAX)),
        ("unsignedLong", 0, i128::from(u64::MAX)),
    ] {
        for value in [min, max] {
            assert_committed_key(&typed(&value.to_string(), datatype), decimal_key(value));
        }
        for value in [min - 1, max + 1] {
            assert_committed_key(&typed(&value.to_string(), datatype), [0; 31]);
        }
    }
    for (datatype, lexical, value, invalid) in [
        ("nonPositiveInteger", "-00", 0, "+01"),
        ("negativeInteger", "-01", -1, "-00"),
        ("nonNegativeInteger", "-00", 0, "-01"),
        ("positiveInteger", "+01", 1, "+00"),
    ] {
        assert_committed_key(&typed(lexical, datatype), decimal_key(value));
        assert_committed_key(&typed(invalid, datatype), [0; 31]);
    }
    for lexical in ["", " 255", "255.0", "2e2", "２５５"] {
        assert_committed_key(&typed(lexical, "unsignedByte"), [0; 31]);
    }
    let mut unrepresentable = [0; 31];
    unrepresentable[0] = 0x7f;
    let large = "9".repeat(60);
    assert_committed_key(&typed(&large, "positiveInteger"), unrepresentable);
    assert_committed_key(&typed(&large, "unsignedLong"), [0; 31]);
}

fn hashes(document: &str) -> Vec<[u8; 32]> {
    oxttl::NQuadsParser::new()
        .for_slice(document.as_bytes())
        .map(|quad| salted::quad_hash(&quad.unwrap()).unwrap())
        .collect()
}

fn using_suite(id: &str) -> String {
    format!("<urn:p> <https://w3id.org/security#cryptosuite> \"{id}\" .\n")
}

fn hex<const N: usize>(text: &str) -> [u8; N] {
    assert_eq!(text.len(), 2 * N);
    std::array::from_fn(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
}

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&hex(
        "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
    ))
}

#[test]
fn equal_comparison_keys_preserve_distinct_rdf_terms() {
    let objects = [
        typed("+0255", "unsignedByte"),
        typed("255", "unsignedByte"),
        typed("255", "integer"),
        typed("255.00", "decimal"),
    ];
    let mut document = String::new();
    for object in &objects {
        assert_eq!(comparison_key(object), decimal_key(255));
        let term = Term::from(object.clone());
        document.push_str(&format!("<urn:s> <urn:p> {term} .\n"));
    }
    let key = signing_key();
    let credential = salted::issue(&document, &using_suite(SUITE), &key, [7; 32]).unwrap();
    let committed: std::collections::HashSet<_> = oxttl::NQuadsParser::new()
        .for_slice(credential.document.as_bytes())
        .map(|quad| quad.unwrap().object)
        .collect();
    assert_eq!(committed, objects.into_iter().map(Term::from).collect());
    assert_eq!(committed.len(), 4);
    assert!(salted::verify(&credential, &key.verifying_key()));
}

#[test]
fn independent_successor_vectors_pin_order_leaves_root_and_issuance() {
    let document = [INTEGER, DATE, LANG].concat();
    let ordered = [DATE, LANG, INTEGER].concat();
    let config = using_suite(SUITE);
    let digest = hex("e288615f086849a7af1ccf66fa6c282e66d6fa5230d34713ad98505e7a21cd19");
    let key = signing_key();
    let quads = hashes(&ordered);
    assert_eq!(
        quads,
        [
            hex("134823ee4a3e5be57e9e691bb473c26e3acb3e8010e1e1ef70e0bbae74f203e4"),
            hex("5bc85d70b5b815b7dccd37680ffb5aea52d752ab2136147f53f95d77d529b57f"),
            hex("d6f4489e0f3e5567fd518d19af26266090afeb03805c66ae3d0f4030de02bf91"),
        ],
    );
    let mut previous = None;
    for (salt, first_leaf_salt, leaves, root, message, signature) in [
        (
            [7; 32],
            hex("6e131a1aa2cc94415b81ff499a4f5c1009aee64b931f95739333943ae5d9efda"),
            [
                hex("a6d469f15e349d40c6d2356e4794ecf10857fee03835bcd9aa8e9fee7189fba7"),
                hex("415a80b73c3851ac7e798f07120b1130bce018c8d1dfc9be2b244882f7236aac"),
                hex("0fe3809b47e290217e53b3df5771d49b20efbc87cc5f418a7f8bb8d578e47aa2"),
            ],
            hex("fc4a4ce8668698ac3b558df26d761d77fbf270ae1b510a0dd111d60c33324fbe"),
            hex("7ada8086e99b1c2fabeffd699576cde588612b558f875b6eb707965f1f31e326"),
            hex::<64>(concat!(
                "e9306fc81e70d844b766670b7b213ced2ee477eea991a04f45bcc704b47d28fc19",
                "1f5756e91f0db8f9d1b3030136d3d2dc7abf9208cee6ae2086802a6f8da600",
            )),
        ),
        (
            [8; 32],
            hex("df2338385c722beea85ee26c0192a52c0e360950d6b3c1101f8a329f5754563d"),
            [
                hex("81ace4007c52121943af5278ad250816717084669af6193c1d6a9316b688870c"),
                hex("4dd5286cfe993581a62546014bd8f44260c219df77a8df53c28416fe49ef8516"),
                hex("b2b12e4be99c2fef83af86f34d9cfe86a59737223d4d5b8820c6881098e00a8c"),
            ],
            hex("0a86edfe390fd862543869e3976c3d68dcb97a7a6fb2ffaca94560d671f79ee4"),
            hex("106b206b806737233d2c58482c14824dd942b1cf3a6cc338466caf5caed28a0a"),
            hex::<64>(concat!(
                "11a804b1bf74a15a9338ca7b1288a597ba30195762ab907b18c618b7b5742fce67",
                "db35b0da07eccce14f4fe32390d84fef8d58821fa3a979a27831eee9735b02",
            )),
        ),
    ] {
        assert_eq!(salted::leaf_salt(&salt, 0), first_leaf_salt);
        assert_eq!(
            std::array::from_fn::<_, 3, _>(|i| salted::leaf(&quads[i], &salt, i as u32)),
            leaves,
        );
        assert_eq!(salted::root(&quads, &salt), Some(root));
        assert_eq!(salted::root(&quads[..1], &salt), Some(leaves[0]));
        assert_eq!(salted::signed_message(&salt, 3, &root, &digest), message);
        assert_ne!(sha256::signed_message(&salt, 3, &root, &digest), message);
        if let Some((old_leaves, old_root)) = previous {
            assert!(leaves.iter().zip(old_leaves).all(|(a, b)| *a != b));
            assert_ne!(root, old_root);
        }
        previous = Some((leaves, root));

        let expected = SignedCredential {
            document: ordered.clone(),
            proof_config: config.clone(),
            signature: [signature.as_slice(), salt.as_slice()].concat(),
        };
        assert!(salted::verify(&expected, &key.verifying_key()));
        assert_eq!(
            salted::issue(&document, &config, &key, salt).unwrap(),
            expected
        );
        assert_eq!(
            salted::issue(&ordered, &config, &key, salt).unwrap(),
            expected
        );
    }
    for unordered in [vec![], vec![quads[1], quads[0]], vec![quads[0]; 2]] {
        assert_eq!(salted::root(&unordered, &[7; 32]), None);
    }
}

#[test]
fn each_edition_refuses_the_other_edition() {
    let key = signing_key();
    let pk = key.verifying_key();
    let legacy_config = using_suite(sha256::EDDSA_SHA256_MERKLE_2026);
    let successor_config = using_suite(SUITE);
    let legacy = sha256::issue(DATE, &legacy_config, &key, [7; 32]).unwrap();
    let successor = salted::issue(DATE, &successor_config, &key, [7; 32]).unwrap();
    assert!(sha256::verify(&legacy, &pk));
    assert!(salted::verify(&successor, &pk));
    assert!(!sha256::verify(&successor, &pk));
    assert!(!salted::verify(&legacy, &pk));
    let relabelled = |mut c: SignedCredential, config: &str| {
        c.proof_config = config.to_owned();
        c
    };
    assert!(!sha256::verify(&relabelled(successor, &legacy_config), &pk));
    assert!(!salted::verify(&relabelled(legacy, &successor_config), &pk));
    assert!(sha256::issue(DATE, &successor_config, &key, [7; 32]).is_err());
    assert!(salted::issue(DATE, &legacy_config, &key, [7; 32]).is_err());
}

#[test]
fn successor_verifier_requires_its_suite_in_the_signed_configuration() {
    let key = signing_key();
    let salt = [7; 32];
    let legacy_config = using_suite(sha256::EDDSA_SHA256_MERKLE_2026);
    let root = salted::root(&hashes(DATE), &salt).unwrap();
    let digest = Sha256::digest(legacy_config.as_bytes()).into();
    let signature = key.sign(&salted::signed_message(&salt, 1, &root, &digest));
    let credential = SignedCredential {
        document: DATE.to_owned(),
        proof_config: legacy_config,
        signature: [signature.to_bytes().as_slice(), salt.as_slice()].concat(),
    };
    assert!(!salted::verify(&credential, &key.verifying_key()));
}

#[test]
fn successor_rechecks_canonical_document_configuration_and_proof_value() {
    let key = signing_key();
    let pk = key.verifying_key();
    let config = using_suite(SUITE);
    let document = [DATE, LANG, INTEGER].concat();
    let credential = salted::issue(&document, &config, &key, [7; 32]).unwrap();
    for invalid_document in [
        [LANG, DATE, INTEGER].concat(),
        [DATE, LANG, LANG, INTEGER].concat(),
        credential.document.trim_end().to_owned(),
        credential.document.replace("café", "cafe"),
    ] {
        let mut altered = credential.clone();
        altered.document = invalid_document;
        assert!(!salted::verify(&altered, &pk));
    }
    let mut altered = credential.clone();
    altered
        .proof_config
        .push_str("<urn:z> <urn:option> \"changed\" .\n");
    assert!(!salted::verify(&altered, &pk));
    altered.proof_config = format!("\n{config}");
    assert!(!salted::verify(&altered, &pk));

    for index in [0, 63, 64, 95] {
        let mut altered = credential.clone();
        altered.signature[index] ^= 1;
        assert!(!salted::verify(&altered, &pk));
    }
    let mut altered = credential.clone();
    altered.signature.push(0);
    assert!(!salted::verify(&altered, &pk));
    altered.signature.truncate(95);
    assert!(!salted::verify(&altered, &pk));
    assert!(!salted::verify(
        &credential,
        &SigningKey::from_bytes(&[2; 32]).verifying_key()
    ));
    assert!(salted::issue("", &config, &key, [7; 32]).is_err());
}

#[test]
fn successor_canonicalizes_blank_nodes_before_ordering() {
    let key = signing_key();
    let salt = [7; 32];
    let config = using_suite(SUITE);
    let mut credential =
        salted::issue("_:issuer <urn:p> <urn:o> .\n", &config, &key, salt).unwrap();
    assert_eq!(credential.document, "_:c14n0 <urn:p> <urn:o> .\n");
    assert_eq!(
        credential,
        salted::issue("_:renamed <urn:p> <urn:o> .\n", &config, &key, salt).unwrap(),
    );
    assert!(salted::verify(&credential, &key.verifying_key()));
    credential.document = credential.document.replace("_:c14n0", "_:renamed");
    assert!(!salted::verify(&credential, &key.verifying_key()));
}
