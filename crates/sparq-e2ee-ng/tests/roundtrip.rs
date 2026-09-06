//! Behavioral tests: deterministic encoding, fail-closed parsing, seal/open,
//! sign/verify, capability separation + delegation, and epoch transitions.

use sparq_e2ee_ng::capability::{
    base_grant, delegate, unwrap_capability, wrap_capability, Authority, Capability, Delegation,
    PublicGrant, ScopedBranch, Validity, CAPABILITY_WRAP_AAD,
};
use sparq_e2ee_ng::cbor::{enc_bytes, enc_map, enc_uint, Limits, Reader, Writer};
use sparq_e2ee_ng::envelope::{
    open_block, seal_block, seal_block_random, BlockContext, BlockEnvelope, Commit, ObjectKind,
    PAD_CLASSES,
};
use sparq_e2ee_ng::epoch::{EpochTransition, HistoryPolicy};
use sparq_e2ee_ng::error::Error;
use sparq_e2ee_ng::ids::{
    AuthorKeyId, BranchId, CommitId, Epoch, ObjectId, RepoId, Secret32, TopicId,
};
use sparq_e2ee_ng::sign::SecretSigningKey;
use sparq_e2ee_ng::suite::AEAD_NONCE_LEN;
use sparq_e2ee_ng::wrap::{
    unwrap, wrap, wrap_with, RecipientPublicKey, RecipientSecretKey, WrappedSecret,
};

fn lim() -> Limits {
    Limits::default()
}

// --- helpers ----------------------------------------------------------------
fn sample_grant() -> PublicGrant {
    base_grant(
        RepoId::from_bytes([1u8; 32]),
        BranchId::from_bytes([2u8; 32]),
        Epoch(7),
        TopicId::from_bytes([3u8; 32]),
        Validity { not_before: 100, not_after: 200 },
        vec!["wss://broker.example".to_string()],
    )
}

// ============================================================================
// CBOR determinism + fail-closed parsing
// ============================================================================

#[test]
fn cbor_canonical_is_deterministic() {
    // A map supplied out of key order encodes identically to in-order.
    let a = enc_map(vec![
        (3, enc_uint(30)),
        (1, enc_uint(10)),
        (2, enc_bytes(b"x")),
    ]);
    let b = enc_map(vec![
        (1, enc_uint(10)),
        (2, enc_bytes(b"x")),
        (3, enc_uint(30)),
    ]);
    assert_eq!(a, b, "map key order must not affect canonical bytes");
}

#[test]
fn cbor_rejects_indefinite_length() {
    // 0xbf = map(indefinite). Must be rejected.
    let mut r = Reader::new(&[0xbf], lim());
    assert!(matches!(r.map_header(), Err(Error::NonCanonical(_))));
}

#[test]
fn cbor_rejects_non_shortest_int() {
    // 0x18 0x05 = uint 5 in 1-byte form, but 5 fits inline -> non-canonical.
    let mut r = Reader::new(&[0x18, 0x05], lim());
    assert!(matches!(r.uint(), Err(Error::NonCanonical(_))));
}

#[test]
fn cbor_rejects_trailing_bytes() {
    let mut w = Writer::new();
    w.uint(1);
    let mut bytes = w.into_bytes();
    bytes.push(0x00); // trailing
    let mut r = Reader::new(&bytes, lim());
    r.uint().unwrap();
    assert!(matches!(r.finish(), Err(Error::NonCanonical(_))));
}

#[test]
fn cbor_enforces_limits() {
    // Declare a byte string of length 10 but with a tiny max_str_len.
    let tight = Limits { max_str_len: 4, ..Limits::default() };
    let mut w = Writer::new();
    w.bytes(&[0u8; 10]);
    let bytes = w.into_bytes();
    let mut r = Reader::new(&bytes, tight);
    assert!(matches!(r.bytes(), Err(Error::LimitExceeded(_))));
}

#[test]
fn cbor_negative_extension_key_is_skipped() {
    // A map with one positive field (1 -> 9) and one negative extension key
    // (-1 -> "ext") must decode, ignoring the extension. RFC 8949 canonical
    // order is by encoded key bytes, so uint 1 (0x01) precedes -1 (0x20).
    let mut w = Writer::new();
    // map(2): key 1 -> 9, then key -1 (nint 0) -> text "ext"
    w.raw(&[0xa2]); // map of 2
    w.uint(1);
    w.uint(9);
    w.raw(&[0x20]); // nint 0 == -1
    w.text("ext");
    let bytes = w.into_bytes();
    let mut r = Reader::new(&bytes, lim());
    let mut got = None;
    sparq_e2ee_ng::cbor::read_struct_map(&mut r, |r, k| {
        if k == 1 {
            got = Some(r.uint()?);
            Ok(true)
        } else {
            Ok(false)
        }
    })
    .unwrap();
    assert_eq!(got, Some(9));
}

#[test]
fn cbor_rejects_negative_key_before_same_length_positive() {
    // Encoded-byte order puts uint 1 (0x01) before -1 (0x20); supplying -1
    // first is non-canonical and must be rejected.
    let mut w = Writer::new();
    w.raw(&[0xa2]); // map of 2
    w.raw(&[0x20]); // nint 0 == -1
    w.text("ext");
    w.uint(1);
    w.uint(9);
    let bytes = w.into_bytes();
    let mut r = Reader::new(&bytes, lim());
    let res = sparq_e2ee_ng::cbor::read_struct_map(&mut r, |r, _| {
        r.uint()?;
        Ok(true)
    });
    assert!(matches!(res, Err(Error::NonCanonical(_))));
}

