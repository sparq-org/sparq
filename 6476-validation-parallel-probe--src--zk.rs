//! zk-trace seam (plan §4.E, stage 2): per-operator proof-input capture.
//!
//! Records, for an executed SELECT/ASK, the **per-obligation input sets** the
//! ZK witness builder needs: for each BGP triple pattern the EXACT set of
//! input triples consumed (post-scan, pre-join: the rows the scan kept after
//! constant/consistency/pushed-filter tests, BEFORE any join discarded them —
//! what the per-property proof must witness), the executed scan / join /
//! filter order with operator boundaries (OPTIONAL / UNION / MINUS / GRAPH /
//! DISTINCT / aggregation), and FILTER obligations with their per-row operand
//! bindings. The consumer (`sparq-zk`) resolves the recorded triples to leaf
//! indices in the per-graph commitment orderings and applies the Q6
//! cross-graph bnode-join guard.
//!
//! Pattern: exactly the EXPLAIN-ANALYZE operator trace's (`exec::trace`) —
//! a thread-local recorder armed by [`install`], drained by [`take`], with a
//! drop guard so a failed query never leaks a trace. The recording hooks in
//! `exec.rs` sit behind `#[cfg(feature = "zk")]` (this whole module does):
//! default builds carry ZERO zk code (the wasm artifact is byte-identical),
//! and a `zk` build without an installed recorder pays one thread-local
//! `Cell<bool>` read per *scan / operator entry* (not per row) — the same
//! cost class as `trace::enabled()` / the budget check.
//!
//! # Cost model when armed (and only when armed)
//!
//! The per-row path records **dictionary ids, never terms**: one id-set probe
//! per matched row (`record_scan_ids`) or one memo probe per filter operand
//! cell (`OperandMemo`). Terms are materialized **once per unique input
//! triple / unique operand value** — the floor for a term-carrying trace —
//! at record time, against the graph that produced the ids (correct for
//! `GRAPH` sub-graphs and FROM-assembled datasets, whose dictionaries do not
//! outlive evaluation).
//!
//! # Result-preserving plan changes under an armed recorder
//!
//! Shortcuts that ANSWER WITHOUT CONSUMING attributable inputs are disabled
//! while a recorder is armed, so the trace always contains the witness sets:
//! COUNT/ASK index-range pushdowns (`try_count` / `count_pushdown` /
//! `count_leftjoin`), LIMIT early-termination (`try_capped`), and the
//! streaming single-pattern JSON path. Results are identical either way (the
//! shortcuts are result-equivalent by construction — differentially fuzzed);
//! only the executed plan changes. Consequently the **pre-aggregation /
//! pre-DISTINCT input sets are always captured in full**.
//!
//! Proof-semantics note (DISTINCT / aggregation, plan §6.5 + Q9): an
//! aggregation or DISTINCT obligation witnesses the full pre-reduction input
//! sets (the pattern entries recorded between its [`Step::Enter`]/[`Step::Exit`]
//! markers); the reduction itself (grouping, counting, deduplication) is
//! re-checked verifier-side over the disclosed result — hidden-aggregate
//! circuits are a later deliverable.
//!
//! # Determinism
//!
//! [`take`] canonicalizes: each pattern's triple set is sorted by dictionary
//! id (set semantics — already deduplicated), and each filter's rows are
//! sorted lexicographically. Same query + same data (same load order) ⇒
//! identical trace. The step stream is the executed order, which is
//! deterministic for a given build (the planner is deterministic).
//!
//! # Coverage honesty
//!
//! Captured: BGP scans (binary GOO plans), index-nested-loop rescans
//! (bind-join), WCOJ trie builds (the consistency-passing source scan,
//! pre-projection), constant-only pattern existence checks, unsatisfiable
//! patterns (provably-empty input sets), FILTER operand rows, and scans
//! inside OPTIONAL / UNION / MINUS / GRAPH / sub-selects / DISTINCT /
//! aggregation (operator boundaries appear as [`Step::Enter`] /
//! [`Step::Exit`]; `GRAPH` scans carry the named graph, see
//! [`PatternMatches::graph`]).
//!
//! NOT captured — marked instead: property-path expansion and RDF-1.2
//! triple-term structural-unification relations scan the store without
//! per-pattern attribution; they record an [`Op::Path`] /
//! [`Op::QuotedTriples`] marker so a consumer can FAIL CLOSED
//! ([`ZkTrace::first_uncaptured`]) instead of building an insufficient
//! witness set. (NOT) EXISTS inner scans ARE captured (tagged
//! [`PatternMatches::in_exists`]) but their per-row re-evaluations are
//! step-suppressed and their FILTER obligations are not recorded — EXISTS is
//! outside the stage-1 verifiable fragment (`sparq-zk::verify` rejects it
//! from the query text; the tag is for forensics, not proofs).
//!
//! Non-monotone semantics note: for OPTIONAL / MINUS / UNION the recorded
//! per-pattern sets are the **post-scan consumed sets** — a conservative
//! superset of what any one disclosed row needs (e.g. an OPTIONAL right side
//! records rows that joined nothing). With flat per-graph commitments the
//! whole graph is in-circuit anyway (plan §2.2), so supersets cost padding,
//! not soundness. OPTIONAL's embedded join condition (`LeftJoin.expression`)
//! is evaluated inside the join and is NOT recorded as a [`FilterObligation`]
//! — flagged here; an obligation compiler must derive it from the query text
//! (it is outside the stage-1 fragment regardless).
//!
//! Base-vs-derived (entailmentRegime ≠ none, plan §5): the engine cannot
//! distinguish reasoner-materialized triples — that classification needs the
//! reasoner's base set and lives in `sparq-zk` (`trace_infer`, feature
//! `infer`), which joins this trace against `sparq-reason`'s materialized
//! graphs and their `explain` ProofTrees.

