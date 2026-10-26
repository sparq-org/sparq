//! Read-only **UFO/gUFO priors reader** — the design's "optional/last" gUFO prior (structure-aware
//! vectorisation §2 row "gUFO rigidity and roles", §9.5), wired as a **UFO-provable disjointness +
//! subsumption mask** for the [`DisjointnessOracle`].
//!
//! kern/ufo-priors (epic sq-0wo9e; deferred remainder noted in `taxonomy.rs`: "the
//! `gufo:Kind`/`gufo:Role` rigidity split is the design's optional/last prior … deferred"). Like
//! every other slice of the epic it is **opt-in** (the same `structure` cargo feature, off by
//! default) and changes **nothing** in the default `sparq-vectors` build or the core engine.
//!
//! # What this reads (READ-ONLY, from the graph as-is)
//!
//! [gUFO](https://nemo-ufes.github.io/gufo/) (the lightweight OWL implementation of the Unified
//! Foundational Ontology, Almeida et al.) annotates a domain ontology with second-order
//! **meta-types** and attaches domain classes to UFO's taxonomy of individuals. This module mines,
//! without writing anything back:
//!
//! - **meta-types** ([`MetaType`]): `C rdf:type gufo:Kind|SubKind|Role|Phase|Category|RoleMixin|
//!   PhaseMixin|Mixin` — and the **rigidity** ([`Rigidity`]) each meta-type carries by definition
//!   (a Kind is rigid: its instances instantiate it necessarily; a Role/Phase is anti-rigid:
//!   contingently held);
//! - **identity providers**: UFO's sortal principle — *every endurant instantiates exactly one
//!   Kind*, the class that supplies its identity criterion (Guizzardi 2005, ch. 4; the gUFO class
//!   descriptions). For each class the unique `gufo:Kind` among its `rdfs:subClassOf` ancestors
//!   (reflexive) is its identity provider, when exactly one exists;
//! - **ontological natures** ([`Nature`]): the gUFO taxonomy-of-individuals class a domain class
//!   specialises (`gufo:FunctionalComplex`/`Collection`/`Quantity` — the object natures —
//!   `gufo:Relator`, `gufo:Quality`, `gufo:IntrinsicMode`, `gufo:ExtrinsicMode`, `gufo:Event`,
//!   `gufo:Situation`), read from `rdfs:subClassOf` reachability;
//! - **mediation and inherence** (instance level): a subject of `gufo:mediates` is provably a
//!   relator and a subject of `gufo:inheresIn` is provably an aspect — these are exactly the
//!   entailments of gUFO's declared `rdfs:domain` for those properties (`gufo:Relator` /
//!   `gufo:Aspect`), so they hold even when the gUFO ontology axioms themselves are not loaded.
//!
//! # What it proves (and feeds to the oracle) — ANSWER-SAFE by construction
//!
//! The mask only ever asserts pairs UFO **proves**; absence of evidence asserts nothing (the honest
//! open-world default). Two class-level rules produce disjointness:
//!
//! 1. **Kind ⊥ Kind.** Kinds are UFO's unique identity suppliers: no individual instantiates two
//!    kinds, and a kind never specialises another kind (Guizzardi 2005 §4.2; gUFO "every concrete
//!    object … instance of exactly one kind"). Two *distinct* kinds are therefore provably
//!    disjoint — **unless** the graph (invalidly, per UFO) relates them by `rdfs:subClassOf`, in
//!    which case this module asserts nothing for that pair (fail closed: an ill-formed model never
//!    produces a mask that could drop a correct answer).
//! 2. **Identity-provider propagation.** If `X`'s identity provider is `K1`, `Y`'s is `K2`, and
//!    `K1 ⊥ K2` by rule 1, then `X ⊥ Y` (an instance shared by `X` and `Y` would instantiate two
//!    kinds). A class with **zero or several** kind ancestors gets **no** provider and joins no
//!    propagation (fail closed again).
//! 3. **Nature ⊥ nature.** UFO's taxonomy of individuals partitions concrete individuals by
//!    ontological nature (objects vs relators vs qualities vs modes vs events vs situations, and
//!    the object natures functional-complex/collection/quantity among themselves — the disjointness
//!    axioms gUFO declares over its individual taxonomy). Classes attached (via the `subClassOf`
//!    closure) to **different leaf natures** are provably disjoint. A class reaching **two**
//!    different leaf natures is contradictory modelling and is excluded entirely (fail closed).
//!
//! Deliberately **not** used for disjointness: rigidity (a Role's instances are its Kind's
//! instances — anti-rigid vs rigid is *not* disjointness), `gufo:Category`/mixins (non-sortals
//! classify across kinds by design), and instance-level mediation/inherence witnesses (an instance
//! of `C` mediating something does not prove all of `C` is a relator — witnesses are exposed as
//! instance facts, never lifted to class-level pairs).
//!
//! **Subsumption** ([`UfoPriors::proven_subsumptions`]) records the UFO-entailed super-classes a
//! class provably has beyond the asserted graph: every endurant-meta-typed class is subsumed by
//! `gufo:Endurant`, and a class of nature N is subsumed by N's fixed gUFO upper chain (e.g. a
//! relator-natured class ⊑ `gufo:ExtrinsicAspect` ⊑ `gufo:Aspect` ⊑ `gufo:Endurant`). Pairs are
//! emitted only for gUFO terms actually present in the graph's dictionary (an id-level module
//! cannot mint new terms — and must not: read-only).
//!
//! Feed the disjointness into the serve-time mask with [`UfoPriors::augment_oracle`]; the oracle's
//! [`mask_candidates`](crate::taxonomy::DisjointnessOracle::mask_candidates) then drops only
//! candidates whose type is **provably** incompatible — never a merely-dissimilar one (design §6.A
//! "verify-soundness / answer-safety"). Whether the mask *helps* any downstream metric is empirical
//! and **not claimed here** — the eval harness's `gufo_prior` ablation axis (default **off**)
//! measures it; see `EvalConfig::gufo_prior` in `crate::eval` (the `kge` feature — not linked
//! here so a `structure`-only doc build carries no broken intra-doc link).

