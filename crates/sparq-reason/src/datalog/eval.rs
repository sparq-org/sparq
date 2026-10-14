//! sq-6tykl.3 / sq-8sve7 — the **semi-naive per-stratum evaluator** (Phase 2).
//!
//! Strata run in ascending order; within a stratum, rules iterate to fixpoint with
//! **semi-naive delta restriction** — the exact round discipline
//! [`crate::n3::compiled`]'s eval loop implements:
//!
//! * **round 0**: every fact in the store (inputs + lower-strata derivations) is the
//!   delta, so the first round is equivalent to a full run;
//! * **each later round**: a monotonic rule re-runs once per positive-atom index `k`
//!   with atom `k` restricted to the previous round's newly-derived delta (all other
//!   atoms read the full store; duplicates de-duplicate at store insertion). Any new
//!   derivation must use at least one delta fact in some positive position, so the
//!   union over `k` is complete; instantiations entirely inside the older store were
//!   produced in an earlier round.
//! * rules with **no positive atoms** draw only on relations that are constant within
//!   the stratum (aggregate tables, `NOT` over completed lower strata), so they fire
//!   in round 0 only — the n3 `join_steps.is_empty()` discipline;
//! * rules carrying **`NOT`** keep the full re-run every round ([`needs_full`], the n3
//!   discipline for scoped negation). The stratification checker already guarantees
//!   every negated relation is complete before its stratum runs (so delta restriction
//!   would remain sound); the full re-run is deliberate defence-in-depth for this
//!   soundness-sensitive path — it can only cost work, never correctness.
//!
//! Because the checker guarantees every `NOT`-negated / `AGGREGATE`-read predicate is
//! complete before its stratum runs:
//!
//! * `AGGREGATE` tables are computed **once at stratum entry** (their bodies read
//!   strictly-lower strata, which no longer change) and then joined like positive
//!   relations — they are constant within the stratum, so they never gate delta
//!   restriction of the positive atoms;
//! * `NOT` is a per-row absence check against the store's predicate index —
//!   the indexed relation is final, so no retraction can ever be needed.
//!
//! [`EvalStats`] counts rounds and the candidate rows fed into positive-atom join
//! steps — a DETERMINISTIC work measure (no wall-clock; work-box timings are
//! non-canonical). The differential suite asserts the semi-naive closure equals both
//! the forced-full (naive) closure and the independent oracle on every battery input.
//!
//! Positive-atom and aggregate-table joins drive the SHARED
//! [`sparq_substrate::join`] kernels via the same thin layout-adapter pattern as
//! `crate::substrate_join` and `crate::n3::compiled` — build side = binding rows,
//! probe side = candidate rows, combined rows reshaped back into the rule's slot
//! layout.

use super::{
    numeric_value, AggAtom, AggFunc, Atom, CmpOp, DTerm, FactStore, Program, Rule, Stratification,
};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};
use sparq_substrate::join::{self as sjoin, JoinKeys, NoBudget};
use sparq_substrate::numeric::{as_numeric, ArithOp, Num};
use sparq_substrate::rows::{Row, NO_ID};
use std::cmp::Ordering;

const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

/// Deterministic evaluation counters (sq-8sve7): fixpoint `rounds` summed across
/// strata and the number of candidate rows (`tuples_considered`) fed into
/// positive-atom join steps during rule firing. Aggregate-table construction is
/// excluded — it runs once per stratum in both the semi-naive and the forced-full
/// mode, so it cancels out of any comparison. A pure work measure: NO wall-clock.
#[cfg(any(test, feature = "datalog"))]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EvalStats {
    /// Fixpoint rounds run, summed over all strata.
    pub(crate) rounds: usize,
    /// Candidate rows fed into positive-atom join steps (delta-restricted or full).
    pub(crate) tuples_considered: u64,
}