#[test]
fn cbor_rejects_duplicate_negative_keys() {
    let mut w = Writer::new();
    w.raw(&[0xa2]); // map of 2
    w.raw(&[0x20]); // nint 0 == -1
    w.text("a");
    w.raw(&[0x20]); // -1 again: duplicate extension key
    w.text("b");
    let bytes = w.into_bytes();
    let mut r = Reader::new(&bytes, lim());
    let res = sparq_e2ee_ng::cbor::read_struct_map(&mut r, |_, _| Ok(true));
    assert!(matches!(res, Err(Error::NonCanonical(_))));
}

/// Build a struct map `{1: 9, -1: <ext_value>}` whose extension value is the
/// raw bytes `ext_value`, and decode it through the real `read_struct_map`
/// extension-skipping path.
fn decode_with_extension_value(ext_value: &[u8]) -> Result<(), Error> {
    let mut w = Writer::new();
    w.raw(&[0xa2]); // map of 2
    w.uint(1);
    w.uint(9);
    w.raw(&[0x20]); // nint 0 == -1
    w.raw(ext_value);
    let bytes = w.into_bytes();
    let mut r = Reader::new(&bytes, lim());
    sparq_e2ee_ng::cbor::read_struct_map(&mut r, |r, k| {
        if k == 1 {
            r.uint()?;
            Ok(true)
        } else {
            Ok(false)
        }
    })?;
    r.finish()
}

#[test]
fn cbor_skipped_extension_map_must_be_canonical() {
    // A canonical nested map {1: 0, 2: 0} as the extension value is fine.
    decode_with_extension_value(&[0xa2, 0x01, 0x00, 0x02, 0x00]).unwrap();
    // Reverse key order {2: 0, 1: 0} inside the skipped value is non-canonical.
    assert!(matches!(
        decode_with_extension_value(&[0xa2, 0x02, 0x00, 0x01, 0x00]),
        Err(Error::NonCanonical(_))
    ));
    // Duplicate keys {1: 0, 1: 0} inside the skipped value are rejected too.
    assert!(matches!(
        decode_with_extension_value(&[0xa2, 0x01, 0x00, 0x01, 0x00]),
        Err(Error::NonCanonical(_))
    ));
    // A non-integer (text) key inside the skipped value is outside the profile.
    assert!(matches!(
        decode_with_extension_value(&[0xa1, 0x61, 0x61, 0x00]), // {"a": 0}
        Err(Error::Schema(_))
    ));
}

// ============================================================================
// Signatures
// ============================================================================

#[test]
fn sign_verify_roundtrip_and_tamper() {
    let sk = SecretSigningKey::from_seed([9u8; 32]);
    let pk = sk.public();
    let sig = sk.sign(b"message");
    pk.verify(b"message", &sig).unwrap();
    assert!(matches!(pk.verify(b"tampered", &sig), Err(Error::BadSignature)));
}

// ============================================================================
// Recipient wrapping
// ============================================================================

#[test]
fn wrap_unwrap_roundtrip() {
    let recipient = RecipientSecretKey::from_bytes([5u8; 32]);
    let w = wrap(&recipient.public(), b"K_read-secret-bytes", b"purpose:cap").unwrap();
    let pt = unwrap(&recipient, &w, b"purpose:cap").unwrap();
    assert_eq!(pt, b"K_read-secret-bytes");
}

#[test]
fn wrap_wrong_aad_fails_closed() {
    let recipient = RecipientSecretKey::from_bytes([6u8; 32]);
    let w = wrap(&recipient.public(), b"secret", b"aad-A").unwrap();
    assert!(matches!(unwrap(&recipient, &w, b"aad-B"), Err(Error::Decrypt)));
}

#[test]
fn wrap_wrong_recipient_fails_closed() {
    let recipient = RecipientSecretKey::from_bytes([7u8; 32]);
    let attacker = RecipientSecretKey::from_bytes([8u8; 32]);
    let w = wrap(&recipient.public(), b"secret", b"aad").unwrap();
    assert!(matches!(unwrap(&attacker, &w, b"aad"), Err(Error::Decrypt)));
}

#[test]
fn wrap_rejects_low_order_recipient_key() {
    // The all-zero u-coordinate is a low-order point: X25519 against it yields
    // the all-zero shared secret regardless of the ephemeral key, so sealing
    // must fail closed rather than derive a key from public context only.
    let low_order = RecipientPublicKey([0u8; 32]);
    assert!(matches!(wrap(&low_order, b"secret", b"aad"), Err(Error::BadKey(_))));
    assert!(matches!(
        wrap_with([11u8; 32], [12u8; AEAD_NONCE_LEN], &low_order, b"secret", b"aad"),
        Err(Error::BadKey(_))
    ));
}

#[test]
fn unwrap_rejects_low_order_ephemeral_key() {
    // An attacker-supplied low-order ephemeral key must fail closed on open.
    let recipient = RecipientSecretKey::from_bytes([5u8; 32]);
    let good = wrap(&recipient.public(), b"secret", b"aad").unwrap();
    let forged = WrappedSecret { ephemeral_pub: [0u8; 32], ..good };
    assert!(matches!(unwrap(&recipient, &forged, b"aad"), Err(Error::Decrypt)));
}

