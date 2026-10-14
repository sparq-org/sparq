// sq-xetf7: RBox (role-box) saturation — CR10 (role inclusion) + CR11 (role
// composition), the EL+ role automaton the spike's Phase E2 adds on top of the E1 CR1-CR5
// concept calculus (research/owl2-el-ql-reasoning-spike.md §5).
//
// EL+ role reasoning is a finite, fully-specified calculus (spike §"Hard parts" (2)). The two
// role completion rules are:
//
//   CR10  r ⊑ s ∈ R,      (X, Y) ∈ R(r)                    ⇒  (X, Y) ∈ R(s)
//   CR11  r1 ∘ r2 ⊑ s ∈ R,  (X, Y) ∈ R(r1),  (Y, Z) ∈ R(r2) ⇒  (X, Z) ∈ R(s)
//
// Transitive roles (`r ∘ r ⊑ r`) and SNOMED right-identity (`r ∘ s ⊑ s`) are special cases of
// CR11, so a single composition table handles all three. CR10 alone is a reachability problem
// over the told-inclusion graph; we precompute its REFLEXIVE-TRANSITIVE CLOSURE once (`r ⊑* s`)
// so the hot saturation loop in `classify` answers "every super-role of r" by a table lookup,
// never a fixpoint over inclusions.
//
// This whole module is `#[cfg(feature = "rbox")]`: the default build (and the lean wasm port)
// carry zero role-automaton code, and every RBox axiom stays in `Report::skipped_axioms`.

use crate::normal::{Role, RoleAxiom};
use rustc_hash::{FxHashMap, FxHashSet};

/// The saturated role box: the reflexive-transitive closure of told role inclusions (CR10) plus
/// the indexed binary composition axioms (CR11). Built once from the extracted [`RoleAxiom`]s,
/// then consulted read-only by the concept saturator.
pub struct RoleBox {
    /// `super_of[r]` = every role `s` with `r ⊑* s` under the told inclusions, INCLUDING `r`
    /// itself (reflexive). So a link `(X, Y) ∈ R(r)` immediately holds for each `s ∈ super_of[r]`.
    super_of: Vec<Vec<Role>>,
    /// Composition axioms keyed by the FIRST component: `comp_by_first[r1]` = `(r2, s)` pairs
    /// with `r1 ∘ r2 ⊑ s`. The keying matches CR11's firing shape — a new link `(X, Y) ∈ R(r1)`
    /// looks for a `(Y, Z) ∈ R(r2)` to compose with.
    comp_by_first: FxHashMap<Role, Vec<(Role, Role)>>,
    /// Composition axioms keyed by the SECOND component: `comp_by_second[r2]` = `(r1, s)` pairs
    /// with `r1 ∘ r2 ⊑ s`. CR11 also fires when the `(Y, Z) ∈ R(r2)` link is the new trigger and
    /// the `(X, Y) ∈ R(r1)` predecessor already exists.
    comp_by_second: FxHashMap<Role, Vec<(Role, Role)>>,
    /// Whether ANY composition axiom exists. When false the box degenerates to a pure role
    /// hierarchy and the saturator can skip every composition probe.
    has_compositions: bool,
}

