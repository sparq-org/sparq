//! sq-4foq0 (design record `research/stratified-datalog-rules.md` §6.3) —
//! **incremental maintenance under insert/delete across strata**: DRed
//! (delete-and-rederive, Gupta–Mumick–Subrahmanian, SIGMOD 1993) for positive
//! strata plus rederivation at stratum boundaries for the non-monotonic ones.
//!
//! [`MaterializedProgram`] keeps a stratified program's full materialization live
//! across base-fact inserts and deletes, without re-running the whole fixpoint:
//!
//! * **State.** The asserted `base` set plus one `derived` set per stratum, kept
//!   DISJOINT from everything visible below it (`derived[s] ∩ below[s] = ∅`, where
//!   `below[s] = base ∪ derived[0..s]`) — so the closure is the disjoint union
//!   and every fact has exactly one owner, the lowest layer that supports it.
//! * **Change propagation.** An update walks strata in order carrying the exact
//!   `(added, removed)` delta to each stratum's visible input. A stratum whose
//!   rules read none of the changed predicates (and, for removals, whose rules
//!   cannot re-own a removed fact — its HEAD predicates) is **skipped outright**:
//!   its derivations are a function of its input, which did not change
//!   relevantly. The walk stops early once the delta drains to nothing.
//! * **Positive strata → DRed.** Overdelete (semi-naive over the deletion delta,
//!   against the pre-update store — an over-approximation), prune, then
//!   **rederive**: one full pass over the reduced store reinstates every
//!   overdeleted fact with an alternative derivation, and its semi-naive
//!   continuation chases reinstatement chains. Insertions then propagate
//!   semi-naive with the inserted facts as the round-0 delta. Rederivation
//!   candidates include the removed base facts themselves — a fact deleted from
//!   `base` but still derivable stays in the closure, it just changes owner
//!   (moves into `derived`).
//! * **Non-monotonic strata → stratum-boundary rederivation.** A stratum
//!   carrying `NOT` or `AGGREGATE` rules whose input changed is re-derived from
//!   its (complete, already-maintained) input — deletions can CREATE derivations
//!   here and insertions can KILL them, so delta-restricted firing is unsound;
//!   the stratification guarantee makes recomputation from the maintained lower
//!   layers exact. The new derived set is diffed against the old one, so higher
//!   strata still see a minimal delta.
//! * **Variable-predicate relevance.** A variable-predicate read is
//!   marked `read_any`, and a variable-predicate head is marked `head_any`.
//!   Therefore no changed predicate can be skipped at that stratum (including
//!   removal/re-ownership through a dynamic head), mirroring the stratifier's
//!   conservative top node. Grouped `NOT` and projected DISTINCT remain boundary
//!   recomputations, so each maintained layer stays set-equal to from-scratch eval.
//!
//! # Honest scope (v1)
//!
//! * Per-AFFECTED-stratum set/index bookkeeping is `O(|visible input|)` (fresh
//!   `FactStore` indexes per update; no persistent deletable index yet). The
//!   incrementality claim is about RULE-FIRING work — measured by the
//!   deterministic `tuples_considered` counter, asserted smaller than
//!   from-scratch in the test suite — not about set bookkeeping.
//! * DRed over-deletes and rederives; FBF-style backward-chaining limits on
//!   over-deletion belong to the deletion-heavy benchmark program (sq-6tykl.4),
//!   which must precede any such optimization here (profile first).
//! * No counting variant: counts are unsound under recursion without derivation
//!   depth tracking, and the profile-first mandate wants the benchmark
//!   (sq-6tykl.4) before a nonrecursive-stratum fast path is added.

use super::eval::{run_rule, run_stratum, EvalStats};
use super::{stratify, FactStore, Program, Rule, Stratification};
use rustc_hash::FxHashSet;
use sparq_core::dict::{Dict, Id};

