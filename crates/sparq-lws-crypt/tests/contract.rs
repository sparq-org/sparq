#![cfg(not(target_arch = "wasm32"))]

use std::{collections::HashSet, io::Write, sync::Arc};

use proptest::prelude::*;
use sparq_lws_crypt::*;

fn dek() -> Dek {
    Dek::new(SecretKey::from_bytes([0x31; 32]), [0x71; 16], 0).unwrap()
}

fn context() -> KeyContext {
    KeyContext {
        pod: "https://pods.example/alice/".into(),
        scheme: "b1".into(),
    }
}

fn provider(bytes: &[u8]) -> (tempfile::NamedTempFile, LocalKeyProvider) {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(bytes).unwrap();
    let provider = LocalKeyProvider::from_file(file.path()).unwrap();
    (file, provider)
}

#[test]
fn envelope_roundtrip_and_sniff() {
    let dek = dek();
    for plaintext in [b"".as_slice(), b"hello\0\xff"] {
        let envelope = seal(&dek, "b1/\u{e9}", plaintext).unwrap();
        assert!(is_envelope(&envelope));
        let header = parse_header(&envelope).unwrap();
        assert_eq!(header.dek_id, *dek.id());
        assert_eq!(parse_header(&envelope[..33]), Some(header));
        assert_eq!(open(&dek, "b1/\u{e9}", &envelope).unwrap(), plaintext);
    }
    assert_eq!(dek.seal_count(), 2);
    assert!(!is_envelope(b"b1/plaintext"));
    assert!(!is_envelope(b"SQL"));
    assert!(is_envelope(b"SQLB"));
    assert!(is_envelope(b"SQLB\x02"));
    assert_eq!(parse_header(b"SQLB\x02"), None);
}

#[test]
fn every_envelope_byte_is_checked() {
    let dek = dek();
    let original = seal(&dek, "b1/key", b"payload with multiple bytes").unwrap();
    for offset in 0..original.len() {
        let mut tampered = original.clone();
        tampered[offset] ^= 1;
        let expected = match offset {
            0..=3 => Error::MalformedInput,
            4 => Error::UnsupportedVersion,
            _ => Error::AuthenticationFailed,
        };
        assert_eq!(
            open(&dek, "b1/key", &tampered),
            Err(expected),
            "byte {offset}"
        );
    }
}

#[test]
fn wrong_blob_and_dek_are_rejected() {
    let dek = dek();
    let original = seal(&dek, "b1/key", b"payload").unwrap();
    assert_eq!(
        open(&dek, "b1/other", &original),
        Err(Error::AuthenticationFailed)
    );
    let wrong_key = Dek::new(SecretKey::from_bytes([0x32; 32]), *dek.id(), 0).unwrap();
    assert_eq!(
        open(&wrong_key, "b1/key", &original),
        Err(Error::AuthenticationFailed)
    );
    let wrong_id = Dek::new(SecretKey::from_bytes([0x31; 32]), [0x72; 16], 0).unwrap();
    assert_eq!(
        open(&wrong_id, "b1/key", &original),
        Err(Error::AuthenticationFailed)
    );
}

#[test]
fn envelope_truncation_versions_and_trailing_bytes() {
    let dek = dek();
    let original = seal(&dek, "b1/key", b"payload").unwrap();
    for len in 0..original.len() {
        let expected = if len < 49 {
            Error::MalformedInput
        } else {
            Error::AuthenticationFailed
        };
        assert_eq!(
            open(&dek, "b1/key", &original[..len]),
            Err(expected),
            "length {len}"
        );
        if len < 33 {
            assert_eq!(parse_header(&original[..len]), None);
        }
    }
    for version in 0..=255 {
        if version == 1 {
            continue;
        }
        let mut bytes = original.clone();
        bytes[4] = version;
        assert!(is_envelope(&bytes));
        assert_eq!(parse_header(&bytes), None);
        assert_eq!(open(&dek, "b1/key", &bytes), Err(Error::UnsupportedVersion));
    }
    let mut trailing = original;
    trailing.push(0);
    assert_eq!(
        open(&dek, "b1/key", &trailing),
        Err(Error::AuthenticationFailed)
    );
}

