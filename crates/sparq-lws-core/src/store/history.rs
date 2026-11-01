//! Typed, index-only version and trash records.

use super::{ResourceMeta, SparqError};
use std::time::{Duration, SystemTime};

pub const VERSIONS: &str = "urn:x-sparq-lws:versions/";
pub const TRASH: &str = "urn:x-sparq-lws:trash/";

/// Retention settings shared by the store and the protocol adapters. Disabled by default.
#[derive(Clone, Debug)]
pub struct HistoryConfig {
    pub versions: bool,
    pub trash: bool,
    pub versions_max: usize,
    pub versions_retention: Duration,
    pub trash_retention: Duration,
}
impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            versions: false,
            trash: false,
            versions_max: 20,
            versions_retention: Duration::from_secs(30 * 86400),
            trash_retention: Duration::from_secs(30 * 86400),
        }
    }
}
impl HistoryConfig {
    /// Read the opt-in settings. Retention accepts positive day/time XSD durations.
    pub fn from_env() -> Result<Self, String> {
        let flag = |k| std::env::var(k).is_ok_and(|v| v == "1");
        let mut c = Self {
            versions: flag("SOLID_SERVER_VERSIONS"),
            trash: flag("SOLID_SERVER_TRASH"),
            ..Self::default()
        };
        if let Some(v) = c
            .versions
            .then(|| std::env::var("SOLID_SERVER_VERSIONS_MAX").ok())
            .flatten()
        {
            c.versions_max = v
                .parse()
                .ok()
                .filter(|n| *n > 0)
                .ok_or("SOLID_SERVER_VERSIONS_MAX must be positive")?;
        }
        for (key, slot, enabled) in [
            (
                "SOLID_SERVER_VERSIONS_RETENTION",
                &mut c.versions_retention,
                c.versions,
            ),
            (
                "SOLID_SERVER_TRASH_RETENTION",
                &mut c.trash_retention,
                c.trash,
            ),
        ] {
            if !enabled {
                continue;
            }
            if let Ok(v) = std::env::var(key) {
                *slot = parse_duration(&v)
                    .ok_or_else(|| format!("{key} must be a positive day/time XSD duration"))?;
            }
        }
        Ok(c)
    }
    pub fn enabled(&self) -> bool {
        self.versions || self.trash
    }
    pub fn validate_backend(&self, backend: &str) -> Result<(), String> {
        if self.enabled() && backend == "http" {
            return Err("versions and trash require the memory or embedded index backend".into());
        }
        Ok(())
    }
}

fn parse_duration(s: &str) -> Option<Duration> {
    let mut rest = s.strip_prefix('P')?;
    let mut total = 0u64;
    let mut time = false;
    let mut last = 0;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('T') {
            if time {
                return None;
            }
            time = true;
            rest = r;
            if rest.is_empty() {
                return None;
            }
            continue;
        }
        let end = rest.find(|c: char| !c.is_ascii_digit())?;
        let n: u64 = rest[..end].parse().ok()?;
        let unit = rest.as_bytes()[end];
        let (rank, factor) = match (time, unit) {
            (false, b'D') => (1, 86400),
            (true, b'H') => (2, 3600),
            (true, b'M') => (3, 60),
            (true, b'S') => (4, 1),
            _ => return None,
        };
        if rank <= last {
            return None;
        }
        last = rank;
        total = total.checked_add(n.checked_mul(factor)?)?;
        rest = &rest[end + 1..];
    }
    (total > 0).then(|| Duration::from_secs(total))
}

/// Fields committed alongside the superseded metadata.
#[derive(Clone, Debug)]
pub struct VersionFields {
    pub original: String,
    pub max_versions: usize,
}
/// A retained immutable state. `order` resolves equal modification timestamps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionEntry {
    pub member: String,
    pub original: String,
    pub meta: ResourceMeta,
    pub order: u64,
}
/// Fields captured by the DELETE authorization decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashFields {
    pub original: String,
    pub trashed_at: SystemTime,
    pub deleted_by: Option<String>,
    pub deleter_could_read: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashEntry {
    pub member: String,
    pub meta: ResourceMeta,
    pub fields: TrashFields,
}

