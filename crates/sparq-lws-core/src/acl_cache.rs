// AUTHORED-BY Claude Opus 4.8
//! An **ETag-keyed parsed-ACL cache** that removes the redundant per-request `.acl` byte-fetch +
//! `oxttl` re-parse on repeated reads of the same resource (read-path optimisation #3) **without ever
//! changing an authorization decision**.
//!
//! ## The optimisation (and what it must NOT change)
//! Every authenticated AND anonymous GET/HEAD resolves the effective ACL of the target — the
//! child→root walk, a `store.read(<acl>)` (an index `get_meta` PLUS a blob byte-fetch) per candidate
//! ACL, and an `oxttl` parse of the found ACL's bytes into triples (see
//! `WacAuthorizer::resolve_effective_acl`). For a HOT resource whose ACL does not
//! change between reads, the byte-fetch + parse of that ACL is pure waste — the SAME triples come out
//! every time.
//!
//! This cache stores the **parsed triples** of an ACL resource keyed by **`(acl-iri, etag)`**.
//! Resolver lookups also require the immutable blob pointer that was parsed.
//! A changed pointer is re-read even when its content-derived ETag is unchanged.
//! On each probe the resolver obtains the ACL's CURRENT [`ResourceMeta`](crate::store::ResourceMeta) CHEAPLY (a
//! `store.meta` — an index lookup, NO blob byte-fetch, NO parse) and:
//! - if a cached entry exists for that `acl-iri` with the same ETag and blob pointer, the
//!   cached triples are reused (the byte-fetch + `oxttl` parse are SKIPPED);
//! - otherwise it is a MISS: the resolver reads
//!   + parses the ACL afresh and REFRESHES the entry under the new etag.
//!
//! ## 🔒 The cache is NEVER authoritative (the charter rule)
//! - **Keyed by ACL IRI, ETag and blob pointer in the resolver.** A cached parse requires all three, so a
//!   rotated ACL (new bytes ⇒ new etag) can NEVER be honoured stale — the etag mismatch forces a
//!   re-read+re-parse. The cache only avoids the re-PARSE of an UNCHANGED ACL; it can never change the
//!   triples the resolver sees, hence never the decision or the `WAC-Allow` output.
//! - **Per-instance only** (the stateless-core charter rule + the explicit design note): a miss just
//!   re-derives the SAME answer, so coherence across instances is neither needed NOR added — a shared
//!   ACL cache would add a stale-grant coherence hole for no gain.
//! - **Bounded** (LRU + a validation TTL): capacity-capped (default [`DEFAULT_ACL_CACHE_CAPACITY`]) so
//!   the cache can never grow unbounded (a DoS vector); LRU eviction on overflow; AND a short
//!   validation TTL (default [`DEFAULT_MAX_ENTRY_TTL_SECS`]) so even if a store somehow returned a
//!   STALE etag (an index/etag bug), a cached parse is force-re-validated within one TTL window rather
//!   than honoured indefinitely. The effective freshness gate is `etag-match AND now < inserted_at +
//!   max_entry_ttl`.
//! - **Default-on, capacity-tunable** via `SOLID_SERVER_ACL_CACHE_CAPACITY`; `=0` DISABLES the cache
//!   (the [`AclCache::disabled`] sentinel never stores and always misses → byte-identical pre-cache
//!   behaviour).
//!
//! ## Why this can never turn a 401/403 into a 200 (or serve a stale grant)
//! The cache stores ONLY the *parse* of an ACL's bytes; it never stores a decision, a mode set, or a
//! "this resource is public" flag. The resolver ALWAYS performs the SAME child→root walk and the SAME
//! `store.meta` existence/etag probe per candidate ACL — a MISSING ACL is still observed as missing
//! (the cache holds no entry that could fabricate one), and a PRESENT ACL's triples are reused ONLY
//! when its ETag and blob pointer are unchanged. Modes use the SAME `modes_for` over those triples.
//! So: a removed ACL → the `meta` probe returns `None` → the walk continues exactly as before (the
//! cache cannot resurrect deleted grants); a rotated ACL → etag mismatch → re-parse (no stale grant);
//! a broken ACL → re-parsed to empty-triples (fail-closed) and that empty parse is what gets cached
//! under its etag, so a broken own-ACL still DENIES.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use oxrdf::Triple;

