//! [SONNET-4.6] sq-q4apb (#2396): the opt-in hosted-web
//! `Store.deriveForm(data, shapes, focus, format, optionsJson)` forms bridge.
//!
//! Exposes `sparq-forms`' existing SHACL-to-form derivation
//! ([`sparq_forms::derive_form`]) to the hosted `/app` web workbench. The GUI's
//! runtime adapter (`gui/app/src/lib/forms-bridge.ts`) selects exactly one host
//! per request: desktop uses the Tauri `derive_form` command, hosted web
//! feature-detects THIS method on the wasm `Store`. Both hosts return the
//! [`sparq_forms::FormDescription`] serde JSON **verbatim** — the frontend never
//! reconstructs keys, groups, or widgets — so this binding serialises with
//! `serde_json` exactly as the desktop bridge does (`gui/src-tauri/src/forms.rs`),
//! not with the crate's hand-rolled JSON helpers.
//!
//! The call is a stateless one-shot over serialized workspace snapshots (like
//! the shacl-feature `Store::validate` — not linked: it is behind an independent
//! feature): it does not consult the receiver's stored triples. `data`
//! and `shapes` are RDF documents in `format` (the workbench sends N-Quads),
//! parsed dataset-style so named graphs are preserved — identical to the desktop
//! bridge's `Graph::load_dataset`. `focus` is a structured RDF-node string (an
//! absolute IRI or a `_:`-prefixed blank-node label). `options_json` is the small
//! snake_case object `forms-bridge.ts`'s `formOptionsJson` produces:
//!
//! ```json
//! {"mode": "edit", "shape": "http://example.org/AuditShape"}
//! ```
//!
//! `mode` is `"edit"` or `"view"` (absent defaults to edit, matching
//! [`sparq_forms::FormOptions::default`]); `shape` optionally pins an explicit
//! node shape (IRI or `_:` blank-node string) instead of the first applicable
//! one. Unknown option keys are ignored (forward compatibility).
//!
//! This module compiles ONLY under the opt-in `forms` feature, so the default
//! (lean) browser bundle carries zero forms/serde code.

use oxrdf::Term;
use sparq_core::Graph;
use sparq_forms::{FormOptions, Mode};
use wasm_bindgen::prelude::*;

use crate::Store;

/// Parses a structured RDF-node string — an absolute IRI or a `_:`-prefixed
/// blank-node label — exactly as the desktop Tauri bridge does, so the two hosts
/// reject the same inputs with the same shape of error.
fn parse_node(value: &str, field: &str) -> Result<Term, String> {
    if let Some(label) = value.strip_prefix("_:") {
        return oxrdf::BlankNode::new(label)
            .map(Term::from)
            .map_err(|error| format!("Invalid {} blank node {:?}: {}", field, value, error));
    }
    oxrdf::NamedNode::new(value)
        .map(Term::from)
        .map_err(|error| format!("Invalid {} IRI {:?}: {}", field, value, error))
}

/// Parses the snake_case options JSON (`{"mode": …, "shape"?: …}`) into
/// [`FormOptions`]. Fail-closed on malformed input: a document that is not a JSON
/// object, an unsupported `mode`, or a non-string `shape` is an `Err` — never a
/// silent default derivation.
fn parse_options(options_json: &str) -> Result<FormOptions, String> {
    let value: serde_json::Value = serde_json::from_str(options_json)
        .map_err(|error| format!("Invalid deriveForm options JSON: {}", error))?;
    let object = value
        .as_object()
        .ok_or_else(|| "deriveForm options must be a JSON object.".to_string())?;

    let mode = match object.get("mode") {
        None | Some(serde_json::Value::Null) => Mode::Edit,
        Some(serde_json::Value::String(mode)) if mode == "edit" => Mode::Edit,
        Some(serde_json::Value::String(mode)) if mode == "view" => Mode::View,
        Some(other) => {
            return Err(format!(
                "Unsupported form mode {}; expected \"edit\" or \"view\".",
                other
            ));
        }
    };
    let shape = match object.get("shape") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(shape)) => Some(parse_node(shape, "shape")?),
        Some(other) => {
            return Err(format!(
                "Invalid shape option {}; expected an IRI or _:label string.",
                other
            ));
        }
    };
    Ok(FormOptions {
        mode,
        shape,
        ..FormOptions::default()
    })
}

/// The whole bridge as a plain function (the `#[wasm_bindgen]` export is one
/// `.map_err` away), so the contract is testable on the native target: parse the
/// two snapshots dataset-style, derive, and return the FormDescription serde JSON
/// verbatim — byte-identical to what the desktop Tauri bridge returns for the
/// same inputs.
pub(crate) fn derive_form_json(
    data: &str,
    shapes: &str,
    focus: &str,
    format: &str,
    options_json: &str,
) -> Result<String, String> {
    let data = Graph::load_dataset(data, format).map_err(|error| {
        format!(
            "Could not parse the workspace data graph as {}: {}",
            format, error
        )
    })?;
    let shapes = Graph::load_dataset(shapes, format).map_err(|error| {
        format!(
            "Could not parse the workspace shapes graph as {}: {}",
            format, error
        )
    })?;
    let focus = parse_node(focus, "focus")?;
    let options = parse_options(options_json)?;
    let description = sparq_forms::derive_form(&data, &shapes, &focus, &options);
    serde_json::to_string(&description).map_err(|error| {
        format!(
            "Could not serialize the derived form description: {}",
            error
        )
    })
}

