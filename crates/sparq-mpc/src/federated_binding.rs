// [OPUS-5] sq-34ml — the two NON-RESEARCH M4-v1 prerequisites split out of the
// sq-bjl Q1 SPIKE: (1) explicit freshness/replay binding for the OUT-OF-CIRCUIT
// issuer signature, and (2) the federated/multi-source `reconstruct_public_inputs`
// layout spanning all holders' commitments/rows/attribution.
//! Out-of-circuit **freshness/replay binding** + the **federated public-input
//! layout** for the M4-v1 verifier-side-attestation interim.
//!
//! ## What this module is (and emphatically is not)
//!
//! `research/mpc-m4-distributed-sig-feasibility.md` §2 confirms the honestly
//! *buildable* M4 v1 is the **verifier-side-attestation interim** — the Artemis
//! commit-and-prove anchor (signature verified OUTSIDE the SNARK, only the
//! commitment linked to the proof) combined with Dutta authenticated-input MPC —
//! and is a different thing from the genuinely unsolved Q1 join (verifying a
//! signature over a *secret-shared* witness in-circuit), which stays a DEFERRED,
//! audit-gated spike in [`crate::proof`].
//!
//! §4 item 4 of that record names two concrete prerequisites of the interim that
//! are *not* research. This module is exactly those two, and nothing else:
//!
//! 1. **Freshness/replay binding** ([`FreshnessBinding`], [`check_freshness`]).
//!    Because the interim checks the issuer signature out-of-circuit, ZK audit
//!    item **#4 is not automatic**: in-circuit the fresh challenge is a public
//!    input the proof commits to, but a signature checked *beside* the proof is
//!    perfectly replayable on its own. So the freshness inputs are made an
//!    EXPLICIT, domain-separated transcript ([`FreshnessBinding::tag`]) that the
//!    federated public-input vector carries as a field.
//! 2. **The federated public-input layout** ([`FederatedStatement`],
//!    [`FederatedStatement::reconstruct_public_inputs`]). The single-source
//!    verifier reconstructs bb's `public_inputs` byte vector from the DECLARED
//!    inputs and **byte-compares** it against the proof's (ZK audit #1). M4 v1
//!    needs the multi-source variant of that layout — spanning every holder's
//!    `commitments[]` / `rows[]` / `attribution[]` — reconstructed the same way
//!    and byte-compared unchanged.
//!
//! ### HARD GATE — this module proves nothing on its own
//!
//! Per the feasibility record §5.1 and [`crate::proof`], M4 is hard-gated on the
//! single-prover ZK-foundation remediation (#3 issuer signature + key-set
//! membership, #4 replay/freshness, #5/#6 FILTER binding, #8/#9 attribution +
//! salt separation, #12 revocation). **Nothing here closes any of those.** This
//! module is *scoping + out-of-circuit binding*: a deterministic byte layout and
//! a fail-closed transcript check. It becomes load-bearing only once a
//! collaborative proof actually emits these public inputs, at which point the
//! byte-comparison in [`FederatedStatement::bind_public_inputs`] is what ties the
//! proof to the reconstructed statement. Until then it is a data structure with
//! tests, and [`crate::proof::CollaborativeProof`] still returns
//! [`MpcError::NotYetImplemented`] — this module deliberately does not change
//! that.
//!
//! **No soundness or privacy property is asserted as achieved.** The ZK verifier
//! this layout targets is internally re-audited but NOT externally audited
//! (`sq-qhy4`), and the MPC substrate is honest-majority **semi-honest only**.
//! The interim additionally *gives up* source-unlinkability (the verifier sees
//! which issuer key signed each graph) and a single succinct signature↔witness
//! binding — both stated loudly in the feasibility record §2 and repeated here so
//! the deferral is auditable at the call site.
//!
//! ## Byte layout (the federated generalisation of ZK audit #1)
//!
//! Same word convention as the single-source reconstruction: every public input
//! is one **32-byte big-endian word**; field elements are the word verbatim,
//! `bool` is `{0,1}`, `u32`/`u64` is the integer value; arrays are flattened in
//! index order (row-major); no header, no trailing length.
//!
//! ```text
//! challenge                     -- field 0, the VERIFIER's fresh challenge N (#4)
//! query_digest                  -- canonical query-text digest (#10)
//! key_set_digest                -- digest of the disclosed issuer key-set K (#3)
//! freshness_tag                 -- FreshnessBinding::tag (#4, out-of-circuit)
//! holder_count                  -- u32
//! for each holder, in CANONICAL (HolderId-sorted) order:
//!     holder_tag                -- domain-separated digest of the HolderId
//!     k_i                       -- u32: this holder's commitment count
//!     r_i                       -- u32: this holder's DECLARED row capacity
//!     commitments[k_i]          -- field elements
//!     rows[r_i][3]              -- row-major, zero-padded to r_i
//!     row_count                 -- u32: actual disclosed rows (<= r_i)
//!     attribution[k_i]          -- bool -> {0,1}, one word per committed graph (#8)
//! ```
//!
//! ### Why the arity words are load-bearing
//!
//! The single-source layout can leave `k`/`r` implicit because they are pinned by
//! the declared `CircuitId` — a wrong `r` yields a wrong-length vector that cannot
//! byte-match. A *federated* vector has no such external authority: it is a
//! concatenation of N variable-arity segments, and a segment body
//! (`commitments[k] ‖ rows[r][3] ‖ row_count ‖ attribution[k]`) is `2k + 3r + 1`
//! words with NO marker at the internal boundaries. Different `(k, r)` pairs
//! therefore produce the same word count — `(k=6, r=0)` and `(k=3, r=2)` are both
//! 13 — and, for the right values, the same bytes. A prover could then present
//! three committed graphs as six, inventing attested sources: precisely the
//! attribution property #8 exists to protect. `holder_count` plus a per-segment
//! `k_i`/`r_i` makes the segmentation itself part of the transcript.
//! `federated_layout_binds_segmentation` in this module's tests exhibits the exact
//! colliding pair and pins that the arity words separate them.
//!
//! ## Field words
//!
//! [`FieldWord`] is an OPAQUE 32-byte big-endian word. This crate deliberately
//! keeps BN254/arkworks arithmetic out (`PLAN.md` "Poseidon2 / Schnorr
//! share-commitments" — wrong field, heavy dependency), so a `FieldWord` is
//! **not** range-checked against any curve modulus: canonicality is the ZK
//! layer's job, and this layer is a byte layout that is byte-compared. The words
//! this module *derives* itself (digests, [`VerifierChallenge::fresh`]) are
//! masked to 248 bits so they are unambiguously below the BN254 scalar modulus
//! (`≈ 2^254`) and can be used verbatim as field elements.

use std::collections::BTreeSet;

use sha2::{Digest, Sha512};

use crate::partial::{HolderId, MpcError};
use crate::rng::{MpcRng, SecureRng};

