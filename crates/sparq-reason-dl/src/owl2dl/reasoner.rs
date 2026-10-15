//! The OWL 2 DL reasoner: consistency, class satisfiability and classification over the
//! hypertableau calculus.
//!
//! Every answer is three-valued and fail-closed. A verdict is given only when the ontology is
//! in OWL 2 DL (no [blocking](super::Violation::blocks_reasoning) profile violation) and
//! inside the fragment the clausifier translates; otherwise the answer is
//! [`Unknown`](Consistency::Unknown) with the reason, and the triples behind it when there are
//! any.
//!
//! The fragment is currently SHOIQ(D) plus the OWL 2 role characteristics and ABox:
//! classes and all class constructors including nominals (`ObjectOneOf`,
//! `ObjectHasValue`), object properties with inverses, hierarchies, transitivity,
//! functionality, reflexivity, irreflexivity, symmetry, asymmetry and disjointness,
//! `hasSelf`, qualified cardinality, class / object-property / negative-property / same /
//! different assertions, and data properties: domain, range, sub- / equivalent / disjoint
//! data properties, functionality, the data restrictions (some, all, has-value, qualified
//! cardinality) and positive and negative data property assertions, over data ranges of the
//! OWL 2 datatype map (datatypes, facet restrictions, `DataOneOf`, `DataIntersectionOf`,
//! `DataUnionOf`, `DataComplementOf` and acyclic `DatatypeDefinition`s), decided by
//! [`super::data_ranges`], and keys (`HasKey`, over named individuals; a key whose data
//! values the tableau has not fixed to literals answers `Unknown(Unconfirmed)`). Property
//! chains other than `R∘R ⊑ R`, the top and bottom
//! properties, datatypes outside the map, and number restrictions above
//! [`MAX_CARDINALITY`] answer `Unknown(OutOfFragment)`; a datatype question the oracle cannot
//! decide answers `Unknown(DatatypeUnknown)`. An ill-typed literal in an assertion,
//! `DataHasValue` or `DataOneOf` has no value, so the ontology has no model: `Inconsistent`.
//!
//! Nominals are decided without SHOIQ's NI-rule. A candidate model is accepted only when
//! its completion graph provably unravels into a model (see the tableau's `unravels`); when
//! nominals tie a repeated part of the graph to an individual whose number restrictions the
//! repetition could break, that branch is closed as unconfirmed, and if no other branch
//! gives a model the answer is `Unknown(Unconfirmed)`, never a guess.
//!
//! Classification is TBox-only when the ontology has no nominals, which is exact for a
//! consistent ontology: for SHIQ(D) the ABox cannot change a subsumption between classes.
//! With nominals every class test includes the ABox.

pub use super::clausify::MAX_CARDINALITY;
use super::clausify::{clausify, AtomKind, Clauses};
use super::datatypes::{self, ValueError};
use super::dl_profile::{
    check_dl, for_each_class_in_axiom, for_each_range_in_axiom, DlProfileReport, Violation,
};
use super::hypertableau::{ClauseIndex, Stop, Tableau};
use super::model::{Axiom, ClassExpression, DataRange, Individual, Literal, Ontology};
use super::vocab::{iri_of, OWL};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};
use std::cell::{Cell, OnceCell};

pub use super::hypertableau::Budget;

/// Why the reasoner gave no verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnknownReason {
    /// The ontology uses a construct outside the decided fragment.
    OutOfFragment {
        /// The construct, e.g. `"ObjectOneOf"`.
        construct: &'static str,
        /// The input triples of the axiom that uses it.
        source: Vec<usize>,
    },
    /// The ontology breaks an OWL 2 DL global restriction the calculus relies on.
    NotInDl(Vec<Violation>),
    /// The deterministic work budget ran out.
    ResourceBudget,
    /// The datatype oracle ([`super::data_ranges`]) answered Unknown for a data range, a
    /// literal or a combination of data constraints the decision depends on (for example
    /// an XSD pattern with a block escape, or a value beyond the represented range).
    DatatypeUnknown {
        /// The input triples of the axioms whose data ranges are involved.
        source: Vec<usize>,
    },
    /// The ontology is consistent on its own, but it imports ontologies that were not
    /// loaded, so it may still be inconsistent.
    ImportsNotLoaded(Vec<Id>),
    /// The tableau built a candidate model that it could not confirm: nominals tie a part of
    /// the completion graph that blocking repeats to an individual whose number
    /// restrictions the repetitions could break (SHOIQ's NI-rule case, not implemented), or
    /// a key compares data values the tableau has not fixed.
    Unconfirmed,
}

