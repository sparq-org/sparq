//! [OPUS-4.8] (sq-b4fns, gh-906) A DURABLE, REPLAYABLE change-data-capture (CDC)
//! stream in the Amazon-Neptune-Streams shape — the serialized, on-disk form of the
//! in-memory group-commit delta the [`Writer`](crate::Writer) already produces.
//!
//! ## What this is (and what it is NOT)
//!
//! Every committed generation emits ONE ordered, monotonically-sequenced **change
//! record**: a `(seq, generation, timestamp, inserts[], deletes[])` tuple persisted to
//! a segmented, fsync'd append-only log. A consumer polls the log by sequence number
//! and **replays from any offset after a restart** ([`open`](ChangeLog::open) re-reads the
//! segments from disk and resumes mid-stream) — unlike the in-process WebSocket SELECT-diff
//! subscriptions path, which is ephemeral (a disconnect misses every change) and per-query
//! rather than a raw durable cross-service change log.
//!
//! ```text
//!   commit G3 ──▶ record seq=3 (gen 3, +2 inserts, −1 delete)
//!   commit G4 ──▶ record seq=4 (gen 4, +0 inserts, −3 deletes)
//!                  └─ poll(from_seq=3) ⇒ [record 3, record 4, …]   (resumable after restart)
//! ```
//!
//! This is the **opt-in `change-stream` feature** (default OFF). The serving core (ring +
//! sequenced writer) builds, clips, and tests byte-identically without it; turning it on
//! pulls only `sparq-engine/serialize-rdf` (the N-Quads serialiser already used by
//! [`backup`](crate::backup)) — no new external dependency, no HTTP, no async runtime
//! (library-first §6.1).
//!
//! ## Where the change-set comes from (same-lineage quad diff)
//!
//! A change record is computed exactly like a [`backup_delta`](crate::backup_delta):
//! the **quad-set difference** of two consecutive same-lineage generations' N-Quads
//! serialisations — a quad in `to` but not `from` is an insert, a quad in `from` but not
//! `to` is a delete. This is correct precisely because the two generations share a writer
//! lineage (the writer forks a generation and applies updates, preserving every surviving
//! term's blank-node identity). The load-bearing boundary is identical to `backup_delta`'s:
//! diffing snapshots from two unrelated load paths is NOT supported and is not what the
//! change stream does — [`record_commit`](ChangeLog::record_commit) takes the predecessor
//! and successor generations of ONE ring's commit.
//!
//! ## Durability + ordering contract (Neptune-Streams shape)
//!
//! - **Ordered, gapless, monotonic.** Sequence numbers start at the first recorded
//!   generation and increase by exactly one per record; a record's `generation` equals its
//!   `seq` for a single-writer ring (recorded explicitly so the two never silently drift).
//!   [`record_commit`](ChangeLog::record_commit) rejects a non-forward or out-of-order
//!   commit (fail-closed).
//! - **Durable.** Each appended record is followed by an `fsync` of the active segment file
//!   before the call returns (configurable via [`ChangeLogConfig::fsync`]), so an
//!   acknowledged record survives a crash.
//! - **Segmented.** Records roll into a new segment file once the active segment reaches
//!   [`ChangeLogConfig::segment_target_bytes`], so retention/truncation can drop whole old
//!   segments.
//! - **Retention (truncation).** [FABLE-5] (sq-n9s4d) [`ChangeLog::apply_retention`] drops
//!   whole OLD segments — oldest-first, never the active segment, never a partial segment —
//!   under a [`RetentionPolicy`] combining consumer-ack (a HARD safety bound: a segment
//!   holding any unacked record is never dropped), age, and total-size pressure. Trimming
//!   preserves the gapless order of everything retained; a [`poll`](ChangeLog::poll) from a
//!   trimmed-away offset **fails closed** (it never silently skips dropped records) — resume
//!   from [`first_seq`](ChangeLog::first_seq) instead. Retention is EXPLICIT (the host
//!   decides when to call it, e.g. periodically or after commits); nothing is dropped by
//!   default.
//! - **Replayable after restart.** [`ChangeLog::open`] re-reads every segment in order,
//!   validates each record's framing + digest (fail-closed on a torn tail — a half-written
//!   trailing record is truncated, not silently replayed), and resumes appending at the next
//!   seq. [`ChangeLog::poll`] streams records from any `from_seq` onward.
//! - **Re-baseline (honest gap).** [FABLE-5] (sq-r2cu1) A BROKEN stream — a dropped record
//!   after a recording I/O failure (see [`into_commit_hook`](ChangeLog::into_commit_hook)'s
//!   error policy), or a `Writer::restore` (new lineage; the commit hook deliberately never
//!   fires) — fail-closes every later [`record_commit`](ChangeLog::record_commit) via the
//!   discontinuity check. [`ChangeLog::rebase_to`] is the explicit operator resync: it
//!   appends a **gap record** ([`ChangeRecord::rebase`] `= true`, no changes) honestly
//!   marking the uncaptured span and re-arms recording from the given generation — the log
//!   is never wiped, and a consumer replaying past the gap record KNOWS the stream is not a
//!   complete diff history there (re-bootstrap from a backup at or after the gap's
//!   generation, then resume). [FABLE-5] (gh-2436) On a RUNNING recorder, rebase through
//!   [`ChangeLog::into_commit_hook_with_control`]'s [`ChangeStreamControl`] (no stop /
//!   re-open needed); for a restore that REPLACED the lineage (generation numbering
//!   restarted), use [`ChangeLog::rebase_to_new_lineage`].
//!
//! ## Fail-closed, same digest family as backup
//!
//! Each record is length-framed and carries the SAME non-cryptographic FNV-1a body digest
//! the [`backup`](crate::backup) family uses: it detects truncation/corruption/version
//! mismatch, NOT tampering. At-rest **encryption + cryptographic authenticity are out of
//! scope** (a separate concern), exactly as for backup. A consumer that needs an authentic
//! cross-service feed must wrap its own signing around the records.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sparq_core::Graph;

use crate::backup::{body_digest, BackupError};
use crate::ring::Generation;

/// Magic line that opens every change-stream SEGMENT file — distinct from the backup /
/// delta magics so the three artifact kinds can never be confused (opening a segment as a
/// base/delta, or vice-versa, fails closed at the magic line).
pub const SEGMENT_MAGIC: &str = "SPARQ-CHANGESTREAM";

/// Change-stream segment format version. Bumped independently of the backup/delta
/// versions; [`ChangeLog::open`] rejects an unknown version (fail-closed) rather than
/// mis-reading a newer layout.
const SEGMENT_FORMAT_VERSION: u32 = 1;

/// Default roll-over size for a segment file (bytes). Once the active segment reaches this
/// many bytes, the next record opens a fresh segment, so retention can drop whole old
/// segments. Tune via [`ChangeLogConfig`].
pub const DEFAULT_SEGMENT_TARGET_BYTES: u64 = 64 * 1024 * 1024;

/// Filename prefix + extension for segment files: `changestream-<first-seq>.cdc`.
const SEGMENT_PREFIX: &str = "changestream-";
const SEGMENT_EXT: &str = "cdc";

/// One quad-level change operation within a commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeOp {
    /// The quad was added between the predecessor and successor generation.
    Insert,
    /// The quad was removed between the predecessor and successor generation.
    Delete,
}

impl ChangeOp {
    /// The single-character tag persisted in the segment body (`+` insert, `-` delete).
    fn tag(self) -> char {
        match self {
            ChangeOp::Insert => '+',
            ChangeOp::Delete => '-',
        }
    }
}

/// One quad-level change: a [`ChangeOp`] plus the affected quad as a single N-Quads line
/// (default graph or named graph — the line is rendered verbatim by the serialiser, so the
/// graph term, if any, is its fourth position).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Whether this quad was inserted or deleted in the commit.
    pub op: ChangeOp,
    /// The affected quad, as one N-Quads line (no trailing newline).
    pub quad: String,
}

/// One ordered, monotonically-sequenced change record — the unit of the CDC stream, one
/// per committed generation (Neptune-Streams shape). Records are produced by
/// [`ChangeLog::record_commit`] and read back by [`ChangeLog::poll`] / [`ChangeLog::open`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeRecord {
    /// The record's monotonic sequence number — the poll offset. Increases by exactly one
    /// per record across the whole log (gapless), so `poll(from_seq)` resumes deterministically.
    pub seq: u64,
    /// The generation/writer-seq this record captures the commit TO (its predecessor is the
    /// previous recorded generation). Recorded explicitly so `seq` and `generation` never
    /// silently drift (they are equal for a single-writer ring).
    pub generation: u64,
    /// Commit timestamp in nanoseconds since the Unix epoch (wall clock at
    /// [`record_commit`](ChangeLog::record_commit) time). Advisory ordering metadata only —
    /// `seq` is the authoritative order.
    pub timestamp_unix_nanos: u128,
    /// The quad-level changes of this commit (inserts first, then deletes — each sorted).
    pub changes: Vec<Change>,
    /// [FABLE-5] (sq-r2cu1) `true` iff this record is an operator **re-base marker** — an
    /// HONEST GAP in the stream ([`ChangeLog::rebase_to`]), not a commit diff. The changes
    /// between the PREVIOUS record's generation and this record's [`generation`](Self::generation)
    /// were NOT captured (a dropped record after a recording failure, or a `Writer::restore`
    /// that replaced the lineage); `changes` is empty. A consumer that needs a complete view
    /// must re-bootstrap (e.g. from a backup at or after this generation) before resuming
    /// past this record — replaying the gap as "no changes" would be silently wrong.
    pub rebase: bool,
}

impl ChangeRecord {
    /// The number of inserted quads in this record.
    pub fn insert_count(&self) -> usize {
        self.changes
            .iter()
            .filter(|c| c.op == ChangeOp::Insert)
            .count()
    }

    /// The number of deleted quads in this record.
    pub fn delete_count(&self) -> usize {
        self.changes
            .iter()
            .filter(|c| c.op == ChangeOp::Delete)
            .count()
    }
}

/// Configuration for a [`ChangeLog`].
#[derive(Clone, Debug)]
pub struct ChangeLogConfig {
    /// Roll into a new segment file once the active segment reaches this many bytes
    /// ([`DEFAULT_SEGMENT_TARGET_BYTES`]). A single record always lands in one segment even
    /// if it alone exceeds the target (records are never split across segments).
    pub segment_target_bytes: u64,
    /// `fsync` the active segment after every appended record before
    /// [`record_commit`](ChangeLog::record_commit) returns (durability; default `true`).
    /// Set `false` only for tests/throughput experiments where a crash may lose the tail.
    pub fsync: bool,
}

impl Default for ChangeLogConfig {
    fn default() -> Self {
        ChangeLogConfig {
            segment_target_bytes: DEFAULT_SEGMENT_TARGET_BYTES,
            fsync: true,
        }
    }
}

/// [FABLE-5] (sq-n9s4d) A retention/truncation policy for [`ChangeLog::apply_retention`]:
/// which whole OLD segments may be dropped. The three criteria compose as follows:
///
/// - [`acked_through_seq`](Self::acked_through_seq) is a **hard safety bound**: when set, a
///   segment is only ever droppable if EVERY record it holds has `seq <=` the watermark (a
///   consumer has durably acknowledged it). A segment holding any unacked record — and every
///   segment after it (prefix rule) — is retained regardless of age/size pressure.
/// - [`max_age`](Self::max_age) / [`max_total_bytes`](Self::max_total_bytes) are **pressure
///   criteria**: an eligible segment is dropped if its NEWEST record is older than `max_age`,
///   OR while the log's total on-disk size still exceeds `max_total_bytes` (oldest first).
/// - With NO pressure criterion set but an ack watermark set, ack alone drives dropping:
///   every fully-acked old segment is dropped (the "keep only what consumers still need"
///   shape).
///
/// The default policy (all `None`) is a **no-op** — nothing is ever dropped implicitly.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Size pressure: drop oldest eligible segments while the total on-disk size of ALL
    /// segment files (including the active one) exceeds this many bytes. The active segment
    /// is never dropped, so the floor is one segment even under a `Some(0)` budget.
    pub max_total_bytes: Option<u64>,
    /// Age pressure: a segment whose NEWEST record's commit timestamp is at least this much
    /// older than "now" is droppable (every record in it is stale).
    pub max_age: Option<Duration>,
    /// Consumer-ack watermark (hard safety bound): the highest `seq` every consumer has
    /// durably processed. A segment holding any record with `seq >` this is never dropped.
    pub acked_through_seq: Option<u64>,
}

