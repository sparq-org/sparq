//! Continuous queries: the RSP-QL pipeline glued together.
//!
//! [`ContinuousQuery`] owns one windowed stream (the S2R operator), one
//! registered SPARQL SELECT (validated at [`register`](ContinuousQuery::register)
//! time), and one [`R2S`] relation-to-stream operator. Every push drives the
//! window; every window that closes is materialised into a
//! [`sparq_core::Graph`] (set semantics — duplicate triples in the window
//! collapse, as in any RDF graph) by the configured [`EvalMode`], evaluated by
//! `sparq_engine`, run through the R2S operator, and handed to the caller's
//! callback as a [`WindowResult`].
//!
//! [`ContinuousConstruct`] is the stream-to-stream sibling: a registered
//! CONSTRUCT whose per-window result is itself an RDF graph, delivered as
//! [`GraphResult`]s (with R2S set-diff semantics over the constructed
//! triples). [`ContinuousAsk`] delivers one boolean per window.
//!
//! # R2S semantics (exact multiset differences)
//!
//! SPARQL SELECT results are multisets of rows. The three operators:
//!
//! * **RSTREAM** — every window emits its FULL result. Stateless.
//! * **ISTREAM** — every window emits the rows *added* relative to the previous
//!   window's result: the multiset difference `cur ∖ prev`. The first window
//!   diffs against the empty multiset, i.e. emits everything.
//! * **DSTREAM** — the rows *removed*: `prev ∖ cur`. The first window emits
//!   nothing. Note DSTREAM needs empty windows to be reported (they are — see
//!   [`crate::window`]): a result only "disappears" when a later, possibly
//!   empty, window closes without it.
//!
//! Diffs compare complete rows and count them as multisets, so a row appearing
//! twice in `cur` and once in `prev` ISTREAMs exactly once. Emission order is
//! deterministic: ISTREAM preserves the engine's row order of the current
//! window, DSTREAM the row order of the previous window.
//!
//! CONSTRUCT results are triple SETS (the engine dedups), so
//! [`ContinuousConstruct`]'s ISTREAM/DSTREAM are exact set differences over
//! full triples — no hashing caveat.

use std::collections::HashMap;

use oxrdf::{Term, Triple, Variable};
use sparq_engine::PreparedQuery;
use spargebra::Query;

use crate::budget::BudgetSpec;
use crate::eval::{EvalMode, WindowEval};
use crate::window::WindowSpec;
use sparq_engine::QueryBudget;

/// The relation-to-stream operator: what part of each window's result is
/// streamed to the callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum R2S {
    /// The full result of every window (the RSP-QL default).
    #[default]
    RStream,
    /// Rows added relative to the previous window (multiset difference).
    IStream,
    /// Rows removed relative to the previous window (multiset difference).
    DStream,
}

/// One R2S-filtered SELECT result, delivered per closed window.
#[derive(Debug, Clone)]
pub struct WindowResult {
    /// Window bounds, as reported by the window operator: time windows are
    /// half-open `[start, end)`; count and session windows carry the inclusive
    /// `[first.ts, last.ts]` of their content (see `crate::window`).
    pub start: u64,
    /// See `start`.
    pub end: u64,
    /// Projected variables, in SELECT order.
    pub vars: Vec<Variable>,
    /// The emitted rows (full / added / removed per [`R2S`]); each row has one
    /// entry per `vars` position, `None` = unbound.
    pub rows: Vec<Vec<Option<Term>>>,
}

/// One R2S-filtered CONSTRUCT result (a graph), delivered per closed window.
#[derive(Debug, Clone)]
pub struct GraphResult {
    /// Window bounds (see [`WindowResult::start`]).
    pub start: u64,
    /// See `start`.
    pub end: u64,
    /// The constructed triples (full / added / removed per [`R2S`]), in the
    /// engine's first-production order; always a set (no duplicates).
    pub triples: Vec<Triple>,
}

/// One ASK result, delivered per closed window (RSTREAM semantics: every
/// window reports its boolean).
#[derive(Debug, Clone, Copy)]
pub struct AskResult {
    /// Window bounds (see [`WindowResult::start`]).
    pub start: u64,
    /// See `start`.
    pub end: u64,
    /// Whether the registered pattern has at least one solution in the window.
    pub value: bool,
}

