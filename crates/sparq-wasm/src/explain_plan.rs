//! [FABLE-5] sq-ixc3.19: the `Store::explainPlanJson` / `Store::explainPlanAnalyzeJson`
//! STRUCTURED-explain bindings.
//!
//! Exposes `sparq-engine`'s existing typed query-plan tree
//! ([`sparq_engine::explain_plan`] / [`sparq_engine::explain_plan_analyze`] — the
//! sq-u4lgr/#902 `explain_json::PlanNode`) to JS/WASM consumers as camelCase JSON. It
//! does NOT invent a schema — it calls straight through to the engine and returns
//! `PlanNode::to_json()` verbatim, so the browser sees exactly the shape the Rust API
//! (and the server's `explain-format=json` response) emits — the sq-jbqh4 schema
//! contract the GUI plan explorer (`gui/app` `plan` tool) renders:
//!
//! ```json
//! {
//!   "operator": "BGP [binary GOO] (2 patterns, 0 filters)",
//!   "estimated": 4,
//!   "actual": 2,
//!   "nanos": 18042,
//!   "qError": 2.0,
//!   "children": []
//! }
//! ```
//!
//! `estimated` is the planner's cardinality estimate (populated on BGP/conjunctive
//! leaves, `null` elsewhere); `actual` (output rows), `nanos` (wall time) and
//! `qError` (= max(est/actual, actual/est)) are filled only by the ANALYZE form and
//! are `null` in the planning-only dry run.
//!
//! **Wall times on `wasm32` ([FABLE-5] sq-vx7ez, #2428):** wasm32-unknown-unknown has
//! no monotonic clock (`std::time::Instant` panics), so the ANALYZE binding installs
//! `performance.now()` (browser + Node ≥ 16, sub-ms resolution) as the engine's
//! per-thread trace clock (`sparq_engine::set_trace_clock`) before executing — the
//! `nanos` the in-tab GUI plan explorer renders are REAL per-operator times, at the
//! host timer's resolution (browsers may coarsen `performance.now()` against timing
//! attacks; tiny operators can still legitimately read 0). The clock is bound directly
//! via `wasm-bindgen` (no new dependency, not even `js-sys`) and is only read while an
//! ANALYZE trace runs, so every other query path is untouched. The install is
//! per-thread and sticky: after the first ANALYZE call the text `explainAnalyze`
//! trace on the same (only) thread gets real times too.
//!
//! This module compiles ONLY under the opt-in `explain-json` feature, so the default
//! (lean) browser bundle carries zero plan-tree code.

use wasm_bindgen::prelude::*;

use crate::Store;

// [FABLE-5] sq-vx7ez (#2428): the host-clock bridge — `performance.now()` bound
// directly via wasm-bindgen (zero new dependencies; present in every browser and in
// Node ≥ 16, the two runtimes this bundle targets).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    /// `performance.now()` — fractional milliseconds since the time origin.
    #[wasm_bindgen(js_namespace = performance, js_name = now)]
    fn performance_now() -> f64;
}

/// The `fn() -> u64` nanosecond reading the engine's trace-clock hook expects
/// (only differences are taken, so the `performance` time origin is irrelevant).
#[cfg(target_arch = "wasm32")]
fn performance_now_nanos() -> u64 {
    (performance_now() * 1_000_000.0) as u64
}

#[wasm_bindgen]
impl Store {
    /// STRUCTURED planning-only `EXPLAIN` — the typed plan tree as camelCase JSON
    /// (`operator` / `estimated` / `actual` / `nanos` / `qError` / `children`).
    ///
    /// A dry run: nothing executes, so every node's `actual` / `nanos` / `qError` is
    /// `null` and only the planner's `estimated` cardinalities are populated. Works
    /// for every query form (SELECT / ASK / CONSTRUCT / DESCRIBE); a malformed query
    /// is rejected with the parser's error.
    #[wasm_bindgen(js_name = explainPlanJson)]
    pub fn explain_plan_json(&self, sparql: &str) -> Result<String, JsError> {
        sparq_engine::explain_plan(&self.graph, sparql)
            .map(|plan| plan.to_json())
            .map_err(|e| JsError::new(&e))
    }

