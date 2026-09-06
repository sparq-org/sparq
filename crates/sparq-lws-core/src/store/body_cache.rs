// AUTHORED-BY Claude Fable 5
//! The **blob-body LRU cache** — read-4 of the read-path perf plan
//! (`research/lws-design-records.md` §7; RSS `docs/design/backend-read-path.md` §3.4 +
//! `docs/design/throughput-hard-cases.md` §5 `read-4-bodycache`): a byte-budgeted LRU of resource
//! BODIES keyed by **`(blob_key, etag)`**, sat in front of
//! [`BlobStore::get`](super::blob::BlobStore::get) inside [`CompositeStore`](super::CompositeStore)
//! — so a hot, unchanged resource's repeat GET pays **zero** blob-store round-trips (blob-gets/op 1
//! → 0 on a hit).
//!
//! ## Why a hit can NEVER be stale (immutable by construction)
//! [`CompositeStore::mint_blob_key`](super::CompositeStore) mints a fresh 128-bit-random key on
//! EVERY write, so a blob object never changes after creation — the cache therefore needs **no
//! invalidation protocol at all**:
//!
//! - **The lookup key comes from THIS request's authoritative index round-trip.** Every consumer
//!   (`CompositeStore::read` / `read_at`) keys the lookup with the `(blob_key, etag)` pair from the
//!   [`ResourceMeta`](super::ResourceMeta) the SPARQ index returned FOR THIS REQUEST — never from a
//!   remembered/cached pointer. A write commits a NEW blob key (and a new etag) into the index, so
//!   the next read's authoritative meta names a key this cache has never seen ⇒ a MISS ⇒ a fresh
//!   fetch of the new bytes. The old entry is dead weight, evicted by LRU. This is the PSS
//!   "cache bodies by `(s3Key, etag)`; the cache is never authoritative — validate against the index
//!   ETag" invariant, with one improvement the unique-per-write keys buy: validation degenerates to
//!   key equality, so there is no revalidation traffic and no TTL (a TTL would add nothing — if the
//!   index ever returned a stale meta, an UNCACHED `blob.get` through that same stale pointer would
//!   fetch the identical old bytes; the staleness surface is the index's, not this cache's).
//! - **A delete needs nothing:** existence is decided by the index (the read plan / `get_meta`)
//!   BEFORE any body lookup; a deleted resource 404s upstream and this cache is never consulted, so
//!   it cannot resurrect deleted bytes.
//! - **The etag in the key is defence-in-depth** (the PSS keying): even on a hypothetical backend
//!   path that REUSED a blob key, changed bytes get a different content-derived etag ⇒ a different
//!   cache key ⇒ a miss. Two writes can only collide on a cache entry by sharing BOTH the
//!   128-bit-random key AND the content etag.
//!
//! ## Why a hit can NEVER bypass authorization
//! The cache stores BYTES keyed by CONTENT identity — no principal, no decision, no modes. It sits
//! INSIDE the store, strictly BELOW the WAC gate: `serve_read` runs `authorize_read` (the full WAC
//! resolution) and only then calls `read_at`, hit or miss (`src/ldp/handler.rs` — the "bytes are
//! fetched only after the Allow" invariant). An unauthorized request is rejected before the store is
//! ever asked for a body, so a warm cache changes nothing about who gets 401/403 — it only changes
//! where an ALREADY-authorized read's bytes come from. (The ACL bodies the WAC engine itself reads
//! flow through the same cache — they are ordinary resources read by the resolver, gated by the live
//! index probe in `read_acl_confirmed`.)
//!
//! ## Bounding (a DoS-safe budget)
//! - **Byte-budgeted LRU** (`SOLID_SERVER_BODY_CACHE_BYTES`, default [`DEFAULT_BODY_CACHE_BYTES`];
//!   `=0` DISABLES the cache — byte-identical pre-cache behaviour). Each entry is charged its body
//!   length plus key overhead (so a flood of zero-byte bodies still consumes budget and the map stays
//!   bounded); inserting past the budget evicts least-recently-used entries until it fits.
//! - **A per-entry size cap** (`SOLID_SERVER_BODY_CACHE_MAX_ENTRY_BYTES`, default 1/8 of the budget)
//!   so ONE large media blob cannot evict the whole hot set: an oversize body BYPASSES the cache
//!   (it is served, just never stored).
//! - Entries are [`Bytes`] — a hit clones a refcount, never the payload.
//! - **Per-instance only** (the stateless-core rule): a miss re-derives the same answer, so no
//!   cross-instance coherence is needed or added.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use bytes::Bytes;

