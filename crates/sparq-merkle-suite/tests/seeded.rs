use sparq_merkle_suite::{metadata, poseidon as p};
use sparq_zk::sig;

#[path = "support/solid_profile.rs"]
mod solid_profile;

const DOC: &str = "_:a <urn:p> \"1\" .\n_:a <urn:q> <urn:o> .\n";
const CONFIG: &str =
    "<urn:proof> <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n";

// Compare every credential field through its canonical byte representation;
// this test framing does not define another credential wire format.
fn bytes(c: &p::Credential) -> Vec<u8> {
    let mut out = Vec::new();
    for text in [&c.document, &c.proof_config] {
        out.extend_from_slice(&(text.len() as u64).to_be_bytes());
        out.extend_from_slice(text.as_bytes());
    }
    out.extend_from_slice(&p::field_bytes(c.salt));
    out.extend_from_slice(&c.count.to_be_bytes());
    out.extend_from_slice(&p::field_bytes(c.root));
    out.extend_from_slice(sig::signature_to_hex(&c.signature).as_bytes());
    out
}

#[test]
fn keypair_from_seed_is_deterministic_and_distinct_across_seeds() {
    let seed = [7; 32];
    let a = p::keypair_from_seed(&seed);
    let b = p::keypair_from_seed(&seed);
    assert!(bool::from(a.ct_eq(&b)));
    assert_eq!(a.public_key(), b.public_key());
    let mut changed = seed;
    changed[31] ^= 1;
    assert_ne!(a.public_key(), p::keypair_from_seed(&changed).public_key());
    for seed in [[0; 32], [255; 32], seed, changed] {
        assert!(p::keypair_from_seed(&seed).public_key().is_prime_order());
    }
}

#[test]
fn deterministic_issuance_is_byte_identical() {
    let key = p::keypair_from_seed(&[7; 32]);
    let a = p::issue_deterministic(DOC, CONFIG, &key, &[9; 32]).unwrap();
    let b = p::issue_deterministic(DOC, CONFIG, &key, &[9; 32]).unwrap();
    assert_eq!(bytes(&a), bytes(&b));
}

#[test]
fn canonical_equivalent_inputs_are_reproducible() {
    let key = p::keypair_from_seed(&[7; 32]);
    let a = p::issue_deterministic(DOC, CONFIG, &key, &[9; 32]).unwrap();
    let b = p::issue_deterministic(
        "_:renamed <urn:q> <urn:o> .\n_:renamed <urn:p> \"1\" .\n",
        &format!("\n  {CONFIG}"),
        &key,
        &[9; 32],
    )
    .unwrap();
    assert_eq!(bytes(&a), bytes(&b));
}

#[test]
fn different_salt_seeds_change_salts_and_roots() {
    let key = p::keypair_from_seed(&[7; 32]);
    let a = p::issue_deterministic(DOC, CONFIG, &key, &[9; 32]).unwrap();
    let b = p::issue_deterministic(DOC, CONFIG, &key, &[10; 32]).unwrap();
    assert_ne!(a.salt, b.salt);
    assert_ne!(a.root, b.root);
}

#[test]
fn different_document_changes_derived_salt() {
    let key = p::keypair_from_seed(&[7; 32]);
    let a = p::issue_deterministic(DOC, CONFIG, &key, &[9; 32]).unwrap();
    let b = p::issue_deterministic(&DOC.replace("\"1\"", "\"2\""), CONFIG, &key, &[9; 32]).unwrap();
    assert_ne!(a.salt, b.salt);
}

#[test]
fn different_configuration_changes_derived_salt() {
    let key = p::keypair_from_seed(&[7; 32]);
    let a = p::issue_deterministic(
        DOC,
        &format!("{CONFIG}<urn:proof> <urn:option> \"first\" .\n"),
        &key,
        &[9; 32],
    )
    .unwrap();
    let b = p::issue_deterministic(
        DOC,
        &format!("{CONFIG}<urn:proof> <urn:option> \"other\" .\n"),
        &key,
        &[9; 32],
    )
    .unwrap();
    assert_ne!(a.salt, b.salt);
}

