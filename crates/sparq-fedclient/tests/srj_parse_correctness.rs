//! sq-bif.2 — SPARQL-Results-JSON (SRJ) parser correctness + error-path suite.
//!
//! [`parse_srj`](sparq_fedclient::parse_srj) is the load-bearing wire-decode of the federation
//! interpreter: every endpoint leaf result enters the join through it, so a misbehaving or
//! adversarial remote endpoint is its threat model. The crate's inline tests cover the happy
//! `uri`/`literal` round-trip + the ASK-boolean rejection + invalid JSON, but the rich
//! per-term-kind and structural ERROR branches of `parse_srj` / `srj_term` were untested. This
//! file targets exactly those uncovered branches against the REAL parser (no mock), asserting
//! each malformed/partial shape is a clean `Err` (never a panic, never a wrong term) and each
//! well-formed-but-previously-untested term kind (bnode / typed-literal / language tag /
//! RDF 1.2 directional literal / RDF 1.2 triple term) decodes to the exact `oxrdf::Term`.
//!
//! It also exercises [`solutions_equal`](sparq_fedclient::solutions_equal) on the **negative**
//! side — distinct multisets, different multiplicities, different bound terms must compare
//! UNEQUAL — the inline suite only asserted the positive (order-independence) direction.
//!
//! Gated on `fedclient`; the default build compiles this file to nothing.
//!
//! sq-bif.2 — flagged for Fable re-review when available.

#![cfg(feature = "fedclient")]

use oxrdf::{BlankNode, Literal, NamedNode, Term};
use sparq_fedclient::{parse_srj, solutions_equal};

fn nn(s: &str) -> Term {
    Term::NamedNode(NamedNode::new(s).unwrap())
}

// ─── Structural error paths (the body shape itself is wrong) ────────────────────────────