#[test]
fn nonces_are_fresh_across_ten_thousand_seals() {
    let dek = dek();
    let mut nonces = HashSet::new();
    for _ in 0..10_000 {
        let bytes = seal(&dek, "b1/key", b"same plaintext").unwrap();
        assert!(nonces.insert(parse_header(&bytes).unwrap().nonce));
        assert_eq!(open(&dek, "b1/key", &bytes).unwrap(), b"same plaintext");
    }
    assert_eq!(dek.seal_count(), 10_000);
}

#[test]
fn seal_budget_is_bounded_and_reads_still_work() {
    let key = Dek::new(
        SecretKey::from_bytes([0x31; 32]),
        [0x71; 16],
        MAX_SEALS_PER_DEK - 1,
    )
    .unwrap();
    let bytes = seal(&key, "b1/key", b"last seal").unwrap();
    assert_eq!(key.seal_count(), MAX_SEALS_PER_DEK);
    assert_eq!(
        seal(&key, "b1/key", b"one too many"),
        Err(Error::SealLimitReached)
    );
    assert_eq!(key.seal_count(), MAX_SEALS_PER_DEK);
    assert_eq!(open(&key, "b1/key", &bytes).unwrap(), b"last seal");
    assert!(matches!(
        Dek::new(
            SecretKey::from_bytes([0; 32]),
            [0; 16],
            MAX_SEALS_PER_DEK + 1
        ),
        Err(Error::SealLimitReached)
    ));
}

#[test]
fn concurrent_seals_share_one_budget() {
    let key = Arc::new(
        Dek::new(
            SecretKey::from_bytes([0x31; 32]),
            [0x71; 16],
            MAX_SEALS_PER_DEK - 4,
        )
        .unwrap(),
    );
    let threads: Vec<_> = (0..12)
        .map(|_| {
            let key = Arc::clone(&key);
            std::thread::spawn(move || seal(&key, "b1/key", b"concurrent"))
        })
        .collect();
    let mut nonces = HashSet::new();
    for thread in threads {
        match thread.join().unwrap() {
            Ok(bytes) => {
                assert!(nonces.insert(parse_header(&bytes).unwrap().nonce));
            }
            Err(e) => assert_eq!(e, Error::SealLimitReached),
        }
    }
    assert_eq!(nonces.len(), 4);
    assert_eq!(key.seal_count(), MAX_SEALS_PER_DEK);
}

#[tokio::test]
async fn provider_roundtrip_through_trait_and_serialization() {
    let (_file, local) = provider(&[0x41; 32]);
    let local: &dyn KeyProvider = &local;
    assert_eq!(local.id(), "local");
    let ctx = context();
    let (first, wrapped) = local.generate_data_key(&ctx).await.unwrap();
    let first_bytes = *first.as_bytes();
    let stored = wrapped.to_bytes().unwrap();
    let restored = WrappedKey::from_bytes(&stored).unwrap();
    let second = local.unwrap(&restored, &ctx).await.unwrap();
    assert_eq!(first.as_bytes(), second.as_bytes());
    let original = Dek::new(first, [0x71; 16], 0).unwrap();
    let reopened = Dek::new(second, [0x71; 16], 1).unwrap();
    let envelope = seal(&original, "b1/key", b"roundtrip").unwrap();
    assert_eq!(open(&reopened, "b1/key", &envelope).unwrap(), b"roundtrip");
    let (fresh, another) = local.generate_data_key(&ctx).await.unwrap();
    assert_ne!(fresh.as_bytes(), &first_bytes);
    assert_eq!(
        fresh.as_bytes(),
        local.unwrap(&another, &ctx).await.unwrap().as_bytes()
    );
    assert_ne!(&wrapped.bytes[..12], &another.bytes[..12]);
}

#[tokio::test]
async fn unwrap_binds_both_context_fields() {
    let (_file, local) = provider(&[0x41; 32]);
    let (_, wrapped) = local.generate_data_key(&context()).await.unwrap();
    for wrong in [
        KeyContext {
            pod: "https://pods.example/bob/".into(),
            scheme: "b1".into(),
        },
        KeyContext {
            pod: context().pod,
            scheme: "other".into(),
        },
    ] {
        assert!(matches!(
            local.unwrap(&wrapped, &wrong).await,
            Err(Error::AuthenticationFailed)
        ));
    }
}

