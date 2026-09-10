//! Characteristic-set (CS) cardinality estimation for STAR joins — the opt-in
//! `cs-planner` feature (the planner tie-in recorded in sparq-introspect's TODO).
//!
//! The greedy BGP planner's per-predicate statistics (`sparq_core::store::PredStat`)
//! are marginals: scoring a star join (several patterns sharing a subject variable,
//! predicates bound) multiplies per-predicate selectivities under an INDEPENDENCE
//! assumption. Characteristic sets (Neumann & Moerkotte, ICDE 2011) capture exactly
//! the predicate CORRELATION the marginals lose: for each distinct predicate set `C`
//! emitted by some subject, the table keeps `count(C)` (subjects whose exact set is
//! `C`) and the per-predicate average multiplicity. A star over predicate set `Q`
//! then has the (exact-on-the-data) estimates
//!
//! * subjects (the subject variable's ndv):  `Σ_{C ⊇ Q} count(C)`
//! * result cardinality:                     `Σ_{C ⊇ Q} count(C) · Π_{p∈Q} avg_mult(C, p)`
//!
//! A [`CsTable`] is built by the caller — typically from
//! `sparq_introspect::characteristic_set_ids` (the dict-id-space accessor) — and
//! installed around query execution with [`with_cs_table`]. While installed, the
//! greedy planner consults it for star-join scoring (per-candidate output estimate
//! and subject-variable ndv); everything else, and every graph without an installed
//! table, keeps the `PredStat` path. The table only influences JOIN ORDER — results
//! are identical with and without it.
//!
//! Scope: the table is consulted on the thread that runs the planner ([`with_cs_table`]
//! installs thread-locally, mirroring [`crate::QueryBudget`] installation); ids must be
//! the SAME dictionary ids as the queried graph (rebuild the table when the graph is
//! rebuilt). Stale tables degrade estimates, never correctness.

use rustc_hash::FxHashMap;
use sparq_core::dict::Id;
use std::sync::Arc;

/// One characteristic set in dictionary-id space: a distinct per-subject predicate
/// set with its subject count and per-predicate triple totals.
#[derive(Debug, Clone)]
pub struct CsSet {
    /// The predicate ids (any order; [`CsTable::new`] sorts them, keeping
    /// `predicate_triples` aligned).
    pub predicates: Box<[Id]>,
    /// Subjects whose exact predicate set this is (`count(C)`).
    pub subjects: u64,
    /// Aligned with `predicates`: total triples those subjects emit per predicate
    /// (`avg_mult(C, p) = predicate_triples[i] / subjects`).
    pub predicate_triples: Box<[u64]>,
}

/// The characteristic-set table the planner consults — see the module docs.
#[derive(Debug, Default)]
pub struct CsTable {
    /// Sets with predicates sorted ascending (so superset tests are merge walks).
    sets: Vec<CsSet>,
}

impl CsTable {
    /// Builds a table from characteristic sets, normalising each set's predicates
    /// to sorted order (with `predicate_triples` kept aligned). Sets with
    /// `subjects == 0` or mismatched `predicate_triples` length are dropped.
    pub fn new(sets: impl IntoIterator<Item = CsSet>) -> CsTable {
        let sets = sets
            .into_iter()
            .filter(|s| s.subjects > 0 && s.predicates.len() == s.predicate_triples.len())
            .map(|s| {
                if s.predicates.windows(2).all(|w| w[0] < w[1]) {
                    return s;
                }
                let mut paired: Vec<(Id, u64)> =
                    s.predicates.iter().copied().zip(s.predicate_triples.iter().copied()).collect();
                paired.sort_unstable();
                CsSet {
                    predicates: paired.iter().map(|&(p, _)| p).collect(),
                    predicate_triples: paired.iter().map(|&(_, t)| t).collect(),
                    subjects: s.subjects,
                }
            })
            .collect();
        CsTable { sets }
    }

