//! (epic `sq-tag1q`, child `sq-tag1q.7.2`) SPARQL-CRDT **codec +
//! journal** layer for sparq: the bounded, versioned canonical delta envelope,
//! causal summaries, the durable append/recovery journal, snapshots,
//! missing-interval exchange primitives, and membership-epoch /
//! causal-stability tracking from `research/sparql-crdt-gpt56-2026-07.md`
//! (§4.1, §6, §8) and the proposal draft `site/specs/sparql-crdt.typ`.
//!
//! # What this crate is (and is not)
//!
//! This is the **interchange and durability** layer of the SPARQL-CRDT design:
//!
//! * [`ReplicaId`] / [`DatasetId`] / [`Dot`] — the identifier contract
//!   (research §4.1);
//! * [`CausalSummary`] — the compact `clock` (version vector) + `cloud`
//!   (sparse dots) causal-context representation, used both for **data** dots
//!   and for **envelope-identity** journal frontiers;
//! * [`DeltaEnvelope`] — the bounded canonical envelope with its strict codec:
//!   decode **rejects** non-canonical bytes, oversized input, wrong-dataset,
//!   wrong-epoch, unknown versions, blank-node quads, and duplicate dots
//!   rather than normalising them;
//! * [`Journal`] — an append-only, checksummed, crash-recovering envelope log
//!   that serves missing intervals to peers and supports compaction beneath a
//!   snapshot;
//! * [`Snapshot`] — dataset id, dot store, causal context, journal frontier,
//!   and causal-stability frontier under one content hash, with atomic
//!   write/read helpers;
//! * [`SyncHello`], [`missing_intervals`], [`StabilityTracker`] — the peer
//!   handshake and reconciliation primitives of research §6.2 and the
//!   membership-epoch / causal-stability frontier rule of research §4.3.
//!
//! The replicated **dot-store state and merge algebra** (join), the
//! **evaluate-at-origin SPARQL Update compiler**, and the sparq-core
//! materialisation adapter are sibling beads of epic `sq-tag1q.7` and are
//! **not** in this crate yet, so the crate is `publish = false` and claims no
//! conformance class from the proposal draft. Convergence, atomic-visibility,
//! and admission-policy properties of the full design are therefore **not**
//! claims made by this crate; it supplies the byte formats and durability
//! those beads build on.
//!
//! # Canonical form
//!
//! Every wire document (envelope, journal header, snapshot payload, sync
//! hello) is a canonical JSON text in the RFC 8785 (JCS) subset this crate
//! emits: object keys sorted, no insignificant whitespace, all scalars encoded
//! as strings (counters as shortest-form decimal, replica identifiers as
//! unpadded base64url), arrays sorted and duplicate-free as the proposal
//! (`CRDT-WIRE-2/3`) requires. Decoding parses strictly, re-encodes, and
//! **byte-compares** against the input, so exactly one byte representation of
//! an identity is ever accepted (`CRDT-UPD-RETRY-1`).
//!
//! The envelope carries a `membership_epoch` field (research §6.1) that the
//! proposal draft's envelope sketch does not yet show; freezing it is decision
//! 5 of research §11 and admission rejects a wrong-epoch envelope here, as the
//! implementation bead requires.
//!
//! # Dependencies
//!
//! Only existing, attested workspace dependencies are used (`oxrdf` + `oxttl`
//! for quad validation, `serde_json` as the strict JSON reader, `sha2` for
//! checksums/content hashes); this bead adds no new supply-chain surface.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;

pub mod codec;
pub mod envelope;
pub mod id;
pub mod journal;
pub mod quad;
pub mod summary;
pub mod sync;

pub use envelope::{Admission, DeltaEnvelope, DottedAdd, Limits, ObservedRemove, envelope_hash};
pub use id::{DatasetId, Dot, EnvelopeId, ReplicaId};
pub use journal::{
    AppendOutcome, Journal, Snapshot, read_snapshot_file, write_snapshot_file,
};
pub use quad::CanonicalQuad;
pub use summary::CausalSummary;
pub use sync::{
    Membership, SequenceInterval, StabilityTracker, SyncHello, missing_intervals,
};