use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Id, TermParts};
use sparq_core::Graph;

use crate::taxonomy::DisjointnessOracle;

/// The canonical gUFO namespace (<https://nemo-ufes.github.io/gufo/>).
pub const GUFO_NS: &str = "http://purl.org/nemo/gufo#";

/// `rdf:type`.
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
/// `rdfs:subClassOf`.
const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";

/// A gUFO **endurant-type meta-type** — the second-order class a domain class is `rdf:type`-d
/// with. The set mirrors gUFO's taxonomy of endurant types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetaType {
    /// `gufo:Kind` — rigid sortal, supplies the identity criterion. UFO: every endurant
    /// instantiates exactly one kind.
    Kind,
    /// `gufo:SubKind` — rigid sortal specialising a kind (inherits its identity).
    SubKind,
    /// `gufo:Role` — anti-rigid sortal held contingently in a relational context.
    Role,
    /// `gufo:Phase` — anti-rigid sortal held contingently by an intrinsic condition.
    Phase,
    /// `gufo:Category` — rigid NON-sortal: classifies individuals of several kinds.
    Category,
    /// `gufo:RoleMixin` — anti-rigid non-sortal.
    RoleMixin,
    /// `gufo:PhaseMixin` — anti-rigid non-sortal.
    PhaseMixin,
    /// `gufo:Mixin` — semi-rigid non-sortal.
    Mixin,
}

impl MetaType {
    /// The gUFO local name.
    fn local(self) -> &'static str {
        match self {
            MetaType::Kind => "Kind",
            MetaType::SubKind => "SubKind",
            MetaType::Role => "Role",
            MetaType::Phase => "Phase",
            MetaType::Category => "Category",
            MetaType::RoleMixin => "RoleMixin",
            MetaType::PhaseMixin => "PhaseMixin",
            MetaType::Mixin => "Mixin",
        }
    }

    /// All meta-types, in a fixed order.
    const ALL: [MetaType; 8] = [
        MetaType::Kind,
        MetaType::SubKind,
        MetaType::Role,
        MetaType::Phase,
        MetaType::Category,
        MetaType::RoleMixin,
        MetaType::PhaseMixin,
        MetaType::Mixin,
    ];

    /// The **rigidity** this meta-type carries *by definition* in UFO (Guizzardi 2005 §4.1;
    /// the gUFO class descriptions). This is definitional, not mined — a `gufo:Role` IS anti-rigid.
    pub fn rigidity(self) -> Rigidity {
        match self {
            MetaType::Kind | MetaType::SubKind | MetaType::Category => Rigidity::Rigid,
            MetaType::Role | MetaType::Phase | MetaType::RoleMixin | MetaType::PhaseMixin => {
                Rigidity::AntiRigid
            }
            MetaType::Mixin => Rigidity::SemiRigid,
        }
    }

    /// Is this a **sortal** (carries/inherits an identity principle: Kind/SubKind/Role/Phase)?
    pub fn is_sortal(self) -> bool {
        matches!(
            self,
            MetaType::Kind | MetaType::SubKind | MetaType::Role | MetaType::Phase
        )
    }
}

/// UFO rigidity of a meta-type — informational for consumers (e.g. a future split type
/// sub-vector, design §2). **No disjointness is derived from rigidity**: a Student (anti-rigid)
/// is still a Person (rigid).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rigidity {
    /// Instantiated necessarily (Kind, SubKind, Category).
    Rigid,
    /// Instantiated contingently — can be acquired and lost (Role, Phase, RoleMixin, PhaseMixin).
    AntiRigid,
    /// Rigid for some instances, anti-rigid for others (Mixin).
    SemiRigid,
}