impl RoleBox {
    /// Builds the saturated role box for `role_count` roles from the told [`RoleAxiom`]s.
    /// Computes the reflexive-transitive inclusion closure (CR10) with a per-role BFS over the
    /// told-inclusion adjacency, and indexes compositions (CR11) on both components. Roles that
    /// have no inclusion/composition axiom still get a singleton reflexive `super_of` row.
    pub fn build(axioms: &[RoleAxiom], role_count: usize) -> RoleBox {
        // Told inclusion adjacency: incl_adj[r] = roles s with `r ⊑ s` asserted (excluding the
        // reflexive self-edge, which the closure adds unconditionally).
        let mut incl_adj: Vec<Vec<Role>> = vec![Vec::new(); role_count];
        let mut comp_by_first: FxHashMap<Role, Vec<(Role, Role)>> = FxHashMap::default();
        let mut comp_by_second: FxHashMap<Role, Vec<(Role, Role)>> = FxHashMap::default();
        let mut has_compositions = false;
        for &ax in axioms {
            match ax {
                RoleAxiom::Sub(r, s) => {
                    if r != s {
                        incl_adj[r as usize].push(s);
                    }
                }
                RoleAxiom::Chain(r1, r2, s) => {
                    has_compositions = true;
                    comp_by_first.entry(r1).or_default().push((r2, s));
                    comp_by_second.entry(r2).or_default().push((r1, s));
                }
            }
        }

        // Reflexive-transitive closure of the inclusion graph, one BFS per role. role_count is
        // the number of object properties in the TBox (tiny vs the concept space), so the
        // quadratic-worst-case closure is cheap and never the bottleneck.
        let mut super_of: Vec<Vec<Role>> = Vec::with_capacity(role_count);
        for r in 0..role_count as Role {
            let mut seen: FxHashSet<Role> = FxHashSet::default();
            let mut stack = vec![r];
            seen.insert(r);
            while let Some(cur) = stack.pop() {
                for &nxt in &incl_adj[cur as usize] {
                    if seen.insert(nxt) {
                        stack.push(nxt);
                    }
                }
            }
            let mut row: Vec<Role> = seen.into_iter().collect();
            row.sort_unstable(); // deterministic emission order.
            super_of.push(row);
        }

        RoleBox {
            super_of,
            comp_by_first,
            comp_by_second,
            has_compositions,
        }
    }

