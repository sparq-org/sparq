// AUTHORED-BY Claude Fable 5
//! Deterministic backend round-trip counters at the [`SparqClient`] / [`BlobStore`] seams —
//! read-1 of the read-path perf plan (`research/lws-design-records.md` §7).
//!
//! ## What this measures (and why it is deterministic)
//! [`CountingSparqClient`] / [`CountingBlobStore`] are transparent decorators that count, per trait
//! method, **the number of backend protocol requests the LIVE client issues for that method** —
//! verified against `super::http::HttpSparqClient` (a code span — the module is behind the opt-in
//! `http-sparq` feature, so an intra-doc link would break the default build's rustdoc), where
//! every read method is exactly ONE SPARQL
//! Protocol query (the protocol permits exactly one query string per request, so there is no
//! request-level batching to blur the count). The counts are integers derived from the code path
//! taken, not from timing — so a test can PIN them exactly (the repo's perf-gate discipline:
//! deterministic metrics hard, wall-clock advisory).
//!
//! ## The await-depth (sequential-RTT) witness
//! Both decorators share one [`BackendCounters`] and wrap every backend call in an in-flight
//! guard. `max_in_flight` is the high-water mark of concurrently-outstanding backend calls: when it
//! reads **1** for an operation, every backend call strictly awaited the previous one — so the
//! operation's **sequential RTT depth equals its total backend-call count**
//! (`sparql_queries + sparql_updates + blob ops`). That is the §1.1 "sequential RTT depth" column,
//! measured rather than asserted.
//!
//! **The global mark is a lifetime high-water, so a per-op reading MUST be operation-scoped:**
//! measure a window with [`BackendCounters::measure`], which hands out a [`MeasureScope`] owning its
//! OWN peak cell (every backend call `fetch_max`es the live in-flight count into every open window's
//! cell). Do NOT read a raw `snapshot().since()` `max_in_flight` per op — that carries the global
//! high-water, which a prior, already-completed OVERLAPPING op leaves >1, permanently contaminating
//! every later strictly-sequential reading. Per-scope cells (not a destructively-reset shared mark)
//! make the witness sound under real concurrency — overlapping windows each get their own correct
//! peak, and one window can never erase another's.
//!
//! ## Query-count mapping (per [`SparqClient`] method, mirroring `HttpSparqClient`)
//! - `get_meta` / `exists` / `list_children` / `referenced_blob_keys` / `read_plan`: **1 query**
//!   (`read_plan` is the ONE combined read-plan SELECT on the live client — §3.1; the in-memory
//!   double answers it in one atomic index pass, so the 1-query model holds for it too).
//! - `put_meta` / `delete_meta` / `remove_child`: **1 update**.
//! - `create_child`: **1 update + 1 query** (the guarded insert, then the create-marker ASK).
//! - `delete_meta_if_empty`: **1 update**, then **1 query** (marker ASK) when the outcome is
//!   `Deleted`, else **2 queries** (marker ASK + exists ASK).
//!
//! Error paths are counted best-effort (the live client may abort mid-sequence); the pinned tests
//! only assert success paths, where the mapping is exact.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;

use super::blob::{BlobEntry, BlobError, BlobStore};
use super::sparq::{DeleteOutcome, ReadPlan, ResourceMeta, SparqClient, SparqError};

