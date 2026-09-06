//! Dots and causal contexts (`CRDT-DOT-1`, `CRDT-CTX-1`).
//!
//! A causal context denotes a *set of dots*. Its required compact interchange
//! representation is a `clock` of contiguous per-replica prefixes plus a sparse
//! `cloud` of dots above, or in gaps outside, those prefixes. Normalisation
//! repeatedly moves `(r, clock[r] + 1)` from the cloud into the clock; context
//! union is set union followed by normalisation; membership is tested against
//! the *denoted* set, not merely the cloud.

use std::collections::{BTreeMap, BTreeSet};

/// A replica identifier. The proposal uses opaque globally unique byte strings;
/// the bounded model only needs equality/ordering, so a small integer suffices.
pub type ReplicaId = u8;

/// A dot counter. Per `CRDT-DOT-1` it is unsigned, non-zero, and never reused
/// by its replica.
pub type Counter = u64;

/// A dot `(replica, counter)`: the globally unique name of one quad-addition
/// event (`CRDT-DOT-1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dot {
    /// The replica that minted this dot.
    pub replica: ReplicaId,
    /// The (non-zero) position in that replica's durable counter lineage.
    pub counter: Counter,
}

impl Dot {
    /// Construct a dot. `CRDT-DOT-1` requires a non-zero counter; a zero
    /// counter is a model bug, so it panics rather than propagating.
    #[must_use]
    pub fn new(replica: ReplicaId, counter: Counter) -> Self {
        assert!(counter > 0, "CRDT-DOT-1: dot counters are non-zero");
        Self { replica, counter }
    }
}

/// A causal context in the normative `clock`-plus-`cloud` compact form
/// (`CRDT-CTX-1`). All public constructors and mutators keep the value
/// *normalised*, so structural equality of two `CausalContext` values equals
/// equality of their denoted dot sets; `denotation_eq` re-checks that against
/// the fully expanded denotation.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CausalContext {
    /// `clock[r] = n` denotes every dot `(r, 1) ..= (r, n)`.
    clock: BTreeMap<ReplicaId, Counter>,
    /// Dots above, or in gaps outside, the contiguous clock prefixes.
    cloud: BTreeSet<Dot>,
}

impl CausalContext {
    /// The empty context (denotes no dots).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a context from an arbitrary set of dots, normalising the
    /// representation (`CRDT-CTX-1`).
    #[must_use]
    pub fn from_dots<I: IntoIterator<Item = Dot>>(dots: I) -> Self {
        let mut ctx = Self::new();
        ctx.cloud.extend(dots);
        ctx.normalize();
        ctx
    }

    /// Rebuild a context from an explicit `clock`/`cloud` split (e.g. a
    /// deserialised or deliberately de-compacted encoding), normalising it.
    /// `CRDT-CTX-1` requires the denotation to survive any such re-encoding.
    #[must_use]
    pub fn from_parts(clock: BTreeMap<ReplicaId, Counter>, cloud: BTreeSet<Dot>) -> Self {
        let mut ctx = Self { clock, cloud };
        ctx.normalize();
        ctx
    }

    /// `contains(C, d)`: membership in the *denoted* set, not merely the cloud.
    #[must_use]
    pub fn contains(&self, d: Dot) -> bool {
        self.clock.get(&d.replica).copied().unwrap_or(0) >= d.counter || self.cloud.contains(&d)
    }

    /// Observe one dot (set insertion followed by normalisation).
    pub fn insert(&mut self, d: Dot) {
        if !self.contains(d) {
            self.cloud.insert(d);
            self.normalize();
        }
    }

