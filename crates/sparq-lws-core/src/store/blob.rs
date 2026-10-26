// AUTHORED-BY Claude Opus 4.8
//! The blob (byte) store seam.
//!
//! Per the architecture, `object_store`/S3 is **backup-only** for resource bytes: SPARQ is the
//! authoritative index, and the blob store holds a durable copy of the bytes keyed by an opaque
//! `s3Key`. This module defines the [`BlobStore`] trait and an in-memory test impl. The real
//! `object_store`-backed impl (S3 / Local) is an M2 adapter behind the same trait.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;
use bytes::Bytes;

use super::{InMemoryStoreLimits, StoreUsage};

/// A blob-store error (kept opaque so it never leaks backend detail to a client).
#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("blob not found")]
    NotFound,
    /// The configured in-memory byte or entry ceiling would be exceeded.
    #[error("in-memory blob-store quota exceeded")]
    QuotaExceeded,
    /// An envelope names no durable wrapped-key record.
    #[error("UnknownDataKey")]
    UnknownDataKey,
    #[error("blob backend error: {0}")]
    Backend(String),
}

/// The stable pod identity supplied by the resource write path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlobScope(pub String);

/// One stored blob, as surfaced by [`BlobStore::list`] for the reconciler's orphan sweep.
///
/// Two distinct properties travel here, used for two distinct decisions:
///
/// - `last_modified` is the AGE witness, LOAD-BEARING for the reconciler's grace period: an orphan is
///   only GC'd when it is OLDER than the grace window, so a blob whose bytes were just written but whose
///   index row has not yet committed (the write-in-progress race) is protected. Backends that do not
///   expose a timestamp must report `None` and are then NEVER GC'd by the reconciler (fail-closed — we
///   can't prove an undated blob is old enough to be safe to delete).
///
/// - `generation` is the CAS WITNESS for [`BlobStore::delete_if_unchanged`]. A `SystemTime` is NOT a
///   unique write version — two same-key writes can collide on a timestamp (clock granularity, a clock
///   rollback, coarse backend precision), so a same-timestamp recreate landing between the reconciler's
///   fresh stat and its delete could slip past a `last_modified`-only CAS and be clobbered. The
///   generation is a TRUE, strictly-increasing write version: every overwrite gets a STRICTLY DIFFERENT
///   generation regardless of the clock, so the reconciler can compare-and-delete on it race-free and
///   clock-independently. For the in-memory store it is a store-wide monotonic counter stamped on each
///   write; a real `object_store` backend maps it to the backend's native version/ETag/generation (the
///   `M2-next:` seam below). `None` ⇒ the backend exposes no write version ⇒ the reconciler cannot do a
///   safe CAS and MUST instead rely on unique-per-write keys (documented in `delete_if_unchanged`).
#[derive(Debug, Clone)]
pub struct BlobEntry {
    /// The opaque storage key the bytes live under.
    pub key: String,
    /// When the bytes were last written, if the backend records it. `None` ⇒ unknown ⇒ never GC'd.
    /// The AGE witness for the grace check ONLY — NEVER the CAS-delete witness (a timestamp is not a
    /// unique write version); the CAS uses `generation`.
    pub last_modified: Option<SystemTime>,
    /// A strictly-increasing, immutable-per-write version of THIS stored blob — the authoritative CAS
    /// witness for [`BlobStore::delete_if_unchanged`]. Every write (even a same-millisecond, even a
    /// clock-rolled-back overwrite) gets a strictly different value, so it distinguishes two writes the
    /// way a `last_modified` cannot. `None` ⇒ the backend has no native write version ⇒ no safe CAS
    /// (use unique-per-write keys instead).
    pub generation: Option<u64>,
}