/// A registered continuous SELECT: window spec + SPARQL + R2S operator.
///
/// ```
/// use oxrdf::{NamedNode, Term, Literal};
/// use sparq_rsp::{ContinuousQuery, WindowSpec};
///
/// let mut q = ContinuousQuery::register(
///     "SELECT (AVG(?v) AS ?avg) WHERE { ?s <http://ex/value> ?v }",
///     WindowSpec::time(10, 10), // tumbling: RANGE 10 STEP 10
/// ).unwrap();
///
/// let t = |n: i32| -> [Term; 3] {
///     [NamedNode::new_unchecked("http://ex/s").into(),
///      NamedNode::new_unchecked("http://ex/value").into(),
///      Literal::from(n).into()]
/// };
/// let mut results = Vec::new();
/// q.push(t(2), 1, |r| results.push(r)).unwrap();
/// q.push(t(4), 5, |r| results.push(r)).unwrap();
/// q.push(t(9), 12, |r| results.push(r)).unwrap(); // ts 12 closes [0,10)
/// assert_eq!(results.len(), 1);
/// // AVG(2,4) — integer averages come back as xsd:decimal per SPARQL typing.
/// let avg = Literal::new_typed_literal("3.0", oxrdf::vocab::xsd::DECIMAL);
/// assert_eq!(results[0].rows[0][0], Some(avg.into()));
/// ```
pub struct ContinuousQuery {
    sparql: String,
    /// Parsed once at registration; every window executes the prepared form
    /// (sparq-engine's parse/plan-once seam) instead of re-parsing `sparql`.
    prepared: PreparedQuery,
    eval: WindowEval,
    r2s: R2S,
    /// Previous window's full result, in engine row order —
    /// the ISTREAM/DSTREAM diff base. Unused (empty) under RSTREAM.
    prev_rows: Vec<Vec<Option<Term>>>,
    /// sq-xqu: the per-window-evaluation resource budget
    /// (unlimited by default).
    budget: BudgetSpec,
}

impl ContinuousQuery {
    /// Registers a continuous SELECT over a windowed stream, with the RSP-QL
    /// default output operator (RSTREAM) and the default [`EvalMode`]. The
    /// query string is parsed and validated NOW — a malformed or non-SELECT
    /// query is rejected at registration, not at the first window — and the
    /// parsed form is kept: every window executes the prepared algebra, no
    /// per-window re-parse. (For CONSTRUCT use [`ContinuousConstruct`], for
    /// ASK [`ContinuousAsk`].)
    pub fn register(sparql: &str, spec: WindowSpec) -> Result<Self, String> {
        let prepared = PreparedQuery::parse(sparql)?;
        if !matches!(prepared.query(), Query::Select { .. }) {
            return Err("continuous queries must be SELECT queries".into());
        }
        Ok(ContinuousQuery {
            sparql: sparql.to_owned(),
            prepared,
            eval: WindowEval::new(spec, EvalMode::default()),
            r2s: R2S::RStream,
            prev_rows: Vec::new(),
            budget: BudgetSpec::default(),
        })
    }

    /// Sets the relation-to-stream operator (builder style):
    /// `ContinuousQuery::register(q, spec)?.with_r2s(R2S::IStream)`.
    pub fn with_r2s(mut self, r2s: R2S) -> Self {
        self.r2s = r2s;
        self
    }

    /// sq-xqu: sets the per-window-evaluation resource budget
    /// (builder style). The budget is applied to EVERY window evaluation, and
    /// every limit in it is COOPERATIVE: `max_rows`, `max_bytes` and the
    /// `cancel` flag are observed at the executor's coarse polling sites, so
    /// they take effect at the NEXT poll rather than instantly — an evaluation
    /// that reaches no polling site (answered straight from the index, or
    /// finished before the first poll) runs to completion unchecked.
    /// `max_bytes` caps the executor-accounted ESTIMATED evaluation working
    /// set, not total process memory nor the memory of the materialised
    /// windows themselves. A `deadline` inside `budget` stays an ABSOLUTE
    /// instant (it does not reset per window — once it passes, every later
    /// window fails); for a deadline REFRESHED at each window evaluation use
    /// [`with_window_timeout`](Self::with_window_timeout). A tripped budget
    /// surfaces as the evaluation error of the `push`/`flush` that closed
    /// the window (`"query budget exceeded (…)"`), so — like any evaluation
    /// error — remaining closed windows are dropped.
    pub fn with_budget(mut self, budget: QueryBudget) -> Self {
        self.budget.base = budget;
        self
    }