/// Default ACL-cache capacity (number of distinct cached ACL resources). A pod has far fewer distinct
/// `.acl` resources than requests (most resources INHERIT a handful of container ACLs), so a few
/// thousand entries comfortably covers the live ACL set of a busy multi-tenant pod while bounding
/// memory (each entry is the ACL IRI + its etag string + its parsed triples). Tunable via the env var;
/// conformance-neutral.
pub const DEFAULT_ACL_CACHE_CAPACITY: usize = 4096;

/// Default max lifetime of a cached parsed-ACL entry, in seconds, INDEPENDENT of the etag match. An
/// entry older than this is force-re-validated (treated as a miss) even if the etag still matches —
/// belt-and-braces against a store/index that ever returned a STALE etag for a changed ACL. The
/// default matches the auth caches' validation-freshness window (300s); the effective per-entry
/// freshness gate is `etag-match AND now < inserted_at + this`.
pub const DEFAULT_MAX_ENTRY_TTL_SECS: i64 = 300;

/// A cached, already-parsed ACL keyed (in the map) by its IRI; the stored `etag` is what the current
/// etag must equal for the parse to be reusable.
#[derive(Clone)]
struct AclEntry {
    /// The etag the bytes had when parsed. A reuse requires the CURRENT etag to equal this — a
    /// different etag means the ACL was rewritten (new bytes) and the cached parse is stale.
    etag: String,
    /// Immutable blob pointer authenticated when the resolver parsed these bytes.
    blob_key: String,
    /// The parsed ACL triples (the expensive `oxttl` output reused on a hit). An empty `Vec` is a
    /// LEGITIMATE cached value — a PRESENT-but-malformed ACL parses to empty triples (fail-closed), and
    /// caching that empty parse under its etag is correct (it still DENIES, as the cold path does).
    triples: Vec<Triple>,
    /// Epoch seconds the entry was inserted — for the validation-TTL freshness bound.
    inserted_at: i64,
    /// Monotonic last-access tick for LRU eviction (higher = more recently used).
    last_access: u64,
}

/// An ETag-keyed parsed-ACL cache: a bounded (LRU + validation-TTL) map from an ACL resource IRI to
/// its last parsed triples + the etag they were parsed at. Thread-safe (one `Mutex`), per-instance.
///
/// A `capacity` of 0 builds the [`AclCache::disabled`] sentinel: it NEVER stores and ALWAYS reports a
/// miss, so the resolver's behaviour is byte-identical to the pre-cache code path.
pub struct AclCache {
    /// `None` ⇒ the cache is DISABLED (the `=0` sentinel): every `get` misses, every `insert` no-ops.
    inner: Option<Mutex<HashMap<String, AclEntry>>>,
    capacity: usize,
    /// Monotonic access clock for LRU ordering (incremented on every hit/insert).
    clock: AtomicU64,
    /// Max entry lifetime (seconds) independent of the etag match — the validation-freshness bound.
    max_entry_ttl_secs: i64,
}

impl AclCache {
    /// Build an enabled cache with the given capacity. A `capacity` of 0 builds the
    /// [`disabled`](Self::disabled) sentinel (NEVER caches → byte-identical to no cache). A positive
    /// capacity is the live LRU bound (clamped to >=1 internally so a misconfigured tiny value cannot
    /// make every insert evict itself before it is ever read).
    pub fn new(capacity: usize) -> Self {
        Self::with_max_entry_ttl(capacity, DEFAULT_MAX_ENTRY_TTL_SECS)
    }

