//! (sq-pi44) **Incremental add / remove / update of vectors** against an
//! already-finalized [`VectorStore`](crate::store::VectorStore) — WITHOUT a full re-embed
//! and file rebuild.
//!
//! # Why
//!
//! A `.spqv` store is build-once-immutable: [`VectorStore::put`](crate::store::VectorStore::put)
//! errors after [`finalize`](crate::store::VectorStore::finalize), and there is no remove or
//! update. So today *any* graph change — one new entity, one deleted entity — forces a full
//! re-embed and a fresh file. The structural indexes ([`sparq-sim`](https://docs.rs/sparq-sim)
//! style) track graph mutation for free because the index *is* the data; the vector store does
//! not, because it is keyed by dictionary id and stored densely.
//!
//! # What this adds (the **delta sidecar**)
//!
//! A small in-RAM layer over the immutable base mmap:
//!   * an **append map** `id → vector` of vectors added or updated since the base was built, and
//!   * a **tombstone set** of ids removed since the base was built.
//!
//! Every read path on the store ([`get`](crate::store::VectorStore::get),
//! [`iter`](crate::store::VectorStore::iter), [`len`](crate::store::VectorStore::len)) consults
//! the delta transparently: a tombstoned id reads as absent, an appended/updated id reads its
//! delta vector (the delta SHADOWS the base for an id present in both), and a plain base id reads
//! the mmap. Search (`nearest_exact`, the ANN indexes that consume `iter`) therefore unions
//! base+delta and honours tombstones with no change to the search code itself.
//!
//! [`VectorStore::compact`](crate::store::VectorStore::compact) folds the delta back into a fresh
//! base file: the result is **equivalent to a from-scratch rebuild over the same final vector
//! set** (same `(id, vector)` pairs ⇒ a store whose `get`/`iter`/`len` agree exactly with one
//! built by `create` + `put` over that set).
//!
//! # Generation tie (the staleness guard, sq-32i5)
//!
//! A delta is only meaningful against the graph generation it was built against — the ids it
//! appends/tombstones are that generation's dictionary ids. A [`VectorDelta`] therefore carries
//! the [`Fingerprint`] of the graph it targets, and [`VectorStore::apply_delta`](crate::store::VectorStore::apply_delta) REJECTS a delta
//! whose generation does not match the base store's bound fingerprint (gate: a delta tied to
//! generation N applied to a base of generation M ≠ N is an error, not a silent mis-key). This
//! reuses the sq-32i5 header fingerprint rather than inventing a second staleness mechanism.
//!
//! # Scope (honest boundary)
//!
//! The delta is **in-RAM only**: it lives on the open [`VectorStore`](crate::store::VectorStore) handle and is lost when the
//! handle is dropped (the base `.spqv` on disk is unchanged until
//! [`compact`](crate::store::VectorStore::compact) writes a new one). A *persisted* delta sidecar
//! (its own on-disk format, crash-durable across process restarts) is tracked as a follow-up — see
//! the crate docs / the sq-pi44 follow-up bead. `compact` is the durability path today: it
//! materializes the delta into a normal, validated `.spqv`.
//!
//! # (sq-7e50) Persisted on-disk delta sidecar (`.spqd`)
//!
//! The in-RAM delta above is lost when the [`VectorStore`](crate::store::VectorStore) handle drops
//! — [`compact`](crate::store::VectorStore::compact) was the only durability path. sq-7e50 adds a
//! **persisted** sidecar so incremental add/remove/update survive a process restart *without* a
//! compact: [`VectorStore::save_delta`](crate::store::VectorStore::save_delta) serializes the
//! in-RAM [`VectorDelta`] to a little-endian `.spqd` file alongside the `.spqv` base, and
//! [`VectorStore::open_with_delta`](crate::store::VectorStore::open_with_delta) opens the base mmap
//! and replays the persisted delta back onto it (through the same generation-tie validation as
//! [`apply_delta`](crate::store::VectorStore::apply_delta)).
//!
//! ## On-disk format (version 1, little-endian — mirrors the `.spqv` header discipline)
//!
//! ```text
//! offset 0   magic         b"SPQD"                         4 bytes
//! offset 4   version       u32 = 1                         4 bytes
//! offset 8   dim           u32                             4 bytes
//! offset 12  append_count  u64                             8 bytes
//! offset 20  tomb_count    u64                             8 bytes
//! offset 28  reserved      zero padding                    4 bytes
//! offset 32  fingerprint   base graph fingerprint          24 bytes
//!            (dict_len: u64, triple_count: u64, content_hash: u64)
//! offset 56  appends       [append_count × (id: u32, dim × f32)]
//!            tombstones     [tomb_count × (id: u32)]        (after the appends)
//! ```
//!
//! The header carries the **base graph fingerprint** (the [`generation`](VectorDelta::generation)
//! the delta is keyed against — the sq-32i5 generation tie): on open the persisted fingerprint is
//! decoded into the delta's `generation`, so replaying it through `apply_delta` REJECTS a delta
//! whose generation does not match the base store's bound fingerprint — a persisted delta written
//! against generation N can never be silently mis-keyed onto a base of generation M ≠ N. The
//! all-zero block decodes to `None` (an unbound/unverified delta), exactly as the `.spqv`
//! fingerprint block does.
//!
//! **Crash-durability.** [`save_delta`](crate::store::VectorStore::save_delta) writes to a sibling
//! `.spqd-tmp`, `fsync`s it, then atomically `rename`s it over the final path — the same
//! write-tmp-then-rename + `sync_all` discipline as [`StreamingWriter`](crate::store::StreamingWriter):
//! a crash mid-write leaves the previous `.spqd` (or none) intact, never a half-written sidecar at
//! the live path. **Partial-write detection.** `from_bytes` (the crate-internal decoder) recomputes
//! the EXACT expected length from the header (`56 + append_count·(4 + dim·4) + tomb_count·4`) and
//! rejects any file that is shorter OR longer — a truncated (or trailing-garbage) sidecar is a
//! descriptive `Err`, never an out-of-bounds read or a silently-short replay.
//!
//! Opt-in: the whole layer is gated behind the **`delta`** cargo feature, off by default, and adds
//! NO dependency (it reuses the in-tree `rustc-hash` map/set), so the default build carries zero
//! delta code.

