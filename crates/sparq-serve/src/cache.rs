//! Response-bytes result cache (research/concurrent-serving.md §5 verdict 2, §6.3).
//!
//! sq-jluc — written while Fable 5 unavailable; re-review when Fable
//! returns. **OPT-IN, OFF by default** (cargo feature `result-cache`).
//!
//! This is the §5-row-2 **RECOMMEND**: a cache from a *request identity* to the
//! complete, pre-serialized response body the engine produced, so a repeated read
//! returns bytes in tens of nanoseconds instead of re-executing (the only route to
//! the Mreq/s envelope of §1). It sits **in front of** `prepare()`/engine dispatch
//! and is a strictly different layer from `sparq-engine`'s `cache.rs` (the embedded
//! whole-query algebra-keyed LRU that lives inside one engine instance); this cache
//! is keyed on *visibility scope* and invalidated by the serving writer's per-pod
//! epoch bumps, neither of which the engine layer knows about.
//!
//! # The key: `(canonical-query, visibility-scope, epoch-vector)` — §6.3
//!
//! - **Canonical-query** ([`canonicalize`](crate::canonicalize)): cheap whitespace (and opt-in
//!   variable) normalization, hashed. So `SELECT ?x WHERE{…}` and the same query
//!   reindented share an entry.
//! - **Visibility-scope** ([`ScopeKey`]): the identity of the *accessible graph
//!   set* a request runs under — `sparq_solid::AuthIndex::accessible(session,
//!   mode)`'s sorted set, hashed. **This is a correctness / access-control
//!   obligation, not a privacy guarantee** (stated honestly below): two sessions
//!   that can see different graph sets get different keys, so cached bytes computed
//!   for one access scope can **never** be served to a different one. We key on the
//!   *scope* (the Hasura minimal-footprint lesson), **never** on the WebID/identity
//!   — many WebIDs share one public-read scope, which is exactly the high-tenancy
//!   key-fragmentation mitigation §6.3 calls for.
//! - **Epoch-vector**: not part of the *lookup* key (that would mint a fresh,
//!   never-reused slot on every write); instead each entry **records** the per-pod
//!   epoch of the graphs its query touched. On read the recorded epochs are checked
//!   against the live generation's [`PodEpochs`](crate::PodEpochs); any advance is a
//!   stale miss and evicts the entry. A query whose footprint is *unbounded* (it
//!   could read graphs we cannot statically enumerate) records the **global
//!   generation** number and is invalidated by ANY write — the conservative,
//!   answer-safe choice.
//!
//! ## Why scope keying is correctness, not privacy (honest boundary)
//!
//! Keying on `accessible` guarantees the cache never *introduces* a cross-scope
//! leak the engine wouldn't already prevent: the bytes stored under a scope were
//! produced by the engine evaluating *under that scope's `DatasetView`*, so they
//! contain only what that scope may see, and they are only ever returned to a
//! request that presents the same scope key. It is an **invariant of this cache**
//! (tested: a different scope MUST miss), enforcing the access-control boundary the
//! auth layer already defines. It is **not** itself a confidentiality mechanism
//! (it makes no cryptographic claim, hides nothing from a side channel, and trusts
//! the caller to derive the scope key faithfully from a *correctly evaluated*
//! `accessible` set — a wrong scope key would be an auth-layer bug, not a cache
//! bug). See the privacy-claims discipline in `AGENTS.md`.
//!
//! # Single-flight / lease dedup (§5 verdict 1, the one MQO survivor)
//!
//! On a miss the first caller takes a **lease** for the key; concurrent callers for
//! the same key **wait** on that lease rather than each re-executing (a stampede
//! becomes one execution, N waiters). [`ResultCache::lease`] returns either
//! [`LeaseOutcome::Hit`] (bytes already cached and fresh), [`LeaseOutcome::Lead`] (you
//! execute, then [`LeadGuard::fulfill`] publishes the bytes and wakes waiters), or
//! [`LeaseOutcome::Wait`] (block on [`WaitGuard::wait`] for the leader's bytes).
//!
//! # Bounds: byte-budget LRU + admission (§6.3)
//!
//! The cache holds at most `max_bytes` of response bodies; inserting evicts
//! least-recently-used entries until it fits. A body larger than
//! `max_entry_bytes` is **never admitted** (oversize/streaming results are not
//! cacheable — they would evict everything else for one entry). Admission returns
//! the bytes to the caller regardless; rejection only means "don't keep it".
//!
//! # Library-first (§6.1)
//!
//! Sync, runtime-agnostic, no HTTP/async types. The cache stores opaque
//! [`Bytes`](std::sync::Arc) blobs and `&str` formats; it never parses a query
//! (only canonicalizes the text) and never touches `sparq-solid` — the caller
//! derives the [`ScopeKey`] and the touched-pod footprint and hands them in.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use rustc_hash::FxHashMap;

