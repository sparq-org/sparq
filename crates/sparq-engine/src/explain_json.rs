//! Structured EXPLAIN: a typed plan tree with serde-free JSON, per-operator
//! q-error, and a bounded slow-query ring (`explain-json` feature, sq-u4lgr, #902).
//!
//! The human-readable [`crate::explain`] / [`crate::explain_analyze`] text remains
//! the default. This module exposes the SAME planning + ANALYZE information as a
//! MACHINE-READABLE tree:
//!
//! * [`PlanNode`] — a typed operator node: operator label, optional planner
//!   `estimated` cardinality, and (after ANALYZE) the `actual` output rows + wall
//!   `nanos`, plus the per-operator **q-error** = `max(est/actual, actual/est)`
//!   (1.0 = perfect; larger = a worse estimate, in either direction). The tree is
//!   reconstructed from the executor's pre-order operator trace (`exec::trace`)
//!   using each node's recorded depth.
//! * [`explain_plan`] — the planning-only dry run as a tree (no `actual`, no
//!   q-error: nothing ran).
//! * [`explain_plan_analyze`] — executes (SELECT/ASK) and fills in `actual` /
//!   `nanos` / `q_error` per node.
//! * [`PlanNode::to_json`] — a hand-written JSON projection (NO serde dependency,
//!   matching the crate's existing `json`/`serialize` writers), so the default
//!   build's dependency graph is byte-identical when this feature is off.
//! * [`SlowQueryRing`] — a bounded ring buffer keeping the N worst-by-wall-time
//!   recent analyzed plans, for an ops/admin slow-query view.
//!
//! ## Honest boundaries
//!
//! * The `estimated` cardinality is the engine's OWN index-range + GOO estimate
//!   (the number the text EXPLAIN already prints), attached only to BGP nodes —
//!   the only operators with a cardinality model. Non-BGP operators carry
//!   `estimated = None` and therefore no q-error; this is by design, not a gap.
//! * q-error is undefined when either side is 0 (an empty estimate or empty
//!   result); those nodes report `q_error = None`.
//! * On `wasm32` the wall `nanos` read 0 (no monotonic clock — `Instant` is
//!   unusable there) UNLESS the host installs a clock via [`set_trace_clock`]:
//!   the wasm binding routes `performance.now()` through it (sq-vx7ez, #2428),
//!   so the in-tab GUI plan explorer shows real per-operator times. Without an
//!   installed clock, `slowest`-by-time degenerates to row-count-only there.
//!
//! [OPUS-4.8]

use std::collections::VecDeque;
use std::fmt::Write as _;

use sparq_core::Graph;
use spargebra::{Query, SparqlParser};

use crate::exec;
use crate::QueryBudget;

/// One node of a structured query plan.
///
/// A `PlanNode` mirrors one operator in the executor's plan: its `operator` label,
/// the planner's `estimated` output cardinality (BGP nodes only — `None` elsewhere),
/// and, after [`explain_plan_analyze`], the `actual` output rows, wall-clock `nanos`,
/// and the computed `q_error`. `children` holds the operator's input subtrees in
/// evaluation order.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanNode {
    /// Human-readable operator label (e.g. `"BGP [binary GOO] (2 patterns, 0 filters)"`,
    /// `"LeftJoin (OPTIONAL)"`, `"Filter"`). The same labels the text trace prints.
    pub operator: String,
    /// The planner's estimated output cardinality, when the operator has a
    /// cardinality model (BGP nodes). `None` for operators the planner does not
    /// size (joins/filters/modifiers/etc.).
    pub estimated: Option<f64>,
    /// The actual output row count, filled in by [`explain_plan_analyze`]; `None`
    /// for a planning-only [`explain_plan`].
    pub actual: Option<usize>,
    /// The operator's wall-clock time in nanoseconds (ANALYZE only; 0 on `wasm32`
    /// unless a host clock is installed via [`set_trace_clock`]). `None` for a
    /// planning-only plan.
    pub nanos: Option<u64>,
    /// The per-operator q-error `max(est/actual, actual/est)` — 1.0 is a perfect
    /// estimate, larger means a worse estimate in either direction. `None` when no
    /// estimate exists, ANALYZE did not run, or either side is 0 (undefined).
    pub q_error: Option<f64>,
    /// Input subtrees, in evaluation order.
    pub children: Vec<PlanNode>,
}

