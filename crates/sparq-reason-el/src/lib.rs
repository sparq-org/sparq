#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // [OPUS-4.8] sq-evb1: crate has zero `unsafe`.

//! See the crate README (rendered above) for the capability overview. The load-bearing
//! entry points are [`classify_graph`] (emit the complete subsumption lattice into a
//! `(Dict, triples)` pair) and [`Classifier`] (the typed [`ClassHierarchy`] API). The
//! algorithm lives in private modules: `normal` (Baader–Brandt–Lutz normalization),
//! `classify` (CR1–CR6 saturation), `extract` (RDF → EL+⊥ axioms), under the
//! `cdomain` feature `cdomain` (CR7–CR9 concrete-domain facet satisfiability on the
//! shared `sparq_substrate::numeric` value tower), and under the `abox` feature `abox`
//! (ABox assertion internalization as safe nominals + the realisation readoff — the
//! `realize` / `realize_graph` entry points). Under the `par` feature (Phase E4) the
//! `Classifier::classify_par` / `classify_graph_par` entries run the same saturation as
//! deterministic bulk-synchronous parallel rounds with an identical derived closure at
//! every thread count (all the feature-gated names here are code spans, not intra-doc
//! links, so this always-compiled note stays clean under the no-`--all-features`
//! rustdoc check).

use rustc_hash::FxHashMap;
use sparq_core::dict::{Dict, Id};

// [OPUS-4.8] sq-pbz04.2.5: ABox internalization + realisation + whole-ontology consistency —
// only under `abox`, so the default TBox classifier carries zero assertion-reasoning code.
#[cfg(feature = "abox")]
mod abox;
// [FABLE-5] sq-pbz04.2.2: CR7–CR9 (concrete domains) — only under `cdomain`, so the
// default build carries zero concrete-domain code and no sparq-substrate dependency.
#[cfg(feature = "cdomain")]
mod cdomain;
mod classify;
mod extract;
#[cfg(feature = "hasse")]
mod hasse;
// [SONNET-4.6] sq-clsv6: incremental classification under TBox edits — only under
// `incremental`, so the stateless build carries zero edit-tracking state or code.
#[cfg(feature = "incremental")]
mod incremental;
mod normal;
#[cfg(feature = "rbox")]
mod rbox;

#[cfg(feature = "abox")]
pub use abox::{realize, realize_graph, AboxReport, Realization};
#[cfg(feature = "hasse")]
pub use hasse::{classify_hasse_graph, DirectHierarchy};
// [SONNET-4.6] sq-clsv6 (Phase E5): the stateful edit-tracking classifier. Read
// `IncrementalClassifier`'s docs for what "incremental" does and does NOT cover here (additions
// are folded in; a retraction takes an honest full-re-classification fallback).
#[cfg(feature = "incremental")]
pub use incremental::{EditDisposition, FullReason, IncrementalClassifier, IncrementalReport};
// [SONNET-4.6] sq-q0o82 (E4 follow-up): the compute/apply attribution the parallel-apply
// refinement decision is measured on — see `classify_graph_par_stats`.
#[cfg(feature = "par")]
pub use classify::ParPhaseStats;

const RDFS_SUB_CLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const OWL_NOTHING: &str = "http://www.w3.org/2002/07/owl#Nothing";
/// [SONNET-4.6] sq-pbz04.2.7 (`rbox`): the `rdfs:subPropertyOf` predicate IRI — interned by
/// `classify_graph` when the `rbox` feature is on so the role-lattice readoff can emit
/// non-reflexive inclusion pairs as triples.
#[cfg(feature = "rbox")]
const RDFS_SUB_PROPERTY_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subPropertyOf";

