// sq-lfmf — headless wasm smoke test of the JS-facing sparq-shacl-wasm API.
//
// The crate's `#[cfg(test)] mod tests` in src/lib.rs runs on the NATIVE target only, so it
// exercises the sparq-shacl validate + serialiser functions the wrappers delegate to — NOT
// the actual `#[wasm_bindgen]` exports (`JsError::new` is a wasm-bindgen import that panics
// off-wasm). This module closes that gap: every test drives the REAL exported `Validator`
// API compiled to and executed in a genuine wasm32 runtime via `wasm-pack test --node`, so
// the SHACL validation + report serialisation across the JS boundary is asserted end to end
// — exactly the surface a `cargo build --target wasm32` never RUNS.
//
// `wasm-pack test --node` runs each `#[wasm_bindgen_test]` in the Node executor (no browser,
// no DOM); Node is the default target, so no `wasm_bindgen_test_configure!(run_in_browser)`
// directive is needed.

#![cfg(target_arch = "wasm32")]

use sparq_shacl_wasm::Validator;
use wasm_bindgen_test::*;

// The headline example from skills/shacl-validation/SKILL.md.
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

/// The headline demo: the datatype-violation example validates to a non-conforming JSON
/// report with exactly one result naming the offending value and message.
#[wasm_bindgen_test]
fn validate_json_reports_datatype_violation() {
    let json = Validator::validate(DATA, SHAPES, "turtle").expect("validate");
    assert!(
        json.contains("\"conforms\":false"),
        "report must not conform: {json}"
    );
    assert!(
        json.contains("\"focusNode\":\"<http://example.org/alice>\""),
        "{json}"
    );
    assert!(
        json.contains("\"message\":\"age must be an integer\""),
        "{json}"
    );
}

/// The Turtle rendering uses the SHACL report vocabulary.
#[wasm_bindgen_test]
fn validate_turtle_uses_report_vocabulary() {
    let ttl = Validator::validate_turtle(DATA, SHAPES, "turtle").expect("validate");
    assert!(ttl.contains("sh:ValidationReport"), "{ttl}");
    assert!(ttl.contains("sh:conforms false"), "{ttl}");
}

/// The text rendering is human-readable and names the non-conformance.
#[wasm_bindgen_test]
fn validate_text_is_human_readable() {
    let text = Validator::validate_text(DATA, SHAPES, "turtle").expect("validate");
    assert!(text.starts_with("Does not conform"), "{text}");
}

/// The conformance flag is false for the violating graph (both severity modes agree here:
/// the single result is a Violation).
#[wasm_bindgen_test]
fn conforms_flag_false_for_violation() {
    let all = Validator::conforms(DATA, SHAPES, "turtle", false).expect("conforms");
    let viol = Validator::conforms(DATA, SHAPES, "turtle", true).expect("conforms");
    assert!(!all, "sh:conforms must be false");
    assert!(!viol, "violations-only conformance must be false");
}

/// A conforming data graph yields an empty-results JSON report and conforms:true.
#[wasm_bindgen_test]
fn conforming_data_validates_clean() {
    let data = r#"@prefix ex: <http://example.org/> .
        ex:bob a ex:Person ; ex:age 30 ."#;
    let json = Validator::validate(data, SHAPES, "turtle").expect("validate");
    assert_eq!(json, "{\"conforms\":true,\"results\":[]}", "{json}");
    assert!(Validator::conforms(data, SHAPES, "turtle", false).expect("conforms"));
}

/// A malformed data document surfaces as a parse error (a `JsError`), not a panic.
#[wasm_bindgen_test]
fn malformed_data_is_an_error() {
    assert!(
        Validator::validate("@prefix bad", SHAPES, "turtle").is_err(),
        "unparseable data must error"
    );
}

// sq-01xlp: the opt-in `stateful` feature's pre-parsed `ParsedGraph` handle —
// parse once, validate many times. These drive the REAL wasm exports (including the
// `JsError`-returning `parse`, which the native suite cannot run) under
// `wasm-pack test --node --features stateful`.
#[cfg(feature = "stateful")]
mod stateful {
    use super::DATA;
    use sparq_shacl_wasm::{ParsedGraph, Validator};
    use wasm_bindgen_test::*;

    // A NAMED property shape: blank-node labels are randomized per parse, so an
    // anonymous shape would make the cross-parse byte-identical assertion below
    // flaky on `sourceShape`.
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
          sh:message "age must be an integer" .
    "#;

    /// Repeat validation off one pre-parsed data handle is deterministic and
    /// byte-identical to the stateless one-shot surface over the same documents.
    #[wasm_bindgen_test]
    fn parsed_handle_matches_one_shot_and_is_reusable() {
        let data = ParsedGraph::parse(DATA, "turtle").expect("parse data");
        let shapes = ParsedGraph::parse(SHAPES, "turtle").expect("parse shapes");
        let first = data.validate(&shapes);
        assert_eq!(
            first,
            Validator::validate(DATA, SHAPES, "turtle").expect("one-shot"),
            "pre-parsed and one-shot reports must agree"
        );
        assert_eq!(first, data.validate(&shapes), "repeat validation must be stable");
        assert!(!data.conforms(&shapes, false));
    }

    /// The Turtle / text renderings are the same report surface as the stateless API.
    #[wasm_bindgen_test]
    fn parsed_handle_full_report_surface() {
        let data = ParsedGraph::parse(DATA, "turtle").expect("parse data");
        let shapes = ParsedGraph::parse(SHAPES, "turtle").expect("parse shapes");
        assert!(data.validate_turtle(&shapes).contains("sh:ValidationReport"));
        assert!(data.validate_text(&shapes).starts_with("Does not conform"));
    }

    /// A malformed document surfaces as a parse error (a `JsError`), not a panic.
    #[wasm_bindgen_test]
    fn malformed_document_is_an_error() {
        assert!(
            ParsedGraph::parse("@prefix bad", "turtle").is_err(),
            "unparseable document must error"
        );
    }
}