/// Shared, lock-free counters for the backend seams. Cheap to clone via `Arc`; a test holds the
/// `Arc` and diffs [`snapshot`](BackendCounters::snapshot)s around one operation.
#[derive(Debug, Default)]
pub struct BackendCounters {
    /// SPARQL Protocol **query** requests (SELECT / ASK / CONSTRUCT) the live client would issue.
    sparql_queries: AtomicU64,
    /// SPARQL Protocol **update** requests.
    sparql_updates: AtomicU64,
    /// Blob-store byte fetches (`get`).
    blob_gets: AtomicU64,
    /// Blob-store byte writes (`put`).
    blob_puts: AtomicU64,
    /// Every other blob-store call (`exists` / `delete` / `list` / `stat` / CAS-delete).
    blob_others: AtomicU64,
    /// GLOBAL monotonic high-water mark of concurrency (never reset) — the lifetime peak, reported by
    /// the raw [`snapshot`](BackendCounters::snapshot). Written under the `conc` lock (below), read
    /// lock-free here for observability. For a PER-OP reading use [`measure`], not this.
    max_in_flight: AtomicU64,
    /// The concurrency state that MUST move as one unit: the current in-flight count AND every active
    /// measurement window's peak cell, behind ONE lock. `op_guard` bumps `in_flight` and fans the new
    /// count into `max_in_flight` + every scope cell in a SINGLE critical section, and
    /// [`MeasureScope::delta`] reads its cell under the same lock — so no reader can observe an
    /// in-flight increment that has not yet reached the scope peaks (the boundary race a lock-free
    /// increment + a separate scopes lock had), and no window can erase another's peak.
    conc: Mutex<Conc>,
    /// TEST-ONLY instrumentation seam (zero-cost in non-test builds: the field does not exist).
    /// When a test installs a flag here, `op_guard` sets it IMMEDIATELY BEFORE attempting the
    /// `conc` lock — i.e. after everything `op_guard` does outside the critical section (which is
    /// nothing; that emptiness is exactly what the boundary-race test pins). A test holding the
    /// lock can therefore wait on this flag as a DETERMINISTIC proof that another thread has
    /// reached the lock-acquisition boundary and is blocked there, instead of sleeping.
    /// Installable and replaceable per test (not one-shot), so tests sharing a counters
    /// instance can each install their own probe.
    #[cfg(test)]
    boundary_probe: std::sync::Mutex<Option<Arc<std::sync::atomic::AtomicBool>>>,
}

/// The lock-protected concurrency state (see [`BackendCounters::conc`]): the live in-flight count +
/// the peak cell of every active measurement window. Kept together so the in-flight increment and
/// the per-window peak fan-out are ONE atomic-with-respect-to-the-lock step.
#[derive(Debug, Default)]
struct Conc {
    /// Currently-outstanding backend calls (both seams).
    in_flight: u64,
    /// One peak cell per active [`MeasureScope`] (registered by `measure`, removed on scope drop).
    scopes: Vec<Arc<AtomicU64>>,
}

/// A point-in-time copy of the counters. Subtract two with [`CounterSnapshot::since`] to get the
/// per-operation deltas a test pins. `max_in_flight` is NOT differenced (it is a high-water mark);
/// `since` carries the LATER snapshot's GLOBAL high-water for the counters' lifetime. For a sound
/// OPERATION-SCOPED peak use [`BackendCounters::measure`] + [`MeasureScope::delta`] (which reads the
/// window's OWN peak cell), not a raw `snapshot().since()` — the latter's `max_in_flight` can be
/// contaminated by a prior, already-completed overlapping op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterSnapshot {
    pub sparql_queries: u64,
    pub sparql_updates: u64,
    pub blob_gets: u64,
    pub blob_puts: u64,
    pub blob_others: u64,
    pub max_in_flight: u64,
}

impl CounterSnapshot {
    /// The per-operation delta `self - earlier` (counter fields), keeping `self`'s high-water mark.
    pub fn since(&self, earlier: &CounterSnapshot) -> CounterSnapshot {
        CounterSnapshot {
            sparql_queries: self.sparql_queries - earlier.sparql_queries,
            sparql_updates: self.sparql_updates - earlier.sparql_updates,
            blob_gets: self.blob_gets - earlier.blob_gets,
            blob_puts: self.blob_puts - earlier.blob_puts,
            blob_others: self.blob_others - earlier.blob_others,
            max_in_flight: self.max_in_flight,
        }
    }

