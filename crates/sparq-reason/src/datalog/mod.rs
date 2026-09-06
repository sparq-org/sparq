//! [FABLE-5] sq-6tykl.3 (epic sq-6tykl, RDFox-parity BIG ROCK) — **stratified Datalog
//! rules** with negation-as-failure and aggregation, behind the opt-in `datalog` feature.
//!
//! The stratified-Datalog program (design record
//! `research/stratified-datalog-rules.md`): a small **native rule dialect** modelled on
//! RDFox's Datalog surface (the maintainer's open question 4 — see the record §2 for the
//! decision), a **stratification checker**, a **semi-naive per-stratum evaluator**
//! (Phase 2, sq-8sve7 — delta-restricted positive joins; see `eval`) supporting `NOT`
//! (store-scoped negation as failure), `AGGREGATE … BIND
//! COUNT/SUM/MIN/MAX/AVG(?v) AS ?c` atoms (Phase 2b, sq-citho) and a minimal numeric
//! `FILTER`, plus **incremental maintenance under insert/delete across strata**
//! (Phase 3, sq-4foq0 — see [`MaterializedProgram`]: DRed for positive strata,
//! rederivation at stratum boundaries for `NOT`/`AGGREGATE` strata). [GPT-5.6]
//! Phase 4 (sq-a7bmo) adds grouped `NOT`, projected `COUNT(DISTINCT ?v)`, the full
//! numeric tower for `FILTER`, and conservative variable-predicate atoms. Surface
//! wiring stays beaded from the design record; the external-engine differential arm
//! (sq-xzb9p) has SHIPPED as the test-only `souffle` module — Soufflé stratifies the
//! translated program itself, so that arm also checks our stratification. See its
//! module docs for the relational fragment it covers and why it stops there.
//!
//! # The dialect
//!
//! ```text
//! @prefix ex: <http://example.org/> .
//!
//! [?y, ex:reach, "y"] :- [?x, ex:reach, "y"], [?x, ex:edge, ?y] .          # recursion
//! [?x, ex:orphan, "y"] :- [?x, a, ex:Node], NOT [?p, ex:child, ?x] .      # NAF
//! [?x, ex:deg, ?c]     :- AGGREGATE([?x, ex:edge, ?y] ON ?x
//!                                    BIND COUNT(?y) AS ?c) .               # aggregation
//! [?x, a, ex:Hub]      :- [?x, ex:deg, ?c], FILTER(?c >= 3) .             # threshold
//! ```
//!
//! * **Atoms** are triple patterns `[s, p, o]`; `p` may be an IRI, `a` = `rdf:type`,
//!   or a variable. Subjects/objects are IRIs, literals (bare integers are
//!   `xsd:integer`; `"…"` with optional `^^dt`), or `?variables`.
//! * **`NOT atom` / `NOT { atom, atom }`** is negation as failure against the
//!   completed lower strata. A group succeeds only when its whole conjunction has a
//!   joint match. Variables not bound by a positive atom are existential wildcards;
//!   their bindings are shared within one group and discarded outside it.
//! * **`AGGREGATE(atoms ON ?g… BIND COUNT(?v) AS ?c)`** groups the DISTINCT matches of
//!   its positive body (set semantics — Datalog relations are sets, so `COUNT(?v)`
//!   counts distinct body matches per group; `COUNT(DISTINCT ?v)` instead projects and
//!   de-duplicates `?v`; there are no rows for empty groups) by the `ON` variables and
//!   binds the count (an `xsd:integer`) to `?c`. Body variables other
//!   than the `ON` list are aggregate-local. `ON` may be omitted for a global count.
//! * **`SUM`/`MIN`/`MAX`/`AVG`** read the same distinct body matches over the **whole**
//!   shared XSD numeric tower — `xsd:float`/`xsd:double` are accepted alongside the exact
//!   `integer`/`decimal` tiers, and a non-numeric value fails only that row (fail-closed,
//!   as `FILTER` does). `SUM`/`AVG` mint a result under XPath operand promotion (so a
//!   double operand yields an `xsd:double`; `AVG` over integers is an `xsd:decimal`);
//!   `MIN`/`MAX` return an INPUT term verbatim. Because floating-point addition is not
//!   associative, the fold order is PINNED so the closure stays a function of the
//!   completed lower strata, and `NaN`/`-0.0` get an explicit rule — see
//!   `eval::fold_numeric_group` for the full semantics.
//! * **`FILTER(x op y)`** compares two bound-or-constant XSD numeric values with
//!   `= != < <= > >=` via [`sparq_substrate::numeric::Num::cmp_relational`].
//!   Float/double operands participate with XPath promotion; NaN and non-numeric
//!   operands fail the row.
//!
//! Everything else is a **loud [`parse_program`] error**, never a silent divergence:
//! nested `NOT`/`AGGREGATE` inside an aggregate body, unbound head/filter variables, and
//! aggregate-local variable capture.
//!
//! # Stratified semantics
//!
//! [`stratify`] builds the predicate dependency graph (an edge per body atom of each
//! rule's head predicate; `NOT` atoms and every predicate inside an `AGGREGATE` body are
//! **non-monotonic** edges) and either assigns each rule a stratum or rejects the
//! program with an error naming a predicate on a negation/aggregation cycle. [`eval`]
//! then runs a fixpoint per stratum in order, so every negated or aggregated predicate
//! is **complete** before it is read — the textbook perfect-model semantics, and the
//! checked replacement for the caller-discipline stratification the N3 compiled path
//! documents (`crate::n3::compiled`).
//!
//! # Join machinery
//!
//! Positive-atom and aggregate-table joins drive the SHARED [`sparq_substrate::join`]
//! kernels ([`sparq_substrate::join::build_table`] +
//! [`sparq_substrate::join::hash_probe_serial`] — the same monomorphic bodies the SPARQL
//! engine, the RDFS materialiser and the N3 compiled path drive; deliberately **not**
//! another join implementation, only this module's thin layout adapter over them).
//! `FILTER` numerics delegate to the shared substrate `numeric` tower — the same
//! `Dec` the engine's FILTER path uses, so the two can never diverge on exact-decimal
//! comparison.
//!
//! The correctness oracle is the in-module differential suite: an independent naive
//! substitution-based evaluator (`oracle`, test-only — no substrate kernels, no
//! indexes) must agree with the kernel-driven evaluator on every fixture and on
//! seed-randomised graphs.

