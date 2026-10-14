//! sq-ixc3.19: the `Store::explainPlanJson` / `Store::explainPlanAnalyzeJson`
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
//! are `null` in the planning-only dry run. On `wasm32` `nanos` reads 0 (no monotonic
//! clock — the same caveat as the text `explainAnalyze`); row counts and q-errors are
//! exact. Real per-operator wall times come from the desktop-native
//! (`explain_native` Tauri command) or server (`explain-format=json`) paths.
//!
//! This module compiles ONLY under the opt-in `explain-json` feature, so the default
//! (lean) browser bundle carries zero plan-tree code.

use wasm_bindgen::prelude::*;

use crate::Store;

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
    /// output rows, wall `nanos` (0 on `wasm32` — no monotonic clock), and `qError`
    /// (= max(est/actual, actual/est)) filled in.
    ///
    /// A CONSTRUCT / DESCRIBE / UPDATE query is rejected with a clear error — use
    /// [`explainPlanJson`](Self::explain_plan_json) for the graph-valued forms.
    #[wasm_bindgen(js_name = explainPlanAnalyzeJson)]
    pub fn explain_plan_analyze_json(&self, sparql: &str) -> Result<String, JsError> {
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
        // ANALYZE fills wall time as a number (0 is legal — and is what wasm32 reads).
        assert!(!json.contains("\"nanos\":null"), "got: {json}");
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
