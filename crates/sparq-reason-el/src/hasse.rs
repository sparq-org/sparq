// sq-s2nob: Phase E3 — transitive reduction to the DIRECT-subsumer Hasse diagram,
// with equivalence-class collapse and a membership-indexed reduction step.
//
// The E1/E2 saturator ([`crate::classify`]) produces, for every named class C, the COMPLETE set
// of named subsumers S(C) = { D : C ⊑ D }. That is the full transitive closure — exactly what a
// `is_subclass_of` query needs, but O(N²) edges to MATERIALIZE on a deep taxonomy (a chain of
// depth N has N·(N+1)/2 closure edges). A class HIERARCHY (the thing a user browses, and the
// `rdfs:subClassOf` taxonomy most ontology tools want) is the *transitive reduction*: the
// minimal edge set whose transitive closure is S — i.e. only the DIRECT (immediate) subsumers.
//
// ELK ("The Incredible ELK", Kazakov–Krötzsch–Simančík, JAR 2014, §"taxonomy construction")
// emits exactly this Hasse diagram, plus equivalence classes (mutually-subsuming cliques). This
// module reproduces that step over sparq's `(Dict, Id)` lattice:
//
//   1. EQUIVALENCE collapse — C ≡ D iff C ⊑ D and D ⊑ C. Group named classes into equivalence
//      cliques; pick the smallest dict id as each clique's representative. The subsumption DAG
//      over representatives is acyclic (a partial order), the precondition for a well-defined
//      transitive reduction.
//   2. TRANSITIVE REDUCTION over the representative DAG — a (strict) super-rep D of rep C is
//      DIRECT iff no OTHER strict super-rep E of C also has E ⊑ D. Equivalently: D is direct iff
//      it is not implied by any other super-rep of C. We test this with an O(1) membership query
//      against the precomputed subsumer HASHSET of C (the "join cache" / indexing this phase adds
//      — without it the inner check would re-scan S(E) for every pair, O(N³)).
//
// The result is DETERMINISTIC (representative = min dict id; outputs sorted by dict id), so the
// emitted-edge COUNT is a hard, load-robust assertion target — the E3 acceptance gate — while
// wall-clock stays advisory (spike §5 E3). This whole module is behind the default-OFF `hasse`
// feature: the E1/E2 build carries zero reduction code and is byte-identical without it.

use crate::ClassHierarchy;
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};

const RDFS_SUB_CLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const OWL_EQUIVALENT_CLASS: &str = "http://www.w3.org/2002/07/owl#equivalentClass";

/// The transitive-reduced class hierarchy: the DIRECT-subsumer Hasse diagram plus the
/// equivalence classes, derived from a [`ClassHierarchy`]'s complete subsumer sets.
///
/// "Direct" means *immediate*: `D ∈ direct_super_classes(C)` iff `C ⊑ D` and there is no other
/// class `E` (not equivalent to `C` or `D`) with `C ⊑ E ⊑ D`. The transitive closure of the
/// direct edges (chased through equivalence cliques) re-derives the full subsumption relation —
/// this is the minimal edge set with that property (a transitive reduction over the partial
/// order). All ids are source dict ids; ⊤/⊥ and fresh normalization names never appear.
pub struct DirectHierarchy {
    /// `direct[c]` = the DIRECT (immediate) named super-class representatives of representative
    /// `c`, by dict id, sorted ascending. Keyed by representative; non-representative members of
    /// an equivalence clique do not appear here (look them up via [`Self::representative`]).
    direct: FxHashMap<Id, Vec<Id>>,
    /// `equivalents[r]` = the OTHER members of `r`'s equivalence clique (excluding `r` itself),
    /// by dict id, sorted ascending. Present only for representatives heading a clique of size ≥ 2.
    equivalents: FxHashMap<Id, Vec<Id>>,
    /// `rep_of[c]` = the dict id of the equivalence-clique representative of `c` (the smallest
    /// dict id in its clique). A class equivalent to nothing maps to itself; only non-trivial
    /// members are stored (a class absent here is its own representative).
    rep_of: FxHashMap<Id, Id>,
}

