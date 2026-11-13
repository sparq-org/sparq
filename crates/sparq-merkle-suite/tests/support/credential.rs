use oxrdf::Term;
use serde_json::{json, Value};
use sparq_merkle_suite::{poseidon as p, Fr};
use sparq_zk::{field::field_to_hex, sig};
fn bytes(text: &str, capacity: usize) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    assert!(bytes.len() <= capacity);
    bytes.resize(capacity, 0);
    bytes
}

fn opening(term: &Term) -> Value {
    let (kind, lexical, dt, lang, key) = match term {
        Term::NamedNode(n) => (1, n.as_str(), "", "", [0; 31]),
        Term::BlankNode(n) => (2, n.as_str(), "", "", [0; 31]),
        Term::Literal(l) => (
            3,
            l.value(),
            l.datatype().as_str(),
            l.language().unwrap_or(""),
            sparq_merkle_suite::comparison_key(l),
        ),
        Term::Triple(_) => panic!("triple terms are not admitted"),
    };
    json!({"bound": true, "kind": kind, "lexical": bytes(lexical,32), "lexical_len": lexical.len(), "datatype":bytes(dt,128),"datatype_len":dt.len(),"language":bytes(lang,64),"language_len":lang.len(),"key":key})
}
pub fn input(salt_value: u64, configuration: [u8; 32]) -> (Value, Fr) {
    let quad = oxttl::NQuadsParser::new().for_slice(b"_:c14n0 <http://ex/balance> \"1250.50\"^^<http://www.w3.org/2001/XMLSchema#decimal> .").next().unwrap().unwrap();
    let mut graph = opening(&Term::NamedNode(oxrdf::NamedNode::new_unchecked("")));
    graph["kind"] = json!(0);
    let raw = vec![vec![
        opening(&Term::from(quad.subject.clone())),
        opening(&Term::NamedNode(quad.predicate.clone())),
        opening(&quad.object),
        graph,
    ]];
    let q = p::quad_hash(&quad).unwrap();
    let salt = Fr::from(salt_value);
    let root = p::root(&[q], salt).unwrap();
    let message = p::signed_message(salt, 1, root, &configuration);
    let sk = sig::SecretKey::from_seed(19);
    let signature = sig::sign_deterministic(&sk, &message);
    let w = sig::in_circuit_witness(&sk.public_key(), &message, &signature).unwrap();
    (
        json!({"raw":raw,"salt":field_to_hex(&salt),"count":1,"salts":[field_to_hex(&p::leaf_salt(salt,0))],
        "config":[u128::from_be_bytes(configuration[..16].try_into().unwrap()).to_string(),u128::from_be_bytes(configuration[16..].try_into().unwrap()).to_string()],
        "pk":{"x":field_to_hex(&w.pk_x),"y":field_to_hex(&w.pk_y)},"r":{"x":field_to_hex(&w.r_x),"y":field_to_hex(&w.r_y)},
        "s":field_to_hex(&w.s),"e":field_to_hex(&w.e),"e_k":field_to_hex(&w.e_k)}),
        message,
    )
}
