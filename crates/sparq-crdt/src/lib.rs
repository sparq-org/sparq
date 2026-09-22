//! [FABLE-5] (epic `sq-tag1q`) SPARQL-CRDT support crate for sparq. It carries
//! two layers that landed as sibling beads of epic `sq-tag1q.7`:
//!
//! * **codec + journal** (`sq-tag1q.7.2`) — the bounded, versioned canonical
//!   delta envelope, causal summaries, the durable append/recovery journal,
//!   snapshots, missing-interval exchange primitives, and membership-epoch /
//!   causal-stability tracking from `research/sparql-crdt-gpt56-2026-07.md`
//!   (§4.1, §6, §8) and the proposal draft `site/specs/sparql-crdt.typ`;
//! * **executable formal model + convergence verification harness**
//!   (`sq-tag1q.7.4`) — a small, direct transcription of the proposal's merge
//!   algebra together with a bounded exhaustive model checker and a
//!   generated-schedule property harness.
//!
//! The two layers are deliberately independent: the model layer has **no**
//! runtime dependency on the codec layer, and vice versa. They also use
//! different identifier types — see *Two `Dot` types* below.
//!
//! # Layer 1 — interchange and durability
//!
//! Modules `codec`, `envelope`, `id`, `journal`, `quad`, `summary`, `sync`:
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
//! ## Canonical form
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
//! # Layer 2 — executable formal model
//!
//! Modules [`context`], [`state`], [`origin`], [`schedule`], [`checker`].
//!
//! The proposal specifies a replicated RDF dataset as an add-wins observed-remove
//! quad set: each concrete quad addition carries a globally unique *dot*
//! `(replica, counter)`; a removal carries exactly the dots observed at the
//! update's origin; and replicas merge a per-quad dot store plus a causal
//! context with an associative, commutative, idempotent join. This layer
//! transcribes that normative algebra — the *exact* join equations of
//! `CRDT-JOIN-1`, the clock-plus-cloud causal context of `CRDT-CTX-1`, the
//! primitive mutators of `CRDT-MUT-1..3`, and the evaluate-at-origin update
//! compilation of `CRDT-UPD-*` — into small, direct Rust so that it can be
//! model-checked and property-tested, and so the future production
//! implementation (the rest of epic `sq-tag1q.7`) can be differentially tested
//! against it. Each item's documentation cites the proposal identifiers it
//! implements.
//!
//! The model abstracts RDF terms as small opaque identifiers: `CRDT-DATA-1`
//! requires only *term equality* for quad identity, so nothing in the merge
//! algebra depends on term structure, lexical forms, or skolemisation
//! mechanics. Those belong to the production surface and its conformance
//! fixtures, not to this algebraic model.
//!
//! ## Two `Dot` types
//!
//! The two layers name their identifiers independently and **do not share
//! them**. [`Dot`] and [`ReplicaId`] at the crate root are the *wire* types of
//! [`id`] — a replica identifier is an opaque byte string, and construction is
//! fallible. The model's counterparts are deliberately smaller (a `u8` replica
//! index, an infallible `Copy` dot) because the bounded model only needs
//! equality and ordering; they are reached explicitly as [`context::Dot`] and
//! [`context::ReplicaId`] and are **not** re-exported at the crate root.
//! Reconciling the two is production work for the remaining `sq-tag1q.7`
//! beads, not something this crate asserts today.
//!
//! # What verification this crate provides — and what it does not
//!
//! Layer 1 claims byte formats and durability only. It makes **no**
//! convergence, atomic-visibility, or admission-policy claim about the full
//! design. Layer 2 supplies bounded evidence about the *model*, not about any
//! production implementation. Three distinct kinds of evidence must not be
//! conflated:
//!
//! 1. **Bounded, exhaustive model checking** ([`checker`]): every reachable
//!    configuration of a *bounded* multi-replica system — all interleavings of
//!    origin operations and delta deliveries — is explored; state invariants
//!    (`CRDT-STATE-1`) are checked at every configuration, and strong eventual
//!    consistency (`CRDT-SEC-2`: equal dot stores, equal causal-context
//!    denotations, equal visible quad sets) at every terminal configuration.
//!    The verdict holds **only within the explored bounds**.
//! 2. **Generated-schedule property tests** ([`schedule`], `tests/`): randomized
//!    schedules that permute, duplicate, batch, snapshot, compact, and replay
//!    deltas across replicas. These are *sampled evidence* and regression
//!    protection, not exhaustive.
//! 3. **A formal convergence argument**: the proposal's `CRDT-SEC-2` proof
//!    obligation (the dotted-set join is a join-semilattice least upper bound,
//!    hence order/grouping/duplication independent). **This crate does not
//!    provide that proof.** The semilattice argument in the proposal is an
//!    informative sketch; no proof is claimed until a mechanized or
//!    peer-reviewed proof artifact exists and has been independently reviewed.
//!
//! In short: this crate can *falsify* the design within its bounds and supply
//! strong empirical evidence, but a green run is not a convergence theorem.
//!
//! # Scope and dependencies
//!
//! The production dot-store implementation and the sparq-core materialisation
//! adapter are still sibling beads of epic `sq-tag1q.7` and are **not** in this
//! crate; it stays `publish = false` and claims no conformance class from the
//! proposal draft. Only existing, attested workspace dependencies are used
//! (`oxrdf` + `oxttl` for quad validation, `serde_json` as the strict JSON
//! reader, `sha2` for checksums/content hashes, and dev-only `tempfile` +
//! `proptest` for tests); this crate adds no new supply-chain surface.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;

pub mod checker;
pub mod codec;
pub mod context;
pub mod envelope;
pub mod id;
pub mod journal;
pub mod origin;
pub mod quad;
pub mod schedule;
pub mod state;
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

// [OPUS-5] Model layer (`sq-tag1q.7.4`). `context::Dot` / `context::ReplicaId` are
// intentionally NOT re-exported here: the crate-root `Dot` / `ReplicaId` are the wire
// types from `id`, and shadowing them with the model's smaller bounded-checking
// counterparts would make the two layers look interchangeable when they are not.
// See the "Two `Dot` types" section above.
pub use checker::{check_convergence, CheckReport, LawReport, Scenario};
pub use context::{CausalContext, Counter};
pub use origin::{Envelope, Op, Replica};
pub use schedule::{assert_converged, deliver_all, run_schedule, Step};
pub use state::{Delta, GraphKey, Quad, State};

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