    /// Total backend calls in this (delta) snapshot — with `max_in_flight == 1` this IS the
    /// operation's sequential RTT depth (every call awaited the previous one).
    pub fn total_backend_ops(&self) -> u64 {
        self.sparql_queries
            + self.sparql_updates
            + self.blob_gets
            + self.blob_puts
            + self.blob_others
    }
}

impl BackendCounters {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn snapshot(&self) -> CounterSnapshot {
        CounterSnapshot {
            sparql_queries: self.sparql_queries.load(Ordering::Relaxed),
            sparql_updates: self.sparql_updates.load(Ordering::Relaxed),
            blob_gets: self.blob_gets.load(Ordering::Relaxed),
            blob_puts: self.blob_puts.load(Ordering::Relaxed),
            blob_others: self.blob_others.load(Ordering::Relaxed),
            max_in_flight: self.max_in_flight.load(Ordering::Relaxed),
        }
    }

    /// Open an OPERATION-SCOPED measurement window with its OWN peak cell. The window's
    /// [`MeasureScope::delta`] yields the counter deltas since open AND *this window's* peak
    /// concurrency — never a global high-water, and never a value another scope could erase. This is
    /// the sound way to read per-op await-depth under concurrency (not just in isolated tests): each
    /// window's peak is an INDEPENDENT cell that `op_guard` `fetch_max`es into, so nothing is reset.
    ///
    /// The cell is initialised — UNDER the `conc` lock — to the concurrency ALREADY outstanding at
    /// open (genuine concurrency counts), so it cannot race an `op_guard`: an in-flight bump is
    /// either already reflected in the `in_flight` read here, or lands on this cell via `op_guard`'s
    /// fan-out (which takes the same lock) afterwards. Hold the scope across the measured
    /// operation(s), then call `delta()`; the cell is de-registered on drop.
    pub fn measure(&self) -> MeasureScope<'_> {
        let mut conc = self.conc.lock().expect("conc lock poisoned");
        let peak = Arc::new(AtomicU64::new(conc.in_flight));
        conc.scopes.push(Arc::clone(&peak));
        drop(conc);
        MeasureScope {
            counters: self,
            peak,
            start: self.snapshot(),
        }
    }

    /// RAII in-flight guard. In ONE critical section under the `conc` lock it bumps `in_flight` and
    /// fans the resulting count into the GLOBAL high-water AND every ACTIVE window's peak cell — so a
    /// reader can NEVER observe an in-flight increment that has not yet reached the scope peaks (the
    /// boundary race a lock-free increment + a separate scopes lock had). Two overlapping guards ⇒
    /// each open window's peak ≥ 2. The lock is released BEFORE the backend call runs (only the
    /// counter arithmetic is under it — deadlock-free, never held across an `.await`).
    fn op_guard(self: &Arc<Self>) -> OpGuard {
        // TEST-ONLY: announce arrival at the lock-acquisition boundary (see `boundary_probe`).
        // This is the LAST thing before the lock attempt, so once a test observes the flag, every
        // pre-lock effect of `op_guard` (there must be none — the invariant under test) has
        // already happened-before the observation.
        #[cfg(test)]
        if let Some(probe) = self
            .boundary_probe
            .lock()
            .expect("boundary probe lock poisoned")
            .as_ref()
        {
            probe.store(true, Ordering::SeqCst);
        }
        let mut conc = self.conc.lock().expect("conc lock poisoned");
        conc.in_flight += 1;
        let now = conc.in_flight;
        self.max_in_flight.fetch_max(now, Ordering::SeqCst);
        for peak in &conc.scopes {
            peak.fetch_max(now, Ordering::SeqCst);
        }
        drop(conc);
        OpGuard {
            counters: Arc::clone(self),
        }
    }

    fn count_queries(&self, n: u64) {
        self.sparql_queries.fetch_add(n, Ordering::Relaxed);
    }
    fn count_update(&self) {
        self.sparql_updates.fetch_add(1, Ordering::Relaxed);
    }
}