#[tokio::test]
async fn wrong_kek_is_reported_before_unwrap() {
    let (_first_file, first) = provider(&[0x41; 32]);
    let (_other_file, other) = provider(&[0x42; 32]);
    let (_, mut wrapped) = first.generate_data_key(&context()).await.unwrap();
    assert!(matches!(
        other.unwrap(&wrapped, &context()).await,
        Err(Error::WrongKek)
    ));
    wrapped.bytes.clear();
    assert!(matches!(
        other.unwrap(&wrapped, &context()).await,
        Err(Error::WrongKek)
    ));
    wrapped.provider_id = "other".into();
    assert!(matches!(
        first.unwrap(&wrapped, &context()).await,
        Err(Error::WrongProvider)
    ));
}

#[tokio::test]
async fn every_wrapped_payload_byte_is_authenticated() {
    let (_file, local) = provider(&[0x41; 32]);
    let (_, wrapped) = local.generate_data_key(&context()).await.unwrap();
    for offset in 0..wrapped.bytes.len() {
        let mut changed = wrapped.clone();
        changed.bytes[offset] ^= 1;
        assert!(
            matches!(
                local.unwrap(&changed, &context()).await,
                Err(Error::AuthenticationFailed)
            ),
            "byte {offset}"
        );
    }
    for len in 0..wrapped.bytes.len() {
        let mut changed = wrapped.clone();
        changed.bytes.truncate(len);
        assert!(
            matches!(
                local.unwrap(&changed, &context()).await,
                Err(Error::MalformedInput)
            ),
            "length {len}"
        );
    }
    let mut changed = wrapped;
    changed.bytes.push(0);
    assert!(matches!(
        local.unwrap(&changed, &context()).await,
        Err(Error::MalformedInput)
    ));
}

#[test]
fn wrapped_serialization_is_strict() {
    let wrapped = WrappedKey {
        provider_id: "local".into(),
        kek_id: "test-id".into(),
        bytes: vec![3; 60],
    };
    let original = wrapped.to_bytes().unwrap();
    assert_eq!(WrappedKey::from_bytes(&original).unwrap(), wrapped);
    for len in 0..original.len() {
        assert_eq!(
            WrappedKey::from_bytes(&original[..len]),
            Err(Error::MalformedInput),
            "length {len}"
        );
    }
    let mut changed = original.clone();
    changed.push(0);
    assert_eq!(WrappedKey::from_bytes(&changed), Err(Error::MalformedInput));
    for version in 0..=255 {
        if version == 1 {
            continue;
        }
        let mut changed = original.clone();
        changed[4] = version;
        assert_eq!(
            WrappedKey::from_bytes(&changed),
            Err(Error::UnsupportedVersion)
        );
    }
    let mut changed = original.clone();
    changed[0] ^= 1;
    assert_eq!(WrappedKey::from_bytes(&changed), Err(Error::MalformedInput));
    let mut changed = original.clone();
    changed[5..9].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(WrappedKey::from_bytes(&changed), Err(Error::MalformedInput));
    let mut changed = original;
    changed[9] = 0xff;
    assert_eq!(WrappedKey::from_bytes(&changed), Err(Error::MalformedInput));
}

#[test]
fn context_uses_unambiguous_utf8_byte_lengths() {
    let first = KeyContext {
        pod: "a".into(),
        scheme: "bc".into(),
    };
    let second = KeyContext {
        pod: "ab".into(),
        scheme: "c".into(),
    };
    assert_ne!(first.to_bytes().unwrap(), second.to_bytes().unwrap());
    let unicode = KeyContext {
        pod: "\u{e9}\0".into(),
        scheme: "b1".into(),
    };
    assert_eq!(
        unicode.to_bytes().unwrap(),
        b"SQLC\x01\x00\x00\x00\x03\xc3\xa9\x00\x00\x00\x00\x02b1"
    );
}

