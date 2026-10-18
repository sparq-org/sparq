// AUTHORED-BY Claude Opus 4.8
//! The composite [`Store`] — the LDP handler's single view of storage.
//!
//! A [`Store`] reads/writes RDF + metadata via [`SparqClient`] (authoritative) and bytes via
//! [`BlobStore`] (backup), mirroring prod-solid-server's S3+index composite. The default impl,
//! [`CompositeStore`], wires the two seams together; both seams have in-memory test doubles so the
//! whole stack is testable without a running SPARQ or S3.

pub mod blob;
// The blob-body LRU cache (read-4 — backend-read-path.md §3.4): `(blob_key, etag)`-keyed,
// byte-budgeted, sat in front of BlobStore::get inside CompositeStore so a hot unchanged
// resource's repeat read pays zero blob round-trips. Immutable-by-construction (unique-per-write
// blob keys), so a hit can never be stale — see the module docs.
pub mod body_cache;
// Deterministic backend round-trip counters at the SparqClient/BlobStore seams (read-1 of the
// read-path perf plan — `research/lws-design-records.md` §7). Decorators used by the pinned counter
// tests + the bench harness; zero-cost when not wired in.
pub mod counting;
// The in-process embedded SPARQ backend (`embedded-sparq` feature, ON BY DEFAULT — sq-gg0qq.3): the
// FIRST-CLASS `SparqClient` impl now that this crate lives in the sparq workspace. Build with
// `--no-default-features` for the engine-free profile (the in-memory double only). See [`embedded`]
// + `research/lws-design-records.md` §3.
#[cfg(all(feature = "embedded-sparq", not(target_arch = "wasm32")))]
pub mod embedded;
// The live SPARQL-over-HTTP client (opt-in `http-sparq` feature — sq-gg0qq.3): the REMOTE
// shared-service backend, `PSS_SPARQ_BACKEND=http`. Off by default — in-workspace builds bind the
// engine in-process (`embedded` above) instead of paying an HTTP round-trip per index operation.
#[cfg(all(feature = "http-sparq", not(target_arch = "wasm32")))]
pub mod http;
mod limits;
pub mod reconcile;
pub mod sparq;
pub mod sparql;
// SystemTime ↔ `xsd:dateTime` round-trip for the `pss:modified` index timestamp (jx3c). Kept in the
// storage layer (where the timestamp is written + read), dependency-free — see the module doc.
pub mod timestamp;

use async_trait::async_trait;
use bytes::Bytes;
use oxrdf::NamedNode;

/// The lease one running service holds over a backend for as long as it runs: shared by every
/// handle to that backend (see [`Store::lifecycle`]), and held by at most one service at a time,
/// so a second one built over the same backend is refused rather than run with a registry of
/// its own beside the first. Within one process only.
#[derive(Debug, Default)]
pub struct Lifecycle {
    held: std::sync::atomic::AtomicBool,
}

impl Lifecycle {
    /// Take the lease, unless it is held: the guard gives it back when dropped.
    pub(crate) fn acquire(self: &std::sync::Arc<Self>) -> Option<LifecycleGuard> {
        use std::sync::atomic::Ordering;
        self.held
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| LifecycleGuard {
                lease: self.clone(),
            })
    }
}

/// A held [`Lifecycle`] lease: the authority of the one service running over a backend, which
/// alone may lower a marker ([`Store::reset_marker`]). Only this crate makes one.
#[derive(Debug)]
pub struct LifecycleGuard {
    lease: std::sync::Arc<Lifecycle>,
}