/// An operation-scoped measurement window (from [`BackendCounters::measure`]). Owns an INDEPENDENT
/// peak cell that every `op_guard` `fetch_max`es into for the window's lifetime, so
/// [`delta`](Self::delta) reports the peak concurrency DURING this window only — with no shared
/// high-water reset, and no way for a concurrent scope to erase this window's real peak.
pub struct MeasureScope<'a> {
    counters: &'a BackendCounters,
    /// This window's OWN peak-concurrency cell (registered in `counters.conc.scopes` for its life).
    peak: Arc<AtomicU64>,
    start: CounterSnapshot,
}

impl MeasureScope<'_> {
    /// The window's deltas: counter fields since the window start, and the op-scoped peak
    /// `max_in_flight` from THIS window's own cell (NOT the global high-water).
    ///
    /// The peak is read UNDER the `conc` lock, so it cannot observe a half-applied `op_guard`
    /// critical section (an in-flight increment whose peak fan-out has not yet run) — the boundary
    /// race. Holding the lock means every completed increment's fan-out is already visible; the load
    /// then returns the settled peak. The lock is dropped before the (sync, lock-free) counter
    /// snapshot, so nothing is held across other work.
    pub fn delta(&self) -> CounterSnapshot {
        let peak = {
            let conc = self.counters.conc.lock().expect("conc lock poisoned");
            let p = self.peak.load(Ordering::SeqCst);
            drop(conc);
            p
        };
        let mut d = self.counters.snapshot().since(&self.start);
        d.max_in_flight = peak;
        d
    }
}

impl Drop for MeasureScope<'_> {
    fn drop(&mut self) {
        // De-register this window's peak cell (by identity) so `op_guard` stops updating it.
        self.counters
            .conc
            .lock()
            .expect("conc lock poisoned")
            .scopes
            .retain(|p| !Arc::ptr_eq(p, &self.peak));
    }
}

struct OpGuard {
    counters: Arc<BackendCounters>,
}

impl Drop for OpGuard {
    fn drop(&mut self) {
        // Decrement in_flight under the SAME lock op_guard increments it (the count only ever moves
        // under `conc`). The peak cells are high-water marks, so a decrement never lowers them.
        self.counters
            .conc
            .lock()
            .expect("conc lock poisoned")
            .in_flight -= 1;
    }
}

/// A [`SparqClient`] decorator counting the SPARQL Protocol requests each method costs on the live
/// client (see the module docs for the mapping). Fully transparent: every call forwards to `inner`.
pub struct CountingSparqClient<S: SparqClient> {
    inner: S,
    counters: Arc<BackendCounters>,
}

impl<S: SparqClient> CountingSparqClient<S> {
    pub fn new(inner: S, counters: Arc<BackendCounters>) -> Self {
        Self { inner, counters }
    }
}

#[async_trait]
impl<S: SparqClient> SparqClient for CountingSparqClient<S> {
    async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError> {
        let _g = self.counters.op_guard();
        self.counters.count_queries(1);
        self.inner.get_meta(iri).await
    }