/// Domain tag for the out-of-circuit freshness transcript ([`FreshnessBinding::tag`]).
pub const FRESHNESS_DOMAIN_TAG: &[u8] = b"sparq-mpc/m4v1/freshness/v1\0";
/// Domain tag for the disclosed issuer key-set digest ([`key_set_digest`]).
pub const KEY_SET_DOMAIN_TAG: &[u8] = b"sparq-mpc/m4v1/key-set/v1\0";
/// Domain tag for the per-holder identity word ([`HolderSegment::holder_tag`]).
pub const HOLDER_DOMAIN_TAG: &[u8] = b"sparq-mpc/m4v1/holder-tag/v1\0";
/// Domain tag for the aggregate commitment digest ([`FederatedStatement::commitment_digest`]).
pub const COMMITMENT_DOMAIN_TAG: &[u8] = b"sparq-mpc/m4v1/commitments/v1\0";

/// Width of one public-input word, in bytes (bb's layout: one 32-byte big-endian
/// field element per public input).
pub const WORD_BYTES: usize = 32;

/// One public-input word: an opaque 32-byte **big-endian** value.
///
/// Opaque on purpose — see the module docs. This crate does not reduce, compare
/// or range-check against a curve modulus; it lays bytes out and byte-compares
/// them. Words derived *here* are masked to 248 bits (see [`FieldWord::digest`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct FieldWord([u8; WORD_BYTES]);

impl FieldWord {
    /// The all-zero word (the padding row slot, and an unset field).
    pub const ZERO: FieldWord = FieldWord([0u8; WORD_BYTES]);

    /// Wrap 32 big-endian bytes verbatim.
    pub fn from_be_bytes(bytes: [u8; WORD_BYTES]) -> Self {
        FieldWord(bytes)
    }

    /// The word's big-endian bytes.
    pub fn as_be_bytes(&self) -> &[u8; WORD_BYTES] {
        &self.0
    }

    /// Parse a `0x`-prefixed (or bare) 64-hex-digit big-endian word — the shape
    /// the ZK estate carries its field elements in. Fails closed on any length
    /// or digit error rather than padding or truncating.
    pub fn from_hex(s: &str) -> Result<Self, BindingError> {
        let body = s.strip_prefix("0x").unwrap_or(s);
        if body.len() != WORD_BYTES * 2 {
            return Err(BindingError::MalformedField {
                what: "field word must be exactly 64 hex digits",
            });
        }
        let mut out = [0u8; WORD_BYTES];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&body[i * 2..i * 2 + 2], 16).map_err(|_| {
                BindingError::MalformedField {
                    what: "field word contains a non-hex digit",
                }
            })?;
        }
        Ok(FieldWord(out))
    }

    /// Domain-separated digest of `parts` as a field word: the leading 32 bytes
    /// of `SHA-512(tag ‖ len(part) ‖ part …)`, with the top byte cleared.
    ///
    /// Two deliberate choices:
    /// - **Length-prefixed parts.** Each part is preceded by its length as an
    ///   8-byte big-endian word, so `["ab", "c"]` and `["a", "bc"]` are distinct
    ///   pre-images. Without it the digest would inherit the same concatenation
    ///   ambiguity the layout's arity words exist to remove.
    /// - **Top byte cleared** → a value `< 2^248`, hence unambiguously below the
    ///   BN254 scalar modulus (`≈ 2^254`), so the digest is usable verbatim as a
    ///   field element. The residual 248-bit output keeps a ~124-bit birthday
    ///   margin; this is a *canonicality* measure, not a security claim.
    pub fn digest(tag: &[u8], parts: &[&[u8]]) -> Self {
        let mut hasher = Sha512::new();
        hasher.update(tag);
        for part in parts {
            hasher.update((part.len() as u64).to_be_bytes());
            hasher.update(part);
        }
        let digest = hasher.finalize();
        let mut out = [0u8; WORD_BYTES];
        out.copy_from_slice(&digest[..WORD_BYTES]);
        out[0] = 0; // < 2^248 < BN254 modulus.
        FieldWord(out)
    }
}

/// A fail-closed binding failure. Distinct from [`MpcError::NotYetImplemented`]
/// (a deferred capability): every variant here is a REFUSAL of a statement or
/// proof the verifier was actually able to check and found wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingError {
    /// A field word could not be parsed. `what` names the defect.
    MalformedField { what: &'static str },
    /// A federation with no holders — there is nothing to bind.
    EmptyFederation,
    /// A holder segment carries no commitments, so it attests to no graph.
    NoCommitments { holder: HolderId },
    /// `attribution.len() != commitments.len()` — the per-graph attribution bits
    /// (#8) must be exactly one per committed graph, or the layout would admit a
    /// pad-to-false bypass.
    AttributionArity {
        holder: HolderId,
        commitments: usize,
        attribution: usize,
    },
    /// More disclosed rows than the holder's declared row capacity `r_i`.
    RowOverflow {
        holder: HolderId,
        rows: usize,
        capacity: u32,
    },
    /// The same [`HolderId`] appears twice — the canonical order would be
    /// ambiguous and the attribution ambiguous with it.
    DuplicateHolder { holder: HolderId },
    /// The statement's challenge is not the challenge THIS verifier issued. The
    /// verifier always checks against its own fresh value, never the prover's.
    ChallengeMismatch,
    /// The statement is outside its validity window at the supplied instant.
    OutsideValidityWindow { now: u64, not_before: u64, not_after: u64 },
    /// `not_after < not_before` — an unsatisfiable window is refused at
    /// construction rather than silently never matching.
    InvertedValidityWindow { not_before: u64, not_after: u64 },
    /// This challenge has already been burnt: a replayed transcript.
    ChallengeReplayed,
    /// The proof's `public_inputs` vector has a different length from the
    /// reconstruction — a wrong arity cannot byte-match.
    PublicInputLength { expected: usize, actual: usize },
    /// The proof's `public_inputs` differ from the reconstruction at word
    /// `word_index` (0-based, in the layout documented on this module).
    PublicInputMismatch { word_index: usize },
}

