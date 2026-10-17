// sq-5ruwm: mutation-witnessed boot wiring for the opt-in periodic reconciler.

#[path = "../src/reconcile_runtime.rs"]
mod reconcile_runtime;

use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use bytes::Bytes;
use reconcile_runtime::{
    reconcile_interval_from_env, spawn_periodic_if_configured, SharedStore,
    ENV_RECONCILE_INTERVAL_SECS,
};
use sparq_lws_core::store::{
    BlobEntry, BlobError, BlobStore, InMemorySparqClient, ResourceMeta, SparqClient, DEFAULT_GRACE,
};

#[derive(Clone)]
struct TestBlob {
    body: Bytes,
    last_modified: Option<SystemTime>,
    generation: u64,
}

#[derive(Default)]
struct TestBlobState {
    entries: HashMap<String, TestBlob>,
    next_generation: u64,
}

/// A deterministic blob seam that can represent the backend's fail-closed `last_modified = None`
/// case. Its compare-and-delete holds one mutex across the comparison and removal, matching the
/// atomicity contract the production reconciler requires.
#[derive(Default)]
struct TestBlobStore {
    state: Mutex<TestBlobState>,
}

impl TestBlobStore {
    fn insert(&self, key: &str, last_modified: Option<SystemTime>) {
        let mut state = self.state.lock().expect("test blob mutex poisoned");
        state.next_generation += 1;
        let generation = state.next_generation;
        state.entries.insert(
            key.to_string(),
            TestBlob {
                body: Bytes::from_static(b"test"),
                last_modified,
                generation,
            },
        );
    }
}

#[async_trait]
impl BlobStore for TestBlobStore {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        self.state
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?
            .entries
            .get(key)
            .map(|entry| entry.body.clone())
            .ok_or(BlobError::NotFound)
    }

    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?;
        state.next_generation += 1;
        let generation = state.next_generation;
        state.entries.insert(
            key.to_string(),
            TestBlob {
                body,
                last_modified: Some(SystemTime::now()),
                generation,
            },
        );
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?
            .entries
            .contains_key(key))
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        self.state
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?
            .entries
            .remove(key);
        Ok(())
    }

    async fn list(&self) -> Result<Vec<BlobEntry>, BlobError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?
            .entries
            .iter()
            .map(|(key, entry)| BlobEntry {
                key: key.clone(),
                last_modified: entry.last_modified,
                generation: Some(entry.generation),
            })
            .collect())
    }

    async fn stat(&self, key: &str) -> Result<Option<BlobEntry>, BlobError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?
            .entries
            .get(key)
            .map(|entry| BlobEntry {
                key: key.to_string(),
                last_modified: entry.last_modified,
                generation: Some(entry.generation),
            }))
    }

    async fn delete_if_unchanged(
        &self,
        key: &str,
        expected_generation: u64,
    ) -> Result<bool, BlobError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?;
        let matches = state
            .entries
            .get(key)
            .is_some_and(|entry| entry.generation == expected_generation);
        if matches {
            state.entries.remove(key);
        }
        Ok(matches)
    }
}

struct EnvRestore {
    previous: Option<OsString>,
}

impl EnvRestore {
    fn capture() -> Self {
        Self {
            previous: std::env::var_os(ENV_RECONCILE_INTERVAL_SECS),
        }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(ENV_RECONCILE_INTERVAL_SECS, value),
            None => std::env::remove_var(ENV_RECONCILE_INTERVAL_SECS),
        }
    }
}

async fn wait_until_deleted(blob: &SharedStore<TestBlobStore>, key: &str) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while blob.exists(key).await.expect("probe test blob") {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("periodic reconciler did not reclaim the old orphan");
}

#[tokio::test]
async fn runtime_flag_controls_periodic_sweep_without_weakening_delete_safety() {
    let _env_restore = EnvRestore::capture();
    let old = SystemTime::now() - DEFAULT_GRACE - Duration::from_secs(60);

    for invalid in ["", "0", "not-a-number"] {
        std::env::set_var(ENV_RECONCILE_INTERVAL_SECS, invalid);
        assert!(
            reconcile_interval_from_env().is_err(),
            "invalid configured interval {invalid:?} must fail boot"
        );
    }

    // Flag ON: one delayed tick must invoke the existing pure sweep with its DEFAULT (one-hour)
    // grace. The old orphan is the only deletable entry; referenced, young, and undated entries are
    // mutation witnesses for all three fail-closed retention paths.
    std::env::set_var(ENV_RECONCILE_INTERVAL_SECS, "1");
    let interval = reconcile_interval_from_env()
        .expect("valid periodic reconciler configuration")
        .expect("a present flag must select the shared boot path");
    assert_eq!(interval, Duration::from_secs(1));
    let sparq = SharedStore::new(InMemorySparqClient::new());
    let blob = SharedStore::new(TestBlobStore::default());
    blob.as_ref().insert("old-orphan", Some(old));
    blob.as_ref()
        .insert("young-write-in-progress", Some(SystemTime::now()));
    blob.as_ref().insert("undated-orphan", None);
    blob.as_ref().insert("referenced-live", Some(old));
    sparq
        .put_meta(
            "https://pod.example/live",
            ResourceMeta {
                content_type: "text/turtle".into(),
                blob_key: "referenced-live".into(),
                appended: Vec::new(),
                etag: "\"live\"".into(),
                last_modified: Some(SystemTime::now()),
            },
        )
        .await
        .expect("index live blob");

    let task = spawn_periodic_if_configured(sparq.clone(), blob.clone(), Some(interval))
        .expect("set interval must spawn exactly one task");
    wait_until_deleted(&blob, "old-orphan").await;
    assert!(
        blob.exists("young-write-in-progress").await.unwrap(),
        "the unchanged grace period must retain a too-young orphan"
    );
    assert!(
        blob.exists("undated-orphan").await.unwrap(),
        "an undated blob must remain fail-closed"
    );
    assert!(
        blob.exists("referenced-live").await.unwrap(),
        "the authoritative referenced-key snapshot must retain live bytes"
    );
    task.abort();
    let _ = task.await;

    // Flag OFF: the helper returns no handle and even an old orphan remains untouched. Removing the
    // opt-in check (or spawning unconditionally) makes this assertion fail.
    std::env::remove_var(ENV_RECONCILE_INTERVAL_SECS);
    let interval = reconcile_interval_from_env().expect("unset interval is valid");
    assert_eq!(interval, None, "an absent flag must select the direct path");
    let off_sparq = SharedStore::new(InMemorySparqClient::new());
    let off_blob = SharedStore::new(TestBlobStore::default());
    off_blob.as_ref().insert("off-old-orphan", Some(old));
    assert!(
        spawn_periodic_if_configured(off_sparq, off_blob.clone(), interval).is_none(),
        "flag OFF must not spawn a reconciler task"
    );
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    assert!(
        off_blob.exists("off-old-orphan").await.unwrap(),
        "with no task, an old orphan must remain untouched beyond an enabled tick interval"
    );
}