use oxrdf::{Term, Variable};
use rustc_hash::FxHashMap;
use sparq_core::dict::Id;
use sparq_core::Graph;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;

/// One slot of a recorded triple pattern: a constant term or a variable name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SlotPattern {
    Term(Term),
    Var(String),
    /// The slot was an unnamed position (no variable, no constant id — does
    /// not occur for store scans, kept for totality).
    Wildcard,
}

impl std::fmt::Display for SlotPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SlotPattern::Term(t) => write!(f, "{t}"),
            SlotPattern::Var(v) => write!(f, "?{v}"),
            SlotPattern::Wildcard => write!(f, "_"),
        }
    }
}

/// A recorded triple pattern (with the graph context and EXISTS tag, the
/// dedup key for matched-triple sets).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PatternKey {
    pub slots: [SlotPattern; 3],
}

/// The matched-triple input set of one BGP triple pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternMatches {
    pub pattern: PatternKey,
    /// The named graph the scan ran inside (`GRAPH <g> { … }` / each
    /// iteration of `GRAPH ?g { … }`); `None` = the default graph.
    pub graph: Option<Term>,
    /// `true` when the scans of this entry ran inside a FILTER (NOT) EXISTS
    /// sub-pattern (outside the stage-1 fragment; recorded for forensics).
    pub in_exists: bool,
    /// Matched triples (s, p, o), deduplicated, canonically ordered by
    /// dictionary id (see the module's determinism note).
    pub triples: Vec<[Term; 3]>,
}

/// One FILTER obligation: the expression, the binding variables in scope, the
/// distinct operand values, and the per-row operand references + verdict.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterObligation {
    /// Debug rendering of the `spargebra` expression (informational; the
    /// obligation compiler re-derives structure from the query text).
    pub expr: String,
    /// Variables of the filtered binding set, in column order.
    pub vars: Vec<String>,
    /// The named-graph context the filter ran inside (as for patterns).
    pub graph: Option<Term>,
    /// Distinct operand values (`None` = unbound), indexed by `rows` cells —
    /// the witness builder wants distinct operands, and this avoids
    /// materializing a term per row × column.
    pub operands: Vec<Option<Term>>,
    /// Per input row: one index into `operands` per var (column order) and
    /// whether the row passed the filter. Canonically sorted by [`take`].
    pub rows: Vec<(Vec<u32>, bool)>,
}

