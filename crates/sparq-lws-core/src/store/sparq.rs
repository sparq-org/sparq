// AUTHORED-BY Claude Opus 4.8
//! The SPARQ client seam — the **authoritative** source for RDF data, metadata, and containment.
//!
//! Per the maintainer's directive, SPARQ (queried over its HTTP API) is the system of record for the
//! resource graph and its metadata (existence, content type, the `s3Key` byte-pointer). Read paths
//! consult SPARQ, **not** an S3 LIST/HEAD (the same "QLever/SPARQ is the source of truth" invariant
//! as the production server). This module defines the [`SparqClient`] trait + an in-memory test impl.
//!
//! M2: the live HTTP client (a SPARQL Query/Update client over SPARQ's endpoint, with the bearer
//! gating SPARQ requires for UPDATE) plugs in behind this trait. It needs a running SPARQ instance,
//! so it is exercised by an integration test, not the M1 unit tests.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;

use super::InMemoryStoreLimits;

/// The authoritative metadata SPARQ holds for a resource (the index record, not the bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceMeta {
    /// The RDF content type the resource was stored as (e.g. `text/turtle`).
    pub content_type: String,
    /// The opaque blob-store key the bytes live under (the `pss:s3Key` pointer).
    pub blob_key: String,
    /// An opaque entity tag for conditional requests. M2: derived from the SPARQ index state.
    pub etag: String,
    /// The resource's server-recorded modification time (the `pss:modified` `xsd:dateTime` in the
    /// index), or `None` when the index holds no modification time for this resource.
    ///
    /// This is what makes conditional `If-Modified-Since` (RFC 9110 §13.1.3) LIVE: the read path
    /// threads it into [`crate::ldp::conditional::evaluate_read`], which serves a 304 iff this time
    /// is `≤` the header date. `None` ⇒ the modification time is unknown ⇒ the evaluator serves a
    /// fresh 200 (never a spurious 304). A write ([`super::Store::write`] /
    /// [`super::Store::create_in_container`]) stamps it to the write instant, so a re-write bumps it
    /// and a subsequent `If-Modified-Since` correctly re-serves the changed representation.
    pub last_modified: Option<SystemTime>,
}

/// The result of ONE combined read-plan lookup ([`SparqClient::read_plan`]) — the whole per-read
/// metadata round-trip of the read path (`research/lws-design-records.md` §7): the target's
/// authoritative metadata AND the presence/etag of every ACL candidate on its resolution chain,
/// answered together.
///
/// Two distinct IRI roles (load-bearing for `.acl` targets — the design's "two roles" note): the
/// **target** is the RAW request target (for a GET of `foo.acl` that is `foo.acl` itself — the
/// bytes to serve), while the **candidates** are derived from the PROTECTED resource (`foo`), so
/// the ACL chain starts at `foo.acl`, never probes a non-existent `foo.acl.acl`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPlan {
    /// The target's authoritative metadata, or `None` if it is not indexed (⇒ 404 — decided by the
    /// caller AFTER authorization, exactly as the sequential path did).
    pub target: Option<ResourceMeta>,
    /// One entry PER candidate, in the caller's (nearest-first) order: `(acl IRI, Some(etag))` when
    /// that ACL is indexed, `(acl IRI, None)` when authoritatively absent. Presence here came from
    /// SPARQ itself (never a cache), so an absent row is authoritatively absent — the "SPARQ is the
    /// source of truth" invariant is intact.
    pub acls: Vec<(String, Option<String>)>,
}

/// The result of an atomic empty-container delete ([`SparqClient::delete_meta_if_empty`] /
/// [`super::Store::delete_container_if_empty`]).
///
/// The three variants are what let the LDP handler map the HTTP status WITHOUT a separate pre-read
/// that could race the delete: the existence + empty check and the delete are decided in ONE store
/// operation, so a child POSTed concurrently is either observed (⇒ [`NotEmpty`](Self::NotEmpty),
/// nothing deleted) or arrives strictly after the container's record is gone (⇒ its create then
/// fails the container-EXISTS guard) — never a window where an empty-check passes and the delete
/// then orphans a just-created child (the TOCTOU the separate `list_children` + `delete` had).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    /// The container existed, was empty, and was deleted.
    Deleted,
    /// The container existed but had members — NOTHING was deleted (the handler maps this to 409).
    NotEmpty,
    /// The container did not exist (the handler maps this to 404).
    NotFound,
}