#[test]
fn wrapped_secret_encode_decode_roundtrip() {
    let recipient = RecipientSecretKey::from_bytes([5u8; 32]);
    let w = wrap_with(
        [11u8; 32],
        [12u8; AEAD_NONCE_LEN],
        &recipient.public(),
        b"payload",
        b"aad",
    )
    .unwrap();
    let bytes = w.encode();
    let w2 = WrappedSecret::decode(&bytes, lim()).unwrap();
    assert_eq!(w, w2);
    assert_eq!(unwrap(&recipient, &w2, b"aad").unwrap(), b"payload");
}

// ============================================================================
// Capabilities: separation, encoding, delegation
// ============================================================================

#[test]
fn read_write_admin_separation() {
    let admin = SecretSigningKey::from_seed([1u8; 32]);
    let publisher = SecretSigningKey::from_seed([2u8; 32]);

    let read = Capability::new_read(sample_grant(), Secret32([9u8; 32])).unwrap();
    assert_eq!(read.grant.authority, vec![Authority::Read]);
    read.validate().unwrap();

    let write =
        Capability::new_write(sample_grant(), Secret32([9u8; 32]), &publisher).unwrap();
    assert!(write.grant.authority.contains(&Authority::Publish));
    assert!(write.grant.publisher_pub.is_some());
    write.validate().unwrap();

    let adm = Capability::new_admin(sample_grant(), &admin).unwrap();
    assert_eq!(adm.grant.authority, vec![Authority::Admin]);
    adm.validate().unwrap();
}

#[test]
fn publisher_and_admin_keys_never_combined() {
    // Manually construct an invalid capability and confirm validate() rejects it.
    let mut cap = Capability {
        grant: sample_grant(),
        read_secret: None,
        publisher_sk: Some([1u8; 32]),
        admin_sk: Some([2u8; 32]),
    };
    cap.grant.authority = vec![Authority::Publish, Authority::Admin];
    assert!(matches!(cap.validate(), Err(Error::Separation(_))));
}

#[test]
fn publisher_secret_must_match_signed_publisher_pub() {
    let key_a = SecretSigningKey::from_seed([2u8; 32]);
    let key_b = SecretSigningKey::from_seed([4u8; 32]);
    let mut cap = Capability::new_write(sample_grant(), Secret32([9u8; 32]), &key_a).unwrap();
    // The bearer secret signs under key A, but the grant binds key B's public
    // key: validation must reject the split identity.
    cap.grant.publisher_pub = Some(key_b.public().to_bytes());
    assert!(matches!(cap.validate(), Err(Error::Separation(_))));
    // The mismatch is also caught on the decode path (decode_secret validates).
    let bytes = cap.encode_secret();
    assert!(matches!(Capability::decode_secret(&bytes, lim()), Err(Error::Separation(_))));
}

#[test]
fn public_grant_excludes_secret_fields() {
    let publisher = SecretSigningKey::from_seed([2u8; 32]);
    let write =
        Capability::new_write(sample_grant(), Secret32([9u8; 32]), &publisher).unwrap();
    let public = write.grant.encode();
    let secret = write.encode_secret();
    assert!(secret.len() > public.len(), "secret encoding carries more");
    // The read secret bytes must not appear in the public grant.
    assert!(
        !public.windows(32).any(|w| w == [9u8; 32]),
        "read secret leaked into public grant"
    );
    // But they DO appear in the secret-bearing encoding.
    assert!(secret.windows(32).any(|w| w == [9u8; 32]));
}

#[test]
fn public_grant_admin_sign_verify() {
    let admin = SecretSigningKey::from_seed([1u8; 32]);
    let mut grant = sample_grant();
    grant.authority = vec![Authority::Read];
    grant.sign(&admin);
    grant.verify(&admin.public()).unwrap();

    // Tamper: flip epoch, signature must fail.
    let mut bad = grant.clone();
    bad.epoch = Epoch(999);
    assert!(matches!(bad.verify(&admin.public()), Err(Error::BadSignature)));
}

#[test]
fn cap_id_is_stable_and_nonce_sensitive() {
    let g = sample_grant();
    assert_eq!(g.cap_id(), g.cap_id());
    let mut g2 = g.clone();
    g2.cap_nonce = [0xAA; 32];
    assert_ne!(g.cap_id(), g2.cap_id(), "cap_id must depend on the nonce");
}

#[test]
fn capability_secret_roundtrip() {
    let publisher = SecretSigningKey::from_seed([2u8; 32]);
    let mut write =
        Capability::new_write(sample_grant(), Secret32([9u8; 32]), &publisher).unwrap();
    let admin = SecretSigningKey::from_seed([1u8; 32]);
    write.grant.sign(&admin);

    let bytes = write.encode_secret();
    let decoded = Capability::decode_secret(&bytes, lim()).unwrap();
    assert_eq!(decoded.grant, write.grant);
    assert_eq!(decoded.read_secret.as_ref().unwrap().expose(), &[9u8; 32]);
    assert_eq!(decoded.publisher_sk, Some(publisher.to_seed()));
    assert!(decoded.admin_sk.is_none());
    decoded.grant.verify(&admin.public()).unwrap();
}

// --- typed capability wrapping (§4.2 recommended secret-transfer path) -------

