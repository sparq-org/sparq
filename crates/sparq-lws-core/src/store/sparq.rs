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
    /// A write failed in a way after which it may still commit (an HTTP update that timed out),
    /// and it could not be fenced yet: whether it committed is not known, and may change, until
    /// [`SparqClient::fence`] with `fence` has succeeded. After that, a read tells.
    #[error("sparq backend error: {detail}; whether the write committed is not known yet")]
    Unsettled {
        /// What failed.
        detail: String,
        /// What settles it.
        fence: Fence,
    },
}

/// What settles a write whose outcome is not known ([`SparqError::Unsettled`],
/// [`ServerError::Unsettled`](crate::error::ServerError::Unsettled)): once
/// [`SparqClient::fence`] (or [`Store::fence`](super::Store::fence)) has succeeded with it, the
/// write has either committed or never will, so a read tells which and that no longer changes.
/// Opaque: only the client that failed the write can use it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fence {
    nonce: String,
    touches: Vec<String>,
}

impl Fence {
    /// The fence of the write tagged `nonce`, which changes the resources `touches` (a create's
    /// container and child; a delete's resource and parent).
    pub fn new(nonce: impl Into<String>, touches: Vec<String>) -> Self {
        Self {
            nonce: nonce.into(),
            touches,
        }
    }

    /// The nonce of the write it fences.
    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    /// The resources the write it fences changes, if it lands.
    pub fn touches(&self) -> &[String] {
        &self.touches
    }
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
/// flow through it, never an S3 LIST. The in-memory implementation exposes its concrete
/// `usage()` and `quota()` views without imposing that backend-specific model on this trait. M2-next
/// retains the WAC/ACP ACL-document graphs the (future) access-evaluation step reads.
#[async_trait]
pub trait SparqClient: Send + Sync {
    /// Fetch the authoritative metadata for a resource IRI, or [`SparqError::NotFound`].
    async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError>;

    /// Upsert the authoritative metadata record for a resource IRI.
    async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError>;

    /// Upsert the metadata record for `iri` and return the record it REPLACED (`None` on a create).
    ///
    /// The composite store uses the returned record's `blob_key` to reclaim the superseded bytes
    /// once this commit has succeeded. The DEFAULT implementation is a [`get_meta`](Self::get_meta)
    /// followed by a [`put_meta`](Self::put_meta) — two round-trips, NOT atomic: a concurrent writer
    /// can commit between them, in which case the returned record is that writer's predecessor and
    /// the concurrent writer's own key is left for the reconciler. That is still safe to delete:
    /// blob keys are minted unique per write and an index pointer never moves BACK to a key it has
    /// left, so a key observed here is unreferenced once this commit lands. Backends that can do
    /// both in one indivisible step (the in-memory double, the embedded engine) override this.
    async fn replace_meta(
        &self,
        iri: &str,
        meta: ResourceMeta,
    ) -> Result<Option<ResourceMeta>, SparqError> {
        let previous = match self.get_meta(iri).await {
            Ok(m) => Some(m),
            Err(SparqError::NotFound) => None,
            Err(e) => return Err(e),
        };
        self.put_meta(iri, meta).await?;
        Ok(previous)
    }

    /// Whether the resource is indexed (the authoritative existence check — never an S3 HEAD).
    async fn exists(&self, iri: &str) -> Result<bool, SparqError>;

    /// Name the bytes under `blob_key` as being reclaimed, in one atomic step that also finds no
    /// record naming them as its bytes (a resource's, or a hold's): `Ok(false)`, with nothing
    /// named, when one does. Keys are minted per write and only a record already naming them
    /// (a hold's, by its holder) can bring a record back to them, so once named no record ever
    /// names them again and they may be deleted; then [`end_reclaim`](Self::end_reclaim). A
    /// reclaim cut short is found again with [`reclaiming`](Self::reclaiming).
    ///
    /// The DEFAULT never names any (the bytes are left to the orphan sweep).
    async fn begin_reclaim(&self, blob_key: &str) -> Result<bool, SparqError> {
        let _ = blob_key;
        Ok(false)
    }

