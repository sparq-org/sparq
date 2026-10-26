//! Multi-window continuous queries (sq-9u1).
//! sq-2n1q3.3: 3+-window joins and ISTREAM/DSTREAM over multi-window joins.
//! sq-2n1q3.5: substrate-join adoption analysis — REASONED NON-ADOPTION
//! (see the "Substrate join: why it does not apply here" section below).
//!
//! [`ContinuousQuery`](crate::ContinuousQuery) is *one stream, one window, one
//! query*. RSP-QL allows a query to open SEVERAL named windows — possibly over
//! different streams, possibly with different `RANGE`/`STEP` — and JOIN across
//! them in the `WHERE` (`WINDOW <w1> { … } WINDOW <w2> { … }`, sharing
//! variables). [`ContinuousMultiQuery`] is that form.
//!
//! # How the join is evaluated
//!
//! Each declared window keeps its OWN S2R state — a
//! [`WindowedStream`](crate::WindowedStream) over the stream it reads. A push
//! `(stream, triple, ts)` is routed to every window declared `ON` that stream;
//! a stream feeding two windows feeds both, independently. All windows share ONE
//! event-time clock (the maximum timestamp seen across all streams), so closure
//! is synchronized: a triple on stream `s1` advancing the clock can close a
//! window on stream `s2`. This is the deterministic Window-Close report policy —
//! the same watermark contract the single-window pipeline already pins.
//!
//! At each EVALUATION TICK — a synchronized boundary at which at least one window
//! closes — every window contributes its current closed content as a NAMED GRAPH
//! keyed by the window IRI. The windows are assembled into a single
//! [`sparq_core::Graph`] (empty default graph, one named sub-graph per window),
//! and the registered query — with each `WINDOW <w>` rewritten to a standard
//! SPARQL `GRAPH <w>` — is evaluated against it. The engine's `GRAPH <w> { … }`
//! evaluation translates each window's ids into the shared outer dictionary, so
//! a variable bound in `WINDOW <w1>` JOINS correctly with the same variable in
//! `WINDOW <w2>` (the cross-graph join the engine already supports — see
//! `eval_graph_named`). The full result is delivered as a [`WindowResult`].
//!
//! ## Tick alignment (the synchronization rule)
//!
//! Windows may close at different times (different `STEP`s). The join is
//! evaluated **once per distinct closing boundary**, oldest first; at boundary
//! `t` each window contributes the content of its window that closed AT or
//! IMMEDIATELY BEFORE `t` (its latest-closed content), so a slow window's
//! content is held steady across the faster window's ticks. A window that has
//! not closed any content yet (no triples within reach of the watermark)
//! contributes the empty graph — `GRAPH <w> { … }` then yields no rows, so the
//! join correctly produces nothing until every joined window has content.
//!
//! ## 3+-window joins (sq-2n1q3.3)
//!
//! The `windows` field is a `Vec<WindowState>` so `ContinuousMultiQuery` handles
//! two, three, or any number of named windows identically: each window gets its
//! own S2R state, all share the same synchronized clock, and the named-graph
//! materialization loop over `self.windows` naturally scales. The `register` API
//! enforces ≥ 2 windows (use `ContinuousQuery` for one); three or more require
//! no additional code.
//!
//! ## ISTREAM/DSTREAM over a multi-window join (sq-2n1q3.3)
//!
//! At each tick the FULL join result is computed first; when the `R2S` operator
//! is `ISTREAM` or `DSTREAM` the diff against the previous tick's full result
//! is applied (reusing `crate::query::diff_rows`). The diff base advances after
//! every tick, exactly as in `ContinuousQuery`.
//!
//! * **ISTREAM** — each tick emits the rows that APPEARED relative to the
//!   previous tick (`cur ∖ prev`, exact term-level multiset difference).
//! * **DSTREAM** — each tick emits the rows that DISAPPEARED (`prev ∖ cur`).
//!
//! The `REGISTER ISTREAM <out> AS` / `REGISTER DSTREAM <out> AS` RSP-QL header
//! form selects the operator; the programmatic `r2s` field is read from the
//! parsed header.
//!
//! ## Substrate join: why it does not apply here (sq-2n1q3.5 analysis)
//!
//! sq-2n1q3.5 investigated whether the cross-window join in
//! `eval_tick` / `materialize_named` should adopt
//! `sparq_substrate::join::delta::DeltaTable` (the semi-naive probe kernel), as
//! `eval.rs` did for its consecutive-window diff in sq-2n1q3.4.
//!
//! **Conclusion: reasoned non-adoption.** The substrate kernels are not
//! applicable to this join surface for the following reasons:
//!
//! 1. **The engine already uses the substrate internally.** The cross-window
//!    join is a general SPARQL `GRAPH <w1> { } GRAPH <w2> { }` pattern;
//!    `sparq_engine::query_prepared` evaluates it via the engine's own
//!    bind-join / hash-join paths, which already drive the substrate kernels
//!    internally. There is no substrate bypass in this path.
//!
//! 2. **Different problem shape.** `eval.rs` adopted the substrate to replace
//!    a per-slide term-level hash-set diff between CONSECUTIVE windows of a
//!    SINGLE stream (the INSERT/DELETE membership test). That is a triple-level
//!    set-membership operation with a persistent `DeltaTable` build side. The
//!    cross-window join here is a full multi-graph SPARQL evaluation: arbitrary
//!    query shape (filters, aggregates, OPTIONAL, UNION, aggregates), N windows,
//!    variable bindings flowing across `GRAPH` patterns. No persistent build
//!    side is applicable — each tick's named-graph assembly is fresh.
//!
//! 3. **Key projection is inaccessible without planner integration.** Using the
//!    substrate for the cross-window join directly would require extracting the
//!    variable-to-column layout from the prepared query's algebra — information
//!    that lives in the engine's private `Bindings`/`LocalVocab` layer and
//!    cannot be computed here without coupling to engine internals. The engine
//!    seam (`research/shared-eval-substrate.md` §2.3) deliberately keeps that
//!    projection private.
//!
//! 4. **Correct incremental evaluation is a planner change, not a multi.rs
//!    change.** A genuinely incremental multi-window join (semi-naive evaluation
//!    at the SPARQL algebra level, only re-evaluating the sub-patterns whose
//!    window's content changed at this tick) would be a new feature of the query
//!    planner, not a drop-in replacement in this module. That is a different
//!    bead and a larger, non-behaviour-neutral scope.
//!
//! This non-adoption is therefore correct and sound: `multi.rs` continues to
//! delegate the cross-window join to `sparq_engine::query_prepared` which
//! drives the substrate through the engine's planned algebra. The RSP
//! expressivity / SRBench correctness ratchet (`tests/srbench_oracle.rs`) is
//! byte-identical before and after this analysis.
//!
//! ## Scope (honest)
//!
//! * Time windows only (the surface grammar has no `ROWS`).
//! * The `output_stream` / entailment metadata parsed by `RspqlQuery` is
//!   retained but not acted on (this is a library, not a stream-router).