    async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
        let _g = self.counters.op_guard();
        self.counters.count_update();
        self.inner.put_meta(iri, meta).await
    }

    async fn exists(&self, iri: &str) -> Result<bool, SparqError> {
        let _g = self.counters.op_guard();
        self.counters.count_queries(1);
        self.inner.exists(iri).await
    }

    async fn delete_meta(&self, iri: &str) -> Result<(), SparqError> {
        let _g = self.counters.op_guard();
        self.counters.count_update();
        self.inner.delete_meta(iri).await
    }

    async fn delete_meta_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<DeleteOutcome, SparqError> {
        let _g = self.counters.op_guard();
        // The live client: 1 guarded update, then the marker ASK; a non-Deleted outcome needs the
        // second (exists) ASK to split NotEmpty from NotFound.
        self.counters.count_update();
        let outcome = self.inner.delete_meta_if_empty(iri, parent).await?;
        self.counters
            .count_queries(if outcome == DeleteOutcome::Deleted {
                1
            } else {
                2
            });
        Ok(outcome)
    }

    async fn create_child(
        &self,
        container: &str,
        child: &str,
        meta: ResourceMeta,
    ) -> Result<(), SparqError> {
        let _g = self.counters.op_guard();
        // The live client: 1 guarded update + 1 create-marker ASK (issued for BOTH the created and
        // the container-missing outcome — the ASK is how NotFound is learned).
        self.counters.count_update();
        self.counters.count_queries(1);
        self.inner.create_child(container, child, meta).await
    }

    async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError> {
        let _g = self.counters.op_guard();
        self.counters.count_update();
        self.inner.remove_child(container, child).await
    }

    async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError> {
        let _g = self.counters.op_guard();
        self.counters.count_queries(1);
        self.inner.list_children(container).await
    }

    async fn referenced_blob_keys(&self) -> Result<HashSet<String>, SparqError> {
        let _g = self.counters.op_guard();
        self.counters.count_queries(1);
        self.inner.referenced_blob_keys().await
    }

    async fn read_plan(
        &self,
        target: &str,
        acl_candidates: &[String],
    ) -> Result<ReadPlan, SparqError> {
        let _g = self.counters.op_guard();
        // ONE combined SELECT on the live client (§3.1) — the read-2 win this decorator exists to
        // evidence. Forwarded to `inner` (never the default loop), so the wrapped client's
        // one-round-trip override is what actually answers.
        self.counters.count_queries(1);
        self.inner.read_plan(target, acl_candidates).await
    }
}

/// A [`BlobStore`] decorator counting byte fetches/writes (and every other backend call) against
/// the shared [`BackendCounters`].
pub struct CountingBlobStore<B: BlobStore> {
    inner: B,
    counters: Arc<BackendCounters>,
}

impl<B: BlobStore> CountingBlobStore<B> {
    pub fn new(inner: B, counters: Arc<BackendCounters>) -> Self {
        Self { inner, counters }
    }
}