impl RetentionPolicy {
    /// `true` iff the policy can never drop anything (all criteria `None`).
    /// [`ChangeLog::apply_retention`] short-circuits on a no-op policy.
    pub fn is_noop(&self) -> bool {
        self.max_total_bytes.is_none() && self.max_age.is_none() && self.acked_through_seq.is_none()
    }
}

/// [FABLE-5] (sq-n9s4d) What one [`ChangeLog::apply_retention`] pass actually dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetentionReport {
    /// Number of whole segment files deleted by this pass.
    pub segments_dropped: usize,
    /// Total on-disk bytes reclaimed by this pass.
    pub bytes_dropped: u64,
    /// The earliest seq still retained after this pass (== [`ChangeLog::first_seq`]).
    pub first_retained_seq: u64,
}

/// The durable, segmented, append-only change-data-capture log over one ring's commit
/// history. Append with [`record_commit`](Self::record_commit); read with
/// [`poll`](Self::poll); reopen after a restart with [`open`](Self::open) and resume.
///
/// One `ChangeLog` owns one log directory; it is single-writer (mirroring the single
/// sequenced [`Writer`](crate::Writer)). A reader can [`poll`](Self::poll) the same
/// directory concurrently through a SEPARATE `ChangeLog::open` handle — segments are
/// append-only and a poll never observes a half-written trailing record (it stops at the
/// last fully-framed, digest-valid record).
#[derive(Debug)]
pub struct ChangeLog {
    dir: PathBuf,
    config: ChangeLogConfig,
    /// The active (most-recent) segment file, open for append.
    active: File,
    /// The first seq stored in the active segment (its filename key).
    active_first_seq: u64,
    /// Current size of the active segment in bytes (avoids an fstat per append).
    active_bytes: u64,
    /// The seq the NEXT recorded commit will get. `0` for a fresh, empty log.
    next_seq: u64,
    /// The earliest seq still retained on disk — `0` until retention drops a segment, then
    /// the first seq of the oldest surviving segment. Recovered from the segment filenames
    /// on [`open`](Self::open).
    first_seq: u64,
    /// The generation of the last recorded commit (`None` for an empty log), used to reject
    /// a non-forward commit.
    last_generation: Option<u64>,
}

impl ChangeLog {
    /// Opens (creating if absent) the change log rooted at `dir`, recovering any existing
    /// stream: every segment is re-read in order, each record's framing + digest validated,
    /// and the next append resumes at the recovered seq. **Fail-closed on corruption** — a
    /// bad digest, an out-of-order seq, or an unknown segment version yields a [`BackupError`]
    /// and opens NOTHING (the operator gets a clean error, never a silently-wrong log).
    ///
    /// A clean torn TAIL (the process died mid-append) is the one corruption that is
    /// *recovered* rather than rejected: a trailing record whose framed length runs past the
    /// end of the file is treated as never-committed and the segment is truncated back to the
    /// last complete record, so the log resumes from the last durably-acknowledged commit.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, BackupError> {
        Self::open_with_config(dir, ChangeLogConfig::default())
    }

    /// [`open`](Self::open) with an explicit [`ChangeLogConfig`].
    pub fn open_with_config(
        dir: impl AsRef<Path>,
        config: ChangeLogConfig,
    ) -> Result<Self, BackupError> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;

        let mut segments = segment_files(&dir)?;
        segments.sort_by_key(|(seq, _)| *seq);

        // Recover the stream by reading every segment in order; the last record's seq + the
        // recovered next-seq / last-generation drive resumed appends. A retention-trimmed
        // log no longer starts at seq 0: the stream (and the seq validation) starts at the
        // oldest RETAINED segment's filename key.
        let first_seq = segments.first().map(|(seq, _)| *seq).unwrap_or(0);
        let mut next_seq = first_seq;
        let mut last_generation: Option<u64> = None;
        let mut active_first_seq = 0u64;
        let mut active_path: Option<PathBuf> = None;
        let mut active_valid_bytes = 0u64;

        for (first_seq, path) in &segments {
            let (records_end, recovered) = recover_segment(path, next_seq)?;
            for rec in &recovered {
                next_seq = rec.seq + 1;
                last_generation = Some(rec.generation);
            }
            active_first_seq = *first_seq;
            active_path = Some(path.clone());
            active_valid_bytes = records_end;
        }

        // No segment yet (fresh log): start a first segment keyed at seq 0.
        let (active, active_first_seq, active_bytes) = match active_path {
            Some(path) => {
                // Truncate any torn tail back to the last valid record, then open for append.
                let mut f = OpenOptions::new().read(true).write(true).open(&path)?;
                f.set_len(active_valid_bytes)?;
                f.seek_end()?;
                (f, active_first_seq, active_valid_bytes)
            }
            None => {
                let path = segment_path(&dir, 0);
                let (f, bytes) = create_segment(&path)?;
                (f, 0, bytes)
            }
        };

        Ok(ChangeLog {
            dir,
            config,
            active,
            active_first_seq,
            active_bytes,
            next_seq,
            first_seq,
            last_generation,
        })
    }

    /// The seq the NEXT [`record_commit`](Self::record_commit) will assign — i.e. the count
    /// of records already in the log, and the resume offset after a restart.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// The generation of the last recorded commit, or `None` for an empty log.
    pub fn last_generation(&self) -> Option<u64> {
        self.last_generation
    }

    /// [FABLE-5] (sq-n9s4d) The earliest seq still retained on disk — `0` until
    /// [`apply_retention`](Self::apply_retention) drops a segment. A consumer whose resume
    /// offset was trimmed away (its [`poll`](Self::poll) fails closed) restarts from here.
    pub fn first_seq(&self) -> u64 {
        self.first_seq
    }

    /// [FABLE-5] (sq-n9s4d) Applies `policy` (see [`RetentionPolicy`] for how the criteria
    /// compose) against the wall clock, dropping whole OLD segments oldest-first. Equivalent
    /// to [`apply_retention_at`](Self::apply_retention_at) with `now` = the current time.
    ///
    /// Only this handle's view is mutated ([`first_seq`](Self::first_seq)); a concurrent
    /// reader handle observes the trim through its next `poll` (which re-reads the
    /// directory). Never touches the active segment, so appends continue unaffected.
    pub fn apply_retention(
        &mut self,
        policy: &RetentionPolicy,
    ) -> Result<RetentionReport, BackupError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        self.apply_retention_at(policy, now)
    }

    /// [FABLE-5] (sq-n9s4d) [`apply_retention`](Self::apply_retention) with an explicit
    /// `now` (nanoseconds since the Unix epoch, the same clock as
    /// [`ChangeRecord::timestamp_unix_nanos`]) — the deterministic core, for hosts/tests
    /// that inject time.
    ///
    /// Drops a PREFIX of whole segments, oldest-first, stopping at the first segment that
    /// must be kept (so the retained stream stays gapless) and never touching the active
    /// segment. A segment is dropped iff:
    /// 1. every record it holds is acked (when [`RetentionPolicy::acked_through_seq`] is
    ///    set — the hard safety bound), AND
    /// 2. a pressure criterion trips — its newest record is older than
    ///    [`RetentionPolicy::max_age`], or the log's total size still exceeds
    ///    [`RetentionPolicy::max_total_bytes`] — or NO pressure criterion is configured
    ///    (pure-ack mode).
    ///
    /// A no-op policy returns immediately with nothing dropped.
    pub fn apply_retention_at(
        &mut self,
        policy: &RetentionPolicy,
        now_unix_nanos: u128,
    ) -> Result<RetentionReport, BackupError> {
        let mut report = RetentionReport {
            segments_dropped: 0,
            bytes_dropped: 0,
            first_retained_seq: self.first_seq,
        };
        if policy.is_noop() {
            return Ok(report);
        }

        let mut segments = segment_files(&self.dir)?;
        segments.sort_by_key(|(seq, _)| *seq);

        // Total on-disk size of ALL segment files (including the active one) — the quantity
        // the `max_total_bytes` budget is checked against, updated as segments drop.
        let mut total_bytes = 0u64;
        for (_, path) in &segments {
            total_bytes += fs::metadata(path)?.len();
        }

        let has_pressure = policy.max_age.is_some() || policy.max_total_bytes.is_some();
        // `windows(2)` pairs each segment with its successor, so (a) the LAST segment — the
        // active one — is never a drop candidate, and (b) the successor's first seq bounds
        // the candidate's record range: a segment keyed `first` whose successor is keyed
        // `next_first` holds exactly seqs `first..next_first` (the stream is gapless).
        for pair in segments.windows(2) {
            let (first, path) = &pair[0];
            let (next_first, _) = &pair[1];
            if *first == self.active_first_seq {
                break; // defence-in-depth: never drop the active segment
            }
            let last_seq = next_first - 1;

            // 1. Hard safety bound: every record in the segment must be acked.
            if let Some(acked) = policy.acked_through_seq {
                if last_seq > acked {
                    break; // an unacked record: keep this segment and everything after it
                }
            }

            // 2. Pressure: age/size when configured; pure-ack mode drops every acked prefix.
            let drop = if has_pressure {
                let by_size = policy
                    .max_total_bytes
                    .is_some_and(|budget| total_bytes > budget);
                let by_age = match policy.max_age {
                    None => false,
                    Some(age) => {
                        // Re-read the segment (validated) to find its newest record's
                        // timestamp; retention traffic is bounded by retention itself.
                        let (_, records) = recover_segment(path, *first)?;
                        match records.last() {
                            // A sealed segment always holds >= 1 record; an empty one is
                            // anomalous — keep it (fail-safe) rather than guess its age.
                            None => false,
                            Some(newest) => {
                                newest
                                    .timestamp_unix_nanos
                                    .saturating_add(age.as_nanos())
                                    <= now_unix_nanos
                            }
                        }
                    }
                };
                by_size || by_age
            } else {
                true
            };
            if !drop {
                break; // prefix rule: the retained stream must stay gapless
            }

            let len = fs::metadata(path)?.len();
            fs::remove_file(path)?;
            total_bytes -= len;
            report.segments_dropped += 1;
            report.bytes_dropped += len;
            self.first_seq = *next_first;
        }

        report.first_retained_seq = self.first_seq;
        Ok(report)
    }

    /// Records ONE commit — the change between the same-lineage `from` and `to` generations
    /// — as the next ordered change record, durably appended (and fsync'd, per config) before
    /// returning the assigned [`ChangeRecord`]. `to` must be strictly later than `from`
    /// (`to.number() > from.number()`), and `from.number()` must equal the
    /// [`last_generation`](Self::last_generation) of the previous record (or, for the first
    /// record, be the base the stream starts from) — a non-forward or out-of-order commit is
    /// rejected **fail-closed** ([`BackupError::Format`]) and nothing is appended.
    ///
    /// Both generations are already immutable (held by their `Arc`s), so this runs **while
    /// serving** — nothing here touches the ring or the writer; the diff reads two pinned
    /// snapshots.
    pub fn record_commit(
        &mut self,
        from: &Generation<Graph>,
        to: &Generation<Graph>,
    ) -> Result<ChangeRecord, BackupError> {
        if to.number() <= from.number() {
            return Err(BackupError::Format(format!(
                "change record range must move forward: from-generation {} >= to-generation {}",
                from.number(),
                to.number()
            )));
        }
        if let Some(last) = self.last_generation {
            if from.number() != last {
                return Err(BackupError::Format(format!(
                    "change record discontinuity: previous record ended at generation {}, \
                     but this commit starts from generation {}",
                    last,
                    from.number()
                )));
            }
        }

        let changes = diff_changes(from.snapshot(), to.snapshot());
        let timestamp_unix_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let record = ChangeRecord {
            seq: self.next_seq,
            generation: to.number(),
            timestamp_unix_nanos,
            changes,
            rebase: false,
        };

        self.append(&record)?;
        self.next_seq = record.seq + 1;
        self.last_generation = Some(record.generation);
        Ok(record)
    }

    /// [FABLE-5] (sq-r2cu1) **Explicit operator re-base** — the resync primitive for a
    /// BROKEN stream. Once a record is dropped (a recording I/O failure inside
    /// [`into_commit_hook`](Self::into_commit_hook)) or a `Writer::restore` replaces the
    /// lineage (the hook deliberately never fires for a restore publish),
    /// [`last_generation`](Self::last_generation) stops matching the writer's next commit
    /// and the discontinuity check fail-closes EVERY later
    /// [`record_commit`](Self::record_commit). This appends an honest **gap record**
    /// ([`ChangeRecord::rebase`] `= true`, empty changes, `generation` = the new baseline)
    /// and re-arms recording: the next `record_commit` must start `from` exactly
    /// `generation`. The log is never wiped — everything already recorded stays replayable,
    /// and a consumer replaying past the gap record knows the span
    /// `(previous generation, generation]` was NOT captured (re-bootstrap from a
    /// [`backup`](crate::backup) at or after `generation`, then resume).
    ///
    /// `generation` is the generation the stream re-baselines TO — typically the ring's
    /// CURRENT generation number at the moment the operator resyncs (the next commits will
    /// chain forward from it). Fail-closed ([`BackupError::Format`], nothing appended) when
    /// the stream would not move forward: `generation` must be strictly greater than the
    /// last recorded generation (an equal generation means the stream is not broken there —
    /// `record_commit` already chains — so a "re-base" to it is operator error, not a gap).
    /// On an EMPTY log this sets the stream's explicit starting baseline.
    ///
    /// Single-writer discipline: call this on THE append handle. If the log has been
    /// consumed into a commit hook ([`into_commit_hook`](Self::into_commit_hook)), use
    /// [`into_commit_hook_with_control`](Self::into_commit_hook_with_control) instead — its
    /// control handle rebases the LIVE log, serialized with the recorder — or stop the
    /// recorder and re-open the directory (a second concurrent append handle is outside
    /// the single-writer contract).
    pub fn rebase_to(&mut self, generation: u64) -> Result<ChangeRecord, BackupError> {
        if let Some(last) = self.last_generation {
            if generation <= last {
                return Err(BackupError::Format(format!(
                    "rebase must move the stream forward: last recorded generation {} >= \
                     rebase generation {}",
                    last, generation
                )));
            }
        }
        self.append_gap(generation)
    }

    /// [FABLE-5] (gh-2436) [`rebase_to`](Self::rebase_to) for a **replaced writer lineage**
    /// whose generation numbering RESTARTED — the post-restore resync. sparq-server's online
    /// restore installs a fresh ring that restarts at generation 0, so the new baseline is
    /// typically NOT strictly greater than the last recorded generation and the same-lineage
    /// forward-only check in [`rebase_to`](Self::rebase_to) would reject it forever. This
    /// variant appends the same honest gap record with NO forward check: the caller asserts
    /// the lineage (and thus the numbering) was replaced. Record `seq`s still advance
    /// monotonically — only the `generation` column may restart, exactly as it did
    /// server-side. On a same-lineage stream prefer [`rebase_to`](Self::rebase_to): its
    /// forward-only check catches an operator re-basing into already-recorded history.
    pub fn rebase_to_new_lineage(&mut self, generation: u64) -> Result<ChangeRecord, BackupError> {
        self.append_gap(generation)
    }

    /// Appends the honest **gap record** (`rebase = true`, empty changes) that re-arms
    /// recording from `generation` — shared by [`rebase_to`](Self::rebase_to) (forward-only,
    /// same lineage) and [`rebase_to_new_lineage`](Self::rebase_to_new_lineage) (numbering
    /// restarted).
    fn append_gap(&mut self, generation: u64) -> Result<ChangeRecord, BackupError> {
        let timestamp_unix_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let record = ChangeRecord {
            seq: self.next_seq,
            generation,
            timestamp_unix_nanos,
            changes: Vec::new(),
            rebase: true,
        };

        self.append(&record)?;
        self.next_seq = record.seq + 1;
        self.last_generation = Some(record.generation);
        Ok(record)
    }

    /// Appends one already-built record to the active segment (rolling first if the segment
    /// is at target), then fsyncs per config. The on-disk frame is `length-line + body +
    /// terminator`, so a torn tail is detectable on recovery.
    fn append(&mut self, record: &ChangeRecord) -> Result<(), BackupError> {
        let body = encode_record_body(record);
        let body_bytes = body.as_bytes();
        let digest = body_digest(body_bytes);
        // Frame: `record <byte-len> <fnv1a64-hex>\n` + body + `\n`. The length lets a reader
        // read the body in one shot and a truncated tail (short read) is rejected.
        let frame_header = format!("record {} {:016x}\n", body_bytes.len(), digest);
        let frame_len = frame_header.len() as u64 + body_bytes.len() as u64 + 1;

        // Roll to a new segment if the active one is at/over target AND already holds at least
        // one record (a single oversize record still lands in its own segment, never split).
        let holds_record = self.next_seq > self.active_first_seq;
        if holds_record && self.active_bytes >= self.config.segment_target_bytes {
            self.roll_segment()?;
        }

        self.active.write_all(frame_header.as_bytes())?;
        self.active.write_all(body_bytes)?;
        self.active.write_all(b"\n")?;
        self.active.flush()?;
        if self.config.fsync {
            self.active.sync_data()?;
        }
        self.active_bytes += frame_len;
        Ok(())
    }

    /// Closes the active segment and opens a fresh one keyed at the next seq.
    fn roll_segment(&mut self) -> Result<(), BackupError> {
        let path = segment_path(&self.dir, self.next_seq);
        let (file, bytes) = create_segment(&path)?;
        self.active = file;
        self.active_first_seq = self.next_seq;
        self.active_bytes = bytes;
        Ok(())
    }

    /// Polls the durable log, returning every change record with `seq >= from_seq`, in order.
    /// This re-reads the segments from disk (so it observes everything durably appended,
    /// including by a separate writer handle) and stops at the last fully-framed, digest-valid
    /// record — a half-written trailing record is never returned. **Fail-closed** on a
    /// mid-stream corruption (a bad digest or out-of-order seq before the tail).
    ///
    /// This is the resume primitive: after a restart a consumer that persisted its last-seen
    /// `seq` calls `poll(last_seen + 1)` and continues exactly where it left off.
    ///
    /// [FABLE-5] (sq-n9s4d) If `from_seq` precedes the earliest RETAINED record (older
    /// segments were dropped by [`apply_retention`](Self::apply_retention)), the poll
    /// **fails closed** with [`BackupError::Format`] rather than silently skipping the
    /// dropped records — a consumer that cannot tolerate the gap must handle it explicitly
    /// (e.g. re-bootstrap from a [`backup`](crate::backup), then resume from
    /// [`first_seq`](Self::first_seq)).
    pub fn poll(&self, from_seq: u64) -> Result<Vec<ChangeRecord>, BackupError> {
        read_from(&self.dir, from_seq)
    }

    /// [FABLE-5] (sq-bdaw5) Consumes the log into a **commit hook** for
    /// [`Writer::spawn_with_commit_hook`](crate::Writer::spawn_with_commit_hook): every
    /// commit the writer publishes is recorded via [`record_commit`](Self::record_commit)
    /// ON THE WRITER THREAD — once per published generation, so recording rides the
    /// group-commit batch instead of serialising submitters (the pre-hook wiring held a
    /// caller-side `Mutex<ChangeLog>` across `submit`, forcing one commit per recording
    /// update while the stream was on). The writer being the ring's sole publisher is
    /// exactly what makes the recorded `(from, to)` pairs gapless and monotonic — the
    /// [`record_commit`](Self::record_commit) contract — with no lock anywhere.
    ///
    /// **Error policy (a recording failure never fails the write):** by the time the
    /// hook runs, the commit is already published (and, for a durable applier, already
    /// durably sealed) — so a recording failure must not reject or roll back the write.
    /// The failed append is DROPPED and reported to `on_error` (surface it to the
    /// operator's log). Fail-closed honesty is preserved downstream: the log's
    /// [`last_generation`](Self::last_generation) does not advance on a dropped record,
    /// so the NEXT [`record_commit`](Self::record_commit) fails its discontinuity check
    /// (and every later one, until the log is re-based) rather than silently papering
    /// over the gap — `on_error` keeps firing, never a quietly incomplete stream.
    /// Re-baselining a broken stream (as after a [`Writer::restore`](crate::Writer::restore),
    /// which also breaks lineage and deliberately never fires the hook) is a separate,
    /// explicit operator action: stop the recorder, re-open the log directory, and
    /// [`rebase_to`](Self::rebase_to) the writer's current generation (sq-r2cu1) — the gap
    /// record marks the uncaptured span honestly and recording resumes without wiping
    /// the log.
    ///
    /// The hook performs the append + fsync on the writer thread, adding one record
    /// write per generation to write-ack latency (amortised across the whole batch).
    pub fn into_commit_hook<E>(
        mut self,
        mut on_error: E,
    ) -> impl FnMut(&Generation<Graph>, &Generation<Graph>) + Send
    where
        E: FnMut(&BackupError) + Send,
    {
        move |from, to| {
            if let Err(e) = self.record_commit(from, to) {
                on_error(&e);
            }
        }
    }

    /// [FABLE-5] (gh-2436) [`into_commit_hook`](Self::into_commit_hook) plus an **operator
    /// control handle** over the SAME live log. Plain `into_commit_hook` consumes the append
    /// handle into the hook closure, so re-baselining a broken stream on a running server
    /// used to require stopping the recorder and re-opening the directory offline. Here the
    /// log instead lives behind one shared mutex: the returned hook locks it per recorded
    /// commit (uncontended in the single-writer steady state — the cost is noise next to
    /// the per-record fsync), and the returned [`ChangeStreamControl`] locks it to run
    /// [`rebase_to`](Self::rebase_to) / [`rebase_to_new_lineage`](Self::rebase_to_new_lineage)
    /// WITHOUT stopping the recorder. Same hook semantics and error policy as
    /// `into_commit_hook` otherwise.
    pub fn into_commit_hook_with_control<E>(
        self,
        mut on_error: E,
    ) -> (
        impl FnMut(&Generation<Graph>, &Generation<Graph>) + Send,
        ChangeStreamControl,
    )
    where
        E: FnMut(&BackupError) + Send,
    {
        let log = Arc::new(Mutex::new(self));
        let hook_log = Arc::clone(&log);
        let hook = move |from: &Generation<Graph>, to: &Generation<Graph>| {
            let mut log = hook_log.lock().unwrap_or_else(PoisonError::into_inner);
            if let Err(e) = log.record_commit(from, to) {
                on_error(&e);
            }
        };
        (hook, ChangeStreamControl { log })
    }
}

