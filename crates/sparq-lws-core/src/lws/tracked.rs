//! The store as the LWS server sees it: every change to what a container listing shows goes
//! through here, so none can skip moving the listing's snapshot.
//!
//! A container's listing shows the container's own date and, for each member, its type, size
//! and date. A listing that may have read part of a change, or read it while it was being made,
//! must not go out under a date (a later `If-Modified-Since` could match it, and a different page
//! answer `304`). Every mutating [`Store`] method of [`Tracked`] runs inside
//! `Generations::mutating`, whatever made the call (a request's own steps, a touch, a rollback
//! or a retried undo): it moves the [generation](Generations::generation) of the resource changed
//! and of the container listing it, before and after the change, and marks both
//! [in flight](Generations::in_flight) meanwhile. A listing made while either holds has no date,
//! and one during which the generation moved is made again.
//!
//! The wrapped store is reachable only through [`Store`] in non-test code: there is no other way
//! to change it.

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use bytes::Bytes;

use super::{resources, META_SUFFIX};
use crate::error::ServerResult;
use crate::store::sparq::{DeleteOutcome, ResourceMeta};
use crate::store::{Fence, ReadPlan, Resource, Store, ValidatedChildIri};

/// How many counters the resources are spread over.
const SLOTS: usize = 256;

fn slot(uri: &str) -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    uri.hash(&mut h);
    (h.finish() as usize) % SLOTS
}

/// The snapshot counters of the resources a [`Tracked`] store holds.
pub struct Generations {
    root: String,
    generations: [AtomicU64; SLOTS],
    in_flight: [AtomicU64; SLOTS],
}

impl Generations {
    fn new(root: String) -> Self {
        Self {
            root,
            generations: std::array::from_fn(|_| Default::default()),
            in_flight: std::array::from_fn(|_| Default::default()),
        }
    }

    /// A count that moves whenever what the container `uri`'s listing shows may have changed:
    /// before and after every change to it or to a member of it, and when a change to its listing
    /// is published or its touch ends (see `LwsState::touching`). Resources share
    /// counters, so an unrelated change can make a listing be made again, never a change go
    /// unnoticed.
    pub fn generation(&self, uri: &str) -> u64 {
        self.generations[slot(uri)].load(Ordering::Acquire)
    }

    /// Whether a change to `uri`, or to a member of it, is being made.
    pub fn in_flight(&self, uri: &str) -> bool {
        self.in_flight[slot(uri)].load(Ordering::Acquire) > 0
    }

    /// Move the generation of `uri` and of the container listing it.
    pub(crate) fn changed(&self, uri: &str) {
        for s in self.slots(&[uri]) {
            self.generations[s].fetch_add(1, Ordering::AcqRel);
        }
    }

    /// The counters a change to the store keys `keys` concerns: of each resource (a metadata
    /// record's key stands for its resource) and of the container listing it.
    fn slots(&self, keys: &[&str]) -> Vec<usize> {
        let mut slots = Vec::with_capacity(keys.len() * 2);
        for key in keys {
            let iri = key.strip_suffix(META_SUFFIX).unwrap_or(key);
            let parent = resources::parent_of(iri, &self.root);
            for s in std::iter::once(slot(iri)).chain(parent.as_deref().map(slot)) {
                if !slots.contains(&s) {
                    slots.push(s);
                }
            }
        }
        slots
    }

    /// Mark a change to `keys` begun, until the guard is dropped (see the module docs).
    fn mutating(&self, keys: &[&str]) -> Mutating<'_> {
        let slots = self.slots(keys);
        for &s in &slots {
            self.in_flight[s].fetch_add(1, Ordering::AcqRel);
            self.generations[s].fetch_add(1, Ordering::AcqRel);
        }
        Mutating { gens: self, slots }
    }
}