impl Drop for LifecycleGuard {
    fn drop(&mut self) {
        self.lease
            .held
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

pub use blob::{BlobEntry, BlobError, BlobStore, InMemoryBlobStore};
pub use body_cache::{BodyCache, DEFAULT_BODY_CACHE_BYTES};
pub use counting::{
    BackendCounters, CounterSnapshot, CountingBlobStore, CountingSparqClient, MeasureScope,
};
#[cfg(all(feature = "embedded-sparq", not(target_arch = "wasm32")))]
pub use embedded::EmbeddedSparqClient;
#[cfg(all(feature = "http-sparq", not(target_arch = "wasm32")))]
pub use http::{HttpSparqClient, SparqHttpError};
pub use limits::{
    in_service_pool, is_hold_record, is_pooled_blob_key, is_service_marker, InMemoryStoreLimits,
    StoreUsage, DEFAULT_IN_MEMORY_MAX_POOL_BYTES, DEFAULT_IN_MEMORY_MAX_POOL_RESOURCE_COUNT,
    DEFAULT_IN_MEMORY_MAX_RESOURCE_COUNT, DEFAULT_IN_MEMORY_MAX_TOTAL_BYTES, POOLED_BLOB_KEY_MARK,
    SERVICE_MARKER_FRAGMENT, SERVICE_POOL_PATH,
};
#[cfg(not(target_arch = "wasm32"))]
pub use reconcile::spawn_periodic;
pub use reconcile::{
    reconcile_orphans, ReconcileError, ReconcileOptions, ReconcileReport, DEFAULT_GRACE,
};
pub use sparq::{
    DeleteOutcome, Fence, InMemorySparqClient, ReadPlan, ResourceMeta, SparqClient, SparqError,
};
pub use sparql::{BodyObject, BuildError};

use crate::error::{ServerError, ServerResult};

/// A failed marker write, as the store reports it.
fn marker_error(e: SparqError) -> ServerError {
    match e {
        SparqError::QuotaExceeded => ServerError::InsufficientStorage,
        SparqError::Unsettled { detail, fence } => ServerError::Unsettled { detail, fence },
        other => ServerError::Storage(format!("{other}")),
    }
}

/// A resource as the LDP handler sees it: bytes + the authoritative metadata.
#[derive(Debug, Clone)]
pub struct Resource {
    pub body: Bytes,
    pub meta: ResourceMeta,
}

/// A container child IRI **validated as RFC-3987 at the [`Store::list_children`] boundary** — the
/// architecturally-correct home for child-IRI validation (bead wg3).
///
/// # Why this newtype exists (the invariant it carries)
/// The container-listing render (`ldp::handler`) serialises each child IRI into a Turtle/N-Triples
/// `<...>` term. Previously it did so on the hot path behind a *cheap structural* guard plus a
/// `debug_assert!` — which left a residual: an invalid-but-serialisable IRI (e.g. a bad percent-escape)
/// could slip into a release build's output. Validating **once, here, at the store boundary** — where a
/// malformed/injected row from storage first crosses into the server's own logic — makes "every child
/// IRI that reaches the render is a full RFC-3987-valid IRI" a **type-level invariant**: the render
/// receives `ValidatedChildIri` values and can construct the RDF term with NO per-child re-parse and NO
/// structural guard. A malformed row is FAIL-CLOSED **omitted** at the boundary (see
/// [`ValidatedChildIri::parse`] / the [`CompositeStore`] impl), so it never flows unchecked into a
/// response.
///
/// It wraps a validated [`NamedNode`], so a consumer that needs the RDF term (the render) gets it for
/// free (`as_named_node`/`into_named_node`) without re-parsing, and one that needs the string form uses
/// `as_str`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedChildIri(NamedNode);

impl ValidatedChildIri {
    /// Validate a raw child IRI (full RFC-3987 via oxrdf/oxiri's `NamedNode::new`). Returns `None` for a
    /// malformed IRI, so the caller can FAIL CLOSED (omit it) rather than let it flow unchecked.
    pub fn parse(raw: &str) -> Option<Self> {
        NamedNode::new(raw).ok().map(Self)
    }

    /// The validated IRI as a string slice.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the validated [`NamedNode`] (for building an RDF term without a re-parse).
    pub fn as_named_node(&self) -> &NamedNode {
        &self.0
    }

    /// Consume into the validated [`NamedNode`] (moved into an RDF term on the render path).
    pub fn into_named_node(self) -> NamedNode {
        self.0
    }
}

/// The composite storage seam used by the LDP handlers.
///
/// M1 covered the single-resource GET/HEAD/PUT path. M2 adds DELETE, containment (POST mints a child
/// + records membership; an empty-container check governs DELETE), and the metadata needed for the
/// conditional-write ETag CAS (the [`ResourceMeta::etag`] the handler compares).
///
/// Next: the reconciler that GCs orphaned bytes/index rows after a crash between the byte and index
/// writes. Container delete is supported for EMPTY containers — the handler refuses a non-empty one
/// with 409 (the conservative spec choice); an opt-in recursive/cascade delete is intentionally not
/// offered yet.
///
/// # Consistency: read your writes
///
/// Once a mutating call (`write`, `create_in_container`, `restore`, `delete`,
/// `delete_container_if_empty`) has completed, every later read of current state
/// ([`read`](Store::read), [`meta`](Store::meta), [`exists`](Store::exists),
/// [`list_children`](Store::list_children)) must see what it committed. ([`read_at`](Store::read_at)
/// is not a read of current state: it serves the version earlier metadata names.) A call has
/// completed when it returns successfully, or returns an error after which nothing it started
/// can still commit. An error does not always mean that: an HTTP update that times out can keep
/// running on the server and commit later. After such an error the write's outcome is unknown,
/// and a read that does not see it is not final.
///
/// The LWS service containers (access grants and requests, subscriptions) rely on this: when a
/// create reports a failure, the server reads the container listing to tell whether the record
/// was stored anyway, answering `201` if it is listed and `500` (releasing the client's quota) if
/// it is not. That answer is exact only when the failed create had completed; one that failed
/// [unsettled](crate::error::ServerError::Unsettled) is fenced first ([`Store::fence`]). The in-memory and embedded backends read their own writes, as does one
/// `sparq-server` behind the `http-sparq` client. A backend that cannot (an eventually
/// consistent or replicated remote store, whose listing can lag a completed write) must not be
/// used for an LWS server.
#[async_trait]
pub trait Store: Send + Sync {
    /// Read a resource by IRI: its authoritative metadata (SPARQ) + its bytes (blob store).
    async fn read(&self, iri: &str) -> ServerResult<Resource>;

    /// Fetch just the authoritative metadata for a resource IRI (no bytes), or `None` if absent.
    ///
    /// Used by the conditional-request path to learn the current ETag without paying for the body.
    async fn meta(&self, iri: &str) -> ServerResult<Option<ResourceMeta>>;

    /// Whether a resource exists (the authoritative SPARQ existence check — never an S3 HEAD).
    async fn exists(&self, iri: &str) -> ServerResult<bool>;

    /// Create-or-replace a resource: write the bytes, then the authoritative metadata.
    async fn write(&self, iri: &str, body: Bytes, content_type: &str)
        -> ServerResult<ResourceMeta>;

    /// Create a resource AND record it as a child of `container` (the POST containment path). The
    /// `child` IRI is the server-minted target. Returns the new resource's metadata.
    async fn create_in_container(
        &self,
        container: &str,
        child: &str,
        body: Bytes,
        content_type: &str,
    ) -> ServerResult<ResourceMeta>;

    /// Put a record back as it was: `body` under `meta`'s content type, entity tag and
    /// modification time (its blob key is minted afresh), as a member of `container` when one is
    /// given (the [`create_in_container`](Store::create_in_container) path) and in place otherwise
    /// (the [`write`](Store::write) path). This is how a mutation that fails partway is undone
    /// without changing what it reports as unchanged: a restored resource keeps its validators.
    ///
    /// The default refuses: a store that cannot keep a record's validators cannot undo a change
    /// faithfully, and says so rather than restoring something that looks newer.
    async fn restore(
        &self,
        iri: &str,
        container: Option<&str>,
        body: Bytes,
        meta: &ResourceMeta,
    ) -> ServerResult<ResourceMeta> {
        let _ = (iri, container, body, meta);
        Err(ServerError::Storage(
            "this store cannot restore a record as it was".into(),
        ))
    }

    /// Hold, for `holder`, the bytes `iri`'s record points at now: until `holder`
    /// [`release`](Store::release)s them, the store keeps them, whatever later rewrites or removes
    /// the record, so the record can be put back as it was ([`reinstate`](Store::reinstate))
    /// without its bytes being copied or read. Returns the record's metadata (its blob key names
    /// the held bytes), or `None` when `iri` has no record. Each holder's hold is its own, so one
    /// holder's release never ends another's, and the bytes go only once none holds them,
    /// whichever instance over the same backends took the holds. A hold is itself stored, so it
    /// outlives the process: a process that starts over the store finds the holds a stop left
    /// with [`holds`](Store::holds), each with its holder.
    ///
    /// The default refuses: a store that cannot keep bytes past their record cannot put a record
    /// back by reference.
    async fn hold(&self, iri: &str, holder: &str) -> ServerResult<Option<ResourceMeta>> {
        let _ = (iri, holder);
        Err(ServerError::Storage(
            "this store cannot hold a record's bytes".into(),
        ))
    }

    /// Put `iri`'s record back as `held` says, pointing at the bytes a [`hold`](Store::hold)
    /// kept, validators included: as a member of `container` when one is given (the
    /// [`create_in_container`](Store::create_in_container) path), in place otherwise (the
    /// [`write`](Store::write) path). The hold stays until released. A record that is put back
    /// this way may be removed or rewritten again, and put back again, as long as the hold stands.
    ///
    /// The default refuses (see [`hold`](Store::hold)).
    async fn reinstate(
        &self,
        iri: &str,
        container: Option<&str>,
        held: &ResourceMeta,
    ) -> ServerResult<ResourceMeta> {
        let _ = (iri, container, held);
        Err(ServerError::Storage(
            "this store cannot hold a record's bytes".into(),
        ))
    }

    /// End `holder`'s [`hold`](Store::hold): the held bytes go now unless a record still names
    /// them (the record was put back on them, or another hold stands). Idempotent.
    async fn release(&self, held: &ResourceMeta, holder: &str) -> ServerResult<()> {
        let _ = (held, holder);
        Err(ServerError::Storage(
            "this store cannot hold a record's bytes".into(),
        ))
    }

    /// Every hold that stands ([`hold`](Store::hold)), whichever storage's: its holder, and what
    /// it holds (its blob key), so a process that starts over the store can release those of its
    /// own that nothing needs any longer. The container of a key no hold is left on (a release
    /// cut short between its two steps) is removed as it is found.
    async fn holds(&self) -> ServerResult<Vec<(String, ResourceMeta)>> {
        Err(ServerError::Storage(
            "this store cannot hold a record's bytes".into(),
        ))
    }

    /// Finish every reclaim a stop cut short: bytes the index names as being reclaimed (no record
    /// can name them again) are deleted, and the name withdrawn. The default does nothing.
    async fn finish_reclaims(&self) -> ServerResult<()> {
        Ok(())
    }

    /// Delete a resource: remove its index record + its bytes, and detach it from `parent`'s
    /// containment (if `parent` is given). The caller is responsible for the existence (404) and
    /// empty-container (409) decisions; this performs the removal.
    ///
    /// This is the NON-container delete path. A CONTAINER delete must instead go through
    /// [`delete_container_if_empty`](Store::delete_container_if_empty), which folds the empty-check
    /// into the delete atomically.
    async fn delete(&self, iri: &str, parent: Option<&str>) -> ServerResult<()>;

    /// ATOMICALLY delete a container ONLY if it is empty (the container-DELETE path).
    ///
    /// The membership check (`ldp:contains` empty?), the record delete, AND the detach of the
    /// container's edge in `parent`'s containment graph are performed as ONE store operation with NO
    /// interleaving, so neither (a) a child POSTed concurrently can slip between an empty-check and the
    /// delete and be orphaned under a deleted container (the TOCTOU the separate `list_children` +
    /// `delete` had), NOR (b) a concurrent POST can recreate the child under the parent in a window
    /// between the graph delete and a separate parent-edge detach and then be orphaned by that stale
    /// detach. Once that index delete has committed, the container's own bytes are reclaimed inline,
    /// best effort (see [`CompositeStore`]'s `delete_container_if_empty`): blob keys are minted UNIQUE
    /// per write, so a concurrent same-IRI recreate writes under a DIFFERENT key and the inline delete
    /// can never clobber it. If that blob delete fails, the bytes stay as an orphan for the
    /// reconciler's sweep — space only, never an observable inconsistency. Returns:
    /// - [`DeleteOutcome::Deleted`] — it existed, was empty, and is gone;
    /// - [`DeleteOutcome::NotEmpty`] — it existed with members; NOTHING was deleted (⇒ 409);
    /// - [`DeleteOutcome::NotFound`] — it did not exist (⇒ 404).
    async fn delete_container_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> ServerResult<DeleteOutcome>;

    /// List the direct children of a container — the authoritative `ldp:contains` membership — each as a
    /// [`ValidatedChildIri`] (RFC-3987-validated at THIS boundary, so a malformed/injected row never
    /// flows unchecked into the container-listing render; see [`ValidatedChildIri`]).
    ///
    /// SCOPE (load-bearing): this method is for the container-listing **render ONLY**. Because a
    /// malformed stored membership row is FAIL-CLOSED OMITTED here, this list is NOT authoritative for
    /// **emptiness** — a container with a (malformed) membership edge would appear shorter/empty in
    /// THIS filtered view. The empty-container DELETE decision therefore does NOT use this method: it
    /// goes through [`delete_container_if_empty`](Store::delete_container_if_empty) →
    /// `SparqClient::delete_meta_if_empty`, which counts ALL raw membership edges at the SPARQ level
    /// (atomically, with no filtering), so a malformed edge still keeps the container non-empty and it
    /// is NOT deleted. Do NOT introduce an emptiness check over this filtered list — use the atomic
    /// path.
    ///
    /// The listing must reflect every write that has completed (see the trait's "read your
    /// writes" section): the LWS service containers read it to settle a create that reported a
    /// failure.
    async fn list_children(&self, container: &str) -> ServerResult<Vec<ValidatedChildIri>>;

    /// Make the outcome of a write that failed with [`ServerError::Unsettled`] final: once this
    /// succeeds, that write has committed or never will, and a read tells which (and no longer
    /// changes). Idempotent: retried until it succeeds by whoever must know the outcome.
    ///
    /// The default refuses: a store whose failed writes are always settled never reports one
    /// unsettled.
    async fn fence(&self, fence: &Fence) -> ServerResult<()> {
        let _ = fence;
        Err(ServerError::Storage(
            "this store has no unsettled writes to fence".into(),
        ))
    }

    /// The value last stored under `iri` by [`raise_marker`](Store::raise_marker), as written,
    /// or `None` when none is. A record under `iri` that is not a marker reads as an empty value.
    ///
    /// The default refuses, as [`raise_marker`](Store::raise_marker) does.
    async fn read_marker(&self, iri: &str) -> ServerResult<Option<String>> {
        let _ = iri;
        Err(ServerError::Storage("this store keeps no markers".into()))
    }

    /// Whether [`raise_marker`](Store::raise_marker) is atomic here. The default is `false`.
    fn raises_markers(&self) -> bool {
        false
    }

    /// The lease every handle to this backend shares, when it keeps one: a service built over
    /// it takes the lease for as long as it runs. The default is none (each service built over
    /// such a store takes a lease of its own).
    fn lifecycle(&self) -> Option<std::sync::Arc<Lifecycle>> {
        None
    }

    /// Raise the time stored under `iri` to `value`, unless a later one is stored, in one
    /// indivisible step under the index's own write lock, and return the time stored once it is
    /// done. A raise never lowers what is stored, whatever order raises land in: one that fails
    /// and lands late is a no-op once a later time is stored. With nothing stored under `iri`,
    /// or nothing that reads as a time, `value` is stored. The value is kept in the index alone,
    /// with no bytes, so it takes no blob capacity.
    ///
    /// The default refuses: a store that cannot raise atomically says so.
    async fn raise_marker(&self, iri: &str, value: i64) -> ServerResult<i64> {
        let _ = (iri, value);
        Err(ServerError::Storage(
            "this store cannot raise a marker atomically".into(),
        ))
    }

    /// Store `value` under `iri` as a marker in one step, whatever is stored there, earlier or
    /// later: the one write that may lower a marker, for a start that finds the stored one
    /// unusable and has removed what it decided ([`raise_marker`](Store::raise_marker) keeps
    /// every other write from lowering it). Only the service holding the backend's
    /// [`Lifecycle`] lease may make it: `lease` is its guard, which no caller outside this crate
    /// can make.
    ///
    /// The default refuses, as [`raise_marker`](Store::raise_marker) does.
    async fn reset_marker(
        &self,
        iri: &str,
        value: i64,
        lease: &LifecycleGuard,
    ) -> ServerResult<()> {
        let _ = (iri, value, lease);
        Err(ServerError::Storage(
            "this store cannot raise a marker atomically".into(),
        ))
    }

    /// [`list_children`](Store::list_children) while it fits (see
    /// [`SparqClient::list_children_within`]): `None` when it does not, without the whole
    /// listing having been held. A store that cannot bound its read refuses the call.
    async fn list_children_within(
        &self,
        _container: &str,
        _max_bytes: usize,
    ) -> ServerResult<Option<Vec<ValidatedChildIri>>> {
        Err(ServerError::Storage(
            "this store cannot read a listing within a bound".into(),
        ))
    }

    /// ONE combined read-plan lookup for the read path (read-2 — `research/lws-design-records.md`
    /// §7): the target's authoritative metadata + the presence/etag of every ACL candidate, in a
    /// single index round-trip. See [`SparqClient::read_plan`] for the contract (candidate
    /// ordering, the two IRI roles, fail-closed on any backend error).
    ///
    /// The DEFAULT implementation loops [`meta`](Store::meta) (one round-trip per IRI) so every
    /// [`Store`] impl — including the handler-level test doubles — keeps its exact per-IRI
    /// error/presence semantics unchanged; [`CompositeStore`] overrides it to delegate to the
    /// [`SparqClient`] seam, where the live client answers it in ONE combined query.
    async fn read_plan(&self, target: &str, acl_candidates: &[String]) -> ServerResult<ReadPlan> {
        let target_meta = self.meta(target).await?;
        let mut acls = Vec::with_capacity(acl_candidates.len());
        for candidate in acl_candidates {
            let etag = self.meta(candidate).await?.map(|m| m.etag);
            acls.push((candidate.clone(), etag));
        }
        Ok(ReadPlan {
            target: target_meta,
            acls,
        })
    }

    /// Fetch a resource's bytes through ALREADY-HELD authoritative metadata (from THIS request's
    /// [`read_plan`](Store::read_plan) round) — the §3.3 `read_at`: no second `get_meta` on the
    /// common path. Blob keys are minted UNIQUE PER WRITE (`CompositeStore::mint_blob_key`), so
    /// bytes fetched through a held pointer are exactly the bytes that pointer committed with.
    ///
    /// `read_at` serves ONLY the version `meta` names — the one the caller authorized. The held
    /// pointer can go stale: a concurrent rewrite or delete commits and the store reclaims the
    /// superseded blob. Then:
    /// - the resource is gone ⇒ [`ServerError::NotFound`];
    /// - a DIFFERENT version is current ⇒ [`ServerError::ResourceChanged`]. Its bytes are NOT
    ///   returned: the caller authorized the old version, and a rewrite can change the governing
    ///   ACL along with the body (public → private). The caller must restart its read,
    ///   authorization included (see [`READ_RACE_RETRIES`]).
    ///
    /// The DEFAULT implementation re-reads via [`read`](Store::read) (metadata + bytes — the
    /// pre-read-2 cost) and applies the same rule by etag, so non-composite [`Store`] impls (test
    /// doubles) never substitute a newer version either; [`CompositeStore`] overrides it with the
    /// direct blob fetch.
    async fn read_at(&self, iri: &str, meta: &ResourceMeta) -> ServerResult<Bytes> {
        let current = self.read(iri).await?;
        if current.meta.etag != meta.etag {
            return Err(ServerError::ResourceChanged);
        }
        Ok(current.body)
    }
}

/// How many times a read starts over after finding its blob reclaimed by a concurrent rewrite/delete:
/// [`Store::read`] re-reads the metadata (see `CompositeStore::read_through`), and a planned read
/// that gets [`ServerError::ResourceChanged`] from [`Store::read_at`] restarts its read plan AND its
/// authorization. Each retry needs ANOTHER commit to land in the window between its metadata read
/// and its blob fetch, so a small bound suffices.
pub const READ_RACE_RETRIES: usize = 3;

/// The default [`Store`]: SPARQ (authoritative metadata) + a blob store (backup bytes), with a
/// [`BodyCache`] (read-4 — `backend-read-path.md` §3.4) in front of the blob byte-fetch.
pub struct CompositeStore<S: SparqClient, B: BlobStore> {
    sparq: S,
    blob: B,
    /// The `(blob_key, etag)`-keyed blob-body LRU (read-4). Every lookup is keyed with THIS
    /// request's authoritative index metadata, so a hit is provably the bytes that metadata
    /// committed with — never stale, never an authz surface (see [`body_cache`]'s module docs).
    body_cache: BodyCache,
    /// The holds ([`Store::hold`]) taken through this instance and not yet released, counted by
    /// the blob key they hold: bytes this names are held, without asking the index. Bytes it does
    /// not name may still be held, through another instance over the same backends, so whether
    /// they are is then asked of the index (see [`CompositeStore::reclaim_blob`]).
    holding: std::sync::Mutex<std::collections::HashMap<String, usize>>,
}

/// The container every hold's record is a member of ([`Store::hold`]): outside every storage (a
/// `urn:`), so no request can name it and no listing shows it.
pub const HELD_CONTAINER: &str = "urn:x-sparq-lws:held/";

/// The media type a hold's records are stored with.
const HELD_TYPE: &str = "application/x-sparq-lws-held";

fn sha_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The container of the holds on the bytes under `blob_key`: there while any stands, so whether
/// they are held is one lookup.
fn held_dir(blob_key: &str) -> String {
    format!("{HELD_CONTAINER}{}/", sha_hex(blob_key))
}

/// The record of `holder`'s hold on the bytes under `blob_key`: one per holder and key, so a
/// holder only ever withdraws its own.
fn held_record(blob_key: &str, holder: &str) -> String {
    format!("{}{}", held_dir(blob_key), sha_hex(holder))
}

impl<S: SparqClient, B: BlobStore> CompositeStore<S, B> {
    /// Build with the DEFAULT-ON body cache ([`DEFAULT_BODY_CACHE_BYTES`] budget) — mirroring the
    /// default-on `AclCache`. Use [`with_body_cache`](Self::with_body_cache) to size or disable it
    /// (`BodyCache::disabled()` ⇒ byte-identical pre-cache behaviour).
    pub fn new(sparq: S, blob: B) -> Self {
        Self::with_body_cache(sparq, blob, BodyCache::new(DEFAULT_BODY_CACHE_BYTES))
    }

    /// Build with an explicitly-configured [`BodyCache`] (the `SOLID_SERVER_BODY_CACHE_BYTES` /
    /// `SOLID_SERVER_BODY_CACHE_MAX_ENTRY_BYTES` boot wiring, or a disabled sentinel in tests).
    pub fn with_body_cache(sparq: S, blob: B, body_cache: BodyCache) -> Self {
        Self {
            sparq,
            blob,
            body_cache,
            holding: Default::default(),
        }
    }

    /// Fetch a resource's bytes through its AUTHORITATIVE metadata: body-cache first (keyed by this
    /// request's `(blob_key, etag)` — read-4), then the blob store on a miss (inserting the fetched
    /// bytes so the next same-version read hits). A HIT skips only the blob round-trip (blob-gets/op
    /// 1 → 0), never a decision — authorization has already run upstream of every caller (see
    /// [`body_cache`]'s no-bypass argument). `Ok(None)` means the blob is absent — see
    /// [`read_through`](Self::read_through) for why that is usually a benign race.
    async fn try_fetch_body(&self, meta: &ResourceMeta) -> ServerResult<Option<Bytes>> {
        if let Some(body) = self.body_cache.get(&meta.blob_key, &meta.etag) {
            return Ok(Some(body));
        }
        let body = match self.blob.get(&meta.blob_key).await {
            Ok(body) => body,
            Err(BlobError::NotFound) => return Ok(None),
            Err(BlobError::QuotaExceeded) => return Err(ServerError::InsufficientStorage),
            Err(BlobError::Backend(msg)) => return Err(ServerError::Storage(msg)),
        };
        self.body_cache.insert(&meta.blob_key, &meta.etag, &body);
        Ok(Some(body))
    }

    /// Resolve `iri`'s bytes starting from a just-read metadata row, tolerating the reclaim race.
    ///
    /// A rewrite or delete reclaims the superseded blob right after its index commit. A reader that
    /// read the metadata just BEFORE that commit can therefore find its blob gone. That is not an
    /// inconsistency: the index has moved on. So on a missing blob this re-reads the metadata and
    /// retries through the CURRENT pointer, at most [`READ_RACE_RETRIES`] times, and returns the
    /// metadata the bytes were actually read through so the caller's validators match the body. A
    /// resource deleted meanwhile is [`ServerError::NotFound`]. A blob missing under a pointer the
    /// index STILL holds (the re-read returns the same key) is a genuine byte/index inconsistency.
    ///
    /// Serving the CURRENT version is correct here only because [`Store::read`] holds no prior
    /// version: it promises "the latest committed version", not a version a caller authorized.
    /// [`Store::read_at`] — which does hold one — never comes through here (see its doc).
    async fn read_through(&self, iri: &str, mut meta: ResourceMeta) -> ServerResult<Resource> {
        let mut retries = 0;
        loop {
            if let Some(body) = self.try_fetch_body(&meta).await? {
                return Ok(Resource { body, meta });
            }
            if retries == READ_RACE_RETRIES {
                // The resource kept being rewritten under us: transient, so retryable (503).
                return Err(ServerError::ResourceChanged);
            }
            retries += 1;
            let current = self.get_meta_mapped(iri).await?;
            if current.blob_key == meta.blob_key {
                // The index still points at the missing blob: not a race, a real inconsistency.
                return Err(ServerError::Storage("byte/index inconsistency".into()));
            }
            meta = current;
        }
    }

    /// [`SparqClient::get_meta`] with the read path's error mapping (absent ⇒ [`ServerError::NotFound`]).
    async fn get_meta_mapped(&self, iri: &str) -> ServerResult<ResourceMeta> {
        self.sparq.get_meta(iri).await.map_err(map_sparq)
    }

    /// Reclaim a blob no record names any longer, best effort.
    ///
    /// Called only AFTER the index commit that dropped the reference has succeeded, so a crash before
    /// this point leaves an unreferenced orphan (the reconciler's job), never an index row pointing at
    /// missing bytes. A failed delete is ignored: the bytes stay as an orphan for the reconciler.
    ///
    /// Two steps, the index the one arbiter whatever instance acts over it. First the index names
    /// the bytes as being reclaimed, in one atomic step that finds no record naming them
    /// ([`SparqClient::begin_reclaim`]): a resource's record (the bytes were put back), or a
    /// hold's ([`Store::hold`]: every hold is a record naming them). Keys are minted per write,
    /// and only a holder, while its own hold's record names them, puts a record back on them; a
    /// hold is taken by committing its record, then checking that the resource still names the
    /// bytes. So once named, no record ever names them again: no hold can then find them current,
    /// and none is left to put them back. Only then are they deleted, and the name withdrawn; a
    /// reclaim cut short in between is finished at start ([`Store::finish_reclaims`]). A count of
    /// the holds taken through this instance may say they are held without asking (never that
    /// they are not).
    async fn reclaim_blob(&self, key: &str) {
        if self.holds_locally(key) || !matches!(self.sparq.begin_reclaim(key).await, Ok(true)) {
            return;
        }
        match self.blob.delete(key).await {
            Ok(()) | Err(BlobError::NotFound) => {
                let _ = self.sparq.end_reclaim(key).await;
            }
            Err(_) => {}
        }
    }

    /// Whether a hold taken through this instance on the bytes under `key` stands.
    fn holds_locally(&self, key: &str) -> bool {
        self.holding
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(key)
    }

    /// Count a hold taken through this instance (`up`), or uncount one released.
    fn count_hold(&self, key: &str, up: bool) {
        let mut holding = self.holding.lock().unwrap_or_else(|e| e.into_inner());
        if up {
            *holding.entry(key.to_string()).or_default() += 1;
        } else if let Some(n) = holding.get_mut(key) {
            *n -= 1;
            if *n == 0 {
                holding.remove(key);
            }
        }
    }

    /// Commit `holder`'s record of a hold on `key`, creating the holds' containers when absent.
    async fn publish_hold(&self, key: &str, holder: &str) -> ServerResult<()> {
        let record = |blob_key: &str, etag: &str| ResourceMeta {
            content_type: HELD_TYPE.to_string(),
            blob_key: blob_key.to_string(),
            // The holder, so a start can tell whose it is (see `Store::holds`).
            etag: etag.to_string(),
            last_modified: None,
        };
        let dir = held_dir(key);
        for _ in 0..3 {
            match self
                .sparq
                .create_child(&dir, &held_record(key, holder), record(key, holder))
                .await
            {
                Ok(()) => return Ok(()),
                Err(SparqError::NotFound) => {}
                Err(e) => return Err(map_sparq(e)),
            }
            // The key's container is absent (no hold stood, or the last one's release removed
            // it meanwhile): created, under the holds' container, created first if need be.
            match self
                .sparq
                // It names no bytes: only its records keep them from the orphan sweep.
                .create_child(HELD_CONTAINER, &dir, record("", ""))
                .await
            {
                Ok(()) => {}
                Err(SparqError::NotFound) => {
                    self.write(HELD_CONTAINER, Bytes::new(), HELD_TYPE).await?;
                }
                Err(e) => return Err(map_sparq(e)),
            }
        }
        Err(ServerError::Storage(
            "the holds' container could not be created".into(),
        ))
    }

    /// Remove `holder`'s record of a hold on `key` (absent is fine), and the key's container once
    /// no hold is left in it.
    async fn withdraw_hold(&self, key: &str, holder: &str) -> ServerResult<()> {
        let dir = held_dir(key);
        self.sparq
            .delete_meta_if_empty(&held_record(key, holder), Some(&dir))
            .await
            .map_err(map_sparq)?;
        // Removed only while empty: a hold published meanwhile keeps it (or recreates it).
        self.sparq
            .delete_meta_if_empty(&dir, Some(HELD_CONTAINER))
            .await
            .map_err(map_sparq)?;
        Ok(())
    }

    /// Mint a fresh, **unique-per-write** opaque blob-store key for an IRI.
    ///
    /// # The root fix: unique keys retire the deterministic-key race class
    /// The earlier `blob_key_for` derived the key DETERMINISTICALLY from the IRI (a percent-flatten),
    /// so every write to the same logical resource REUSED the same blob key. That single design choice
    /// was the source of a whole race class: two concurrent writes to the same IRI collided on one key
    /// (the loser's bytes could interleave with / clobber the winner's), and a delete's inline
    /// `blob.delete(key)` raced a concurrent same-IRI recreate that had just rewritten the SAME key —
    /// forcing the elaborate `generation`-CAS + grace-window + snapshot-threading machinery in
    /// [`super::blob`] and [`super::reconcile`] to exist purely to make a reused key safe.
    ///
    /// This mints a NEW key on EVERY write: an IRI-derived prefix (kept for operator-debuggability — a
    /// key still traces back to its resource) plus a hyphen-joined **128-bit cryptographically-random
    /// suffix** from the OS RNG. So:
    ///
    /// - **Two concurrent writes to the same IRI get DIFFERENT keys** — they write to disjoint blob
    ///   objects and can never collide or interleave. The "latest committed" winner is decided solely by
    ///   which write's index `put_meta` commits last (SPARQ is authoritative); whichever metadata pointer
    ///   wins, a read resolves through it to that write's own bytes. The superseded key is reclaimed
    ///   inline after the index commit (best effort; a concurrent loser's key the reclaim cannot see
    ///   stays as an orphan for the reconciler) — never a clobber of live bytes.
    /// - **A delete's inline blob delete can no longer race a recreate.** A recreate mints a brand-new
    ///   key, so deleting the OLD key's bytes can never touch the recreate's bytes. (The collision the
    ///   `generation`-CAS existed to catch simply cannot arise once keys are never reused.)
    ///
    /// The metadata pointer ([`ResourceMeta::blob_key`]) records the minted key, and every read already
    /// resolves bytes THROUGH that pointer (`read` does `get(&meta.blob_key)`), so correctness
    /// (read-after-write returns the latest committed blob) is preserved unchanged — the only difference
    /// is that the pointer now names a unique object rather than a shared one.
    ///
    /// 128 bits of OS entropy makes a key collision across independent writes cryptographically
    /// negligible; the IRI prefix is cosmetic (debuggability) and carries no uniqueness requirement.
    ///
    /// # Why this FAILS CLOSED on RNG failure (the Medium fix)
    /// Minting is **fallible**: if the OS CSPRNG ([`getrandom`]) cannot fill the 128-bit suffix, this
    /// returns a [`ServerError::Storage`] and the write is failed — it NEVER mints a key from a
    /// weak-entropy fallback. The earlier code fell back to a timestamp-only suffix, but the wall clock is
    /// NOT unique per call: two concurrent mints in the same clock tick would derive the SAME suffix and
    /// thus the SAME key — reintroducing the exact collision class the unique-per-write keys exist to
    /// eliminate. `getrandom` does not fail on any platform we target (it reads `getrandom(2)`/`/dev/urandom`
    /// on Linux, `getentropy` on the BSDs/macOS, `BCryptGenRandom` on Windows), so failing the write on the
    /// theoretical error is correct: a genuinely unavailable OS RNG is an environment fault, and failing
    /// closed (no key minted) is strictly safer than minting a key that could collide.
    fn mint_blob_key(iri: &str) -> ServerResult<String> {
        // The IRI-derived prefix is for human/operator traceability only; uniqueness comes entirely from
        // the random suffix, so the prefix need not be collision-free.
        let prefix = iri.replace([':', '/', '?', '#', '%'], "_");
        let mut suffix = [0u8; 16];
        // The OS CSPRNG. `getrandom` is the de-facto OS-entropy source and does not fail on any platform
        // we target. On the (effectively unreachable) error we FAIL CLOSED — the write errors rather than
        // minting a key from non-unique fallback entropy that could collide with a concurrent same-tick
        // mint. There is no safe "still-unique" fallback that does not require either a CSPRNG or a
        // process-wide monotonic counter, and failing the write is the correct, simplest response to a
        // missing OS RNG.
        getrandom::getrandom(&mut suffix).map_err(|e| {
            ServerError::Storage(format!("OS RNG unavailable for blob-key minting: {e}"))
        })?;
        let hex = suffix.iter().fold(String::with_capacity(32), |mut acc, b| {
            use std::fmt::Write as _;
            let _ = write!(acc, "{b:02x}");
            acc
        });
        // A record in the service pool ([`in_service_pool`]) is marked here, where the IRI says
        // which it is: the blob store admits it against the pool's ceilings and never against
        // stored content's, and it has only the key to go on.
        if limits::in_service_pool(iri) {
            return Ok(format!("{}{prefix}-{hex}", limits::POOLED_BLOB_KEY_MARK));
        }
        Ok(format!("{prefix}-{hex}"))
    }

    /// A deterministic strong ETag for a stored representation: its byte length and an FNV-1a hash
    /// over the media type, a `0xFF` separator (never part of a header value) and the bytes.
    ///
    /// The media type is part of the representation (RFC 9110 §8.8.3), so re-storing the same bytes
    /// under a different `Content-Type` must change the validator: otherwise a cache or an `If-Match`
    /// holder of the old tag would treat the relabelled resource as unchanged. M2: derive it from the
    /// SPARQ index state so it participates in the conditional-request CAS (If-None-Match/If-Match).
    fn etag_for(content_type: &str, body: &Bytes) -> String {
        let hash = fnv1a_chain(
            content_type
                .as_bytes()
                .iter()
                .chain(std::iter::once(&0xFF))
                .chain(body.iter()),
        );
        format!("\"{}-{}\"", body.len(), hash)
    }
}

// The concrete wasm/dev-store pair exposes one combined limits and usage view without
// widening the production backend traits, whose remote stores have backend-specific quota models.
impl CompositeStore<InMemorySparqClient, InMemoryBlobStore> {
    /// Store `value` under `iri` as a marker as it is, whatever is stored there: what a test
    /// plants to stand for an earlier, or a damaged, one.
    #[cfg(test)]
    pub(crate) async fn plant_marker(&self, iri: &str, value: &str) {
        self.sparq
            .put_meta(iri, sparq::marker_meta(value))
            .await
            .expect("planted");
    }

    /// Build a bounded in-memory store with the same limits enforced at both storage seams.
    pub fn in_memory_with_limits(limits: InMemoryStoreLimits) -> Self {
        Self::new(
            InMemorySparqClient::with_limits(limits),
            InMemoryBlobStore::with_limits(limits),
        )
    }

    /// Return physical byte usage and the fuller storage map's occupied-entry count.
    pub fn usage(&self) -> ServerResult<StoreUsage> {
        let blob = self.blob.usage().map_err(|error| match error {
            BlobError::NotFound => ServerError::Storage("blob usage lookup failed".into()),
            BlobError::QuotaExceeded => ServerError::InsufficientStorage,
            BlobError::Backend(message) => ServerError::Storage(message),
        })?;
        let indexed = self.sparq.usage().map_err(|error| match error {
            SparqError::NotFound => ServerError::Storage("index usage lookup failed".into()),
            SparqError::QuotaExceeded => ServerError::InsufficientStorage,
            SparqError::Backend(message) => ServerError::Storage(message),
            SparqError::Unsettled { detail, fence } => ServerError::Unsettled { detail, fence },
        })?;
        Ok(StoreUsage {
            total_bytes: blob.total_bytes,
            resource_count: blob.resource_count.max(indexed),
        })
    }

    /// Return the effective limits; mismatched manually-constructed backends report the tighter cap.
    pub fn quota(&self) -> InMemoryStoreLimits {
        let blob = self.blob.quota();
        let index = self.sparq.quota();
        InMemoryStoreLimits::new(
            blob.max_total_bytes,
            blob.max_resource_count.min(index.max_resource_count),
        )
    }
}

#[async_trait]
impl<S: SparqClient, B: BlobStore> Store for CompositeStore<S, B> {
    async fn read(&self, iri: &str) -> ServerResult<Resource> {
        // Authoritative existence + metadata FIRST (SPARQ), then fetch the bytes it points at —
        // through the read-4 body cache, keyed by THIS lookup's authoritative `(blob_key, etag)`
        // (a hit is exactly the bytes this metadata committed with; a miss is the pre-cache path).
        // If a concurrent rewrite/delete reclaims the blob between the two reads, `read_through`
        // re-reads the metadata and returns the current version (validators matching its bytes).
        let meta = self.get_meta_mapped(iri).await?;
        self.read_through(iri, meta).await
    }

    async fn meta(&self, iri: &str) -> ServerResult<Option<ResourceMeta>> {
        match self.sparq.get_meta(iri).await {
            Ok(m) => Ok(Some(m)),
            Err(SparqError::NotFound) => Ok(None),
            Err(SparqError::QuotaExceeded) => Err(ServerError::InsufficientStorage),
            Err(SparqError::Backend(e)) => Err(ServerError::Storage(e)),
            Err(SparqError::Unsettled { detail, fence }) => {
                Err(ServerError::Unsettled { detail, fence })
            }
        }
    }

    async fn exists(&self, iri: &str) -> ServerResult<bool> {
        self.sparq
            .exists(iri)
            .await
            .map_err(|e| ServerError::Storage(format!("{e}")))
    }

    async fn write(
        &self,
        iri: &str,
        body: Bytes,
        content_type: &str,
    ) -> ServerResult<ResourceMeta> {
        // Crash-consistency: bytes FIRST, then the authoritative index (spike §6). On an index-write
        // failure prod-solid-server issues a compensating delete; M2 ports that + the reconciler.
        //
        // The blob key is minted UNIQUE PER WRITE (`mint_blob_key`): a concurrent same-IRI write gets a
        // DIFFERENT key and so writes a disjoint object — no collision/interleave on a shared key. The
        // "latest committed" winner is whichever write's `put_meta` commits last (SPARQ authoritative);
        // a read then resolves through that winner's pointer to ITS bytes (never a clobber of the live
        // bytes). A re-write of an existing resource likewise lands on a NEW key; the key it replaced is
        // reclaimed inline AFTER the index commit below (best effort — on failure, or for a concurrent
        // loser's key this commit never observed, the bytes stay as an orphan for the reconciler).
        //
        // Minting FAILS CLOSED if the OS RNG is unavailable (rather than minting a weak, possibly-colliding
        // key) — the write errors instead, so the no-two-writes-share-a-key invariant holds even then.
        let blob_key = Self::mint_blob_key(iri)?;
        let etag = Self::etag_for(content_type, &body);
        self.blob.put(&blob_key, body).await.map_err(|e| match e {
            BlobError::QuotaExceeded => ServerError::InsufficientStorage,
            other => ServerError::Storage(format!("{other}")),
        })?;
        let meta = ResourceMeta {
            content_type: content_type.to_string(),
            blob_key,
            etag,
            // Stamp the write instant so `If-Modified-Since` sees a real Last-Modified: a re-write
            // bumps it, so a later conditional GET correctly re-serves the changed representation.
            last_modified: Some(crate::clock::now()),
        };
        let previous = self
            .sparq
            .replace_meta(iri, meta.clone())
            .await
            .map_err(|e| match e {
                SparqError::QuotaExceeded => ServerError::InsufficientStorage,
                other => ServerError::Storage(format!("{other}")),
            })?;
        // The index now points at `blob_key`; the superseded body is unreferenced. Reclaim it so
        // sustained rewrites cannot fill the blob store with dead versions.
        if let Some(old) = previous.filter(|old| old.blob_key != meta.blob_key) {
            self.reclaim_blob(&old.blob_key).await;
        }
        Ok(meta)
    }

    async fn create_in_container(
        &self,
        container: &str,
        child: &str,
        body: Bytes,
        content_type: &str,
    ) -> ServerResult<ResourceMeta> {
        // Write the bytes FIRST (content-addressed by key; idempotent), then commit the child's
        // metadata AND its containment edge in ONE atomic index operation (`create_child`). Because
        // the metadata + the edge commit together, there is no window in which the edge exists
        // without backing metadata — so the POST path needs NO removal-based compensation and a
        // concurrent same-IRI creator can never observe or tear down a half-built containment. A
        // missing container ⇒ 404; the bytes written above are then orphaned and GC'd by the
        // reconciler — the same crash-consistency model `write` documents. (The handler only takes
        // this path for an absent child; a concurrent same-IRI create that wins the race overwrites
        // this record, and the key it replaced is left for the reconciler, as `create_child` does not
        // report the record it replaced.)
        //
        // The child's blob key is minted UNIQUE PER WRITE (`mint_blob_key`), so a concurrent same-IRI
        // create writes a disjoint object and cannot collide with this one on a shared key. Minting FAILS
        // CLOSED on an unavailable OS RNG (the create errors rather than minting a weak, possibly-colliding
        // key), so the no-shared-key invariant holds even then.
        let blob_key = Self::mint_blob_key(child)?;
        let etag = Self::etag_for(content_type, &body);
        self.blob.put(&blob_key, body).await.map_err(|e| match e {
            BlobError::QuotaExceeded => ServerError::InsufficientStorage,
            other => ServerError::Storage(format!("{other}")),
        })?;
        let meta = ResourceMeta {
            content_type: content_type.to_string(),
            blob_key,
            etag,
            // Stamp the create instant (see `write`) — the new child's Last-Modified for
            // `If-Modified-Since`.
            last_modified: Some(crate::clock::now()),
        };
        match self
            .sparq
            .create_child(container, child, meta.clone())
            .await
        {
            Ok(()) => Ok(meta),
            Err(SparqError::NotFound) => Err(ServerError::NotFound),
            Err(SparqError::QuotaExceeded) => Err(ServerError::InsufficientStorage),
            Err(SparqError::Backend(e)) => Err(ServerError::Storage(e)),
            // Its bytes stay: the record may still commit pointing at them.
            Err(SparqError::Unsettled { detail, fence }) => {
                Err(ServerError::Unsettled { detail, fence })
            }
        }
    }

    async fn restore(
        &self,
        iri: &str,
        container: Option<&str>,
        body: Bytes,
        meta: &ResourceMeta,
    ) -> ServerResult<ResourceMeta> {
        // As `write` and `create_in_container`, with the record's own validators kept.
        let blob_key = Self::mint_blob_key(iri)?;
        self.blob.put(&blob_key, body).await.map_err(|e| match e {
            BlobError::QuotaExceeded => ServerError::InsufficientStorage,
            other => ServerError::Storage(format!("{other}")),
        })?;
        let meta = ResourceMeta {
            blob_key,
            ..meta.clone()
        };
        let committed = match container {
            Some(c) => self
                .sparq
                .create_child(c, iri, meta.clone())
                .await
                .map(|()| None),
            None => self.sparq.replace_meta(iri, meta.clone()).await,
        };
        match committed {
            Ok(previous) => {
                if let Some(old) = previous.filter(|old| old.blob_key != meta.blob_key) {
                    self.reclaim_blob(&old.blob_key).await;
                }
                Ok(meta)
            }
            Err(e) => {
                // A restore is retried until it lands, each attempt under a fresh key: the bytes
                // of one whose record did not land are reclaimed here, so failed attempts do not
                // pile up in the blob store. A record that did land (a lost reply) is kept; when
                // that cannot be told, the bytes are left to the reconciler.
                // A record whose commit is not settled may still land pointing at the bytes:
                // they are left to the reconciler.
                match self.sparq.get_meta(iri).await {
                    Ok(now) if now.blob_key == meta.blob_key => return Ok(meta),
                    _ if matches!(e, SparqError::Unsettled { .. }) => {}
                    Ok(_) | Err(SparqError::NotFound) => self.reclaim_blob(&meta.blob_key).await,
                    Err(_) => {}
                }
                Err(match e {
                    SparqError::NotFound => ServerError::NotFound,
                    SparqError::QuotaExceeded => ServerError::InsufficientStorage,
                    SparqError::Backend(e) => ServerError::Storage(e),
                    SparqError::Unsettled { detail, fence } => {
                        ServerError::Unsettled { detail, fence }
                    }
                })
            }
        }
    }

    async fn hold(&self, iri: &str, holder: &str) -> ServerResult<Option<ResourceMeta>> {
        // Published, then checked: a reclaim after the commit that leaves these bytes runs after
        // this hold's record is visible, or this sees the record moved on (see `reclaim_blob`).
        for _ in 0..=READ_RACE_RETRIES {
            let meta = match self.sparq.get_meta(iri).await {
                Ok(meta) => meta,
                Err(SparqError::NotFound) => return Ok(None),
                Err(e) => return Err(map_sparq(e)),
            };
            // Counted before it is published, so no reclaim through this instance meanwhile
            // misses it.
            self.count_hold(&meta.blob_key, true);
            if let Err(e) = self.publish_hold(&meta.blob_key, holder).await {
                // It may have landed (a lost reply): released, best effort, as the holder is
                // told nothing is held (else the next start's sweep releases it). Released, not
                // only withdrawn: a reclaim through this instance meanwhile skipped these bytes
                // for the count, and they are reclaimed here unless a record names them.
                let _ = self.release(&meta, holder).await;
                return Err(e);
            }
            match self.sparq.get_meta(iri).await {
                Ok(now) if now.blob_key == meta.blob_key => return Ok(Some(meta)),
                // Rewritten or removed meanwhile: what was held may already be gone.
                Ok(_) | Err(SparqError::NotFound) => {
                    let _ = self.release(&meta, holder).await;
                }
                Err(e) => {
                    let _ = self.release(&meta, holder).await;
                    return Err(map_sparq(e));
                }
            }
        }
        Err(ServerError::ResourceChanged)
    }

    async fn reinstate(
        &self,
        iri: &str,
        container: Option<&str>,
        held: &ResourceMeta,
    ) -> ServerResult<ResourceMeta> {
        // As `restore`, pointing at the held bytes rather than a copy: the hold keeps them.
        let committed = match container {
            Some(c) => self
                .sparq
                .create_child(c, iri, held.clone())
                .await
                .map(|()| None),
            None => self.sparq.replace_meta(iri, held.clone()).await,
        };
        let previous = committed.map_err(map_sparq)?;
        if let Some(old) = previous.filter(|old| old.blob_key != held.blob_key) {
            self.reclaim_blob(&old.blob_key).await;
        }
        Ok(held.clone())
    }

    async fn release(&self, held: &ResourceMeta, holder: &str) -> ServerResult<()> {
        // Uncounted whatever the withdrawal's outcome: a record left behind still holds them,
        // through the index.
        let withdrawn = self.withdraw_hold(&held.blob_key, holder).await;
        self.count_hold(&held.blob_key, false);
        withdrawn?;
        self.reclaim_blob(&held.blob_key).await;
        Ok(())
    }

    async fn holds(&self) -> ServerResult<Vec<(String, ResourceMeta)>> {
        let list = |c: String| async move {
            match self.sparq.list_children(&c).await {
                Ok(children) => Ok(children),
                Err(SparqError::NotFound) => Ok(Vec::new()),
                Err(e) => Err(map_sparq(e)),
            }
        };
        let mut held = Vec::new();
        for dir in list(HELD_CONTAINER.to_string()).await? {
            let records = list(dir.clone()).await?;
            if records.is_empty() {
                // Left by a release or a publication cut short: removed (best effort) while
                // still empty, as a hold published meanwhile recreates it.
                let _ = self
                    .sparq
                    .delete_meta_if_empty(&dir, Some(HELD_CONTAINER))
                    .await;
            }
            for record in records {
                match self.sparq.get_meta(&record).await {
                    Ok(mut meta) => {
                        let holder = std::mem::take(&mut meta.etag);
                        held.push((holder, meta));
                    }
                    Err(SparqError::NotFound) => {}
                    Err(e) => return Err(map_sparq(e)),
                }
            }
        }
        Ok(held)
    }

    async fn finish_reclaims(&self) -> ServerResult<()> {
        for key in self.sparq.reclaiming().await.map_err(map_sparq)? {
            match self.blob.delete(&key).await {
                Ok(()) | Err(BlobError::NotFound) => {
                    self.sparq.end_reclaim(&key).await.map_err(map_sparq)?
                }
                Err(e) => return Err(ServerError::Storage(format!("{e}"))),
            }
        }
        Ok(())
    }

    async fn delete(&self, iri: &str, parent: Option<&str>) -> ServerResult<()> {
        // Look up the byte-pointer from the authoritative index so we delete the right blob.
        let blob_key = match self.sparq.get_meta(iri).await {
            Ok(m) => Some(m.blob_key),
            Err(SparqError::NotFound) => None,
            Err(SparqError::QuotaExceeded) => return Err(ServerError::InsufficientStorage),
            Err(SparqError::Backend(e)) => return Err(ServerError::Storage(e)),
            Err(SparqError::Unsettled { detail, fence }) => {
                return Err(ServerError::Unsettled { detail, fence })
            }
        };
        // Detach from the parent's containment first, then drop the index record, then the bytes.
        // Index-before-bytes keeps the invariant "if it's indexed, its bytes exist" — a crash after
        // the index delete leaves orphaned bytes (the reconciler GCs them), never an index row
        // pointing at missing bytes. The byte delete is best effort: the resource is already gone
        // from the index, so a failed blob delete must not fail the request; it stays an orphan.
        // Either step's failure is reported as what it was (`map_sparq`), as the metadata
        // read above is: a refused admission and a write that failed unsettled (which the caller
        // settles through the fence it carries) are not flattened into a backend fault.
        if let Some(p) = parent {
            self.sparq.remove_child(p, iri).await.map_err(map_sparq)?;
        }
        self.sparq.delete_meta(iri).await.map_err(map_sparq)?;
        if let Some(key) = blob_key {
            self.reclaim_blob(&key).await;
        }
        Ok(())
    }

    async fn delete_container_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> ServerResult<DeleteOutcome> {
        // The ATOMIC empty-check + record delete + PARENT-edge detach in ONE index op (no interleaving
        // — see `delete_meta_if_empty`). The parent-edge detach is folded INTO this single op (it is no
        // longer a separate `remove_child` afterwards) so there is no window in which the container
        // graph is gone but the parent still `ldp:contains` it — a window a concurrent recreate could
        // exploit to be orphaned by a stale detach.
        //
        // Bytes: the container's blob key is read from the index FIRST, and its bytes are reclaimed
        // inline only AFTER the atomic index delete has committed (best effort — a failed delete, or a
        // crash in between, leaves an orphan for the reconciler's sweep, never an index row pointing at
        // missing bytes). The blob store's `delete` is unconditional, but keys are UNIQUE PER WRITE
        // (`mint_blob_key`): a concurrent same-IRI rewrite or recreate between the lookup and the index
        // delete writes under a DIFFERENT key, so this can never clobber live bytes — at worst that
        // other key is the orphan the reconciler collects.
        let blob_key = match self.sparq.get_meta(iri).await {
            Ok(m) => Some(m.blob_key),
            Err(SparqError::NotFound) => None,
            Err(SparqError::QuotaExceeded) => return Err(ServerError::InsufficientStorage),
            Err(SparqError::Backend(e)) => return Err(ServerError::Storage(e)),
            Err(SparqError::Unsettled { detail, fence }) => {
                return Err(ServerError::Unsettled { detail, fence })
            }
        };
        // A failure is reported as what it was (`map_sparq`): a refused admission stays
        // `InsufficientStorage`, and one not known yet keeps its fence, so the caller can
        // settle it.
        let outcome = self
            .sparq
            .delete_meta_if_empty(iri, parent)
            .await
            .map_err(map_sparq)?;
        // NotEmpty / NotFound: nothing was deleted, so the bytes stay. Deleted: the record AND the parent
        // edge are gone atomically (above), so the bytes are unreferenced — reclaim them now.
        if outcome == DeleteOutcome::Deleted {
            if let Some(key) = blob_key {
                self.reclaim_blob(&key).await;
            }
        }
        Ok(outcome)
    }

    async fn fence(&self, fence: &Fence) -> ServerResult<()> {
        self.sparq
            .fence(fence)
            .await
            .map_err(|e| ServerError::Storage(format!("{e}")))
    }

    async fn read_marker(&self, iri: &str) -> ServerResult<Option<String>> {
        match self.sparq.get_meta(iri).await {
            Ok(m) if sparq::is_marker(&m) => Ok(Some(m.etag)),
            Ok(_) => Ok(Some(String::new())),
            Err(SparqError::NotFound) => Ok(None),
            Err(e) => Err(ServerError::Storage(format!("{e}"))),
        }
    }

    fn raises_markers(&self) -> bool {
        self.sparq.raises_markers()
    }

    async fn raise_marker(&self, iri: &str, value: i64) -> ServerResult<i64> {
        self.sparq
            .raise_marker(iri, value)
            .await
            .map_err(marker_error)
    }

    async fn reset_marker(&self, iri: &str, value: i64, _: &LifecycleGuard) -> ServerResult<()> {
        if !self.sparq.raises_markers() {
            return Err(ServerError::Storage(
                "this store cannot raise a marker atomically".into(),
            ));
        }
        // One record put: the index admits a marker's record apart from every ceiling.
        self.sparq
            .put_meta(iri, sparq::marker_meta(&value.to_string()))
            .await
            .map_err(marker_error)
    }

    async fn read_plan(&self, target: &str, acl_candidates: &[String]) -> ServerResult<ReadPlan> {
        // Delegate to the SparqClient seam: the live HTTP client answers this with ONE combined
        // SELECT (§3.1); the in-memory double with one atomic index pass. Fail-closed: any backend
        // error fails the whole plan.
        self.sparq
            .read_plan(target, acl_candidates)
            .await
            .map_err(|e| match e {
                SparqError::NotFound => ServerError::NotFound,
                SparqError::QuotaExceeded => ServerError::InsufficientStorage,
                SparqError::Backend(msg) => ServerError::Storage(msg),
                SparqError::Unsettled { detail, fence } => ServerError::Unsettled { detail, fence },
            })
    }

    async fn read_at(&self, iri: &str, meta: &ResourceMeta) -> ServerResult<Bytes> {
        // The §3.3 direct byte fetch through the held pointer — NO second `get_meta` on the common
        // path — through the read-4 body cache: the held `(blob_key, etag)` came from THIS request's
        // authoritative read-plan round, so a hit is exactly the bytes that metadata committed with
        // (the unique-per-write blob key names an immutable object).
        if let Some(body) = self.try_fetch_body(meta).await? {
            return Ok(body);
        }
        // The blob is gone. Only now (the race path) re-read the index to say why — and NEVER serve
        // the current version's bytes in place of the authorized one (see the trait doc).
        let current = self.get_meta_mapped(iri).await?;
        if current.blob_key == meta.blob_key {
            // The index still points at the missing blob: not a race, a real inconsistency.
            return Err(ServerError::Storage("byte/index inconsistency".into()));
        }
        Err(ServerError::ResourceChanged)
    }

    async fn list_children(&self, container: &str) -> ServerResult<Vec<ValidatedChildIri>> {
        let raw = self
            .sparq
            .list_children(container)
            .await
            .map_err(|e| ServerError::Storage(format!("{e}")))?;
        Ok(validated_children(raw))
    }

    async fn list_children_within(
        &self,
        container: &str,
        max_bytes: usize,
    ) -> ServerResult<Option<Vec<ValidatedChildIri>>> {
        let raw = self
            .sparq
            .list_children_within(container, max_bytes)
            .await
            .map_err(|e| ServerError::Storage(format!("{e}")))?;
        Ok(raw.map(validated_children))
    }
}

/// An index error as the server reports it: an absent record is [`ServerError::NotFound`], a
/// refused admission [`ServerError::InsufficientStorage`], and a write that failed unsettled
/// keeps its detail and its fence, so the caller can settle it ([`Store::fence`]) rather than
/// read the failure as a plain backend fault.
fn map_sparq(e: SparqError) -> ServerError {
    match e {
        SparqError::NotFound => ServerError::NotFound,
        SparqError::QuotaExceeded => ServerError::InsufficientStorage,
        SparqError::Backend(e) => ServerError::Storage(e),
        SparqError::Unsettled { detail, fence } => ServerError::Unsettled { detail, fence },
    }
}

/// The child IRIs of `raw` that are RFC 3987 IRIs.
fn validated_children(raw: Vec<String>) -> Vec<ValidatedChildIri> {
    // Validate each raw child IRI (full RFC-3987) at THIS boundary — the point where a
    // malformed/injected row from storage first crosses into the server's own logic. A malformed
    // IRI is FAIL-CLOSED OMITTED (never flows unchecked into the render), preserving the render's
    // prior skip-on-invalid behaviour but moving the guarantee to the architecturally-correct place.
    // Unreachable via the validated LDP write path (every stored child IRI is RFC-3987-valid by
    // construction); a store-layer bug that produced one is caught in debug/test by the assert.
    let mut out = Vec::with_capacity(raw.len());
    for iri in raw {
        match ValidatedChildIri::parse(&iri) {
            Some(v) => out.push(v),
            None => {
                debug_assert!(false, "store yielded a non-RFC-3987 child IRI: {iri}");
                eprintln!("  STORE: omitting non-RFC-3987 child IRI from list_children: {iri:?}");
            }
        }
    }
    out
}

/// A tiny FNV-1a hash used only for the placeholder ETag (NOT a cryptographic digest).
fn fnv1a_chain<'a>(bytes: impl Iterator<Item = &'a u8>) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::blob::InMemoryBlobStore;
    use crate::store::sparq::InMemorySparqClient;

