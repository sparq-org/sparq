// sq-evb1: EL concept IR + Baader–Brandt–Lutz normalization.
//
// The classifier works over a SMALL integer index space SEPARATE from the sparq `Dict` id
// space: a `Concept` is either a named class (carrying the original dict `Id` so the result
// lattice can be re-emitted), the top concept ⊤, the bottom concept ⊥, or a FRESH conjunct/
// existential name introduced by normalization. Keeping a private index space (instead of
// interning fresh names back into the Dict) means normalization never pollutes the store's
// dictionary, and the saturation runs over dense `u32` keys.

use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::Id;

/// A `Concept` is a node in the internal classification index space. It is NOT a dict id —
/// it is a dense `u32` assigned by [`Names`]. Named concepts remember their dict id so the
/// emitted lattice can name them; ⊤/⊥ and fresh normalization names have none.
pub type Concept = u32;

/// A role (object property) in the internal index space. Like [`Concept`] this is a dense
/// `u32` separate from dict ids. Without the `rbox` feature a role is only ever compared for
/// equality (no role hierarchy). With `rbox` (Phase E2, bead sq-xetf7) the saturator reasons
/// over role inclusions and compositions via the [`RoleBox`] role automaton.
pub type Role = u32;

/// An RBox (role-box) axiom in normal form — the two EL+ role forms the Baader–Brandt–Lutz
/// calculus adds on top of the four concept forms (spike §5, CR10/CR11). Only built when the
/// `rbox` feature is on; the MVP (E1) drops every role axiom into [`crate::Report::skipped_axioms`].
#[cfg(feature = "rbox")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoleAxiom {
    /// `r ⊑ s` — a simple role inclusion (`rdfs:subPropertyOf` between object properties).
    /// `owl:TransitiveProperty(r)` is normalized to the composition `r ∘ r ⊑ r`, NOT this form.
    Sub(Role, Role),
    /// `r1 ∘ r2 ⊑ s` — a (binary) role composition. An `owl:propertyChainAxiom` of length `n`
    /// is left-folded into `n-1` of these over fresh intermediate roles; a transitive role is
    /// the special case `r ∘ r ⊑ r`; SNOMED right-identity is `r ∘ s ⊑ s`.
    Chain(Role, Role, Role),
}

/// The internal name table: a bijection between the dense classification index space and the
/// source dict ids, plus a counter that mints fresh (anonymous) concepts for normalization.
///
/// ⊤ (top) is ALWAYS concept `0` and ⊥ (bottom) is ALWAYS concept `1`, so the saturator can
/// special-case them without a map lookup. Fresh names get no dict id (they are structural).
#[derive(Default)]
pub struct Names {
    /// `dict Id -> internal Concept` for named classes.
    by_dict: FxHashMap<Id, Concept>,
    /// `internal Concept -> dict Id` for named classes (None for ⊤/⊥/fresh names/nominals).
    to_dict: Vec<Option<Id>>,
    /// `dict Id -> internal Role` for named object properties.
    role_by_dict: FxHashMap<Id, Role>,
    /// `internal Role -> dict Id`.
    role_to_dict: Vec<Id>,
    /// sq-pbz04.2.1: `individual dict Id -> internal Concept` for nominals `{a}`
    /// (`owl:oneOf` singletons / `owl:hasValue` values). SEPARATE from `by_dict` so a punned
    /// id (used both as a class and as an individual) yields two distinct concepts, matching
    /// OWL 2 direct-semantics punning. A nominal concept has NO dict id in `to_dict` (it must
    /// never be emitted as a named class in the subsumption lattice).
    nominal_by_dict: FxHashMap<Id, Concept>,
    /// The set of concepts that are nominals (for the CR6 saturation pass), plus the mint-order
    /// list used as the roots of the "reachable from a nominal" non-emptiness search.
    nominal_set: FxHashSet<Concept>,
    nominal_list: Vec<Concept>,
    /// sq-pbz04.2.6: `role -> ∃role.Self` self-restriction concept (`owl:hasSelf`).
    /// A self-concept is a basic atom with NO dict id (like a nominal, it must never surface as a
    /// named class); the completion rules CR-Self-1/CR-Self-2 (see `classify.rs`) connect its
    /// membership `∃r.Self ∈ S(X)` to the reflexive role link `(X,X) ∈ R(r)`.
    self_by_role: FxHashMap<Role, Concept>,
    /// `self-concept -> its role` — the inverse of `self_by_role`, driving the CR-Self-1 worklist
    /// trigger (a derived `∃r.Self ∈ S(X)` must add the self-loop link) and the ABox self-loop
    /// realisation readoff.
    self_of_concept: FxHashMap<Concept, Role>,
}