/// Deterministic per-update work counters (test/bench instrumentation): how many
/// strata took each maintenance path, DRed's overdeletion/rederivation volume,
/// and the shared rule-firing counters ([`EvalStats`] — `tuples_considered` is
/// the honest cross-mode work measure; NO wall-clock).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UpdateStats {
    /// Strata whose visible input did not change relevantly (skipped outright).
    pub(crate) skipped_strata: usize,
    /// Positive strata maintained by DRed.
    pub(crate) dred_strata: usize,
    /// Non-monotonic strata re-derived from their maintained input.
    pub(crate) recomputed_strata: usize,
    /// Facts the DRed overdeletion pass retracted (before rederivation).
    pub(crate) overdeleted: u64,
    /// Overdeleted facts reinstated by the rederivation pass.
    pub(crate) rederived: u64,
    /// Rule-firing counters, summed across every phase of the update.
    pub(crate) eval: EvalStats,
}

/// A live materialization of a stratified-Datalog [`Program`] over a base-fact
/// set, incrementally maintained under [`insert`](MaterializedProgram::insert) /
/// [`delete`](MaterializedProgram::delete) (design record
/// `research/stratified-datalog-rules.md` §6.3: DRed for positive strata,
/// rederivation at stratum boundaries for `NOT`/`AGGREGATE` strata). The closure
/// it maintains is always identical, as a set, to a from-scratch
/// [`eval`](super::eval()) over the current base — the invariant the
/// differential suite pins on randomized insert/delete sequences.
///
/// # Examples
///
/// ```
/// use sparq_reason::datalog::{parse_program, MaterializedProgram};
/// let mut dict = sparq_core::dict::Dict::new();
/// let p = parse_program(
///     &mut dict,
///     "@prefix ex: <http://ex/> . [?x, ex:q, ?y] :- [?x, ex:p, ?y] .",
/// )?;
/// let (s, pr, o) = (
///     dict.intern_iri("http://ex/s"),
///     dict.intern_iri("http://ex/p"),
///     dict.intern_iri("http://ex/o"),
/// );
/// let mut m = MaterializedProgram::new(&mut dict, &[[s, pr, o]], p)?;
/// assert_eq!(m.len(), 2); // the fact + its derivation
/// m.delete(&mut dict, &[[s, pr, o]]);
/// assert_eq!(m.len(), 0); // the derivation retracts with its support
/// # Ok::<(), String>(())
/// ```
pub struct MaterializedProgram {
    program: Program,
    strat: Stratification,
    /// Asserted facts (the EDB).
    base: FxHashSet<[Id; 3]>,
    /// Per-stratum derived facts, disjoint from `base` and every lower stratum.
    derived: Vec<FxHashSet<[Id; 3]>>,
    /// Rule indices per stratum.
    stratum_rules: Vec<Vec<usize>>,
    /// Per stratum: predicates its rules READ (positive + `NOT` + aggregate bodies).
    read_preds: Vec<FxHashSet<Id>>,
    /// Per stratum: a variable-predicate body reads every predicate.
    read_any: Vec<bool>,
    /// Per stratum: predicates its rules DERIVE (head predicates).
    head_preds: Vec<FxHashSet<Id>>,
    /// Per stratum: a variable-predicate head may derive every predicate.
    head_any: Vec<bool>,
    /// Per stratum: no rule carries `NOT`/`AGGREGATE` (DRed-eligible).
    positive_only: Vec<bool>,
}