/// The byte store: a key/value store of resource bodies, keyed by an opaque storage key.
///
/// M2: an `object_store`-backed impl (`object_store::aws::AmazonS3` / `LocalFileSystem`) plugs in
/// here, using `PutMode::Create`/`Update` for the S3 If-None-Match/If-Match CAS (spike §6).
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Read the bytes for a storage key.
    async fn get(&self, key: &str) -> Result<Bytes, BlobError>;

    /// Write (create-or-replace) the bytes for a storage key.
    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError>;

    /// Write resource bytes for a pod. Ordinary backends ignore the scope.
    async fn put_scoped(&self, _scope: &BlobScope, key: &str, body: Bytes) -> Result<(), BlobError> {
        self.put(key, body).await
    }

    /// Whether a key exists.
    async fn exists(&self, key: &str) -> Result<bool, BlobError>;

    /// Remove the bytes for a storage key. Idempotent: deleting an absent key is `Ok(())` (the
    /// authoritative existence decision lives in the index, not here).
    async fn delete(&self, key: &str) -> Result<(), BlobError>;

    /// List every stored blob key with its last-modified time (if the backend records one).
    ///
    /// This is the read side the reconciler's orphaned-bytes sweep needs: SPARQ is authoritative for
    /// *which* bytes are referenced, but only the blob store can enumerate which bytes *physically
    /// exist*, so the reconciler diffs the two. This is the ONLY read path permitted to enumerate the
    /// blob store directly — the LDP request path must never LIST/HEAD the blob store (the
    /// "SPARQ is the source of truth" invariant); the reconciler is the documented exception because GC
    /// is *about* the bytes the index does not know about.
    ///
    /// M2-next (the `object_store` adapter): implement via `object_store::ObjectStore::list` — its
    /// `ObjectMeta` carries `location` (→ `key`) + `last_modified` (a `chrono::DateTime<Utc>` → map to
    /// [`SystemTime`]), so the real S3/Local backend reports a real timestamp and the grace window
    /// works against true object age. Until that adapter lands the in-memory double below is the only
    /// impl.
    async fn list(&self) -> Result<Vec<BlobEntry>, BlobError>;

    /// Re-read ONE key's current state (its present `last_modified`), or `None` if the key no longer
    /// exists. The single-key counterpart to [`list`](BlobStore::list).
    ///
    /// The reconciler uses this to RE-STAT a candidate orphan immediately before deleting it: the
    /// [`list`](BlobStore::list) snapshot taken at sweep start can be stale by the time the delete loop
    /// reaches a key. The composite store now mints UNIQUE-PER-WRITE blob keys
    /// (`CompositeStore::mint_blob_key`) so a recreate gets a DIFFERENT key and can no longer
    /// collide on a candidate's key — but this re-stat + the CAS below remain a defence-in-depth for any
    /// backend or path where a key could still be reused. Re-stat lets the reconciler notice "this key's
    /// bytes are now NEWER than my snapshot saw" (⇒ rewritten ⇒ skip) — see
    /// [`super::reconcile::reconcile_orphans`].
    ///
    /// The default implementation derives the answer from [`list`](BlobStore::list) so existing/future
    /// impls keep working unchanged; a backend with a cheap single-key HEAD (object_store, the in-memory
    /// double) SHOULD override this with the O(1) path rather than re-enumerating the whole store.
    ///
    /// M2 (the `object_store` adapter): override via `object_store::ObjectStore::head(&location)` — its
    /// `ObjectMeta.last_modified` maps to [`SystemTime`]; a `NotFound` from the backend maps to `Ok(None)`
    /// (the key is gone), any other error propagates.
    async fn stat(&self, key: &str) -> Result<Option<BlobEntry>, BlobError> {
        Ok(self.list().await?.into_iter().find(|e| e.key == key))
    }

    /// ATOMIC compare-and-delete: remove `key` **iff** its current `generation` still equals
    /// `expected_generation`. Returns `Ok(true)` if it deleted, `Ok(false)` if the witness no longer
    /// matched (the bytes were rewritten ⇒ a new generation / the key vanished) — in which case NOTHING
    /// was removed. The single race-closing primitive the reconciler's final delete uses.
    ///
    /// # Why the witness is the GENERATION, not `last_modified` (the HIGH fix)
    /// A [`SystemTime`] is NOT a unique write version. Two same-key writes can share a `last_modified`:
    /// clock granularity (two writes in one tick), a clock rollback (NTP step), or a backend's coarse
    /// timestamp precision can all give a recreate the SAME stamp as the bytes it replaced. A
    /// `last_modified`-keyed CAS would then see "stamp unchanged" and DELETE the recreate's live bytes —
    /// the very clobber the CAS exists to prevent. The `generation` is a STRICTLY-INCREASING write
    /// version: every overwrite gets a strictly different generation regardless of what the clock did, so
    /// it is a TRUE witness for "are these still the bytes I decided to GC?". The reconciler therefore
    /// compares + deletes on the generation, and the CAS is correct even under clock rollback / coarse
    /// timestamps. (`last_modified` is still used — but ONLY for the time-based grace/age check, which is
    /// correct for "old enough"; it is never the delete witness.)
    ///
    /// # Why a CAS at all (the residual stat→delete TOCTOU)
    /// The composite store now mints UNIQUE-PER-WRITE keys (`CompositeStore::mint_blob_key`), so
    /// the primary clobber path — a same-IRI overwrite REUSING a candidate's key — no longer arises (a
    /// recreate gets a fresh key). This atomic CAS is retained as DEFENCE-IN-DEPTH: any path or backend
    /// that could still present a key collision is covered. The reconciler re-stats a candidate just
    /// before deleting it, but a separate `stat()` then `delete()` could not close the residual gap (a
    /// rewrite landing between them would have its NEW live bytes clobbered) — so this method collapses
    /// the compare and the delete into ONE atomic step: a concurrent rewrite either lands BEFORE it (a new
    /// generation ⇒ witness mismatch ⇒ `Ok(false)`, not deleted) or AFTER it (the old bytes are already
    /// gone) — there is no clobber window.
    ///
    /// # Atomicity contract (load-bearing — an impl MUST honour it)
    /// The comparison AND the removal MUST happen under a single, uninterrupted critical section with no
    /// suspension point (no `await`, no lock release) between them. The in-memory double does the compare
    /// + remove under ONE `Mutex` lock acquisition, which is genuinely race-free.
    ///
    /// No trait DEFAULT is provided: a default built on `stat()`-then-`delete()` would NOT be atomic and
    /// would reintroduce exactly the TOCTOU this method exists to close — a non-atomic "default" would be
    /// a silent footgun. So every impl MUST provide a genuinely atomic implementation.
    ///
    /// M2-next (the `object_store` adapter): implement with a backend-native conditional/versioned delete
    /// keyed on the backend's OWN write version — the native ETag / object-version / generation that maps
    /// into [`BlobEntry::generation`] (`object_store` `PutMode`/delete with an `if_match`/version
    /// precondition on the backends that support it: S3 conditional writes / object versioning). On a
    /// backend WITHOUT a conditional delete (and no native version ⇒ [`BlobEntry::generation`] is `None`),
    /// safety rests on the **unique-per-write blob keys** the composite store now mints
    /// (`CompositeStore::mint_blob_key`): an overwrite never reuses a candidate's key, so the
    /// reconciler can never target live bytes and the delete is safe to be unconditional.
    async fn delete_if_unchanged(
        &self,
        key: &str,
        expected_generation: u64,
    ) -> Result<bool, BlobError>;
}