/// Operator boundaries recorded in the step stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// `OPTIONAL` (left join). Its embedded condition is NOT a recorded
    /// [`FilterObligation`] — see the module docs.
    Optional,
    Union,
    Minus,
    /// `GRAPH <g>` / `GRAPH ?g` — the enclosed scans carry the graph name.
    Graph,
    Distinct,
    /// GROUP BY / aggregation: the enclosed pattern inputs are the
    /// pre-aggregation input sets (see the module docs).
    Group,
    /// Property-path expansion — inputs NOT captured (fail closed).
    Path,
    /// RDF 1.2 triple-term unification relation — inputs NOT captured.
    QuotedTriples,
}

/// One step of the executed order, in execution sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A full index scan of a pattern (also: a WCOJ trie build).
    Scan { pattern: usize },
    /// An index-nested-loop (bind-join) rescan of a pattern against the
    /// running result.
    BindJoin { pattern: usize },
    /// A FILTER application.
    Filter { filter: usize },
    /// An operator boundary opening.
    Enter(Op),
    /// An operator boundary closing (paired with the matching [`Step::Enter`]).
    Exit(Op),
}

/// The drained zk trace of one executed query.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ZkTrace {
    /// Per-pattern matched-triple input sets (`Step::Scan`/`Step::BindJoin`
    /// indices point here).
    pub patterns: Vec<PatternMatches>,
    /// FILTER obligations (`Step::Filter` indices point here).
    pub filters: Vec<FilterObligation>,
    /// Executed scan / join / filter order with operator boundaries.
    pub steps: Vec<Step>,
}

impl ZkTrace {
    /// The first operator whose inputs are NOT captured by this seam
    /// ([`Op::Path`] / [`Op::QuotedTriples`]), if any executed. A witness
    /// builder must fail closed when this is `Some` — the trace is not a
    /// sufficient input set for the result.
    pub fn first_uncaptured(&self) -> Option<Op> {
        self.steps.iter().find_map(|s| match s {
            Step::Enter(op @ (Op::Path | Op::QuotedTriples)) => Some(*op),
            _ => None,
        })
    }
}

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    /// Depth of FILTER (NOT) EXISTS evaluation (step/filter suppression).
    static EXISTS_DEPTH: Cell<u32> = const { Cell::new(0) };
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// One pattern's accumulating input set. Terms are materialized at record
/// time (the producing graph's dictionary is only guaranteed alive then),
/// but only ONCE per unique id-triple; `ids` stays parallel to
/// `matches.triples` for the canonical sort in [`take`].
struct PatternState {
    matches: PatternMatches,
    ids: Vec<[Id; 3]>,
    seen: HashSet<[Id; 3]>,
}

#[derive(Default)]
struct State {
    patterns: Vec<PatternState>,
    filters: Vec<FilterObligation>,
    steps: Vec<Step>,
    /// Innermost-last stack of named-graph contexts (`GRAPH` operator).
    graph_ctx: Vec<Term>,
}

/// Disarms the recorder (and clears any partial trace) when the installing
/// entry point returns — also on error/unwind.
pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        ENABLED.with(|e| e.set(false));
        EXISTS_DEPTH.with(|d| d.set(0));
        STATE.with(|s| *s.borrow_mut() = State::default());
    }
}

/// Arms the thread-local zk-trace recorder for queries executed on this
/// thread until the guard drops. Drain with [`take`] before dropping.
pub fn install() -> Guard {
    ENABLED.with(|e| e.set(true));
    EXISTS_DEPTH.with(|d| d.set(0));
    STATE.with(|s| *s.borrow_mut() = State::default());
    Guard
}