#[test]
fn missing_head_vars_is_clean_error() {
    // A body with valid results.bindings but NO head.vars — the parser cannot name the columns.
    let err = parse_srj(r#"{"head":{},"results":{"bindings":[]}}"#).unwrap_err();
    assert!(err.contains("head.vars"), "got {}", err);
}

#[test]
fn missing_results_bindings_is_clean_error() {
    // Valid head.vars but NO results.bindings array.
    let err = parse_srj(r#"{"head":{"vars":["x"]},"results":{}}"#).unwrap_err();
    assert!(err.contains("results.bindings"), "got {}", err);
}

#[test]
fn results_bindings_not_array_is_clean_error() {
    // results.bindings present but the wrong JSON type (an object, not an array).
    let err = parse_srj(r#"{"head":{"vars":["x"]},"results":{"bindings":{}}}"#).unwrap_err();
    assert!(err.contains("results.bindings"), "got {}", err);
}

#[test]
fn solution_binding_not_object_is_clean_error() {
    // A binding element that is not a JSON object (an array here) — a misbehaving server.
    let err = parse_srj(r#"{"head":{"vars":["x"]},"results":{"bindings":[[]]}}"#).unwrap_err();
    assert!(err.contains("not a JSON object"), "got {}", err);
}

#[test]
fn invalid_json_is_clean_error() {
    // Not JSON at all — surfaced as a parse error, never a panic.
    assert!(parse_srj("definitely { not json").is_err());
    assert!(parse_srj("").is_err());
}

#[test]
fn ask_boolean_body_is_rejected_for_select() {
    // An ASK response body has a top-level `boolean`; the SELECT interpreter must refuse it
    // rather than silently return an empty relation.
    let err = parse_srj(r#"{"head":{},"boolean":false}"#).unwrap_err();
    assert!(err.contains("ASK"), "got {}", err);
}

#[test]
fn results_before_head_and_unused_malformed_cells_preserve_dom_semantics() {
    // sq-1rtc2: witnesses order-independent borrowed parsing and deferred cell
    // decoding. Eagerly decoding every binding cell would reject `ignored`.
    let relation = parse_srj(
        r#"{"results":{"bindings":[{"ignored":17,"x":{"type":"literal","value":"ok"}}]},"head":{"vars":["x"]}}"#,
    )
    .unwrap();
    assert_eq!(relation.vars, vec!["x"]);
    assert_eq!(
        relation.rows,
        vec![vec![Some(Term::Literal(Literal::new_simple_literal("ok")))]]
    );
}

// ─── Per-cell term-kind error paths (srj_term branches) ─────────────────────────────────

#[test]
fn uri_cell_missing_value_is_bad_iri() {
    // A `uri` cell with no `value` defaults to the empty string, which is not a valid IRI.
    let err = parse_srj(r#"{"head":{"vars":["x"]},"results":{"bindings":[{"x":{"type":"uri"}}]}}"#)
        .unwrap_err();
    assert!(err.contains("bad IRI"), "got {}", err);
}

#[test]
fn bnode_cell_missing_value_is_bad_bnode() {
    let err =
        parse_srj(r#"{"head":{"vars":["x"]},"results":{"bindings":[{"x":{"type":"bnode"}}]}}"#)
            .unwrap_err();
    assert!(err.contains("bad bnode"), "got {}", err);
}

#[test]
fn literal_cell_missing_value_is_clean_error() {
    let err =
        parse_srj(r#"{"head":{"vars":["x"]},"results":{"bindings":[{"x":{"type":"literal"}}]}}"#)
            .unwrap_err();
    assert!(err.contains("value"), "got {}", err);
}

#[test]
fn typed_literal_bad_datatype_iri_is_clean_error() {
    // A `datatype` that is not a valid IRI must error, not panic.
    let err = parse_srj(
        r#"{"head":{"vars":["x"]},"results":{"bindings":[{"x":{"type":"typed-literal","value":"v","datatype":"not an iri"}}]}}"#,
    )
    .unwrap_err();
    assert!(err.contains("bad datatype"), "got {}", err);
}

#[test]
fn literal_bad_language_tag_is_clean_error() {
    // An invalid BCP47 language tag must error.
    let err = parse_srj(
        r#"{"head":{"vars":["x"]},"results":{"bindings":[{"x":{"type":"literal","value":"v","xml:lang":"bad lang!!"}}]}}"#,
    )
    .unwrap_err();
    assert!(err.contains("language tag"), "got {}", err);
}

#[test]
fn unknown_binding_type_is_clean_error() {
    let err = parse_srj(
        r#"{"head":{"vars":["x"]},"results":{"bindings":[{"x":{"type":"banana","value":"v"}}]}}"#,
    )
    .unwrap_err();
    assert!(err.contains("unknown binding type"), "got {}", err);
}

// ─── RDF 1.2 triple-term branches (srj_term "triple") ──────────────────────────────────

#[test]
fn triple_term_round_trips() {
    // A well-formed SPARQL-1.2 triple term decodes to a Term::Triple with the exact inner terms.
    let rel = parse_srj(
        r#"{"head":{"vars":["t"]},"results":{"bindings":[{"t":{"type":"triple","value":{
            "subject":{"type":"uri","value":"http://ex/s"},
            "predicate":{"type":"uri","value":"http://ex/p"},
            "object":{"type":"literal","value":"o"}}}}]}}"#,
    )
    .unwrap();
    assert_eq!(rel.vars, vec!["t"]);
    assert_eq!(rel.rows.len(), 1);
    match &rel.rows[0][0] {
        Some(Term::Triple(t)) => {
            assert_eq!(t.subject.to_string(), "<http://ex/s>");
            assert_eq!(t.predicate.as_str(), "http://ex/p");
            assert_eq!(t.object, Term::Literal(Literal::new_simple_literal("o")));
        }
        other => panic!("expected a triple term, got {:?}", other),
    }
}

#[test]
fn triple_term_missing_value_is_clean_error() {
    let err = parse_srj(
        r#"{"head":{"vars":["t"]},"results":{"bindings":[{"t":{"type":"triple"}}]}}"#,
    )
    .unwrap_err();
    assert!(err.contains("triple term without value"), "got {}", err);
}

#[test]
fn triple_term_missing_component_is_clean_error() {
    // subject + predicate present but object absent.
    let err = parse_srj(
        r#"{"head":{"vars":["t"]},"results":{"bindings":[{"t":{"type":"triple","value":{
            "subject":{"type":"uri","value":"http://ex/s"},
            "predicate":{"type":"uri","value":"http://ex/p"}}}}]}}"#,
    )
    .unwrap_err();
    assert!(err.contains("triple term without object"), "got {}", err);
}

#[test]
fn triple_term_invalid_predicate_is_clean_error() {
    // A triple-term predicate that is a blank node (not an IRI) is invalid RDF.
    let err = parse_srj(
        r#"{"head":{"vars":["t"]},"results":{"bindings":[{"t":{"type":"triple","value":{
            "subject":{"type":"uri","value":"http://ex/s"},
            "predicate":{"type":"bnode","value":"b"},
            "object":{"type":"literal","value":"o"}}}}]}}"#,
    )
    .unwrap_err();
    assert!(err.contains("invalid triple-term predicate"), "got {}", err);
}

#[test]
fn triple_term_invalid_subject_is_clean_error() {
    // A triple-term subject that is a literal is invalid RDF (subject must be IRI or blank node).
    let err = parse_srj(
        r#"{"head":{"vars":["t"]},"results":{"bindings":[{"t":{"type":"triple","value":{
            "subject":{"type":"literal","value":"oops"},
            "predicate":{"type":"uri","value":"http://ex/p"},
            "object":{"type":"literal","value":"o"}}}}]}}"#,
    )
    .unwrap_err();
    assert!(err.contains("invalid triple-term subject"), "got {}", err);
}

// ─── Well-formed term-kind branches not previously round-tripped ────────────────────────

