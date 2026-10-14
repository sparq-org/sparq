// sq-clsv6 (Phase E5, `incremental`): incremental classification under TBox edits.
//
// Spike: research/owl2-el-ql-reasoning-spike.md §5 (EL-track E5). The E1–E4 entries
// (`Classifier::classify` / `classify_graph` / their `par` twins) are STATELESS: every call
// re-extracts the TBox and re-runs the CR1–CR6 fixpoint from scratch. An ontology-authoring or
// stream-ingest loop that edits a TBox and re-classifies pays that from-scratch cost per edit even
// when the edit added three triples to a 300k-axiom ontology. This module keeps the saturation
// ALIVE across edits and, for the edit shape that is provably monotone, folds the delta into it.
//
// ## What is actually incremental, and what is not (read this before believing the name)
//
// EL saturation is a MONOTONE least fixpoint, which cuts exactly one way:
//
// * **Addition is incremental.** Adding axioms can only ADD subsumptions, so the retained closure
//   stays valid and only the contexts a new axiom can trigger need re-processing. That is
//   `classify::resaturate`, whose doc comment carries the least-fixpoint argument for why the
//   result is bit-for-bit the closure a from-scratch run computes.
// * **Retraction is NOT.** Removing an axiom can invalidate a derived subsumption, and the
//   saturation keeps no provenance to find which — repairing it needs either derivation
//   bookkeeping (DRed) or ELK's affected-context tracking (Kazakov & Klinov, "Incremental
//   Reasoning in OWL EL without Bookkeeping", ISWC 2013). NEITHER IS IMPLEMENTED HERE. A retraction
//   falls back to a full re-classification, reported as [`FullReason::Retraction`].
//
// So this is an incremental-ADDITION classifier with an honest full-rebuild fallback, not a
// general incremental reasoner. Every edit returns the [`EditDisposition`] it took, so a caller can
// SEE which path ran instead of assuming the fast one; the deletion half is deferred, never faked.
// The one guarantee that holds unconditionally, on BOTH paths, is the one the E5 acceptance
// criterion asks for: the hierarchy after an edit equals a full re-classification of the edited
// graph (`tests/incremental.rs` pins it edit-by-edit against the ELK-derived closure oracle and
// against `Classifier::classify`).
//
// ## Honest cost notes
//
// Every edit rebuilds the structural index over the FULL post-edit graph (one linear pass, no
// fixpoint) so a new axiom's class nodes decode against the same neighbourhood a from-scratch run
// would see, and the incremental path additionally scans the retained closure once to locate the
// contexts a new axiom can trigger. The win is skipping the SATURATION, not the extraction scan;
// on a small graph the fixed costs can dominate and the incremental path is not automatically
// faster. No timing figure is asserted here — this box is not canonical.

use crate::extract::{self, AdditionBlocker, ExtractOpts};
use crate::{classify, ClassHierarchy, Report};
use rustc_hash::FxHashSet;
use sparq_core::dict::{Dict, Id};

/// Which path an [`IncrementalClassifier::apply_edits`] call took — the honest disclosure of
/// whether the edit was actually handled incrementally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditDisposition {
    /// The edit changed nothing: every added triple was already present and every removed one
    /// absent. No re-classification of any kind ran.
    NoOp,
    /// The edit was a monotone extension and was folded into the live saturation.
    Incremental,
    /// The edit was re-classified from scratch, for the given reason.
    Full(FullReason),
}

impl EditDisposition {
    /// Whether the edit was folded into the live saturation (as opposed to a no-op or a full
    /// re-classification). The hierarchy is correct either way — this reports which path RAN.
    pub fn is_incremental(&self) -> bool {
        matches!(self, EditDisposition::Incremental)
    }
}

/// Why an edit needed a full re-classification instead of an incremental fold. Each variant is a
/// capability this phase deliberately did not build, not a transient failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullReason {
    /// The edit REMOVED at least one triple that was present. EL saturation is monotone and keeps
    /// no derivation provenance, so a retraction cannot be repaired incrementally without the
    /// DRed / affected-context machinery this phase does not implement (see the module note).
    Retraction,
    /// An added triple attached class-expression structure to a node the graph already mentioned,
    /// so it may CHANGE what an existing axiom means rather than only adding axioms — including the
    /// first structure on a node an existing axiom currently reads as an opaque class atom.
    ExistingNode,
    /// An added triple carried vocabulary whose effect is not delta-local — RBox role axioms
    /// (`rdfs:subPropertyOf` / `owl:propertyChainAxiom` / `owl:TransitiveProperty`, which change
    /// the automaton every existing link is closed under), concrete-domain facets, or an
    /// unrecognised predicate the fast path will not reason about by guess.
    Vocabulary,
    /// (`cdomain` only) the graph carries concrete-domain vocabulary. Those datatype concepts are
    /// minted by a whole-graph pre-pass whose node-to-concept map the delta path cannot
    /// reconstruct, so every edit on such a graph takes the full path.
    ConcreteDomain,
}