impl MaterializedProgram {
    /// Check stratifiability, run the initial materialization and return the
    /// live handle. `dict` is mutable because aggregation mints numeric
    /// literals into the caller's id space (here and on every update).
    ///
    /// # Errors
    ///
    /// Returns `Err` (via [`stratify`]) when the program has a cycle through
    /// negation or aggregation — no stratified model exists.
    ///
    /// # Examples
    ///
    /// ```
    /// use sparq_reason::datalog::{parse_program, MaterializedProgram};
    /// let mut dict = sparq_core::dict::Dict::new();
    /// let p = parse_program(
    ///     &mut dict,
    ///     "@prefix ex: <http://ex/> . [?x, ex:q, ?y] :- [?x, ex:p, ?y] .",
    /// )?;
    /// let m = MaterializedProgram::new(&mut dict, &[], p)?;
    /// assert!(m.is_empty());
    /// # Ok::<(), String>(())
    /// ```
    pub fn new(dict: &mut Dict, facts: &[[Id; 3]], program: Program) -> Result<Self, String> {
        let strat = stratify(dict, &program)?;
        let n = strat.n_strata();
        let mut stratum_rules: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut read_preds: Vec<FxHashSet<Id>> = vec![FxHashSet::default(); n];
        let mut read_any = vec![false; n];
        let mut head_preds: Vec<FxHashSet<Id>> = vec![FxHashSet::default(); n];
        let mut head_any = vec![false; n];
        let mut positive_only = vec![true; n];
        for (i, (rule, &s)) in program.rules.iter().zip(&strat.rule_stratum).enumerate() {
            stratum_rules[s].push(i);
            for a in &rule.positive {
                if let Some(pred) = a.pred {
                    read_preds[s].insert(pred);
                } else {
                    read_any[s] = true;
                }
            }
            for a in rule.negated.iter().flatten() {
                if let Some(pred) = a.pred {
                    read_preds[s].insert(pred);
                } else {
                    read_any[s] = true;
                }
            }
            for agg in &rule.aggregates {
                for a in &agg.body {
                    if let Some(pred) = a.pred {
                        read_preds[s].insert(pred);
                    } else {
                        read_any[s] = true;
                    }
                }
            }
            for a in &rule.head {
                if let Some(pred) = a.pred {
                    head_preds[s].insert(pred);
                } else {
                    head_any[s] = true;
                }
            }
            if !rule.negated.is_empty() || !rule.aggregates.is_empty() {
                positive_only[s] = false;
            }
        }
        let base: FxHashSet<[Id; 3]> = facts.iter().copied().collect();
        // Initial build: the plain per-stratum fixpoint, slicing each stratum's
        // derivations out of the store's append-only list.
        let mut store = FactStore::default();
        for f in &base {
            store.insert(*f);
        }
        let mut stats = EvalStats::default();
        let mut derived: Vec<FxHashSet<[Id; 3]>> = Vec::with_capacity(n);
        for rule_ixs in &stratum_rules {
            let rules: Vec<&Rule> = rule_ixs.iter().map(|&i| &program.rules[i]).collect();
            let before = store.list.len();
            run_stratum(dict, &rules, &mut store, &mut stats, false);
            derived.push(store.list[before..].iter().copied().collect());
        }
        Ok(Self {
            program,
            strat,
            base,
            derived,
            stratum_rules,
            read_preds,
            read_any,
            head_preds,
            head_any,
            positive_only,
        })
    }

    /// Assert `facts` and incrementally extend the materialization. Returns the
    /// number of facts the CLOSURE gained (already-asserted or already-derived
    /// inputs add nothing; their consequences are already present).
    ///
    /// # Examples
    ///
    /// ```
    /// use sparq_reason::datalog::{parse_program, MaterializedProgram};
    /// let mut dict = sparq_core::dict::Dict::new();
    /// let p = parse_program(
    ///     &mut dict,
    ///     "@prefix ex: <http://ex/> . [?x, ex:q, ?y] :- [?x, ex:p, ?y] .",
    /// )?;
    /// let (s, pr, o) = (
    ///     dict.intern_iri("http://ex/s"),
    ///     dict.intern_iri("http://ex/p"),
    ///     dict.intern_iri("http://ex/o"),
    /// );
    /// let mut m = MaterializedProgram::new(&mut dict, &[], p)?;
    /// assert_eq!(m.insert(&mut dict, &[[s, pr, o]]), 2);
    /// # Ok::<(), String>(())
    /// ```
    pub fn insert(&mut self, dict: &mut Dict, facts: &[[Id; 3]]) -> usize {
        self.update(dict, facts, &[]).0
    }