impl std::fmt::Display for BindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindingError::MalformedField { what } => write!(f, "malformed field word: {what}"),
            BindingError::EmptyFederation => write!(f, "federation has no holders"),
            BindingError::NoCommitments { holder } => {
                write!(f, "holder {holder} contributed no commitments")
            }
            BindingError::AttributionArity {
                holder,
                commitments,
                attribution,
            } => write!(
                f,
                "holder {holder}: attribution has {attribution} bits for {commitments} commitments (must be equal)"
            ),
            BindingError::RowOverflow {
                holder,
                rows,
                capacity,
            } => write!(
                f,
                "holder {holder}: {rows} disclosed rows exceed the declared capacity {capacity}"
            ),
            BindingError::DuplicateHolder { holder } => {
                write!(f, "holder {holder} appears more than once")
            }
            BindingError::ChallengeMismatch => {
                write!(f, "statement challenge is not the verifier's own challenge")
            }
            BindingError::OutsideValidityWindow {
                now,
                not_before,
                not_after,
            } => write!(
                f,
                "instant {now} is outside the validity window [{not_before}, {not_after}]"
            ),
            BindingError::InvertedValidityWindow {
                not_before,
                not_after,
            } => write!(f, "inverted validity window [{not_before}, {not_after}]"),
            BindingError::ChallengeReplayed => write!(f, "challenge already burnt (replay)"),
            BindingError::PublicInputLength { expected, actual } => write!(
                f,
                "public_inputs length {actual} != reconstructed length {expected}"
            ),
            BindingError::PublicInputMismatch { word_index } => {
                write!(f, "public_inputs differ from the reconstruction at word {word_index}")
            }
        }
    }
}

impl std::error::Error for BindingError {}

impl From<BindingError> for MpcError {
    /// A binding failure is a REAL, checkable protocol refusal, so it maps onto
    /// [`MpcError::Protocol`] — never onto [`MpcError::NotYetImplemented`], which
    /// is reserved for genuinely deferred capabilities.
    fn from(e: BindingError) -> MpcError {
        MpcError::Protocol(e.to_string())
    }
}

/// The verifier's fresh challenge `N` (ZK audit #4). Public-input field 0.
///
/// The verifier generates this itself and checks the statement against its OWN
/// value; a prover-supplied challenge is only ever a claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VerifierChallenge(FieldWord);

impl VerifierChallenge {
    /// Draw a fresh challenge from the crate's OS-seeded ChaCha20 CSPRNG
    /// ([`SecureRng`]). 248 bits of entropy (the top byte is cleared so the word
    /// is unambiguously a canonical BN254 field element — see the module docs).
    pub fn fresh() -> Self {
        Self::fresh_from(&mut SecureRng::from_os())
    }

    /// Draw a fresh challenge from a caller-supplied [`MpcRng`]. Exists so a test
    /// or a benchmark harness can pin the value; the protocol path is
    /// [`VerifierChallenge::fresh`].
    pub fn fresh_from<R: MpcRng>(rng: &mut R) -> Self {
        let mut bytes = [0u8; WORD_BYTES];
        for chunk in bytes.chunks_mut(8) {
            chunk.copy_from_slice(&rng.next_u64().to_be_bytes());
        }
        bytes[0] = 0;
        VerifierChallenge(FieldWord(bytes))
    }

    /// Wrap an existing word (e.g. one carried in a received statement).
    pub fn from_word(word: FieldWord) -> Self {
        VerifierChallenge(word)
    }

    /// The challenge as a public-input word.
    pub fn as_word(&self) -> FieldWord {
        self.0
    }
}

/// A closed validity window `[not_before, not_after]` in whatever integer time
/// base the deployment uses (typically Unix seconds).
///
/// The crate takes **no clock dependency**: the instant is always supplied by the
/// caller ([`ValidityWindow::contains`], [`check_freshness`]), so the check is
/// deterministic and testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidityWindow {
    not_before: u64,
    not_after: u64,
}

impl ValidityWindow {
    /// Build a window, refusing an inverted one up front.
    pub fn new(not_before: u64, not_after: u64) -> Result<Self, BindingError> {
        if not_after < not_before {
            return Err(BindingError::InvertedValidityWindow {
                not_before,
                not_after,
            });
        }
        Ok(ValidityWindow {
            not_before,
            not_after,
        })
    }

    /// Inclusive lower bound.
    pub fn not_before(&self) -> u64 {
        self.not_before
    }

    /// Inclusive upper bound.
    pub fn not_after(&self) -> u64 {
        self.not_after
    }

    /// Whether `now` lies in the (inclusive) window.
    pub fn contains(&self, now: u64) -> bool {
        now >= self.not_before && now <= self.not_after
    }
}

/// The explicit freshness/replay transcript for the M4-v1 interim (ZK audit #4).
///
/// In-circuit, freshness rides along for free: the challenge is a public input
/// the proof commits to. The interim checks the issuer signature **outside** the
/// circuit over an already-public commitment, and a detached signature over a
/// static commitment is replayable forever. So the interim must bind, explicitly:
/// the verifier's fresh challenge, *what* was asked (the query digest), *whose*
/// keys were accepted (the key-set digest), *which* commitments the out-of-circuit
/// signatures covered, and *when* the attestation is valid.
///
/// [`FreshnessBinding::tag`] folds those into one domain-separated word that the
/// federated public-input vector carries as a field — so once the collaborative
/// proof exists, the freshness inputs are inside the byte-compared transcript
/// rather than beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshnessBinding {
    /// The verifier's fresh challenge `N`.
    pub challenge: VerifierChallenge,
    /// Canonical digest of the normalized query text (#10 query-text binding).
    pub query_digest: FieldWord,
    /// Digest of the disclosed issuer key-set `K` (#3). Build it with
    /// [`key_set_digest`] so prover and verifier agree on the canonical form.
    pub key_set_digest: FieldWord,
    /// Digest over every holder's commitments in canonical order — the subject
    /// of the out-of-circuit signatures. Supplied by
    /// [`FederatedStatement::commitment_digest`].
    pub commitment_digest: FieldWord,
    /// When the attestation is valid.
    pub window: ValidityWindow,
}

impl FreshnessBinding {
    /// The domain-separated freshness word carried in the public inputs.
    pub fn tag(&self) -> FieldWord {
        FieldWord::digest(
            FRESHNESS_DOMAIN_TAG,
            &[
                self.challenge.as_word().as_be_bytes(),
                self.query_digest.as_be_bytes(),
                self.key_set_digest.as_be_bytes(),
                self.commitment_digest.as_be_bytes(),
                &self.window.not_before().to_be_bytes(),
                &self.window.not_after().to_be_bytes(),
            ],
        )
    }
}

/// Canonical digest of the disclosed issuer key-set `K` (#3).
///
/// The key identifiers are **sorted and de-duplicated** first, so a prover cannot
/// change the digest (hence the freshness tag, hence the public inputs) by
/// reordering or repeating a key it was going to disclose anyway.
pub fn key_set_digest(keys: &[String]) -> FieldWord {
    let canonical: BTreeSet<&str> = keys.iter().map(String::as_str).collect();
    let parts: Vec<&[u8]> = canonical.iter().map(|k| k.as_bytes()).collect();
    FieldWord::digest(KEY_SET_DOMAIN_TAG, &parts)
}

/// A record of challenges already spent, so a transcript cannot be replayed.
///
/// Deliberately a trait: a real deployment burns nonces in shared, durable
/// storage, and the in-process [`InMemorySeenChallenges`] is only the reference
/// implementation for tests and single-verifier runs.
pub trait SeenChallenges {
    /// Burn `challenge`, returning [`BindingError::ChallengeReplayed`] if it had
    /// already been spent. Implementations MUST be atomic with respect to
    /// concurrent verifiers, or the replay window reopens.
    fn burn(&mut self, challenge: &VerifierChallenge) -> Result<(), BindingError>;
}