use crate::fingerprint::{Fingerprint, FINGERPRINT_LEN};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::Id;

/// (sq-7e50) First four bytes of every persisted `.spqd` delta sidecar file.
pub const SPQD_MAGIC: [u8; 4] = *b"SPQD";
/// (sq-7e50) Current `.spqd` format version.
pub const SPQD_VERSION: u32 = 1;
/// (sq-7e50) Byte length of the `.spqd` header: magic(4) + version(4) + dim(4) +
/// append_count(8) + tomb_count(8) + reserved(4) + fingerprint(`FINGERPRINT_LEN`). All counts and
/// the fingerprint are fixed-width little-endian, so a sidecar written on one machine decodes
/// identically on another (the same cross-machine discipline as the `.spqv` header / fingerprint).
const SPQD_HEADER_LEN: usize = 32 + FINGERPRINT_LEN;

/// (sq-pi44) The in-RAM delta sidecar: vectors appended/updated and ids tombstoned
/// since the base store was built, plus the graph [`Fingerprint`] the delta is keyed against.
///
/// An id is in AT MOST one of `appended` / `tombstones` at any time — adding/updating an id clears
/// its tombstone, removing an id drops it from `appended` and tombstones it. The base store is never
/// mutated; the delta merely shadows it on read until
/// [`compact`](crate::store::VectorStore::compact) folds it in.
#[derive(Clone, Debug, Default)]
pub struct VectorDelta {
    /// The graph generation this delta targets, or `None` for a delta over a store with no base
    /// fingerprint. [`VectorStore::apply_delta`](crate::store::VectorStore::apply_delta) checks it against the receiving store.
    pub(crate) generation: Option<Fingerprint>,
    /// `id → vector` for vectors added or updated since the base; SHADOWS the base for a shared id.
    pub(crate) appended: FxHashMap<Id, Vec<f32>>,
    /// ids removed since the base (a base id reads as absent; an appended id is dropped, not kept).
    pub(crate) tombstones: FxHashSet<Id>,
}