/// Default body-cache byte budget (256 MiB — "a few hundred MB" per the §3.4 design note). Sized to
/// hold a busy pod's hot document set (typical RDF documents are KBs) while bounding worst-case
/// memory; tunable via `SOLID_SERVER_BODY_CACHE_BYTES`, `=0` disables.
pub const DEFAULT_BODY_CACHE_BYTES: usize = 256 * 1024 * 1024;

/// Per-entry bookkeeping charge added to every cached body (map/key/entry overhead). Charging a
/// non-zero floor per entry means even zero-length bodies consume budget, so the ENTRY COUNT is
/// bounded by `budget / OVERHEAD` — a flood of tiny bodies can never grow the map unbounded.
const ENTRY_OVERHEAD_BYTES: usize = 64;

/// One cached body: the bytes, what they are charged against the budget, and an LRU recency tick.
struct BodyEntry {
    body: Bytes,
    /// `body.len() + key lengths + ENTRY_OVERHEAD_BYTES` — subtracted back on eviction/replacement.
    charge: usize,
    /// Monotonic last-access tick (higher = more recently used).
    last_access: u64,
}

/// The lock-protected state: the `(blob_key, etag)`-keyed map + the running total of charged bytes.
/// Kept together so an insert's eviction loop and total adjustment are one critical section.
#[derive(Default)]
struct BodyCacheInner {
    map: HashMap<(String, String), BodyEntry>,
    total_charged: usize,
}

/// A byte-budgeted, `(blob_key, etag)`-keyed LRU cache of blob bodies (see the module docs for the
/// staleness/authz arguments). Thread-safe (one `Mutex`), per-instance.
///
/// A `max_bytes` of 0 builds the [`disabled`](BodyCache::disabled) sentinel: it NEVER stores and
/// ALWAYS misses, so every read pays the pre-cache blob fetch — byte-identical behaviour.
pub struct BodyCache {
    /// `None` ⇒ DISABLED (the `=0` sentinel): every `get` misses, every `insert` no-ops.
    inner: Option<Mutex<BodyCacheInner>>,
    /// The total byte budget (charged bytes across all entries never exceed this).
    max_bytes: usize,
    /// The per-entry cap: a body whose charge exceeds this BYPASSES the cache entirely.
    max_entry_bytes: usize,
    /// Monotonic access clock for LRU ordering (incremented on every hit/insert).
    clock: AtomicU64,
}

impl BodyCache {
    /// Build an enabled cache with the given byte budget and the DEFAULT per-entry cap (1/8 of the
    /// budget — one entry can never displace more than an eighth of the hot set). `max_bytes == 0`
    /// builds the [`disabled`](Self::disabled) sentinel.
    pub fn new(max_bytes: usize) -> Self {
        Self::with_max_entry_bytes(max_bytes, (max_bytes / 8).max(1))
    }

    /// Build with an explicit per-entry cap (clamped into `1..=max_bytes` so a cacheable entry always
    /// fits the budget and the insert's eviction loop provably terminates with room). `max_bytes == 0`
    /// ⇒ the disabled sentinel.
    pub fn with_max_entry_bytes(max_bytes: usize, max_entry_bytes: usize) -> Self {
        if max_bytes == 0 {
            return Self::disabled();
        }
        Self {
            inner: Some(Mutex::new(BodyCacheInner::default())),
            max_bytes,
            max_entry_bytes: max_entry_bytes.clamp(1, max_bytes),
            clock: AtomicU64::new(0),
        }
    }

    /// The DISABLED sentinel (`SOLID_SERVER_BODY_CACHE_BYTES=0`): never stores, always misses —
    /// every read then pays the blob fetch, byte-identical to the pre-cache path.
    pub fn disabled() -> Self {
        Self {
            inner: None,
            max_bytes: 0,
            max_entry_bytes: 0,
            clock: AtomicU64::new(0),
        }
    }

    /// Whether this cache is enabled (false ⇒ the `=0` sentinel).
    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// The configured byte budget (0 when disabled) — for the boot log.
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// The configured per-entry cap (0 when disabled) — for the boot log.
    pub fn max_entry_bytes(&self) -> usize {
        self.max_entry_bytes
    }