use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};

mod eval;
mod incr;
#[cfg(test)]
mod oracle;
mod parser;
#[cfg(test)]
mod souffle;
mod stratify;

pub use incr::MaterializedProgram;
pub use stratify::{stratify, Stratification};

/// One term position of a parsed atom: a dictionary constant or a rule-scoped
/// (or aggregate-local) variable slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DTerm {
    /// A ground term, already interned into the caller's [`Dict`].
    Const(Id),
    /// A variable slot (a column of the binding row in its scope).
    Var(u32),
}

/// One triple-pattern atom. `pred` caches the constant predicate for indexed
/// access; `None` denotes a variable predicate and scans the predicate-index union.
#[derive(Clone, Debug)]
pub(crate) struct Atom {
    pub(crate) t: [DTerm; 3],
    pub(crate) pred: Option<Id>,
}

/// One `AGGREGATE(body ON ?g… BIND FUNC(?v) AS ?out)` atom. The body has its own
/// variable scope (`n_slots` local slots); `on` maps each grouping variable's
/// aggregate-local slot to its outer rule slot. Every function consumes distinct
/// full-body matches first; `value` identifies the numeric input slot, or the
/// projected slot for `COUNT(DISTINCT)` (legacy `COUNT` needs no value slot).
#[derive(Clone, Debug)]
pub(crate) struct AggAtom {
    pub(crate) func: AggFunc,
    /// `COUNT(DISTINCT ?v)` projects `value` before de-duplication.
    pub(crate) distinct: bool,
    pub(crate) value: Option<u32>,
    pub(crate) body: Vec<Atom>,
    /// `(aggregate-local slot, outer rule slot)` per `ON` variable, in `ON` order.
    pub(crate) on: Vec<(u32, u32)>,
    /// Outer rule slot the aggregate value binds to (fresh in the outer scope).
    pub(crate) out: u32,
    /// Aggregate-local slot count.
    pub(crate) n_slots: usize,
}

/// [GPT-5.6] Aggregate functions supported by the native Datalog dialect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AggFunc {
    Count,
    Sum,
    Min,
    Max,
    Avg,
}

/// A `FILTER` comparison operator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// One `FILTER(a op b)` condition over bound variables / numeric constants.
#[derive(Clone, Debug)]
pub(crate) struct Filter {
    pub(crate) a: DTerm,
    pub(crate) op: CmpOp,
    pub(crate) b: DTerm,
}

/// One parsed rule. Body elements are grouped by kind; positive atoms keep their
/// source order (the join order), and `NOT`/`FILTER` run after all joins — every
/// correlated variable is bound by then (checked at parse time).
#[derive(Clone, Debug)]
pub(crate) struct Rule {
    pub(crate) head: Vec<Atom>,
    pub(crate) positive: Vec<Atom>,
    pub(crate) aggregates: Vec<AggAtom>,
    /// Each entry is one jointly-matched negated conjunction.
    pub(crate) negated: Vec<Vec<Atom>>,
    pub(crate) filters: Vec<Filter>,
    /// Rule-scope slot count (head + positive + NOT + `ON` + `AS` variables).
    pub(crate) n_slots: usize,
}