impl VectorDelta {
    /// A fresh empty delta bound to `generation` (the fingerprint of the graph it targets).
    pub(crate) fn new(generation: Option<Fingerprint>) -> VectorDelta {
        VectorDelta {
            generation,
            appended: FxHashMap::default(),
            tombstones: FxHashSet::default(),
        }
    }

    /// The graph generation this delta is keyed against (the base store's fingerprint), or `None`
    /// if the base carried none. See [`VectorStore::apply_delta`](crate::store::VectorStore::apply_delta).
    pub fn generation(&self) -> Option<Fingerprint> {
        self.generation
    }

    /// Number of appended/updated vectors held in the delta.
    pub fn appended_len(&self) -> usize {
        self.appended.len()
    }

    /// Number of tombstoned ids held in the delta.
    pub fn tombstone_len(&self) -> usize {
        self.tombstones.len()
    }

    /// Whether the delta holds no appends and no tombstones (a no-op delta).
    pub fn is_empty(&self) -> bool {
        self.appended.is_empty() && self.tombstones.is_empty()
    }

    /// (sq-7e50) Serializes the delta to the little-endian `.spqd` on-disk format (see
    /// the module docs). `dim` is the base store's vector dimension — every appended vector MUST
    /// be exactly `dim` long (the in-RAM `add`/`update` validation guarantees this) so the body is
    /// a fixed `dim·4`-byte stride per append, and `from_bytes` can recompute the exact length.
    ///
    /// Appends and tombstones are each emitted in ASCENDING id order, so the bytes are a
    /// deterministic function of the logical delta (re-saving an unchanged delta is byte-stable and
    /// machine-independent), not of `FxHashMap`/`FxHashSet` iteration order.
    pub(crate) fn to_bytes(&self, dim: usize) -> Vec<u8> {
        let append_count = self.appended.len();
        let tomb_count = self.tombstones.len();
        let mut out = Vec::with_capacity(
            SPQD_HEADER_LEN + append_count * (4 + dim * 4) + tomb_count * 4,
        );
        out.extend_from_slice(&SPQD_MAGIC);
        out.extend_from_slice(&SPQD_VERSION.to_le_bytes());
        out.extend_from_slice(&(dim as u32).to_le_bytes());
        out.extend_from_slice(&(append_count as u64).to_le_bytes());
        out.extend_from_slice(&(tomb_count as u64).to_le_bytes());
        out.extend_from_slice(&[0u8; 4]); // reserved
        // The base generation tie: a zero block for an unbound (None) generation — decoded back to
        // None by `Fingerprint::from_bytes_opt`, exactly as the `.spqv` fingerprint block is.
        match self.generation {
            Some(fp) => out.extend_from_slice(&fp.to_bytes()),
            None => out.extend_from_slice(&[0u8; FINGERPRINT_LEN]),
        }
        debug_assert_eq!(out.len(), SPQD_HEADER_LEN);

        // Appends in ascending id order: (id: u32, dim × f32).
        let mut appends: Vec<(&Id, &Vec<f32>)> = self.appended.iter().collect();
        appends.sort_unstable_by_key(|&(id, _)| *id);
        for (id, vec) in appends {
            out.extend_from_slice(&id.to_le_bytes());
            for &v in vec {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        // Tombstones in ascending id order: (id: u32).
        let mut tombs: Vec<Id> = self.tombstones.iter().copied().collect();
        tombs.sort_unstable();
        for id in tombs {
            out.extend_from_slice(&id.to_le_bytes());
        }
        out
    }

    /// (sq-7e50) Parses a `.spqd` delta sidecar from `bytes`, returning the reconstructed
    /// in-RAM [`VectorDelta`] (its `generation` set from the header fingerprint block) and the base
    /// `dim` it was written for (the caller checks `dim` against its base store).
    ///
    /// **Validation is fail-closed.** A bad magic, an unknown version, a `dim` of zero, an append/
    /// tombstone count whose implied body length overflows or does not EXACTLY match `bytes.len()`
    /// (the partial-write / truncation guard — a short OR long file is rejected), or a duplicate /
    /// conflicting id (an id appearing in both the append and tombstone sections, or twice in one
    /// section) is a descriptive `Err`, never an out-of-bounds read or a silently-truncated replay.
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<(VectorDelta, usize), String> {
        if bytes.len() < SPQD_HEADER_LEN {
            return Err(format!(
                ".spqd: truncated header ({} bytes, need at least {})",
                bytes.len(),
                SPQD_HEADER_LEN
            ));
        }
        if bytes[0..4] != SPQD_MAGIC {
            return Err(".spqd: not a delta sidecar (bad magic)".into());
        }
        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        if version != SPQD_VERSION {
            return Err(format!(
                ".spqd: unsupported version {} (this build writes/reads version {})",
                version, SPQD_VERSION
            ));
        }
        let dim = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        if dim == 0 {
            return Err(".spqd: invalid zero vector dimension".into());
        }
        let append_count = u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
        let tomb_count = u64::from_le_bytes(bytes[20..28].try_into().unwrap()) as usize;
        // The base generation tie (offset 32..56), all-zero ⇒ None (unbound/unverified).
        let generation = Fingerprint::from_bytes_opt(&bytes[32..32 + FINGERPRINT_LEN]);

        // The EXACT expected total length, computed with checked arithmetic so a hostile header
        // count cannot wrap the multiply/add into a small in-bounds value. Any mismatch (short =
        // truncation/partial write, long = trailing garbage) is rejected before any body read.
        let append_stride = 4usize
            .checked_add(dim.checked_mul(4).ok_or_else(|| ".spqd: dim overflow".to_string())?)
            .ok_or_else(|| ".spqd: append-stride overflow".to_string())?;
        let body_len = append_count
            .checked_mul(append_stride)
            .and_then(|a| tomb_count.checked_mul(4).and_then(|t| a.checked_add(t)))
            .ok_or_else(|| ".spqd: append/tombstone count overflow".to_string())?;
        let expected = SPQD_HEADER_LEN
            .checked_add(body_len)
            .ok_or_else(|| ".spqd: total-length overflow".to_string())?;
        if bytes.len() != expected {
            return Err(format!(
                ".spqd: length mismatch — header implies {} bytes (dim={}, appends={}, \
                 tombstones={}) but the file is {} bytes; the sidecar is truncated or corrupt",
                expected,
                dim,
                append_count,
                tomb_count,
                bytes.len()
            ));
        }

        let mut appended: FxHashMap<Id, Vec<f32>> = FxHashMap::default();
        appended.reserve(append_count);
        let mut off = SPQD_HEADER_LEN;
        for _ in 0..append_count {
            let id = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            off += 4;
            let mut vec = Vec::with_capacity(dim);
            for _ in 0..dim {
                vec.push(f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()));
                off += 4;
            }
            if vec.iter().any(|v| !v.is_finite()) {
                return Err(format!(".spqd: appended vector for id {} has a non-finite component", id));
            }
            if appended.insert(id, vec).is_some() {
                return Err(format!(".spqd: id {} appears twice in the append section", id));
            }
        }
        let mut tombstones: FxHashSet<Id> = FxHashSet::default();
        tombstones.reserve(tomb_count);
        for _ in 0..tomb_count {
            let id = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            off += 4;
            if !tombstones.insert(id) {
                return Err(format!(".spqd: id {} appears twice in the tombstone section", id));
            }
            if appended.contains_key(&id) {
                // The in-RAM invariant is "an id is in AT MOST one of appended/tombstones"; a file
                // that violates it is corrupt, not a valid delta — reject rather than silently pick.
                return Err(format!(".spqd: id {} is both appended and tombstoned (corrupt)", id));
            }
        }
        debug_assert_eq!(off, bytes.len());
        Ok((VectorDelta { generation, appended, tombstones }, dim))
    }
}

#[cfg(test)]
mod tests {
    // (sq-7e50) Focused unit tests for the `.spqd` serde: the round-trip preserves the
    // logical delta (incl. the None/unbound generation), the bytes are deterministic regardless of
    // map iteration order, and the corruption paths the integration tests do not exercise directly
    // (a both-appended-and-tombstoned id) are rejected. The store-level open/save and the
    // result-equivalence + crash + mismatch gates live in `tests/delta_persist.rs`.
    use super::*;