/// A **leaf ontological nature** in gUFO's taxonomy of individuals. UFO partitions concrete
/// individuals by nature, so distinct leaf natures are **provably disjoint** (the disjointness
/// gUFO axiomatises over `gufo:Endurant`/`gufo:Event`/`gufo:Situation`, `gufo:Object`/`gufo:Aspect`
/// and their sub-partitions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nature {
    /// `gufo:FunctionalComplex` (an object nature).
    FunctionalComplex,
    /// `gufo:Collection` (an object nature).
    Collection,
    /// `gufo:Quantity` (an object nature).
    Quantity,
    /// `gufo:Relator` (an extrinsic aspect — the truth-maker of a material relation).
    Relator,
    /// `gufo:Quality` (an intrinsic aspect with a measurable value).
    Quality,
    /// `gufo:IntrinsicMode` (an intrinsic aspect without a value structure).
    IntrinsicMode,
    /// `gufo:ExtrinsicMode` (an externally-dependent mode).
    ExtrinsicMode,
    /// `gufo:Event` (a perdurant).
    Event,
    /// `gufo:Situation`.
    Situation,
}

impl Nature {
    /// The gUFO local name.
    fn local(self) -> &'static str {
        match self {
            Nature::FunctionalComplex => "FunctionalComplex",
            Nature::Collection => "Collection",
            Nature::Quantity => "Quantity",
            Nature::Relator => "Relator",
            Nature::Quality => "Quality",
            Nature::IntrinsicMode => "IntrinsicMode",
            Nature::ExtrinsicMode => "ExtrinsicMode",
            Nature::Event => "Event",
            Nature::Situation => "Situation",
        }
    }

    /// All leaf natures, fixed order.
    const ALL: [Nature; 9] = [
        Nature::FunctionalComplex,
        Nature::Collection,
        Nature::Quantity,
        Nature::Relator,
        Nature::Quality,
        Nature::IntrinsicMode,
        Nature::ExtrinsicMode,
        Nature::Event,
        Nature::Situation,
    ];

    /// The **fixed gUFO upper chain** above this leaf nature (local names, nearest first) — the
    /// taxonomy-of-individuals edges the gUFO specification declares. Used for the proven
    /// subsumptions: a class of this nature is subsumed by every class in the chain.
    fn upper_chain(self) -> &'static [&'static str] {
        match self {
            Nature::FunctionalComplex | Nature::Collection | Nature::Quantity => {
                &["Object", "Endurant"]
            }
            Nature::Relator | Nature::ExtrinsicMode => &["ExtrinsicAspect", "Aspect", "Endurant"],
            Nature::Quality | Nature::IntrinsicMode => &["IntrinsicAspect", "Aspect", "Endurant"],
            // Events and situations sit outside the endurant subtree.
            Nature::Event | Nature::Situation => &[],
        }
    }
}

/// The mined UFO/gUFO priors of one graph: meta-types, identity providers, natures, the
/// instance-level mediation/inherence witnesses, and the **UFO-provable** disjointness +
/// subsumption pairs derived from them. Construction is a read-only scan; see the module docs for
/// the exact rules and their soundness argument.
#[derive(Clone, Debug, Default)]
pub struct UfoPriors {
    /// Class id → its gUFO meta-type (from `rdf:type`).
    meta: FxHashMap<Id, MetaType>,
    /// Class id → its unique identity-providing `gufo:Kind` (reflexive: a kind provides itself).
    /// A class with zero or several kind ancestors is absent (fail closed).
    identity_provider: FxHashMap<Id, Id>,
    /// Class id → its single leaf [`Nature`]. A class reaching two different leaf natures is
    /// absent (contradictory modelling, fail closed) and counted in `nature_conflicts`.
    nature: FxHashMap<Id, Nature>,
    /// Number of classes excluded because they reached contradictory leaf natures.
    nature_conflicts: usize,
    /// Symmetric set of UFO-proven disjoint class pairs, stored as `(min, max)`.
    disjoint: FxHashSet<(Id, Id)>,
    /// UFO-proven `(sub, super)` subsumptions beyond the asserted `subClassOf` closure
    /// (only over gUFO terms present in the dictionary).
    subsumptions: Vec<(Id, Id)>,
    /// Instances proven relators by a `gufo:mediates` assertion (sorted, deduped).
    relator_instances: Vec<Id>,
    /// Instances proven aspects by a `gufo:inheresIn` assertion (sorted, deduped).
    aspect_instances: Vec<Id>,
}

