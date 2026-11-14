//! Reproduce the authenticated metadata fixtures consumed by the Noir tests.
#[path = "../tests/support/credential.rs"]
mod credential;
fn main() {
    use sparq_merkle_suite::{metadata, Fr};
    use sparq_zk::{field::field_to_hex, sig};
    let (known, known_message) =
        credential::input_created(7, [9; 32], Some("2026-01-01T00:00:00Z"));
    let (unknown, unknown_message) = credential::input(7, [9; 32]);
    let bounds_doc = "<urn:vc> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> .\n<urn:vc> <https://www.w3.org/2018/credentials#validFrom> \"2026-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n<urn:vc> <https://www.w3.org/2018/credentials#validUntil> \"2027-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n";
    let (bounds, bounds_message) = credential::input_document(bounds_doc, 128, 7, [9; 32], None);
    let (unknown_value, unknown_value_message) =
        credential::input_created(7, [9; 32], Some("unknown"));
    let created_enc = sparq_merkle_suite::poseidon::term_cell(
        &oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
            "2026-01-01T00:00:00Z",
            oxrdf::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#dateTime"),
        )),
        0,
    )
    .unwrap()
    .enc;
    let method_enc = sparq_merkle_suite::poseidon::term_cell(
        &oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked("urn:key:19")),
        0,
    )
    .unwrap()
    .enc;
    let sk = sig::SecretKey::from_seed(19);
    let other = sig::SecretKey::from_seed(20).public_key().coords().unwrap();
    println!(
        "{}",
        serde_json::json!({"known":known,"unknown":unknown,"unknown_value":unknown_value,"unknown_value_message":field_to_hex(&unknown_value_message),"created_enc":field_to_hex(&created_enc),"method_enc":field_to_hex(&method_enc),"bounds":bounds,"bounds_message":field_to_hex(&bounds_message),
        "known_message":field_to_hex(&known_message),"unknown_message":field_to_hex(&unknown_message),
        "key_enc":field_to_hex(&metadata::key_cell(&sk.public_key()).unwrap().enc),
        "graph_enc":field_to_hex(&metadata::graph_name([11,13],1).unwrap().enc),
        "other_pk":{"x":field_to_hex(&other.0),"y":field_to_hex(&other.1)},"zero":field_to_hex(&Fr::from(0u64))})
    );
}