    type S = CompositeStore<InMemorySparqClient, InMemoryBlobStore>;

    /// Review finding: every attempt at a restore uploaded its bytes under a fresh key, and an
    /// attempt whose record did not land left them behind, so a restore retried against a
    /// failing index could fill the blob store. They are reclaimed now.
    #[tokio::test]
    async fn a_restore_that_does_not_land_leaves_no_bytes() {
        let store = S::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let meta = ResourceMeta {
            content_type: "text/plain".into(),
            blob_key: String::new(),
            etag: "\"e\"".into(),
            last_modified: None,
        };
        for _ in 0..3 {
            let r = store
                .restore(
                    "https://pod.example/missing/a",
                    Some("https://pod.example/missing/"),
                    Bytes::from_static(b"x"),
                    &meta,
                )
                .await;
            assert!(matches!(r, Err(ServerError::NotFound)), "{r:?}");
        }
        assert!(store.blob.list().await.unwrap().is_empty());
    }

    /// A hold keeps a record's bytes through a rewrite and a removal, so the record is put back
    /// pointing at them, validators included, with nothing copied; released, the bytes go unless
    /// the record points at them again, and a hold that outlives its process is found again,
    /// with its holder.
    #[tokio::test]
    async fn held_bytes_outlive_their_record_until_released() {
        let store = S::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let (c, x, me) = (
            "https://pod.example/c/",
            "https://pod.example/c/x",
            "urn:me",
        );
        store.write(c, Bytes::new(), "text/turtle").await.unwrap();
        let first = store
            .create_in_container(c, x, Bytes::from_static(b"one"), "text/plain")
            .await
            .unwrap();
        assert!(store
            .hold("https://pod.example/c/none", me)
            .await
            .unwrap()
            .is_none());
        let held = store.hold(x, me).await.unwrap().expect("held");
        assert_eq!(held.blob_key, first.blob_key);
        // Rewritten, then removed: the held bytes stay.
        let second = store
            .write(x, Bytes::from_static(b"two"), "text/plain")
            .await
            .unwrap();
        store.delete_container_if_empty(x, Some(c)).await.unwrap();
        assert_eq!(&store.read_at(x, &held).await.unwrap()[..], b"one");
        // The rewrite's own bytes, held by nothing, went with the removal.
        assert!(!store.blob.exists(&second.blob_key).await.unwrap());
        let holds: Vec<(String, String)> = store
            .holds()
            .await
            .unwrap()
            .into_iter()
            .map(|(holder, h)| (holder, h.blob_key))
            .collect();
        assert_eq!(holds, [(me.to_string(), held.blob_key.clone())]);
        // Put back by reference, as it was.
        store.reinstate(x, Some(c), &held).await.unwrap();
        let back = store.read(x).await.unwrap();
        assert_eq!(&back.body[..], b"one");
        assert_eq!(back.meta.etag, first.etag);
        assert!(store
            .list_children(c)
            .await
            .unwrap()
            .iter()
            .any(|m| m.as_str() == x));
        // Released while the record points at them, they stay; released after a rewrite, they
        // go.
        store.release(&held, me).await.unwrap();
        assert!(store.holds().await.unwrap().is_empty());
        assert!(store.blob.exists(&held.blob_key).await.unwrap());
        let again = store.hold(x, me).await.unwrap().expect("held");
        store
            .write(x, Bytes::from_static(b"three"), "text/plain")
            .await
            .unwrap();
        assert!(store.blob.exists(&again.blob_key).await.unwrap());
        store.release(&again, me).await.unwrap();
        assert!(!store.blob.exists(&again.blob_key).await.unwrap());
        // Releasing again does nothing.
        store.release(&again, me).await.unwrap();
        assert_eq!(&store.read(x).await.unwrap().body[..], b"three");
    }

