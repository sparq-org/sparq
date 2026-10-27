//! Reusable behavioural contract for byte backends. Each run needs an empty, isolated store.
//!
//! Use the in-memory backend's create-only constructor when testing the same contract locally.

use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt};

use super::{BlobError, BlobStore};

fn key(n: u128) -> String {
    sparq_lws_blob_key::format(n.to_be_bytes(), false)
}

/// Exercise the shared store contract, including timestamp, pool, concurrency and delete semantics.
pub async fn run<B: BlobStore + ?Sized>(store: &B) {
    let first = key(1);
    let body = Bytes::from_static(b"first body");
    assert!(!store.exists(&first).await.unwrap());
    assert!(matches!(store.get(&first).await, Err(BlobError::NotFound)));
    assert!(store.stat(&first).await.unwrap().is_none());
    store.put(&first, body.clone()).await.unwrap();
    assert_eq!(store.get(&first).await.unwrap(), body);
    assert!(store.exists(&first).await.unwrap());
    let replacement = Bytes::from_static(b"replacement");
    assert!(store.put(&first, replacement).await.is_err());
    assert_eq!(store.get(&first).await.unwrap(), body);
    for invalid in [
        "",
        "b1-0",
        "../b1-00000000000000000000000000000000",
        "b1-0000000000000000000000000000000A",
        "b1-0000000000000000000000000000000g",
        "$pool$../x",
        "$pool$$pool$b1-00000000000000000000000000000000",
        "b1-00000000000000000000000000000000/x",
        "pool/b1-00000000000000000000000000000000",
    ] {
        assert!(
            matches!(
                store.put(invalid, body.clone()).await,
                Err(BlobError::Backend(_))
            ),
            "accepted {invalid:?}"
        );
        assert!(matches!(
            store.get(invalid).await,
            Err(BlobError::Backend(_))
        ));
        assert!(matches!(
            store.exists(invalid).await,
            Err(BlobError::Backend(_))
        ));
        assert!(matches!(
            store.delete(invalid).await,
            Err(BlobError::Backend(_))
        ));
        assert!(matches!(
            store.stat(invalid).await,
            Err(BlobError::Backend(_))
        ));
        assert!(matches!(
            store.delete_if_unchanged(invalid, 0).await,
            Err(BlobError::Backend(_))
        ));
    }
    // Racing creates must have one winner and preserve that winner's complete bytes.
    let raced = key(100);
    let (left, right) = futures_util::join!(
        store.put(&raced, Bytes::from_static(b"left")),
        store.put(&raced, Bytes::from_static(b"right")),
    );
    assert_ne!(left.is_ok(), right.is_ok());
    assert_eq!(
        store.get(&raced).await.unwrap(),
        if left.is_ok() { "left" } else { "right" }
    );
    store.delete(&raced).await.unwrap();
    let pool = sparq_lws_blob_key::format(1u128.to_be_bytes(), true);
    store
        .put(&pool, Bytes::from_static(b"pool body"))
        .await
        .unwrap();
    assert_eq!(store.get(&pool).await.unwrap(), "pool body");
    assert_ne!(store.get(&first).await.unwrap(), "pool body");

    let writes = (2..34).map(|n| async move {
        let key = key(n);
        let body = Bytes::from(key.clone());
        store.put(&key, body.clone()).await.unwrap();
        assert_eq!(store.get(&key).await.unwrap(), body);
        key
    });
    let mut expected: std::collections::BTreeSet<_> = futures_util::future::join_all(writes)
        .await
        .into_iter()
        .collect();
    expected.extend([first.clone(), pool.clone()]);
    let entries: Vec<_> = store
        .list_stream()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(entries.iter().all(|e| e.last_modified.is_some()));
    assert_eq!(entries.len(), expected.len());
    assert_eq!(
        entries
            .iter()
            .map(|e| e.key.clone())
            .collect::<std::collections::BTreeSet<_>>(),
        expected
    );
    let snapshot_keys: std::collections::BTreeSet<_> = store
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.key)
        .collect();
    assert_eq!(snapshot_keys, expected);
    let snapshot = store.stat(&first).await.unwrap().unwrap();
    // HEAD and LIST can expose different timestamp precision for the same write.
    assert!(snapshot.last_modified.is_some());
    if let Some(generation) = snapshot.generation {
        assert!(!store
            .delete_if_unchanged(&first, generation.wrapping_add(1))
            .await
            .unwrap());
        assert!(store.exists(&first).await.unwrap());
        assert!(store.delete_if_unchanged(&first, generation).await.unwrap());
        assert!(!store.delete_if_unchanged(&first, generation).await.unwrap());
    } else {
        // Versionless backends reject conditional deletion; callers use delete directly.
        assert!(matches!(store.delete_if_unchanged(&first, 0).await, Err(BlobError::Backend(_))));
        assert_eq!(store.get(&first).await.unwrap(), body);
        store.delete(&first).await.unwrap();
        assert!(matches!(store.delete_if_unchanged(&first, 0).await, Err(BlobError::Backend(_))));
    }
    assert!(!store.exists(&first).await.unwrap());
    store.delete(&first).await.unwrap();
    store.delete(&first).await.unwrap();
    for key in expected {
        store.delete(&key).await.unwrap();
    }
    assert!(store.list_stream().await.unwrap().next().await.is_none());
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn in_memory_contract() {
        super::run(&crate::store::InMemoryBlobStore::with_immutable_keys()).await;
    }
}