/// A change being made (see [`Generations::mutating`]); its end is marked on drop, so a change
/// cut short is marked over too.
struct Mutating<'a> {
    gens: &'a Generations,
    slots: Vec<usize>,
}

impl Drop for Mutating<'_> {
    fn drop(&mut self) {
        for &s in &self.slots {
            self.gens.generations[s].fetch_add(1, Ordering::AcqRel);
            self.gens.in_flight[s].fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// A [`Store`] whose every change moves the snapshot counters of what it changes (see the
/// module docs).
///
/// Cloning it gives another handle to the same store and counters: what a task that must not keep
/// the server's state alive holds (see `records::Settlements`).
pub struct Tracked<S> {
    inner: std::sync::Arc<S>,
    gens: std::sync::Arc<Generations>,
    #[cfg(feature = "sparql-endpoint")]
    snapshot: std::sync::Arc<super::snapshot::Gate>,
    /// The backend's lease ([`Store::lifecycle`]): `inner`'s own when it keeps one (so a
    /// `Tracked` over a `Tracked` shares its lease), shared by every clone, so a service built
    /// over a clone, or over a wrapper of one, is refused while this one runs.
    lifecycle: std::sync::Arc<crate::store::Lifecycle>,
}

impl<S> Clone for Tracked<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            gens: self.gens.clone(),
            #[cfg(feature = "sparql-endpoint")]
            snapshot: self.snapshot.clone(),
            lifecycle: self.lifecycle.clone(),
        }
    }
}

impl<S: Store> Tracked<S> {
    /// Track `inner`, whose storage root is `root`. Its backend's lease is `inner`'s own when it
    /// keeps one ([`Store::lifecycle`]), and a new one otherwise: no caller chooses it, so
    /// wrapping a store that is already tracked keeps the lease of the service running over it.
    pub fn new(inner: S, root: String) -> Self {
        Self {
            lifecycle: inner.lifecycle().unwrap_or_default(),
            inner: std::sync::Arc::new(inner),
            gens: std::sync::Arc::new(Generations::new(root)),
            #[cfg(feature = "sparql-endpoint")]
            snapshot: Default::default(),
        }
    }

    pub(crate) async fn change<T>(
        &self,
        work: impl std::future::Future<Output = T>,
    ) -> ServerResult<T> {
        #[cfg(feature = "sparql-endpoint")]
        {
            self.snapshot.change(work).await
        }
        #[cfg(not(feature = "sparql-endpoint"))]
        {
            Ok(work.await)
        }
    }

    pub(crate) async fn publish<T>(&self, work: impl std::future::Future<Output = T>) -> T {
        let mut work = std::pin::pin!(work);
        loop {
            if let Ok(out) = self.change(work.as_mut()).await {
                return out;
            }
            tokio::task::yield_now().await;
        }
    }

    #[cfg(feature = "sparql-endpoint")]
    pub(crate) async fn snapshot(&self, barrier: bool) -> Option<super::snapshot::Snapshot> {
        self.snapshot.snapshot(barrier).await
    }

    #[cfg(all(test, feature = "sparql-endpoint"))]
    pub(crate) fn barrier_pending(&self) -> bool {
        self.snapshot.barrier_pending()
    }

    /// The snapshot counters.
    pub fn generations(&self) -> &Generations {
        &self.gens
    }
}

/// Tests reach the test double's controls; nothing else can reach the store untracked.
#[cfg(test)]
impl<S> std::ops::Deref for Tracked<S> {
    type Target = S;
    fn deref(&self) -> &S {
        &self.inner
    }
}