/// Evaluate `program` (already checked by [`super::stratify`]) over `facts` and
/// return the full closure (inputs + derivations, de-duplicated; treat as a set).
pub(super) fn eval_stratified(
    dict: &mut Dict,
    facts: &[[Id; 3]],
    program: &Program,
    strat: &Stratification,
) -> Vec<[Id; 3]> {
    let mut stats = EvalStats::default();
    eval_stratified_with_stats(dict, facts, program, strat, &mut stats, false)
}

/// The instrumented engine behind [`eval_stratified`]. `force_full` disables the
/// delta restriction so every rule re-runs against the full store each round —
/// exactly the Phase-1 naive discipline; the differential and stats tests use it as
/// the in-engine baseline (the independent `oracle` stays the external reference).
pub(super) fn eval_stratified_with_stats(
    dict: &mut Dict,
    facts: &[[Id; 3]],
    program: &Program,
    strat: &Stratification,
    stats: &mut EvalStats,
    force_full: bool,
) -> Vec<[Id; 3]> {
    let mut store = FactStore::default();
    for f in facts {
        store.insert(*f);
    }
    for s in 0..strat.n_strata {
        let rules: Vec<&Rule> = program
            .rules
            .iter()
            .zip(&strat.rule_stratum)
            .filter(|(_, rs)| **rs == s)
            .map(|(r, _)| r)
            .collect();
        run_stratum(dict, &rules, &mut store, stats, force_full);
    }
    store.list
}

/// Run ONE stratum's rules to fixpoint over `store`, which must already hold the
/// stratum's COMPLETE input (base facts plus every lower stratum's derivations —
/// the stratification contract). New derivations append to `store.list`, so the
/// caller can slice everything this stratum derived from the pre-call length.
/// Shared by [`eval_stratified_with_stats`] and the incremental maintainer's
/// stratum-recompute path (`super::incr`).
pub(super) fn run_stratum(
    dict: &mut Dict,
    rules: &[&Rule],
    store: &mut FactStore,
    stats: &mut EvalStats,
    force_full: bool,
) {
    if rules.is_empty() {
        return;
    }
    // Aggregate tables: computed once per stratum (bodies read lower strata).
    let agg_tables: Vec<Vec<Vec<Row>>> = rules
        .iter()
        .map(|r| {
            r.aggregates
                .iter()
                .map(|a| aggregate_table(dict, a, store))
                .collect()
        })
        .collect();
    // Round 0: everything derived so far (inputs + lower strata) is the delta.
    let mut delta: Vec<[Id; 3]> = store.list.clone();
    let mut first_round = true;
    loop {
        stats.rounds += 1;
        let mut produced: Vec<[Id; 3]> = Vec::new();
        for (r, tables) in rules.iter().zip(&agg_tables) {
            if force_full || needs_full(r) || r.positive.is_empty() {
                // Full re-run (non-monotonic-conservative), or a rule with no
                // positive atoms whose inputs are constant within the stratum:
                // the latter fires in round 0 only.
                if force_full || needs_full(r) || first_round {
                    run_rule(dict, r, tables, store, None, stats, &mut produced, false);
                }
            } else {
                // Semi-naive: once per positive-atom position, with that atom
                // restricted to the delta (dedup happens at store insertion).
                for k in 0..r.positive.len() {
                    run_rule(
                        dict,
                        r,
                        tables,
                        store,
                        Some((&delta, k)),
                        stats,
                        &mut produced,
                        false,
                    );
                }
            }
        }
        let mut new_delta: Vec<[Id; 3]> = Vec::new();
        for f in produced {
            if store.insert(f) {
                new_delta.push(f);
            }
        }
        first_round = false;
        if new_delta.is_empty() {
            break;
        }
        delta = new_delta;
    }
}

/// Conservative non-monotonicity flag, mirroring the n3 compiled path's
/// `needs_full`: a rule carrying `NOT` re-runs against the full store every round.
/// The stratification checker already guarantees the negated relations are complete
/// (strictly-lower strata) before the stratum runs — so this is defence-in-depth,
/// not a soundness requirement; see the module docs.
fn needs_full(rule: &Rule) -> bool {
    !rule.negated.is_empty()
}

