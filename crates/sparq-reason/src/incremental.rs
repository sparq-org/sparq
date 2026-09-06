//! **Incremental maintenance** of the RDFS closure under inserts and deletes (T18).
//!
//! Batch materialization ([`crate::materialize_rdfs`]) recomputes the whole closure; with a
//! delta-overlay store (T17) that means a single triple update invalidates everything. This
//! module keeps the closure up to date in time proportional to the *change*, RDFox-style.
//!
//! # Algorithm: exact **counting** (not DRed) — and why
//!
//! The two standard incremental-materialization algorithms are DRed (overdelete, then
//! rederive survivors) and counting (track how many distinct derivations support each derived
//! fact; a fact dies when its count hits 0). Counting is usually dismissed for recursive
//! datalog because a fact can support its own (re-)derivation, making counts ill-defined or
//! requiring derivation-depth stratification; DRed pays for that generality with a rederivation
//! pass that can touch far more facts than actually change.
//!
//! Our batch RDFS engine, however, is deliberately **non-recursive over the ABox**: it
//! saturates the TBox once (transitive `subClassOf`/`subPropertyOf` closures, domain/range sets
//! closed through both — see [`crate::rdfs`]) and then derives *every* entailed triple in a
//! **single sweep** where each base assertion's consequences (`emit_consequences`) depend
//! only on the closed TBox, never on other derived ABox facts. So with the TBox held fixed:
//!
//! * every derivation is **one step** from a *base* triple — derived facts never feed back;
//! * the multiset of derivations of a derived triple is exactly the multiset of emissions of
//!   `emit_consequences` over the base triples that produce it;
//! * therefore a per-derived-triple **derivation count is exact**: insert a base triple →
//!   `+1` per emission; delete it → `-1` per the *identical* (deterministic, TBox-only)
//!   emission set; a derived fact leaves the closure exactly when its count reaches 0.
//!
//! No overdeletion, no rederivation proofs, no recursion bookkeeping — deletion costs the same
//! as insertion. Memory cost is one `u32` per distinct derived triple (the classic counting
//! objection — one count per *derivation depth* — vanishes because depth is always 1). This is
//! strictly cheaper and simpler than DRed for this rule set, which is why counting was chosen.
//!
//! # TBox mutations (v1)
//!
//! Inserting/deleting a *schema* triple (`rdfs:subClassOf`, `rdfs:subPropertyOf`,
//! `rdfs:domain`, `rdfs:range`) changes the closed TBox that every count was computed against,
//! so v1 falls back to a **full re-materialization** (recomputing closures and all counts).
//! This is documented, deliberate, and cheap to detect (predicate check); ABox-only updates —
//! the overwhelmingly common case — never trigger it. Incremental TBox maintenance (delta
//! TBox closure + re-sweep of affected predicates/classes only) is a follow-up, as is wiring
//! this into the T17 store overlay.
//!
//! # Semantics
//!
//! * `insert`/`delete` operate on the **base** (explicitly asserted) triples with set
//!   semantics: re-inserting a present triple or deleting an absent one is a no-op.
//! * Deleting a base triple that is *also* derivable from the remaining base keeps it in the
//!   closure (its count is still positive) — exactly what from-scratch materialization of the
//!   remainder would produce, and the case DRed needs its rederivation pass for.
//! * Deleting a derived-only triple is a no-op: entailed facts cannot be retracted while
//!   their support exists (standard materialized-view semantics).

use crate::rdfs::{close_dr, sweep, transitive_closure};
use crate::Vocab;
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};

/// Opt-in `why()` provers (the `explain` feature) — a child module so it can reach the
/// private maintenance state, kept in its own file. Everything explanation-related is
/// cfg'd out entirely without the feature.
#[cfg(feature = "explain")]
#[path = "incremental_explain.rs"]
mod incremental_explain;
#[cfg(feature = "explain")]
use incremental_explain::{OwlExplain, RdfsExplain};

const OWL_NS: &str = "http://www.w3.org/2002/07/owl#";
const XSD_NS: &str = "http://www.w3.org/2001/XMLSchema#";
#[cfg(feature = "quoted-triples")]
const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// An RDFS-materialized triple set maintained **incrementally** under base inserts/deletes.
///
/// Owns the base (asserted) triples, the derived set with exact derivation counts, and the
/// closed TBox the counts were computed against. The full closure (base ∪ derived) is exposed
/// via [`closure`](Self::closure) / [`contains`](Self::contains) and always equals what
/// [`crate::materialize_rdfs`] would produce from scratch on the current base.
pub struct MaterializedGraph {
    v: Vocab,
    /// Explicitly asserted triples (set semantics).
    base: FxHashSet<[Id; 3]>,
    /// Derived triple -> number of one-step derivations currently supporting it (always > 0;
    /// zero-count entries are removed eagerly). May overlap `base`: a fact can be both
    /// asserted and derived, and survives deletion of either support alone.
    counts: FxHashMap<[Id; 3], u32>,
    /// TBox-closure triples (transitive `subClassOf`/`subPropertyOf` facts, rdfs11/rdfs5).
    /// Kept separate from `counts`: they are derived from the TBox alone and are rebuilt
    /// wholesale on any TBox mutation, never adjusted by ABox deltas.
    schema_facts: FxHashSet<[Id; 3]>,
    // The closed TBox (see `crate::rdfs::rdfs_closure` steps 1-2), kept so ABox deltas can be
    // swept against it without recomputation.
    sc_closure: FxHashMap<Id, Vec<Id>>,
    sp_closure: FxHashMap<Id, Vec<Id>>,
    dom_full: FxHashMap<Id, Vec<Id>>,
    rng_full: FxHashMap<Id, Vec<Id>>,
    /// How many times the v1 full-rematerialization fallback ran (TBox mutations). The initial
    /// materialization in [`new`](Self::new) is not counted. Exposed for tests/telemetry.
    rebuilds: usize,
    /// Explanation state (`explain` feature): raw TBox edge maps + base reverse indexes —
    /// everything `why()` needs to reconstruct a derivation deterministically.
    #[cfg(feature = "explain")]
    explain: RdfsExplain,
}

impl MaterializedGraph {
    /// Build the graph from `base_triples` and fully materialize the RDFS closure (same rule
    /// set and result as [`crate::materialize_rdfs`]), recording the derivation counts that
    /// make subsequent [`insert`](Self::insert)/[`delete`](Self::delete) incremental.
    ///
    /// `dict` is only needed to intern the RDFS vocabulary ids once; incremental updates are
    /// dictionary-free (callers intern new terms through their own `Dict` as usual).
    pub fn new(dict: &mut Dict, base_triples: &[[Id; 3]]) -> Self {
        let mut g = MaterializedGraph {
            v: Vocab::intern(dict),
            base: base_triples.iter().copied().collect(),
            counts: FxHashMap::default(),
            schema_facts: FxHashSet::default(),
            sc_closure: FxHashMap::default(),
            sp_closure: FxHashMap::default(),
            dom_full: FxHashMap::default(),
            rng_full: FxHashMap::default(),
            rebuilds: 0,
            #[cfg(feature = "explain")]
            explain: RdfsExplain::default(),
        };
        g.rematerialize();
        g.rebuilds = 0; // the initial materialization is not a fallback
        g
    }

    /// Is `p` one of the four TBox predicates whose triples define the closed schema?
    fn is_tbox_predicate(&self, p: Id) -> bool {
        p == self.v.sub_class || p == self.v.sub_prop || p == self.v.domain || p == self.v.range
    }

    /// Full re-materialization from the current base: recompute the TBox closures, re-sweep
    /// every base triple, and rebuild all derivation counts. Mirrors
    /// `crate::rdfs::rdfs_closure(.., emit_dr_closure = false, MonoOwl::default())` exactly,
    /// except emissions are *counted* instead of deduplicated away.
    fn rematerialize(&mut self) {
        self.rebuilds += 1;
        let v = &self.v;
        // 1. Raw TBox maps from the base.
        let mut sc: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        let mut sp: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        let mut dom: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        let mut rng: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        for &[s, p, o] in &self.base {
            if p == v.sub_class {
                sc.entry(s).or_default().push(o);
            } else if p == v.sub_prop {
                sp.entry(s).or_default().push(o);
            } else if p == v.domain {
                dom.entry(s).or_default().push(o);
            } else if p == v.range {
                rng.entry(s).or_default().push(o);
            }
        }
        // 2. Saturate the TBox.
        self.sc_closure = transitive_closure(&sc);
        self.sp_closure = transitive_closure(&sp);
        self.dom_full = close_dr(&dom, &self.sp_closure, &self.sc_closure);
        self.rng_full = close_dr(&rng, &self.sp_closure, &self.sc_closure);
        // 3. Sweep every base triple against the closed TBox, counting emissions.
        let asserted: Vec<[Id; 3]> = self.base.iter().copied().collect();
        let emitted = sweep(
            &asserted,
            v,
            &self.sc_closure,
            &self.sp_closure,
            &self.dom_full,
            &self.rng_full,
            None,
        );
        self.counts.clear();
        for t in emitted {
            *self.counts.entry(t).or_insert(0) += 1;
        }
        // 4. TBox-closure facts (rdfs11 / rdfs5), maintained as a set of their own.
        self.schema_facts.clear();
        for (&c, ds) in &self.sc_closure {
            self.schema_facts.extend(ds.iter().map(|&d| [c, v.sub_class, d]));
        }
        for (&p, qs) in &self.sp_closure {
            self.schema_facts.extend(qs.iter().map(|&q| [p, v.sub_prop, q]));
        }
        #[cfg(feature = "explain")]
        self.explain.rebuild(&self.base, sc, sp, dom, rng);
    }

    /// All one-step consequences (the exact emission multiset) of `triples` against the
    /// current closed TBox. Parallel for large deltas (same `sweep` as batch).
    fn consequences(&self, triples: &[[Id; 3]]) -> Vec<[Id; 3]> {
        sweep(
            triples,
            &self.v,
            &self.sc_closure,
            &self.sp_closure,
            &self.dom_full,
            &self.rng_full,
            None,
        )
    }

    /// Insert base triples; the closure is updated incrementally (one sweep over the delta).
    /// Triples already asserted are ignored (set semantics). Returns the number of triples
    /// actually added to the base.
    ///
    /// If any *newly added* triple is a TBox triple (`subClassOf` / `subPropertyOf` /
    /// `domain` / `range`), v1 falls back to full re-materialization (see module docs).
    pub fn insert(&mut self, triples: &[[Id; 3]]) -> usize {
        let mut added: Vec<[Id; 3]> = Vec::with_capacity(triples.len());
        let mut tbox = false;
        for &t in triples {
            if self.base.insert(t) {
                tbox |= self.is_tbox_predicate(t[1]);
                added.push(t);
            }
        }
        if added.is_empty() {
            return 0;
        }
        if tbox {
            self.rematerialize();
            return added.len();
        }
        #[cfg(feature = "explain")]
        self.explain.add_triples(&added);
        for t in self.consequences(&added) {
            *self.counts.entry(t).or_insert(0) += 1;
        }
        added.len()
    }

    /// Delete base triples; the closure is updated incrementally by *decrementing* the exact
    /// derivation counts of the delta's consequences — derived facts disappear precisely when
    /// their last support goes (no overdelete/rederive pass needed; see module docs). Triples
    /// not currently asserted — including derived-only facts — are ignored. Returns the number
    /// of triples actually removed from the base.
    ///
    /// TBox deletions fall back to full re-materialization (v1), like [`insert`](Self::insert).
    pub fn delete(&mut self, triples: &[[Id; 3]]) -> usize {
        let mut removed: Vec<[Id; 3]> = Vec::with_capacity(triples.len());
        let mut tbox = false;
        for t in triples {
            if self.base.remove(t) {
                tbox |= self.is_tbox_predicate(t[1]);
                removed.push(*t);
            }
        }
        if removed.is_empty() {
            return 0;
        }
        if tbox {
            self.rematerialize();
            return removed.len();
        }
        #[cfg(feature = "explain")]
        self.explain.remove_triples(&removed);
        for t in self.consequences(&removed) {
            match self.counts.get_mut(&t) {
                Some(c) if *c > 1 => *c -= 1,
                Some(_) => {
                    self.counts.remove(&t);
                }
                None => debug_assert!(
                    false,
                    "derivation-count underflow: delete decremented a consequence that was \
                     never counted (TBox drifted without a rebuild?)"
                ),
            }
        }
        removed.len()
    }

    /// Is `t` in the materialized closure (asserted or derived)?
    pub fn contains(&self, t: &[Id; 3]) -> bool {
        self.base.contains(t) || self.counts.contains_key(t) || self.schema_facts.contains(t)
    }

    /// The full materialized closure (base ∪ derived), deduplicated and sorted — identical as
    /// a set to running [`crate::materialize_rdfs`] from scratch on the current base. The
    /// caller can rebuild a `Graph`/index from this; live integration with the T17 store
    /// overlay is a documented follow-up.
    pub fn closure(&self) -> Vec<[Id; 3]> {
        let mut set: FxHashSet<[Id; 3]> = self.base.clone();
        set.extend(self.counts.keys().copied());
        set.extend(self.schema_facts.iter().copied());
        let mut out: Vec<[Id; 3]> = set.into_iter().collect();
        out.sort_unstable();
        out
    }

    /// Iterate the closure without materializing a `Vec` (unsorted, deduplicated).
    pub fn iter(&self) -> impl Iterator<Item = [Id; 3]> + '_ {
        self.base
            .iter()
            .copied()
            .chain(self.counts.keys().copied().filter(|t| !self.base.contains(t)))
            .chain(
                self.schema_facts
                    .iter()
                    .copied()
                    .filter(|t| !self.base.contains(t) && !self.counts.contains_key(t)),
            )
    }

    /// Number of triples in the closure.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// Is the closure empty?
    pub fn is_empty(&self) -> bool {
        self.base.is_empty() && self.counts.is_empty() && self.schema_facts.is_empty()
    }

    /// Number of *asserted* (base) triples.
    pub fn base_len(&self) -> usize {
        self.base.len()
    }

    /// Iterate the *asserted* (base) triples (unsorted).
    pub fn base_triples(&self) -> impl Iterator<Item = [Id; 3]> + '_ {
        self.base.iter().copied()
    }

    /// How many times a mutation fell back to full re-materialization (TBox changes; v1).
    pub fn full_rebuilds(&self) -> usize {
        self.rebuilds
    }
}

// ════════════════════════════════════════════════════════════════════════════════════════
// Incremental OWL 2 RL maintenance
// ════════════════════════════════════════════════════════════════════════════════════════
//
// Extends the T18 counting approach to the OWL 2 RL profile. The same closed-TBox argument
// applies to the *monotone, non-recursive* OWL-RL assertional rules — prp-spo1 (rdfs7),
// cax-sco (rdfs9), prp-dom/prp-rng (rdfs2/3), prp-inv1/2, prp-symp, prp-eqp1/2, cax-eqc1/2 —
// because with the TBox fixed, every derivation is one deterministic step from a base triple
// (the property-orientation closure `px` precomposes subPropertyOf/inverseOf/symmetric/
// equivalentProperty chains, exactly like the batch engine's `PropExpand`).
//
// **prp-trp (TransitiveProperty)** is the one recursive-over-the-ABox rule supported here.
// Plain counting cannot handle it (derived edges feed their own rederivation), so it gets a
// dedicated **transitive layer**: for each transitive property the *effective edge set* (base
// assertions oriented through `px`, plus inflow from other transitive properties' closures) is
// maintained as an exact multiset, and the property's transitive closure is recomputed — cost
// proportional to that property's subgraph, not the dataset — whenever a delta touches an
// inflow predicate. The old/new closure sets are diffed and the diff is pushed through the
// counting sweep as virtual base facts, so counts stay exact and deletes stay cheap.
//
// **Documented fallbacks** (each detected cheaply, counted in `full_rebuilds`):
// * TBox mutations (subClassOf/subPropertyOf/domain/range/equivalentClass/equivalentProperty/
//   inverseOf/sameAs/restriction-family predicates, or typing a property Symmetric/Transitive/
//   Functional/InverseFunctional) → full re-materialization, like the RDFS v1 fallback. This
//   covers the scm-* schema rules: they only fire on TBox facts, which makes the "scm-* rules
//   derive TBox facts" problem a non-issue for ABox deltas — the closed TBox already includes
//   the full scm closure (computed at rebuild), and ABox deltas can never trigger scm-*.
// * Ontologies using the *recursive/equality* features — `owl:sameAs` (entity rewriting is
//   non-local), Functional/InverseFunctionalProperty (derive sameAs), property chains,
//   restrictions/cardinality/hasKey/oneOf/intersection/union — run in **fallback mode**:
//   every mutation re-materializes via `materialize_owl_rl` (correct, just not incremental).
//   On realistic ABox-update workloads these features live in the (static) TBox, so the mode
//   is decided once at load; the hot path is the counting mode. With the `quoted-triples`
//   feature the re-materialization runs `materialize_owl_rl_reify` in the graph's
//   construction-time `ReifyMode` instead (`Bridge` for `new`, i.e. exactly
//   `materialize_owl_rl`) — sq-afun3 third increment.
// * Deltas mentioning the occurrence-guarded vocabulary (`owl:Thing`/`owl:Nothing`, the XSD
//   numeric-tower datatypes) rebuild, because the batch engine's `pre_monotone` facts are
//   occurrence-dependent.
//
// The differential property test (tests/incremental_owl_prop.rs) holds the closure equal to
// from-scratch `materialize_owl_rl` after every batch of a randomized edit sequence.
//
// NOTE (px fall-through, kept in lockstep with the batch engine): a property that has a
// domain/range but appears nowhere in the property-orientation TBox (no subPropertyOf/
// inverseOf/symmetric/equivalent edge) is absent from the px map. The batch monotone path
// (`rdfs::emit_consequences`) falls through to the plain RDFS emission for such properties
// (regression test `prop_expand_keeps_domain_range_only_properties`); `emit_std` below does
// the same via the identity orientation, so the differential tests hold.