    /// Number of characteristic sets in the table.
    pub fn len(&self) -> usize {
        self.sets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    /// `Σ_{C ⊇ Q} count(C)`: distinct subjects carrying EVERY predicate of `q`
    /// (the star's subject-variable ndv). `q` must be sorted ascending.
    pub fn star_subjects(&self, q: &[Id]) -> u64 {
        self.sets.iter().filter(|c| is_subset(q, &c.predicates)).map(|c| c.subjects).sum()
    }

    /// Neumann & Moerkotte's star estimate `Σ_{C ⊇ Q} count(C) · Π_{p∈Q} avg_mult(C, p)`:
    /// the expected result rows of the star `?s p ?o_p` for every `p ∈ q` (each
    /// subject contributes the product of its per-predicate multiplicities). `q`
    /// must be sorted ascending. Exact on the data the table was built from.
    pub fn star_cardinality(&self, q: &[Id]) -> f64 {
        let mut total = 0.0f64;
        for c in &self.sets {
            if !is_subset(q, &c.predicates) {
                continue;
            }
            let mut rows = c.subjects as f64;
            for &p in q {
                // p is present in c (subset test passed): binary-search its slot.
                let i = c.predicates.binary_search(&p).expect("subset member");
                rows *= c.predicate_triples[i] as f64 / c.subjects as f64;
            }
            total += rows;
        }
        total
    }
}

/// `a ⊆ b` over two ascending-sorted id slices (merge walk).
fn is_subset(a: &[Id], b: &[Id]) -> bool {
    let mut j = 0;
    'outer: for &x in a {
        while j < b.len() {
            match b[j].cmp(&x) {
                std::cmp::Ordering::Less => j += 1,
                std::cmp::Ordering::Equal => {
                    j += 1;
                    continue 'outer;
                }
                std::cmp::Ordering::Greater => return false,
            }
        }
        return false;
    }
    true
}

// ---- thread-local installation (mirrors exec::budget / exec::view) -------------

thread_local! {
    static ACTIVE: std::cell::RefCell<Option<Arc<CsTable>>> = const { std::cell::RefCell::new(None) };
}

/// Uninstalls the table when the [`with_cs_table`] closure returns (also on
/// unwind, so a panicking query never leaks a stale table into the thread).
pub(crate) struct Guard {
    prev: Option<Arc<CsTable>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|a| *a.borrow_mut() = self.prev.take());
    }
}

pub(crate) fn install(table: &Arc<CsTable>) -> Guard {
    let prev = ACTIVE.with(|a| a.borrow_mut().replace(Arc::clone(table)));
    Guard { prev }
}

/// The installed table, if any (an `Arc` clone — cheap).
pub(crate) fn active() -> Option<Arc<CsTable>> {
    ACTIVE.with(|a| a.borrow().clone())
}

/// Runs `f` with `table` installed as the active characteristic-set table: every
/// query the closure runs on this thread plans star joins with CS estimates.
/// Composes with [`crate::with_functions`] / [`crate::with_view`] in any nesting
/// order:
///
/// ```ignore
/// let table = Arc::new(CsTable::new(
///     sparq_introspect::characteristic_set_ids(&graph)
///         .into_iter()
///         .map(|s| CsSet { predicates: s.predicates, subjects: s.subjects, predicate_triples: s.predicate_triples }),
/// ));
/// let result = with_cs_table(&table, || query(&graph, sparql))?;
/// ```
pub fn with_cs_table<T>(table: &Arc<CsTable>, f: impl FnOnce() -> T) -> T {
    let _guard = install(table);
    f()
}

// ---- the planner-side star context ---------------------------------------------
// Built per BGP by `exec::eval_bgp_binary` (and the EXPLAIN replay); tracks, per
// subject variable, the bound predicates of the patterns joined SO FAR, so a
// candidate's CS estimate conditions on exactly the star joined to date.

use oxrdf::Variable;

pub(crate) struct StarCtx {
    table: Arc<CsTable>,
    /// Per pattern: `Some((subject var, predicate id))` when the pattern has the
    /// star shape `?s <p> ?o` (subject variable, bound predicate, unbound object).
    pat_star: Vec<Option<(Variable, Id)>>,
    /// Subject var -> sorted predicate ids of the star patterns already joined.
    joined: FxHashMap<Variable, Vec<Id>>,
}

impl StarCtx {
    pub(crate) fn new(
        table: Arc<CsTable>,
        pats: impl Iterator<Item = Option<(Variable, Id)>>,
    ) -> StarCtx {
        StarCtx { table, pat_star: pats.collect(), joined: FxHashMap::default() }
    }

    /// Marks pattern `i` joined: its predicate now conditions later star estimates.
    pub(crate) fn note_done(&mut self, i: usize) {
        if let Some((v, p)) = &self.pat_star[i] {
            let preds = self.joined.entry(v.clone()).or_default();
            if let Err(pos) = preds.binary_search(p) {
                preds.insert(pos, *p);
            }
        }
    }