/// `(binding slot, candidate column)` pairs mapping variable slots to the atom's
/// triple positions — used for equi-join keys and write-back slots alike.
type SlotCols = Vec<(usize, usize)>;
/// The join plan for one atom: `(key_cols, new_writes)`.
type AtomPlan = (SlotCols, SlotCols);

/// The join plan for one atom given the running bound-slot set: equi-join key
/// column pairs `(binding slot, candidate column)` for already-bound variables and
/// write-back slots for new ones (constants are pre-filtered by [`atom_admits`]).
/// Marks the atom's new variables as bound.
fn plan_atom(atom: &Atom, bound: &mut FxHashSet<u32>) -> AtomPlan {
    let mut key_cols = Vec::new();
    let mut new_writes = Vec::new();
    for (i, t) in atom.t.iter().enumerate() {
        match t {
            DTerm::Const(_) => {}
            DTerm::Var(v) => {
                if bound.contains(v) {
                    key_cols.push((*v as usize, i));
                } else {
                    new_writes.push((*v as usize, i));
                }
            }
        }
    }
    for &(v, _) in &new_writes {
        bound.insert(v as u32);
    }
    (key_cols, new_writes)
}

/// Does the fact pass the atom's constant pre-filters (predicate + any constant
/// subject/object positions)?
fn atom_admits(atom: &Atom, f: &[Id; 3]) -> bool {
    atom.t.iter().zip(f).all(|(t, id)| match t {
        DTerm::Const(c) => c == id,
        DTerm::Var(_) => true,
    })
}

/// Candidate facts for one atom, as substrate probe rows `[s, p, o]`: drawn from
/// the store's predicate index — or from `delta` when this atom is the semi-naive
/// restricted position — and narrowed by the constant pre-filters.
fn candidates(atom: &Atom, store: &FactStore, delta: Option<&[[Id; 3]]>) -> Vec<Row> {
    let source: &[[Id; 3]] = match delta {
        Some(d) => d,
        None => match atom.pred {
            Some(pred) => store.by_pred.get(&pred).map_or(&[][..], |v| v.as_slice()),
            None => store.list.as_slice(),
        },
    };
    source
        .iter()
        .filter(|t| atom_admits(atom, t))
        .map(|t| Row::from_slice(t))
        .collect()
}

/// One join step over the SHARED substrate kernels: binding rows are the build
/// side, `cands` the probe side; each combined row (`bindings ++ candidate`) is
/// reshaped back into the `width`-slot layout by writing the new variables'
/// columns (a repeated new variable within one atom must agree).
fn join_rows(
    rows: &[Row],
    key_cols: &[(usize, usize)],
    new_writes: &[(usize, usize)],
    cands: &[Row],
    width: usize,
    cand_width: usize,
) -> Vec<Row> {
    if rows.is_empty() || cands.is_empty() {
        return Vec::new();
    }
    let keys = JoinKeys {
        key_cols: key_cols.to_vec(),
        right_only: Vec::new(),
    };
    let tables = vec![sjoin::build_table(rows, &keys)];
    let probe_only: Vec<usize> = (0..cand_width).collect();
    let mut combined: Vec<Row> = Vec::new();
    sjoin::hash_probe_serial(
        cands,
        &keys,
        rows,
        &tables,
        &probe_only,
        &NoBudget,
        &mut combined,
    );
    let mut out = Vec::with_capacity(combined.len());
    'row: for c in &combined {
        let (b, f) = c.split_at(width);
        let mut row = Row::from_slice(b);
        for &(v, i) in new_writes {
            if row[v] != NO_ID && row[v] != f[i] {
                continue 'row; // repeated new variable in one atom must agree
            }
            row[v] = f[i];
        }
        out.push(row);
    }
    out
}