/// OWL vocabulary ids for the incremental OWL-RL graph, interned once at construction
/// (mirrors the private `owl::Owl` — owned by the owl.rs thread, so duplicated additively).
struct OwlIds {
    same_as: Id,
    inverse_of: Id,
    symmetric: Id,
    transitive: Id,
    equiv_prop: Id,
    equiv_class: Id,
    functional: Id,
    inv_functional: Id,
    property_chain: Id,
    on_property: Id,
    some_values: Id,
    all_values: Id,
    has_value: Id,
    intersection: Id,
    union: Id,
    has_key: Id,
    max_cardinality: Id,
    max_qual_card: Id,
    on_class: Id,
    one_of: Id,
    different_from: Id,
    thing: Id,
    nothing: Id,
    owl_class: Id,
    /// XSD numeric-tower edges `(sub, sup)`, topologically ordered subs-before-sups
    /// (mirrors `owl::XSD_HIERARCHY`).
    xsd: Vec<(Id, Id)>,
    /// Occurrence-guarded ids: `owl:Thing`, `owl:Nothing`, every XSD tower datatype. A delta
    /// triple mentioning one of these can change the batch `pre_monotone` facts → rebuild.
    special: FxHashSet<Id>,
    /// [Kern] `quoted-triples`: the reify trigger ids — `rdf:reifies` + the classic
    /// `rdf:subject`/`rdf:predicate`/`rdf:object`. Mirrors `reify::occurs`: an
    /// ANY-position occurrence in the base routes it to [`OwlMode::Fallback`] (the batch
    /// reify rules are not modelled by the counting modes), and a mutation mentioning one
    /// forces a re-detection rebuild.
    #[cfg(feature = "quoted-triples")]
    reify: [Id; 4],
}

impl OwlIds {
    fn intern(dict: &mut Dict) -> OwlIds {
        let mut o = |frag: &str| dict.intern_iri(&format!("{OWL_NS}{frag}"));
        let same_as = o("sameAs");
        let inverse_of = o("inverseOf");
        let symmetric = o("SymmetricProperty");
        let transitive = o("TransitiveProperty");
        let equiv_prop = o("equivalentProperty");
        let equiv_class = o("equivalentClass");
        let functional = o("FunctionalProperty");
        let inv_functional = o("InverseFunctionalProperty");
        let property_chain = o("propertyChainAxiom");
        let on_property = o("onProperty");
        let some_values = o("someValuesFrom");
        let all_values = o("allValuesFrom");
        let has_value = o("hasValue");
        let intersection = o("intersectionOf");
        let union = o("unionOf");
        let has_key = o("hasKey");
        let max_cardinality = o("maxCardinality");
        let max_qual_card = o("maxQualifiedCardinality");
        let on_class = o("onClass");
        let one_of = o("oneOf");
        let different_from = o("differentFrom");
        let thing = o("Thing");
        let nothing = o("Nothing");
        let owl_class = o("Class");
        const XSD_HIERARCHY: &[(&str, &str)] = &[
            ("byte", "short"),
            ("short", "int"),
            ("int", "long"),
            ("long", "integer"),
            ("integer", "decimal"),
            ("unsignedByte", "unsignedShort"),
            ("unsignedShort", "unsignedInt"),
            ("unsignedInt", "unsignedLong"),
            ("unsignedLong", "nonNegativeInteger"),
            ("nonNegativeInteger", "integer"),
            ("positiveInteger", "nonNegativeInteger"),
            ("negativeInteger", "nonPositiveInteger"),
            ("nonPositiveInteger", "integer"),
        ];
        let xsd: Vec<(Id, Id)> = XSD_HIERARCHY
            .iter()
            .map(|&(a, b)| {
                (dict.intern_iri(&format!("{XSD_NS}{a}")), dict.intern_iri(&format!("{XSD_NS}{b}")))
            })
            .collect();
        let mut special: FxHashSet<Id> = xsd.iter().flat_map(|&(a, b)| [a, b]).collect();
        special.insert(thing);
        special.insert(nothing);
        OwlIds {
            same_as,
            inverse_of,
            symmetric,
            transitive,
            equiv_prop,
            equiv_class,
            functional,
            inv_functional,
            property_chain,
            on_property,
            some_values,
            all_values,
            has_value,
            intersection,
            union,
            has_key,
            max_cardinality,
            max_qual_card,
            on_class,
            one_of,
            different_from,
            thing,
            nothing,
            owl_class,
            xsd,
            special,
            // rdf:type is NOT a trigger (reif-dtr only CONCLUDES rdf:type) — mirrors
            // the `reify::occurs` trigger set exactly.
            #[cfg(feature = "quoted-triples")]
            reify: ["reifies", "subject", "predicate", "object"]
                .map(|frag| dict.intern_iri(&format!("{RDF_NS}{frag}"))),
        }
    }
}

/// Maintenance regime the current base supports (telemetry; decided at every rebuild).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwlMode {
    /// Counting-exact, no transitive properties: mirrors the batch *monotone* fast path
    /// (`rdfs_closure` + `MonoOwl`) — subClassOf/subPropertyOf/domain/range/equivalentClass/
    /// equivalentProperty/inverseOf/SymmetricProperty.
    CountingMono,
    /// Counting-exact with TransitiveProperty: mirrors the batch *fixpoint* path; prp-trp is
    /// handled by the exact transitive layer (see module notes).
    CountingFixpoint,
    /// The base uses recursive/equality features (sameAs, Functional/InverseFunctional,
    /// chains, restrictions, cardinality, hasKey, oneOf, intersection/union) — every mutation
    /// re-materializes via [`crate::materialize_owl_rl`] (documented fallback).
    Fallback,
}

/// An OWL-2-RL-materialized triple set maintained **incrementally** under base inserts and
/// deletes — the OWL sibling of [`MaterializedGraph`]. The closure always equals what
/// [`crate::materialize_owl_rl`] would produce from scratch on the current base (the
/// differential property tests assert exactly that after every random edit batch).
///
/// Unlike the RDFS sibling, mutations take `&mut Dict`: the fallback path re-materializes
/// through the batch engine, which interns vocabulary on the fly.
pub struct MaterializedOwlGraph {
    v: Vocab,
    ow: OwlIds,
    /// Explicitly asserted triples (set semantics).
    base: FxHashSet<[Id; 3]>,
    mode: OwlMode,
    /// Derived triple -> exact one-step derivation count (counting modes; empty in fallback).
    counts: FxHashMap<[Id; 3], u32>,
    /// TBox-derived facts rebuilt wholesale at every rebuild: subClassOf/subPropertyOf closure
    /// triples, the domain/range (scm-dom*/rng*) closure, post-equivalences (scm-eqc2/eqp2),
    /// and the occurrence-guarded `pre_monotone` facts plus their sweep emissions.
    schema_facts: FxHashSet<[Id; 3]>,
    // ---- the closed TBox (counting modes) ----
    sc_closure: FxHashMap<Id, Vec<Id>>,
    sp_closure: FxHashMap<Id, Vec<Id>>,
    dom_used: FxHashMap<Id, Vec<Id>>,
    rng_used: FxHashMap<Id, Vec<Id>>,
    /// Property-orientation closure `p -> [(r, swapped)]` (the batch `PropExpand` semantics).
    px: FxHashMap<Id, Vec<(Id, bool)>>,
    /// Whether the px map is consulted at all (mono mode: only when inverseOf/symmetric
    /// present, mirroring the batch; fixpoint mode: always, with identity default).
    px_active: bool,
    // ---- transitive layer (fixpoint mode) ----
    transitive: FxHashSet<Id>,
    /// Asserted predicate -> the transitive properties (with orientation) its edges feed.
    inflow: FxHashMap<Id, Vec<(Id, bool)>>,
    /// Transitive property -> other transitive properties (with orientation) reachable via px
    /// (closure edges transfer there during the layer fixpoint).
    trans_transfer: FxHashMap<Id, Vec<(Id, bool)>>,
    /// Exact multiset of base-derived effective edges per transitive property.
    e0: FxHashMap<Id, FxHashMap<(Id, Id), u32>>,
    /// Current transitive-layer facts (each supported by +1 self count + counted emissions).
    virtual_facts: FxHashSet<[Id; 3]>,
    // ---- fallback mode ----
    fallback_closure: FxHashSet<[Id; 3]>,
    /// [Kern] `quoted-triples` (third increment, sq-afun3): which reify bridge rules the
    /// Fallback re-materialization runs — [`crate::ReifyMode::Bridge`] (the
    /// [`MaterializedOwlGraph::new`] default, matching [`crate::materialize_owl_rl`]) or the
    /// STRICT-OPACITY [`crate::ReifyMode::DestructureOnly`] from
    /// [`MaterializedOwlGraph::with_reify_mode`]. The reify vocabulary always routes a base
    /// to Fallback, so this single site is where the mode is observable.
    #[cfg(feature = "quoted-triples")]
    reify_mode: crate::reify::ReifyMode,
    // ---- rebuild triggers ----
    tbox_preds: FxHashSet<Id>,
    axiom_types: FxHashSet<Id>,
    rebuilds: usize,
    /// Explanation state (`explain` feature): raw TBox edges with origins + base reverse
    /// indexes — everything `why()` needs to reconstruct a derivation deterministically.
    #[cfg(feature = "explain")]
    explain: OwlExplain,
}

impl MaterializedOwlGraph {
    /// Build the graph from `base_triples` and fully materialize the OWL 2 RL closure (same
    /// result as [`crate::materialize_owl_rl`]), recording whatever supporting state the
    /// detected [`OwlMode`] needs for incremental updates.
    ///
    /// With the `quoted-triples` feature this runs the FULL reify bridge (reif-dtr +
    /// reif-ctr), exactly like [`crate::materialize_owl_rl`]; the feature-gated
    /// `with_reify_mode` is the strict-opacity variant.
    pub fn new(dict: &mut Dict, base_triples: &[[Id; 3]]) -> Self {
        let mut g = Self::seed(dict, base_triples);
        g.rematerialize(dict);
        g.rebuilds = 0; // the initial materialization is not a fallback
        g
    }

    /// Kern `quoted-triples` (third increment, sq-afun3): build the graph with the reify
    /// bridge driven in an explicit [`ReifyMode`](crate::ReifyMode) — the incremental
    /// counterpart of [`crate::materialize_owl_rl_reify`], which
    /// [`MaterializedOwlGraph::new`] previously left unreachable (the Fallback
    /// re-materialization always ran the full bridge, so strict opacity was batch-only).
    ///
    /// `ReifyMode::Bridge` is exactly [`MaterializedOwlGraph::new`].
    /// [`ReifyMode::DestructureOnly`](crate::ReifyMode::DestructureOnly) is STRICT OPACITY:
    /// every re-materialization — the initial one and every mutation's — runs reif-dtr alone,
    /// so the graph's closure never contains an inference-minted triple term, and the
    /// invariant this type promises still holds against the matching batch oracle
    /// (`materialize_owl_rl_reify(dict, base, mode)` from scratch). The mode is a property of
    /// the graph, not of a mutation: it cannot change after construction, so the
    /// closure-equals-from-scratch property is stable across the whole edit sequence.
    ///
    /// Mode DETECTION is unchanged — a base (or a mutation) mentioning the reification
    /// vocabulary routes to [`OwlMode::Fallback`] in both reify modes, since the counting
    /// modes model neither bridge rule.
    #[cfg(feature = "quoted-triples")]
    pub fn with_reify_mode(
        dict: &mut Dict,
        base_triples: &[[Id; 3]],
        reify_mode: crate::reify::ReifyMode,
    ) -> Self {
        let mut g = Self::seed(dict, base_triples);
        g.reify_mode = reify_mode;
        g.rematerialize(dict);
        g.rebuilds = 0; // the initial materialization is not a fallback
        g
    }

    /// The un-materialized graph: interned vocabulary, the base, and empty incremental state.
    /// Split out of [`MaterializedOwlGraph::new`] so every constructor shares one struct
    /// literal and differs only in the state it sets before the first `rematerialize`.
    fn seed(dict: &mut Dict, base_triples: &[[Id; 3]]) -> Self {
        let v = Vocab::intern(dict);
        let ow = OwlIds::intern(dict);
        let tbox_preds: FxHashSet<Id> = [
            v.sub_class,
            v.sub_prop,
            v.domain,
            v.range,
            ow.equiv_class,
            ow.equiv_prop,
            ow.inverse_of,
            ow.same_as,
            ow.property_chain,
            ow.on_property,
            ow.some_values,
            ow.all_values,
            ow.has_value,
            ow.intersection,
            ow.union,
            ow.has_key,
            ow.max_cardinality,
            ow.max_qual_card,
            ow.on_class,
            ow.one_of,
        ]
        .into_iter()
        .collect();
        let axiom_types: FxHashSet<Id> =
            [ow.symmetric, ow.transitive, ow.functional, ow.inv_functional].into_iter().collect();
        MaterializedOwlGraph {
            v,
            ow,
            base: base_triples.iter().copied().collect(),
            mode: OwlMode::Fallback,
            counts: FxHashMap::default(),
            schema_facts: FxHashSet::default(),
            sc_closure: FxHashMap::default(),
            sp_closure: FxHashMap::default(),
            dom_used: FxHashMap::default(),
            rng_used: FxHashMap::default(),
            px: FxHashMap::default(),
            px_active: false,
            transitive: FxHashSet::default(),
            inflow: FxHashMap::default(),
            trans_transfer: FxHashMap::default(),
            e0: FxHashMap::default(),
            virtual_facts: FxHashSet::default(),
            fallback_closure: FxHashSet::default(),
            #[cfg(feature = "quoted-triples")]
            reify_mode: crate::reify::ReifyMode::default(),
            tbox_preds,
            axiom_types,
            rebuilds: 0,
            #[cfg(feature = "explain")]
            explain: OwlExplain::default(),
        }
    }

    /// Does mutating `t` require a full rebuild (TBox / axiom typing / occurrence-guarded id)?
    fn triggers_rebuild(&self, t: &[Id; 3]) -> bool {
        // [Kern] `quoted-triples`: a mutation mentioning the reification vocabulary in ANY
        // position re-detects the mode (`rematerialize` then routes the base to Fallback).
        #[cfg(feature = "quoted-triples")]
        if t.iter().any(|id| self.ow.reify.contains(id)) {
            return true;
        }
        self.tbox_preds.contains(&t[1])
            || (t[1] == self.v.ty && self.axiom_types.contains(&t[2]))
            || t.iter().any(|id| self.ow.special.contains(id))
    }

