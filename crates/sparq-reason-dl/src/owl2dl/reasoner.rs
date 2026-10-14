//! The OWL 2 DL reasoner: consistency, class satisfiability and classification over the
//! hypertableau calculus.
//!
//! Every answer is three-valued and fail-closed. A verdict is given only when the ontology is
//! in OWL 2 DL (no [blocking](super::Violation::blocks_reasoning) profile violation) and
//! inside the fragment the clausifier translates; otherwise the answer is
//! [`Unknown`](Consistency::Unknown) with the reason, and the triples behind it when there are
//! any.
//!
//! The fragment is currently SHIQ plus the OWL 2 role characteristics and ABox: classes and
//! all class constructors except nominals (`ObjectOneOf`, `ObjectHasValue`), object
//! properties with inverses, hierarchies, transitivity, functionality, reflexivity,
//! irreflexivity, symmetry, asymmetry and disjointness, `hasSelf`, qualified cardinality, and
//! class / object-property / negative-property / same / different assertions. Data
//! properties, datatypes, nominals, property chains other than `R∘R ⊑ R`, keys, and number
//! restrictions above [`MAX_CARDINALITY`] answer `Unknown`.
//!
//! Classification is TBox-only, which is exact when the ontology is consistent and has no
//! nominals: for SHIQ the ABox cannot change a subsumption between classes.

pub use super::clausify::MAX_CARDINALITY;
use super::clausify::{clausify, AtomKind, Clauses};
use super::dl_profile::{check_dl, DlProfileReport, Violation};
use super::hypertableau::{ClauseIndex, Exhausted, Tableau};
use super::model::Ontology;
use super::vocab::Vocab;
use rustc_hash::FxHashSet;
use sparq_core::dict::{Dict, Id, NO_ID};
use std::cell::OnceCell;

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
    /// The ontology is consistent on its own, but it imports ontologies that were not
    /// loaded, so it may still be inconsistent.
    ImportsNotLoaded(Vec<Id>),
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
}

