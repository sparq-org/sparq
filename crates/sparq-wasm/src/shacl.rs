//! [OPUS-4.8] sq-yqi1 (#162): the `Store::validate(data, shapes)` SHACL binding.
//!
//! Exposes `sparq-shacl`'s existing public validation API
//! ([`sparq_shacl::validate`] — SHACL Core + SHACL-SPARQL §5.2) to JS/WASM
//! consumers as a drop-in for `rdf-validate-shacl` (PSS's ADR-0014
//! `ShaclValidator` seam). It does NOT reimplement validation — it loads the two
//! graphs exactly as [`Store::load`] does and calls straight through to the
//! engine, then hand-serialises the [`sparq_shacl::ValidationReport`] to a small
//! JSON string (the bundle has no serde, mirroring the SPARQL-JSON path).
//!
//! Two entry points share that one report serialiser:
//!
//! - [`Store::validate`] — **stateless** one-shot: both graphs arrive as strings
//!   and are re-parsed per call. The `rdf-validate-shacl` drop-in.
//! - [`Store::validate_store`] (`validateStore`) — **store-backed**: validates
//!   the triples ALREADY loaded in the receiver, so only the shapes document is
//!   parsed per call ([SONNET-4.6] gh-2520).
//!
//! This module compiles ONLY under the opt-in `shacl` feature, so the default
//! browser bundle carries zero SHACL code.
//!
//! ## Report shape (the JSON returned by `validate`)
//!
//! ```json
//! {
//!   "conforms": false,
//!   "results": [
//!     {
//!       "focusNode": "http://example.org/bob",
//!       "path": "<http://example.org/age>",
//!       "value": "\"-1\"^^<http://www.w3.org/2001/XMLSchema#integer>",
//!       "sourceShape": "_:b0",
//!       "sourceConstraintComponent": "http://www.w3.org/ns/shacl#MinInclusiveConstraintComponent",
//!       "severity": "http://www.w3.org/ns/shacl#Violation",
//!       "message": "Value is not >= 0"
//!     }
//!   ]
//! }
//! ```
//!
//! `path`, `value` and `message` are `null` when the result carries none.
//! `focusNode`/`value`/`sourceShape` are N-Triples term strings (the same lexical
//! form `Term`'s `Display` produces); `path` is a SHACL Turtle path expression.
//! `message` is the plain text of the shape's first `sh:message` (or the
//! generated default), suitable for surfacing directly in a UI.

use oxrdf::Term;
use sparq_core::Graph;
use sparq_shacl::{ValidationReport, ValidationResult};
use wasm_bindgen::prelude::*;

use crate::Store;

/// Appends `s` to `out` as a JSON string literal (quotes + minimal escaping).
/// The bundle carries no serde, so — exactly like the SPARQL-JSON serialiser —
/// strings are escaped by hand. Escapes the two mandatory characters (`"` and
/// `\`) plus the C0 control characters JSON forbids unescaped.
fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Appends `"key":<json-string>` (or `"key":null` when `val` is `None`).
fn push_field(out: &mut String, key: &str, val: Option<&str>) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":");
    match val {
        Some(v) => push_json_string(out, v),
        None => out.push_str("null"),
    }
}

/// The plain text of a result's first `sh:message`, or its generated default —
/// the human-readable string a UI surfaces (mirrors `to_text`'s message choice).
fn result_message(r: &ValidationResult) -> String {
    match r.messages.first() {
        Some(Term::Literal(l)) => l.value().to_string(),
        _ => r.default_message.clone(),
    }
}

/// Serialises a [`ValidationReport`] to the JS-facing JSON described in the
/// module docs. Kept here (not in `sparq-shacl`) so the report struct stays
/// serde-free and the wasm bundle keeps its hand-rolled-JSON convention.
fn report_to_json(report: &ValidationReport) -> String {
    let mut out = String::from("{\"conforms\":");
    out.push_str(if report.conforms { "true" } else { "false" });
    out.push_str(",\"results\":[");
    for (i, r) in report.results.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('{');
        push_field(&mut out, "focusNode", Some(&r.focus_node.to_string()));
        out.push(',');
        let path = r.path.as_ref().map(|p| p.to_turtle());
        push_field(&mut out, "path", path.as_deref());
        out.push(',');
        let value = r.value.as_ref().map(|v| v.to_string());
        push_field(&mut out, "value", value.as_deref());
        out.push(',');
        push_field(&mut out, "sourceShape", Some(&r.source_shape.to_string()));
        out.push(',');
        push_field(
            &mut out,
            "sourceConstraintComponent",
            Some(&r.source_component),
        );
        out.push(',');
        push_field(&mut out, "severity", Some(&r.severity));
        out.push(',');
        push_field(&mut out, "message", Some(&result_message(r)));
        out.push('}');
    }
    out.push_str("]}");
    out
}