/// A SPARQ-client error (opaque — never leaks backend detail to a client).
#[derive(Debug, thiserror::Error)]
pub enum SparqError {
    #[error("resource not indexed")]
    NotFound,
    /// The configured in-memory resource-count ceiling would be exceeded.
    #[error("in-memory index quota exceeded")]
    QuotaExceeded,
    #[error("sparq backend error: {0}")]
    Backend(String),
}

/// A query-build failure (an IRIREF-invalid untrusted IRI) is a FATAL backend error — fail-closed,
/// never silently escaped/aliased.
impl From<super::sparql::BuildError> for SparqError {
    fn from(e: super::sparql::BuildError) -> Self {
        SparqError::Backend(format!("fatal: {e}"))
    }
}

/// The authoritative RDF index over SPARQ.
///
/// M1 defined only the metadata-record operations needed by GET/HEAD/PUT. M2 adds DELETE
/// ([`SparqClient::delete_meta`]) and containment membership ([`SparqClient::create_child`] /
/// [`remove_child`](SparqClient::remove_child) / [`list_children`](SparqClient::list_children)) —
/// SPARQ is authoritative for containment, so POST (mint a child) + the empty-container DELETE check
/// flow through it, never an S3 LIST. [GPT-5.6] The in-memory implementation exposes its concrete
/// `usage()` and `quota()` views without imposing that backend-specific model on this trait. M2-next
/// retains the WAC/ACP ACL-document graphs the (future) access-evaluation step reads.
#[async_trait]
pub trait SparqClient: Send + Sync {
    /// Fetch the authoritative metadata for a resource IRI, or [`SparqError::NotFound`].
    async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError>;

    /// Upsert the authoritative metadata record for a resource IRI.
    async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError>;

    /// Whether the resource is indexed (the authoritative existence check — never an S3 HEAD).
    async fn exists(&self, iri: &str) -> Result<bool, SparqError>;

    /// Remove a resource's metadata record. Idempotent: deleting an absent IRI is `Ok(())` (the
    /// caller's existence check governs the 404, so this is a no-op-on-absent at the index layer).
    async fn delete_meta(&self, iri: &str) -> Result<(), SparqError>;

    /// ATOMICALLY delete a container's record + its (empty) containment set + its edge in the PARENT's
    /// graph, ONLY if it is empty — all in ONE index operation.
    ///
    /// The existence check, the `ldp:contains`-empty check, the record delete, AND the parent-edge
    /// detach (`<parent> ldp:contains <container>`, which lives in the PARENT's graph) are ONE index
    /// operation with NO interleaving. This closes BOTH TOCTOU windows: (1) a concurrent `create_child`
    /// adding a member between a separate empty-check and a separate delete, and (2) — the reason the
    /// parent edge is folded in here rather than detached by the caller afterwards — a concurrent POST
    /// recreating the child under the parent in the window between the graph delete and a separate, later
    /// parent-edge detach (which would then orphan the just-recreated child). On the live SPARQ path
    /// this is a SINGLE `DELETE { container-graph ; parent-edge } INSERT { marker } WHERE { exists+empty
    /// guard }` modify whose WHERE is evaluated once pre-modification (see
    /// [`super::sparql::update_delete_container_if_empty`]).
    ///
    /// `parent` is `None` for a root/parentless container (nothing to detach). Returns
    /// [`DeleteOutcome::Deleted`] if it existed + was empty + is now gone, [`DeleteOutcome::NotEmpty`]
    /// if it had members (nothing deleted), or [`DeleteOutcome::NotFound`] if it was not indexed.
    async fn delete_meta_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<DeleteOutcome, SparqError>;