    /// sq-xqu: installs a REFRESHED deadline for EACH window
    /// evaluation (builder style): at every window evaluation start the
    /// effective budget's deadline becomes the EARLIER of the budget's own
    /// absolute deadline (if any) and `now + timeout`, so one pathological
    /// window aborts with `"query budget exceeded (timeout)"` instead of
    /// stalling the embedder's push loop — and a refreshed window deadline
    /// never grants time past the absolute one. Like the other limits the
    /// deadline is observed CO-OPERATIVELY, at the executor's next polling
    /// site, so it is not a strict cap on an evaluation's wall-clock
    /// duration: an evaluation that reaches no polling site (answered from
    /// the index, or finished before the first poll) is never checked. A
    /// `timeout` so large
    /// that `now + timeout` is unrepresentable can never trip and is treated
    /// as unlimited (the budget's own absolute deadline, if any, still
    /// applies). Native only — the engine's wall-clock deadline does not
    /// exist on `wasm32` (where `std::time::Instant` is unusable); the
    /// row/byte/cancel limits of [`with_budget`](Self::with_budget) stay
    /// fully portable.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_window_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.budget.per_window_timeout = Some(timeout);
        self
    }

    /// Sets the window materialisation strategy (builder style). Call BEFORE
    /// the first push: switching modes resets the stream state (buffered open
    /// windows are discarded).
    pub fn with_mode(mut self, mode: EvalMode) -> Self {
        self.eval = WindowEval::new(self.eval.spec(), mode);
        self
    }

    /// The active window materialisation strategy.
    pub fn mode(&self) -> EvalMode {
        self.eval.mode()
    }

    /// The registered SPARQL text.
    pub fn sparql(&self) -> &str {
        &self.sparql
    }

    /// The window specification.
    pub fn spec(&self) -> WindowSpec {
        self.eval.spec()
    }

    /// Observability hook: the number of distinct non-inline terms
    /// currently held by the persistent dictionary (`None` outside
    /// [`EvalMode::PersistentDict`]). This is the quantity dictionary
    /// compaction bounds — for a long-running query over an unbounded
    /// vocabulary it tracks the live window content, not the all-time history.
    #[doc(hidden)]
    pub fn dict_len(&self) -> Option<usize> {
        self.eval.dict_len()
    }

    /// Pushes one stream element; `on_result` fires once per window this
    /// closes (zero or more times), oldest window first.
    ///
    /// Errors are evaluation errors from the engine (the query itself was
    /// validated at registration). On error, remaining closed windows are
    /// dropped.
    pub fn push(
        &mut self,
        triple: [Term; 3],
        ts: u64,
        mut on_result: impl FnMut(WindowResult),
    ) -> Result<(), String> {
        self.eval.push(triple, ts);
        self.emit(false, &mut on_result)
    }

    /// End-of-stream: closes every window up to the last timestamp seen
    /// (ignoring `max_delay`) and delivers the results. See
    /// [`WindowedStream::flush`](crate::WindowedStream::flush) for the exact
    /// closure rule.
    pub fn flush(&mut self, mut on_result: impl FnMut(WindowResult)) -> Result<(), String> {
        self.emit(true, &mut on_result)
    }

    /// Arrivals dropped as too late (every covering window already closed).
    pub fn late_dropped(&self) -> u64 {
        self.eval.late_dropped()
    }

    fn emit(&mut self, flush: bool, on_result: &mut impl FnMut(WindowResult)) -> Result<(), String> {
        let prepared = &self.prepared;
        let r2s = self.r2s;
        let prev_rows = &mut self.prev_rows;
        let budget = &self.budget;
        self.eval.eval_closed(flush, &mut |start, end, graph| {
            // sq-xqu: one effective budget per window evaluation
            // (the per-window timeout's deadline is measured from NOW).
            let result =
                sparq_engine::query_prepared_with_budget(graph, prepared, &budget.window_budget())?;
            let rows = match r2s {
                R2S::RStream => result.rows,
                R2S::IStream | R2S::DStream => {
                    diff_rows(r2s, result.rows, prev_rows)
                }
            };
            on_result(WindowResult { start, end, vars: result.vars, rows });
            Ok(())
        })
    }
}