/// A stored blob in the in-memory double: the bytes, the insert/overwrite time (for the grace check),
/// and a strictly-increasing `generation` (the CAS witness — a true write version that a `last_modified`
/// cannot be, since two writes can share a timestamp but never a generation).
struct StoredBlob {
    body: Bytes,
    last_modified: SystemTime,
    generation: u64,
}

/// The stored blobs, with how many of them and how many of their bytes are in the service pool
/// ([`super::is_pooled_blob_key`]) kept as they change, so admission can hold the pool to its own
/// ceilings without walking the map.
#[derive(Default)]
struct Entries {
    map: HashMap<String, StoredBlob>,
    pooled: usize,
    pooled_bytes: usize,
}

impl Entries {
    fn get(&self, key: &str) -> Option<&StoredBlob> {
        self.map.get(key)
    }

    fn contains_key(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    fn len(&self) -> usize {
        self.map.len()
    }

    fn iter(&self) -> std::collections::hash_map::Iter<'_, String, StoredBlob> {
        self.map.iter()
    }

    /// Entries in the service pool, and the bytes they hold.
    fn pooled(&self) -> (usize, usize) {
        (self.pooled, self.pooled_bytes)
    }

    fn insert(&mut self, key: String, blob: StoredBlob) -> Option<StoredBlob> {
        let pooled = super::is_pooled_blob_key(&key);
        let bytes = blob.body.len();
        let replaced = self.map.insert(key, blob);
        if pooled {
            if let Some(old) = &replaced {
                self.pooled_bytes -= old.body.len();
            } else {
                self.pooled += 1;
            }
            self.pooled_bytes += bytes;
        }
        replaced
    }