    /// Build with an explicit validation TTL (seconds). `capacity == 0` ⇒ the disabled sentinel. A
    /// non-positive `ttl` is clamped to 1 (an entry is always re-validated at least every second; never
    /// an unbounded validation lifetime).
    pub fn with_max_entry_ttl(capacity: usize, max_entry_ttl_secs: i64) -> Self {
        if capacity == 0 {
            return Self::disabled();
        }
        Self {
            inner: Some(Mutex::new(HashMap::new())),
            capacity: capacity.max(1),
            clock: AtomicU64::new(0),
            max_entry_ttl_secs: max_entry_ttl_secs.max(1),
        }
    }

    /// The DISABLED sentinel (`SOLID_SERVER_ACL_CACHE_CAPACITY=0`): never stores, always misses. The
    /// resolver then reads + parses every ACL every time — byte-identical to the pre-cache path.
    pub fn disabled() -> Self {
        Self {
            inner: None,
            capacity: 0,
            clock: AtomicU64::new(0),
            max_entry_ttl_secs: 1,
        }
    }

    /// Whether this cache is enabled (false ⇒ the `=0` sentinel).
    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// Look up the parsed triples for `acl_iri` IFF the cached entry's etag equals `current_etag` AND
    /// the entry is within the validation TTL (`now < inserted_at + max_entry_ttl`). Returns `None` on
    /// a disabled cache, no entry, an etag mismatch (the ACL was rotated), or a stale-by-TTL entry —
    /// in every such case the caller re-reads + re-parses (and should then [`insert`](Self::insert) the
    /// fresh parse). Bumps LRU recency on a hit.
    ///
    /// SECURITY: a hit returns the SAME triples a cold parse of the UNCHANGED ACL bytes would — the
    /// etag-equality gate guarantees the cached parse corresponds to the bytes currently at `acl_iri`.
    /// It can never return triples for a DIFFERENT (rotated) ACL.
    pub fn get(&self, acl_iri: &str, current_etag: &str, now: i64) -> Option<Vec<Triple>> {
        self.get_for_blob(acl_iri, current_etag, "", now)
    }

    pub(crate) fn get_for_blob(
        &self,
        acl_iri: &str,
        current_etag: &str,
        blob_key: &str,
        now: i64,
    ) -> Option<Vec<Triple>> {
        let inner = self.inner.as_ref()?;
        let mut map = match inner.lock() {
            Ok(m) => m,
            // A poisoned cache lock must NEVER fail to a stale hit; treat as a miss (full re-parse).
            Err(_) => return None,
        };
        let entry = map.get(acl_iri)?;
        // Freshness gate, RE-CHECKED on every hit: the etag MUST match the current bytes, AND the entry
        // must be within the validation TTL. A mismatch OR a stale-by-TTL entry is evicted and missed.
        let within_ttl = now < entry.inserted_at.saturating_add(self.max_entry_ttl_secs);
        if entry.etag != current_etag || entry.blob_key != blob_key || !within_ttl {
            map.remove(acl_iri);
            return None;
        }
        let tick = self.clock.fetch_add(1, Ordering::Relaxed) + 1;
        let triples = entry.triples.clone();
        if let Some(slot) = map.get_mut(acl_iri) {
            slot.last_access = tick;
        }
        Some(triples)
    }

    /// Cache (or refresh) the parsed `triples` for `acl_iri` at `etag`. Called by the resolver after a
    /// MISS re-reads + re-parses an ACL — so the NEXT read of the same unchanged ACL is a hit. A
    /// re-insert of the same IRI overwrites in place (never grows the map); a NEW IRI at capacity evicts
    /// the LRU entry first (bounded — never unbounded growth). No-op on a disabled cache.
    ///
    /// `now` is the insertion time (recorded for the validation-TTL bound).
    pub fn insert(&self, acl_iri: &str, etag: &str, triples: Vec<Triple>, now: i64) {
        self.insert_for_blob(acl_iri, etag, "", triples, now);
    }