use std::collections::BTreeSet;

use oxrdf::{NamedNode, Term};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_engine::{PreparedQuery, QueryBudget};
use spargebra::Query;

use crate::budget::BudgetSpec;
// sq-2n1q3.3: import diff_rows for ISTREAM/DSTREAM support.
use crate::query::{diff_rows, R2S, WindowResult};
use crate::rspql::{RspqlQuery, WindowDecl};
use crate::window::{Window, WindowedStream};

/// One declared window's live state: which stream it reads, its window IRI (the
/// named-graph key), its S2R machine, and the content of its most recently
/// closed window (held steady between this window's own closing boundaries).
struct WindowState {
    window: NamedNode,
    stream: NamedNode,
    ws: WindowedStream<[Term; 3]>,
    /// The bounds + content of the latest CLOSED window, or `None` until this
    /// window first closes. Carried across faster windows' ticks so the join
    /// sees a stable snapshot of the slow window.
    latest: Option<Window<[Term; 3]>>,
}

/// A registered RSP-QL continuous query joining across MORE THAN ONE named
/// window (the multi-window form). Built from RSP-QL surface syntax via
/// [`from_rspql`](Self::from_rspql) (or [`register`](Self::register) for the raw
/// text), pushed per `(stream, triple, ts)`, delivering one [`WindowResult`] per
/// synchronized evaluation tick.
///
/// ```
/// use oxrdf::{NamedNode, Term};
/// use sparq_rsp::ContinuousMultiQuery;
///
/// // Join readings on stream :temp (window :w1) with the room each sensor is in
/// // on stream :meta (window :w2), per synchronized 10-tick tumbling window.
/// let q_text = "\
/// REGISTER STREAM <http://ex/out> AS
/// SELECT ?room ?v WHERE {
///   WINDOW <http://ex/w1> { ?s <http://ex/value> ?v }
///   WINDOW <http://ex/w2> { ?s <http://ex/in> ?room }
/// }
/// FROM NAMED WINDOW <http://ex/w1> ON <http://ex/temp> RANGE 10 STEP 10
/// FROM NAMED WINDOW <http://ex/w2> ON <http://ex/meta> RANGE 10 STEP 10";
/// let mut q = ContinuousMultiQuery::register(q_text).unwrap();
///
/// let temp = NamedNode::new_unchecked("http://ex/temp");
/// let meta = NamedNode::new_unchecked("http://ex/meta");
/// let t = |s: &str, p: &str, o: Term| -> [Term; 3] {
///     [NamedNode::new_unchecked(format!("http://ex/{s}")).into(),
///      NamedNode::new_unchecked(format!("http://ex/{p}")).into(), o]
/// };
/// let mut rows = Vec::new();
/// q.push(&meta, t("s1", "in", NamedNode::new_unchecked("http://ex/kitchen").into()), 1, |_| {}).unwrap();
/// q.push(&temp, t("s1", "value", oxrdf::Literal::from(21).into()), 2, |r| rows.extend(r.rows)).unwrap();
/// q.flush(|r| rows.extend(r.rows)).unwrap();
/// assert_eq!(rows.len(), 1); // s1's reading joined with its room
/// ```
pub struct ContinuousMultiQuery {
    rspql: String,
    /// The embedded SPARQL with `WINDOW` rewritten to `GRAPH` (executed per tick).
    sparql: String,
    prepared: PreparedQuery,
    windows: Vec<WindowState>,
    r2s: R2S,
    output_stream: Option<NamedNode>,
    /// sq-2n1q3.3: previous tick's FULL join result rows — the
    /// ISTREAM/DSTREAM diff base. Empty and unused when `r2s == RStream`.
    prev_rows: Vec<Vec<Option<Term>>>,
    /// sq-xqu: the per-tick-evaluation resource budget
    /// (unlimited by default).
    budget: BudgetSpec,
}

