//! sparq-shacl-wasm: the sparq **SHACL** validator compiled to WebAssembly — the
//! tier-b "W-shacl" bundle ([OPUS-4.8] sq-lfmf).
//!
//! This is a SEPARATE, lazy-loaded bundle from the lean `sparq-wasm` triplestore bundle.
//! It exposes SHACL Core + SHACL-SPARQL (`sh:sparql`, W3C SHACL §5.2) validation of a data
//! graph against a shapes graph — returning a W3C validation report as JSON, as report-RDF
//! Turtle, or as a human-readable text rendering — for in-tab live SHACL on the showcase
//! site's `/surface/shacl` page.
//!
//! The lean `sparq-wasm` bundle ALSO carries a minimal `Store::validate(...)` behind its
//! non-default `shacl` feature (sq-yqi1, #162) for the PSS ADR-0014 seam. This standalone
//! bundle is the showcase artifact: a stateless [`Validator`] with the FULL report surface
//! (JSON + W3C-vocabulary Turtle + plain text + a severity-filtered conformance flag), so
//! the page can render the per-violation report — e.g. the `ex:age "thirty"`
//! datatype-violation example from `skills/shacl-validation/SKILL.md` — without putting
//! SHACL code on the landing page.
//!
//! Like the lean bundle it carries no serde: the JSON report is serialised by hand
//! (mirroring the lean bundle's SHACL-JSON serialiser), and the Turtle / text renderings
//! come from `sparq-shacl`'s own `ValidationReport::to_turtle`/`to_text`.
//!
//! ```js
//! import init, { Validator } from "./sparq_shacl_wasm.js";
//! await init();
//! // The W3C report as JSON ({conforms, results:[...]}):
//! const report = JSON.parse(Validator.validate(dataTurtle, shapesTurtle, "turtle"));
//! // The same report as report-RDF Turtle (sh:ValidationReport vocabulary):
//! const ttl = Validator.validateTurtle(dataTurtle, shapesTurtle, "turtle");
//! // A human-readable rendering:
//! const text = Validator.validateText(dataTurtle, shapesTurtle, "turtle");
//! ```
//!
//! For **repeat validation without re-parse** (a large fixed data graph re-checked as
//! shapes are edited, or one shapes graph over many documents), the opt-in `stateful`
//! feature adds a pre-parsed `ParsedGraph` handle — parse once, validate many times.
//! OFF by default so the showcase artifact (and its deterministic bundle-bytes record)
//! is unchanged; see the `stateful` module and `research/shacl-wasm-stateful-2026-07.md`
//! (the sq-01xlp measurement + decision).
#![forbid(unsafe_code)] // [OPUS-4.8] sq-lfmf: crate has zero `unsafe`

use sparq_core::Graph;
use sparq_shacl::ValidationReport;
use wasm_bindgen::prelude::*;

// [FABLE-5] sq-01xlp: the opt-in pre-parsed/stateful validation handle. Behind the
// non-default `stateful` feature so the default showcase bundle carries zero extra
// surface; CI builds + tests the bundle in both feature states.
#[cfg(feature = "stateful")]
mod stateful;
#[cfg(feature = "stateful")]
pub use stateful::ParsedGraph;

/// Parses the two RDF documents (`data` and `shapes`) in `format` and validates, returning
/// the report. The single place both graphs are loaded the same way (the syntaxes
/// [`Graph::load_str`] accepts: `"turtle"` | `"ntriples"` | `"nquads"` | `"trig"`; named
/// graphs are folded into the default graph). Errors (as a `JsError`) only if a document
/// fails to parse — malformed shapes are skipped by the engine, never surfaced as an error.
fn run(data: &str, shapes: &str, format: &str) -> Result<ValidationReport, JsError> {
    let data_graph = Graph::load_str(data, format).map_err(|e| JsError::new(&e))?;
    let shapes_graph = Graph::load_str(shapes, format).map_err(|e| JsError::new(&e))?;
    Ok(sparq_shacl::validate(&data_graph, &shapes_graph))
}

/// The minimal in-tab SHACL validation entry point: validate an RDF data document against a
/// SHACL shapes document and return a W3C validation report in one of three renderings.
///
/// Every method is a stateless one-shot — it parses the two supplied documents, runs
/// validation, and returns a string — so the JS side never holds a long-lived handle.
#[wasm_bindgen]
pub struct Validator;

#[wasm_bindgen]
impl Validator {
    /// Validates the RDF `data` document against the SHACL `shapes` document (both in
    /// `format`) and returns the W3C validation report as a JSON string
    /// `{"conforms":bool,"results":[{...}]}`. Each result carries `focusNode`, `path`,
    /// `value`, `sourceShape`, `sourceConstraintComponent`, `severity` and `message`
    /// (`path`/`value` are `null` when the result carries none). `JSON.parse` it on the JS
    /// side.
    ///
    /// `conforms` counts EVERY result regardless of severity (the W3C-suite notion of
    /// `sh:conforms`); filter `results` by `severity` for a violations-only gate, or call
    /// [`conforms`](Self::conforms) directly. Errors only if a document fails to parse.
    pub fn validate(data: &str, shapes: &str, format: &str) -> Result<String, JsError> {
        Ok(run(data, shapes, format)?.to_js_json())
    }