/// Evaluate an `AGGREGATE` atom's positive body over the (stratum-complete) store,
/// de-duplicate the full binding tuples (set semantics), group by the `ON`
/// variables and fold the selected aggregate, minting computed numeric results.
/// Returns rows `[on₀ … onₖ₋₁, value]` in the OUTER join
/// layout (`ON` columns first, in `ON` order). Empty groups produce no row.
///
/// The numeric functions' operand/result typing and their float/`NaN`/`-0.0`
/// determinism rule live on [`fold_numeric_group`].
fn aggregate_table(dict: &mut Dict, agg: &AggAtom, store: &FactStore) -> Vec<Row> {
    let empty: Row = std::iter::repeat_n(NO_ID, agg.n_slots).collect();
    let mut rows: Vec<Row> = vec![empty];
    let mut bound: FxHashSet<u32> = FxHashSet::default();
    for atom in &agg.body {
        let (key_cols, new_writes) = plan_atom(atom, &mut bound);
        let cands = candidates(atom, store, None);
        rows = join_rows(&rows, &key_cols, &new_writes, &cands, agg.n_slots, 3);
    }
    // Distinct full tuples, then aggregate per ON-group. A non-numeric
    // input fails only that body row, matching FILTER's fail-closed posture.
    let distinct: FxHashSet<Row> = rows.into_iter().collect();
    let mut groups: FxHashMap<Vec<Id>, Vec<(Id, Num)>> = FxHashMap::default();
    let mut counts: FxHashMap<Vec<Id>, u64> = FxHashMap::default();
    let mut distinct_counts: FxHashMap<Vec<Id>, FxHashSet<Id>> = FxHashMap::default();
    for r in &distinct {
        let key: Vec<Id> = agg.on.iter().map(|&(l, _)| r[l as usize]).collect();
        if agg.func == AggFunc::Count {
            if agg.distinct {
                let value = r[agg.value.expect("COUNT DISTINCT has a value slot") as usize];
                distinct_counts.entry(key).or_default().insert(value);
            } else {
                *counts.entry(key).or_insert(0) += 1;
            }
        } else {
            let id = r[agg.value.expect("numeric aggregate has a value slot") as usize];
            let oxrdf::Term::Literal(lit) = dict.term(id) else {
                continue;
            };
            let Some(num) = as_numeric(&lit) else {
                continue;
            };
            groups.entry(key).or_default().push((id, num));
        }
    }
    for (key, values) in distinct_counts {
        counts.insert(key, values.len() as u64);
    }
    let mut out = Vec::with_capacity(groups.len() + counts.len());
    for (key, cnt) in counts {
        let value_id = dict.intern_lit(&cnt.to_string(), XSD_INTEGER, None);
        let mut row: Row = Row::from_slice(&key);
        row.push(value_id);
        out.push(row);
    }
    for (key, mut values) in groups {
        if let Some(value_id) = fold_numeric_group(dict, agg.func, &mut values) {
            let mut row: Row = Row::from_slice(&key);
            row.push(value_id);
            out.push(row);
        }
    }
    out
}