    /// Every super-role of `r` (reflexive-transitive closure of `⊑`), including `r` itself.
    /// A link `(X, Y) ∈ R(r)` holds for each role this returns (CR10).
    #[inline]
    pub fn super_roles(&self, r: Role) -> &[Role] {
        self.super_of
            .get(r as usize)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Composition axioms whose FIRST component is `r1`: `(r2, s)` with `r1 ∘ r2 ⊑ s`.
    #[inline]
    pub fn compositions_first(&self, r1: Role) -> &[(Role, Role)] {
        self.comp_by_first
            .get(&r1)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Composition axioms whose SECOND component is `r2`: `(r1, s)` with `r1 ∘ r2 ⊑ s`.
    #[inline]
    pub fn compositions_second(&self, r2: Role) -> &[(Role, Role)] {
        self.comp_by_second
            .get(&r2)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Whether the box has any composition axiom. When false the saturator skips CR11 entirely
    /// (the common case: a TBox with only `rdfs:subPropertyOf` and no chains/transitivity).
    #[inline]
    pub fn has_compositions(&self) -> bool {
        self.has_compositions
    }

    /// sq-pbz04.2.7: yields every NON-REFLEXIVE `(sub, sup)` role-inclusion pair
    /// from the already-computed told-inclusion closure — every `(r, s)` where `r != s` and
    /// `s ∈ super_of[r]`. This is a READOFF of the closure built by [`RoleBox::build`]; no new
    /// saturation is performed. The pairs cover the full reflexive-transitive closure minus the
    /// self-pairs, matching the `rdfs:subPropertyOf` emission contract in
    /// `crate::classify_graph` (under the `rbox` feature).
    pub fn inclusion_pairs(&self) -> impl Iterator<Item = (Role, Role)> + '_ {
        self.super_of.iter().enumerate().flat_map(|(r, sups)| {
            let r = r as Role;
            sups.iter().filter_map(move |&s| {
                if r != s {
                    Some((r, s))
                } else {
                    None
                }
            })
        })
    }
}

/// sq-oj06v: TOLD-RBox regularity check (cycle-with-composition detection), the E2
/// follow-up honesty gate. The OWL 2 global restrictions require the RBox to be REGULAR: a
/// strict order `<` on roles must admit every told chain axiom `r1 ∘ … ∘ rn ⊑ s` in one of the
/// allowed forms — a `owl:topObjectProperty` superproperty (`s = top`, no constraint — the
/// FIRST case of the normative restriction), transitivity (`n = 2`, `r1 = r2 = s`, no
/// constraint), left identity (`r1 = s`, needs `ri < s` for the rest), right identity
/// (`rn = s`, ditto), or the general form (`ri < s` for every component). The CR10/CR11
/// saturation terminates and stays SOUND on any input, but the EL+ COMPLETENESS argument
/// assumes regularity — so a non-regular RBox must be flagged, never silently classified as if
/// complete.
///
/// `top` is the role id of `owl:topObjectProperty` when it occurs as a role (`None` otherwise):
/// a chain whose superproperty is `top` is unconditionally admissible and contributes no
/// constraint — without the exemption, e.g. `top ∘ top ∘ top ⊑ top` would false-positive via
/// the `s < s` check below.
///
/// `inclusions` are the told `r ⊑ s` axioms (incl. degenerate length-1 chains); `chains` the
/// told n-ary (`n ≥ 2`) composition axioms BEFORE binarization — checking the left-folded
/// binary [`RoleAxiom::Chain`]s instead would false-positive on a told left-identity chain
/// (`s ∘ r2 ∘ r3 ⊑ s` binarizes into an `s → fresh → s` constraint cycle that the told form
/// does not have). Returns `true` iff regular.
///
/// The check builds the constraint digraph — a NON-STRICT edge `r → s` per inclusion, a STRICT
/// edge `r → s` per constrained chain component — and reports non-regular iff some cycle
/// contains a strict edge. Pure inclusion cycles (equivalent roles) stay regular. Inclusion
/// edges PARTICIPATE in the cycle detection, so the check is deliberately CONSERVATIVE: an
/// exotic RBox that feeds a chain super-role back into a component via inclusions is flagged
/// even where the spec's chain-only ordering would admit it — a false "may be incomplete" note
/// there is preferred over ever missing a genuinely non-regular input.
pub fn told_rbox_regular(
    inclusions: &[(Role, Role)],
    chains: &[(Vec<Role>, Role)],
    top: Option<Role>,
) -> bool {
    // adjacency: role -> (successor, is_strict). Strict = a chain (composition) constraint.
    let mut adj: FxHashMap<Role, Vec<(Role, bool)>> = FxHashMap::default();
    let mut strict_edges: FxHashSet<(Role, Role)> = FxHashSet::default();
    for &(r, s) in inclusions {
        if r != s {
            adj.entry(r).or_default().push((s, false));
        }
    }
    for (chain, s) in chains {
        let n = chain.len();
        if n < 2 {
            continue; // degenerate chains are inclusions; the caller routes them there.
        }
        if top == Some(*s) {
            // `… ⊑ owl:topObjectProperty` — the restriction's first case admits ANY chain with
            // the top superproperty, ahead of the self-constraint and cycle checks below.
            continue;
        }
        if n == 2 && chain[0] == *s && chain[1] == *s {
            continue; // transitivity `r ∘ r ⊑ r` — always admissible, no constraint.
        }
        // Weakest admissible form: a left identity (`r1 = s`) exempts the FIRST component,
        // else a right identity (`rn = s`) exempts the LAST; otherwise every component is
        // constrained. (If both ends equal `s`, neither form admits the other end — the
        // constrained slice then still contains `s` and the self-constraint below fires.)
        let constrained: &[Role] = if chain[0] == *s {
            &chain[1..]
        } else if chain[n - 1] == *s {
            &chain[..n - 1]
        } else {
            &chain[..]
        };
        for &r in constrained {
            if r == *s {
                return false; // `s < s` is unsatisfiable in any strict order.
            }
            if strict_edges.insert((r, *s)) {
                adj.entry(r).or_default().push((*s, true));
            }
        }
    }
    // Non-regular iff a strict edge lies on a cycle: for each strict `a -> b`, can `b` reach
    // `a` over ANY edges? Role counts are tiny (object properties, not concepts), so the
    // per-edge BFS is never a bottleneck.
    for &(a, b) in &strict_edges {
        let mut seen: FxHashSet<Role> = FxHashSet::default();
        let mut stack = vec![b];
        seen.insert(b);
        while let Some(cur) = stack.pop() {
            if cur == a {
                return false;
            }
            if let Some(nexts) = adj.get(&cur) {
                for &(nxt, _) in nexts {
                    if seen.insert(nxt) {
                        stack.push(nxt);
                    }
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflexive_transitive_inclusion_closure() {
        // r0 ⊑ r1 ⊑ r2. Each role's super-set is the upward chain incl. itself.
        let rb = RoleBox::build(&[RoleAxiom::Sub(0, 1), RoleAxiom::Sub(1, 2)], 3);
        assert_eq!(rb.super_roles(0), &[0, 1, 2]);
        assert_eq!(rb.super_roles(1), &[1, 2]);
        assert_eq!(rb.super_roles(2), &[2]);
    }

    #[test]
    fn inclusion_cycle_is_safe() {
        // r0 ⊑ r1, r1 ⊑ r0 (equivalent roles). The closure must terminate and contain both.
        let rb = RoleBox::build(&[RoleAxiom::Sub(0, 1), RoleAxiom::Sub(1, 0)], 2);
        assert_eq!(rb.super_roles(0), &[0, 1]);
        assert_eq!(rb.super_roles(1), &[0, 1]);
    }

    #[test]
    fn compositions_indexed_on_both_components() {
        // r0 ∘ r1 ⊑ r2 (a right-identity-flavoured chain).
        let rb = RoleBox::build(&[RoleAxiom::Chain(0, 1, 2)], 3);
        assert!(rb.has_compositions());
        assert_eq!(rb.compositions_first(0), &[(1, 2)]);
        assert_eq!(rb.compositions_second(1), &[(0, 2)]);
        assert!(rb.compositions_first(1).is_empty());
    }

    #[test]
    fn pure_hierarchy_has_no_compositions() {
        let rb = RoleBox::build(&[RoleAxiom::Sub(0, 1)], 2);
        assert!(!rb.has_compositions());
    }

    // sq-pbz04.2.7: unit tests for `inclusion_pairs` (the role-lattice readoff).

    #[test]
    fn inclusion_pairs_chain_closure() {
        // r0 ⊑ r1 ⊑ r2: non-reflexive closure = (0,1), (0,2), (1,2). Self-pairs excluded.
        let rb = RoleBox::build(&[RoleAxiom::Sub(0, 1), RoleAxiom::Sub(1, 2)], 3);
        let mut pairs: Vec<(Role, Role)> = rb.inclusion_pairs().collect();
        pairs.sort_unstable();
        assert_eq!(pairs, vec![(0, 1), (0, 2), (1, 2)], "chain yields 3 non-reflexive pairs");
        // No self-pairs.
        assert!(
            !pairs.iter().any(|&(r, s)| r == s),
            "no self-pair may appear in inclusion_pairs"
        );
    }

    #[test]
    fn inclusion_pairs_mutual_inclusion() {
        // r0 ⊑ r1, r1 ⊑ r0 (equivalent roles): both directions emitted, no self-pairs.
        let rb = RoleBox::build(&[RoleAxiom::Sub(0, 1), RoleAxiom::Sub(1, 0)], 2);
        let mut pairs: Vec<(Role, Role)> = rb.inclusion_pairs().collect();
        pairs.sort_unstable();
        assert_eq!(
            pairs,
            vec![(0, 1), (1, 0)],
            "mutual inclusion emits both directions"
        );
    }

    #[test]
    fn inclusion_pairs_isolated_role() {
        // A role with NO inclusions: only the reflexive self-entry in super_of, so inclusion_pairs
        // yields nothing for it.
        let rb = RoleBox::build(&[], 2);
        assert!(
            rb.inclusion_pairs().next().is_none(),
            "no told inclusions => no inclusion pairs"
        );
    }

    // sq-oj06v: told-RBox regularity check (cycle-with-composition detection).

    #[test]
    fn regular_hierarchy_transitivity_and_chain() {
        // r0 ⊑ r1, transitive r2, chain r3 ∘ r4 ⊑ r5 — the ordinary regular shapes.
        assert!(told_rbox_regular(
            &[(0, 1)],
            &[(vec![2, 2], 2), (vec![3, 4], 5)],
            None
        ));
    }

    #[test]
    fn regular_left_and_right_identity() {
        // SNOMED right identity r0 ∘ r1 ⊑ r1 and left identity r2 ∘ r3 ⊑ r2: both admissible.
        assert!(told_rbox_regular(
            &[],
            &[(vec![0, 1], 1), (vec![2, 3], 2)],
            None
        ));
    }

    #[test]
    fn regular_told_left_identity_long_chain() {
        // s ∘ r1 ∘ r2 ⊑ s (told form 3, admissible: r1 < s, r2 < s). The BINARIZED encoding
        // (s ∘ r1 ⊑ f, f ∘ r2 ⊑ s) LOOKS cyclic (s → f → s) — this pins that the check runs on
        // the TOLD chain, where no cycle exists.
        assert!(told_rbox_regular(&[], &[(vec![0, 1, 2], 0)], None));
    }

    #[test]
    fn regular_equivalent_roles() {
        // A pure inclusion cycle (equivalent roles) is regular — only chain constraints are
        // strict; the closure already handles it (see inclusion_cycle_is_safe above).
        assert!(told_rbox_regular(&[(0, 1), (1, 0)], &[], None));
    }

    #[test]
    fn non_regular_mutual_chain_cycle() {
        // The spec's classic: hasFather ∘ hasBrother ⊑ hasUncle, hasUncle ∘ hasFriend ⊑
        // hasBrother — hasBrother < hasUncle and hasUncle < hasBrother cannot both hold.
        assert!(!told_rbox_regular(
            &[],
            &[(vec![0, 1], 2), (vec![2, 3], 1)],
            None
        ));
    }

    #[test]
    fn non_regular_chain_fed_back_through_inclusion() {
        // r0 ∘ r1 ⊑ r2 with r2 ⊑ r0: the chain constraint r0 < r2 cycles back through the
        // inclusion — flagged (conservative hierarchy-aware detection).
        assert!(!told_rbox_regular(&[(2, 0)], &[(vec![0, 1], 2)], None));
    }

    #[test]
    fn non_regular_ternary_self_chain() {
        // s ∘ s ∘ s ⊑ s: only BINARY transitivity is exempt; every admissible form here still
        // constrains an `s` component (`s < s`), so no strict order exists.
        assert!(!told_rbox_regular(&[], &[(vec![0, 0, 0], 0)], None));
    }

    #[test]
    fn non_regular_both_ends_identity() {
        // s ∘ r ∘ s ⊑ s: the left-identity form still constrains the trailing s (s < s).
        assert!(!told_rbox_regular(&[], &[(vec![0, 1, 0], 0)], None));
    }

    #[test]
    fn top_superproperty_chain_is_exempt() {
        // The restriction's FIRST case: any chain whose superproperty is owl:topObjectProperty
        // is admissible. top ∘ top ∘ top ⊑ top (role 7 = top) would otherwise fail the `s < s`
        // self-constraint — pin that the exemption fires ahead of it.
        assert!(told_rbox_regular(&[], &[(vec![7, 7, 7], 7)], Some(7)));
        // Without the top identity the same told chain is (correctly) non-regular.
        assert!(!told_rbox_regular(&[], &[(vec![7, 7, 7], 7)], None));
        // The exemption also removes would-be cycle constraints: r0 ∘ r1 ⊑ top adds no strict
        // edge, so pairing it with top ∘ r2 ⊑ r1 stays regular (top < r1 alone is satisfiable).
        assert!(told_rbox_regular(
            &[],
            &[(vec![0, 1], 7), (vec![7, 2], 1)],
            Some(7)
        ));
    }

    #[test]
    fn empty_rbox_is_regular() {
        assert!(told_rbox_regular(&[], &[], None));
    }
}