    /// `Ok` when this index is this process's alone while the client lives, so nothing another
    /// process does changes it: one held in memory, or one opened from a directory this process
    /// holds locked. A server that keeps state of its own over the index (as the LWS service
    /// does) starts only over such an index. The DEFAULT says it is not: an index another
    /// process may also write (a remote endpoint) gives no such assurance.
    fn exclusive(&self) -> Result<(), SparqError> {
        Err(SparqError::Backend(
            "this index may be written by other processes too".into(),
        ))
    }

    /// Stop naming `blob_key` as being reclaimed ([`begin_reclaim`](Self::begin_reclaim)).
    async fn end_reclaim(&self, blob_key: &str) -> Result<(), SparqError> {
        let _ = blob_key;
        Ok(())
    }

    /// Every blob key named as being reclaimed.
    async fn reclaiming(&self) -> Result<Vec<String>, SparqError> {
        Ok(Vec::new())
    }

    /// Remove a resource's metadata record. Idempotent: deleting an absent IRI is `Ok(())` (the
    /// caller's existence check governs the 404, so this is a no-op-on-absent at the index layer).
    async fn delete_meta(&self, iri: &str) -> Result<(), SparqError>;

    /// Delete a resource and its parent edge, returning the removed record's blob key.
    ///
    /// Embedded and in-memory clients read the key and remove both index entries atomically.
    /// The default preserves the HTTP client's separate read, detach, and delete operations.
    /// `None` means the resource was absent; an existing parent edge is still removed.
    ///
    /// # Errors
    ///
    /// Returns the backend's read or mutation error without reclaiming any bytes.
    async fn delete_resource(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<Option<String>, SparqError> {
        let key = match self.get_meta(iri).await {
            Ok(meta) => Some(meta.blob_key),
            Err(SparqError::NotFound) => None,
            Err(e) => return Err(e),
        };
        if let Some(parent) = parent {
            self.remove_child(parent, iri).await?;
        }
        self.delete_meta(iri).await?;
        Ok(key)
    }

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
    /// BEFORE this call; the caller reclaims them if the commit fails with a settled outcome.
    async fn create_child(
        &self,
        container: &str,
        child: &str,
        meta: ResourceMeta,
    ) -> Result<(), SparqError>;

    /// Commit a child record and containment together, reporting the replaced record when supported.
    ///
    /// The default calls `create_child` and returns `None` without reading the previous record.
    /// Clients without an override, such as the remote HTTP client, do not report the replaced
    /// record; any superseded blob key is left to the reconciler. The in-memory and embedded
    /// clients return the replaced record in the same index operation.
    async fn replace_child(
        &self,
        container: &str,
        child: &str,
        meta: ResourceMeta,
    ) -> Result<Option<ResourceMeta>, SparqError> {
        self.create_child(container, child, meta).await?;
        Ok(None)
    }

    /// Remove `child` from `container`'s membership. Idempotent on an absent edge.
    async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError>;

    /// List the IRIs of `container`'s direct children (its `ldp:contains` members).
    /// Make the outcome of a write that failed with [`SparqError::Unsettled`] final: once this
    /// succeeds, that write has committed or never will. Idempotent; may be retried until it
    /// succeeds.
    ///
    /// The default refuses: a client whose failed writes are always settled (one in process, or
    /// one that waits for its writes) never reports one unsettled.
    async fn fence(&self, fence: &Fence) -> Result<(), SparqError> {
        let _ = fence;
        Err(SparqError::Backend(
            "this index has no unsettled writes to fence".into(),
        ))
    }

    /// Whether [`raise_marker`](SparqClient::raise_marker) is atomic here. The default is
    /// `false`.
    fn raises_markers(&self) -> bool {
        false
    }

    /// How long one call to this index may take before it is given up (see
    /// `Store::call_timeout`). The default is [`crate::store::STORE_CALL_TIMEOUT`].
    fn call_timeout(&self) -> std::time::Duration {
        crate::store::STORE_CALL_TIMEOUT
    }

    /// Raise the time stored as a marker under `iri` to `value`, unless a later one is stored,
    /// reading and writing in one step under this index's own write lock, and return the time
    /// stored once it is done (see `Store::raise_marker`). The marker is an index record with no
    /// blob, its value in its tag.
    ///
    /// The default refuses: a client whose read and write are separate requests (the HTTP one)
    /// cannot keep a late, lower raise from landing over a later one.
    async fn raise_marker(&self, iri: &str, value: i64) -> Result<i64, SparqError> {
        let _ = (iri, value);
        Err(SparqError::Backend(
            "this index cannot raise a marker atomically".into(),
        ))
    }

    async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError>;

    /// [`list_children`](SparqClient::list_children) while it fits: at most
    /// `max_rows``(container, max_bytes)` members whose IRIs come to at most `max_bytes`;
    /// `None` past either. A backend reads no more of the listing than that, and establishes
    /// that nothing was left out (an exact count, or one row past the bound asked for), so a
    /// listing too large to keep is refused without being held whole. A backend that cannot
    /// bound its read refuses the call.
    async fn list_children_within(
        &self,
        _container: &str,
        _max_bytes: usize,
    ) -> Result<Option<Vec<String>>, SparqError> {
        Err(SparqError::Backend(
            "this backend cannot read a listing within a bound".into(),
        ))
    }

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
    /// Immutable admission ceiling, enforced under the index lock.
    limits: InMemoryStoreLimits,
}

impl Default for InMemorySparqClient {
    fn default() -> Self {
        Self::with_limits(InMemoryStoreLimits::default())
    }
}

/// The content type of a marker ([`SparqClient::raise_marker`]): an index record with no bytes,
/// its value in its tag.
pub(crate) const MARKER_TYPE: &str = "application/x-sparq-marker";

/// The index record of a marker holding `value`.
pub(crate) fn marker_meta(value: &str) -> ResourceMeta {
    ResourceMeta {
        content_type: MARKER_TYPE.to_string(),
        blob_key: String::new(),
        etag: value.to_string(),
        last_modified: None,
    }
}

/// Whether `meta` is a marker's record.
pub(crate) fn is_marker(meta: &ResourceMeta) -> bool {
    meta.content_type == MARKER_TYPE && meta.blob_key.is_empty()
}

/// The time a marker's record holds, when it is one and holds one.
pub(crate) fn marker_time(meta: &ResourceMeta) -> Option<i64> {
    is_marker(meta).then(|| meta.etag.parse().ok()).flatten()
}

/// The metadata records, with how many of them are in the service pool
/// ([`super::in_service_pool`]) kept as they change, so admission can hold the pool to its own
/// ceiling without walking the map.
#[derive(Default)]
struct Meta {
    map: HashMap<String, ResourceMeta>,
    pooled: usize,
    /// Records of holds ([`super::is_hold_record`]), admitted apart from every ceiling.
    holds: usize,
    /// The one notification marker ([`super::is_service_marker`]) holding the slot kept for it,
    /// admitted apart from every ceiling and counted in none: the first stored. Any other IRI of
    /// that shape (another origin's) is an ordinary pool record.
    marker: Option<String>,
}

impl Meta {
    fn get(&self, iri: &str) -> Option<&ResourceMeta> {
        self.map.get(iri)
    }