/// A registered continuous CONSTRUCT: stream-to-stream transformation. Each
/// closed window's constructed graph is delivered as a [`GraphResult`];
/// ISTREAM/DSTREAM emit the triples added/removed relative to the previous
/// window's constructed graph (exact set difference — constructed results are
/// triple sets).
///
/// ```
/// use oxrdf::{NamedNode, Term};
/// use sparq_rsp::{ContinuousConstruct, WindowSpec};
///
/// // Re-emit each window's readings as a normalised observation graph.
/// let mut q = ContinuousConstruct::register(
///     "CONSTRUCT { ?s <http://ex/observed> ?v } WHERE { ?s <http://ex/value> ?v }",
///     WindowSpec::time(10, 10),
/// ).unwrap();
/// let t = [Term::from(NamedNode::new_unchecked("http://ex/s")),
///          NamedNode::new_unchecked("http://ex/value").into(),
///          NamedNode::new_unchecked("http://ex/v1").into()];
/// let mut out = Vec::new();
/// q.push(t, 1, |r| out.push(r)).unwrap();
/// q.flush(|r| out.push(r)).unwrap();
/// assert_eq!(out.len(), 1);
/// assert_eq!(out[0].triples.len(), 1);
/// assert_eq!(out[0].triples[0].predicate.as_str(), "http://ex/observed");
/// ```
pub struct ContinuousConstruct {
    sparql: String,
    /// Parsed once at registration; executed per window via the prepared seam.
    prepared: PreparedQuery,
    eval: WindowEval,
    r2s: R2S,
    /// Previous window's constructed graph (the ISTREAM/DSTREAM diff base).
    prev: Vec<Triple>,
    /// sq-xqu: the per-window-evaluation resource budget
    /// (unlimited by default).
    budget: BudgetSpec,
}

impl ContinuousConstruct {
    /// Registers a continuous CONSTRUCT over a windowed stream (RSTREAM, the
    /// default [`EvalMode`]). Malformed or non-CONSTRUCT queries are rejected
    /// at registration.
    pub fn register(sparql: &str, spec: WindowSpec) -> Result<Self, String> {
        let prepared = PreparedQuery::parse(sparql)?;
        if !matches!(prepared.query(), Query::Construct { .. }) {
            return Err("continuous construct queries must be CONSTRUCT queries".into());
        }
        Ok(ContinuousConstruct {
            sparql: sparql.to_owned(),
            prepared,
            eval: WindowEval::new(spec, EvalMode::default()),
            r2s: R2S::RStream,
            prev: Vec::new(),
            budget: BudgetSpec::default(),
        })
    }

    /// Sets the relation-to-stream operator (builder style).
    pub fn with_r2s(mut self, r2s: R2S) -> Self {
        self.r2s = r2s;
        self
    }

    /// sq-xqu: sets the per-window-evaluation resource budget
    /// (builder style). Semantics as
    /// [`ContinuousQuery::with_budget`](crate::ContinuousQuery::with_budget):
    /// applied to every window evaluation; a tripped budget is the evaluation
    /// error of the `push`/`flush` that closed the window.
    pub fn with_budget(mut self, budget: QueryBudget) -> Self {
        self.budget.base = budget;
        self
    }