    /// A release cut short after its record went leaves the key's container: it names no bytes
    /// (so the orphan sweep may collect them), and a start's listing of the holds removes it.
    #[tokio::test]
    async fn a_holds_container_left_behind_names_no_bytes_and_goes_at_start() {
        let store = S::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let x = "https://pod.example/x";
        store
            .write(x, Bytes::from_static(b"one"), "text/plain")
            .await
            .unwrap();
        let held = store.hold(x, "urn:me").await.unwrap().expect("held");
        // Only the record is withdrawn: the process stops before the container goes.
        store
            .sparq
            .delete_meta_if_empty(
                &held_record(&held.blob_key, "urn:me"),
                Some(&held_dir(&held.blob_key)),
            )
            .await
            .unwrap();
        store.count_hold(&held.blob_key, false);
        store
            .write(x, Bytes::from_static(b"two"), "text/plain")
            .await
            .unwrap();
        assert!(store.sparq.exists(&held_dir(&held.blob_key)).await.unwrap());
        assert!(!store
            .sparq
            .referenced_blob_keys()
            .await
            .unwrap()
            .contains(&held.blob_key));
        assert!(store.holds().await.unwrap().is_empty());
        assert!(!store.sparq.exists(&held_dir(&held.blob_key)).await.unwrap());
    }