    fn fp(d: u64, t: u64, h: u64) -> Fingerprint {
        Fingerprint { dict_len: d, triple_count: t, content_hash: h }
    }

    fn sample(generation: Option<Fingerprint>) -> VectorDelta {
        let mut d = VectorDelta::new(generation);
        d.appended.insert(7, vec![1.0, 2.0, 3.0, 4.0]);
        d.appended.insert(3, vec![-0.5, 0.0, 0.25, 9.0]);
        d.tombstones.insert(11);
        d.tombstones.insert(2);
        d
    }

    #[test]
    fn round_trip_with_a_bound_generation() {
        let d = sample(Some(fp(4, 2, 0xdead_beef)));
        let bytes = d.to_bytes(4);
        let (back, dim) = VectorDelta::from_bytes(&bytes).expect("round-trips");
        assert_eq!(dim, 4);
        assert_eq!(back.generation(), d.generation());
        assert_eq!(back.appended, d.appended);
        assert_eq!(back.tombstones, d.tombstones);
    }

    #[test]
    fn round_trip_with_an_unbound_generation_is_none() {
        // An unbound (None) generation must serialize as an all-zero block and decode back to None
        // (the same convention as the `.spqv` fingerprint block) — never to Some(0,0,0).
        let d = sample(None);
        let (back, _) = VectorDelta::from_bytes(&d.to_bytes(4)).expect("round-trips");
        assert_eq!(back.generation(), None, "an all-zero header decodes to an unbound generation");
        assert_eq!(back.appended, d.appended);
        assert_eq!(back.tombstones, d.tombstones);
    }