/// The three-valued consistency answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Consistency {
    /// The ontology has a model.
    Consistent,
    /// The ontology has no model.
    Inconsistent,
    /// No verdict.
    Unknown(UnknownReason),
}

/// The class hierarchy of a consistent ontology, over named classes (`owl:Thing` and
/// `owl:Nothing` excluded).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Classification {
    /// The unsatisfiable named classes (each equivalent to `owl:Nothing`).
    pub unsatisfiable: Vec<Id>,
    /// Every entailed `A ⊑ B` between distinct satisfiable named classes, sorted.
    pub subsumptions: Vec<(Id, Id)>,
    /// The entailed `A ⊑ B` that are not asserted (as `SubClassOf(A B)` or by `A` and `B`
    /// sharing an `EquivalentClasses` axiom), sorted.
    pub inferred: Vec<(Id, Id)>,
}

/// Everything [`check`] found out about an ontology.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DlReport {
    /// The OWL 2 DL profile check.
    pub profile: DlProfileReport,
    /// Consistency of TBox plus ABox.
    pub consistency: Consistency,
    /// The classification; `None` unless the ontology was found [`Consistent`]
    /// (an inconsistent ontology entails every subsumption, and an unloaded import could add
    /// some).
    ///
    /// [`Consistent`]: Consistency::Consistent
    pub classification: Option<Classification>,
    /// Why a consistent ontology has no classification (the budget ran out), so callers
    /// can tell a missing hierarchy from an inconsistent or undecided ontology.
    pub classification_unknown: Option<UnknownReason>,
}

/// A reasoner over one ontology.
pub struct Reasoner {
    clauses: Clauses,
    index: ClauseIndex,
    budget: Budget,
    imports: Vec<Id>,
    /// The atom of each class the clauses mention, by IRI, so queries do not depend on
    /// dictionary ids (an IRI interned after construction, or never, is still recognised).
    class_iris: FxHashMap<String, u32>,
    /// The source triples of each axiom, by axiom index.
    sources: Vec<Vec<usize>>,
    /// Whether TBox plus ABox has a model, once computed.
    model: OnceCell<Result<bool, UnknownReason>>,
}

impl Reasoner {
    /// Prepares `onto` for reasoning, or says why no verdict can be given for it.
    ///
    /// # Errors
    /// [`UnknownReason::NotInDl`] or [`UnknownReason::OutOfFragment`].
    pub fn new(dict: &Dict, onto: &Ontology, budget: Budget) -> Result<Reasoner, UnknownReason> {
        Self::with_profile(dict, onto, &check_dl(dict, onto), budget)
    }