/// Drains the recorded trace (call before the [`Guard`] drops), applying the
/// canonical ordering (module docs: determinism).
pub fn take() -> ZkTrace {
    let state = STATE.with(|s| std::mem::take(&mut *s.borrow_mut()));
    let patterns = state
        .patterns
        .into_iter()
        .map(|ps| {
            let PatternState { mut matches, ids, .. } = ps;
            // Joint canonical sort of (ids, triples) by id triple.
            let mut order: Vec<usize> = (0..ids.len()).collect();
            order.sort_unstable_by_key(|&i| ids[i]);
            matches.triples = order.iter().map(|&i| matches.triples[i].clone()).collect();
            matches
        })
        .collect();
    let mut filters = state.filters;
    for f in &mut filters {
        f.rows.sort_unstable();
    }
    ZkTrace { patterns, filters, steps: state.steps }
}

#[inline]
pub(crate) fn enabled() -> bool {
    ENABLED.with(|e| e.get())
}

/// `true` while inside a FILTER (NOT) EXISTS evaluation (recording armed):
/// per-row re-evaluations there suppress steps and filter obligations.
#[inline]
pub(crate) fn in_exists() -> bool {
    EXISTS_DEPTH.with(|d| d.get()) > 0
}

// ---- RAII scopes for the exec.rs hooks ----------------------------------------

/// Records `Enter(op)` now and `Exit(op)` on drop. No-op when the recorder is
/// disarmed (or inside EXISTS) at construction.
pub(crate) struct OpScope(Option<Op>);

pub(crate) fn op_scope(op: Op) -> OpScope {
    if !enabled() || in_exists() {
        return OpScope(None);
    }
    STATE.with(|s| s.borrow_mut().steps.push(Step::Enter(op)));
    OpScope(Some(op))
}

impl Drop for OpScope {
    fn drop(&mut self) {
        if let Some(op) = self.0 {
            STATE.with(|s| s.borrow_mut().steps.push(Step::Exit(op)));
        }
    }
}

/// [`op_scope`] for `GRAPH`: additionally pushes the named-graph context the
/// enclosed scans/filters are tagged with.
pub(crate) struct GraphScope(bool);

pub(crate) fn graph_scope(name: &Term) -> GraphScope {
    if !enabled() || in_exists() {
        return GraphScope(false);
    }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.steps.push(Step::Enter(Op::Graph));
        s.graph_ctx.push(name.clone());
    });
    GraphScope(true)
}

impl Drop for GraphScope {
    fn drop(&mut self) {
        if self.0 {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.graph_ctx.pop();
                s.steps.push(Step::Exit(Op::Graph));
            });
        }
    }
}

/// Marks a FILTER (NOT) EXISTS evaluation (suppression scope).
pub(crate) struct ExistsScope(bool);

pub(crate) fn exists_scope() -> ExistsScope {
    if !enabled() {
        return ExistsScope(false);
    }
    EXISTS_DEPTH.with(|d| d.set(d.get() + 1));
    ExistsScope(true)
}

impl Drop for ExistsScope {
    fn drop(&mut self) {
        if self.0 {
            EXISTS_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        }
    }
}

// ---- Recording entries (exec.rs hooks; armed recorder only) --------------------