/// What one [`IncrementalClassifier::apply_edits`] call did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IncrementalReport {
    /// Which path ran (and, on a full re-classification, why).
    pub disposition: EditDisposition,
    /// Triples in `added` that were genuinely new (already-present ones are ignored).
    pub added_triples: usize,
    /// Triples in `removed` that were genuinely present (absent ones are ignored).
    pub removed_triples: usize,
    /// Normal-form axioms the edit contributed. `0` on a full re-classification (which re-derives
    /// the whole axiom set rather than a delta) and on a no-op.
    pub added_axioms: usize,
    /// Retained memberships re-queued as the delta frontier — the incremental work measure. `0` on
    /// a full re-classification and on a no-op.
    pub reseeded_memberships: usize,
    /// The post-edit classification [`Report`] (also available via
    /// [`ClassHierarchy::report`](crate::ClassHierarchy::report) on [`IncrementalClassifier::hierarchy`]).
    ///
    /// `skipped_axioms` agrees with a from-scratch extraction of the edited graph on BOTH paths:
    /// the incremental path accumulates each class-axiom triple's verdict exactly once over the
    /// graph's lifetime. `named_classes` does NOT: it reports the live concept-index size, which on
    /// the incremental path also counts the fresh normalization names earlier edits minted (see
    /// `classify::resaturate`). Every field that describes the ENTAILMENTS agrees; that counter
    /// describes the index.
    pub report: Report,
}

/// sq-clsv6 (Phase E5, `incremental`): an OWL 2 EL classifier that keeps its
/// saturation alive across TBox edits — the stateful sibling of the stateless
/// [`Classifier`](crate::Classifier).
///
/// A monotone extension (added class axioms and brand-new class-expression nodes) is folded into
/// the live fixpoint; anything else — **any retraction**, an RBox change, a mutation of a live
/// class-expression node — is re-classified from scratch. Which path ran is reported, never hidden;
/// read the module documentation for exactly what "incremental" covers here.
///
/// The hierarchy after every edit equals a full re-classification of the edited graph, whichever
/// path ran. This is a **TBox** classifier: like [`Classifier::classify`](crate::Classifier::classify)
/// it never internalizes ABox assertions in any feature state.
///
/// The caller owns the [`Dict`]: every IRI a later edit mentions must already be interned in the
/// dict handed to [`IncrementalClassifier::apply_edits`], exactly as for the stateless entries.
///
/// ```
/// use sparq_core::Graph;
/// use sparq_reason_el::IncrementalClassifier;
///
/// // Classify A ⊑ B once, then ADD B ⊑ C and re-classify incrementally: A ⊑ C appears.
/// let ttl = r#"
///   @prefix : <http://ex/> .
///   @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
///   :A rdfs:subClassOf :B .
///   :B rdfs:subClassOf :C .
/// "#;
/// let (dict, all) = Graph::parse_to_triples(ttl, "turtle").unwrap();
/// let iri = |s: &str| dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new(s).unwrap()));
/// let (a, c) = (iri("http://ex/A"), iri("http://ex/C"));
/// let sub_of = iri("http://www.w3.org/2000/01/rdf-schema#subClassOf");
/// let b_sub_c = [iri("http://ex/B"), sub_of, c];
///
/// // Start from the graph WITHOUT B ⊑ C.
/// let start: Vec<_> = all.iter().copied().filter(|t| *t != b_sub_c).collect();
/// let mut inc = IncrementalClassifier::new(&dict, &start);
/// assert!(!inc.hierarchy().is_subclass_of(a, c));
///
/// let r = inc.apply_edits(&dict, &[b_sub_c], &[]);
/// assert!(r.disposition.is_incremental());
/// assert!(inc.hierarchy().is_subclass_of(a, c));
/// ```
pub struct IncrementalClassifier {
    /// The live graph — the pre-edit triple set plus every applied edit, de-duplicated.
    triples: Vec<[Id; 3]>,
    /// Set view of `triples`, so an edit can tell a genuinely-new triple from a re-add.
    present: FxHashSet<[Id; 3]>,
    /// EVERY term id mentioned by `triples`, in ANY position. An added structural triple is
    /// delta-local only if its subject is NOT here (see `extract::addition_is_incrementally_safe`).
    /// SUBJECTS ALONE WOULD BE WRONG: a node that so far occurs only as an OBJECT — e.g. `_:b` in
    /// `:A rdfs:subClassOf _:b`, which decodes as an opaque class atom while it carries no
    /// structure — changes what that existing axiom means the moment `_:b owl:onProperty :r`
    /// arrives, so it is not a brand-new node either.
    mentioned: FxHashSet<Id>,
    /// The live extraction: the accumulated axiom set, the STABLE name table every retained
    /// membership is indexed by, and the running report.
    ex: extract::Extracted,
    /// The live saturation, resumed rather than rebuilt whenever the edit allows it.
    sat: classify::Saturation,
    /// (`cdomain`) whether the graph carries concrete-domain vocabulary, which disables the
    /// incremental path. Recomputed on every full rebuild; an edit cannot set it (the vocabulary is
    /// rejected as [`FullReason::Vocabulary`] before it can be folded in).
    #[cfg(feature = "cdomain")]
    concrete_domain: bool,
}