/// The internal concept index for ⊤ (`owl:Thing`).
pub const TOP: Concept = 0;
/// The internal concept index for ⊥ (`owl:Nothing`).
pub const BOTTOM: Concept = 1;

impl Names {
    /// A fresh table pre-seeded with ⊤ at [`TOP`] and ⊥ at [`BOTTOM`].
    pub fn new() -> Names {
        Names {
            to_dict: vec![None, None],
            ..Names::default()
        }
    }

    /// Maps a dict class id to its internal concept, minting one on first sight. ⊤/⊥ must be
    /// mapped via [`Names::map_class`] too: the caller seeds them by recognising the
    /// `owl:Thing`/`owl:Nothing` dict ids and routing to [`TOP`]/[`BOTTOM`] before calling
    /// here (see [`crate::extract`]).
    pub fn class(&mut self, dict_id: Id) -> Concept {
        if let Some(&c) = self.by_dict.get(&dict_id) {
            return c;
        }
        let c = self.to_dict.len() as Concept;
        self.to_dict.push(Some(dict_id));
        self.by_dict.insert(dict_id, c);
        c
    }

    /// Maps a dict object-property id to its internal role, minting one on first sight.
    pub fn role(&mut self, dict_id: Id) -> Role {
        if let Some(&r) = self.role_by_dict.get(&dict_id) {
            return r;
        }
        let r = self.role_to_dict.len() as Role;
        self.role_to_dict.push(dict_id);
        self.role_by_dict.insert(dict_id, r);
        r
    }

    /// Mints a fresh anonymous role (no dict id) — used to left-fold an `owl:propertyChainAxiom`
    /// of length `n` into `n-1` binary [`RoleAxiom::Chain`] forms over intermediate roles. The
    /// fresh role uses a sentinel dict id (`Id::MAX`) that no real property interns to; it never
    /// appears in an emitted link because chain heads are always named roles.
    #[cfg(feature = "rbox")]
    pub fn fresh_role(&mut self) -> Role {
        let r = self.role_to_dict.len() as Role;
        self.role_to_dict.push(Id::MAX);
        r
    }

    /// The number of roles minted so far (the dense role-index upper bound). Used to size the
    /// role-saturation tables in [`RoleBox`].
    #[cfg(feature = "rbox")]
    pub fn role_count(&self) -> usize {
        self.role_to_dict.len()
    }

    /// Mints a fresh anonymous concept (a normalization name with no dict id).
    pub fn fresh(&mut self) -> Concept {
        let c = self.to_dict.len() as Concept;
        self.to_dict.push(None);
        c
    }

    /// sq-pbz04.2.1: maps an individual's dict id to its NOMINAL concept `{a}`,
    /// minting one on first sight. The same individual (dict id) always yields the same
    /// concept, so `owl:hasValue :a` and `owl:someValuesFrom [owl:oneOf (:a)]` unify on one
    /// filler. The concept carries NO dict id (`dict_of` returns `None`): a nominal is not a
    /// named class and must never surface in the emitted `rdfs:subClassOf` lattice.
    pub fn nominal(&mut self, dict_id: Id) -> Concept {
        if let Some(&c) = self.nominal_by_dict.get(&dict_id) {
            return c;
        }
        let c = self.to_dict.len() as Concept;
        self.to_dict.push(None);
        self.nominal_by_dict.insert(dict_id, c);
        self.nominal_set.insert(c);
        self.nominal_list.push(c);
        c
    }

    /// Whether `c` is a nominal concept `{a}` (the CR6 trigger test).
    pub fn is_nominal(&self, c: Concept) -> bool {
        self.nominal_set.contains(&c)
    }

