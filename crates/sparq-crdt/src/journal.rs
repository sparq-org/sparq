//! Durable envelope journal + snapshots (research §6.2/§8 `journal`,
//! proposal `CRDT-JOURNAL-1/2`).
//!
//! [FABLE-5] On-disk layout: one append-only log file of checksummed,
//! length-prefixed records:
//!
//! ```text
//! record   = type(1 byte) || len(u32 BE) || sha256(32 bytes) || payload(len bytes)
//! sha256   = SHA-256(type || len-BE || payload)
//! log file = header-record , envelope-record*
//! ```
//!
//! The header payload is the canonical JSON document
//! `{"dataset":…,"epoch":…,"format":"sparq-crdt-journal/1","frontier":…}`,
//! where `frontier` summarises envelope identities that were **pruned into a
//! snapshot** by [`Journal::compact`] (so the total frontier — header frontier
//! ∪ indexed records — survives compaction and restart). Envelope payloads
//! are the exact canonical envelope bytes; append is acknowledged only after
//! the record is written and fdatasync'd (`CRDT-UPD-1` step 5 durability).
//!
//! **Crash recovery**: on open, records are *streamed* in order — one frame at
//! a time into a single reused payload buffer, so peak recovery memory is
//! bounded by the largest single record rather than by the log size. A *torn
//! tail* — a *structurally* incomplete final frame, whose frame header is
//! partial or whose declared payload runs past end-of-file — is truncated
//! away, because a crash mid-append leaves exactly that and the record was
//! never acknowledged.
//! A checksum failure on any structurally-complete frame, **including the final
//! one**, is treated as real corruption and fails closed with
//! [`CrdtError::CorruptJournal`]: a full-length record whose checksum does not
//! match cannot be distinguished from an acknowledged, fdatasync'd append that
//! later suffered bit corruption, so it is never silently discarded. EOF
//! position is not a commit marker; nothing is guessed.
//!
//! A [`Snapshot`] carries dataset id, membership epoch, the complete data
//! causal context, the live dot store, the journal (envelope-identity)
//! frontier, and the causal-stability frontier, framed under the same
//! checksummed record format plus a magic prefix, and is written atomically
//! (temp file + rename + directory sync) by [`write_snapshot_file`].

use crate::CrdtError;
use crate::codec::{
    expect_array, expect_object, expect_str, parse_dec_u64, parse_summary, write_json_string,
    write_summary,
};
use crate::envelope::{Admission, DeltaEnvelope, Limits};
use crate::id::{DatasetId, Dot, EnvelopeId};
use crate::quad::CanonicalQuad;
use crate::summary::CausalSummary;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The version-1 journal header format tag.
pub const JOURNAL_FORMAT_V1: &str = "sparq-crdt-journal/1";

/// The version-1 snapshot format tag.
pub const SNAPSHOT_FORMAT_V1: &str = "sparq-crdt-snapshot/1";

/// Snapshot file magic prefix.
const SNAPSHOT_MAGIC: &[u8; 8] = b"SPQCSNP1";

/// Domain separator for [`Snapshot::content_hash`].
const SNAPSHOT_HASH_DOMAIN: &[u8] = b"sparq-crdt:snapshot:v1\0";

const REC_HEADER: u8 = 1;
const REC_ENVELOPE: u8 = 2;
const REC_SNAPSHOT: u8 = 3;
/// type + len + sha256.
const FRAME_OVERHEAD: u64 = 1 + 4 + 32;

/// Read-ahead window used while streaming recovery. Recovery walks the log
/// strictly sequentially, so a modest buffer amortises the syscalls without
/// reintroducing a whole-log allocation.
const RECOVERY_READ_BUF: usize = 64 * 1024;

fn frame(record_type: u8, payload: &[u8]) -> Vec<u8> {
    let len = u32::try_from(payload.len()).expect("record payloads are bounded far below 4 GiB");
    let mut hasher = Sha256::new();
    hasher.update([record_type]);
    hasher.update(len.to_be_bytes());
    hasher.update(payload);
    let digest = hasher.finalize();
    let mut out = Vec::with_capacity(FRAME_OVERHEAD as usize + payload.len());
    out.push(record_type);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&digest);
    out.extend_from_slice(payload);
    out
}

/// One scanned record: `(type, payload)`. Record *offsets* are only needed
/// when building a servable index, which is the streaming [`RecordReader`]'s
/// job; the in-memory scan below backs [`Snapshot::decode`], which reads a
/// single already-resident frame.
struct ScannedRecord<'b> {
    record_type: u8,
    payload: &'b [u8],
}

/// The fixed-size head of a record frame: type, declared payload length, and
/// the expected SHA-256 of `type || len-BE || payload`. Shared by the
/// in-memory [`scan_records`] and the streaming [`RecordReader`] so both read
/// the framing exactly the same way.
struct FrameHead {
    record_type: u8,
    payload_len: u64,
    digest: [u8; 32],
}

impl FrameHead {
    /// Parses the first `FRAME_OVERHEAD` bytes of `bytes`, which the caller
    /// must have established are present.
    fn parse(bytes: &[u8]) -> Self {
        FrameHead {
            record_type: bytes[0],
            payload_len: u32::from_be_bytes(bytes[1..5].try_into().expect("4 bytes")) as u64,
            digest: bytes[5..37].try_into().expect("32 bytes"),
        }
    }

    /// True iff `payload` hashes to the checksum this head declares.
    fn checksum_matches(&self, payload: &[u8]) -> bool {
        let mut hasher = Sha256::new();
        hasher.update([self.record_type]);
        hasher.update((self.payload_len as u32).to_be_bytes());
        hasher.update(payload);
        let digest: [u8; 32] = hasher.finalize().into();
        digest == self.digest
    }

    /// The end offset of the record whose head starts at `frame_offset`.
    fn record_end(&self, frame_offset: u64) -> Result<u64, CrdtError> {
        (frame_offset + FRAME_OVERHEAD)
            .checked_add(self.payload_len)
            .ok_or(CrdtError::CorruptJournal {
                offset: frame_offset,
                reason: "record length overflows".into(),
            })
    }
}

