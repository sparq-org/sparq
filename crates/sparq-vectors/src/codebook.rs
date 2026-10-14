//! Categorical **codebook** encoder for enumeration members (structure-aware vectorisation **P2**).
//!
//! sq-0wo9e.3 (epic sq-0wo9e; design `research/structure-aware-vectorisation.md`
//! §2 row "`owl:oneOf` / `sh:in` enum → a CATEGORICAL CODEBOOK" + §6.A "enum / boolean exactness").
//! The encoder for [`crate::encode::Encoder::Enum`]. It is **opt-in** (the `structure` cargo
//! feature, off by default — pure, no new dependency) and changes **nothing** in the default
//! `sparq-vectors` build or the core engine.
//!
//! # The provable invariant — enum equality is a SLOT MATCH, not a cosine threshold
//!
//! An enumeration declared by `sh:in (…)` or `owl:oneOf (…)` is a **closed set** of members. The
//! codebook reserves **one dimension per member** plus a **single reserved dimension for the
//! out-of-enum / invalid code** (slot `0`). Encoding sets exactly one slot to `1.0`:
//!
//! - a member sets its own member slot → membership is decided by **which slot is hot**, an exact
//!   equality, **never a cosine threshold** (design §2: "no recall loss");
//! - a value **not in the enum** sets the reserved invalid slot (`0`) → it is a *distinct, reserved
//!   code* a closed-world SHACL `sh:in` shape rejects (design §2: "out-of-enum → reserved invalid
//!   code SHACL rejects"), never silently collapsed onto a member.
//!
//! [`Codebook::encode`] / [`Codebook::decode`] **round-trip** every member and the invalid code
//! (the `encode_decode_roundtrip` test is the gate). The block is one-hot non-negative, so it is correct
//! under L2/cosine and tagged [`Metric::Euclidean`](crate::encode::Metric::Euclidean).
//!
//! # What is NOT claimed
//!
//! No embedding-quality claim. A learned codebook (rather than one-hot) is a future variant; the
//! one-hot form is the *provable* baseline (exact membership), and whether a learned codebook lifts
//! downstream retrieval is empirical and ablation-gated (design §6.B). The width grows with the
//! enum size, so this is intended for **small, closed** enumerations (the common `sh:in` case).

use oxrdf::Term;
use rustc_hash::FxHashMap;

/// A one-hot categorical codebook over a **closed** set of enumeration members (the `sh:in` /
/// `owl:oneOf` members). Width is `members + 1`: slot `0` is the reserved out-of-enum / invalid
/// code, slots `1..=members` are the members in declaration order.
#[derive(Clone, Debug)]
pub struct Codebook {
    /// Member term → its 1-based slot index (slot `0` is reserved for the invalid code).
    slot_of: FxHashMap<Term, usize>,
    /// Members in declaration order (index `i` ↦ slot `i + 1`) — for [`decode`](Self::decode) and
    /// [`members`](Self::members).
    members: Vec<Term>,
}

/// The reserved slot index for the **out-of-enum / invalid** code (design §2). A value not in the
/// enum encodes here, distinct from every member slot.
pub const INVALID_SLOT: usize = 0;

impl Codebook {
    /// Build a codebook over `members` (the `sh:in` / `owl:oneOf` members, in declaration order).
    /// Duplicate members collapse to their first slot (a malformed enum with repeats does not widen
    /// the block); the order of first appearance is preserved.
    pub fn new(members: impl IntoIterator<Item = Term>) -> Codebook {
        let mut slot_of: FxHashMap<Term, usize> = FxHashMap::default();
        let mut ordered: Vec<Term> = Vec::new();
        for m in members {
            if !slot_of.contains_key(&m) {
                slot_of.insert(m.clone(), ordered.len() + 1); // 1-based; slot 0 is reserved
                ordered.push(m);
            }
        }
        Codebook { slot_of, members: ordered }
    }

    /// The number of **distinct** enum members (excludes the reserved invalid slot).
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// The codebook block width — `member_count + 1` (the members plus the reserved invalid slot).
    pub fn dim(&self) -> usize {
        self.members.len() + 1
    }

    /// The members in declaration order (slot `i + 1` is `members()[i]`).
    pub fn members(&self) -> &[Term] {
        &self.members
    }

    /// The 1-based slot of `term`, or [`INVALID_SLOT`] (`0`) if `term` is **not** an enum member —
    /// the reserved out-of-enum code. This is the exact membership decision (a slot match).
    pub fn slot(&self, term: &Term) -> usize {
        self.slot_of.get(term).copied().unwrap_or(INVALID_SLOT)
    }

    /// Is `term` a member of this enumeration? (`slot(term) != INVALID_SLOT`.)
    pub fn is_member(&self, term: &Term) -> bool {
        self.slot_of.contains_key(term)
    }

