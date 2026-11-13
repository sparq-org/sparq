use oxrdf::{Literal, NamedNode, Quad};
use sparq_merkle_suite::{comparison_key, key_class as k, poseidon as p, sha256 as s, Fr};
use sparq_zk::{field::field_to_hex, sig};

// The frozen source is read, never rewritten or added to the detached graph.
pub use sparq_merkle_suite::Error as Rejected;
mod authenticated_rdf {
    pub type SignedCredential = sparq_merkle_suite::SignedCredential;
}
#[allow(dead_code)]
#[path = "../../../zk/sparql-evaluator/model/src/merkle_suite.rs"]
mod frozen_sha256;

fn typed(text: &str, dt: &str) -> Literal {
    Literal::new_typed_literal(
        text,
        NamedNode::new_unchecked(format!("http://www.w3.org/2001/XMLSchema#{dt}")),
    )
}
fn quad(text: &str) -> Quad {
    oxttl::NQuadsParser::new()
        .for_slice(text.as_bytes())
        .next()
        .unwrap()
        .unwrap()
}
const DOC: &str = "_:b <http://ex/balance> \"1250.50\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n<urn:vc:1> <http://ex/name> \"Alice\"@en .\n";
const CONFIG: &str =
    "_:p <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n";

#[test]
fn frozen_sha256_copy_matches_existing_vectors_and_new_source() {
    let a = quad(
        "_:c14n0 <http://ex/balance> \"1250.50\"^^<http://www.w3.org/2001/XMLSchema#decimal> .",
    );
    let b = quad("<urn:vc:1> <http://ex/name> \"Alice\"@en .");
    let leaf = s::leaf(&a).unwrap();
    assert_eq!(leaf, frozen_sha256::leaf(&a).unwrap());
    assert_eq!(
        leaf.iter().map(|x| format!("{x:02x}")).collect::<String>(),
        "8c4a491021d82e07c77c748df8c123d9721163cb8da614c7d21a4aef901c1146"
    );
    let mut leaves = [leaf, s::leaf(&b).unwrap()];
    leaves.sort();
    let root = s::root(&leaves).unwrap();
    assert_eq!(Some(root), frozen_sha256::root(&leaves));
    assert_eq!(
        root.iter().map(|x| format!("{x:02x}")).collect::<String>(),
        "311072a0f565868c243e392ea085e4c2fe752b39751e0707bcd7e42c78683682"
    );
    let msg = s::signed_message(&[7; 32], 2, &root, &[9; 32]);
    assert_eq!(
        msg,
        frozen_sha256::signed_message(&[7; 32], 2, &root, &[9; 32])
    );
    assert_eq!(
        msg.iter().map(|x| format!("{x:02x}")).collect::<String>(),
        "d79eb8afe1cbb6c7c146089a2522aa54a6a875176585d135a39d129d50be81a8"
    );
    for (dt, values) in [
        ("integer", vec!["-1", "0", "+005", "bad"]),
        ("decimal", vec!["-.0", "0.0000000000000000001", "1250.50"]),
        ("double", vec!["-INF", "-0", "0.1", "NaN", "1e300"]),
        ("float", vec!["0.1", "1e-50"]),
        (
            "dateTime",
            vec!["2026-01-01T00:00:00Z", "-0044-03-15T12:00:00Z"],
        ),
        ("boolean", vec!["false", "true", "bad"]),
        ("string", vec!["", "é", "abc\0"]),
    ] {
        for text in values {
            let l = typed(text, dt);
            assert_eq!(comparison_key(&l), frozen_sha256::comparison_key(&l), "{l}");
        }
    }
}

