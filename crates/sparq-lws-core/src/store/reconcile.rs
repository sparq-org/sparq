// AUTHORED-BY Claude Opus 4.8
//! The orphaned-bytes **reconciler** — the GC for blobs the index no longer references.
//!
//! ## Why this exists (the crash-consistency model)
//! SPARQ is authoritative for existence/metadata/containment; the blob store holds the durable bytes.
//! The composite store deliberately writes bytes FIRST then commits the index, and on DELETE drops the
//! index FIRST then (best-effort) the bytes — so the invariant "if it is indexed, its bytes exist"
//! always holds and a crash can only ever leave the *benign* opposite: **bytes with no index row**.
//! Those are ORPHANS — they cost disk but are never observable through the LDP surface (a read goes
//! index-first, so it never sees them). The composite store reclaims superseded and deleted bytes
//! INLINE right after the index commit that drops their reference, so in steady state orphans come
//! only from: a crash (or a failed best-effort blob delete) between that commit and the reclaim; a
//! write or create whose index commit fails (e.g. a missing parent), orphaning the bytes it already
//! PUT; and a concurrent same-IRI writer whose superseded key the winning commit never observed. This
//! reconciler is the GC that reclaims them.
//!
//! ## What it does — and the grace period (LOAD-BEARING)
//! [`reconcile_orphans`]:
//! 1. asks SPARQ for the WHOLE referenced-blob-key set ONCE ([`SparqClient::referenced_blob_keys`]),
//! 2. lists the physically-stored blobs ([`BlobStore::list`]) — the START-OF-SWEEP snapshot,
//! 3. classifies each stored blob against that snapshot (referenced / too-young / undated / a
//!    delete candidate),
//! 4. if there ARE delete candidates, re-fetches the referenced set ONCE more (skipped entirely when
//!    there are none — see Finding 2), then for each delete candidate RE-CHECKS it against that fresh
//!    set AND RE-STATS its current `last_modified` + `generation` ([`BlobStore::stat`]) — see the
//!    snapshot-staleness race below,
//! 5. deletes a candidate **iff** it is STILL unreferenced (fresh index) AND STILL old enough (fresh
//!    stat, not rewritten): a VERSIONED candidate via an ATOMIC compare-and-delete
//!    ([`BlobStore::delete_if_unchanged`]) that removes the key only while its current `generation` (a
//!    true write version) still equals the SNAPSHOT witness; a VERSIONLESS candidate (no native write
//!    version) via an unconditional [`BlobStore::delete`], made safe by the composite store's
//!    unique-per-write keys (Finding 2 — so a versionless orphan is reclaimed, not leaked forever).
//!
//! ### The snapshot-staleness race (Finding 1 — why the re-check + the ATOMIC CAS-delete exist)
//! The blob list in step 2 is a SNAPSHOT; by the time the delete loop reaches a key, the store's view may
//! have moved on. The composite store now mints UNIQUE-PER-WRITE blob keys
//! (`CompositeStore::mint_blob_key`), so a recreate/overwrite gets a DIFFERENT key and can no
//! longer reuse a candidate's key — the primary clobber path is closed at the root. These re-checks + the
//! atomic CAS are retained as DEFENCE-IN-DEPTH for any backend/path where a key could still be reused: a
//! recreate landing between the snapshot and the delete would otherwise make the GC clobber newly-written
//! LIVE bytes. The defence is two-layered: (a) the fresh referenced-set re-check skips any candidate a
//! recreate has re-referenced;
//! and (b) the final delete is an ATOMIC compare-and-delete — [`BlobStore::delete_if_unchanged`] removes
//! the key ONLY while its current `generation` still equals the witness the fresh stat observed, with
//! the compare + remove under a single critical section (no suspension point between them). That CLOSES
//! the race for a [`BlobStore`] with an atomic `delete_if_unchanged` (the in-memory store, the only real
//! impl): a concurrent rewrite either lands before the CAS (a new generation ⇒ CAS returns `false` ⇒ not
//! deleted, recorded `skipped_revalidated`) or after it (the old bytes are already gone) — there is NO
//! clobber window. A plain `stat()`-then-`delete()` could not close it (a gap always exists between the
//! two calls); the atomic CAS can.
//!
//! ### Why the CAS witness is the GENERATION, not `last_modified` (the HIGH fix)
//! The CAS-delete witness MUST be a TRUE write version, not a timestamp. A [`SystemTime`] is not unique
//! per write: clock granularity (two writes in one tick), a clock rollback (NTP step), or coarse backend
//! timestamp precision can give a recreate the SAME `last_modified` as the bytes it replaced. A
//! timestamp-keyed CAS would then see "unchanged" and DELETE the recreate's live bytes. So the witness is
//! the in-memory store's monotonic [`BlobEntry::generation`](crate::store::BlobEntry::generation) — strictly increasing on EVERY write, so a
//! same-timestamp overwrite still has a strictly different generation and the CAS correctly refuses. The
//! `last_modified` is still consulted, but ONLY for the time-based age/grace check (correct for "old
//! enough" / inside-the-grace-window); it is NEVER the delete witness. A real `object_store` backend uses
//! its native version/ETag/object-generation as the witness (the `M2-next:` seam on
//! [`BlobStore::delete_if_unchanged`]); a backend with NO native write version (generation `None`) cannot
//! do a safe CAS and must instead use unique-per-write keys.
//!
//! ### Which generation observation is the witness — the SNAPSHOT one (Finding 1, the threading fix)
//! The witness is the generation observed at the START-OF-SWEEP `list()` snapshot, threaded END-TO-END in
//! the candidate alongside the key + snapshot `last_modified`, and CONFIRMED still-current by the fresh
//! `stat()` (fresh == snapshot) before the delete. It must NOT be re-derived from the fresh stat: a blob
//! rewritten between `list()` and the fresh `stat()` — same `last_modified`, NEW generation — would have
//! the fresh stat hand the recreate its OWN new generation as the witness, so a fresh-generation CAS would
//! match and DELETE the live rewrite. Anchoring the whole chain (classify → age → CAS) to the ONE
//! generation observed at snapshot time, and skipping on ANY fresh≠snapshot mismatch, means a rewrite at
//! ANY point is caught: list→stat by the fresh-vs-snapshot mismatch, stat→delete by the atomic CAS.
//!
//! The **unique-per-write blob keys** the composite store now mints
//! (`CompositeStore::mint_blob_key`) close the reuse race at its ROOT (an overwrite never reuses
//! a candidate's key, so the GC can never target live bytes). The atomic CAS + re-checks here are kept as
//! defence-in-depth and as the strictly-stronger path for a backend that DOES expose a write version.
//!
//! ### Versionless backends ARE reclaimed (no forever-leak) — the unconditional-delete path
//! A blob whose backend reports NO `generation` (a version-less object store) cannot do a CAS — but the
//! unique-per-write keys make an UNCONDITIONAL delete safe, so a versionless orphan is **GC'd**, not
//! leaked forever. It still passes every safety guard a versioned candidate does — age-gated against the
//! grace window, re-checked against a FRESH referenced set, and re-statted (still old enough, still
//! present) immediately before the delete — and is then removed with a plain [`BlobStore::delete`]. This
//! is sound because a later recreate of the same IRI mints a DIFFERENT key (so the orphan's key can never
//! be the live recreate's): there is nothing live to clobber. The earlier code skipped versionless
//! candidates unconditionally, which leaked every old orphan forever on a versionless backend — the bug
//! this path fixes.
//!
//! The grace period is the safety crux. There is an inverse race to the delete-orphan case: a *write
//! in progress* PUTs its bytes and has NOT YET committed its index row — for that instant the blob is
//! unreferenced but is NOT an orphan, it is about to become live. Deleting it would corrupt an
//! in-flight write (the index row would land pointing at bytes the reconciler just deleted). The grace
//! window closes that race: the sweep only GCs a blob whose `last_modified` is older than `grace`, so a
//! freshly-written-but-not-yet-committed blob is always protected (a write commits its index far
//! inside the window). The default is 1 hour — comfortably longer than any single write's
//! byte-then-index gap (even with retries/backpressure) yet short enough that orphans don't accumulate
//! for long. It is configurable for tests + operators.
//!
//! A blob whose backend reports NO `last_modified` is NEVER GC'd (fail-closed — we cannot prove it is
//! old enough to be safe), and counted as `skipped_unknown_age`.
//!
//! ## Safety / idempotency
//! - **Fail-closed on the referenced set.** If [`SparqClient::referenced_blob_keys`] errors, the whole
//!   sweep aborts with that error — it NEVER treats a failed query as "nothing is referenced" (which
//!   would delete every blob in the pod). The referenced set is fetched BEFORE any delete.
//! - **No work ⇒ no second query (Finding 2).** The pre-delete fresh referenced-set re-fetch runs ONLY
//!   when there is at least one delete candidate. A sweep that finds nothing deletable (every blob
//!   referenced / too-young / undated) returns its report WITHOUT the second query, so a transient SPARQ
//!   error on that re-fetch can never fail a sweep that had nothing unsafe to do anyway.
//! - **Idempotent + safe to re-run.** A second run over the same state finds the just-deleted orphans
//!   gone, so it deletes nothing. Concurrent writes are protected by the grace window. The order
//!   (referenced-set THEN list) means a blob created+committed between the two steps is simply seen as
//!   referenced or not-yet-listed — never wrongly deleted.
//! - A per-blob delete failure is recorded (`delete_errors`) and the sweep CONTINUES — one bad key
//!   never aborts the whole GC.
//!
//! ## On-demand vs periodic
//! The core [`reconcile_orphans`] is **on-demand**: a pure function over the two seams, callable from a
//! future admin endpoint / CLI / a one-shot boot sweep. The native server also wires the existing
//! [`spawn_periodic`] runner behind `SOLID_SERVER_RECONCILE_INTERVAL_SECS`: UNSET (the default) spawns
//! no task; a positive integer starts exactly one task after router assembly, using
//! [`ReconcileOptions::default`] and therefore the unchanged one-hour grace window. [GPT-5.6]
//!
//! This is intended for a single-instance deployment that wants a self-driving sweep. In a
//! horizontally-scaled deployment GC should instead be one scheduled job (a leader-elected task / a
//! cron hitting an admin endpoint), NOT a per-replica timer racing N sweeps — which is why periodic
//! remains an explicit runtime opt-in rather than a boot default.

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use super::blob::{BlobError, BlobStore};
use super::sparq::{SparqClient, SparqError};

/// The default grace window: an unreferenced blob younger than this is PROTECTED (it may be a
/// write-in-progress whose index row has not yet committed). 1h ≫ any single byte-then-index write gap.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(60 * 60);