    /// One-hot-encode `term` into `out` (length [`dim`](Self::dim)). Exactly one slot is `1.0`: the
    /// member's slot for a member, or the reserved [`INVALID_SLOT`] for an out-of-enum value.
    /// Returns `Err` on a length mismatch (a layout bug, never silently truncated).
    pub fn encode_into(&self, term: &Term, out: &mut [f32]) -> Result<(), String> {
        if out.len() != self.dim() {
            return Err(format!(
                "Codebook: out.len()={} but block dim={}",
                out.len(),
                self.dim()
            ));
        }
        out.iter_mut().for_each(|s| *s = 0.0);
        out[self.slot(term)] = 1.0;
        Ok(())
    }

    /// Convenience: one-hot-encode `term` to a freshly-allocated `Vec<f32>` of length
    /// [`dim`](Self::dim).
    pub fn encode(&self, term: &Term) -> Vec<f32> {
        let mut out = vec![0.0f32; self.dim()];
        let _ = self.encode_into(term, &mut out); // only errors on length mismatch, impossible here
        out
    }

    /// **Decode** a codebook block back to the member it encodes — the exactness witness (design
    /// §6.A "slot round-trip"). Returns `Some(member)` for a member slot, `None` for the reserved
    /// invalid slot (the out-of-enum code) — so `decode(encode(member)) == Some(member)` and
    /// `decode(encode(non_member)) == None`. The hot slot is the **argmax** (the block is one-hot,
    /// so this is unambiguous for a well-formed block); `None` is also returned for an all-zero
    /// block.
    pub fn decode(&self, block: &[f32]) -> Option<Term> {
        let (hot, &val) = block
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))?;
        if val <= 0.0 || hot == INVALID_SLOT {
            return None; // all-zero, or the reserved invalid slot
        }
        // slot `hot` (1-based) ↦ member index `hot - 1`.
        self.members.get(hot - 1).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::NamedNode;

    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new_unchecked(s))
    }

    fn sample() -> Codebook {
        Codebook::new(vec![
            iri("http://ex/Red"),
            iri("http://ex/Green"),
            iri("http://ex/Blue"),
        ])
    }

    #[test]
    fn dim_is_members_plus_invalid_slot() {
        let cb = sample();
        assert_eq!(cb.member_count(), 3);
        assert_eq!(cb.dim(), 4, "3 members + 1 reserved invalid slot");
    }

    #[test]
    fn encode_decode_roundtrip() {
        let cb = sample();
        // Every member round-trips through encode→decode (exactness, design §6.A).
        for m in cb.members() {
            let block = cb.encode(m);
            assert_eq!(cb.decode(&block).as_ref(), Some(m), "member must round-trip: {m}");
            // One-hot: exactly one slot is 1.0, and it is NOT the invalid slot.
            let hot: Vec<usize> = block.iter().enumerate().filter(|(_, &v)| v == 1.0).map(|(i, _)| i).collect();
            assert_eq!(hot.len(), 1, "exactly one hot slot");
            assert_ne!(hot[0], INVALID_SLOT, "a member must not use the invalid slot");
        }
    }

    #[test]
    fn out_of_enum_uses_reserved_invalid_code() {
        let cb = sample();
        let outsider = iri("http://ex/Purple"); // NOT in the enum
        assert!(!cb.is_member(&outsider));
        assert_eq!(cb.slot(&outsider), INVALID_SLOT);
        let block = cb.encode(&outsider);
        // The invalid slot is hot, and decode reports None (not a member) — the reserved code a
        // closed-world SHACL `sh:in` shape rejects, never silently a member.
        assert_eq!(block[INVALID_SLOT], 1.0);
        assert!(cb.decode(&block).is_none(), "out-of-enum decodes to None, not a member");
    }

    #[test]
    fn slot_match_not_cosine_threshold() {
        // Two distinct members are ORTHOGONAL one-hot codes (cosine 0) — equality is the slot, not a
        // distance threshold; no near-member ever aliases onto another member.
        let cb = sample();
        let red = cb.encode(&iri("http://ex/Red"));
        let green = cb.encode(&iri("http://ex/Green"));
        let dot: f32 = red.iter().zip(green.iter()).map(|(a, b)| a * b).sum();
        assert_eq!(dot, 0.0, "distinct members are orthogonal — membership is exact, not fuzzy");
    }

    #[test]
    fn duplicate_members_collapse() {
        let cb = Codebook::new(vec![iri("http://ex/A"), iri("http://ex/A"), iri("http://ex/B")]);
        assert_eq!(cb.member_count(), 2, "a repeated member does not widen the block");
        assert_eq!(cb.dim(), 3);
    }

    #[test]
    fn encode_into_rejects_length_mismatch() {
        let cb = sample();
        let mut wrong = [0.0f32; 3];
        assert!(cb.encode_into(&iri("http://ex/Red"), &mut wrong).is_err());
        let mut right = [0.0f32; 4];
        assert!(cb.encode_into(&iri("http://ex/Red"), &mut right).is_ok());
    }

    #[test]
    fn all_zero_block_decodes_to_none() {
        let cb = sample();
        assert!(cb.decode(&[0.0; 4]).is_none(), "an all-zero block is not a member");
    }
}