    fn contains_key(&self, iri: &str) -> bool {
        self.map.contains_key(iri)
    }

    fn len(&self) -> usize {
        self.map.len()
    }

    /// Records in the service pool.
    fn pooled(&self) -> usize {
        self.pooled
    }

    /// Records of holds.
    fn holds(&self) -> usize {
        self.holds
    }

    /// Records in the marker's slot: none or one.
    fn markers(&self) -> usize {
        usize::from(self.marker.is_some())
    }

    /// Whether `iri` is the marker holding its slot, or would take it as a new record: the slot
    /// is taken only by a first insert while no marker holds it, so a record admitted to the pool
    /// stays in the pool, whatever is removed after.
    fn is_marker_slot(&self, iri: &str) -> bool {
        match self.marker.as_deref() {
            Some(m) => m == iri,
            None => super::is_service_marker(iri) && !self.map.contains_key(iri),
        }
    }

    fn values(&self) -> std::collections::hash_map::Values<'_, String, ResourceMeta> {
        self.map.values()
    }

    fn insert(&mut self, iri: String, meta: ResourceMeta) -> Option<ResourceMeta> {
        let marker = self.is_marker_slot(&iri);
        let pooled = super::in_service_pool(&iri) && !marker;
        let hold = super::is_hold_record(&iri);
        if marker && self.marker.is_none() {
            self.marker = Some(iri.clone());
        }
        let replaced = self.map.insert(iri, meta);
        if replaced.is_none() {
            self.pooled += usize::from(pooled);
            self.holds += usize::from(hold);
        }
        replaced
    }