/// Scans `buf` into records. Returns the records, the offset after the last
/// valid record, and whether a *torn tail* — a structurally incomplete final
/// frame (a partial frame header, or a payload declared to run past EOF) —
/// must be truncated. A checksum mismatch on any structurally-complete frame,
/// including the final one, is real corruption and fails closed.
fn scan_records(buf: &[u8]) -> Result<(Vec<ScannedRecord<'_>>, u64, bool), CrdtError> {
    let mut records = Vec::new();
    let mut off: u64 = 0;
    let len = buf.len() as u64;
    while off < len {
        if len - off < FRAME_OVERHEAD {
            return Ok((records, off, true)); // torn tail: partial frame header
        }
        let head = FrameHead::parse(&buf[off as usize..]);
        let payload_start = off + FRAME_OVERHEAD;
        let record_end = head.record_end(off)?;
        if record_end > len {
            return Ok((records, off, true)); // torn tail: payload ran past EOF
        }
        let payload = &buf[payload_start as usize..record_end as usize];
        if !head.checksum_matches(payload) {
            // A structurally-complete frame with a bad checksum is corruption,
            // even at EOF: a full-length final record is indistinguishable from
            // an acknowledged, fdatasync'd append whose bytes were later
            // corrupted, so truncating it would silently roll the frontier back
            // past acknowledged data. EOF position is not a commit marker; only
            // a *structurally* incomplete tail (handled above) is a torn append.
            return Err(CrdtError::CorruptJournal {
                offset: off,
                reason: "record checksum mismatch".into(),
            });
        }
        records.push(ScannedRecord {
            record_type: head.record_type,
            payload,
        });
        off = record_end;
    }
    Ok((records, off, false))
}

/// Streams the records of a log, one frame at a time, into a single reused
/// payload buffer.
///
/// Same framing, torn-tail rule, and fail-closed checksum discipline as the
/// in-memory [`scan_records`], but peak memory is bounded by the largest
/// single record instead of by the whole log, which is what makes recovery of
/// a multi-GiB journal viable. A declared payload length is only ever
/// allocated after it has been shown to fit inside the log (otherwise the
/// frame is a torn tail), so a garbled length cannot provoke an allocation
/// larger than the file itself.
struct RecordReader<R> {
    reader: R,
    /// Byte length of the log being scanned.
    len: u64,
    /// Offset of the next unread frame; once the scan ends this is the offset
    /// just past the last structurally complete record.
    offset: u64,
    /// The current record's payload; reused across records.
    payload: Vec<u8>,
    /// Set when a structurally incomplete final frame was seen.
    torn: bool,
}

impl<R: Read> RecordReader<R> {
    fn new(reader: R, len: u64) -> Self {
        RecordReader {
            reader,
            len,
            offset: 0,
            payload: Vec::new(),
            torn: false,
        }
    }

    /// Reads the next record into [`Self::payload`], returning its type and
    /// payload offset. `None` ends the scan — either a clean end of log or a
    /// torn tail, distinguished by [`Self::torn`].
    fn next_record(&mut self) -> Result<Option<(u8, u64)>, CrdtError> {
        if self.torn || self.offset >= self.len {
            return Ok(None);
        }
        if self.len - self.offset < FRAME_OVERHEAD {
            self.torn = true; // torn tail: partial frame header
            return Ok(None);
        }
        let mut frame_head = [0u8; FRAME_OVERHEAD as usize];
        self.reader.read_exact(&mut frame_head)?;
        let head = FrameHead::parse(&frame_head);
        let payload_start = self.offset + FRAME_OVERHEAD;
        let record_end = head.record_end(self.offset)?;
        if record_end > self.len {
            self.torn = true; // torn tail: payload runs past EOF
            return Ok(None);
        }
        // `record_end <= len` bounds this by the largest record in the log,
        // and the buffer's capacity is carried over to the next record.
        // `reserve_exact` keeps that carried capacity at the largest record
        // seen rather than letting amortised growth overshoot it.
        self.payload.clear();
        self.payload.reserve_exact(head.payload_len as usize);
        self.payload.resize(head.payload_len as usize, 0);
        self.reader.read_exact(&mut self.payload)?;
        if !head.checksum_matches(&self.payload) {
            // Structurally complete but mis-checksummed: real corruption, and
            // fatal wherever it appears — see `scan_records`.
            return Err(CrdtError::CorruptJournal {
                offset: self.offset,
                reason: "record checksum mismatch".into(),
            });
        }
        self.offset = record_end;
        Ok(Some((head.record_type, payload_start)))
    }
}

fn encode_journal_header(admission: &Admission, pruned: &CausalSummary) -> Vec<u8> {
    let mut out = String::new();
    out.push_str("{\"dataset\":");
    write_json_string(&mut out, admission.dataset.as_str());
    out.push_str(",\"epoch\":");
    write_json_string(&mut out, &admission.epoch.to_string());
    out.push_str(",\"format\":");
    write_json_string(&mut out, JOURNAL_FORMAT_V1);
    out.push_str(",\"frontier\":");
    write_summary(&mut out, pruned);
    out.push('}');
    out.into_bytes()
}

fn decode_journal_header(
    payload: &[u8],
    admission: &Admission,
    limits: &Limits,
) -> Result<CausalSummary, CrdtError> {
    let value: Value = serde_json::from_slice(payload).map_err(|e| CrdtError::Invalid {
        what: "journal header",
        reason: format!("not valid JSON: {e}"),
    })?;
    let map = expect_object(&value, "journal header", &["dataset", "epoch", "format", "frontier"])?;
    let format = expect_str(map, "journal header", "format")?;
    if format != JOURNAL_FORMAT_V1 {
        return Err(CrdtError::UnsupportedFormat { found: format.to_owned() });
    }
    let dataset = DatasetId::new(expect_str(map, "journal header", "dataset")?)?;
    let epoch = parse_dec_u64(expect_str(map, "journal header", "epoch")?)?;
    admission.check(&dataset, epoch)?;
    let pruned = parse_summary(
        map.get("frontier").expect("key checked above"),
        "journal header frontier",
        limits.max_clock_entries,
        limits.max_cloud_dots,
    )?;
    if encode_journal_header(admission, &pruned) != payload {
        return Err(CrdtError::NonCanonical {
            reason: "journal header bytes are not canonical".into(),
        });
    }
    Ok(pruned)
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> Result<(), CrdtError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            File::open(parent)?.sync_all()?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) -> Result<(), CrdtError> {
    Ok(()) // directory fsync is a POSIX notion; other platforms rely on rename
}

/// Result of [`Journal::append`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppendOutcome {
    /// The envelope was validated, durably appended, and fdatasync'd.
    Appended,
    /// The identity was already incorporated (byte-identical record in the
    /// log, or an identity already pruned into a snapshot); nothing was
    /// written. Duplicate delivery is harmless (`CRDT-JOURNAL-1`).
    Duplicate,
}