/// A tally of what the extractor read and what it could not (so an honest
/// "n axioms outside the EL fragment were ignored" can be surfaced).
///
/// The classifier is SOUND but only COMPLETE for the fragment it recognises. By default that is
/// EL+⊥-minus-RBox (the E1 MVP) **plus safe nominals** (sq-pbz04.2.1: singleton `owl:oneOf` and
/// object-valued `owl:hasValue` — the `{a}` / `∃r.{a}` concepts, completion rule **CR6**) **and
/// self-restrictions** (sq-pbz04.2.6: `owl:hasSelf "true"^^xsd:boolean` — the `∃r.Self` local
/// reflexivity concept, completion rules **CR-Self**); with
/// the `rbox` feature it adds the RBox role automaton (`rdfs:subPropertyOf` role inclusions,
/// `owl:propertyChainAxiom` chains, `owl:TransitiveProperty` — completion rules CR10/CR11);
/// completeness there additionally assumes a REGULAR told RBox — a non-regular one (forbidden
/// by the OWL 2 global restrictions) is flagged via the `rbox_non_regular` field, not silently
/// classified as if complete.
/// Axioms using constructs outside the active fragment are NOT applied and their class-axiom
/// occurrences are counted in [`Report::skipped_axioms`] (honest incompleteness rather than a
/// silent wrong answer). Two kinds of skip are distinguished by the EL theory:
/// - **Outside EL entirely** — `owl:unionOf`, `owl:complementOf`, `owl:allValuesFrom`, cardinality,
///   and a MULTI-individual `owl:oneOf` (the profile's `ObjectOneOf` admits exactly one individual;
///   more is a disjunction): these need ALC / Horn-SHIQ expressivity. (`owl:hasSelf` is NOT in this
///   list since sq-pbz04.2.6 — `ObjectHasSelf` is IN the EL profile and reasoned over by CR-Self;
///   only a malformed `owl:hasSelf` — non-`true`/non-boolean, or missing `owl:onProperty` — skips.)
/// - **Concrete domains (CR7–CR9)** — gated behind the `cdomain` feature (sq-pbz04.2.2).
///   WITHOUT it, every concrete-domain occurrence (`owl:onDataRange` / `owl:withRestrictions` /
///   `owl:onDatatype` and the literal-valued `owl:hasValue`/`owl:oneOf` forms, i.e.
///   `DataHasValue`/`DataOneOf`) is deferred into `skipped_axioms`. WITH it, the EXACT-tier
///   slice is applied on the shared `sparq_substrate::numeric` value tower — faceted
///   min/max{In,Ex}clusive restrictions over `xsd:decimal`/`xsd:integer`(+ the derived integer
///   types) and exact-numeric `DataHasValue`/singleton-`DataOneOf` points — while everything
///   else (pattern/length/digit facets, float/double or non-numeric bases and values,
///   `owl:onDataRange`, `owl:datatypeComplementOf`) STAYS skipped: honest deferral, never a
///   guessed sat/unsat verdict. See `src/cdomain.rs` for the full supported/deferred matrix.
///
/// Nominal honesty notes (sq-pbz04.2.1): CR6 uses the classic reachability side-condition — every
/// derived subsumption is sound, but completeness is claimed only for the typical "safe" nominal
/// usage, not every EL++ nominal interplay (see `classify.rs` `cr6_pass` for the boundary). The
/// TBox [`Classifier::classify`] / [`classify_graph`] do NOT internalize ABox assertions — that
/// stays their documented contract in EVERY feature state. Under the opt-in `abox` feature
/// (sq-pbz04.2.5) a SEPARATE `realize` / `realize_graph` entry internalizes
/// `ClassAssertion`/`ObjectPropertyAssertion` as safe-nominal axioms over exactly this CR6
/// machinery and reads off instance typings, `owl:sameAs`, and a whole-ontology `inconsistent`
/// verdict (the `realize`/`realize_graph` names are code spans, not intra-doc links, so this
/// always-compiled note stays clean under the no-`--all-features` rustdoc check).
///
/// Without `rbox`, RBox axioms (`rdfs:subPropertyOf` / property chains / transitivity) are also
/// left unapplied (roles are compared for equality only) — but those are gated capability, not the
/// deferred CR7–CR9 fragment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Distinct named classes (and ⊤/⊥ if they occur) seen by the extractor.
    pub named_classes: usize,
    /// Class-axioms (`subClassOf`/`equivalentClass`/`disjointWith`) whose class expression
    /// used a construct OUTSIDE the recognised fragment, and were therefore IGNORED. This
    /// includes constructs outside EL entirely (union/complement/universal/cardinality/
    /// multi-individual `oneOf`) AND the still-deferred concrete-domain shapes (CR7–CR9):
    /// ALL of them without the `cdomain` feature; with it, only the unsupported remainder
    /// (see [`Report`]'s fragment notes — the supported exact-numeric faceted ranges and
    /// point forms are then APPLIED, not skipped). Safe nominals (singleton `owl:oneOf`,
    /// object-valued `owl:hasValue`) are APPLIED since sq-pbz04.2.1 (CR6), no longer
    /// skipped. A non-zero count is the honest signal that the input used a construct this
    /// classifier does not reason over, so the emitted lattice may be incomplete for those
    /// axioms.
    pub skipped_axioms: usize,
    /// New `rdfs:subClassOf` triples emitted by [`classify_graph`].
    pub emitted_subsumptions: usize,
    /// Named classes found unsatisfiable (`⊑ owl:Nothing`).
    pub unsatisfiable_classes: usize,
    /// [SONNET-4.6] sq-26zuf: whether the saturation derived
    /// `owl:Thing rdfs:subClassOf owl:Nothing` (`⊤ ⊑ ⊥`). This global clash is separate from
    /// [`unsatisfiable_classes`](Self::unsatisfiable_classes), which counts only named classes
    /// and therefore excludes `owl:Thing`.
    pub thing_unsatisfiable: bool,
    /// [OPUS-4.8] sq-pbz04.2.5 (`abox`): ABox assertions the realiser DEFERRED as counted skips
    /// — a `DataPropertyAssertion` whose literal has no minted point range (EVERY literal object
    /// without `cdomain`; [SONNET-4.6] sq-vkq9u rescues the exact-numeric ones as `{a} ⊑ ∃q.{v}`
    /// when `abox` and `cdomain` are composed, leaving the string / lang-tagged / float-tier /
    /// ill-formed cases deferred) or a `ClassAssertion` whose class expression is outside the EL
    /// fragment. Set only by [`realize`] / [`realize_graph`]; the TBox `Classifier::classify` /
    /// `classify_graph` leave it `0`. A non-zero count is the honest "n instance facts not
    /// internalized" signal (fail-closed — never a guessed typing).
    #[cfg(feature = "abox")]
    pub skipped_assertions: usize,
    /// [SONNET-4.6] sq-pbz04.2.7 (`rbox`): new `rdfs:subPropertyOf` triples emitted by
    /// `classify_graph` for the told-inclusion role-lattice closure — the count of NON-REFLEXIVE
    /// closure pairs that were NOT already present in the graph (i.e. the purely-derived
    /// transitive-closure edges, excluding told inclusions already in the input). `0` without the
    /// `rbox` feature; `0` on a second (idempotent) call. Set only by `classify_graph`; the
    /// typed `Classifier::classify` API leaves it `0`.
    #[cfg(feature = "rbox")]
    pub emitted_role_subsumptions: usize,
    /// [SONNET-4.6] sq-oj06v (`rbox`): honest RBox-regularity flag — `true` when the TOLD RBox
    /// (role inclusions + property chains + transitive roles) is NOT regular, i.e. its
    /// dependency graph has a cycle through a property-chain constraint (detected by
    /// `rbox::told_rbox_regular` over the pre-binarization axioms). The OWL 2 global
    /// restrictions forbid such an RBox; the CR10/CR11 saturation still TERMINATES and every
    /// derived subsumption stays SOUND, but the EL+ completeness argument assumes regularity —
    /// so when this is `true` the classification MAY BE INCOMPLETE (the `skipped_axioms`
    /// honesty posture: flagged, never silently wrong). Conservative: told-inclusion edges
    /// participate in the cycle detection, so a chain constraint fed back through an inclusion
    /// is also flagged; pure inclusion cycles (equivalent roles), binary transitivity
    /// (`r ∘ r ⊑ r`), and chains with an `owl:topObjectProperty` superproperty (normatively
    /// admissible, unconditionally) are NOT. Set by every extraction-based entry (`Classifier::classify`,
    /// `classify_graph`, their `par` twins, and the `abox` realiser); `false` on a regular or
    /// RBox-free input, and absent (like the whole role automaton) without the `rbox` feature.
    #[cfg(feature = "rbox")]
    pub rbox_non_regular: bool,
}