    /// ATOMICALLY create a child resource record: in a SINGLE index operation, verify `container` is
    /// indexed (else [`SparqError::NotFound`]) and commit BOTH `child`'s metadata record AND the
    /// `container`→`child` containment edge together.
    ///
    /// Committing the metadata and the membership in one atomic step is what makes the POST path
    /// race-free: there is NO window in which the edge exists without the child's metadata (or vice
    /// versa), so no removal-based compensation is needed and a concurrent same-IRI creator cannot
    /// observe — or have removed — a half-built containment. (The live impl is one SPARQL UPDATE with
    /// a `container`-EXISTS guard that inserts both triples.) The blob bytes are written by the caller
    /// BEFORE this call; if this fails (missing container), those bytes are orphaned and GC'd by the
    /// reconciler — the same crash-consistency model as [`SparqClient::put_meta`].
    async fn create_child(
        &self,
        container: &str,
        child: &str,
        meta: ResourceMeta,
    ) -> Result<(), SparqError>;

    /// Remove `child` from `container`'s membership. Idempotent on an absent edge.
    async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError>;

    /// List the IRIs of `container`'s direct children (its `ldp:contains` members).
    async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError>;

    /// The set of blob-store keys that ANY index record currently references (the `pss:blobKey`
    /// pointers across every resource graph).
    ///
    /// This is the authoritative answer to "which bytes are still referenced?", the half of the orphan
    /// sweep that only SPARQ can give: the reconciler enumerates the physically-stored blobs (via
    /// [`super::BlobStore::list`]) and treats any stored key NOT in this set as a candidate orphan.
    /// Returned as ONE set (computed once per sweep) rather than a per-key `is_referenced` check, so a
    /// GC is O(1) backend calls, not O(N-blobs) — see [`super::sparql::select_referenced_blob_keys`].
    /// Fail-closed: any backend error propagates, so the reconciler ABORTS rather than treating a
    /// failed referenced-set query as "nothing is referenced" (which would delete the whole pod).
    async fn referenced_blob_keys(&self) -> Result<std::collections::HashSet<String>, SparqError>;

    /// ONE combined read-plan lookup (read-2 — `research/lws-design-records.md` §7): the target's
    /// metadata + the presence/etag of every ACL candidate, in one backend round-trip.
    ///
    /// The candidate set is computed up front by the caller (pure string work over the PROTECTED
    /// resource); probing every candidate is semantically identical to the sequential child→root
    /// walk because each probe is an independent read and "nearest present wins" is decided by the
    /// caller's ORDERING over the returned rows, not by call sequence.
    ///
    /// The DEFAULT implementation loops [`get_meta`](SparqClient::get_meta) — semantically exact,
    /// one round-trip per IRI — so every existing impl (the embedded engine) works unchanged. The
    /// in-memory double overrides it with one atomic index pass and the HTTP client overrides it
    /// with the ONE combined `VALUES ?g { … }` SELECT (the RTT win). Fail-closed: any backend error
    /// on any part of the plan fails the WHOLE plan — there is no per-candidate partial-degrade
    /// path (design invariant 4).
    async fn read_plan(
        &self,
        target: &str,
        acl_candidates: &[String],
    ) -> Result<ReadPlan, SparqError> {
        let target_meta = match self.get_meta(target).await {
            Ok(m) => Some(m),
            Err(SparqError::NotFound) => None,
            Err(e) => return Err(e),
        };
        let mut acls = Vec::with_capacity(acl_candidates.len());
        for candidate in acl_candidates {
            let etag = match self.get_meta(candidate).await {
                Ok(m) => Some(m.etag),
                Err(SparqError::NotFound) => None,
                Err(e) => return Err(e),
            };
            acls.push((candidate.clone(), etag));
        }
        Ok(ReadPlan {
            target: target_meta,
            acls,
        })
    }
}

/// An in-memory [`SparqClient`] for tests and the M1/M2 boot-without-SPARQ path.
///
/// Holds the metadata records AND the containment edges (container IRI → ordered child IRIs) behind
/// a single lock, so a POST/DELETE that touches both stays internally consistent under the test
/// double's coarse locking.
pub struct InMemorySparqClient {
    inner: Mutex<Index>,
    /// [GPT-5.6] Immutable admission ceiling, enforced under the index lock.
    limits: InMemoryStoreLimits,
}