    fn remove(&mut self, iri: &str) -> Option<ResourceMeta> {
        let gone = self.map.remove(iri)?;
        if self.marker.as_deref() == Some(iri) {
            self.marker = None;
        } else if super::in_service_pool(iri) {
            self.pooled -= 1;
        }
        if super::is_hold_record(iri) {
            self.holds -= 1;
        }
        Some(gone)
    }
}

/// The in-memory index state: metadata records + containment membership.
#[derive(Default)]
struct Index {
    meta: Meta,
    /// container IRI → its direct children, kept in insertion order (a `Vec`, de-duplicated).
    children: HashMap<String, Vec<String>>,
    /// The blob keys named as being reclaimed ([`SparqClient::begin_reclaim`]).
    reclaiming: std::collections::BTreeSet<String>,
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
    ///
    /// A record in the service pool ([`super::in_service_pool`]) is admitted against the pool's
    /// own ceiling and counts towards nothing else, so no number of them makes a write of
    /// content fail. The notification marker ([`super::is_service_marker`]) has a slot of its
    /// own, as holds do: however full the pool, it is admitted. The slot is one: the first marker
    /// stored holds it, and no other IRI of that shape is admitted in it.
    fn admit_resource(&self, index: &Index, iri: &str) -> Result<(), SparqError> {
        if index.meta.contains_key(iri)
            || super::is_hold_record(iri)
            || index.meta.is_marker_slot(iri)
        {
            return Ok(());
        }
        let pooled = index.meta.pooled();
        let (held, ceiling) = if super::in_service_pool(iri) {
            (pooled, self.limits.max_pool_resource_count)
        } else {
            (
                index.meta.len() - pooled - index.meta.holds() - index.meta.markers(),
                self.limits.max_resource_count,
            )
        };
        if held >= ceiling {
            return Err(SparqError::QuotaExceeded);
        }
        Ok(())
    }
}

/// The most members a listing of `container` read within `max_bytes` may have: a member lies
/// below its container, so its IRI is longer, and more of them than this cannot fit. A listing
/// with more (members that are not below it included) is refused, never cut short.
pub(crate) fn max_rows(container: &str, max_bytes: usize) -> usize {
    max_bytes / (container.len() + 1)
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

    async fn replace_meta(
        &self,
        iri: &str,
        meta: ResourceMeta,
    ) -> Result<Option<ResourceMeta>, SparqError> {
        // One step under the single lock: the returned record is exactly the one this insert replaced.
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        self.admit_resource(&guard, iri)?;
        Ok(guard.meta.insert(iri.to_string(), meta))
    }

    fn raises_markers(&self) -> bool {
        true
    }

    async fn raise_marker(&self, iri: &str, value: i64) -> Result<i64, SparqError> {
        // Read and written under the one index lock: no other write lands in between.
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        let stored = guard.meta.get(iri).and_then(marker_time);
        let raised = stored.map_or(value, |t| t.max(value));
        if stored != Some(raised) {
            self.admit_resource(&guard, iri)?;
            guard
                .meta
                .insert(iri.to_string(), marker_meta(&raised.to_string()));
        }
        Ok(raised)
    }

    async fn exists(&self, iri: &str) -> Result<bool, SparqError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        Ok(guard.meta.contains_key(iri))
    }