/// The complete class-subsumption lattice for the named classes of a TBox, as a typed view
/// over the source dict ids. Built by [`Classifier::classify`]; the triple-emitting
/// [`classify_graph`] wraps the same computation.
pub struct ClassHierarchy {
    /// `subClassOf[c]` = the named super-classes of dict class `c` (every class subsuming it —
    /// the full transitive closure, which `is_subclass_of` queries directly). The DIRECT-subsumer
    /// (transitive-reduction) Hasse diagram is the separate `hasse` feature's `DirectHierarchy`
    /// (Phase E3, bead sq-s2nob); the closure is kept here because subsumption queries need it.
    subsumers: FxHashMap<Id, Vec<Id>>,
    /// Dict ids of the unsatisfiable named classes (`⊑ owl:Nothing`).
    unsatisfiable: Vec<Id>,
    report: Report,
    /// [OPUS-4.8] sq-pbz04.2.5 (`abox`): whole-ontology inconsistency (`{a} ⊑ ⊥` for an asserted
    /// individual, or a global `⊤ ⊑ ⊥`). `false` for a TBox `Classifier::classify` (which by its
    /// documented contract decides NAMED-class satisfiability, not whole-ontology consistency);
    /// set true only by the `abox` realiser.
    #[cfg(feature = "abox")]
    inconsistent: bool,
}

impl ClassHierarchy {
    /// All (transitive) named super-classes of `class`, by dict id (empty if `class` is not a
    /// classified named class or has no proper super-class besides ⊤). Excludes the class
    /// itself, `owl:Thing`, and `owl:Nothing`.
    pub fn super_classes(&self, class: Id) -> &[Id] {
        self.subsumers.get(&class).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Whether `sub` is subsumed by `sup` (`sub ⊑ sup`) under the classified TBox. Reflexive
    /// (`sub == sup`) and `sup == owl:Thing` are NOT reported here — this answers the proper
    /// derived subsumptions only, matching [`super_classes`](Self::super_classes).
    pub fn is_subclass_of(&self, sub: Id, sup: Id) -> bool {
        self.subsumers.get(&sub).is_some_and(|s| s.contains(&sup))
    }

    /// Dict ids of the named classes found unsatisfiable (`⊑ owl:Nothing`). A non-empty list
    /// means the TBox forces those classes to be empty (an EL+⊥ clash, e.g. via
    /// `owl:disjointWith`); it does NOT by itself make the whole ontology inconsistent (that
    /// needs an asserted instance of an unsatisfiable class — ABox reasoning, out of scope).
    pub fn unsatisfiable_classes(&self) -> &[Id] {
        &self.unsatisfiable
    }

    /// [OPUS-4.8] sq-pbz04.2.5 (`abox`): whether the WHOLE ontology is inconsistent — an asserted
    /// individual forced into `owl:Nothing` (`{a} ⊑ ⊥`, e.g. two disjoint `ClassAssertion`s) or a
    /// global `⊤ ⊑ ⊥`. Only ever `true` on a hierarchy produced by [`realize`] / [`realize_graph`]
    /// (the ABox path); a plain [`Classifier::classify`] result is always `false` (it decides
    /// named-class satisfiability, not whole-ontology consistency — see [`unsatisfiable_classes`]).
    ///
    /// [`unsatisfiable_classes`]: ClassHierarchy::unsatisfiable_classes
    #[cfg(feature = "abox")]
    pub fn is_inconsistent(&self) -> bool {
        self.inconsistent
    }

    /// The extraction/classification [`Report`] (named-class count, skipped non-EL axioms,
    /// unsatisfiable-class count). `emitted_subsumptions` is zero here — it is set by
    /// [`classify_graph`], which materializes the lattice as triples.
    pub fn report(&self) -> Report {
        self.report
    }

    /// Iterator over `(class dict id, complete named-subsumer slice)` for every classified class
    /// with a proper super-class. The slice is the full transitive closure `S(C)` (sorted by dict
    /// id). Consumed by the `hasse` transitive-reduction (Phase E3) to build the Hasse diagram.
    #[cfg(feature = "hasse")]
    pub(crate) fn subsumer_sets(&self) -> impl Iterator<Item = (Id, &[Id])> {
        self.subsumers.iter().map(|(&c, sups)| (c, sups.as_slice()))
    }
}

/// The OWL 2 EL consequence-based classifier. Stateless wrapper — [`Classifier::classify`]
/// runs the whole normalize → saturate → project pipeline over a borrowed graph.
///
/// ```
/// use sparq_core::Graph;
/// use sparq_reason_el::Classifier;
///
/// // A ⊑ ∃r.B, B ⊑ C, ∃r.C ⊑ D  ⊨  A ⊑ D  (CR4 — reasoning through an ∃r successor).
/// // [OPUS-5] sq-2ch27: the "…which OWL 2 RL cannot derive" gloss this line used to carry was
/// // measured WRONG for THIS shape — sparq's RL implements `scm-svf1`, and both restriction
/// // nodes appear syntactically, so RL closes it too. The RL gap that does hold is pinned in
/// // `crates/sparq-cli/tests/el_cli.rs::el_derives_what_rl_cannot` (TBox conjunction).
/// let ttl = r#"
///   @prefix : <http://ex/> .
///   @prefix owl: <http://www.w3.org/2002/07/owl#> .
///   @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
///   :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
///   :B rdfs:subClassOf :C .
///   [ owl:onProperty :r ; owl:someValuesFrom :C ] rdfs:subClassOf :D .
/// "#;
/// let (dict, triples) = Graph::parse_to_triples(ttl, "turtle").unwrap();
/// let h = Classifier::classify(&dict, &triples);
/// let a = dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new("http://ex/A").unwrap()));
/// let d = dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new("http://ex/D").unwrap()));
/// assert!(h.is_subclass_of(a, d));
/// ```
pub struct Classifier;

impl Classifier {
    /// Classifies the EL+⊥ TBox embedded in `triples`, returning the complete named-class
    /// subsumption [`ClassHierarchy`]. Does NOT mutate the graph; use [`classify_graph`] to
    /// materialize the lattice as triples. Single-threaded.
    pub fn classify(dict: &Dict, triples: &[[Id; 3]]) -> ClassHierarchy {
        // TBox only: `ExtractOpts::default()` never internalizes ABox assertions, so this path is
        // byte-identical in every `abox` feature state (the ABox reasoning is the separate
        // `abox::realize` entry — `classify` / `classify_graph` keep their documented TBox-only
        // contract, which is exactly what the conformance lane pins).
        let ex = extract::extract(dict, triples, extract::ExtractOpts::default());
        let sat = saturate_extracted(&ex);
        let cls = classify::classify(&sat, &ex.names);
        hierarchy_from(cls, &ex.names, ex.report)
    }