#[test]
fn wrap_capability_roundtrip_recovers_every_secret_field() {
    let publisher = SecretSigningKey::from_seed([2u8; 32]);
    let admin = SecretSigningKey::from_seed([1u8; 32]);
    let mut write =
        Capability::new_write(sample_grant(), Secret32([9u8; 32]), &publisher).unwrap();
    write.grant.sign(&admin);

    let recipient = RecipientSecretKey::from_bytes([5u8; 32]);
    let wrapped = wrap_capability(&write, &recipient.public()).unwrap();

    // The bearer secrets must not survive in the clear anywhere in the wrapping.
    let bytes = wrapped.encode();
    assert!(
        !bytes.windows(32).any(|w| w == [9u8; 32]),
        "read secret leaked into the wrapping"
    );
    assert!(
        !bytes.windows(32).any(|w| w == publisher.to_seed()),
        "publisher key leaked into the wrapping"
    );

    let opened = unwrap_capability(&recipient, &wrapped, lim()).unwrap();
    assert_eq!(opened.grant, write.grant);
    assert_eq!(opened.read_secret.as_ref().unwrap().expose(), &[9u8; 32]);
    assert_eq!(opened.publisher_sk, Some(publisher.to_seed()));
    assert!(opened.admin_sk.is_none());
    opened.grant.verify(&admin.public()).unwrap();
}

#[test]
fn wrap_capability_aad_is_domain_separated() {
    let recipient = RecipientSecretKey::from_bytes([5u8; 32]);
    let cap = Capability::new_read(sample_grant(), Secret32([9u8; 32])).unwrap();

    // A wrapping of the very same bytes under any other AAD is NOT a capability
    // wrapping: the typed unwrap must reject it rather than open a payload some
    // other purpose produced.
    let foreign = wrap(&recipient.public(), &cap.encode_secret(), b"purpose:dek").unwrap();
    assert!(matches!(
        unwrap_capability(&recipient, &foreign, lim()),
        Err(Error::Decrypt)
    ));

    // ...and symmetrically, a capability wrapping does not open under a
    // caller-chosen label, only under the fixed domain-separation AAD.
    let wrapped = wrap_capability(&cap, &recipient.public()).unwrap();
    assert!(matches!(
        unwrap(&recipient, &wrapped, b"purpose:dek"),
        Err(Error::Decrypt)
    ));
    assert_eq!(
        unwrap(&recipient, &wrapped, CAPABILITY_WRAP_AAD).unwrap(),
        cap.encode_secret()
    );
}

#[test]
fn wrap_capability_wrong_recipient_fails_closed() {
    let recipient = RecipientSecretKey::from_bytes([5u8; 32]);
    let attacker = RecipientSecretKey::from_bytes([6u8; 32]);
    let cap = Capability::new_read(sample_grant(), Secret32([9u8; 32])).unwrap();
    let wrapped = wrap_capability(&cap, &recipient.public()).unwrap();
    assert!(matches!(
        unwrap_capability(&attacker, &wrapped, lim()),
        Err(Error::Decrypt)
    ));
}

#[test]
fn wrap_capability_rejects_a_capability_that_violates_separation() {
    // A hand-built capability combining publisher and admin keys must fail at
    // wrap time, not produce bytes whose only possible outcome is a rejected
    // unwrap on the far side.
    let mut cap = Capability {
        grant: sample_grant(),
        read_secret: None,
        publisher_sk: Some([1u8; 32]),
        admin_sk: Some([2u8; 32]),
    };
    cap.grant.authority = vec![Authority::Publish, Authority::Admin];
    let recipient = RecipientSecretKey::from_bytes([5u8; 32]);
    assert!(matches!(
        wrap_capability(&cap, &recipient.public()),
        Err(Error::Separation(_))
    ));
}

#[test]
fn wrap_capability_rejects_a_structurally_invalid_public_grant() {
    // Separation-correct secrets are NOT enough: every `PublicGrant` field is
    // public, so a caller can hand `wrap_capability` a read capability whose
    // grant half only `PublicGrant::decode` would have caught. Wrapping such a
    // grant would produce bytes that `unwrap_capability` must reject on the far
    // side, so it has to fail here instead.
    let recipient = RecipientSecretKey::from_bytes([5u8; 32]);
    let read_cap = || Capability::new_read(sample_grant(), Secret32([9u8; 32])).unwrap();

    // An unsupported suite.
    let mut cap = read_cap();
    cap.grant.suite = "urn:jeswr:w3id:e2ee-ng:suite:not-a-suite".to_string();
    assert!(matches!(
        wrap_capability(&cap, &recipient.public()),
        Err(Error::UnknownSuite)
    ));

    // An inverted validity window.
    let mut cap = read_cap();
    cap.grant.validity = Validity { not_before: 200, not_after: 100 };
    assert!(matches!(
        wrap_capability(&cap, &recipient.public()),
        Err(Error::Schema(_))
    ));

    // A max_epoch ceiling behind the epoch the grant is scoped to.
    let mut cap = read_cap();
    cap.grant.max_epoch = Some(Epoch(3));
    assert!(matches!(
        wrap_capability(&cap, &recipient.public()),
        Err(Error::Schema(_))
    ));

    // A branch set assigned directly rather than via `set_branch_scope`, so not
    // strictly ascending by branch id.
    let mut cap = read_cap();
    cap.grant.extra_branches = vec![ScopedBranch {
        branch: BranchId::from_bytes([1u8; 32]),
        topic: TopicId::from_bytes([4u8; 32]),
    }];
    assert!(matches!(
        wrap_capability(&cap, &recipient.public()),
        Err(Error::NonCanonical(_))
    ));

    // The baseline the four above differ from by exactly one field still wraps
    // AND opens with the matching recipient — the invariant this guards is that
    // a successful typed wrap is always openable, not that wrapping is hard.
    let cap = read_cap();
    let wrapped = wrap_capability(&cap, &recipient.public()).unwrap();
    let opened = unwrap_capability(&recipient, &wrapped, lim()).unwrap();
    assert_eq!(opened.grant, cap.grant);
}

