//! sq-cnuqd (issue #1569) — the BOUNDED, SHARDED, `&self`-readable session cache.
//!
//! # Why this exists
//!
//! Before this, every read-side entry point on [`crate::PodStore`] (`accessible`,
//! `accessible_set`, `view_for`, `query_as`, `query_json_as`, `ask_as`, `wac_allow`)
//! took `&mut self`, because the memoized authorized-graph-set cache was a plain
//! `FxHashMap` mutated in place. That EXCLUSIVE borrow serialised every concurrent
//! reader through one lock even though the underlying engine, the `Arc<Generation>`
//! graph ring, and the `Arc<AuthIndex>` are all read-only after materialization — so a
//! per-request LDP/SPARQL server sharing one `&PodStore` across threads could not run
//! two queries at once (issue #1569, the PSS >50k req/s target).
//!
//! This module replaces that map with a cache that is:
//!
//! - **`&self`-readable** — interior mutability ([`std::sync::RwLock`] per shard), so all
//!   read entry points take `&self` and `&PodStore: Sync` lets N threads read concurrently.
//! - **sharded** — [`SHARDS`] independent lock stripes keyed by the session-key hash, so
//!   two requests for DIFFERENT sessions almost never contend on the same lock. A cache
//!   HIT is a shard read-lock + two `Arc` clones; only a cache MISS (or eviction) takes
//!   that shard's write lock, and only that one shard.
//! - **bounded** — each shard holds at most [`SHARD_CAP`] entries with LRU eviction
//!   ([`Shard::recency`]), so a server passing per-request `now` timestamps (which key
//!   distinctly, [`crate::SessionKey`]) can no longer grow the cache without bound. The LRU
//!   recency queue is ALSO bounded (compacted to one entry per live key once it exceeds
//!   `RECENCY_SLACK * SHARD_CAP`), so re-touching a hot key under invalidation churn cannot
//!   grow it without bound either.
//!
//! # Exact-semantics preservation (the load-bearing invariant)
//!
//! The cache is a TRANSIENT index over the auth-view triples (design doc D3) — it holds
//! zero authorization state of record. It composes with the recently-landed AclIndex
//! (#1577), generation pinning (#1584) and diff-based origin-scoped invalidation (#1585)
//! by keeping their exact seams:
//!
//! - **generation pinning / snapshot consistency** — a read pins `Arc::clone(&self.auth)`
//!   ONCE and computes the whole set against that one snapshot, so a re-materialization
//!   racing a read never yields a half-old/half-new set (the read either sees the whole
//!   old generation or, on the next read after `reindex`, the whole new one).
//! - **invalidation still hits the right shards** — [`SessionCache::clear`] drops every
//!   shard; [`SessionCache::invalidate_origin`] visits EVERY shard and applies the SAME
//!   per-entry origin surgery the pre-sharded map did (drop the origin's slice, mark it
//!   `dirty`, drop the memo), so the diff-based per-origin invalidation still reaches every
//!   cached session regardless of which shard it landed in. Eviction is pure capacity
//!   management and can only ever DROP a cached set (which is re-derived, fail-closed on
//!   the current auth view) — it can never surface a stale grant.
//!
//! Fail-closed throughout: a dropped/evicted entry is simply re-derived from the current
//! `AuthIndex`, never assumed to still be authorized.

use crate::{SessionEntry, SessionKey, SessionSets};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use std::collections::VecDeque;
use std::hash::BuildHasher;
use std::sync::RwLock;

/// Number of independent lock stripes. A power of two so shard selection is a mask, not a
/// modulo. Sized so a realistic thread pool (tens of workers) rarely collides two live
/// requests onto one stripe, while keeping the fixed per-`PodStore` overhead tiny (one
/// `RwLock<Shard>` each, each an empty map until first use).
pub(crate) const SHARDS: usize = 16;

