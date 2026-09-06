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
    InMemoryStoreLimits, StoreUsage, DEFAULT_IN_MEMORY_MAX_RESOURCE_COUNT,
    DEFAULT_IN_MEMORY_MAX_TOTAL_BYTES,
};
#[cfg(not(target_arch = "wasm32"))]
pub use reconcile::spawn_periodic;
pub use reconcile::{
    reconcile_orphans, ReconcileError, ReconcileOptions, ReconcileReport, DEFAULT_GRACE,
};
pub use sparq::{
    DeleteOutcome, InMemorySparqClient, ReadPlan, ResourceMeta, SparqClient, SparqError,
};
pub use sparql::{BodyObject, BuildError};

use crate::error::{ServerError, ServerResult};

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
    /// detach. The container's own bytes are NOT deleted inline: after the atomic index delete they are
    /// ORPHANED (no index row references them) and GC'd by the reconciler's orphaned-bytes sweep. We
    /// leave the bytes to the reconciler rather than delete them inline because the blob store is a
    /// separate system (its `delete` is unconditional). Since the composite store now mints UNIQUE
    /// blob keys per write (`CompositeStore::mint_blob_key`), a concurrent same-IRI recreate gets a
    /// DIFFERENT key, so an inline delete of THIS container's key could no longer clobber a recreate's
    /// bytes — but leaving them to the reconciler keeps the path uniform and side-effect-free (the sweep
    /// only GCs bytes with NO index row). Transient orphan until a sweep runs — space only, never an
    /// observable inconsistency. Returns:
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
    async fn list_children(&self, container: &str) -> ServerResult<Vec<ValidatedChildIri>>;

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
    /// [`read_plan`](Store::read_plan) round) — the §3.3 `read_at`: no second `get_meta`. Safe
    /// because blob keys are minted UNIQUE PER WRITE (`CompositeStore::mint_blob_key`): the key
    /// names an immutable object, so bytes fetched through a held pointer are exactly the bytes
    /// that pointer committed with — never a torn read of a newer write.
    ///
    /// The DEFAULT implementation re-reads via [`read`](Store::read) (metadata + bytes — the
    /// pre-read-2 cost and semantics), so non-composite [`Store`] impls (test doubles) behave
    /// exactly as before; [`CompositeStore`] overrides it with the direct blob fetch.
    async fn read_at(&self, iri: &str, meta: &ResourceMeta) -> ServerResult<Bytes> {
        let _ = meta;
        Ok(self.read(iri).await?.body)
    }
}

/// The default [`Store`]: SPARQ (authoritative metadata) + a blob store (backup bytes), with a
/// [`BodyCache`] (read-4 — `backend-read-path.md` §3.4) in front of the blob byte-fetch.
pub struct CompositeStore<S: SparqClient, B: BlobStore> {
    sparq: S,
    blob: B,
    /// The `(blob_key, etag)`-keyed blob-body LRU (read-4). Every lookup is keyed with THIS
    /// request's authoritative index metadata, so a hit is provably the bytes that metadata
    /// committed with — never stale, never an authz surface (see [`body_cache`]'s module docs).
    body_cache: BodyCache,
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
        }
    }

    /// Fetch a resource's bytes through its AUTHORITATIVE metadata: body-cache first (keyed by this
    /// request's `(blob_key, etag)` — read-4), then the blob store on a miss (inserting the fetched
    /// bytes so the next same-version read hits). The error mapping is exactly the pre-cache
    /// `blob.get` mapping, so a MISS is byte- and error-identical to the uncached path; a HIT skips
    /// only the blob round-trip (blob-gets/op 1 → 0), never a decision — authorization has already
    /// run upstream of every caller (see [`body_cache`]'s no-bypass argument).
    async fn fetch_body(&self, meta: &ResourceMeta) -> ServerResult<Bytes> {
        if let Some(body) = self.body_cache.get(&meta.blob_key, &meta.etag) {
            return Ok(body);
        }
        let body = self.blob.get(&meta.blob_key).await.map_err(|e| match e {
            // The index says it exists but bytes are missing: a reconciler-class inconsistency.
            BlobError::NotFound => ServerError::Storage("byte/index inconsistency".into()),
            BlobError::QuotaExceeded => ServerError::InsufficientStorage,
            BlobError::Backend(msg) => ServerError::Storage(msg),
        })?;
        self.body_cache.insert(&meta.blob_key, &meta.etag, &body);
        Ok(body)
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
    ///   wins, a read resolves through it to that write's own bytes. The other write's bytes become an
    ///   unreferenced orphan, reclaimed by the reconciler — never a clobber of live bytes.
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
        Ok(format!("{prefix}-{hex}"))
    }

    /// A trivial, deterministic ETag for the slice. M2: derive it from the SPARQ index state so it
    /// participates in the conditional-request CAS (If-None-Match/If-Match).
    fn etag_for(body: &Bytes) -> String {
        format!("\"{}-{}\"", body.len(), fnv1a(body))
    }
}