#[test]
fn wrap_capability_rejects_low_order_recipient_key() {
    let cap = Capability::new_read(sample_grant(), Secret32([9u8; 32])).unwrap();
    let low_order = RecipientPublicKey([0u8; 32]);
    assert!(matches!(
        wrap_capability(&cap, &low_order),
        Err(Error::BadKey(_))
    ));
}

#[test]
fn public_grant_decode_rejects_secret_field() {
    // A secret field (key 10) is not permitted in a standalone public grant.
    let publisher = SecretSigningKey::from_seed([2u8; 32]);
    let write =
        Capability::new_write(sample_grant(), Secret32([9u8; 32]), &publisher).unwrap();
    let secret_bytes = write.encode_secret();
    assert!(matches!(
        PublicGrant::decode(&secret_bytes, lim()),
        Err(Error::Schema(_))
    ));
}

#[test]
fn delegation_narrows_only() {
    let admin = SecretSigningKey::from_seed([1u8; 32]);
    let mut parent = sample_grant();
    parent.authority = vec![Authority::Read, Authority::Publish];
    parent.publisher_pub = Some(SecretSigningKey::from_seed([2u8; 32]).public().to_bytes());
    parent.sign(&admin);

    // Valid narrowing: drop publish, tighten validity, cap the forward extent.
    let child = delegate(
        &parent,
        &admin,
        Delegation {
            branches: None,
            authority: vec![Authority::Read],
            validity: Validity { not_before: 120, not_after: 180 },
            max_epoch: Some(9),
        },
    )
    .unwrap();
    child.verify(&admin.public()).unwrap();
    assert_eq!(child.parent_grant_id, Some(parent.cap_id()));
    assert_eq!(child.authority, vec![Authority::Read]);
    // The epoch ceiling is its OWN field: the child keeps the parent's exact
    // epoch scope and its epoch-specific topic, so the grant never claims an
    // epoch that disagrees with the topic it carries.
    assert_eq!(child.epoch, parent.epoch);
    assert_eq!(child.topic, parent.topic);
    assert_eq!(child.max_epoch, Some(Epoch(9)));

    // ...and that ceiling is admin-authenticated: it survives a canonical
    // round-trip, and tampering with it invalidates the signature.
    let decoded = PublicGrant::decode(&child.encode(), lim()).unwrap();
    assert_eq!(decoded, child);
    decoded.verify(&admin.public()).unwrap();
    let mut tampered = child.clone();
    tampered.max_epoch = Some(Epoch(11));
    assert!(tampered.verify(&admin.public()).is_err());

    // A ceiling below the epoch the grant is scoped to is unusable, not narrow.
    assert!(matches!(
        delegate(
            &parent,
            &admin,
            Delegation {
                branches: None,
                authority: vec![Authority::Read],
                validity: Validity { not_before: 120, not_after: 180 },
                max_epoch: Some(5),
            }
        ),
        Err(Error::Delegation(_))
    ));

    // Re-delegating may only lower the ceiling, never raise it.
    assert!(matches!(
        delegate(
            &child,
            &admin,
            Delegation {
                branches: None,
                authority: vec![Authority::Read],
                validity: Validity { not_before: 120, not_after: 180 },
                max_epoch: Some(10),
            }
        ),
        Err(Error::Delegation(_))
    ));
    // An unspecified ceiling inherits the parent's rather than escaping it.
    let grandchild = delegate(
        &child,
        &admin,
        Delegation {
            branches: None,
            authority: vec![Authority::Read],
            validity: Validity { not_before: 120, not_after: 180 },
            max_epoch: None,
        },
    )
    .unwrap();
    assert_eq!(grandchild.max_epoch, Some(Epoch(9)));

    // Widen authority -> rejected.
    assert!(matches!(
        delegate(
            &parent,
            &admin,
            Delegation {
                branches: None,
                authority: vec![Authority::Read, Authority::Publish, Authority::Admin],
                validity: Validity { not_before: 120, not_after: 180 },
                max_epoch: None,
            }
        ),
        Err(Error::Delegation(_))
    ));

    // Widen validity -> rejected.
    assert!(matches!(
        delegate(
            &parent,
            &admin,
            Delegation {
                branches: None,
                authority: vec![Authority::Read],
                validity: Validity { not_before: 0, not_after: 999 },
                max_epoch: None,
            }
        ),
        Err(Error::Delegation(_))
    ));
}

// --- branch-set delegation (design §4.2) ------------------------------------

fn sb(branch: u8, topic: u8) -> ScopedBranch {
    ScopedBranch {
        branch: BranchId::from_bytes([branch; 32]),
        topic: TopicId::from_bytes([topic; 32]),
    }
}

/// A grant scoped to three branches. The entries are supplied out of order on
/// purpose: `set_branch_scope` is what establishes the canonical order.
fn branch_set_grant() -> PublicGrant {
    let mut g = sample_grant();
    g.authority = vec![Authority::Admin];
    g.set_branch_scope(vec![sb(0x30, 0xA3), sb(0x10, 0xA1), sb(0x20, 0xA2)]).unwrap();
    g
}