/// Maximum live entries PER SHARD (so the whole cache is bounded by `SHARDS * SHARD_CAP`).
/// When a shard is full, the least-recently-used entry is evicted to make room — this is
/// what stops a server that keys per-request `now` timestamps ([`crate::SessionKey`]'s
/// clock slot) from growing the cache without bound. Eviction only drops a re-derivable
/// memoized set; it never affects correctness (fail-closed re-derivation on the next read).
pub(crate) const SHARD_CAP: usize = 1024;

/// Hard cap on the recency queue length, as a multiple of [`SHARD_CAP`]. The queue can
/// hold at most one live-per-key entry ([`SHARD_CAP`] of them) plus stale copies left by
/// re-touches; when it crosses this multiple it is compacted back down to one entry per
/// live key. Bounding it is load-bearing: a hot session that is repeatedly invalidated
/// (miss → fill → `invalidate_origin` → miss → fill …) re-touches the SAME key without ever
/// growing `map` past the cap, so without this the recency queue would grow WITHOUT BOUND
/// (breaking the whole "bounded" guarantee) and make each eviction O(queue) —
/// sq-cnuqd fix.
const RECENCY_SLACK: usize = 2;

/// One lock stripe: a bounded LRU map of session key → cached entry.
struct Shard {
    map: FxHashMap<SessionKey, SessionEntry>,
    /// LRU recency queue (front = least-recently-used, back = most-recent). A key may
    /// appear more than once after a re-touch; the stale front copies are skipped on
    /// eviction (a key whose LATER occurrence still survives further back is not its true
    /// LRU position). The queue is compacted to one-entry-per-live-key once it exceeds
    /// `RECENCY_SLACK * SHARD_CAP`, so it is bounded by `RECENCY_SLACK * SHARD_CAP` at all
    /// times regardless of the re-touch churn.
    recency: VecDeque<SessionKey>,
}

impl Shard {
    fn new() -> Shard {
        Shard { map: FxHashMap::default(), recency: VecDeque::new() }
    }

    /// Record `key` as most-recently-used (append to the back). Called under the shard's
    /// write lock on every miss-fill. Compacts the recency queue if the accumulated stale
    /// copies have pushed it past `RECENCY_SLACK * SHARD_CAP`, keeping it bounded even when
    /// the same hot key is re-touched indefinitely (miss/invalidate churn).
    fn touch(&mut self, key: &SessionKey) {
        self.recency.push_back(key.clone());
        if self.recency.len() > RECENCY_SLACK * SHARD_CAP {
            self.compact_recency();
        }
    }

    /// Rebuild the recency queue keeping ONLY the last (most-recent) occurrence of each key
    /// that is still live in `map`, preserving relative order. This drops every stale copy
    /// left by re-touches (and any key already evicted), so the queue returns to at most one
    /// entry per live key (≤ `map.len()` ≤ [`SHARD_CAP`]). O(queue) but amortised O(1) per
    /// touch because it only fires once per `RECENCY_SLACK * SHARD_CAP` touches.
    fn compact_recency(&mut self) {
        let mut seen: FxHashSet<SessionKey> = FxHashSet::default();
        let mut compacted: VecDeque<SessionKey> = VecDeque::with_capacity(self.map.len());
        // Walk newest → oldest, keep the first (= newest) sighting of each live key, then
        // reverse so the queue is oldest → newest again (front = LRU).
        for key in self.recency.iter().rev() {
            if self.map.contains_key(key) && seen.insert(key.clone()) {
                compacted.push_front(key.clone());
            }
        }
        self.recency = compacted;
    }

    /// Evict least-recently-used entries until the map is within [`SHARD_CAP`]. Skips
    /// recency entries that are stale (already evicted, or superseded by a later touch of
    /// the same key that is still the live back-most occurrence).
    fn evict_to_cap(&mut self) {
        while self.map.len() > SHARD_CAP {
            let Some(candidate) = self.recency.pop_front() else { break };
            // Skip a stale recency record: the key was already evicted, OR a more-recent
            // touch of the same key exists further back in the queue (so this front copy is
            // not its true LRU position).
            if !self.map.contains_key(&candidate) {
                continue;
            }
            if self.recency.iter().any(|k| *k == candidate) {
                // A later touch of this key survives further back → this copy is stale.
                continue;
            }
            self.map.remove(&candidate);
        }
    }
}