/// A parsed, validated stratified-Datalog program whose constants are interned
/// into the [`Dict`] passed to [`parse_program`]. Evaluate with [`eval`] (or check
/// stratifiability alone with [`stratify`]).
#[derive(Clone, Debug)]
pub struct Program {
    pub(crate) rules: Vec<Rule>,
}

impl Program {
    /// Number of parsed rules.
    ///
    /// # Examples
    ///
    /// ```
    /// use sparq_reason::datalog::parse_program;
    /// let mut dict = sparq_core::dict::Dict::new();
    /// let p = parse_program(
    ///     &mut dict,
    ///     "@prefix ex: <http://ex/> . [?x, ex:q, ?y] :- [?x, ex:p, ?y] .",
    /// )?;
    /// assert_eq!(p.n_rules(), 1);
    /// # Ok::<(), String>(())
    /// ```
    pub fn n_rules(&self) -> usize {
        self.rules.len()
    }
}

/// Parse a stratified-Datalog rule document (see the module
/// docs) and intern its ground terms into `dict`.
///
/// # Errors
///
/// Returns `Err` on a syntax error or on any construct outside the documented
/// fragment (unknown aggregate functions, unbound head or `FILTER` variables,
/// aggregate-local variable capture, …) — always a loud
/// error naming the construct, never a silent divergence.
///
/// # Examples
///
/// ```
/// use sparq_reason::datalog::parse_program;
/// let mut dict = sparq_core::dict::Dict::new();
/// let p = parse_program(
///     &mut dict,
///     "@prefix ex: <http://ex/> .
///      [?x, ex:big, \"y\"] :- AGGREGATE([?x, ex:member, ?m] ON ?x
///                                       BIND COUNT(?m) AS ?n),
///                             FILTER(?n >= 2) .",
/// )?;
/// assert_eq!(p.n_rules(), 1);
/// # Ok::<(), String>(())
/// ```
pub fn parse_program(dict: &mut Dict, src: &str) -> Result<Program, String> {
    parser::parse(dict, src)
}

/// Check stratifiability and evaluate `program` over `facts`: run the per-stratum
/// forward fixpoint to completion and return the full ground closure — input facts
/// plus every derivation, de-duplicated. Treat the result as a SET (order is
/// unspecified). Non-incremental semi-naive evaluation (delta-restricted rounds per
/// stratum — identical derived-fact sets to the naive discipline, by differential
/// test); `dict` is needed mutably because aggregation mints numeric literals into
/// the caller's id space.
///
/// # Errors
///
/// Returns `Err` (via [`stratify`]) when the program has a cycle through negation
/// or aggregation — such programs have no stratified model and are rejected, never
/// silently mis-evaluated.
///
/// # Examples
///
/// ```
/// use sparq_reason::datalog::{eval, parse_program};
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
/// let closure = eval(&mut dict, &[[s, pr, o]], &p)?;
/// assert_eq!(closure.len(), 2); // the fact + the derivation
/// # Ok::<(), String>(())
/// ```
pub fn eval(dict: &mut Dict, facts: &[[Id; 3]], program: &Program) -> Result<Vec<[Id; 3]>, String> {
    let strat = stratify(dict, program)?;
    Ok(eval::eval_stratified(dict, facts, program, &strat))
}

/// Resolve a dictionary id through the shared XSD numeric tower.
pub(crate) fn numeric_value(dict: &Dict, id: Id) -> Option<sparq_substrate::numeric::Num> {
    let oxrdf::Term::Literal(l) = dict.term(id) else {
        return None;
    };
    sparq_substrate::numeric::as_numeric(&l)
}

/// The set of variable slots a rule's positive machinery binds: positive-atom
/// variables, aggregate `ON` variables (outer slots) and aggregate outputs. Head,
/// `FILTER` and correlated `NOT` variables must come from this set.
pub(crate) fn bound_slots(rule: &Rule) -> FxHashSet<u32> {
    let mut bound: FxHashSet<u32> = FxHashSet::default();
    for a in &rule.positive {
        for t in &a.t {
            if let DTerm::Var(v) = t {
                bound.insert(*v);
            }
        }
    }
    for agg in &rule.aggregates {
        for &(_, outer) in &agg.on {
            bound.insert(outer);
        }
        bound.insert(agg.out);
    }
    bound
}

/// Predicate-indexed fact store shared by the evaluator: the de-duplicating set,
/// the append-only list (the closure in insertion order) and a per-predicate index.
#[derive(Default)]
pub(crate) struct FactStore {
    pub(crate) all: FxHashSet<[Id; 3]>,
    pub(crate) list: Vec<[Id; 3]>,
    pub(crate) by_pred: FxHashMap<Id, Vec<[Id; 3]>>,
}

impl FactStore {
    /// Insert a fact; returns `true` when new.
    pub(crate) fn insert(&mut self, f: [Id; 3]) -> bool {
        if self.all.insert(f) {
            self.list.push(f);
            self.by_pred.entry(f[1]).or_default().push(f);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests;