    pub(crate) fn insert_for_blob(
        &self,
        acl_iri: &str,
        etag: &str,
        blob_key: &str,
        triples: Vec<Triple>,
        now: i64,
    ) {
        let Some(inner) = self.inner.as_ref() else {
            return; // disabled: never cache.
        };
        let tick = self.clock.fetch_add(1, Ordering::Relaxed) + 1;
        let entry = AclEntry {
            etag: etag.to_string(),
            blob_key: blob_key.to_owned(),
            triples,
            inserted_at: now,
            last_access: tick,
        };
        let mut map = match inner.lock() {
            Ok(m) => m,
            Err(_) => return, // a poisoned lock simply means no caching — correctness unaffected.
        };
        if !map.contains_key(acl_iri) && map.len() >= self.capacity {
            Self::evict_one(&mut map);
        }
        map.insert(acl_iri.to_string(), entry);
    }

    /// Explicitly invalidate the cached entry for `acl_iri` (a WRITE / DELETE of the ACL resource). The
    /// etag gate already prevents serving a rotated ACL stale, so this is belt-and-braces — but it also
    /// frees the slot immediately on a delete (whose new "etag" is the absence of the resource, which
    /// the `meta` probe reports as `None` ⇒ the resolver never even calls `get`). No-op on a disabled
    /// cache or an absent key.
    pub fn invalidate(&self, acl_iri: &str) {
        if let Some(inner) = self.inner.as_ref() {
            if let Ok(mut map) = inner.lock() {
                map.remove(acl_iri);
            }
        }
    }