/// In-process reference [`SeenChallenges`]. Not durable and not shared across
/// verifier processes — adequate for a single in-process federation run and for
/// tests, not for a deployment.
#[derive(Debug, Default, Clone)]
pub struct InMemorySeenChallenges {
    seen: BTreeSet<FieldWord>,
}

impl InMemorySeenChallenges {
    /// An empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many challenges have been burnt.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Whether no challenge has been burnt yet.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

impl SeenChallenges for InMemorySeenChallenges {
    fn burn(&mut self, challenge: &VerifierChallenge) -> Result<(), BindingError> {
        if self.seen.insert(challenge.as_word()) {
            Ok(())
        } else {
            Err(BindingError::ChallengeReplayed)
        }
    }
}

/// One holder's segment of the federated public-input vector: its committed
/// graphs, its disclosed rows, and its per-graph attribution bits (#8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolderSegment {
    holder: HolderId,
    commitments: Vec<FieldWord>,
    rows: Vec<[FieldWord; 3]>,
    row_capacity: u32,
    attribution: Vec<bool>,
}

impl HolderSegment {
    /// Build a segment, refusing every shape the layout cannot bind.
    ///
    /// - `commitments` — this holder's `C(G_i)`, one per committed graph; must be
    ///   non-empty (a holder attesting to no graph contributes nothing).
    /// - `rows` — the disclosed `[s, p, o]` encodings, at most `row_capacity`.
    /// - `row_capacity` — the DECLARED per-holder row capacity `r_i`; the segment
    ///   is zero-padded up to it, mirroring the single-source `pad_rows`.
    /// - `attribution` — exactly one bit per commitment (#8). A short vector is
    ///   REFUSED rather than padded with `false`: padding would silently widen the
    ///   accepted shape.
    pub fn new(
        holder: HolderId,
        commitments: Vec<FieldWord>,
        rows: Vec<[FieldWord; 3]>,
        row_capacity: u32,
        attribution: Vec<bool>,
    ) -> Result<Self, BindingError> {
        if commitments.is_empty() {
            return Err(BindingError::NoCommitments { holder });
        }
        if attribution.len() != commitments.len() {
            return Err(BindingError::AttributionArity {
                holder,
                commitments: commitments.len(),
                attribution: attribution.len(),
            });
        }
        if rows.len() > row_capacity as usize {
            return Err(BindingError::RowOverflow {
                holder,
                rows: rows.len(),
                capacity: row_capacity,
            });
        }
        Ok(HolderSegment {
            holder,
            commitments,
            rows,
            row_capacity,
            attribution,
        })
    }

    /// The holder this segment belongs to.
    pub fn holder(&self) -> &HolderId {
        &self.holder
    }

    /// This holder's committed-graph commitments (`k_i` of them).
    pub fn commitments(&self) -> &[FieldWord] {
        &self.commitments
    }

    /// This holder's disclosed rows (`row_count` of them, before padding).
    pub fn rows(&self) -> &[[FieldWord; 3]] {
        &self.rows
    }

    /// The declared row capacity `r_i` the segment pads to.
    pub fn row_capacity(&self) -> u32 {
        self.row_capacity
    }

    /// The per-graph attribution bits (#8), exactly one per commitment.
    pub fn attribution(&self) -> &[bool] {
        &self.attribution
    }

    /// The domain-separated identity word for this holder. Binds *which* source a
    /// segment belongs to into the transcript, so re-labelling a segment changes
    /// the bytes.
    pub fn holder_tag(&self) -> FieldWord {
        FieldWord::digest(HOLDER_DOMAIN_TAG, &[self.holder.as_str().as_bytes()])
    }
}

/// The federated (multi-source) public statement: everything the relying party
/// reconstructs and byte-compares the M4-v1 proof's `public_inputs` against.
///
/// This is the multi-source generalisation of the single-source reconstruction
/// (ZK audit #1). It is a *statement shape plus a serialisation* — see the module
/// docs for what it does and does not establish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederatedStatement {
    challenge: VerifierChallenge,
    query_digest: FieldWord,
    key_set_digest: FieldWord,
    window: ValidityWindow,
    holders: Vec<HolderSegment>,
}

impl FederatedStatement {
    /// Build a statement, canonicalising the holder order and refusing an empty
    /// federation or a duplicated holder.
    ///
    /// Holders are sorted by [`HolderId`] so prover and verifier serialise the
    /// same bytes without agreeing on a wire order out of band. A duplicate is an
    /// error, not a merge: two segments for one holder would make the per-graph
    /// attribution ambiguous.
    pub fn new(
        challenge: VerifierChallenge,
        query_digest: FieldWord,
        key_set_digest: FieldWord,
        window: ValidityWindow,
        mut holders: Vec<HolderSegment>,
    ) -> Result<Self, BindingError> {
        if holders.is_empty() {
            return Err(BindingError::EmptyFederation);
        }
        holders.sort_by(|a, b| a.holder.cmp(&b.holder));
        if let Some(dup) = holders.windows(2).find(|w| w[0].holder == w[1].holder) {
            return Err(BindingError::DuplicateHolder {
                holder: dup[0].holder.clone(),
            });
        }
        Ok(FederatedStatement {
            challenge,
            query_digest,
            key_set_digest,
            window,
            holders,
        })
    }

    /// The verifier's fresh challenge `N`.
    pub fn challenge(&self) -> VerifierChallenge {
        self.challenge
    }

    /// The canonical query digest (#10).
    pub fn query_digest(&self) -> FieldWord {
        self.query_digest
    }

    /// The disclosed issuer key-set digest (#3).
    pub fn key_set_digest(&self) -> FieldWord {
        self.key_set_digest
    }

    /// The attestation's validity window.
    pub fn window(&self) -> ValidityWindow {
        self.window
    }

    /// The holder segments, in canonical (sorted) order.
    pub fn holders(&self) -> &[HolderSegment] {
        &self.holders
    }