    /// Look up the cached body for `(blob_key, etag)`. `None` on a disabled cache or no entry — the
    /// caller then fetches from the blob store (and [`insert`](Self::insert)s). Bumps LRU recency on
    /// a hit; the returned [`Bytes`] is a refcount clone (no payload copy).
    ///
    /// SECURITY: the caller MUST key this with the `(blob_key, etag)` pair from THIS request's
    /// authoritative index metadata (a [`super::ResourceMeta`] the SPARQ index just returned) — never
    /// a remembered pointer. Under that contract a hit returns exactly the bytes that metadata
    /// committed with (see the module docs' staleness argument).
    pub fn get(&self, blob_key: &str, etag: &str) -> Option<Bytes> {
        let inner = self.inner.as_ref()?;
        let mut guard = match inner.lock() {
            Ok(g) => g,
            // A poisoned lock must never turn into a hit; treat as a miss (the blob fetch is paid).
            Err(_) => return None,
        };
        let tick = self.clock.fetch_add(1, Ordering::Relaxed) + 1;
        let entry = guard
            .map
            .get_mut(&(blob_key.to_string(), etag.to_string()))?;
        entry.last_access = tick;
        Some(entry.body.clone())
    }

    /// Cache the body fetched for `(blob_key, etag)` — called after a MISS's blob fetch so the next
    /// read of the same unchanged resource is a hit. An OVERSIZE body (charge > per-entry cap)
    /// bypasses the cache (never stored — one large blob must not evict the whole hot set). At
    /// budget, least-recently-used entries are evicted until the new entry fits. No-op on a disabled
    /// cache. The stored [`Bytes`] is a refcount clone of `body`.
    pub fn insert(&self, blob_key: &str, etag: &str, body: &Bytes) {
        let Some(inner) = self.inner.as_ref() else {
            return; // disabled: never cache.
        };
        let charge = body
            .len()
            .saturating_add(blob_key.len())
            .saturating_add(etag.len())
            .saturating_add(ENTRY_OVERHEAD_BYTES);
        if charge > self.max_entry_bytes {
            return; // oversize: bypass — served, never stored.
        }
        let tick = self.clock.fetch_add(1, Ordering::Relaxed) + 1;
        let mut guard = match inner.lock() {
            Ok(g) => g,
            Err(_) => return, // a poisoned lock simply means no caching — correctness unaffected.
        };
        let key = (blob_key.to_string(), etag.to_string());
        // Replacing an existing entry (same key ⇒ same immutable bytes in practice): release its
        // charge first so the running total stays exact.
        if let Some(old) = guard.map.remove(&key) {
            guard.total_charged = guard.total_charged.saturating_sub(old.charge);
        }
        // Evict LRU entries until the new charge fits the budget — in ONE pass (roborev Medium:
        // a per-victim whole-map min-scan is O(n·k) under the lock, so a byte-budgeted insert that
        // must evict many tiny entries could stall request handling). Unlike `AclCache` (one new
        // key ⇒ at most one eviction), a byte-budgeted insert can free many entries, so the victim
        // set is selected in a single O(n log n) pass, not k separate O(n) scans.
        if guard.total_charged.saturating_add(charge) > self.max_bytes {
            let needed = guard
                .total_charged
                .saturating_add(charge)
                .saturating_sub(self.max_bytes);
            Self::evict_until_freed(&mut guard, needed);
        }
        guard.total_charged = guard.total_charged.saturating_add(charge);
        guard.map.insert(
            key,
            BodyEntry {
                body: body.clone(),
                charge,
                last_access: tick,
            },
        );
    }

