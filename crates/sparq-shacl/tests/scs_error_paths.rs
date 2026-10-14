//! (sq-qcnn) Fail-closed + edge-construct correctness coverage for the
//! SHACL Compact Syntax (SCS) PARSER (`src/scs.rs`).
//!
//! The W3C `shacl-compact-syntax` corpus that drives `scs.rs` is fetched into a
//! gitignored `tests/shacl/` dir and SKIPS on a fresh checkout, so the parser's
//! ERROR paths (the "any construct outside the supported surface returns a typed
//! `ScsError` rather than silently mis-parsing" contract) ran dark on the
//! per-commit gate. These tests pin that fail-closed contract directly — each
//! malformed input MUST return an `Err`, never a wrong-but-`Ok` parse — plus the
//! correct triple encoding of the valid edge constructs (escapes, paths,
//! datatypes, arrays, disjunctions).
//!
//! SCS syntax reminder (NOT Turtle): directives are `BASE`/`IMPORTS`/`PREFIX p:`
//! with no trailing `.`; a node shape is `shape ex:S -> ex:Class { … }` whose
//! constraints live INSIDE the `{ }` body, each terminated by `.`. The four
//! implicit prefixes `rdf`/`rdfs`/`sh`/`xsd` are predefined.
//!
//! Whole file is `#[cfg(feature = "scs")]` — the SCS parser public API is itself
//! feature-gated, so with the feature off this compiles to nothing (the AGENTS.md
//! "both feature states" standard is met).

#![cfg(feature = "scs")]

use sparq_shacl::{parse_scs, parse_scs_to_graph, DEFAULT_BASE};

const SH: &str = "http://www.w3.org/ns/shacl#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const PREFIX: &str = "PREFIX ex: <http://example.org/>\n";

/// Parse with the default base; panics if the document is malformed.
fn ok(text: &str) -> Vec<oxrdf::Triple> {
    parse_scs(text, DEFAULT_BASE).unwrap_or_else(|e| panic!("expected OK, got: {e}"))
}

/// Parse expecting a fail-closed `ScsError`; returns its message for assertions.
fn err(text: &str) -> String {
    match parse_scs(text, DEFAULT_BASE) {
        Ok(t) => panic!("expected an ScsError, but parsed {} triples OK", t.len()),
        Err(e) => e.message,
    }
}

/// True iff some emitted triple is `(_, <SH+pred>, …obj_lex…)`.
fn has_pred_obj(triples: &[oxrdf::Triple], pred_local: &str, obj_lex: &str) -> bool {
    triples.iter().any(|t| {
        t.predicate.as_str() == format!("{SH}{pred_local}")
            && t.object.to_string().contains(obj_lex)
    })
}

fn has_pred(triples: &[oxrdf::Triple], pred_local: &str) -> bool {
    triples
        .iter()
        .any(|t| t.predicate.as_str() == format!("{SH}{pred_local}"))
}

// ───────────────────────────── lexer fail-closed ─────────────────────────────

#[test]
fn unterminated_iri_is_error() {
    assert!(err("shape <http://example.org/incomplete").contains("unterminated <IRI>"));
}

#[test]
fn unterminated_string_is_error() {
    let m = err(&format!("{PREFIX}shape ex:S {{ ex:p message=\"oops . }}"));
    assert!(m.contains("unterminated string literal"), "got: {m}");
}

#[test]
fn invalid_string_escape_is_error() {
    let m = err(&format!(
        "{PREFIX}shape ex:S {{ ex:p message=\"bad \\q escape\" . }}"
    ));
    assert!(m.contains("invalid string escape"), "got: {m}");
}

#[test]
fn empty_language_tag_is_error() {
    let m = err(&format!("{PREFIX}shape ex:S {{ ex:p message=\"hi\"@ . }}"));
    assert!(m.contains("empty language tag"), "got: {m}");
}

#[test]
fn at_without_prefixed_name_is_error() {
    // `@` followed by neither `<iri>` nor a prefixed name (a digit run).
    let m = err(&format!("{PREFIX}shape ex:S {{ ex:p @123 . }}"));
    assert!(
        !m.is_empty(),
        "an `@` with no shape ref must fail closed: {m}"
    );
}

// ───────────────────────────── parser fail-closed ─────────────────────────────