#[wasm_bindgen]
impl Store {
    /// [SONNET-4.6] sq-q4apb (#2396): derives one complete `FormDescription`
    /// (SHACL-to-form, DASH widget scoring) from serialized workspace snapshots —
    /// the hosted-web half of the GUI forms bridge (desktop uses the Tauri
    /// `derive_form` command; `forms-bridge.ts` feature-detects this method).
    ///
    /// `data` / `shapes` are RDF documents in `format` (the syntaxes
    /// [`Store::load`] accepts; named graphs are preserved dataset-style).
    /// `focus` is an absolute IRI or a `_:`-prefixed blank-node label.
    /// `options_json` is the snake_case `{"mode": "edit"|"view", "shape"?: …}`
    /// object described in the module docs. Stateless one-shot: the receiver's
    /// stored triples are not consulted.
    ///
    /// Returns the derived `FormDescription` as its serde JSON string, verbatim
    /// (`JSON.parse` it on the JS side; the frontend must not reconstruct keys,
    /// groups, or widgets). Errors — an unparseable graph, focus, shape, or
    /// options document — throw a `JsError`; there is no demo-data fallback.
    ///
    /// Available only when the crate is built with the OPT-IN `forms` feature —
    /// the hosted `/app` + site bundle (js `build:wasm`) enables it; the lean
    /// default bundle does not, and there this method is simply absent (which is
    /// exactly what `forms-bridge.ts` feature-detects).
    #[wasm_bindgen(js_name = deriveForm)]
    pub fn derive_form(
        &self,
        data: &str,
        shapes: &str,
        focus: &str,
        format: &str,
        options_json: &str,
    ) -> Result<String, JsError> {
        derive_form_json(data, shapes, focus, format, options_json).map_err(|e| JsError::new(&e))
    }
}

#[cfg(test)]
mod tests {
    // The `#[wasm_bindgen]` export's `JsError` mapping cannot run off-wasm, so —
    // exactly as the shacl/policy/explain modules do — these native tests exercise
    // the `derive_form_json` function the export is one `.map_err` away from.

    use super::{derive_form_json, parse_options};

    // The same workspace fixtures the desktop Tauri bridge tests use
    // (gui/src-tauri/src/forms.rs), so the two hosts are pinned to one contract.
    const DATA: &str = r#"
        @prefix ex: <http://example.org/> .
        ex:alice a ex:Person ; ex:name "Alice" ; ex:audit "reviewed" .
    "#;

    const SHAPES: &str = r#"
        @prefix ex: <http://example.org/> .
        @prefix sh: <http://www.w3.org/ns/shacl#> .

        ex:PersonShape a sh:NodeShape ;
            sh:targetClass ex:Person ;
            sh:name "Person" ;
            sh:property [ sh:path ex:name ; sh:name "Name" ; sh:minCount 1 ] .

        ex:AuditShape a sh:NodeShape ;
            sh:name "Audit" ;
            sh:property [ sh:path ex:audit ; sh:name "Audit status" ] .
    "#;

    /// The FormDescription JSON serialised straight from the sparq-forms API —
    /// the bridge must return exactly this document, never a reconstruction.
    fn direct_description(mode: sparq_forms::Mode, shape: Option<&str>) -> String {
        let data =
            sparq_core::Graph::load_dataset(DATA, "turtle").expect("data fixture parses");
        let shapes =
            sparq_core::Graph::load_dataset(SHAPES, "turtle").expect("shapes fixture parses");
        let focus =
            oxrdf::Term::from(oxrdf::NamedNode::new_unchecked("http://example.org/alice"));
        let options = sparq_forms::FormOptions {
            mode,
            shape: shape.map(|iri| oxrdf::Term::from(oxrdf::NamedNode::new_unchecked(iri))),
            ..sparq_forms::FormOptions::default()
        };
        serde_json::to_string(&sparq_forms::derive_form(&data, &shapes, &focus, &options))
            .expect("direct FormDescription serializes")
    }