/// The bounded, sharded, interior-mutable session cache. Each read entry point on
/// [`crate::PodStore`] goes through [`SessionCache::get_or_compute`]; the write/reindex
/// path calls [`SessionCache::clear`] / [`SessionCache::invalidate_origin`]. All methods
/// take `&self` — that is the whole point (concurrent `&PodStore` readers).
pub(crate) struct SessionCache {
    shards: Vec<RwLock<Shard>>,
}

impl SessionCache {
    pub(crate) fn new() -> SessionCache {
        SessionCache { shards: (0..SHARDS).map(|_| RwLock::new(Shard::new())).collect() }
    }

    /// The shard index for `key` (mask, not modulo — [`SHARDS`] is a power of two).
    fn shard_of(&self, key: &SessionKey) -> usize {
        (FxBuildHasher.hash_one(key) as usize) & (SHARDS - 1)
    }

    /// Return the memoized [`SessionSets`] for `key`, computing (and caching) it via
    /// `compute` on a miss or a scoped-invalidated entry.
    ///
    /// `compute` is the caller's re-derivation closure ([`crate::PodStore::session_sets`]
    /// pins the auth snapshot and calls [`SessionEntry::fill`]); it takes the entry by
    /// `&mut` and returns the assembled [`SessionSets`]. It is invoked ONLY under the
    /// shard's write lock, and only when the entry has no live memo — so a warm hit is a
    /// read-lock + two `Arc` clones and never touches `compute`.
    ///
    /// The returned [`SessionSets`] is owned (two `Arc` clones); nothing borrows the lock
    /// past this call, so the shard is unlocked before the (potentially long) query runs.
    pub(crate) fn get_or_compute<F>(&self, key: &SessionKey, compute: F) -> SessionSets
    where
        F: FnOnce(&mut SessionEntry) -> SessionSets,
    {
        let idx = self.shard_of(key);
        // Fast path: shared read lock, clone the memo if warm.
        {
            let shard = self.shards[idx].read().expect("session-cache shard poisoned");
            if let Some(entry) = shard.map.get(key) {
                if let Some(memo) = entry.memo.as_ref() {
                    return memo.clone();
                }
            }
        }
        // Miss or stale memo: take the write lock, fill (re-checking under the lock in case
        // another writer filled it first), evict to cap, return the owned sets.
        let mut shard = self.shards[idx].write().expect("session-cache shard poisoned");
        // Double-check: a concurrent writer may have filled it between the two locks.
        if let Some(entry) = shard.map.get(key) {
            if let Some(memo) = entry.memo.as_ref() {
                return memo.clone();
            }
        }
        let entry = shard.map.entry(key.clone()).or_default();
        let sets = compute(entry);
        shard.touch(key);
        shard.evict_to_cap();
        sets
    }

    /// Drop every cached entry across every shard — the [`crate::ReindexScope::Full`]
    /// invalidation (D3: the cache is transient index state, the triples are the storage).
    pub(crate) fn clear(&self) {
        for shard in &self.shards {
            let mut s = shard.write().expect("session-cache shard poisoned");
            s.map.clear();
            s.recency.clear();
        }
    }

    /// Per-origin invalidation ([`crate::ReindexScope::Origin`], issue #1585/#1571): for
    /// EVERY cached session in EVERY shard, drop that `origin`'s slice, mark it `dirty`, and
    /// drop the memo — so the next read re-derives only that origin and keeps every other
    /// origin's slice warm. Visiting all shards is required: the diff-based invalidation
    /// must reach a session regardless of which shard its key hashed into.
    pub(crate) fn invalidate_origin(&self, origin: &str) {
        for shard in &self.shards {
            let mut s = shard.write().expect("session-cache shard poisoned");
            for entry in s.map.values_mut() {
                entry.per_origin.remove(origin);
                entry.dirty.insert(origin.to_owned());
                entry.memo = None;
            }
        }
    }