    /// Whether ANY nominal was minted — the O(1) fast-path guard that keeps the CR6 pass a
    /// no-op (zero scans) on nominal-free ontologies, so the SNOMED/GO-shaped scaling bound
    /// is unaffected by this rule.
    pub fn has_nominals(&self) -> bool {
        !self.nominal_list.is_empty()
    }

    /// The minted nominal concepts, in mint order — the roots of CR6's "reachable from a
    /// nominal ⇒ provably non-empty" search (a nominal `{a}` always denotes the non-empty
    /// set containing `a`).
    pub fn nominal_concepts(&self) -> &[Concept] {
        &self.nominal_list
    }

    /// sq-pbz04.2.6: maps a role to its self-restriction concept `∃r.Self`
    /// (`owl:hasSelf`), minting one on first sight. Carries NO dict id (`dict_of` returns
    /// `None`): a self-concept is a basic atom in the EL++ normal form, never a named class in
    /// the emitted lattice. The same role always yields the same concept, so `∃r.Self` on both
    /// sides of a `⊑` unifies on one atom (CR1 then threads `∃r.Self ⊑ D`, i.e. CR-Self-2).
    pub fn self_concept(&mut self, role: Role) -> Concept {
        if let Some(&c) = self.self_by_role.get(&role) {
            return c;
        }
        let c = self.to_dict.len() as Concept;
        self.to_dict.push(None);
        self.self_by_role.insert(role, c);
        self.self_of_concept.insert(c, role);
        c
    }

    /// The role `r` iff `c` is the self-restriction concept `∃r.Self` — the CR-Self-1 trigger
    /// test (a derived `∃r.Self ∈ S(X)` fires the reflexive link `(X,X) ∈ R(r)`); `None`
    /// otherwise. Also drives the ABox self-loop realisation readoff (`a r a`).
    pub fn self_role(&self, c: Concept) -> Option<Role> {
        self.self_of_concept.get(&c).copied()
    }

    /// sq-8zqwb: the self-restriction concept `∃role.Self` IF one was minted for
    /// `role` (non-minting, unlike [`Names::self_concept`]) — the CRs3 nominal-reflexivity
    /// trigger test: a SAME-NOMINAL self-link `({a},{a}) ∈ R(r)` reads off as `∃r.Self ∈
    /// S({a})`. Returns `None` on a `hasSelf`-free ontology, so that path's cost is unchanged.
    pub fn self_concept_of(&self, role: Role) -> Option<Concept> {
        self.self_by_role.get(&role).copied()
    }

    /// Whether ANY self-restriction was minted — the O(1) fast-path guard that keeps the CR-Self
    /// rules a no-op (zero cost) on `owl:hasSelf`-free ontologies, so hasSelf-free classification
    /// is byte-identical in behaviour AND cost to the pre-CR-Self path.
    pub fn has_self_restrictions(&self) -> bool {
        !self.self_by_role.is_empty()
    }

    /// The dict id of a concept, if it is a NAMED class (⊤/⊥/fresh names return `None`).
    pub fn dict_of(&self, c: Concept) -> Option<Id> {
        self.to_dict.get(c as usize).copied().flatten()
    }