    /// Edit and view modes both return the direct serde serialization verbatim,
    /// with the mode-dependent editability reflected in the document.
    #[test]
    fn applicable_shape_edit_and_view_match_direct_serialization() {
        for (mode_text, mode) in [
            ("edit", sparq_forms::Mode::Edit),
            ("view", sparq_forms::Mode::View),
        ] {
            let options = format!("{{\"mode\":\"{}\"}}", mode_text);
            let actual = derive_form_json(
                DATA,
                SHAPES,
                "http://example.org/alice",
                "turtle",
                &options,
            )
            .expect("applicable shape derives");
            assert_eq!(actual, direct_description(mode, None));

            let json: serde_json::Value =
                serde_json::from_str(&actual).expect("bridge returns JSON");
            assert_eq!(json["mode"], mode_text);
            assert_eq!(json["shape"]["value"], "http://example.org/PersonShape");
            assert_eq!(json["groups"][0]["fields"][0]["label"], "Name");
            assert_eq!(
                json["groups"][0]["fields"][0]["editable"],
                mode_text == "edit"
            );
        }
    }

    /// An explicit `shape` option pins the derivation to that shape (it need not
    /// target the focus node), matching the direct API call.
    #[test]
    fn explicit_shape_request_matches_direct_serialization() {
        const AUDIT_SHAPE: &str = "http://example.org/AuditShape";
        let options = format!("{{\"mode\":\"edit\",\"shape\":\"{}\"}}", AUDIT_SHAPE);
        let actual = derive_form_json(
            DATA,
            SHAPES,
            "http://example.org/alice",
            "turtle",
            &options,
        )
        .expect("explicit shape derives");
        assert_eq!(
            actual,
            direct_description(sparq_forms::Mode::Edit, Some(AUDIT_SHAPE))
        );

        let json: serde_json::Value = serde_json::from_str(&actual).expect("bridge returns JSON");
        assert_eq!(json["shape"]["value"], AUDIT_SHAPE);
        assert_eq!(json["shapes"][0]["via"], "explicit");
        assert_eq!(json["groups"][0]["fields"][0]["label"], "Audit status");
    }

    /// An absent mode defaults to edit ([`sparq_forms::FormOptions::default`]);
    /// null shape is treated as absent.
    #[test]
    fn absent_mode_defaults_to_edit() {
        let actual = derive_form_json(
            DATA,
            SHAPES,
            "http://example.org/alice",
            "turtle",
            "{\"shape\":null}",
        )
        .expect("optionless request derives");
        assert_eq!(actual, direct_description(sparq_forms::Mode::Edit, None));
    }

    /// Malformed inputs fail closed with a named error — never a silent default
    /// or a fabricated description.
    #[test]
    fn malformed_inputs_are_errors() {
        let derive = |data: &str, shapes: &str, focus: &str, options: &str| {
            derive_form_json(data, shapes, focus, "turtle", options)
        };
        // Unparseable options document / non-object options / bad mode / bad shape.
        assert!(derive(DATA, SHAPES, "http://example.org/alice", "not json")
            .unwrap_err()
            .contains("options JSON"));
        assert!(derive(DATA, SHAPES, "http://example.org/alice", "[]")
            .unwrap_err()
            .contains("JSON object"));
        assert!(
            derive(DATA, SHAPES, "http://example.org/alice", "{\"mode\":\"delete\"}")
                .unwrap_err()
                .contains("Unsupported form mode")
        );
        assert!(derive(DATA, SHAPES, "http://example.org/alice", "{\"mode\":3}")
            .unwrap_err()
            .contains("Unsupported form mode"));
        assert!(
            derive(DATA, SHAPES, "http://example.org/alice", "{\"shape\":3}")
                .unwrap_err()
                .contains("shape option")
        );
        assert!(
            derive(DATA, SHAPES, "http://example.org/alice", "{\"shape\":\"not an iri\"}")
                .unwrap_err()
                .contains("Invalid shape IRI")
        );
        // Bad focus node / unparseable graphs.
        assert!(derive(DATA, SHAPES, "not an iri", "{}")
            .unwrap_err()
            .contains("Invalid focus IRI"));
        assert!(derive("@prefix broken", SHAPES, "http://example.org/alice", "{}")
            .unwrap_err()
            .contains("data graph"));
        assert!(derive(DATA, "@prefix broken", "http://example.org/alice", "{}")
            .unwrap_err()
            .contains("shapes graph"));
    }

    /// A `_:`-prefixed shape option parses as a blank node (the desktop bridge's
    /// structured-node contract), and unknown option keys are ignored.
    #[test]
    fn blank_node_shape_and_unknown_keys_accepted() {
        let options = parse_options("{\"mode\":\"view\",\"shape\":\"_:b0\",\"role\":\"x\"}")
            .expect("blank-node shape parses");
        assert_eq!(options.mode, sparq_forms::Mode::View);
        assert!(matches!(options.shape, Some(oxrdf::Term::BlankNode(_))));
    }

    /// `parse_options` on the exact document `formOptionsJson` emits.
    #[test]
    fn parses_bridge_emitted_options() {
        let options = parse_options("{\"mode\":\"edit\"}").expect("bridge options parse");
        assert_eq!(options.mode, sparq_forms::Mode::Edit);
        assert!(options.shape.is_none());
        assert_eq!(options.max_depth, sparq_forms::FormOptions::default().max_depth);
    }
}