    /// The current number of cached entries (0 when disabled) — for tests/observability.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner
            .as_ref()
            .and_then(|m| m.lock().ok().map(|g| g.map.len()))
            .unwrap_or(0)
    }

    /// The current charged-byte total (0 when disabled) — for tests.
    #[cfg(test)]
    fn total_charged(&self) -> usize {
        self.inner
            .as_ref()
            .and_then(|m| m.lock().ok().map(|g| g.total_charged))
            .unwrap_or(0)
    }

    /// Evict least-recently-used entries in ONE pass until at least `needed` charged bytes have been
    /// freed (or the map is empty). Bounds the per-`insert` eviction work: the keys are collected +
    /// sorted by recency ONCE (O(n log n)), then the oldest are removed until enough budget is freed
    /// — never a whole-map min-scan PER victim (the O(n·k)-under-the-lock latency risk a byte-budgeted
    /// cache invites, since one large insert can displace many tiny entries). Keeps the house LRU
    /// style (no second index structure that could desync from the map — the recency order is derived
    /// from the entries' own `last_access`). Terminates with room: the caller only evicts for a
    /// cacheable entry (`charge ≤ max_entry_bytes ≤ max_bytes`), so freeing `needed` always leaves it
    /// space.
    fn evict_until_freed(guard: &mut BodyCacheInner, needed: usize) {
        // One pass: snapshot (recency, key) and sort oldest-first.
        let mut by_recency: Vec<(u64, (String, String))> = guard
            .map
            .iter()
            .map(|(k, e)| (e.last_access, k.clone()))
            .collect();
        by_recency.sort_unstable_by_key(|(tick, _)| *tick);
        let mut freed = 0usize;
        for (_, key) in by_recency {
            if freed >= needed {
                break;
            }
            if let Some(evicted) = guard.map.remove(&key) {
                freed = freed.saturating_add(evicted.charge);
                guard.total_charged = guard.total_charged.saturating_sub(evicted.charge);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(s: &str) -> Bytes {
        Bytes::copy_from_slice(s.as_bytes())
    }

    #[test]
    fn miss_on_empty_then_hit_after_insert_same_key() {
        let c = BodyCache::new(1024);
        assert!(c.get("k1", "\"e1\"").is_none(), "cold cache misses");
        c.insert("k1", "\"e1\"", &body("hello"));
        assert_eq!(c.get("k1", "\"e1\""), Some(body("hello")));
    }

    #[test]
    fn different_etag_is_a_different_key_never_the_old_bytes() {
        // Defence-in-depth keying: even a (hypothetically) REUSED blob key with changed bytes gets a
        // different content etag ⇒ a different cache key ⇒ a miss, never the old body.
        let c = BodyCache::new(1024);
        c.insert("k1", "\"e1\"", &body("old"));
        assert!(
            c.get("k1", "\"e2\"").is_none(),
            "a changed etag must MISS — the old bytes are unreachable under the new key"
        );
        c.insert("k1", "\"e2\"", &body("new"));
        assert_eq!(c.get("k1", "\"e2\""), Some(body("new")));
        // The old entry still answers only its OWN key (it names immutable old bytes).
        assert_eq!(c.get("k1", "\"e1\""), Some(body("old")));
    }

    #[test]
    fn different_blob_key_is_a_different_key() {
        // The production shape: every write mints a NEW blob key, so a rewritten resource's read
        // (new authoritative meta) can never look up the old entry.
        let c = BodyCache::new(1024);
        c.insert("doc-aaaa", "\"e1\"", &body("v1"));
        assert!(c.get("doc-bbbb", "\"e1\"").is_none());
    }

    #[test]
    fn byte_budget_evicts_lru_first() {
        // Budget fits two of these entries but not three; the LRU one goes.
        let charge_one = 5 + 2 + 4 + ENTRY_OVERHEAD_BYTES; // body(5) + key(2) + etag(4) + overhead
        let c = BodyCache::with_max_entry_bytes(2 * charge_one, charge_one);
        c.insert("k1", "\"e\"", &body("aaaaa"));
        c.insert("k2", "\"e\"", &body("bbbbb"));
        // Touch k1 so k2 becomes LRU.
        assert!(c.get("k1", "\"e\"").is_some());
        c.insert("k3", "\"e\"", &body("ccccc"));
        assert_eq!(c.len(), 2, "budget must stay bounded at two entries");
        assert!(
            c.get("k2", "\"e\"").is_none(),
            "the LRU entry (k2) must have been evicted"
        );
        assert!(c.get("k1", "\"e\"").is_some());
        assert!(c.get("k3", "\"e\"").is_some());
    }

    #[test]
    fn oversize_body_bypasses_and_evicts_nothing() {
        // A body over the per-entry cap is NOT stored and must not displace the existing hot set.
        let c = BodyCache::with_max_entry_bytes(10_000, 100);
        c.insert("hot", "\"e\"", &body("small"));
        let big = Bytes::from(vec![0u8; 5_000]);
        c.insert("big", "\"e\"", &big);
        assert!(c.get("big", "\"e\"").is_none(), "oversize is never stored");
        assert!(
            c.get("hot", "\"e\"").is_some(),
            "the hot set survives an oversize insert untouched"
        );
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn zero_length_bodies_still_consume_budget_map_stays_bounded() {
        // The ENTRY_OVERHEAD floor: a flood of empty bodies cannot grow the map unbounded — each
        // entry is charged its key+overhead, so the count is capped by budget/charge.
        let per_entry = 3 + 3 + ENTRY_OVERHEAD_BYTES; // key(3) + etag(3) + empty body
        let c = BodyCache::with_max_entry_bytes(4 * per_entry, per_entry);
        for i in 0..100 {
            c.insert(&format!("k{i:02}"), "\"e\"", &Bytes::new());
        }
        assert!(
            c.len() <= 4,
            "empty bodies are charged overhead — the map stays budget-bounded (len {})",
            c.len()
        );
        assert!(c.total_charged() <= 4 * per_entry);
    }

    #[test]
    fn replacement_of_the_same_key_does_not_leak_charge() {
        let c = BodyCache::new(1024);
        c.insert("k1", "\"e\"", &body("aaaa"));
        let t1 = c.total_charged();
        c.insert("k1", "\"e\"", &body("aaaa"));
        assert_eq!(
            c.total_charged(),
            t1,
            "re-inserting the same key must not double-charge"
        );
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn disabled_cache_never_stores_and_always_misses() {
        let c = BodyCache::disabled();
        assert!(!c.is_enabled());
        c.insert("k1", "\"e\"", &body("x"));
        assert!(c.get("k1", "\"e\"").is_none(), "disabled always misses");
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn budget_zero_builds_the_disabled_sentinel() {
        let c = BodyCache::new(0);
        assert!(!c.is_enabled());
        c.insert("k1", "\"e\"", &body("x"));
        assert!(c.get("k1", "\"e\"").is_none());
    }

    #[test]
    fn large_insert_evicts_many_tiny_entries_in_one_pass_and_stays_bounded() {
        // roborev Medium regression: a byte-budgeted insert that must displace MANY tiny entries
        // does it in ONE pass (not a whole-map min-scan per victim). Fill the cache with many tiny
        // bodies, then insert ONE large body that needs most of them evicted; the budget invariant
        // must hold and the large body must be resident.
        let per_tiny = 4 + 3 + ENTRY_OVERHEAD_BYTES; // key(4) + etag(3) + tiny body
        let budget = 400 * per_tiny;
        let c = BodyCache::with_max_entry_bytes(budget, budget); // large entries allowed
        for i in 0..300 {
            c.insert(&format!("t{i:03}"), "\"e\"", &body("x"));
        }
        assert!(c.total_charged() <= budget, "budget holds while filling");
        let filled = c.len();
        assert!(
            filled > 100,
            "the cache filled with many tiny entries: {filled}"
        );

        // One large body that needs ~90% of the budget → forces a large single-pass eviction.
        let big = Bytes::from(vec![0u8; budget * 9 / 10]);
        c.insert("BIG", "\"e\"", &big);
        assert!(
            c.total_charged() <= budget,
            "budget invariant holds after the large insert: {} > {budget}",
            c.total_charged()
        );
        assert!(
            c.get("BIG", "\"e\"").is_some(),
            "the large body is resident after displacing the tiny hot set"
        );
    }

    #[test]
    fn eviction_frees_exactly_enough_never_the_whole_cache() {
        // A single new entry must not needlessly clear MORE than required: after inserting one entry
        // that needs a little room, older entries that still fit must survive.
        let charge = 2 + 2 + 3 + ENTRY_OVERHEAD_BYTES; // body(2) + key(2) + etag(3) + overhead
        let c = BodyCache::with_max_entry_bytes(3 * charge, charge); // holds exactly 3 entries
        c.insert("k1", "\"e\"", &body("aa")); // oldest
        c.insert("k2", "\"e\"", &body("bb"));
        c.insert("k3", "\"e\"", &body("cc")); // now full (3 entries)
                                              // A 4th entry evicts ONLY the LRU (k1), leaving k2+k3 intact — not a whole-cache flush.
        c.insert("k4", "\"e\"", &body("dd"));
        assert_eq!(c.len(), 3, "stays at capacity, evicts only what's needed");
        assert!(c.get("k1", "\"e\"").is_none(), "the LRU (k1) is gone");
        assert!(c.get("k2", "\"e\"").is_some(), "k2 survived");
        assert!(c.get("k3", "\"e\"").is_some(), "k3 survived");
        assert!(c.get("k4", "\"e\"").is_some(), "k4 inserted");
    }

    #[test]
    fn per_entry_cap_is_clamped_into_the_budget() {
        // A misconfigured cap larger than the budget is clamped so the eviction loop always
        // terminates with room for any cacheable entry.
        let c = BodyCache::with_max_entry_bytes(100, 10_000);
        assert_eq!(c.max_entry_bytes(), 100);
        let c2 = BodyCache::with_max_entry_bytes(100, 0);
        assert_eq!(c2.max_entry_bytes(), 1);
    }
}