impl DirectHierarchy {
    /// Computes the transitive reduction of `h` into the direct-subsumer Hasse diagram with
    /// equivalence-class collapse. Deterministic: clique representative = smallest dict id, all
    /// output lists sorted by dict id. Runs in O(E_closure) membership-indexed work over the
    /// closure `h` already holds (no re-saturation).
    pub fn from_closure(h: &ClassHierarchy) -> DirectHierarchy {
        // --- 1. Equivalence cliques ------------------------------------------------------------
        // C ≡ D iff D ∈ S(C) and C ∈ S(D). Build per-class subsumer HASHSETS once (the index this
        // phase adds) so the symmetric membership test — and the reduction below — are O(1).
        let closure: FxHashMap<Id, FxHashSet<Id>> = h
            .subsumer_sets()
            .map(|(c, sups)| (c, sups.iter().copied().collect()))
            .collect();

        // Assign each class a representative = the smallest dict id among its equivalence clique.
        let mut rep_of: FxHashMap<Id, Id> = FxHashMap::default();
        let mut equivalents: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        for (&c, c_sups) in &closure {
            // The clique of c: c plus every D ∈ S(c) with c ∈ S(D) (mutual subsumption).
            let mut clique: Vec<Id> = vec![c];
            for &d in c_sups {
                if closure.get(&d).is_some_and(|d_sups| d_sups.contains(&c)) {
                    clique.push(d);
                }
            }
            let rep = *clique.iter().min().expect("clique has ≥ 1 member (c)");
            if clique.len() > 1 {
                rep_of.insert(c, rep);
            }
        }
        // Materialize the equivalents list per representative (members other than the rep itself).
        let mut clique_members: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        for (&c, &rep) in &rep_of {
            clique_members.entry(rep).or_default().push(c);
        }
        for (rep, mut members) in clique_members {
            members.retain(|&m| m != rep); // the rep may itself be a keyed member; drop it.
            members.sort_unstable();
            members.dedup();
            if !members.is_empty() {
                equivalents.insert(rep, members);
            }
        }
        let rep = |c: Id| rep_of.get(&c).copied().unwrap_or(c);

        // --- 2. Transitive reduction over representatives --------------------------------------
        // For representative C, the candidate strict super-reps are { rep(D) : D ∈ S(C) } minus C
        // (drops ⊤/⊥-free closure already; equivalents fold to C and are excluded). A candidate D
        // is DIRECT iff no OTHER candidate E has D ∈ S(E) — i.e. D is not reachable through a
        // longer path. The O(1) membership test against the precomputed `closure` hashset is the
        // join cache that turns the naive O(N³) pairwise check into O(Σ deg²).
        let mut direct: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        // Iterate representatives deterministically (sorted) so construction order is stable.
        let mut reps: Vec<Id> = closure.keys().copied().filter(|&c| rep(c) == c).collect();
        reps.sort_unstable();
        for c in reps {
            let c_sups = &closure[&c];
            // Candidate super-reps of c (deduped, excluding c's own clique).
            let mut cands: Vec<Id> = Vec::new();
            {
                let mut seen = FxHashSet::default();
                for &d in c_sups {
                    let rd = rep(d);
                    if rd != c && seen.insert(rd) {
                        cands.push(rd);
                    }
                }
            }
            // D is direct iff no OTHER candidate E (E ≠ D) has D ∈ S(E): then C ⊑ E ⊑ D would make
            // the C→D edge redundant. Membership uses the rep's own closure set, which contains
            // every member's subsumers (clique members share the same S up to equivalents).
            let mut row: Vec<Id> = cands
                .iter()
                .copied()
                .filter(|&d| {
                    !cands.iter().any(|&e| {
                        e != d && closure.get(&e).is_some_and(|e_sups| e_sups.contains(&d))
                    })
                })
                .collect();
            row.sort_unstable();
            if !row.is_empty() {
                direct.insert(c, row);
            }
        }

        DirectHierarchy {
            direct,
            equivalents,
            rep_of,
        }
    }

