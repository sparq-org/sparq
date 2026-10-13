//! [FABLE-5] sq-01xlp: the opt-in **pre-parsed / stateful** validation handle.
//!
//! The default surface (`Validator`) is deliberately stateless one-shot: every call
//! re-parses both documents, which is the right shape for the showcase page's
//! paste-and-validate flow and keeps the default artifact byte-stable. At scale-tier
//! corpora, however, data-graph parsing dominates the one-shot cost (measured in
//! `research/shacl-wasm-stateful-2026-07.md`), so repeat validation — re-checking a
//! large fixed data graph while shapes are edited, or one shapes graph against many
//! documents — pays the dominant cost every call for no new information.
//!
//! `ParsedGraph` is that repeat-validation seam: parse once, validate many times,
//! with the same report surface as `Validator` (JSON / report-RDF Turtle / text /
//! severity-filtered conformance). Either side (data or shapes) of a validation can
//! be held; both arguments are `ParsedGraph` handles.
//!
//! This module compiles ONLY under the opt-in `stateful` feature so the default
//! showcase bundle (and its deterministic bundle-bytes record) is unchanged.
//!
//! ```js
//! import init, { ParsedGraph } from "./sparq_shacl_wasm.js"; // built --features stateful
//! await init();
//! const data = ParsedGraph.parse(bigDataTurtle, "turtle");   // parse ONCE
//! const shapesA = ParsedGraph.parse(shapesTurtleA, "turtle");
//! const reportA = JSON.parse(data.validate(shapesA));        // validate-only cost
//! const reportB = JSON.parse(data.validate(ParsedGraph.parse(shapesTurtleB, "turtle")));
//! data.free(); // wasm handles hold linear memory — free when finished
//! ```

use sparq_core::Graph;
use wasm_bindgen::prelude::*;


/// A pre-parsed RDF graph handle for **repeat validation without re-parse**
/// (opt-in `stateful` feature; the stateless `Validator` is the default surface).
///
/// Parse a document once with [`ParsedGraph::parse`], then validate it any number
/// of times against other handles. A handle owns wasm linear memory for the life
/// of the graph — call `.free()` from JS when finished with it.
#[wasm_bindgen]
pub struct ParsedGraph {
    pub(crate) graph: Graph,
}

#[wasm_bindgen]
impl ParsedGraph {
    /// Parses an RDF document into a reusable handle. `format` accepts the same
    /// syntaxes as the stateless surface (`"turtle"` | `"ntriples"` | `"nquads"` |
    /// `"trig"`; named graphs folded into the default graph). Errors (a `JsError`)
    /// only if the document fails to parse.
    pub fn parse(text: &str, format: &str) -> Result<ParsedGraph, JsError> {
        let graph = Graph::load_str(text, format).map_err(|e| JsError::new(&e))?;
        Ok(ParsedGraph { graph })
    }

    /// Validates this graph (as the **data** graph) against `shapes`, returning the
    /// W3C validation report as the same JSON string the stateless
    /// `Validator.validate` documents (`{"conforms":bool,"results":[...]}`).
    /// Infallible once both sides are parsed — malformed shapes are skipped by the
    /// engine, never surfaced as an error.
    pub fn validate(&self, shapes: &ParsedGraph) -> String {
        sparq_shacl::validate(&self.graph, &shapes.graph).to_js_json()
    }

    /// Like [`validate`](Self::validate) but returns the report as report-RDF
    /// Turtle in the SHACL validation-report vocabulary (`sh:ValidationReport`).
    #[wasm_bindgen(js_name = validateTurtle)]
    pub fn validate_turtle(&self, shapes: &ParsedGraph) -> String {
        sparq_shacl::validate(&self.graph, &shapes.graph).to_turtle()
    }

    /// Like [`validate`](Self::validate) but returns the human-readable rendering
    /// (one line per result).
    #[wasm_bindgen(js_name = validateText)]
    pub fn validate_text(&self, shapes: &ParsedGraph) -> String {
        sparq_shacl::validate(&self.graph, &shapes.graph).to_text()
    }

    /// Validates and returns the boolean conformance flag — the W3C-suite
    /// `sh:conforms` by default, or (with `violations_only`) conformance ignoring
    /// `sh:Warning` / `sh:Info` severities, exactly as the stateless
    /// `Validator.conforms` documents.
    pub fn conforms(&self, shapes: &ParsedGraph, violations_only: bool) -> bool {
        let report = sparq_shacl::validate(&self.graph, &shapes.graph);
        if violations_only {
            report.conforms_violations_only()
        } else {
            report.conforms
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The headline example (skills/shacl-validation/SKILL.md), with a NAMED property
    // shape: blank-node labels are randomized per parse, so an anonymous shape would
    // make the cross-parse byte-identical assertion below flaky on `sourceShape`.
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
          sh:property ex:AgeShape .
        ex:AgeShape
          sh:path ex:age ;
          sh:datatype xsd:integer ;
          sh:severity sh:Warning ;
          sh:message "age must be an integer" .
    "#;

    // `ParsedGraph::parse` returns `Result<_, JsError>`, so — per the crate's native-test
    // convention — the native suite constructs handles directly and exercises the
    // (infallible) validate surface; the headless `wasm-pack test --node` suite
    // (tests/web.rs, `--features stateful`) drives `parse` itself in a real wasm runtime.
    fn handle(text: &str) -> ParsedGraph {
        ParsedGraph {
            graph: Graph::load_str(text, "turtle").unwrap(),
        }
    }

    /// The JSON report off a pre-parsed pair is byte-identical to the stateless
    /// one-shot surface's report over the same documents.
    #[test]
    fn parsed_validate_matches_one_shot_json() {
        let json = handle(DATA).validate(&handle(SHAPES));
        let one_shot = sparq_shacl::validate(
            &Graph::load_str(DATA, "turtle").unwrap(),
            &Graph::load_str(SHAPES, "turtle").unwrap(),
        )
        .to_js_json();
        assert_eq!(json, one_shot, "{json}");
        assert!(json.contains("\"message\":\"age must be an integer\""), "{json}");
    }

    /// One data handle validates repeatedly — against the same and a different
    /// shapes handle — with deterministic results (the repeat-validation contract).
    #[test]
    fn handle_is_reusable_across_validations() {
        let data = handle(DATA);
        let shapes = handle(SHAPES);
        let first = data.validate(&shapes);
        let second = data.validate(&shapes);
        assert_eq!(first, second);
        let empty_shapes = handle("@prefix ex: <http://example.org/> .");
        assert_eq!(data.validate(&empty_shapes), "{\"conforms\":true,\"results\":[]}");
    }

    /// The Turtle and text renderings match the stateless surface's renderings.
    #[test]
    fn parsed_turtle_and_text_renderings() {
        let data = handle(DATA);
        let shapes = handle(SHAPES);
        let ttl = data.validate_turtle(&shapes);
        assert!(ttl.contains("sh:ValidationReport"), "{ttl}");
        assert!(ttl.contains("sh:conforms false"), "{ttl}");
        let text = data.validate_text(&shapes);
        assert!(text.starts_with("Does not conform"), "{text}");
        assert!(text.contains("http://example.org/alice"), "{text}");
    }

    /// The severity-filtered conformance toggle: the sh:Warning result fails the
    /// W3C `sh:conforms` but passes a violations-only gate.
    #[test]
    fn conforms_severity_toggle() {
        let data = handle(DATA);
        let shapes = handle(SHAPES);
        assert!(!data.conforms(&shapes, false), "sh:conforms counts every result");
        assert!(data.conforms(&shapes, true), "the only result is a Warning");
    }
}
