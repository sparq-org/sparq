//! Native filesystem adapter for [`BlobStore`]. Filesystem work runs on the blocking pool.
//!
//! Hold the exclusive data-directory lock before opening the store and throughout
//! its lifetime. Keys are immutable and unique per write; there is no generation
//! witness, so reconciliation uses unconditional deletion.

use std::io;

use async_trait::async_trait;
use bytes::Bytes;

use super::{BlobEntry, BlobError, BlobStore};

/// The standalone synchronous store, with this module's async [`BlobStore`] implementation.
pub use sparq_lws_blob_fs::FsBlobStore;

fn map_error(error: io::Error) -> BlobError {
    if error.kind() == io::ErrorKind::NotFound {
        BlobError::NotFound
    } else {
        BlobError::Backend(error.to_string())
    }
}

async fn blocking<T: Send + 'static>(
    store: &FsBlobStore,
    operation: impl FnOnce(FsBlobStore) -> io::Result<T> + Send + 'static,
) -> Result<T, BlobError> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || operation(store))
        .await
        .map_err(|error| BlobError::Backend(error.to_string()))?
        .map_err(map_error)
}

fn entry(blob: sparq_lws_blob_fs::BlobEntry) -> BlobEntry {
    BlobEntry {
        key: blob.key,
        last_modified: blob.last_modified,
        generation: blob.generation,
    }
}