    /// Digest over every holder's commitments in canonical order — the subject of
    /// the out-of-circuit issuer signatures, and an input to the freshness tag.
    ///
    /// The pre-image carries the same **arity words** as
    /// [`FederatedStatement::reconstruct_public_inputs`]: `holder_count`, then per
    /// holder `holder_tag ‖ k_i ‖ commitments[k_i]`. A holder tag prefix ALONE
    /// would not bind the partition, because [`FieldWord`] is opaque and accepts
    /// arbitrary bytes: a prover may set a commitment equal to the (publicly
    /// computable) `holder_tag` of another holder, so one holder `A` committing
    /// `[x, holder_tag(B), y]` flattens to exactly the part sequence of the two
    /// holders `A:[x]`, `B:[y]`. With `holder_count` and each `k_i` present the
    /// segmentation is length-determined and that re-partition is a different
    /// pre-image — no SHA-512 property is being leaned on to separate them.
    /// `commitment_digest_binds_holder_arities` pins the exact collision.
    pub fn commitment_digest(&self) -> FieldWord {
        let mut parts: Vec<[u8; WORD_BYTES]> = Vec::new();
        parts.push(*uint_word(self.holders.len() as u64).as_be_bytes());
        for seg in &self.holders {
            parts.push(*seg.holder_tag().as_be_bytes());
            parts.push(*uint_word(seg.commitments.len() as u64).as_be_bytes());
            for c in &seg.commitments {
                parts.push(*c.as_be_bytes());
            }
        }
        let refs: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
        FieldWord::digest(COMMITMENT_DOMAIN_TAG, &refs)
    }

    /// The out-of-circuit freshness binding for this statement (#4).
    pub fn freshness_binding(&self) -> FreshnessBinding {
        FreshnessBinding {
            challenge: self.challenge,
            query_digest: self.query_digest,
            key_set_digest: self.key_set_digest,
            commitment_digest: self.commitment_digest(),
            window: self.window,
        }
    }

    /// Number of 32-byte words the reconstruction will emit.
    pub fn public_input_word_count(&self) -> usize {
        // challenge, query_digest, key_set_digest, freshness_tag, holder_count.
        let mut words = 5;
        for seg in &self.holders {
            // holder_tag, k_i, r_i, row_count.
            words += 4;
            words += seg.commitments.len();
            words += seg.row_capacity as usize * 3;
            words += seg.attribution.len();
        }
        words
    }

    /// Reconstruct the federated `public_inputs` byte vector — the multi-source
    /// generalisation of ZK audit #1.
    ///
    /// Deterministic: the same statement always serialises to the same bytes, in
    /// the layout documented on this module. Every value is one 32-byte
    /// big-endian word; rows are zero-padded to each holder's declared capacity.
    pub fn reconstruct_public_inputs(&self) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::with_capacity(self.public_input_word_count() * WORD_BYTES);
        let mut push_word = |w: &FieldWord| out.extend_from_slice(w.as_be_bytes());

        push_word(&self.challenge.as_word());
        push_word(&self.query_digest);
        push_word(&self.key_set_digest);
        push_word(&self.freshness_binding().tag());
        push_word(&uint_word(self.holders.len() as u64));

        for seg in &self.holders {
            push_word(&seg.holder_tag());
            push_word(&uint_word(seg.commitments.len() as u64));
            push_word(&uint_word(u64::from(seg.row_capacity)));
            for c in &seg.commitments {
                push_word(c);
            }
            for j in 0..seg.row_capacity as usize {
                match seg.rows.get(j) {
                    Some(row) => {
                        for slot in row {
                            push_word(slot);
                        }
                    }
                    // Padding row: three zero words (matches the prover's pad_rows).
                    None => {
                        for _ in 0..3 {
                            push_word(&FieldWord::ZERO);
                        }
                    }
                }
            }
            push_word(&uint_word(seg.rows.len() as u64));
            for bit in &seg.attribution {
                push_word(&uint_word(u64::from(*bit)));
            }
        }
        out
    }

    /// Byte-compare a proof's `public_inputs` against the reconstruction,
    /// reporting the first differing word.
    ///
    /// This is the operation that will make the layout load-bearing — once a
    /// collaborative proof exists to supply `proof_public_inputs`. It is a plain
    /// `==` because every word here is a PUBLIC input; the crate's constant-time
    /// primitives are reserved for secret-bearing comparisons (none exist on this
    /// path).
    pub fn bind_public_inputs(&self, proof_public_inputs: &[u8]) -> Result<(), BindingError> {
        let expected = self.reconstruct_public_inputs();
        if expected.len() != proof_public_inputs.len() {
            return Err(BindingError::PublicInputLength {
                expected: expected.len(),
                actual: proof_public_inputs.len(),
            });
        }
        for (i, (a, b)) in expected
            .chunks_exact(WORD_BYTES)
            .zip(proof_public_inputs.chunks_exact(WORD_BYTES))
            .enumerate()
        {
            if a != b {
                return Err(BindingError::PublicInputMismatch { word_index: i });
            }
        }
        Ok(())
    }
}

/// Encode a small integer (`u32`/`u64`/`bool`) as a 32-byte big-endian word.
fn uint_word(v: u64) -> FieldWord {
    let mut word = [0u8; WORD_BYTES];
    word[WORD_BYTES - 8..].copy_from_slice(&v.to_be_bytes());
    FieldWord(word)
}