/// An append-only, checksummed, crash-recovering durable log of canonical
/// envelopes for one `(dataset, membership epoch)` admission.
///
/// Maintains the exact envelope-identity **frontier** (header/pruned frontier
/// ∪ indexed records) and serves peers the envelopes a remote frontier is
/// missing. Cross-envelope semantic checks that need replica *state* (e.g.
/// "this dot is already live under another quad", `CRDT-WIRE-4`'s last
/// clause) belong to the state/merge layer, which is a sibling bead — the
/// journal validates each envelope's own canonical form, dataset, epoch, and
/// bounds, and the identity/byte-identity discipline across envelopes.
#[derive(Debug)]
pub struct Journal {
    file: File,
    path: PathBuf,
    admission: Admission,
    limits: Limits,
    /// Envelope-identity → (payload offset, payload length) in `file`.
    index: BTreeMap<EnvelopeId, (u64, u32)>,
    /// Identities folded into the header by compaction.
    pruned: CausalSummary,
    /// `pruned` ∪ identities in `index`.
    frontier: CausalSummary,
    /// End-of-log offset (where the next record starts).
    end: u64,
}

impl Journal {
    /// Opens (or creates) the journal at `path`, recovering from a torn tail
    /// by truncating the unacknowledged final record. Every retained envelope
    /// record is fully re-validated against `admission` and `limits`; a
    /// journal for a different dataset or epoch is rejected.
    ///
    /// The log is streamed one record at a time, so peak recovery memory is
    /// bounded by the largest single record and the resident index, not by
    /// the size of the log.
    pub fn open(path: &Path, admission: Admission, limits: Limits) -> Result<Self, CrdtError> {
        if !path.exists() {
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)?;
            let header = frame(REC_HEADER, &encode_journal_header(&admission, &CausalSummary::new()));
            file.write_all(&header)?;
            file.sync_data()?;
            sync_parent_dir(path)?;
            return Ok(Journal {
                file,
                path: path.to_owned(),
                admission,
                limits,
                index: BTreeMap::new(),
                pruned: CausalSummary::new(),
                frontier: CausalSummary::new(),
                end: header.len() as u64,
            });
        }
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let log_len = file.metadata()?.len();
        // Streamed, not slurped: one bounded record buffer walks the log, so a
        // multi-GiB journal recovers without a proportional memory spike.
        let mut scan =
            RecordReader::new(BufReader::with_capacity(RECOVERY_READ_BUF, &file), log_len);
        let (header_type, _) = scan.next_record()?.ok_or(CrdtError::CorruptJournal {
            offset: 0,
            reason: "journal has no header record".into(),
        })?;
        if header_type != REC_HEADER {
            return Err(CrdtError::CorruptJournal {
                offset: 0,
                reason: format!("first record has type {}, expected header", header_type),
            });
        }
        let pruned = decode_journal_header(&scan.payload, &admission, &limits)?;
        let mut index = BTreeMap::new();
        let mut frontier = pruned.clone();
        while let Some((record_type, payload_offset)) = scan.next_record()? {
            if record_type != REC_ENVELOPE {
                return Err(CrdtError::CorruptJournal {
                    offset: payload_offset - FRAME_OVERHEAD,
                    reason: format!("unexpected record type {}", record_type),
                });
            }
            let envelope = DeltaEnvelope::decode(&scan.payload, &admission, &limits)?;
            let id = envelope.id();
            if index
                .insert(id.clone(), (payload_offset, scan.payload.len() as u32))
                .is_some()
            {
                return Err(CrdtError::CorruptJournal {
                    offset: payload_offset - FRAME_OVERHEAD,
                    reason: "duplicate envelope identity in the log".into(),
                });
            }
            frontier.insert(id);
        }
        let (valid_end, torn) = (scan.offset, scan.torn);
        if torn {
            file.set_len(valid_end)?;
            file.sync_data()?;
        }
        Ok(Journal {
            file,
            path: path.to_owned(),
            admission,
            limits,
            index,
            pruned,
            frontier,
            end: valid_end,
        })
    }

    /// Validates and durably appends one canonical envelope, acknowledging
    /// only after fdatasync. Retry-safe: a byte-identical known identity is a
    /// no-op [`AppendOutcome::Duplicate`]; a known identity with *different*
    /// bytes is rejected (`CRDT-UPD-RETRY-1`).
    ///
    /// An identity already pruned into a snapshot also reports `Duplicate`:
    /// its bytes are no longer available for comparison, and the snapshot has
    /// already incorporated its effect, so re-admitting it is harmless and
    /// re-verification is impossible by construction.
    pub fn append(&mut self, canonical_bytes: &[u8]) -> Result<AppendOutcome, CrdtError> {
        let envelope = DeltaEnvelope::decode(canonical_bytes, &self.admission, &self.limits)?;
        let id = envelope.id();
        if let Some(&(offset, len)) = self.index.get(&id) {
            let existing = self.read_payload(offset, len)?;
            if existing == canonical_bytes {
                return Ok(AppendOutcome::Duplicate);
            }
            return Err(CrdtError::ConflictingEnvelope {
                origin: id.replica().to_base64url(),
                sequence: id.counter(),
            });
        }
        if self.pruned.contains(&id) {
            return Ok(AppendOutcome::Duplicate);
        }
        let record = frame(REC_ENVELOPE, canonical_bytes);
        self.file.seek(SeekFrom::Start(self.end))?;
        self.file.write_all(&record)?;
        self.file.sync_data()?;
        self.index.insert(
            id.clone(),
            (self.end + FRAME_OVERHEAD, canonical_bytes.len() as u32),
        );
        self.end += record.len() as u64;
        self.frontier.insert(id);
        Ok(AppendOutcome::Appended)
    }

    /// True iff the identity is in the frontier (present or pruned).
    pub fn contains(&self, id: &EnvelopeId) -> bool {
        self.frontier.contains(id)
    }

    /// Returns the canonical bytes of one retained envelope, or `None` if the
    /// identity is unknown **or** was pruned into a snapshot (peers needing a
    /// pruned identity must take the snapshot path, `CRDT-JOURNAL-2`).
    pub fn get(&mut self, id: &EnvelopeId) -> Result<Option<Vec<u8>>, CrdtError> {
        match self.index.get(id) {
            None => Ok(None),
            Some(&(offset, len)) => Ok(Some(self.read_payload(offset, len)?)),
        }
    }

    /// The exact envelope-identity frontier: pruned identities ∪ retained
    /// records (`CRDT-JOURNAL-1`).
    pub fn frontier(&self) -> &CausalSummary {
        &self.frontier
    }

    /// Number of retained (servable) envelope records.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// True iff no envelope record is retained.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Serves up to `max` retained envelopes whose identities the `remote`
    /// frontier is missing, in (origin replica bytes, sequence) order —
    /// the transfer step of the peer protocol (research §6.2 step 2).
    /// Identities already pruned here cannot be served; if the remote still
    /// misses them after this transfer, it needs a [`Snapshot`].
    pub fn serve_missing(
        &mut self,
        remote: &CausalSummary,
        max: usize,
    ) -> Result<Vec<Vec<u8>>, CrdtError> {
        let wanted: Vec<(u64, u32)> = self
            .index
            .iter()
            .filter(|(id, _)| !remote.contains(id))
            .take(max)
            .map(|(_, &loc)| loc)
            .collect();
        let mut out = Vec::with_capacity(wanted.len());
        for (offset, len) in wanted {
            out.push(self.read_payload(offset, len)?);
        }
        Ok(out)
    }

    /// Drops every retained envelope whose identity is contained in
    /// `covered` (normally a durable snapshot's journal frontier), folding
    /// the dropped identities into the header frontier so the total frontier
    /// is unchanged (`CRDT-JOURNAL-2`: pruning must not discard frontier
    /// knowledge). The log is rewritten atomically (temp file + rename +
    /// directory sync). Returns the number of records dropped.
    ///
    /// The caller is responsible for the retention rule: prune only what a
    /// durable snapshot incorporates and peers can re-obtain via that
    /// snapshot.
    pub fn compact(&mut self, covered: &CausalSummary) -> Result<usize, CrdtError> {
        let (keep, dropped): (Vec<_>, Vec<_>) = self
            .index
            .iter()
            .map(|(id, &loc)| (id.clone(), loc))
            .partition(|(id, _)| !covered.contains(id));
        if dropped.is_empty() {
            return Ok(0);
        }
        let mut pruned = self.pruned.clone();
        for (id, _) in &dropped {
            pruned.insert(id.clone());
        }
        let tmp_path = self.path.with_extension("log.tmp");
        let mut tmp = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)?;
        let header = frame(REC_HEADER, &encode_journal_header(&self.admission, &pruned));
        tmp.write_all(&header)?;
        let mut new_index = BTreeMap::new();
        let mut end = header.len() as u64;
        for (id, (offset, len)) in keep {
            let payload = self.read_payload(offset, len)?;
            let record = frame(REC_ENVELOPE, &payload);
            tmp.write_all(&record)?;
            new_index.insert(id, (end + FRAME_OVERHEAD, len));
            end += record.len() as u64;
        }
        tmp.sync_data()?;
        std::fs::rename(&tmp_path, &self.path)?;
        sync_parent_dir(&self.path)?;
        self.file = tmp;
        self.index = new_index;
        self.pruned = pruned;
        self.end = end;
        // The frontier is unchanged by construction: dropped identities moved
        // from the index into `pruned`.
        Ok(dropped.len())
    }

    fn read_payload(&mut self, offset: u64, len: u32) -> Result<Vec<u8>, CrdtError> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len as usize];
        self.file.read_exact(&mut buf)?;
        Ok(buf)
    }
}