#[wasm_bindgen]
impl Store {
    /// [OPUS-4.8] sq-yqi1 (#162): validates an RDF **data graph** against a SHACL
    /// **shapes graph**, returning a SHACL validation report as a JSON string.
    ///
    /// Both arguments are RDF documents in the same syntaxes [`Store::load`]
    /// accepts (`"turtle"` | `"ntriples"` | `"nquads"` | `"trig"`); they are
    /// parsed identically (named graphs folded into the default graph). This is a
    /// stateless one-shot — it does not consult the receiver's stored triples —
    /// so it is the drop-in replacement for `rdf-validate-shacl`'s
    /// `validate(dataDataset, { shapes })`: validation runs through
    /// `sparq-shacl`'s SHACL Core + SHACL-SPARQL (`sh:sparql`, §5.2) engine. To
    /// validate the triples the store already holds instead, use
    /// [`validate_store`](Self::validate_store) (`validateStore`).
    ///
    /// Returns a JSON object `{ conforms: boolean, results: [...] }`; each result
    /// has `focusNode`, `path`, `value`, `sourceShape`,
    /// `sourceConstraintComponent`, `severity` and `message` (see the module
    /// docs for the exact shape). `JSON.parse` it on the JS side. `sh:conforms`
    /// counts EVERY result regardless of severity (the W3C-suite notion); filter
    /// `results` by `severity` for a violations-only gate.
    ///
    /// Errors only if a graph fails to parse (a `JsError` carrying the parse
    /// error) — malformed shapes are skipped by the engine, never surfaced as an
    /// error. Small-document write-validation (~10–100 triples) sits far below
    /// the wasm linear-memory ceiling; very large data graphs should use the
    /// server-side HTTP `validate` path instead (#162 path (c)).
    ///
    /// The `data`/`shapes` arguments take ownership of two parameters; both
    /// graphs are dropped when the call returns.
    pub fn validate(&self, data: &str, shapes: &str, format: &str) -> Result<String, JsError> {
        let data_graph = Graph::load_str(data, format).map_err(|e| JsError::new(&e))?;
        let shapes_graph = Graph::load_str(shapes, format).map_err(|e| JsError::new(&e))?;
        let report = sparq_shacl::validate(&data_graph, &shapes_graph);
        Ok(report_to_json(&report))
    }