impl PlanNode {
    /// The q-error of an `(estimated, actual)` pair: `max(e/a, a/e)`. `None` when
    /// either side is absent or 0 (the ratio is undefined).
    fn compute_q_error(estimated: Option<f64>, actual: Option<usize>) -> Option<f64> {
        let e = estimated?;
        let a = actual? as f64;
        if e <= 0.0 || a <= 0.0 {
            return None;
        }
        Some((e / a).max(a / e))
    }

    /// The largest q-error anywhere in this subtree (this node + descendants), or
    /// `None` if no node carries one. A single scalar "how wrong was the planner on
    /// its worst operator" for the whole plan.
    pub fn max_q_error(&self) -> Option<f64> {
        let mut worst = self.q_error;
        for c in &self.children {
            if let Some(cq) = c.max_q_error() {
                worst = Some(match worst {
                    Some(w) => w.max(cq),
                    None => cq,
                });
            }
        }
        worst
    }

    /// A hand-written JSON object for this node and its subtree (NO serde — matches
    /// the crate's existing `json` / `serialize` writers, so the default build pulls
    /// in no new dependency). Numeric fields are emitted as JSON numbers; absent
    /// optionals as `null`.
    pub fn to_json(&self) -> String {
        let mut s = String::new();
        self.write_json(&mut s);
        s
    }

    fn write_json(&self, s: &mut String) {
        s.push_str("{\"operator\":");
        write_json_string(s, &self.operator);
        s.push_str(",\"estimated\":");
        write_json_opt_f64(s, self.estimated);
        s.push_str(",\"actual\":");
        match self.actual {
            Some(a) => {
                let _ = write!(s, "{}", a);
            }
            None => s.push_str("null"),
        }
        s.push_str(",\"nanos\":");
        match self.nanos {
            Some(n) => {
                let _ = write!(s, "{}", n);
            }
            None => s.push_str("null"),
        }
        s.push_str(",\"qError\":");
        write_json_opt_f64(s, self.q_error);
        s.push_str(",\"children\":[");
        for (i, c) in self.children.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            c.write_json(s);
        }
        s.push_str("]}");
    }
}

/// Writes a JSON string literal with the minimal required escaping.
fn write_json_string(s: &mut String, v: &str) {
    s.push('"');
    for c in v.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(s, "\\u{:04x}", c as u32);
            }
            c => s.push(c),
        }
    }
    s.push('"');
}

/// Writes an `Option<f64>` as a JSON number (or `null`). A non-finite value (the
/// q-error never produces one, but be defensive) degrades to `null` rather than
/// emitting invalid JSON (`NaN`/`Infinity` are not JSON).
fn write_json_opt_f64(s: &mut String, v: Option<f64>) {
    match v {
        Some(x) if x.is_finite() => {
            // Integers print without a trailing `.0`; fractional values keep up to
            // 6 significant digits (estimates are products of integer range sizes —
            // more precision is noise).
            if x.fract() == 0.0 && x.abs() < 1e15 {
                let _ = write!(s, "{}", x as i64);
            } else {
                let _ = write!(s, "{:.6}", x);
            }
        }
        _ => s.push_str("null"),
    }
}

