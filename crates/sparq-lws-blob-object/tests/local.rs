#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt};
use object_store::{local::LocalFileSystem, path::Path, ObjectStoreExt};
use sparq_lws_blob_object::{Error, ObjectBlobStore};

const KEY: &str = "b1-0123456789abcdef0123456789abcdef";

#[tokio::test]
async fn invalid_keys_are_refused_by_local_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    let store = ObjectBlobStore::new(
        Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap()),
        "blobs",
    )
    .unwrap();
    for key in [
        "",
        "b1-0",
        "0123456789abcdef0123456789abcdef",
        "b1-0123456789abcdef0123456789abcde",
        "b1-0123456789abcdef0123456789abcdef0",
        "b1-0123456789abcdef0123456789abcdeA",
        "b1-0123456789abcdef0123456789abcdeg",
        "b1-0123456789abcdef0123456789abcdé",
        "b1-0123456789abcdef0123456789abcdef\n",
        "../outside",
        "$pool$../outside",
        "$pool$$pool$b1-0123456789abcdef0123456789abcdef",
        "$pool$0123456789abcdef0123456789abcdef",
        "b1-0123456789abcdef0123456789abcdef/x",
        "pool/b1-0123456789abcdef0123456789abcdef",
    ] {
        assert_eq!(store.put(key, Bytes::new()).await, Err(Error::InvalidKey));
        assert_eq!(store.get(key).await, Err(Error::InvalidKey));
        assert_eq!(store.exists(key).await, Err(Error::InvalidKey));
        assert_eq!(store.delete(key).await, Err(Error::InvalidKey));
        assert!(matches!(store.stat(key).await, Err(Error::InvalidKey)));
    }
    assert!(store.list().next().await.is_none());
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
}

#[tokio::test]
async fn create_only_pool_layout_and_foreign_objects() {
    let dir = tempfile::tempdir().unwrap();
    let raw = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    let store = ObjectBlobStore::new(raw.clone(), "tenant/blobs").unwrap();
    let body = Bytes::from_static(b"original");
    store.put(KEY, body.clone()).await.unwrap();
    assert_eq!(
        store.put(KEY, Bytes::from_static(b"replacement")).await,
        Err(Error::AlreadyExists)
    );
    assert_eq!(store.get(KEY).await.unwrap(), body);
    let pooled = format!("$pool${KEY}");
    store
        .put(&pooled, Bytes::from_static(b"pool"))
        .await
        .unwrap();
    assert_eq!(
        raw.get(&Path::from(format!("tenant/blobs/pool/{KEY}")))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        "pool"
    );
    for path in [
        format!("tenant/blobs-other/{KEY}"),
        format!("tenant/blobs/nested/{KEY}"),
        "tenant/blobs/foreign".into(),
        "tenant/blobs/pool/foreign".into(),
        format!("tenant/blobs/$pool${KEY}"),
        "tenant/blobs/pool/b1-00000000000000000000000000000000/nested".into(),
    ] {
        raw.put(&Path::from(path), Bytes::from_static(b"foreign").into())
            .await
            .unwrap();
    }
    let entries: Vec<_> = store.list().try_collect().await.unwrap();
    let keys: std::collections::BTreeSet<_> = entries.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(keys, [KEY, &pooled].into_iter().collect());
    assert_eq!(entries.len(), 2);
    for entry in &entries {
        let path = if entry.key.starts_with("$pool$") {
            format!("tenant/blobs/pool/{KEY}")
        } else {
            format!("tenant/blobs/{KEY}")
        };
        assert_eq!(
            entry.last_modified,
            raw.head(&Path::from(path))
                .await
                .unwrap()
                .last_modified
                .into()
        );
    }
    store.delete(KEY).await.unwrap();
    store.delete(KEY).await.unwrap();
    assert!(!store.exists(KEY).await.unwrap());
    assert!(store.exists(&pooled).await.unwrap());
}

