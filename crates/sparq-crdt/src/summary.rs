//! Compact causal summaries: a version-vector `clock` plus a sparse dot
//! `cloud` (research §4.1, proposal `CRDT-CTX-1`).
//!
//! The same representation serves two namespaces that must never be
//! mixed: **data** causal contexts (over quad-addition [`Dot`]s) and
//! **envelope-identity** journal frontiers (over `(origin, sequence)`
//! identities). The invariant maintained everywhere is *normalisation*:
//! `(r, clock[r] + 1)` is always absorbed into the clock, and no cloud dot is
//! at or below its replica's clock, so equal denotations have equal
//! representations and `==` is denotation equality.

use crate::CrdtError;
use crate::id::{Dot, ReplicaId};
use std::collections::{BTreeMap, BTreeSet};

/// A compact, always-normalised set of observed dots.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CausalSummary {
    clock: BTreeMap<ReplicaId, u64>,
    cloud: BTreeSet<Dot>,
}

impl CausalSummary {
    /// The empty summary (no observed dots).
    pub fn new() -> Self {
        CausalSummary::default()
    }

    /// Builds a summary from decoded interchange parts, rejecting a zero
    /// clock entry or a cloud that is not in normalised form (`CRDT-CTX-1`
    /// requires the interchange encoding to be the normalised one; accepting
    /// a second representation of the same denotation would break the
    /// byte-canonicality of every enclosing document).
    pub fn from_parts(
        clock: BTreeMap<ReplicaId, u64>,
        cloud: Vec<Dot>,
    ) -> Result<Self, CrdtError> {
        if clock.values().any(|&n| n == 0) {
            return Err(CrdtError::NonCanonical {
                reason: "a clock entry of 0 must be omitted".into(),
            });
        }
        let candidate = CausalSummary {
            clock,
            cloud: cloud.into_iter().collect(),
        };
        let mut normalised = candidate.clone();
        normalised.normalise();
        if normalised != candidate {
            return Err(CrdtError::NonCanonical {
                reason: "causal summary is not in normalised clock+cloud form".into(),
            });
        }
        Ok(candidate)
    }

    /// The contiguous-prefix component: `clock[r] = n` denotes dots
    /// `(r, 1) ..= (r, n)`.
    pub fn clock(&self) -> &BTreeMap<ReplicaId, u64> {
        &self.clock
    }

    /// The sparse component: observed dots above (or in gaps outside) the
    /// contiguous prefixes, ordered by raw replica bytes then counter.
    pub fn cloud(&self) -> &BTreeSet<Dot> {
        &self.cloud
    }

    /// Membership in the *denoted* set (clock prefix or cloud), not merely in
    /// the cloud.
    pub fn contains(&self, dot: &Dot) -> bool {
        self.clock.get(dot.replica()).copied().unwrap_or(0) >= dot.counter()
            || self.cloud.contains(dot)
    }

    /// Observes one dot, re-normalising afterwards. Idempotent.
    pub fn insert(&mut self, dot: Dot) {
        if self.contains(&dot) {
            return;
        }
        self.cloud.insert(dot);
        self.normalise();
    }

    /// Set-unions another summary into this one (denotation union followed by
    /// normalisation). Commutative, associative, idempotent.
    pub fn union(&mut self, other: &CausalSummary) {
        for (replica, &n) in &other.clock {
            let entry = self.clock.entry(replica.clone()).or_insert(0);
            if *entry < n {
                *entry = n;
            }
        }
        for dot in &other.cloud {
            self.cloud.insert(dot.clone());
        }
        self.normalise();
    }

    /// True iff no dot is denoted.
    pub fn is_empty(&self) -> bool {
        self.clock.is_empty() && self.cloud.is_empty()
    }