    /// STRUCTURED `EXPLAIN ANALYZE` — executes the query (SELECT / ASK only) and
    /// returns the typed plan tree as camelCase JSON with each operator's `actual`
    /// output rows, wall `nanos` (real times, measured via the `performance.now()`
    /// host clock — see the module docs), and `qError` (= max(est/actual,
    /// actual/est)) filled in.
    ///
    /// A CONSTRUCT / DESCRIBE / UPDATE query is rejected with a clear error — use
    /// [`explainPlanJson`](Self::explain_plan_json) for the graph-valued forms.
    #[wasm_bindgen(js_name = explainPlanAnalyzeJson)]
    pub fn explain_plan_analyze_json(&self, sparql: &str) -> Result<String, JsError> {
        // Route the engine's trace clock through `performance.now()` so the trace
        // measures real wall time (`Instant` panics on wasm32). Idempotent
        // per-thread set; wasm is single-threaded.
        #[cfg(target_arch = "wasm32")]
        sparq_engine::set_trace_clock(performance_now_nanos);
        sparq_engine::explain_plan_analyze(&self.graph, sparql)
            .map(|plan| plan.to_json())
            .map_err(|e| JsError::new(&e))
    }
}

#[cfg(test)]
mod tests {
    use sparq_core::Graph;

    const DATA: &str = r#"@prefix ex: <http://ex/> .
        ex:alice ex:name "Alice" ; ex:age 30 ; ex:knows ex:bob .
        ex:bob ex:name "Bob"@en ; ex:age 25 ."#;

    // The bindings are thin wrappers over `JsError`-returning methods (host-untestable),
    // so the native tests exercise the SAME engine calls + `to_json()` serialisation the
    // wrappers delegate to — the schema contract is what the GUI depends on.

    #[test]
    fn plan_json_is_planning_only_and_camel_case() {
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let plan = sparq_engine::explain_plan(
            &g,
            "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }",
        )
        .unwrap();
        let json = plan.to_json();
        // The sq-jbqh4 schema contract: camelCase keys, `children` nesting.
        assert!(json.contains("\"operator\":"), "got: {json}");
        assert!(json.contains("\"children\":"), "got: {json}");
        // Planning-only: nothing executed, so no actual rows / wall time / q-error.
        assert!(json.contains("\"actual\":null"), "got: {json}");
        assert!(json.contains("\"nanos\":null"), "got: {json}");
        assert!(json.contains("\"qError\":null"), "got: {json}");
    }

    #[test]
    fn analyze_json_fills_actuals() {
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let plan = sparq_engine::explain_plan_analyze(
            &g,
            "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }",
        )
        .unwrap();
        // Both stores' names match ⇒ the root operator observed 2 output rows.
        assert_eq!(plan.actual, Some(2), "root actual rows");
        let json = plan.to_json();
        assert!(json.contains("\"actual\":2"), "got: {json}");
        // ANALYZE fills wall time as a number (0 is legal for sub-resolution operators).
        assert!(!json.contains("\"nanos\":null"), "got: {json}");
    }

    /// [FABLE-5] sq-vx7ez (#2428): the ANALYZE binding installs a host clock
    /// (`performance.now()` on wasm32) via `sparq_engine::set_trace_clock` before
    /// delegating. The extern can't run natively, so this drives the SAME hook with a
    /// fake nano counter and asserts the emitted `nanos` come from it (every reading
    /// a multiple of the 1000-nano tick — `Instant` would virtually never do that).
    #[test]
    fn analyze_json_nanos_come_from_installed_host_clock() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static TICKS: AtomicU64 = AtomicU64::new(0);
        fn fake_clock() -> u64 {
            TICKS.fetch_add(1_000, Ordering::Relaxed)
        }
        // Per-thread install (each #[test] runs on its own thread — no cross-test leak).
        sparq_engine::set_trace_clock(fake_clock);
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let plan = sparq_engine::explain_plan_analyze(
            &g,
            "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }",
        )
        .unwrap();
        let nanos = plan.nanos.expect("ANALYZE fills nanos");
        assert!(
            nanos > 0 && nanos.is_multiple_of(1_000),
            "host clock supplies the reading: {nanos}"
        );
    }

    #[test]
    fn analyze_json_rejects_graph_valued_forms() {
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let err = sparq_engine::explain_plan_analyze(
            &g,
            "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
        )
        .unwrap_err();
        assert!(err.contains("EXPLAIN ANALYZE supports"), "got: {err}");
    }
}