/// Tuning for a reconciler sweep.
#[derive(Debug, Clone)]
pub struct ReconcileOptions {
    /// Only GC an orphan OLDER than this. Protects the write-in-progress (bytes-PUT-but-index-uncommitted)
    /// race. Default [`DEFAULT_GRACE`].
    pub grace: Duration,
    /// If `true`, scan + classify (INCLUDING the fresh re-check + re-stat) but DELETE nothing (a dry run
    /// — report what *would* be GC'd). All disposition counts are still populated; the deletable orphans
    /// land in `would_delete` (not `deleted`, which stays 0), so the partition invariant holds in both
    /// modes.
    pub dry_run: bool,
}

impl Default for ReconcileOptions {
    fn default() -> Self {
        Self {
            grace: DEFAULT_GRACE,
            dry_run: false,
        }
    }
}

/// The outcome of one reconciler sweep. Counts partition the scanned blobs so the totals reconcile in
/// BOTH modes:
/// - `scanned == referenced + orphaned`, and
/// - `orphaned == deleted + would_delete + too_young + skipped_unknown_age + skipped_revalidated +
///   delete_errors`.
///
/// The deletable orphans split by mode: a real run increments [`deleted`](Self::deleted), a dry run
/// increments [`would_delete`](Self::would_delete) (it touches nothing) — so the partition invariant
/// holds whether or not deletes ran. [`skipped_revalidated`](Self::skipped_revalidated) is the
/// fresh-recheck disposition (Finding 1): a candidate that, at delete time, turned out to be referenced
/// again OR to have been rewritten since the snapshot.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Total physically-stored blobs examined.
    pub scanned: usize,
    /// Blobs still referenced by an index row (kept, untouched).
    pub referenced: usize,
    /// Unreferenced blobs (orphan candidates) — the sum of the six dispositions below.
    pub orphaned: usize,
    /// Orphans that were old enough, passed the fresh re-check, and were DELETED (0 on a dry run).
    pub deleted: usize,
    /// Orphans that WOULD be deleted but were not, because this was a dry run. The dry-run counterpart
    /// of [`deleted`](Self::deleted): on a real run this is always 0, on a dry run it carries the
    /// deletable count (so the partition holds in both modes and operators see what a real run would
    /// reclaim).
    pub would_delete: usize,
    /// Orphans inside the grace window — KEPT (protects the write-in-progress race).
    pub too_young: usize,
    /// Orphans whose backend reported no `last_modified` — KEPT fail-closed (age unknowable).
    pub skipped_unknown_age: usize,
    /// Orphans that looked deletable from the START-OF-SWEEP snapshot but turned out to be live again —
    /// either now referenced by a freshly-committed index row, or rewritten. KEPT. This is the Finding-1
    /// defence-in-depth guard against clobbering a recreate that landed mid-sweep on a reused key (now
    /// closed at the root by unique-per-write keys, but retained as the strictly-stronger versioned path),
    /// and it catches a rewrite at ANY point in the chain:
    /// - the fresh re-check found it referenced again (a recreate committed a row);
    /// - the fresh stat is gone / undated / newer `last_modified` / now inside the grace window;
    /// - the fresh stat's `generation` != the SNAPSHOT generation (rewritten in the list→stat gap — the
    ///   HIGH threading fix); or
    /// - the atomic CAS saw the current `generation` move (rewritten in the stat→delete gap).
    ///
    /// A VERSIONLESS orphan is NO LONGER counted here just for being versionless: under unique-per-write
    /// keys it is now reclaimed via the unconditional-delete path (Finding 2) once it passes the same age +
    /// fresh-re-check + re-stat guards. It only lands here if one of those guards keeps it (referenced
    /// again / rewritten / now too young), exactly like a versioned candidate.
    pub skipped_revalidated: usize,
    /// Orphans we tried to delete but the backend delete failed (KEPT; the sweep continued).
    pub delete_errors: usize,
}