/// Builds a planning-only structured plan tree (a dry run — nothing is executed,
/// so every node's `actual` / `nanos` / `q_error` is `None`). Mirrors
/// [`crate::explain`] but returns a typed [`PlanNode`] instead of text.
///
/// Supports SELECT / ASK / CONSTRUCT / DESCRIBE (every query form, like the text
/// `explain`). Returns `Err` for a malformed query.
pub fn explain_plan(graph: &Graph, sparql: &str) -> Result<PlanNode, String> {
    let q = SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?;
    let active = crate::active_dataset(graph, &q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    let pattern = query_pattern(&q);
    Ok(plan_from_pattern(graph, pattern))
}

/// Builds a structured plan tree AND executes the query (SELECT / ASK only),
/// filling in each node's `actual` output rows, wall `nanos`, and per-operator
/// `q_error`. The ANALYZE analogue of [`explain_plan`]; mirrors
/// [`crate::explain_analyze`] but returns a typed [`PlanNode`].
pub fn explain_plan_analyze(graph: &Graph, sparql: &str) -> Result<PlanNode, String> {
    explain_plan_analyze_with_budget(graph, sparql, &QueryBudget::unlimited())
}

/// [`explain_plan_analyze`] under a cooperative [`QueryBudget`] (deadline / max rows).
pub fn explain_plan_analyze_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<PlanNode, String> {
    let q = SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?;
    let active = crate::active_dataset(graph, &q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    if !matches!(q, Query::Select { .. } | Query::Ask { .. }) {
        return Err("EXPLAIN ANALYZE supports SELECT and ASK queries only (use explain_plan for CONSTRUCT/DESCRIBE)".into());
    }

    // Execute under the budget with the operator trace installed (exactly as the
    // text `explain_analyze` does), then reconstruct the typed tree from the trace.
    let _bguard = exec::budget::install(budget);
    let _tguard = exec::trace::install();
    match &q {
        Query::Select { pattern, .. } => {
            exec::eval_select(graph, pattern)?;
        }
        Query::Ask { pattern, .. } => {
            exec::eval_ask(graph, pattern)?;
        }
        _ => unreachable!(),
    }
    let nodes = exec::trace::take();
    tree_from_trace(&nodes).ok_or_else(|| "empty execution trace".to_string())
}

/// Installs `clock` as THIS thread's ANALYZE trace clock: a monotonic reading in
/// **nanoseconds** (only differences are taken, so any epoch works). sq-vx7ez, #2428.
///
/// The hook exists for hosts without a monotonic `std::time::Instant` —
/// wasm32-unknown-unknown, where `Instant` panics and every ANALYZE wall time
/// otherwise reads 0. The `sparq-wasm` binding routes `performance.now()` (scaled
/// to nanos) through it so the structured [`explain_plan_analyze`] tree carries
/// real per-operator times in the browser. An installed clock takes precedence on
/// any target (which is what makes it unit-testable off-wasm); when none is
/// installed, native builds keep using `Instant` and nothing changes.
///
/// Per-thread and read ONLY while an ANALYZE trace stopwatch is running: queries
/// without a trace never touch it.
pub fn set_trace_clock(clock: fn() -> u64) {
    exec::trace::set_host_clock(clock);
}

/// The query's root graph pattern (independent of form).
fn query_pattern(q: &Query) -> &spargebra::algebra::GraphPattern {
    match q {
        Query::Select { pattern, .. }
        | Query::Ask { pattern, .. }
        | Query::Construct { pattern, .. }
        | Query::Describe { pattern, .. } => pattern,
    }
}

// ---- planning-only tree (no execution) ----------------------------------------

/// Builds a planning-only [`PlanNode`] tree from the algebra, attaching the BGP
/// cardinality estimate to conjunctive nodes (the only operators the planner sizes)
/// — the same estimate the text EXPLAIN prints, computed by replaying the planner
/// (`exec::bgp_estimate`).
fn plan_from_pattern(graph: &Graph, p: &spargebra::algebra::GraphPattern) -> PlanNode {
    use spargebra::algebra::GraphPattern as G;
    if exec::is_conjunctive(p) {
        return PlanNode {
            operator: exec::trace_label_pub(p),
            estimated: exec::bgp_estimate(graph, p).ok(),
            actual: None,
            nanos: None,
            q_error: None,
            children: Vec::new(),
        };
    }
    let label = exec::trace_label_pub(p);
    let children = match p {
        G::Filter { inner, .. }
        | G::Extend { inner, .. }
        | G::Graph { inner, .. }
        | G::Project { inner, .. }
        | G::Distinct { inner }
        | G::Reduced { inner }
        | G::Slice { inner, .. }
        | G::OrderBy { inner, .. }
        | G::Group { inner, .. } => vec![plan_from_pattern(graph, inner)],
        G::Join { left, right } | G::Union { left, right } | G::Minus { left, right } => {
            vec![plan_from_pattern(graph, left), plan_from_pattern(graph, right)]
        }
        G::LeftJoin { left, right, .. } => {
            vec![plan_from_pattern(graph, left), plan_from_pattern(graph, right)]
        }
        _ => Vec::new(),
    };
    PlanNode {
        operator: label,
        estimated: None,
        actual: None,
        nanos: None,
        q_error: None,
        children,
    }
}

// ---- ANALYZE tree (reconstructed from the pre-order operator trace) ------------

/// Reconstructs a `PlanNode` tree from the executor's pre-order, depth-tagged trace
/// nodes (the same `exec::trace::Node` list the text `explain_analyze` prints flat).
/// Each node's `q_error` is computed from its recorded `(estimated, actual)` pair.
fn tree_from_trace(nodes: &[exec::trace::Node]) -> Option<PlanNode> {
    if nodes.is_empty() {
        return None;
    }
    let mut idx = 0usize;
    Some(build_subtree(nodes, &mut idx))
}

/// Consumes one node at `*idx` and (recursively) every following node deeper than
/// it, in pre-order — the standard depth-stamped-preorder → tree reconstruction.
fn build_subtree(nodes: &[exec::trace::Node], idx: &mut usize) -> PlanNode {
    let here = &nodes[*idx];
    let my_depth = here.depth;
    let estimated = node_est(here);
    let actual = Some(here.rows);
    let q_error = PlanNode::compute_q_error(estimated, actual);
    let mut node = PlanNode {
        operator: here.label.clone(),
        estimated,
        actual,
        nanos: Some(here.nanos),
        q_error,
        children: Vec::new(),
    };
    *idx += 1;
    while *idx < nodes.len() && nodes[*idx].depth > my_depth {
        node.children.push(build_subtree(nodes, idx));
    }
    node
}

/// The recorded planner estimate for a trace node (always present under this
/// feature — the field exists only when `explain-json` is on).
fn node_est(n: &exec::trace::Node) -> Option<f64> {
    n.est
}

// ---- bounded slow-query ring --------------------------------------------------

/// A captured slow query: the SPARQL text, the analyzed plan tree, the total wall
/// time, and the result row count.
#[derive(Debug, Clone, PartialEq)]
pub struct SlowQuery {
    /// The query text that was executed.
    pub sparql: String,
    /// The analyzed plan tree (with per-operator actuals + q-errors).
    pub plan: PlanNode,
    /// Total wall-clock execution time in nanoseconds (0 on `wasm32` unless a
    /// host clock is installed via [`set_trace_clock`]).
    pub total_nanos: u64,
    /// The result row count.
    pub total_rows: usize,
}

/// A bounded ring buffer of the N slowest-by-wall-time recently analyzed queries.
///
/// Recording a query is O(capacity): if the buffer is full and the new query is
/// faster than every retained one, it is dropped; otherwise it replaces the
/// current fastest retained query, so the buffer always holds the N worst seen.
/// Intended for an ops/admin slow-query view (the server-side opt-in described in
/// #902); the engine provides the data structure, the server decides the policy.
///
/// On `wasm32` wall times are 0 — so the ring degenerates to "most recent N",
/// still useful (it keeps the latest analyzed plans) but not time-ordered —
/// unless a host clock is installed via [`set_trace_clock`].
#[derive(Debug, Clone)]
pub struct SlowQueryRing {
    capacity: usize,
    entries: VecDeque<SlowQuery>,
}

impl SlowQueryRing {
    /// A ring holding at most `capacity` queries (clamped to at least 1).
    pub fn new(capacity: usize) -> Self {
        SlowQueryRing {
            capacity: capacity.max(1),
            entries: VecDeque::new(),
        }
    }

    /// Analyzes `sparql` against `graph` and records it if it ranks among the N
    /// slowest seen. Returns the captured [`SlowQuery`] (also retained iff it made
    /// the cut), or `Err` if the query is not analyzable (not SELECT/ASK, or
    /// malformed). The plan is built via [`explain_plan_analyze`], so the query IS
    /// executed — call only where running the query is acceptable.
    pub fn record(&mut self, graph: &Graph, sparql: &str) -> Result<SlowQuery, String> {
        let sw = exec::trace::Stopwatch::start();
        let plan = explain_plan_analyze(graph, sparql)?;
        let total_nanos = sw.elapsed_nanos();
        let total_rows = plan.actual.unwrap_or(0);
        let q = SlowQuery {
            sparql: sparql.to_string(),
            plan,
            total_nanos,
            total_rows,
        };
        self.push(q.clone());
        Ok(q)
    }

    /// Inserts an already-measured [`SlowQuery`], keeping only the N slowest. Use
    /// this when the timing was measured elsewhere (e.g. the server already timed
    /// the request) rather than re-running via [`record`](Self::record).
    pub fn push(&mut self, q: SlowQuery) {
        if self.entries.len() < self.capacity {
            self.entries.push_back(q);
            return;
        }
        // Full: replace the current fastest retained query iff the newcomer is slower.
        let (min_i, min_nanos) = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (i, e.total_nanos))
            .min_by_key(|&(_, n)| n)
            .expect("non-empty when full");
        if q.total_nanos > min_nanos {
            self.entries[min_i] = q;
        }
    }

    /// The retained queries, slowest first.
    pub fn slowest(&self) -> Vec<&SlowQuery> {
        let mut v: Vec<&SlowQuery> = self.entries.iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.total_nanos));
        v
    }

    /// The number of retained queries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the ring currently holds no queries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// A hand-written JSON array of the retained queries, slowest first, each as
    /// `{"sparql":…,"totalNanos":…,"totalRows":…,"plan":{…}}` (NO serde dependency).
    pub fn to_json(&self) -> String {
        let mut s = String::from("[");
        for (i, q) in self.slowest().into_iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str("{\"sparql\":");
            write_json_string(&mut s, &q.sparql);
            let _ = write!(s, ",\"totalNanos\":{},\"totalRows\":{},\"plan\":", q.total_nanos, q.total_rows);
            q.plan.write_json(&mut s);
            s.push('}');
        }
        s.push(']');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = r#"
        @prefix ex: <http://ex/> .
        ex:alice ex:knows ex:bob ; ex:age 30 ; ex:name "Alice" .
        ex:bob   ex:knows ex:carol ; ex:age 25 ; ex:name "Bob" .
        ex:carol ex:age 35 ; ex:name "Carol" .
    "#;

    fn g() -> Graph {
        Graph::load_str(DATA, "turtle").unwrap()
    }

    #[test]
    fn plan_tree_shape_and_estimate() {
        // SELECT ?a ?b ?age WHERE { ?a ex:knows ?b . ?b ex:age ?age } — a Project over
        // a single conjunctive BGP. The planning-only tree: Project → BGP(2 patterns),
        // and the BGP node carries the planner's estimate, nothing else.
        let plan = explain_plan(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?a ?b ?age WHERE { ?a ex:knows ?b . ?b ex:age ?age }",
        )
        .unwrap();
        assert!(plan.operator.contains("Project"), "{}", plan.operator);
        assert_eq!(plan.children.len(), 1, "Project has one input");
        let bgp = &plan.children[0];
        assert!(bgp.operator.contains("BGP"), "{}", bgp.operator);
        assert!(bgp.children.is_empty(), "a flattened BGP is a leaf");
        // The planner estimates the join output (knows ⋈ age) — a positive number.
        let est = bgp.estimated.expect("BGP node carries an estimate");
        assert!(est > 0.0, "estimate should be positive: {est}");
        // Planning-only: no actuals, no q-error anywhere.
        assert!(bgp.actual.is_none() && bgp.q_error.is_none(), "no execution => no actual/q-error");
        assert!(plan.max_q_error().is_none(), "no q-error in a dry-run plan");
    }

    /// A host clock installed via [`set_trace_clock`] supplies the ANALYZE wall
    /// times (sq-vx7ez, #2428): every reading here is a multiple of the fake
    /// clock's 1000-nano tick, which `Instant` (nano-resolution) would virtually
    /// never produce for every node — so this pins that the trace clock really
    /// routed through the hook, on the wasm32 code path's exact mechanism.
    #[test]
    fn host_clock_overrides_analyze_trace_clock() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static TICKS: AtomicU64 = AtomicU64::new(0);
        fn fake_clock() -> u64 {
            TICKS.fetch_add(1_000, Ordering::Relaxed)
        }
        // Per-thread install; #[test] threads are fresh, so no leak across tests.
        set_trace_clock(fake_clock);
        let plan = explain_plan_analyze(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?a ?b WHERE { ?a ex:knows ?b . ?b ex:age ?age }",
        )
        .unwrap();
        fn assert_ticked(n: &PlanNode) {
            let nanos = n.nanos.expect("ANALYZE fills nanos");
            assert!(nanos > 0, "host clock ticks between enter/exit: {}", n.operator);
            assert_eq!(nanos % 1_000, 0, "reading comes from the fake clock, not Instant: {nanos}");
            n.children.iter().for_each(assert_ticked);
        }
        assert_ticked(&plan);
    }

    #[test]
    fn analyze_fills_actual_rows_and_q_error() {
        let plan = explain_plan_analyze(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?a ?b ?age WHERE { ?a ex:knows ?b . ?b ex:age ?age }",
        )
        .unwrap();
        // Find the BGP node (the conjunctive leaf) in the analyzed tree.
        let bgp = find(&plan, |n| n.operator.contains("BGP")).expect("a BGP node");
        assert_eq!(bgp.actual, Some(2), "the query yields 2 rows");
        assert!(bgp.nanos.is_some(), "ANALYZE records wall time");
        let est = bgp.estimated.expect("BGP node carries the planner estimate");
        let q = bgp.q_error.expect("BGP node with non-zero est+actual has a q-error");
        // q-error is exactly max(est/actual, actual/est) for this node.
        let expected = (est / 2.0).max(2.0 / est);
        assert!((q - expected).abs() < 1e-9, "q={q} expected={expected} est={est}");
        assert!(q >= 1.0, "q-error is always >= 1.0: {q}");
        // The whole-plan worst q-error is at least this node's.
        assert!(plan.max_q_error().unwrap() >= q - 1e-9);
    }

    #[test]
    fn q_error_none_on_empty_result() {
        // An unsatisfiable BGP (absent constant) → 0 actual rows AND 0 estimate;
        // q-error is undefined (None), not a divide-by-zero.
        let plan = explain_plan_analyze(&g(), "SELECT ?s WHERE { ?s <http://ex/nope> ?o }").unwrap();
        let bgp = find(&plan, |n| n.operator.contains("BGP")).expect("a BGP node");
        assert_eq!(bgp.actual, Some(0));
        assert!(bgp.q_error.is_none(), "0/0 q-error must be None, not NaN/Inf");
    }

    #[test]
    fn json_round_trips_via_serde_json() {
        // The hand-written JSON parses as valid JSON and preserves the tree shape +
        // every field. serde_json is a DEV-dependency only — the production writer
        // pulls in nothing.
        let plan = explain_plan_analyze(
            &g(),
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:age ?a OPTIONAL { ?s ex:knows ?k } }",
        )
        .unwrap();
        let json = plan.to_json();
        let v: serde_json::Value = serde_json::from_str(&json).expect("emitted JSON must parse");
        // Top-level fields present.
        assert!(v.get("operator").and_then(|o| o.as_str()).is_some());
        assert!(v.get("children").and_then(|c| c.as_array()).is_some());
        // Reconstruct a PlanNode from the parsed JSON and assert structural equality
        // (operator labels + estimated/actual/q-error + child arity) — a true round trip.
        let back = from_json_value(&v);
        assert_eq!(back, plan, "JSON must round-trip to an equal PlanNode tree");
    }

    #[test]
    fn slow_query_ring_keeps_worst_by_time() {
        // Synthetic queries with hand-set timings: the ring of capacity 2 keeps the
        // two slowest regardless of insertion order, slowest first.
        let mut ring = SlowQueryRing::new(2);
        let mk = |name: &str, nanos: u64| SlowQuery {
            sparql: name.to_string(),
            plan: PlanNode {
                operator: "BGP".into(),
                estimated: Some(1.0),
                actual: Some(1),
                nanos: Some(nanos),
                q_error: Some(1.0),
                children: vec![],
            },
            total_nanos: nanos,
            total_rows: 1,
        };
        ring.push(mk("fast", 10));
        ring.push(mk("slow", 100));
        ring.push(mk("medium", 50)); // full now: medium (50) > fast (10) → evicts fast.
        let worst = ring.slowest();
        assert_eq!(worst.len(), 2);
        assert_eq!(worst[0].sparql, "slow", "slowest first");
        assert_eq!(worst[1].sparql, "medium");
        // A query slower than nothing retained never displaces a slower one.
        ring.push(mk("tiny", 1));
        assert_eq!(ring.slowest()[0].sparql, "slow");
        assert_eq!(ring.len(), 2);
    }

    #[test]
    fn slow_query_ring_records_real_query() {
        let mut ring = SlowQueryRing::new(4);
        let q = ring
            .record(&g(), "PREFIX ex: <http://ex/> SELECT ?a ?b WHERE { ?a ex:knows ?b }")
            .unwrap();
        assert_eq!(q.total_rows, 2);
        assert_eq!(ring.len(), 1);
        // The recorded plan has real per-operator actuals (it executed the query).
        let bgp = find(&q.plan, |n| n.operator.contains("BGP")).unwrap();
        assert_eq!(bgp.actual, Some(2));
        // The ring's JSON is valid JSON.
        let v: serde_json::Value = serde_json::from_str(&ring.to_json()).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
        // CONSTRUCT is not analyzable → an honest Err, ring unchanged.
        assert!(ring.record(&g(), "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }").is_err());
        assert_eq!(ring.len(), 1);
    }

    // ---- test helpers ----

    /// Pre-order search for the first node matching `pred`.
    fn find(n: &PlanNode, pred: impl Fn(&PlanNode) -> bool + Copy) -> Option<&PlanNode> {
        if pred(n) {
            return Some(n);
        }
        for c in &n.children {
            if let Some(m) = find(c, pred) {
                return Some(m);
            }
        }
        None
    }

    /// Reconstructs a `PlanNode` from a parsed serde_json value (test-only — proves
    /// the hand-written JSON carries every field needed to rebuild the tree).
    fn from_json_value(v: &serde_json::Value) -> PlanNode {
        let opt_f64 = |k: &str| v.get(k).and_then(|x| if x.is_null() { None } else { x.as_f64() });
        PlanNode {
            operator: v["operator"].as_str().unwrap().to_string(),
            estimated: opt_f64("estimated"),
            actual: v.get("actual").and_then(|x| if x.is_null() { None } else { x.as_u64().map(|n| n as usize) }),
            nanos: v.get("nanos").and_then(|x| if x.is_null() { None } else { x.as_u64() }),
            q_error: opt_f64("qError"),
            children: v["children"].as_array().unwrap().iter().map(from_json_value).collect(),
        }
    }
}