use crate::canon;
use crate::epoch::{Epoch, PodEpochs, PodId};

/// A shared, immutable, pre-serialized response body. Cheap to clone (one `Arc`
/// bump) — what Tier 0 can hand straight to the socket without re-serializing.
pub type Bytes = Arc<[u8]>;

/// Default total byte budget for cached bodies: 64 MiB. Tunable via
/// [`CacheConfig`]; the cache never exceeds it (LRU eviction on insert).
pub const DEFAULT_MAX_BYTES: usize = 64 * 1024 * 1024;

/// Default per-entry admission cap: 256 KiB. A body larger than this is never
/// cached (oversize/streaming — §6.3). Tunable via [`CacheConfig`].
pub const DEFAULT_MAX_ENTRY_BYTES: usize = 256 * 1024;

/// Opaque hash of a request's **visibility scope** — the identity of the set of
/// graphs a session may read, NOT the session/WebID (§6.3, the Hasura lesson).
///
/// Construct it from `sparq_solid::AuthIndex::accessible(session, mode)` (an
/// already-sorted `Vec<NamedNode>`) via [`ScopeKey::of_graphs`]. Two sessions with
/// the same accessible set — e.g. every anonymous reader of public graphs —
/// collapse to the **same** scope key, which is the high-tenancy key-fragmentation
/// mitigation. Sessions with different accessible sets get different keys, so the
/// cache can never serve one scope's bytes to another (the isolation invariant).
///
/// `sparq-serve` deliberately does **not** depend on `sparq-solid`: the caller owns
/// the auth layer and passes the derived scope in. A wrong derivation is an
/// auth-layer bug; this type only guarantees that *equal scopes hash equal and
/// distinct scopes (almost surely) hash distinct*.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ScopeKey(u64);

impl ScopeKey {
    /// The scope of a request that runs with **no access control** (the whole
    /// dataset is visible) — a single shared key. Distinct from any restricted
    /// scope produced by [`ScopeKey::of_graphs`] (it is salted differently), so an
    /// unauthenticated full-dataset read never collides with a *restricted* scope
    /// that happens to enumerate the same graphs.
    pub fn unrestricted() -> Self {
        // A fixed sentinel distinct from the domain of hashed graph sets.
        ScopeKey(0x00A1_1FAC_CE55_u64 ^ SCOPE_SALT)
    }

    /// The scope of a request whose accessible set is **empty** (fail-closed: no
    /// auth view, or no matching grants — `sparq-solid`'s empty-visibility case).
    /// All such requests share one key; they all see zero rows, so caching one
    /// empty answer per query is correct and cheap.
    pub fn empty() -> Self {
        ScopeKey::of_graphs(std::iter::empty::<&str>())
    }