    /// Full re-detection + re-materialization from the current base (counts/state rebuilt for
    /// whichever [`OwlMode`] the base now supports).
    fn rematerialize(&mut self, dict: &mut Dict) {
        self.rebuilds += 1;
        #[cfg(feature = "explain")]
        self.explain.rebuild(&self.base, &self.v, &self.ow);
        self.counts.clear();
        self.schema_facts.clear();
        self.virtual_facts.clear();
        self.e0.clear();
        self.fallback_closure.clear();

        // ---- 1. Scan the base: raw TBox maps + feature detection + special occurrences ----
        let (v, ow) = (&self.v, &self.ow);
        let mut sc: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        let mut sp: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        let mut dom: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        let mut rng: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        let mut inverse: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        let mut symmetric: FxHashSet<Id> = FxHashSet::default();
        let mut transitive: FxHashSet<Id> = FxHashSet::default();
        let mut equiv_class: Vec<(Id, Id)> = Vec::new();
        let mut equiv_prop: Vec<(Id, Id)> = Vec::new();
        let mut fallback = false;
        let fallback_preds: FxHashSet<Id> = [
            ow.same_as,
            ow.property_chain,
            ow.on_property,
            ow.some_values,
            ow.all_values,
            ow.has_value,
            ow.intersection,
            ow.union,
            ow.has_key,
            ow.max_cardinality,
            ow.max_qual_card,
            ow.on_class,
            ow.one_of,
        ]
        .into_iter()
        .collect();
        let mut occurring_special: FxHashSet<Id> = FxHashSet::default();
        for &[s, p, o] in &self.base {
            if p == v.sub_class {
                sc.entry(s).or_default().push(o);
            } else if p == v.sub_prop {
                sp.entry(s).or_default().push(o);
            } else if p == v.domain {
                dom.entry(s).or_default().push(o);
            } else if p == v.range {
                rng.entry(s).or_default().push(o);
            } else if p == ow.inverse_of {
                inverse.entry(s).or_default().push(o);
                inverse.entry(o).or_default().push(s);
            } else if p == ow.equiv_class {
                equiv_class.push((s, o));
            } else if p == ow.equiv_prop {
                equiv_prop.push((s, o));
            } else if p == v.ty {
                if o == ow.symmetric {
                    symmetric.insert(s);
                } else if o == ow.transitive {
                    transitive.insert(s);
                } else if o == ow.functional || o == ow.inv_functional {
                    fallback = true;
                }
            }
            if fallback_preds.contains(&p) {
                fallback = true;
            }
            // [Kern] `quoted-triples`: ANY-position occurrence of the reification
            // vocabulary (mirroring `reify::occurs` — `ex:p rdfs:subPropertyOf
            // rdf:subject` mentions it only as an OBJECT, yet rdfs7 later derives
            // premises the batch reify rules see) routes the base to Fallback: the
            // counting modes do not model the reify rules.
            #[cfg(feature = "quoted-triples")]
            if [s, p, o].iter().any(|id| ow.reify.contains(id)) {
                fallback = true;
            }
            for id in [s, p, o] {
                if ow.special.contains(&id) {
                    occurring_special.insert(id);
                }
            }
        }

        // ---- 2. Guards: vocabulary entanglement the counting modes do not model ----
        // (a) reserved predicates participating in the property-orientation graph;
        // (b) OWL axiom classes reachable in the class graph (typing could *derive* an axiom).
        if !fallback {
            let reserved: [Id; 9] = [
                v.ty,
                v.sub_class,
                v.sub_prop,
                v.domain,
                v.range,
                ow.equiv_class,
                ow.equiv_prop,
                ow.inverse_of,
                ow.different_from,
            ];
            let in_prop_graph = |id: Id| {
                sp.contains_key(&id)
                    || sp.values().any(|qs| qs.contains(&id))
                    || inverse.contains_key(&id)
                    || symmetric.contains(&id)
                    || transitive.contains(&id)
                    || equiv_prop.iter().any(|&(a, b)| a == id || b == id)
                    || dom.contains_key(&id)
                    || rng.contains_key(&id)
            };
            if reserved.iter().any(|&id| in_prop_graph(id)) {
                fallback = true;
            }
            let axioms = [ow.symmetric, ow.transitive, ow.functional, ow.inv_functional];
            let in_class_graph = |id: Id| {
                sc.contains_key(&id)
                    || sc.values().any(|ds| ds.contains(&id))
                    || equiv_class.iter().any(|&(a, b)| a == id || b == id)
            };
            if axioms.iter().any(|&id| in_class_graph(id)) {
                fallback = true;
            }
        }

        if fallback {
            self.mode = OwlMode::Fallback;
            let mut all: Vec<[Id; 3]> = self.base.iter().copied().collect();
            // [Kern] `quoted-triples` (sq-afun3): drive the graph's configured reify mode, so
            // a `with_reify_mode(…, DestructureOnly)` graph is strict on EVERY
            // re-materialization (the mode is immutable after construction). `Bridge` — the
            // `new` default — is exactly `materialize_owl_rl`, so the default path is
            // byte-identical to before this plumbing existed.
            #[cfg(feature = "quoted-triples")]
            crate::materialize_owl_rl_reify(dict, &mut all, self.reify_mode);
            #[cfg(not(feature = "quoted-triples"))]
            crate::materialize_owl_rl(dict, &mut all);
            self.fallback_closure = all.into_iter().collect();
            return;
        }

        // ---- 3. pre_monotone facts (occurrence-guarded; mirror owl::pre_monotone) ----
        let mut pre_facts: Vec<[Id; 3]> = Vec::new();
        for &id in [ow.thing, ow.nothing].iter() {
            if occurring_special.contains(&id) {
                pre_facts.push([id, v.ty, ow.owl_class]);
            }
        }
        let mut xsd_chain: Vec<(Id, Id)> = Vec::new();
        for &(sub, sup) in &ow.xsd {
            if occurring_special.contains(&sub) || xsd_chain.iter().any(|&(_, b)| b == sub) {
                xsd_chain.push((sub, sup));
            }
        }
        for &(sub, sup) in &xsd_chain {
            pre_facts.push([sub, v.sub_class, sup]);
            sc.entry(sub).or_default().push(sup); // they join the TBox, like the batch
        }

        // ---- 4. Fold equivalences, saturate the TBox ----
        for &(a, b) in &equiv_class {
            sc.entry(a).or_default().push(b);
            sc.entry(b).or_default().push(a);
        }
        for &(a, b) in &equiv_prop {
            sp.entry(a).or_default().push(b);
            sp.entry(b).or_default().push(a);
        }
        self.sc_closure = transitive_closure(&sc);
        self.sp_closure = transitive_closure(&sp);
        self.transitive = transitive;

        let fixpoint_mode = !self.transitive.is_empty();
        self.mode = if fixpoint_mode { OwlMode::CountingFixpoint } else { OwlMode::CountingMono };

        // ---- 5. Domain/range closure ----
        if fixpoint_mode {
            // The batch FIXPOINT additionally transposes domain/range through owl:inverseOf
            // ((p inv q),(p domain c) ⊢ (q range c) and vice versa) — iterate transposition +
            // re-closure to the (small, TBox-only) fixpoint.
            let (mut d, mut r) = (dom.clone(), rng.clone());
            loop {
                let df = close_dr(&d, &self.sp_closure, &self.sc_closure);
                let rf = close_dr(&r, &self.sp_closure, &self.sc_closure);
                let mut changed = false;
                let add =
                    |m: &mut FxHashMap<Id, Vec<Id>>, q: Id, cs: &[Id], changed: &mut bool| {
                        let e = m.entry(q).or_default();
                        for &c in cs {
                            if !e.contains(&c) {
                                e.push(c);
                                *changed = true;
                            }
                        }
                    };
                for (&p, qs) in &inverse {
                    for &q in qs {
                        if let Some(cs) = df.get(&p) {
                            add(&mut r, q, cs, &mut changed);
                        }
                        if let Some(cs) = rf.get(&p) {
                            add(&mut d, q, cs, &mut changed);
                        }
                    }
                }
                if !changed {
                    self.dom_used = df;
                    self.rng_used = rf;
                    break;
                }
            }
        } else {
            self.dom_used = close_dr(&dom, &self.sp_closure, &self.sc_closure);
            self.rng_used = close_dr(&rng, &self.sp_closure, &self.sc_closure);
        }

        // ---- 6. Property-orientation closure (mirrors rdfs::build_prop_expand) ----
        self.px_active = fixpoint_mode || !inverse.is_empty() || !symmetric.is_empty();
        self.px = build_px(&self.sp_closure, &inverse, &symmetric);

        // ---- 7. Transitive layer (fixpoint mode) ----
        self.inflow.clear();
        self.trans_transfer.clear();
        if fixpoint_mode {
            // Predicates whose asserted edges feed a transitive property's effective edge set.
            let mut preds: FxHashSet<Id> = self.px.keys().copied().collect();
            preds.extend(self.transitive.iter().copied());
            for q in preds {
                let mut feeds: Vec<(Id, bool)> = Vec::new();
                // A transitive property with no other TBox mention has no px entry; its
                // identity orientation still feeds its own edge set.
                let identity = [(q, false)];
                let entries: &[(Id, bool)] = match self.px.get(&q) {
                    Some(e) => e,
                    None => &identity,
                };
                for &(r, swap) in entries {
                    if self.transitive.contains(&r) {
                        feeds.push((r, swap));
                    }
                }
                if !feeds.is_empty() {
                    self.inflow.insert(q, feeds);
                }
            }
            // Closure-edge transfer between transitive properties (everything in px[r] except
            // r's own identity).
            for &r in &self.transitive {
                let mut tr: Vec<(Id, bool)> = Vec::new();
                for &(r2, swap) in px_entries(&self.px, r) {
                    if self.transitive.contains(&r2) && (r2 != r || swap) {
                        tr.push((r2, swap));
                    }
                }
                if !tr.is_empty() {
                    self.trans_transfer.insert(r, tr);
                }
            }
            // Exact effective-edge multisets from the asserted base.
            for &[s, p, o] in &self.base {
                if p == self.v.ty {
                    continue;
                }
                if let Some(feeds) = self.inflow.get(&p) {
                    for &(r, swap) in feeds {
                        let e = if swap { (o, s) } else { (s, o) };
                        *self.e0.entry(r).or_default().entry(e).or_insert(0) += 1;
                    }
                }
            }
            self.virtual_facts = self.compute_virtual();
        }

        // ---- 8. Counting sweep over the base + the transitive layer ----
        let asserted: Vec<[Id; 3]> = self.base.iter().copied().collect();
        for t in self.count_sweep(&asserted) {
            *self.counts.entry(t).or_insert(0) += 1;
        }
        let virt: Vec<[Id; 3]> = self.virtual_facts.iter().copied().collect();
        for &t in &virt {
            *self.counts.entry(t).or_insert(0) += 1; // the layer fact itself
        }
        for t in self.count_sweep(&virt) {
            *self.counts.entry(t).or_insert(0) += 1; // its one-step consequences
        }

        // ---- 9. Schema facts (rebuilt wholesale; never touched by ABox deltas) ----
        for (&c, ds) in &self.sc_closure {
            self.schema_facts.extend(ds.iter().map(|&d| [c, v.sub_class, d]));
        }
        for (&p, qs) in &self.sp_closure {
            self.schema_facts.extend(qs.iter().map(|&q| [p, v.sub_prop, q]));
        }
        for (&p, cs) in &self.dom_used {
            self.schema_facts.extend(cs.iter().map(|&c| [p, v.domain, c]));
        }
        for (&p, cs) in &self.rng_used {
            self.schema_facts.extend(cs.iter().map(|&c| [p, v.range, c]));
        }
        // post_equivalences (scm-eqc2/eqp2): mutual subsumption ⊢ equivalence, both ways.
        for (rel, eq) in [(&self.sc_closure, ow.equiv_class), (&self.sp_closure, ow.equiv_prop)] {
            for (&a, bs) in rel {
                for &b in bs {
                    if a != b && rel.get(&b).is_some_and(|r| r.contains(&a)) {
                        self.schema_facts.insert([a, eq, b]);
                        self.schema_facts.insert([b, eq, a]);
                    }
                }
            }
        }
        // pre_monotone facts + their one-step emissions.
        let mut pre_emit = Vec::new();
        for &t in &pre_facts {
            self.schema_facts.insert(t);
            self.emit_owl(t, &mut pre_emit);
        }
        self.schema_facts.extend(pre_emit);
    }

    /// All one-step consequences (exact emission multiset) of one triple against the closed
    /// TBox — the OWL analogue of `rdfs::emit_consequences` (see the note on the px
    /// fall-through this mirrors).
    fn emit_owl(&self, [s, p, o]: [Id; 3], out: &mut Vec<[Id; 3]>) {
        let v = &self.v;
        if p == v.ty {
            if let Some(ds) = self.sc_closure.get(&o) {
                out.extend(ds.iter().map(|&d| [s, v.ty, d]));
            }
            return;
        }
        if p == self.ow.different_from {
            // owl::pre_monotone adds the symmetric edge to the base before the closure, so
            // both the mirror and the mirror's own consequences are part of this emission.
            out.push([o, p, s]);
            self.emit_std([o, p, s], out);
        }
        self.emit_std([s, p, o], out);
    }

    /// The plain-edge emission (everything except the rdf:type branch / differentFrom mirror).
    fn emit_std(&self, [s, p, o]: [Id; 3], out: &mut Vec<[Id; 3]>) {
        let v = &self.v;
        if self.px_active {
            // px-absent (e.g. a domain/range-only property): fall through to the identity
            // orientation — plain RDFS emission, matching the fixed batch path
            // (`rdfs::emit_consequences`; regression test
            // `prop_expand_keeps_domain_range_only_properties`).
            let entries: &[(Id, bool)] = self.px.get(&p).map_or(&[], |e| e);
            let identity = [(p, false)];
            let entries = if entries.is_empty() { &identity[..] } else { entries };
            for &(r, swap) in entries {
                let (rs, ro) = if swap { (o, s) } else { (s, o) };
                if r != p || swap {
                    out.push([rs, r, ro]);
                }
                if let Some(cs) = self.dom_used.get(&r) {
                    out.extend(cs.iter().map(|&c| [rs, v.ty, c]));
                }
                if let Some(cs) = self.rng_used.get(&r) {
                    out.extend(cs.iter().map(|&c| [ro, v.ty, c]));
                }
            }
        } else {
            if let Some(qs) = self.sp_closure.get(&p) {
                out.extend(qs.iter().map(|&q| [s, q, o]));
            }
            if let Some(cs) = self.dom_used.get(&p) {
                out.extend(cs.iter().map(|&c| [s, v.ty, c]));
            }
            if let Some(cs) = self.rng_used.get(&p) {
                out.extend(cs.iter().map(|&c| [o, v.ty, c]));
            }
        }
    }

    /// [`emit_owl`] over a slice (parallel for large inputs, mirroring `rdfs::sweep`).
    #[cfg(feature = "parallel")]
    fn count_sweep(&self, triples: &[[Id; 3]]) -> Vec<[Id; 3]> {
        use rayon::prelude::*;
        const PAR_THRESHOLD: usize = 4096;
        if triples.len() < PAR_THRESHOLD {
            let mut out = Vec::new();
            for &t in triples {
                self.emit_owl(t, &mut out);
            }
            return out;
        }
        triples
            .par_iter()
            .fold(Vec::new, |mut acc, &t| {
                self.emit_owl(t, &mut acc);
                acc
            })
            .reduce(Vec::new, |mut a, mut b| {
                a.append(&mut b);
                a
            })
    }
    #[cfg(not(feature = "parallel"))]
    fn count_sweep(&self, triples: &[[Id; 3]]) -> Vec<[Id; 3]> {
        let mut out = Vec::new();
        for &t in triples {
            self.emit_owl(t, &mut out);
        }
        out
    }