#[tokio::test]
async fn file_url_and_invalid_configuration_are_opaque() {
    let dir = tempfile::tempdir().unwrap();
    let url = url::Url::from_directory_path(dir.path()).unwrap();
    let store =
        ObjectBlobStore::from_url(url.as_str(), std::iter::empty::<(&str, &str)>()).unwrap();
    store
        .put(KEY, Bytes::from_static(b"file URL"))
        .await
        .unwrap();
    assert_eq!(store.get(KEY).await.unwrap(), "file URL");
    for url in [
        "https://secret.example/presigned?token=secret",
        "s3://secret@bucket/prefix",
        "file:///tmp/#secret",
        "invalid secret",
    ] {
        let error = ObjectBlobStore::from_url(url, std::iter::empty::<(&str, &str)>()).unwrap_err();
        assert_eq!(error, Error::Configuration);
        assert!(!format!("{error:?} {error}").contains("secret"));
    }
    let bad = ObjectBlobStore::new(Arc::new(LocalFileSystem::new()), "../secret").unwrap_err();
    assert_eq!(bad, Error::Configuration);
    assert!(!format!("{store:?}").contains(dir.path().to_str().unwrap()));
}

#[tokio::test]
async fn listing_streams_across_many_entries() {
    let dir = tempfile::tempdir().unwrap();
    let store = ObjectBlobStore::new(
        Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap()),
        "blobs",
    )
    .unwrap();
    for n in 0..1100 {
        store
            .put(&format!("b1-{n:032x}"), Bytes::new())
            .await
            .unwrap();
    }
    let mut stream = store.list();
    let mut keys = std::collections::BTreeSet::new();
    while let Some(entry) = stream.next().await {
        assert!(keys.insert(entry.unwrap().key));
    }
    assert_eq!(keys, (0..1100).map(|n| format!("b1-{n:032x}")).collect());
}

#[tokio::test]
async fn backend_errors_remain_opaque_and_head_errors_propagate() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("secret"),
        b"a file cannot be an object directory",
    )
    .unwrap();
    let raw = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    let store = ObjectBlobStore::new(raw, "secret").unwrap();
    let error = store.put(KEY, Bytes::new()).await.unwrap_err();
    assert_eq!(error, Error::Backend);
    assert!(!format!("{error:?} {error}").contains("secret"));
    assert_eq!(store.exists(KEY).await, Err(Error::Backend));
}

#[test]
fn provider_features_control_url_construction() {
    for scheme in ["file", "s3", "az", "gs"] {
        let url = if scheme == "file" {
            "file:///tmp/prefix".to_owned()
        } else {
            format!("{scheme}://bucket/prefix")
        };
        assert_eq!(
            ObjectBlobStore::from_url(&url, [("unknown_secret_option", "secret")],).unwrap_err(),
            Error::Configuration
        );
    }
    #[cfg(feature = "aws")]
    assert!(ObjectBlobStore::from_url(
        "s3://bucket/prefix",
        [
            ("aws_region", "us-east-1"),
            ("aws_access_key_id", "fixture"),
            ("aws_secret_access_key", "fixture"),
        ]
    )
    .is_ok());
    #[cfg(feature = "azure")]
    assert!(ObjectBlobStore::from_url(
        "az://container/prefix",
        [
            ("azure_storage_account_name", "fixture"),
            ("azure_storage_account_key", "Zml4dHVyZQ=="),
        ]
    )
    .is_ok());
    #[cfg(feature = "gcp")]
    assert!(
        ObjectBlobStore::from_url("gs://bucket/prefix", [("google_skip_signature", "true"),])
            .is_ok()
    );
    for (scheme, enabled) in [
        ("s3", cfg!(feature = "aws")),
        ("az", cfg!(feature = "azure")),
        ("gs", cfg!(feature = "gcp")),
    ] {
        if !enabled {
            assert_eq!(
                ObjectBlobStore::from_url(
                    &format!("{scheme}://bucket/prefix"),
                    std::iter::empty::<(&str, &str)>()
                )
                .unwrap_err(),
                Error::Configuration
            );
        }
    }
}