    #[test]
    fn an_empty_delta_serializes_to_just_the_header() {
        let d = VectorDelta::new(Some(fp(1, 1, 1)));
        let bytes = d.to_bytes(8);
        assert_eq!(bytes.len(), SPQD_HEADER_LEN, "no appends/tombstones ⇒ header-only");
        let (back, dim) = VectorDelta::from_bytes(&bytes).expect("round-trips");
        assert_eq!(dim, 8);
        assert!(back.is_empty());
        assert_eq!(back.generation(), Some(fp(1, 1, 1)));
    }

    #[test]
    fn the_bytes_are_deterministic_regardless_of_insertion_order() {
        // Two deltas with the same logical content but different insertion order must serialize to
        // IDENTICAL bytes (appends + tombstones are emitted in ascending id order).
        let mut a = VectorDelta::new(Some(fp(2, 2, 2)));
        a.appended.insert(9, vec![1.0, 1.0]);
        a.appended.insert(1, vec![2.0, 2.0]);
        a.tombstones.insert(8);
        a.tombstones.insert(4);
        let mut b = VectorDelta::new(Some(fp(2, 2, 2)));
        b.appended.insert(1, vec![2.0, 2.0]);
        b.appended.insert(9, vec![1.0, 1.0]);
        b.tombstones.insert(4);
        b.tombstones.insert(8);
        assert_eq!(a.to_bytes(2), b.to_bytes(2), "the serialization is order-independent");
    }