impl IncrementalClassifier {
    /// Classifies `triples` from scratch and keeps the saturation for later
    /// [`IncrementalClassifier::apply_edits`] calls. Equivalent to
    /// [`Classifier::classify`](crate::Classifier::classify) for the initial hierarchy.
    pub fn new(dict: &Dict, triples: &[[Id; 3]]) -> IncrementalClassifier {
        Self::from_scratch(dict, triples.to_vec())
    }

    /// The full classify-from-scratch build shared by [`IncrementalClassifier::new`] and every
    /// full-re-classification fallback, so the two cannot drift.
    fn from_scratch(dict: &Dict, triples: Vec<[Id; 3]>) -> IncrementalClassifier {
        let ex = extract::extract(dict, &triples, ExtractOpts::default());
        let sat = crate::saturate_extracted(&ex);
        IncrementalClassifier {
            present: triples.iter().copied().collect(),
            mentioned: triples.iter().flatten().copied().collect(),
            #[cfg(feature = "cdomain")]
            concrete_domain: extract::has_concrete_domain_vocab(dict, &triples),
            triples,
            ex,
            sat,
        }
    }

    /// Applies a TBox edit — add `added`, remove `removed` — and re-classifies, incrementally when
    /// the edit is a monotone extension and from scratch otherwise. Returns the
    /// [`IncrementalReport`] describing which path ran; [`IncrementalClassifier::hierarchy`] then
    /// yields the post-edit lattice, which equals a full re-classification of the edited graph on
    /// either path.
    ///
    /// Already-present additions and absent removals are ignored (they change nothing), so a
    /// re-applied edit is a [`EditDisposition::NoOp`] rather than a duplicated axiom.
    pub fn apply_edits(
        &mut self,
        dict: &Dict,
        added: &[[Id; 3]],
        removed: &[[Id; 3]],
    ) -> IncrementalReport {
        // Reduce the edit to its NET EFFECT, applying "remove, then add": a triple in both lists
        // ends up present, so it is neither an effective removal nor (if it was already there) an
        // effective addition. Only the net effect can change the classification, and only it may
        // force the full path — so a no-op edit stays a no-op.
        let readd: FxHashSet<[Id; 3]> = added.iter().copied().collect();
        let drop: FxHashSet<[Id; 3]> = removed
            .iter()
            .copied()
            .filter(|t| self.present.contains(t) && !readd.contains(t))
            .collect();
        let mut seen: FxHashSet<[Id; 3]> = FxHashSet::default();
        let new_triples: Vec<[Id; 3]> = added
            .iter()
            .copied()
            .filter(|t| !self.present.contains(t) && seen.insert(*t))
            .collect();

        let mut report = IncrementalReport {
            disposition: EditDisposition::NoOp,
            added_triples: new_triples.len(),
            removed_triples: drop.len(),
            added_axioms: 0,
            reseeded_memberships: 0,
            report: self.ex.report,
        };
        if drop.is_empty() && new_triples.is_empty() {
            return report;
        }

        // The disposition is decided against the PRE-edit graph (whether a node was mentioned
        // before is what makes a structural triple delta-local), so classify before mutating.
        let disposition = self.disposition_for(dict, &new_triples, &drop);
        self.apply_triples(&new_triples, &drop);

        match disposition {
            EditDisposition::Incremental => {
                let added_axioms =
                    extract::extract_added(dict, &self.triples, &new_triples, &mut self.ex.names);
                self.ex.report.skipped_axioms += added_axioms.skipped;
                let first = self.ex.axioms.len();
                self.ex.axioms.extend(added_axioms.axioms);
                let delta = &self.ex.axioms[first..];
                report.added_axioms = delta.len();
                // The delta interned the edit's named classes and minted its own fresh
                // normalization names, so re-read the live index size with the SAME semantics the
                // full extraction uses (`extract::extract`'s closing `names.concept_count()`).
                // Without this the counter would report the pre-edit index forever.
                self.ex.report.named_classes = self.ex.names.concept_count();
                report.reseeded_memberships = Self::resume(&mut self.sat, &self.ex, first);
            }
            EditDisposition::Full(_) => {
                let triples = std::mem::take(&mut self.triples);
                *self = Self::from_scratch(dict, triples);
            }
            // `disposition_for` never returns NoOp — the no-op edit returned above.
            EditDisposition::NoOp => {}
        }
        // `named_classes` tracks the live index, so read the report back AFTER the re-classification
        // rather than reusing the pre-edit snapshot.
        report.disposition = disposition;
        report.report = self.ex.report;
        report
    }