#[test]
fn derived_integer_classes_validate_the_value_space() {
    for dt in [
        "integer",
        "nonPositiveInteger",
        "nonNegativeInteger",
        "long",
        "int",
        "short",
        "byte",
        "unsignedLong",
        "unsignedInt",
        "unsignedShort",
        "unsignedByte",
    ] {
        assert_eq!(
            comparison_key(&typed("-0", dt)),
            comparison_key(&typed("0", "decimal")),
            "{dt}"
        );
    }
    for (dt, good, bad) in [
        ("negativeInteger", "-1", "0"),
        ("positiveInteger", "1", "0"),
        ("byte", "127", "128"),
        ("unsignedByte", "255", "256"),
        (
            "unsignedLong",
            "18446744073709551615",
            "18446744073709551616",
        ),
        ("int", "-2147483648", "-2147483649"),
    ] {
        assert_eq!(comparison_key(&typed(good, dt))[0], k::DECIMAL);
        assert_eq!(comparison_key(&typed(bad, dt))[0], k::NONE);
    }
    assert_eq!(
        comparison_key(&typed(&"9".repeat(100), "positiveInteger"))[0],
        k::UNREPRESENTABLE
    );
    assert_eq!(
        comparison_key(&typed(&format!("-{}", "9".repeat(100)), "negativeInteger"))[0],
        k::UNREPRESENTABLE
    );
}

#[test]
fn date_keys_are_calendar_ordered_and_timezones_unrepresentable() {
    let a = comparison_key(&typed("1969-12-31", "date"));
    let b = comparison_key(&typed("1970-01-01", "date"));
    assert_eq!(a[0], k::DATE);
    assert!(a < b);
    for date in [
        "2026-01-01Z",
        "2026-01-01+01:00",
        "2026-01-01-01:00",
        "0000-01-01",
        "10000-01-01",
        "-0044-03-15",
    ] {
        assert_eq!(
            comparison_key(&typed(date, "date"))[0],
            k::UNREPRESENTABLE,
            "{date}"
        );
    }
    for date in ["2026-02-29", "2026-01-01+15:00", "02026-01-01"] {
        assert_eq!(comparison_key(&typed(date, "date"))[0], k::NONE, "{date}");
    }
}

#[test]
fn poseidon_vector_and_length_binding() {
    let q = p::quad_hash(&quad(
        "_:c14n0 <http://ex/balance> \"1250.50\"^^<http://www.w3.org/2001/XMLSchema#decimal> .",
    ))
    .unwrap();
    assert_eq!(
        field_to_hex(&q),
        "0x18b29490f68c4af6b3b2d96e0824344927f4f541e6dec5bb2802f3e288c5c1a2"
    );
    let root = p::root(&[q], Fr::from(7u64)).unwrap();
    assert_eq!(
        field_to_hex(&root),
        "0x0a6bb130a5e4691b0f13a1abe83604e81f62a55e3c0f91ef15c859dee31c83ff"
    );
    assert_eq!(
        field_to_hex(&p::signed_message(
            Fr::from(7u64),
            1,
            root,
            &[9; 32],
            Fr::from(0u64)
        )),
        "0x11d3772b6e07a2d71fb63ccef479fc94c43e6c8837cf7e387737043504aed322"
    );
    assert_ne!(
        p::hash_bytes(p::TERM_DOMAIN, &[1]),
        p::hash_bytes(p::TERM_DOMAIN, &[0, 1])
    );
    assert_ne!(
        p::hash_bytes(p::TERM_DOMAIN, &[1]),
        p::hash_bytes(p::TERM_DOMAIN, &[1, 0])
    );
    assert_ne!(
        p::hash_bytes(p::TERM_DOMAIN, &[1]),
        p::hash_bytes(p::LANGUAGE_DOMAIN, &[1])
    );
}

#[test]
fn both_members_issue_and_verify() {
    let sk = sig::SecretKey::from_seed(19);
    let c = p::issue(DOC, CONFIG, &sk, Fr::from(7u64)).unwrap();
    assert!(p::verify(&c, &sk.public_key()));
    assert!(p::verify_and_open(&c, &sk.public_key()).is_ok());
    assert!(p::verify_and_open(&c, &sig::SecretKey::from_seed(20).public_key()).is_err());
    let sk = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let c = s::issue(
        DOC,
        &CONFIG.replace(
            p::SCHNORR_POSEIDON2_MERKLE_2026,
            s::EDDSA_SHA256_MERKLE_2026,
        ),
        &sk,
        [7; 32],
    )
    .unwrap();
    assert!(s::verify(&c, &sk.verifying_key()));
    let frozen = frozen_sha256::issue(
        DOC,
        &CONFIG.replace(
            p::SCHNORR_POSEIDON2_MERKLE_2026,
            s::EDDSA_SHA256_MERKLE_2026,
        ),
        &sk,
        [7; 32],
    )
    .unwrap();
    assert_eq!(c, frozen);
}