    fn with_profile(
        dict: &Dict,
        onto: &Ontology,
        profile: &DlProfileReport,
        budget: Budget,
    ) -> Result<Reasoner, UnknownReason> {
        let blocking: Vec<Violation> = profile.blocking().cloned().collect();
        if !blocking.is_empty() {
            return Err(UnknownReason::NotInDl(blocking));
        }
        let sources: Vec<Vec<usize>> = onto.axioms.iter().map(|r| r.source.clone()).collect();
        if has_ill_typed_literal(onto) {
            // No interpretation gives an ill-typed literal a value: no model at all.
            let clauses = Clauses::default();
            let index = ClauseIndex::new(&clauses);
            return Ok(Reasoner {
                clauses,
                index,
                budget,
                imports: onto.imports.clone(),
                class_iris: FxHashMap::default(),
                sources,
                model: OnceCell::from(Ok(false)),
            });
        }
        let clauses = clausify(dict, onto).map_err(|u| {
            let source = onto.axioms[u.axiom].source.clone();
            if u.datatype {
                UnknownReason::DatatypeUnknown { source }
            } else {
                UnknownReason::OutOfFragment {
                    construct: u.construct,
                    source,
                }
            }
        })?;
        let index = ClauseIndex::new(&clauses);
        let class_iris = clauses
            .classes
            .iter()
            .filter_map(|&(id, a)| iri_of(dict, id).map(|iri| (iri, a)))
            .collect();
        Ok(Reasoner {
            clauses,
            index,
            budget,
            imports: onto.imports.clone(),
            class_iris,
            sources,
            model: OnceCell::new(),
        })
    }

    /// Consistency of the whole ontology (TBox and ABox).
    pub fn consistency(&self) -> Consistency {
        match self.model() {
            Err(reason) => Consistency::Unknown(reason),
            Ok(false) => Consistency::Inconsistent,
            Ok(true) if !self.imports.is_empty() => {
                Consistency::Unknown(UnknownReason::ImportsNotLoaded(self.imports.clone()))
            }
            Ok(true) => Consistency::Consistent,
        }
    }

    /// Whether the ontology (TBox and ABox) has a model, computed once.
    fn model(&self) -> Result<bool, UnknownReason> {
        self.model
            .get_or_init(|| self.abox_run().map_err(|s| self.reason(s)))
            .clone()
    }

    fn abox_model(&self) -> Option<bool> {
        self.model().ok()
    }

    /// The reason a run stopped without a verdict.
    fn reason(&self, stop: Stop) -> UnknownReason {
        match stop {
            Stop::Exhausted => UnknownReason::ResourceBudget,
            Stop::Undecided(axioms) => {
                let mut source: Vec<usize> = axioms
                    .iter()
                    .flat_map(|&a| self.sources[a as usize].iter().copied())
                    .collect();
                source.sort_unstable();
                source.dedup();
                UnknownReason::DatatypeUnknown { source }
            }
            Stop::Unconfirmed => UnknownReason::Unconfirmed,
        }
    }

    fn abox_run(&self) -> Result<bool, Stop> {
        let mut t = Tableau::new(&self.clauses, &self.index, self.budget);
        if self.clauses.abox.individuals.is_empty() {
            t.add_root();
        }
        self.load_abox(&mut t);
        t.run()
    }

    /// Adds the ABox to `t`: individual `i` becomes root node `i`.
    fn load_abox(&self, t: &mut Tableau) {
        let abox = &self.clauses.abox;
        for ind in &abox.individuals {
            let n = t.add_root();
            if matches!(ind, Individual::Named(_)) {
                t.mark_named(n);
            }
        }
        for &(a, n, _) in &abox.concepts {
            t.assert_concept(n, a);
        }
        for &(r, a, b, _) in &abox.roles {
            t.assert_role(r, a, b);
        }
        for &(a, b, _) in &abox.different {
            t.assert_different(a, b);
        }
        let data: Vec<_> = abox
            .data
            .iter()
            .map(|(r, a, atom, v, _)| (*r, *a, *atom, v.clone()))
            .collect();
        t.assert_data(&data);
        for &(a, b, _) in &abox.same {
            t.assert_same(a, b);
        }
    }

    /// How a class IRI enters a test.
    fn concept(&self, class: &str) -> Concept {
        match class.strip_prefix(OWL) {
            Some("Thing") => Concept::Thing,
            Some("Nothing") => Concept::Nothing,
            _ => match self.class_iris.get(class) {
                Some(&a) => Concept::Atom(a),
                None => Concept::Fresh,
            },
        }
    }