#[test]
fn branch_scope_is_canonical_and_authenticated() {
    let admin = SecretSigningKey::from_seed([1u8; 32]);
    let mut g = branch_set_grant();

    // Lowest-ordered branch becomes the primary (fields 3 + 5); the rest follow
    // in ascending order, each still paired with its own topic.
    assert_eq!(g.branch, BranchId::from_bytes([0x10; 32]));
    assert_eq!(g.topic, TopicId::from_bytes([0xA1; 32]));
    assert_eq!(g.extra_branches, vec![sb(0x20, 0xA2), sb(0x30, 0xA3)]);
    assert_eq!(g.branch_scope(), vec![sb(0x10, 0xA1), sb(0x20, 0xA2), sb(0x30, 0xA3)]);

    for b in [0x10u8, 0x20, 0x30] {
        assert!(g.covers_branch(&BranchId::from_bytes([b; 32])));
    }
    assert!(!g.covers_branch(&BranchId::from_bytes([0x40; 32])));

    // The set is part of the admin-signed bytes and survives a canonical
    // round-trip unchanged.
    g.sign(&admin);
    let decoded = PublicGrant::decode(&g.encode(), lim()).unwrap();
    assert_eq!(decoded, g);
    assert_eq!(decoded.cap_id(), g.cap_id());
    decoded.verify(&admin.public()).unwrap();

    // Swapping in a different branch — or re-pointing one at another topic —
    // breaks the signature, so a broker cannot be handed a widened set.
    let mut wider = g.clone();
    wider.extra_branches.push(sb(0x40, 0xA4));
    assert!(wider.verify(&admin.public()).is_err());
    let mut retopiced = g.clone();
    retopiced.extra_branches[0].topic = TopicId::from_bytes([0xEE; 32]);
    assert!(retopiced.verify(&admin.public()).is_err());
    // ...and the cap_id moves with the set, so the two are not interchangeable.
    assert_ne!(wider.cap_id(), g.cap_id());
}

#[test]
fn set_branch_scope_rejects_degenerate_sets() {
    let mut g = sample_grant();
    assert!(matches!(g.set_branch_scope(vec![]), Err(Error::Schema(_))));
    // Same branch twice.
    assert!(matches!(
        g.set_branch_scope(vec![sb(0x10, 0xA1), sb(0x10, 0xA2)]),
        Err(Error::NonCanonical(_))
    ));
    // Two branches claiming one epoch-specific topic.
    assert!(matches!(
        g.set_branch_scope(vec![sb(0x10, 0xA1), sb(0x20, 0xA1)]),
        Err(Error::Schema(_))
    ));
}

#[test]
fn delegation_narrows_branch_set() {
    let admin = SecretSigningKey::from_seed([1u8; 32]);
    let mut parent = branch_set_grant();
    parent.sign(&admin);

    // Narrow to a two-branch subset (requested out of order). Each retained
    // branch keeps the topic the PARENT bound to it; the primary is re-picked
    // as the lowest retained branch, so the child is canonical too.
    let child = delegate(
        &parent,
        &admin,
        Delegation {
            branches: Some(vec![
                BranchId::from_bytes([0x30; 32]),
                BranchId::from_bytes([0x20; 32]),
            ]),
            authority: vec![Authority::Admin],
            validity: Validity { not_before: 120, not_after: 180 },
            max_epoch: None,
        },
    )
    .unwrap();
    child.verify(&admin.public()).unwrap();
    assert_eq!(child.branch_scope(), vec![sb(0x20, 0xA2), sb(0x30, 0xA3)]);
    assert_eq!(child.branch, BranchId::from_bytes([0x20; 32]));
    assert_eq!(child.topic, TopicId::from_bytes([0xA2; 32]));
    assert_eq!(child.parent_grant_id, Some(parent.cap_id()));
    assert!(!child.covers_branch(&BranchId::from_bytes([0x10; 32])));
    assert_eq!(PublicGrant::decode(&child.encode(), lim()).unwrap(), child);

    // An unspecified set inherits the parent's whole set rather than escaping it.
    let inherited = delegate(
        &parent,
        &admin,
        Delegation {
            branches: None,
            authority: vec![Authority::Admin],
            validity: Validity { not_before: 120, not_after: 180 },
            max_epoch: None,
        },
    )
    .unwrap();
    assert_eq!(inherited.branch_scope(), parent.branch_scope());

    // Narrowing all the way to one branch drops field 18 entirely, so the child
    // is an ordinary single-branch grant.
    let single = delegate(
        &parent,
        &admin,
        Delegation {
            branches: Some(vec![BranchId::from_bytes([0x30; 32])]),
            authority: vec![Authority::Admin],
            validity: Validity { not_before: 120, not_after: 180 },
            max_epoch: None,
        },
    )
    .unwrap();
    assert_eq!(single.branch_scope(), vec![sb(0x30, 0xA3)]);
    assert!(single.extra_branches.is_empty());
    assert_eq!(PublicGrant::decode(&single.encode(), lim()).unwrap(), single);

    let narrow = |from: &PublicGrant, branches: Option<Vec<BranchId>>| {
        delegate(
            from,
            &admin,
            Delegation {
                branches,
                authority: vec![Authority::Admin],
                validity: Validity { not_before: 120, not_after: 180 },
                max_epoch: None,
            },
        )
    };

    // A branch the parent never had -> rejected, not minted.
    assert!(matches!(
        narrow(&parent, Some(vec![BranchId::from_bytes([0x40; 32])])),
        Err(Error::Delegation(_))
    ));
    // An empty set is not a narrow grant, it is an unusable one.
    assert!(matches!(narrow(&parent, Some(vec![])), Err(Error::Delegation(_))));
    // A repeated branch is rejected rather than silently collapsed.
    assert!(matches!(
        narrow(
            &parent,
            Some(vec![BranchId::from_bytes([0x20; 32]), BranchId::from_bytes([0x20; 32])])
        ),
        Err(Error::Delegation(_))
    ));
    // THE transitive invariant: re-delegating cannot recover a branch an
    // ancestor already dropped, from either the two-branch or one-branch child.
    assert!(matches!(
        narrow(&child, Some(vec![BranchId::from_bytes([0x10; 32])])),
        Err(Error::Delegation(_))
    ));
    assert!(matches!(
        narrow(&single, Some(vec![BranchId::from_bytes([0x20; 32])])),
        Err(Error::Delegation(_))
    ));
    // ...and inheriting from the child inherits only what the child still has.
    assert_eq!(narrow(&child, None).unwrap().branch_scope(), child.branch_scope());
}