    #[test]
    fn a_short_buffer_is_rejected_not_indexed_out_of_bounds() {
        // A buffer shorter than the header must error, never panic on a slice.
        let err = VectorDelta::from_bytes(&[0u8; 10]).expect_err("a sub-header buffer must error");
        assert!(err.contains("truncated"), "err was: {err}");
    }

    #[test]
    fn a_count_that_does_not_match_the_length_is_rejected() {
        // Forge a header that claims 5 appends but carries no body → length mismatch.
        let mut bytes = VectorDelta::new(Some(fp(1, 1, 1))).to_bytes(4);
        bytes[12..20].copy_from_slice(&5u64.to_le_bytes());
        let err = VectorDelta::from_bytes(&bytes).expect_err("a lying count must be rejected");
        assert!(err.contains("length mismatch"), "err was: {err}");
    }

    #[test]
    fn a_bad_magic_or_version_is_rejected() {
        let mut bytes = VectorDelta::new(Some(fp(1, 1, 1))).to_bytes(4);
        let good = bytes.clone();
        bytes[0] = b'Z';
        assert!(VectorDelta::from_bytes(&bytes).unwrap_err().contains("bad magic"));
        let mut v = good.clone();
        v[4..8].copy_from_slice(&999u32.to_le_bytes());
        assert!(VectorDelta::from_bytes(&v).unwrap_err().contains("unsupported version"));
    }

    #[test]
    fn a_zero_dimension_header_is_rejected() {
        // dim==0 is a corrupt header (no valid append stride): the decoder must reject it
        // up front, never divide-by / produce a degenerate zero-width delta. A from-scratch builder
        // can never emit it (`to_bytes` is only called with the base store's dim ≥ 1), so this only
        // arises from a corrupt/forged sidecar — fail closed.
        let mut bytes = VectorDelta::new(Some(fp(1, 1, 1))).to_bytes(4);
        bytes[8..12].copy_from_slice(&0u32.to_le_bytes()); // dim -> 0
        let err = VectorDelta::from_bytes(&bytes).expect_err("a zero-dim header must be rejected");
        assert!(err.contains("zero vector dimension"), "err was: {err}");
    }

    #[test]
    fn a_trailing_garbage_file_is_rejected_as_a_length_mismatch() {
        // The length guard rejects a LONGER file too (trailing garbage), not only a short
        // one — a sidecar with extra bytes after the declared body is corrupt, never silently
        // ignored (which would let a partial overwrite go undetected).
        let mut bytes = sample(Some(fp(4, 2, 7))).to_bytes(4);
        let good_len = bytes.len();
        bytes.extend_from_slice(&[0xAB, 0xCD, 0xEF]); // append trailing garbage
        let err = VectorDelta::from_bytes(&bytes).expect_err("a longer-than-declared file must error");
        assert!(err.contains("length mismatch"), "err was: {err}");
        // Sanity: the un-extended bytes still decode, so it is the trailing bytes that are rejected.
        assert!(VectorDelta::from_bytes(&bytes[..good_len]).is_ok());
    }

