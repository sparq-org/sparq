//! (sq-qcnn) SHACL-SPARQL (`src/sparql.rs`) edge-case correctness
//! coverage — the genuinely-dark branches of the `sh:sparql` constraint path and
//! the SHACL §6 SPARQL-based constraint-COMPONENT validator path that the
//! fixture-gated W3C `sparql/` suites leave uncovered on a fresh checkout.
//!
//! Each test asserts the SHACL-spec-correct outcome (fail-closed skip of an
//! ill-formed query; correct `?value`/`?path`/`?message` mapping; pre-binding
//! pushed BELOW solution modifiers so `$value`/`$this` are in scope). All run
//! through the public `validate` API over real Turtle shapes — the real path, no
//! mocks.

use sparq_core::Graph;
use sparq_shacl::validate;

fn run(data: &str, shapes: &str) -> sparq_shacl::ValidationReport {
    let data = Graph::load_str(data, "turtle").unwrap();
    let shapes = Graph::load_str(shapes, "turtle").unwrap();
    validate(&data, &shapes)
}

const PREFIXES: &str = r#"
    @prefix sh: <http://www.w3.org/ns/shacl#> .
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix ex: <http://example.org/> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

// ───────────────────────── sh:sparql fail-closed forms ─────────────────────────

/// A `sh:sparql` whose `sh:select` is actually an ASK query is NOT a SELECT, so
/// `PreparedSparql::build` rejects it (`!matches!(Query::Select)`) and the
/// constraint is skipped — a parallel Core constraint still fires.
#[test]
fn non_select_sh_sparql_is_skipped() {
    let shapes = format!(
        r#"{PREFIXES}
        ex:S a sh:NodeShape ;
          sh:targetNode ex:thing ;
          sh:sparql [ sh:select "ASK {{ ?s ?p ?o }}" ] ;
          sh:property [ sh:path ex:name ; sh:minCount 1 ] .
    "#
    );
    let data = "@prefix ex: <http://example.org/> . ex:thing ex:other \"x\" .";
    let r = run(data, &shapes);
    // Only the minCount violation; the non-SELECT sh:sparql contributes nothing.
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert!(r.results[0]
        .source_component
        .ends_with("MinCountConstraintComponent"));
}