impl Default for InMemorySparqClient {
    fn default() -> Self {
        Self::with_limits(InMemoryStoreLimits::default())
    }
}

/// The in-memory index state: metadata records + containment membership.
#[derive(Default)]
struct Index {
    meta: HashMap<String, ResourceMeta>,
    /// container IRI → its direct children, kept in insertion order (a `Vec`, de-duplicated).
    children: HashMap<String, Vec<String>>,
}

impl InMemorySparqClient {
    /// Build with the bounded default limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build with an explicit resource-count limit.
    pub fn with_limits(limits: InMemoryStoreLimits) -> Self {
        Self {
            inner: Mutex::new(Index::default()),
            limits,
        }
    }

    /// Return the exact number of indexed resources.
    pub fn usage(&self) -> Result<usize, SparqError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        Ok(guard.meta.len())
    }

    /// Return the immutable admission limits configured for this index.
    pub fn quota(&self) -> InMemoryStoreLimits {
        self.limits
    }

    /// Reserve a metadata slot for a new IRI, or fail without mutating the index.
    fn admit_resource(&self, index: &Index, iri: &str) -> Result<(), SparqError> {
        if !index.meta.contains_key(iri) && index.meta.len() >= self.limits.max_resource_count {
            return Err(SparqError::QuotaExceeded);
        }
        Ok(())
    }
}