    /// Like [`validate`](Self::validate) but returns the report as **report-RDF Turtle**
    /// using the SHACL validation-report vocabulary (`sh:ValidationReport`, `sh:result`,
    /// `sh:focusNode`, `sh:resultPath`, …) — the machine-readable W3C report. Valid Turtle,
    /// re-parseable back into a graph.
    #[wasm_bindgen(js_name = validateTurtle)]
    pub fn validate_turtle(data: &str, shapes: &str, format: &str) -> Result<String, JsError> {
        Ok(run(data, shapes, format)?.to_turtle())
    }

    /// Like [`validate`](Self::validate) but returns a **human-readable** rendering of the
    /// report (one line per result with severity, focus node, path, value and message) —
    /// what the demo page shows directly.
    #[wasm_bindgen(js_name = validateText)]
    pub fn validate_text(data: &str, shapes: &str, format: &str) -> Result<String, JsError> {
        Ok(run(data, shapes, format)?.to_text())
    }

    /// Validates and returns the boolean **conformance** flag. By default this is the
    /// W3C-suite `sh:conforms` (true iff there are zero results, regardless of severity).
    /// When `violations_only` is true it instead reports conformance ignoring
    /// `sh:Warning` / `sh:Info` (and custom) severities — the "violations-only" CI-gate
    /// toggle. Errors only if a document fails to parse.
    pub fn conforms(
        data: &str,
        shapes: &str,
        format: &str,
        violations_only: bool,
    ) -> Result<bool, JsError> {
        let report = run(data, shapes, format)?;
        Ok(if violations_only {
            report.conforms_violations_only()
        } else {
            report.conforms
        })
    }
}





#[cfg(test)]
mod tests {
    use super::*;

    // The headline example from skills/shacl-validation/SKILL.md: ex:age "thirty" violates
    // the xsd:integer datatype constraint.
    const DATA: &str = r#"
        @prefix ex: <http://example.org/> .
        ex:alice a ex:Person ; ex:age "thirty" .
    "#;
    const SHAPES: &str = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
        ex:PersonShape a sh:NodeShape ;
          sh:targetClass ex:Person ;
          sh:property [
            sh:path ex:age ;
            sh:datatype xsd:integer ;
            sh:message "age must be an integer" ;
          ] .
    "#;

    // Native unit tests exercise the engine + serialiser the #[wasm_bindgen] methods
    // delegate to. The `JsError`-returning wasm wrappers themselves cannot run natively
    // (`JsError::new` is a wasm-bindgen import that panics off-wasm), so — exactly as the
    // reason/rsp wasm bundles' tests do — we assert against the underlying calls. The
    // headless `wasm-pack test --node` suite (tests/web.rs) drives the real API.

    /// Build the report the wasm wrappers would, natively (the `JsError` shell aside).
    fn report() -> ValidationReport {
        let data = Graph::load_str(DATA, "turtle").unwrap();
        let shapes = Graph::load_str(SHAPES, "turtle").unwrap();
        sparq_shacl::validate(&data, &shapes)
    }

    /// The datatype violation is detected: the report does not conform and carries exactly
    /// one result naming the datatype constraint component and the offending value.
    #[test]
    fn datatype_violation_detected() {
        let report = report();
        assert!(
            !report.conforms,
            "ex:age \"thirty\" must violate xsd:integer"
        );
        assert_eq!(report.results.len(), 1);
        let r = &report.results[0];
        assert!(
            r.source_component.contains("Datatype"),
            "expected DatatypeConstraintComponent, got {}",
            r.source_component
        );
        assert_eq!(r.message_text(), "age must be an integer");
    }

    /// The JSON serialiser emits the documented report shape, with the offending value and
    /// message present and `conforms:false`.
    #[test]
    fn json_report_shape() {
        let json = report().to_js_json();
        assert!(
            json.starts_with("{\"conforms\":false,\"results\":["),
            "{json}"
        );
        assert!(
            json.contains("\"focusNode\":\"<http://example.org/alice>\""),
            "{json}"
        );
        assert!(json.contains("\"value\":\"\\\"thirty\\\"\""), "{json}");
        assert!(json.contains("\"sourceConstraintComponent\":"), "{json}");
        assert!(
            json.contains("\"severity\":\"http://www.w3.org/ns/shacl#Violation\""),
            "{json}"
        );
        assert!(
            json.contains("\"message\":\"age must be an integer\""),
            "{json}"
        );
    }

    /// A conforming data graph yields `conforms:true` and an empty results array.
    #[test]
    fn conforming_data_has_empty_results() {
        let data = Graph::load_str(
            r#"@prefix ex: <http://example.org/> .
               ex:bob a ex:Person ; ex:age 30 ."#,
            "turtle",
        )
        .unwrap();
        let shapes = Graph::load_str(SHAPES, "turtle").unwrap();
        let report = sparq_shacl::validate(&data, &shapes);
        assert!(report.conforms);
        assert_eq!(
            report.to_js_json(),
            "{\"conforms\":true,\"results\":[]}"
        );
    }

    /// The Turtle rendering uses the SHACL report vocabulary and is non-empty for a
    /// non-conforming graph.
    #[test]
    fn turtle_rendering_uses_report_vocabulary() {
        let ttl = report().to_turtle();
        assert!(ttl.contains("sh:ValidationReport"), "{ttl}");
        assert!(ttl.contains("sh:conforms false"), "{ttl}");
        assert!(ttl.contains("sh:result"), "{ttl}");
    }

    /// The text rendering names the non-conformance and the offending focus node.
    #[test]
    fn text_rendering_is_human_readable() {
        let text = report().to_text();
        assert!(text.starts_with("Does not conform"), "{text}");
        assert!(text.contains("http://example.org/alice"), "{text}");
    }
}