/// A reasoner over one ontology.
pub struct Reasoner {
    clauses: Clauses,
    index: ClauseIndex,
    budget: Budget,
    imports: Vec<Id>,
    thing: Id,
    nothing: Id,
    /// Whether TBox plus ABox has a model (`None`: the budget ran out), once computed.
    model: OnceCell<Option<bool>>,
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
        let clauses = clausify(dict, onto).map_err(|u| UnknownReason::OutOfFragment {
            construct: u.construct,
            source: onto.axioms[u.axiom].source.clone(),
        })?;
        let index = ClauseIndex::new(&clauses);
        let v = Vocab::intern(dict);
        Ok(Reasoner {
            clauses,
            index,
            budget,
            imports: onto.imports.clone(),
            thing: v.thing,
            nothing: v.nothing,
            model: OnceCell::new(),
        })
    }

    /// Consistency of the whole ontology (TBox and ABox).
    pub fn consistency(&self) -> Consistency {
        match self.abox_model() {
            None => Consistency::Unknown(UnknownReason::ResourceBudget),
            Some(false) => Consistency::Inconsistent,
            Some(true) if !self.imports.is_empty() => {
                Consistency::Unknown(UnknownReason::ImportsNotLoaded(self.imports.clone()))
            }
            Some(true) => Consistency::Consistent,
        }
    }

    /// Whether the ontology (TBox and ABox) has a model, computed once.
    fn abox_model(&self) -> Option<bool> {
        *self.model.get_or_init(|| self.abox_run().ok())
    }

    fn abox_run(&self) -> Result<bool, Exhausted> {
        let mut t = Tableau::new(&self.clauses, &self.index, self.budget);
        let abox = &self.clauses.abox;
        if abox.individuals.is_empty() {
            t.add_root();
        }
        for _ in &abox.individuals {
            t.add_root();
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
        for &(a, b, _) in &abox.same {
            t.assert_same(a, b);
        }
        t.run()
    }

    /// How a class IRI enters a test.
    fn concept(&self, class: Id) -> Concept {
        // A vocabulary IRI absent from the dictionary interns as `NO_ID`.
        if class == NO_ID {
            Concept::Fresh
        } else if class == self.thing {
            Concept::Thing
        } else if class == self.nothing {
            Concept::Nothing
        } else {
            match self.clauses.class_atom(class) {
                Some(a) => Concept::Atom(a),
                None => Concept::Fresh,
            }
        }
    }

    /// Whether some element can be in `class` and outside `outside` (a TBox test; the ABox
    /// cannot change it for a consistent ontology without nominals).
    fn test(&self, class: Option<u32>, outside: Option<u32>) -> Option<bool> {
        let mut t = Tableau::new(&self.clauses, &self.index, self.budget);
        let root = t.add_root();
        if let Some(b) = outside {
            t.forbid(root, b);
        }
        if let Some(a) = class {
            t.assert_concept(root, a);
        }
        t.run().ok()
    }

    /// `Some(true)` when the class is satisfiable, `Some(false)` when it is not, `None` when
    /// it cannot be decided: the budget ran out, consistency is unknown, or an unloaded
    /// import could make a satisfiable class unsatisfiable. An inconsistent ontology makes
    /// every class unsatisfiable.
    pub fn is_satisfiable(&self, class: Id) -> Option<bool> {
        if !self.abox_model()? {
            return Some(false);
        }
        let sat = match self.concept(class) {
            Concept::Nothing => false,
            Concept::Thing | Concept::Fresh => self.test(None, None)?,
            Concept::Atom(a) => self.test(Some(a), None)?,
        };
        (!sat || self.imports.is_empty()).then_some(sat)
    }

    /// `Some(true)` when `sub ⊑ sup` is entailed, `Some(false)` when it is not, `None` when
    /// it cannot be decided (as for [`is_satisfiable`](Self::is_satisfiable)). A class that
    /// occurs in no axiom is constrained by nothing: it is below only `owl:Thing`, itself,
    /// and what `owl:Thing` is below.
    pub fn is_subclass_of(&self, sub: Id, sup: Id) -> Option<bool> {
        if !self.abox_model()? {
            return Some(true);
        }
        let below = match (self.concept(sub), self.concept(sup)) {
            _ if sub == sup => true,
            (Concept::Nothing, _) | (_, Concept::Thing) => true,
            (sub_c, Concept::Nothing | Concept::Fresh) => match sub_c {
                Concept::Atom(a) => !self.test(Some(a), None)?,
                _ => !self.test(None, None)?,
            },
            (Concept::Atom(a), Concept::Atom(b)) => !self.test(Some(a), Some(b))?,
            (_, Concept::Atom(b)) => !self.test(None, Some(b))?,
        };
        (below || self.imports.is_empty()).then_some(below)
    }

    /// Classifies the named classes. `None` unless the ontology is consistent with nothing
    /// left unloaded (an unloaded import could add subsumptions), or when the budget ran out.
    pub fn classify(&self) -> Option<Classification> {
        if !self.imports.is_empty() || !self.abox_model()? {
            return None;
        }
        let classes = &self.clauses.classes;
        let is_class = |a: u32| matches!(self.clauses.atoms[a as usize], AtomKind::Class(_));
        let mut out = Classification::default();
        let mut subs: FxHashSet<(Id, Id)> = FxHashSet::default();
        for &(id, a) in classes {
            let mut t = Tableau::new(&self.clauses, &self.index, self.budget);
            let root = t.add_root();
            t.assert_concept(root, a);
            if !t.run().ok()? {
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
                let entailed = deterministic || self.is_subclass_of(id, bid)?;
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
        Some(out)
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
            }
        }
    };
    let consistency = reasoner.consistency();
    let classification = if consistency == Consistency::Consistent {
        reasoner.classify()
    } else {
        None
    };
    DlReport {
        profile,
        consistency,
        classification,
    }
}