#[test]
fn key_file_size_type_and_availability_are_checked() {
    for len in [0, 1, 31, 33, 4096] {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&vec![0x41; len]).unwrap();
        assert!(matches!(
            LocalKeyProvider::from_file(file.path()),
            Err(Error::MalformedInput)
        ));
    }
    let dir = tempfile::tempdir().unwrap();
    assert!(LocalKeyProvider::from_file(dir.path()).is_err());
    assert!(matches!(
        LocalKeyProvider::from_file(dir.path().join("missing")),
        Err(Error::ProviderUnavailable)
    ));
}

#[cfg(unix)]
#[test]
fn unix_permissions_are_checked_on_the_opened_file() {
    use std::{
        fs,
        os::unix::fs::{symlink, PermissionsExt},
    };
    let (file, _local) = provider(&[0x41; 32]);
    for mode in [0o640, 0o604, 0o620, 0o602, 0o610, 0o601] {
        fs::set_permissions(file.path(), fs::Permissions::from_mode(mode)).unwrap();
        assert!(matches!(
            LocalKeyProvider::from_file(file.path()),
            Err(Error::InsecureKeyFilePermissions)
        ));
    }
    let dir = tempfile::tempdir().unwrap();
    let alias = dir.path().join("alias");
    symlink(file.path(), &alias).unwrap();
    assert!(matches!(
        LocalKeyProvider::from_file(&alias),
        Err(Error::InsecureKeyFilePermissions)
    ));
    for mode in [0o600, 0o400] {
        fs::set_permissions(file.path(), fs::Permissions::from_mode(mode)).unwrap();
        assert!(LocalKeyProvider::from_file(&alias).is_ok());
    }
}

#[tokio::test]
async fn diagnostics_are_redacted() {
    let (_file, local) = provider(&[0x41; 32]);
    let ctx = context();
    let (secret, wrapped) = local.generate_data_key(&ctx).await.unwrap();
    let secret_debug = format!("{:?}", secret.as_bytes());
    let secret_hex: String = secret
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let key = Dek::new(secret, [0x71; 16], 0).unwrap();
    let header = parse_header(&seal(&key, "b1/key", b"private body").unwrap()).unwrap();
    let diagnostics = format!(
        "{local:?} {ctx:?} {wrapped:?} {key:?} {header:?} {:?}",
        SecretKey::from_bytes([0x41; 32])
    );
    for private in [
        secret_debug,
        secret_hex,
        format!("{:?}", [0x41; 32]),
        "41".repeat(32),
        "A".repeat(32),
    ] {
        assert!(!diagnostics.contains(&private));
    }
    for error in [
        Error::MalformedInput,
        Error::UnsupportedVersion,
        Error::WrongProvider,
        Error::WrongKek,
        Error::AuthenticationFailed,
        Error::ProviderUnavailable,
        Error::InsecureKeyFilePermissions,
        Error::SealLimitReached,
        Error::InputTooLong,
    ] {
        assert!(!format!("{error:?} {error}").contains("AAAA"));
    }
}

proptest! {
    #[test]
    fn open_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..4096), blob in ".{0,128}") {
        let key = dek();
        let _ = open(&key, &blob, &bytes);
        let _ = parse_header(&bytes);
        let _ = is_envelope(&bytes);
        let _ = WrappedKey::from_bytes(&bytes);
        let mut framed = b"SQLB\x01".to_vec();
        framed.extend_from_slice(key.id());
        framed.extend_from_slice(&[0; 12]);
        framed.extend_from_slice(&bytes);
        let _ = open(&key, &blob, &framed);
    }

    #[test]
    fn envelope_roundtrip_arbitrary_plaintext(plaintext in proptest::collection::vec(any::<u8>(), 0..4096), blob in ".{0,128}") {
        let key = dek();
        let envelope = seal(&key, &blob, &plaintext).unwrap();
        prop_assert_eq!(open(&key, &blob, &envelope).unwrap(), plaintext);
    }

    #[test]
    fn wrapped_serialization_roundtrip(provider_id in ".{0,64}", kek_id in ".{0,64}", bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let wrapped = WrappedKey { provider_id, kek_id, bytes };
        let encoded = wrapped.to_bytes().unwrap();
        prop_assert_eq!(WrappedKey::from_bytes(&encoded).unwrap(), wrapped);
    }
}