    /// Derives a scope key from the accessible graph-IRI set. The set is hashed
    /// **order-independently** (sorted internally), so iteration order does not
    /// matter — equal sets always hash equal. Pass
    /// `accessible(session, mode).iter().map(NamedNode::as_str)`.
    ///
    /// Collision resistance is that of a 64-bit hash: distinct scopes almost surely
    /// differ. A collision would let one scope read another's cached bytes; at
    /// 64 bits this is negligible for the scope cardinalities of a Solid deployment,
    /// but it is an explicit (non-cryptographic) assumption — for a hard isolation
    /// guarantee a deployment can widen the hash here. See the module honesty note.
    pub fn of_graphs<I, S>(graphs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        // Order-independent: each member is hashed and finalized to a well-mixed
        // 64-bit value, then combined into the accumulator with a COMMUTATIVE XOR
        // — so {a,b} and {b,a} agree (the access scope is a SET; iteration order
        // is not part of its identity). The per-member finalize multiply ensures
        // member hashes are spread out before the XOR (raw `DefaultHasher` finish
        // is already well-distributed; the multiply is belt-and-braces against any
        // low-entropy collision). `count` is folded in at the end so a set and a
        // strict superset that XOR-cancel to the same accumulator (a set never has
        // duplicates, but be robust) still differ by cardinality.
        let mut acc: u64 = 0;
        let mut count: u64 = 0;
        for g in graphs {
            let mut h = DefaultHasher::new();
            g.as_ref().hash(&mut h);
            acc ^= h.finish().wrapping_mul(0x9E37_79B9_7F4A_7C15);
            count = count.wrapping_add(1);
        }
        let mut h = DefaultHasher::new();
        acc.hash(&mut h);
        count.hash(&mut h);
        ScopeKey(h.finish() ^ SCOPE_SALT)
    }

    /// The raw 64-bit scope hash (for diagnostics / metrics labels).
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Wraps a pre-computed 64-bit scope hash (for a consumer that derives the
    /// scope identity itself — e.g. a caller that already maintains a stable scope
    /// id). Must satisfy the same contract: equal scopes ⇒ equal value.
    pub fn from_raw(hash: u64) -> Self {
        ScopeKey(hash)
    }
}

/// Salt mixed into every [`ScopeKey`] so a scope hash never collides with a raw
/// query hash or epoch hash used elsewhere (domain separation, cheap).
const SCOPE_SALT: u64 = 0x0005_C05E_5A17_0000;

/// The footprint of a query for invalidation: which pods (named graphs) it read,
/// and whether that footprint is fully known.
///
/// The caller derives this from the parsed algebra's GRAPH/visibility footprint —
/// the same conservative superset the design's §6.3 describes. If the query could
/// read graphs that cannot be statically enumerated (e.g. a `GRAPH ?g {…}` over the
/// whole dataset, or a query with no graph restriction under an unrestricted
/// scope), it is [`Footprint::Unbounded`] and the entry pins the **global
/// generation** — invalidated by any write. Over-approximating the footprint only
/// costs extra (sound) invalidation, never a stale answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Footprint {
    /// The query reads only these pods. The entry records each pod's epoch and is
    /// invalidated when any of them advances. An empty set means the query touches
    /// no pod (a constant query) and is never invalidated by a write.
    Pods(Vec<PodId>),
    /// The footprint is not statically known: pin the global generation, invalidate
    /// on any write (the conservative, answer-safe default).
    Unbounded,
}

/// Configuration for a [`ResultCache`].
#[derive(Clone, Debug)]
pub struct CacheConfig {
    /// Total byte budget for cached bodies; LRU-evict on insert until under it.
    pub max_bytes: usize,
    /// Bodies larger than this are never admitted (oversize/streaming, §6.3).
    pub max_entry_bytes: usize,
    /// Use the label-unsafe variable-renaming canonicalization
    /// ([`canonicalize_renamed`](crate::canonicalize_renamed)). **OFF by default** —
    /// only sound when the consumer re-labels cached bytes to the request's projection
    /// (see [`canonicalize`](crate::canonicalize) for the label-safe default). The
    /// label-safe whitespace-only form is the default.
    pub rename_variables: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        CacheConfig {
            max_bytes: DEFAULT_MAX_BYTES,
            max_entry_bytes: DEFAULT_MAX_ENTRY_BYTES,
            rename_variables: false,
        }
    }
}

/// The lookup identity of a cached response: the canonical-query hash, the
/// visibility-scope, and the result format. The epoch-vector is recorded *in the
/// entry*, not here, so a write reuses the same slot (see the module docs).
///
/// `format` is part of the key because the same query in two result media
/// (`sparql-results+json` vs CSV) is two different byte bodies.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct CacheKey {
    query_hash: u64,
    scope: ScopeKey,
    format: Box<str>,
}