    /// Retract asserted facts and incrementally maintain the materialization.
    /// Returns the number of facts the CLOSURE lost. Deleting a fact that was
    /// never asserted is a no-op — in particular, a DERIVED fact cannot be
    /// deleted away while its derivation stands, and a deleted base fact that
    /// is still derivable stays in the closure.
    ///
    /// # Examples
    ///
    /// ```
    /// use sparq_reason::datalog::{parse_program, MaterializedProgram};
    /// let mut dict = sparq_core::dict::Dict::new();
    /// let p = parse_program(
    ///     &mut dict,
    ///     "@prefix ex: <http://ex/> . [?x, ex:q, ?y] :- [?x, ex:p, ?y] .",
    /// )?;
    /// let (s, pr, o) = (
    ///     dict.intern_iri("http://ex/s"),
    ///     dict.intern_iri("http://ex/p"),
    ///     dict.intern_iri("http://ex/o"),
    /// );
    /// let mut m = MaterializedProgram::new(&mut dict, &[[s, pr, o]], p)?;
    /// assert_eq!(m.delete(&mut dict, &[[s, pr, o]]), 2); // fact + derivation
    /// assert_eq!(m.delete(&mut dict, &[[s, pr, o]]), 0); // no-op: not asserted
    /// # Ok::<(), String>(())
    /// ```
    pub fn delete(&mut self, dict: &mut Dict, facts: &[[Id; 3]]) -> usize {
        self.update(dict, &[], facts).1
    }

    /// Apply one batched update — `new base = (base \ deletes) ∪ inserts` (a
    /// fact in both survives) — and return `(closure gained, closure lost)`.
    /// One batch is cheaper than sequential [`insert`](Self::insert) +
    /// [`delete`](Self::delete) calls: each stratum is maintained once.
    ///
    /// # Examples
    ///
    /// ```
    /// use sparq_reason::datalog::{parse_program, MaterializedProgram};
    /// let mut dict = sparq_core::dict::Dict::new();
    /// let p = parse_program(
    ///     &mut dict,
    ///     "@prefix ex: <http://ex/> . [?x, ex:q, ?y] :- [?x, ex:p, ?y] .",
    /// )?;
    /// let (s, s2, pr, o) = (
    ///     dict.intern_iri("http://ex/s"),
    ///     dict.intern_iri("http://ex/s2"),
    ///     dict.intern_iri("http://ex/p"),
    ///     dict.intern_iri("http://ex/o"),
    /// );
    /// let mut m = MaterializedProgram::new(&mut dict, &[[s, pr, o]], p)?;
    /// assert_eq!(m.update(&mut dict, &[[s2, pr, o]], &[[s, pr, o]]), (2, 2));
    /// # Ok::<(), String>(())
    /// ```
    pub fn update(
        &mut self,
        dict: &mut Dict,
        inserts: &[[Id; 3]],
        deletes: &[[Id; 3]],
    ) -> (usize, usize) {
        let mut stats = UpdateStats::default();
        self.update_with_stats(dict, inserts, deletes, &mut stats)
    }

    /// Is `f` in the maintained closure (asserted or derived)?
    ///
    /// # Examples
    ///
    /// ```
    /// use sparq_reason::datalog::{parse_program, MaterializedProgram};
    /// let mut dict = sparq_core::dict::Dict::new();
    /// let p = parse_program(
    ///     &mut dict,
    ///     "@prefix ex: <http://ex/> . [?x, ex:q, ?y] :- [?x, ex:p, ?y] .",
    /// )?;
    /// let (s, pr, q, o) = (
    ///     dict.intern_iri("http://ex/s"),
    ///     dict.intern_iri("http://ex/p"),
    ///     dict.intern_iri("http://ex/q"),
    ///     dict.intern_iri("http://ex/o"),
    /// );
    /// let m = MaterializedProgram::new(&mut dict, &[[s, pr, o]], p)?;
    /// assert!(m.contains(&[s, q, o])); // derived
    /// # Ok::<(), String>(())
    /// ```
    pub fn contains(&self, f: &[Id; 3]) -> bool {
        self.base.contains(f) || self.derived.iter().any(|d| d.contains(f))
    }