/// Fold one `ON`-group's `(term id, numeric value)` pairs into the aggregate's
/// result term — `None` for an empty group (and for an exact-type division by zero,
/// which `AVG` cannot reach since a non-empty group has a non-zero count).
///
/// # Numeric semantics of `SUM` / `MIN` / `MAX` / `AVG` (sq-r2nor)
///
/// The DECIDED rule, matching the `FILTER` posture documented on the module: the
/// operands are the **whole shared XSD numeric tower** — `xsd:float` and `xsd:double`
/// are first-class alongside the exact `integer`/`decimal` tiers, not rejected. Rows
/// whose value term is not a well-formed numeric literal fail closed (the caller drops
/// them before this point); a group left empty by that filtering produces no row.
///
/// * **`SUM` / `AVG` promote** per XPath operand promotion
///   ([`Num::binop`]): any `xsd:double` operand makes the result `xsd:double`, any
///   `xsd:float` (and no double) makes it `xsd:float`; otherwise the exact tier is kept
///   (`AVG` over integers is an `xsd:decimal`, as SPARQL requires).
/// * **`MIN` / `MAX` never promote or mint** — they return one of the INPUT terms
///   verbatim, so `MIN` over `{"1"^^xsd:long, "9"^^xsd:short}` yields the `xsd:long`.
///
/// ## Determinism (the explicit rule the float tiers need)
///
/// Floating-point addition is **not associative**, and the value order has ties across
/// tiers, so a fold in hash-iteration order could derive a different closure from the
/// same store depending on the order facts happened to be derived in (semi-naive vs
/// forced-full, or a different insertion order). Aggregates are already non-monotonic —
/// that is why they are stratified — but they must still be a FUNCTION of the completed
/// lower strata. So the fold order is pinned:
///
/// * `SUM`/`AVG` add in ascending [`Num::cmp_total`] order, ties broken by RDF-term
///   CONTENT (`cmp_term_content`), so the result depends only on the group's multiset
///   of value terms;
/// * `MIN`/`MAX` are order-independent by construction: `cmp_total` is a total order, and
///   a tie between two DISTINCT terms of equal value (`"1"^^xsd:integer` vs
///   `"1.0"^^xsd:double`, or `+0.0` vs `-0.0`) is resolved by that same content order for
///   both functions.
///
/// The tie-break is deliberately NOT the dictionary [`Id`]: an id is assigned by
/// interning order, which is not itself a function of the completed lower strata — see
/// `cmp_term_content`.
///
/// ## `NaN` and `-0.0`
///
/// [`Num::cmp_total`] totalises `NaN` BELOW `-INF`, so `MIN` of a group containing a
/// `NaN` is that `NaN`, and `MAX` ignores `NaN`s unless every value is one. `NaN` is
/// deliberately NOT a row-level failure here, unlike `FILTER` — which uses the
/// *relational* comparison, where `NaN` is an XPath type error. `SUM`/`AVG` propagate
/// `NaN` per IEEE-754. `+0.0` and `-0.0` compare EQUAL under `cmp_total`, so the sign of
/// the term `MIN`/`MAX` returns for such a group is decided by the content tie-break
/// (`-0.0` sorts before `0.0` lexically): it is not determined by the values alone, but it
/// IS determined by the group's terms. `SUM`/`AVG` follow IEEE-754 (`+0.0 + -0.0` is
/// `+0.0`).
// `pub(super)` so the acceptance suite can pin the fold-order rule directly, with the
// group handed to it in adversarial permutations (the join/hash order is not steerable
// from the program text).
pub(super) fn fold_numeric_group(
    dict: &mut Dict,
    func: AggFunc,
    values: &mut [(Id, Num)],
) -> Option<Id> {
    match func {
        AggFunc::Count => unreachable!("COUNT is folded from the count maps, not the value list"),
        AggFunc::Min | AggFunc::Max => values
            .iter()
            .copied()
            .reduce(|a, b| {
                let keep_a = match a.1.cmp_total(b.1) {
                    Ordering::Equal => cmp_term_content(dict, a.0, b.0) != Ordering::Greater,
                    Ordering::Less => func == AggFunc::Min,
                    Ordering::Greater => func == AggFunc::Max,
                };
                if keep_a {
                    a
                } else {
                    b
                }
            })
            .map(|(id, _)| id),
        AggFunc::Sum | AggFunc::Avg => {
            let count = values.len() as i64;
            values.sort_by(|a, b| {
                a.1.cmp_total(b.1)
                    .then_with(|| cmp_term_content(dict, a.0, b.0))
            });
            values
                .iter()
                .map(|(_, n)| *n)
                .reduce(|a, b| a.binop(b, ArithOp::Add).expect("addition is total"))
                .and_then(|sum| {
                    let result = if func == AggFunc::Avg {
                        sum.binop(Num::Int(count), ArithOp::Div)?
                    } else {
                        sum
                    };
                    Some(dict.intern_lit(
                        &result.canonical_lexical(),
                        result.datatype().as_str(),
                        None,
                    ))
                })
        }
    }
}