#[test]
fn different_signer_changes_derived_salt() {
    let a = p::issue_deterministic(DOC, CONFIG, &p::keypair_from_seed(&[7; 32]), &[9; 32]).unwrap();
    let b = p::issue_deterministic(DOC, CONFIG, &p::keypair_from_seed(&[8; 32]), &[9; 32]).unwrap();
    assert_ne!(a.salt, b.salt);
}

#[test]
fn deterministic_credentials_verify_and_open() {
    let key = p::keypair_from_seed(&[7; 32]);
    let c = p::issue_deterministic(DOC, CONFIG, &key, &[9; 32]).unwrap();
    assert!(p::verify(&c, &key.public_key()));
    let opened = p::verify_and_open(&c, &key.public_key()).unwrap();
    assert_eq!(
        opened.verification_key,
        metadata::key_cell(&key.public_key()).unwrap()
    );
}

#[test]
fn solid_app_suite_profile_issues_and_verifies() {
    let document = solid_profile::document();
    let config = solid_profile::configuration();
    let key = p::keypair_from_seed(&[7; 32]);
    let c = p::issue_deterministic(&document, &config, &key, &[9; 32]).unwrap();
    assert_eq!(c.count, 42);
    assert!(c.count <= 64);
    assert!(p::verify(&c, &key.public_key()));
    assert!(p::verify_and_open(&c, &key.public_key())
        .unwrap()
        .created
        .is_some());
    assert_eq!(
        bytes(&c),
        bytes(&p::issue_deterministic(&document, &config, &key, &[9; 32]).unwrap())
    );
    let rows = oxttl::NQuadsParser::new()
        .for_slice(c.document.as_bytes())
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    let mut facts = 0;
    let mut classes = std::collections::BTreeSet::new();
    let mut datatypes = std::collections::BTreeMap::new();
    let mut max_integer = 0;
    let mut status_indices = 0;
    let mut status_purposes = 0;
    for row in rows {
        if row.predicate.as_str().starts_with("urn:solid:fact:") {
            facts += 1;
            let oxrdf::Term::Literal(l) = &row.object else {
                panic!("literal fact required")
            };
            classes.insert(sparq_merkle_suite::comparison_key(l)[0]);
            let datatype = l.datatype().as_str().rsplit('#').next().unwrap();
            *datatypes.entry(datatype.to_owned()).or_insert(0) += 1;
            match datatype {
                "integer" => {
                    let value = l.value().parse::<u64>().unwrap();
                    assert!(value <= 1u64 << 48);
                    max_integer = max_integer.max(value);
                }
                "decimal" => assert_eq!(l.value().split_once('.').unwrap().1.len(), 2),
                "date" => assert_eq!(l.value().len(), 10),
                "string" => assert!(l.value().starts_with("medicine-")),
                _ => panic!("unexpected profile datatype"),
            }
        }
        if row.predicate.as_str().ends_with("#statusListIndex") {
            let oxrdf::Term::Literal(l) = &row.object else {
                panic!("string index required")
            };
            assert_eq!(metadata::status_index(l), Ok(7));
            status_indices += 1;
        }
        if row.predicate.as_str().ends_with("#statusPurpose") {
            assert_eq!(
                row.object,
                oxrdf::Term::Literal(oxrdf::Literal::new_simple_literal("revocation"))
            );
            status_purposes += 1;
        }
    }
    assert_eq!(facts, 32);
    assert_eq!(classes, [1, 6, 8].into_iter().collect());
    assert_eq!(max_integer, 1u64 << 48);
    assert_eq!(
        datatypes,
        ["integer", "decimal", "date", "string"]
            .map(|s| (s.to_owned(), 8))
            .into_iter()
            .collect()
    );
    assert_eq!((status_indices, status_purposes), (1, 1));
}

#[cfg(feature = "os-random")]
#[test]
fn randomised_issuance_verifies() {
    let key = p::keypair_from_seed(&[7; 32]);
    let c = sparq_merkle_suite::random::issue(DOC, CONFIG, &key).unwrap();
    assert!(p::verify(&c, &key.public_key()));
}
