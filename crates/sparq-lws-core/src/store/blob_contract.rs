//! Reusable behavioural contract for byte backends. Each run needs an empty, isolated store.
//!
//! The legacy in-memory double permits arbitrary keys and replacement writes for race tests.
//! Persistent backends use the stricter immutable-key profile.

use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt};

use super::{BlobError, BlobStore};

/// Existing replacement test double versus durable create-only backends.
#[derive(Clone, Copy)]
pub enum Profile {
    /// Historical in-memory semantics, used by overwrite/generation race tests.
    InMemory,
    /// Validated opaque keys and atomic create-only writes.
    ImmutableKeys,
}

fn key(n: u128) -> String {
    format!("b1-{n:032x}")
}

/// Exercise the shared store contract, including timestamp, pool, concurrency and delete semantics.
pub async fn run<B: BlobStore + ?Sized>(store: &B, profile: Profile) {
    let first = key(1);
    let body = Bytes::from_static(b"first body");
    assert!(!store.exists(&first).await.unwrap());
    assert!(matches!(store.get(&first).await, Err(BlobError::NotFound)));
    assert!(store.stat(&first).await.unwrap().is_none());
    store.put(&first, body.clone()).await.unwrap();
    assert_eq!(store.get(&first).await.unwrap(), body);
    assert!(store.exists(&first).await.unwrap());
    let replacement = Bytes::from_static(b"replacement");
    match profile {
        Profile::ImmutableKeys => {
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
                    store.put(invalid, body.clone()).await.is_err(),
                    "accepted {invalid:?}"
                );
                assert!(store.get(invalid).await.is_err());
                assert!(store.exists(invalid).await.is_err());
                assert!(store.delete(invalid).await.is_err());
                assert!(store.stat(invalid).await.is_err());
                assert!(store.delete_if_unchanged(invalid, 0).await.is_err());
            }
        }
        Profile::InMemory => {
            store.put(&first, replacement.clone()).await.unwrap();
            assert_eq!(store.get(&first).await.unwrap(), replacement);
        }
    }
    let pool = format!("$pool${first}");
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
    let snapshot = store.stat(&first).await.unwrap().unwrap();
    if let Some(generation) = snapshot.generation {
        assert!(!store
            .delete_if_unchanged(&first, generation.wrapping_add(1))
            .await
            .unwrap());
        assert!(store.exists(&first).await.unwrap());
        assert!(store.delete_if_unchanged(&first, generation).await.unwrap());
        assert!(!store.delete_if_unchanged(&first, generation).await.unwrap());
    } else {
        // Versionless backends use the trait's unique-key unconditional-delete fallback.
        assert!(store.delete_if_unchanged(&first, 0).await.unwrap());
        assert!(!store.delete_if_unchanged(&first, 0).await.unwrap());
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
        super::run(
            &crate::store::InMemoryBlobStore::new(),
            super::Profile::InMemory,
        )
        .await;
    }
}