impl UfoPriors {
    /// Mine the priors under the canonical [`GUFO_NS`] namespace. The graph should ideally be
    /// **closed** ([`crate::structure::materialise_closure`]) so `subClassOf`/`rdf:type`
    /// entailments are materialised, but every rule is sound on the asserted graph too — a
    /// less-closed graph simply proves less. Safe on a graph with no gUFO annotations at all:
    /// the result is empty and nothing is ever masked.
    pub fn mine(graph: &Graph) -> UfoPriors {
        Self::mine_with_namespace(graph, GUFO_NS)
    }

    /// [`mine`](Self::mine) with an explicit gUFO namespace — for datasets that mint the gUFO
    /// meta-vocabulary under a non-canonical IRI (e.g. the synthetic eval slice's
    /// `http://ex/gufo#`). The namespace choice is the **caller's explicit declaration**, never a
    /// heuristic guess (no silent fallback).
    pub fn mine_with_namespace(graph: &Graph, ns: &str) -> UfoPriors {
        let mut out = UfoPriors::default();

        // ---- resolve the vocabulary (absent terms simply match nothing) ----
        let rdf_type = graph.id_of(&named(RDF_TYPE));
        let subclass = graph.id_of(&named(RDFS_SUBCLASS_OF));
        let mediates = graph.id_of(&named(&format!("{ns}mediates")));
        let inheres_in = graph.id_of(&named(&format!("{ns}inheresIn")));
        let meta_ids: FxHashMap<Id, MetaType> = MetaType::ALL
            .iter()
            .filter_map(|&m| {
                graph
                    .id_of(&named(&format!("{ns}{}", m.local())))
                    .map(|id| (id, m))
            })
            .collect();
        let nature_ids: FxHashMap<Id, Nature> = Nature::ALL
            .iter()
            .filter_map(|&n| {
                graph
                    .id_of(&named(&format!("{ns}{}", n.local())))
                    .map(|id| (id, n))
            })
            .collect();

        // ---- one scan: meta-typing, subClassOf adjacency, mediation/inherence witnesses ----
        let mut supers: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        let mut classes: FxHashSet<Id> = FxHashSet::default();
        let mut relators: FxHashSet<Id> = FxHashSet::default();
        let mut aspects: FxHashSet<Id> = FxHashSet::default();
        for [s, p, o] in graph.iter_ids() {
            if Some(p) == rdf_type {
                if let Some(&m) = meta_ids.get(&o) {
                    if is_node(graph, s) {
                        out.meta.insert(s, m);
                        classes.insert(s);
                    }
                }
            } else if Some(p) == subclass {
                // Skip reflexive edges (no signal) and non-node ends, exactly as taxonomy.rs does.
                if s != o && is_node(graph, s) && is_node(graph, o) {
                    supers.entry(s).or_default().push(o);
                    classes.insert(s);
                    classes.insert(o);
                }
            } else if Some(p) == mediates {
                // gUFO declares `gufo:mediates rdfs:domain gufo:Relator`: the SUBJECT of a
                // mediation is provably a relator INSTANCE (an entailment, not a guess).
                if is_node(graph, s) {
                    relators.insert(s);
                }
            } else if Some(p) == inheres_in {
                // `gufo:inheresIn rdfs:domain gufo:Aspect`: the subject is provably an aspect.
                if is_node(graph, s) {
                    aspects.insert(s);
                }
            }
        }
        out.relator_instances = sorted(relators);
        out.aspect_instances = sorted(aspects);

        // Deterministic class order (independent of hash iteration order).
        let class_list = sorted(classes);

        // ---- reflexive-transitive ancestors per class (cycle-safe BFS over `supers`) ----
        let ancestors: FxHashMap<Id, FxHashSet<Id>> = class_list
            .iter()
            .map(|&c| (c, reachable_up(c, &supers)))
            .collect();

        // ---- identity providers: the unique gufo:Kind in a class's reflexive ancestry ----
        let kinds: Vec<Id> = class_list
            .iter()
            .copied()
            .filter(|c| out.meta.get(c) == Some(&MetaType::Kind))
            .collect();
        let kind_set: FxHashSet<Id> = kinds.iter().copied().collect();
        for &c in &class_list {
            let anc = &ancestors[&c];
            let mut providers = anc.iter().copied().filter(|k| kind_set.contains(k));
            if let (Some(k), None) = (providers.next(), providers.next()) {
                out.identity_provider.insert(c, k);
            }
            // zero or ≥2 kind ancestors → NO provider recorded (fail closed).
        }

        // ---- natures: reachability to a leaf nature class; contradictory → excluded ----
        for &c in &class_list {
            let mut found: Option<Nature> = None;
            let mut conflict = false;
            for anc in &ancestors[&c] {
                if let Some(&n) = nature_ids.get(anc) {
                    match found {
                        None => found = Some(n),
                        Some(prev) if prev != n => conflict = true,
                        Some(_) => {}
                    }
                }
            }
            if conflict {
                out.nature_conflicts += 1;
            } else if let Some(n) = found {
                out.nature.insert(c, n);
            }
        }

        // ---- rule 1: Kind ⊥ Kind (unless subClassOf-related — ill-formed per UFO, fail closed) --
        let mut base_kind_pairs: Vec<(Id, Id)> = Vec::new();
        for (i, &k1) in kinds.iter().enumerate() {
            for &k2 in &kinds[i + 1..] {
                if !ancestors[&k1].contains(&k2) && !ancestors[&k2].contains(&k1) {
                    base_kind_pairs.push((k1, k2));
                    out.add_disjoint(k1, k2);
                }
            }
        }

        // ---- rule 2: identity-provider propagation ----
        // Group classes by their (unique) provider, then cross the groups of each base pair.
        let mut group: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        for (&c, &k) in &out.identity_provider {
            group.entry(k).or_default().push(c);
        }
        for &(k1, k2) in &base_kind_pairs {
            let (Some(g1), Some(g2)) = (group.get(&k1), group.get(&k2)) else {
                continue;
            };
            for &x in g1 {
                for &y in g2 {
                    // Extra guard (defence in depth): never assert a pair the subClassOf
                    // closure relates — a mask must not be able to drop a class's own super.
                    if !ancestors[&x].contains(&y) && !ancestors[&y].contains(&x) {
                        out.add_disjoint(x, y);
                    }
                }
            }
        }

        // ---- rule 3: distinct leaf natures are disjoint ----
        let mut natured: Vec<(Id, Nature)> = out.nature.iter().map(|(&c, &n)| (c, n)).collect();
        natured.sort_unstable_by_key(|&(c, _)| c);
        for (i, &(x, nx)) in natured.iter().enumerate() {
            for &(y, ny) in &natured[i + 1..] {
                if nx != ny && !ancestors[&x].contains(&y) && !ancestors[&y].contains(&x) {
                    out.add_disjoint(x, y);
                }
            }
        }

        // ---- proven subsumptions (only over gUFO terms actually in the dictionary) ----
        let endurant = graph.id_of(&named(&format!("{ns}Endurant")));
        let mut subs: FxHashSet<(Id, Id)> = FxHashSet::default();
        for &c in &class_list {
            let anc = &ancestors[&c];
            // Every endurant-meta-typed class is subsumed by gufo:Endurant (all its instances
            // are endurants — the gUFO endurant-type definition).
            if out.meta.contains_key(&c) {
                if let Some(e) = endurant {
                    if c != e && !anc.contains(&e) {
                        subs.insert((c, e));
                    }
                }
            }
            // A class of nature N is subsumed by N's fixed gUFO upper chain.
            if let Some(&n) = out.nature.get(&c) {
                for upper in n.upper_chain() {
                    if let Some(u) = graph.id_of(&named(&format!("{ns}{upper}"))) {
                        if c != u && !anc.contains(&u) {
                            subs.insert((c, u));
                        }
                    }
                }
            }
        }
        out.subsumptions = {
            let mut v: Vec<(Id, Id)> = subs.into_iter().collect();
            v.sort_unstable();
            v
        };

        out
    }