/// [FABLE-5] (gh-2436) Cloneable operator control handle over a [`ChangeLog`] that has been
/// consumed into a live commit hook via
/// [`into_commit_hook_with_control`](ChangeLog::into_commit_hook_with_control). Every call
/// is serialized with the recording hook on the shared log mutex, so the single-writer
/// append discipline holds without stopping the recorder. A rebase that loses the race with
/// an in-flight commit stays honest either way: the commit either chains from the new
/// baseline or is dropped fail-closed (reported to the hook's `on_error`), never silently
/// mis-diffed — retry the rebase if the stream is still broken.
#[derive(Clone)]
pub struct ChangeStreamControl {
    log: Arc<Mutex<ChangeLog>>,
}

impl ChangeStreamControl {
    /// [`ChangeLog::rebase_to`] on the live log — the same-lineage, forward-only resync.
    pub fn rebase_to(&self, generation: u64) -> Result<ChangeRecord, BackupError> {
        self.lock().rebase_to(generation)
    }

    /// [`ChangeLog::rebase_to_new_lineage`] on the live log — the post-restore resync for a
    /// replaced lineage whose generation numbering restarted.
    pub fn rebase_to_new_lineage(&self, generation: u64) -> Result<ChangeRecord, BackupError> {
        self.lock().rebase_to_new_lineage(generation)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ChangeLog> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Reads all records with `seq >= from_seq` from the segments in `dir`, in order. Shared by
/// [`ChangeLog::poll`] and the recovery path's cross-check. Fail-closed on mid-stream
/// corruption; tolerates a torn tail (stops).
pub(crate) fn read_from(dir: &Path, from_seq: u64) -> Result<Vec<ChangeRecord>, BackupError> {
    let mut segments = segment_files(dir)?;
    segments.sort_by_key(|(seq, _)| *seq);

    // [FABLE-5] (sq-n9s4d) The stream starts at the oldest RETAINED segment's key (0 for a
    // never-trimmed log). Asking for records BEFORE that is fail-closed: retention dropped
    // them, and silently resuming later would violate the gapless-replay contract.
    let earliest = segments.first().map(|(seq, _)| *seq).unwrap_or(0);
    if from_seq < earliest {
        return Err(BackupError::Format(format!(
            "poll offset {} precedes the earliest retained record {} — older segments were \
             dropped by retention; resume at or after the earliest retained seq",
            from_seq, earliest
        )));
    }

    let mut out = Vec::new();
    let mut expected_seq = earliest;
    for (_first_seq, path) in &segments {
        let (_end, recovered) = recover_segment(path, expected_seq)?;
        for rec in recovered {
            expected_seq = rec.seq + 1;
            if rec.seq >= from_seq {
                out.push(rec);
            }
        }
    }
    Ok(out)
}

/// Re-reads one segment file, validating framing + digest + monotonic seq starting at
/// `expected_first_seq`. Returns `(valid_bytes, records)` where `valid_bytes` is the byte
/// offset just past the last COMPLETE record (the truncation point for a torn tail).
///
/// Fail-closed on a mid-stream corruption (bad magic/version, bad digest, or a seq that is
/// not exactly `expected`); a torn TAIL (a frame header whose declared body length runs past
/// EOF, or a missing body terminator) stops reading cleanly and is reported via `valid_bytes`
/// so the caller can truncate it away on open.
fn recover_segment(
    path: &Path,
    expected_first_seq: u64,
) -> Result<(u64, Vec<ChangeRecord>), BackupError> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    // --- Magic + version line ---
    let header = read_line_opt(&mut reader)?
        .ok_or_else(|| BackupError::Format(format!("empty change-stream segment {:?}", path)))?;
    let mut parts = header.splitn(2, ' ');
    let magic = parts.next().unwrap_or("");
    if magic != SEGMENT_MAGIC {
        return Err(BackupError::Format(format!(
            "not a sparq change-stream segment (expected magic {:?})",
            SEGMENT_MAGIC
        )));
    }
    let version: u32 = parts
        .next()
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| BackupError::Format("missing/invalid segment format version".into()))?;
    if version != SEGMENT_FORMAT_VERSION {
        return Err(BackupError::Format(format!(
            "unsupported change-stream segment version {} (this build reads/writes {})",
            version, SEGMENT_FORMAT_VERSION
        )));
    }