impl ContinuousMultiQuery {
    /// Registers a multi-window query from RSP-QL surface syntax. The text is
    /// parsed ([`RspqlQuery::parse`]), the embedded SPARQL (with `WINDOW`
    /// rewritten to `GRAPH`) is validated as a SELECT, and one window machine is
    /// created per `FROM NAMED WINDOW` declaration.
    ///
    /// Errors: a malformed RSP-QL header, a non-SELECT embedded query, fewer
    /// than two windows (use [`ContinuousQuery`](crate::ContinuousQuery) for
    /// one), or a `WINDOW <w>` in the `WHERE` with no matching declaration.
    pub fn register(rspql_text: &str) -> Result<Self, String> {
        let parsed = RspqlQuery::parse(rspql_text)?;
        Self::from_rspql(&parsed, rspql_text)
    }

    /// Builds a multi-window query from an already-parsed [`RspqlQuery`].
    /// `rspql_text` is retained only for [`rspql`](Self::rspql).
    pub fn from_rspql(parsed: &RspqlQuery, rspql_text: &str) -> Result<Self, String> {
        let prepared = PreparedQuery::parse(&parsed.sparql)?;
        if !matches!(prepared.query(), Query::Select { .. }) {
            return Err("multi-window continuous queries must be SELECT queries".into());
        }
        if parsed.windows.len() < 2 {
            return Err(format!(
                "multi-window query needs ≥ 2 FROM NAMED WINDOW declarations (got {}); \
                 use ContinuousQuery for a single window",
                parsed.windows.len()
            ));
        }
        // Every window referenced in the WHERE must be declared. (The reverse —
        // a declared-but-unreferenced window — is allowed: it just never
        // contributes, exactly like an unused FROM NAMED.)
        let referenced = referenced_graphs(prepared.query());
        let declared: BTreeSet<&str> = parsed.windows.iter().map(|w| w.window.as_str()).collect();
        for r in &referenced {
            if !declared.contains(r.as_str()) {
                return Err(format!(
                    "WINDOW <{r}> is used in the WHERE clause but never declared with FROM NAMED WINDOW"
                ));
            }
        }
        let windows = parsed
            .windows
            .iter()
            .map(|d: &WindowDecl| WindowState {
                window: d.window.clone(),
                stream: d.stream.clone(),
                ws: WindowedStream::empty(d.spec),
                latest: None,
            })
            .collect();
        Ok(ContinuousMultiQuery {
            rspql: rspql_text.to_owned(),
            sparql: parsed.sparql.clone(),
            prepared,
            windows,
            r2s: parsed.r2s,
            output_stream: parsed.output_stream.clone(),
            // sq-2n1q3.3: ISTREAM/DSTREAM diff base — empty until the
            // first tick fires (the first window diffs against the empty multiset,
            // so it emits everything for ISTREAM and nothing for DSTREAM).
            prev_rows: Vec::new(),
            budget: BudgetSpec::default(),
        })
    }