/// What invalidation generation an entry's freshness is checked against.
#[derive(Clone, Debug)]
enum Validity {
    /// Bounded footprint: each touched pod and the epoch it had when cached. Fresh
    /// while every recorded epoch equals the live generation's epoch for that pod.
    Pods(Vec<(PodId, Epoch)>),
    /// Unbounded footprint: the global generation number when cached. Fresh while
    /// the live generation number is unchanged.
    Generation(u64),
}

impl Validity {
    /// Is an entry recorded with this validity still fresh against the live state?
    fn is_fresh(&self, epochs: &PodEpochs, generation: u64) -> bool {
        match self {
            Validity::Pods(recorded) => recorded
                .iter()
                .all(|(pod, e)| epochs.epoch(pod) == *e),
            Validity::Generation(g) => *g == generation,
        }
    }
}

/// One cached body plus its freshness record and LRU stamp.
struct Entry {
    bytes: Bytes,
    validity: Validity,
    /// Monotonic access tick for LRU ordering (the most recently touched entry has
    /// the largest tick).
    last_used: u64,
}

/// A single-flight lease slot: present while a leader is executing the key.
struct Lease {
    /// Set by the leader when it fulfills; waiters read it. `None` while in flight,
    /// `Some(Err)` if the leader dropped without fulfilling (it must re-execute).
    result: Mutex<Option<Bytes>>,
    /// Waiters block here until the leader publishes (or abandons).
    ready: Condvar,
    /// True once the leader has finished (fulfilled or abandoned); distinguishes
    /// "no result yet, keep waiting" from "leader gone, you lead now".
    done: Mutex<bool>,
}

impl Lease {
    fn new() -> Arc<Lease> {
        Arc::new(Lease {
            result: Mutex::new(None),
            ready: Condvar::new(),
            done: Mutex::new(false),
        })
    }
}

/// The response-bytes result cache: byte-budget LRU keyed on
/// `(canonical-query, visibility-scope, format)`, epoch-validated on read,
/// single-flight on miss.
///
/// Cheap to share (`Arc<ResultCache>`); all methods take `&self`. The hot read path
/// takes one mutex (the map) — the spike (§4.1) shows the contention that motivates
/// the eventual arc-swap read snapshot, recorded as a follow-up; v1 ships the
/// correct sharded-map shape and the bench measures the lock cost on a canonical
/// host (this work-box is non-canonical, §below).
pub struct ResultCache {
    config: CacheConfig,
    inner: Mutex<Inner>,
    /// Monotonic LRU clock, bumped on every access. Atomic so reads that only touch
    /// `last_used` don't need the big lock for the tick (they still take it to
    /// mutate the entry, but the tick source is lock-free).
    tick: AtomicU64,
    /// Cheap counters for hit-rate measurement (§6.3 "measure before going deeper").
    hits: AtomicU64,
    misses: AtomicU64,
    stale: AtomicU64,
    inserts: AtomicU64,
    evictions: AtomicU64,
    admission_rejects: AtomicU64,
}

struct Inner {
    map: FxHashMap<CacheKey, Entry>,
    leases: FxHashMap<CacheKey, Arc<Lease>>,
    /// Sum of `bytes.len()` over `map` — the live byte budget usage.
    bytes_used: usize,
}

/// Outcome of [`ResultCache::lease`]: hit, lead, or wait.
pub enum LeaseOutcome {
    /// A fresh body is cached; here it is. No execution needed.
    Hit(Bytes),
    /// You are the single-flight leader: execute the query, then call
    /// [`LeadGuard::fulfill`] with the bytes (or drop the guard to abandon, which
    /// wakes waiters to elect a new leader).
    Lead(LeadGuard),
    /// Another caller is already executing this exact key; block on
    /// [`WaitGuard::wait`] to receive their bytes (or be promoted to leader if they
    /// abandon).
    Wait(WaitGuard),
}

/// Handle for the single-flight leader. On [`LeadGuard::fulfill`] the bytes are
/// inserted (subject to admission) and all waiters are woken with them. Dropping
/// without fulfilling abandons the lease (waiters re-elect a leader).
pub struct LeadGuard {
    cache: Arc<ResultCache>,
    key: CacheKey,
    lease: Arc<Lease>,
    footprint: Footprint,
    fulfilled: bool,
}