#[test]
fn duplicated_quad_is_rejected() {
    assert!(p::root(&[Fr::from(1u64); 2], Fr::from(7u64)).is_err());
}
#[test]
fn out_of_order_quads_are_rejected() {
    assert!(p::root(&[Fr::from(2u64), Fr::from(1u64)], Fr::from(7u64)).is_err());
}
#[test]
fn wrong_count_is_rejected() {
    let sk = sig::SecretKey::from_seed(19);
    let mut c = p::issue(DOC, CONFIG, &sk, Fr::from(7u64)).unwrap();
    c.count += 1;
    assert!(!p::verify(&c, &sk.public_key()));
}
#[test]
fn signature_under_another_key_is_rejected() {
    let sk = sig::SecretKey::from_seed(19);
    let c = p::issue(DOC, CONFIG, &sk, Fr::from(7u64)).unwrap();
    assert!(!p::verify(&c, &sig::SecretKey::from_seed(20).public_key()));
}
#[test]
fn message_for_another_suite_is_rejected() {
    let sk = sig::SecretKey::from_seed(19);
    let c = p::issue(DOC, CONFIG, &sk, Fr::from(7u64)).unwrap();
    assert!(!sig::verify(
        &sk.public_key(),
        &p::message_for_suite(
            Fr::from(1u64),
            c.salt,
            c.count,
            c.root,
            &[9; 32],
            Fr::from(0u64)
        ),
        &c.signature
    ));
}
#[test]
fn message_for_another_configuration_is_rejected() {
    let sk = sig::SecretKey::from_seed(19);
    let mut c = p::issue(DOC, CONFIG, &sk, Fr::from(7u64)).unwrap();
    c.proof_config =
        sparq_canon::canonicalize_nquads(&format!("{}<urn:p> <urn:p> <urn:p> .\n", c.proof_config))
            .unwrap();
    assert!(!p::verify(&c, &sk.public_key()));
}
#[test]
fn blank_node_label_reused_across_term_kinds_is_distinct() {
    use oxrdf::{BlankNode, Term};
    let b = p::term_cell(&Term::BlankNode(BlankNode::new_unchecked("same")), 1).unwrap();
    let iri = p::term_cell(&Term::NamedNode(NamedNode::new_unchecked("same")), 1).unwrap();
    let l = p::term_cell(&Term::Literal(Literal::new_simple_literal("same")), 1).unwrap();
    assert_ne!(b.enc, iri.enc);
    assert_ne!(b.enc, l.enc);
    assert_ne!(iri.enc, l.enc);
    assert_eq!(b.credential, 1);
    assert_eq!(
        p::term_cell(&Term::BlankNode(BlankNode::new_unchecked("same")), 2)
            .unwrap()
            .credential,
        2
    );
}

#[path = "support/cases.rs"]
mod cases;
#[test]
fn random_key_classes_agree_with_engine_comparison() {
    let cases = cases::cases();
    let classes: std::collections::BTreeSet<_> = cases
        .iter()
        .flat_map(|c| [c.key_a[0], c.key_b[0]])
        .collect();
    assert_eq!(
        classes,
        [
            k::NONE,
            k::DECIMAL,
            k::DOUBLE,
            k::DATE_TIME,
            k::LOCAL_DATE_TIME,
            k::BOOLEAN,
            k::STRING,
            k::STRING_PREFIX,
            k::DATE,
            k::UNREPRESENTABLE
        ]
        .into_iter()
        .collect()
    );
    assert!(cases.iter().any(|c| c.order.is_some()));
    assert!(cases.iter().all(|c| !c.datatype.is_empty()
        && !c.datatype_b.is_empty()
        && !c.a.datatype().as_str().is_empty()
        && !c.b.datatype().as_str().is_empty()));
}