    /// sq-pbz04.2.5 (`abox`): every minted nominal as `(individual dict id, concept)`.
    /// The bijection `nominal_by_dict` drives the ABox realisation readoff (per-individual typing
    /// plus the concept→individual reverse map for `owl:sameAs`); it spans BOTH the
    /// assertion-minted individuals (`abox`) and any TBox `owl:hasValue`/singleton-`owl:oneOf`
    /// nominals.
    #[cfg(feature = "abox")]
    pub fn nominals(&self) -> impl Iterator<Item = (Id, Concept)> + '_ {
        self.nominal_by_dict.iter().map(|(&id, &c)| (id, c))
    }

    /// sq-pbz04.2.5 (`abox`): the internal role for `dict_id` IF one was already
    /// minted, WITHOUT minting a new one (unlike [`Names::role`]). Used to append the
    /// `owl:bottomObjectProperty` empty-role axiom only when that property actually occurs,
    /// and by the regularity check (`rbox`) to resolve `owl:topObjectProperty` only when it
    /// occurs as a role.
    #[cfg(any(feature = "abox", feature = "rbox"))]
    pub fn role_of(&self, dict_id: Id) -> Option<Role> {
        self.role_by_dict.get(&dict_id).copied()
    }

    /// sq-pbz04.2.6 (`abox`): the source dict id of a role, if it is a NAMED object
    /// property. Used by the self-loop realisation readoff (`abox`) to name the predicate of a
    /// derived `a r a` assertion, and by the role-lattice readoff (`rbox`, sq-pbz04.2.7) to map
    /// role-index pairs back to dict ids for `rdfs:subPropertyOf` emission. A fresh anonymous
    /// chain role (`rbox`, sentinel `Id::MAX`) has no dict id and returns `None`; a
    /// self-restriction's role is always a named property, so this resolves for every self-loop
    /// the `abox` readoff emits.
    #[cfg(any(feature = "abox", feature = "rbox"))]
    pub fn role_dict_of(&self, r: Role) -> Option<Id> {
        match self.role_to_dict.get(r as usize).copied() {
            Some(id) if id != Id::MAX => Some(id),
            _ => None,
        }
    }

    /// The number of concepts minted so far (the dense index upper bound).
    pub fn concept_count(&self) -> usize {
        self.to_dict.len()
    }
}

/// A general concept inclusion in EL normal form (Baader–Brandt–Lutz, "Pushing the EL
/// Envelope", IJCAI-05). Every EL+⊥ TBox axiom reduces to a multiset of these four forms via
/// [`Normalizer`]; the saturator ([`crate::classify`]) consumes exactly this enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Normal {
    /// `C ⊑ D` — a basic concept on each side (NF1).
    Sub(Concept, Concept),
    /// `C1 ⊓ C2 ⊑ D` (NF2).
    AndSub(Concept, Concept, Concept),
    /// `C ⊑ ∃r.D` (NF3).
    SubExists(Concept, Role, Concept),
    /// `∃r.C ⊑ D` (NF4).
    ExistsSub(Role, Concept, Concept),
}

/// A concept expression as extracted from the graph, BEFORE normalization. EL+⊥ admits only
/// these constructors; anything else makes the axiom non-EL and is dropped by the extractor
/// with a recorded skip (see [`crate::Report`]).
#[derive(Clone, Debug)]
pub enum Expr {
    /// A named class, ⊤, or ⊥ (already an atomic concept).
    Atom(Concept),
    /// `C1 ⊓ … ⊓ Cn` (`owl:intersectionOf`). Always length ≥ 2 by construction.
    And(Vec<Expr>),
    /// `∃r.C` (`owl:someValuesFrom` restriction).
    Exists(Role, Box<Expr>),
}

/// Normalizes EL+⊥ concept inclusions into [`Normal`] forms, minting fresh names through a
/// borrowed [`Names`]. Structural normalization is conservative (the introduced axioms entail
/// exactly the input over the original signature) and linear in the input size.
pub struct Normalizer<'a> {
    pub(crate) names: &'a mut Names,
    out: Vec<Normal>,
    /// Memoizes `∃r.C ⊑ X` and `X ⊑ ∃r.C` fresh names so identical sub-expressions reuse one
    /// fresh concept (keeps the index space small on restriction-heavy ontologies).
    exists_name: FxHashMap<(Role, Concept), Concept>,
}

