//! Replica state, invariants, and the normative dotted-set join
//! (`CRDT-DATA-1`, `CRDT-STATE-1`, `CRDT-STATE-2`, `CRDT-JOIN-1`).

use std::collections::{BTreeMap, BTreeSet};

use crate::context::{CausalContext, Dot};

/// The graph component of a quad key: the distinguished default graph or a
/// named graph (`CRDT-DATA-1`). The model abstracts the graph IRI as a small
/// identifier; only term equality matters to the algebra.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GraphKey {
    /// The distinguished default graph.
    Default,
    /// A named graph, abstracted to an opaque identifier.
    Named(u8),
}

/// A quad key `(s, p, o, g)` (`CRDT-DATA-1`). Terms are abstracted to opaque
/// identifiers compared by equality, which is all the normative algebra uses;
/// equal triples in different graphs are distinct CRDT elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Quad {
    /// Subject term (opaque model identifier).
    pub s: u8,
    /// Predicate term (opaque model identifier).
    pub p: u8,
    /// Object term (opaque model identifier).
    pub o: u8,
    /// Graph component of the quad key.
    pub g: GraphKey,
}

impl Quad {
    /// Convenience constructor.
    #[must_use]
    pub fn new(s: u8, p: u8, o: u8, g: GraphKey) -> Self {
        Self { s, p, o, g }
    }
}

/// A full replica state `(store, context)` (`CRDT-STATE-1`): `store[q]` is the
/// finite non-empty set of live dots asserting quad `q`, and `context` is the
/// causal context of *all* add dots ever observed, live or removed.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct State {
    store: BTreeMap<Quad, BTreeSet<Dot>>,
    context: CausalContext,
}

/// A data delta is a fragment of the *same* dot-store/causal-context
/// semilattice as full replica state, joined with the same `join`
/// (`CRDT-JOIN-1`); this alias records that identification.
pub type Delta = State;

impl State {
    /// The bottom element (empty store, empty context).
    #[must_use]
    pub fn bottom() -> Self {
        Self::default()
    }

    /// Assemble a state/delta from parts. Empty dot sets are dropped rather
    /// than stored, matching "a missing store entry denotes the empty set".
    #[must_use]
    pub fn from_parts(store: BTreeMap<Quad, BTreeSet<Dot>>, context: CausalContext) -> Self {
        let store = store.into_iter().filter(|(_, d)| !d.is_empty()).collect();
        Self { store, context }
    }

    /// The live dot store.
    #[must_use]
    pub fn store(&self) -> &BTreeMap<Quad, BTreeSet<Dot>> {
        &self.store
    }

    /// The causal context of all observed add dots.
    #[must_use]
    pub fn context(&self) -> &CausalContext {
        &self.context
    }

    /// The exact normative join of `CRDT-JOIN-1`, transcribed from the
    /// proposal's equations:
    ///
    /// ```text
    /// join(A, B):
    ///   C = normalize(union(CA, CB))
    ///   for q in union(keys(SA), keys(SB)):
    ///     both       = intersection(SA[q], SB[q])
    ///     only_a_new = {d in SA[q] where not contains(CB, d)}
    ///     only_b_new = {d in SB[q] where not contains(CA, d)}
    ///     live       = union(both, only_a_new, only_b_new)
    ///     if live is not empty: S[q] = live
    ///   return (S, C)
    /// ```
    ///
    /// A dot present on both sides is retained; a dot present on one side and
    /// *not observed* by the other side's context is a concurrent add and
    /// survives; a dot absent from one store but contained in that side's
    /// context was observed and removed, so it is not retained.
    #[must_use]
    pub fn join(&self, other: &Self) -> Self {
        let context = self.context.union(&other.context);
        let empty = BTreeSet::new();
        let keys: BTreeSet<&Quad> = self.store.keys().chain(other.store.keys()).collect();
        let mut store = BTreeMap::new();
        for &q in &keys {
            let sa = self.store.get(q).unwrap_or(&empty);
            let sb = other.store.get(q).unwrap_or(&empty);
            let both = sa.intersection(sb).copied();
            let only_a_new = sa.iter().copied().filter(|&d| !other.context.contains(d));
            let only_b_new = sb.iter().copied().filter(|&d| !self.context.contains(d));
            let live: BTreeSet<Dot> = both.chain(only_a_new).chain(only_b_new).collect();
            if !live.is_empty() {
                store.insert(*q, live);
            }
        }
        Self { store, context }
    }

    /// The materialised visible dataset (`CRDT-STATE-2`): exactly the keys
    /// whose live dot set is non-empty. Causal metadata never appears here.
    #[must_use]
    pub fn visible(&self) -> BTreeSet<Quad> {
        self.store.keys().copied().collect()
    }

    /// True iff quad `q` is visible.
    #[must_use]
    pub fn is_visible(&self, q: &Quad) -> bool {
        self.store.contains_key(q)
    }

    /// Check the `CRDT-STATE-1` well-formedness invariants: every store dot is
    /// contained in the context; no store entry is empty; one dot occurs under
    /// at most one quad; every dot has a non-zero counter. Violations are
    /// returned as an error so callers can fail closed.
    ///
    /// # Errors
    ///
    /// Returns a description of the first violated invariant.
    pub fn check_invariants(&self) -> Result<(), String> {
        let mut seen: BTreeMap<Dot, Quad> = BTreeMap::new();
        for (q, dots) in &self.store {
            if dots.is_empty() {
                return Err(format!("CRDT-STATE-1: empty dot set stored for {:?}", q));
            }
            for &d in dots {
                if d.counter == 0 {
                    return Err(format!("CRDT-DOT-1: zero counter dot under {:?}", q));
                }
                if !self.context.contains(d) {
                    return Err(format!(
                        "CRDT-STATE-1: live dot {:?} under {:?} not in context",
                        d, q
                    ));
                }
                if let Some(prev) = seen.insert(d, *q) {
                    return Err(format!(
                        "CRDT-STATE-1: dot {:?} under two quads {:?} and {:?}",
                        d, prev, q
                    ));
                }
            }
        }
        Ok(())
    }