#[async_trait]
impl SparqClient for InMemorySparqClient {
    async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        guard.meta.get(iri).cloned().ok_or(SparqError::NotFound)
    }

    async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        self.admit_resource(&guard, iri)?;
        guard.meta.insert(iri.to_string(), meta);
        Ok(())
    }

    async fn exists(&self, iri: &str) -> Result<bool, SparqError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        Ok(guard.meta.contains_key(iri))
    }

    async fn delete_meta(&self, iri: &str) -> Result<(), SparqError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        guard.meta.remove(iri);
        // Parity with the live SPARQ path (`DROP SILENT GRAPH <iri>`): a resource's named graph holds
        // BOTH its index record AND — if it is a container — its `ldp:contains` edges, so dropping the
        // record drops the containment set too. Mirror that here by clearing `iri`'s own children
        // entry, so a delete-then-recreate of a container at the same IRI cannot inherit a stale
        // (empty-or-not) membership list. (The empty-container DELETE check has already run in the
        // handler, so any surviving entry would be a leak, not a live member.)
        guard.children.remove(iri);
        Ok(())
    }

    async fn delete_meta_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<DeleteOutcome, SparqError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        // ONE atomic step under the SINGLE lock — there is no `await` between the checks, the delete,
        // and the parent-edge detach, so no concurrent `create_child` (which takes the same lock) can
        // interleave a member between the empty-check and the delete, NOR recreate the child under the
        // parent between the delete and the detach: a concurrent create either runs fully BEFORE this
        // (its child is then observed ⇒ NotEmpty, nothing deleted) or fully AFTER (the container
        // record is gone ⇒ its container-EXISTS guard rejects it). No orphaning window exists. This
        // mirrors the live path's SINGLE atomic modify (container graph + parent edge + marker).
        if !guard.meta.contains_key(iri) {
            return Ok(DeleteOutcome::NotFound);
        }
        // Empty iff there is no non-empty `ldp:contains` set for this container.
        let has_members = guard.children.get(iri).is_some_and(|kids| !kids.is_empty());
        if has_members {
            return Ok(DeleteOutcome::NotEmpty);
        }
        // Empty + present ⇒ drop the record AND its (empty) containment entry together — parity with
        // the live `DROP SILENT GRAPH`, so a re-created container at the same IRI inherits no stale set.
        guard.meta.remove(iri);
        guard.children.remove(iri);
        // ...and detach the parent edge in the SAME atomic step (folded in, per Finding 2), so there is
        // no window in which the container graph is gone but the parent still `ldp:contains` it.
        if let Some(p) = parent {
            if let Some(entry) = guard.children.get_mut(p) {
                entry.retain(|c| c != iri);
            }
        }
        Ok(DeleteOutcome::Deleted)
    }

    async fn create_child(
        &self,
        container: &str,
        child: &str,
        meta: ResourceMeta,
    ) -> Result<(), SparqError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        // ONE atomic step under the single lock: verify the container exists, then commit BOTH the
        // child's metadata AND the containment edge together. No window separates them, so there is
        // nothing for a concurrent creator (or a failed-request compensation) to observe half-built.
        if !guard.meta.contains_key(container) {
            return Err(SparqError::NotFound);
        }
        self.admit_resource(&guard, child)?;
        guard.meta.insert(child.to_string(), meta);
        let entry = guard.children.entry(container.to_string()).or_default();
        if !entry.iter().any(|c| c == child) {
            entry.push(child.to_string());
        }
        Ok(())
    }

    async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        if let Some(entry) = guard.children.get_mut(container) {
            entry.retain(|c| c != child);
        }
        Ok(())
    }

    async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        Ok(guard.children.get(container).cloned().unwrap_or_default())
    }

    async fn referenced_blob_keys(&self) -> Result<std::collections::HashSet<String>, SparqError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        // Every metadata record's `blob_key` is a live reference. Mirrors the live path's
        // `SELECT DISTINCT ?bk` over the `pss:blobKey` predicate across all graphs.
        Ok(guard.meta.values().map(|m| m.blob_key.clone()).collect())
    }

    async fn read_plan(
        &self,
        target: &str,
        acl_candidates: &[String],
    ) -> Result<ReadPlan, SparqError> {
        // ONE atomic index pass under the single lock — the in-memory analogue of the live
        // client's ONE combined SELECT (one consistent snapshot of target + all candidates), so the
        // counting decorator's 1-query model holds for this double too.
        let guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        Ok(ReadPlan {
            target: guard.meta.get(target).cloned(),
            acls: acl_candidates
                .iter()
                .map(|c| (c.clone(), guard.meta.get(c).map(|m| m.etag.clone())))
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn meta(blob_key: &str) -> ResourceMeta {
        ResourceMeta {
            content_type: "text/plain".into(),
            blob_key: blob_key.into(),
            etag: "\"etag\"".into(),
            last_modified: None,
        }
    }

    #[tokio::test]
    async fn resource_limit_rejects_new_records_but_allows_replacement() {
        // [GPT-5.6] Mutation witness: removing `admit_resource` makes the second IRI succeed and
        // changes usage from the configured one-resource ceiling.
        let limits = InMemoryStoreLimits::new(usize::MAX, 1);
        let index = InMemorySparqClient::with_limits(limits);

        index.put_meta("urn:a", meta("a1")).await.unwrap();
        assert_eq!(index.usage().unwrap(), 1);
        assert!(matches!(
            index.put_meta("urn:b", meta("b")).await,
            Err(SparqError::QuotaExceeded)
        ));
        assert_eq!(index.usage().unwrap(), 1);

        index.put_meta("urn:a", meta("a2")).await.unwrap();
        assert_eq!(index.get_meta("urn:a").await.unwrap().blob_key, "a2");
        assert_eq!(index.quota(), limits);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_metadata_writes_cannot_over_admit_resource_limit() {
        // [GPT-5.6] Admission and insertion happen under one lock, so racing writers cannot each
        // observe a spare final slot.
        let index = Arc::new(InMemorySparqClient::with_limits(InMemoryStoreLimits::new(
            usize::MAX,
            8,
        )));
        let mut handles = Vec::new();
        for n in 0..64 {
            let index = Arc::clone(&index);
            handles.push(tokio::spawn(async move {
                index.put_meta(&format!("urn:{n}"), meta("blob")).await
            }));
        }

        let mut admitted = 0;
        for handle in handles {
            match handle.await.unwrap() {
                Ok(()) => admitted += 1,
                Err(SparqError::QuotaExceeded) => {}
                Err(other) => panic!("unexpected index error: {other}"),
            }
        }
        assert_eq!(admitted, 8);
        assert_eq!(index.usage().unwrap(), 8);
    }
}