    /// sq-xqu: installs a refreshed deadline for EACH window
    /// evaluation (tightened to at most `now + timeout` per window).
    /// Semantics as
    /// [`ContinuousQuery::with_window_timeout`](crate::ContinuousQuery::with_window_timeout);
    /// native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_window_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.budget.per_window_timeout = Some(timeout);
        self
    }

    /// Sets the window materialisation strategy (builder style). Call BEFORE
    /// the first push (switching resets the stream state).
    pub fn with_mode(mut self, mode: EvalMode) -> Self {
        self.eval = WindowEval::new(self.eval.spec(), mode);
        self
    }

    /// The registered SPARQL text.
    pub fn sparql(&self) -> &str {
        &self.sparql
    }

    /// The window specification.
    pub fn spec(&self) -> WindowSpec {
        self.eval.spec()
    }

    /// Pushes one stream element; `on_result` fires once per closed window.
    pub fn push(
        &mut self,
        triple: [Term; 3],
        ts: u64,
        mut on_result: impl FnMut(GraphResult),
    ) -> Result<(), String> {
        self.eval.push(triple, ts);
        self.emit(false, &mut on_result)
    }

    /// End-of-stream: closes every window up to the last timestamp seen.
    pub fn flush(&mut self, mut on_result: impl FnMut(GraphResult)) -> Result<(), String> {
        self.emit(true, &mut on_result)
    }

    /// Arrivals dropped as too late (every covering window already closed).
    pub fn late_dropped(&self) -> u64 {
        self.eval.late_dropped()
    }

    fn emit(&mut self, flush: bool, on_result: &mut impl FnMut(GraphResult)) -> Result<(), String> {
        let prepared = &self.prepared;
        let r2s = self.r2s;
        let prev = &mut self.prev;
        let budget = &self.budget;
        self.eval.eval_closed(flush, &mut |start, end, graph| {
            let cur = sparq_engine::construct_prepared_with_budget(
                graph,
                prepared,
                &budget.window_budget(),
            )?;
            let triples = match r2s {
                R2S::RStream => cur.clone(),
                // Constructed graphs are SETS: exact set difference, in the
                // surviving side's first-production order.
                R2S::IStream => set_minus(&cur, prev),
                R2S::DStream => set_minus(prev, &cur),
            };
            *prev = cur;
            on_result(GraphResult { start, end, triples });
            Ok(())
        })
    }
}

/// A registered continuous ASK: one boolean per closed window (RSTREAM
/// semantics — every window reports). The cheapest way to watch a stream for
/// a condition: the engine's ASK path early-exits on the first solution.
pub struct ContinuousAsk {
    sparql: String,
    /// Parsed once at registration; executed per window via the prepared seam.
    prepared: PreparedQuery,
    eval: WindowEval,
    /// sq-xqu: the per-window-evaluation resource budget
    /// (unlimited by default).
    budget: BudgetSpec,
}

impl ContinuousAsk {
    /// Registers a continuous ASK over a windowed stream (default
    /// [`EvalMode`]). Malformed or non-ASK queries are rejected at
    /// registration.
    pub fn register(sparql: &str, spec: WindowSpec) -> Result<Self, String> {
        let prepared = PreparedQuery::parse(sparql)?;
        if !matches!(prepared.query(), Query::Ask { .. }) {
            return Err("continuous ask queries must be ASK queries".into());
        }
        Ok(ContinuousAsk {
            sparql: sparql.to_owned(),
            prepared,
            eval: WindowEval::new(spec, EvalMode::default()),
            budget: BudgetSpec::default(),
        })
    }

    /// sq-xqu: sets the per-window-evaluation resource budget
    /// (builder style). Semantics as
    /// [`ContinuousQuery::with_budget`](crate::ContinuousQuery::with_budget):
    /// applied to every window evaluation; a tripped budget is the evaluation
    /// error of the `push`/`flush` that closed the window.
    pub fn with_budget(mut self, budget: QueryBudget) -> Self {
        self.budget.base = budget;
        self
    }