/// Locates (or creates) the accumulating entry for `(pattern, current graph
/// ctx, in_exists)` and merges `matched` (id-level dedup; terms materialized
/// only for ids not seen before). Pushes a step unless inside EXISTS.
fn record_ids_with_key(graph: &Graph, pattern: PatternKey, matched: &[[Id; 3]], bind_join: bool) {
    let in_exists = in_exists();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let gctx = s.graph_ctx.last().cloned();
        let idx = match s.patterns.iter().position(|p| {
            p.matches.pattern == pattern && p.matches.graph == gctx && p.matches.in_exists == in_exists
        }) {
            Some(i) => i,
            None => {
                s.patterns.push(PatternState {
                    matches: PatternMatches { pattern, graph: gctx, in_exists, triples: Vec::new() },
                    ids: Vec::new(),
                    seen: HashSet::new(),
                });
                s.patterns.len() - 1
            }
        };
        let ps = &mut s.patterns[idx];
        for &spo in matched {
            if ps.seen.insert(spo) {
                ps.ids.push(spo);
                ps.matches.triples.push([
                    graph.dict.term(spo[0]),
                    graph.dict.term(spo[1]),
                    graph.dict.term(spo[2]),
                ]);
            }
        }
        if !in_exists {
            s.steps.push(if bind_join {
                Step::BindJoin { pattern: idx }
            } else {
                Step::Scan { pattern: idx }
            });
        }
    });
}

/// Id-level recording entry used by the `exec.rs` scan hooks: builds the
/// pattern key through the graph dictionary and merges the matched triples.
/// Only called when [`enabled`] — all materialization cost exists only under
/// an armed recorder.
pub(crate) fn record_scan_ids(
    graph: &Graph,
    id_pat: &sparq_core::store::Pattern,
    pos_vars: &[Option<Variable>; 3],
    matched: &[[Id; 3]],
    bind_join: bool,
) {
    let slot = |i: usize| -> SlotPattern {
        match (id_pat[i], &pos_vars[i]) {
            (Some(id), _) => SlotPattern::Term(graph.dict.term(id)),
            (None, Some(v)) => SlotPattern::Var(v.as_str().to_string()),
            (None, None) => SlotPattern::Wildcard,
        }
    };
    let pattern = PatternKey { slots: [slot(0), slot(1), slot(2)] };
    record_ids_with_key(graph, pattern, matched, bind_join);
}

/// Records a pattern that was answered WITHOUT a scan: an unsatisfiable
/// constant (a term not in the dictionary ⇒ provably empty input set) or a
/// pattern short-circuited by a sibling's emptiness. The key is built from
/// the algebra pattern, since unresolvable constants have no ids. `None`
/// keys (triple-term slots) are skipped — the [`Op::QuotedTriples`] marker
/// covers them.
pub(crate) fn record_empty_pattern(pattern: Option<PatternKey>) {
    let Some(pattern) = pattern else { return };
    let in_exists = in_exists();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let gctx = s.graph_ctx.last().cloned();
        let present = s.patterns.iter().any(|p| {
            p.matches.pattern == pattern && p.matches.graph == gctx && p.matches.in_exists == in_exists
        });
        if !present {
            s.patterns.push(PatternState {
                matches: PatternMatches { pattern, graph: gctx, in_exists, triples: Vec::new() },
                ids: Vec::new(),
                seen: HashSet::new(),
            });
        }
    });
}

/// The [`PatternKey`] of an algebra triple pattern (`None` for triple-term
/// slots — those take the [`Op::QuotedTriples`] marker path). Blank nodes in
/// query patterns are non-distinguished variables; they key as a reserved
/// variable name (the stage-1 fragment rejects them anyway).
pub(crate) fn key_of_algebra_pattern(tp: &spargebra::term::TriplePattern) -> Option<PatternKey> {
    use spargebra::term::{NamedNodePattern, TermPattern};
    let slot = |t: &TermPattern| -> Option<SlotPattern> {
        match t {
            TermPattern::NamedNode(n) => Some(SlotPattern::Term(Term::NamedNode(n.clone()))),
            TermPattern::Literal(l) => Some(SlotPattern::Term(Term::Literal(l.clone()))),
            TermPattern::Variable(v) => Some(SlotPattern::Var(v.as_str().to_string())),
            TermPattern::BlankNode(b) => Some(SlotPattern::Var(format!("#bnode#{}", b.as_str()))),
            _ => None,
        }
    };
    let predicate = match &tp.predicate {
        NamedNodePattern::NamedNode(n) => SlotPattern::Term(Term::NamedNode(n.clone())),
        NamedNodePattern::Variable(v) => SlotPattern::Var(v.as_str().to_string()),
    };
    Some(PatternKey { slots: [slot(&tp.subject)?, predicate, slot(&tp.object)?] })
}