    /// The transitive layer: per transitive property, the transitive closure of its effective
    /// edge set (e0 ∪ inflow from other transitive properties' closures), to a fixpoint across
    /// properties. Cost is proportional to the transitive subgraph, never the whole base.
    fn compute_virtual(&self) -> FxHashSet<[Id; 3]> {
        let mut order: Vec<Id> = self.transitive.iter().copied().collect();
        order.sort_unstable();
        let mut edges: FxHashMap<Id, FxHashSet<(Id, Id)>> = FxHashMap::default();
        for &r in &order {
            edges.insert(r, self.e0.get(&r).map(|m| m.keys().copied().collect()).unwrap_or_default());
        }
        loop {
            let mut changed = false;
            for &r in &order {
                let closed = tc_pairs(&edges[&r]);
                if closed.len() != edges[&r].len() {
                    changed = true;
                }
                let mut transfers: Vec<(Id, (Id, Id))> = Vec::new();
                if let Some(tr) = self.trans_transfer.get(&r) {
                    for &(r2, swap) in tr {
                        for &(x, z) in &closed {
                            transfers.push((r2, if swap { (z, x) } else { (x, z) }));
                        }
                    }
                }
                edges.insert(r, closed);
                for (r2, e) in transfers {
                    if edges.entry(r2).or_default().insert(e) {
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let mut out = FxHashSet::default();
        for (r, es) in edges {
            out.extend(es.into_iter().map(|(x, z)| [x, r, z]));
        }
        out
    }

    /// Recompute the transitive layer and apply the (usually tiny) diff to the counts: each
    /// entering/leaving layer fact adjusts its own +1 support and its emission multiset.
    fn refresh_virtual(&mut self) {
        let new_virtual = self.compute_virtual();
        let added: Vec<[Id; 3]> = new_virtual.difference(&self.virtual_facts).copied().collect();
        let removed: Vec<[Id; 3]> = self.virtual_facts.difference(&new_virtual).copied().collect();
        if added.is_empty() && removed.is_empty() {
            return;
        }
        let mut inc_emit = self.count_sweep(&added);
        inc_emit.extend(added.iter().copied());
        for t in inc_emit {
            *self.counts.entry(t).or_insert(0) += 1;
        }
        let mut dec_emit = self.count_sweep(&removed);
        dec_emit.extend(removed.iter().copied());
        for t in dec_emit {
            self.decrement(t);
        }
        self.virtual_facts = new_virtual;
    }

    fn decrement(&mut self, t: [Id; 3]) {
        match self.counts.get_mut(&t) {
            Some(c) if *c > 1 => *c -= 1,
            Some(_) => {
                self.counts.remove(&t);
            }
            None => debug_assert!(
                false,
                "OWL derivation-count underflow: decremented a consequence never counted"
            ),
        }
    }

    /// Update the transitive layer's effective-edge multisets for a delta (sign = +1 insert,
    /// -1 delete). Returns whether any multiset changed at the SET level (layer refresh due).
    fn apply_e0(&mut self, delta: &[[Id; 3]], positive: bool) -> bool {
        let mut touched = false;
        for &[s, p, o] in delta {
            if p == self.v.ty {
                continue;
            }
            let Some(feeds) = self.inflow.get(&p) else { continue };
            for &(r, swap) in feeds {
                let e = if swap { (o, s) } else { (s, o) };
                let m = self.e0.entry(r).or_default();
                if positive {
                    let c = m.entry(e).or_insert(0);
                    *c += 1;
                    if *c == 1 {
                        touched = true;
                    }
                } else {
                    match m.get_mut(&e) {
                        Some(c) if *c > 1 => *c -= 1,
                        Some(_) => {
                            m.remove(&e);
                            touched = true;
                        }
                        None => debug_assert!(false, "transitive-layer e0 underflow"),
                    }
                }
            }
        }
        touched
    }

    /// Insert base triples; the closure is updated incrementally in the counting modes (one
    /// emission sweep over the delta + a transitive-layer refresh when touched). Returns the
    /// number of triples actually added to the base. TBox / axiom-typing / occurrence-guarded
    /// deltas, and any mutation in fallback mode, re-materialize (see module notes).
    pub fn insert(&mut self, dict: &mut Dict, triples: &[[Id; 3]]) -> usize {
        let mut added: Vec<[Id; 3]> = Vec::with_capacity(triples.len());
        let mut rebuild = false;
        for &t in triples {
            if self.base.insert(t) {
                rebuild |= self.triggers_rebuild(&t);
                added.push(t);
            }
        }
        if added.is_empty() {
            return 0;
        }
        if rebuild || self.mode == OwlMode::Fallback {
            self.rematerialize(dict);
            return added.len();
        }
        #[cfg(feature = "explain")]
        self.explain.add_triples(&added);
        for t in self.count_sweep(&added) {
            *self.counts.entry(t).or_insert(0) += 1;
        }
        if self.mode == OwlMode::CountingFixpoint && self.apply_e0(&added, true) {
            self.refresh_virtual();
        }
        added.len()
    }

    /// Delete base triples; exact counting makes deletion as cheap as insertion (no
    /// overdelete/rederive pass). Triples not currently asserted — including derived-only
    /// facts — are ignored. Returns the number of triples actually removed from the base.
    pub fn delete(&mut self, dict: &mut Dict, triples: &[[Id; 3]]) -> usize {
        let mut removed: Vec<[Id; 3]> = Vec::with_capacity(triples.len());
        let mut rebuild = false;
        for t in triples {
            if self.base.remove(t) {
                rebuild |= self.triggers_rebuild(t);
                removed.push(*t);
            }
        }
        if removed.is_empty() {
            return 0;
        }
        if rebuild || self.mode == OwlMode::Fallback {
            self.rematerialize(dict);
            return removed.len();
        }
        #[cfg(feature = "explain")]
        self.explain.remove_triples(&removed);
        for t in self.count_sweep(&removed) {
            self.decrement(t);
        }
        if self.mode == OwlMode::CountingFixpoint && self.apply_e0(&removed, false) {
            self.refresh_virtual();
        }
        removed.len()
    }

    /// Is `t` in the materialized closure (asserted or derived)?
    pub fn contains(&self, t: &[Id; 3]) -> bool {
        if self.mode == OwlMode::Fallback {
            return self.fallback_closure.contains(t);
        }
        self.base.contains(t) || self.counts.contains_key(t) || self.schema_facts.contains(t)
    }

    /// The full materialized closure (base ∪ derived), deduplicated and sorted — identical as
    /// a set to running [`crate::materialize_owl_rl`] from scratch on the current base (or,
    /// for a `with_reify_mode` graph, `materialize_owl_rl_reify` in that same mode).
    pub fn closure(&self) -> Vec<[Id; 3]> {
        let mut set: FxHashSet<[Id; 3]> = if self.mode == OwlMode::Fallback {
            self.fallback_closure.clone()
        } else {
            let mut s = self.base.clone();
            s.extend(self.counts.keys().copied());
            s.extend(self.schema_facts.iter().copied());
            s
        };
        set.extend(self.base.iter().copied());
        let mut out: Vec<[Id; 3]> = set.into_iter().collect();
        out.sort_unstable();
        out
    }

    /// Number of triples in the closure.
    pub fn len(&self) -> usize {
        if self.mode == OwlMode::Fallback {
            return self.fallback_closure.len();
        }
        let mut n = self.base.len();
        n += self.counts.keys().filter(|t| !self.base.contains(*t)).count();
        n += self
            .schema_facts
            .iter()
            .filter(|t| !self.base.contains(*t) && !self.counts.contains_key(*t))
            .count();
        n
    }

    /// Is the closure empty?
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of *asserted* (base) triples.
    pub fn base_len(&self) -> usize {
        self.base.len()
    }

    /// Iterate the *asserted* (base) triples (unsorted).
    pub fn base_triples(&self) -> impl Iterator<Item = [Id; 3]> + '_ {
        self.base.iter().copied()
    }

    /// The active maintenance regime (telemetry; re-decided at every rebuild).
    pub fn mode(&self) -> OwlMode {
        self.mode
    }

    /// Kern `quoted-triples` (sq-afun3): the reify bridge mode this graph maintains —
    /// fixed at construction ([`crate::ReifyMode::Bridge`] for
    /// [`MaterializedOwlGraph::new`]), never re-decided by a rebuild.
    #[cfg(feature = "quoted-triples")]
    pub fn reify_mode(&self) -> crate::reify::ReifyMode {
        self.reify_mode
    }

    /// How many times a mutation fell back to full re-materialization (TBox mutations,
    /// occurrence-guarded deltas, and every mutation while in [`OwlMode::Fallback`]).
    pub fn full_rebuilds(&self) -> usize {
        self.rebuilds
    }
}

/// px entries for `q`, defaulting to the empty slice.
fn px_entries(px: &FxHashMap<Id, Vec<(Id, bool)>>, q: Id) -> &[(Id, bool)] {
    static EMPTY: [(Id, bool); 0] = [];
    match px.get(&q) {
        Some(v) => v.as_slice(),
        None => &EMPTY,
    }
}

/// The property-orientation closure: BFS over `(property, orientation)` through subPropertyOf
/// (same orientation; equivalentProperty pre-folded), inverseOf (flip) and SymmetricProperty
/// (flip, same property). Mirrors `rdfs::build_prop_expand` exactly — including its `all_props`
/// domain; px-absent properties take the identity fall-through in `emit_std` (see the px
/// fall-through note) — so emissions match the batch engine.
fn build_px(
    sp_closure: &FxHashMap<Id, Vec<Id>>,
    inverse: &FxHashMap<Id, Vec<Id>>,
    symmetric: &FxHashSet<Id>,
) -> FxHashMap<Id, Vec<(Id, bool)>> {
    let mut all_props: FxHashSet<Id> = sp_closure
        .keys()
        .chain(inverse.keys())
        .chain(symmetric.iter())
        .copied()
        .collect();
    for sup in sp_closure.values() {
        all_props.extend(sup.iter().copied());
    }
    for inv in inverse.values() {
        all_props.extend(inv.iter().copied());
    }
    let mut map: FxHashMap<Id, Vec<(Id, bool)>> = FxHashMap::default();
    for &p in &all_props {
        let mut seen: FxHashSet<(Id, bool)> = FxHashSet::default();
        let mut stack: Vec<(Id, bool)> = vec![(p, false)];
        while let Some((q, or)) = stack.pop() {
            if !seen.insert((q, or)) {
                continue;
            }
            if let Some(sups) = sp_closure.get(&q) {
                for &r in sups {
                    if !seen.contains(&(r, or)) {
                        stack.push((r, or));
                    }
                }
            }
            if let Some(invs) = inverse.get(&q) {
                for &r in invs {
                    if !seen.contains(&(r, !or)) {
                        stack.push((r, !or));
                    }
                }
            }
            if symmetric.contains(&q) && !seen.contains(&(q, !or)) {
                stack.push((q, !or));
            }
        }
        map.insert(p, seen.into_iter().collect());
    }
    map
}

/// Transitive closure of a set of `(from, to)` pairs (BFS per distinct source).
fn tc_pairs(pairs: &FxHashSet<(Id, Id)>) -> FxHashSet<(Id, Id)> {
    let mut adj: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    for &(s, o) in pairs {
        adj.entry(s).or_default().push(o);
    }
    let mut out: FxHashSet<(Id, Id)> = FxHashSet::default();
    for &src in adj.keys() {
        let mut seen: FxHashSet<Id> = FxHashSet::default();
        let mut stack: Vec<Id> = adj[&src].clone();
        while let Some(n) = stack.pop() {
            if seen.insert(n) {
                if let Some(succ) = adj.get(&n) {
                    stack.extend(succ.iter().copied());
                }
            }
        }
        out.extend(seen.into_iter().map(|n| (src, n)));
    }
    out
}

// ════════════════════════════════════════════════════════════════════════════════════════
// Incremental N3 maintenance — the NAF-free / input-stratified fast path
// ════════════════════════════════════════════════════════════════════════════════════════
//
// Extends the counting approach to Notation3 forward rules (`{ premise } => { conclusion }`).
// Unlike RDFS/OWL the rule set is USER-SUPPLIED, so whether counting applies is decided by
// ANALYSIS of the parsed rules (once, at construction):
//
// * **Counting** (the fast path) requires the rule set to be *monotone with input-stratified
//   negation*:
//   - forward rules only (no `<=`/log:isImpliedBy);
//   - plain premise/conclusion atoms with GROUND IRI predicates and Iri/Lit/Var terms
//     (no blanks — conclusion blanks are existentials minted fresh per firing, which breaks
//     the insert/delete emission symmetry counting needs);
//   - builtins from a verified-parity whitelist: `log:uri`, `log:equalTo`/`notEqualTo`,
//     `string:concatenation`, `string:scrape`, `string:encodeForUri` (each mirrors the
//     engine's evaluation exactly — see `eval_n3_builtin`); any other SWAP-vocabulary
//     predicate (math:/list:/time:/other log:/string:) disqualifies;
//   - negation as failure ONLY in the engine's documented store-scoped idiom — `?UNSCOPED
//     log:notIncludes { pattern }` with an unbound, otherwise-unused subject — and only over
//     predicates **no rule derives** (input-only). Guard truth then depends solely on base
//     facts, so it is CONSTANT across ABox deltas; a delta that touches a guard predicate
//     triggers the documented full-rebuild fallback (the OWL TBox-mutation analogue).
//
// * **Recursion** is allowed: the predicate dependency graph (premise pred → conclusion pred
//   per rule) is condensed into SCCs. Rules concluding into a recursive SCC form a **layer**
//   (the generalization of the OWL `prp-trp` transitive layer): the layer's fact set is the
//   local fixpoint of its rules over the facts of its input + SCC predicates, recomputed —
//   cost proportional to those predicates' extents, never the dataset — whenever a delta
//   touches them, and the old/new diff joins the propagation as virtual facts. A fact that is
//   BOTH asserted and layer-derivable is charged to the base (it seeds the local fixpoint, so
//   it is excluded from the layer's derived set), so asserting or retracting its base copy
//   hands ownership over without changing the closure; that hand-off is settled by the same
//   local layer re-derivation (`sq-6tykl.6`) rather than by a full rematerialization.
//   The hand-off is LAZY, and deliberately so: a mutation whose facts are all already settled
//   (asserting a fact the closure already has; retracting one the layer or a count still
//   supports) contributes NOTHING to `pending`, so `propagate` recomputes no layer and
//   `layer_derived` keeps its copy while `base` also holds one. That transient double
//   ownership is INERT, not stale — `recompute_layer`'s `stale_own` test is guarded by
//   `!base && !counts`, so while the base copy exists the layer's entry can never suppress a
//   seed, and the layer's local fixpoint is identical whether the fact arrives as a seed or
//   as its own derivation. The first recompute that layer does see re-attributes it (the fact
//   turns up in that round's `removed`, which is exactly what the hand-off branch in
//   `propagate` absorbs). Nothing in between can observe the difference: the only consult
//   that reads `layer_derived` without the base guard is `delete`'s `in_any_layer`
//   short-circuit, and there the entry is accurate — the layer really does still derive the
//   fact, because a mutation that could have changed that would have hit `pending`. Counting
//   handles everything outside the SCCs exactly: per derived fact, the count of rule firings
//   (computed with the classic delta-rewriting: for a delta at plain-atom position i, atoms
//   before i match the pre-delta store, atom i the delta, atoms after i the post-delta store
//   — each changed firing counted exactly once).
//
// * Everything else runs in **fallback mode**: every effective mutation re-materializes by
//   serializing the base back to N3 and running [`crate::reason_n3_terms`] (always correct,
//   just not incremental). Fallback is also entered at RUNTIME (sticky, with the reason
//   recorded) if evaluation meets data the parity whitelist does not cover — e.g.
//   `string:concatenation` over decimal/double literals or rdf:first/rest data lists —
//   and per-rebuild if `log:implies` is asserted as DATA (the engine would parse it as a
//   rule; the analysis only sees the construction-time rule set).
//
// The closure always equals a from-scratch engine run on (rules ∪ current base) — the
// differential property test (tests/incremental_n3_prop.rs) asserts exactly that after
// every random edit batch, for counting, layered-recursive, guarded, and fallback profiles.

use crate::n3::{parser as n3p, Term as N3Term};
use std::cell::Cell;
use std::rc::Rc;

const SWAP_NS: &str = "http://www.w3.org/2000/10/swap/";
const N3_LOG: &str = "http://www.w3.org/2000/10/swap/log#";
const N3_STR: &str = "http://www.w3.org/2000/10/swap/string#";
const N3_XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const N3_RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const N3_RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";

/// Maintenance regime of a [`MaterializedN3Graph`] (decided by rule analysis + base scan).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum N3Mode {
    /// The rule set qualified for exact counting (+ recursive-SCC layers); ABox deltas are
    /// incremental, guard-predicate deltas rebuild.
    Counting,
    /// Every mutation re-materializes through the batch N3 engine (reason recorded — see
    /// [`MaterializedN3Graph::fallback_reason`]).
    Fallback,
}

type NB = FxHashMap<String, N3Term>;

/// Facts + the lookup indexes premise joins need ((pred,subj)→objects, (pred,obj)→subjects,
/// pred→facts). The Term-level sibling of the engine's private `FactIndex`.
#[derive(Default)]
struct N3Index {
    all: FxHashSet<[N3Term; 3]>,
    pred: FxHashMap<N3Term, Vec<[N3Term; 3]>>,
    ps: FxHashMap<(N3Term, N3Term), Vec<N3Term>>,
    po: FxHashMap<(N3Term, N3Term), Vec<N3Term>>,
}

impl N3Index {
    fn insert(&mut self, f: [N3Term; 3]) -> bool {
        if !self.all.insert(f.clone()) {
            return false;
        }
        let [s, p, o] = f;
        self.pred.entry(p.clone()).or_default().push([s.clone(), p.clone(), o.clone()]);
        self.ps.entry((p.clone(), s.clone())).or_default().push(o.clone());
        self.po.entry((p, o)).or_default().push(s);
        true
    }
    fn remove(&mut self, f: &[N3Term; 3]) -> bool {
        if !self.all.remove(f) {
            return false;
        }
        let [s, p, o] = f;
        if let Some(v) = self.pred.get_mut(p) {
            if let Some(ix) = v.iter().position(|x| x == f) {
                v.swap_remove(ix);
            }
        }
        if let Some(v) = self.ps.get_mut(&(p.clone(), s.clone())) {
            if let Some(ix) = v.iter().position(|x| x == o) {
                v.swap_remove(ix);
            }
        }
        if let Some(v) = self.po.get_mut(&(p.clone(), o.clone())) {
            if let Some(ix) = v.iter().position(|x| x == s) {
                v.swap_remove(ix);
            }
        }
        true
    }
    fn contains(&self, f: &[N3Term; 3]) -> bool {
        self.all.contains(f)
    }
    /// Candidate facts for an APPLIED pattern (predicate always ground per qualification).
    fn candidates(&self, s: &N3Term, p: &N3Term, o: &N3Term) -> Vec<[N3Term; 3]> {
        let s_ground = !matches!(s, N3Term::Var(_));
        let o_ground = !matches!(o, N3Term::Var(_));
        if s_ground {
            return match self.ps.get(&(p.clone(), s.clone())) {
                Some(objs) => objs
                    .iter()
                    .filter(|obj| !o_ground || *obj == o)
                    .map(|obj| [s.clone(), p.clone(), obj.clone()])
                    .collect(),
                None => Vec::new(),
            };
        }
        if o_ground {
            return match self.po.get(&(p.clone(), o.clone())) {
                Some(subs) => subs.iter().map(|sub| [sub.clone(), p.clone(), o.clone()]).collect(),
                None => Vec::new(),
            };
        }
        self.pred.get(p).cloned().unwrap_or_default()
    }
}

/// The whitelisted builtins (each verified against the engine's implementation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum N3Builtin {
    LogUri,
    LogEq,
    LogNe,
    Concat,
    Scrape,
    EncodeForUri,
}

#[derive(Clone)]
enum N3Atom {
    /// A store join; `plain_ix` is its position among the rule's plain atoms (the
    /// delta-rewriting partitions by it).
    Plain { pat: [N3Term; 3], plain_ix: usize },
    Builtin { op: N3Builtin, s: N3Term, o: N3Term },
    /// `?UNSCOPED log:notIncludes { inner… }`: passes a binding iff the inner conjunction has
    /// NO match in the (input-only) guard predicates' extents.
    Guard { inner: Vec<[N3Term; 3]> },
}

#[derive(Clone)]
struct N3CompiledRule {
    /// Atoms in evaluation order (the engine's `order_premise` mirrored).
    atoms: Vec<N3Atom>,
    conclusions: Vec<[N3Term; 3]>,
}

struct N3LayerDef {
    rules: Vec<N3CompiledRule>,
    /// Predicates whose facts feed the layer (plain premise preds ∪ SCC preds): a delta
    /// touching any of them re-runs the layer's local fixpoint.
    relevant: FxHashSet<String>,
}

struct N3Compiled {
    counted: Vec<N3CompiledRule>,
    layers: Vec<N3LayerDef>,
    guard_preds: FxHashSet<String>,
}

/// An N3-rule-materialized fact set maintained **incrementally** under base inserts and
/// deletes — the N3 sibling of [`MaterializedGraph`] / [`MaterializedOwlGraph`], operating at
/// the [`Term`](crate::n3::Term) level. The closure always equals a from-scratch
/// [`crate::reason_n3_terms`] run over the rules document plus the current base.
pub struct MaterializedN3Graph {
    rules_src: String,
    compiled: Option<Rc<N3Compiled>>,
    /// Why the RULES disqualified from counting (None when `compiled` is Some).
    disqualified: Option<String>,
    /// Sticky runtime disqualification: evaluation met data outside the builtin-parity
    /// whitelist; every rebuild stays in fallback (always correct).
    data_fallback: Option<String>,
    /// [OPUS-4.8] NON-sticky fallback cause: the base currently contains `log:implies`-family
    /// triples (rules-as-data), which the batch engine would parse as RULES, so counting is
    /// forced off this rematerialize. Recomputed every `rematerialize`; cleared when the
    /// implies-data is removed and counting resumes. Keeps `fallback_reason()`'s
    /// `None ⇔ counting active` contract exact for this cause. See review 1868.
    data_rule_fallback: Option<String>,
    base: FxHashSet<[N3Term; 3]>,
    mode: N3Mode,
    counts: FxHashMap<[N3Term; 3], u32>,
    /// Current closure membership (base ∪ counted-derived ∪ layer-derived) + join indexes.
    index: N3Index,
    /// Per-layer derived facts (parallel to `compiled.layers`).
    layer_derived: Vec<FxHashSet<[N3Term; 3]>>,
    fallback_closure: FxHashSet<[N3Term; 3]>,
    rebuilds: usize,
}

/// Evaluation phase of a non-seed plain atom relative to the delta seed (the counting
/// delta-rewriting; `Free` = no delta context, e.g. the layer fixpoint).
#[derive(Clone, Copy, PartialEq)]
enum N3Phase {
    Pre,
    Post,
    Free,
}

struct N3Cx<'a> {
    /// Plain-atom store (the global membership index, or a layer's local index).
    facts: &'a N3Index,
    /// Guard scope — always the GLOBAL index (guard predicates are input-only, so their
    /// extents there equal the base extents at all times).
    guards: &'a N3Index,
    /// The current round's delta and its sign (true = insert: `facts` already contains it;
    /// false = delete: `facts` no longer contains it).
    delta: Option<(&'a FxHashSet<[N3Term; 3]>, bool)>,
    /// Set when evaluation meets data the parity whitelist does not cover.
    unsupported: &'a Cell<bool>,
}

impl N3Cx<'_> {
    fn atom_candidates(
        &self,
        s: &N3Term,
        p: &N3Term,
        o: &N3Term,
        phase: N3Phase,
    ) -> Vec<[N3Term; 3]> {
        let mut out = self.facts.candidates(s, p, o);
        match (self.delta, phase) {
            // Insert round, pre-seed position: the OLD store = index minus the delta.
            (Some((d, true)), N3Phase::Pre) => out.retain(|f| !d.contains(f)),
            // Delete round, post-seed position: the OLD store = index plus the delta.
            (Some((d, false)), N3Phase::Post) => {
                let s_ground = !matches!(s, N3Term::Var(_));
                let o_ground = !matches!(o, N3Term::Var(_));
                for f in d.iter() {
                    if &f[1] == p && (!s_ground || &f[0] == s) && (!o_ground || &f[2] == o) {
                        out.push(f.clone());
                    }
                }
            }
            _ => {}
        }
        out
    }
}