    /// The current total number of cached entries across all shards (test/inspection only).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.shards.iter().map(|s| s.read().expect("poisoned").map.len()).sum()
    }

    /// Read-only access to one cached entry for white-box tests (clones nothing observable;
    /// returns the values the pre-sharded tests asserted on the plain map).
    #[cfg(test)]
    pub(crate) fn with_entry<R>(
        &self,
        key: &SessionKey,
        f: impl FnOnce(Option<&SessionEntry>) -> R,
    ) -> R {
        let idx = self.shard_of(key);
        let shard = self.shards[idx].read().expect("poisoned");
        f(shard.map.get(key))
    }
}

#[cfg(test)]
mod tests {
    //! sq-cnuqd — DIRECT unit tests for the bounded, sharded cache mechanics
    //! (independent of the WAC/ACP auth path): hit/miss/compute, bounded eviction, `clear`,
    //! per-origin invalidation, and shard-selection stability.
    use super::*;
    use crate::Mode;
    use oxrdf::NamedNode;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn key(agent: &str, mode: Mode) -> SessionKey {
        (Some(agent.to_owned()), None, None, None, mode)
    }

    /// A trivial [`SessionSets`] carrying one graph IRI, so a test can assert which set a
    /// lookup returned without wiring up the whole auth path.
    fn sets_with(iri: &str) -> SessionSets {
        let nn = NamedNode::new(iri).unwrap();
        let sorted = Arc::new(vec![nn.clone()]);
        let set = Arc::new(std::iter::once(oxrdf::Term::NamedNode(nn)).collect());
        SessionSets { sorted, set }
    }