    /// The maintained closure — asserted facts plus every derivation, as a
    /// duplicate-free `Vec` (treat as a SET: order is unspecified).
    ///
    /// # Examples
    ///
    /// ```
    /// use sparq_reason::datalog::{parse_program, MaterializedProgram};
    /// let mut dict = sparq_core::dict::Dict::new();
    /// let p = parse_program(
    ///     &mut dict,
    ///     "@prefix ex: <http://ex/> . [?x, ex:q, ?y] :- [?x, ex:p, ?y] .",
    /// )?;
    /// let (s, pr, o) = (
    ///     dict.intern_iri("http://ex/s"),
    ///     dict.intern_iri("http://ex/p"),
    ///     dict.intern_iri("http://ex/o"),
    /// );
    /// let m = MaterializedProgram::new(&mut dict, &[[s, pr, o]], p)?;
    /// assert_eq!(m.closure().len(), 2);
    /// # Ok::<(), String>(())
    /// ```
    pub fn closure(&self) -> Vec<[Id; 3]> {
        let mut out = Vec::with_capacity(self.len());
        out.extend(self.base.iter().copied());
        for d in &self.derived {
            out.extend(d.iter().copied());
        }
        out
    }

    /// Number of facts in the maintained closure (asserted + derived; the
    /// layers are disjoint, so this is an `O(strata)` sum).
    ///
    /// # Examples
    ///
    /// ```
    /// use sparq_reason::datalog::{parse_program, MaterializedProgram};
    /// let mut dict = sparq_core::dict::Dict::new();
    /// let p = parse_program(
    ///     &mut dict,
    ///     "@prefix ex: <http://ex/> . [?x, ex:q, ?y] :- [?x, ex:p, ?y] .",
    /// )?;
    /// assert_eq!(MaterializedProgram::new(&mut dict, &[], p)?.len(), 0);
    /// # Ok::<(), String>(())
    /// ```
    pub fn len(&self) -> usize {
        self.base.len() + self.derived.iter().map(|d| d.len()).sum::<usize>()
    }

