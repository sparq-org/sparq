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

fn opening_at(term: &Term, capacity: usize) -> Value {
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
    json!({"bound": true, "kind": kind, "lexical": bytes(lexical,capacity), "lexical_len": lexical.len(), "datatype":bytes(dt,128),"datatype_len":dt.len(),"language":bytes(lang,64),"language_len":lang.len(),"key":key})
}
fn absent() -> Value {
    json!({"bound":false,"kind":0,"lexical":vec![0;32],"lexical_len":0,"datatype":vec![0;128],"datatype_len":0,"language":vec![0;64],"language_len":0,"key":vec![0;31]})
}
pub fn input(salt_value: u64, configuration: [u8; 32]) -> (Value, Fr) {
    input_created(salt_value, configuration, None)
}
pub fn input_created(
    salt_value: u64,
    configuration: [u8; 32],
    created: Option<&str>,
) -> (Value, Fr) {
    input_document(
        "_:c14n0 <http://ex/balance> \"1250.50\"^^<http://www.w3.org/2001/XMLSchema#decimal> .",
        32,
        salt_value,
        configuration,
        created,
    )
}
pub fn input_document(
    document: &str,
    capacity: usize,
    salt_value: u64,
    configuration: [u8; 32],
    created: Option<&str>,
) -> (Value, Fr) {
    let mut rows = oxttl::NQuadsParser::new()
        .for_slice(document.as_bytes())
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    rows.sort_by_key(|q| p::field_bytes(p::quad_hash(q).unwrap()));
    let count = rows.len() as u32;
    let mut raw = rows
        .iter()
        .map(|quad| {
            let mut graph = opening_at(
                &Term::NamedNode(oxrdf::NamedNode::new_unchecked("")),
                capacity,
            );
            graph["kind"] = json!(0);
            vec![
                opening_at(&Term::from(quad.subject.clone()), capacity),
                opening_at(&Term::NamedNode(quad.predicate.clone()), capacity),
                opening_at(&quad.object, capacity),
                graph,
            ]
        })
        .collect::<Vec<_>>();
    let mut pad = absent();
    pad["lexical"] = json!(vec![0; capacity]);
    raw.resize(raw.len().next_power_of_two(), vec![pad; 4]);
    let qs = rows
        .iter()
        .map(|q| p::quad_hash(q).unwrap())
        .collect::<Vec<_>>();
    let salt = Fr::from(salt_value);
    let root = p::root(&qs, salt).unwrap();
    let mut salts = (0..count)
        .map(|i| field_to_hex(&p::leaf_salt(salt, i)))
        .collect::<Vec<_>>();
    salts.resize(raw.len(), field_to_hex(&Fr::from(0u64)));
    let sk = sig::SecretKey::from_seed(19);
    let mut config_text = String::from("<urn:proof> <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n<urn:proof> <https://w3id.org/security#verificationMethod> <urn:key:19> .\n");
    if let Some(text) = created {
        config_text.push_str(&format!("<urn:proof> <http://purl.org/dc/terms/created> \"{text}\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n"));
    }
    let metadata =
        sparq_merkle_suite::metadata::from_configuration(&config_text, &sk.public_key()).unwrap();
    let created_raw = metadata
        .created
        .as_ref()
        .map_or_else(absent, |t| opening_at(t, 32));
    let method_raw = opening_at(metadata.verification_method.as_ref().unwrap(), 32);
    let message = p::signed_message(salt, count, root, &configuration, metadata.digest);
    let signature = sig::sign_deterministic(&sk, &message);
    assert!(sig::verify(&sk.public_key(), &message, &signature));
    let w = sig::in_circuit_witness(&sk.public_key(), &message, &signature).unwrap();
    (
        json!({"raw":raw,"created_raw":created_raw,"method_raw":method_raw,"salt":field_to_hex(&salt),"count":count,"salts":salts,
        "config":[u128::from_be_bytes(configuration[..16].try_into().unwrap()).to_string(),u128::from_be_bytes(configuration[16..].try_into().unwrap()).to_string()],
        "pk":{"x":field_to_hex(&w.pk_x),"y":field_to_hex(&w.pk_y)},"r":{"x":field_to_hex(&w.r_x),"y":field_to_hex(&w.r_y)},
        "s":field_to_hex(&w.s),"e":field_to_hex(&w.e),"e_k":field_to_hex(&w.e_k)}),
        message,
    )
}