/// Keys removed by one permanent-delete transaction, including its retained history.
#[derive(Debug)]
pub struct RetiredResource {
    pub live_key: Option<String>,
    pub version_keys: Vec<String>,
}

/// An expiry candidate, selected by the index in timestamp order.
#[derive(Clone, Debug)]
pub struct ExpiredEntry {
    pub member: String,
    pub blob_key: String,
    pub at: SystemTime,
    pub trash: bool,
}

pub(crate) fn data_resource(iri: &str) -> bool {
    (iri.starts_with("http://") || iri.starts_with("https://"))
        && !iri.ends_with('/')
        && !crate::authz::mode::is_acl_resource(iri)
        && !iri.rsplit('/').next().is_some_and(|s| s.ends_with(".acr"))
        && !iri.ends_with(".meta")
        && !iri.split('/').any(|s| {
            matches!(
                s,
                ".lws" | ".versions" | ".trash" | ".notifications" | ".well-known" | ".identity"
            )
        })
}
pub(crate) fn version_dir(iri: &str) -> String {
    format!("{VERSIONS}{}/", super::sha_hex(iri))
}
pub(crate) fn rekey_member(member: &str, target: &str) -> String {
    format!(
        "{}{}",
        version_dir(target),
        member.rsplit('/').next().unwrap_or(member)
    )
}
pub(crate) fn mint(prefix: &str) -> Result<String, SparqError> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|e| SparqError::Backend(e.to_string()))?;
    Ok(format!(
        "{prefix}{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}
pub(crate) fn sort_versions(v: &mut [VersionEntry]) {
    v.sort_by_key(|v| (v.meta.last_modified, v.order));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{
        reconcile_orphans, BlobStore, CompositeStore, InMemoryBlobStore, InMemorySparqClient,
        InMemoryStoreLimits, ReconcileOptions, SparqClient, Store,
    };
    use bytes::Bytes;
    const ROOT: &str = "https://pod.example/";
    const IRI: &str = "https://pod.example/note";
    fn config() -> HistoryConfig {
        HistoryConfig {
            versions: true,
            trash: true,
            ..Default::default()
        }
    }
    fn fields() -> TrashFields {
        TrashFields {
            original: IRI.into(),
            trashed_at: crate::clock::now(),
            deleted_by: Some("https://alice.example/#me".into()),
            deleter_could_read: true,
        }
    }
    async fn race<S: SparqClient>(index: S) {
        let store = CompositeStore::new(index, InMemoryBlobStore::new())
            .with_history(config())
            .unwrap();
        for n in 0..24 {
            store
                .write(IRI, Bytes::from_static(b"initial"), "text/plain")
                .await
                .unwrap();
            let m = store.meta(IRI).await.unwrap().unwrap();
            store
                .blob
                .put_at(
                    &m.blob_key,
                    Bytes::from_static(b"initial"),
                    SystemTime::UNIX_EPOCH,
                )
                .unwrap();
            store
                .blob
                .put_at(
                    "orphan",
                    Bytes::from_static(b"orphan"),
                    SystemTime::UNIX_EPOCH,
                )
                .unwrap();
            let a = format!("left-{n}");
            let b = format!("right-{n}");
            let options = ReconcileOptions::default();
            let (left, right, sweep) = tokio::join!(
                store.write(IRI, Bytes::from(a.clone()), "text/plain"),
                store.write(IRI, Bytes::from(b.clone()), "text/plain"),
                reconcile_orphans(&store.sparq, &store.blob, &options)
            );
            left.unwrap();
            right.unwrap();
            sweep.unwrap();
            let versions = store.versions(Some(IRI)).await.unwrap();
            assert_eq!(versions.len(), 2);
            for v in versions {
                let bytes = store
                    .blob
                    .get(&v.meta.blob_key)
                    .await
                    .expect("every retained record still names readable bytes");
                assert!(
                    [b"initial".as_slice(), a.as_bytes(), b.as_bytes()].contains(&bytes.as_ref())
                );
            }
            store.delete(IRI, Some(ROOT)).await.unwrap();
            assert!(store.versions(Some(IRI)).await.unwrap().is_empty());
        }
    }
    #[tokio::test]
    async fn racing_overwrites_and_reconciler_preserve_every_version_memory() {
        race(InMemorySparqClient::new()).await;
    }
    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn racing_overwrites_and_reconciler_preserve_every_version_embedded() {
        race(crate::store::EmbeddedSparqClient::in_memory().unwrap()).await;
    }

    #[tokio::test]
    async fn ceiling_exchanges_slots_for_trash_and_overwrites() {
        let index = InMemorySparqClient::with_limits(InMemoryStoreLimits::new(1024, 3));
        let store = CompositeStore::new(index, InMemoryBlobStore::new())
            .with_history(config())
            .unwrap();
        store
            .write(ROOT, Bytes::new(), "text/turtle")
            .await
            .unwrap();
        store
            .create_in_container(ROOT, IRI, Bytes::from_static(b"one"), "text/plain")
            .await
            .unwrap();
        store
            .write(IRI, Bytes::from_static(b"two"), "text/plain")
            .await
            .unwrap();
        assert_eq!(store.sparq.usage().unwrap(), 3);
        store
            .write(IRI, Bytes::from_static(b"three"), "text/plain")
            .await
            .unwrap();
        let v = store.versions(Some(IRI)).await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(
            store.read_at(&v[0].member, &v[0].meta).await.unwrap(),
            "two"
        );
        assert_eq!(store.sparq.usage().unwrap(), 3);
        store
            .trash_resource(IRI, Some(ROOT), fields())
            .await
            .unwrap();
        assert_eq!(store.sparq.usage().unwrap(), 3);
        assert!(!store.exists(IRI).await.unwrap());
        assert!(store.list_children(ROOT).await.unwrap().is_empty());
        let t = store.trash_entries().await.unwrap().remove(0);
        store.restore_trash(&t.member, IRI, ROOT).await.unwrap();
        assert_eq!(store.sparq.usage().unwrap(), 3);
        assert_eq!(store.read(IRI).await.unwrap().body, "three");
    }
    async fn retention<S: SparqClient>(index: S) {
        let mut c = config();
        c.versions_retention = Duration::from_secs(10);
        c.trash_retention = Duration::from_secs(20);
        let store = CompositeStore::new(index, InMemoryBlobStore::new())
            .with_history(c)
            .unwrap();
        store
            .write(ROOT, Bytes::new(), "text/turtle")
            .await
            .unwrap();
        store
            .create_in_container(ROOT, IRI, Bytes::from_static(b"one"), "text/plain")
            .await
            .unwrap();
        store
            .write(IRI, Bytes::from_static(b"two"), "text/plain")
            .await
            .unwrap();
        let v = store.versions(Some(IRI)).await.unwrap().remove(0);
        let at = v.meta.last_modified.unwrap();
        let mut f = fields();
        f.trashed_at = at;
        store.trash_resource(IRI, Some(ROOT), f).await.unwrap();
        assert_eq!(
            store
                .sweep_history(at + Duration::from_secs(9), 64)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .sweep_history(at + Duration::from_secs(10), 1)
                .await
                .unwrap(),
            1
        );
        assert!(store.versions(Some(IRI)).await.unwrap().is_empty());
        assert_eq!(store.trash_entries().await.unwrap().len(), 1);
        assert!(!store.blob.exists(&v.meta.blob_key).await.unwrap());
        assert_eq!(
            store
                .sweep_history(at + Duration::from_secs(19), 64)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .sweep_history(at + Duration::from_secs(20), 1)
                .await
                .unwrap(),
            1
        );
        assert!(store.trash_entries().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn retention_boundaries_memory() {
        retention(InMemorySparqClient::new()).await;
    }
    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn retention_boundaries_embedded() {
        retention(crate::store::EmbeddedSparqClient::in_memory().unwrap()).await;
    }

    #[tokio::test]
    async fn permanent_delete_without_trash_clears_history() {
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new())
            .with_history(HistoryConfig {
                versions: true,
                ..Default::default()
            })
            .unwrap();
        for text in ["one", "two"] {
            store
                .write(IRI, Bytes::copy_from_slice(text.as_bytes()), "text/plain")
                .await
                .unwrap();
        }
        assert_eq!(store.versions(Some(IRI)).await.unwrap().len(), 1);
        store.delete(IRI, None).await.unwrap();
        store
            .write(IRI, Bytes::from_static(b"fresh"), "text/plain")
            .await
            .unwrap();
        assert!(store.versions(Some(IRI)).await.unwrap().is_empty());
    }
    async fn trash_removal<S: SparqClient>(index: S) {
        let mut policy = config();
        policy.trash_retention = Duration::from_secs(10);
        let store = CompositeStore::new(index, InMemoryBlobStore::new())
            .with_history(policy)
            .unwrap();
        for expire in [false, true] {
            store
                .write(IRI, Bytes::from_static(b"old one"), "text/plain")
                .await
                .unwrap();
            store
                .write(IRI, Bytes::from_static(b"old two"), "text/plain")
                .await
                .unwrap();
            let old = store.versions(Some(IRI)).await.unwrap().remove(0);
            let mut f = fields();
            let trashed_at = crate::clock::now();
            f.trashed_at = trashed_at;
            store.trash_resource(IRI, None, f).await.unwrap();
            let trash = store.trash_entries().await.unwrap().remove(0);
            assert!(store.version_entry(&old.member).await.unwrap().is_none());
            let held = store.versions(Some(&trash.member)).await.unwrap();
            assert_eq!(held.len(), 1);
            assert!(held[0].member.starts_with(&version_dir(&trash.member)));
            assert_eq!(held[0].meta, old.meta);
            store
                .write(IRI, Bytes::from_static(b"new one"), "text/plain")
                .await
                .unwrap();
            store
                .write(IRI, Bytes::from_static(b"new two"), "text/plain")
                .await
                .unwrap();
            let new = store.versions(Some(IRI)).await.unwrap();
            assert_eq!(new.len(), 1);
            if expire {
                assert_eq!(
                    store
                        .sweep_history(trashed_at + Duration::from_secs(10), 1)
                        .await
                        .unwrap(),
                    1
                );
            } else {
                store.purge_trash(&trash.member).await.unwrap();
            }
            assert_eq!(store.versions(Some(IRI)).await.unwrap(), new);
            assert!(store
                .versions(Some(&trash.member))
                .await
                .unwrap()
                .is_empty());
            assert!(!store.blob.exists(&old.meta.blob_key).await.unwrap());
            assert!(!store.blob.exists(&trash.meta.blob_key).await.unwrap());
            assert!(store.blob.exists(&new[0].meta.blob_key).await.unwrap());
            assert_eq!(store.read(IRI).await.unwrap().body, "new two");
            store.delete(IRI, None).await.unwrap();
        }
    }
    #[tokio::test]
    async fn trash_purge_and_expiry_preserve_live_history_memory() {
        trash_removal(InMemorySparqClient::new()).await;
    }
    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn trash_purge_and_expiry_preserve_live_history_embedded() {
        trash_removal(crate::store::EmbeddedSparqClient::in_memory().unwrap()).await;
    }

    async fn selection_by_container<S: SparqClient>(index: S) {
        let store = CompositeStore::new(index, InMemoryBlobStore::new())
            .with_history(config())
            .unwrap();
        store
            .write(IRI, Bytes::from_static(b"first"), "text/plain")
            .await
            .unwrap();
        let first = store.meta(IRI).await.unwrap().unwrap();
        let member = mint(&version_dir(IRI)).unwrap();
        let mut next = first.clone();
        next.etag = "next".into();
        store
            .sparq
            .replace_meta_retaining(
                IRI,
                next,
                &member,
                VersionFields {
                    original: ROOT.into(),
                    max_versions: 20,
                },
            )
            .await
            .unwrap();
        let entries = store.versions(Some(IRI)).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].member, member);
        assert_eq!(entries[0].meta, first);
        assert!(store.versions(Some(ROOT)).await.unwrap().is_empty());
        store.trash_resource(IRI, None, fields()).await.unwrap();
        let trash = store.trash_entries().await.unwrap().remove(0);
        assert!(store.versions(Some(IRI)).await.unwrap().is_empty());
        assert_eq!(store.versions(Some(&trash.member)).await.unwrap().len(), 1);
        store.purge_trash(&trash.member).await.unwrap();
        assert!(store.versions(None).await.unwrap().is_empty());
        assert!(!store.blob.exists(&first.blob_key).await.unwrap());
    }
    #[tokio::test]
    async fn retained_records_are_selected_by_container_memory() {
        selection_by_container(InMemorySparqClient::new()).await;
    }
    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn retained_records_are_selected_by_container_embedded() {
        selection_by_container(crate::store::EmbeddedSparqClient::in_memory().unwrap()).await;
    }

    #[test]
    fn data_resources_exclude_access_control_documents() {
        for name in [".acl", "note.acl", ".acr", "note.acr"] {
            assert!(!data_resource(&format!("{ROOT}{name}")), "{name}");
        }
        assert!(data_resource(IRI));
        assert!(data_resource(&format!("{ROOT}folder.acr/note")));
    }

    async fn overwrite_reclaims_only_pruned_keys<S: SparqClient>(index: S) {
        use crate::store::counting::{BackendCounters, CountingSparqClient};
        let counters = std::sync::Arc::new(BackendCounters::default());
        let store = CompositeStore::new(
            CountingSparqClient::new(index, counters.clone()),
            InMemoryBlobStore::new(),
        )
        .with_history(config())
        .unwrap();
        for body in ["one", "two", "three"] {
            let before = counters.snapshot();
            store
                .write(IRI, Bytes::copy_from_slice(body.as_bytes()), "text/plain")
                .await
                .unwrap();
            let delta = counters.snapshot().since(&before);
            assert_eq!(
                delta.sparql_updates, 0,
                "no reclaim when no version was pruned"
            );
            assert_eq!(delta.sparql_queries, 0);
        }
        let versions = store.versions(Some(IRI)).await.unwrap();
        assert_eq!(versions.len(), 2);
        for v in versions {
            assert!(store.blob.exists(&v.meta.blob_key).await.unwrap());
        }
    }
    #[tokio::test]
    async fn overwrite_without_pruning_makes_no_reclaim_calls_memory() {
        overwrite_reclaims_only_pruned_keys(InMemorySparqClient::new()).await;
    }
    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn overwrite_without_pruning_makes_no_reclaim_calls_embedded() {
        overwrite_reclaims_only_pruned_keys(
            crate::store::EmbeddedSparqClient::in_memory().unwrap(),
        )
        .await;
    }

    #[test]
    fn http_backend_refused_only_when_enabled() {
        assert!(HistoryConfig::default().validate_backend("http").is_ok());
        for (versions, trash) in [(true, false), (false, true), (true, true)] {
            let c = HistoryConfig {
                versions,
                trash,
                ..Default::default()
            };
            assert!(c.validate_backend("http").is_err());
            assert!(c.validate_backend("memory").is_ok());
            assert!(c.validate_backend("embedded").is_ok());
        }
    }
    async fn bounded_sweep<S: SparqClient>(index: S) {
        let store = CompositeStore::new(index, InMemoryBlobStore::new())
            .with_history(config())
            .unwrap();
        for n in 0..5 {
            let iri = format!("{ROOT}item-{n}");
            store
                .write(&iri, Bytes::from_static(b"body"), "text/plain")
                .await
                .unwrap();
            let mut f = fields();
            f.original = iri.clone();
            f.trashed_at = SystemTime::UNIX_EPOCH + Duration::from_secs(100 + n);
            store.trash_resource(&iri, None, f).await.unwrap();
        }
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000);
        assert_eq!(store.sweep_history(now, 2).await.unwrap(), 2);
        let left = store.trash_entries().await.unwrap();
        assert_eq!(left.len(), 3);
        assert_eq!(left[0].fields.original, format!("{ROOT}item-2"));
        assert_eq!(store.sweep_history(now, 2).await.unwrap(), 2);
        assert_eq!(store.sweep_history(now, 2).await.unwrap(), 1);
        assert!(store.trash_entries().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn sweep_removes_a_bounded_oldest_batch_memory() {
        bounded_sweep(InMemorySparqClient::new()).await;
    }
    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn sweep_removes_a_bounded_oldest_batch_embedded() {
        bounded_sweep(crate::store::EmbeddedSparqClient::in_memory().unwrap()).await;
    }

    async fn expiry_commit_order<S: SparqClient>(index: S) {
        let store = CompositeStore::new(index, InMemoryBlobStore::new())
            .with_history(config())
            .unwrap();
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        for state in 0..13 {
            let key = format!("state-{state}");
            store
                .blob
                .put(&key, Bytes::from(key.clone()))
                .await
                .unwrap();
            let meta = crate::store::ResourceMeta {
                content_type: "text/plain".into(),
                blob_key: key.clone(),
                etag: key,
                last_modified: Some(at),
            };
            store
                .sparq
                .replace_meta_retaining(
                    IRI,
                    meta,
                    // Member order deliberately opposes commit order.
                    &format!("{}{:032x}", version_dir(IRI), 20 - state),
                    VersionFields {
                        original: IRI.into(),
                        max_versions: 20,
                    },
                )
                .await
                .unwrap();
        }
        let before = store.versions(Some(IRI)).await.unwrap();
        assert_eq!(before.len(), 12);
        assert_eq!(
            before
                .iter()
                .map(|v| v.meta.etag.clone())
                .collect::<Vec<_>>(),
            (0..12)
                .map(|state| format!("state-{state}"))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            store
                .sweep_history(at + config().versions_retention, 2)
                .await
                .unwrap(),
            2
        );
        let remaining = store.versions(Some(IRI)).await.unwrap();
        assert_eq!(remaining, before[2..]);
        for (position, version) in before.iter().enumerate() {
            assert_eq!(
                store.blob.exists(&version.meta.blob_key).await.unwrap(),
                position >= 2
            );
        }
        assert_eq!(store.read(IRI).await.unwrap().body, "state-12");
    }
    #[tokio::test]
    async fn expiry_breaks_equal_timestamps_by_commit_order_memory() {
        expiry_commit_order(InMemorySparqClient::new()).await;
    }
    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn expiry_breaks_equal_timestamps_by_commit_order_embedded() {
        expiry_commit_order(crate::store::EmbeddedSparqClient::in_memory().unwrap()).await;
    }

    async fn maximum<S: SparqClient>(index: S) {
        let mut c = config();
        c.versions_max = 2;
        let store = CompositeStore::new(index, InMemoryBlobStore::new())
            .with_history(c)
            .unwrap();
        let first = store
            .write(IRI, Bytes::from_static(b"one"), "text/plain")
            .await
            .unwrap();
        for body in ["two", "three", "four"] {
            store
                .write(IRI, Bytes::copy_from_slice(body.as_bytes()), "text/plain")
                .await
                .unwrap();
        }
        let versions = store.versions(Some(IRI)).await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(
            store
                .read_at(&versions[0].member, &versions[0].meta)
                .await
                .unwrap(),
            "two"
        );
        assert_eq!(
            store
                .read_at(&versions[1].member, &versions[1].meta)
                .await
                .unwrap(),
            "three"
        );
        assert!(!store.blob.exists(&first.blob_key).await.unwrap());
    }
    #[tokio::test]
    async fn maximum_retains_the_latest_states_and_reclaims_removed_bytes_memory() {
        maximum(InMemorySparqClient::new()).await;
    }
    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn maximum_retains_the_latest_states_and_reclaims_removed_bytes_embedded() {
        maximum(crate::store::EmbeddedSparqClient::in_memory().unwrap()).await;
    }

    #[tokio::test]
    async fn permanent_delete_reclaims_history_after_both_services_are_disabled() {
        let mut store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new())
            .with_history(config())
            .unwrap();
        let first = store
            .write(IRI, Bytes::from_static(b"one"), "text/plain")
            .await
            .unwrap();
        store
            .write(IRI, Bytes::from_static(b"two"), "text/plain")
            .await
            .unwrap();
        store.history = HistoryConfig::default();
        store.delete(IRI, None).await.unwrap();
        assert!(store.versions(Some(IRI)).await.unwrap().is_empty());
        assert!(!store
            .sparq
            .referenced_blob_keys()
            .await
            .unwrap()
            .contains(&first.blob_key));
        assert!(!store.blob.exists(&first.blob_key).await.unwrap());
    }
}