    fn remove(&mut self, key: &str) -> Option<StoredBlob> {
        let gone = self.map.remove(key)?;
        if super::is_pooled_blob_key(key) {
            self.pooled -= 1;
            self.pooled_bytes -= gone.body.len();
        }
        Some(gone)
    }
}

/// Blob entries and their exact aggregate body-byte usage, protected by one lock so admission and
/// accounting are atomic under concurrent PUTs.
#[derive(Default)]
struct BlobState {
    entries: Entries,
    total_bytes: usize,
}

/// An in-memory [`BlobStore`] for tests and the M1 boot-without-S3 path.
///
/// Each [`put`](InMemoryBlobStore::put) stamps the wall-clock insert time, mirroring an object store's
/// `last_modified`, so the reconciler's grace window can be exercised without a real backend. Tests
/// that need a *specific* age use [`put_with_time`](InMemoryBlobStore::put_with_time) to back-date a
/// blob deterministically (no `sleep`).
///
/// Every write ALSO stamps a fresh `generation` from a store-wide monotonic counter (`next_generation`):
/// the counter is bumped and the new value written onto the entry on EVERY `put`/`put_with_time`, so any
/// overwrite — even one with an identical `last_modified` (clock granularity / rollback) — gets a
/// STRICTLY DIFFERENT generation. That generation is the CAS witness the reconciler deletes on, making
/// [`delete_if_unchanged`](BlobStore::delete_if_unchanged) race-free AND clock-independent (the HIGH fix:
/// a `SystemTime` is not a unique write version, a monotonic generation is).
pub struct InMemoryBlobStore {
    inner: Mutex<BlobState>,
    /// Store-wide monotonic write counter. Every write does `fetch_add(1)` and stamps the returned value
    /// onto the entry's `generation`, so generations are globally unique + strictly increasing across the
    /// store. The bump is done WHILE the store `Mutex` is held (Finding 2: `bump_generation_locked`), in
    /// the SAME critical section as the entry insert, so the increment and the insertion cannot interleave
    /// ⇒ generations are strictly WRITE-ORDERED, matching the strictly-increasing-write-version contract.
    /// The `AtomicU64` type is retained only to keep the bump callable through the `&self` API; all
    /// ordering/visibility comes from the surrounding `Mutex`, so `Relaxed` suffices.
    next_generation: AtomicU64,
    /// Immutable admission ceilings; checks happen under `inner`'s lock.
    limits: InMemoryStoreLimits,
}