/// A total order on two term ids by RDF-term CONTENT — the tie-break
/// [`fold_numeric_group`] applies to two terms of equal numeric value.
///
/// It is deliberately NOT the dictionary [`Id`] order. An `Id` is assigned by INTERNING
/// order, which is not a function of the completed lower strata: an earlier stratum's
/// aggregate mints its computed literals while iterating a hash map, so two evaluations
/// of the same program over the same facts (semi-naive vs forced-full, or a different
/// insertion order) can intern two equal-valued terms with opposite ids — as can two
/// separately loaded copies of logically equal input. An id tie-break would then let
/// `MIN`/`MAX` emit a DIFFERENT input term, and `SUM`/`AVG` add equal-valued operands in
/// a different (promotion-sensitive, so not value-preserving) order, for the same logical
/// facts. Ordering the literal's `(datatype, lexical form, language)` instead depends on
/// nothing but the terms themselves.
///
/// Every id reaching here is a numeric literal — the caller drops rows whose value term
/// is not one — so the non-literal arm is unreachable; it falls back to the id purely to
/// keep the order total.
fn cmp_term_content(dict: &Dict, a: Id, b: Id) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    match (dict.term(a), dict.term(b)) {
        (oxrdf::Term::Literal(x), oxrdf::Term::Literal(y)) => x
            .datatype()
            .as_str()
            .cmp(y.datatype().as_str())
            .then_with(|| x.value().cmp(y.value()))
            .then_with(|| x.language().cmp(&y.language())),
        _ => a.cmp(&b),
    }
}