    // The byte offset just past the header line == the start of the first record frame.
    let mut valid_bytes = header.len() as u64 + 1; // +1 for the '\n'
    let mut records = Vec::new();
    let mut expected_seq = expected_first_seq;

    loop {
        // Read a frame header line: `record <len> <digest-hex>`. EOF here = clean end.
        let frame = match read_line_opt(&mut reader)? {
            None => break, // clean EOF at a record boundary
            Some(l) => l,
        };
        let frame_bytes = frame.len() as u64 + 1;
        let mut fp = frame.splitn(3, ' ');
        let kind = fp.next().unwrap_or("");
        if kind != "record" {
            break; // a torn tail (partial header line) — not a complete record
        }
        let len: usize = match fp.next().and_then(|v| v.parse().ok()) {
            Some(n) => n,
            None => break, // partial/garbled header → torn tail
        };
        let digest_hdr: u64 = match fp
            .next()
            .and_then(|v| u64::from_str_radix(v.trim(), 16).ok())
        {
            Some(d) => d,
            None => break, // partial/garbled header → torn tail
        };

        // Read exactly `len` body bytes + the trailing '\n'. A short read = torn tail.
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).is_err() {
            break; // torn tail: body shorter than declared
        }
        let mut nl = [0u8; 1];
        if reader.read_exact(&mut nl).is_err() || nl[0] != b'\n' {
            break; // torn tail: missing body terminator
        }

        // Digest is a HARD check (mid-stream corruption is fail-closed, not a tail).
        let actual = body_digest(&body);
        if actual != digest_hdr {
            return Err(BackupError::Corrupt(format!(
                "change-stream record digest mismatch in {:?} (header {:016x}, computed {:016x})",
                path, digest_hdr, actual
            )));
        }
        let body_str = String::from_utf8(body)
            .map_err(|_| BackupError::Corrupt(format!("record body not UTF-8 in {:?}", path)))?;
        let record = decode_record_body(&body_str, path)?;
        if record.seq != expected_seq {
            return Err(BackupError::Corrupt(format!(
                "change-stream seq gap in {:?} (expected {}, found {})",
                path, expected_seq, record.seq
            )));
        }
        expected_seq = record.seq + 1;
        valid_bytes += frame_bytes + len as u64 + 1;
        records.push(record);
    }

    Ok((valid_bytes, records))
}

/// Encodes a record body: a small line-oriented header (`seq` / `generation` / `timestamp` /
/// `inserts` / `deletes`) followed by the change lines (`+`/`-` tag, then the N-Quads line).
/// Deterministic so the digest is reproducible.
fn encode_record_body(record: &ChangeRecord) -> String {
    let mut s = String::new();
    let ins = record.insert_count();
    let del = record.delete_count();
    // [OPUS-4.8] positional format args (CodeQL rust/unused-variable false-positive).
    s.push_str(&format!("seq {}\n", record.seq));
    s.push_str(&format!("generation {}\n", record.generation));
    s.push_str(&format!("timestamp {}\n", record.timestamp_unix_nanos));
    // [FABLE-5] (sq-r2cu1) The re-base gap marker is an OPTIONAL header line, emitted only
    // when set — every pre-rebase record body (and its digest) stays byte-identical, and a
    // reader that predates the marker fails closed on the unknown key rather than replaying
    // a gap record as an ordinary empty commit.
    if record.rebase {
        s.push_str("rebase 1\n");
    }
    s.push_str(&format!("inserts {}\n", ins));
    s.push_str(&format!("deletes {}\n", del));
    for c in &record.changes {
        s.push(c.op.tag());
        s.push(' ');
        s.push_str(&c.quad);
        s.push('\n');
    }
    s
}

/// Decodes a record body produced by [`encode_record_body`]. Fail-closed on a malformed
/// header, a missing field, a change line that does not start with a `+`/`-` tag, or a
/// declared-vs-actual count mismatch.
fn decode_record_body(body: &str, path: &Path) -> Result<ChangeRecord, BackupError> {
    let mut seq: Option<u64> = None;
    let mut generation: Option<u64> = None;
    let mut timestamp: Option<u128> = None;
    let mut declared_inserts: Option<usize> = None;
    let mut declared_deletes: Option<usize> = None;
    let mut rebase = false;

    let mut change_lines: Vec<&str> = Vec::new();
    let mut in_changes = false;
    for line in body.lines() {
        if !in_changes {
            let mut kv = line.splitn(2, ' ');
            let key = kv.next().unwrap_or("");
            let val = kv.next().unwrap_or("").trim();
            match key {
                "seq" => seq = Some(parse_num(val, "seq", path)?),
                "generation" => generation = Some(parse_num(val, "generation", path)?),
                "timestamp" => timestamp = Some(parse_u128(val, "timestamp", path)?),
                // [FABLE-5] (sq-r2cu1) Optional re-base gap marker (absent = ordinary
                // commit record). Only the exact values `1`/`0` decode — anything else is
                // fail-closed corruption, never a silent default.
                "rebase" => {
                    rebase = match val {
                        "1" => true,
                        "0" => false,
                        _ => {
                            return Err(BackupError::Corrupt(format!(
                                "invalid `rebase` value {:?} in {:?}",
                                val, path
                            )))
                        }
                    }
                }
                "inserts" => declared_inserts = Some(parse_usize(val, "inserts", path)?),
                "deletes" => {
                    declared_deletes = Some(parse_usize(val, "deletes", path)?);
                    in_changes = true; // `deletes` is the last header line; changes follow
                }
                other => {
                    return Err(BackupError::Corrupt(format!(
                        "unknown change-record header key {:?} in {:?}",
                        other, path
                    )))
                }
            }
        } else {
            change_lines.push(line);
        }
    }

    let seq = seq.ok_or_else(|| miss("seq", path))?;
    let generation = generation.ok_or_else(|| miss("generation", path))?;
    let timestamp = timestamp.ok_or_else(|| miss("timestamp", path))?;
    let declared_inserts = declared_inserts.ok_or_else(|| miss("inserts", path))?;
    let declared_deletes = declared_deletes.ok_or_else(|| miss("deletes", path))?;

    let mut changes = Vec::with_capacity(change_lines.len());
    let (mut n_ins, mut n_del) = (0usize, 0usize);
    for line in change_lines {
        // The tag is the first char; the quad is the remainder after a single space.
        let mut chars = line.chars();
        let tag = chars.next();
        let after_tag = chars.as_str();
        let rest = after_tag.strip_prefix(' ').unwrap_or(after_tag);
        let op = match tag {
            Some('+') => {
                n_ins += 1;
                ChangeOp::Insert
            }
            Some('-') => {
                n_del += 1;
                ChangeOp::Delete
            }
            other => {
                return Err(BackupError::Corrupt(format!(
                    "invalid change-line tag {:?} in {:?}",
                    other, path
                )))
            }
        };
        changes.push(Change {
            op,
            quad: rest.to_string(),
        });
    }
    if n_ins != declared_inserts || n_del != declared_deletes {
        return Err(BackupError::Corrupt(format!(
            "change-record count mismatch in {:?} (declared +{} -{}, found +{} -{})",
            path, declared_inserts, declared_deletes, n_ins, n_del
        )));
    }

    Ok(ChangeRecord {
        seq,
        generation,
        timestamp_unix_nanos: timestamp,
        changes,
        rebase,
    })
}

/// Computes the inserts + deletes between two same-lineage generations as the quad-set
/// difference of their N-Quads serialisations (same approach + same-lineage boundary as
/// [`backup_delta`](crate::backup_delta)). Deterministic order: inserts (sorted) first, then
/// deletes (sorted).
fn diff_changes(from: &Graph, to: &Graph) -> Vec<Change> {
    use sparq_engine::serialize::graph_to_nquads;
    use std::collections::BTreeSet;

    let from_lines: BTreeSet<String> = graph_to_nquads(from)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect();
    let to_lines: BTreeSet<String> = graph_to_nquads(to)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect();

    let mut changes = Vec::new();
    for line in to_lines.difference(&from_lines) {
        changes.push(Change {
            op: ChangeOp::Insert,
            quad: line.clone(),
        });
    }
    for line in from_lines.difference(&to_lines) {
        changes.push(Change {
            op: ChangeOp::Delete,
            quad: line.clone(),
        });
    }
    changes
}

// --- segment file helpers ---

/// Builds the path of a segment whose first record is `first_seq`.
fn segment_path(dir: &Path, first_seq: u64) -> PathBuf {
    dir.join(format!(
        "{}{:020}.{}",
        SEGMENT_PREFIX, first_seq, SEGMENT_EXT
    ))
}

/// Creates a new (empty) segment file, writing its magic+version header, and returns the
/// open-for-append file plus its current byte length.
fn create_segment(path: &Path) -> Result<(File, u64), BackupError> {
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)?;
    let header = format!("{} {}\n", SEGMENT_MAGIC, SEGMENT_FORMAT_VERSION);
    f.write_all(header.as_bytes())?;
    f.flush()?;
    f.sync_data()?;
    Ok((f, header.len() as u64))
}

/// Lists every change-stream segment file in `dir` as `(first_seq, path)`. Non-segment files
/// are ignored. A filename whose seq key does not parse is a [`BackupError::Format`]
/// (fail-closed: a foreign file under the segment naming scheme is a misconfiguration).
fn segment_files(dir: &Path) -> Result<Vec<(u64, PathBuf)>, BackupError> {
    let mut out = Vec::new();
    let suffix = format!(".{}", SEGMENT_EXT);
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(rest) = name.strip_prefix(SEGMENT_PREFIX) else {
            continue;
        };
        let Some(seq_str) = rest.strip_suffix(&suffix) else {
            continue;
        };
        let seq: u64 = seq_str.parse().map_err(|_| {
            BackupError::Format(format!(
                "malformed change-stream segment filename {:?}",
                name
            ))
        })?;
        out.push((seq, path));
    }
    Ok(out)
}

/// [OPUS-5] (sq-l6zks) The earliest seq still RETAINED on disk — the first segment's key
/// (`0` for a never-trimmed or empty log). This is where a consumer with NO prior position
/// starts (the [`change_sink`](crate::change_sink) relay's fresh-consumer case); a consumer
/// that DOES have a prior position must not use it — [`read_from`] fails that case closed
/// rather than silently skipping trimmed records.
#[cfg(feature = "change-sink")]
pub(crate) fn earliest_retained_seq(dir: &Path) -> Result<u64, BackupError> {
    let mut segments = segment_files(dir)?;
    segments.sort_by_key(|(seq, _)| *seq);
    Ok(segments.first().map(|(seq, _)| *seq).unwrap_or(0))
}