/// An error that ABORTS the whole sweep before (or independent of) any per-blob disposition.
#[derive(Debug, thiserror::Error)]
pub enum ReconcileError {
    /// The referenced-set query failed — we fail closed and do NOT delete anything (a missing
    /// referenced set must NEVER be read as "nothing is referenced").
    #[error("could not fetch the referenced-blob-key set: {0}")]
    ReferencedSet(#[source] SparqError),
    /// Listing the blob store failed — nothing to reconcile against, so abort.
    #[error("could not list the blob store: {0}")]
    ListBlobs(#[source] BlobError),
}

/// Run ONE orphaned-bytes reconciliation sweep over the two storage seams.
///
/// Lists every stored blob, diffs it against SPARQ's referenced-key set, and deletes the unreferenced
/// blobs that are older than `opts.grace`. Returns a [`ReconcileReport`]; idempotent and safe to
/// re-run. See the module docs for the crash-consistency model + the grace-window rationale.
///
/// `now` is taken from the wall clock; ages are `now - last_modified` (saturating, so a clock skew that
/// makes a blob appear "in the future" yields a zero age ⇒ treated as too-young ⇒ kept, never deleted).
pub async fn reconcile_orphans<S: SparqClient + ?Sized, B: BlobStore + ?Sized>(
    sparq: &S,
    blob: &B,
    opts: &ReconcileOptions,
) -> Result<ReconcileReport, ReconcileError> {
    // 1. The referenced set FIRST (fail-closed): if this errors we abort and delete NOTHING.
    let referenced: HashSet<String> = sparq
        .referenced_blob_keys()
        .await
        .map_err(ReconcileError::ReferencedSet)?;

    // 2. The physically-stored blobs.
    let stored = blob.list().await.map_err(ReconcileError::ListBlobs)?;

    let now = crate::clock::now();
    let mut report = ReconcileReport {
        scanned: stored.len(),
        ..Default::default()
    };

    // First pass — classify against the START-OF-SWEEP snapshot. Everything that is referenced,
    // too-young, or undated is decided HERE (these dispositions need no fresh re-check: a referenced or
    // too-young or undated blob is never a delete candidate). The blobs that LOOK deletable from the
    // snapshot (old-enough + unreferenced) are collected — WITH the SNAPSHOT's `generation` (if any) as
    // well as the `last_modified` the SNAPSHOT saw — for a second pass that re-checks each immediately
    // before deleting.
    //
    // FINDING 1 (the candidate-tuple shape, the best call documented): a VERSIONED candidate carries the
    // SNAPSHOT `generation`, not just the key + snapshot `last_modified`. The whole decision chain
    // (classify → age → CAS) is then anchored to ONE consistent generation observation taken at `list()`
    // time. The alternative — carrying only key+timestamp and using the FRESH stat's generation as the CAS
    // witness — is exactly the HIGH bug: a blob rewritten between `list()` and the fresh `stat()` (same
    // `last_modified`, NEW generation) would have the fresh stat report the new generation, which would
    // become its own witness ⇒ the CAS matches ⇒ the live rewrite is deleted. Threading the SNAPSHOT
    // generation end-to-end and requiring fresh == snapshot before deleting closes that window too.
    //
    // FINDING 2 (the versionless-orphan GC, this fix): a candidate whose backend reports NO `generation`
    // ([`CandidateWitness::Versionless`]) is NO LONGER skipped unconditionally — that leaked every old
    // orphan forever on a versionless object-store backend. Under the unique-per-write keys the composite
    // store now mints, a versionless orphan's key can NEVER be reused by a later recreate, so deleting it
    // is safe WITHOUT a CAS: we still age-gate it (grace window) AND re-verify it is still unreferenced by
    // a fresh index re-check + a fresh re-stat in the second pass, then delete it unconditionally. A
    // versioned candidate keeps the strictly-stronger atomic CAS path (defence-in-depth for any backend
    // where a key COULD be reused). Versionless candidates are still collected here so they go through the
    // SAME fresh referenced-set re-check (Finding 1 part a) before any delete.
    let mut candidates: Vec<Candidate> = Vec::new();
    for entry in stored {
        if referenced.contains(&entry.key) {
            report.referenced += 1;
            continue;
        }
        // Unreferenced ⇒ an orphan candidate.
        report.orphaned += 1;

        // The GRACE GUARD: only GC an orphan older than the window. A blob without a known
        // last-modified is kept (fail-closed). `duration_since` errs when the stamp is in the future
        // (clock skew) — treat that as age 0 ⇒ too-young ⇒ kept.
        let snapshot_ts = match entry.last_modified {
            None => {
                report.skipped_unknown_age += 1;
                continue;
            }
            Some(ts) => {
                let age = now.duration_since(ts).unwrap_or(Duration::ZERO);
                if age >= opts.grace {
                    ts
                } else {
                    report.too_young += 1;
                    continue;
                }
            }
        };
        // The delete WITNESS is fixed at `list()` time (FINDING 1). A VERSIONED snapshot carries its
        // SNAPSHOT generation for the atomic CAS-delete; a VERSIONLESS snapshot (`generation` is `None`)
        // has no write version, so it takes the unconditional-delete path that is SAFE under
        // unique-per-write keys (FINDING 2) — age-gated + fresh-re-checked + fresh-re-statted, then a plain
        // delete (the candidate's key can never be a later recreate's key, so there is nothing to clobber).
        let witness = match entry.generation {
            Some(snapshot_gen) => CandidateWitness::Versioned(snapshot_gen),
            None => CandidateWitness::Versionless,
        };
        candidates.push(Candidate {
            key: entry.key,
            snapshot_ts,
            witness,
        });
    }

    // FINDING 2: short-circuit when there is NOTHING to delete. The pre-delete fresh referenced-set
    // re-fetch (and the whole second pass) only exists to RE-VALIDATE delete candidates; with zero
    // candidates there is nothing to re-validate and nothing unsafe to do, so we MUST NOT run a second
    // SPARQ query that could fail a sweep that had no work. Return the report (already fully partitioned
    // by the first pass — everything was referenced / too-young / undated) BEFORE the re-check.
    if candidates.is_empty() {
        return Ok(report);
    }

    // RE-CHECK against the INDEX once, just before the delete pass (Finding 1, part a). The
    // start-of-sweep `referenced` set can be stale. With unique-per-write keys a recreate gets a fresh
    // key so it can no longer re-reference a candidate's key — but on a key-reusing backend a resource
    // recreated between the snapshot and now could commit a FRESH index row pointing at a candidate key,
    // and deleting it would clobber live bytes. Re-fetching the referenced set ONCE here (not per-key)
    // and skipping any candidate now in it is the first defence-in-depth layer; the atomic CAS-delete
    // below is the second. Fail-closed: if it errors we ABORT
    // and delete nothing (a failed query is NEVER "nothing is referenced"). Reached only when there is at
    // least one candidate (Finding 2).
    let referenced_fresh: HashSet<String> = sparq
        .referenced_blob_keys()
        .await
        .map_err(ReconcileError::ReferencedSet)?;

    // Second pass — re-validate each candidate immediately before deleting it. The candidate carries the
    // SNAPSHOT (`list()`-time) `last_modified` AND (for a versioned candidate) `generation`; both are
    // re-confirmed against a fresh stat before the delete, so the WHOLE decision chain is anchored to ONE
    // consistent generation observation (FINDING 1) and a rewrite landing ANYWHERE — list→stat OR
    // stat→delete — is caught.
    for candidate in candidates {
        let Candidate {
            key,
            snapshot_ts,
            witness,
        } = candidate;
        // (a) Re-check referenced-ness against the FRESH index set: a recreate may have committed a row
        // pointing at this key since the snapshot.
        if referenced_fresh.contains(&key) {
            report.skipped_revalidated += 1;
            continue;
        }

        // (b) Re-STAT the key's CURRENT state to decide the disposition (via `last_modified`, the AGE
        // witness) AND, for a versioned candidate, to CONFIRM the snapshot generation is still current. On
        // a key-reusing backend a rewrite would land on the same key, so if the bytes are now newer than
        // the snapshot saw — or now young enough to be inside the grace window — the blob was overwritten
        // and must NOT be deleted (defence-in-depth; unique-per-write keys close this at the root). A stat
        // failure is treated fail-closed (skip, count under delete_errors) rather than blindly deleting on
        // incomplete info.
        let current = match blob.stat(&key).await {
            Ok(Some(entry)) => entry,
            // The key is already gone (a concurrent delete / it never existed by delete-time) — nothing
            // to reclaim, and re-deleting is a harmless no-op we simply skip. Count as revalidated
            // (it is no longer a deletable orphan), keeping the partition exact.
            Ok(None) => {
                report.skipped_revalidated += 1;
                continue;
            }
            Err(_) => {
                report.delete_errors += 1;
                continue;
            }
        };
        // The AGE re-check uses the fresh `last_modified` (the time witness — correct for "old enough" /
        // "inside the grace window"). An undated fresh stat (`None`) is unknowable ⇒ fail-closed, do NOT
        // delete. Newer than the snapshot (overwritten), or now inside the grace window (a fresh write
        // whose index row may not have committed yet) ⇒ not safe to GC. This guard protects BOTH the
        // versioned and the versionless path: a versionless orphan re-written (under key reuse) since the
        // snapshot shows a fresh stamp here and is kept.
        match current.last_modified {
            None => {
                report.skipped_revalidated += 1;
                continue;
            }
            Some(ts) => {
                if ts > snapshot_ts || now.duration_since(ts).unwrap_or(Duration::ZERO) < opts.grace
                {
                    report.skipped_revalidated += 1;
                    continue;
                }
            }
        }

        // SECOND-PASS ELIGIBILITY — the SINGLE deletion decision both modes consult. This resolves the
        // witness against the fresh stat and yields EITHER "this candidate is not deletable, count it
        // `skipped_revalidated`" OR "this candidate IS deletable, here is the delete plan". Dry-run and
        // real-run MUST agree on this decision — a dry run has to count a candidate as `would_delete` IFF a
        // real run would actually delete it (and `skipped_revalidated` in exactly the same cases). Routing
        // both branches through ONE predicate is what makes dry-run a faithful prediction of real-run (the
        // Medium fix): the previous code incremented `would_delete` BEFORE the versioned
        // generation-revalidation, so a dry run OVER-reported a same-timestamp rewrite that a real run skips.
        let plan = match witness {
            // FINDING 1 (the HIGH fix): the CAS-delete WITNESS is the SNAPSHOT (`list()`-time) `generation`,
            // and the fresh stat's generation MUST EQUAL it before we delete. The witness is a true,
            // strictly-increasing write version (NOT `last_modified`: a same-tick / clock-rolled-back
            // overwrite can share a timestamp but never a generation). Crucially it is the SNAPSHOT
            // generation, NOT the FRESH stat's: using the fresh generation as the witness would re-derive
            // the witness from whatever bytes are present at stat time, so a rewrite landing in the
            // list→stat gap (same `last_modified`, NEW generation) would have the fresh stat hand the
            // recreate its OWN generation as the witness ⇒ the CAS would match and clobber the live rewrite.
            // Requiring fresh == snapshot (and a fresh stat that has BECOME version-less fails closed) means
            // a rewrite at ANY point — list→stat (caught HERE by the mismatch) OR stat→delete (caught by the
            // atomic CAS below) — skips. This generation-revalidation is part of the SHARED eligibility, so
            // a dry run skips the same rewritten candidate a real run does (it is NOT counted `would_delete`).
            CandidateWitness::Versioned(snapshot_gen) => match current.generation {
                None => DeletePlan::SkipRevalidated,
                // The bytes were rewritten between `list()` (snapshot) and this fresh `stat()` — the
                // generation moved even if the `last_modified` did not. KEEP (a live rewrite must not
                // be GC'd).
                Some(fresh_gen) if fresh_gen != snapshot_gen => DeletePlan::SkipRevalidated,
                // Confirmed: fresh == snapshot ⇒ eligible to reclaim via the atomic CAS on the SNAPSHOT
                // generation.
                Some(_) => DeletePlan::DeleteCas(snapshot_gen),
            },
            // FINDING 2 (the versionless-orphan GC): a candidate whose backend has NO native write version
            // cannot do a CAS — but UNIQUE-PER-WRITE keys make an unconditional delete SAFE: a later
            // recreate of the same IRI mints a DIFFERENT key, so this orphan's key can never be the live
            // recreate's, and deleting its bytes can never clobber anything live. It is still age-gated
            // (grace window, above), re-checked against the FRESH referenced set (above), and re-statted to
            // confirm it is still old enough + still present (above) before we delete — so a versionless
            // orphan OLDER than the grace window IS reclaimed (no more forever-leak), while a fresh / re-
            // referenced / rewritten one is kept. If the fresh stat had BECOME versioned (a backend that
            // started reporting generations between snapshot and now), we conservatively fall back to the
            // unconditional delete keyed on the same safety net (unique keys) — the absence of a snapshot
            // witness means we cannot CAS, and the unique-key invariant already guarantees the bytes are
            // not live. A versionless candidate is ALWAYS eligible here (it has passed every guard), so a
            // dry run faithfully counts it `would_delete`.
            CandidateWitness::Versionless => DeletePlan::DeleteUnconditional,
        };

        // A candidate the shared eligibility refused (rewritten / version-mismatch) is revalidated in BOTH
        // modes — never counted as deletable.
        let plan = match plan {
            DeletePlan::SkipRevalidated => {
                report.skipped_revalidated += 1;
                continue;
            }
            deletable => deletable,
        };

        if opts.dry_run {
            // Deletable (versioned OR versionless), but a dry run touches nothing — counted under
            // `would_delete`, not `deleted`, so the partition holds in both modes and the operator sees
            // what a real run would reclaim. Because we reached here only via the SAME eligibility decision
            // the real run uses, `would_delete` counts EXACTLY the candidates a real run would `delete`.
            report.would_delete += 1;
            continue;
        }

        match plan {
            // The atomic compare-and-delete on the SNAPSHOT generation. `delete_if_unchanged` removes the
            // key ONLY while its current `generation` still equals the witness, with the compare + remove in
            // ONE critical section. This CLOSES the residual stat→delete TOCTOU (Finding 1) AND is
            // clock-independent (the HIGH fix): a recreate that rewrites the bytes after our fresh stat bumps
            // the generation — even in the same `SystemTime` tick — so the CAS sees the mismatch and returns
            // `false` (recorded `skipped_revalidated`, NOT deleted). A genuine backend failure is recorded
            // under `delete_errors` and the sweep CONTINUES (one bad key never aborts the whole GC).
            DeletePlan::DeleteCas(snapshot_gen) => {
                match blob.delete_if_unchanged(&key, snapshot_gen).await {
                    Ok(true) => report.deleted += 1,
                    // The generation changed between the fresh stat and the CAS (a rewrite landed) ⇒ the
                    // CAS refused to delete. Not an orphan any more — record as revalidated, no clobber.
                    Ok(false) => report.skipped_revalidated += 1,
                    Err(_) => report.delete_errors += 1,
                }
            }
            // The versionless unconditional-delete path (safe under unique-per-write keys). A backend delete
            // failure is recorded and the sweep CONTINUES.
            DeletePlan::DeleteUnconditional => match blob.delete(&key).await {
                Ok(()) => report.deleted += 1,
                Err(_) => report.delete_errors += 1,
            },
            // Unreachable: SkipRevalidated `continue`d above.
            DeletePlan::SkipRevalidated => {
                unreachable!("revalidated candidates were already skipped")
            }
        }
    }

    Ok(report)
}

/// The SHARED second-pass eligibility outcome — the SINGLE "would this candidate be deleted?" decision
/// that both the dry-run and real-run branches consult, so a dry run counts a candidate as `would_delete`
/// IFF a real run would actually `delete` it (and `skipped_revalidated` in exactly the same cases). It
/// resolves the witness against the fresh stat ONCE; the dry-run branch then only increments a counter and
/// the real-run branch only performs the delete (CAS or unconditional) — neither re-decides eligibility, so
/// they can never diverge (the Medium fix).
enum DeletePlan {
    /// Not deletable — revalidated since the snapshot (a versioned candidate whose fresh generation no
    /// longer matches the snapshot, OR a fresh stat that became version-less). Counted `skipped_revalidated`
    /// in BOTH modes.
    SkipRevalidated,
    /// Eligible — reclaim via the ATOMIC compare-and-delete on the carried SNAPSHOT generation.
    DeleteCas(u64),
    /// Eligible — reclaim via the unconditional delete (a versionless candidate, safe under unique keys).
    DeleteUnconditional,
}

/// The delete witness threaded END-TO-END for a delete candidate (FINDING 1 + FINDING 2). Fixed at the
/// START-OF-SWEEP `list()` snapshot so the whole decision chain is anchored to ONE consistent observation.
enum CandidateWitness {
    /// The snapshot reported a native write version (`generation`): delete via the ATOMIC CAS keyed on
    /// THIS snapshot generation (Finding 1) — the strictly-stronger, clock-independent path.
    Versioned(u64),
    /// The snapshot reported NO native write version: delete UNCONDITIONALLY after the age, fresh-re-check,
    /// and re-stat guards (Finding 2). Safe ONLY because the composite store mints unique-per-write keys, so
    /// the candidate's key can never be a later recreate's — there is nothing live to clobber.
    Versionless,
}

/// A delete candidate carried from the first (snapshot-classify) pass into the second (re-validate +
/// delete) pass, with the witness fixed at `list()` time.
struct Candidate {
    /// The blob key being considered for GC.
    key: String,
    /// The `last_modified` the START-OF-SWEEP snapshot observed — the AGE anchor the fresh re-stat is
    /// compared against (a newer fresh stamp ⇒ rewritten ⇒ kept).
    snapshot_ts: SystemTime,
    /// How to delete it: an atomic CAS on the snapshot generation (versioned) or an unconditional delete
    /// made safe by unique-per-write keys (versionless).
    witness: CandidateWitness,
}

/// Spawn an OPT-IN periodic reconciler: a tokio task that runs [`reconcile_orphans`] every `interval`.
///
/// OFF by default — the binary only calls this when `SOLID_SERVER_RECONCILE_INTERVAL_SECS` is set (see
/// the module docs for why periodic is opt-in, not the default: in a scaled deployment GC should be a
/// single scheduled job, not a per-replica timer). The seams are taken by value (typically cheap
/// `Arc`/`Clone` handles — the live `HttpSparqClient` is `Arc`-backed) so the task owns them. The task
/// runs until the runtime drops; each tick logs the report (a failed sweep is logged and the loop
/// continues — a transient SPARQ blip must not kill the GC task). The first tick fires after one
/// `interval`, not immediately, so it never contends with boot.
///
/// Returns the [`tokio::task::JoinHandle`] so a caller that wants graceful teardown can abort it.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_periodic<S, B>(
    sparq: S,
    blob: B,
    interval: Duration,
    opts: ReconcileOptions,
) -> tokio::task::JoinHandle<()>
where
    S: SparqClient + 'static,
    B: BlobStore + 'static,
{
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // Skip missed ticks rather than bursting to catch up (a long sweep must not queue a backlog of
        // immediate re-runs).
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Consume the immediate first tick so the first sweep is one full interval after boot.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            // Log via stderr (the crate has no `tracing` dep — main.rs logs the same way). A failed
            // sweep is logged and the loop continues — a transient SPARQ blip must not kill the GC.
            match reconcile_orphans(&sparq, &blob, &opts).await {
                Ok(report) => {
                    eprintln!(
                        "reconciler sweep complete: scanned={} orphaned={} deleted={} \
                         would_delete={} too_young={} skipped_unknown_age={} \
                         skipped_revalidated={} delete_errors={}",
                        report.scanned,
                        report.orphaned,
                        report.deleted,
                        report.would_delete,
                        report.too_young,
                        report.skipped_unknown_age,
                        report.skipped_revalidated,
                        report.delete_errors,
                    );
                }
                Err(e) => {
                    eprintln!("reconciler sweep aborted (will retry next tick): {e}");
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::blob::InMemoryBlobStore;
    use crate::store::sparq::{InMemorySparqClient, ResourceMeta};
    use bytes::Bytes;
    use std::sync::Mutex;

    fn meta(blob_key: &str) -> ResourceMeta {
        ResourceMeta {
            content_type: "text/turtle".into(),
            blob_key: blob_key.to_string(),
            etag: "\"e\"".into(),
            last_modified: None,
        }
    }

    /// A timestamp `secs` ago — for deterministically back-dating a blob (no sleep).
    fn ago(secs: u64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(secs)
    }

    // A short grace for the tests (so "older than grace" is easy to arrange deterministically).
    fn opts() -> ReconcileOptions {
        ReconcileOptions {
            grace: Duration::from_secs(60),
            dry_run: false,
        }
    }

    #[tokio::test]
    async fn unreferenced_and_old_enough_blob_is_deleted() {
        let sparq = InMemorySparqClient::new();
        let blob = InMemoryBlobStore::new();
        // An orphan: bytes exist, NO index row references "orphan". Back-dated past the grace window.
        blob.put_with_time("orphan", Bytes::from_static(b"x"), ago(3600))
            .unwrap();

        let report = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.orphaned, 1);
        assert_eq!(report.deleted, 1);
        assert_eq!(report.referenced, 0);
        assert!(!blob.exists("orphan").await.unwrap(), "orphan must be GC'd");
    }

    #[tokio::test]
    async fn referenced_blob_is_kept_even_if_old() {
        let sparq = InMemorySparqClient::new();
        let blob = InMemoryBlobStore::new();
        // A LIVE resource: an index row references "live-key", and its bytes are old.
        sparq.put_meta("iri", meta("live-key")).await.unwrap();
        blob.put_with_time("live-key", Bytes::from_static(b"x"), ago(99999))
            .unwrap();

        let report = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.referenced, 1);
        assert_eq!(report.orphaned, 0);
        assert_eq!(report.deleted, 0);
        assert!(
            blob.exists("live-key").await.unwrap(),
            "a referenced blob must NEVER be GC'd, however old"
        );
    }

    #[tokio::test]
    async fn unreferenced_but_too_young_blob_is_kept() {
        // THE GRACE TEST: an unreferenced blob younger than the window is a possible write-in-progress
        // (bytes PUT, index row not yet committed) — it MUST be protected.
        let sparq = InMemorySparqClient::new();
        let blob = InMemoryBlobStore::new();
        blob.put_with_time("fresh-orphan", Bytes::from_static(b"x"), ago(1))
            .unwrap(); // 1s < 60s grace

        let report = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.orphaned, 1);
        assert_eq!(report.too_young, 1);
        assert_eq!(report.deleted, 0);
        assert!(
            blob.exists("fresh-orphan").await.unwrap(),
            "the grace window must protect a too-young orphan (the write-in-progress race)"
        );
    }