#[async_trait]
impl<S: Store> Store for Tracked<S> {
    fn history_config(&self) -> crate::store::history::HistoryConfig {
        self.inner.history_config()
    }
    async fn versions(
        &self,
        original: Option<&str>,
    ) -> ServerResult<Vec<crate::store::history::VersionEntry>> {
        self.inner.versions(original).await
    }
    async fn trash_entries(&self) -> ServerResult<Vec<crate::store::history::TrashEntry>> {
        self.inner.trash_entries().await
    }
    async fn version_entry(
        &self,
        member: &str,
    ) -> ServerResult<Option<crate::store::history::VersionEntry>> {
        self.inner.version_entry(member).await
    }
    async fn trash_entry(
        &self,
        member: &str,
    ) -> ServerResult<Option<crate::store::history::TrashEntry>> {
        self.inner.trash_entry(member).await
    }
    async fn trash_resource(
        &self,
        iri: &str,
        parent: Option<&str>,
        fields: crate::store::history::TrashFields,
    ) -> ServerResult<String> {
        let _m = self.gens.mutating(&[iri]);
        self.inner.trash_resource(iri, parent, fields).await
    }
    async fn restore_trash(&self, member: &str, target: &str, parent: &str) -> ServerResult<()> {
        let _m = self.gens.mutating(&[target, parent]);
        self.inner.restore_trash(member, target, parent).await
    }
    async fn purge_trash(&self, member: &str) -> ServerResult<()> {
        self.inner.purge_trash(member).await
    }
    async fn sweep_history(&self, now: std::time::SystemTime, limit: usize) -> ServerResult<usize> {
        self.inner.sweep_history(now, limit).await
    }

    async fn read(&self, iri: &str) -> ServerResult<Resource> {
        self.inner.read(iri).await
    }

    async fn meta(&self, iri: &str) -> ServerResult<Option<ResourceMeta>> {
        self.inner.meta(iri).await
    }

    async fn exists(&self, iri: &str) -> ServerResult<bool> {
        self.inner.exists(iri).await
    }

    async fn write(
        &self,
        iri: &str,
        body: Bytes,
        content_type: &str,
    ) -> ServerResult<ResourceMeta> {
        self.change(async {
            let _m = self.gens.mutating(&[iri]);
            self.inner.write(iri, body, content_type).await
        })
        .await?
    }

    async fn create_in_container(
        &self,
        container: &str,
        child: &str,
        body: Bytes,
        content_type: &str,
    ) -> ServerResult<ResourceMeta> {
        self.change(async {
            let _m = self.gens.mutating(&[container, child]);
            self.inner
                .create_in_container(container, child, body, content_type)
                .await
        })
        .await?
    }

    async fn restore(
        &self,
        iri: &str,
        container: Option<&str>,
        body: Bytes,
        meta: &ResourceMeta,
    ) -> ServerResult<ResourceMeta> {
        let keys: Vec<&str> = std::iter::once(iri).chain(container).collect();
        self.change(async {
            let _m = self.gens.mutating(&keys);
            self.inner.restore(iri, container, body, meta).await
        })
        .await?
    }

    async fn hold(&self, iri: &str, holder: &str) -> ServerResult<Option<ResourceMeta>> {
        self.inner.hold(iri, holder).await
    }

    async fn reinstate(
        &self,
        iri: &str,
        container: Option<&str>,
        held: &ResourceMeta,
    ) -> ServerResult<ResourceMeta> {
        let keys: Vec<&str> = std::iter::once(iri).chain(container).collect();
        self.change(async {
            let _m = self.gens.mutating(&keys);
            self.inner.reinstate(iri, container, held).await
        })
        .await?
    }

    async fn release(&self, held: &ResourceMeta, holder: &str) -> ServerResult<()> {
        self.inner.release(held, holder).await
    }

    async fn holds(&self) -> ServerResult<Vec<(String, ResourceMeta)>> {
        self.inner.holds().await
    }

    async fn finish_reclaims(&self) -> ServerResult<()> {
        self.inner.finish_reclaims().await
    }

    fn exclusive(&self) -> ServerResult<()> {
        self.inner.exclusive()
    }