    /// Held in this process's memory: nobody else's.
    fn exclusive(&self) -> Result<(), SparqError> {
        Ok(())
    }

    async fn begin_reclaim(&self, blob_key: &str) -> Result<bool, SparqError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        if guard.meta.values().any(|m| m.blob_key == blob_key) {
            return Ok(false);
        }
        guard.reclaiming.insert(blob_key.to_string());
        Ok(true)
    }

    async fn end_reclaim(&self, blob_key: &str) -> Result<(), SparqError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        guard.reclaiming.remove(blob_key);
        Ok(())
    }

    async fn reclaiming(&self) -> Result<Vec<String>, SparqError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        Ok(guard.reclaiming.iter().cloned().collect())
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

    async fn delete_resource(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<Option<String>, SparqError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        let key = guard.meta.remove(iri).map(|meta| meta.blob_key);
        guard.children.remove(iri);
        if let Some(parent) = parent {
            if let Some(children) = guard.children.get_mut(parent) {
                children.retain(|child| child != iri);
            }
        }
        Ok(key)
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
        self.replace_child(container, child, meta).await.map(|_| ())
    }

    async fn replace_child(
        &self,
        container: &str,
        child: &str,
        meta: ResourceMeta,
    ) -> Result<Option<ResourceMeta>, SparqError> {
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
        let previous = guard.meta.insert(child.to_string(), meta);
        let entry = guard.children.entry(container.to_string()).or_default();
        if !entry.iter().any(|c| c == child) {
            entry.push(child.to_string());
        }
        Ok(previous)
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

    async fn list_children_within(
        &self,
        container: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<String>>, SparqError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| SparqError::Backend("poisoned".into()))?;
        let Some(all) = guard.children.get(container) else {
            return Ok(Some(Vec::new()));
        };
        if all.len() > max_rows(container, max_bytes) {
            return Ok(None);
        }
        let mut out = Vec::new();
        let mut bytes = 0usize;
        for c in all {
            bytes = bytes.saturating_add(c.len());
            if bytes > max_bytes {
                return Ok(None);
            }
            out.push(c.clone());
        }
        Ok(Some(out))
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
    async fn ordinary_delete_returns_the_removed_key_and_detaches_even_an_absent_record() {
        let index = InMemorySparqClient::new();
        index.put_meta("urn:parent", meta("parent")).await.unwrap();
        index
            .create_child("urn:parent", "urn:child", meta("old"))
            .await
            .unwrap();
        index.replace_meta("urn:child", meta("new")).await.unwrap();
        assert_eq!(
            index.delete_resource("urn:child", Some("urn:parent")).await.unwrap(),
            Some("new".into())
        );
        assert!(!index.exists("urn:child").await.unwrap());
        assert!(index.list_children("urn:parent").await.unwrap().is_empty());
        index
            .create_child("urn:parent", "urn:child", meta("again"))
            .await
            .unwrap();
        index.delete_meta("urn:child").await.unwrap();
        assert_eq!(
            index.delete_resource("urn:child", Some("urn:parent")).await.unwrap(),
            None
        );
        assert!(index.list_children("urn:parent").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn resource_limit_rejects_new_records_but_allows_replacement() {
        // Mutation witness: removing `admit_resource` makes the second IRI succeed and
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

    /// The marker's slot is kept for one IRI: the first marker stored. Another of the same shape
    /// (another origin, or with user information) takes room in the pool like any record of it.
    #[tokio::test]
    async fn the_marker_slot_holds_one_marker_alone() {
        let limits = InMemoryStoreLimits::default().with_pool(1, usize::MAX);
        let index = InMemorySparqClient::with_limits(limits);
        let pooled = "http://a.example/.lws/subscriptions/";
        index.put_meta(pooled, meta("")).await.unwrap();
        let marker = "http://a.example/.lws/subscriptions/#high-water";
        assert_eq!(index.raise_marker(marker, 5).await.unwrap(), 5);
        for other in [
            "http://b.example/.lws/subscriptions/#high-water",
            "http://u@a.example/.lws/subscriptions/#high-water",
        ] {
            assert!(
                matches!(
                    index.raise_marker(other, 5).await,
                    Err(SparqError::QuotaExceeded)
                ),
                "{other}"
            );
        }
        assert_eq!(index.raise_marker(marker, 9).await.unwrap(), 9);
        for not in [
            "http://:/.lws/subscriptions/#high-water",
            "http://a b/.lws/subscriptions/#high-water",
            "http:// /.lws/subscriptions/#high-water",
            "http://[garbage]/.lws/subscriptions/#high-water",
            "http://[::1]x/.lws/subscriptions/#high-water",
            "http://a%zz/.lws/subscriptions/#high-water",
        ] {
            assert!(!crate::store::is_service_marker(not), "{not}");
        }
        assert!(crate::store::is_service_marker(
            "https://127.0.0.1:8443/.lws/subscriptions/#high-water"
        ));
        assert!(!crate::store::is_service_marker(
            "http://a.example:x/.lws/subscriptions/#high-water"
        ));
        assert!(crate::store::is_service_marker(
            "http://a.example:3000/.lws/subscriptions/#high-water"
        ));
        assert!(crate::store::is_service_marker(
            "http://[::1]:3000/.lws/subscriptions/#high-water"
        ));
        assert!(!crate::store::is_service_marker(
            "http://u@a.example/.lws/subscriptions/#high-water"
        ));
        assert!(!crate::store::is_service_marker(
            "http://a.example/x/.lws/subscriptions/#high-water"
        ));
    }

    /// A marker admitted to the pool (the slot held by another) stays in it when the slot's
    /// holder is removed and it is written again: the pool's count is exact throughout, so it
    /// frees its room when removed and never wraps.
    #[tokio::test]
    async fn a_pooled_marker_is_never_moved_into_the_slot() {
        let limits = InMemoryStoreLimits::default().with_pool(2, usize::MAX);
        let index = InMemorySparqClient::with_limits(limits);
        let a = "http://a.example/.lws/subscriptions/#high-water";
        let b = "http://b.example/.lws/subscriptions/#high-water";
        let pooled = |index: &InMemorySparqClient| index.inner.lock().unwrap().meta.pooled();
        index.raise_marker(a, 1).await.unwrap();
        index.raise_marker(b, 1).await.unwrap();
        assert_eq!(pooled(&index), 1);
        index.delete_meta(a).await.unwrap();
        index.raise_marker(b, 2).await.unwrap();
        assert_eq!(pooled(&index), 1, "moved into the slot");
        index.delete_meta(b).await.unwrap();
        assert_eq!(pooled(&index), 0);
        // The pool's two places are both free again, and the slot takes a new marker apart.
        for at in ["x", "y"] {
            let iri = format!("http://a.example/.lws/subscriptions/{at}");
            index.put_meta(&iri, meta("")).await.unwrap();
        }
        index.raise_marker(a, 3).await.unwrap();
        assert_eq!(pooled(&index), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_metadata_writes_cannot_over_admit_resource_limit() {
        // Admission and insertion happen under one lock, so racing writers cannot each
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