/// A `sh:sparql` constraint with NO `sh:message` and NO bound `?message` falls
/// back to the default "Violates SPARQL-based constraint" message.
#[test]
fn sh_sparql_default_message_when_none_supplied() {
    let shapes = format!(
        r#"{PREFIXES}
        ex:S a sh:NodeShape ;
          sh:targetNode ex:bob ;
          sh:sparql [ sh:select "SELECT $this WHERE {{ $this <http://example.org/bad> ?o }}" ] .
    "#
    );
    let data = "@prefix ex: <http://example.org/> . ex:bob ex:bad \"y\" .";
    let r = run(data, &shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert!(
        r.results[0].effective_messages()[0]
            .to_string()
            .contains("Violates SPARQL-based constraint"),
        "default message expected: {}",
        r.results[0].effective_messages()[0]
    );
}

/// A `sh:sparql` SELECT that projects `?value` but leaves it UNBOUND on the
/// solution row reports NO sh:value (None), not the focus node — distinguishing
/// "value projected but unbound" from "value not projected".
#[test]
fn sh_sparql_value_projected_but_unbound_is_none() {
    let shapes = format!(
        r#"{PREFIXES}
        ex:S a sh:NodeShape ;
          sh:targetNode ex:bob ;
          sh:sparql [
            sh:select """
              SELECT $this ?value WHERE {{
                $this <http://example.org/flag> ?x .
                OPTIONAL {{ $this <http://example.org/missing> ?value }}
              }}
            """ ;
          ] .
    "#
    );
    let data = "@prefix ex: <http://example.org/> . ex:bob ex:flag \"on\" .";
    let r = run(data, &shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    // ?value is projected but the OPTIONAL did not bind it -> no sh:value.
    assert!(
        r.results[0].value.is_none(),
        "a projected-but-unbound ?value must yield None, got {:?}",
        r.results[0].value
    );
}

/// A `?path` binding that is an IRI maps to `sh:resultPath` (a predicate path); a
/// `?path` binding that is NOT an IRI (a literal) maps to no path.
#[test]
fn sh_sparql_path_iri_maps_non_iri_does_not() {
    let shapes_iri = format!(
        r#"{PREFIXES}
        ex:S a sh:NodeShape ;
          sh:targetNode ex:bob ;
          sh:sparql [
            sh:select """
              SELECT $this ?path WHERE {{
                $this <http://example.org/bad> ?o .
                BIND(<http://example.org/age> AS ?path)
              }}
            """ ;
          ] .
    "#
    );
    let data = "@prefix ex: <http://example.org/> . ex:bob ex:bad \"x\" .";
    let r = run(data, &shapes_iri);
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert!(
        r.results[0].path.is_some(),
        "an IRI ?path must map to sh:resultPath: {}",
        r.to_text()
    );

    // A literal ?path -> no resultPath.
    let shapes_lit = format!(
        r#"{PREFIXES}
        ex:S a sh:NodeShape ;
          sh:targetNode ex:bob ;
          sh:sparql [
            sh:select """
              SELECT $this ?path WHERE {{
                $this <http://example.org/bad> ?o .
                BIND("not-an-iri" AS ?path)
              }}
            """ ;
          ] .
    "#
    );
    let r2 = run(data, &shapes_lit);
    assert_eq!(r2.results.len(), 1, "{}", r2.to_text());
    assert!(
        r2.results[0].path.is_none(),
        "a literal ?path must NOT map to sh:resultPath"
    );
}

// ───────────────── SPARQL-based constraint COMPONENT validators ─────────────────

const SUBCLASS: &str = "ex:MyCC rdfs:subClassOf sh:ConstraintComponent .\n";

/// A SELECT validator component whose solution binds `?message` (and the
/// component declares no `sh:message`) uses the row's `?message` as the result
/// message (the `(None, Some(m))` precedence arm).
#[test]
fn select_validator_row_message_used_when_no_component_message() {
    let shapes = format!(
        r#"{PREFIXES}
        {SUBCLASS}
        ex:MsgComp a ex:MyCC ;
          sh:parameter [ sh:path ex:tag ] ;
          sh:validator [
            a sh:SPARQLSelectValidator ;
            sh:select """
              SELECT $this ?message WHERE {{
                $this <http://example.org/v> ?v .
                BIND(CONCAT("bad value ", STR(?v)) AS ?message)
              }}
            """ ;
          ] .
        ex:S a sh:NodeShape ;
          sh:targetNode ex:bob ;
          ex:tag "anything" .
    "#
    );
    let data = "@prefix ex: <http://example.org/> . ex:bob ex:v 7 .";
    let r = run(data, &shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert!(
        r.results[0].effective_messages()[0]
            .to_string()
            .contains("bad value 7"),
        "the row ?message must drive the result message: {}",
        r.results[0].effective_messages()[0]
    );
}

/// A SELECT validator component with a `sh:message` template substitutes the
/// solution's `{?var}` (the `(Some(tpl), _)` precedence arm).
#[test]
fn select_validator_message_template_substitution() {
    let shapes = format!(
        r#"{PREFIXES}
        {SUBCLASS}
        ex:TplComp a ex:MyCC ;
          sh:parameter [ sh:path ex:tag ] ;
          sh:validator [
            a sh:SPARQLSelectValidator ;
            sh:message "value was {{?value}}" ;
            sh:select """
              SELECT $this ?value WHERE {{
                $this <http://example.org/v> ?value .
              }}
            """ ;
          ] .
        ex:S a sh:NodeShape ;
          sh:targetNode ex:bob ;
          ex:tag "x" .
    "#
    );
    let data = "@prefix ex: <http://example.org/> . ex:bob ex:v 42 .";
    let r = run(data, &shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert!(
        r.results[0].effective_messages()[0]
            .to_string()
            .contains("value was 42"),
        "the template must substitute ?value from the solution: {}",
        r.results[0].effective_messages()[0]
    );
}

/// An ASK validator whose WHERE has an `OPTIONAL` (a `LeftJoin` algebra node)
/// over the FILTER must still pre-bind `$value`/`$this` on the REQUIRED (left)
/// side, so the constraint evaluates correctly (`push_values_down` LeftJoin arm).
/// The component flags a value outside the allowed numeric range.
#[test]
fn ask_validator_pre_binding_through_left_join() {
    let shapes = format!(
        r#"{PREFIXES}
        {SUBCLASS}
        ex:RangeComp a ex:MyCC ;
          sh:parameter [ sh:path ex:maxV ] ;
          sh:validator [
            a sh:SPARQLAskValidator ;
            sh:ask """
              ASK {{
                OPTIONAL {{ $this <http://example.org/note> ?n }}
                FILTER (?value <= $maxV)
              }}
            """ ;
          ] .
        ex:S a sh:NodeShape ;
          sh:targetNode 3, 9 ;
          ex:maxV 5 .
    "#
    );
    // value 9 > 5 -> ASK false -> violation; value 3 <= 5 -> conforms. The
    // OPTIONAL is a LeftJoin; pre-binding must land on its required (left) side.
    let data = "@prefix ex: <http://example.org/> . ex:x ex:y ex:z .";
    let r = run(data, &shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert!(
        r.results[0]
            .value
            .as_ref()
            .unwrap()
            .to_string()
            .starts_with("\"9\""),
        "only value 9 violates the <= 5 range: {}",
        r.to_text()
    );
}

/// A component whose validator query is neither ASK nor SELECT in the slot it is
/// declared for (`sh:ask` carrying a SELECT) is ill-formed and the validator is
/// not built — the component contributes no violation (fail-closed skip).
#[test]
fn mismatched_validator_form_is_skipped() {
    let shapes = format!(
        r#"{PREFIXES}
        {SUBCLASS}
        ex:BadComp a ex:MyCC ;
          sh:parameter [ sh:path ex:p ] ;
          sh:validator [
            a sh:SPARQLAskValidator ;
            sh:ask "SELECT $this WHERE {{ $this ?p ?o }}" ;
          ] .
        ex:S a sh:NodeShape ;
          sh:targetNode ex:bob ;
          ex:p "x" ;
          sh:property [ sh:path ex:name ; sh:minCount 1 ] .
    "#
    );
    let data = "@prefix ex: <http://example.org/> . ex:bob ex:other \"v\" .";
    let r = run(data, &shapes);
    // Only the minCount violation; the SELECT-in-sh:ask validator is not built.
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert!(r.results[0]
        .source_component
        .ends_with("MinCountConstraintComponent"));
}