    /// sq-xqu: installs a refreshed deadline for EACH window
    /// evaluation (tightened to at most `now + timeout` per window).
    /// Semantics as
    /// [`ContinuousQuery::with_window_timeout`](crate::ContinuousQuery::with_window_timeout);
    /// native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_window_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.budget.per_window_timeout = Some(timeout);
        self
    }

    /// Sets the window materialisation strategy (builder style). Call BEFORE
    /// the first push (switching resets the stream state).
    pub fn with_mode(mut self, mode: EvalMode) -> Self {
        self.eval = WindowEval::new(self.eval.spec(), mode);
        self
    }

    /// The registered SPARQL text.
    pub fn sparql(&self) -> &str {
        &self.sparql
    }

    /// The window specification.
    pub fn spec(&self) -> WindowSpec {
        self.eval.spec()
    }

    /// Pushes one stream element; `on_result` fires once per closed window.
    pub fn push(
        &mut self,
        triple: [Term; 3],
        ts: u64,
        mut on_result: impl FnMut(AskResult),
    ) -> Result<(), String> {
        self.eval.push(triple, ts);
        self.emit(false, &mut on_result)
    }

    /// End-of-stream: closes every window up to the last timestamp seen.
    pub fn flush(&mut self, mut on_result: impl FnMut(AskResult)) -> Result<(), String> {
        self.emit(true, &mut on_result)
    }

    /// Arrivals dropped as too late (every covering window already closed).
    pub fn late_dropped(&self) -> u64 {
        self.eval.late_dropped()
    }

    fn emit(&mut self, flush: bool, on_result: &mut impl FnMut(AskResult)) -> Result<(), String> {
        let prepared = &self.prepared;
        let budget = &self.budget;
        self.eval.eval_closed(flush, &mut |start, end, graph| {
            let value =
                sparq_engine::ask_prepared_with_budget(graph, prepared, &budget.window_budget())?;
            on_result(AskResult { start, end, value });
            Ok(())
        })
    }
}

/// Applies ISTREAM/DSTREAM to a SELECT result: exact term-level multiset diff
/// of the current window's full result against the previous window's, then
/// advances the diff base.
// sq-2n1q3.3: made pub(crate) so ContinuousMultiQuery can reuse it.
pub(crate) fn diff_rows(
    r2s: R2S,
    cur_rows: Vec<Vec<Option<Term>>>,
    prev_rows: &mut Vec<Vec<Option<Term>>>,
) -> Vec<Vec<Option<Term>>> {
    let emitted = match r2s {
        // cur ∖ prev, in current row order.
        R2S::IStream => multiset_minus(&cur_rows, prev_rows),
        // prev ∖ cur, in previous row order.
        R2S::DStream => multiset_minus(prev_rows, &cur_rows),
        R2S::RStream => unreachable!("diff_rows() is only called for ISTREAM/DSTREAM"),
    };
    *prev_rows = cur_rows;
    emitted
}

/// `keep ∖ minus` as multisets of complete rows: each row in `minus` cancels at
/// most that many occurrences in `keep`; survivors are returned in `keep`'s
/// row order.
fn multiset_minus(
    keep_rows: &[Vec<Option<Term>>],
    minus: &[Vec<Option<Term>>],
) -> Vec<Vec<Option<Term>>> {
    let mut budget: HashMap<&[Option<Term>], usize> = HashMap::with_capacity(minus.len());
    for row in minus {
        *budget.entry(row.as_slice()).or_insert(0) += 1;
    }
    keep_rows
        .iter()
        .filter(|row| match budget.get_mut(row.as_slice()) {
            Some(n) if *n > 0 => {
                *n -= 1;
                false // cancelled by an occurrence on the other side
            }
            _ => true, // survives the difference: emit
        })
        .cloned()
        .collect()
}

/// `keep ∖ minus` as triple SETS, in `keep`'s order (CONSTRUCT results are
/// engine-deduplicated sets, so no multiset counting is needed).
fn set_minus(keep: &[Triple], minus: &[Triple]) -> Vec<Triple> {
    let minus: rustc_hash::FxHashSet<&Triple> = minus.iter().collect();
    keep.iter().filter(|t| !minus.contains(*t)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::{diff_rows, R2S};
    use oxrdf::{Literal, Term};

    fn row(value: &str) -> Vec<Option<Term>> {
        vec![Some(Literal::new_simple_literal(value).into()), None]
    }

    /// sq-ifv: cancellation is keyed by complete terms and retains
    /// the multiplicity and input order of rows that do not cancel.
    #[test]
    fn select_diff_uses_exact_term_rows() {
        let a = row("a");
        let b = row("b");
        let c = row("c");
        let mut previous = vec![a.clone(), b.clone(), b.clone()];

        let emitted = diff_rows(
            R2S::IStream,
            vec![b.clone(), c.clone(), b.clone(), c.clone()],
            &mut previous,
        );

        assert_eq!(emitted, vec![c.clone(), c]);
        assert_eq!(previous, vec![b.clone(), row("c"), b, row("c")]);
    }
}