fn n3_apply(t: &N3Term, b: &NB) -> N3Term {
    match t {
        N3Term::Var(v) => b.get(v).cloned().unwrap_or_else(|| t.clone()),
        N3Term::List(ms) => N3Term::List(ms.iter().map(|m| n3_apply(m, b)).collect()),
        N3Term::Formula(ts) => N3Term::Formula(
            ts.iter()
                .map(|r| [n3_apply(&r[0], b), n3_apply(&r[1], b), n3_apply(&r[2], b)])
                .collect(),
        ),
        _ => t.clone(),
    }
}

fn n3_unify(pat: &N3Term, val: &N3Term, b: &mut NB) -> bool {
    match pat {
        N3Term::Var(v) => match b.get(v) {
            Some(bound) => bound == val,
            None => {
                b.insert(v.clone(), val.clone());
                true
            }
        },
        _ => pat == val,
    }
}

fn n3_unify_atom(pat: &[N3Term; 3], fact: &[N3Term; 3], b: &NB) -> Option<NB> {
    let mut nb = b.clone();
    for k in 0..3 {
        if !n3_unify(&pat[k], &fact[k], &mut nb) {
            return None;
        }
    }
    Some(nb)
}

/// Does the guard's inner conjunction match anywhere in the guard scope (under `b`; inner
/// free variables are wildcards, bindings never leak — the engine's notIncludes semantics)?
fn n3_guard_matches(inner: &[[N3Term; 3]], idx: &N3Index, b: &NB) -> bool {
    let mut binds = vec![b.clone()];
    for pat in inner {
        let mut next = Vec::new();
        for bb in &binds {
            let (s, o) = (n3_apply(&pat[0], bb), n3_apply(&pat[2], bb));
            for f in idx.candidates(&s, &pat[1], &o) {
                if let Some(nb) = n3_unify_atom(pat, &f, bb) {
                    next.push(nb);
                }
            }
        }
        binds = next;
        if binds.is_empty() {
            return false;
        }
    }
    true
}

/// Evaluate one whitelisted builtin under `b`; `None` ⇒ the premise fails for this binding.
/// Mirrors the engine's `eval_binder` (log:uri), `eval_builtin` (log:equalTo/notEqualTo) and
/// `eval_functional` (concatenation/scrape/encodeForUri) EXACTLY for the data shapes the
/// whitelist covers; anything outside (numeric-literal concatenation beyond integers,
/// rdf:first/rest data lists as functional subjects) flags `unsupported` — the caller falls
/// back to the batch engine, which handles them all.
fn eval_n3_builtin(
    op: N3Builtin,
    s: &N3Term,
    o: &N3Term,
    b: &NB,
    unsupported: &Cell<bool>,
) -> Option<NB> {
    let sv = n3_apply(s, b);
    match op {
        N3Builtin::LogEq => (sv == n3_apply(o, b)).then(|| b.clone()),
        N3Builtin::LogNe => (sv != n3_apply(o, b)).then(|| b.clone()),
        N3Builtin::LogUri => {
            let ov = n3_apply(o, b);
            let mut nb = b.clone();
            match (&sv, &ov) {
                (N3Term::Iri(i), _) => {
                    let lit = N3Term::Lit(i.clone(), N3_XSD_STRING.into(), None);
                    n3_unify(o, &lit, &mut nb).then_some(nb)
                }
                (N3Term::Var(_), N3Term::Lit(v, _, _)) => {
                    n3_unify(s, &N3Term::Iri(v.clone()), &mut nb).then_some(nb)
                }
                _ => None,
            }
        }
        N3Builtin::Concat => {
            let N3Term::List(ms) = &sv else {
                // The engine would walk an rdf:first/rest data list here.
                if matches!(sv, N3Term::Iri(_) | N3Term::Blank(_)) {
                    unsupported.set(true);
                }
                return None;
            };
            let mut out = String::new();
            for m in ms {
                match m {
                    N3Term::Lit(v, dt, _) => {
                        match dt.strip_prefix("http://www.w3.org/2001/XMLSchema#") {
                            Some("boolean") => out
                                .push_str(if v == "0" || v == "false" { "false" } else { "true" }),
                            Some("integer") => match v.trim().parse::<i128>() {
                                Ok(i) => out.push_str(&i.to_string()),
                                Err(_) => out.push_str(v),
                            },
                            Some("decimal" | "float" | "double") => {
                                // Engine: canonical numeric value string (scaled-decimal
                                // normalization) — outside the parity whitelist.
                                unsupported.set(true);
                                return None;
                            }
                            _ => out.push_str(v),
                        }
                    }
                    N3Term::Iri(i) => out.push_str(i),
                    _ => return None,
                }
            }
            let lit = N3Term::Lit(out, N3_XSD_STRING.into(), None);
            let mut nb = b.clone();
            n3_unify(o, &lit, &mut nb).then_some(nb)
        }
        N3Builtin::Scrape => {
            let N3Term::List(ms) = &sv else {
                if matches!(sv, N3Term::Iri(_) | N3Term::Blank(_)) {
                    unsupported.set(true);
                }
                return None;
            };
            if ms.len() != 2 {
                return None;
            }
            let (N3Term::Lit(text, _, _), N3Term::Lit(pat, _, _)) = (&ms[0], &ms[1]) else {
                return None;
            };
            let re = regex::Regex::new(pat).ok()?;
            let cap = re.captures(text)?.get(1)?.as_str().to_string();
            let lit = N3Term::Lit(cap, N3_XSD_STRING.into(), None);
            let mut nb = b.clone();
            n3_unify(o, &lit, &mut nb).then_some(nb)
        }
        N3Builtin::EncodeForUri => {
            if matches!(sv, N3Term::Iri(_) | N3Term::Blank(_) | N3Term::List(_)) {
                // List subject (engine encodes the first member) / data list: out of parity.
                unsupported.set(true);
                return None;
            }
            let N3Term::Lit(v, _, _) = &sv else { return None };
            let lit = N3Term::Lit(crate::n3::encode_for_uri(v), N3_XSD_STRING.into(), None);
            let mut nb = b.clone();
            n3_unify(o, &lit, &mut nb).then_some(nb)
        }
    }
}

/// Evaluate one rule's premise. `seed = Some((plain_ix, fact))` is the counting
/// delta-rewriting: the seed plain atom matches exactly `fact`; plain atoms BEFORE it (by
/// their ORIGINAL `plain_ix`) match the pre-delta store, atoms after it the post-delta store
/// (see [`N3Cx::atom_candidates`]).
///
/// EVALUATION order is dynamic — joins are commutative, so the binding set is unchanged —
/// to keep each step anchored: the seed atom runs first, then greedily the most-bound plain
/// atom / any ready builtin; guards (pure filters over already-bound variables) run last,
/// which is where the engine's `order_premise` lands them too. Without this, atoms placed
/// before the seed in premise order would enumerate whole predicate extents per seed fact
/// (quadratic in the extent — measured 18s on the 1k-doc WAC pod, vs ~1s with anchoring).
fn n3_fire(rule: &N3CompiledRule, cx: &N3Cx, seed: Option<(usize, &[N3Term; 3])>) -> Vec<NB> {
    let mut binds: Vec<NB> = vec![NB::default()];
    let mut bound: FxHashSet<String> = FxHashSet::default();
    let mut remaining: Vec<&N3Atom> = Vec::with_capacity(rule.atoms.len());
    // Seed first.
    for atom in &rule.atoms {
        if let (N3Atom::Plain { pat, plain_ix }, Some((six, f))) = (atom, seed) {
            if *plain_ix == six {
                let mut next = Vec::new();
                for b in &binds {
                    if let Some(nb) = n3_unify_atom(pat, f, b) {
                        next.push(nb);
                    }
                }
                binds = next;
                for t in pat {
                    n3_term_vars(t, &mut bound);
                }
                continue;
            }
        }
        remaining.push(atom);
    }
    let anchored = |t: &N3Term, bound: &FxHashSet<String>| match t {
        N3Term::Var(v) => bound.contains(v),
        _ => true,
    };
    while !remaining.is_empty() && !binds.is_empty() {
        // Pick: ready builtin > most-anchored plain atom > guard (last).
        let mut pick: Option<usize> = None;
        let mut best_score = -1i32;
        for (i, atom) in remaining.iter().enumerate() {
            match atom {
                N3Atom::Builtin { op, s, o } => {
                    let s_ready = {
                        let mut vs = FxHashSet::default();
                        n3_term_vars(s, &mut vs);
                        vs.is_subset(&bound)
                    };
                    let o_ready = {
                        let mut vs = FxHashSet::default();
                        n3_term_vars(o, &mut vs);
                        vs.is_subset(&bound)
                    };
                    let ready = match op {
                        N3Builtin::LogEq | N3Builtin::LogNe => s_ready && o_ready,
                        N3Builtin::LogUri => s_ready || o_ready,
                        _ => s_ready,
                    };
                    if ready {
                        pick = Some(i);
                        break;
                    }
                }
                N3Atom::Plain { pat, .. } => {
                    let score = i32::from(anchored(&pat[0], &bound))
                        + i32::from(anchored(&pat[2], &bound));
                    if score > best_score {
                        best_score = score;
                        pick = Some(i);
                    }
                }
                N3Atom::Guard { .. } => {}
            }
        }
        // Only guards (or a never-ready builtin) left: take the first remaining.
        let i = pick.unwrap_or(0);
        let atom = remaining.remove(i);
        match atom {
            N3Atom::Plain { pat, plain_ix } => {
                let phase = match seed {
                    Some((six, _)) if *plain_ix < six => N3Phase::Pre,
                    Some(_) => N3Phase::Post,
                    None => N3Phase::Free,
                };
                let mut next = Vec::new();
                for b in &binds {
                    let (s, o) = (n3_apply(&pat[0], b), n3_apply(&pat[2], b));
                    for f in cx.atom_candidates(&s, &pat[1], &o, phase) {
                        if let Some(nb) = n3_unify_atom(pat, &f, b) {
                            next.push(nb);
                        }
                    }
                }
                binds = next;
                for t in pat {
                    n3_term_vars(t, &mut bound);
                }
            }
            N3Atom::Builtin { op, s, o } => {
                binds = binds
                    .into_iter()
                    .filter_map(|b| eval_n3_builtin(*op, s, o, &b, cx.unsupported))
                    .collect();
                n3_term_vars(s, &mut bound);
                n3_term_vars(o, &mut bound);
            }
            N3Atom::Guard { inner } => {
                binds.retain(|b| !n3_guard_matches(inner, cx.guards, b));
            }
        }
    }
    binds
}

/// Ground conclusion instantiations of one full premise binding (non-ground conclusion atoms
/// are skipped, mirroring the engine's `ground_triple`).
fn n3_emit(rule: &N3CompiledRule, b: &NB, out: &mut Vec<[N3Term; 3]>) {
    for c in &rule.conclusions {
        let g = [n3_apply(&c[0], b), n3_apply(&c[1], b), n3_apply(&c[2], b)];
        if g.iter().all(N3Term::is_ground) {
            out.push(g);
        }
    }
}

// ---- rule analysis -----------------------------------------------------------------------

fn n3_pred_iri(t: &N3Term) -> Option<&str> {
    match t {
        N3Term::Iri(i) => Some(i.as_str()),
        _ => None,
    }
}