#[test]
fn bnode_typed_literal_lang_and_directional_decode_exactly() {
    // All four kinds the engine emits but the inline suite did not round-trip through parse_srj:
    // a blank node, a typed (xsd:integer) literal, a plain language-tagged literal, and an
    // RDF-1.2 base-direction (its:dir) directional literal.
    let rel = parse_srj(
        r#"{"head":{"vars":["b","i","l","d"]},"results":{"bindings":[{
            "b":{"type":"bnode","value":"x1"},
            "i":{"type":"typed-literal","value":"42","datatype":"http://www.w3.org/2001/XMLSchema#integer"},
            "l":{"type":"literal","value":"chat","xml:lang":"fr"},
            "d":{"type":"literal","value":"shalom","xml:lang":"he","its:dir":"rtl"}
        }]}}"#,
    )
    .unwrap();
    let row = &rel.rows[0];
    assert_eq!(row[0], Some(Term::BlankNode(BlankNode::new("x1").unwrap())));
    assert_eq!(
        row[1],
        Some(Term::Literal(Literal::new_typed_literal(
            "42",
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap()
        )))
    );
    assert_eq!(
        row[2],
        Some(Term::Literal(
            Literal::new_language_tagged_literal("chat", "fr").unwrap()
        ))
    );
    // The directional literal carries the rtl base direction (RDF 1.2 / its:dir).
    assert_eq!(
        row[3],
        Some(Term::Literal(
            Literal::new_directional_language_tagged_literal(
                "shalom",
                "he",
                oxrdf::BaseDirection::Rtl
            )
            .unwrap()
        ))
    );
}

#[test]
fn invalid_its_dir_degrades_to_plain_language_tag() {
    // An unrecognised its:dir token must NOT error — the parser degrades to a plain
    // language-tagged literal (the same recall-safe decision the engine's parser makes), so a
    // server emitting a junk direction still yields a usable term.
    let rel = parse_srj(
        r#"{"head":{"vars":["x"]},"results":{"bindings":[{"x":{"type":"literal","value":"v","xml:lang":"en","its:dir":"sideways"}}]}}"#,
    )
    .unwrap();
    assert_eq!(
        rel.rows[0][0],
        Some(Term::Literal(
            Literal::new_language_tagged_literal("v", "en").unwrap()
        )),
        "an unknown its:dir falls back to a plain language-tagged literal, not an error"
    );
}

#[test]
fn type_absent_defaults_to_simple_literal() {
    // SRJ permits omitting `type` for a plain literal; an object with only `value` is a simple
    // literal (the `None` arm of the type match).
    let rel = parse_srj(
        r#"{"head":{"vars":["x"]},"results":{"bindings":[{"x":{"value":"plain"}}]}}"#,
    )
    .unwrap();
    assert_eq!(
        rel.rows[0][0],
        Some(Term::Literal(Literal::new_simple_literal("plain")))
    );
}

#[test]
fn variable_absent_from_a_solution_is_unbound() {
    // A column listed in head.vars but absent from a solution object is unbound (None) — the
    // SPARQL semantics of an OPTIONAL-style missing binding.
    let rel = parse_srj(
        r#"{"head":{"vars":["s","o"]},"results":{"bindings":[
            {"s":{"type":"uri","value":"http://ex/a"}}
        ]}}"#,
    )
    .unwrap();
    assert_eq!(rel.rows[0][0], Some(nn("http://ex/a")));
    assert_eq!(rel.rows[0][1], None, "?o absent ⇒ unbound");
}

// ─── solutions_equal — the NEGATIVE direction (must distinguish) ────────────────────────

#[test]
fn solutions_equal_distinguishes_different_multisets() {
    let vars = vec!["s".to_string()];
    let a = vec![vec![Some(nn("http://ex/a"))], vec![Some(nn("http://ex/b"))]];
    // A different bound term ⇒ unequal.
    let b = vec![vec![Some(nn("http://ex/a"))], vec![Some(nn("http://ex/c"))]];
    assert!(
        !solutions_equal(&vars, &a, &vars, &b),
        "a different bound term must compare unequal"
    );
}

#[test]
fn solutions_equal_is_multiplicity_sensitive() {
    // Bag semantics: the same solution at multiplicity 2 differs from multiplicity 1.
    let vars = vec!["s".to_string()];
    let once = vec![vec![Some(nn("http://ex/a"))]];
    let twice = vec![vec![Some(nn("http://ex/a"))], vec![Some(nn("http://ex/a"))]];
    assert!(
        !solutions_equal(&vars, &once, &vars, &twice),
        "differing multiplicity must compare unequal (bag, not set, semantics)"
    );
}

#[test]
fn solutions_equal_ignores_unbound_only_columns_and_order() {
    // Equal: same bound bindings, even though one side lists the columns in the opposite order
    // AND carries an extra all-unbound column (an unbound cell is dropped from the bag).
    let a_vars = vec!["s".to_string(), "o".to_string()];
    let a = vec![vec![Some(nn("http://ex/a")), Some(nn("http://ex/x"))]];
    // b: columns reversed, plus a third unbound column ?z.
    let b_vars = vec!["o".to_string(), "s".to_string(), "z".to_string()];
    let b = vec![vec![Some(nn("http://ex/x")), Some(nn("http://ex/a")), None]];
    assert!(
        solutions_equal(&a_vars, &a, &b_vars, &b),
        "column order + an unbound-only extra column must not change the solution bag"
    );
}