    /// Round-trip the causal context through the lossless de-compacted
    /// encoding and re-normalise, verifying the denotation is preserved
    /// (`CRDT-CTX-1`: the exact denotation survives normalisation,
    /// serialisation, snapshot, and merge). Models the `Compact` schedule step.
    ///
    /// # Errors
    ///
    /// Returns an error if the round-trip altered the denotation — a
    /// normalisation bug in the model.
    pub fn recompact_context(&mut self) -> Result<(), String> {
        let (clock, cloud) = self.context.decompacted();
        let back = CausalContext::from_parts(clock, cloud);
        if !back.denotation_eq(&self.context) {
            return Err("CRDT-CTX-1: compaction round-trip changed the denotation".to_owned());
        }
        self.context = back;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: u8) -> Quad {
        Quad::new(s, 0, 0, GraphKey::Default)
    }

    fn add_delta(quad: Quad, d: Dot) -> Delta {
        Delta::from_parts(
            BTreeMap::from([(quad, BTreeSet::from([d]))]),
            CausalContext::from_dots([d]),
        )
    }

    fn remove_delta(dots: impl IntoIterator<Item = Dot>) -> Delta {
        Delta::from_parts(BTreeMap::new(), CausalContext::from_dots(dots))
    }

    #[test]
    fn add_round_trip_makes_quad_visible() {
        let d = Dot::new(1, 1);
        let s = State::bottom().join(&add_delta(q(1), d));
        assert!(s.is_visible(&q(1)));
        assert_eq!(s.visible(), BTreeSet::from([q(1)]));
        assert_eq!(s.store().get(&q(1)), Some(&BTreeSet::from([d])));
        s.check_invariants().unwrap();
    }

    #[test]
    fn observed_remove_hides_quad_but_keeps_context() {
        let d = Dot::new(1, 1);
        let s = State::bottom().join(&add_delta(q(1), d)).join(&remove_delta([d]));
        assert!(!s.is_visible(&q(1)));
        assert!(s.context().contains(d)); // the context is the logical tombstone
        s.check_invariants().unwrap();
    }

    #[test]
    fn concurrent_add_survives_remove_in_both_delivery_orders() {
        // Base: q visible at dot a1 on both sides. A removes observing {a1};
        // B concurrently adds a fresh dot b1 (add-wins: CRDT-MUT-2).
        let a1 = Dot::new(1, 1);
        let b1 = Dot::new(2, 1);
        let base = State::bottom().join(&add_delta(q(1), a1));
        let removal = remove_delta([a1]);
        let concurrent_add = add_delta(q(1), b1);
        let ab = base.join(&removal).join(&concurrent_add);
        let ba = base.join(&concurrent_add).join(&removal);
        assert_eq!(ab, ba);
        assert!(ab.is_visible(&q(1)));
        assert_eq!(ab.store().get(&q(1)), Some(&BTreeSet::from([b1])));
    }

    #[test]
    fn graph_component_separates_equal_triples() {
        let dg = Quad::new(1, 2, 3, GraphKey::Default);
        let ng = Quad::new(1, 2, 3, GraphKey::Named(7));
        let s = State::bottom()
            .join(&add_delta(dg, Dot::new(1, 1)))
            .join(&add_delta(ng, Dot::new(1, 2)));
        // Removing the named-graph element leaves the default-graph one intact.
        let s = s.join(&remove_delta([Dot::new(1, 2)]));
        assert!(s.is_visible(&dg));
        assert!(!s.is_visible(&ng));
    }

    #[test]
    fn join_is_idempotent_on_states() {
        let s = State::bottom()
            .join(&add_delta(q(1), Dot::new(1, 1)))
            .join(&remove_delta([Dot::new(1, 1)]))
            .join(&add_delta(q(2), Dot::new(2, 1)));
        assert_eq!(s.join(&s), s);
    }

    #[test]
    fn from_parts_drops_empty_dot_sets() {
        let s = State::from_parts(
            BTreeMap::from([(q(1), BTreeSet::new())]),
            CausalContext::new(),
        );
        assert!(s.store().is_empty());
        assert!(s.context().is_empty());
        assert_eq!(s, State::bottom());
    }

    #[test]
    fn invariants_reject_dot_under_two_quads_and_uncovered_dot() {
        let d = Dot::new(1, 1);
        let two = State::from_parts(
            BTreeMap::from([(q(1), BTreeSet::from([d])), (q(2), BTreeSet::from([d]))]),
            CausalContext::from_dots([d]),
        );
        assert!(two.check_invariants().is_err());
        let uncovered = State::from_parts(
            BTreeMap::from([(q(1), BTreeSet::from([d]))]),
            CausalContext::new(),
        );
        assert!(uncovered.check_invariants().is_err());
    }

    #[test]
    fn recompact_context_preserves_state_meaning() {
        let mut s = State::bottom()
            .join(&add_delta(q(1), Dot::new(1, 1)))
            .join(&add_delta(q(2), Dot::new(1, 2)));
        let before = s.clone();
        s.recompact_context().unwrap();
        assert_eq!(s, before);
    }
}