    fn add_disjoint(&mut self, a: Id, b: Id) {
        if a != b {
            self.disjoint.insert((a.min(b), a.max(b)));
        }
    }

    /// The mined meta-type of a class, if any.
    pub fn meta_of(&self, class: Id) -> Option<MetaType> {
        self.meta.get(&class).copied()
    }

    /// The (definitional) rigidity of a class's meta-type, if the class carries one.
    pub fn rigidity_of(&self, class: Id) -> Option<Rigidity> {
        self.meta_of(class).map(MetaType::rigidity)
    }

    /// The unique identity-providing `gufo:Kind` of a class (reflexive), if exactly one exists.
    pub fn identity_provider_of(&self, class: Id) -> Option<Id> {
        self.identity_provider.get(&class).copied()
    }

    /// The single leaf [`Nature`] of a class, if unambiguous.
    pub fn nature_of(&self, class: Id) -> Option<Nature> {
        self.nature.get(&class).copied()
    }

    /// Number of classes excluded because they reached contradictory leaf natures (fail closed).
    pub fn nature_conflicts(&self) -> usize {
        self.nature_conflicts
    }

    /// Does UFO **prove** classes `a` and `b` disjoint (per the module rules)? Symmetric;
    /// `false` for `a == b` and for any unproven pair.
    pub fn proves_disjoint(&self, a: Id, b: Id) -> bool {
        a != b && self.disjoint.contains(&(a.min(b), a.max(b)))
    }

    /// The UFO-proven disjoint (unordered) class pairs, deterministic order.
    pub fn provable_disjoint_pairs(&self) -> Vec<(Id, Id)> {
        let mut v: Vec<(Id, Id)> = self.disjoint.iter().copied().collect();
        v.sort_unstable();
        v
    }