    async fn delete(&self, iri: &str, parent: Option<&str>) -> ServerResult<()> {
        let keys: Vec<&str> = std::iter::once(iri).chain(parent).collect();
        self.change(async {
            let _m = self.gens.mutating(&keys);
            self.inner.delete(iri, parent).await
        })
        .await?
    }

    async fn delete_container_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> ServerResult<DeleteOutcome> {
        let keys: Vec<&str> = std::iter::once(iri).chain(parent).collect();
        self.change(async {
            let _m = self.gens.mutating(&keys);
            self.inner.delete_container_if_empty(iri, parent).await
        })
        .await?
    }

    /// A write not settled ([`crate::error::ServerError::Unsettled`]) may still land after its
    /// call returned, until its fence has succeeded: the fence is marked as a change to what that
    /// write changes (the fence carries it, so nothing is kept per unsettled write).
    async fn fence(&self, fence: &Fence) -> ServerResult<()> {
        let keys: Vec<&str> = fence.touches().iter().map(String::as_str).collect();
        if !keys.is_empty()
            && keys
                .iter()
                .all(|key| crate::ldp::target::names_lws_service_record(key, &self.gens.root))
        {
            let _m = self.gens.mutating(&keys);
            return self.inner.fence(fence).await;
        }
        self.change(async {
            let _m = self.gens.mutating(&keys);
            self.inner.fence(fence).await
        })
        .await?
    }

    async fn read_marker(&self, iri: &str) -> ServerResult<Option<String>> {
        self.inner.read_marker(iri).await
    }

    fn raises_markers(&self) -> bool {
        self.inner.raises_markers()
    }

    fn call_timeout(&self) -> std::time::Duration {
        self.inner.call_timeout()
    }

    /// A marker shows in no listing, so it moves no snapshot.
    async fn raise_marker(&self, iri: &str, value: i64) -> ServerResult<i64> {
        self.inner.raise_marker(iri, value).await
    }

    async fn reset_marker(
        &self,
        iri: &str,
        value: i64,
        lease: &crate::store::LifecycleGuard,
    ) -> ServerResult<()> {
        self.inner.reset_marker(iri, value, lease).await
    }

    fn lifecycle(&self) -> Option<std::sync::Arc<crate::store::Lifecycle>> {
        Some(self.lifecycle.clone())
    }

    async fn list_children(&self, container: &str) -> ServerResult<Vec<ValidatedChildIri>> {
        self.inner.list_children(container).await
    }

    async fn list_children_within(
        &self,
        container: &str,
        max_bytes: usize,
    ) -> ServerResult<Option<Vec<ValidatedChildIri>>> {
        self.inner.list_children_within(container, max_bytes).await
    }

    async fn read_plan(&self, target: &str, acl_candidates: &[String]) -> ServerResult<ReadPlan> {
        self.inner.read_plan(target, acl_candidates).await
    }

    async fn read_at(&self, iri: &str, meta: &ResourceMeta) -> ServerResult<Bytes> {
        self.inner.read_at(iri, meta).await
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_store::state;
    use super::*;

    /// Codex review of #6836: an unsettled write was remembered until its fence, so one settled
    /// by a read instead leaked its entry. Nothing is kept per write now: the fence carries what
    /// the write changes, and fencing it moves those snapshots and leaves nothing in flight.
    #[tokio::test]
    async fn a_fence_moves_the_snapshot_of_what_its_write_changes() {
        let (st, store) = state(100).await;
        let root = st.cfg.storage();
        let c = st.cfg.absolute("/c/");
        st.store
            .create_in_container(&root, &c, Bytes::new(), "application/lws+json")
            .await
            .unwrap();
        let d = format!("{c}d");
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        *store.unsettle_next_create.lock().unwrap() = Some(gate.clone());
        let fence = match st
            .store
            .create_in_container(&c, &d, Bytes::from_static(b"d"), "text/plain")
            .await
        {
            Err(crate::error::ServerError::Unsettled { fence, .. }) => fence,
            other => panic!("expected an unsettled write, got {other:?}"),
        };
        assert_eq!(fence.touches(), [c.clone(), d.clone()]);
        let before = [st.generation(&c), st.generation(&d)];
        st.store.fence(&fence).await.unwrap();
        assert!(st.generation(&c) >= before[0] + 2);
        assert!(st.generation(&d) >= before[1] + 2);
        assert!(!st.in_flight(&c) && !st.in_flight(&d));
        // Fenced first: the write, when it arrives, changes nothing.
        gate.add_permits(1);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!st.store.exists(&d).await.unwrap());
    }