impl<'a> Normalizer<'a> {
    pub fn new(names: &'a mut Names) -> Normalizer<'a> {
        Normalizer {
            names,
            out: Vec::new(),
            exists_name: FxHashMap::default(),
        }
    }

    /// Consumes the normalizer, returning the accumulated normal-form axioms.
    pub fn finish(self) -> Vec<Normal> {
        self.out
    }

    /// Adds the inclusion `lhs ⊑ rhs` (both general [`Expr`]s) to the output in normal form.
    pub fn add_sub(&mut self, lhs: &Expr, rhs: &Expr) {
        // Reduce each side to a basic concept (a named atom / ⊤ / ⊥ / fresh name), emitting
        // the structural axioms that DEFINE the fresh name, then connect them with one Sub /
        // AndSub. A conjunction on the LHS is handled specially to land directly on AndSub.
        match lhs {
            Expr::And(parts) if parts.len() >= 2 => {
                let d = self.rhs_concept(rhs);
                self.and_lhs(parts, d);
            }
            _ => {
                let c = self.lhs_concept(lhs);
                let d = self.rhs_concept(rhs);
                self.out.push(Normal::Sub(c, d));
            }
        }
    }

    /// Encodes `C1 ⊓ … ⊓ Cn ⊑ D` by left-folding into binary `AndSub` over fresh names:
    /// `(C1 ⊓ C2) ⊑ f1`, `(f1 ⊓ C3) ⊑ f2`, …, finally `(f_{n-2} ⊓ Cn) ⊑ D`.
    fn and_lhs(&mut self, parts: &[Expr], d: Concept) {
        // Each part is first reduced to a basic concept (LHS position: existentials become
        // ∃r.C ⊑ fresh, so the conjunct is the fresh name).
        let atoms: Vec<Concept> = parts.iter().map(|p| self.lhs_concept(p)).collect();
        debug_assert!(atoms.len() >= 2);
        let mut acc = atoms[0];
        for &next in &atoms[1..atoms.len() - 1] {
            let f = self.names.fresh();
            self.out.push(Normal::AndSub(acc, next, f));
            acc = f;
        }
        let last = atoms[atoms.len() - 1];
        self.out.push(Normal::AndSub(acc, last, d));
    }

    /// Reduces a concept appearing in a SUBclass (LHS) position to a basic concept, emitting
    /// the defining axioms. In LHS position an existential `∃r.C` is named by a fresh `X` with
    /// `∃r.C ⊑ X` (NF4 direction), and a nested conjunction by fresh AndSub chains.
    fn lhs_concept(&mut self, e: &Expr) -> Concept {
        match e {
            Expr::Atom(c) => *c,
            Expr::And(parts) if parts.len() >= 2 => {
                // (C1 ⊓ … ⊓ Cn) as a sub-CONCEPT: name it X with each Ci ⊑-chain ⊑ X is wrong;
                // instead introduce X with `C1 ⊓ … ⊓ Cn ⊑ X` AND, for soundness in LHS reuse,
                // `X ⊑ Ci`. The MVP only needs the ⊑ X direction (LHS occurrence), so emit the
                // AndSub chain that lands on a fresh X.
                let x = self.names.fresh();
                self.and_lhs(parts, x);
                x
            }
            Expr::And(parts) => {
                // Degenerate 0/1-element conjunction: identity / single atom.
                parts.first().map(|p| self.lhs_concept(p)).unwrap_or(TOP)
            }
            Expr::Exists(r, inner) => {
                let c = self.rhs_concept(inner); // filler in a positive (RHS-like) position
                let key = (*r, c);
                if let Some(&x) = self.exists_name.get(&key) {
                    return x;
                }
                let x = self.names.fresh();
                self.exists_name.insert(key, x);
                self.out.push(Normal::ExistsSub(*r, c, x));
                x
            }
        }
    }

    /// Reduces a concept appearing in a SUPERclass (RHS) position to a basic concept. In RHS
    /// position an existential `∃r.C` is named by a fresh `X` with `X ⊑ ∃r.C` (NF3), the
    /// filler `C` recursively reduced. A conjunction `D1 ⊓ … ⊓ Dn` on the RHS becomes `n`
    /// separate inclusions sharing the same subclass (handled by the caller via fresh fan-out).
    fn rhs_concept(&mut self, e: &Expr) -> Concept {
        match e {
            Expr::Atom(c) => *c,
            Expr::And(parts) => {
                // Name the conjunction X with `X ⊑ Di` for each part; return X. This is the
                // RHS conjunction split: a subclass of the conjunction is a subclass of each.
                let x = self.names.fresh();
                for p in parts {
                    let d = self.rhs_concept(p);
                    self.out.push(Normal::Sub(x, d));
                }
                x
            }
            Expr::Exists(r, inner) => {
                let c = self.rhs_concept(inner);
                let x = self.names.fresh();
                self.out.push(Normal::SubExists(x, *r, c));
                x
            }
        }
    }
}