    /// Number of distinct proven disjoint pairs.
    pub fn disjoint_pair_count(&self) -> usize {
        self.disjoint.len()
    }

    /// The UFO-proven `(sub, super)` subsumptions **beyond** the asserted `subClassOf` closure,
    /// over gUFO terms present in the graph. Deterministic order.
    pub fn proven_subsumptions(&self) -> &[(Id, Id)] {
        &self.subsumptions
    }

    /// Instances proven **relators** by an asserted `gufo:mediates` (sorted). Instance-level
    /// facts — deliberately NOT lifted to class-level disjointness (see the module docs).
    pub fn relator_instances(&self) -> &[Id] {
        &self.relator_instances
    }

    /// Instances proven **aspects** by an asserted `gufo:inheresIn` (sorted).
    pub fn aspect_instances(&self) -> &[Id] {
        &self.aspect_instances
    }

    /// No gUFO structure found at all?
    pub fn is_empty(&self) -> bool {
        self.meta.is_empty()
            && self.disjoint.is_empty()
            && self.subsumptions.is_empty()
            && self.relator_instances.is_empty()
            && self.aspect_instances.is_empty()
    }

    /// Feed the UFO-proven disjointness into a [`DisjointnessOracle`], making its serve-time
    /// [`mask_candidates`](DisjointnessOracle::mask_candidates) UFO-aware while staying
    /// **answer-safe**: every absorbed pair is proven (module rules), so the mask still removes
    /// only provably-wrong candidates.
    pub fn augment_oracle(&self, oracle: &mut DisjointnessOracle) {
        oracle.absorb_proven_pairs(self.disjoint.iter().copied());
    }
}

// ============================================================================================
// helpers
// ============================================================================================

/// A named-node term for `iri`.
fn named(iri: &str) -> oxrdf::Term {
    oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(iri))
}

/// Is the id a named or blank node (the only things that can be classes/individuals here)?
fn is_node(graph: &Graph, id: Id) -> bool {
    matches!(
        graph.dict().term_parts(id),
        TermParts::Iri { .. } | TermParts::Blank(_)
    )
}

/// Sorted Vec from a set (deterministic order independent of hash iteration).
fn sorted(set: FxHashSet<Id>) -> Vec<Id> {
    let mut v: Vec<Id> = set.into_iter().collect();
    v.sort_unstable();
    v
}