fn n3_whitelisted(p: &str) -> Option<N3Builtin> {
    if let Some(f) = p.strip_prefix(N3_LOG) {
        return match f {
            "uri" => Some(N3Builtin::LogUri),
            "equalTo" => Some(N3Builtin::LogEq),
            "notEqualTo" => Some(N3Builtin::LogNe),
            _ => None,
        };
    }
    if let Some(f) = p.strip_prefix(N3_STR) {
        return match f {
            "concatenation" => Some(N3Builtin::Concat),
            "scrape" => Some(N3Builtin::Scrape),
            "encodeForUri" => Some(N3Builtin::EncodeForUri),
            _ => None,
        };
    }
    None
}

/// Is this term shape allowed in a plain (join) atom / conclusion atom position?
fn n3_simple_term(t: &N3Term) -> bool {
    matches!(t, N3Term::Iri(_) | N3Term::Lit(..) | N3Term::Var(_))
}

fn n3_term_vars(t: &N3Term, out: &mut FxHashSet<String>) {
    match t {
        N3Term::Var(v) => {
            out.insert(v.clone());
        }
        N3Term::List(ms) => ms.iter().for_each(|m| n3_term_vars(m, out)),
        N3Term::Formula(ts) => {
            ts.iter().for_each(|r| r.iter().for_each(|m| n3_term_vars(m, out)))
        }
        _ => {}
    }
}

/// Mirror of the engine's `order_premise`, restricted to the categories the whitelist admits
/// (plain joins always ready and in order; comparisons need both sides; log:uri either side;
/// functional builtins the subject; guards the — never-produced — subject, so they run when
/// nothing else is ready, exactly like the engine).
fn n3_order_premise(premise: &[[N3Term; 3]]) -> Vec<[N3Term; 3]> {
    let vars_of = |t: &N3Term| {
        let mut s = FxHashSet::default();
        n3_term_vars(t, &mut s);
        s
    };
    let is_scope = |p: &N3Term| {
        n3_pred_iri(p).is_some_and(|i| {
            matches!(i.strip_prefix(N3_LOG), Some("includes" | "notIncludes" | "supports"))
        })
    };
    let mut remaining: Vec<usize> = (0..premise.len()).collect();
    let mut produced: FxHashSet<String> = FxHashSet::default();
    let mut out = Vec::new();
    while !remaining.is_empty() {
        let ready = |i: usize| -> bool {
            let pat = &premise[i];
            let kind = n3_pred_iri(&pat[1]).and_then(n3_whitelisted);
            if kind.is_none() && !is_scope(&pat[1]) {
                return true; // join atom
            }
            let subj_ready = vars_of(&pat[0]).is_subset(&produced);
            let obj_ready = vars_of(&pat[2]).is_subset(&produced);
            match kind {
                Some(N3Builtin::LogEq | N3Builtin::LogNe) => subj_ready && obj_ready,
                Some(N3Builtin::LogUri) => subj_ready || obj_ready,
                Some(_) => subj_ready,
                None => subj_ready, // scope op
            }
        };
        let pos = remaining.iter().position(|&i| ready(i)).unwrap_or(0);
        let i = remaining.remove(pos);
        for t in &premise[i] {
            n3_term_vars(t, &mut produced);
        }
        out.push(premise[i].clone());
    }
    out
}

fn n3_iri(ns: &str, frag: &str) -> String {
    let mut s = String::with_capacity(ns.len() + frag.len());
    s.push_str(ns);
    s.push_str(frag);
    s
}

/// Analyze + compile the parsed rules for the counting fast path (Err = the human-readable
/// disqualification reason; the graph then runs in fallback mode).
fn n3_compile(parsed: &n3p::Parsed) -> Result<N3Compiled, String> {
    if !parsed.backward_rules.is_empty() {
        return Err("backward (<=) rules present".into());
    }
    struct RawRule {
        atoms: Vec<N3Atom>,
        conclusions: Vec<[N3Term; 3]>,
        premise_preds: FxHashSet<String>,
        conclusion_preds: FxHashSet<String>,
        guard_preds: FxHashSet<String>,
    }
    let not_includes = n3_iri(N3_LOG, "notIncludes");
    let mut raws: Vec<RawRule> = Vec::new();
    for (rix, rule) in parsed.rules.iter().enumerate() {
        let ordered = n3_order_premise(&rule.premise);
        let mut atoms = Vec::new();
        let mut n_plain = 0usize;
        let mut premise_preds = FxHashSet::default();
        let mut guard_preds = FxHashSet::default();
        let mut guard_subject_vars: Vec<String> = Vec::new();
        for pat in &ordered {
            let Some(p) = n3_pred_iri(&pat[1]) else {
                return Err(format!("rule {rix}: variable/non-IRI predicate in premise"));
            };
            if let Some(op) = n3_whitelisted(p) {
                for t in [&pat[0], &pat[2]] {
                    if matches!(t, N3Term::Formula(_)) {
                        return Err(format!("rule {rix}: formula argument to builtin {p}"));
                    }
                }
                atoms.push(N3Atom::Builtin { op, s: pat[0].clone(), o: pat[2].clone() });
                continue;
            }
            if p == not_includes {
                let N3Term::Var(sv) = &pat[0] else {
                    return Err(format!(
                        "rule {rix}: log:notIncludes with a non-variable subject \
                         (formula containment) is outside the counting profile"
                    ));
                };
                let N3Term::Formula(inner) = &pat[2] else {
                    return Err(format!("rule {rix}: log:notIncludes object is not a formula"));
                };
                if inner.is_empty() {
                    return Err(format!("rule {rix}: empty log:notIncludes pattern"));
                }
                for ia in inner {
                    let Some(ip) = n3_pred_iri(&ia[1]) else {
                        return Err(format!("rule {rix}: variable predicate inside notIncludes"));
                    };
                    if ip.starts_with(SWAP_NS) || ip == N3_RDF_FIRST || ip == N3_RDF_REST {
                        return Err(format!("rule {rix}: builtin inside notIncludes pattern"));
                    }
                    if !n3_simple_term(&ia[0]) || !n3_simple_term(&ia[2]) {
                        return Err(format!("rule {rix}: non-simple term inside notIncludes"));
                    }
                    guard_preds.insert(ip.to_string());
                }
                guard_subject_vars.push(sv.clone());
                atoms.push(N3Atom::Guard { inner: inner.clone() });
                continue;
            }
            if p.starts_with(SWAP_NS) || p == N3_RDF_FIRST || p == N3_RDF_REST {
                return Err(format!("rule {rix}: unsupported builtin predicate {p}"));
            }
            // Plain join atom.
            if !n3_simple_term(&pat[0]) || !n3_simple_term(&pat[2]) {
                return Err(format!(
                    "rule {rix}: non-simple term in join atom (blank/list/formula)"
                ));
            }
            premise_preds.insert(p.to_string());
            atoms.push(N3Atom::Plain { pat: pat.clone(), plain_ix: n_plain });
            n_plain += 1;
        }
        // [OPUS-4.8] Counting propagation seeds emissions ONLY from N3Atom::Plain premise atoms
        // (see `propagate` / the rematerialize seed). A rule with NO plain join atom — an empty
        // premise `{}` rule or a builtin/guard-only premise — would therefore never fire in
        // counting mode, so its conclusions would be missing from the maintained closure even
        // though the batch engine (reason_n3_terms) derives them. Reject such rules so the graph
        // runs in Fallback mode (always correct). See reviews 1868 / 1884.
        if n_plain == 0 {
            return Err(format!(
                "rule {rix}: premise has no plain join atom (empty or builtin/guard-only premise) \
                 — outside the counting profile (seeds only from plain premise atoms)"
            ));
        }
        // The guard subject (?UNSCOPED) must stay unbound: at most this one occurrence.
        for sv in &guard_subject_vars {
            let occurrences: usize = ordered
                .iter()
                .flat_map(|pat| pat.iter())
                .chain(rule.conclusion.iter().flat_map(|c| c.iter()))
                .map(|t| {
                    let mut vs = FxHashSet::default();
                    n3_term_vars(t, &mut vs);
                    usize::from(vs.contains(sv))
                })
                .sum();
            if occurrences > 1 {
                return Err(format!("rule {rix}: notIncludes subject ?{sv} is used elsewhere"));
            }
        }
        // Conclusions: simple ground-predicate atoms, no blanks (existentials).
        let mut conclusion_preds = FxHashSet::default();
        for c in &rule.conclusion {
            let Some(cp) = n3_pred_iri(&c[1]) else {
                return Err(format!("rule {rix}: variable/non-IRI predicate in conclusion"));
            };
            if cp.starts_with(SWAP_NS) {
                return Err(format!("rule {rix}: SWAP-vocabulary conclusion predicate {cp}"));
            }
            if !n3_simple_term(&c[0]) || !n3_simple_term(&c[2]) {
                return Err(format!(
                    "rule {rix}: non-simple conclusion term \
                     (blank-node existential / list / formula)"
                ));
            }
            conclusion_preds.insert(cp.to_string());
        }
        raws.push(RawRule {
            atoms,
            conclusions: rule.conclusion.clone(),
            premise_preds,
            conclusion_preds,
            guard_preds,
        });
    }
    // Guard predicates must be input-only (derived by no rule).
    let mut guard_preds: FxHashSet<String> = FxHashSet::default();
    for r in &raws {
        guard_preds.extend(r.guard_preds.iter().cloned());
    }
    for r in &raws {
        if let Some(p) = r.conclusion_preds.iter().find(|p| guard_preds.contains(*p)) {
            return Err(format!("negation over derived predicate {p} (not input-stratified)"));
        }
    }
    // Predicate dependency graph → SCCs → recursive layers.
    let mut node_ix: FxHashMap<String, usize> = FxHashMap::default();
    let mut nodes: Vec<String> = Vec::new();
    let ix_of = |p: &str, nodes: &mut Vec<String>, node_ix: &mut FxHashMap<String, usize>| {
        *node_ix.entry(p.to_string()).or_insert_with(|| {
            nodes.push(p.to_string());
            nodes.len() - 1
        })
    };
    let mut edges: FxHashMap<usize, FxHashSet<usize>> = FxHashMap::default();
    for r in &raws {
        for q in &r.premise_preds {
            let qi = ix_of(q, &mut nodes, &mut node_ix);
            for c in &r.conclusion_preds {
                let ci = ix_of(c, &mut nodes, &mut node_ix);
                edges.entry(qi).or_default().insert(ci);
            }
        }
        for c in &r.conclusion_preds {
            ix_of(c, &mut nodes, &mut node_ix);
        }
    }
    let sccs = n3_sccs(nodes.len(), &edges);
    let recursive: Vec<&Vec<usize>> = sccs
        .iter()
        .filter(|scc| {
            scc.len() > 1 || edges.get(&scc[0]).is_some_and(|e| e.contains(&scc[0]))
        })
        .collect();
    let scc_of: FxHashMap<usize, usize> = recursive
        .iter()
        .enumerate()
        .flat_map(|(k, scc)| scc.iter().map(move |&n| (n, k)))
        .collect();
    let mut layer_rules: Vec<Vec<N3CompiledRule>> = vec![Vec::new(); recursive.len()];
    let mut layer_in: Vec<FxHashSet<String>> = vec![FxHashSet::default(); recursive.len()];
    let mut counted = Vec::new();
    for r in &raws {
        let compiled =
            N3CompiledRule { atoms: r.atoms.clone(), conclusions: r.conclusions.clone() };
        let in_sccs: FxHashSet<usize> = r
            .conclusion_preds
            .iter()
            .filter_map(|p| scc_of.get(&node_ix[p]).copied())
            .collect();
        match in_sccs.len() {
            0 => counted.push(compiled),
            1 => {
                let k = *in_sccs.iter().next().unwrap();
                if !r.conclusion_preds.iter().all(|p| scc_of.get(&node_ix[p]) == Some(&k)) {
                    return Err(
                        "rule concludes both inside and outside a recursive component".into()
                    );
                }
                layer_in[k].extend(r.premise_preds.iter().cloned());
                layer_rules[k].push(compiled);
            }
            _ => return Err("rule concludes into two distinct recursive components".into()),
        }
    }
    let layers: Vec<N3LayerDef> = recursive
        .iter()
        .enumerate()
        .map(|(k, scc)| {
            let mut relevant = std::mem::take(&mut layer_in[k]);
            relevant.extend(scc.iter().map(|&n| nodes[n].clone()));
            N3LayerDef { rules: std::mem::take(&mut layer_rules[k]), relevant }
        })
        .collect();
    Ok(N3Compiled { counted, layers, guard_preds })
}

/// Iterative Tarjan SCC; emitted in DEPENDENCY-FIRST (topological) order of the condensation.
fn n3_sccs(n: usize, edges: &FxHashMap<usize, FxHashSet<usize>>) -> Vec<Vec<usize>> {
    let succ: Vec<Vec<usize>> = (0..n)
        .map(|i| edges.get(&i).map(|s| s.iter().copied().collect()).unwrap_or_default())
        .collect();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next_index = 0usize;
    let mut out: Vec<Vec<usize>> = Vec::new();
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut call: Vec<(usize, usize)> = vec![(root, 0)]; // (node, next-successor pos)
        while let Some(&mut (v, ref mut pos)) = call.last_mut() {
            if *pos == 0 {
                index[v] = next_index;
                low[v] = next_index;
                next_index += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            if let Some(&w) = succ[v].get(*pos) {
                *pos += 1;
                if index[w] == usize::MAX {
                    call.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                call.pop();
                if let Some(&(parent, _)) = call.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut scc = Vec::new();
                    loop {
                        let w = stack.pop().unwrap();
                        on_stack[w] = false;
                        scc.push(w);
                        if w == v {
                            break;
                        }
                    }
                    out.push(scc);
                }
            }
        }
    }
    out.reverse(); // Tarjan emits dependents-first; reverse for dependencies-first.
    out
}

// ---- serialization (the fallback / oracle path) --------------------------------------------
//
// [OPUS-5] sq-xqchl.2 — the writer itself now lives in `n3::serialize`, shared with the
// rule writer that echoes rules back into an EYE `--pass-all` document. One definition, so
// a serializer and its parser cannot drift apart.
pub(crate) use crate::n3::serialize::serialize_facts as n3_serialize;

// ---- the graph -----------------------------------------------------------------------------

impl MaterializedN3Graph {
    /// Build the graph from an N3 RULES document (forward rules; any facts in the document
    /// join the base) plus `base_facts`, and fully materialize the closure. The rules are
    /// analyzed once: if they qualify (see the module notes) the graph runs in
    /// [`N3Mode::Counting`] and ABox mutations are incremental; otherwise it runs in
    /// [`N3Mode::Fallback`] (every mutation re-runs the batch engine — always correct).
    ///
    /// Errors only on rules-document parse failure.
    pub fn new(rules_src: &str, base_facts: &[[N3Term; 3]]) -> Result<Self, String> {
        let parsed = n3p::parse(rules_src)?;
        let (compiled, disqualified) = match n3_compile(&parsed) {
            Ok(c) => (Some(Rc::new(c)), None),
            Err(reason) => (None, Some(reason)),
        };
        let mut base: FxHashSet<[N3Term; 3]> = parsed.facts.iter().cloned().collect();
        base.extend(base_facts.iter().cloned());
        let n_layers = compiled.as_ref().map_or(0, |c| c.layers.len());
        let mut g = MaterializedN3Graph {
            rules_src: rules_src.to_string(),
            compiled,
            disqualified,
            data_fallback: None,
            data_rule_fallback: None,
            base,
            mode: N3Mode::Fallback,
            counts: FxHashMap::default(),
            index: N3Index::default(),
            layer_derived: vec![FxHashSet::default(); n_layers],
            fallback_closure: FxHashSet::default(),
            rebuilds: 0,
        };
        g.rematerialize();
        g.rebuilds = 0;
        Ok(g)
    }

    /// Does mutating `t` require a full rebuild? (Guard predicates — their truth is baked
    /// into every count — and `log:implies`-family predicates, which the batch engine would
    /// parse as RULES.)
    fn triggers_rebuild(&self, t: &[N3Term; 3]) -> bool {
        let Some(p) = n3_pred_iri(&t[1]) else { return true };
        if p == n3_iri(N3_LOG, "implies")
            || p == n3_iri(N3_LOG, "isImpliedBy")
            || p == n3_iri(N3_LOG, "impliedBy")
        {
            return true;
        }
        self.compiled.as_ref().is_some_and(|c| c.guard_preds.contains(p))
    }