/// The out-of-circuit freshness/replay gate (ZK audit #4) for the M4-v1 interim.
///
/// Checks, **in this order**:
/// 1. the statement carries the challenge THIS verifier issued
///    ([`BindingError::ChallengeMismatch`]);
/// 2. `now` lies in the statement's validity window
///    ([`BindingError::OutsideValidityWindow`]);
/// 3. only then, the challenge is burnt in `seen`
///    ([`BindingError::ChallengeReplayed`]).
///
/// The order is load-bearing: burning last means a mismatched or expired
/// statement does NOT consume the verifier's challenge, so an attacker cannot
/// grief a pending honest transcript by racing a bad one at it. (The same
/// "nothing before the nonce is burnt" discipline the single-source verifier
/// applies to its structural gates.) `expect_freshness_gate_orders_checks` in
/// this module's tests pins it.
///
/// On success returns the freshness tag now bound into the public inputs. This
/// gate is necessary for the interim, **not sufficient for anything on its own**:
/// it establishes only that this statement is fresh for this verifier. What the
/// statement *says* is true is the collaborative proof's job, and that proof is
/// still [`MpcError::NotYetImplemented`] pending the ZK-foundation gate.
pub fn check_freshness(
    statement: &FederatedStatement,
    expected: &VerifierChallenge,
    now: u64,
    seen: &mut dyn SeenChallenges,
) -> Result<FieldWord, BindingError> {
    if statement.challenge() != *expected {
        return Err(BindingError::ChallengeMismatch);
    }
    let window = statement.window();
    if !window.contains(now) {
        return Err(BindingError::OutsideValidityWindow {
            now,
            not_before: window.not_before(),
            not_after: window.not_after(),
        });
    }
    seen.burn(expected)?;
    Ok(statement.freshness_binding().tag())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(byte: u8) -> FieldWord {
        let mut w = [0u8; WORD_BYTES];
        w[WORD_BYTES - 1] = byte;
        FieldWord::from_be_bytes(w)
    }

    fn segment(id: &str, commitments: usize, rows: usize, capacity: u32) -> HolderSegment {
        HolderSegment::new(
            HolderId::new(id),
            (0..commitments).map(|i| word(i as u8 + 1)).collect(),
            (0..rows)
                .map(|i| [word(i as u8 + 10), word(i as u8 + 11), word(i as u8 + 12)])
                .collect(),
            capacity,
            vec![true; commitments],
        )
        .expect("valid segment")
    }

    fn statement(segments: Vec<HolderSegment>) -> FederatedStatement {
        FederatedStatement::new(
            VerifierChallenge::from_word(word(0xAB)),
            word(0xC1),
            key_set_digest(&["did:example:dmv#key-1".to_string()]),
            ValidityWindow::new(100, 200).unwrap(),
            segments,
        )
        .expect("valid statement")
    }

    // ---- layout ----------------------------------------------------------

    /// The reconstruction is deterministic and exactly `public_input_word_count`
    /// words wide, in the documented layout.
    #[test]
    fn federated_layout_is_deterministic_and_sized() {
        let s = statement(vec![segment("alice", 2, 1, 3), segment("bob", 1, 2, 2)]);
        let bytes = s.reconstruct_public_inputs();
        assert_eq!(bytes.len() % WORD_BYTES, 0);
        assert_eq!(bytes.len() / WORD_BYTES, s.public_input_word_count());
        // 5 header + alice(4 + 2 + 3*3 + 2) + bob(4 + 1 + 2*3 + 1) = 5 + 17 + 12.
        assert_eq!(s.public_input_word_count(), 34);
        assert_eq!(bytes, s.reconstruct_public_inputs());
        s.bind_public_inputs(&bytes).expect("self-binds");
    }

    /// Holder order is canonicalised, so the two provers' wire orders agree.
    #[test]
    fn holder_order_is_canonical() {
        let a = statement(vec![segment("alice", 1, 1, 2), segment("bob", 1, 1, 2)]);
        let b = statement(vec![segment("bob", 1, 1, 2), segment("alice", 1, 1, 2)]);
        assert_eq!(a.reconstruct_public_inputs(), b.reconstruct_public_inputs());
        assert_eq!(a.holders()[0].holder().as_str(), "alice");
    }

    /// THE load-bearing federated invariant, pinned by an EXACT collision: these
    /// two statements re-partition the *same word sequence* into different
    /// (commitments, rows, attribution) arities, and without the per-segment
    /// `k_i` / `r_i` words they serialise to byte-identical vectors.
    ///
    /// Alice-6: `k=6, r=0` → `tag, C1..C6, row_count=0, [f,f,t,A,B,C]`
    /// Alice-3: `k=3, r=2` → `tag, C1..C3, [C4,C5,C6], [0,0,0], row_count=1, [A,B,C]`
    ///
    /// Both segments are the same width and, word for word, the same values:
    /// Alice-6's `row_count=0` plus its first two `false` bits line up with
    /// Alice-3's zero-padding row, and Alice-6's third `true` bit lines up with
    /// Alice-3's `row_count=1`. So a prover could present three committed graphs
    /// as six — inventing attested sources (#8) — and still byte-match.
    ///
    /// Two INDEPENDENT defences stop it, and this test isolates each:
    /// - the per-segment `k_i` / `r_i` arity words, which disambiguate the
    ///   segment layout itself (checked below with the *derived* freshness word
    ///   neutralised, so only the layout is under test — delete `k_i`/`r_i` from
    ///   [`FederatedStatement::reconstruct_public_inputs`] and that half goes red);
    /// - the freshness tag, which digests
    ///   [`FederatedStatement::commitment_digest`] and so covers the partition
    ///   too — it is word 3, the FIRST mismatch on the full vector.
    #[test]
    fn federated_layout_binds_segmentation() {
        let c = |i: u8| word(0x40 + i);
        let alice6 = HolderSegment::new(
            HolderId::new("alice"),
            (1..=6).map(c).collect(),
            vec![],
            0,
            vec![false, false, true, true, true, true],
        )
        .unwrap();
        let alice3 = HolderSegment::new(
            HolderId::new("alice"),
            (1..=3).map(c).collect(),
            vec![[c(4), c(5), c(6)]],
            2,
            vec![true, true, true],
        )
        .unwrap();

        let six = statement(vec![alice6]);
        let three = statement(vec![alice3]);

        // The collision is real: identical width, and — with the arity words and
        // the DERIVED freshness word removed — byte-identical. The layout is
        // words `[challenge, query_digest, key_set_digest, freshness_tag,
        // holder_count, holder_tag, k_i, r_i, ...]`, so dropping indices 6..8
        // removes the arity and index 3 removes the freshness tag.
        assert_eq!(six.public_input_word_count(), three.public_input_word_count());
        let strip_arity_and_freshness = |bytes: Vec<u8>| {
            let mut words: Vec<Vec<u8>> =
                bytes.chunks_exact(WORD_BYTES).map(<[u8]>::to_vec).collect();
            words.drain(6..8); // k_i, r_i of the single segment
            words.remove(3); // the derived freshness tag
            words.concat()
        };
        assert_eq!(
            strip_arity_and_freshness(six.reconstruct_public_inputs()),
            strip_arity_and_freshness(three.reconstruct_public_inputs()),
            "the raw segment layout MUST actually collide, or this test proves nothing"
        );

        // Defence 1 — the arity words alone disambiguate the segment layout, with
        // the freshness tag still neutralised.
        let strip_freshness = |bytes: Vec<u8>| {
            let mut words: Vec<Vec<u8>> =
                bytes.chunks_exact(WORD_BYTES).map(<[u8]>::to_vec).collect();
            words.remove(3);
            words.concat()
        };
        assert_ne!(
            strip_freshness(six.reconstruct_public_inputs()),
            strip_freshness(three.reconstruct_public_inputs()),
            "k_i / r_i must make the segmentation part of the transcript"
        );

        // Defence 2 — on the full vector the freshness tag (word 3) catches it first.
        assert_eq!(
            six.bind_public_inputs(&three.reconstruct_public_inputs()),
            Err(BindingError::PublicInputMismatch { word_index: 3 })
        );
    }

    /// Moving a commitment from one holder to another changes the bytes AND the
    /// commitment digest — a graph cannot be re-attributed to a different source.
    #[test]
    fn commitments_are_bound_to_their_holder() {
        let split_a = statement(vec![segment("alice", 2, 0, 0), segment("bob", 1, 0, 0)]);
        let split_b = statement(vec![segment("alice", 1, 0, 0), segment("bob", 2, 0, 0)]);
        assert_ne!(
            split_a.reconstruct_public_inputs(),
            split_b.reconstruct_public_inputs()
        );
        assert_ne!(split_a.commitment_digest(), split_b.commitment_digest());
        let err = split_a
            .bind_public_inputs(&split_b.reconstruct_public_inputs())
            .unwrap_err();
        assert!(matches!(
            err,
            BindingError::PublicInputMismatch { .. } | BindingError::PublicInputLength { .. }
        ));
    }

    /// The commitment digest binds the holder/commitment ARITIES, not just the
    /// holder tags — pinned by the exact collision the arity words remove.
    ///
    /// [`FieldWord`] is opaque and accepts arbitrary bytes, and a holder tag is
    /// publicly computable from the [`HolderId`], so a prover can plant
    /// `holder_tag("bob")` as one of alice's commitments. Under a pre-image of
    /// bare `holder_tag ‖ commitments…` parts, alice committing `[x, tag(bob), y]`
    /// is then byte-identical to the two holders `alice:[x]`, `bob:[y]` — no
    /// SHA-512 collision required. That would falsify the digest's re-attribution
    /// claim and leave [`FreshnessBinding::tag`] (which digests this word, and is
    /// the subject of the out-of-circuit issuer signatures) blind to the claimed
    /// holder partition.
    ///
    /// The first assertion reconstructs the OLD flattened pre-image and shows it
    /// really did collide, so the assertions after it are not vacuous. Delete
    /// either arity word from [`FederatedStatement::commitment_digest`] and this
    /// test goes red.
    #[test]
    fn commitment_digest_binds_holder_arities() {
        let x = word(0x11);
        let y = word(0x22);
        // Publicly computable: exactly what HolderSegment::holder_tag derives.
        let bob_tag = FieldWord::digest(HOLDER_DOMAIN_TAG, &[b"bob"]);
        assert_eq!(
            bob_tag,
            segment("bob", 1, 0, 0).holder_tag(),
            "the planted commitment must be a real holder tag"
        );

        let merged = statement(vec![HolderSegment::new(
            HolderId::new("alice"),
            vec![x, bob_tag, y],
            vec![],
            0,
            vec![true; 3],
        )
        .unwrap()]);
        let split = statement(vec![
            HolderSegment::new(HolderId::new("alice"), vec![x], vec![], 0, vec![true]).unwrap(),
            HolderSegment::new(HolderId::new("bob"), vec![y], vec![], 0, vec![true]).unwrap(),
        ]);

        // NON-VACUITY: the pre-image WITHOUT the arity words is identical.
        let flattened = |s: &FederatedStatement| {
            let mut parts: Vec<[u8; WORD_BYTES]> = Vec::new();
            for seg in s.holders() {
                parts.push(*seg.holder_tag().as_be_bytes());
                for c in seg.commitments() {
                    parts.push(*c.as_be_bytes());
                }
            }
            parts
        };
        assert_eq!(
            flattened(&merged),
            flattened(&split),
            "the flattened pre-images MUST actually collide, or this test proves nothing"
        );

        // With holder_count and each k_i in the pre-image, they separate.
        assert_ne!(
            merged.commitment_digest(),
            split.commitment_digest(),
            "holder_count / k_i must make the holder partition part of the digest"
        );
        // And so does the freshness tag that digests it.
        assert_ne!(
            merged.freshness_binding().tag(),
            split.freshness_binding().tag()
        );
    }

    /// Merging two holders into one segment carrying both their commitments is a
    /// different transcript, even though the flattened commitment list matches.
    #[test]
    fn merging_holders_changes_the_transcript() {
        let split = statement(vec![segment("alice", 1, 0, 0), segment("bob", 1, 0, 0)]);
        let merged = statement(vec![segment("alice", 2, 0, 0)]);
        assert_ne!(
            split.reconstruct_public_inputs(),
            merged.reconstruct_public_inputs()
        );
    }

    /// Rows are zero-padded to the DECLARED capacity, and `row_count` records the
    /// real count — so a short row set cannot masquerade as a full one.
    #[test]
    fn rows_pad_to_capacity_and_row_count_is_bound() {
        let padded = statement(vec![segment("alice", 1, 1, 3)]);
        let full = statement(vec![segment("alice", 1, 3, 3)]);
        assert_eq!(
            padded.public_input_word_count(),
            full.public_input_word_count(),
            "padding makes both statements the same width"
        );
        assert_ne!(
            padded.reconstruct_public_inputs(),
            full.reconstruct_public_inputs(),
            "but the row_count and the row slots differ"
        );
    }

    /// A flipped attribution bit changes the bytes (#8 is proof-bound, not advisory).
    #[test]
    fn attribution_bit_is_bound() {
        let base = segment("alice", 2, 1, 2);
        let flipped = HolderSegment::new(
            HolderId::new("alice"),
            base.commitments().to_vec(),
            base.rows().to_vec(),
            base.row_capacity(),
            vec![true, false],
        )
        .unwrap();
        assert_ne!(
            statement(vec![base]).reconstruct_public_inputs(),
            statement(vec![flipped]).reconstruct_public_inputs()
        );
    }

    /// `bind_public_inputs` reports the FIRST differing word, and refuses a
    /// wrong-length vector before comparing.
    #[test]
    fn bind_public_inputs_fails_closed() {
        let s = statement(vec![segment("alice", 1, 1, 1)]);
        let good = s.reconstruct_public_inputs();

        let mut tampered = good.clone();
        tampered[WORD_BYTES] ^= 0x01; // word 1 = query_digest
        assert_eq!(
            s.bind_public_inputs(&tampered),
            Err(BindingError::PublicInputMismatch { word_index: 1 })
        );

        assert_eq!(
            s.bind_public_inputs(&good[..good.len() - WORD_BYTES]),
            Err(BindingError::PublicInputLength {
                expected: good.len(),
                actual: good.len() - WORD_BYTES,
            })
        );
    }

    // ---- segment / statement validation ----------------------------------

    #[test]
    fn malformed_segments_are_refused() {
        // attribution arity != commitment count.
        assert!(matches!(
            HolderSegment::new(
                HolderId::new("alice"),
                vec![word(1), word(2)],
                vec![],
                0,
                vec![true],
            ),
            Err(BindingError::AttributionArity { .. })
        ));
        // no commitments at all.
        assert!(matches!(
            HolderSegment::new(HolderId::new("alice"), vec![], vec![], 0, vec![]),
            Err(BindingError::NoCommitments { .. })
        ));
        // more rows than the declared capacity.
        assert!(matches!(
            HolderSegment::new(
                HolderId::new("alice"),
                vec![word(1)],
                vec![[word(1), word(2), word(3)]],
                0,
                vec![true],
            ),
            Err(BindingError::RowOverflow { .. })
        ));
    }

    #[test]
    fn empty_and_duplicate_federations_are_refused() {
        assert_eq!(
            FederatedStatement::new(
                VerifierChallenge::from_word(word(1)),
                word(2),
                word(3),
                ValidityWindow::new(0, 1).unwrap(),
                vec![],
            ),
            Err(BindingError::EmptyFederation)
        );
        assert!(matches!(
            FederatedStatement::new(
                VerifierChallenge::from_word(word(1)),
                word(2),
                word(3),
                ValidityWindow::new(0, 1).unwrap(),
                vec![segment("alice", 1, 0, 0), segment("alice", 1, 0, 0)],
            ),
            Err(BindingError::DuplicateHolder { .. })
        ));
    }

    // ---- freshness / replay ----------------------------------------------

    /// The freshness tag covers every one of its inputs: change any and the tag
    /// changes. (The tag is what carries #4 into the public inputs.)
    #[test]
    fn freshness_tag_covers_every_input() {
        let base = statement(vec![segment("alice", 1, 1, 1)]);
        let tag = base.freshness_binding().tag();

        let other_challenge = FederatedStatement::new(
            VerifierChallenge::from_word(word(0xAC)),
            base.query_digest(),
            base.key_set_digest(),
            base.window(),
            base.holders().to_vec(),
        )
        .unwrap();
        assert_ne!(tag, other_challenge.freshness_binding().tag());

        let other_query = FederatedStatement::new(
            base.challenge(),
            word(0xC2),
            base.key_set_digest(),
            base.window(),
            base.holders().to_vec(),
        )
        .unwrap();
        assert_ne!(tag, other_query.freshness_binding().tag());

        let other_keys = FederatedStatement::new(
            base.challenge(),
            base.query_digest(),
            key_set_digest(&["did:example:other#key-1".to_string()]),
            base.window(),
            base.holders().to_vec(),
        )
        .unwrap();
        assert_ne!(tag, other_keys.freshness_binding().tag());

        let other_window = FederatedStatement::new(
            base.challenge(),
            base.query_digest(),
            base.key_set_digest(),
            ValidityWindow::new(100, 201).unwrap(),
            base.holders().to_vec(),
        )
        .unwrap();
        assert_ne!(tag, other_window.freshness_binding().tag());

        let other_commitments = statement(vec![segment("bob", 1, 1, 1)]);
        assert_ne!(tag, other_commitments.freshness_binding().tag());
    }

    /// The key-set digest is order- and duplicate-insensitive, so a prover cannot
    /// move the transcript by reshuffling `K`.
    #[test]
    fn key_set_digest_is_canonical() {
        let a = key_set_digest(&["b".into(), "a".into()]);
        let b = key_set_digest(&["a".into(), "b".into(), "a".into()]);
        assert_eq!(a, b);
        assert_ne!(a, key_set_digest(&["a".into()]));
        // Length-prefixing means "ab" ‖ "c" and "a" ‖ "bc" are distinct pre-images.
        assert_ne!(
            key_set_digest(&["ab".into(), "c".into()]),
            key_set_digest(&["a".into(), "bc".into()])
        );
    }

    /// A fresh statement passes once and is REJECTED on replay.
    #[test]
    fn replay_is_refused() {
        let s = statement(vec![segment("alice", 1, 1, 1)]);
        let challenge = s.challenge();
        let mut seen = InMemorySeenChallenges::new();

        let tag = check_freshness(&s, &challenge, 150, &mut seen).expect("fresh");
        assert_eq!(tag, s.freshness_binding().tag());
        assert_eq!(seen.len(), 1);

        assert_eq!(
            check_freshness(&s, &challenge, 150, &mut seen),
            Err(BindingError::ChallengeReplayed)
        );
    }

    /// A statement carrying someone else's challenge, or presented outside its
    /// window, is refused — and CRUCIALLY neither refusal burns the challenge, so
    /// a bad transcript cannot grief a pending honest one.
    #[test]
    fn expect_freshness_gate_orders_checks() {
        let s = statement(vec![segment("alice", 1, 1, 1)]);
        let mine = s.challenge();
        let mut seen = InMemorySeenChallenges::new();

        // (1) wrong challenge -> mismatch, nothing burnt.
        let theirs = VerifierChallenge::from_word(word(0xFF));
        assert_eq!(
            check_freshness(&s, &theirs, 150, &mut seen),
            Err(BindingError::ChallengeMismatch)
        );
        assert!(seen.is_empty(), "a mismatch must not burn the challenge");

        // (2) outside the window -> expiry, still nothing burnt.
        assert!(matches!(
            check_freshness(&s, &mine, 99, &mut seen),
            Err(BindingError::OutsideValidityWindow { .. })
        ));
        assert!(matches!(
            check_freshness(&s, &mine, 201, &mut seen),
            Err(BindingError::OutsideValidityWindow { .. })
        ));
        assert!(seen.is_empty(), "an expiry must not burn the challenge");

        // (3) the honest transcript still succeeds afterwards.
        check_freshness(&s, &mine, 200, &mut seen).expect("boundary is inclusive");
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn inverted_window_is_refused() {
        assert_eq!(
            ValidityWindow::new(10, 9),
            Err(BindingError::InvertedValidityWindow {
                not_before: 10,
                not_after: 9,
            })
        );
    }

    // ---- field words -----------------------------------------------------

    #[test]
    fn fresh_challenges_are_distinct_and_canonical() {
        let a = VerifierChallenge::fresh();
        let b = VerifierChallenge::fresh();
        assert_ne!(a, b, "two OS-seeded draws must not collide");
        for c in [a, b] {
            assert_eq!(
                c.as_word().as_be_bytes()[0],
                0,
                "the top byte is cleared so the word is a canonical field element"
            );
        }
    }

    #[test]
    fn digests_are_canonical_field_elements() {
        assert_eq!(FieldWord::digest(FRESHNESS_DOMAIN_TAG, &[b"x"]).as_be_bytes()[0], 0);
        // Domain separation: the same pre-image under a different tag differs.
        assert_ne!(
            FieldWord::digest(FRESHNESS_DOMAIN_TAG, &[b"x"]),
            FieldWord::digest(HOLDER_DOMAIN_TAG, &[b"x"])
        );
    }

    #[test]
    fn field_word_hex_round_trips_and_fails_closed() {
        let w = word(0x2A);
        let mut hex = String::from("0x");
        for b in w.as_be_bytes() {
            use std::fmt::Write;
            write!(hex, "{b:02x}").unwrap();
        }
        assert_eq!(FieldWord::from_hex(&hex).unwrap(), w);
        // Bare (un-prefixed) hex is accepted too.
        assert_eq!(FieldWord::from_hex(&hex[2..]).unwrap(), w);
        assert!(FieldWord::from_hex("0x00").is_err());
        assert!(FieldWord::from_hex(&"z".repeat(64)).is_err());
    }

    /// The binding layer is NOT a deferred capability: its failures are honest
    /// protocol refusals, so they surface as [`MpcError::Protocol`] and never as
    /// [`MpcError::NotYetImplemented`] (which stays reserved for the still-gated
    /// collaborative proof itself).
    #[test]
    fn binding_errors_are_protocol_errors_not_deferrals() {
        let e: MpcError = BindingError::ChallengeReplayed.into();
        assert!(matches!(e, MpcError::Protocol(_)));
        assert!(!matches!(e, MpcError::NotYetImplemented { .. }));
    }
}