// [GPT-5.6] The concrete wasm/dev-store pair exposes one combined limits and usage view without
// widening the production backend traits, whose remote stores have backend-specific quota models.
impl CompositeStore<InMemorySparqClient, InMemoryBlobStore> {
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
        let meta = match self.sparq.get_meta(iri).await {
            Ok(m) => m,
            Err(SparqError::NotFound) => return Err(ServerError::NotFound),
            Err(SparqError::QuotaExceeded) => return Err(ServerError::InsufficientStorage),
            Err(SparqError::Backend(e)) => return Err(ServerError::Storage(e)),
        };
        let body = self.fetch_body(&meta).await?;
        Ok(Resource { body, meta })
    }

    async fn meta(&self, iri: &str) -> ServerResult<Option<ResourceMeta>> {
        match self.sparq.get_meta(iri).await {
            Ok(m) => Ok(Some(m)),
            Err(SparqError::NotFound) => Ok(None),
            Err(SparqError::QuotaExceeded) => Err(ServerError::InsufficientStorage),
            Err(SparqError::Backend(e)) => Err(ServerError::Storage(e)),
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
        // a read then resolves through that winner's pointer to ITS bytes, and the loser's bytes become
        // an unreferenced orphan the reconciler GCs (never a clobber of the live bytes). A re-write of
        // an existing resource likewise lands on a NEW key, leaving the previous key orphaned for GC.
        //
        // Minting FAILS CLOSED if the OS RNG is unavailable (rather than minting a weak, possibly-colliding
        // key) — the write errors instead, so the no-two-writes-share-a-key invariant holds even then.
        let blob_key = Self::mint_blob_key(iri)?;
        let etag = Self::etag_for(&body);
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
        self.sparq
            .put_meta(iri, meta.clone())
            .await
            .map_err(|e| match e {
                SparqError::QuotaExceeded => ServerError::InsufficientStorage,
                other => ServerError::Storage(format!("{other}")),
            })?;
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
        // reconciler (M2-next) — the same crash-consistency model `write` documents.
        //
        // The child's blob key is minted UNIQUE PER WRITE (`mint_blob_key`), so a concurrent same-IRI
        // create writes a disjoint object and cannot collide with this one on a shared key. Minting FAILS
        // CLOSED on an unavailable OS RNG (the create errors rather than minting a weak, possibly-colliding
        // key), so the no-shared-key invariant holds even then.
        let blob_key = Self::mint_blob_key(child)?;
        let etag = Self::etag_for(&body);
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
        }
    }

    async fn delete(&self, iri: &str, parent: Option<&str>) -> ServerResult<()> {
        // Look up the byte-pointer from the authoritative index so we delete the right blob.
        let blob_key = match self.sparq.get_meta(iri).await {
            Ok(m) => Some(m.blob_key),
            Err(SparqError::NotFound) => None,
            Err(SparqError::QuotaExceeded) => return Err(ServerError::InsufficientStorage),
            Err(SparqError::Backend(e)) => return Err(ServerError::Storage(e)),
        };
        // Detach from the parent's containment first, then drop the index record, then the bytes.
        // Index-before-bytes keeps the invariant "if it's indexed, its bytes exist" — a crash after
        // the index delete leaves orphaned bytes (the reconciler GCs them — M2-next), never an index
        // row pointing at missing bytes.
        if let Some(p) = parent {
            self.sparq
                .remove_child(p, iri)
                .await
                .map_err(|e| ServerError::Storage(format!("{e}")))?;
        }
        self.sparq
            .delete_meta(iri)
            .await
            .map_err(|e| ServerError::Storage(format!("{e}")))?;
        if let Some(key) = blob_key {
            self.blob
                .delete(&key)
                .await
                .map_err(|e| ServerError::Storage(format!("{e}")))?;
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
        // reconciler: the container's bytes are NOT deleted inline here. After the atomic index delete,
        // the bytes become ORPHANED (no index row references them) and are GC'd by the reconciler's
        // orphaned-bytes sweep. We leave the bytes to the reconciler rather than delete them inline: the
        // blob store is a SEPARATE system (object store) whose `delete` is unconditional. With the
        // UNIQUE-PER-WRITE keys the composite store now mints (`mint_blob_key`), a concurrent same-IRI
        // recreate writes its bytes under a DIFFERENT key, so an inline delete of THIS container's key
        // could no longer clobber a recreate's bytes even if we did it (the root-cause race is closed) —
        // but deleting NO bytes here keeps the path uniform with `write`/`create_in_container`'s
        // orphan-then-GC model and side-effect-free. The trade-off is a transient orphan until a sweep
        // runs: benign (disk space only, never an observable inconsistency), and it IS the documented
        // architecture (SPARQ authoritative; blob store durable bytes; reconciler GCs orphans —
        // `decisions`/the spike crash-consistency model).
        let outcome = self
            .sparq
            .delete_meta_if_empty(iri, parent)
            .await
            .map_err(|e| ServerError::Storage(format!("{e}")))?;
        // NotEmpty / NotFound: nothing was deleted. Deleted: the record AND the parent edge are gone
        // atomically (above); the now-orphaned bytes are the reconciler's responsibility (see above).
        Ok(outcome)
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
            })
    }

    async fn read_at(&self, iri: &str, meta: &ResourceMeta) -> ServerResult<Bytes> {
        // The §3.3 direct byte fetch through the held pointer — NO second `get_meta` — now through
        // the read-4 body cache: the held `(blob_key, etag)` came from THIS request's authoritative
        // read-plan round, so a hit is exactly the bytes that metadata committed with (the unique-
        // per-write blob key names an immutable object). A miss pays the same blob fetch (and the
        // same error mapping) as before. `iri` is not needed here (the pointer is authoritative);
        // it is part of the trait signature so the default (re-read) impl can exist for doubles.
        let _ = iri;
        self.fetch_body(meta).await
    }

    async fn list_children(&self, container: &str) -> ServerResult<Vec<ValidatedChildIri>> {
        let raw = self
            .sparq
            .list_children(container)
            .await
            .map_err(|e| ServerError::Storage(format!("{e}")))?;
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
                    eprintln!(
                        "  STORE: omitting non-RFC-3987 child IRI from list_children: {iri:?}"
                    );
                }
            }
        }
        Ok(out)
    }
}

/// A tiny FNV-1a hash used only for the placeholder ETag (NOT a cryptographic digest).
fn fnv1a(bytes: &[u8]) -> u64 {
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