/// A full-state checkpoint: dataset id, membership epoch, complete **data**
/// causal context, live dot store, **envelope-identity** journal frontier,
/// and the causal-stability frontier (research §6.2 step 4; proposal
/// `CRDT-JOURNAL-2`), under one checksummed frame with a domain-separated
/// content hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    dataset: DatasetId,
    epoch: u64,
    context: CausalSummary,
    store: BTreeMap<CanonicalQuad, BTreeSet<Dot>>,
    frontier: CausalSummary,
    stability: CausalSummary,
}

impl Snapshot {
    /// Builds a snapshot, enforcing the `CRDT-STATE-1` invariants the codec
    /// can check without replica state: every store entry has a non-empty dot
    /// set, every store dot is contained in the data context, and no dot
    /// occurs under two quads.
    pub fn new(
        dataset: DatasetId,
        epoch: u64,
        context: CausalSummary,
        store: BTreeMap<CanonicalQuad, BTreeSet<Dot>>,
        frontier: CausalSummary,
        stability: CausalSummary,
    ) -> Result<Self, CrdtError> {
        let mut seen: BTreeSet<&Dot> = BTreeSet::new();
        for (quad, dots) in &store {
            if dots.is_empty() {
                return Err(CrdtError::Invalid {
                    what: "snapshot store",
                    reason: format!("empty dot set under quad {:?}", quad.as_str()),
                });
            }
            for dot in dots {
                if !context.contains(dot) {
                    return Err(CrdtError::Invalid {
                        what: "snapshot store",
                        reason: "a live dot is not contained in the causal context".into(),
                    });
                }
                if !seen.insert(dot) {
                    return Err(CrdtError::DuplicateDot {
                        replica: dot.replica().to_base64url(),
                        counter: dot.counter(),
                    });
                }
            }
        }
        Ok(Snapshot {
            dataset,
            epoch,
            context,
            store,
            frontier,
            stability,
        })
    }

    /// The dataset this snapshot belongs to.
    pub fn dataset(&self) -> &DatasetId {
        &self.dataset
    }

    /// The membership epoch the snapshot was taken under.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The complete data causal context (the logical tombstone store).
    pub fn context(&self) -> &CausalSummary {
        &self.context
    }

    /// The live dot store: quad → non-empty set of live dots.
    pub fn store(&self) -> &BTreeMap<CanonicalQuad, BTreeSet<Dot>> {
        &self.store
    }

    /// The envelope-identity journal frontier this snapshot incorporates.
    pub fn frontier(&self) -> &CausalSummary {
        &self.frontier
    }

    /// The causal-stability frontier preserved across compaction
    /// (research §4.3: context may be dropped only below it).
    pub fn stability(&self) -> &CausalSummary {
        &self.stability
    }