impl LeadGuard {
    /// Publishes `bytes` as the result for this key: records the freshness against
    /// the live `epochs`/`generation`, inserts under the byte budget (admission may
    /// reject an oversize body — the bytes are still returned and shared with
    /// waiters), and wakes all waiters. Returns the same `bytes` for the caller's
    /// own use (so the caller writes its response from the shared `Arc`).
    pub fn fulfill(mut self, bytes: Bytes, epochs: &PodEpochs, generation: u64) -> Bytes {
        let validity = self.cache.validity_for(&self.footprint, epochs, generation);
        self.cache
            .complete_lease(&self.key, &self.lease, bytes.clone(), Some((validity, &bytes)));
        self.fulfilled = true;
        bytes
    }
}

impl Drop for LeadGuard {
    fn drop(&mut self) {
        if !self.fulfilled {
            // Abandon: wake waiters with no result so one of them re-leads.
            self.cache.abandon_lease(&self.key, &self.lease);
        }
    }
}

/// Handle for a single-flight waiter.
pub struct WaitGuard {
    cache: Arc<ResultCache>,
    key: CacheKey,
    lease: Arc<Lease>,
}

/// What a waiter gets when it stops waiting.
pub enum WaitOutcome {
    /// The leader fulfilled; here are their bytes.
    Bytes(Bytes),
    /// The leader abandoned without a result; you are promoted to leader — execute
    /// and fulfill via this guard.
    Promoted(LeadGuard),
}

impl WaitGuard {
    /// Blocks until the leader fulfills (returns [`WaitOutcome::Bytes`]) or abandons
    /// (returns [`WaitOutcome::Promoted`] — you must now execute and fulfill).
    pub fn wait(self, footprint: Footprint) -> WaitOutcome {
        // Scope the `done` guard so it is dropped before we move `self.lease` below.
        {
            let mut done = self.lease.done.lock().expect("lease poisoned");
            while !*done {
                done = self.lease.ready.wait(done).expect("lease poisoned");
            }
        }
        let result = self.lease.result.lock().expect("lease poisoned").clone();
        match result {
            Some(bytes) => WaitOutcome::Bytes(bytes),
            None => WaitOutcome::Promoted(LeadGuard {
                cache: Arc::clone(&self.cache),
                key: self.key,
                lease: self.lease,
                footprint,
                fulfilled: false,
            }),
        }
    }
}

/// Snapshot of the cache's measurement counters (§6.3 hit-rate instrumentation).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Fresh cache hits served without execution.
    pub hits: u64,
    /// Lookups with no entry at all (cold miss).
    pub misses: u64,
    /// Lookups that found an entry but it was epoch-stale (and was evicted).
    pub stale: u64,
    /// Bodies inserted into the cache.
    pub inserts: u64,
    /// Entries LRU-evicted to stay under the byte budget.
    pub evictions: u64,
    /// Bodies refused admission for exceeding `max_entry_bytes`.
    pub admission_rejects: u64,
}

impl ResultCache {
    /// Creates a cache with the default configuration.
    pub fn new() -> Arc<Self> {
        Self::with_config(CacheConfig::default())
    }

    /// Creates a cache with an explicit [`CacheConfig`].
    pub fn with_config(config: CacheConfig) -> Arc<Self> {
        Arc::new(ResultCache {
            config,
            inner: Mutex::new(Inner {
                map: FxHashMap::default(),
                leases: FxHashMap::default(),
                bytes_used: 0,
            }),
            tick: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            stale: AtomicU64::new(0),
            inserts: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            admission_rejects: AtomicU64::new(0),
        })
    }

    /// Builds the lookup key for a request: canonicalize the query (per
    /// [`CacheConfig::rename_variables`]), hash it, and combine with the scope and
    /// format. Pure; does no map access.
    fn key(&self, query: &str, scope: ScopeKey, format: &str) -> CacheKey {
        let canonical = if self.config.rename_variables {
            canon::canonicalize_renamed(query)
        } else {
            canon::canonicalize(query)
        };
        let mut h = DefaultHasher::new();
        canonical.hash(&mut h);
        CacheKey {
            query_hash: h.finish(),
            scope,
            format: format.into(),
        }
    }