/// Every `PublicGrant` field is public, so a parent can be hand-built (or
/// mutated) into a non-canonical branch set without ever passing through
/// `decode` / `set_branch_scope`. `delegate` must reject such a parent on BOTH
/// paths — the inheriting one (`branches: None`, which copies the parent's set
/// verbatim) as much as the selecting one. Otherwise it would admin-sign a child
/// over a malformed set: `verify` would then succeed on a grant its own `decode`
/// rejects, i.e. an authenticated but structurally invalid grant.
#[test]
fn delegation_rejects_a_non_canonical_parent() {
    let admin = SecretSigningKey::from_seed([1u8; 32]);
    let good = branch_set_grant();

    // Each of these is a shape `PublicGrant::decode` refuses; the primary entry
    // of `branch_set_grant` is sb(0x10, 0xA1).
    let bad_sets = [
        // extras out of ascending order
        vec![sb(0x30, 0xA3), sb(0x20, 0xA2)],
        // an extra repeating the primary branch
        vec![sb(0x10, 0xA9), sb(0x20, 0xA2)],
        // an extra below the primary (the primary must be the lowest)
        vec![sb(0x05, 0xA5), sb(0x20, 0xA2)],
        // two branches claiming one epoch-specific topic
        vec![sb(0x20, 0xA1)],
    ];

    for bad in bad_sets {
        let mut parent = good.clone();
        parent.extra_branches = bad;
        parent.sign(&admin);
        // The malformed parent really is one `decode` refuses...
        assert!(PublicGrant::decode(&parent.encode(), lim()).is_err());

        for branches in [
            None,
            // ...and selecting out of it is refused too, including a selection
            // that would look canonical on its own.
            Some(vec![BranchId::from_bytes([0x20; 32])]),
        ] {
            let r = delegate(
                &parent,
                &admin,
                Delegation {
                    branches,
                    authority: vec![Authority::Admin],
                    validity: Validity { not_before: 120, not_after: 180 },
                    max_epoch: None,
                },
            );
            // No child at all — never a signed one.
            assert!(r.is_err(), "delegate minted a child from a non-canonical parent");
        }
    }

    // The same delegation off the canonical parent still succeeds, so the guard
    // rejects malformed parents rather than delegation generally.
    delegate(
        &good,
        &admin,
        Delegation {
            branches: None,
            authority: vec![Authority::Admin],
            validity: Validity { not_before: 120, not_after: 180 },
            max_epoch: None,
        },
    )
    .unwrap();
}

// ============================================================================
// Block & commit envelopes
// ============================================================================

fn sample_ctx() -> BlockContext {
    BlockContext {
        repo: RepoId::from_bytes([1u8; 32]),
        branch: BranchId::from_bytes([2u8; 32]),
        epoch: Epoch(7),
        kind: ObjectKind::Operation,
    }
}

#[test]
fn block_seal_open_roundtrip() {
    let k = Secret32([4u8; 32]);
    let ctx = sample_ctx();
    let object = ObjectId::from_bytes([5u8; 32]);
    let env = seal_block_random(&k, &ctx, &object, 0, 1, b"some quads").unwrap();
    let pt = open_block(&k, &ctx, &env).unwrap();
    assert_eq!(pt, b"some quads");
}

#[test]
fn block_open_wrong_epoch_fails_closed() {
    let k = Secret32([4u8; 32]);
    let ctx = sample_ctx();
    let object = ObjectId::from_bytes([5u8; 32]);
    let env = seal_block_random(&k, &ctx, &object, 0, 1, b"payload").unwrap();

    let mut wrong = ctx;
    wrong.epoch = Epoch(8); // wrong epoch -> AD mismatch AND wrong key
    assert!(matches!(open_block(&k, &wrong, &env), Err(Error::Decrypt)));
}

#[test]
fn block_open_wrong_branch_fails_closed() {
    let k = Secret32([4u8; 32]);
    let ctx = sample_ctx();
    let object = ObjectId::from_bytes([5u8; 32]);
    let env = seal_block_random(&k, &ctx, &object, 0, 1, b"payload").unwrap();

    let mut wrong = ctx;
    wrong.branch = BranchId::from_bytes([0xEE; 32]);
    assert!(matches!(open_block(&k, &wrong, &env), Err(Error::Decrypt)));
}