    /// The DIRECT (immediate) named super-classes of `class`, by dict id, sorted ascending. If
    /// `class` is a non-representative member of an equivalence clique, this returns the direct
    /// supers of its representative (the shared parents of the clique). Empty if `class` is not a
    /// classified named class or its only super-class is ⊤.
    pub fn direct_super_classes(&self, class: Id) -> &[Id] {
        let rep = self.representative(class);
        self.direct.get(&rep).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The equivalence-clique representative of `class` (the smallest dict id among the classes
    /// mutually subsuming it). A class equivalent to nothing is its own representative.
    pub fn representative(&self, class: Id) -> Id {
        self.rep_of.get(&class).copied().unwrap_or(class)
    }

    /// The OTHER members of `class`'s equivalence clique (the classes `D` with `class ≡ D`,
    /// excluding `class` itself), by dict id, sorted ascending. Empty if `class` is equivalent to
    /// no other named class. Looked up via the clique representative, so it is the same set for
    /// every member of a clique (minus that member).
    pub fn equivalent_classes(&self, class: Id) -> Vec<Id> {
        let rep = self.representative(class);
        // The full clique = { rep } ∪ equivalents[rep]; the answer for `class` is that minus class.
        let mut clique: Vec<Id> = self.equivalents.get(&rep).cloned().unwrap_or_default();
        if rep != class {
            clique.push(rep);
        }
        clique.retain(|&m| m != class);
        clique.sort_unstable();
        clique
    }

    /// The number of DIRECT subsumption edges in the Hasse diagram (one per `(C, direct-super)`
    /// pair, counting only representatives). This is the DETERMINISTIC E3 assertion target — far
    /// smaller than the closure-edge count on a deep taxonomy. Independent of timing.
    pub fn direct_edge_count(&self) -> usize {
        self.direct.values().map(Vec::len).sum()
    }

    /// The number of equivalence cliques of size ≥ 2 (one per representative heading a non-trivial
    /// clique). Deterministic; another count-based acceptance assertion.
    pub fn equivalence_class_count(&self) -> usize {
        self.equivalents.len()
    }
}

/// Classifies the EL+⊥ TBox in `triples` and MATERIALIZES only the DIRECT-subsumer Hasse diagram
/// in place — appends each `(C rdfs:subClassOf D)` for a *direct* super-class D not already
/// present, and one `(C owl:equivalentClass D)` per equivalent pair (each unordered pair once,
/// `C < D` by dict id). This is the COMPACT taxonomy emission: on a deep ontology it adds O(N)
/// Hasse edges instead of the O(N²) full closure [`crate::classify_graph`] emits, while the
/// transitive closure of the emitted `rdfs:subClassOf` edges (chased by the RL `rdfs11` rule, or
/// by a property-path query) re-derives the complete subsumption relation.
///
/// Returns `(direct_subsumptions_emitted, equivalences_emitted)` — the counts of NEW triples
/// added of each kind. Idempotent: a second call adds nothing. Behind the `hasse` feature.
pub fn classify_hasse_graph(dict: &mut Dict, triples: &mut Vec<[Id; 3]>) -> (usize, usize) {
    let hierarchy = crate::Classifier::classify(dict, triples);
    let direct = DirectHierarchy::from_closure(&hierarchy);
    let sub_class_of = dict.intern_iri(RDFS_SUB_CLASS_OF);
    let equivalent_class = dict.intern_iri(OWL_EQUIVALENT_CLASS);

    let mut present: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    let mut sub_emitted = 0usize;
    let mut equiv_emitted = 0usize;

    // Direct subsumption edges, deterministic order: by subclass rep dict id, then super dict id.
    let mut reps: Vec<Id> = direct.direct.keys().copied().collect();
    reps.sort_unstable();
    for c in reps {
        for &d in &direct.direct[&c] {
            let t = [c, sub_class_of, d];
            if present.insert(t) {
                triples.push(t);
                sub_emitted += 1;
            }
        }
    }

    // Equivalence edges: emit each unordered pair once as (min, equivalentClass, max).
    let mut equiv_reps: Vec<Id> = direct.equivalents.keys().copied().collect();
    equiv_reps.sort_unstable();
    for rep in equiv_reps {
        for &m in &direct.equivalents[&rep] {
            let (a, b) = if rep < m { (rep, m) } else { (m, rep) };
            let t = [a, equivalent_class, b];
            if present.insert(t) {
                triples.push(t);
                equiv_emitted += 1;
            }
        }
    }

    (sub_emitted, equiv_emitted)
}