    /// Two holders of the same bytes each hold them: the first to release, however late, never
    /// ends the other's hold, and the bytes go only once both have.
    #[tokio::test]
    async fn one_holders_release_never_ends_anothers_hold() {
        let store = S::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let (c, x) = ("https://pod.example/c/", "https://pod.example/c/x");
        store.write(c, Bytes::new(), "text/turtle").await.unwrap();
        store
            .create_in_container(c, x, Bytes::from_static(b"one"), "text/plain")
            .await
            .unwrap();
        let a = store.hold(x, "urn:a").await.unwrap().expect("held");
        let b = store.hold(x, "urn:b").await.unwrap().expect("held");
        assert_eq!(a.blob_key, b.blob_key);
        store.delete_container_if_empty(x, Some(c)).await.unwrap();
        // The earlier holder's release comes late, after the record went.
        store.release(&a, "urn:a").await.unwrap();
        let holds: Vec<(String, String)> = store
            .holds()
            .await
            .unwrap()
            .into_iter()
            .map(|(holder, h)| (holder, h.blob_key))
            .collect();
        assert_eq!(
            holds,
            [("urn:b".to_string(), b.blob_key.clone())],
            "b's record went"
        );
        assert!(
            store.blob.exists(&b.blob_key).await.unwrap(),
            "b's hold ended"
        );
        store.reinstate(x, Some(c), &b).await.unwrap();
        assert_eq!(&store.read(x).await.unwrap().body[..], b"one");
        store.release(&b, "urn:b").await.unwrap();
        assert!(
            store.blob.exists(&b.blob_key).await.unwrap(),
            "put back, yet gone"
        );
        store
            .write(x, Bytes::from_static(b"two"), "text/plain")
            .await
            .unwrap();
        assert!(
            !store.blob.exists(&b.blob_key).await.unwrap(),
            "never released"
        );
    }