#[test]
fn block_tampered_ciphertext_fails_closed() {
    let k = Secret32([4u8; 32]);
    let ctx = sample_ctx();
    let object = ObjectId::from_bytes([5u8; 32]);
    let mut env =
        seal_block(&k, &ctx, &object, &sparq_e2ee_ng::ids::BlockId::from_bytes([6u8; 32]),
                   0, 1, [7u8; AEAD_NONCE_LEN], b"payload").unwrap();
    env.ciphertext[0] ^= 0x01;
    assert!(matches!(open_block(&k, &ctx, &env), Err(Error::Decrypt)));
}

#[test]
fn block_padding_hides_length() {
    let k = Secret32([4u8; 32]);
    let ctx = sample_ctx();
    let object = ObjectId::from_bytes([5u8; 32]);
    // A 1-byte and a 100-byte payload land in the same smallest pad class.
    let e1 = seal_block_random(&k, &ctx, &object, 0, 1, b"a").unwrap();
    let e2 = seal_block_random(&k, &ctx, &object, 0, 1, &[0u8; 100]).unwrap();
    assert_eq!(e1.pad_class, PAD_CLASSES[0] as u64);
    assert_eq!(e1.pad_class, e2.pad_class);
    assert_eq!(e1.ciphertext.len(), e2.ciphertext.len());
}

#[test]
fn block_envelope_encode_decode_roundtrip() {
    let k = Secret32([4u8; 32]);
    let ctx = sample_ctx();
    let object = ObjectId::from_bytes([5u8; 32]);
    let env = seal_block_random(&k, &ctx, &object, 2, 4, b"payload").unwrap();
    let bytes = env.encode();
    let env2 = BlockEnvelope::decode(&bytes, lim()).unwrap();
    assert_eq!(env, env2);
    assert_eq!(open_block(&k, &ctx, &env2).unwrap(), b"payload");
}

#[test]
fn commit_sign_verify_and_id_stable() {
    let author = SecretSigningKey::from_seed([3u8; 32]);
    let mut commit = Commit {
        repo: RepoId::from_bytes([1u8; 32]),
        branch: BranchId::from_bytes([2u8; 32]),
        epoch: Epoch(7),
        parents: vec![CommitId::from_bytes([8u8; 32])],
        author: AuthorKeyId::from_bytes([0u8; 32]),
        clock: 42,
        crdt_kind: "or-set-quads-v0".to_string(),
        operations: vec![ObjectId::from_bytes([5u8; 32])],
        snapshot: None,
        author_sig: None,
    };
    commit.sign(&author);
    commit.verify().unwrap();
    assert_eq!(commit.author.as_bytes(), &author.public().to_bytes());

    // Seal into a root block and check CommitId = SHA-256(canonical envelope).
    let k = Secret32([4u8; 32]);
    let ctx = BlockContext {
        repo: commit.repo,
        branch: commit.branch,
        epoch: commit.epoch,
        kind: ObjectKind::Commit,
    };
    let object = ObjectId::from_bytes([9u8; 32]);
    let env = seal_block(&k, &ctx, &object,
                         &sparq_e2ee_ng::ids::BlockId::from_bytes([10u8; 32]),
                         0, 1, [11u8; AEAD_NONCE_LEN], &commit.encode()).unwrap();
    assert_eq!(env.commit_id(), env.commit_id());

    // Decode the plaintext back and re-verify.
    let pt = open_block(&k, &ctx, &env).unwrap();
    let commit2 = Commit::decode(&pt, lim()).unwrap();
    assert_eq!(commit, commit2);
    commit2.verify().unwrap();
}

// ============================================================================
// Epoch transitions
// ============================================================================

#[test]
fn epoch_transition_sign_verify() {
    let admin = SecretSigningKey::from_seed([1u8; 32]);
    let mut t = EpochTransition {
        repo: RepoId::from_bytes([1u8; 32]),
        branch: BranchId::from_bytes([2u8; 32]),
        old_epoch: Epoch(7),
        new_epoch: Epoch(8),
        old_topic: TopicId::from_bytes([3u8; 32]),
        new_topic: TopicId::from_bytes([4u8; 32]),
        new_publishers: vec![SecretSigningKey::from_seed([2u8; 32]).public().to_bytes()],
        history_policy: HistoryPolicy::ForwardOnly,
        admin_sig: None,
    };
    t.sign(&admin).unwrap();
    t.verify(&admin.public()).unwrap();

    let bytes = t.encode();
    let t2 = EpochTransition::decode(&bytes, lim()).unwrap();
    assert_eq!(t, t2);
    t2.verify(&admin.public()).unwrap();
}

#[test]
fn epoch_transition_must_increase() {
    let admin = SecretSigningKey::from_seed([1u8; 32]);
    let mut t = EpochTransition {
        repo: RepoId::from_bytes([1u8; 32]),
        branch: BranchId::from_bytes([2u8; 32]),
        old_epoch: Epoch(8),
        new_epoch: Epoch(8), // not strictly increasing
        old_topic: TopicId::from_bytes([3u8; 32]),
        new_topic: TopicId::from_bytes([4u8; 32]),
        new_publishers: vec![],
        history_policy: HistoryPolicy::HistoryRekeyed,
        admin_sig: None,
    };
    assert!(matches!(t.sign(&admin), Err(Error::Schema(_))));
}