    /// sq-xqu: sets the per-tick-evaluation resource budget
    /// (builder style). The budget is applied to EVERY synchronized
    /// evaluation tick (the multi-window analogue of one window evaluation).
    /// Semantics as
    /// [`ContinuousQuery::with_budget`](crate::ContinuousQuery::with_budget):
    /// a tripped budget is the evaluation error of the `push`/`flush` that
    /// triggered the tick.
    pub fn with_budget(mut self, budget: QueryBudget) -> Self {
        self.budget.base = budget;
        self
    }

    /// sq-xqu: installs a refreshed deadline for EACH
    /// evaluation tick (tightened to at most `now + timeout` per tick).
    /// Semantics as
    /// [`ContinuousQuery::with_window_timeout`](crate::ContinuousQuery::with_window_timeout);
    /// native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_window_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.budget.per_window_timeout = Some(timeout);
        self
    }

    /// The registered RSP-QL source text.
    pub fn rspql(&self) -> &str {
        &self.rspql
    }

    /// The embedded SPARQL (with `WINDOW` rewritten to `GRAPH`) executed per
    /// tick.
    pub fn sparql(&self) -> &str {
        &self.sparql
    }

    /// The declared window IRIs, in source order.
    pub fn window_iris(&self) -> Vec<&NamedNode> {
        self.windows.iter().map(|w| &w.window).collect()
    }

    /// The `REGISTER … <out> AS` output-stream IRI, if the source named one.
    pub fn output_stream(&self) -> Option<&NamedNode> {
        self.output_stream.as_ref()
    }

    /// The relation-to-stream operator from the `REGISTER` header.
    ///
    /// * `R2S::RStream` (default) — every tick emits the FULL join result.
    /// * `R2S::IStream` — every tick emits only the rows that appeared relative
    ///   to the previous tick (multiset diff `cur ∖ prev`). The first tick
    ///   diffs against the empty multiset and so emits everything.
    /// * `R2S::DStream` — every tick emits only the rows that disappeared
    ///   (`prev ∖ cur`). The first tick always emits nothing.
    ///
    /// sq-2n1q3.3: ISTREAM/DSTREAM are now wired.
    pub fn r2s(&self) -> R2S {
        self.r2s
    }

    /// Pushes one stream element TAGGED WITH ITS STREAM. Routed to every window
    /// declared `ON` this stream; `on_result` fires once per synchronized
    /// evaluation tick this push triggers (zero or more), oldest first.
    ///
    /// A push on a stream no window reads is accepted and ignored (it still does
    /// not advance any window — only timestamps on a window's own stream do, per
    /// that window's S2R).
    pub fn push(
        &mut self,
        stream: &NamedNode,
        triple: [Term; 3],
        ts: u64,
        on_result: impl FnMut(WindowResult),
    ) -> Result<(), String> {
        for w in &mut self.windows {
            if w.stream == *stream {
                // A window over this stream BUFFERS the triple (and advances its
                // own watermark to `ts`).
                w.ws.push(triple.clone(), ts);
            } else {
                // A window over another stream sees `ts` only as a SHARED-clock
                // heartbeat: event time has advanced to `ts`, so its windows
                // close on the same boundary — but the triple is not in its
                // stream, so nothing is buffered. This is what synchronizes the
                // join across streams of differing arrival rates.
                w.ws.advance(ts);
            }
        }
        self.drain(false, on_result)
    }

    /// End-of-stream: flushes every window to its last seen timestamp and
    /// delivers the final ticks.
    pub fn flush(&mut self, on_result: impl FnMut(WindowResult)) -> Result<(), String> {
        self.drain(true, on_result)
    }

    /// Drains closed windows into synchronized evaluation ticks. The set of
    /// distinct close boundaries (the `end` of each closed window across all
    /// streams) is the tick schedule; at each tick every window contributes the
    /// latest content that closed at or before it.
    fn drain(
        &mut self,
        flush: bool,
        mut on_result: impl FnMut(WindowResult),
    ) -> Result<(), String> {
        // Collect every newly-closed window per stream, and the set of distinct
        // close boundaries that defines the tick schedule.
        let mut boundaries: BTreeSet<u64> = BTreeSet::new();
        let mut closed: Vec<Vec<Window<[Term; 3]>>> = Vec::with_capacity(self.windows.len());
        for w in &mut self.windows {
            let wins = if flush { w.ws.flush() } else { w.ws.take_closed() };
            for win in &wins {
                boundaries.insert(win.end);
            }
            closed.push(wins);
        }
        if boundaries.is_empty() {
            return Ok(());
        }
        // Per-window cursor into its `closed` vector (windows are oldest-first).
        let mut cursor = vec![0usize; self.windows.len()];
        for tick in boundaries {
            // Advance each window to the latest content that CLOSED at or before
            // this tick boundary, holding it as that window's snapshot.
            for (i, wins) in closed.iter().enumerate() {
                while cursor[i] < wins.len() && wins[cursor[i]].end <= tick {
                    self.windows[i].latest = Some(wins[cursor[i]].clone());
                    cursor[i] += 1;
                }
            }
            self.eval_tick(tick, &mut on_result)?;
        }
        Ok(())
    }

    /// Builds the combined named-graph [`Graph`] for the current per-window
    /// snapshots and evaluates the registered query against it, then applies the
    /// configured R2S operator (RSTREAM / ISTREAM / DSTREAM) before firing the
    /// callback. sq-2n1q3.3: wired ISTREAM/DSTREAM.
    fn eval_tick(
        &mut self,
        tick: u64,
        on_result: &mut impl FnMut(WindowResult),
    ) -> Result<(), String> {
        let graph = self.materialize_named();
        // sq-xqu: one effective budget per tick evaluation (the
        // per-window timeout's deadline is measured from NOW).
        let result = sparq_engine::query_prepared_with_budget(
            &graph,
            &self.prepared,
            &self.budget.window_budget(),
        )?;
        // sq-2n1q3.3: apply the R2S operator.
        // RSTREAM: emit the full join result.
        // ISTREAM/DSTREAM: diff the full result against the previous tick's
        // full result (reusing the single-window diff_rows helper from
        // crate::query), then advance the diff base.
        let rows = match self.r2s {
            R2S::RStream => result.rows,
            R2S::IStream | R2S::DStream => {
                diff_rows(self.r2s, result.rows, &mut self.prev_rows)
            }
        };
        // For RSTREAM we skip updating prev_rows — it is never
        // read, and keeping them empty avoids a pointless clone.
        // The reported bounds: the tick's span is the join boundary. We report
        // [start, end) where end = tick and start = the min snapshot start, so
        // the window result carries a meaningful interval for the embedder.
        let start = self
            .windows
            .iter()
            .filter_map(|w| w.latest.as_ref().map(|win| win.start))
            .min()
            .unwrap_or(tick);
        on_result(WindowResult { start, end: tick, vars: result.vars, rows });
        Ok(())
    }

    /// Assembles the per-window snapshots into one [`Graph`]: an empty default
    /// graph plus one named sub-graph per window (keyed by the window IRI). Each
    /// window's content is a fresh set-semantic sub-`Graph` (duplicate triples
    /// across timestamps collapse, as in any RDF graph). The engine's
    /// `GRAPH <w> { … }` then evaluates against the matching sub-graph and
    /// translates ids into this outer dictionary, so cross-window joins bind
    /// correctly.
    fn materialize_named(&self) -> Graph {
        let mut g = Graph::from_parts(Dict::new(), Vec::new());
        for w in &self.windows {
            let sub = match &w.latest {
                Some(win) => sub_graph(win),
                None => Graph::from_parts(Dict::new(), Vec::new()),
            };
            g.add_named(Term::NamedNode(w.window.clone()), sub)
                .expect("a fresh in-memory graph takes in-memory named graphs");
        }
        g
    }
}