    /// A [`SparqClient`] over which another instance puts a record back on bytes just before
    /// the next reclaim decides on them.
    struct PutBackFirst {
        inner: InMemorySparqClient,
        put_back: std::sync::Mutex<Option<(String, ResourceMeta)>>,
    }

    #[async_trait]
    impl SparqClient for PutBackFirst {
        async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError> {
            self.inner.get_meta(iri).await
        }
        async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
            self.inner.put_meta(iri, meta).await
        }
        async fn replace_meta(
            &self,
            iri: &str,
            meta: ResourceMeta,
        ) -> Result<Option<ResourceMeta>, SparqError> {
            self.inner.replace_meta(iri, meta).await
        }
        async fn exists(&self, iri: &str) -> Result<bool, SparqError> {
            self.inner.exists(iri).await
        }
        async fn begin_reclaim(&self, blob_key: &str) -> Result<bool, SparqError> {
            // Another instance puts a record back on the bytes just before this reclaim decides.
            let put_back = self.put_back.lock().unwrap().take();
            if let Some((iri, meta)) = put_back {
                self.inner.put_meta(&iri, meta).await?;
            }
            self.inner.begin_reclaim(blob_key).await
        }
        async fn end_reclaim(&self, blob_key: &str) -> Result<(), SparqError> {
            self.inner.end_reclaim(blob_key).await
        }
        async fn reclaiming(&self) -> Result<Vec<String>, SparqError> {
            self.inner.reclaiming().await
        }
        async fn delete_meta(&self, iri: &str) -> Result<(), SparqError> {
            self.inner.delete_meta(iri).await
        }
        async fn delete_meta_if_empty(
            &self,
            iri: &str,
            parent: Option<&str>,
        ) -> Result<DeleteOutcome, SparqError> {
            self.inner.delete_meta_if_empty(iri, parent).await
        }
        async fn create_child(
            &self,
            container: &str,
            child: &str,
            meta: ResourceMeta,
        ) -> Result<(), SparqError> {
            self.inner.create_child(container, child, meta).await
        }
        async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError> {
            self.inner.remove_child(container, child).await
        }
        async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError> {
            self.inner.list_children(container).await
        }
        async fn referenced_blob_keys(
            &self,
        ) -> Result<std::collections::HashSet<String>, SparqError> {
            self.inner.referenced_blob_keys().await
        }
    }

    /// Loop review: a release read the record, then another holder put it back on the bytes
    /// and released, then the first deleted them. The decision to reclaim is now one atomic
    /// step of the index, so a record put back up to that step keeps them.
    #[tokio::test]
    async fn bytes_put_back_up_to_the_reclaim_stay() {
        let store = CompositeStore::new(
            PutBackFirst {
                inner: InMemorySparqClient::new(),
                put_back: Default::default(),
            },
            InMemoryBlobStore::new(),
        );
        let x = "https://pod.example/x";
        store
            .write(x, Bytes::from_static(b"one"), "text/plain")
            .await
            .unwrap();
        let held = store.hold(x, "urn:a").await.unwrap().expect("held");
        store.delete(x, None).await.unwrap();
        *store.sparq.put_back.lock().unwrap() = Some((x.to_string(), held.clone()));
        store.release(&held, "urn:a").await.unwrap();
        assert_eq!(&store.read(x).await.unwrap().body[..], b"one");
        assert!(store.sparq.reclaiming().await.unwrap().is_empty());
    }

    /// Bytes are named as being reclaimed only while no record names them, a hold's included;
    /// a reclaim a stop cut short, once named, is finished at start.
    #[tokio::test]
    async fn a_reclaim_is_named_only_on_unnamed_bytes_and_finished_at_start() {
        let store = S::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let x = "https://pod.example/x";
        let first = store
            .write(x, Bytes::from_static(b"one"), "text/plain")
            .await
            .unwrap();
        assert!(!store.sparq.begin_reclaim(&first.blob_key).await.unwrap());
        let held = store.hold(x, "urn:a").await.unwrap().expect("held");
        // Removed from the index alone, as a write would, the hold still names the bytes.
        store.sparq.delete_meta(x).await.unwrap();
        assert!(!store.sparq.begin_reclaim(&held.blob_key).await.unwrap());
        // The hold's record withdrawn and the process stopped before reclaiming:
        store
            .sparq
            .delete_meta_if_empty(
                &held_record(&held.blob_key, "urn:a"),
                Some(&held_dir(&held.blob_key)),
            )
            .await
            .unwrap();
        assert!(store.sparq.begin_reclaim(&held.blob_key).await.unwrap());
        assert!(store.blob.exists(&held.blob_key).await.unwrap());
        store.finish_reclaims().await.unwrap();
        assert!(!store.blob.exists(&held.blob_key).await.unwrap());
        assert!(store.sparq.reclaiming().await.unwrap().is_empty());
    }

    /// One backend shared by several stores, as several processes share one.
    struct Shared<T>(std::sync::Arc<T>);

    #[async_trait]
    impl<T: SparqClient> SparqClient for Shared<T> {
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
        async fn begin_reclaim(&self, blob_key: &str) -> Result<bool, SparqError> {
            self.0.begin_reclaim(blob_key).await
        }
        async fn end_reclaim(&self, blob_key: &str) -> Result<(), SparqError> {
            self.0.end_reclaim(blob_key).await
        }
        async fn reclaiming(&self) -> Result<Vec<String>, SparqError> {
            self.0.reclaiming().await
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
        async fn referenced_blob_keys(
            &self,
        ) -> Result<std::collections::HashSet<String>, SparqError> {
            self.0.referenced_blob_keys().await
        }
    }

    #[async_trait]
    impl<T: BlobStore> BlobStore for Shared<T> {
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
        async fn delete_if_unchanged(&self, key: &str, generation: u64) -> Result<bool, BlobError> {
            self.0.delete_if_unchanged(key, generation).await
        }
    }

    /// A hold taken through one instance keeps the bytes from a reclaim through another over the
    /// same backends: whether bytes are held is asked of the index, not of the instance.
    #[tokio::test]
    async fn a_hold_through_one_instance_keeps_bytes_from_another() {
        let (sparq, blob) = (
            std::sync::Arc::new(InMemorySparqClient::new()),
            std::sync::Arc::new(InMemoryBlobStore::new()),
        );
        let one = CompositeStore::new(Shared(sparq.clone()), Shared(blob.clone()));
        let two = CompositeStore::new(Shared(sparq), Shared(blob.clone()));
        let (c, x) = ("https://pod.example/c/", "https://pod.example/c/x");
        one.write(c, Bytes::new(), "text/turtle").await.unwrap();
        one.create_in_container(c, x, Bytes::from_static(b"one"), "text/plain")
            .await
            .unwrap();
        let held = one.hold(x, "urn:a").await.unwrap().expect("held");
        // Rewritten through the other instance, which reclaims what it replaced.
        two.write(x, Bytes::from_static(b"two"), "text/plain")
            .await
            .unwrap();
        assert!(
            blob.exists(&held.blob_key).await.unwrap(),
            "reclaimed while held"
        );
        one.release(&held, "urn:a").await.unwrap();
        assert!(!blob.exists(&held.blob_key).await.unwrap());
    }

    /// An index whose detaching steps fail with the error set for each (taken when it is used),
    /// every other call answered by a working in-memory index.
    struct FailsDetaching {
        inner: InMemorySparqClient,
        on_remove_child: std::sync::Mutex<Option<SparqError>>,
        on_delete_meta: std::sync::Mutex<Option<SparqError>>,
        on_delete_meta_if_empty: std::sync::Mutex<Option<SparqError>>,
    }

    impl FailsDetaching {
        fn new() -> Self {
            Self {
                inner: InMemorySparqClient::new(),
                on_remove_child: std::sync::Mutex::new(None),
                on_delete_meta: std::sync::Mutex::new(None),
                on_delete_meta_if_empty: std::sync::Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl SparqClient for FailsDetaching {
        async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError> {
            self.inner.get_meta(iri).await
        }
        async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
            self.inner.put_meta(iri, meta).await
        }
        async fn exists(&self, iri: &str) -> Result<bool, SparqError> {
            self.inner.exists(iri).await
        }
        async fn delete_meta(&self, iri: &str) -> Result<(), SparqError> {
            let set = self.on_delete_meta.lock().unwrap().take();
            match set {
                Some(e) => Err(e),
                None => self.inner.delete_meta(iri).await,
            }
        }
        async fn delete_meta_if_empty(
            &self,
            iri: &str,
            parent: Option<&str>,
        ) -> Result<crate::store::sparq::DeleteOutcome, SparqError> {
            let set = self.on_delete_meta_if_empty.lock().unwrap().take();
            match set {
                Some(e) => Err(e),
                None => self.inner.delete_meta_if_empty(iri, parent).await,
            }
        }
        async fn create_child(
            &self,
            container: &str,
            child: &str,
            meta: ResourceMeta,
        ) -> Result<(), SparqError> {
            self.inner.create_child(container, child, meta).await
        }
        async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError> {
            let set = self.on_remove_child.lock().unwrap().take();
            match set {
                Some(e) => Err(e),
                None => self.inner.remove_child(container, child).await,
            }
        }
        async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError> {
            self.inner.list_children(container).await
        }
        async fn referenced_blob_keys(
            &self,
        ) -> Result<std::collections::HashSet<String>, SparqError> {
            self.inner.referenced_blob_keys().await
        }
    }

    /// A delete whose detach or record step fails reports the failure as what it was, as the
    /// metadata read that precedes them does: a refused admission stays `InsufficientStorage`,
    /// and a write that failed unsettled keeps its fence, so the caller can settle it.
    #[tokio::test]
    async fn a_delete_reports_each_failure_as_what_it_was() {
        let container = "https://pod.example/c/";
        let child = "https://pod.example/c/note";
        let build = || async {
            let store = CompositeStore::new(FailsDetaching::new(), InMemoryBlobStore::new());
            store
                .write(container, Bytes::from_static(b""), "text/turtle")
                .await
                .expect("mint container");
            store
                .create_in_container(container, child, Bytes::from_static(b"x"), "text/turtle")
                .await
                .expect("add child");
            store
        };
        // A refused admission stays one.
        let store = build().await;
        *store.sparq.on_remove_child.lock().unwrap() = Some(SparqError::QuotaExceeded);
        let r = store.delete(child, Some(container)).await;
        assert!(matches!(r, Err(ServerError::InsufficientStorage)), "{r:?}");
        // A write that failed unsettled keeps its fence, so the caller can settle it.
        let store = build().await;
        *store.sparq.on_delete_meta.lock().unwrap() = Some(SparqError::Unsettled {
            detail: "the update timed out".into(),
            fence: Fence::new("n1".to_string(), vec![child.to_string()]),
        });
        let r = store.delete(child, Some(container)).await;
        match r {
            Err(ServerError::Unsettled { fence, .. }) => assert_eq!(fence.nonce(), "n1"),
            other => panic!("{:?}", other),
        }
    }

    /// The atomic removal (`delete_container_if_empty`) reports its index step's failure as
    /// what it was, as `delete` does: a refused admission stays `InsufficientStorage`, a write
    /// that failed unsettled keeps its fence, and the resource is left in place either way.
    #[tokio::test]
    async fn an_atomic_removal_reports_each_failure_as_what_it_was() {
        let container = "https://pod.example/c/";
        let child = "https://pod.example/c/note";
        let store = CompositeStore::new(FailsDetaching::new(), InMemoryBlobStore::new());
        store
            .write(container, Bytes::from_static(b""), "text/turtle")
            .await
            .expect("mint container");
        store
            .create_in_container(container, child, Bytes::from_static(b"x"), "text/turtle")
            .await
            .expect("add child");
        *store.sparq.on_delete_meta_if_empty.lock().unwrap() = Some(SparqError::QuotaExceeded);
        let r = store
            .delete_container_if_empty(child, Some(container))
            .await;
        assert!(matches!(r, Err(ServerError::InsufficientStorage)), "{r:?}");
        *store.sparq.on_delete_meta_if_empty.lock().unwrap() = Some(SparqError::Unsettled {
            detail: "the update timed out".into(),
            fence: Fence::new("n2".to_string(), vec![child.to_string()]),
        });
        let r = store
            .delete_container_if_empty(child, Some(container))
            .await;
        match r {
            Err(ServerError::Unsettled { fence, .. }) => assert_eq!(fence.nonce(), "n2"),
            other => panic!("{:?}", other),
        }
        assert!(store.exists(child).await.unwrap());
        let r = store
            .delete_container_if_empty(child, Some(container))
            .await;
        assert!(matches!(r, Ok(DeleteOutcome::Deleted)), "{r:?}");
    }

    #[test]
    fn validated_child_iri_accepts_valid_rejects_malformed() {
        // The fail-closed gate (bead wg3): full RFC-3987 validation. A valid IRI parses (and exposes
        // both the string + the NamedNode without a re-parse); a malformed one returns None so the
        // caller omits it rather than letting it flow unchecked.
        let v = ValidatedChildIri::parse("https://pod.example/c/item-0042")
            .expect("a valid http IRI must parse");
        assert_eq!(v.as_str(), "https://pod.example/c/item-0042");
        assert_eq!(
            v.as_named_node().as_str(),
            "https://pod.example/c/item-0042"
        );
        assert_eq!(
            v.clone().into_named_node(),
            NamedNode::new_unchecked("https://pod.example/c/item-0042")
        );

        // Malformed / injected forms RFC-3987 forbids in an IRI ⇒ None (fail-closed).
        for bad in [
            "",                              // empty
            "not an iri",                    // spaces
            "https://pod.example/c/a b",     // embedded space
            "https://pod.example/c/a\nb",    // control (newline)
            "https://pod.example/c/<a>",     // angle brackets (would corrupt a term)
            "https://pod.example/c/a\u{7f}", // DEL control
            "https://pod.example/c/a`b",     // backtick delimiter
        ] {
            assert!(
                ValidatedChildIri::parse(bad).is_none(),
                "malformed child IRI {bad:?} must be rejected (parse ⇒ None)"
            );
        }
    }

    #[test]
    fn composite_list_children_returns_validated_iris() {
        // The Store boundary yields RFC-3987-validated children. Written through the authoritative path,
        // then listed back — each child is a ValidatedChildIri whose string matches what was stored.
        let store = S::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let container = "https://pod.example/c/";
        let child = "https://pod.example/c/note1";
        tokio_test_block_on(async {
            store
                .write(container, Bytes::from_static(b""), "text/turtle")
                .await
                .expect("mint container");
            store
                .create_in_container(container, child, Bytes::from_static(b"x"), "text/turtle")
                .await
                .expect("add child");
            let kids = store.list_children(container).await.expect("list");
            assert_eq!(
                kids.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
                vec![child],
                "list_children returns the child as a validated IRI"
            );
        });
    }

    #[test]
    fn mint_blob_key_is_unique_per_call_for_the_same_iri() {
        // The ROOT-fix unit invariant: minting a key for the SAME IRI twice yields DIFFERENT keys (no
        // deterministic reuse). A handful of repeats makes a chance collision of the 128-bit random
        // suffix astronomically unlikely, so a single `assert_ne!` is enough; we mint a batch and assert
        // all-distinct to be thorough. MUTATION-CHECK: revert to the old deterministic `iri.replace(...)`
        // and every minted key is identical ⇒ this fails.
        let iri = "https://pod.example/alice/data";
        let mut keys: Vec<String> = (0..32)
            .map(|_| S::mint_blob_key(iri).expect("the OS RNG must be available in tests"))
            .collect();
        let total = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(
            keys.len(),
            total,
            "every mint for the same IRI must be unique (the deterministic-key reuse is gone)"
        );
    }

    #[test]
    fn mint_blob_key_keeps_an_iri_derived_prefix_for_traceability() {
        // Uniqueness comes from the random suffix; the IRI-derived prefix is retained (cosmetic) so an
        // operator can still trace a key back to its resource. The minted key must START with the
        // percent-flattened IRI followed by the `-` separator.
        let iri = "https://pod.example/alice/data";
        let prefix = iri.replace([':', '/', '?', '#', '%'], "_");
        let key = S::mint_blob_key(iri).expect("the OS RNG must be available in tests");
        assert!(
            key.starts_with(&format!("{prefix}-")),
            "minted key {key:?} must keep the IRI-derived prefix {prefix:?} for traceability"
        );
        // ...and the suffix is the 32-hex-char (128-bit) random tail.
        let suffix = &key[prefix.len() + 1..];
        assert_eq!(
            suffix.len(),
            32,
            "the random suffix is 16 bytes = 32 hex chars"
        );
        assert!(
            suffix.bytes().all(|b| b.is_ascii_hexdigit()),
            "the suffix must be lowercase hex"
        );
    }

    #[test]
    fn mint_blob_key_is_fallible_and_succeeds_on_a_working_os_rng() {
        // The Medium fix: `mint_blob_key` is FALLIBLE — it propagates an RNG failure (fails closed) rather
        // than minting a weak, possibly-colliding key from a timestamp-only fallback. On every supported
        // platform the OS RNG IS available, so it returns `Ok`; the contract this pins is that the return
        // type is a `Result` carrying a `ServerError::Storage` on the (unreachable here) RNG-failure path —
        // verified by the `?` propagation at the `write`/`create_in_container` call sites compiling, and by
        // the success here. There is NO infallible fallback that could mint a same-tick-colliding key.
        let iri = "https://pod.example/alice/data";
        let key =
            S::mint_blob_key(iri).expect("the OS RNG is available on every supported platform");
        let prefix = iri.replace([':', '/', '?', '#', '%'], "_");
        assert!(
            key.starts_with(&format!("{prefix}-")) && key.len() == prefix.len() + 1 + 32,
            "the success path still mints a prefix + 32-hex-char random suffix"
        );

        // And a write through the public API succeeds end-to-end (the fallible mint does not regress the
        // happy path): the resource is then readable, its bytes resolved through the minted blob key.
        let store = S::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        tokio_test_block_on(async {
            let meta = store
                .write(iri, Bytes::from_static(b"body"), "text/turtle")
                .await
                .expect("a write with a working RNG must succeed");
            assert!(
                meta.blob_key.starts_with(&format!("{prefix}-")),
                "the persisted pointer names the minted unique key"
            );
            let resource = store.read(iri).await.expect("read-after-write");
            assert_eq!(resource.body, Bytes::from_static(b"body"));
        });
    }

    /// A tiny single-thread block-on so the test above can drive the async `Store` API without pulling in
    /// the `#[tokio::test]` macro for this otherwise-synchronous module.
    fn tokio_test_block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime")
            .block_on(f)
    }
}