    /// CS-based estimated output of joining candidate `i` onto a running result of
    /// `cur_card` rows: `cur_card · star_card(Q ∪ {p_i}) / star_card(Q)` where `Q`
    /// is the star joined so far on the candidate's subject variable. `None` when
    /// the candidate is not a star pattern, nothing is joined on its subject var
    /// yet, or the table has no covering set (fall back to `PredStat`).
    pub(crate) fn pick_score(&self, i: usize, cur_card: f64) -> Option<f64> {
        let (v, p) = self.pat_star[i].as_ref()?;
        let q = self.joined.get(v).filter(|q| !q.is_empty())?;
        if q.binary_search(p).is_ok() {
            // Same predicate repeated in the star (e.g. `?s :p ?a . ?s :p ?b`):
            // the CS multiplicity model conditions on the predicate already being
            // joined — fall back to the marginal path.
            return None;
        }
        let card_q = self.table.star_cardinality(q);
        if card_q <= 0.0 {
            // No subject carries all of Q: the data says the star is empty.
            return Some(0.0);
        }
        let mut q_new = q.clone();
        let pos = q_new.binary_search(p).unwrap_err();
        q_new.insert(pos, *p);
        Some(cur_card * (self.table.star_cardinality(&q_new) / card_q))
    }

    /// CS-based distinct-subject estimate for pattern `i`'s subject variable, given
    /// every star predicate joined so far on it (call AFTER [`note_done`]): the
    /// Neumann & Moerkotte subject-var ndv `Σ_{C ⊇ Q} count(C)`. `None` when the
    /// pattern is not a star pattern or the table has no covering set.
    pub(crate) fn subject_ndv(&self, i: usize) -> Option<f64> {
        let (v, _) = self.pat_star[i].as_ref()?;
        let q = self.joined.get(v).filter(|q| !q.is_empty())?;
        let subjects = self.table.star_subjects(q);
        (subjects > 0).then_some(subjects as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(preds: &[Id], subjects: u64, triples: &[u64]) -> CsSet {
        CsSet { predicates: preds.into(), subjects, predicate_triples: triples.into() }
    }

    #[test]
    fn star_estimates() {
        // Two entity shapes: 100 subjects {1,2} (p1 avg 1x, p2 avg 2x),
        // 50 subjects {1,3} (p1 avg 1x, p3 avg 1x).
        let t = CsTable::new([set(&[1, 2], 100, &[100, 200]), set(&[1, 3], 50, &[50, 50])]);
        assert_eq!(t.star_subjects(&[1]), 150);
        assert_eq!(t.star_subjects(&[1, 2]), 100);
        assert_eq!(t.star_subjects(&[2, 3]), 0, "no subject has both p2 and p3");
        assert_eq!(t.star_cardinality(&[1]), 150.0);
        assert_eq!(t.star_cardinality(&[1, 2]), 200.0, "100 subjects x 1 x 2");
        assert_eq!(t.star_cardinality(&[2, 3]), 0.0);
    }

    #[test]
    fn new_normalises_unsorted_and_drops_invalid() {
        let t = CsTable::new([
            set(&[3, 1], 10, &[30, 10]), // unsorted: must re-align triples with preds
            set(&[5], 0, &[7]),          // zero subjects: dropped
            set(&[5, 6], 4, &[1]),       // length mismatch: dropped
        ]);
        assert_eq!(t.len(), 1);
        assert_eq!(t.star_cardinality(&[1]), 10.0, "p1 avg_mult is 1 after re-alignment");
        assert_eq!(t.star_cardinality(&[3]), 30.0, "p3 avg_mult is 3 after re-alignment");
    }

    #[test]
    fn install_scoping_restores_previous() {
        let a = Arc::new(CsTable::new([set(&[1], 1, &[1])]));
        let b = Arc::new(CsTable::new([set(&[2], 2, &[2])]));
        assert!(active().is_none());
        with_cs_table(&a, || {
            assert_eq!(active().unwrap().len(), 1);
            with_cs_table(&b, || assert_eq!(active().unwrap().star_subjects(&[2]), 2));
            // Inner uninstall restores the OUTER table, not none.
            assert_eq!(active().unwrap().star_subjects(&[1]), 1);
        });
        assert!(active().is_none());
    }
}