    /// [SONNET-4.6] gh-2520: validates the triples **already loaded in this
    /// store** against a SHACL shapes document, returning the same JSON report
    /// [`validate`](Self::validate) does.
    ///
    /// This is the stateful counterpart of [`validate`](Self::validate): the data
    /// graph is the receiver's own contents (whatever `load` / `loadDataset` /
    /// `update` / `applyDelta` left in it), so a repeat validation — the same
    /// store re-checked as shapes are edited — parses only the *shapes* document
    /// per call instead of re-parsing the data document every time. `shapes` is an
    /// RDF document in any syntax [`Store::load`] accepts (`"turtle"` |
    /// `"ntriples"` | `"nquads"` | `"trig"`); the report shape, `sh:conforms`
    /// semantics and error behaviour are identical to
    /// [`validate`](Self::validate)'s (only a shapes parse failure errors —
    /// malformed shapes are skipped by the engine, never surfaced). Given the same
    /// two documents the two methods report the same results, *up to blank-node
    /// labels*: parsing a shapes document mints fresh labels, so a `sourceShape`
    /// naming an anonymous property shape (`_:…`) differs between any two calls —
    /// of either method. Treat those labels as per-call identifiers, not stable keys.
    ///
    /// **Scope:** validation observes the store's **default graph** only. Triples
    /// loaded into named graphs by [`load_dataset`](Self::load_dataset) are not
    /// focus-node candidates or value nodes here — `load` folds named graphs into
    /// the default graph, so a store built that way validates in full. The wasm
    /// linear-memory ceiling still applies: validating a very large store is
    /// better done server-side (the `sparq-server` HTTP `validate` path).
    #[wasm_bindgen(js_name = validateStore)]
    pub fn validate_store(&self, shapes: &str, format: &str) -> Result<String, JsError> {
        let shapes_graph = Graph::load_str(shapes, format).map_err(|e| JsError::new(&e))?;
        let report = sparq_shacl::validate(&self.graph, &shapes_graph);
        Ok(report_to_json(&report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAPES: &str = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
        ex:PersonShape a sh:NodeShape ;
          sh:targetClass ex:Person ;
          sh:property [
            sh:path ex:age ;
            sh:datatype xsd:integer ;
            sh:minInclusive 0 ;
            sh:message "age must be a non-negative integer" ;
          ] ;
          sh:property [
            sh:path ex:name ;
            sh:minCount 1 ;
          ] .
    "#;

    /// The serialiser used by the wasm `validate` export runs natively, so the
    /// report-shape contract is tested here without a wasm runtime (the
    /// `JsError`-returning wasm wrapper itself cannot run off-wasm — `JsError::new`
    /// is a wasm-bindgen import that panics natively — so, exactly as the other
    /// wasm tests do, we exercise the engine call + the JSON serialiser it feeds).
    fn validate_json(data: &str) -> String {
        let data = Graph::load_str(data, "turtle").unwrap();
        let shapes = Graph::load_str(SHAPES, "turtle").unwrap();
        let report = sparq_shacl::validate(&data, &shapes);
        report_to_json(&report)
    }

    /// Conforming data: the report says `conforms: true` with an empty results array.
    #[test]
    fn conforming_report_json() {
        let json = validate_json(
            r#"
            @prefix ex: <http://example.org/> .
            ex:alice a ex:Person ; ex:age 30 ; ex:name "Alice" .
        "#,
        );
        assert_eq!(json, r#"{"conforms":true,"results":[]}"#, "{json}");
    }

    /// Violating data: a constraint violation surfaces with focusNode / path /
    /// severity / message — the fields PSS's `ShaclValidator` seam consumes.
    #[test]
    fn violating_report_json_has_focus_path_message() {
        let json = validate_json(
            r#"
            @prefix ex: <http://example.org/> .
            ex:bob a ex:Person ; ex:age -1 .
        "#,
        );
        // Not conforming, and at least the minInclusive (negative age) + minCount
        // (missing name) violations are present.
        assert!(json.starts_with(r#"{"conforms":false,"results":[{"#), "{json}");
        // The focus node is the offending instance, as an N-Triples IRI term.
        assert!(
            json.contains(r#""focusNode":"<http://example.org/bob>""#),
            "{json}"
        );
        // The age property shape's path + declared message surface.
        assert!(
            json.contains(r#""path":"<http://example.org/age>""#),
            "path present: {json}"
        );
        assert!(
            json.contains(r#""message":"age must be a non-negative integer""#),
            "declared message present: {json}"
        );
        // The offending value is reported as an N-Triples literal term.
        assert!(
            json.contains(r#""value":"\"-1\"^^<http://www.w3.org/2001/XMLSchema#integer>""#),
            "value present: {json}"
        );
        // Severity defaults to sh:Violation, and the constraint component is named.
        assert!(
            json.contains(r#""severity":"http://www.w3.org/ns/shacl#Violation""#),
            "{json}"
        );
        assert!(
            json.contains("MinInclusiveConstraintComponent"),
            "minInclusive component present: {json}"
        );
        assert!(
            json.contains("MinCountConstraintComponent"),
            "minCount component present: {json}"
        );
    }

    /// A result with no path / value (a node-shape minCount) emits explicit
    /// `null`s, and the generated default message is used when none is declared.
    #[test]
    fn null_path_and_value_and_default_message() {
        let shapes = Graph::load_str(
            r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            ex:S a sh:NodeShape ;
              sh:targetNode ex:x ;
              sh:property [ sh:path ex:p ; sh:minCount 1 ] .
        "#,
            "turtle",
        )
        .unwrap();
        // ex:x has no ex:p => minCount violation with no value node.
        let data = Graph::load_str(
            "@prefix ex: <http://example.org/> . ex:x a ex:Thing .",
            "turtle",
        )
        .unwrap();
        let report = sparq_shacl::validate(&data, &shapes);
        let json = report_to_json(&report);
        assert!(json.contains("\"conforms\":false"), "{json}");
        // minCount on a property shape: path IS present (the ex:p path), value is null.
        assert!(json.contains(r#""value":null"#), "value null: {json}");
        // A non-empty default message string (not null) is always emitted.
        assert!(!json.contains(r#""message":null"#), "{json}");
    }

    /// [SONNET-4.6] gh-2520: `validateStore` validates the store's OWN triples —
    /// the load-bearing difference from the stateless `validate`, whose data
    /// argument ignores the receiver. The `Ok` arm never touches `JsError::new`,
    /// so the binding itself runs natively here (the `Err` arm — a malformed
    /// shapes document — is covered by the wasm32 `tests/web.rs` twin).
    #[test]
    fn validate_store_reads_the_receivers_triples() {
        // A store holding VIOLATING data: bob has a negative age and no name.
        let store = Store::load(
            r#"
            @prefix ex: <http://example.org/> .
            ex:bob a ex:Person ; ex:age -1 .
        "#,
            "turtle",
        )
        .unwrap();
        let json = store.validate_store(SHAPES, "turtle").unwrap();
        assert!(json.contains(r#""conforms":false"#), "{json}");
        assert!(
            json.contains(r#""focusNode":"<http://example.org/bob>""#),
            "the receiver's own focus node: {json}"
        );
        assert!(
            json.contains(r#""message":"age must be a non-negative integer""#),
            "declared message: {json}"
        );
        assert!(
            json.contains("MinCountConstraintComponent"),
            "minCount (missing name): {json}"
        );

        // A store holding CONFORMING data reports conformance over the same shapes.
        let ok = Store::load(
            r#"
            @prefix ex: <http://example.org/> .
            ex:alice a ex:Person ; ex:age 30 ; ex:name "Alice" .
        "#,
            "turtle",
        )
        .unwrap();
        assert_eq!(
            ok.validate_store(SHAPES, "turtle").unwrap(),
            r#"{"conforms":true,"results":[]}"#
        );
    }

    /// Rewrites every `_:<label>` blank-node label to a positional `_:bN` so two
    /// reports can be compared for the equivalence the binding actually promises.
    /// Parsing a shapes document mints FRESH blank-node labels each time, so a
    /// `sourceShape` naming an anonymous property shape differs byte-wise between
    /// any two parses — including two calls of the same method. What must match is
    /// the report modulo that labelling.
    fn mask_bnode_labels(json: &str) -> String {
        let mut out = String::with_capacity(json.len());
        let mut labels: Vec<&str> = Vec::new();
        let mut rest = json;
        while let Some(at) = rest.find("_:") {
            out.push_str(&rest[..at]);
            let after = &rest[at + 2..];
            // The label runs to the first character that cannot appear in an
            // N-Triples blank-node label (in this JSON, always the closing quote).
            // `_`/`-`/`.` MUST be included: the `native-ttl` parser labels
            // anonymous nodes `__ttl_anon_<n>_<m>`, so an alphanumeric-only scan
            // would stop at the first `_` and mask nothing.
            let end = after
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
                .unwrap_or(after.len());
            let label = &after[..end];
            let idx = match labels.iter().position(|l| *l == label) {
                Some(i) => i,
                None => {
                    labels.push(label);
                    labels.len() - 1
                }
            };
            out.push_str("_:b");
            out.push_str(&idx.to_string());
            rest = &after[end..];
        }
        out.push_str(rest);
        out
    }

    /// The store-backed report equals the stateless one-shot's over the same
    /// documents (modulo shapes-graph blank-node labels) — the equivalence
    /// obligation that makes `validateStore` a parse-saving path rather than a
    /// second, subtly different validator.
    #[test]
    fn validate_store_matches_the_stateless_one_shot() {
        const DATA: &str = r#"
            @prefix ex: <http://example.org/> .
            ex:bob a ex:Person ; ex:age -1 .
            ex:alice a ex:Person ; ex:age 30 ; ex:name "Alice" .
        "#;
        let loaded = Store::load(DATA, "turtle").unwrap();
        let empty = Store::load("", "turtle").unwrap();
        let stateful = loaded.validate_store(SHAPES, "turtle").unwrap();
        let stateless = empty.validate(DATA, SHAPES, "turtle").unwrap();
        assert_eq!(
            mask_bnode_labels(&stateful),
            mask_bnode_labels(&stateless),
            "stateful={stateful} stateless={stateless}"
        );
        // The masking is not vacuous: the raw reports DO carry a blank-node label
        // (an anonymous property shape), so the comparison above is a real one.
        assert!(stateful.contains("\"sourceShape\":\"_:"), "{stateful}");

        // Repeat validation of the same store is stable (no state is consumed).
        assert_eq!(
            mask_bnode_labels(&loaded.validate_store(SHAPES, "turtle").unwrap()),
            mask_bnode_labels(&stateful),
        );
    }

    /// Pins the documented **scope** of `validateStore`: focus nodes come from the
    /// store's DEFAULT graph only. The same document loaded with `load` (which
    /// folds named graphs in) validates in full; loaded with `loadDataset` (which
    /// keeps them named) the named-graph triples are invisible to targeting.
    #[test]
    fn validate_store_targets_the_default_graph_only() {
        const TRIG: &str = r#"
            @prefix ex: <http://example.org/> .
            ex:g { ex:bob a ex:Person ; ex:age -1 . }
        "#;
        // `load` folds ex:g into the default graph => bob is a focus node.
        let folded = Store::load(TRIG, "trig").unwrap();
        let json = folded.validate_store(SHAPES, "turtle").unwrap();
        assert!(
            json.contains(r#""focusNode":"<http://example.org/bob>""#),
            "folded load must see the triples: {json}"
        );
        // `loadDataset` keeps ex:g named => nothing in the default graph to target.
        let dataset = Store::load_dataset(TRIG, "trig").unwrap();
        assert_eq!(
            dataset.validate_store(SHAPES, "turtle").unwrap(),
            r#"{"conforms":true,"results":[]}"#,
            "named-graph triples are out of scope for targeting"
        );
        // ...and the store really does hold them, in the named graph (the assertion
        // above is about scope, not an empty store). Note `size()` counts the
        // default graph, so it reads 0 here — the `GRAPH ?g` count is what proves
        // the triples are present.
        assert_eq!(dataset.size(), 0, "nothing landed in the default graph");
        assert_eq!(
            dataset
                .count("SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }")
                .unwrap(),
            2,
            "the named graph holds bob's two triples"
        );
    }

    /// The label masker maps distinct labels to distinct positional ids and keeps
    /// repeats consistent — otherwise the equivalence test above could pass on
    /// reports that genuinely disagree about which shape reported a result.
    #[test]
    fn mask_bnode_labels_is_a_consistent_renaming() {
        assert_eq!(
            mask_bnode_labels(r#"["_:aa","_:bb","_:aa","<http://x>"]"#),
            r#"["_:b0","_:b1","_:b0","<http://x>"]"#
        );
        // A whole label is consumed, including the `_`/`-`/`.` an N-Triples label
        // may carry (the `native-ttl` parser emits `__ttl_anon_<n>_<m>`).
        assert_eq!(
            mask_bnode_labels(r#"["_:__ttl_anon_44_1","_:a-b.c"]"#),
            r#"["_:b0","_:b1"]"#
        );
        // Two labellings of the same structure mask to the same string; a report
        // that swaps WHICH shape is named does not.
        assert_eq!(
            mask_bnode_labels("_:x _:y _:x"),
            mask_bnode_labels("_:p _:q _:p")
        );
        assert_ne!(
            mask_bnode_labels("_:x _:y _:x"),
            mask_bnode_labels("_:p _:q _:q")
        );
    }

    /// An EMPTY store conforms vacuously, and — the mirror of the above — the
    /// stateless `validate` on a LOADED store still ignores those stored triples.
    /// Together these pin which method reads what.
    #[test]
    fn empty_store_conforms_and_validate_stays_stateless() {
        let empty = Store::load("", "turtle").unwrap();
        assert_eq!(
            empty.validate_store(SHAPES, "turtle").unwrap(),
            r#"{"conforms":true,"results":[]}"#
        );
        // The store holds violating triples, but `validate`'s data argument is
        // conforming — so the stateless path reports conformance.
        let violating = Store::load(
            r#"
            @prefix ex: <http://example.org/> .
            ex:bob a ex:Person ; ex:age -1 .
        "#,
            "turtle",
        )
        .unwrap();
        let conforming = r#"
            @prefix ex: <http://example.org/> .
            ex:alice a ex:Person ; ex:age 30 ; ex:name "Alice" .
        "#;
        assert_eq!(
            violating.validate(conforming, SHAPES, "turtle").unwrap(),
            r#"{"conforms":true,"results":[]}"#
        );
    }

    /// The hand-rolled JSON escaper handles quotes, backslashes and control chars
    /// in a message so the document stays valid JSON.
    #[test]
    fn json_string_escaping() {
        let mut s = String::new();
        push_json_string(&mut s, "a\"b\\c\nd\te");
        assert_eq!(s, r#""a\"b\\c\nd\te""#, "{s}");
        // A C0 control char is \u-escaped (raw control chars are invalid in JSON).
        let mut ctrl = String::new();
        push_json_string(&mut ctrl, "\u{0001}");
        assert_eq!(ctrl, "\"\\u0001\"", "{ctrl}");
    }
}