impl Default for InMemoryBlobStore {
    fn default() -> Self {
        Self::with_limits(InMemoryStoreLimits::default())
    }
}

impl InMemoryBlobStore {
    /// Build with the bounded default limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build with explicit aggregate byte and stored-entry limits.
    pub fn with_limits(limits: InMemoryStoreLimits) -> Self {
        Self {
            inner: Mutex::new(BlobState::default()),
            next_generation: AtomicU64::new(0),
            limits,
        }
    }

    /// Return exact physical usage, including unreferenced versions awaiting reconciliation.
    pub fn usage(&self) -> Result<StoreUsage, BlobError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?;
        Ok(StoreUsage {
            total_bytes: guard.total_bytes,
            resource_count: guard.entries.len(),
        })
    }

    /// Return the immutable admission limits configured for this store.
    pub fn quota(&self) -> InMemoryStoreLimits {
        self.limits
    }

    /// Insert one body while atomically enforcing byte and entry ceilings and updating accounting.
    fn put_at(&self, key: &str, body: Bytes, last_modified: SystemTime) -> Result<(), BlobError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?;
        let previous_bytes = guard.entries.get(key).map_or(0, |blob| blob.body.len());
        let is_new = !guard.entries.contains_key(key);
        // A service-pool record is admitted against the pool's own ceilings and counts towards
        // nothing else, so no number of them makes a write of content fail.
        let pooled = super::is_pooled_blob_key(key);
        let (pool_entries, pool_bytes) = guard.entries.pooled();
        let held = if pooled {
            pool_entries
        } else {
            guard.entries.len() - pool_entries
        };
        let ceiling = if pooled {
            self.limits.max_pool_resource_count
        } else {
            self.limits.max_resource_count
        };
        if is_new && held >= ceiling {
            return Err(BlobError::QuotaExceeded);
        }
        let retained_bytes = guard
            .total_bytes
            .checked_sub(previous_bytes)
            .ok_or_else(|| BlobError::Backend("blob byte accounting underflow".into()))?;
        let next_total = retained_bytes
            .checked_add(body.len())
            .ok_or(BlobError::QuotaExceeded)?;
        let next_pool_bytes = if pooled {
            pool_bytes
                .checked_sub(previous_bytes)
                .ok_or_else(|| BlobError::Backend("blob byte accounting underflow".into()))?
                .checked_add(body.len())
                .ok_or(BlobError::QuotaExceeded)?
        } else {
            pool_bytes
        };
        let next_content_bytes = next_total
            .checked_sub(next_pool_bytes)
            .ok_or_else(|| BlobError::Backend("blob byte accounting underflow".into()))?;
        if next_pool_bytes > self.limits.max_pool_bytes
            || next_content_bytes > self.limits.max_total_bytes
        {
            return Err(BlobError::QuotaExceeded);
        }

        let generation = self.bump_generation_locked(&guard);
        guard.entries.insert(
            key.to_string(),
            StoredBlob {
                body,
                last_modified,
                generation,
            },
        );
        guard.total_bytes = next_total;
        Ok(())
    }

    /// The next strictly-increasing generation, read+incremented WHILE the store `Mutex` is held (the
    /// caller passes its lock guard). Generating AND stamping the generation inside the SAME critical
    /// section as the entry insert (Finding 2) makes the counter bump and the insertion ONE atomic step, so
    /// generations are strictly WRITE-ORDERED: a write that observes generation N is the one that inserts
    /// generation N, and no other write can interleave between the bump and the insert. (The earlier
    /// before-the-lock bump let two concurrent writes increment the counter and THEN race for the lock, so
    /// a write could be stamped out of insertion order — violating the strictly-increasing-write-version
    /// contract `delete_if_unchanged` relies on. Taking the guard makes the doc TRUE.)
    ///
    /// `Relaxed` is sufficient on the atomic: it is only ever touched under the `Mutex`, so the lock already
    /// provides the ordering/visibility — the atomic's own ordering carries no additional guarantee here.
    /// (The `AtomicU64` is retained over a plain `u64` only to avoid threading a `&mut` field through the
    /// `&self` API; correctness comes entirely from the surrounding lock.)
    fn bump_generation_locked(&self, _guard: &BlobState) -> u64 {
        self.next_generation.fetch_add(1, Ordering::Relaxed)
    }

    /// Insert bytes with an EXPLICIT last-modified time. Test-only helper so the grace-window tests can
    /// back-date a blob deterministically (e.g. "2 hours ago") instead of sleeping. Not part of the
    /// [`BlobStore`] trait — production code uses [`put`](InMemoryBlobStore::put), which stamps `now`.
    ///
    /// Still stamps a FRESH monotonic `generation` (the CAS witness) on every call, exactly like `put` —
    /// so two `put_with_time` calls with the SAME `last_modified` are correctly distinguished by their
    /// generations (the property the grace-vs-CAS separation relies on).
    pub fn put_with_time(
        &self,
        key: &str,
        body: Bytes,
        last_modified: SystemTime,
    ) -> Result<(), BlobError> {
        self.put_at(key, body, last_modified)
    }

    /// Read the current stamped generation of a key (test helper — lets a test capture the CAS witness a
    /// fresh stat would observe, so it can prove that a SAME-`last_modified` overwrite bumps the
    /// generation and is therefore refused by `delete_if_unchanged`).
    pub fn generation_of(&self, key: &str) -> Option<u64> {
        let guard = self.inner.lock().expect("blob store mutex poisoned");
        guard.entries.get(key).map(|b| b.generation)
    }
}