    /// Whether some element can be in `class` and outside `outside`, in a model of the
    /// ontology. Without nominals the ABox cannot change that for a consistent ontology, so
    /// the test is TBox-only; with nominals the individuals and their facts come along.
    fn test(
        &self,
        class: Option<u32>,
        outside: Option<u32>,
        pool: Pool,
    ) -> Result<bool, UnknownReason> {
        let mut t = self.tableau(pool);
        let root = t.add_root();
        if let Some(b) = outside {
            t.forbid(root, b);
        }
        if let Some(a) = class {
            t.assert_concept(root, a);
        }
        let r = t.run();
        settle(pool, &t);
        r.map_err(|s| self.reason(s))
    }

    /// A tableau with the whole budget, or with what is left of a shared `pool`.
    fn tableau(&self, pool: Pool) -> Tableau<'_> {
        let mut budget = self.budget;
        if let Some(left) = pool {
            budget.max_steps = left.get();
        }
        let mut t = Tableau::new(&self.clauses, &self.index, budget);
        if !self.clauses.nominals.is_empty() {
            self.load_abox(&mut t);
        }
        t
    }

    /// `Some(true)` when the class is satisfiable, `Some(false)` when it is not, `None` when
    /// it cannot be decided: the budget ran out, consistency is unknown, or an unloaded
    /// import could make a satisfiable class unsatisfiable. An inconsistent ontology makes
    /// every class unsatisfiable.
    pub fn is_satisfiable(&self, class: &str) -> Option<bool> {
        if !self.abox_model()? {
            return Some(false);
        }
        let pool = None;
        let sat = match self.concept(class) {
            Concept::Nothing => false,
            Concept::Thing | Concept::Fresh => self.test(None, None, pool).ok()?,
            Concept::Atom(a) => self.test(Some(a), None, pool).ok()?,
        };
        (!sat || self.imports.is_empty()).then_some(sat)
    }

    /// `Some(true)` when `sub ⊑ sup` is entailed, `Some(false)` when it is not, `None` when
    /// it cannot be decided (as for [`is_satisfiable`](Self::is_satisfiable)). A class that
    /// occurs in no axiom is constrained by nothing: it is below only `owl:Thing`, itself,
    /// and what `owl:Thing` is below.
    pub fn is_subclass_of(&self, sub: &str, sup: &str) -> Option<bool> {
        if !self.abox_model()? {
            return Some(true);
        }
        let below = self
            .below(self.concept(sub), self.concept(sup), sub == sup, None)
            .ok()?;
        (below || self.imports.is_empty()).then_some(below)
    }

    /// Whether `sub ⊑ sup` holds in a consistent ontology (`same`: the same class).
    fn below(
        &self,
        sub: Concept,
        sup: Concept,
        same: bool,
        pool: Pool,
    ) -> Result<bool, UnknownReason> {
        Ok(match (sub, sup) {
            _ if same => true,
            (Concept::Nothing, _) | (_, Concept::Thing) => true,
            (sub_c, Concept::Nothing | Concept::Fresh) => match sub_c {
                Concept::Atom(a) => !self.test(Some(a), None, pool)?,
                _ => !self.test(None, None, pool)?,
            },
            (Concept::Atom(a), Concept::Atom(b)) => !self.test(Some(a), Some(b), pool)?,
            (_, Concept::Atom(b)) => !self.test(None, Some(b), pool)?,
        })
    }

    /// Classifies the named classes. `None` unless the ontology is consistent with nothing
    /// left unloaded (an unloaded import could add subsumptions), or when the budget ran out
    /// or a datatype question was undecided.
    pub fn classify(&self) -> Option<Classification> {
        if !self.imports.is_empty() || !self.abox_model()? {
            return None;
        }
        self.classify_consistent().ok()
    }

    /// Classifies a consistent ontology, or says why it could not.
    fn classify_consistent(&self) -> Result<Classification, UnknownReason> {
        let classes = &self.clauses.classes;
        let is_class = |a: u32| matches!(self.clauses.atoms[a as usize], AtomKind::Class(_));
        let mut out = Classification::default();
        let mut subs: FxHashSet<(Id, Id)> = FxHashSet::default();
        // One budget for the whole classification, not one per class.
        let left = Cell::new(self.budget.max_steps);
        let pool = Some(&left);
        for &(id, a) in classes {
            let mut t = self.tableau(pool);
            let root = t.add_root();
            t.assert_concept(root, a);
            let r = t.run();
            settle(pool, &t);
            if !r.map_err(|s| self.reason(s))? {
                out.unsatisfiable.push(id);
                continue;
            }
            // The model's root is a witness: a class absent from its label is not a
            // subsumer; one derived without any choice is.
            for (b, deterministic) in t.root_label(root) {
                if b == a || !is_class(b) {
                    continue;
                }
                let AtomKind::Class(bid) = self.clauses.atoms[b as usize] else {
                    unreachable!("checked")
                };
                let entailed =
                    deterministic || self.below(Concept::Atom(a), Concept::Atom(b), false, pool)?;
                if entailed {
                    subs.insert((id, bid));
                }
            }
        }
        let unsat: FxHashSet<Id> = out.unsatisfiable.iter().copied().collect();
        out.subsumptions = subs
            .into_iter()
            .filter(|(_, b)| !unsat.contains(b))
            .collect();
        out.subsumptions.sort_unstable();
        out.unsatisfiable.sort_unstable();
        out.inferred = out
            .subsumptions
            .iter()
            .copied()
            .filter(|p| !self.clauses.told.contains(p))
            .collect();
        Ok(out)
    }
}