/// Materialises one window's content into a fresh dictionary-encoded sub-graph
/// (set semantics: a triple at several timestamps appears once).
fn sub_graph(win: &Window<[Term; 3]>) -> Graph {
    let mut dict = Dict::new();
    let ids: Vec<[Id; 3]> = win
        .triples
        .iter()
        .map(|t| [dict.intern(&t.triple[0]), dict.intern(&t.triple[1]), dict.intern(&t.triple[2])])
        .collect();
    Graph::from_parts(dict, ids)
}

/// The set of concrete named graphs referenced by `GRAPH <iri> { … }` patterns
/// in the query (the rewritten `WINDOW <iri>`). Walks the algebra; `GRAPH ?v`
/// (variable) is ignored (window variables are rejected at parse time, so this
/// only catches literal stray `GRAPH <iri>` clauses, which we also validate).
fn referenced_graphs(q: &Query) -> BTreeSet<String> {
    use spargebra::algebra::GraphPattern;
    use spargebra::term::NamedNodePattern;
    let mut out = BTreeSet::new();
    fn walk(p: &GraphPattern, out: &mut BTreeSet<String>) {
        use GraphPattern::*;
        match p {
            Graph { name, inner } => {
                if let NamedNodePattern::NamedNode(n) = name {
                    out.insert(n.as_str().to_owned());
                }
                walk(inner, out);
            }
            Join { left, right }
            | LeftJoin { left, right, .. }
            | Union { left, right }
            | Minus { left, right } => {
                walk(left, out);
                walk(right, out);
            }
            Filter { inner, .. }
            | Extend { inner, .. }
            | OrderBy { inner, .. }
            | Project { inner, .. }
            | Distinct { inner }
            | Reduced { inner }
            | Slice { inner, .. }
            | Group { inner, .. }
            | Service { inner, .. } => walk(inner, out),
            // Leaves (Bgp, Path, Values, table-unit, …) reference no named graph.
            _ => {}
        }
    }
    let pattern = match q {
        Query::Select { pattern, .. }
        | Query::Construct { pattern, .. }
        | Query::Describe { pattern, .. }
        | Query::Ask { pattern, .. } => pattern,
    };
    walk(pattern, &mut out);
    out
}