    /// Is the maintained closure empty?
    ///
    /// # Examples
    ///
    /// ```
    /// use sparq_reason::datalog::{parse_program, MaterializedProgram};
    /// let mut dict = sparq_core::dict::Dict::new();
    /// let p = parse_program(
    ///     &mut dict,
    ///     "@prefix ex: <http://ex/> . [?x, ex:q, ?y] :- [?x, ex:p, ?y] .",
    /// )?;
    /// assert!(MaterializedProgram::new(&mut dict, &[], p)?.is_empty());
    /// # Ok::<(), String>(())
    /// ```
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The instrumented engine behind [`update`](Self::update) (and so behind
    /// `insert`/`delete`): walks strata in order carrying the exact visible-input
    /// delta, choosing skip / DRed / recompute per stratum. See the module docs
    /// for the algorithm and the disjoint-ownership invariant.
    pub(crate) fn update_with_stats(
        &mut self,
        dict: &mut Dict,
        inserts: &[[Id; 3]],
        deletes: &[[Id; 3]],
        stats: &mut UpdateStats,
    ) -> (usize, usize) {
        // Batch semantics: new base = (base \ deletes) ∪ inserts. Deterministic
        // caller-order deltas, deduplicated.
        let ins_set: FxHashSet<[Id; 3]> = inserts.iter().copied().collect();
        let mut seen: FxHashSet<[Id; 3]> = FxHashSet::default();
        let mut rems: Vec<[Id; 3]> = deletes
            .iter()
            .filter(|f| self.base.contains(*f) && !ins_set.contains(*f) && seen.insert(**f))
            .copied()
            .collect();
        seen.clear();
        let mut adds: Vec<[Id; 3]> = inserts
            .iter()
            .filter(|f| !self.base.contains(*f) && seen.insert(**f))
            .copied()
            .collect();
        if adds.is_empty() && rems.is_empty() {
            return (0, 0);
        }
        // Running visible sets BELOW the stratum being maintained, old and new.
        let mut below_old = self.base.clone();
        for f in &rems {
            self.base.remove(f);
        }
        for f in &adds {
            self.base.insert(*f);
        }
        let mut below_new = self.base.clone();

        for s in 0..self.strat.n_strata() {
            if adds.is_empty() && rems.is_empty() {
                break; // input delta drained: nothing above can change
            }
            let derived_old = &self.derived[s];
            // A stratum is affected when a changed fact's predicate is READ by
            // one of its rules, or when a REMOVED fact could be re-owned by one
            // of its rules (its predicate is a head predicate here — the fact
            // may be derivable at this stratum and must then stay visible).
            let affected = adds
                .iter()
                .chain(&rems)
                .any(|f| self.read_any[s] || self.read_preds[s].contains(&f[1]))
                || rems
                    .iter()
                    .any(|f| self.head_any[s] || self.head_preds[s].contains(&f[1]));
            let derived_new: FxHashSet<[Id; 3]> = if !affected {
                stats.skipped_strata += 1;
                // Derivations are unchanged; only ownership bookkeeping: an
                // added base fact this stratum used to own moves down to `base`.
                let mut d = derived_old.clone();
                for f in &adds {
                    d.remove(f);
                }
                d
            } else if self.positive_only[s] {
                stats.dred_strata += 1;
                self.dred_stratum(dict, s, &below_old, &below_new, &adds, &rems, stats)
            } else {
                stats.recomputed_strata += 1;
                // Stratum-boundary rederivation: NOT/AGGREGATE make firing
                // non-monotonic in the input, so recompute from the maintained
                // (complete) input and diff.
                let mut store = FactStore::default();
                for f in &below_new {
                    store.insert(*f);
                }
                let rules: Vec<&Rule> = self.stratum_rules[s]
                    .iter()
                    .map(|&i| &self.program.rules[i])
                    .collect();
                let before = store.list.len();
                run_stratum(dict, &rules, &mut store, &mut stats.eval, false);
                store.list[before..].iter().copied().collect()
            };
            let derived_old = &self.derived[s]; // reborrow after the &mut self call
                                                // Propagate the delta across the stratum boundary. `below'` gains
                                                // this stratum's derived layer, so facts that merely changed OWNER
                                                // (base ↔ derived) drop out of the delta here.
            let mut next_adds: Vec<[Id; 3]> = adds
                .iter()
                .filter(|f| !derived_old.contains(*f))
                .copied()
                .collect();
            next_adds.extend(
                derived_new
                    .iter()
                    .filter(|f| !derived_old.contains(*f) && !below_old.contains(*f)),
            );
            let mut next_rems: Vec<[Id; 3]> = rems
                .iter()
                .filter(|f| !derived_new.contains(*f))
                .copied()
                .collect();
            next_rems.extend(
                derived_old
                    .iter()
                    .filter(|f| !derived_new.contains(*f) && !below_new.contains(*f)),
            );
            below_old.extend(derived_old.iter().copied());
            below_new.extend(derived_new.iter().copied());
            self.derived[s] = derived_new;
            adds = next_adds;
            rems = next_rems;
        }
        // After the last stratum, `below'` is the FULL closure on both sides, so
        // the propagated deltas are exactly `closure_new \ closure_old` and
        // `closure_old \ closure_new` (deduplicated by construction). An early
        // break leaves both empty — the visible set was already unchanged.
        (adds.len(), rems.len())
    }

    /// DRed for one POSITIVE stratum (no `NOT`/`AGGREGATE`; `FILTER` is
    /// row-local, hence monotone in the store): overdelete → prune → rederive →
    /// insert-propagate. Returns the stratum's new derived layer, disjoint from
    /// `below_new`.
    #[allow(clippy::too_many_arguments)]
    fn dred_stratum(
        &self,
        dict: &Dict,
        s: usize,
        below_old: &FxHashSet<[Id; 3]>,
        below_new: &FxHashSet<[Id; 3]>,
        adds: &[[Id; 3]],
        rems: &[[Id; 3]],
        stats: &mut UpdateStats,
    ) -> FxHashSet<[Id; 3]> {
        let derived_old = &self.derived[s];
        let rules: Vec<&Rule> = self.stratum_rules[s]
            .iter()
            .map(|&i| &self.program.rules[i])
            .collect();
        // --- 1. Overdeletion: semi-naive over the deletion delta against the
        // PRE-update store (the standard over-approximation). Seeds are the
        // removed input facts; every one-step consequence THIS stratum owns is
        // overdeleted and becomes the next delta. `emit_known = true`: the
        // heads we hunt are (still) in the store.
        let mut v_old = FactStore::default();
        for f in below_old.iter().chain(derived_old) {
            v_old.insert(*f);
        }
        let mut over: FxHashSet<[Id; 3]> = rems.iter().copied().collect();
        let mut delta: Vec<[Id; 3]> = rems.to_vec();
        while !delta.is_empty() {
            let mut produced: Vec<[Id; 3]> = Vec::new();
            for r in &rules {
                for k in 0..r.positive.len() {
                    run_rule(
                        dict,
                        r,
                        &[],
                        &v_old,
                        Some((&delta, k)),
                        &mut stats.eval,
                        &mut produced,
                        true,
                    );
                }
            }
            delta = produced
                .into_iter()
                .filter(|h| derived_old.contains(h) && over.insert(*h))
                .collect();
        }
        stats.overdeleted += over.len() as u64;
        // --- 2. Prune: drop the overdeleted facts; also hand adds-owned facts
        // back to base (an inserted fact this stratum used to own).
        let mut derived_new: FxHashSet<[Id; 3]> = derived_old
            .iter()
            .filter(|f| !over.contains(*f) && !below_new.contains(*f))
            .copied()
            .collect();
        let mut store = FactStore::default();
        for f in below_new.iter().chain(&derived_new) {
            store.insert(*f);
        }
        // --- 3. Rederivation: one full pass over the reduced store reinstates
        // overdeleted facts with an alternative derivation (any derivable fact
        // missing from the reduced store is overdeleted-or-insert-consequence;
        // the `over` filter keeps this phase to reinstation, the insertion
        // phase finds the rest). Semi-naive continuation chases chains of
        // reinstatements. Skipped entirely on a pure-insert update (`over`
        // empty): nothing can need reinstating, so the full pass would be
        // wasted work.
        let mut delta: Vec<[Id; 3]> = Vec::new();
        if !over.is_empty() {
            let mut produced: Vec<[Id; 3]> = Vec::new();
            for r in &rules {
                run_rule(
                    dict,
                    r,
                    &[],
                    &store,
                    None,
                    &mut stats.eval,
                    &mut produced,
                    false,
                );
            }
            for h in produced {
                if over.contains(&h) && store.insert(h) {
                    derived_new.insert(h);
                    delta.push(h);
                }
            }
        }
        while !delta.is_empty() {
            let mut produced: Vec<[Id; 3]> = Vec::new();
            for r in &rules {
                for k in 0..r.positive.len() {
                    run_rule(
                        dict,
                        r,
                        &[],
                        &store,
                        Some((&delta, k)),
                        &mut stats.eval,
                        &mut produced,
                        false,
                    );
                }
            }
            delta = produced
                .into_iter()
                .filter(|h| {
                    if over.contains(h) && store.insert(*h) {
                        derived_new.insert(*h);
                        true
                    } else {
                        false
                    }
                })
                .collect();
        }
        stats.rederived += derived_new.iter().filter(|f| over.contains(*f)).count() as u64;
        // --- 4. Insertion: standard semi-naive propagation with the inserted
        // facts as the round-0 delta (they are already in the store via
        // `below_new`; consequences chain through later rounds).
        let mut delta: Vec<[Id; 3]> = adds.to_vec();
        while !delta.is_empty() {
            let mut produced: Vec<[Id; 3]> = Vec::new();
            for r in &rules {
                for k in 0..r.positive.len() {
                    run_rule(
                        dict,
                        r,
                        &[],
                        &store,
                        Some((&delta, k)),
                        &mut stats.eval,
                        &mut produced,
                        false,
                    );
                }
            }
            delta = produced
                .into_iter()
                .filter(|h| {
                    if store.insert(*h) {
                        derived_new.insert(*h);
                        true
                    } else {
                        false
                    }
                })
                .collect();
        }
        derived_new
    }
}
