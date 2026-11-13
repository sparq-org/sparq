//! Emit deterministic suite vectors for native/Noir compatibility checks.
use oxrdf::{Literal, NamedNode, Term};
use sparq_merkle_suite::{comparison_key, poseidon as p, Fr};
use sparq_zk::field::field_to_hex;
fn main() {
    let datatypes = [
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
    ];
    for dt in datatypes {
        println!(
            "DT_{}={}",
            dt.to_ascii_uppercase(),
            field_to_hex(&p::hash_bytes(
                p::DATATYPE_DOMAIN,
                format!("http://www.w3.org/2001/XMLSchema#{dt}").as_bytes()
            ))
        );
    }
    println!(
        "DT_LANGSTRING={}",
        field_to_hex(&p::hash_bytes(
            p::DATATYPE_DOMAIN,
            b"http://www.w3.org/1999/02/22-rdf-syntax-ns#langString"
        ))
    );
    let t = Term::Literal(Literal::new_typed_literal(
        "1250.50",
        NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#decimal"),
    ));
    let c = p::term_cell(&t, 0).unwrap();
    println!("TERM={}", field_to_hex(&c.enc));
    println!(
        "TERM_HASH={}",
        field_to_hex(&p::hash_bytes(p::TERM_DOMAIN, &c.bytes))
    );
    println!("KEY={}", field_to_hex(&c.key));
    let terms = [
        Term::BlankNode(oxrdf::BlankNode::new_unchecked("c14n0")),
        Term::NamedNode(NamedNode::new_unchecked("http://ex/balance")),
        t,
    ];
    let cells = terms.map(|t| p::term_cell(&t, 0).unwrap());
    let q = p::hash_quad([
        cells[0].enc,
        cells[1].enc,
        cells[2].enc,
        p::default_graph().enc,
    ]);
    let leaf = p::leaf(q, Fr::from(7u64), 0);
    println!("QUAD={}", field_to_hex(&q));
    println!("LEAF={}", field_to_hex(&leaf));
    println!(
        "ROOT={}",
        field_to_hex(&p::root(&[q], Fr::from(7u64)).unwrap())
    );
    println!(
        "MESSAGE={}",
        field_to_hex(&p::signed_message(Fr::from(7u64), 1, leaf, &[9; 32]))
    );
    println!("SUITE_ID={}", field_to_hex(&p::suite_id()));
    println!(
        "LANG_EMPTY={}",
        field_to_hex(&p::hash_bytes(p::LANGUAGE_DOMAIN, b""))
    );
    println!(
        "LANG_EN={}",
        field_to_hex(&p::hash_bytes(p::LANGUAGE_DOMAIN, b"en"))
    );
    let distinct: [u8; 32] = std::array::from_fn(|i| i as u8);
    println!(
        "LEAF_SECOND_SALT={}",
        field_to_hex(&p::leaf(q, Fr::from(11u64), 0))
    );
    println!(
        "MESSAGE_DISTINCT_CONFIG={}",
        field_to_hex(&p::signed_message(Fr::from(7u64), 1, leaf, &distinct))
    );

    let parse = |text: &str| {
        oxttl::NQuadsParser::new()
            .for_slice(text.as_bytes())
            .next()
            .unwrap()
            .unwrap()
    };
    let other = p::quad_hash(&parse("<urn:vc:1> <http://ex/name> \"Alice\"@en .")).unwrap();
    let third = p::quad_hash(&parse("<urn:vc:1> <http://ex/name> <urn:extra> .")).unwrap();
    let mut three = [q, other, third];
    three.sort_unstable_by_key(|q| p::field_bytes(*q));
    println!(
        "ROOT_THREE={}",
        field_to_hex(&p::root(&three, Fr::from(7u64)).unwrap())
    );
    println!(
        "MESSAGE_THREE={}",
        field_to_hex(&p::signed_message(
            Fr::from(11u64),
            3,
            p::root(&three, Fr::from(11u64)).unwrap(),
            &distinct
        ))
    );

    let message = p::signed_message(Fr::from(7u64), 1, leaf, &[9; 32]);
    let sk = sparq_zk::sig::SecretKey::from_seed(19);
    let signature = sparq_zk::sig::sign_deterministic(&sk, &message);
    let witness =
        sparq_zk::sig::in_circuit_witness(&sk.public_key(), &message, &signature).unwrap();
    for (name, field) in [
        ("PK_X", witness.pk_x),
        ("PK_Y", witness.pk_y),
        ("R_X", witness.r_x),
        ("R_Y", witness.r_y),
        ("SIG_S", witness.s),
        ("SIG_E", witness.e),
        ("SIG_K", witness.e_k),
    ] {
        println!("{name}={}", field_to_hex(&field));
    }
    let wrong_suite = p::message_for_suite(
        p::suite_id() + Fr::from(1u64),
        Fr::from(7u64),
        1,
        leaf,
        &[9; 32],
    );
    let signature = sparq_zk::sig::sign_deterministic(&sk, &wrong_suite);
    let witness =
        sparq_zk::sig::in_circuit_witness(&sk.public_key(), &wrong_suite, &signature).unwrap();
    for (name, field) in [
        ("WRONG_SUITE_R_X", witness.r_x),
        ("WRONG_SUITE_R_Y", witness.r_y),
        ("WRONG_SUITE_S", witness.s),
        ("WRONG_SUITE_E", witness.e),
        ("WRONG_SUITE_K", witness.e_k),
    ] {
        println!("{name}={}", field_to_hex(&field));
    }
    let other = sparq_zk::sig::SecretKey::from_seed(20)
        .public_key()
        .coords()
        .unwrap();
    println!("OTHER_PK_X={}", field_to_hex(&other.0));
    println!("OTHER_PK_Y={}", field_to_hex(&other.1));
    assert_eq!(comparison_key(&Literal::new_simple_literal("Alice"))[0], 6);
}