    /// Pure read-only probe: returns the cached body iff a **fresh** entry exists
    /// for `(query, scope, format)` against the live `epochs`/`generation`. Does
    /// NOT take a lease (use [`ResultCache::lease`] for the single-flight path).
    /// Updates the LRU stamp on a hit and the hit/miss/stale counters.
    pub fn get(
        self: &Arc<Self>,
        query: &str,
        scope: ScopeKey,
        format: &str,
        epochs: &PodEpochs,
        generation: u64,
    ) -> Option<Bytes> {
        let key = self.key(query, scope, format);
        let mut inner = self.inner.lock().expect("result cache poisoned");
        self.lookup_locked(&mut inner, &key, epochs, generation)
    }

    /// Single-flight lease over `(query, scope, format)`. Returns:
    /// - [`LeaseOutcome::Hit`] if a fresh body is cached (no execution);
    /// - [`LeaseOutcome::Lead`] if this caller should execute (and then
    ///   [`LeadGuard::fulfill`]);
    /// - [`LeaseOutcome::Wait`] if another caller is already executing the same key
    ///   (block on [`WaitGuard::wait`]).
    ///
    /// The leader/waiter handshake converts a request stampede on a hot, uncached
    /// key into one execution + N waiters (§5 verdict 1).
    pub fn lease(
        self: &Arc<Self>,
        query: &str,
        scope: ScopeKey,
        format: &str,
        footprint: Footprint,
        epochs: &PodEpochs,
        generation: u64,
    ) -> LeaseOutcome {
        let key = self.key(query, scope, format);
        let mut inner = self.inner.lock().expect("result cache poisoned");
        // Fresh hit short-circuits the lease entirely.
        if let Some(bytes) = self.lookup_locked(&mut inner, &key, epochs, generation) {
            return LeaseOutcome::Hit(bytes);
        }
        // Existing lease ⇒ wait. New lease ⇒ lead.
        if let Some(lease) = inner.leases.get(&key) {
            let lease = Arc::clone(lease);
            drop(inner);
            LeaseOutcome::Wait(WaitGuard {
                cache: Arc::clone(self),
                key,
                lease,
            })
        } else {
            let lease = Lease::new();
            inner.leases.insert(key.clone(), Arc::clone(&lease));
            drop(inner);
            LeaseOutcome::Lead(LeadGuard {
                cache: Arc::clone(self),
                key,
                lease,
                footprint,
                fulfilled: false,
            })
        }
    }

