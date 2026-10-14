//! Property tests for Ed25519 `did:key` multibase round trips.

use proptest::prelude::*;
use sparq_vc::{
    did::{did_key_for, DidError, DidKeyResolver, DidResolver},
    SigningKey,
};

const ED25519_MULTICODEC: [u8; 2] = [0xed, 0x01];

fn multibase_payload(did: &str) -> Vec<u8> {
    let id = did
        .strip_prefix("did:key:z")
        .expect("did_key_for returns a base58btc did:key");
    bs58::decode(id)
        .into_vec()
        .expect("did_key_for returns valid base58btc")
}

fn did_key_from_payload(payload: &[u8]) -> String {
    format!("did:key:z{}", bs58::encode(payload).into_string())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_ed25519_keys_round_trip_through_did_key(seed in any::<[u8; 32]>()) {
        let signing_key = SigningKey::from_seed(&seed);
        let expected = signing_key.verifying_key();
        let did = did_key_for(&expected);

        let payload = multibase_payload(&did);
        prop_assert_eq!(&payload[..ED25519_MULTICODEC.len()], &ED25519_MULTICODEC);
        prop_assert_eq!(&payload[ED25519_MULTICODEC.len()..], expected.as_bytes());

        let decoded = DidKeyResolver
            .resolve_str(&did)
            .expect("a minted did:key resolves");
        prop_assert_eq!(decoded.as_bytes(), expected.as_bytes());

        // Mutation witness: changing one encoded public-key byte cannot reproduce
        // the original key, whether the changed point is rejected or decodes.
        let mut mutated_payload = payload;
        let key_byte = ED25519_MULTICODEC.len() + usize::from(seed[0]) % 32;
        mutated_payload[key_byte] ^= 1;
        let mutated_did = did_key_from_payload(&mutated_payload);
        if let Ok(mutated) = DidKeyResolver.resolve_str(&mutated_did) {
            prop_assert_ne!(mutated.as_bytes(), expected.as_bytes());
        }
    }
}

#[test]
fn corrupted_multibase_and_multicodec_fail_closed() {
    let key = SigningKey::from_seed(&[0x5a; 32]);
    let did = did_key_for(&key.verifying_key());

    let wrong_multibase = did.replacen("did:key:z", "did:key:x", 1);
    assert!(matches!(
        DidKeyResolver.resolve_str(&wrong_multibase),
        Err(DidError::BadKeyMaterial(_))
    ));

    let mut wrong_multicodec = multibase_payload(&did);
    wrong_multicodec[..ED25519_MULTICODEC.len()].copy_from_slice(&[0xec, 0x01]);
    assert!(matches!(
        DidKeyResolver.resolve_str(&did_key_from_payload(&wrong_multicodec)),
        Err(DidError::BadKeyMaterial(_))
    ));
}