#[async_trait]
impl BlobStore for FsBlobStore {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        let key = key.to_string();
        blocking(self, move |store| store.get(&key).map(Bytes::from)).await
    }

    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        let key = key.to_string();
        blocking(self, move |store| store.put(&key, &body)).await
    }

    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        let key = key.to_string();
        blocking(self, move |store| store.exists(&key)).await
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        let key = key.to_string();
        blocking(self, move |store| store.delete(&key)).await
    }

    async fn list(&self) -> Result<Vec<BlobEntry>, BlobError> {
        blocking(self, move |store| {
            let listing = store.list()?;
            if !listing.foreign.is_empty() {
                eprintln!(
                    "  WARNING: filesystem blob listing skipped foreign entries: {:?}",
                    listing.foreign
                );
            }
            Ok(listing.blobs.into_iter().map(entry).collect())
        })
        .await
    }

    async fn stat(&self, key: &str) -> Result<Option<BlobEntry>, BlobError> {
        let key = key.to_string();
        blocking(self, move |store| {
            store.stat(&key).map(|blob| blob.map(entry))
        })
        .await
    }

    async fn delete_if_unchanged(&self, _key: &str, _generation: u64) -> Result<bool, BlobError> {
        // Never pretend to compare a version the filesystem does not supply.
        Err(BlobError::Backend(
            "filesystem blobs have no generation witness".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::store::{
        reconcile_orphans, BodyCache, CompositeStore, InMemorySparqClient, ReconcileOptions, Store,
        POOLED_BLOB_KEY_MARK,
    };

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "lws-blob-adapter-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("test directory: {e}"),
                }
            }
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn pooled_key_marker_matches_store_limits() {
        assert_eq!(
            sparq_lws_blob_fs::POOLED_BLOB_KEY_MARK,
            crate::store::limits::POOLED_BLOB_KEY_MARK
        );
    }

    #[tokio::test]
    async fn composite_round_trip_overwrite_hold_reclaim_and_delete() {
        let directory = Directory::new();
        let store = CompositeStore::with_body_cache(
            InMemorySparqClient::new(),
            FsBlobStore::open(&directory.0).unwrap(),
            BodyCache::disabled(),
        );
        let iri = "https://pod.example/private-document";
        let first = store
            .write(iri, Bytes::from_static(b"first"), "text/plain")
            .await
            .unwrap();
        assert_eq!(store.read(iri).await.unwrap().body, b"first"[..]);
        assert!(BlobStore::exists(&store.blob, &first.blob_key)
            .await
            .unwrap());
        let held = store.hold(iri, "urn:test:holder").await.unwrap().unwrap();
        let second = store
            .write(iri, Bytes::from_static(b"second"), "text/plain")
            .await
            .unwrap();
        assert_ne!(first.blob_key, second.blob_key);
        assert_eq!(store.read(iri).await.unwrap().body, b"second"[..]);
        assert_eq!(
            BlobStore::get(&store.blob, &first.blob_key).await.unwrap(),
            b"first"[..]
        );
        store.release(&held, "urn:test:holder").await.unwrap();
        // Releasing the last hold drives CompositeStore::reclaim_blob through the adapter.
        assert!(!BlobStore::exists(&store.blob, &first.blob_key)
            .await
            .unwrap());
        let third = store
            .write(iri, Bytes::from_static(b"third"), "text/plain")
            .await
            .unwrap();
        assert!(!BlobStore::exists(&store.blob, &second.blob_key)
            .await
            .unwrap());
        store.delete(iri, None).await.unwrap();
        assert!(!store.exists(iri).await.unwrap());
        assert!(!BlobStore::exists(&store.blob, &third.blob_key)
            .await
            .unwrap());
        store
            .delete(crate::store::HELD_CONTAINER, None)
            .await
            .unwrap();
        assert!(BlobStore::list(&store.blob).await.unwrap().is_empty());

        let pooled = store
            .write(
                "https://pod.example/.lws/subscriptions/topic",
                Bytes::from_static(b"pool"),
                "text/plain",
            )
            .await
            .unwrap();
        assert_eq!(
            POOLED_BLOB_KEY_MARK,
            sparq_lws_blob_fs::POOLED_BLOB_KEY_MARK
        );
        assert!(pooled.blob_key.starts_with("$pool$b1-"));
        let random = &pooled.blob_key[9..];
        assert!(directory
            .0
            .join("pool")
            .join(&random[..2])
            .join(&pooled.blob_key)
            .is_file());
        let blobs = BlobStore::list(&store.blob).await.unwrap();
        assert_eq!(blobs.len(), 1);
        assert_eq!(blobs[0].key, pooled.blob_key);
        assert!(blobs[0].last_modified.is_some());
        assert_eq!(blobs[0].generation, None);
        // The same index and a new adapter must read bytes from disk, without the body cache.
        let CompositeStore { sparq, blob, .. } = store;
        drop(blob);
        let reopened = CompositeStore::with_body_cache(
            sparq,
            FsBlobStore::open(&directory.0).unwrap(),
            BodyCache::disabled(),
        );
        assert_eq!(
            reopened
                .read("https://pod.example/.lws/subscriptions/topic")
                .await
                .unwrap()
                .body,
            b"pool"[..]
        );
    }

    #[tokio::test]
    async fn reconciler_lists_and_deletes_versionless_filesystem_orphans() {
        let directory = Directory::new();
        let store = CompositeStore::with_body_cache(
            InMemorySparqClient::new(),
            FsBlobStore::open(&directory.0).unwrap(),
            BodyCache::disabled(),
        );
        let live = store
            .write(
                "https://pod.example/live",
                Bytes::from_static(b"live"),
                "text/plain",
            )
            .await
            .unwrap();
        let orphan = "b1-0123456789abcdef0123456789abcdef";
        BlobStore::put(&store.blob, orphan, Bytes::from_static(b"orphan"))
            .await
            .unwrap();
        // Fix only the orphan's age, avoiding sleeps or reliance on clock precision.
        let file = fs::OpenOptions::new()
            .write(true)
            .open(directory.0.join("objects/01/23").join(orphan))
            .unwrap();
        file.set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        let stats = BlobStore::stat(&store.blob, orphan).await.unwrap().unwrap();
        assert_eq!(stats.last_modified, Some(SystemTime::UNIX_EPOCH));
        assert_eq!(stats.generation, None);
        fs::write(directory.0.join("tmp/unpublished"), b"in flight").unwrap();
        let stray_file = directory.0.join("objects/01/23/.DS_Store");
        fs::write(&stray_file, b"foreign").unwrap();
        let stray_dir = directory.0.join("objects/01/23/stray-directory");
        fs::create_dir(&stray_dir).unwrap();
        let options = ReconcileOptions {
            grace: Duration::from_secs(60),
            dry_run: false,
        };
        let report = reconcile_orphans(&store.sparq, &store.blob, &options)
            .await
            .unwrap();
        assert_eq!(report.scanned, 2);
        assert_eq!(report.referenced, 1);
        assert_eq!(report.orphaned, 1);
        assert_eq!(report.deleted, 1);
        assert_eq!(report.delete_errors, 0);
        assert!(!BlobStore::exists(&store.blob, orphan).await.unwrap());
        assert_eq!(
            BlobStore::get(&store.blob, &live.blob_key).await.unwrap(),
            b"live"[..]
        );
        assert!(directory.0.join("tmp/unpublished").is_file());
        assert_eq!(fs::read(stray_file).unwrap(), b"foreign");
        assert!(stray_dir.is_dir());
        assert_eq!(
            reconcile_orphans(&store.sparq, &store.blob, &options)
                .await
                .unwrap()
                .deleted,
            0
        );
    }

    #[tokio::test]
    async fn adapter_preserves_errors_and_refuses_a_fabricated_generation() {
        let directory = Directory::new();
        let blob = FsBlobStore::open(&directory.0).unwrap();
        let key = "b1-0123456789abcdef0123456789abcdef";
        assert!(matches!(
            BlobStore::get(&blob, key).await,
            Err(BlobError::NotFound)
        ));
        for invalid in ["../escape", "", "/absolute"] {
            assert!(matches!(
                BlobStore::put(&blob, invalid, Bytes::new()).await,
                Err(BlobError::Backend(_))
            ));
            assert!(BlobStore::get(&blob, invalid).await.is_err());
            assert!(BlobStore::exists(&blob, invalid).await.is_err());
            assert!(BlobStore::delete(&blob, invalid).await.is_err());
            assert!(BlobStore::stat(&blob, invalid).await.is_err());
        }
        BlobStore::put(&blob, key, Bytes::from_static(b"original"))
            .await
            .unwrap();
        assert!(
            BlobStore::put(&blob, key, Bytes::from_static(b"replacement"))
                .await
                .is_err()
        );
        assert!(BlobStore::delete_if_unchanged(&blob, key, 0).await.is_err());
        assert_eq!(BlobStore::get(&blob, key).await.unwrap(), b"original"[..]);
        BlobStore::delete(&blob, key).await.unwrap();
        BlobStore::delete(&blob, key).await.unwrap();
        assert!(BlobStore::stat(&blob, key).await.unwrap().is_none());
    }
}