    #[test]
    fn a_non_finite_appended_component_is_rejected() {
        // The in-RAM `add`/`update` path rejects non-finite vectors, so a `.spqd` carrying
        // one is corrupt — the decoder re-checks (a forged/corrupt file could carry a NaN/inf the
        // builder never would), and must reject it so a non-finite vector can never re-enter the store
        // through a replay and poison cosine. Forge a NaN into the single append's body.
        let mut d = VectorDelta::new(Some(fp(2, 1, 1)));
        d.appended.insert(9, vec![1.0, 2.0]); // dim 2
        let mut bytes = d.to_bytes(2);
        // Body layout: header (SPQD_HEADER_LEN), then [id: u32][f32 × dim]. The first component of the
        // single append starts at SPQD_HEADER_LEN + 4.
        let comp0 = SPQD_HEADER_LEN + 4;
        bytes[comp0..comp0 + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        let err = VectorDelta::from_bytes(&bytes).expect_err("a NaN component must be rejected");
        assert!(err.contains("non-finite"), "err was: {err}");

        // ...and +inf is rejected the same way.
        bytes[comp0..comp0 + 4].copy_from_slice(&f32::INFINITY.to_le_bytes());
        assert!(VectorDelta::from_bytes(&bytes).unwrap_err().contains("non-finite"));
    }

    #[test]
    fn a_duplicate_id_in_the_append_section_is_rejected() {
        // An id may appear at most once per section. A file repeating an append id is
        // corrupt (the in-RAM map could never produce it) — reject rather than silently keep the last,
        // which would mask a corrupt sidecar. Forge two appends of id 5 by hand-building the body.
        let mut bytes = VectorDelta::new(Some(fp(1, 1, 1))).to_bytes(2);
        // Set the header to declare 2 appends, 0 tombstones, dim 2.
        bytes[12..20].copy_from_slice(&2u64.to_le_bytes()); // append_count = 2
        bytes[20..28].copy_from_slice(&0u64.to_le_bytes()); // tomb_count   = 0
        // Append id 5 twice with valid (finite) bodies.
        for _ in 0..2 {
            bytes.extend_from_slice(&5u32.to_le_bytes());
            bytes.extend_from_slice(&1.0f32.to_le_bytes());
            bytes.extend_from_slice(&2.0f32.to_le_bytes());
        }
        let err = VectorDelta::from_bytes(&bytes).expect_err("a repeated append id must be rejected");
        assert!(err.contains("twice in the append section"), "err was: {err}");
    }

    #[test]
    fn a_duplicate_id_in_the_tombstone_section_is_rejected() {
        // Same invariant for tombstones: a repeated tombstone id is a corrupt file.
        let mut bytes = VectorDelta::new(Some(fp(1, 1, 1))).to_bytes(2);
        bytes[12..20].copy_from_slice(&0u64.to_le_bytes()); // append_count = 0
        bytes[20..28].copy_from_slice(&2u64.to_le_bytes()); // tomb_count   = 2
        bytes.extend_from_slice(&8u32.to_le_bytes());
        bytes.extend_from_slice(&8u32.to_le_bytes()); // id 8 twice
        let err = VectorDelta::from_bytes(&bytes).expect_err("a repeated tombstone id must be rejected");
        assert!(err.contains("twice in the tombstone section"), "err was: {err}");
    }

    #[test]
    fn an_id_both_appended_and_tombstoned_is_rejected_as_corrupt() {
        // The load-bearing delta invariant is "an id is in AT MOST one of appended /
        // tombstoned" — `add`/`update` clears the tombstone and `remove` drops the append, so the two
        // sets are always disjoint. A `.spqd` that lists the same id in BOTH sections is corrupt
        // (no valid in-RAM delta could have produced it), and the decoder must reject it rather than
        // silently pick one — otherwise a replay would resolve a removed id to a stale vector (or
        // vice-versa). The inline test module's own header comment claims this path is covered; this
        // is the test that actually exercises it. Forge id 5 into both sections.
        let mut bytes = VectorDelta::new(Some(fp(1, 1, 1))).to_bytes(2);
        bytes[12..20].copy_from_slice(&1u64.to_le_bytes()); // append_count = 1
        bytes[20..28].copy_from_slice(&1u64.to_le_bytes()); // tomb_count   = 1
        // append id 5 with a finite body...
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        bytes.extend_from_slice(&2.0f32.to_le_bytes());
        // ...then tombstone the same id 5.
        bytes.extend_from_slice(&5u32.to_le_bytes());
        let err = VectorDelta::from_bytes(&bytes)
            .expect_err("an id in both sections must be rejected as corrupt");
        assert!(err.contains("both appended and tombstoned"), "err was: {err}");
    }
}