/// Reads one line (without the trailing `\n`/`\r`) or `None` at EOF.
fn read_line_opt<R: io::BufRead>(reader: &mut R) -> Result<Option<String>, BackupError> {
    let mut s = String::new();
    let n = reader.read_line(&mut s)?;
    if n == 0 {
        return Ok(None);
    }
    while s.ends_with('\n') || s.ends_with('\r') {
        s.pop();
    }
    Ok(Some(s))
}

fn parse_num(val: &str, field: &str, path: &Path) -> Result<u64, BackupError> {
    val.parse().map_err(|_| {
        BackupError::Corrupt(format!("invalid `{}` value {:?} in {:?}", field, val, path))
    })
}

fn parse_usize(val: &str, field: &str, path: &Path) -> Result<usize, BackupError> {
    val.parse().map_err(|_| {
        BackupError::Corrupt(format!("invalid `{}` value {:?} in {:?}", field, val, path))
    })
}

fn parse_u128(val: &str, field: &str, path: &Path) -> Result<u128, BackupError> {
    val.parse().map_err(|_| {
        BackupError::Corrupt(format!("invalid `{}` value {:?} in {:?}", field, val, path))
    })
}

fn miss(field: &str, path: &Path) -> BackupError {
    BackupError::Corrupt(format!(
        "missing `{}` in change record in {:?}",
        field, path
    ))
}

/// `File` seek helper: position at end for append-after-truncate (recovery path).
trait SeekEnd {
    fn seek_end(&mut self) -> io::Result<()>;
}

impl SeekEnd for File {
    fn seek_end(&mut self) -> io::Result<()> {
        use std::io::Seek;
        self.seek(io::SeekFrom::End(0))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epoch::PodId;
    use crate::ring::GenerationRing;

    fn graph_with(nq: &str) -> Graph {
        Graph::load_dataset(nq, "nquads").expect("seed parses")
    }

    /// Sorted (op, quad) pairs of a record — the equality oracle for "same change set".
    fn change_pairs(rec: &ChangeRecord) -> Vec<(ChangeOp, String)> {
        let mut v: Vec<(ChangeOp, String)> =
            rec.changes.iter().map(|c| (c.op, c.quad.clone())).collect();
        v.sort();
        v
    }

    /// A `Result<T, BackupError>` where `T` is not `Debug`: pull the error without
    /// Debug-formatting Ok.
    fn err_of<T>(res: Result<T, BackupError>) -> BackupError {
        match res {
            Ok(_) => panic!("expected a fail-closed error, but it succeeded"),
            Err(e) => e,
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sparq-cdc-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ))
    }

    /// THE load-bearing invariant (sq-b4fns): write commits, replay from offset N AFTER a
    /// restart, and assert the EXACT ordered change sequence. Build a ring, record three
    /// commits with real inserts/deletes across default + named graphs, drop the log handle
    /// (simulating a process restart), REOPEN from disk, and poll from offset 1 — the
    /// recovered records must be exactly G1→G2 and G2→G3 in order, quad-for-quad.
    #[test]
    fn replay_from_offset_after_restart_is_exact() {
        let tmp = scratch("restart");
        let _ = fs::remove_dir_all(&tmp);

        let pod = PodId::new("http://ex/g");
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(
            "<http://ex/s0> <http://ex/p> <http://ex/o0> .\n",
        ));
        let g0 = ring.current();
        // G1: add a default + a named-graph triple.
        let g1 = ring.publish(
            graph_with(
                "<http://ex/s0> <http://ex/p> <http://ex/o0> .\n\
                 <http://ex/s1> <http://ex/p> <http://ex/o1> .\n\
                 <http://ex/s1> <http://ex/p> \"in-g\" <http://ex/g> .\n",
            ),
            [pod.clone()],
        );
        // G2: delete the original default triple, add another named-graph triple.
        let g2 = ring.publish(
            graph_with(
                "<http://ex/s1> <http://ex/p> <http://ex/o1> .\n\
                 <http://ex/s1> <http://ex/p> \"in-g\" <http://ex/g> .\n\
                 <http://ex/s2> <http://ex/p> \"in-g\" <http://ex/g> .\n",
            ),
            [pod.clone()],
        );
        // G3: delete the named-graph triple added in G1.
        let g3 = ring.publish(
            graph_with(
                "<http://ex/s1> <http://ex/p> <http://ex/o1> .\n\
                 <http://ex/s2> <http://ex/p> \"in-g\" <http://ex/g> .\n",
            ),
            [pod.clone()],
        );

        // Record three commits into a durable log, then DROP the handle (restart).
        {
            let mut log = ChangeLog::open(&tmp).expect("open log");
            assert_eq!(log.next_seq(), 0);
            let r1 = log.record_commit(&g0, &g1).expect("record 0->1");
            let r2 = log.record_commit(&g1, &g2).expect("record 1->2");
            let r3 = log.record_commit(&g2, &g3).expect("record 2->3");
            assert_eq!((r1.seq, r2.seq, r3.seq), (0, 1, 2));
            assert_eq!((r1.generation, r2.generation, r3.generation), (1, 2, 3));
            assert_eq!(log.next_seq(), 3);
        }

        // REOPEN from disk (process restart) and resume from offset 1.
        let reopened = ChangeLog::open(&tmp).expect("reopen log");
        assert_eq!(reopened.next_seq(), 3, "resumes at the recovered seq");
        assert_eq!(reopened.last_generation(), Some(3));
        let replayed = reopened.poll(1).expect("poll from offset 1");

        // Exactly records seq 1 and seq 2 (the G1->G2 and G2->G3 commits), in order.
        assert_eq!(replayed.len(), 2, "poll(1) skips seq 0");
        assert_eq!(replayed[0].seq, 1);
        assert_eq!(replayed[0].generation, 2);
        assert_eq!(replayed[1].seq, 2);
        assert_eq!(replayed[1].generation, 3);

        // G1->G2: delete s0 (default), add s2 (named graph).
        assert_eq!(
            change_pairs(&replayed[0]),
            vec![
                (
                    ChangeOp::Insert,
                    "<http://ex/s2> <http://ex/p> \"in-g\" <http://ex/g> .".to_string()
                ),
                (
                    ChangeOp::Delete,
                    "<http://ex/s0> <http://ex/p> <http://ex/o0> .".to_string()
                ),
            ]
        );
        // G2->G3: delete the s1 named-graph triple, nothing inserted.
        assert_eq!(
            change_pairs(&replayed[1]),
            vec![(
                ChangeOp::Delete,
                "<http://ex/s1> <http://ex/p> \"in-g\" <http://ex/g> .".to_string()
            )]
        );
        assert_eq!(replayed[1].insert_count(), 0);
        assert_eq!(replayed[1].delete_count(), 1);