#[async_trait]
impl<B: BlobStore> BlobStore for CountingBlobStore<B> {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        let _g = self.counters.op_guard();
        self.counters.blob_gets.fetch_add(1, Ordering::Relaxed);
        self.inner.get(key).await
    }

    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        let _g = self.counters.op_guard();
        self.counters.blob_puts.fetch_add(1, Ordering::Relaxed);
        self.inner.put(key, body).await
    }

    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        let _g = self.counters.op_guard();
        self.counters.blob_others.fetch_add(1, Ordering::Relaxed);
        self.inner.exists(key).await
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        let _g = self.counters.op_guard();
        self.counters.blob_others.fetch_add(1, Ordering::Relaxed);
        self.inner.delete(key).await
    }

    async fn list(&self) -> Result<Vec<BlobEntry>, BlobError> {
        let _g = self.counters.op_guard();
        self.counters.blob_others.fetch_add(1, Ordering::Relaxed);
        self.inner.list().await
    }

    async fn stat(&self, key: &str) -> Result<Option<BlobEntry>, BlobError> {
        let _g = self.counters.op_guard();
        self.counters.blob_others.fetch_add(1, Ordering::Relaxed);
        self.inner.stat(key).await
    }

    async fn delete_if_unchanged(
        &self,
        key: &str,
        expected_generation: u64,
    ) -> Result<bool, BlobError> {
        let _g = self.counters.op_guard();
        self.counters.blob_others.fetch_add(1, Ordering::Relaxed);
        self.inner
            .delete_if_unchanged(key, expected_generation)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{InMemoryBlobStore, InMemorySparqClient};

    #[tokio::test]
    async fn counters_pin_the_per_method_mapping() {
        let counters = BackendCounters::new();
        let sparq = CountingSparqClient::new(InMemorySparqClient::new(), Arc::clone(&counters));
        let blob = CountingBlobStore::new(InMemoryBlobStore::new(), Arc::clone(&counters));

        let meta = ResourceMeta {
            content_type: "text/turtle".into(),
            blob_key: "k1".into(),
            etag: "\"e1\"".into(),
            last_modified: None,
        };
        sparq.put_meta("https://p/c/", meta.clone()).await.unwrap();
        let s0 = counters.snapshot();
        assert_eq!((s0.sparql_updates, s0.sparql_queries), (1, 0));

        sparq.get_meta("https://p/c/").await.unwrap();
        sparq.exists("https://p/c/").await.unwrap();
        sparq.list_children("https://p/c/").await.unwrap();
        let d = counters.snapshot().since(&s0);
        assert_eq!(d.sparql_queries, 3, "3 read methods = 3 queries");
        assert_eq!(d.sparql_updates, 0);

        blob.put("k1", Bytes::from_static(b"x")).await.unwrap();
        blob.get("k1").await.unwrap();
        let d2 = counters.snapshot().since(&s0);
        assert_eq!((d2.blob_puts, d2.blob_gets), (1, 1));

        // create_child = 1 update + 1 (marker-ASK-modelled) query.
        let before = counters.snapshot();
        sparq
            .create_child("https://p/c/", "https://p/c/doc", meta.clone())
            .await
            .unwrap();
        let d3 = counters.snapshot().since(&before);
        assert_eq!((d3.sparql_updates, d3.sparql_queries), (1, 1));

        // delete_meta_if_empty on a NON-empty container: 1 update + 2 queries (marker + exists ASK).
        let before = counters.snapshot();
        let outcome = sparq
            .delete_meta_if_empty("https://p/c/", None)
            .await
            .unwrap();
        assert_eq!(outcome, DeleteOutcome::NotEmpty);
        let d4 = counters.snapshot().since(&before);
        assert_eq!((d4.sparql_updates, d4.sparql_queries), (1, 2));

        // Sequential calls throughout ⇒ the await-depth witness stays 1.
        assert_eq!(counters.snapshot().max_in_flight, 1);
    }

    #[tokio::test]
    async fn max_in_flight_detects_overlap() {
        let counters = BackendCounters::new();
        let sparq = Arc::new(CountingSparqClient::new(
            InMemorySparqClient::new(),
            Arc::clone(&counters),
        ));
        // Two concurrent backend calls ⇒ the high-water mark exceeds 1 (the witness is live, not
        // vacuously 1). `join!` polls both futures in one task; each holds its guard across an
        // `.await` on the in-memory client, so the overlap is observable deterministically… the
        // in-memory client has no await points inside the lock, so drive overlap explicitly:
        // hold one guard while issuing a call.
        let g = counters.op_guard();
        sparq.exists("https://p/x").await.unwrap();
        drop(g);
        assert!(counters.snapshot().max_in_flight >= 2);
    }

    #[tokio::test]
    async fn scoped_measurement_is_not_contaminated_by_a_prior_overlap() {
        // The MEASUREMENT fix (roborev Medium): a scoped `measure()` window must report only ITS
        // OWN peak concurrency, not the counters' lifetime global high-water. Here a PRIOR op
        // overlaps (global max → 2); a later, strictly-SEQUENTIAL op measured via `measure()` must
        // still read `max_in_flight == 1`.
        let counters = BackendCounters::new();
        let sparq = CountingSparqClient::new(InMemorySparqClient::new(), Arc::clone(&counters));

        // Prior overlap → the GLOBAL high-water is now 2.
        {
            let g = counters.op_guard();
            sparq.exists("https://p/prior").await.unwrap();
            drop(g);
        }
        assert!(
            counters.snapshot().max_in_flight >= 2,
            "sanity: the prior overlap raised the GLOBAL high-water"
        );

        // A later strictly-sequential op, measured in its OWN scope, sees peak = 1 (not the stale 2).
        let scope = counters.measure();
        sparq.exists("https://p/a").await.unwrap();
        sparq.exists("https://p/b").await.unwrap();
        let d = scope.delta();
        assert_eq!(d.sparql_queries, 2, "two sequential queries in the window");
        assert_eq!(
            d.max_in_flight, 1,
            "op-scoped peak is 1 — the prior overlap must NOT contaminate this window: {d:?}"
        );

        // CONTRAST — an UNSCOPED reading of the same sequential window IS contaminated: capture the
        // global high-water BEFORE opening the scope (still 2 from the prior overlap) and diff with a
        // raw since(), which carries the later snapshot's global mark. This is exactly the bug
        // `measure()` fixes (and why the harness must use `measure()`, not `snapshot().since()`).
        let counters2 = BackendCounters::new();
        let sparq2 = CountingSparqClient::new(InMemorySparqClient::new(), Arc::clone(&counters2));
        {
            let g = counters2.op_guard();
            sparq2.exists("https://p/prior").await.unwrap();
            drop(g);
        }
        let before = counters2.snapshot(); // global mark already 2 — no reset
        sparq2.exists("https://p/a").await.unwrap(); // one strictly-sequential call
        let raw = counters2.snapshot().since(&before);
        assert_eq!(raw.sparql_queries, 1);
        assert!(
            raw.max_in_flight >= 2,
            "unscoped since() is contaminated by the prior overlap ({raw:?}) — the reason measure() exists"
        );
    }

    #[tokio::test]
    async fn overlapping_scopes_each_keep_their_own_peak_no_reset_erasure() {
        // The MEASUREMENT Medium (per-scope cells, not a destructively-reset shared mark): a REAL
        // peak raised during window A must NEVER be erased by another window B opening. The old
        // `reset_max_in_flight` stored the current in-flight count into a SHARED cell on every
        // `measure()`, so B opening while quiescent would overwrite A's real peak with 0. This test
        // pins the correct per-scope behaviour (and would FAIL against that reset design).
        let counters = BackendCounters::new();

        // Window A opens while quiescent (its own peak cell starts at 0).
        let scope_a = counters.measure();
        // A REAL peak of 2 occurs DURING A (two concurrent in-flight guards), then clears.
        {
            let _g1 = counters.op_guard(); // in_flight 1 → A's cell fetch_max 1
            let _g2 = counters.op_guard(); // in_flight 2 → A's cell fetch_max 2
        }
        // Window B opens while quiescent (in_flight back to 0). A shared-reset design would store 0
        // here, ERASING A's peak; independent per-scope cells do not.
        let scope_b = counters.measure();

        assert_eq!(
            scope_a.delta().max_in_flight,
            2,
            "window A's real peak of 2 must survive window B opening (no shared-reset erasure)"
        );
        assert_eq!(
            scope_b.delta().max_in_flight,
            0,
            "window B saw no in-flight calls of its own ⇒ its own peak is 0"
        );
    }

    #[tokio::test]
    async fn scope_opened_during_in_flight_counts_the_outstanding_concurrency() {
        // A window opened while calls are already outstanding counts that GENUINE concurrency in its
        // peak (only STALE history is excluded). This is the coordinator's "measure() concurrent
        // with a backend call" race made deterministic: opening with 2 in flight ⇒ peak ≥ 2, and a
        // later `op_guard` during the window still lands on this window's own cell.
        let counters = BackendCounters::new();
        let sparq = CountingSparqClient::new(InMemorySparqClient::new(), Arc::clone(&counters));

        let g1 = counters.op_guard(); // in_flight 1
        let g2 = counters.op_guard(); // in_flight 2
        let scope = counters.measure(); // opens with 2 outstanding → own cell initialised to 2
        drop(g2);
        drop(g1);
        // A further sequential call inside the window updates THIS window's cell (still ≤ its peak).
        sparq.exists("https://p/x").await.unwrap();
        assert_eq!(
            scope.delta().max_in_flight,
            2,
            "a window opened with 2 outstanding calls counts them as its peak"
        );
    }

    /// The BOUNDARY-race regression (roborev Medium). A DOCUMENTED TARGETED barrier test (not a pure
    /// scheduler-adversarial one): it holds the `conc` critical-section lock on the test thread and
    /// spawns a real OS thread whose backend op must BLOCK entering `op_guard`. Because the in-flight
    /// increment AND the per-scope peak fan-out are now ONE critical section under that lock, while
    /// the lock is held NEITHER moves — there is no "in_flight raised but peak not yet updated"
    /// window a reader could catch. The old design incremented `in_flight` LOCK-FREE before taking a
    /// separate scopes lock, so here it would show `in_flight == 1` while the scope peak was still 0
    /// (the boundary under-report) — this test's `in_flight == 0` assertion pins that against
    /// regression.
    ///
    /// Readiness is DETERMINISTIC, not sleep-based: the test installs the cfg(test)-only
    /// `boundary_probe`, which `op_guard` sets as its very last action BEFORE attempting the lock.
    /// The main thread spins until the probe fires, so by the time it asserts, the worker has
    /// provably executed ALL of `op_guard`'s pre-lock code (i.e. actually attempted the boundary) —
    /// any regressed pre-lock counter mutation would be sequenced-before the probe store and thus
    /// visible to the assertions. No scheduler timing is relied on.
    #[test]
    fn boundary_increment_and_peak_update_are_one_critical_section() {
        use std::sync::atomic::AtomicBool;
        use std::thread;

        let counters = BackendCounters::new();
        // Install the boundary probe BEFORE any op_guard can run.
        let probe = Arc::new(AtomicBool::new(false));
        *counters
            .boundary_probe
            .lock()
            .expect("boundary probe lock poisoned") = Some(Arc::clone(&probe));

        let scope = counters.measure(); // window open, own peak cell = 0

        // Hold the critical-section lock, so any `op_guard` must block BEFORE it can touch in_flight
        // or the scope peaks.
        let guard = counters.conc.lock().expect("conc lock poisoned");

        let c2 = Arc::clone(&counters);
        let worker = thread::spawn(move || {
            // BLOCKS on conc.lock() inside op_guard (the test thread holds it). Once it proceeds, the
            // whole critical section (in_flight += 1 + peak fan-out) runs atomically.
            let _g = c2.op_guard();
        });

        // DETERMINISTIC readiness: wait until the worker has reached op_guard's lock-acquisition
        // boundary (the probe is stored immediately before the lock attempt). After this, the
        // worker's only remaining path is THROUGH the lock we hold — it is blocked at the boundary.
        while !probe.load(Ordering::SeqCst) {
            thread::yield_now();
        }
        // Sanity: the worker cannot have completed — its critical section needs the lock we hold.
        assert!(
            !worker.is_finished(),
            "worker must be blocked at the lock-acquisition boundary while we hold the lock"
        );

        // The blocked op has NOT entered its critical section: in_flight is still 0 AND the scope
        // peak is still 0 — the two move together, never separately (the fix). (Old lock-free
        // increment would make in_flight == 1 here while the peak stayed 0.)
        assert_eq!(
            guard.in_flight, 0,
            "the blocked op must not have incremented in_flight ahead of the peak fan-out"
        );
        assert_eq!(
            scope.peak.load(Ordering::SeqCst),
            0,
            "no half-applied peak while the increment is blocked behind the lock"
        );

        drop(guard); // release → the worker's critical section runs atomically
        worker.join().expect("worker thread panicked");

        // After the atomic critical section, the peak reflects the call — observed via delta (which
        // reads under the same lock, so it can never catch a half-applied update).
        assert_eq!(
            scope.delta().max_in_flight,
            1,
            "the atomic critical section applied BOTH the increment and the peak update"
        );
    }
}