    /// Resumes the live saturation over `ex`, whose axioms from index `first` on are the edit's
    /// delta. Split out so the `rbox` role-box rebuild sits in ONE place: the told role axioms are
    /// unchanged (an RBox edit is rejected as [`FullReason::Vocabulary`]), but the delta may have
    /// minted new roles, so the automaton is rebuilt at the new role count — otherwise
    /// `super_roles` would index past its table.
    fn resume(
        sat: &mut classify::Saturation,
        ex: &extract::Extracted,
        first: usize,
    ) -> usize {
        #[cfg(feature = "rbox")]
        {
            let role_box = crate::rbox::RoleBox::build(&ex.role_axioms, ex.names.role_count());
            classify::resaturate(
                sat,
                &ex.axioms,
                &ex.axioms[first..],
                &ex.names,
                &role_box,
            )
        }
        #[cfg(not(feature = "rbox"))]
        classify::resaturate(sat, &ex.axioms, &ex.axioms[first..], &ex.names)
    }

    /// The disposition for an edit with effective delta `new_triples` / `drop`, decided against the
    /// PRE-edit state. Never returns [`EditDisposition::NoOp`] (the caller handles that case).
    fn disposition_for(
        &self,
        dict: &Dict,
        new_triples: &[[Id; 3]],
        drop: &FxHashSet<[Id; 3]>,
    ) -> EditDisposition {
        if !drop.is_empty() {
            return EditDisposition::Full(FullReason::Retraction);
        }
        #[cfg(feature = "cdomain")]
        if self.concrete_domain {
            return EditDisposition::Full(FullReason::ConcreteDomain);
        }
        match extract::addition_is_incrementally_safe(dict, &self.mentioned, new_triples) {
            Ok(()) => EditDisposition::Incremental,
            Err(AdditionBlocker::ExistingNode) => EditDisposition::Full(FullReason::ExistingNode),
            Err(AdditionBlocker::Vocabulary) => EditDisposition::Full(FullReason::Vocabulary),
        }
    }

    /// Folds the effective delta into the live triple set and its derived views. On the
    /// full-re-classification path `from_scratch` rebuilds `present` / `subjects` anyway; keeping
    /// them right here means the two paths leave identical state either way.
    fn apply_triples(&mut self, new_triples: &[[Id; 3]], drop: &FxHashSet<[Id; 3]>) {
        if !drop.is_empty() {
            self.triples.retain(|t| !drop.contains(t));
            for t in drop {
                self.present.remove(t);
            }
        }
        for &t in new_triples {
            if self.present.insert(t) {
                self.triples.push(t);
                self.mentioned.extend(t);
            }
        }
        if !drop.is_empty() {
            // A retraction can orphan a term, and `mentioned` must stay EXACT: over-approximating
            // only costs a needless full rebuild, but UNDER-approximating would wrongly admit a
            // mutation of a live node as delta-local.
            self.mentioned = self.triples.iter().flatten().copied().collect();
        }
    }

    /// The current subsumption lattice — the same [`ClassHierarchy`] a from-scratch
    /// [`Classifier::classify`](crate::Classifier::classify) of
    /// [`IncrementalClassifier::triples`] returns.
    pub fn hierarchy(&self) -> ClassHierarchy {
        let cls = classify::classify(&self.sat, &self.ex.names);
        crate::hierarchy_from(cls, &self.ex.names, self.ex.report)
    }

    /// The live graph: the triples this classifier was built from plus every applied edit,
    /// de-duplicated. Order is insertion order, so it is stable across edits.
    pub fn triples(&self) -> &[[Id; 3]] {
        &self.triples
    }
}
