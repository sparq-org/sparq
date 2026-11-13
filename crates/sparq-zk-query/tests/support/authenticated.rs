use sparq_zk::{field::field_to_be_bytes_32, sig};
use sparq_zk_query::{credential::Credential, interp::Word};

pub fn issuer_root() -> Word {
    let pk = sig::SecretKey::from_seed(19).public_key();
    field_to_be_bytes_32(&sparq_zk_compose::issuer::key_set_root(&[pk], 4).unwrap())
}
pub fn sign(source: &str, index: u64) -> Credential {
    let key = sig::SecretKey::from_seed(19);
    let config =
        "_:proof <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n";
    let credential = sparq_merkle_suite::poseidon::issue(
        source,
        config,
        &key,
        sparq_merkle_suite::Fr::from(index + 101),
    )
    .unwrap();
    let pk = key.public_key();
    let siblings =
        sparq_zk_compose::issuer::key_membership_witness(std::slice::from_ref(&pk), 4, 0).unwrap();
    Credential {
        credential,
        issuer: pk,
        issuer_index: 0,
        issuer_siblings: siblings,
    }
}
#[allow(dead_code)]
pub fn fixture() -> Vec<Credential> {
    vec![sign("<urn:a> <urn:p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n<urn:b> <urn:p> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",0)]
}