    fn rematerialize(&mut self) {
        self.rebuilds += 1;
        self.counts.clear();
        self.index = N3Index::default();
        for d in &mut self.layer_derived {
            d.clear();
        }
        self.fallback_closure.clear();
        let implies = [
            n3_iri(N3_LOG, "implies"),
            n3_iri(N3_LOG, "isImpliedBy"),
            n3_iri(N3_LOG, "impliedBy"),
        ];
        let data_rules = self
            .base
            .iter()
            .any(|t| n3_pred_iri(&t[1]).is_some_and(|p| implies.iter().any(|i| i == p)));
        // [OPUS-4.8] Record (or clear) the non-sticky data-rule fallback cause so fallback_reason()
        // reports it instead of silently returning None. See review 1868.
        self.data_rule_fallback = data_rules.then(|| {
            "base contains log:implies-family triples (rules-as-data); the batch engine reasons \
             over them — outside the counting profile"
                .to_string()
        });
        if self.compiled.is_some() && self.data_fallback.is_none() && !data_rules {
            self.mode = N3Mode::Counting;
            let pending: Vec<[N3Term; 3]> = self.base.iter().cloned().collect();
            if self.propagate(pending, true) {
                return;
            }
            // Runtime disqualification (unsupported data) — fall through to the engine.
            self.counts.clear();
            self.index = N3Index::default();
            for d in &mut self.layer_derived {
                d.clear();
            }
        }
        self.mode = N3Mode::Fallback;
        let src = format!("{}\n{}", self.rules_src, n3_serialize(self.base.iter()));
        let closure = crate::n3::reason_n3_terms(&src, None)
            .expect("re-serialized base must re-parse (serializer bug)");
        self.fallback_closure = closure.facts.into_iter().collect();
    }

    fn in_any_layer(&self, f: &[N3Term; 3]) -> bool {
        self.layer_derived.iter().any(|d| d.contains(f))
    }

    /// Round-based delta propagation (sign-homogeneous; see the module notes). Returns false
    /// if evaluation met unsupported data — the caller re-materializes via the engine.
    fn propagate(&mut self, mut pending: Vec<[N3Term; 3]>, inserting: bool) -> bool {
        let Some(compiled) = self.compiled.clone() else { return false };
        let unsupported = Cell::new(false);
        while !pending.is_empty() {
            let mut pend_set: FxHashSet<[N3Term; 3]> = pending.iter().cloned().collect();
            for f in &pending {
                if inserting {
                    self.index.insert(f.clone());
                } else {
                    self.index.remove(f);
                }
            }
            let mut next: Vec<[N3Term; 3]> = Vec::new();
            let mut queued: FxHashSet<[N3Term; 3]> = FxHashSet::default();
            // 1. Recursive layers (dependency order), recomputed when touched. This runs
            //    BEFORE the counted-rule step so a base↔layer ownership transfer is settled
            //    while the round's delta can still be corrected — no derivation count has
            //    been decremented yet (see the hand-off note below). Layers are emitted in
            //    dependency-first order, so re-inserting a fact a later layer owns cannot
            //    invalidate an already-recomputed earlier layer.
            for li in 0..compiled.layers.len() {
                let def = &compiled.layers[li];
                let relevant = |f: &[N3Term; 3]| {
                    n3_pred_iri(&f[1]).is_some_and(|p| def.relevant.contains(p))
                };
                if !pending.iter().any(relevant) {
                    continue;
                }
                let (added, removed) = self.recompute_layer(&compiled, li, &unsupported);
                if unsupported.get() {
                    self.data_fallback =
                        Some("evaluation met data outside the builtin-parity whitelist".into());
                    return false;
                }
                // Monotone rules + fixed guards ⇒ diffs normally share the round's sign. The
                // exception is a base↔layer OWNERSHIP TRANSFER: a fact that is both asserted
                // and layer-derivable is excluded from `layer_derived` while asserted (it
                // seeds the local fixpoint), so deleting its base copy makes it APPEAR in the
                // layer diff (and asserting an already-derived fact makes it vanish) without
                // its closure membership changing. That is a state hand-off, not a data
                // disqualification — and the layer fixpoint just recomputed above IS the
                // targeted local re-derivation that decides it, so settle it here instead of
                // re-materializing the whole closure. Anything the hand-off does not explain
                // is still a genuine opposite-sign diff: bail and let the caller rebuild.
                if inserting {
                    // A fact may only leave the layer because it is now independently
                    // present (asserted, counted, or owned by another layer).
                    let handed_off = removed.iter().all(|f| {
                        self.index.contains(f)
                            && (self.base.contains(f)
                                || self.counts.contains_key(f)
                                || self.in_any_layer(f))
                    });
                    if !handed_off {
                        return false;
                    }
                    for f in added {
                        if !self.index.contains(&f) && queued.insert(f.clone()) {
                            next.push(f);
                        }
                    }
                } else {
                    // A fact may only enter the layer because this round removed the base
                    // copy it was charged to; it never left the closure, so put it back and
                    // drop it from the round's delta — nothing downstream of it changed.
                    if !added.iter().all(|f| pend_set.contains(f)) {
                        return false;
                    }
                    for f in &added {
                        self.index.insert(f.clone());
                        pend_set.remove(f);
                    }
                    if !added.is_empty() {
                        pending.retain(|f| pend_set.contains(f));
                    }
                    for f in removed {
                        if self.index.contains(&f)
                            && !self.base.contains(&f)
                            && !self.counts.contains_key(&f)
                            && !self.in_any_layer(&f)
                            && queued.insert(f.clone())
                        {
                            next.push(f);
                        }
                    }
                }
            }
            // 2. Counted rules: the delta-rewriting emission multiset.
            let mut emissions: Vec<[N3Term; 3]> = Vec::new();
            {
                let cx = N3Cx {
                    facts: &self.index,
                    guards: &self.index,
                    delta: Some((&pend_set, inserting)),
                    unsupported: &unsupported,
                };
                for rule in &compiled.counted {
                    for atom in &rule.atoms {
                        let N3Atom::Plain { pat, plain_ix } = atom else { continue };
                        for f in &pending {
                            if f[1] != pat[1] {
                                continue;
                            }
                            for b in n3_fire(rule, &cx, Some((*plain_ix, f))) {
                                n3_emit(rule, &b, &mut emissions);
                            }
                        }
                    }
                }
            }
            if unsupported.get() {
                self.data_fallback =
                    Some("evaluation met data outside the builtin-parity whitelist".into());
                return false;
            }
            if inserting {
                for g in emissions {
                    let c = self.counts.entry(g.clone()).or_insert(0);
                    *c += 1;
                    if *c == 1
                        && !self.index.contains(&g)
                        && !self.in_any_layer(&g)
                        && queued.insert(g.clone())
                    {
                        next.push(g);
                    }
                }
            } else {
                for g in emissions {
                    match self.counts.get_mut(&g) {
                        Some(c) if *c > 1 => *c -= 1,
                        Some(_) => {
                            self.counts.remove(&g);
                            if self.index.contains(&g)
                                && !self.base.contains(&g)
                                && !self.in_any_layer(&g)
                                && queued.insert(g.clone())
                            {
                                next.push(g);
                            }
                        }
                        None => debug_assert!(false, "N3 derivation-count underflow"),
                    }
                }
            }
            pending = next;
        }
        true
    }

    /// Re-run layer `li`'s local fixpoint over the current closure's relevant-predicate
    /// extents; returns the (added, removed) diff of the layer's derived contribution.
    fn recompute_layer(
        &mut self,
        compiled: &N3Compiled,
        li: usize,
        unsupported: &Cell<bool>,
    ) -> (Vec<[N3Term; 3]>, Vec<[N3Term; 3]>) {
        let def = &compiled.layers[li];
        // Seed: relevant facts of the closure, minus the layer's own (possibly stale)
        // contribution — unless a fact is independently present (base / counted).
        let mut seed: Vec<[N3Term; 3]> = Vec::new();
        for p in &def.relevant {
            if let Some(facts) = self.index.pred.get(&N3Term::Iri(p.clone())) {
                for f in facts {
                    let stale_own = self.layer_derived[li].contains(f)
                        && !self.base.contains(f)
                        && !self.counts.contains_key(f);
                    if !stale_own {
                        seed.push(f.clone());
                    }
                }
            }
        }
        let seed_set: FxHashSet<[N3Term; 3]> = seed.iter().cloned().collect();
        let mut local = N3Index::default();
        for f in &seed {
            local.insert(f.clone());
        }
        let mut pending = seed;
        while !pending.is_empty() {
            let mut emissions = Vec::new();
            {
                let cx =
                    N3Cx { facts: &local, guards: &self.index, delta: None, unsupported };
                for rule in &def.rules {
                    for atom in &rule.atoms {
                        let N3Atom::Plain { pat, plain_ix } = atom else { continue };
                        for f in &pending {
                            if f[1] != pat[1] {
                                continue;
                            }
                            for b in n3_fire(rule, &cx, Some((*plain_ix, f))) {
                                n3_emit(rule, &b, &mut emissions);
                            }
                        }
                    }
                }
            }
            if unsupported.get() {
                return (Vec::new(), Vec::new());
            }
            let mut fresh = Vec::new();
            for g in emissions {
                if local.insert(g.clone()) {
                    fresh.push(g);
                }
            }
            pending = fresh;
        }
        let new_derived: FxHashSet<[N3Term; 3]> =
            local.all.into_iter().filter(|f| !seed_set.contains(f)).collect();
        let added: Vec<[N3Term; 3]> =
            new_derived.difference(&self.layer_derived[li]).cloned().collect();
        let removed: Vec<[N3Term; 3]> =
            self.layer_derived[li].difference(&new_derived).cloned().collect();
        self.layer_derived[li] = new_derived;
        (added, removed)
    }

    /// Insert base facts (set semantics; non-ground triples are ignored). Counting mode
    /// updates the closure incrementally; guard-predicate / implies-as-data deltas, and any
    /// mutation in fallback mode, re-materialize. Returns how many facts joined the base.
    pub fn insert(&mut self, facts: &[[N3Term; 3]]) -> usize {
        let mut added = Vec::new();
        let mut rebuild = false;
        for f in facts {
            if !f.iter().all(N3Term::is_ground) {
                continue;
            }
            if self.base.insert(f.clone()) {
                rebuild |= self.triggers_rebuild(f);
                added.push(f.clone());
            }
        }
        if added.is_empty() {
            return 0;
        }
        if rebuild || self.mode == N3Mode::Fallback {
            self.rematerialize();
            return added.len();
        }
        let pending: Vec<[N3Term; 3]> =
            added.iter().filter(|f| !self.index.contains(f)).cloned().collect();
        if !self.propagate(pending, true) {
            self.rematerialize();
        }
        added.len()
    }

    /// Delete base facts (facts not asserted — including derived-only facts — are ignored;
    /// a deleted base fact that is still derivable stays in the closure). Returns how many
    /// facts left the base.
    pub fn delete(&mut self, facts: &[[N3Term; 3]]) -> usize {
        let mut removed = Vec::new();
        let mut rebuild = false;
        for f in facts {
            if self.base.remove(f) {
                rebuild |= self.triggers_rebuild(f);
                removed.push(f.clone());
            }
        }
        if removed.is_empty() {
            return 0;
        }
        if rebuild || self.mode == N3Mode::Fallback {
            self.rematerialize();
            return removed.len();
        }
        let pending: Vec<[N3Term; 3]> = removed
            .iter()
            .filter(|f| !self.counts.contains_key(*f) && !self.in_any_layer(f))
            .cloned()
            .collect();
        if !self.propagate(pending, false) {
            self.rematerialize();
        }
        removed.len()
    }

    /// Is `t` in the materialized closure (asserted or derived)?
    pub fn contains(&self, t: &[N3Term; 3]) -> bool {
        match self.mode {
            N3Mode::Counting => self.index.contains(t),
            N3Mode::Fallback => self.fallback_closure.contains(t),
        }
    }

    /// The full materialized closure (unordered; [`Term`](crate::n3::Term) has no ordering).
    pub fn closure(&self) -> Vec<[N3Term; 3]> {
        match self.mode {
            N3Mode::Counting => self.index.all.iter().cloned().collect(),
            N3Mode::Fallback => self.fallback_closure.iter().cloned().collect(),
        }
    }

    /// Number of facts in the closure.
    pub fn len(&self) -> usize {
        match self.mode {
            N3Mode::Counting => self.index.all.len(),
            N3Mode::Fallback => self.fallback_closure.len(),
        }
    }

    /// Is the closure empty?
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of asserted (base) facts.
    pub fn base_len(&self) -> usize {
        self.base.len()
    }

    /// The active maintenance regime.
    pub fn mode(&self) -> N3Mode {
        self.mode
    }

    /// Why the graph is (or would be) in fallback mode: the rule-analysis disqualification, the
    /// sticky runtime data disqualification, or the (non-sticky) presence of `log:implies`-family
    /// rules-as-data in the base. `None` ⇔ the counting path is active. See review 1868.
    pub fn fallback_reason(&self) -> Option<&str> {
        // [OPUS-4.8] Include the data-rule cause so the documented `None ⇔ counting` contract
        // holds even when fallback is forced by implies-as-data rather than rule analysis.
        self.data_fallback
            .as_deref()
            .or(self.disqualified.as_deref())
            .or(self.data_rule_fallback.as_deref())
    }