    /// Restores the normal form: absorb `(r, clock[r]+1)` cloud dots into the
    /// clock and drop cloud dots at or below the clock. One pass over the
    /// ordered cloud suffices because dots sort by (replica, counter).
    fn normalise(&mut self) {
        let cloud = std::mem::take(&mut self.cloud);
        for dot in cloud {
            let current = self.clock.get(dot.replica()).copied().unwrap_or(0);
            if dot.counter() <= current {
                continue; // already denoted by the clock
            }
            if dot.counter() == current + 1 {
                self.clock.insert(dot.replica().clone(), dot.counter());
            } else {
                self.cloud.insert(dot);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rid(bytes: &[u8]) -> ReplicaId {
        ReplicaId::new(bytes.to_vec()).expect("valid replica id")
    }

    fn dot(r: &[u8], c: u64) -> Dot {
        Dot::new(rid(r), c).expect("valid dot")
    }

    #[test]
    fn new_is_empty() {
        let s = CausalSummary::new();
        assert!(s.is_empty());
        assert!(s.clock().is_empty());
        assert!(s.cloud().is_empty());
    }

    #[test]
    fn insert_contiguous_dots_grow_the_clock() {
        let mut s = CausalSummary::new();
        s.insert(dot(b"a", 1));
        s.insert(dot(b"a", 2));
        assert_eq!(s.clock().get(&rid(b"a")), Some(&2));
        assert!(s.cloud().is_empty());
        assert!(!s.is_empty());
    }

    #[test]
    fn insert_gapped_dot_stays_in_cloud_until_the_gap_fills() {
        let mut s = CausalSummary::new();
        s.insert(dot(b"a", 1));
        s.insert(dot(b"a", 3));
        assert_eq!(s.clock().get(&rid(b"a")), Some(&1));
        assert!(s.cloud().contains(&dot(b"a", 3)));
        // Filling the gap absorbs the whole run into the clock.
        s.insert(dot(b"a", 2));
        assert_eq!(s.clock().get(&rid(b"a")), Some(&3));
        assert!(s.cloud().is_empty());
    }

    #[test]
    fn insert_is_idempotent() {
        let mut s = CausalSummary::new();
        s.insert(dot(b"a", 1));
        let before = s.clone();
        s.insert(dot(b"a", 1));
        assert_eq!(s, before);
    }

    #[test]
    fn contains_tests_the_denoted_set_not_just_the_cloud() {
        let mut s = CausalSummary::new();
        s.insert(dot(b"a", 1));
        s.insert(dot(b"a", 2));
        s.insert(dot(b"a", 5));
        assert!(s.contains(&dot(b"a", 1))); // clock prefix
        assert!(s.contains(&dot(b"a", 2))); // clock frontier
        assert!(s.contains(&dot(b"a", 5))); // cloud
        assert!(!s.contains(&dot(b"a", 3)));
        assert!(!s.contains(&dot(b"a", 4)));
        assert!(!s.contains(&dot(b"b", 1)));
    }

    #[test]
    fn union_is_commutative_associative_idempotent_on_samples() {
        let mut a = CausalSummary::new();
        a.insert(dot(b"a", 1));
        a.insert(dot(b"a", 2));
        a.insert(dot(b"b", 4));
        let mut b = CausalSummary::new();
        b.insert(dot(b"a", 3));
        b.insert(dot(b"b", 1));
        let mut c = CausalSummary::new();
        c.insert(dot(b"b", 2));
        c.insert(dot(b"b", 3));

        let mut ab = a.clone();
        ab.union(&b);
        let mut b_then_a = b.clone();
        b_then_a.union(&a);
        assert_eq!(ab, b_then_a); // commutative

        let mut ab_c = ab.clone();
        ab_c.union(&c);
        let mut bc = b.clone();
        bc.union(&c);
        let mut a_bc = a.clone();
        a_bc.union(&bc);
        assert_eq!(ab_c, a_bc); // associative

        let mut aa = a.clone();
        aa.union(&a);
        assert_eq!(aa, a); // idempotent

        // The (b,1)+(b,2)+(b,3)+(b,4) run collapses into the clock.
        assert_eq!(ab_c.clock().get(&rid(b"b")), Some(&4));
        assert!(ab_c.cloud().is_empty());
    }

    #[test]
    fn from_parts_accepts_normal_form_and_rejects_everything_else() {
        let mut clock = BTreeMap::new();
        clock.insert(rid(b"a"), 3);
        assert!(CausalSummary::from_parts(clock.clone(), vec![dot(b"a", 5)]).is_ok());
        // clock+1 must have been absorbed.
        assert!(CausalSummary::from_parts(clock.clone(), vec![dot(b"a", 4)]).is_err());
        // At or below the clock is redundant.
        assert!(CausalSummary::from_parts(clock.clone(), vec![dot(b"a", 3)]).is_err());
        // (r, 1) with no clock entry normalises into the clock.
        assert!(CausalSummary::from_parts(BTreeMap::new(), vec![dot(b"a", 1)]).is_err());
        // Zero clock entries are forbidden.
        let mut zero = BTreeMap::new();
        zero.insert(rid(b"a"), 0);
        assert!(CausalSummary::from_parts(zero, Vec::new()).is_err());
    }

    #[test]
    fn equality_is_denotation_equality_via_normal_form() {
        let mut a = CausalSummary::new();
        a.insert(dot(b"a", 2));
        a.insert(dot(b"a", 1));
        let mut b = CausalSummary::new();
        b.insert(dot(b"a", 1));
        b.insert(dot(b"a", 2));
        assert_eq!(a, b);
    }
}