/// Fire one rule against the current store, appending (possibly duplicate)
/// conclusions to `out` — the caller's store insertion de-duplicates.
///
/// `delta`: `Some((delta_facts, k))` restricts positive atom `k` to the delta (the
/// semi-naive run for position `k`); `None` is a full run. When the restricted
/// atom admits no delta fact the whole conjunction is empty, so the run is skipped
/// up front — this keeps `stats.tuples_considered` an honest work measure (the
/// other atoms' candidates are never materialised for a run that cannot fire).
///
/// `emit_known`: when `false` (forward evaluation), conclusions already in the
/// store are suppressed; when `true`, every conclusion is emitted — the DRed
/// overdeletion pass (`super::incr`) needs heads that ARE in the store, because
/// it is hunting facts to retract, not facts to add.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_rule(
    dict: &Dict,
    rule: &Rule,
    agg_tables: &[Vec<Row>],
    store: &FactStore,
    delta: Option<(&[[Id; 3]], usize)>,
    stats: &mut EvalStats,
    out: &mut Vec<[Id; 3]>,
    emit_known: bool,
) {
    if let Some((d, k)) = delta {
        if !d.iter().any(|f| atom_admits(&rule.positive[k], f)) {
            return;
        }
    }
    let empty: Row = std::iter::repeat_n(NO_ID, rule.n_slots).collect();
    let mut rows: Vec<Row> = vec![empty];
    let mut bound: FxHashSet<u32> = FxHashSet::default();
    for (idx, atom) in rule.positive.iter().enumerate() {
        let (key_cols, new_writes) = plan_atom(atom, &mut bound);
        let restricted = match delta {
            Some((d, k)) if k == idx => Some(d),
            _ => None,
        };
        let cands = candidates(atom, store, restricted);
        stats.tuples_considered += cands.len() as u64;
        rows = join_rows(&rows, &key_cols, &new_writes, &cands, rule.n_slots, 3);
        if rows.is_empty() {
            return;
        }
    }
    // Aggregate tables join like positive relations: columns are the ON outer
    // slots (in order) then the output slot.
    for (agg, table) in rule.aggregates.iter().zip(agg_tables) {
        let cand_width = agg.on.len() + 1;
        let mut key_cols = Vec::new();
        let mut new_writes = Vec::new();
        for (i, &(_, outer)) in agg.on.iter().enumerate() {
            if bound.contains(&outer) {
                key_cols.push((outer as usize, i));
            } else {
                new_writes.push((outer as usize, i));
                bound.insert(outer);
            }
        }
        new_writes.push((agg.out as usize, agg.on.len()));
        bound.insert(agg.out);
        rows = join_rows(
            &rows,
            &key_cols,
            &new_writes,
            table,
            rule.n_slots,
            cand_width,
        );
        if rows.is_empty() {
            return;
        }
    }
    // NOT: each group is one existential conjunction over the stratum-complete
    // store. Group-local wildcard bindings join its atoms and never escape.
    for group in &rule.negated {
        rows.retain(|r| !naf_matches(group, r, &bound, store));
        if rows.is_empty() {
            return;
        }
    }
    // FILTER: relational numeric comparison via the shared substrate tower;
    // non-numeric and NaN operands fail the row (fail-closed).
    for f in &rule.filters {
        rows.retain(|r| {
            let val = |t: &DTerm| match t {
                DTerm::Const(id) => numeric_value(dict, *id),
                DTerm::Var(v) => numeric_value(dict, r[*v as usize]),
            };
            let (Some(a), Some(b)) = (val(&f.a), val(&f.b)) else {
                return false;
            };
            let Some(ord) = a.cmp_relational(b) else {
                return false;
            };
            match f.op {
                CmpOp::Eq => ord.is_eq(),
                CmpOp::Ne => ord.is_ne(),
                CmpOp::Lt => ord.is_lt(),
                CmpOp::Le => ord.is_le(),
                CmpOp::Gt => ord.is_gt(),
                CmpOp::Ge => ord.is_ge(),
            }
        });
        if rows.is_empty() {
            return;
        }
    }
    for r in &rows {
        for head in &rule.head {
            let g = [
                resolve(&head.t[0], r),
                resolve(&head.t[1], r),
                resolve(&head.t[2], r),
            ];
            if head.pred.is_none() && !matches!(dict.term(g[1]), oxrdf::Term::NamedNode(_)) {
                continue;
            }
            if emit_known || !store.all.contains(&g) {
                out.push(g);
            }
        }
    }
}

fn resolve(t: &DTerm, row: &Row) -> Id {
    match t {
        DTerm::Const(id) => *id,
        DTerm::Var(v) => row[*v as usize],
    }
}

/// Does a negated conjunction have at least one joint match under `row`?
fn naf_matches(group: &[Atom], row: &Row, bound: &FxHashSet<u32>, store: &FactStore) -> bool {
    fn search(
        group: &[Atom],
        index: usize,
        row: &Row,
        bound: &FxHashSet<u32>,
        wild: &FxHashMap<u32, Id>,
        store: &FactStore,
    ) -> bool {
        if index == group.len() {
            return true;
        }
        let atom = &group[index];
        let facts: &[[Id; 3]] = match atom.pred {
            Some(pred) => store.by_pred.get(&pred).map_or(&[][..], |v| v.as_slice()),
            None => store.list.as_slice(),
        };
        facts.iter().any(|fact| {
            let mut next = wild.clone();
            let matches = atom.t.iter().enumerate().all(|(i, term)| match term {
                DTerm::Const(id) => fact[i] == *id,
                DTerm::Var(v) if bound.contains(v) => fact[i] == row[*v as usize],
                DTerm::Var(v) => *next.entry(*v).or_insert(fact[i]) == fact[i],
            });
            matches && search(group, index + 1, row, bound, &next, store)
        })
    }

    search(group, 0, row, bound, &FxHashMap::default(), store)
}
