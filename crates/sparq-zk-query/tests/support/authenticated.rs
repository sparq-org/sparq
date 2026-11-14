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

#[allow(dead_code)]
pub fn canonical_policy() -> sparq_zk_query::envelope_policy::PolicyInput {
    use oxrdf::NamedNode;
    sparq_zk_query::envelope_policy::PolicyInput {
        trust_members: vec![sparq_zk_query::envelope_policy::TrustMember {
            issuer: NamedNode::new_unchecked("urn:issuer"),
            verification_method: NamedNode::new_unchecked("urn:method"),
            key: sig::SecretKey::from_seed(19).public_key(),
        }],
        ..Default::default()
    }
}
#[allow(dead_code)]
pub fn canonical_sign(source: &str, index: u64) -> Credential {
    let key = sig::SecretKey::from_seed(19);
    let document = format!("{source}\n<urn:credential> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> .\n<urn:credential> <https://www.w3.org/2018/credentials#issuer> <urn:issuer> .\n<urn:credential> <https://www.w3.org/2018/credentials#credentialSubject> <urn:subject> .\n");
    let config = "_:proof <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n_:proof <https://w3id.org/security#verificationMethod> <urn:method> .\n";
    Credential {
        credential: sparq_merkle_suite::poseidon::issue(
            &document,
            config,
            &key,
            sparq_merkle_suite::Fr::from(index + 101),
        )
        .unwrap(),
        issuer: key.public_key(),
        issuer_index: 0,
        issuer_siblings: vec![],
    }
}
#[allow(dead_code)]
pub fn canonical_fixture() -> Vec<Credential> {
    vec![canonical_sign("<urn:a> <urn:p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n<urn:b> <urn:p> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",0)]
}

#[allow(dead_code)]
pub fn canonical_status_fixture(
    depth: u32,
) -> (
    Vec<Credential>,
    sparq_zk_query::envelope_policy::PolicyInput,
    sparq_zk_query::witness::EnvelopeWitness,
) {
    use oxrdf::{Literal, NamedNode, Term};
    use sparq_merkle_suite::poseidon;
    use sparq_zk_query::{
        envelope_policy,
        witness::{CredentialEvidence, EnvelopeWitness, StatusWitness},
    };
    assert!(depth <= 32);
    let credential = canonical_sign(
        "<urn:credential> <https://www.w3.org/2018/credentials#credentialStatus> <urn:entry> .\n\
         <urn:entry> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/ns/credentials/status#BitstringStatusListEntry> .\n\
         <urn:entry> <https://www.w3.org/ns/credentials/status#statusListCredential> <urn:list> .\n\
         <urn:entry> <https://www.w3.org/ns/credentials/status#statusPurpose> \"revocation\" .\n\
         <urn:entry> <https://www.w3.org/ns/credentials/status#statusListIndex> \"0\" .", 0,
    );
    let quad_index = oxttl::NQuadsParser::new()
        .for_slice(credential.credential.document.as_bytes())
        .map(Result::unwrap)
        .position(|q| {
            q.predicate.as_str() == "https://www.w3.org/2018/credentials#credentialStatus"
        })
        .unwrap();
    let mut root = envelope_policy::status_leaf(false);
    let mut empty = sparq_merkle_suite::Fr::from(0u64);
    let mut path = vec![];
    for _ in 0..depth {
        path.push(field_to_be_bytes_32(&empty));
        root = sparq_zk::poseidon2::hash(&[root, empty]);
        empty = sparq_zk::poseidon2::hash(&[empty, empty]);
    }
    let list =
        poseidon::term_cell(&Term::NamedNode(NamedNode::new_unchecked("urn:list")), 0).unwrap();
    let purpose =
        poseidon::term_cell(&Literal::new_simple_literal("revocation").into(), 0).unwrap();
    let digest = envelope_policy::snapshot_digest(
        field_to_be_bytes_32(&list.enc),
        field_to_be_bytes_32(&purpose.enc),
        1,
        depth,
        field_to_be_bytes_32(&root),
    )
    .unwrap();
    let mut policy = canonical_policy();
    policy.accepted_snapshots = Some(envelope_policy::AcceptedSnapshots {
        digests: vec![digest],
        allow_missing: false,
    });
    let evidence = EnvelopeWitness {
        holders: vec![],
        credentials: vec![CredentialEvidence {
            holder_slot: 0,
            statuses: vec![StatusWitness {
                quad_index,
                bit_len: 1,
                depth,
                root: field_to_be_bytes_32(&root),
                accepted_index: 0,
                accepted_siblings: vec![],
                bit: false,
                path_index: 0,
                path,
            }],
        }],
    };
    (vec![credential], policy, evidence)
}