    /// Context union: set union of the denotations followed by normalisation
    /// (`CRDT-JOIN-1` uses this as `normalize(union(CA, CB))`).
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let mut clock = self.clock.clone();
        for (&r, &n) in &other.clock {
            let e = clock.entry(r).or_insert(0);
            *e = (*e).max(n);
        }
        let mut cloud = self.cloud.clone();
        cloud.extend(other.cloud.iter().copied());
        Self::from_parts(clock, cloud)
    }

    /// Normalisation: repeatedly move `(r, clock[r] + 1)` from the cloud into
    /// the clock, then drop cloud dots already covered by a clock prefix.
    fn normalize(&mut self) {
        loop {
            let mut promoted = false;
            let candidates: Vec<Dot> = self.cloud.iter().copied().collect();
            for d in candidates {
                let next = self.clock.get(&d.replica).copied().unwrap_or(0) + 1;
                if d.counter == next {
                    self.clock.insert(d.replica, next);
                    self.cloud.remove(&d);
                    promoted = true;
                }
            }
            if !promoted {
                break;
            }
        }
        let clock = &self.clock;
        self.cloud
            .retain(|d| d.counter > clock.get(&d.replica).copied().unwrap_or(0));
    }

    /// The full denoted dot set. Only usable because the model is bounded;
    /// tests use it as the reference denotation for `CRDT-CTX-1` checks.
    #[must_use]
    pub fn dots(&self) -> BTreeSet<Dot> {
        let mut out = BTreeSet::new();
        for (&r, &n) in &self.clock {
            for c in 1..=n {
                out.insert(Dot { replica: r, counter: c });
            }
        }
        out.extend(self.cloud.iter().copied());
        out
    }

    /// Denotation equality (`CRDT-JOIN-1`: context equality compares denoted
    /// dot sets). Because values stay normalised, this agrees with `==`; tests
    /// assert both to catch a normalisation bug.
    #[must_use]
    pub fn denotation_eq(&self, other: &Self) -> bool {
        self.dots() == other.dots()
    }

    /// The contiguous clock prefixes of the compact form.
    #[must_use]
    pub fn clock(&self) -> &BTreeMap<ReplicaId, Counter> {
        &self.clock
    }

    /// The sparse dot cloud of the compact form.
    #[must_use]
    pub fn cloud(&self) -> &BTreeSet<Dot> {
        &self.cloud
    }

    /// True iff the context denotes no dots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.clock.is_empty() && self.cloud.is_empty()
    }

    /// A lossless but *non-normalised* alternate encoding: the top dot of every
    /// clock prefix is demoted into the cloud. Feeding the result back through
    /// `from_parts` must reproduce an equal context — the compaction round-trip
    /// the `CRDT-CTX-1` fixtures and the `Compact` schedule step exercise.
    #[must_use]
    pub fn decompacted(&self) -> (BTreeMap<ReplicaId, Counter>, BTreeSet<Dot>) {
        let mut clock = BTreeMap::new();
        let mut cloud = self.cloud.clone();
        for (&r, &n) in &self.clock {
            if n > 1 {
                clock.insert(r, n - 1);
            }
            cloud.insert(Dot { replica: r, counter: n });
        }
        (clock, cloud)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_new_rejects_zero_counter() {
        let d = Dot::new(1, 1);
        assert_eq!(d.counter, 1);
        assert!(std::panic::catch_unwind(|| Dot::new(1, 0)).is_err());
    }

    #[test]
    fn insert_and_contains_cover_clock_and_cloud() {
        let mut c = CausalContext::new();
        assert!(c.is_empty());
        c.insert(Dot::new(1, 1));
        c.insert(Dot::new(1, 3)); // gap: stays in the cloud
        assert!(c.contains(Dot::new(1, 1)));
        assert!(!c.contains(Dot::new(1, 2)));
        assert!(c.contains(Dot::new(1, 3)));
        assert_eq!(c.clock().get(&1), Some(&1));
        assert!(c.cloud().contains(&Dot::new(1, 3)));
    }

    #[test]
    fn normalization_promotes_gap_fills_from_cloud() {
        let mut c = CausalContext::from_dots([Dot::new(1, 1), Dot::new(1, 3)]);
        c.insert(Dot::new(1, 2)); // fills the gap: (1,2) then (1,3) promote
        assert_eq!(c.clock().get(&1), Some(&3));
        assert!(c.cloud().is_empty());
    }

    #[test]
    fn from_parts_normalizes_covered_and_promotable_cloud_dots() {
        let clock = BTreeMap::from([(2u8, 2u64)]);
        let cloud = BTreeSet::from([Dot::new(2, 1), Dot::new(2, 3)]);
        let c = CausalContext::from_parts(clock, cloud);
        assert_eq!(c.clock().get(&2), Some(&3));
        assert!(c.cloud().is_empty());
    }

    #[test]
    fn union_is_denotation_union() {
        let a = CausalContext::from_dots([Dot::new(1, 1), Dot::new(2, 2)]);
        let b = CausalContext::from_dots([Dot::new(1, 2), Dot::new(2, 1)]);
        let u = a.union(&b);
        let mut expect: BTreeSet<Dot> = a.dots();
        expect.extend(b.dots());
        assert_eq!(u.dots(), expect);
        assert_eq!(u.clock().get(&1), Some(&2));
        assert_eq!(u.clock().get(&2), Some(&2));
    }

    #[test]
    fn decompacted_round_trip_preserves_denotation() {
        let c = CausalContext::from_dots([Dot::new(1, 1), Dot::new(1, 2), Dot::new(3, 5)]);
        let (clock, cloud) = c.decompacted();
        let back = CausalContext::from_parts(clock, cloud);
        assert!(back.denotation_eq(&c));
        assert_eq!(back, c);
    }

    #[test]
    fn from_dots_and_dots_are_inverse_on_denotations() {
        let dots = BTreeSet::from([Dot::new(1, 1), Dot::new(1, 2), Dot::new(2, 7)]);
        let c = CausalContext::from_dots(dots.iter().copied());
        assert_eq!(c.dots(), dots);
        assert!(c.denotation_eq(&c));
    }
}
