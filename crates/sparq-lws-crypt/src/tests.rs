use crate::{envelope::seal_with_nonce, *};

fn hex(input: &str) -> Vec<u8> {
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}

#[tokio::test]
async fn known_answer_vectors() {
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../tests/vectors/e1.json")).unwrap();
    for v in json["vectors"].as_array().unwrap() {
        let s = |name: &str| v[name].as_str().unwrap();
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, &hex(s("kek"))).unwrap();
        let provider = LocalKeyProvider::from_file(file.path()).unwrap();
        let context = KeyContext {
            pod: s("pod").into(),
            scheme: s("scheme").into(),
        };
        assert_eq!(context.to_bytes().unwrap(), hex(s("context")));
        let raw: [u8; 32] = hex(s("dek")).try_into().unwrap();
        let wrapped = provider
            .wrap_with_nonce(
                &SecretKey::from_bytes(raw),
                &context,
                hex(s("wrap_nonce")).try_into().unwrap(),
            )
            .unwrap();
        assert_eq!(wrapped.kek_id, s("kek_id"));
        assert_eq!(wrapped.bytes, hex(s("wrapped_key")));
        assert_eq!(
            wrapped.to_bytes().unwrap(),
            hex(s("serialized_wrapped_key"))
        );
        let restored = WrappedKey::from_bytes(&hex(s("serialized_wrapped_key"))).unwrap();
        let key = provider.unwrap(&restored, &context).await.unwrap();
        assert_eq!(key.as_bytes(), &raw);
        let dek = Dek::new(key, hex(s("dek_id")).try_into().unwrap(), 0).unwrap();
        let envelope = seal_with_nonce(
            &dek,
            s("blob_key"),
            &hex(s("plaintext")),
            hex(s("nonce")).try_into().unwrap(),
        )
        .unwrap();
        assert_eq!(envelope, hex(s("envelope")), "{}", s("name"));
        assert_eq!(
            open(&dek, s("blob_key"), &envelope).unwrap(),
            hex(s("plaintext"))
        );
    }
}