#[async_trait]
impl BlobStore for InMemoryBlobStore {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?;
        guard
            .entries
            .get(key)
            .map(|b| b.body.clone())
            .ok_or(BlobError::NotFound)
    }

    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        self.put_at(key, body, crate::clock::now())
    }

    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?;
        Ok(guard.entries.contains_key(key))
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?;
        if let Some(blob) = guard.entries.remove(key) {
            guard.total_bytes = guard
                .total_bytes
                .checked_sub(blob.body.len())
                .ok_or_else(|| BlobError::Backend("blob byte accounting underflow".into()))?;
        }
        Ok(())
    }

    async fn list(&self) -> Result<Vec<BlobEntry>, BlobError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?;
        Ok(guard
            .entries
            .iter()
            .map(|(key, blob)| BlobEntry {
                key: key.clone(),
                last_modified: Some(blob.last_modified),
                generation: Some(blob.generation),
            })
            .collect())
    }

    /// O(1) single-key re-stat (a HashMap lookup) instead of the trait default's whole-store
    /// enumeration — the shape the real object_store HEAD will take. Surfaces BOTH witnesses: the
    /// `last_modified` (the age/grace witness) and the `generation` (the CAS-delete witness).
    async fn stat(&self, key: &str) -> Result<Option<BlobEntry>, BlobError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?;
        Ok(guard.entries.get(key).map(|blob| BlobEntry {
            key: key.to_string(),
            last_modified: Some(blob.last_modified),
            generation: Some(blob.generation),
        }))
    }

    /// ATOMIC compare-and-delete: the compare AND the remove happen under a SINGLE `Mutex` lock
    /// acquisition with NO `await`/lock release between them, so it is genuinely race-free. The witness is
    /// the entry's `generation` (a strictly-increasing write version), NOT its `last_modified` — so it is
    /// also immune to clock issues: a concurrent `put`/`put_with_time` (a same-key overwrite) cannot
    /// interleave AND always bumps the generation, even if it lands in the same `SystemTime` tick or after
    /// a clock rollback. The overwrite either ran before this lock was taken (so the current generation no
    /// longer equals `expected_generation` ⇒ we DON'T remove, returning `false`) or it runs after we
    /// release (the old entry is already gone). Either way the overwrite's live bytes are never clobbered —
    /// there is no TOCTOU window and no same-timestamp ambiguity. Returns whether a row was removed.
    async fn delete_if_unchanged(
        &self,
        key: &str,
        expected_generation: u64,
    ) -> Result<bool, BlobError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| BlobError::Backend("poisoned".into()))?;
        // Compare the CURRENT generation to the witness, then remove, all while still holding the lock.
        match guard.entries.get(key) {
            Some(blob) if blob.generation == expected_generation => {
                let blob = guard
                    .entries
                    .remove(key)
                    .expect("entry was present under the same lock");
                guard.total_bytes = guard
                    .total_bytes
                    .checked_sub(blob.body.len())
                    .ok_or_else(|| BlobError::Backend("blob byte accounting underflow".into()))?;
                Ok(true)
            }
            // Key gone, or its generation moved since the witness was observed (a rewrite landed — even a
            // same-millisecond one) ⇒ do NOT delete. The bytes under this key are no longer the ones the
            // reconciler decided to GC.
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn two_writes_acquire_strictly_increasing_generations() {
        // Finding 2 (the write-order contract): each successive write must observe a STRICTLY GREATER
        // generation than the one before it — the property `delete_if_unchanged` relies on to tell two
        // writes apart. (Mutation-check: a counter that did not strictly increase — e.g. a stamp that
        // reused a value — would fail the `<` assertions.)
        let blob = InMemoryBlobStore::new();
        let same_stamp = SystemTime::now() - Duration::from_secs(3600);

        blob.put_with_time("k", Bytes::from_static(b"v1"), same_stamp)
            .unwrap();
        let g1 = blob.generation_of("k").expect("v1 exists");
        // A same-key OVERWRITE at the IDENTICAL last_modified still bumps the generation.
        blob.put_with_time("k", Bytes::from_static(b"v2"), same_stamp)
            .unwrap();
        let g2 = blob.generation_of("k").expect("v2 exists");
        // A DIFFERENT key advances the same store-wide counter.
        blob.put_with_time("other", Bytes::from_static(b"o"), same_stamp)
            .unwrap();
        let g3 = blob.generation_of("other").expect("other exists");

        assert!(
            g1 < g2,
            "an overwrite must get a strictly greater generation"
        );
        assert!(
            g2 < g3,
            "the store-wide generation counter is strictly increasing across keys too"
        );
    }

    #[tokio::test]
    async fn byte_and_entry_limits_fail_without_mutating_usage() {
        // Mutation witness: removing either admission check makes its corresponding
        // rejected put succeed and changes the pinned usage below.
        let limits = InMemoryStoreLimits::new(4, 2);
        let blob = InMemoryBlobStore::with_limits(limits);

        blob.put("a", Bytes::from_static(b"123")).await.unwrap();
        assert_eq!(
            blob.usage().unwrap(),
            StoreUsage {
                total_bytes: 3,
                resource_count: 1,
            }
        );
        assert!(matches!(
            blob.put("b", Bytes::from_static(b"45")).await,
            Err(BlobError::QuotaExceeded)
        ));
        assert_eq!(blob.usage().unwrap().total_bytes, 3);

        // Replacing a key subtracts its old body before testing the aggregate byte ceiling.
        blob.put("a", Bytes::from_static(b"1234")).await.unwrap();
        blob.put("b", Bytes::new()).await.unwrap();
        assert!(matches!(
            blob.put("c", Bytes::new()).await,
            Err(BlobError::QuotaExceeded)
        ));
        assert_eq!(
            blob.usage().unwrap(),
            StoreUsage {
                total_bytes: 4,
                resource_count: 2,
            }
        );

        blob.delete("a").await.unwrap();
        assert_eq!(
            blob.usage().unwrap(),
            StoreUsage {
                total_bytes: 0,
                resource_count: 1,
            }
        );
        assert_eq!(blob.quota(), limits);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_puts_cannot_over_admit_the_limits() {
        // The check and insert share one lock: a racing burst can fill, never overshoot.
        let blob = Arc::new(InMemoryBlobStore::with_limits(InMemoryStoreLimits::new(
            8, 8,
        )));
        let mut handles = Vec::new();
        for n in 0..64 {
            let blob = Arc::clone(&blob);
            handles.push(tokio::spawn(async move {
                blob.put(&format!("k{n}"), Bytes::from_static(b"x")).await
            }));
        }

        let mut admitted = 0;
        for handle in handles {
            match handle.await.unwrap() {
                Ok(()) => admitted += 1,
                Err(BlobError::QuotaExceeded) => {}
                Err(other) => panic!("unexpected blob error: {other}"),
            }
        }
        assert_eq!(admitted, 8);
        assert_eq!(
            blob.usage().unwrap(),
            StoreUsage {
                total_bytes: 8,
                resource_count: 8,
            }
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_writes_get_unique_strictly_ordered_generations() {
        // Finding 2 (the concurrency crux): with the bump done WHILE the lock is held, the counter
        // increment and the entry insert are one atomic critical section, so concurrent writes can never be
        // stamped out of write order. Two observable invariants after a burst of concurrent same-key
        // overwrites:
        //   (1) every generation handed out is UNIQUE (no two writes share a CAS witness), and
        //   (2) the SURVIVING entry's body matches the generation it carries — i.e. the write that won the
        //       lock is the write whose generation is stamped (bump and insert did not interleave). Under
        //       the OLD before-the-lock bump, a write could consume a high generation, then lose the lock
        //       race to a lower-generation write that inserts afterwards, leaving the entry's stamped
        //       generation NOT the highest actually inserted — the contract violation this fixes.
        let blob = Arc::new(InMemoryBlobStore::new());
        let n: usize = 200;

        let mut handles = Vec::new();
        for i in 0..n {
            let blob = Arc::clone(&blob);
            handles.push(tokio::spawn(async move {
                // Each task writes a DISTINCT body so the surviving body identifies the winning write.
                blob.put(&format!("k{i}"), Bytes::from(format!("v{i}")))
                    .await
                    .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        // (1) All N writes landed under distinct keys, each with a UNIQUE generation.
        let entries = blob.list().await.unwrap();
        assert_eq!(entries.len(), n, "every concurrent write must persist");
        let mut gens: Vec<u64> = entries
            .iter()
            .map(|e| {
                e.generation
                    .expect("in-memory store always stamps a generation")
            })
            .collect();
        gens.sort_unstable();
        gens.dedup();
        assert_eq!(
            gens.len(),
            n,
            "every concurrent write must get a UNIQUE generation (no duplicate CAS witnesses)"
        );

        // (2) Now hammer ONE key concurrently and assert the surviving entry's generation is the GREATEST
        // stamped to that key — the bump and the insert were atomic, so the last-ordered write won and was
        // stamped consistently (no interleave that leaves a stale body under a non-max generation).
        let blob2 = Arc::new(InMemoryBlobStore::new());
        let mut handles2 = Vec::new();
        for i in 0..n {
            let blob2 = Arc::clone(&blob2);
            handles2.push(tokio::spawn(async move {
                blob2
                    .put("hot", Bytes::from(format!("v{i}")))
                    .await
                    .unwrap();
                blob2.generation_of("hot")
            }));
        }
        let mut observed = Vec::new();
        for h in handles2 {
            if let Some(g) = h.await.unwrap() {
                observed.push(g);
            }
        }
        let surviving_gen = blob2.generation_of("hot").expect("hot exists");
        let max_observed = *observed.iter().max().expect("at least one write");
        assert_eq!(
            surviving_gen, max_observed,
            "the surviving entry must carry the GREATEST generation stamped to the key — bump+insert atomic"
        );
    }
}