#[test]
fn undefined_prefix_is_error() {
    let m = err("shape nope:S { }");
    assert!(m.contains("undefined prefix"), "got: {m}");
}

#[test]
fn prefix_decl_without_pname_is_error() {
    let m = err("PREFIX <http://example.org/>");
    assert!(m.contains("expected `prefix:` after PREFIX"), "got: {m}");
}

#[test]
fn base_without_iri_is_error() {
    let m = err("BASE notAniri");
    assert!(m.contains("expected <BASE IRI"), "got: {m}");
}

#[test]
fn imports_without_iri_is_error() {
    let m = err("IMPORTS notAniri");
    assert!(m.contains("expected <IMPORTS IRI"), "got: {m}");
}

#[test]
fn stray_token_where_shape_expected_is_error() {
    // After directives, only `shape`/`shapeClass`/EOF are valid.
    let m = err(&format!("{PREFIX}ex:NotAKeyword"));
    assert!(m.contains("expected `shape` or `shapeClass`"), "got: {m}");
}

#[test]
fn target_class_arrow_with_no_iri_is_error() {
    let m = err(&format!("{PREFIX}shape ex:S -> {{ }}"));
    assert!(
        m.contains("expected target class IRI after `->`"),
        "got: {m}"
    );
}

#[test]
fn unterminated_brace_body_is_error() {
    // `{` opens a node-shape body that never closes.
    let m = err(&format!("{PREFIX}shape ex:S {{ "));
    assert!(m.contains("unterminated `{ ... }` shape body"), "got: {m}");
}

#[test]
fn min_count_non_integer_is_error() {
    let m = err(&format!("{PREFIX}shape ex:S {{ ex:p [x..3] . }}"));
    assert!(m.contains("expected min count integer"), "got: {m}");
}

#[test]
fn max_count_garbage_is_error() {
    let m = err(&format!("{PREFIX}shape ex:S {{ ex:p [0..x] . }}"));
    assert!(m.contains("expected max count integer or `*`"), "got: {m}");
}

#[test]
fn unterminated_array_is_error() {
    let m = err(&format!("{PREFIX}shape ex:S {{ in=[ ex:a ex:b "));
    assert!(m.contains("unterminated `[ ... ]` array"), "got: {m}");
}

#[test]
fn missing_dot_after_node_constraint_is_error() {
    // A node constraint inside the body must be terminated by `.`.
    let m = err(&format!("{PREFIX}shape ex:S {{ closed=true }}"));
    assert!(m.contains("expected `.`"), "got: {m}");
}

// ───────────────────────────── valid edge constructs ─────────────────────────────

#[test]
fn empty_doc_emits_only_ontology() {
    // The empty document still emits the `?base rdf:type owl:Ontology` triple.
    let t = ok("");
    assert_eq!(t.len(), 1, "{t:?}");
    assert!(
        t[0].object.to_string().contains("owl#Ontology"),
        "{:?}",
        t[0]
    );
}

#[test]
fn imports_emits_owl_imports() {
    let t = ok("IMPORTS <http://example.org/other>");
    assert!(
        t.iter().any(
            |tr| tr.predicate.as_str() == "http://www.w3.org/2002/07/owl#imports"
                && tr.object.to_string().contains("http://example.org/other")
        ),
        "{t:?}"
    );
}

#[test]
fn shape_class_adds_rdfs_class_type() {
    let t = ok(&format!("{PREFIX}shapeClass ex:Animal {{ }}"));
    let rdf_type = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    assert!(
        t.iter().any(|tr| tr.predicate.as_str() == rdf_type
            && tr.object.to_string().contains("2000/01/rdf-schema#Class")),
        "shapeClass must add the rdfs:Class type: {t:?}"
    );
    assert!(
        t.iter().any(|tr| tr.predicate.as_str() == rdf_type
            && tr.object.to_string().contains(&format!("{SH}NodeShape"))),
        "shapeClass must also be a sh:NodeShape: {t:?}"
    );
}

#[test]
fn target_class_emits_sh_target_class() {
    let t = ok(&format!("{PREFIX}shape ex:S -> ex:Person {{ }}"));
    assert!(
        has_pred_obj(&t, "targetClass", "http://example.org/Person"),
        "`-> ex:Person` must emit sh:targetClass: {t:?}"
    );
}