        let _ = fs::remove_dir_all(&tmp);
    }

    /// Polling from offset 0 replays the WHOLE stream (every record from the start).
    #[test]
    fn poll_from_zero_replays_all() {
        let tmp = scratch("all");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let g2 = ring.publish(
            graph_with(
                "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
                 <http://ex/c> <http://ex/p> <http://ex/d> .\n",
            ),
            [],
        );
        let mut log = ChangeLog::open(&tmp).expect("open");
        log.record_commit(&g0, &g1).expect("0->1");
        log.record_commit(&g1, &g2).expect("1->2");
        let all = log.poll(0).expect("poll 0");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].seq, 0);
        assert_eq!(all[1].seq, 1);
        assert_eq!(all[0].insert_count(), 1);
        assert_eq!(all[1].insert_count(), 1);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Segment rolling: a tiny segment target forces one segment per record, and recovery +
    /// poll stitch the segments back into one ordered, gapless stream.
    #[test]
    fn segments_roll_and_recover_in_order() {
        let tmp = scratch("seg");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let mut prev = ring.current();
        let config = ChangeLogConfig {
            segment_target_bytes: 1, // roll after every record
            fsync: false,
        };
        {
            let mut log = ChangeLog::open_with_config(&tmp, config.clone()).expect("open");
            for i in 0..5u64 {
                let next = ring.publish(
                    graph_with(&format!(
                        "<http://ex/s{}> <http://ex/p> <http://ex/o{}> .\n",
                        i, i
                    )),
                    [],
                );
                log.record_commit(&prev, &next).expect("record");
                prev = next;
            }
        }
        // Multiple segment files exist.
        let segs = segment_files(&tmp).expect("list");
        assert!(
            segs.len() >= 2,
            "expected multiple segments, got {}",
            segs.len()
        );

        // Reopen and replay the whole stream — gapless, in order.
        let reopened = ChangeLog::open_with_config(&tmp, config).expect("reopen");
        assert_eq!(reopened.next_seq(), 5);
        let all = reopened.poll(0).expect("poll");
        assert_eq!(all.len(), 5);
        for (i, rec) in all.iter().enumerate() {
            assert_eq!(rec.seq, i as u64);
            assert_eq!(rec.generation, i as u64 + 1);
        }
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A non-forward commit is rejected fail-closed (and appends nothing).
    #[test]
    fn rejects_non_forward_commit() {
        let tmp = scratch("nf");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let mut log = ChangeLog::open(&tmp).expect("open");
        let err = err_of(log.record_commit(&g1, &g0)); // backwards
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
        assert_eq!(log.next_seq(), 0, "nothing appended");
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A discontinuous commit (its `from` does not match the last recorded generation) is
    /// rejected fail-closed — the gapless-ordering contract.
    #[test]
    fn rejects_discontinuous_commit() {
        let tmp = scratch("disc");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let g2 = ring.publish(
            graph_with("<http://ex/c> <http://ex/p> <http://ex/d> .\n"),
            [],
        );
        let mut log = ChangeLog::open(&tmp).expect("open");
        log.record_commit(&g0, &g1).expect("0->1");
        // Skip 1->2; recording g0->g2 has from-gen 0 != last recorded generation 1.
        let err = err_of(log.record_commit(&g0, &g2));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A mid-stream corrupted record body (digest mismatch) is rejected fail-closed on
    /// recovery — it is NOT mistaken for a torn tail.
    #[test]
    fn recover_rejects_midstream_corruption() {
        let tmp = scratch("corrupt");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let g2 = ring.publish(
            graph_with(
                "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
                 <http://ex/c> <http://ex/p> <http://ex/d> .\n",
            ),
            [],
        );
        {
            let mut log = ChangeLog::open(&tmp).expect("open");
            log.record_commit(&g0, &g1).expect("0->1");
            log.record_commit(&g1, &g2).expect("1->2");
        }
        // Corrupt the FIRST record's body (a mid-stream record, not the tail) by flipping a
        // byte well inside the first segment, after its header line + first frame header line.
        let seg = segment_path(&tmp, 0);
        let mut bytes = fs::read(&seg).expect("read seg");
        let first_nl = bytes.iter().position(|&b| b == b'\n').expect("magic nl") + 1;
        let frame_nl = bytes[first_nl..]
            .iter()
            .position(|&b| b == b'\n')
            .expect("frame nl")
            + first_nl
            + 1;
        bytes[frame_nl + 1] ^= 0xff; // flip a body byte of record 0
        fs::write(&seg, &bytes).expect("write back");
        let err = err_of(ChangeLog::open(&tmp));
        assert!(matches!(err, BackupError::Corrupt(_)), "{:?}", err);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A torn TAIL (process died mid-append: a trailing record whose body is truncated) is
    /// RECOVERED — the log resumes from the last complete record, not rejected.
    #[test]
    fn recover_truncates_torn_tail() {
        let tmp = scratch("tail");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let g2 = ring.publish(
            graph_with(
                "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
                 <http://ex/c> <http://ex/p> <http://ex/d> .\n",
            ),
            [],
        );
        {
            let mut log = ChangeLog::open(&tmp).expect("open");
            log.record_commit(&g0, &g1).expect("0->1");
            log.record_commit(&g1, &g2).expect("1->2");
        }
        // Chop the tail mid-way through the SECOND record (simulate a crash during append).
        let seg = segment_path(&tmp, 0);
        let mut bytes = fs::read(&seg).expect("read seg");
        bytes.truncate(bytes.len() - 4); // lose the tail of record 1
        fs::write(&seg, &bytes).expect("write back");

        let reopened = ChangeLog::open(&tmp).expect("reopen recovers torn tail");
        assert_eq!(
            reopened.next_seq(),
            1,
            "resumes after the last complete record"
        );
        let all = reopened.poll(0).expect("poll");
        assert_eq!(all.len(), 1, "only the complete first record survives");
        assert_eq!(all[0].seq, 0);
        // A fresh commit after recovery extends the stream cleanly from seq 1. The new
        // commit's `from` must be the last recorded generation (g1 == generation 1).
        let mut log = reopened;
        let g3 = ring.publish(
            graph_with("<http://ex/e> <http://ex/p> <http://ex/f> .\n"),
            [],
        );
        let r = log.record_commit(&g1, &g3).expect("record after recovery");
        assert_eq!(r.seq, 1);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A segment file with the wrong KIND of magic (a backup base masquerading as a segment)
    /// fails closed — the three artifact kinds can never be confused.
    #[test]
    fn open_rejects_wrong_magic_segment() {
        let tmp = scratch("magic");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("mkdir");
        let seg = segment_path(&tmp, 0);
        fs::write(&seg, b"SPARQ-BACKUP 1\ngeneration 0\n\n").expect("write fake");
        let err = err_of(ChangeLog::open(&tmp));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
        let _ = fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------------
    // [OPUS-4.8] (sq-bif) Additional correctness coverage: the fail-closed decoder
    // validation arms (reachable only with a digest-VALID but structurally-corrupt
    // record body — so a regression that dropped one of these checks would let a
    // corrupt CDC record replay silently), the malformed-segment-filename fail-closed
    // path, the poll-past-end resume edge, and the documented concurrent-poll-through-
    // a-separate-handle invariant. Each asserts a specific behaviour, not "no panic".
    // -----------------------------------------------------------------------

    /// Builds a single valid on-disk frame around an ARBITRARY body string, with the
    /// real FNV-1a body digest the reader recomputes — so the framing/digest layer
    /// passes and recovery reaches `decode_record_body`, which is the layer under test.
    fn frame_for(body: &str) -> Vec<u8> {
        let body_bytes = body.as_bytes();
        let digest = body_digest(body_bytes);
        let mut out = format!("record {} {:016x}\n", body_bytes.len(), digest).into_bytes();
        out.extend_from_slice(body_bytes);
        out.push(b'\n');
        out
    }

    /// Writes a fresh segment (magic+version header) carrying exactly one record with
    /// the given digest-valid body, at the given dir. Returns nothing — the caller then
    /// opens the dir and asserts the decoder's verdict.
    fn write_segment_with_body(dir: &Path, body: &str) {
        fs::create_dir_all(dir).expect("mkdir");
        let seg = segment_path(dir, 0);
        let mut bytes = format!("{} {}\n", SEGMENT_MAGIC, SEGMENT_FORMAT_VERSION).into_bytes();
        bytes.extend_from_slice(&frame_for(body));
        fs::write(&seg, &bytes).expect("write seg");
    }

    /// A record body with an UNKNOWN header key (digest valid, structure corrupt) is
    /// rejected fail-closed on recovery — NOT mistaken for a torn tail. Guards the
    /// `decode_record_body` unknown-key arm: a regression that ignored unknown keys
    /// would let a forward-incompatible/garbled record decode silently.
    #[test]
    fn decode_rejects_unknown_header_key() {
        let tmp = scratch("badkey");
        let _ = fs::remove_dir_all(&tmp);
        // `bogus` is not one of seq/generation/timestamp/inserts/deletes.
        write_segment_with_body(
            &tmp,
            "seq 0\ngeneration 1\ntimestamp 5\nbogus 9\ninserts 0\ndeletes 0\n",
        );
        let err = err_of(ChangeLog::open(&tmp));
        assert!(matches!(err, BackupError::Corrupt(_)), "{:?}", err);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A required header field MISSING (here `generation`) is rejected fail-closed —
    /// guards the `decode_record_body` missing-field arm.
    #[test]
    fn decode_rejects_missing_required_field() {
        let tmp = scratch("missing");
        let _ = fs::remove_dir_all(&tmp);
        // No `generation` line; `deletes` (last header) terminates the header block.
        write_segment_with_body(&tmp, "seq 0\ntimestamp 5\ninserts 0\ndeletes 0\n");
        let err = err_of(ChangeLog::open(&tmp));
        assert!(matches!(err, BackupError::Corrupt(_)), "{:?}", err);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A declared insert/delete count that disagrees with the actual change lines is
    /// rejected fail-closed — guards the count-mismatch arm. A regression dropping this
    /// check would let a record claim N changes while carrying a different set.
    #[test]
    fn decode_rejects_count_mismatch() {
        let tmp = scratch("count");
        let _ = fs::remove_dir_all(&tmp);
        // Declares 2 inserts but supplies only 1 change line.
        write_segment_with_body(
            &tmp,
            "seq 0\ngeneration 1\ntimestamp 5\ninserts 2\ndeletes 0\n\
             + <http://ex/s> <http://ex/p> <http://ex/o> .\n",
        );
        let err = err_of(ChangeLog::open(&tmp));
        assert!(matches!(err, BackupError::Corrupt(_)), "{:?}", err);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A change line whose tag is neither `+` nor `-` is rejected fail-closed — guards
    /// the invalid-tag arm of `decode_record_body`.
    #[test]
    fn decode_rejects_invalid_change_tag() {
        let tmp = scratch("tag");
        let _ = fs::remove_dir_all(&tmp);
        // `*` is not a valid op tag; declared counts are zero so only the tag is wrong.
        write_segment_with_body(
            &tmp,
            "seq 0\ngeneration 1\ntimestamp 5\ninserts 0\ndeletes 0\n\
             * <http://ex/s> <http://ex/p> <http://ex/o> .\n",
        );
        let err = err_of(ChangeLog::open(&tmp));
        assert!(matches!(err, BackupError::Corrupt(_)), "{:?}", err);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A well-formed body actually DECODES — proving the four reject tests above fail for
    /// the right reason (the corruption), not because every body of this shape is rejected.
    /// This is the positive control: `encode_record_body` → `decode_record_body` round-trips.
    #[test]
    fn decode_accepts_well_formed_body() {
        let rec = ChangeRecord {
            seq: 0,
            generation: 1,
            timestamp_unix_nanos: 5,
            changes: vec![
                Change {
                    op: ChangeOp::Insert,
                    quad: "<http://ex/s> <http://ex/p> <http://ex/o> .".to_string(),
                },
                Change {
                    op: ChangeOp::Delete,
                    quad: "<http://ex/x> <http://ex/y> <http://ex/z> .".to_string(),
                },
            ],
            rebase: false,
        };
        let tmp = scratch("wellformed");
        let _ = fs::remove_dir_all(&tmp);
        write_segment_with_body(&tmp, &encode_record_body(&rec));
        let reopened = ChangeLog::open(&tmp).expect("a well-formed record must decode");
        let all = reopened.poll(0).expect("poll");
        assert_eq!(all, vec![rec], "round-tripped record is identical");
        assert_eq!(all[0].insert_count(), 1);
        assert_eq!(all[0].delete_count(), 1);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A foreign file matching the segment naming scheme but with a non-numeric seq key
    /// (`changestream-NOTANUM.cdc`) is a fail-closed `Format` error — a misconfiguration
    /// must never be silently ignored. Guards the `.parse()` arm of `segment_files`,
    /// which both `open` and `poll` route through.
    #[test]
    fn segment_files_rejects_malformed_filename() {
        let tmp = scratch("badname");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("mkdir");
        let bogus = tmp.join(format!("{}NOTANUM.{}", SEGMENT_PREFIX, SEGMENT_EXT));
        fs::write(&bogus, b"").expect("write bogus seg");
        let err = err_of(ChangeLog::open(&tmp));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A file that does NOT match the segment naming scheme is IGNORED (not an error):
    /// `segment_files` only fails on the on-scheme-but-unparseable case above. A README
    /// or lockfile dropped in the log directory must not break recovery.
    #[test]
    fn unrelated_file_in_log_dir_is_ignored() {
        let tmp = scratch("foreign");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        {
            let mut log = ChangeLog::open(&tmp).expect("open");
            log.record_commit(&g0, &g1).expect("0->1");
        }
        // Drop unrelated files in the directory; neither matches the segment scheme.
        fs::write(tmp.join("README.txt"), b"notes").expect("write readme");
        fs::write(tmp.join("changestream-0.lock"), b"").expect("wrong ext");
        let reopened = ChangeLog::open(&tmp).expect("recovery ignores foreign files");
        assert_eq!(
            reopened.next_seq(),
            1,
            "the one real record still recovered"
        );
        assert_eq!(reopened.poll(0).expect("poll").len(), 1);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Polling from an offset PAST the end of the durable stream returns an empty vec
    /// (not an error) — the steady-state of a caught-up consumer that calls
    /// `poll(last_seen + 1)` and there is nothing new yet. A regression that errored or
    /// panicked here would break the resume loop's idle path.
    #[test]
    fn poll_past_end_returns_empty() {
        let tmp = scratch("pastend");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let g2 = ring.publish(
            graph_with(
                "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
                 <http://ex/c> <http://ex/p> <http://ex/d> .\n",
            ),
            [],
        );
        let mut log = ChangeLog::open(&tmp).expect("open");
        log.record_commit(&g0, &g1).expect("0->1");
        log.record_commit(&g1, &g2).expect("1->2");
        assert_eq!(log.next_seq(), 2);
        // Exactly at the next-seq boundary: nothing yet.
        assert!(log.poll(2).expect("poll at boundary").is_empty());
        // Far past the end: still empty, still Ok.
        assert!(log.poll(1_000).expect("poll far past end").is_empty());
        // And the last existing record is still pollable (boundary is off-by-one safe).
        assert_eq!(log.poll(1).expect("poll last").len(), 1);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// The documented concurrent-poll invariant: a SEPARATE reader handle
    /// (`ChangeLog::open` of the same dir) observes everything a writer handle has
    /// durably appended, and never a half-written trailing record. Here we drive it
    /// deterministically — append through the writer handle, then poll through a second
    /// handle opened over the same directory — which is the cross-service-feed shape the
    /// module docs promise (a reader is a separate `open` of the same log).
    #[test]
    fn separate_reader_handle_observes_writer_appends() {
        let tmp = scratch("twohandle");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let mut prev = ring.current();
        let mut writer = ChangeLog::open(&tmp).expect("writer handle");
        // A reader handle opened up-front over the same directory.
        let reader = ChangeLog::open(&tmp).expect("reader handle");
        assert!(reader.poll(0).expect("empty so far").is_empty());

        for i in 0..3u64 {
            let next = ring.publish(
                graph_with(&format!(
                    "<http://ex/s{}> <http://ex/p> <http://ex/o{}> .\n",
                    i, i
                )),
                [],
            );
            writer.record_commit(&prev, &next).expect("append");
            prev = next;
            // The SEPARATE reader handle sees each durably-appended record immediately
            // (poll re-reads the segments from disk — it does not share in-memory state).
            let seen = reader.poll(0).expect("reader poll");
            assert_eq!(
                seen.len(),
                (i + 1) as usize,
                "reader handle must see every durable append"
            );
            assert_eq!(seen[i as usize].seq, i);
            assert_eq!(seen[i as usize].generation, i + 1);
        }
        // Resume semantics on the reader handle: poll from a mid-offset yields the tail.
        let tail = reader.poll(1).expect("reader resume");
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].seq, 1);
        let _ = fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------------
    // [FABLE-5] (sq-n9s4d) Retention/truncation coverage. Each test is mutation-
    // witnessed: the comment names the specific regression that would turn it red.
    // -----------------------------------------------------------------------

    /// Builds a log with `n` records, ONE PER SEGMENT (`segment_target_bytes: 1`), so
    /// retention has n segments to work with (the last is active). Returns the writer
    /// handle, the ring, and the current (latest) generation for follow-on commits.
    fn one_record_segments(
        tmp: &Path,
        n: u64,
    ) -> (ChangeLog, GenerationRing<Graph>, std::sync::Arc<Generation<Graph>>) {
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let mut prev = ring.current();
        let config = ChangeLogConfig {
            segment_target_bytes: 1,
            fsync: false,
        };
        let mut log = ChangeLog::open_with_config(tmp, config).expect("open");
        for i in 0..n {
            let next = ring.publish(
                graph_with(&format!(
                    "<http://ex/s{}> <http://ex/p> <http://ex/o{}> .\n",
                    i, i
                )),
                [],
            );
            log.record_commit(&prev, &next).expect("record");
            prev = next;
        }
        (log, ring, prev)
    }

    /// The default policy is a NO-OP and `first_seq` starts at 0 — nothing is ever dropped
    /// implicitly. Direct unit tests for `RetentionPolicy::is_noop`, `ChangeLog::first_seq`,
    /// and the no-op early-return arm of `apply_retention_at`. A regression that made the
    /// empty policy drop segments (e.g. `is_noop` inverted, or pure-ack mode firing with no
    /// ack set) turns this red.
    #[test]
    fn default_retention_policy_is_noop() {
        assert!(RetentionPolicy::default().is_noop());
        assert!(!RetentionPolicy {
            max_total_bytes: Some(0),
            ..Default::default()
        }
        .is_noop());
        assert!(!RetentionPolicy {
            max_age: Some(Duration::from_secs(1)),
            ..Default::default()
        }
        .is_noop());
        assert!(!RetentionPolicy {
            acked_through_seq: Some(0),
            ..Default::default()
        }
        .is_noop());

        let tmp = scratch("noop");
        let _ = fs::remove_dir_all(&tmp);
        let (mut log, _ring, _g) = one_record_segments(&tmp, 4);
        assert_eq!(log.first_seq(), 0, "fresh log starts at seq 0");
        let report = log
            .apply_retention_at(&RetentionPolicy::default(), u128::MAX)
            .expect("noop retention");
        assert_eq!(report.segments_dropped, 0);
        assert_eq!(report.bytes_dropped, 0);
        assert_eq!(report.first_retained_seq, 0);
        assert_eq!(segment_files(&tmp).expect("list").len(), 4, "all kept");
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Pure-ack retention (no pressure criteria): every FULLY-acked prefix segment is
    /// dropped, the first partially/un-acked one stops the pass, and the report's byte
    /// accounting equals the dropped files' actual sizes. Regressions witnessed: dropping
    /// past the ack watermark (safety-bound check removed), off-by-one on a segment's
    /// last seq, or bytes accounted from the wrong files.
    #[test]
    fn ack_retention_drops_fully_acked_prefix() {
        let tmp = scratch("ack");
        let _ = fs::remove_dir_all(&tmp);
        let (mut log, _ring, _g) = one_record_segments(&tmp, 5); // segs 0..4, active = 4
        let expected_bytes: u64 = (0..3)
            .map(|s| fs::metadata(segment_path(&tmp, s)).expect("meta").len())
            .sum();

        let report = log
            .apply_retention(&RetentionPolicy {
                acked_through_seq: Some(2),
                ..Default::default()
            })
            .expect("ack retention");
        // Segments holding seqs 0, 1, 2 are fully acked -> dropped; seq 3 is not acked.
        assert_eq!(report.segments_dropped, 3);
        assert_eq!(report.bytes_dropped, expected_bytes);
        assert_eq!(report.first_retained_seq, 3);
        assert_eq!(log.first_seq(), 3);
        assert_eq!(segment_files(&tmp).expect("list").len(), 2);
        // The retained tail replays exactly (seqs 3 and 4).
        let tail = log.poll(3).expect("poll retained tail");
        assert_eq!(tail.len(), 2);
        assert_eq!((tail[0].seq, tail[1].seq), (3, 4));
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A segment holding ANY unacked record is never dropped — the hard safety bound at
    /// sub-segment granularity. Seg 0 holds seqs {0,1}; acking only seq 0 must keep it,
    /// acking through seq 1 drops it. A regression that compared the segment's FIRST seq
    /// to the watermark (instead of its last) would drop the half-acked segment and turn
    /// the first phase red.
    #[test]
    fn ack_never_drops_partially_acked_segment() {
        let tmp = scratch("ackpart");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let mut prev = ring.current();
        // Phase 1: a BIG segment target packs seqs 0 and 1 into segment 0.
        {
            let mut log = ChangeLog::open_with_config(
                &tmp,
                ChangeLogConfig {
                    segment_target_bytes: 1_000_000,
                    fsync: false,
                },
            )
            .expect("open big");
            for i in 0..2u64 {
                let next = ring.publish(
                    graph_with(&format!(
                        "<http://ex/s{}> <http://ex/p> <http://ex/o{}> .\n",
                        i, i
                    )),
                    [],
                );
                log.record_commit(&prev, &next).expect("record");
                prev = next;
            }
        }
        // Phase 2: reopen with a TINY target so seqs 2 and 3 land in their own segments.
        let mut log = ChangeLog::open_with_config(
            &tmp,
            ChangeLogConfig {
                segment_target_bytes: 1,
                fsync: false,
            },
        )
        .expect("reopen tiny");
        for i in 2..4u64 {
            let next = ring.publish(
                graph_with(&format!(
                    "<http://ex/s{}> <http://ex/p> <http://ex/o{}> .\n",
                    i, i
                )),
                [],
            );
            log.record_commit(&prev, &next).expect("record");
            prev = next;
        }
        assert_eq!(segment_files(&tmp).expect("list").len(), 3); // {0,1} {2} {3=active}

        // Ack only seq 0: segment 0 also holds the UNACKED seq 1 -> nothing droppable.
        let report = log
            .apply_retention(&RetentionPolicy {
                acked_through_seq: Some(0),
                ..Default::default()
            })
            .expect("partial ack");
        assert_eq!(report.segments_dropped, 0, "half-acked segment must survive");
        assert_eq!(log.first_seq(), 0);

        // Ack through seq 1: segment 0 is now fully acked -> dropped; seq 2 is not acked.
        let report = log
            .apply_retention(&RetentionPolicy {
                acked_through_seq: Some(1),
                ..Default::default()
            })
            .expect("full ack");
        assert_eq!(report.segments_dropped, 1);
        assert_eq!(log.first_seq(), 2);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Size pressure drops oldest-first until the total is within budget, and NEVER drops
    /// the active segment even under a zero budget; appends continue cleanly afterwards.
    /// Regressions witnessed: budget compared against the wrong total (drops too many /
    /// too few), or the active segment entering the candidate set (the zero-budget phase
    /// would empty the log and the follow-on commit would fail).
    #[test]
    fn size_retention_drops_oldest_until_within_budget() {
        let tmp = scratch("size");
        let _ = fs::remove_dir_all(&tmp);
        let (mut log, ring, g4) = one_record_segments(&tmp, 4); // segs 0..3, active = 3
        let total: u64 = segment_files(&tmp)
            .expect("list")
            .iter()
            .map(|(_, p)| fs::metadata(p).expect("meta").len())
            .sum();

        // Budget = total - 1: dropping ONE segment (the oldest) is enough to fit.
        let report = log
            .apply_retention(&RetentionPolicy {
                max_total_bytes: Some(total - 1),
                ..Default::default()
            })
            .expect("size retention");
        assert_eq!(report.segments_dropped, 1, "one segment suffices");
        assert_eq!(log.first_seq(), 1);

        // Budget = 0: everything except the active segment goes; the log stays usable.
        let report = log
            .apply_retention(&RetentionPolicy {
                max_total_bytes: Some(0),
                ..Default::default()
            })
            .expect("zero budget");
        assert_eq!(report.segments_dropped, 2);
        assert_eq!(log.first_seq(), 3);
        assert_eq!(
            segment_files(&tmp).expect("list").len(),
            1,
            "only the active segment survives a zero budget"
        );
        // Appends continue after aggressive truncation.
        let g5 = ring.publish(
            graph_with("<http://ex/s9> <http://ex/p> <http://ex/o9> .\n"),
            [],
        );
        let rec = log.record_commit(&g4, &g5).expect("append after retention");
        assert_eq!(rec.seq, 4);
        let tail = log.poll(3).expect("poll retained");
        assert_eq!(tail.len(), 2);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Age pressure via the deterministic `apply_retention_at`: with `now` before every
    /// record's cutoff nothing is dropped; with `now` far in the future every non-active
    /// segment is dropped. A flipped age comparison turns the first phase red (everything
    /// would be "stale" at now=0); ignoring `max_age` entirely turns the second phase red.
    #[test]
    fn age_retention_drops_only_stale_segments() {
        let tmp = scratch("age");
        let _ = fs::remove_dir_all(&tmp);
        let (mut log, _ring, _g) = one_record_segments(&tmp, 3); // segs 0..2, active = 2
        let policy = RetentionPolicy {
            max_age: Some(Duration::from_nanos(1_000)),
            ..Default::default()
        };

        // now = 0: no record can be 1000ns old yet (timestamps are real wall-clock nanos).
        let report = log.apply_retention_at(&policy, 0).expect("young");
        assert_eq!(report.segments_dropped, 0, "nothing is stale at now=0");

        // now = far future: every non-active segment's newest record is long stale.
        let report = log.apply_retention_at(&policy, u128::MAX).expect("stale");
        assert_eq!(report.segments_dropped, 2);
        assert_eq!(log.first_seq(), 2);
        assert_eq!(
            segment_files(&tmp).expect("list").len(),
            1,
            "the active segment is kept even when stale"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    /// The ack watermark BOUNDS size pressure: a zero byte budget wants everything gone,
    /// but only the fully-acked prefix may go. A regression that let a pressure criterion
    /// bypass the ack safety bound would drop 2 segments here instead of 1.
    #[test]
    fn ack_bounds_size_pressure() {
        let tmp = scratch("ackbound");
        let _ = fs::remove_dir_all(&tmp);
        let (mut log, _ring, _g) = one_record_segments(&tmp, 3); // segs 0..2, active = 2
        let report = log
            .apply_retention(&RetentionPolicy {
                max_total_bytes: Some(0),
                acked_through_seq: Some(0),
                ..Default::default()
            })
            .expect("bounded");
        assert_eq!(report.segments_dropped, 1, "only the acked seg 0 may go");
        assert_eq!(log.first_seq(), 1);
        assert_eq!(segment_files(&tmp).expect("list").len(), 2);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Polling from a trimmed-away offset FAILS CLOSED (never a silent skip), on the
    /// trimming handle AND on a separate reader handle over the same directory. A
    /// regression that made `poll` silently start at the trim horizon would return Ok
    /// here and turn both phases red.
    #[test]
    fn poll_before_trim_horizon_fails_closed() {
        let tmp = scratch("horizon");
        let _ = fs::remove_dir_all(&tmp);
        let (mut log, _ring, _g) = one_record_segments(&tmp, 4);
        log.apply_retention(&RetentionPolicy {
            acked_through_seq: Some(1),
            ..Default::default()
        })
        .expect("trim");
        assert_eq!(log.first_seq(), 2);

        let err = err_of(log.poll(0));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
        let err = err_of(log.poll(1));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
        // The horizon itself and everything after it still replay.
        assert_eq!(log.poll(2).expect("poll horizon").len(), 2);

        // A SEPARATE reader handle observes the same fail-closed horizon from disk.
        let reader = ChangeLog::open(&tmp).expect("reader");
        let err = err_of(reader.poll(0));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
        assert_eq!(reader.poll(2).expect("reader poll").len(), 2);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A trimmed log REOPENS correctly: `open` recovers the trim horizon from the segment
    /// filenames, resumes the seq/generation counters, replays the retained tail, and
    /// extends the stream. A regression that kept `open`'s seq validation anchored at 0
    /// (instead of the oldest retained segment) would reject the trimmed log outright.
    #[test]
    fn reopen_after_retention_resumes_and_extends() {
        let tmp = scratch("reopen");
        let _ = fs::remove_dir_all(&tmp);
        let (mut log, ring, g5) = one_record_segments(&tmp, 5);
        log.apply_retention(&RetentionPolicy {
            acked_through_seq: Some(2),
            ..Default::default()
        })
        .expect("trim");
        drop(log); // restart

        let mut reopened = ChangeLog::open(&tmp).expect("reopen trimmed log");
        assert_eq!(reopened.first_seq(), 3, "trim horizon recovered from disk");
        assert_eq!(reopened.next_seq(), 5, "seq counter resumed");
        assert_eq!(reopened.last_generation(), Some(5));
        let tail = reopened.poll(3).expect("replay retained tail");
        assert_eq!(tail.len(), 2);
        assert_eq!((tail[0].seq, tail[1].seq), (3, 4));

        // The stream extends past the restart.
        let g6 = ring.publish(
            graph_with("<http://ex/s9> <http://ex/p> <http://ex/o9> .\n"),
            [],
        );
        let rec = reopened.record_commit(&g5, &g6).expect("extend");
        assert_eq!(rec.seq, 5);
        assert_eq!(reopened.poll(3).expect("poll").len(), 3);
        let _ = fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------------
    // [FABLE-5] (sq-r2cu1) Re-baseline / resync coverage: the explicit operator
    // primitive that resumes a broken stream (dropped record / Writer::restore)
    // without wiping the log, via an honest gap record.
    // -----------------------------------------------------------------------

    /// THE acceptance invariant (sq-r2cu1): after a dropped record, `record_commit`
    /// fail-closes every later append (the broken-stream state), `rebase_to` appends an
    /// honest gap record, and recording resumes gapless from the new baseline — with the
    /// full pre-gap history still replayable. A regression that let `record_commit`
    /// succeed across the gap (discontinuity check weakened), or that made `rebase_to`
    /// wipe/skip existing records, turns this red.
    #[test]
    fn rebase_resumes_broken_stream_with_honest_gap() {
        let tmp = scratch("rebase");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let g2 = ring.publish(
            graph_with("<http://ex/c> <http://ex/p> <http://ex/d> .\n"),
            [],
        );
        let g3 = ring.publish(
            graph_with(
                "<http://ex/c> <http://ex/p> <http://ex/d> .\n\
                 <http://ex/e> <http://ex/p> <http://ex/f> .\n",
            ),
            [],
        );

        let mut log = ChangeLog::open(&tmp).expect("open");
        log.record_commit(&g0, &g1).expect("0->1");
        // The g1->g2 record is DROPPED (simulating the commit hook's recording I/O
        // failure): last_generation stays at 1, so EVERY later commit fail-closes.
        let err = err_of(log.record_commit(&g2, &g3));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
        assert_eq!(log.next_seq(), 1, "the broken stream appends nothing");

        // Explicit operator resync: re-base to the writer's current generation (2, the
        // predecessor of the next commit that will be recorded).
        let gap = log.rebase_to(g2.number()).expect("rebase");
        assert_eq!(gap.seq, 1, "the gap record extends the stream, no wipe");
        assert_eq!(gap.generation, 2);
        assert!(gap.rebase, "the gap record is honestly marked");
        assert!(gap.changes.is_empty(), "a gap record carries no diff");
        assert_eq!(log.last_generation(), Some(2), "recording re-armed");

        // Recording resumes gapless from the new baseline.
        let rec = log.record_commit(&g2, &g3).expect("record after rebase");
        assert_eq!((rec.seq, rec.generation), (2, 3));
        assert!(!rec.rebase);

        // The whole stream — pre-gap history, the gap marker, post-gap records — replays.
        let all = log.poll(0).expect("poll");
        assert_eq!(all.len(), 3);
        assert_eq!(
            all.iter().map(|r| (r.seq, r.generation, r.rebase)).collect::<Vec<_>>(),
            vec![(0, 1, false), (1, 2, true), (2, 3, false)]
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A re-based stream survives a restart: `open` recovers the gap record (its `rebase`
    /// flag round-trips through the on-disk body) and the resumed seq/generation counters
    /// chain from the gap's baseline. A regression that dropped the `rebase` header on
    /// encode, defaulted it on decode, or re-anchored `last_generation` elsewhere turns
    /// this red.
    #[test]
    fn rebase_survives_restart() {
        let tmp = scratch("rebase-restart");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let g2 = ring.publish(
            graph_with("<http://ex/c> <http://ex/p> <http://ex/d> .\n"),
            [],
        );
        {
            let mut log = ChangeLog::open(&tmp).expect("open");
            log.record_commit(&g0, &g1).expect("0->1");
            log.rebase_to(g2.number()).expect("rebase"); // g1->g2 was lost
        }

        let mut reopened = ChangeLog::open(&tmp).expect("reopen");
        assert_eq!(reopened.next_seq(), 2);
        assert_eq!(
            reopened.last_generation(),
            Some(2),
            "the recovered baseline is the gap record's generation"
        );
        let all = reopened.poll(0).expect("poll");
        assert!(!all[0].rebase);
        assert!(all[1].rebase, "the gap marker round-trips through disk");

        // The stream extends past the restart, chaining from the recovered baseline.
        let g3 = ring.publish(
            graph_with("<http://ex/e> <http://ex/p> <http://ex/f> .\n"),
            [],
        );
        let rec = reopened.record_commit(&g2, &g3).expect("extend");
        assert_eq!((rec.seq, rec.generation), (2, 3));
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A re-base that does not move the stream FORWARD is rejected fail-closed and appends
    /// nothing — both backwards (below the last recorded generation) and to the last
    /// recorded generation itself (the stream is not broken there; `record_commit` already
    /// chains). A regression that allowed `<=` would let an operator script silently
    /// rewrite the baseline into already-recorded history.
    #[test]
    fn rebase_rejects_non_forward() {
        let tmp = scratch("rebase-nf");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let g2 = ring.publish(
            graph_with("<http://ex/c> <http://ex/p> <http://ex/d> .\n"),
            [],
        );
        let mut log = ChangeLog::open(&tmp).expect("open");
        log.record_commit(&g0, &g1).expect("0->1");
        log.record_commit(&g1, &g2).expect("1->2");

        let err = err_of(log.rebase_to(1)); // backwards
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
        let err = err_of(log.rebase_to(2)); // equal: not a gap
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
        assert_eq!(log.next_seq(), 2, "nothing appended");
        assert_eq!(log.last_generation(), Some(2), "baseline unchanged");
        let _ = fs::remove_dir_all(&tmp);
    }

    /// On an EMPTY log, `rebase_to` sets an explicit starting baseline: the first
    /// `record_commit` must then chain from exactly that generation (whereas a fresh,
    /// never-rebased log accepts any starting `from`).
    #[test]
    fn rebase_on_empty_log_sets_baseline() {
        let tmp = scratch("rebase-empty");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let g2 = ring.publish(
            graph_with("<http://ex/c> <http://ex/p> <http://ex/d> .\n"),
            [],
        );
        let mut log = ChangeLog::open(&tmp).expect("open");
        let gap = log.rebase_to(g1.number()).expect("baseline");
        assert_eq!((gap.seq, gap.generation), (0, 1));
        // A commit not starting from the declared baseline is discontinuous.
        let err = err_of(log.record_commit(&g0, &g2));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);
        // The chained commit records.
        let rec = log.record_commit(&g1, &g2).expect("1->2");
        assert_eq!((rec.seq, rec.generation), (1, 2));
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A gap record's body round-trips exactly through encode/decode (the positive control
    /// for the `rebase` header line), and a garbled `rebase` value is fail-closed
    /// corruption — never silently defaulted.
    #[test]
    fn rebase_record_roundtrips_and_bad_value_fails_closed() {
        let rec = ChangeRecord {
            seq: 0,
            generation: 7,
            timestamp_unix_nanos: 5,
            changes: Vec::new(),
            rebase: true,
        };
        let tmp = scratch("rebase-rt");
        let _ = fs::remove_dir_all(&tmp);
        write_segment_with_body(&tmp, &encode_record_body(&rec));
        let reopened = ChangeLog::open(&tmp).expect("a gap record must decode");
        assert_eq!(reopened.poll(0).expect("poll"), vec![rec]);
        assert_eq!(reopened.last_generation(), Some(7));
        let _ = fs::remove_dir_all(&tmp);

        let tmp = scratch("rebase-bad");
        let _ = fs::remove_dir_all(&tmp);
        write_segment_with_body(
            &tmp,
            "seq 0\ngeneration 7\ntimestamp 5\nrebase 2\ninserts 0\ndeletes 0\n",
        );
        let err = err_of(ChangeLog::open(&tmp));
        assert!(matches!(err, BackupError::Corrupt(_)), "{:?}", err);
        let _ = fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------------
    // [FABLE-5] (gh-2436) Operator surface on a RUNNING recorder: the control handle from
    // `into_commit_hook_with_control` rebases the live log without stopping the recorder.
    // -----------------------------------------------------------------------

    /// THE acceptance invariant (gh-2436): with the append handle consumed into a LIVE
    /// commit hook, `ChangeStreamControl::rebase_to` resyncs a broken stream in place —
    /// no stop, no directory re-open — and the hook then resumes recording gapless from
    /// the new baseline. A regression that made the control a second, unserialized append
    /// handle (interleaved appends), or that lost the hook's error policy, turns this red.
    #[test]
    fn control_rebases_a_live_hook_without_stopping_the_recorder() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let tmp = scratch("control-rebase");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let g2 = ring.publish(
            graph_with("<http://ex/c> <http://ex/p> <http://ex/d> .\n"),
            [],
        );
        let g3 = ring.publish(
            graph_with(
                "<http://ex/c> <http://ex/p> <http://ex/d> .\n\
                 <http://ex/e> <http://ex/p> <http://ex/f> .\n",
            ),
            [],
        );

        let log = ChangeLog::open(&tmp).expect("open");
        let dropped = Arc::new(AtomicUsize::new(0));
        let dropped_in_hook = dropped.clone();
        let (mut hook, control) = log.into_commit_hook_with_control(move |_e| {
            dropped_in_hook.fetch_add(1, Ordering::SeqCst);
        });

        hook(&g0, &g1); // recorded: seq 0, generation 1
        hook(&g2, &g3); // the g1->g2 record was never made: discontinuity, dropped
        assert_eq!(dropped.load(Ordering::SeqCst), 1, "the broken append is reported");

        // Operator resync THROUGH THE CONTROL, with the hook still armed.
        let gap = control.rebase_to(g2.number()).expect("live rebase");
        assert_eq!((gap.seq, gap.generation, gap.rebase), (1, 2, true));

        // The same still-armed hook resumes recording gapless from the new baseline.
        hook(&g2, &g3); // recorded: seq 2, generation 3
        assert_eq!(dropped.load(Ordering::SeqCst), 1, "no further drops");

        drop(hook);
        let reopened = ChangeLog::open(&tmp).expect("reopen");
        let all = reopened.poll(0).expect("poll");
        assert_eq!(
            all.iter()
                .map(|r| (r.seq, r.generation, r.rebase))
                .collect::<Vec<_>>(),
            vec![(0, 1, false), (1, 2, true), (2, 3, false)]
        );
        drop(reopened);

        // The post-restore variant is exposed on the control too: a restarted numbering
        // (generation 0 after generation 3) is accepted only through the explicit
        // new-lineage path.
        assert!(control.rebase_to(0).is_err(), "same-lineage stays forward-only");
        let gap = control
            .rebase_to_new_lineage(0)
            .expect("lineage rebase via the control");
        assert_eq!((gap.seq, gap.generation, gap.rebase), (3, 0, true));
        let _ = fs::remove_dir_all(&tmp);
    }

    /// `rebase_to_new_lineage` accepts a baseline that does NOT move the generation column
    /// forward — the post-restore case where the writer lineage was replaced and its
    /// numbering restarted (sparq-server's online restore installs a ring that restarts at
    /// generation 0). The forward-only `rebase_to` must keep rejecting the same target, and
    /// the next commit must chain from the restarted baseline.
    #[test]
    fn rebase_to_new_lineage_accepts_a_restarted_numbering() {
        let tmp = scratch("rebase-lineage");
        let _ = fs::remove_dir_all(&tmp);
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(""));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with("<http://ex/a> <http://ex/p> <http://ex/b> .\n"),
            [],
        );
        let g2 = ring.publish(
            graph_with("<http://ex/c> <http://ex/p> <http://ex/d> .\n"),
            [],
        );
        let mut log = ChangeLog::open(&tmp).expect("open");
        log.record_commit(&g0, &g1).expect("0->1");
        log.record_commit(&g1, &g2).expect("1->2");

        // A restore replaced the lineage: the NEW ring restarts at generation 0.
        let restored: GenerationRing<Graph> =
            GenerationRing::new(graph_with("<http://ex/r> <http://ex/p> <http://ex/s> .\n"));
        let r0 = restored.current();
        let r1 = restored.publish(
            graph_with(
                "<http://ex/r> <http://ex/p> <http://ex/s> .\n\
                 <http://ex/t> <http://ex/p> <http://ex/u> .\n",
            ),
            [],
        );

        // Same-lineage rebase is (correctly) fail-closed: 0 does not move gen 2 forward.
        let err = err_of(log.rebase_to(r0.number()));
        assert!(matches!(err, BackupError::Format(_)), "{:?}", err);

        // The explicit new-lineage rebase accepts the restarted baseline.
        let gap = log.rebase_to_new_lineage(r0.number()).expect("lineage rebase");
        assert_eq!((gap.seq, gap.generation, gap.rebase), (2, 0, true));
        assert_eq!(log.last_generation(), Some(0), "baseline restarted");

        // Recording chains from the restarted numbering; seqs stay monotonic.
        let rec = log.record_commit(&r0, &r1).expect("record on the new lineage");
        assert_eq!((rec.seq, rec.generation), (3, 1));

        let reopened = ChangeLog::open(&tmp).expect("reopen");
        assert_eq!(reopened.next_seq(), 4);
        assert_eq!(reopened.last_generation(), Some(1));
        let _ = fs::remove_dir_all(&tmp);
    }
}