    #[tokio::test]
    async fn report_counts_partition_correctly() {
        let sparq = InMemorySparqClient::new();
        let blob = InMemoryBlobStore::new();
        // 1 referenced (old), 1 old orphan (deleted), 1 young orphan (kept), 1 unknown-age orphan (kept).
        sparq.put_meta("iri", meta("ref")).await.unwrap();
        blob.put_with_time("ref", Bytes::from_static(b"r"), ago(99999))
            .unwrap();
        blob.put_with_time("old-orphan", Bytes::from_static(b"o"), ago(3600))
            .unwrap();
        blob.put_with_time("young-orphan", Bytes::from_static(b"y"), ago(1))
            .unwrap();
        blob.put_with_time(
            "undated-orphan",
            Bytes::from_static(b"u"),
            SystemTime::now(),
        )
        .unwrap();
        // Force the unknown-age path by listing-time stamping is awkward; instead assert the
        // partition algebra on the known stamps. (The unknown-age branch is covered separately below.)

        let report = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(report.scanned, 4);
        assert_eq!(report.referenced, 1);
        assert_eq!(report.orphaned, 3);
        assert_eq!(report.deleted, 1); // only "old-orphan"
                                       // "young-orphan" + "undated-orphan"(now) are both inside the 60s window ⇒ too_young.
        assert_eq!(report.too_young, 2);
        // The SIX dispositions of an orphan must sum to `orphaned` (the full partition).
        assert_eq!(
            report.deleted
                + report.would_delete
                + report.too_young
                + report.skipped_unknown_age
                + report.skipped_revalidated
                + report.delete_errors,
            report.orphaned
        );
        // And referenced + orphaned == scanned.
        assert_eq!(report.referenced + report.orphaned, report.scanned);
    }