/// Whether a literal the semantics must give a value (in a data property assertion, a
/// `DataHasValue` or a `DataOneOf`) is ill-typed. Facet values are not literals of the
/// interpretation, and annotations have no logical meaning.
fn has_ill_typed_literal(onto: &Ontology) -> bool {
    let ill = |l: &Literal| {
        datatypes::parse_value(&l.lexical, &l.datatype, l.lang.as_deref())
            == Err(ValueError::IllTyped)
    };
    let mut found = false;
    for rec in onto.logical_axioms() {
        if let Axiom::DataPropertyAssertion(_, _, l)
        | Axiom::NegativeDataPropertyAssertion(_, _, l) = &rec.axiom
        {
            found |= ill(l);
        }
        for_each_class_in_axiom(&rec.axiom, &mut |c| {
            if let ClassExpression::DataHasValue(_, l) = c {
                found |= ill(l);
            }
        });
        for_each_range_in_axiom(&rec.axiom, &mut |r| {
            if let DataRange::OneOf(ls) = r {
                found |= ls.iter().any(ill);
            }
        });
    }
    found
}

/// What is left of a budget shared by several tests (`None`: each test has the whole
/// budget).
type Pool<'a> = Option<&'a Cell<u64>>;

/// Charges the work of `t` to `pool`.
fn settle(pool: Pool, t: &Tableau) {
    if let Some(left) = pool {
        left.set(left.get().saturating_sub(t.work()));
    }
}

/// A class as a test sees it.
#[derive(Clone, Copy)]
enum Concept {
    Thing,
    Nothing,
    /// A class with an atom in the clauses.
    Atom(u32),
    /// A class no axiom mentions.
    Fresh,
}

/// Runs the profile check, consistency and (for a consistent ontology) classification.
pub fn check(dict: &Dict, onto: &Ontology, budget: Budget) -> DlReport {
    let profile = check_dl(dict, onto);
    let reasoner = match Reasoner::with_profile(dict, onto, &profile, budget) {
        Ok(r) => r,
        Err(reason) => {
            return DlReport {
                profile,
                consistency: Consistency::Unknown(reason),
                classification: None,
                classification_unknown: None,
            }
        }
    };
    let consistency = reasoner.consistency();
    let (classification, classification_unknown) = if consistency == Consistency::Consistent {
        match reasoner.classify_consistent() {
            Ok(c) => (Some(c), None),
            Err(reason) => (None, Some(reason)),
        }
    } else {
        (None, None)
    };
    DlReport {
        profile,
        consistency,
        classification,
        classification_unknown,
    }
}
