// [GPT-5.6] sq-5ruwm: boot-only periodic reconciler wiring shared by every native backend.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use sparq_lws_core::store::{
    spawn_periodic, BlobEntry, BlobError, BlobStore, DeleteOutcome, ReadPlan, ReconcileOptions,
    ResourceMeta, SparqClient, SparqError,
};

pub(crate) const ENV_RECONCILE_INTERVAL_SECS: &str = "SOLID_SERVER_RECONCILE_INTERVAL_SECS";

/// A private shared-owner adapter used to give the request path and the one boot-time reconciler task
/// the same backend handles. Delegation preserves backend-specific overrides such as `read_plan` and
/// `stat`; no extra query/list fallback is introduced by sharing a handle.
pub(crate) struct SharedStore<T>(Arc<T>);

impl<T> SharedStore<T> {
    pub(crate) fn new(inner: T) -> Self {
        Self(Arc::new(inner))
    }
}

impl<T> AsRef<T> for SharedStore<T> {
    fn as_ref(&self) -> &T {
        self.0.as_ref()
    }
}

impl<T> Clone for SharedStore<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

#[async_trait]
impl<T: SparqClient> SparqClient for SharedStore<T> {
    async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError> {
        self.0.get_meta(iri).await
    }

    async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
        self.0.put_meta(iri, meta).await
    }

    async fn replace_meta(
        &self,
        iri: &str,
        meta: ResourceMeta,
    ) -> Result<Option<ResourceMeta>, SparqError> {
        self.0.replace_meta(iri, meta).await
    }

    async fn exists(&self, iri: &str) -> Result<bool, SparqError> {
        self.0.exists(iri).await
    }

    async fn delete_meta(&self, iri: &str) -> Result<(), SparqError> {
        self.0.delete_meta(iri).await
    }

    async fn delete_meta_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<DeleteOutcome, SparqError> {
        self.0.delete_meta_if_empty(iri, parent).await
    }

    async fn create_child(
        &self,
        container: &str,
        child: &str,
        meta: ResourceMeta,
    ) -> Result<(), SparqError> {
        self.0.create_child(container, child, meta).await
    }

    async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError> {
        self.0.remove_child(container, child).await
    }

    async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError> {
        self.0.list_children(container).await
    }

    async fn list_children_within(
        &self,
        container: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<String>>, SparqError> {
        self.0.list_children_within(container, max_bytes).await
    }

    async fn referenced_blob_keys(&self) -> Result<HashSet<String>, SparqError> {
        self.0.referenced_blob_keys().await
    }

    async fn read_plan(
        &self,
        target: &str,
        acl_candidates: &[String],
    ) -> Result<ReadPlan, SparqError> {
        self.0.read_plan(target, acl_candidates).await
    }
}

#[async_trait]
impl<T: BlobStore> BlobStore for SharedStore<T> {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        self.0.get(key).await
    }

    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        self.0.put(key, body).await
    }

    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        self.0.exists(key).await
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        self.0.delete(key).await
    }

    async fn list(&self) -> Result<Vec<BlobEntry>, BlobError> {
        self.0.list().await
    }

    async fn stat(&self, key: &str) -> Result<Option<BlobEntry>, BlobError> {
        self.0.stat(key).await
    }

    async fn delete_if_unchanged(
        &self,
        key: &str,
        expected_generation: u64,
    ) -> Result<bool, BlobError> {
        self.0.delete_if_unchanged(key, expected_generation).await
    }
}

/// Parse and validate the opt-in interval before backend construction or dev seeding. Unset means the
/// original direct-store boot; malformed or zero values fail boot instead of silently disabling an
/// operator-requested safety job or panicking inside `tokio::time::interval`.
pub(crate) fn reconcile_interval_from_env() -> Result<Option<Duration>, String> {
    let raw = match std::env::var(ENV_RECONCILE_INTERVAL_SECS) {
        Ok(raw) => raw,
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(format!(
                "{ENV_RECONCILE_INTERVAL_SECS} must be valid Unicode containing a positive integer"
            ));
        }
    };
    let seconds = raw.trim().parse::<u64>().map_err(|_| {
        format!("{ENV_RECONCILE_INTERVAL_SECS} must be a positive integer number of seconds")
    })?;
    if seconds == 0 {
        return Err(format!(
            "{ENV_RECONCILE_INTERVAL_SECS} must be greater than zero when set"
        ));
    }
    Ok(Some(Duration::from_secs(seconds)))
}

/// Start exactly one periodic sweep for a validated interval. `None` returns no handle and performs
/// no spawn. The sweep always uses `ReconcileOptions::default()`, so the existing one-hour grace
/// period and fail-closed age handling remain authoritative.
pub(crate) fn spawn_periodic_if_configured<S, B>(
    sparq: S,
    blob: B,
    interval: Option<Duration>,
) -> Option<tokio::task::JoinHandle<()>>
where
    S: SparqClient + 'static,
    B: BlobStore + 'static,
{
    let interval = interval?;
    let options = ReconcileOptions::default();
    eprintln!(
        "  RECONCILER: periodic orphan sweep ENABLED (interval={}s, grace={}s; one boot task).",
        interval.as_secs(),
        options.grace.as_secs()
    );
    Some(spawn_periodic(sparq, blob, interval, options))
}