/// Records one FILTER application: distinct operands + per-row operand
/// references and verdicts. The hook gates on `enabled() && !in_exists()`.
pub(crate) fn record_filter(
    expr: String,
    vars: &[Variable],
    operands: Vec<Option<Term>>,
    rows: Vec<(Vec<u32>, bool)>,
) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let graph = s.graph_ctx.last().cloned();
        let idx = s.filters.len();
        s.filters.push(FilterObligation {
            expr,
            vars: vars.iter().map(|v| v.as_str().to_string()).collect(),
            graph,
            operands,
            rows,
        });
        s.steps.push(Step::Filter { filter: idx });
    });
}

/// Memoized operand-table builder used by the apply_filter hook: one map
/// probe per cell, one term materialization per DISTINCT id.
pub(crate) struct OperandMemo {
    map: FxHashMap<Id, u32>,
    pub(crate) operands: Vec<Option<Term>>,
}

impl OperandMemo {
    pub(crate) fn new() -> Self {
        OperandMemo { map: FxHashMap::default(), operands: Vec::new() }
    }

    /// The operand index of `id`, materializing through `resolve` on first
    /// sight only.
    #[inline]
    pub(crate) fn index(&mut self, id: Id, resolve: impl FnOnce(Id) -> Option<Term>) -> u32 {
        if let Some(&i) = self.map.get(&id) {
            return i;
        }
        let i = self.operands.len() as u32;
        self.operands.push(resolve(id));
        self.map.insert(id, i);
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(turtle: &str) -> Graph {
        Graph::load_str(turtle, "turtle").unwrap()
    }

    const DATA: &str = r#"
        @prefix ex: <http://ex/> .
        ex:a ex:p ex:b . ex:b ex:p ex:c . ex:b ex:name "B" . ex:c ex:name "C" .
    "#;

    #[test]
    fn guard_clears_state() {
        let g = install();
        record_empty_pattern(Some(PatternKey {
            slots: [SlotPattern::Wildcard, SlotPattern::Wildcard, SlotPattern::Wildcard],
        }));
        drop(g);
        let g2 = install();
        let t = take();
        assert!(t.patterns.is_empty() && t.steps.is_empty());
        drop(g2);
    }

    #[test]
    fn query_records_post_scan_input_sets() {
        let g = graph(DATA);
        let _guard = install();
        let r = crate::query(
            &g,
            "SELECT ?n WHERE { <http://ex/a> <http://ex/p> ?x . ?x <http://ex/name> ?n }",
        )
        .unwrap();
        let t = take();
        assert_eq!(r.rows.len(), 1);
        assert_eq!(t.patterns.len(), 2);
        // The constant-subject pattern consumed exactly one triple.
        let p0 = t
            .patterns
            .iter()
            .find(|p| matches!(&p.pattern.slots[0], SlotPattern::Term(_)))
            .unwrap();
        assert_eq!(p0.triples.len(), 1);
        assert_eq!(p0.triples[0][2].to_string(), "<http://ex/b>");
        assert!(p0.graph.is_none() && !p0.in_exists);
    }

    #[test]
    fn trace_is_deterministic_across_runs() {
        let q = "SELECT * WHERE { ?s <http://ex/p> ?o . ?o <http://ex/name> ?n . FILTER(?n != \"B\") }";
        let run = || {
            let g = graph(DATA);
            let _guard = install();
            crate::query(&g, q).unwrap();
            take()
        };
        let (a, b) = (run(), run());
        assert_eq!(a, b, "same query + data must yield an identical trace");
    }
}