    #[test]
    fn compute_runs_once_then_hits_are_warm() {
        let cache = SessionCache::new();
        let k = key("https://alice.ex/#me", Mode::Read);
        let calls = AtomicUsize::new(0);
        let fill = |_e: &mut SessionEntry| {
            calls.fetch_add(1, Ordering::SeqCst);
            let s = sets_with("https://pod.ex/g1");
            _e.memo = Some(s.clone());
            s
        };
        // First call computes; the second is a warm read-lock hit (compute NOT re-run).
        let a = cache.get_or_compute(&k, fill);
        let b = cache.get_or_compute(&k, fill);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "compute ran exactly once");
        assert_eq!(a.sorted.as_ref(), b.sorted.as_ref());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn distinct_keys_land_and_cache_independently() {
        let cache = SessionCache::new();
        let ka = key("https://alice.ex/#me", Mode::Read);
        let kb = key("https://bob.ex/#me", Mode::Read);
        let a = cache.get_or_compute(&ka, |e| {
            let s = sets_with("https://pod.ex/a");
            e.memo = Some(s.clone());
            s
        });
        let b = cache.get_or_compute(&kb, |e| {
            let s = sets_with("https://pod.ex/b");
            e.memo = Some(s.clone());
            s
        });
        assert_eq!(a.sorted[0].as_str(), "https://pod.ex/a");
        assert_eq!(b.sorted[0].as_str(), "https://pod.ex/b");
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn bounded_eviction_caps_each_shard() {
        // Force many entries into ONE shard by fixing everything but a synthetic agent id,
        // then filtering to keys that hash to shard 0 — enough to exceed SHARD_CAP there.
        let cache = SessionCache::new();
        let mut in_shard0 = 0usize;
        let mut i = 0u64;
        // Insert well past SHARD_CAP worth of shard-0 keys.
        while in_shard0 <= SHARD_CAP + 50 {
            let k = key(&format!("https://a.ex/#{i}"), Mode::Read);
            i += 1;
            if cache.shard_of(&k) != 0 {
                continue;
            }
            in_shard0 += 1;
            cache.get_or_compute(&k, |e| {
                let s = sets_with("https://pod.ex/x");
                e.memo = Some(s.clone());
                s
            });
        }
        // Shard 0 is bounded to SHARD_CAP; the whole cache never exceeds SHARDS*SHARD_CAP.
        let shard0_len = cache.shards[0].read().unwrap().map.len();
        assert!(shard0_len <= SHARD_CAP, "shard 0 bounded ({shard0_len} <= {SHARD_CAP})");
        assert!(cache.len() <= SHARDS * SHARD_CAP, "whole cache bounded");
    }

    #[test]
    fn recency_queue_stays_bounded_under_hot_key_churn() {
        // Regression (sq-cnuqd): a hot session re-touched over and over (the real
        // miss → fill → invalidate_origin → miss → fill … churn a per-origin ACL write drives)
        // must NOT grow the recency queue without bound. Before the compaction fix, `map`
        // stayed at 1 entry so `evict_to_cap` never fired and `recency` grew one copy per
        // re-touch — an unbounded leak that also made eviction O(queue).
        let cache = SessionCache::new();
        let k = key("https://hot.ex/#me", Mode::Read);
        let idx = cache.shard_of(&k);
        // Far more re-touches than RECENCY_SLACK * SHARD_CAP so compaction must have fired.
        for _ in 0..(RECENCY_SLACK * SHARD_CAP * 3 + 17) {
            // Each round: drop the memo (as invalidate_origin does) so the next call is a MISS
            // that re-touches the SAME key, then read it back.
            cache.invalidate_origin("https://hot.ex");
            cache.get_or_compute(&k, |e| {
                e.computed = true;
                let s = sets_with("https://hot.ex/g");
                e.memo = Some(s.clone());
                s
            });
        }
        let rec = cache.shards[idx].read().unwrap().recency.len();
        let map = cache.shards[idx].read().unwrap().map.len();
        assert_eq!(map, 1, "only the one hot session is cached");
        assert!(
            rec <= RECENCY_SLACK * SHARD_CAP,
            "recency bounded ({rec} <= {}) despite churn",
            RECENCY_SLACK * SHARD_CAP
        );
    }

    #[test]
    fn clear_drops_everything() {
        let cache = SessionCache::new();
        for n in 0..8u64 {
            let k = key(&format!("https://a.ex/#{n}"), Mode::Read);
            cache.get_or_compute(&k, |e| {
                let s = sets_with("https://pod.ex/x");
                e.memo = Some(s.clone());
                s
            });
        }
        assert!(cache.len() >= 1);
        cache.clear();
        assert_eq!(cache.len(), 0, "clear drops all shards");
    }

    #[test]
    fn invalidate_origin_marks_dirty_and_drops_memo() {
        let cache = SessionCache::new();
        let k = key("https://alice.ex/#me", Mode::Read);
        // Seed an entry with two origin buckets + a memo.
        cache.get_or_compute(&k, |e| {
            e.computed = true;
            e.per_origin.insert("https://a.ex".to_owned(), vec![NamedNode::new("https://a.ex/g").unwrap()]);
            e.per_origin.insert("https://b.ex".to_owned(), vec![NamedNode::new("https://b.ex/g").unwrap()]);
            let s = sets_with("https://a.ex/g");
            e.memo = Some(s.clone());
            s
        });
        cache.invalidate_origin("https://a.ex");
        cache.with_entry(&k, |e| {
            let e = e.expect("entry present");
            assert!(!e.per_origin.contains_key("https://a.ex"), "a.ex slice dropped");
            assert!(e.per_origin.contains_key("https://b.ex"), "b.ex slice kept warm");
            assert!(e.dirty.contains("https://a.ex"), "a.ex marked dirty");
            assert!(e.memo.is_none(), "memo dropped so it re-assembles");
        });
    }

    #[test]
    fn shard_selection_is_stable_and_in_range() {
        let cache = SessionCache::new();
        let k = key("https://alice.ex/#me", Mode::Read);
        let a = cache.shard_of(&k);
        let b = cache.shard_of(&k);
        assert_eq!(a, b, "deterministic");
        assert!(a < SHARDS, "in range");
    }
}