/// Unified error type for every fallible operation in this crate.
///
/// Decode-side variants are deliberately specific so admission code can
/// distinguish "reject and drop" causes ([`CrdtError::WrongDataset`],
/// [`CrdtError::WrongEpoch`], [`CrdtError::Oversized`],
/// [`CrdtError::NonCanonical`], …) from local durability faults
/// ([`CrdtError::Io`], [`CrdtError::CorruptJournal`]).
#[derive(Debug)]
#[non_exhaustive]
pub enum CrdtError {
    /// An input exceeded a configured [`Limits`] bound; nothing was allocated
    /// or applied.
    Oversized {
        /// Which bound was exceeded (e.g. `"envelope bytes"`).
        what: &'static str,
        /// Observed size.
        len: usize,
        /// Configured maximum.
        max: usize,
    },
    /// The document names a dataset other than the one this receiver admits.
    WrongDataset {
        /// Dataset identifier the receiver admits.
        expected: String,
        /// Dataset identifier found in the document.
        found: String,
    },
    /// The document names a membership epoch other than the current one.
    WrongEpoch {
        /// Membership epoch the receiver admits.
        expected: u64,
        /// Epoch found in the document.
        found: u64,
    },
    /// The `format` field names a version this implementation does not
    /// support. Unknown versions fail closed (research §6.1).
    UnsupportedFormat {
        /// The `format` value found.
        found: String,
    },
    /// The bytes are not the canonical encoding of their content (key order,
    /// whitespace, sorting, duplicates, escaping, non-shortest counters, …).
    /// Exactly one byte form per identity is accepted; nothing is normalised.
    NonCanonical {
        /// Human-readable reason.
        reason: String,
    },
    /// A structurally or semantically invalid field value.
    Invalid {
        /// Which field or construct is invalid.
        what: &'static str,
        /// Human-readable reason.
        reason: String,
    },
    /// A dot occurred more than once where uniqueness is required
    /// (`CRDT-WIRE-4`: within an envelope's adds, or as both an add and a
    /// remove of the same envelope).
    DuplicateDot {
        /// Base64url form of the offending dot's replica identifier.
        replica: String,
        /// The offending dot's counter.
        counter: u64,
    },
    /// An envelope identity was seen before with **different** canonical
    /// bytes; `CRDT-UPD-RETRY-1` requires rejection.
    ConflictingEnvelope {
        /// Base64url form of the origin replica identifier.
        origin: String,
        /// The origin journal sequence.
        sequence: u64,
    },
    /// The journal contains a corrupt record that is not a torn tail (torn
    /// tails are silently truncated during recovery; mid-file corruption is
    /// not recoverable and fails closed).
    CorruptJournal {
        /// Byte offset of the corrupt record.
        offset: u64,
        /// Human-readable reason.
        reason: String,
    },
    /// An underlying I/O failure.
    Io(std::io::Error),
}

impl fmt::Display for CrdtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CrdtError::Oversized { what, len, max } => {
                write!(f, "oversized {what}: {len} bytes/items exceeds limit {max}")
            }
            CrdtError::WrongDataset { expected, found } => {
                write!(f, "wrong dataset: expected <{expected}>, found <{found}>")
            }
            CrdtError::WrongEpoch { expected, found } => {
                write!(f, "wrong membership epoch: expected {expected}, found {found}")
            }
            CrdtError::UnsupportedFormat { found } => {
                write!(f, "unsupported format version: {found:?}")
            }
            CrdtError::NonCanonical { reason } => {
                write!(f, "non-canonical encoding rejected: {reason}")
            }
            CrdtError::Invalid { what, reason } => write!(f, "invalid {what}: {reason}"),
            CrdtError::DuplicateDot { replica, counter } => {
                write!(f, "duplicate dot ({replica}, {counter})")
            }
            CrdtError::ConflictingEnvelope { origin, sequence } => write!(
                f,
                "conflicting envelope: identity ({origin}, {sequence}) already \
                 recorded with different canonical bytes"
            ),
            CrdtError::CorruptJournal { offset, reason } => {
                write!(f, "corrupt journal record at offset {offset}: {reason}")
            }
            CrdtError::Io(e) => write!(f, "i/o error: {e}"),
        }
    }
}

impl std::error::Error for CrdtError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CrdtError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for CrdtError {
    fn from(e: std::io::Error) -> Self {
        CrdtError::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_is_specific_per_variant() {
        let cases: Vec<(CrdtError, &str)> = vec![
            (
                CrdtError::Oversized { what: "envelope bytes", len: 9, max: 3 },
                "oversized envelope bytes",
            ),
            (
                CrdtError::WrongDataset {
                    expected: "https://a/".into(),
                    found: "https://b/".into(),
                },
                "wrong dataset",
            ),
            (CrdtError::WrongEpoch { expected: 1, found: 2 }, "wrong membership epoch"),
            (
                CrdtError::UnsupportedFormat { found: "sparq-crdt-delta/9".into() },
                "unsupported format",
            ),
            (CrdtError::NonCanonical { reason: "x".into() }, "non-canonical"),
            (CrdtError::Invalid { what: "quad", reason: "x".into() }, "invalid quad"),
            (CrdtError::DuplicateDot { replica: "cGVlcg".into(), counter: 3 }, "duplicate dot"),
            (
                CrdtError::ConflictingEnvelope { origin: "cGVlcg".into(), sequence: 4 },
                "conflicting envelope",
            ),
            (CrdtError::CorruptJournal { offset: 12, reason: "x".into() }, "corrupt journal"),
            (
                CrdtError::Io(std::io::Error::other("boom")),
                "i/o error",
            ),
        ];
        for (err, needle) in cases {
            let shown = format!("{err}");
            assert!(shown.contains(needle), "{shown:?} should contain {needle:?}");
        }
    }

    #[test]
    fn error_source_is_the_io_cause_only_for_io() {
        use std::error::Error as _;
        let io = CrdtError::Io(std::io::Error::other("boom"));
        assert!(io.source().is_some());
        let other = CrdtError::WrongEpoch { expected: 0, found: 1 };
        assert!(other.source().is_none());
    }
}