    fn encode_payload(&self) -> Vec<u8> {
        let mut out = String::new();
        out.push_str("{\"context\":");
        write_summary(&mut out, &self.context);
        out.push_str(",\"dataset\":");
        write_json_string(&mut out, self.dataset.as_str());
        out.push_str(",\"epoch\":");
        write_json_string(&mut out, &self.epoch.to_string());
        out.push_str(",\"format\":");
        write_json_string(&mut out, SNAPSHOT_FORMAT_V1);
        out.push_str(",\"frontier\":");
        write_summary(&mut out, &self.frontier);
        out.push_str(",\"stability\":");
        write_summary(&mut out, &self.stability);
        out.push_str(",\"store\":[");
        for (i, (quad, dots)) in self.store.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"dots\":[");
            for (j, dot) in dots.iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                crate::codec::write_dot(&mut out, dot);
            }
            out.push_str("],\"quad\":");
            write_json_string(&mut out, quad.as_str());
            out.push('}');
        }
        out.push_str("]}");
        out.into_bytes()
    }

    /// The domain-separated SHA-256 content hash of the canonical payload.
    pub fn content_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(SNAPSHOT_HASH_DOMAIN);
        hasher.update(self.encode_payload());
        hasher.finalize().into()
    }

    /// Encodes the snapshot file image: magic prefix + one checksummed frame
    /// around the canonical payload.
    pub fn encode(&self) -> Vec<u8> {
        let payload = self.encode_payload();
        let mut out = Vec::with_capacity(SNAPSHOT_MAGIC.len() + payload.len() + 64);
        out.extend_from_slice(SNAPSHOT_MAGIC);
        out.extend_from_slice(&frame(REC_SNAPSHOT, &payload));
        out
    }

    /// Strictly decodes and validates a snapshot image for `admission` under
    /// `limits`: magic, frame checksum, byte bound, format version, dataset,
    /// epoch, store bounds, state invariants, and byte-level canonicality.
    pub fn decode(
        bytes: &[u8],
        admission: &Admission,
        limits: &Limits,
    ) -> Result<Self, CrdtError> {
        if bytes.len() > limits.max_snapshot_bytes {
            return Err(CrdtError::Oversized {
                what: "snapshot bytes",
                len: bytes.len(),
                max: limits.max_snapshot_bytes,
            });
        }
        if bytes.len() < SNAPSHOT_MAGIC.len() || &bytes[..SNAPSHOT_MAGIC.len()] != SNAPSHOT_MAGIC {
            return Err(CrdtError::Invalid {
                what: "snapshot",
                reason: "missing snapshot magic prefix".into(),
            });
        }
        let (records, _, torn) = scan_records(&bytes[SNAPSHOT_MAGIC.len()..])?;
        if torn || records.len() != 1 || records[0].record_type != REC_SNAPSHOT {
            return Err(CrdtError::Invalid {
                what: "snapshot",
                reason: "expected exactly one intact snapshot record".into(),
            });
        }
        let payload = records[0].payload;
        let value: Value = serde_json::from_slice(payload).map_err(|e| CrdtError::Invalid {
            what: "snapshot",
            reason: format!("not valid JSON: {e}"),
        })?;
        let map = expect_object(
            &value,
            "snapshot",
            &["context", "dataset", "epoch", "format", "frontier", "stability", "store"],
        )?;
        let format = expect_str(map, "snapshot", "format")?;
        if format != SNAPSHOT_FORMAT_V1 {
            return Err(CrdtError::UnsupportedFormat { found: format.to_owned() });
        }
        let dataset = DatasetId::new(expect_str(map, "snapshot", "dataset")?)?;
        let epoch = parse_dec_u64(expect_str(map, "snapshot", "epoch")?)?;
        admission.check(&dataset, epoch)?;
        let context = parse_summary(
            map.get("context").expect("key checked above"),
            "snapshot context",
            limits.max_clock_entries,
            limits.max_cloud_dots,
        )?;
        let frontier = parse_summary(
            map.get("frontier").expect("key checked above"),
            "snapshot frontier",
            limits.max_clock_entries,
            limits.max_cloud_dots,
        )?;
        let stability = parse_summary(
            map.get("stability").expect("key checked above"),
            "snapshot stability",
            limits.max_clock_entries,
            limits.max_cloud_dots,
        )?;
        let store_arr = expect_array(map, "snapshot", "store")?;
        if store_arr.len() > limits.max_store_entries {
            return Err(CrdtError::Oversized {
                what: "snapshot store entries",
                len: store_arr.len(),
                max: limits.max_store_entries,
            });
        }
        let mut store = BTreeMap::new();
        let mut last_quad: Option<CanonicalQuad> = None;
        for item in store_arr {
            let entry = expect_object(item, "snapshot store entry", &["dots", "quad"])?;
            let quad = CanonicalQuad::parse(
                expect_str(entry, "snapshot store entry", "quad")?,
                limits.max_quad_bytes,
            )?;
            if let Some(prev) = &last_quad {
                if prev >= &quad {
                    return Err(CrdtError::NonCanonical {
                        reason: "snapshot store is not strictly sorted by quad".into(),
                    });
                }
            }
            let dots_arr = expect_array(entry, "snapshot store entry", "dots")?;
            if dots_arr.len() > limits.max_dots_per_quad {
                return Err(CrdtError::Oversized {
                    what: "snapshot store dots",
                    len: dots_arr.len(),
                    max: limits.max_dots_per_quad,
                });
            }
            let mut dots = Vec::with_capacity(dots_arr.len());
            for dot in dots_arr {
                dots.push(crate::codec::parse_dot(dot, "snapshot store entry")?);
            }
            crate::codec::require_strictly_ascending(&dots, "snapshot store dots")?;
            store.insert(quad.clone(), dots.into_iter().collect());
            last_quad = Some(quad);
        }
        let snapshot = Snapshot::new(dataset, epoch, context, store, frontier, stability)?;
        if snapshot.encode_payload() != payload {
            return Err(CrdtError::NonCanonical {
                reason: "snapshot bytes are not the canonical encoding of their content".into(),
            });
        }
        Ok(snapshot)
    }
}

/// Atomically writes a snapshot file: temp file in the same directory, fsync,
/// rename over `path`, directory sync. A crash leaves either the old file or
/// the new one, never a torn mixture.
pub fn write_snapshot_file(path: &Path, snapshot: &Snapshot) -> Result<(), CrdtError> {
    let tmp_path = path.with_extension("snapshot.tmp");
    let mut tmp = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp_path)?;
    tmp.write_all(&snapshot.encode())?;
    tmp.sync_data()?;
    std::fs::rename(&tmp_path, path)?;
    sync_parent_dir(path)?;
    Ok(())
}