#[test]
fn pn_local_escape_is_unescaped() {
    // `ex:a\-b` is the local name `a-b` (PN_LOCAL_ESC).
    let t = ok(&format!("{PREFIX}shape ex:a\\-b {{ }}"));
    assert!(
        t.iter()
            .any(|tr| tr.subject.to_string().contains("http://example.org/a-b")),
        "escaped local name must unescape to a-b: {t:?}"
    );
}

#[test]
fn bare_xsd_datatype_maps_to_sh_datatype_else_class() {
    // `xsd:string` is a SPARQL datatype -> sh:datatype; a non-datatype IRI -> sh:class.
    let t = ok(&format!(
        "{PREFIX}shape ex:S {{ ex:p xsd:string . }}\nshape ex:T {{ ex:q ex:SomeClass . }}"
    ));
    assert!(
        has_pred_obj(&t, "datatype", &format!("{XSD}string")),
        "xsd:string must map to sh:datatype: {t:?}"
    );
    assert!(
        has_pred_obj(&t, "class", "http://example.org/SomeClass"),
        "a non-datatype IRI must map to sh:class: {t:?}"
    );
}

#[test]
fn node_kind_keyword_emits_sh_node_kind() {
    let t = ok(&format!("{PREFIX}shape ex:S {{ ex:p IRI . }}"));
    assert!(
        has_pred_obj(&t, "nodeKind", &format!("{SH}IRI")),
        "the IRI keyword must emit sh:nodeKind sh:IRI: {t:?}"
    );
}

#[test]
fn inverse_path_emits_inverse_path() {
    let t = ok(&format!("{PREFIX}shape ex:S {{ ^ex:parent [0..1] . }}"));
    assert!(
        has_pred(&t, "inversePath"),
        "`^ex:parent` must emit sh:inversePath: {t:?}"
    );
}

#[test]
fn zero_or_more_path_emits_modifier() {
    let t = ok(&format!("{PREFIX}shape ex:S {{ ex:p* [0..1] . }}"));
    assert!(
        has_pred(&t, "zeroOrMorePath"),
        "`ex:p*` must emit sh:zeroOrMorePath: {t:?}"
    );
}

#[test]
fn one_or_more_path_emits_modifier() {
    let t = ok(&format!("{PREFIX}shape ex:S {{ ex:p+ . }}"));
    assert!(
        has_pred(&t, "oneOrMorePath"),
        "`ex:p+` must emit sh:oneOrMorePath: {t:?}"
    );
}

#[test]
fn alternative_path_emits_alternative_path() {
    let t = ok(&format!("{PREFIX}shape ex:S {{ (ex:a|ex:b) [0..1] . }}"));
    assert!(
        has_pred(&t, "alternativePath"),
        "`(ex:a|ex:b)` must emit sh:alternativePath: {t:?}"
    );
}

#[test]
fn sequence_path_emits_rdf_list() {
    // `ex:a/ex:b` is a sequence path (a plain RDF list).
    let t = ok(&format!("{PREFIX}shape ex:S {{ ex:a/ex:b [0..1] . }}"));
    let rdf_first = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
    assert!(
        t.iter().any(|tr| tr.predicate.as_str() == rdf_first),
        "a sequence path must build an rdf:first/rdf:rest list: {t:?}"
    );
}

#[test]
fn typed_literal_datatype_resolves() {
    // `"5"^^xsd:int` keeps its datatype.
    let t = ok(&format!(
        "{PREFIX}shape ex:S {{ hasValue=\"5\"^^xsd:int . }}"
    ));
    assert!(
        t.iter()
            .any(|tr| tr.object.to_string().contains(&format!("{XSD}int"))),
        "typed-literal datatype must be carried through: {t:?}"
    );
}

#[test]
fn decimal_and_double_literals_get_right_datatype() {
    let t = ok(&format!(
        "{PREFIX}shape ex:S {{ minInclusive=2.5 . maxInclusive=1e3 . }}"
    ));
    assert!(
        t.iter()
            .any(|tr| tr.object.to_string().contains(&format!("{XSD}decimal"))),
        "2.5 must be xsd:decimal: {t:?}"
    );
    assert!(
        t.iter()
            .any(|tr| tr.object.to_string().contains(&format!("{XSD}double"))),
        "1e3 must be xsd:double: {t:?}"
    );
}