    /// Every mutating [`Store`] method, and a journal's rollback, moves the snapshot of the
    /// container listing what it changes, and of what it changes, and leaves nothing in flight.
    #[tokio::test]
    async fn every_kind_of_change_moves_the_listing_snapshot() {
        let (st, _store) = state(100).await;
        let root = st.cfg.storage();
        let c = st.cfg.absolute("/c/");
        let (d, e, f) = (format!("{c}d"), format!("{c}e"), format!("{c}f/"));
        let text = |s: &'static str| Bytes::from_static(s.as_bytes());
        st.store
            .create_in_container(&root, &c, Bytes::new(), "application/lws+json")
            .await
            .unwrap();
        st.store
            .create_in_container(&c, &d, text("d"), "text/plain")
            .await
            .unwrap();
        st.store
            .create_in_container(&c, &f, Bytes::new(), "application/lws+json")
            .await
            .unwrap();
        let moved = |what: &str, before: [u64; 2], uri: &str| {
            assert!(
                st.generation(&c) >= before[0] + 2,
                "{what}: the listing's snapshot"
            );
            assert!(
                st.generation(uri) >= before[1] + 2,
                "{what}: its own snapshot"
            );
            assert!(
                !st.in_flight(&c) && !st.in_flight(uri),
                "{what}: still in flight"
            );
        };
        let at = |uri: &str| [st.generation(&c), st.generation(uri)];

        let g = at(&d);
        st.store.write(&d, text("d2"), "text/plain").await.unwrap();
        moved("write", g, &d);

        let g = at(&d);
        st.put_resource_meta(&d, &Default::default()).await.unwrap();
        moved("metadata write", g, &d);

        let g = at(&e);
        st.store
            .create_in_container(&c, &e, text("e"), "text/plain")
            .await
            .unwrap();
        moved("create", g, &e);

        let meta = st.store.meta(&e).await.unwrap().unwrap();
        let g = at(&e);
        st.store.delete(&e, Some(&c)).await.unwrap();
        moved("delete", g, &e);

        let g = at(&e);
        st.store
            .restore(&e, Some(&c), text("e"), &meta)
            .await
            .unwrap();
        moved("restore", g, &e);

        let g = at(&f);
        st.store
            .delete_container_if_empty(&f, Some(&c))
            .await
            .unwrap();
        moved("container delete", g, &f);

        let mut journal = st.journal();
        journal.write(&d, text("d3"), "text/plain").await.unwrap();
        let g = at(&d);
        assert!(journal.rollback().await.is_none());
        moved("rollback", g, &d);
        let back = st.store.read(&d).await.unwrap();
        assert_eq!(back.body, text("d2"));
    }

    /// A change cut short is marked over all the same.
    #[tokio::test]
    async fn a_change_dropped_midway_is_not_left_in_flight() {
        let (st, store) = state(100).await;
        let d = st.cfg.absolute("/d");
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_write_of.lock().unwrap() = Some((d.clone(), gate.clone()));
        let write = st.store.write(&d, Bytes::from_static(b"d"), "text/plain");
        let cut = tokio::time::timeout(std::time::Duration::from_millis(50), write).await;
        assert!(cut.is_err());
        assert!(!st.in_flight(&d) && !st.in_flight(&st.cfg.storage()));
        gate.add_permits(1);
    }
}