    /// Look up `key` under the held lock; on a fresh hit bump LRU + hit counter and
    /// return the bytes; on a stale hit evict + bump stale; on absence bump miss.
    fn lookup_locked(
        self: &Arc<Self>,
        inner: &mut Inner,
        key: &CacheKey,
        epochs: &PodEpochs,
        generation: u64,
    ) -> Option<Bytes> {
        match inner.map.get(key) {
            Some(entry) if entry.validity.is_fresh(epochs, generation) => {
                let bytes = entry.bytes.clone();
                let tick = self.tick.fetch_add(1, Ordering::Relaxed);
                // Re-borrow mutably to stamp; key is known present.
                if let Some(e) = inner.map.get_mut(key) {
                    e.last_used = tick;
                }
                self.hits.fetch_add(1, Ordering::Relaxed);
                Some(bytes)
            }
            Some(_) => {
                // Stale: an epoch advanced since this was cached — evict it so a
                // re-execution can replace it, and report a (non-fresh) miss.
                if let Some(e) = inner.map.remove(key) {
                    inner.bytes_used -= e.bytes.len();
                }
                self.stale.fetch_add(1, Ordering::Relaxed);
                None
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// Derives the [`Validity`] record for a footprint against the live state.
    fn validity_for(&self, footprint: &Footprint, epochs: &PodEpochs, generation: u64) -> Validity {
        match footprint {
            Footprint::Pods(pods) => Validity::Pods(
                pods.iter()
                    .map(|p| (p.clone(), epochs.epoch(p)))
                    .collect(),
            ),
            Footprint::Unbounded => Validity::Generation(generation),
        }
    }

    /// Completes a lease: optionally insert the body (admission + LRU), then publish
    /// to waiters and remove the lease. `payload` is `Some((validity, &bytes))` to
    /// insert, `None` to abandon (no insert, waiters re-lead).
    fn complete_lease(
        &self,
        key: &CacheKey,
        lease: &Arc<Lease>,
        bytes: Bytes,
        payload: Option<(Validity, &Bytes)>,
    ) {
        let mut inner = self.inner.lock().expect("result cache poisoned");
        if let Some((validity, _)) = payload {
            self.insert_locked(&mut inner, key, bytes.clone(), validity);
        }
        inner.leases.remove(key);
        drop(inner);
        // Publish to waiters: set the result, mark done, wake all.
        *lease.result.lock().expect("lease poisoned") = Some(bytes);
        *lease.done.lock().expect("lease poisoned") = true;
        lease.ready.notify_all();
    }

    /// Abandons a lease without a result: remove it and wake waiters with `None`
    /// (one of them is promoted to leader by [`WaitGuard::wait`]).
    fn abandon_lease(&self, key: &CacheKey, lease: &Arc<Lease>) {
        let mut inner = self.inner.lock().expect("result cache poisoned");
        inner.leases.remove(key);
        drop(inner);
        *lease.done.lock().expect("lease poisoned") = true; // result stays None
        lease.ready.notify_all();
    }

    /// Inserts a body under the byte budget. Admission: an oversize body is dropped
    /// (counter bumped) — it is still returned to the caller and shared with
    /// waiters, just not retained. On insert, LRU-evict until the new total fits.
    fn insert_locked(&self, inner: &mut Inner, key: &CacheKey, bytes: Bytes, validity: Validity) {
        let len = bytes.len();
        if len > self.config.max_entry_bytes {
            self.admission_rejects.fetch_add(1, Ordering::Relaxed);
            return;
        }
        // Replacing an existing key: reclaim its bytes first.
        if let Some(old) = inner.map.remove(key) {
            inner.bytes_used -= old.bytes.len();
        }
        // Evict LRU until this body fits within the budget. A single entry can
        // never exceed max_bytes here because max_entry_bytes <= max_bytes is the
        // sane config; if mis-configured we still stop when the map is empty.
        while inner.bytes_used + len > self.config.max_bytes && !inner.map.is_empty() {
            self.evict_one_locked(inner);
        }
        if inner.bytes_used + len > self.config.max_bytes {
            // Even an empty cache can't hold it (max_entry_bytes > max_bytes
            // mis-config): refuse admission rather than overflow the budget.
            self.admission_rejects.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let tick = self.tick.fetch_add(1, Ordering::Relaxed);
        inner.map.insert(
            key.clone(),
            Entry {
                bytes,
                validity,
                last_used: tick,
            },
        );
        inner.bytes_used += len;
        self.inserts.fetch_add(1, Ordering::Relaxed);
    }

    /// Evicts the least-recently-used entry (smallest `last_used`). Caller ensures
    /// the map is non-empty.
    fn evict_one_locked(&self, inner: &mut Inner) {
        if let Some(victim) = inner
            .map
            .iter()
            .min_by_key(|(_, e)| e.last_used)
            .map(|(k, _)| k.clone())
        {
            if let Some(e) = inner.map.remove(&victim) {
                inner.bytes_used -= e.bytes.len();
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Current number of cached bodies.
    pub fn len(&self) -> usize {
        self.inner.lock().expect("result cache poisoned").map.len()
    }

    /// True if no body is cached.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().expect("result cache poisoned").map.is_empty()
    }

    /// Current total bytes held by cached bodies (≤ `max_bytes`).
    pub fn bytes_used(&self) -> usize {
        self.inner
            .lock()
            .expect("result cache poisoned")
            .bytes_used
    }

    /// Snapshot of the measurement counters (§6.3 hit-rate instrumentation).
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            stale: self.stale.load(Ordering::Relaxed),
            inserts: self.inserts.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            admission_rejects: self.admission_rejects.load(Ordering::Relaxed),
        }
    }

    /// Empties the cache (bodies and leases). For tests and explicit flushes.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().expect("result cache poisoned");
        inner.map.clear();
        inner.leases.clear();
        inner.bytes_used = 0;
    }
}
