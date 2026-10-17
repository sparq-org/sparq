//! Rechecking an ontology across edits: [`IncrementalChecker`] keeps what the last check
//! decided and reuses it when the next ontology has the same axioms, or more of them.
//!
//! OWL 2 is monotone, so an ontology that only adds axioms to one whose answer was decided
//! keeps every entailment: an inconsistent ontology stays inconsistent, and a consistent
//! one keeps its unsatisfiable classes and subsumptions, which the new classification then
//! takes without a test (it still tests everything else). Any removed or changed axiom
//! means a full check. Axioms are compared structurally, by the terms they name, so the two
//! ontologies may be read into different dictionaries; an anonymous individual is compared
//! by its blank-node label, so relabelled blank nodes count as a change. The profile check
//! and the translation into clauses run every time (a refusal is never skipped), and only a
//! decided answer is kept: an `Unknown` one is recomputed.

use super::dl_profile::check_dl;
use super::model::{Axiom, MapIds, Ontology};
use super::reasoner::{Budget, Classification, Consistency, DlReport, Known, Reasoner};
use rustc_hash::FxHashSet;
use sparq_core::dict::{Dict, Id, NO_ID};

/// What a check reused from the previous one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reuse {
    /// Nothing: the first check, an axiom removed or changed, or no decided answer kept.
    None,
    /// The same axioms: the previous verdict and classification, without reasoning.
    Identical,
    /// A superset of the previous axioms: the previous inconsistency, or the previous
    /// unsatisfiable classes and subsumptions as known entailments.
    Extension,
}

/// Rechecks ontologies, reusing the last decided answer (see the module documentation).
#[derive(Default)]
pub struct IncrementalChecker {
    /// The terms of the kept answer, independent of each check's dictionary.
    dict: Dict,
    last: Option<Last>,
}

struct Last {
    axioms: FxHashSet<Axiom>,
    imports: FxHashSet<Id>,
    decided: Decided,
}

/// A decided answer, in [`IncrementalChecker::dict`]'s ids.
enum Decided {
    Inconsistent,
    Classified(Classification),
}

impl IncrementalChecker {
    #[must_use]
    pub fn new() -> IncrementalChecker {
        IncrementalChecker::default()
    }

    /// Checks `onto` as [`super::check`] does, reusing the previous check where its axioms
    /// allow, and says what was reused.
    pub fn check(&mut self, dict: &Dict, onto: &Ontology, budget: Budget) -> (DlReport, Reuse) {
        let own = &mut self.dict;
        let mut ours = |id: Id| own.intern(&dict.term(id));
        let axioms: FxHashSet<Axiom> = onto
            .axioms
            .iter()
            .filter(|r| r.axiom.is_logical() || matches!(r.axiom, Axiom::Declaration(..)))
            .map(|r| r.axiom.map_ids(&mut ours))
            .collect();
        let imports: FxHashSet<Id> = onto.imports.iter().map(|&i| ours(i)).collect();
        let profile = check_dl(dict, onto);
        let reasoner = match Reasoner::with_profile(dict, onto, &profile, budget) {
            Ok(r) => r,
            Err(reason) => {
                let report = DlReport {
                    profile,
                    consistency: Consistency::Unknown(reason),
                    classification: None,
                    classification_unknown: None,
                };
                return (report, Reuse::None);
            }
        };
        // A refused version keeps the kept answer for the next one.
        let last = self.last.take();
        let reuse = match &last {
            Some(l) if l.axioms == axioms && l.imports == imports => Reuse::Identical,
            Some(l) if l.axioms.is_subset(&axioms) && l.imports.is_subset(&imports) => {
                Reuse::Extension
            }
            _ => Reuse::None,
        };
        let decided = last.filter(|_| reuse != Reuse::None).map(|l| l.decided);
        let theirs = |own: &Dict, id: Id| dict.lookup(&own.term(id));
        let (consistency, classification, classification_unknown) = match (reuse, decided) {
            (_, Some(Decided::Inconsistent)) => (Consistency::Inconsistent, None, None),
            (Reuse::Identical, Some(Decided::Classified(c))) => {
                let c = remap(&c, &mut |id| theirs(&self.dict, id));
                (Consistency::Consistent, Some(c), None)
            }
            (_, decided) => {
                let known = match decided {
                    Some(Decided::Classified(c)) => Known {
                        unsatisfiable: c
                            .unsatisfiable
                            .iter()
                            .map(|&i| theirs(&self.dict, i))
                            .collect(),
                        subsumptions: c
                            .subsumptions
                            .iter()
                            .map(|&(a, b)| (theirs(&self.dict, a), theirs(&self.dict, b)))
                            .collect(),
                    },
                    _ => Known::default(),
                };
                let consistency = reasoner.consistency();
                if consistency == Consistency::Consistent {
                    match reasoner.classify_consistent(&known) {
                        Ok(c) => (consistency, Some(c), None),
                        Err(reason) => (consistency, None, Some(reason)),
                    }
                } else {
                    (consistency, None, None)
                }
            }
        };
        let kept = match (&consistency, &classification) {
            (Consistency::Inconsistent, _) => Some(Decided::Inconsistent),
            (Consistency::Consistent, Some(c)) => {
                let own = &mut self.dict;
                Some(Decided::Classified(remap(c, &mut |id| {
                    own.intern(&dict.term(id))
                })))
            }
            _ => None,
        };
        self.last = kept.map(|decided| Last {
            axioms,
            imports,
            decided,
        });
        let report = DlReport {
            profile,
            consistency,
            classification,
            classification_unknown,
        };
        (report, reuse)
    }
}

/// `c` with its class ids mapped by `f` (and sorted again). An id `f` cannot map (`NO_ID`)
/// drops its entries.
fn remap(c: &Classification, f: &mut impl FnMut(Id) -> Id) -> Classification {
    let mut unsatisfiable: Vec<Id> = c
        .unsatisfiable
        .iter()
        .map(|&i| f(i))
        .filter(|&i| i != NO_ID)
        .collect();
    unsatisfiable.sort_unstable();
    let mut pairs = |ps: &[(Id, Id)]| {
        let mut v: Vec<(Id, Id)> = ps
            .iter()
            .map(|&(a, b)| (f(a), f(b)))
            .filter(|&(a, b)| a != NO_ID && b != NO_ID)
            .collect();
        v.sort_unstable();
        v
    };
    let subsumptions = pairs(&c.subsumptions);
    let inferred = pairs(&c.inferred);
    Classification {
        unsatisfiable,
        subsumptions,
        inferred,
    }
}