    /// [FABLE-5] sq-wy3i6 (Phase E4, `par`): like [`Classifier::classify`] but the CR
    /// saturation runs on a bounded pool of up to `threads` scoped workers (deterministic
    /// bulk-synchronous rounds — see the E4 design note in `classify.rs`). The derived
    /// subsumption closure — and therefore the returned hierarchy — is IDENTICAL to the
    /// single-threaded path for every `threads` value; `threads = 1` drives the same rule
    /// kernel through the round loop with no spawned worker. Native targets only (do not
    /// enable `par` for wasm, where `std::thread` cannot spawn — the default build stays
    /// single-threaded, which is exactly why `par` is opt-in).
    #[cfg(feature = "par")]
    pub fn classify_par(
        dict: &Dict,
        triples: &[[Id; 3]],
        threads: std::num::NonZeroUsize,
    ) -> ClassHierarchy {
        // Same TBox-only extraction contract as `classify` above.
        let ex = extract::extract(dict, triples, extract::ExtractOpts::default());
        let (sat, _stats) = saturate_extracted_par(&ex, threads);
        let cls = classify::classify(&sat, &ex.names);
        hierarchy_from(cls, &ex.names, ex.report)
    }
}

/// Runs the CR saturation for an [`extract::Extracted`], branching on the `rbox` feature to build
/// the role automaton first (so every asserted existential link is closed under role
/// inclusion/composition before CR4/CR5 fire). Shared by [`Classifier::classify`] (TBox) and the
/// `abox` realiser so both reason over the same fixpoint.
pub(crate) fn saturate_extracted(ex: &extract::Extracted) -> classify::Saturation {
    #[cfg(feature = "rbox")]
    {
        let role_box = rbox::RoleBox::build(&ex.role_axioms, ex.names.role_count());
        classify::saturate(&ex.axioms, &ex.names, &role_box)
    }
    #[cfg(not(feature = "rbox"))]
    {
        classify::saturate(&ex.axioms, &ex.names)
    }
}

/// [FABLE-5] sq-wy3i6 (E4): the parallel twin of [`saturate_extracted`] — same `rbox`
/// branching, but the fixpoint runs on the bounded scoped-worker pool. Kept structurally
/// parallel to `saturate_extracted` so the two entry families cannot drift on the role-box
/// wiring. Returns the [`ParPhaseStats`] compute/apply attribution alongside the fixpoint
/// (sq-q0o82) — callers that do not want it drop the second element.
#[cfg(feature = "par")]
pub(crate) fn saturate_extracted_par(
    ex: &extract::Extracted,
    threads: std::num::NonZeroUsize,
) -> (classify::Saturation, ParPhaseStats) {
    #[cfg(feature = "rbox")]
    {
        let role_box = rbox::RoleBox::build(&ex.role_axioms, ex.names.role_count());
        classify::saturate_par(&ex.axioms, &ex.names, &role_box, threads)
    }
    #[cfg(not(feature = "rbox"))]
    {
        classify::saturate_par(&ex.axioms, &ex.names, threads)
    }
}

/// Projects a [`classify::Classification`] onto dict ids: the named-class subsumption lattice +
/// the unsatisfiable classes, filling `Report::unsatisfiable_classes`. The whole-ontology
/// `abox`-inconsistency verdict is a separate concern the realiser sets afterwards; a plain TBox
/// classification leaves it `false`.
pub(crate) fn hierarchy_from(
    cls: classify::Classification,
    names: &normal::Names,
    mut report: Report,
) -> ClassHierarchy {
    // Re-key the named-concept lattice onto dict ids for the public view.
    let mut subsumers: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    for (&c, sups) in &cls.subsumers {
        let Some(cid) = names.dict_of(c) else {
            continue;
        };
        let row: Vec<Id> = sups.iter().filter_map(|&d| names.dict_of(d)).collect();
        if !row.is_empty() {
            subsumers.insert(cid, row);
        }
    }
    let unsatisfiable: Vec<Id> = cls
        .unsatisfiable
        .iter()
        .filter_map(|&c| names.dict_of(c))
        .collect();
    report.unsatisfiable_classes = unsatisfiable.len();
    report.thing_unsatisfiable = cls.thing_unsatisfiable;
    ClassHierarchy {
        subsumers,
        unsatisfiable,
        report,
        #[cfg(feature = "abox")]
        inconsistent: false,
    }
}

/// Classifies the EL+⊥ TBox in `triples` and MATERIALIZES the complete subsumption lattice
/// IN PLACE: appends every derived `(C rdfs:subClassOf D)` not already present (interning the
/// `rdfs:subClassOf` predicate into `dict` if absent), plus `(C rdf:type owl:Nothing)`-style
/// clashes are surfaced via [`Report::unsatisfiable_classes`] (not emitted as triples — the
/// caller decides whether to flag them, matching the RL `inconsistencies()` reporting shape).
///
/// With the `rbox` feature on, ALSO materializes the NON-REFLEXIVE told-inclusion role-lattice
/// closure as `rdfs:subPropertyOf` triples: every `(r, s)` pair with `r != s` and `s` in the
/// reflexive-transitive closure of the told `rdfs:subPropertyOf` / `owl:propertyChainAxiom` /
/// `owl:TransitiveProperty` role inclusions. This is a READOFF of the already-computed
/// `RoleBox::super_of` table — no new saturation. The `emitted_role_subsumptions` field of the
/// returned `Report` counts the NEW role triples added (told pairs already present are not
/// re-emitted). Idempotent: a second call adds nothing.
///
/// Returns the [`Report`] with `emitted_subsumptions` set to the number of NEW class triples
/// added. This is the seam the CLI/`Graph` integration uses — the emitted edges are ordinary
/// triples queryable by plain BGP eval, exactly like the RL `scm-*` output.
pub fn classify_graph(dict: &mut Dict, triples: &mut Vec<[Id; 3]>) -> Report {
    // [SONNET-4.6] sq-pbz04.2.7: share the extraction so the role box is available for the
    // readoff without a second triple scan. Without `rbox` this is byte-identical to the
    // previous `Classifier::classify` call (same three extraction/saturation/project steps,
    // same `ExtractOpts::default()` — never internalizes ABox assertions).
    let ex = extract::extract(dict, triples, extract::ExtractOpts::default());
    let sat = saturate_extracted(&ex);
    materialize_lattice(dict, triples, &ex, &sat)
}

/// [FABLE-5] sq-wy3i6 (Phase E4, `par`): like [`classify_graph`] but the CR saturation runs on
/// a bounded pool of up to `threads` scoped workers. The emitted triples — content AND order —
/// are IDENTICAL to [`classify_graph`]'s for every `threads` value: the parallel fixpoint
/// reaches the same closure (see the E4 design note in `classify.rs`) and the shared
/// materialization sorts before emitting. Native targets only (see `Classifier::classify_par`).
#[cfg(feature = "par")]
pub fn classify_graph_par(
    dict: &mut Dict,
    triples: &mut Vec<[Id; 3]>,
    threads: std::num::NonZeroUsize,
) -> Report {
    classify_graph_par_stats(dict, triples, threads).0
}

/// [SONNET-4.6] sq-q0o82 (E4 follow-up, `par`): [`classify_graph_par`] plus the
/// [`ParPhaseStats`] attribution of the saturation's PARALLEL compute phase against its
/// SEQUENTIAL apply phase. The graph mutation, the emitted triples and the [`Report`] are
/// byte-identical to [`classify_graph_par`] (which is this function discarding the stats) —
/// the instrumentation only reads two clocks per bulk-synchronous round.
///
/// This is the measurement seam for the open "should the apply phase be parallelised too?"
/// question: parallelising `add_link`'s CR4/CR5 (and the `rbox` CR10/CR11) closure would put
/// the identical-closure invariant at risk, so it is worth doing ONLY where
/// [`ParPhaseStats::apply_fraction`] on a real ontology says apply dominates. Native targets
/// only (see [`Classifier::classify_par`]).
#[cfg(feature = "par")]
pub fn classify_graph_par_stats(
    dict: &mut Dict,
    triples: &mut Vec<[Id; 3]>,
    threads: std::num::NonZeroUsize,
) -> (Report, ParPhaseStats) {
    let ex = extract::extract(dict, triples, extract::ExtractOpts::default());
    let (sat, stats) = saturate_extracted_par(&ex, threads);
    (materialize_lattice(dict, triples, &ex, &sat), stats)
}

/// The shared `classify_graph` / `classify_graph_par` tail: project the saturation onto named
/// classes, append the derived `rdfs:subClassOf` triples in deterministic sorted order (+ the
/// `rbox` role-lattice readoff), and fill the [`Report`] counts. Factored out (sq-wy3i6) so the
/// sequential and parallel entries cannot drift on emission/dedup/ordering behaviour —
/// behaviour-identical to the pre-E4 inline tail.
fn materialize_lattice(
    dict: &mut Dict,
    triples: &mut Vec<[Id; 3]>,
    ex: &extract::Extracted,
    sat: &classify::Saturation,
) -> Report {
    let cls = classify::classify(sat, &ex.names);
    let hierarchy = hierarchy_from(cls, &ex.names, ex.report);

    let sub_class_of = dict.intern_iri(RDFS_SUB_CLASS_OF);
    let _ = dict.intern_iri(OWL_NOTHING); // ensure the term exists for downstream clash checks.

    let mut present: rustc_hash::FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    let mut emitted = 0usize;
    // Deterministic order: by subclass dict id, then super-class dict id.
    let mut subs: Vec<(&Id, &Vec<Id>)> = hierarchy.subsumers.iter().collect();
    subs.sort_unstable_by_key(|(c, _)| **c);
    for (&c, sups) in subs {
        let mut sups = sups.clone();
        sups.sort_unstable();
        for d in sups {
            let t = [c, sub_class_of, d];
            if present.insert(t) {
                triples.push(t);
                emitted += 1;
            }
        }
    }

    // [SONNET-4.6] sq-pbz04.2.7 (`rbox`): role-lattice readoff. Emit the non-reflexive
    // told-inclusion closure as `rdfs:subPropertyOf` triples. READOFF ONLY — builds the
    // RoleBox a second time (cheap: O(role_count) BFS over a tiny graph) to read off
    // `super_of` without changing `saturate_extracted`'s internal structure.
    #[cfg(feature = "rbox")]
    let emitted_roles = emit_role_pairs(dict, ex, &mut present, triples);

    let mut report = hierarchy.report;
    report.emitted_subsumptions = emitted;
    #[cfg(feature = "rbox")]
    {
        report.emitted_role_subsumptions = emitted_roles;
    }
    report
}

/// [SONNET-4.6] sq-pbz04.2.7 (`rbox`): builds the `RoleBox` from the already-extracted role
/// axioms and emits every non-reflexive told-inclusion pair as a `rdfs:subPropertyOf` triple,
/// deduplicating against `present` so idempotency holds. Returns the count of NEW triples added.
/// Anonymous chain-intermediate roles (`Id::MAX` sentinel) are skipped — they never appear in
/// the output vocabulary.
#[cfg(feature = "rbox")]
fn emit_role_pairs(
    dict: &mut Dict,
    ex: &extract::Extracted,
    present: &mut rustc_hash::FxHashSet<[Id; 3]>,
    triples: &mut Vec<[Id; 3]>,
) -> usize {
    let role_box = rbox::RoleBox::build(&ex.role_axioms, ex.names.role_count());
    let sub_prop_of = dict.intern_iri(RDFS_SUB_PROPERTY_OF);
    // Collect, filter anonymous roles, sort for deterministic output order.
    let mut pairs: Vec<(Id, Id)> = role_box
        .inclusion_pairs()
        .filter_map(|(r, s)| {
            let r_id = ex.names.role_dict_of(r)?;
            let s_id = ex.names.role_dict_of(s)?;
            Some((r_id, s_id))
        })
        .collect();
    pairs.sort_unstable();
    let mut n = 0usize;
    for (r_id, s_id) in pairs {
        let t = [r_id, sub_prop_of, s_id];
        if present.insert(t) {
            triples.push(t);
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{NamedNode, Term as OTerm};
    use sparq_core::Graph;

    fn iri(dict: &Dict, s: &str) -> Id {
        dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(s.to_string())))
    }

    fn parse(ttl: &str) -> (Dict, Vec<[Id; 3]>) {
        Graph::parse_to_triples(ttl, "turtle").expect("parse")
    }

    const PRE: &str = r#"
        @prefix : <http://ex/> .
        @prefix owl: <http://www.w3.org/2002/07/owl#> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    "#;

    #[test]
    fn transitive_subclass() {
        let ttl = format!("{PRE} :A rdfs:subClassOf :B . :B rdfs:subClassOf :C .");
        let (dict, triples) = parse(&ttl);
        let h = Classifier::classify(&dict, &triples);
        let (a, c) = (iri(&dict, "http://ex/A"), iri(&dict, "http://ex/C"));
        assert!(h.is_subclass_of(a, c), "A ⊑ C must be derived transitively");
    }

    #[test]
    fn cr4_existential_traversal() {
        // The spike §1.3 counterexample: A ⊑ ∃r.B, B ⊑ C, ∃r.C ⊑ D ⊨ A ⊑ D. RL CANNOT do this.
        let ttl = format!(
            "{PRE}
             :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
             :B rdfs:subClassOf :C .
             [ owl:onProperty :r ; owl:someValuesFrom :C ] rdfs:subClassOf :D ."
        );
        let (dict, triples) = parse(&ttl);
        let h = Classifier::classify(&dict, &triples);
        let (a, d) = (iri(&dict, "http://ex/A"), iri(&dict, "http://ex/D"));
        assert!(
            h.is_subclass_of(a, d),
            "CR4 must derive A ⊑ D through the r-successor"
        );
    }

    #[test]
    fn conjunction_subsumption() {
        // (A ⊓ B) ⊑ C, X ⊑ A, X ⊑ B ⊨ X ⊑ C.
        let ttl = format!(
            "{PRE}
             [ owl:intersectionOf ( :A :B ) ] rdfs:subClassOf :C .
             :X rdfs:subClassOf :A . :X rdfs:subClassOf :B ."
        );
        let (dict, triples) = parse(&ttl);
        let h = Classifier::classify(&dict, &triples);
        let (x, c) = (iri(&dict, "http://ex/X"), iri(&dict, "http://ex/C"));
        assert!(
            h.is_subclass_of(x, c),
            "CR2 must derive X ⊑ C from the conjunction"
        );
    }

    #[test]
    fn disjoint_makes_unsatisfiable() {
        // A disjointWith B, X ⊑ A, X ⊑ B ⊨ X ⊑ ⊥.
        let ttl = format!(
            "{PRE}
             :A owl:disjointWith :B .
             :X rdfs:subClassOf :A . :X rdfs:subClassOf :B ."
        );
        let (dict, triples) = parse(&ttl);
        let h = Classifier::classify(&dict, &triples);
        let x = iri(&dict, "http://ex/X");
        assert!(
            h.unsatisfiable_classes().contains(&x),
            "X must be unsatisfiable (A ⊓ B ⊑ ⊥)"
        );
    }

    #[test]
    fn classify_graph_materializes_and_is_idempotent() {
        let ttl = format!("{PRE} :A rdfs:subClassOf :B . :B rdfs:subClassOf :C .");
        let (mut dict, mut triples) = parse(&ttl);
        let before = triples.len();
        let r1 = classify_graph(&mut dict, &mut triples);
        // A ⊑ C is the one NEW edge (A⊑B, B⊑C are asserted).
        assert_eq!(r1.emitted_subsumptions, 1, "exactly the A ⊑ C edge is new");
        assert_eq!(triples.len(), before + 1);
        let r2 = classify_graph(&mut dict, &mut triples);
        assert_eq!(r2.emitted_subsumptions, 0, "second call is idempotent");
    }

    #[test]
    fn non_el_axioms_are_skipped_not_misapplied() {
        // owl:unionOf is outside EL+⊥: the axiom must be skipped, not crash or mis-derive.
        let ttl = format!(
            "{PRE}
             [ owl:unionOf ( :A :B ) ] rdfs:subClassOf :C .
             :A rdfs:subClassOf :D ."
        );
        let (dict, triples) = parse(&ttl);
        let h = Classifier::classify(&dict, &triples);
        assert!(
            h.report().skipped_axioms >= 1,
            "the unionOf axiom must be recorded as skipped"
        );
        let (a, d) = (iri(&dict, "http://ex/A"), iri(&dict, "http://ex/D"));
        assert!(
            h.is_subclass_of(a, d),
            "the EL part (A ⊑ D) must still classify"
        );
    }

    #[test]
    fn out_of_fragment_nominal_shapes_are_surfaced_as_skipped() {
        // [FABLE-5] sq-pbz04.2.1: safe nominals are now APPLIED (CR6), but the out-of-fragment
        // shapes stay honestly skipped: a MULTI-individual `owl:oneOf` is a disjunction
        // (outside EL — the profile's ObjectOneOf admits exactly one individual) and a
        // LITERAL-valued `owl:hasValue` is DataHasValue (a concrete domain, the deferred
        // CR7–CR9 surface). Neither may be silently mis-classified.
        let ttl = format!(
            "{PRE}
             [ owl:oneOf ( :a :b ) ] rdfs:subClassOf :C .
             :D rdfs:subClassOf [ owl:onProperty :p ; owl:hasValue 5 ] .
             :E rdfs:subClassOf :F ."
        );
        let (dict, triples) = parse(&ttl);
        let h = Classifier::classify(&dict, &triples);
        // [FABLE-5] sq-pbz04.2.2: under `cdomain` the literal `owl:hasValue 5`
        // (DataHasValue over an exact integer) is APPLIED, so only the multi-individual
        // oneOf remains a skip; without it both shapes are skipped as before.
        let want = if cfg!(feature = "cdomain") { 1 } else { 2 };
        assert_eq!(
            h.report().skipped_axioms,
            want,
            "out-of-fragment shapes must be skipped (feature-state-dependent count)"
        );
        // The plain-EL part must still classify, and the skipped axioms derive nothing.
        let (e, f) = (iri(&dict, "http://ex/E"), iri(&dict, "http://ex/F"));
        assert!(h.is_subclass_of(e, f), "the EL part (E ⊑ F) must still classify");
        let (d, c) = (iri(&dict, "http://ex/D"), iri(&dict, "http://ex/C"));
        assert!(
            !h.is_subclass_of(d, c),
            "skipped nominal shapes must not fabricate subsumptions"
        );
    }

    #[test]
    fn safe_nominal_has_value_classifies_instead_of_skipping() {
        // [FABLE-5] sq-pbz04.2.1: the CR6 slice — an object-valued hasValue (∃r.{a}) on both
        // sides of ⊑ must classify (A ⊑ B through the shared nominal filler) with ZERO skips.
        let ttl = format!(
            "{PRE}
             :A rdfs:subClassOf [ owl:onProperty :r ; owl:hasValue :a ] .
             [ owl:onProperty :r ; owl:hasValue :a ] rdfs:subClassOf :B ."
        );
        let (dict, triples) = parse(&ttl);
        let h = Classifier::classify(&dict, &triples);
        let (a, b) = (iri(&dict, "http://ex/A"), iri(&dict, "http://ex/B"));
        assert!(h.is_subclass_of(a, b), "A ⊑ ∃r.{{a}} ⊑ B must classify");
        assert_eq!(h.report().skipped_axioms, 0, "safe nominals are in-fragment now");
    }

    #[test]
    fn deferred_concrete_domains_are_surfaced_as_skipped() {
        // CR7–CR9 (concrete domains): WITHOUT `cdomain` a faceted-datatype restriction must
        // be counted in `skipped_axioms` rather than treating the datatype node as an opaque
        // named class (which would silently drop the concrete-domain semantics). WITH
        // `cdomain` (sq-pbz04.2.2) this supported shape is APPLIED — zero skips, and the
        // satisfiable range derives no clash (tests/cdomain.rs carries the full matrix).
        let ttl = format!(
            "{PRE}
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
             :Adult rdfs:subClassOf
               [ owl:onProperty :age ;
                 owl:someValuesFrom
                   [ owl:onDatatype xsd:integer ;
                     owl:withRestrictions ( [ xsd:minInclusive 18 ] ) ] ] .
             :G rdfs:subClassOf :H ."
        );
        let (dict, triples) = parse(&ttl);
        let h = Classifier::classify(&dict, &triples);
        #[cfg(not(feature = "cdomain"))]
        assert!(
            h.report().skipped_axioms >= 1,
            "the concrete-domain (onDatatype/withRestrictions) axiom must be recorded as skipped (CR7–CR9 deferred), got {}",
            h.report().skipped_axioms
        );
        #[cfg(feature = "cdomain")]
        {
            assert_eq!(
                h.report().skipped_axioms,
                0,
                "a supported faceted range is applied under `cdomain`, not skipped"
            );
            assert!(
                h.unsatisfiable_classes().is_empty(),
                "the SATISFIABLE range [18, ∞) must not clash"
            );
        }
        let (g, hh) = (iri(&dict, "http://ex/G"), iri(&dict, "http://ex/H"));
        assert!(h.is_subclass_of(g, hh), "the EL part (G ⊑ H) must still classify");
    }

    #[test]
    fn tbox_self_restriction_subsumption() {
        // [OPUS-4.8] sq-pbz04.2.6: A ⊑ ∃r.Self, ∃r.Self ⊑ D ⊨ A ⊑ D. Both hasSelf restriction
        // bnodes are distinct but share (onProperty r, hasSelf true), so they decode to the SAME
        // ∃r.Self atom; CR1 threads A ⊑ ∃r.Self ⊑ D (CR-Self-2 realised via the self-concept).
        let ttl = format!(
            "{PRE}
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
             :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ; owl:hasSelf \"true\"^^xsd:boolean ] .
             [ a owl:Restriction ; owl:onProperty :r ; owl:hasSelf \"true\"^^xsd:boolean ] rdfs:subClassOf :D ."
        );
        let (dict, triples) = parse(&ttl);
        let h = Classifier::classify(&dict, &triples);
        let (a, d) = (iri(&dict, "http://ex/A"), iri(&dict, "http://ex/D"));
        assert!(h.is_subclass_of(a, d), "A ⊑ ∃r.Self ⊑ D must classify");
        assert_eq!(h.report().skipped_axioms, 0, "owl:hasSelf is in-fragment (CR-Self)");
    }

    #[test]
    fn general_self_link_does_not_trigger_cr_self_2() {
        // The load-bearing CR-Self-2 side-condition: `A ⊑ ∃r.A` creates a general link (A,A) ∈
        // R(r) whose invariant is only A ⊑ ∃r.A, NOT A ⊑ ∃r.Self (the successor is a FRESH A-node,
        // not A itself). So `∃r.Self ⊑ D` must NOT derive A ⊑ D — firing CR-Self-2 off the raw
        // link would be UNSOUND. No derivation without a positive rule premise.
        let ttl = format!(
            "{PRE}
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
             :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ; owl:someValuesFrom :A ] .
             [ a owl:Restriction ; owl:onProperty :r ; owl:hasSelf \"true\"^^xsd:boolean ] rdfs:subClassOf :D ."
        );
        let (dict, triples) = parse(&ttl);
        let h = Classifier::classify(&dict, &triples);
        let (a, d) = (iri(&dict, "http://ex/A"), iri(&dict, "http://ex/D"));
        assert!(
            !h.is_subclass_of(a, d),
            "a general (A,A) link is not ∃r.Self — CR-Self-2 must not fire"
        );
        assert_eq!(h.report().skipped_axioms, 0, "someValuesFrom + hasSelf are both in-fragment");
    }

    #[test]
    fn malformed_has_self_is_skipped() {
        // [OPUS-4.8] sq-pbz04.2.6 (fail-closed): a `hasSelf "false"` and a `hasSelf true` WITHOUT
        // owl:onProperty are BOTH counted skips, never a guessed self-restriction; the EL part
        // still classifies and the skipped axioms fabricate nothing.
        let ttl = format!(
            "{PRE}
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
             :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ; owl:hasSelf \"false\"^^xsd:boolean ] .
             :B rdfs:subClassOf [ a owl:Restriction ; owl:hasSelf \"true\"^^xsd:boolean ] .
             :E rdfs:subClassOf :F ."
        );
        let (dict, triples) = parse(&ttl);
        let h = Classifier::classify(&dict, &triples);
        assert_eq!(
            h.report().skipped_axioms,
            2,
            "hasSelf false + hasSelf-without-onProperty are both counted skips"
        );
        let (e, f) = (iri(&dict, "http://ex/E"), iri(&dict, "http://ex/F"));
        assert!(h.is_subclass_of(e, f), "the EL part (E ⊑ F) still classifies");
        let a = iri(&dict, "http://ex/A");
        assert!(h.super_classes(a).is_empty(), "a skipped hasSelf axiom fabricates no subsumption");
    }

    #[test]
    fn empty_graph_is_safe() {
        let (dict, triples) = (Dict::new(), Vec::<[Id; 3]>::new());
        let h = Classifier::classify(&dict, &triples);
        assert_eq!(h.report().emitted_subsumptions, 0);
        assert!(h.unsatisfiable_classes().is_empty());
    }
}