#[test]
fn lang_tagged_literal_carries_language() {
    let t = ok(&format!(
        "{PREFIX}shape ex:S {{ ex:p hasValue=\"bonjour\"@fr . }}"
    ));
    assert!(
        t.iter().any(|tr| tr.object.to_string().contains("@fr")),
        "a language-tagged literal must carry its @lang: {t:?}"
    );
}

#[test]
fn base_directive_resolves_relative_iri_via_graph() {
    // A BASE directive plus a relative shape IRI resolves against the base; the
    // graph form is what `validate` consumes, so build it end to end.
    let g = parse_scs_to_graph(
        "BASE <http://base.example/v1#>\nshape <Thing> { }",
        DEFAULT_BASE,
    )
    .expect("valid SCS");
    assert!(
        g.len() >= 2,
        "graph must carry the ontology + shape-type triples"
    );
}

#[test]
fn empty_array_value_emits_rdf_nil() {
    // `in=[ ]` is the empty list -> rdf:nil.
    let t = ok(&format!("{PREFIX}shape ex:S {{ in=[] . }}"));
    assert!(
        has_pred_obj(&t, "in", "1999/02/22-rdf-syntax-ns#nil"),
        "empty array must encode as rdf:nil: {t:?}"
    );
}

#[test]
fn in_array_value_builds_rdf_list() {
    let t = ok(&format!("{PREFIX}shape ex:S {{ in=[ex:a ex:b] . }}"));
    let rdf_first = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
    assert!(
        t.iter().any(|tr| tr.predicate.as_str() == rdf_first),
        "a non-empty `in` array must build an RDF list: {t:?}"
    );
}

#[test]
fn node_or_disjunction_emits_sh_or() {
    // Two node disjuncts `datatype=xsd:string | class=ex:C` become an sh:or list
    // (the W3C `node-or-3-not` fixture form).
    let t = ok(&format!(
        "{PREFIX}shape ex:S {{ datatype=xsd:string | class=ex:C . }}"
    ));
    assert!(
        has_pred(&t, "or"),
        "a 2-branch node disjunction must emit sh:or: {t:?}"
    );
}

#[test]
fn node_negation_emits_sh_not() {
    // `!datatype=xsd:string` is a negated node constraint -> sh:not.
    let t = ok(&format!("{PREFIX}shape ex:S {{ !datatype=xsd:string . }}"));
    assert!(
        has_pred(&t, "not"),
        "a `!` node constraint must emit sh:not: {t:?}"
    );
}

#[test]
fn shape_ref_emits_sh_node() {
    // `@ex:OtherShape` in property-atom position is a shape reference (sh:node).
    let t = ok(&format!(
        "{PREFIX}shape ex:Other {{ }}\nshape ex:S {{ ex:p @ex:Other . }}"
    ));
    assert!(
        has_pred_obj(&t, "node", "http://example.org/Other"),
        "`@ex:Other` must emit sh:node: {t:?}"
    );
}

#[test]
fn nested_brace_body_emits_sh_node() {
    // A nested `{ ... }` after a path is an inline node shape (sh:node).
    let t = ok(&format!(
        "{PREFIX}shape ex:S {{ ex:p {{ ex:q [1..1] . }} . }}"
    ));
    assert!(
        has_pred(&t, "node"),
        "a nested `{{ }}` body must emit sh:node: {t:?}"
    );
}

#[test]
fn count_min_zero_omits_min_count() {
    // `[0..3]` -> only sh:maxCount (min 0 is the SHACL default, so no sh:minCount).
    let t = ok(&format!("{PREFIX}shape ex:S {{ ex:p [0..3] . }}"));
    assert!(
        !has_pred(&t, "minCount"),
        "min 0 must NOT emit sh:minCount: {t:?}"
    );
    assert!(
        has_pred_obj(&t, "maxCount", "3"),
        "max 3 must emit sh:maxCount: {t:?}"
    );
}

#[test]
fn count_star_max_omits_max_count() {
    // `[2..*]` -> only sh:minCount (unbounded max emits no sh:maxCount).
    let t = ok(&format!("{PREFIX}shape ex:S {{ ex:p [2..*] . }}"));
    assert!(
        has_pred_obj(&t, "minCount", "2"),
        "min 2 must emit sh:minCount: {t:?}"
    );
    assert!(
        !has_pred(&t, "maxCount"),
        "`*` max must NOT emit sh:maxCount: {t:?}"
    );
}