/// Reads and strictly validates a snapshot file written by
/// [`write_snapshot_file`].
pub fn read_snapshot_file(
    path: &Path,
    admission: &Admission,
    limits: &Limits,
) -> Result<Snapshot, CrdtError> {
    let bytes = std::fs::read(path)?;
    Snapshot::decode(&bytes, admission, limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{DottedAdd, ObservedRemove};
    use crate::id::ReplicaId;

    fn rid(bytes: &[u8]) -> ReplicaId {
        ReplicaId::new(bytes.to_vec()).expect("valid replica id")
    }

    fn dot(r: &[u8], c: u64) -> Dot {
        Dot::new(rid(r), c).expect("valid dot")
    }

    fn quad(line: &str) -> CanonicalQuad {
        CanonicalQuad::parse(line, 16 * 1024).expect("valid quad")
    }

    fn admission() -> Admission {
        Admission::new(DatasetId::new("https://example.test/datasets/team").unwrap(), 3)
    }

    fn envelope(sequence: u64) -> Vec<u8> {
        let line = format!("<https://ex/s{sequence}> <https://ex/p> \"v\" .");
        DeltaEnvelope::new(
            admission().dataset,
            3,
            rid(b"peer-a"),
            sequence,
            CausalSummary::new(),
            vec![DottedAdd {
                quad: quad(&line),
                dot: dot(b"peer-a", sequence),
            }],
            Vec::new(),
        )
        .unwrap()
        .encode()
    }

    fn remove_envelope(sequence: u64) -> Vec<u8> {
        DeltaEnvelope::new(
            admission().dataset,
            3,
            rid(b"peer-b"),
            sequence,
            CausalSummary::new(),
            Vec::new(),
            vec![ObservedRemove {
                quad: quad("<https://ex/s1> <https://ex/p> \"v\" ."),
                dots: vec![dot(b"peer-a", 1)],
            }],
        )
        .unwrap()
        .encode()
    }

    fn sample_snapshot() -> Snapshot {
        let mut context = CausalSummary::new();
        context.insert(dot(b"peer-a", 1));
        context.insert(dot(b"peer-a", 2));
        let mut store = BTreeMap::new();
        store.insert(
            quad("<https://ex/s1> <https://ex/p> \"v\" ."),
            [dot(b"peer-a", 2)].into_iter().collect(),
        );
        let mut frontier = CausalSummary::new();
        frontier.insert(dot(b"peer-a", 1));
        frontier.insert(dot(b"peer-a", 2));
        let mut stability = CausalSummary::new();
        stability.insert(dot(b"peer-a", 1));
        Snapshot::new(admission().dataset, 3, context, store, frontier, stability).unwrap()
    }

    #[test]
    fn open_creates_a_journal_and_reopens_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crdt.log");
        {
            let journal = Journal::open(&path, admission(), Limits::default()).unwrap();
            assert!(journal.is_empty());
            assert_eq!(journal.len(), 0);
            assert!(journal.frontier().is_empty());
        }
        let journal = Journal::open(&path, admission(), Limits::default()).unwrap();
        assert!(journal.is_empty());
    }

    #[test]
    fn open_rejects_wrong_dataset_and_epoch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crdt.log");
        Journal::open(&path, admission(), Limits::default()).unwrap();
        let other = Admission::new(DatasetId::new("https://example.test/other").unwrap(), 3);
        assert!(matches!(
            Journal::open(&path, other, Limits::default()),
            Err(CrdtError::WrongDataset { .. })
        ));
        let stale = Admission::new(admission().dataset, 4);
        assert!(matches!(
            Journal::open(&path, stale, Limits::default()),
            Err(CrdtError::WrongEpoch { .. })
        ));
    }

    #[test]
    fn append_persists_across_reopen_and_is_retry_safe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crdt.log");
        let bytes = envelope(1);
        {
            let mut journal = Journal::open(&path, admission(), Limits::default()).unwrap();
            assert_eq!(journal.append(&bytes).unwrap(), AppendOutcome::Appended);
            assert_eq!(journal.append(&bytes).unwrap(), AppendOutcome::Duplicate);
            assert_eq!(journal.len(), 1);
        }
        let mut journal = Journal::open(&path, admission(), Limits::default()).unwrap();
        assert_eq!(journal.len(), 1);
        assert!(journal.contains(&dot(b"peer-a", 1)));
        assert_eq!(journal.get(&dot(b"peer-a", 1)).unwrap().unwrap(), bytes);
        assert_eq!(journal.get(&dot(b"peer-a", 9)).unwrap(), None);
    }

    #[test]
    fn append_rejects_same_identity_with_different_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crdt.log");
        let mut journal = Journal::open(&path, admission(), Limits::default()).unwrap();
        journal.append(&envelope(1)).unwrap();
        // Same (origin, sequence) identity, different content.
        let conflicting = DeltaEnvelope::new(
            admission().dataset,
            3,
            rid(b"peer-a"),
            1,
            CausalSummary::new(),
            vec![DottedAdd {
                quad: quad("<https://ex/other> <https://ex/p> \"w\" ."),
                dot: dot(b"peer-a", 1),
            }],
            Vec::new(),
        )
        .unwrap()
        .encode();
        assert!(matches!(
            journal.append(&conflicting),
            Err(CrdtError::ConflictingEnvelope { .. })
        ));
    }

    #[test]
    fn append_rejects_invalid_envelopes_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crdt.log");
        let mut journal = Journal::open(&path, admission(), Limits::default()).unwrap();
        assert!(journal.append(b"not json").is_err());
        let wrong_epoch = DeltaEnvelope::new(
            admission().dataset,
            4,
            rid(b"peer-a"),
            1,
            CausalSummary::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
        .encode();
        assert!(matches!(
            journal.append(&wrong_epoch),
            Err(CrdtError::WrongEpoch { .. })
        ));
        assert!(journal.is_empty());
    }

    #[test]
    fn frontier_summarises_contiguous_sequences_into_the_clock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crdt.log");
        let mut journal = Journal::open(&path, admission(), Limits::default()).unwrap();
        journal.append(&envelope(1)).unwrap();
        journal.append(&envelope(2)).unwrap();
        journal.append(&remove_envelope(1)).unwrap();
        let frontier = journal.frontier();
        assert_eq!(frontier.clock().get(&rid(b"peer-a")), Some(&2));
        assert_eq!(frontier.clock().get(&rid(b"peer-b")), Some(&1));
        assert!(frontier.cloud().is_empty());
    }

    #[test]
    fn torn_tail_is_truncated_on_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crdt.log");
        {
            let mut journal = Journal::open(&path, admission(), Limits::default()).unwrap();
            journal.append(&envelope(1)).unwrap();
            journal.append(&envelope(2)).unwrap();
        }
        // Simulate a crash mid-append at every prefix length of a third record.
        let intact = std::fs::read(&path).unwrap();
        let record = frame(REC_ENVELOPE, &envelope(3));
        for cut in [1usize, 5, 36, 37, record.len() - 1] {
            let mut torn = intact.clone();
            torn.extend_from_slice(&record[..cut]);
            std::fs::write(&path, &torn).unwrap();
            let mut journal = Journal::open(&path, admission(), Limits::default()).unwrap();
            assert_eq!(journal.len(), 2, "cut at {cut}");
            assert!(journal.get(&dot(b"peer-a", 3)).unwrap().is_none());
            // The truncated log accepts the record again after recovery.
            assert_eq!(journal.append(&envelope(3)).unwrap(), AppendOutcome::Appended);
        }
    }

    #[test]
    fn checksum_mismatch_fails_closed_for_final_and_midfile_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crdt.log");
        {
            let mut journal = Journal::open(&path, admission(), Limits::default()).unwrap();
            journal.append(&envelope(1)).unwrap();
            journal.append(&envelope(2)).unwrap();
        }
        let intact = std::fs::read(&path).unwrap();
        // Flip one payload byte of the FINAL, fully-written record. Its frame is
        // structurally complete (all declared bytes present), so this is
        // indistinguishable from bit corruption of an acknowledged, fdatasync'd
        // append. It must fail closed rather than silently truncate and roll the
        // frontier back past acknowledged data. A genuine torn append is instead
        // a structurally incomplete tail — covered by torn_tail_is_truncated_on_recovery.
        let mut tail_garbled = intact.clone();
        let last = tail_garbled.len() - 1;
        tail_garbled[last] ^= 0x01;
        std::fs::write(&path, &tail_garbled).unwrap();
        assert!(matches!(
            Journal::open(&path, admission(), Limits::default()),
            Err(CrdtError::CorruptJournal { .. })
        ));
        // Flip one byte in the FIRST envelope record (not the final record):
        // also real corruption, also fails closed.
        let header_len = frame(
            REC_HEADER,
            &encode_journal_header(&admission(), &CausalSummary::new()),
        )
        .len();
        let mut mid_garbled = intact.clone();
        mid_garbled[header_len + FRAME_OVERHEAD as usize] ^= 0x01;
        std::fs::write(&path, &mid_garbled).unwrap();
        assert!(matches!(
            Journal::open(&path, admission(), Limits::default()),
            Err(CrdtError::CorruptJournal { .. })
        ));
    }

    /// Builds a log of `count` envelope records and returns its bytes.
    fn log_bytes(dir: &Path, count: u64) -> Vec<u8> {
        let path = dir.join("crdt.log");
        {
            let mut journal = Journal::open(&path, admission(), Limits::default()).unwrap();
            for sequence in 1..=count {
                journal.append(&envelope(sequence)).unwrap();
            }
        }
        std::fs::read(&path).unwrap()
    }

    #[test]
    fn streaming_scan_agrees_with_the_whole_buffer_scan() {
        let dir = tempfile::tempdir().unwrap();
        let intact = log_bytes(dir.path(), 5);
        // The intact log, plus a torn tail at every structural cut of a
        // further record (partial head, exact head, partial payload).
        let record = frame(REC_ENVELOPE, &envelope(6));
        let mut cases = vec![intact.clone()];
        for cut in [1usize, 5, 36, 37, record.len() - 1] {
            let mut torn = intact.clone();
            torn.extend_from_slice(&record[..cut]);
            cases.push(torn);
        }
        for bytes in cases {
            let (records, valid_end, torn) = scan_records(&bytes).unwrap();
            let mut stream = RecordReader::new(bytes.as_slice(), bytes.len() as u64);
            let mut streamed = Vec::new();
            while let Some((record_type, payload_offset)) = stream.next_record().unwrap() {
                streamed.push((record_type, payload_offset, stream.payload.clone()));
            }
            assert_eq!(streamed.len(), records.len());
            let mut expected_offset = 0u64;
            for (got, want) in streamed.iter().zip(&records) {
                assert_eq!(got.0, want.record_type);
                assert_eq!(got.1, expected_offset + FRAME_OVERHEAD);
                assert_eq!(got.2.as_slice(), want.payload);
                expected_offset += FRAME_OVERHEAD + want.payload.len() as u64;
            }
            assert_eq!(stream.offset, valid_end);
            assert_eq!(stream.torn, torn);
        }
    }

    #[test]
    fn streaming_scan_fails_closed_on_corruption_at_the_same_offset() {
        let dir = tempfile::tempdir().unwrap();
        let intact = log_bytes(dir.path(), 3);
        let header_len = frame(
            REC_HEADER,
            &encode_journal_header(&admission(), &CausalSummary::new()),
        )
        .len();
        // A flipped byte in the first envelope record, and in the final one:
        // both are structurally complete frames, so both are real corruption.
        for flip in [header_len + FRAME_OVERHEAD as usize, intact.len() - 1] {
            let mut garbled = intact.clone();
            garbled[flip] ^= 0x01;
            let buffered = match scan_records(&garbled) {
                Err(error) => error,
                Ok(_) => panic!("buffered scan missed corruption at {}", flip),
            };
            let mut stream = RecordReader::new(garbled.as_slice(), garbled.len() as u64);
            let streamed = loop {
                match stream.next_record() {
                    Ok(Some(_)) => continue,
                    Ok(None) => panic!("streaming scan missed corruption at {}", flip),
                    Err(error) => break error,
                }
            };
            match (buffered, streamed) {
                (
                    CrdtError::CorruptJournal { offset: buffered, .. },
                    CrdtError::CorruptJournal { offset: streamed, .. },
                ) => assert_eq!(buffered, streamed),
                other => panic!("expected two corrupt-journal errors, got {:?}", other),
            }
        }
    }

    #[test]
    fn streaming_scan_buffer_stays_bounded_by_the_largest_record() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = log_bytes(dir.path(), 64);
        let (records, _, _) = scan_records(&bytes).unwrap();
        let largest = records.iter().map(|r| r.payload.len()).max().unwrap();
        let mut stream = RecordReader::new(bytes.as_slice(), bytes.len() as u64);
        let mut scanned = 0usize;
        let mut peak = 0usize;
        while stream.next_record().unwrap().is_some() {
            scanned += 1;
            peak = peak.max(stream.payload.capacity());
        }
        assert_eq!(scanned, records.len());
        // The reused buffer tracks the largest single record, not the log —
        // this is the property that removes the whole-log memory spike.
        assert!(peak <= 2 * largest, "peak {} exceeded 2x largest record {}", peak, largest);
        assert!(
            peak * 4 < bytes.len(),
            "peak {} is not far below the {}-byte log; the case is too small to be meaningful",
            peak,
            bytes.len()
        );
    }

    #[test]
    fn serve_missing_returns_exactly_the_gap_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crdt.log");
        let mut journal = Journal::open(&path, admission(), Limits::default()).unwrap();
        for sequence in 1..=4 {
            journal.append(&envelope(sequence)).unwrap();
        }
        let mut remote = CausalSummary::new();
        remote.insert(dot(b"peer-a", 1));
        remote.insert(dot(b"peer-a", 3));
        let served = journal.serve_missing(&remote, usize::MAX).unwrap();
        assert_eq!(served, vec![envelope(2), envelope(4)]);
        // The cap is honoured.
        let served = journal.serve_missing(&remote, 1).unwrap();
        assert_eq!(served, vec![envelope(2)]);
        // A remote that has everything needs nothing.
        let complete = journal.frontier().clone();
        assert!(journal.serve_missing(&complete, usize::MAX).unwrap().is_empty());
    }

    #[test]
    fn compact_drops_covered_records_but_preserves_the_frontier() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crdt.log");
        let mut journal = Journal::open(&path, admission(), Limits::default()).unwrap();
        for sequence in 1..=3 {
            journal.append(&envelope(sequence)).unwrap();
        }
        let mut covered = CausalSummary::new();
        covered.insert(dot(b"peer-a", 1));
        covered.insert(dot(b"peer-a", 2));
        let before = journal.frontier().clone();
        assert_eq!(journal.compact(&covered).unwrap(), 2);
        assert_eq!(journal.len(), 1);
        assert_eq!(journal.frontier(), &before);
        assert!(journal.contains(&dot(b"peer-a", 1))); // pruned but in frontier
        assert!(journal.get(&dot(b"peer-a", 1)).unwrap().is_none()); // not servable
        assert!(journal.get(&dot(b"peer-a", 3)).unwrap().is_some());
        // Re-delivery of a pruned identity is a harmless duplicate.
        assert_eq!(journal.append(&envelope(1)).unwrap(), AppendOutcome::Duplicate);
        // Compacting with nothing new to cover is a no-op.
        assert_eq!(journal.compact(&covered).unwrap(), 0);
        // The compacted log survives reopen with the same frontier.
        drop(journal);
        let journal = Journal::open(&path, admission(), Limits::default()).unwrap();
        assert_eq!(journal.frontier(), &before);
        assert_eq!(journal.len(), 1);
    }

    #[test]
    fn snapshot_new_enforces_state_invariants() {
        let good = sample_snapshot();
        assert_eq!(good.epoch(), 3);
        // A store dot outside the context is rejected.
        let mut store = BTreeMap::new();
        store.insert(
            quad("<https://ex/s1> <https://ex/p> \"v\" ."),
            [dot(b"peer-z", 9)].into_iter().collect(),
        );
        assert!(Snapshot::new(
            admission().dataset,
            3,
            good.context().clone(),
            store,
            CausalSummary::new(),
            CausalSummary::new()
        )
        .is_err());
        // An empty dot set is rejected.
        let mut store = BTreeMap::new();
        store.insert(quad("<https://ex/s1> <https://ex/p> \"v\" ."), BTreeSet::new());
        assert!(Snapshot::new(
            admission().dataset,
            3,
            good.context().clone(),
            store,
            CausalSummary::new(),
            CausalSummary::new()
        )
        .is_err());
        // One dot under two quads is rejected.
        let mut store = BTreeMap::new();
        store.insert(
            quad("<https://ex/s1> <https://ex/p> \"v\" ."),
            [dot(b"peer-a", 1)].into_iter().collect(),
        );
        store.insert(
            quad("<https://ex/s2> <https://ex/p> \"v\" ."),
            [dot(b"peer-a", 1)].into_iter().collect(),
        );
        assert!(matches!(
            Snapshot::new(
                admission().dataset,
                3,
                good.context().clone(),
                store,
                CausalSummary::new(),
                CausalSummary::new()
            ),
            Err(CrdtError::DuplicateDot { .. })
        ));
    }

    #[test]
    fn snapshot_accessors_expose_the_parts() {
        let snapshot = sample_snapshot();
        assert_eq!(snapshot.dataset().as_str(), "https://example.test/datasets/team");
        assert!(snapshot.context().contains(&dot(b"peer-a", 1)));
        assert_eq!(snapshot.store().len(), 1);
        assert!(snapshot.frontier().contains(&dot(b"peer-a", 2)));
        assert!(snapshot.stability().contains(&dot(b"peer-a", 1)));
        assert!(!snapshot.stability().contains(&dot(b"peer-a", 2)));
    }

    #[test]
    fn snapshot_encode_decode_round_trips() {
        let snapshot = sample_snapshot();
        let bytes = snapshot.encode();
        let decoded = Snapshot::decode(&bytes, &admission(), &Limits::default()).unwrap();
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn snapshot_decode_rejects_wrong_admission_bounds_and_corruption() {
        let snapshot = sample_snapshot();
        let bytes = snapshot.encode();
        let other = Admission::new(DatasetId::new("https://example.test/other").unwrap(), 3);
        assert!(matches!(
            Snapshot::decode(&bytes, &other, &Limits::default()),
            Err(CrdtError::WrongDataset { .. })
        ));
        let stale = Admission::new(admission().dataset, 2);
        assert!(matches!(
            Snapshot::decode(&bytes, &stale, &Limits::default()),
            Err(CrdtError::WrongEpoch { .. })
        ));
        let limits = Limits { max_snapshot_bytes: 8, ..Limits::default() };
        assert!(matches!(
            Snapshot::decode(&bytes, &admission(), &limits),
            Err(CrdtError::Oversized { .. })
        ));
        let limits = Limits { max_store_entries: 0, ..Limits::default() };
        assert!(matches!(
            Snapshot::decode(&bytes, &admission(), &limits),
            Err(CrdtError::Oversized { .. })
        ));
        // A flipped payload byte breaks the frame checksum.
        let mut corrupt = bytes.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0x01;
        assert!(Snapshot::decode(&corrupt, &admission(), &Limits::default()).is_err());
        // Missing magic.
        assert!(Snapshot::decode(&bytes[8..], &admission(), &Limits::default()).is_err());
    }

    #[test]
    fn snapshot_content_hash_is_stable_and_content_bound() {
        let a = sample_snapshot();
        assert_eq!(a.content_hash(), a.content_hash());
        let mut frontier = a.frontier().clone();
        frontier.insert(dot(b"peer-b", 1));
        let b = Snapshot::new(
            a.dataset().clone(),
            a.epoch(),
            a.context().clone(),
            a.store().clone(),
            frontier,
            a.stability().clone(),
        )
        .unwrap();
        assert_ne!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn write_and_read_snapshot_file_round_trips_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.snapshot");
        let snapshot = sample_snapshot();
        write_snapshot_file(&path, &snapshot).unwrap();
        let read = read_snapshot_file(&path, &admission(), &Limits::default()).unwrap();
        assert_eq!(read, snapshot);
        // Overwrite goes through the same atomic path.
        write_snapshot_file(&path, &snapshot).unwrap();
        assert!(read_snapshot_file(&path, &admission(), &Limits::default()).is_ok());
        // No temp file is left behind.
        assert!(!path.with_extension("snapshot.tmp").exists());
    }
}