/// Reflexive-transitive `subClassOf` ancestors of `start` (cycle-safe BFS).
fn reachable_up(start: Id, supers: &FxHashMap<Id, Vec<Id>>) -> FxHashSet<Id> {
    let mut seen: FxHashSet<Id> = FxHashSet::default();
    let mut stack = vec![start];
    seen.insert(start);
    while let Some(c) = stack.pop() {
        if let Some(ps) = supers.get(&c) {
            for &p in ps {
                if seen.insert(p) {
                    stack.push(p);
                }
            }
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::close_for_vectorise;
    use sparq_reason::Profile;

    // A small gUFO-annotated schema exercising every mined facet: kinds (with natures), roles,
    // phases, a category over two kinds, a relator kind with mediation, inherence, nature-only
    // classes, a contradictory-nature class, subClassOf-related kinds (ill-formed, fail closed),
    // and a two-kind class (ambiguous identity, fail closed).
    const TTL: &str = r#"
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix gufo: <http://purl.org/nemo/gufo#> .
@prefix ex:   <http://ex/> .

# Kinds attached to their ontological natures.
ex:Person       a gufo:Kind ; rdfs:subClassOf gufo:FunctionalComplex .
ex:Organization a gufo:Kind ; rdfs:subClassOf gufo:FunctionalComplex .
ex:Marriage     a gufo:Kind ; rdfs:subClassOf gufo:Relator .

# Anti-rigid sortals under Person.
ex:Student a gufo:Role  ; rdfs:subClassOf ex:Person .
ex:Child   a gufo:Phase ; rdfs:subClassOf ex:Person .

# A rigid non-sortal (category) over BOTH kinds — must never become disjoint from either.
ex:Agent a gufo:Category .
ex:Person rdfs:subClassOf ex:Agent .
ex:Organization rdfs:subClassOf ex:Agent .

# Nature-only classes (no meta-type): provably disjoint by nature.
ex:PhysicalThing rdfs:subClassOf gufo:FunctionalComplex .
ex:Bond          rdfs:subClassOf gufo:Relator .

# Contradictory natures → excluded entirely (fail closed).
ex:Weird rdfs:subClassOf gufo:Relator , gufo:Quality .

# Ill-formed: a kind specialising a kind → NOT asserted disjoint (fail closed).
ex:K1 a gufo:Kind .
ex:K2 a gufo:Kind ; rdfs:subClassOf ex:K1 .

# Ambiguous identity: two kind ancestors → no provider, no propagation.
ex:Chimera rdfs:subClassOf ex:Person , ex:Organization .

# Instance level: mediation proves a relator, inherence proves an aspect.
ex:m1 a ex:Marriage ; gufo:mediates ex:alice , ex:bob .
ex:alice a ex:Person . ex:bob a ex:Person .
ex:h1 gufo:inheresIn ex:alice .

# Mentions that give the gUFO uppers dictionary ids (for the subsumption product).
ex:someEndurant a gufo:Endurant .
ex:someAspect   a gufo:Aspect .
"#;

    fn graph() -> Graph {
        close_for_vectorise(TTL, "turtle", Profile::Rdfs)
            .unwrap()
            .graph
    }

    fn id(g: &Graph, iri: &str) -> Id {
        g.id_of(&named(iri)).unwrap()
    }
    fn ex(g: &Graph, local: &str) -> Id {
        id(g, &format!("http://ex/{local}"))
    }
    fn gufo(g: &Graph, local: &str) -> Id {
        id(g, &format!("{GUFO_NS}{local}"))
    }

    #[test]
    fn meta_types_rigidity_and_identity_providers_are_mined() {
        let g = graph();
        let p = UfoPriors::mine(&g);
        assert_eq!(p.meta_of(ex(&g, "Person")), Some(MetaType::Kind));
        assert_eq!(p.meta_of(ex(&g, "Student")), Some(MetaType::Role));
        assert_eq!(p.meta_of(ex(&g, "Child")), Some(MetaType::Phase));
        assert_eq!(p.meta_of(ex(&g, "Agent")), Some(MetaType::Category));
        assert_eq!(p.rigidity_of(ex(&g, "Person")), Some(Rigidity::Rigid));
        assert_eq!(p.rigidity_of(ex(&g, "Student")), Some(Rigidity::AntiRigid));
        assert_eq!(p.rigidity_of(ex(&g, "Agent")), Some(Rigidity::Rigid));
        // Identity providers: a kind provides itself; sortals inherit the unique kind ancestor.
        assert_eq!(
            p.identity_provider_of(ex(&g, "Person")),
            Some(ex(&g, "Person"))
        );
        assert_eq!(
            p.identity_provider_of(ex(&g, "Student")),
            Some(ex(&g, "Person"))
        );
        // Two kind ancestors → NO provider (fail closed).
        assert_eq!(p.identity_provider_of(ex(&g, "Chimera")), None);
        // A category classifies across kinds → no unique provider either.
        assert_eq!(p.identity_provider_of(ex(&g, "Agent")), None);
    }

    #[test]
    fn kind_disjointness_and_provider_propagation_are_proven() {
        let g = graph();
        let p = UfoPriors::mine(&g);
        // Rule 1: distinct, unrelated kinds are provably disjoint.
        assert!(p.proves_disjoint(ex(&g, "Person"), ex(&g, "Organization")));
        assert!(p.proves_disjoint(ex(&g, "Person"), ex(&g, "Marriage")));
        // Rule 2: propagation down identity providers.
        assert!(p.proves_disjoint(ex(&g, "Student"), ex(&g, "Organization")));
        assert!(p.proves_disjoint(ex(&g, "Child"), ex(&g, "Marriage")));
        // Symmetric.
        assert!(p.proves_disjoint(ex(&g, "Organization"), ex(&g, "Student")));
    }

    #[test]
    fn no_unprovable_pair_is_asserted_answer_safety() {
        let g = graph();
        let p = UfoPriors::mine(&g);
        // A sortal is never disjoint from its own kind or the category above it.
        assert!(!p.proves_disjoint(ex(&g, "Student"), ex(&g, "Person")));
        assert!(!p.proves_disjoint(ex(&g, "Person"), ex(&g, "Agent")));
        assert!(!p.proves_disjoint(ex(&g, "Organization"), ex(&g, "Agent")));
        // Role vs phase under the SAME kind: a Child can be a Student — never disjoint.
        assert!(!p.proves_disjoint(ex(&g, "Student"), ex(&g, "Child")));
        // Self.
        assert!(!p.proves_disjoint(ex(&g, "Person"), ex(&g, "Person")));
        // Ill-formed subClassOf-related kinds: nothing asserted (fail closed).
        assert!(!p.proves_disjoint(ex(&g, "K1"), ex(&g, "K2")));
        // Ambiguous-identity class joins no propagation.
        assert!(!p.proves_disjoint(ex(&g, "Chimera"), ex(&g, "Person")));
        assert!(!p.proves_disjoint(ex(&g, "Chimera"), ex(&g, "Organization")));
    }

    #[test]
    fn natures_are_mined_and_distinct_natures_are_disjoint() {
        let g = graph();
        let p = UfoPriors::mine(&g);
        assert_eq!(
            p.nature_of(ex(&g, "Person")),
            Some(Nature::FunctionalComplex)
        );
        assert_eq!(p.nature_of(ex(&g, "Marriage")), Some(Nature::Relator));
        // Nature-only classes (no meta-type) are provably disjoint by nature alone.
        assert!(p.proves_disjoint(ex(&g, "PhysicalThing"), ex(&g, "Bond")));
        // Contradictory natures → excluded, and nothing asserted about the class (fail closed).
        assert_eq!(p.nature_of(ex(&g, "Weird")), None);
        assert!(p.nature_conflicts() >= 1);
        assert!(!p.proves_disjoint(ex(&g, "Weird"), ex(&g, "PhysicalThing")));
        assert!(!p.proves_disjoint(ex(&g, "Weird"), ex(&g, "Bond")));
    }

    #[test]
    fn mediation_and_inherence_witness_instances() {
        let g = graph();
        let p = UfoPriors::mine(&g);
        assert!(
            p.relator_instances().contains(&ex(&g, "m1")),
            "m1 mediates ⇒ relator"
        );
        assert!(
            p.aspect_instances().contains(&ex(&g, "h1")),
            "h1 inheresIn ⇒ aspect"
        );
        // Witnesses are NOT lifted to class-level disjointness: Marriage vs Person disjointness
        // comes from the kind/nature rules, not from m1; and alice (an instance) is in no pair.
        assert!(!p.proves_disjoint(ex(&g, "alice"), ex(&g, "m1")));
    }

    #[test]
    fn proven_subsumptions_cover_endurant_and_upper_chains() {
        let g = graph();
        let p = UfoPriors::mine(&g);
        let subs: FxHashSet<(Id, Id)> = p.proven_subsumptions().iter().copied().collect();
        // Endurant-meta-typed classes ⊑ gufo:Endurant (the id exists via ex:someEndurant).
        assert!(subs.contains(&(ex(&g, "Person"), gufo(&g, "Endurant"))));
        assert!(subs.contains(&(ex(&g, "Student"), gufo(&g, "Endurant"))));
        // Nature upper chain: a relator-natured class ⊑ gufo:Aspect (ExtrinsicAspect has no id in
        // this graph, so only the present terms are emitted — read-only, no term minting).
        assert!(subs.contains(&(ex(&g, "Marriage"), gufo(&g, "Aspect"))));
        assert!(subs.contains(&(ex(&g, "Bond"), gufo(&g, "Aspect"))));
        // Never reflexive, never already-asserted.
        for &(sub, sup) in p.proven_subsumptions() {
            assert_ne!(sub, sup);
        }
    }

    #[test]
    fn oracle_augmentation_masks_only_ufo_proven_pairs() {
        let g = graph();
        // The fixture carries NO owl:disjointWith — the plain oracle proves nothing.
        let mut oracle = DisjointnessOracle::mine(&g);
        assert_eq!(oracle.pair_count(), 0);
        let p = UfoPriors::mine(&g);
        p.augment_oracle(&mut oracle);
        assert!(oracle.pair_count() > 0);
        assert!(oracle.is_disjoint(ex(&g, "Person"), ex(&g, "Organization")));
        assert!(!oracle.is_disjoint(ex(&g, "Student"), ex(&g, "Person")));
        // The serve-time mask drops exactly the proven-wrong candidate and keeps the rest.
        let person = ex(&g, "Person");
        let org = ex(&g, "Organization");
        let agent = ex(&g, "Agent");
        let kept = oracle.mask_candidates(&[person], &[(org, 1u32), (agent, 2), (person, 3)]);
        let kept: Vec<u32> = kept.iter().map(|&(_, v)| v).collect();
        assert_eq!(
            kept,
            vec![2, 3],
            "org dropped (proven), agent + person kept"
        );
    }

    #[test]
    fn gufo_free_graph_yields_empty_priors() {
        let c = close_for_vectorise(
            "<http://ex/a> <http://ex/p> <http://ex/b> .",
            "ntriples",
            Profile::Rdfs,
        )
        .unwrap();
        let p = UfoPriors::mine(&c.graph);
        assert!(p.is_empty());
        assert_eq!(p.disjoint_pair_count(), 0);
        assert!(p.proven_subsumptions().is_empty());
    }

    #[test]
    fn mining_is_deterministic() {
        let g = graph();
        let a = UfoPriors::mine(&g);
        let b = UfoPriors::mine(&g);
        assert_eq!(a.provable_disjoint_pairs(), b.provable_disjoint_pairs());
        assert_eq!(a.proven_subsumptions(), b.proven_subsumptions());
        assert_eq!(a.relator_instances(), b.relator_instances());
    }
}