    /// The current number of cached entries (0 when disabled) — for tests/observability.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner
            .as_ref()
            .and_then(|m| m.lock().ok().map(|g| g.len()))
            .unwrap_or(0)
    }

    /// Whether the cache currently holds no entries (always true when disabled) — for tests.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Evict exactly one entry to make room: the least-recently-used. Called only when inserting a NEW
    /// key at capacity. (Capacity is small/medium, so an O(n) min-scan is cheap and avoids a second
    /// index structure that could desync from the map.)
    fn evict_one(map: &mut HashMap<String, AclEntry>) {
        if let Some(lru_key) = map
            .iter()
            .min_by_key(|(_, e)| e.last_access)
            .map(|(k, _)| k.clone())
        {
            map.remove(&lru_key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{NamedNode, Triple};

    fn triple(s: &str) -> Triple {
        let n = NamedNode::new(format!("https://ex/{s}")).unwrap();
        Triple::new(n.clone(), n.clone(), n)
    }

    fn triples(tags: &[&str]) -> Vec<Triple> {
        tags.iter().map(|t| triple(t)).collect()
    }

    const ACL: &str = "https://pod.example/a/.acl";

    #[test]
    fn miss_on_empty_then_hit_after_insert_same_etag() {
        let c = AclCache::new(8);
        assert!(c.get(ACL, "etag-1", 0).is_none(), "cold cache misses");
        c.insert(ACL, "etag-1", triples(&["x", "y"]), 0);
        // Same etag, within TTL → hit returns the SAME parsed triples.
        assert_eq!(c.get(ACL, "etag-1", 1), Some(triples(&["x", "y"])));
    }

    #[test]
    fn etag_mismatch_misses_and_evicts() {
        let c = AclCache::new(8);
        c.insert(ACL, "etag-1", triples(&["x"]), 0);
        // A DIFFERENT etag (the ACL was rotated) MUST miss — never serve the old parse.
        assert!(
            c.get(ACL, "etag-2", 1).is_none(),
            "etag mismatch must miss (no stale parse)"
        );
        // The stale entry was evicted: even re-querying the OLD etag now misses (it was removed).
        assert!(
            c.get(ACL, "etag-1", 1).is_none(),
            "the mismatched entry must have been evicted"
        );
    }

    #[test]
    fn refresh_after_rotation_serves_new_parse() {
        let c = AclCache::new(8);
        c.insert(ACL, "etag-1", triples(&["old"]), 0);
        // Rotation: caller saw etag-2, missed, re-parsed, and refreshes.
        assert!(c.get(ACL, "etag-2", 1).is_none());
        c.insert(ACL, "etag-2", triples(&["new"]), 1);
        // The NEXT read at etag-2 hits the NEW parse, never the old one.
        assert_eq!(c.get(ACL, "etag-2", 2), Some(triples(&["new"])));
    }

    #[test]
    fn validation_ttl_forces_remiss_even_on_etag_match() {
        // Belt-and-braces: even with a MATCHING etag, an entry older than the TTL is missed + evicted.
        let c = AclCache::with_max_entry_ttl(8, 10);
        c.insert(ACL, "etag-1", triples(&["x"]), 100);
        // Within TTL (now=105 < 100+10): hit.
        assert_eq!(c.get(ACL, "etag-1", 105), Some(triples(&["x"])));
        // Re-insert resets inserted_at on the bump? No — get does not reset inserted_at. At the
        // boundary now=110 == 100+10 → NOT within TTL (strict <) → miss + evict.
        c.insert(ACL, "etag-1", triples(&["x"]), 100);
        assert!(
            c.get(ACL, "etag-1", 110).is_none(),
            "an entry at exactly inserted_at+ttl is stale (strict <)"
        );
    }

    #[test]
    fn empty_triples_is_a_legitimate_cached_value() {
        // A present-but-malformed ACL parses to empty triples (fail-closed). Caching that empty parse
        // and serving it on a hit is correct — it still grants nothing.
        let c = AclCache::new(8);
        c.insert(ACL, "etag-broken", Vec::new(), 0);
        assert_eq!(c.get(ACL, "etag-broken", 1), Some(Vec::new()));
    }

    #[test]
    fn invalidate_removes_the_entry() {
        let c = AclCache::new(8);
        c.insert(ACL, "etag-1", triples(&["x"]), 0);
        assert_eq!(c.len(), 1);
        c.invalidate(ACL);
        assert_eq!(c.len(), 0);
        assert!(c.get(ACL, "etag-1", 1).is_none());
    }

    #[test]
    fn lru_capacity_bound_holds() {
        let c = AclCache::new(2);
        c.insert("https://pod/a.acl", "e", triples(&["a"]), 0);
        c.insert("https://pod/b.acl", "e", triples(&["b"]), 0);
        // Touch a so b becomes LRU.
        assert!(c.get("https://pod/a.acl", "e", 1).is_some());
        // Inserting a THIRD key evicts the LRU (b), keeping the map at capacity 2.
        c.insert("https://pod/c.acl", "e", triples(&["c"]), 1);
        assert_eq!(c.len(), 2, "capacity must stay bounded at 2");
        assert!(
            c.get("https://pod/b.acl", "e", 2).is_none(),
            "the LRU entry (b) must have been evicted"
        );
        assert!(c.get("https://pod/a.acl", "e", 2).is_some());
        assert!(c.get("https://pod/c.acl", "e", 2).is_some());
    }

    #[test]
    fn reinsert_same_key_does_not_grow_or_evict() {
        let c = AclCache::new(2);
        c.insert("https://pod/a.acl", "e1", triples(&["a"]), 0);
        c.insert("https://pod/b.acl", "e", triples(&["b"]), 0);
        // Re-insert a (a rotation) — must NOT evict b (same key overwrite, not a new key).
        c.insert("https://pod/a.acl", "e2", triples(&["a2"]), 1);
        assert_eq!(c.len(), 2);
        assert!(c.get("https://pod/b.acl", "e", 2).is_some());
        assert_eq!(c.get("https://pod/a.acl", "e2", 2), Some(triples(&["a2"])));
    }

    #[test]
    fn disabled_cache_never_stores_and_always_misses() {
        let c = AclCache::disabled();
        assert!(!c.is_enabled());
        c.insert(ACL, "etag-1", triples(&["x"]), 0);
        assert_eq!(c.len(), 0, "disabled cache stores nothing");
        assert!(
            c.get(ACL, "etag-1", 1).is_none(),
            "disabled cache always misses"
        );
        c.invalidate(ACL); // no-op, must not panic.
    }

    #[test]
    fn capacity_zero_builds_the_disabled_sentinel() {
        let c = AclCache::new(0);
        assert!(!c.is_enabled());
        c.insert(ACL, "e", triples(&["x"]), 0);
        assert!(c.get(ACL, "e", 1).is_none());
    }
}