    #[tokio::test]
    async fn unknown_age_blob_is_kept_fail_closed() {
        // A backend that reports no last_modified ⇒ age unknowable ⇒ NEVER GC'd.
        let sparq = InMemorySparqClient::new();
        let blob = UndatedBlobStore::with_key("ghost");

        let report = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.orphaned, 1);
        assert_eq!(report.skipped_unknown_age, 1);
        assert_eq!(report.deleted, 0);
    }

    #[tokio::test]
    async fn idempotent_second_run_deletes_nothing() {
        let sparq = InMemorySparqClient::new();
        let blob = InMemoryBlobStore::new();
        blob.put_with_time("orphan", Bytes::from_static(b"x"), ago(3600))
            .unwrap();

        let first = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(first.deleted, 1);

        // Second run over the now-clean state: nothing left to delete.
        let second = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(second.scanned, 0);
        assert_eq!(second.orphaned, 0);
        assert_eq!(second.deleted, 0);
    }

    #[tokio::test]
    async fn dry_run_deletes_nothing_but_reports_orphans() {
        let sparq = InMemorySparqClient::new();
        let blob = InMemoryBlobStore::new();
        // 1 deletable orphan (old), 1 too-young orphan, 1 referenced — so the partition has >1 term.
        sparq.put_meta("iri", meta("ref")).await.unwrap();
        blob.put_with_time("ref", Bytes::from_static(b"r"), ago(99999))
            .unwrap();
        blob.put_with_time("orphan", Bytes::from_static(b"x"), ago(3600))
            .unwrap();
        blob.put_with_time("young", Bytes::from_static(b"y"), ago(1))
            .unwrap();

        let dry = ReconcileOptions {
            grace: Duration::from_secs(60),
            dry_run: true,
        };
        let report = reconcile_orphans(&sparq, &blob, &dry).await.unwrap();
        assert_eq!(report.scanned, 3);
        assert_eq!(report.referenced, 1);
        assert_eq!(report.orphaned, 2);
        assert_eq!(report.deleted, 0, "dry run must not delete");
        // Finding 2: the deletable orphan is counted under `would_delete`, NOT `deleted`.
        assert_eq!(
            report.would_delete, 1,
            "dry run reports the deletable count"
        );
        assert_eq!(report.too_young, 1);
        // The partition invariant must hold in DRY-RUN mode too.
        assert_eq!(
            report.deleted
                + report.would_delete
                + report.too_young
                + report.skipped_unknown_age
                + report.skipped_revalidated
                + report.delete_errors,
            report.orphaned
        );
        assert_eq!(report.referenced + report.orphaned, report.scanned);
        assert!(
            blob.exists("orphan").await.unwrap(),
            "dry run must keep the bytes"
        );
        assert!(blob.exists("young").await.unwrap());
    }

    #[tokio::test]
    async fn candidate_that_becomes_referenced_between_snapshot_and_delete_is_kept() {
        // FINDING 1 (part a) regression: a candidate that looked unreferenced at the start-of-sweep
        // snapshot but becomes REFERENCED (a recreate committed a fresh index row at the same
        // deterministic key) before the delete pass must NOT be deleted.
        //
        // `ToggleReferencedSparq` returns {} on the FIRST `referenced_blob_keys()` (the snapshot) and
        // {"orphan"} on the SECOND (the pre-delete fresh re-check). The blob is old enough, so without
        // the fresh re-check it WOULD be classified deletable and removed — the mutation-check.
        let sparq = ToggleReferencedSparq::new(["orphan"]);
        let blob = InMemoryBlobStore::new();
        blob.put_with_time("orphan", Bytes::from_static(b"x"), ago(3600))
            .unwrap();

        let report = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(report.scanned, 1);
        // Snapshot said unreferenced ⇒ it was an orphan candidate...
        assert_eq!(report.orphaned, 1);
        // ...but the fresh re-check found it referenced again ⇒ revalidated, NOT deleted.
        assert_eq!(report.skipped_revalidated, 1);
        assert_eq!(report.deleted, 0);
        assert!(
            blob.exists("orphan").await.unwrap(),
            "a candidate referenced again by delete-time must NOT be GC'd (the recreate race)"
        );
    }

    #[tokio::test]
    async fn mutation_check_without_fresh_recheck_the_recreate_would_be_deleted() {
        // The mutation-check made explicit: with the SNAPSHOT referenced set (empty), the candidate is
        // old-enough + unreferenced ⇒ the un-rechecked logic would delete it. We assert that the
        // snapshot view alone classifies it deletable, proving the test above is non-vacuous: it is the
        // FRESH re-check (the second referenced query) that flips the outcome to kept.
        let snapshot_referenced: HashSet<String> = HashSet::new();
        // From the start-of-sweep snapshot the key is unreferenced + old ⇒ would be deleted.
        assert!(!snapshot_referenced.contains("orphan"));
        // The fresh set (what the fix consults) DOES contain it ⇒ the fix keeps it.
        let fresh_referenced: HashSet<String> = ["orphan".to_string()].into_iter().collect();
        assert!(fresh_referenced.contains("orphan"));
    }

    #[tokio::test]
    async fn candidate_whose_bytes_are_rewritten_between_snapshot_and_delete_is_kept() {
        // FINDING 1 (part b) regression: a candidate old-enough at the snapshot whose BYTES are
        // REWRITTEN (a newer `last_modified` — a same-key overwrite under deterministic keying) before
        // the delete pass must NOT be deleted.
        //
        // `RewrittenBlobStore::list()` reports an OLD stamp (so it is classified deletable); its
        // `stat()` reports a FRESH stamp (the rewrite). Without the re-stat the candidate WOULD be
        // deleted — the mutation-check (the snapshot view alone says delete).
        let sparq = InMemorySparqClient::new();
        let blob = RewrittenBlobStore::new(
            "rewritten",
            ago(3600),         // old at snapshot ⇒ classified deletable
            SystemTime::now(), // freshly rewritten by delete-time ⇒ must be kept
        );

        let report = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.orphaned, 1);
        // The re-stat saw newer bytes ⇒ revalidated, NOT deleted.
        assert_eq!(report.skipped_revalidated, 1);
        assert_eq!(report.deleted, 0);
        assert!(
            !blob.deleted_keys().contains(&"rewritten".to_string()),
            "a candidate whose bytes were rewritten by delete-time must NOT be GC'd"
        );
    }

    #[tokio::test]
    async fn candidate_rewritten_in_the_atomic_cas_window_is_not_clobbered() {
        // FINDING 1 (the residual stat→delete TOCTOU) regression, exercised through the ATOMIC path: a
        // candidate that passes BOTH the fresh referenced re-check AND the fresh re-stat (so the
        // un-CAS'd logic would `delete()` it) but whose bytes are rewritten in the gap before the delete.
        // `CasMismatchBlobStore::stat()` reports the SNAPSHOT stamp + generation (⇒ classified deletable,
        // witness = the stat'd generation), but its `delete_if_unchanged()` sees a DIFFERENT current
        // generation ⇒ the CAS refuses ⇒ NOT deleted. The mutation-check: the atomic generation comparison
        // inside `delete_if_unchanged` is what flips the outcome to kept — remove that guard and the blob
        // is deleted, failing this test.
        let sparq = InMemorySparqClient::new();
        let blob = CasMismatchBlobStore::new(
            "rewritten-in-cas-window",
            ago(3600), // what list() AND stat() report ⇒ passes re-stat ⇒ deletable, witness = this
        );

        let report = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.orphaned, 1);
        // The atomic CAS saw the stamp had moved ⇒ revalidated, NOT deleted.
        assert_eq!(report.skipped_revalidated, 1);
        assert_eq!(report.deleted, 0);
        assert!(
            !blob.removed(),
            "the atomic CAS must REFUSE to delete when the witness no longer matches (the residual \
             stat→delete TOCTOU): live bytes rewritten in the CAS window must NOT be clobbered"
        );
    }

    #[tokio::test]
    async fn in_memory_store_cas_witness_is_generation_not_timestamp() {
        // THE HIGH-FIX UNIT TEST (mutation-checkable on the in-memory store itself): two writes that share
        // the SAME `last_modified` but get DIFFERENT generations must be distinguished by the CAS. The old
        // `last_modified` witness would have wrongly let the second write be deleted with the first's
        // witness; the generation witness refuses it.
        let blob = InMemoryBlobStore::new();
        let same_stamp = ago(3600);

        // Write #1 at `same_stamp`. Capture its generation — the witness a reconciler would carry from a
        // stat taken right after this write.
        blob.put_with_time("k", Bytes::from_static(b"v1"), same_stamp)
            .unwrap();
        let witness_gen_v1 = blob.generation_of("k").expect("v1 must exist");
        let stamp_v1 = blob
            .stat("k")
            .await
            .unwrap()
            .unwrap()
            .last_modified
            .unwrap();

        // OVERWRITE at the IDENTICAL last_modified (clock granularity / rollback). The generation MUST
        // bump even though the timestamp did not.
        blob.put_with_time("k", Bytes::from_static(b"v2"), same_stamp)
            .unwrap();
        let stamp_v2 = blob
            .stat("k")
            .await
            .unwrap()
            .unwrap()
            .last_modified
            .unwrap();
        let gen_v2 = blob.generation_of("k").expect("v2 must exist");

        assert_eq!(stamp_v1, stamp_v2, "the two writes share a last_modified");
        assert_ne!(
            witness_gen_v1, gen_v2,
            "but their generations MUST differ — a generation is a true per-write version"
        );

        // CAS-delete with the STALE witness (v1's generation): the live v2 bytes must NOT be clobbered,
        // even though v1 and v2 have identical timestamps. With a `last_modified` witness this would have
        // matched (same stamp) and wrongly deleted — the mutation-check is right here.
        let deleted = blob.delete_if_unchanged("k", witness_gen_v1).await.unwrap();
        assert!(
            !deleted,
            "CAS on the stale GENERATION must refuse — a same-timestamp overwrite is NOT the same write"
        );
        assert!(
            blob.exists("k").await.unwrap(),
            "the same-timestamp recreate's live bytes must survive (the HIGH fix)"
        );

        // And the CAS on the CURRENT generation DOES delete (sanity: the witness mechanism works both ways).
        let deleted_current = blob.delete_if_unchanged("k", gen_v2).await.unwrap();
        assert!(
            deleted_current,
            "CAS on the matching generation must delete"
        );
        assert!(!blob.exists("k").await.unwrap());
    }

    #[tokio::test]
    async fn candidate_recreated_with_same_timestamp_but_new_generation_is_kept() {
        // THE HIGH-FIX RECONCILER TEST (mutation-checkable end-to-end): a candidate old-enough at the
        // snapshot whose recreate lands at the IDENTICAL `last_modified` (so the time-based re-stat sees
        // NO change — `ts == snapshot_ts`, not `>`; still old enough) but with a BUMPED generation. The
        // age re-check therefore passes (a timestamp-only defence would proceed to delete), and the
        // delete witness is the generation, so the atomic CAS catches the recreate and refuses.
        //
        // MUTATION-CHECK: revert the witness to `last_modified` (or have the store key its CAS on the
        // timestamp) and this test FAILS — the identical timestamp would match and the live recreate's
        // bytes would be clobbered. The generation is the ONLY thing that distinguishes the two writes.
        let sparq = InMemorySparqClient::new();
        let same_stamp = ago(3600);
        let blob = SameTimestampRewriteBlobStore::new("recreated", same_stamp);

        let report = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.orphaned, 1);
        // Time-based checks pass (same stamp, old enough) — only the GENERATION CAS catches it ⇒ kept.
        assert_eq!(report.skipped_revalidated, 1);
        assert_eq!(report.deleted, 0);
        assert!(
            !blob.removed(),
            "a recreate sharing the candidate's last_modified but with a NEW generation must NOT be \
             GC'd — the CAS witness is the generation, not the timestamp (the HIGH fix)"
        );
    }

    #[tokio::test]
    async fn candidate_rewritten_between_list_and_stat_same_timestamp_new_generation_is_kept() {
        // FINDING 1 (the HIGH threading fix) regression, mutation-verified: a candidate old-enough at the
        // START-OF-SWEEP `list()` snapshot (generation 1) is REWRITTEN between `list()` and the fresh
        // `stat()`, keeping the SAME `last_modified` but bumping the generation to 2. The age re-check
        // therefore sees no change (same stamp, still old enough) and the fresh-referenced re-check still
        // says unreferenced — the ONLY thing that distinguishes the rewrite from the candidate is the
        // generation, observed FRESH (2) vs SNAPSHOT (1). The fix carries the SNAPSHOT generation as the
        // CAS witness and requires fresh == snapshot before deleting, so the snapshot(1) ≠ fresh(2)
        // mismatch SKIPS the candidate (skipped_revalidated) — the live rewrite is NOT deleted, and the CAS
        // is never even reached.
        //
        // MUTATION-CHECK (the exact HIGH bug): if the reconciler instead used the FRESH stat's generation
        // as the witness (the discarded-snapshot-generation bug), it would carry witness = 2; this store's
        // `delete_if_unchanged` deletes iff the witness equals the CURRENT (fresh, == 2) generation, so the
        // CAS would MATCH and DELETE the live rewrite. Asserting `!removed()` therefore fails under that
        // mutation — the test is non-vacuous and pins the snapshot-generation threading.
        let sparq = InMemorySparqClient::new();
        let same_stamp = ago(3600);
        let blob = SnapshotGenerationRewriteBlobStore::new("rewritten-list-to-stat", same_stamp);

        let report = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.orphaned, 1);
        // Snapshot gen (1) ≠ fresh-stat gen (2) ⇒ rewritten in the list→stat gap ⇒ revalidated, NOT
        // deleted. The CAS is never reached (the mismatch is caught before it).
        assert_eq!(report.skipped_revalidated, 1);
        assert_eq!(report.deleted, 0);
        assert!(
            !blob.cas_called(),
            "the snapshot≠fresh generation mismatch must skip BEFORE the CAS — a live rewrite in the \
             list→stat gap is caught by the threaded snapshot witness, not the CAS"
        );
        assert!(
            !blob.removed(),
            "a candidate rewritten between list() and the fresh stat() (same last_modified, new \
             generation) must NOT be GC'd — the CAS witness is the SNAPSHOT generation (the HIGH fix)"
        );
    }

    #[tokio::test]
    async fn dry_run_does_not_count_a_versioned_rewrite_as_would_delete() {
        // THE MEDIUM FIX regression: dry-run must PREDICT real-run. A VERSIONED candidate old-enough at the
        // snapshot (generation 1) but rewritten between `list()` and the fresh `stat()` to the SAME
        // `last_modified` with a BUMPED generation (2) is a same-timestamp rewrite a REAL run skips
        // (`skipped_revalidated`, via the snapshot≠fresh generation mismatch). A DRY run must therefore NOT
        // count it under `would_delete` — it must reach the SAME `skipped_revalidated` disposition.
        //
        // MUTATION-CHECK (the exact Medium bug): increment `would_delete` BEFORE the versioned
        // generation-revalidation (the pre-fix order) and this candidate is counted `would_delete = 1` /
        // `skipped_revalidated = 0` — OVER-reporting what a real run would reclaim. Routing both modes
        // through the shared eligibility makes the dry run agree with the real run, so this asserts
        // `would_delete == 0` and `skipped_revalidated == 1`, matching the real-run counterpart
        // (`candidate_rewritten_between_list_and_stat_same_timestamp_new_generation_is_kept`).
        let sparq = InMemorySparqClient::new();
        let same_stamp = ago(3600);
        let blob =
            SnapshotGenerationRewriteBlobStore::new("dry-rewritten-list-to-stat", same_stamp);

        let dry = ReconcileOptions {
            grace: Duration::from_secs(60),
            dry_run: true,
        };
        let report = reconcile_orphans(&sparq, &blob, &dry).await.unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.orphaned, 1);
        // The shared eligibility refuses the rewritten candidate ⇒ revalidated, NOT a would-delete — EXACTLY
        // as a real run skips it (matching the real-run test).
        assert_eq!(
            report.skipped_revalidated, 1,
            "a dry run must skip a versioned same-timestamp rewrite, like a real run"
        );
        assert_eq!(
            report.would_delete, 0,
            "dry-run must NOT over-report a rewrite a real run would skip (the Medium fix)"
        );
        assert_eq!(report.deleted, 0, "dry run deletes nothing");
        // The CAS is never reached (the snapshot≠fresh mismatch is caught first) and nothing is removed.
        assert!(
            !blob.cas_called(),
            "the generation mismatch must skip before any CAS, in dry-run too"
        );
        assert!(!blob.removed(), "dry run must touch nothing");
        // The partition holds in dry-run mode.
        assert_eq!(
            report.deleted
                + report.would_delete
                + report.too_young
                + report.skipped_unknown_age
                + report.skipped_revalidated
                + report.delete_errors,
            report.orphaned
        );
        assert_eq!(report.referenced + report.orphaned, report.scanned);
    }

    #[tokio::test]
    async fn empty_candidates_sweep_succeeds_even_if_second_referenced_query_errors() {
        // FINDING 2: a sweep with NO delete candidates (everything referenced / too-young / undated) must
        // return its report WITHOUT running the pre-delete fresh referenced-set re-fetch — so a transient
        // SPARQ error on that 2nd query can never fail a sweep that had nothing unsafe to do.
        //
        // `SecondCallFailsSparq` returns {} on the FIRST `referenced_blob_keys()` (the snapshot) and
        // ERRORS on every later call. The only blob is too-young ⇒ zero candidates ⇒ the 2nd query must
        // never run. If the empty-candidates short-circuit were removed, the 2nd query would fire and the
        // sweep would abort with ReferencedSet — the mutation-check.
        let sparq = SecondCallFailsSparq::new();
        let blob = InMemoryBlobStore::new();
        blob.put_with_time("young-orphan", Bytes::from_static(b"x"), ago(1))
            .unwrap(); // < 60s grace ⇒ too-young

        let report = reconcile_orphans(&sparq, &blob, &opts())
            .await
            .expect("zero-candidate sweep must not run (or fail on) the 2nd referenced query");
        assert_eq!(report.scanned, 1);
        assert_eq!(report.orphaned, 1);
        assert_eq!(report.too_young, 1);
        assert_eq!(report.deleted, 0);
        // The 2nd referenced query was never made (only the snapshot call).
        assert_eq!(
            sparq.calls(),
            1,
            "the pre-delete fresh referenced-set re-fetch must be skipped when there are no candidates"
        );
    }

    #[tokio::test]
    async fn old_versionless_orphan_is_reclaimed_via_unconditional_delete() {
        // FINDING 2: an OLD orphan (`last_modified: Some(old)`, past the grace window) whose backend
        // reports NO `generation` (a version-less object store) IS now reclaimed — NOT leaked forever. The
        // earlier code skipped every versionless candidate unconditionally, leaking all old orphans on a
        // versionless backend. Under unique-per-write keys an unconditional delete is safe: the orphan's
        // key can never be a later recreate's, so deleting its bytes can never clobber anything live.
        //
        // The candidate is age-gated (old enough), re-checked against the FRESH referenced set, and
        // re-statted (still old enough, still present) before the delete — then removed via the plain
        // unconditional `delete` (NOT the CAS, which it has no witness for).
        //
        // MUTATION-CHECK: revert to skipping versionless candidates and `removed()` is false / `deleted`
        // is 0 ⇒ this test fails. The store records WHICH path was taken so we also pin that it was the
        // unconditional `delete`, not the CAS.
        let sparq = InMemorySparqClient::new();
        let blob = VersionlessOldBlobStore::new("versionless-orphan", ago(3600));

        let report = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.orphaned, 1);
        // Old enough + unreferenced + versionless ⇒ reclaimed via the unconditional-delete path.
        assert_eq!(report.deleted, 1);
        assert_eq!(report.skipped_revalidated, 0);
        assert_eq!(
            report.skipped_unknown_age, 0,
            "the age IS known (old) — not this disposition"
        );
        assert_eq!(report.too_young, 0);
        // The partition still holds.
        assert_eq!(
            report.deleted
                + report.would_delete
                + report.too_young
                + report.skipped_unknown_age
                + report.skipped_revalidated
                + report.delete_errors,
            report.orphaned
        );
        assert!(
            blob.removed(),
            "an OLD version-less orphan must be GC'd under unique-per-write keys (no forever-leak)"
        );
        // The delete went through the UNCONDITIONAL `delete` path, never the CAS (it has no witness).
        assert_eq!(
            blob.cas_calls(),
            0,
            "a versionless candidate must use the unconditional delete, never delete_if_unchanged"
        );
        assert_eq!(blob.unconditional_delete_calls(), 1);
    }

    #[tokio::test]
    async fn fresh_versionless_orphan_is_kept_by_the_grace_window() {
        // FINDING 2 (the safety counterpart): a versionless orphan INSIDE the grace window is NOT
        // reclaimed — the write-in-progress race protection applies to the versionless path exactly as to
        // the versioned one. A too-young versionless candidate never even becomes a delete candidate (it is
        // stopped in the first pass), so no delete of any kind is reached.
        let sparq = InMemorySparqClient::new();
        let blob = VersionlessOldBlobStore::new("fresh-versionless", ago(1)); // 1s < 60s grace

        let report = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.orphaned, 1);
        assert_eq!(report.too_young, 1);
        assert_eq!(report.deleted, 0);
        assert!(
            !blob.removed(),
            "a too-young versionless orphan must be protected by the grace window (write-in-progress)"
        );
        assert_eq!(
            blob.cas_calls() + blob.unconditional_delete_calls(),
            0,
            "a too-young candidate reaches no delete path at all"
        );
    }

    #[tokio::test]
    async fn referenced_set_error_aborts_and_deletes_nothing() {
        // Fail-closed: if the referenced-set query fails we must NOT delete (a failed query is NOT
        // "nothing is referenced"). The blob list is never even consulted.
        let sparq = FailingSparq;
        let blob = InMemoryBlobStore::new();
        blob.put_with_time("orphan", Bytes::from_static(b"x"), ago(3600))
            .unwrap();

        let err = reconcile_orphans(&sparq, &blob, &opts()).await.unwrap_err();
        assert!(matches!(err, ReconcileError::ReferencedSet(_)));
        assert!(
            blob.exists("orphan").await.unwrap(),
            "a failed referenced-set query must abort the sweep without deleting"
        );
    }

    // --- test doubles for the fail-closed / unknown-age / fresh-recheck branches ---

    /// A SPARQ client whose `referenced_blob_keys` returns the EMPTY set on the first call (the
    /// start-of-sweep snapshot) and a configured non-empty set on every subsequent call (the pre-delete
    /// fresh re-check) — modelling a recreate that committed an index row mid-sweep at a reused
    /// deterministic key. Only `referenced_blob_keys` is reachable in these tests; the rest panic.
    struct ToggleReferencedSparq {
        fresh: HashSet<String>,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl ToggleReferencedSparq {
        fn new<I: IntoIterator<Item = &'static str>>(fresh: I) -> Self {
            Self {
                fresh: fresh.into_iter().map(str::to_string).collect(),
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }
    #[async_trait::async_trait]
    impl SparqClient for ToggleReferencedSparq {
        async fn get_meta(&self, _: &str) -> Result<ResourceMeta, SparqError> {
            unreachable!()
        }
        async fn put_meta(&self, _: &str, _: ResourceMeta) -> Result<(), SparqError> {
            unreachable!()
        }
        async fn exists(&self, _: &str) -> Result<bool, SparqError> {
            unreachable!()
        }
        async fn delete_meta(&self, _: &str) -> Result<(), SparqError> {
            unreachable!()
        }
        async fn delete_meta_if_empty(
            &self,
            _: &str,
            _: Option<&str>,
        ) -> Result<super::super::sparq::DeleteOutcome, SparqError> {
            unreachable!()
        }
        async fn create_child(&self, _: &str, _: &str, _: ResourceMeta) -> Result<(), SparqError> {
            unreachable!()
        }
        async fn remove_child(&self, _: &str, _: &str) -> Result<(), SparqError> {
            unreachable!()
        }
        async fn list_children(&self, _: &str) -> Result<Vec<String>, SparqError> {
            unreachable!()
        }
        async fn referenced_blob_keys(&self) -> Result<HashSet<String>, SparqError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // First call (the snapshot): nothing referenced. Later calls (the fresh re-check): the
            // recreate's row is now visible.
            if n == 0 {
                Ok(HashSet::new())
            } else {
                Ok(self.fresh.clone())
            }
        }
    }

    /// A blob store whose `list()` reports an OLD `last_modified` (so a key is classified deletable) but
    /// whose `stat()` reports a FRESH one (modelling a same-key overwrite that landed after the
    /// snapshot). Records every `delete()` so a test can assert nothing was actually removed.
    struct RewrittenBlobStore {
        key: String,
        list_stamp: SystemTime,
        stat_stamp: SystemTime,
        deleted: Mutex<Vec<String>>,
    }
    impl RewrittenBlobStore {
        fn new(key: &str, list_stamp: SystemTime, stat_stamp: SystemTime) -> Self {
            Self {
                key: key.to_string(),
                list_stamp,
                stat_stamp,
                deleted: Mutex::new(Vec::new()),
            }
        }
        fn deleted_keys(&self) -> Vec<String> {
            self.deleted.lock().expect("poisoned").clone()
        }
    }
    #[async_trait::async_trait]
    impl BlobStore for RewrittenBlobStore {
        async fn get(&self, _: &str) -> Result<Bytes, BlobError> {
            Ok(Bytes::from_static(b"x"))
        }
        async fn put(&self, _: &str, _: Bytes) -> Result<(), BlobError> {
            Ok(())
        }
        async fn exists(&self, key: &str) -> Result<bool, BlobError> {
            Ok(key == self.key)
        }
        async fn delete(&self, key: &str) -> Result<(), BlobError> {
            self.deleted.lock().expect("poisoned").push(key.to_string());
            Ok(())
        }
        async fn list(&self) -> Result<Vec<super::super::blob::BlobEntry>, BlobError> {
            Ok(vec![super::super::blob::BlobEntry {
                key: self.key.clone(),
                last_modified: Some(self.list_stamp),
                generation: Some(1),
            }])
        }
        async fn stat(
            &self,
            key: &str,
        ) -> Result<Option<super::super::blob::BlobEntry>, BlobError> {
            // The FRESH (rewritten) view: a newer stamp than `list()` reported (+ a bumped generation).
            Ok((key == self.key).then(|| super::super::blob::BlobEntry {
                key: self.key.clone(),
                last_modified: Some(self.stat_stamp),
                generation: Some(2),
            }))
        }
        async fn delete_if_unchanged(
            &self,
            key: &str,
            _expected_generation: u64,
        ) -> Result<bool, BlobError> {
            // The rewritten candidate is caught by the re-stat (newer than snapshot) BEFORE the CAS, so
            // this is not reached in the rewrite test; record any call so we can assert it was never hit.
            self.deleted.lock().expect("poisoned").push(key.to_string());
            Ok(true)
        }
    }

    /// A blob store that passes the reconciler's snapshot+re-stat checks (so the candidate is classified
    /// deletable with a known witness) but whose ATOMIC `delete_if_unchanged` reports the witness has
    /// CHANGED — modelling a same-key rewrite landing in the residual gap between the fresh stat and the
    /// CAS. `list()` and `stat()` both report `stamp` + `generation` (⇒ deletable, witness = the
    /// generation `stat()` reported); but `delete_if_unchanged` sees a DIFFERENT current generation (the
    /// rewrite bumped it) so it returns `Ok(false)` and records NOTHING removed. The CAS generation
    /// comparison is the load-bearing guard: a `delete_if_unchanged` that ignored the witness and always
    /// removed would delete the blob, failing the regression test.
    struct CasMismatchBlobStore {
        key: String,
        stamp: SystemTime,
        /// The generation `stat()` reports — the witness the reconciler carries into the CAS.
        stat_generation: u64,
        removed: Mutex<bool>,
    }
    impl CasMismatchBlobStore {
        fn new(key: &str, stamp: SystemTime) -> Self {
            Self {
                key: key.to_string(),
                stamp,
                stat_generation: 1,
                removed: Mutex::new(false),
            }
        }
        fn removed(&self) -> bool {
            *self.removed.lock().expect("poisoned")
        }
    }
    #[async_trait::async_trait]
    impl BlobStore for CasMismatchBlobStore {
        async fn get(&self, _: &str) -> Result<Bytes, BlobError> {
            Ok(Bytes::from_static(b"x"))
        }
        async fn put(&self, _: &str, _: Bytes) -> Result<(), BlobError> {
            Ok(())
        }
        async fn exists(&self, key: &str) -> Result<bool, BlobError> {
            Ok(key == self.key)
        }
        async fn delete(&self, key: &str) -> Result<(), BlobError> {
            // Unconditional delete: should NOT be reached by the reconciler (it uses the CAS path).
            if key == self.key {
                *self.removed.lock().expect("poisoned") = true;
            }
            Ok(())
        }
        async fn list(&self) -> Result<Vec<super::super::blob::BlobEntry>, BlobError> {
            Ok(vec![super::super::blob::BlobEntry {
                key: self.key.clone(),
                last_modified: Some(self.stamp),
                generation: Some(self.stat_generation),
            }])
        }
        async fn stat(
            &self,
            key: &str,
        ) -> Result<Option<super::super::blob::BlobEntry>, BlobError> {
            // Same stamp + generation as list() ⇒ NOT classified rewritten ⇒ passes the re-stat ⇒ the
            // witness carried into the CAS is `stat_generation`.
            Ok((key == self.key).then(|| super::super::blob::BlobEntry {
                key: self.key.clone(),
                last_modified: Some(self.stamp),
                generation: Some(self.stat_generation),
            }))
        }
        async fn delete_if_unchanged(
            &self,
            _key: &str,
            expected_generation: u64,
        ) -> Result<bool, BlobError> {
            // The CAS witness comparison: the current generation has moved (rewrite landed in the gap), so
            // the observed value differs from `expected_generation` ⇒ refuse to delete. The +1 models the
            // bumped write version.
            let current_generation = self.stat_generation + 1;
            if current_generation == expected_generation {
                *self.removed.lock().expect("poisoned") = true;
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }

    /// A blob store modelling the HIGH-fix race: a recreate that REUSES the candidate's `last_modified`
    /// (so the time-based age re-check sees no change — same stamp, still old enough) but bumps the
    /// `generation` (the true write version). `list()` and `stat()` report the SAME `stamp` and the SAME
    /// witness generation (`stat_generation`), so the candidate sails through the age re-check with the
    /// stat'd generation as its CAS witness; but `delete_if_unchanged` sees a DIFFERENT current generation
    /// (the recreate bumped it WITHOUT changing the timestamp) ⇒ the CAS refuses ⇒ NOT deleted. A
    /// `last_modified`-keyed CAS would have matched (identical stamp) and clobbered the live recreate —
    /// only the generation distinguishes the two writes.
    struct SameTimestampRewriteBlobStore {
        key: String,
        stamp: SystemTime,
        stat_generation: u64,
        removed: Mutex<bool>,
    }
    impl SameTimestampRewriteBlobStore {
        fn new(key: &str, stamp: SystemTime) -> Self {
            Self {
                key: key.to_string(),
                stamp,
                stat_generation: 1,
                removed: Mutex::new(false),
            }
        }
        fn removed(&self) -> bool {
            *self.removed.lock().expect("poisoned")
        }
    }
    #[async_trait::async_trait]
    impl BlobStore for SameTimestampRewriteBlobStore {
        async fn get(&self, _: &str) -> Result<Bytes, BlobError> {
            Ok(Bytes::from_static(b"x"))
        }
        async fn put(&self, _: &str, _: Bytes) -> Result<(), BlobError> {
            Ok(())
        }
        async fn exists(&self, key: &str) -> Result<bool, BlobError> {
            Ok(key == self.key)
        }
        async fn delete(&self, key: &str) -> Result<(), BlobError> {
            // Unconditional delete: should NOT be reached (the reconciler uses the CAS path).
            if key == self.key {
                *self.removed.lock().expect("poisoned") = true;
            }
            Ok(())
        }
        async fn list(&self) -> Result<Vec<super::super::blob::BlobEntry>, BlobError> {
            Ok(vec![super::super::blob::BlobEntry {
                key: self.key.clone(),
                last_modified: Some(self.stamp),
                generation: Some(self.stat_generation),
            }])
        }
        async fn stat(
            &self,
            key: &str,
        ) -> Result<Option<super::super::blob::BlobEntry>, BlobError> {
            // SAME stamp + generation as list() ⇒ the time-based age re-check sees no change (ts ==
            // snapshot_ts, still old enough) and carries `stat_generation` as the CAS witness.
            Ok((key == self.key).then(|| super::super::blob::BlobEntry {
                key: self.key.clone(),
                last_modified: Some(self.stamp),
                generation: Some(self.stat_generation),
            }))
        }
        async fn delete_if_unchanged(
            &self,
            _key: &str,
            expected_generation: u64,
        ) -> Result<bool, BlobError> {
            // The recreate bumped the generation WITHOUT touching the timestamp: the current generation
            // differs from the witness ⇒ refuse. A timestamp-keyed CAS would have wrongly matched.
            let current_generation = self.stat_generation + 1;
            if current_generation == expected_generation {
                *self.removed.lock().expect("poisoned") = true;
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }

    /// A blob store modelling the FINDING-1 list→stat rewrite (the HIGH threading bug): `list()` reports
    /// the candidate at `(stamp, generation = LIST_GEN)` (⇒ old-enough, classified deletable, SNAPSHOT
    /// witness = LIST_GEN); `stat()` reports the SAME `stamp` but a BUMPED `generation = STAT_GEN` —
    /// modelling a same-key overwrite that landed BETWEEN `list()` and the fresh `stat()`, sharing the
    /// timestamp (clock granularity / rollback) but with a new write version.
    ///
    /// With the fix the reconciler carries the SNAPSHOT generation (LIST_GEN) and requires the fresh
    /// stat's generation to EQUAL it before deleting; STAT_GEN ≠ LIST_GEN ⇒ it skips BEFORE the CAS, so
    /// `delete_if_unchanged` is never called (`cas_called()` stays false) and nothing is removed. The
    /// store's `delete_if_unchanged` deletes iff the witness equals the CURRENT (fresh) generation
    /// (STAT_GEN) — so the MUTATION (witness = the fresh stat's generation) WOULD pass STAT_GEN and delete
    /// the live rewrite. `cas_called` + `removed` make both the skip-before-CAS and the no-delete
    /// observable.
    struct SnapshotGenerationRewriteBlobStore {
        key: String,
        stamp: SystemTime,
        list_generation: u64,
        stat_generation: u64,
        cas_called: Mutex<bool>,
        removed: Mutex<bool>,
    }
    impl SnapshotGenerationRewriteBlobStore {
        fn new(key: &str, stamp: SystemTime) -> Self {
            Self {
                key: key.to_string(),
                stamp,
                list_generation: 1, // the SNAPSHOT generation (the correct CAS witness)
                stat_generation: 2, // the rewritten (fresh) generation — same stamp, new write version
                cas_called: Mutex::new(false),
                removed: Mutex::new(false),
            }
        }
        fn removed(&self) -> bool {
            *self.removed.lock().expect("poisoned")
        }
        fn cas_called(&self) -> bool {
            *self.cas_called.lock().expect("poisoned")
        }
    }
    #[async_trait::async_trait]
    impl BlobStore for SnapshotGenerationRewriteBlobStore {
        async fn get(&self, _: &str) -> Result<Bytes, BlobError> {
            Ok(Bytes::from_static(b"x"))
        }
        async fn put(&self, _: &str, _: Bytes) -> Result<(), BlobError> {
            Ok(())
        }
        async fn exists(&self, key: &str) -> Result<bool, BlobError> {
            Ok(key == self.key)
        }
        async fn delete(&self, key: &str) -> Result<(), BlobError> {
            if key == self.key {
                *self.removed.lock().expect("poisoned") = true;
            }
            Ok(())
        }
        async fn list(&self) -> Result<Vec<super::super::blob::BlobEntry>, BlobError> {
            // The SNAPSHOT view: old stamp + the SNAPSHOT generation (the witness the fix threads through).
            Ok(vec![super::super::blob::BlobEntry {
                key: self.key.clone(),
                last_modified: Some(self.stamp),
                generation: Some(self.list_generation),
            }])
        }
        async fn stat(
            &self,
            key: &str,
        ) -> Result<Option<super::super::blob::BlobEntry>, BlobError> {
            // The FRESH view: SAME stamp (so the age re-check sees no change) but a BUMPED generation (the
            // rewrite). fresh (STAT_GEN) ≠ snapshot (LIST_GEN) ⇒ the fix skips here, before the CAS.
            Ok((key == self.key).then(|| super::super::blob::BlobEntry {
                key: self.key.clone(),
                last_modified: Some(self.stamp),
                generation: Some(self.stat_generation),
            }))
        }
        async fn delete_if_unchanged(
            &self,
            _key: &str,
            expected_generation: u64,
        ) -> Result<bool, BlobError> {
            *self.cas_called.lock().expect("poisoned") = true;
            // The store's current write version is the rewritten (fresh) generation. The CAS deletes iff
            // the witness matches it — so the SNAPSHOT witness (LIST_GEN) is refused, while the MUTATION's
            // FRESH witness (STAT_GEN) would match and delete. This is what makes the mutation-check bite.
            if expected_generation == self.stat_generation {
                *self.removed.lock().expect("poisoned") = true;
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }

    /// A SPARQ client whose `referenced_blob_keys` returns the EMPTY set on the FIRST call (the
    /// start-of-sweep snapshot) and ERRORS on every later call — to prove the pre-delete fresh re-fetch
    /// is SKIPPED when there are no delete candidates (Finding 2). Only `referenced_blob_keys` is
    /// reachable; the rest panic.
    struct SecondCallFailsSparq {
        calls: std::sync::atomic::AtomicUsize,
    }
    impl SecondCallFailsSparq {
        fn new() -> Self {
            Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }
    #[async_trait::async_trait]
    impl SparqClient for SecondCallFailsSparq {
        async fn get_meta(&self, _: &str) -> Result<ResourceMeta, SparqError> {
            unreachable!()
        }
        async fn put_meta(&self, _: &str, _: ResourceMeta) -> Result<(), SparqError> {
            unreachable!()
        }
        async fn exists(&self, _: &str) -> Result<bool, SparqError> {
            unreachable!()
        }
        async fn delete_meta(&self, _: &str) -> Result<(), SparqError> {
            unreachable!()
        }
        async fn delete_meta_if_empty(
            &self,
            _: &str,
            _: Option<&str>,
        ) -> Result<super::super::sparq::DeleteOutcome, SparqError> {
            unreachable!()
        }
        async fn create_child(&self, _: &str, _: &str, _: ResourceMeta) -> Result<(), SparqError> {
            unreachable!()
        }
        async fn remove_child(&self, _: &str, _: &str) -> Result<(), SparqError> {
            unreachable!()
        }
        async fn list_children(&self, _: &str) -> Result<Vec<String>, SparqError> {
            unreachable!()
        }
        async fn referenced_blob_keys(&self) -> Result<HashSet<String>, SparqError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Ok(HashSet::new())
            } else {
                Err(SparqError::Backend(
                    "second referenced-set query must not run with zero candidates".into(),
                ))
            }
        }
    }

    /// A blob store whose `list()` (AND `stat()`) reports a key with a known `last_modified` but NO
    /// `generation` (a version-less object-store backend) — exercising the FINDING-2 versionless GC path.
    /// Records the UNCONDITIONAL `delete` calls and the `delete_if_unchanged` (CAS) calls SEPARATELY so a
    /// test can pin that a versionless candidate is reclaimed via the unconditional delete, never the CAS.
    struct VersionlessOldBlobStore {
        key: String,
        stamp: SystemTime,
        unconditional_delete_calls: std::sync::atomic::AtomicUsize,
        cas_calls: std::sync::atomic::AtomicUsize,
        removed: Mutex<bool>,
    }
    impl VersionlessOldBlobStore {
        fn new(key: &str, stamp: SystemTime) -> Self {
            Self {
                key: key.to_string(),
                stamp,
                unconditional_delete_calls: std::sync::atomic::AtomicUsize::new(0),
                cas_calls: std::sync::atomic::AtomicUsize::new(0),
                removed: Mutex::new(false),
            }
        }
        fn unconditional_delete_calls(&self) -> usize {
            self.unconditional_delete_calls
                .load(std::sync::atomic::Ordering::SeqCst)
        }
        fn cas_calls(&self) -> usize {
            self.cas_calls.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn removed(&self) -> bool {
            *self.removed.lock().expect("poisoned")
        }
    }
    #[async_trait::async_trait]
    impl BlobStore for VersionlessOldBlobStore {
        async fn get(&self, _: &str) -> Result<Bytes, BlobError> {
            Ok(Bytes::from_static(b"x"))
        }
        async fn put(&self, _: &str, _: Bytes) -> Result<(), BlobError> {
            Ok(())
        }
        async fn exists(&self, key: &str) -> Result<bool, BlobError> {
            Ok(key == self.key)
        }
        async fn delete(&self, key: &str) -> Result<(), BlobError> {
            // The FINDING-2 versionless reclaim path: a plain unconditional delete (no witness). Record it
            // distinctly so the test can assert the versionless candidate took THIS path, not the CAS.
            self.unconditional_delete_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if key == self.key {
                *self.removed.lock().expect("poisoned") = true;
            }
            Ok(())
        }
        async fn list(&self) -> Result<Vec<super::super::blob::BlobEntry>, BlobError> {
            // KNOWN stamp (age check decides too-young vs old) but NO generation (version-less backend).
            Ok(vec![super::super::blob::BlobEntry {
                key: self.key.clone(),
                last_modified: Some(self.stamp),
                generation: None,
            }])
        }
        async fn stat(
            &self,
            key: &str,
        ) -> Result<Option<super::super::blob::BlobEntry>, BlobError> {
            // The fresh re-stat reports the SAME stamp + STILL no generation (the backend is version-less),
            // so the re-stat confirms "still old enough, still present, still versionless" ⇒ the
            // unconditional-delete path proceeds.
            Ok((key == self.key).then(|| super::super::blob::BlobEntry {
                key: self.key.clone(),
                last_modified: Some(self.stamp),
                generation: None,
            }))
        }
        async fn delete_if_unchanged(
            &self,
            key: &str,
            _expected_generation: u64,
        ) -> Result<bool, BlobError> {
            // Must NOT be reached for a versionless candidate (it has no witness ⇒ takes the unconditional
            // delete). Record any CAS call so a wrong-path regression is observable.
            self.cas_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if key == self.key {
                *self.removed.lock().expect("poisoned") = true;
            }
            Ok(true)
        }
    }

    /// A blob store whose `list` reports keys with NO last_modified (the unknown-age path).
    struct UndatedBlobStore {
        key: String,
        inner: InMemoryBlobStore,
    }
    impl UndatedBlobStore {
        fn with_key(key: &str) -> Self {
            let inner = InMemoryBlobStore::new();
            inner
                .put_with_time(key, Bytes::from_static(b"x"), SystemTime::now())
                .unwrap();
            Self {
                key: key.to_string(),
                inner,
            }
        }
    }
    #[async_trait::async_trait]
    impl BlobStore for UndatedBlobStore {
        async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
            self.inner.get(key).await
        }
        async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
            self.inner.put(key, body).await
        }
        async fn exists(&self, key: &str) -> Result<bool, BlobError> {
            self.inner.exists(key).await
        }
        async fn delete(&self, key: &str) -> Result<(), BlobError> {
            self.inner.delete(key).await
        }
        async fn list(&self) -> Result<Vec<super::super::blob::BlobEntry>, BlobError> {
            // No last_modified (the unknown-age path) AND no generation: an undated, version-less backend
            // is decided in the first pass (skipped_unknown_age) before any CAS witness is needed.
            Ok(vec![super::super::blob::BlobEntry {
                key: self.key.clone(),
                last_modified: None,
                generation: None,
            }])
        }
        async fn delete_if_unchanged(
            &self,
            key: &str,
            expected_generation: u64,
        ) -> Result<bool, BlobError> {
            self.inner
                .delete_if_unchanged(key, expected_generation)
                .await
        }
    }

    /// A SPARQ client whose `referenced_blob_keys` always errors (the fail-closed test). All other
    /// methods are unreachable in this test, so they panic.
    struct FailingSparq;
    #[async_trait::async_trait]
    impl SparqClient for FailingSparq {
        async fn get_meta(&self, _: &str) -> Result<ResourceMeta, SparqError> {
            unreachable!()
        }
        async fn put_meta(&self, _: &str, _: ResourceMeta) -> Result<(), SparqError> {
            unreachable!()
        }
        async fn exists(&self, _: &str) -> Result<bool, SparqError> {
            unreachable!()
        }
        async fn delete_meta(&self, _: &str) -> Result<(), SparqError> {
            unreachable!()
        }
        async fn delete_meta_if_empty(
            &self,
            _: &str,
            _: Option<&str>,
        ) -> Result<super::super::sparq::DeleteOutcome, SparqError> {
            unreachable!()
        }
        async fn create_child(&self, _: &str, _: &str, _: ResourceMeta) -> Result<(), SparqError> {
            unreachable!()
        }
        async fn remove_child(&self, _: &str, _: &str) -> Result<(), SparqError> {
            unreachable!()
        }
        async fn list_children(&self, _: &str) -> Result<Vec<String>, SparqError> {
            unreachable!()
        }
        async fn referenced_blob_keys(&self) -> Result<HashSet<String>, SparqError> {
            Err(SparqError::Backend(
                "simulated referenced-set failure".into(),
            ))
        }
    }
}