    /// How many times a mutation re-materialized from scratch (guard-predicate deltas,
    /// implies-as-data, and every mutation in fallback mode).
    pub fn full_rebuilds(&self) -> usize {
        self.rebuilds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materialize_rdfs;
    use oxrdf::vocab::{rdf, rdfs};

    struct V {
        ty: Id,
        sc: Id,
        sp: Id,
        dom: Id,
        rng: Id,
    }

    fn vocab(dict: &mut Dict) -> V {
        V {
            ty: dict.intern_iri(rdf::TYPE.as_str()),
            sc: dict.intern_iri(rdfs::SUB_CLASS_OF.as_str()),
            sp: dict.intern_iri(rdfs::SUB_PROPERTY_OF.as_str()),
            dom: dict.intern_iri(rdfs::DOMAIN.as_str()),
            rng: dict.intern_iri(rdfs::RANGE.as_str()),
        }
    }

    fn ex(dict: &mut Dict, local: &str) -> Id {
        dict.intern_iri(&format!("http://ex/{local}"))
    }

    /// Oracle: from-scratch closure of `base` as a set, via the batch API.
    fn oracle(dict: &mut Dict, base: &FxHashSet<[Id; 3]>) -> FxHashSet<[Id; 3]> {
        let mut v: Vec<[Id; 3]> = base.iter().copied().collect();
        materialize_rdfs(dict, &mut v);
        v.into_iter().collect()
    }

    fn assert_matches_oracle(g: &MaterializedGraph, dict: &mut Dict, base: &FxHashSet<[Id; 3]>) {
        let inc: FxHashSet<[Id; 3]> = g.closure().into_iter().collect();
        let full = oracle(dict, base);
        assert_eq!(inc, full, "incremental closure != from-scratch closure");
        assert_eq!(g.len(), inc.len(), "len() inconsistent with closure()");
    }

    /// Small mixed TBox+ABox: Dog ⊑ Mammal ⊑ Animal; hasParent ⊑ relatedTo with
    /// domain/range Person; rex a Dog; a hasParent b.
    fn fixture(dict: &mut Dict) -> (V, Vec<[Id; 3]>) {
        let v = vocab(dict);
        let (dog, mammal, animal) = (ex(dict, "Dog"), ex(dict, "Mammal"), ex(dict, "Animal"));
        let (hp, rel, person) = (ex(dict, "hasParent"), ex(dict, "relatedTo"), ex(dict, "Person"));
        let (rex, a, b) = (ex(dict, "rex"), ex(dict, "a"), ex(dict, "b"));
        let base = vec![
            [dog, v.sc, mammal],
            [mammal, v.sc, animal],
            [hp, v.sp, rel],
            [hp, v.dom, person],
            [hp, v.rng, person],
            [rex, v.ty, dog],
            [a, hp, b],
        ];
        (v, base)
    }

    #[test]
    fn initial_materialization_matches_batch() {
        let mut dict = Dict::new();
        let (_, base) = fixture(&mut dict);
        let g = MaterializedGraph::new(&mut dict, &base);
        let set: FxHashSet<[Id; 3]> = base.iter().copied().collect();
        assert_matches_oracle(&g, &mut dict, &set);
        assert_eq!(g.full_rebuilds(), 0, "new() is not a fallback rebuild");
        assert_eq!(g.base_len(), base.len());
        // [OPUS-4.8] sq-qcnn.16: exercise base_triples() iterator (previously uncovered).
        let base_via_iter: FxHashSet<[Id; 3]> = g.base_triples().collect();
        assert_eq!(base_via_iter, set, "base_triples() iterator yields the exact asserted set");
    }

    #[test]
    fn abox_insert_is_incremental_and_correct() {
        let mut dict = Dict::new();
        let (v, base) = fixture(&mut dict);
        let (dog, hp) = (ex(&mut dict, "Dog"), ex(&mut dict, "hasParent"));
        let (fido, c, d) = (ex(&mut dict, "fido"), ex(&mut dict, "c"), ex(&mut dict, "d"));
        let mut g = MaterializedGraph::new(&mut dict, &base);
        let mut set: FxHashSet<[Id; 3]> = base.iter().copied().collect();

        let delta = [[fido, v.ty, dog], [c, hp, d]];
        assert_eq!(g.insert(&delta), 2);
        set.extend(delta);
        assert_eq!(g.full_rebuilds(), 0, "ABox insert must not rebuild");
        assert_matches_oracle(&g, &mut dict, &set);
        // Spot-check entailments: fido is an Animal (rdfs9 chain); c is a Person (rdfs2).
        let (animal, person, ty) = (ex(&mut dict, "Animal"), ex(&mut dict, "Person"), v.ty);
        assert!(g.contains(&[fido, ty, animal]));
        assert!(g.contains(&[c, ty, person]));
    }

    #[test]
    fn abox_delete_is_incremental_and_correct() {
        let mut dict = Dict::new();
        let (v, base) = fixture(&mut dict);
        let (rex, dog, animal) = (ex(&mut dict, "rex"), ex(&mut dict, "Dog"), ex(&mut dict, "Animal"));
        let mut g = MaterializedGraph::new(&mut dict, &base);
        let mut set: FxHashSet<[Id; 3]> = base.iter().copied().collect();

        assert!(g.contains(&[rex, v.ty, animal]));
        assert_eq!(g.delete(&[[rex, v.ty, dog]]), 1);
        set.remove(&[rex, v.ty, dog]);
        assert_eq!(g.full_rebuilds(), 0, "ABox delete must not rebuild");
        assert!(!g.contains(&[rex, v.ty, animal]), "derived types retract with their support");
        assert_matches_oracle(&g, &mut dict, &set);
    }

    #[test]
    fn deleted_base_fact_survives_if_still_derivable() {
        // (a relatedTo b) asserted AND derived from (a hasParent b) via rdfs7. Deleting the
        // assertion must keep it (count > 0); deleting (a hasParent b) too must drop it.
        // This is exactly the case DRed needs its rederivation pass for.
        let mut dict = Dict::new();
        let (v, mut base) = fixture(&mut dict);
        let (a, b, hp, rel) =
            (ex(&mut dict, "a"), ex(&mut dict, "b"), ex(&mut dict, "hasParent"), ex(&mut dict, "relatedTo"));
        base.push([a, rel, b]); // assert what rdfs7 also derives
        let mut g = MaterializedGraph::new(&mut dict, &base);
        let mut set: FxHashSet<[Id; 3]> = base.iter().copied().collect();

        g.delete(&[[a, rel, b]]);
        set.remove(&[a, rel, b]);
        assert!(g.contains(&[a, rel, b]), "still derived from (a hasParent b)");
        assert_matches_oracle(&g, &mut dict, &set);

        g.delete(&[[a, hp, b]]);
        set.remove(&[a, hp, b]);
        assert!(!g.contains(&[a, rel, b]), "last support gone");
        let person = ex(&mut dict, "Person");
        assert!(!g.contains(&[a, v.ty, person]), "domain typing gone too");
        assert_matches_oracle(&g, &mut dict, &set);
        assert_eq!(g.full_rebuilds(), 0);
    }

    #[test]
    fn multi_support_counting() {
        // Two assertions both derive (s type Person) via hasParent's domain; the fact must
        // survive losing one support and die with the second.
        let mut dict = Dict::new();
        let (v, base) = fixture(&mut dict);
        let (s, o1, o2, hp, person) = (
            ex(&mut dict, "s"),
            ex(&mut dict, "o1"),
            ex(&mut dict, "o2"),
            ex(&mut dict, "hasParent"),
            ex(&mut dict, "Person"),
        );
        let mut g = MaterializedGraph::new(&mut dict, &base);
        let mut set: FxHashSet<[Id; 3]> = base.iter().copied().collect();
        g.insert(&[[s, hp, o1], [s, hp, o2]]);
        set.extend([[s, hp, o1], [s, hp, o2]]);
        assert!(g.contains(&[s, v.ty, person]));

        g.delete(&[[s, hp, o1]]);
        set.remove(&[s, hp, o1]);
        assert!(g.contains(&[s, v.ty, person]), "second derivation still supports it");
        assert_matches_oracle(&g, &mut dict, &set);

        g.delete(&[[s, hp, o2]]);
        set.remove(&[s, hp, o2]);
        assert!(!g.contains(&[s, v.ty, person]), "count reached zero");
        assert_matches_oracle(&g, &mut dict, &set);
    }

    #[test]
    fn insert_of_already_derived_fact_then_unwind() {
        // Insert (rex type Mammal), which is already derived. It must remain after the
        // deriving support (rex type Dog) goes, and only vanish when deleted itself.
        let mut dict = Dict::new();
        let (v, base) = fixture(&mut dict);
        let (rex, dog, mammal) = (ex(&mut dict, "rex"), ex(&mut dict, "Dog"), ex(&mut dict, "Mammal"));
        let mut g = MaterializedGraph::new(&mut dict, &base);
        let mut set: FxHashSet<[Id; 3]> = base.iter().copied().collect();

        g.insert(&[[rex, v.ty, mammal]]);
        set.insert([rex, v.ty, mammal]);
        assert_matches_oracle(&g, &mut dict, &set);

        g.delete(&[[rex, v.ty, dog]]);
        set.remove(&[rex, v.ty, dog]);
        assert!(g.contains(&[rex, v.ty, mammal]), "asserted, even though no longer derived");
        assert_matches_oracle(&g, &mut dict, &set);

        g.delete(&[[rex, v.ty, mammal]]);
        set.remove(&[rex, v.ty, mammal]);
        assert!(!g.contains(&[rex, v.ty, mammal]));
        assert_matches_oracle(&g, &mut dict, &set);
    }

    #[test]
    fn noop_mutations() {
        let mut dict = Dict::new();
        let (v, base) = fixture(&mut dict);
        let (rex, dog, animal, ghost) =
            (ex(&mut dict, "rex"), ex(&mut dict, "Dog"), ex(&mut dict, "Animal"), ex(&mut dict, "ghost"));
        let mut g = MaterializedGraph::new(&mut dict, &base);
        let set: FxHashSet<[Id; 3]> = base.iter().copied().collect();

        assert_eq!(g.insert(&[[rex, v.ty, dog]]), 0, "duplicate insert is a no-op");
        assert_eq!(g.delete(&[[ghost, v.ty, dog]]), 0, "deleting absent triple is a no-op");
        assert_eq!(g.delete(&[[rex, v.ty, animal]]), 0, "derived-only facts cannot be deleted");
        assert!(g.contains(&[rex, v.ty, animal]));
        assert_matches_oracle(&g, &mut dict, &set);
        assert_eq!(g.full_rebuilds(), 0);
    }

    #[test]
    fn tbox_insert_triggers_rebuild_and_is_correct() {
        let mut dict = Dict::new();
        let (v, base) = fixture(&mut dict);
        let (animal, thing, rex) = (ex(&mut dict, "Animal"), ex(&mut dict, "Thing"), ex(&mut dict, "rex"));
        let mut g = MaterializedGraph::new(&mut dict, &base);
        let mut set: FxHashSet<[Id; 3]> = base.iter().copied().collect();

        g.insert(&[[animal, v.sc, thing]]);
        set.insert([animal, v.sc, thing]);
        assert_eq!(g.full_rebuilds(), 1, "TBox insert must fall back to full rematerialization");
        assert!(g.contains(&[rex, v.ty, thing]), "existing ABox re-swept against new TBox");
        assert_matches_oracle(&g, &mut dict, &set);
    }

    #[test]
    fn tbox_delete_triggers_rebuild_and_is_correct() {
        let mut dict = Dict::new();
        let (v, base) = fixture(&mut dict);
        let (mammal, animal, rex) = (ex(&mut dict, "Mammal"), ex(&mut dict, "Animal"), ex(&mut dict, "rex"));
        let mut g = MaterializedGraph::new(&mut dict, &base);
        let mut set: FxHashSet<[Id; 3]> = base.iter().copied().collect();

        g.delete(&[[mammal, v.sc, animal]]);
        set.remove(&[mammal, v.sc, animal]);
        assert_eq!(g.full_rebuilds(), 1, "TBox delete must fall back to full rematerialization");
        assert!(!g.contains(&[rex, v.ty, animal]), "entailments through the removed edge gone");
        assert_matches_oracle(&g, &mut dict, &set);

        // Mixed batch (ABox + TBox) also rebuilds, once.
        let (dog, mammal2, fido) = (ex(&mut dict, "Dog"), ex(&mut dict, "Mammal"), ex(&mut dict, "fido"));
        g.insert(&[[fido, v.ty, dog], [mammal2, v.sc, animal]]);
        set.extend([[fido, v.ty, dog], [mammal2, v.sc, animal]]);
        assert_eq!(g.full_rebuilds(), 2);
        assert_matches_oracle(&g, &mut dict, &set);
    }

    #[test]
    fn empty_graph() {
        let mut dict = Dict::new();
        let g = MaterializedGraph::new(&mut dict, &[]);
        assert!(g.is_empty());
        assert_eq!(g.closure(), Vec::<[Id; 3]>::new());
        assert_eq!(g.len(), 0);
    }

    // ───────────────────────── MaterializedOwlGraph (incremental OWL 2 RL) ────────────────

    fn owl_id(dict: &mut Dict, frag: &str) -> Id {
        dict.intern_iri(&format!("http://www.w3.org/2002/07/owl#{frag}"))
    }

    /// Oracle: from-scratch OWL-RL closure of `base` as a set, via the batch API.
    fn owl_oracle(dict: &mut Dict, base: &FxHashSet<[Id; 3]>) -> FxHashSet<[Id; 3]> {
        let mut v: Vec<[Id; 3]> = base.iter().copied().collect();
        crate::materialize_owl_rl(dict, &mut v);
        v.into_iter().collect()
    }

    fn assert_owl_oracle(g: &MaterializedOwlGraph, dict: &mut Dict, base: &FxHashSet<[Id; 3]>) {
        let inc: FxHashSet<[Id; 3]> = g.closure().into_iter().collect();
        let full = owl_oracle(dict, base);
        assert_eq!(inc, full, "incremental OWL closure != from-scratch closure");
        assert_eq!(g.len(), inc.len(), "len() inconsistent with closure()");
    }

    #[test]
    fn owl_transitive_chain_insert_and_delete() {
        // ancestorOf transitive; parentOf ⊑ ancestorOf. Insert a parent chain a→b→c→d, expect
        // the full ancestor closure incrementally; delete the middle link, expect exactly the
        // closure of the remainder (counting + transitive layer handles the retraction).
        let mut dict = Dict::new();
        let v = vocab(&mut dict);
        let transitive = owl_id(&mut dict, "TransitiveProperty");
        let (anc, par) = (ex(&mut dict, "ancestorOf"), ex(&mut dict, "parentOf"));
        let (a, b, c, d) =
            (ex(&mut dict, "a"), ex(&mut dict, "b"), ex(&mut dict, "c"), ex(&mut dict, "d"));
        let base = vec![[anc, v.ty, transitive], [par, v.sp, anc]];
        let mut g = MaterializedOwlGraph::new(&mut dict, &base);
        assert_eq!(g.mode(), crate::OwlMode::CountingFixpoint);
        let mut set: FxHashSet<[Id; 3]> = base.iter().copied().collect();

        let chain = [[a, par, b], [b, par, c], [c, par, d]];
        g.insert(&mut dict, &chain);
        set.extend(chain);
        assert_eq!(g.full_rebuilds(), 0, "ABox insert must stay incremental");
        assert!(g.contains(&[a, anc, d]), "3-hop transitive entailment");
        assert!(g.contains(&[a, anc, c]));
        assert_owl_oracle(&g, &mut dict, &set);

        g.delete(&mut dict, &[[b, par, c]]);
        set.remove(&[b, par, c]);
        assert_eq!(g.full_rebuilds(), 0, "ABox delete must stay incremental");
        assert!(!g.contains(&[a, anc, d]), "the chain is severed");
        assert!(g.contains(&[a, anc, b]) && g.contains(&[c, anc, d]), "the stubs remain");
        assert_owl_oracle(&g, &mut dict, &set);
    }

    #[test]
    fn owl_inverse_orientation_and_multi_support() {
        // hasPart inverseOf partOf, hasPart domain Whole. (x hasPart y) ⊢ (y partOf x) and
        // (x type Whole); the inverse-derived edge must retract with its support.
        let mut dict = Dict::new();
        let v = vocab(&mut dict);
        let inv = owl_id(&mut dict, "inverseOf");
        let (hp, po, whole) =
            (ex(&mut dict, "hasPart"), ex(&mut dict, "partOf"), ex(&mut dict, "Whole"));
        let (x, y) = (ex(&mut dict, "x"), ex(&mut dict, "y"));
        let base = vec![[hp, inv, po], [hp, v.dom, whole]];
        let mut g = MaterializedOwlGraph::new(&mut dict, &base);
        assert_eq!(g.mode(), crate::OwlMode::CountingMono);
        let mut set: FxHashSet<[Id; 3]> = base.iter().copied().collect();

        g.insert(&mut dict, &[[x, hp, y]]);
        set.insert([x, hp, y]);
        assert!(g.contains(&[y, po, x]), "prp-inv orientation");
        assert!(g.contains(&[x, v.ty, whole]), "domain typing");
        assert_owl_oracle(&g, &mut dict, &set);

        // Assert the derived edge too: deleting the original must keep it (multi-support
        // via its own assertion), then deleting the assertion drops everything.
        g.insert(&mut dict, &[[y, po, x]]);
        set.insert([y, po, x]);
        g.delete(&mut dict, &[[x, hp, y]]);
        set.remove(&[x, hp, y]);
        assert!(g.contains(&[y, po, x]), "still asserted");
        assert!(g.contains(&[x, hp, y]), "re-derived from the inverse assertion");
        assert_owl_oracle(&g, &mut dict, &set);
        g.delete(&mut dict, &[[y, po, x]]);
        set.remove(&[y, po, x]);
        assert!(!g.contains(&[y, po, x]) && !g.contains(&[x, hp, y]));
        assert_owl_oracle(&g, &mut dict, &set);
        assert_eq!(g.full_rebuilds(), 0);
    }

    #[test]
    fn owl_domain_range_only_property_under_active_px() {
        // REGRESSION (mirrors rdfs::prop_expand_keeps_domain_range_only_properties): with the
        // property-orientation closure active (CountingMono via the inverse pair), a property
        // that has ONLY domain/range declarations is absent from the px map — emit_std must
        // fall through to the identity orientation (plain RDFS emission, matching the fixed
        // batch path), not drop its rdfs2/rdfs3 typing.
        let mut dict = Dict::new();
        let v = vocab(&mut dict);
        let inv = owl_id(&mut dict, "inverseOf");
        let (q, r, p) = (ex(&mut dict, "q"), ex(&mut dict, "r"), ex(&mut dict, "p"));
        let (c, d, a, b) =
            (ex(&mut dict, "C"), ex(&mut dict, "D"), ex(&mut dict, "a"), ex(&mut dict, "b"));
        let base = vec![[q, inv, r], [p, v.dom, c], [p, v.rng, d]];
        let mut g = MaterializedOwlGraph::new(&mut dict, &base);
        assert_eq!(g.mode(), crate::OwlMode::CountingMono);
        let mut set: FxHashSet<[Id; 3]> = base.iter().copied().collect();

        g.insert(&mut dict, &[[a, p, b]]);
        set.insert([a, p, b]);
        assert_eq!(g.full_rebuilds(), 0, "ABox insert must stay incremental");
        assert!(g.contains(&[a, v.ty, c]), "rdfs2 domain typing under active px");
        assert!(g.contains(&[b, v.ty, d]), "rdfs3 range typing under active px");
        assert_owl_oracle(&g, &mut dict, &set);

        g.delete(&mut dict, &[[a, p, b]]);
        set.remove(&[a, p, b]);
        assert!(!g.contains(&[a, v.ty, c]), "domain typing retracts with its support");
        assert_owl_oracle(&g, &mut dict, &set);
    }

    #[test]
    fn owl_tbox_mutation_rebuilds() {
        let mut dict = Dict::new();
        let v = vocab(&mut dict);
        let symmetric = owl_id(&mut dict, "SymmetricProperty");
        let near = ex(&mut dict, "near");
        let (x, y) = (ex(&mut dict, "x"), ex(&mut dict, "y"));
        let base = vec![[x, near, y]];
        let mut g = MaterializedOwlGraph::new(&mut dict, &base);
        let mut set: FxHashSet<[Id; 3]> = base.iter().copied().collect();
        assert!(!g.contains(&[y, near, x]));

        // Typing the property symmetric is a TBox mutation → rebuild, then the mirror holds.
        g.insert(&mut dict, &[[near, v.ty, symmetric]]);
        set.insert([near, v.ty, symmetric]);
        assert_eq!(g.full_rebuilds(), 1);
        assert!(g.contains(&[y, near, x]), "existing ABox re-swept under the new axiom");
        assert_owl_oracle(&g, &mut dict, &set);
    }

    #[test]
    fn owl_empty_graph() {
        let mut dict = Dict::new();
        let g = MaterializedOwlGraph::new(&mut dict, &[]);
        assert!(g.is_empty());
        assert_eq!(g.closure(), Vec::<[Id; 3]>::new());
    }
}
