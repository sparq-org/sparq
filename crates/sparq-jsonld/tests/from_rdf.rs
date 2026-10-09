//! [FABLE-5] sq-oy1f.28 — crate-local tests for the native **Deserialize RDF as
//! JSON-LD** algorithm (JSON-LD 1.1 API §8.1, `sparq_jsonld::from_rdf`).
//!
//! These are hermetic, fixture-free tests over hand-built [`RdfQuad`]s: one direct
//! test per public item (the coverage-floor rule) plus targeted probes of every §8.1
//! behaviour the W3C suite pins — list reconstruction (well-formed, nested, shared,
//! malformed), `@json` literals (including the `invalid JSON literal` negative),
//! `rdfDirection` in both modes (including the `invalid language-tagged string`
//! negative), `useNativeTypes`/`useRdfType`, named-graph grouping, and the
//! non-RDF-input tolerances. The authoritative W3C `fromRdf` RATCHET runs in
//! `sparq-conformance` (`tests/jsonld_suite/from_rdf.rs`, floor in
//! `src/floors/from_rdf.rs`); expected values here are transcribed from the same
//! suite semantics so the two lanes agree.

use sparq_jsonld::from_rdf::{from_rdf, FromRdfOptions, RdfQuad, RdfTerm};
use sparq_jsonld::{Json, JsonLdErrorCode, JsonLdOptions, ProcessingMode, RdfDirection};

const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const XSD_NS: &str = "http://www.w3.org/2001/XMLSchema#";

fn rdf(local: &str) -> RdfTerm {
    RdfTerm::iri(format!("{}{}", RDF_NS, local))
}

fn xsd(local: &str) -> String {
    format!("{}{}", XSD_NS, local)
}

fn q(s: RdfTerm, p: RdfTerm, o: RdfTerm) -> RdfQuad {
    RdfQuad::new(s, p, o, None)
}

fn qg(s: RdfTerm, p: RdfTerm, o: RdfTerm, g: RdfTerm) -> RdfQuad {
    RdfQuad::new(s, p, o, Some(g))
}

/// Run `from_rdf` and serialize the expanded document (deterministic: subjects and
/// graphs are emitted in sorted order).
fn render(quads: &[RdfQuad], options: &FromRdfOptions) -> String {
    let mut out = String::new();
    from_rdf(quads, options)
        .expect("from_rdf should succeed")
        .write(&mut out);
    out
}

fn render_default(quads: &[RdfQuad]) -> String {
    render(quads, &FromRdfOptions::default())
}

// ── public-surface constructors (one direct test per public fn) ────────────────

#[test]
fn rdf_term_iri_constructor() {
    assert_eq!(RdfTerm::iri("http://e/x"), RdfTerm::Iri("http://e/x".to_string()));
}

#[test]
fn rdf_term_blank_constructor() {
    assert_eq!(RdfTerm::blank("b0"), RdfTerm::BlankNode("b0".to_string()));
}

#[test]
fn rdf_term_literal_constructor() {
    assert_eq!(
        RdfTerm::literal("hi"),
        RdfTerm::Literal {
            lexical: "hi".to_string(),
            datatype: None,
            language: None
        }
    );
}

#[test]
fn rdf_term_typed_literal_constructor() {
    assert_eq!(
        RdfTerm::typed_literal("1", xsd("integer")),
        RdfTerm::Literal {
            lexical: "1".to_string(),
            datatype: Some(xsd("integer")),
            language: None
        }
    );
}

#[test]
fn rdf_term_lang_literal_constructor() {
    assert_eq!(
        RdfTerm::lang_literal("hallo", "de"),
        RdfTerm::Literal {
            lexical: "hallo".to_string(),
            datatype: None,
            language: Some("de".to_string())
        }
    );
}

#[test]
fn rdf_quad_new_carries_all_terms() {
    let quad = RdfQuad::new(
        RdfTerm::iri("http://e/s"),
        RdfTerm::iri("http://e/p"),
        RdfTerm::literal("o"),
        Some(RdfTerm::iri("http://e/g")),
    );
    assert_eq!(quad.subject, RdfTerm::iri("http://e/s"));
    assert_eq!(quad.predicate, RdfTerm::iri("http://e/p"));
    assert_eq!(quad.object, RdfTerm::literal("o"));
    assert_eq!(quad.graph, Some(RdfTerm::iri("http://e/g")));
}

#[test]
fn from_rdf_options_default_matches_the_spec_defaults() {
    let o = FromRdfOptions::default();
    assert_eq!(o.processing_mode, ProcessingMode::JsonLd11);
    assert_eq!(o.rdf_direction, RdfDirection::None);
    assert!(!o.use_native_types);
    assert!(!o.use_rdf_type);
    assert!(!o.ordered);
}

#[test]
fn from_rdf_options_from_jsonld_lifts_shared_fields() {
    let mut jopts = JsonLdOptions::default();
    jopts.processing_mode = ProcessingMode::JsonLd10;
    jopts.rdf_direction = RdfDirection::I18nDatatype;
    jopts.ordered = true;
    let o = FromRdfOptions::from_jsonld(&jopts);
    assert_eq!(o.processing_mode, ProcessingMode::JsonLd10);
    assert_eq!(o.rdf_direction, RdfDirection::I18nDatatype);
    assert!(o.ordered);
    // The §8.1-only flags start at their spec defaults.
    assert!(!o.use_native_types);
    assert!(!o.use_rdf_type);
}

#[test]
fn from_rdf_empty_dataset_yields_the_empty_expanded_document() {
    assert_eq!(render_default(&[]), "[]");
}

// ── node maps, @type, deduplication ─────────────────────────────────────────────

#[test]
fn groups_by_subject_with_types_and_deduplicates_quads() {
    let s = RdfTerm::iri("http://e/s");
    let p = RdfTerm::iri("http://e/p");
    let quads = [
        q(s.clone(), rdf("type"), RdfTerm::iri("http://e/T")),
        q(s.clone(), rdf("type"), RdfTerm::iri("http://e/T")), // duplicate quad
        q(s.clone(), p.clone(), RdfTerm::iri("http://e/o")),
        q(s.clone(), p.clone(), RdfTerm::iri("http://e/o")), // duplicate quad
        q(s.clone(), p.clone(), RdfTerm::literal("plain")),
    ];
    // The object/type stub nodes carry only @id, so they are dropped from the output.
    assert_eq!(
        render_default(&quads),
        r#"[{"@id":"http://e/s","@type":["http://e/T"],"http://e/p":[{"@id":"http://e/o"},{"@value":"plain"}]}]"#
    );
}

#[test]
fn use_rdf_type_keeps_rdf_type_as_a_property() {
    let quads = [q(
        RdfTerm::iri("http://e/s"),
        rdf("type"),
        RdfTerm::iri("http://e/T"),
    )];
    let mut opts = FromRdfOptions::default();
    opts.use_rdf_type = true;
    assert_eq!(
        render(&quads, &opts),
        format!(r#"[{{"@id":"http://e/s","{}type":[{{"@id":"http://e/T"}}]}}]"#, RDF_NS)
    );
}

#[test]
fn literal_forms_under_default_options() {
    let s = RdfTerm::iri("http://e/s");
    let quads = [
        q(s.clone(), RdfTerm::iri("http://e/a"), RdfTerm::literal("plain")),
        q(s.clone(), RdfTerm::iri("http://e/b"), RdfTerm::lang_literal("English", "en")),
        q(
            s.clone(),
            RdfTerm::iri("http://e/c"),
            RdfTerm::typed_literal("2012-05-12", xsd("date")),
        ),
        // An explicit xsd:string datatype is the same as none.
        q(
            s.clone(),
            RdfTerm::iri("http://e/d"),
            RdfTerm::typed_literal("str", xsd("string")),
        ),
        // Without rdfDirection, an i18n datatype stays a plain typed literal.
        q(
            s.clone(),
            RdfTerm::iri("http://e/e"),
            RdfTerm::typed_literal("x", "https://www.w3.org/ns/i18n#en_ltr"),
        ),
        // Under default options even valid native forms stay typed strings.
        q(
            s.clone(),
            RdfTerm::iri("http://e/f"),
            RdfTerm::typed_literal("1", xsd("integer")),
        ),
    ];
    assert_eq!(
        render_default(&quads),
        concat!(
            r#"[{"@id":"http://e/s","#,
            r#""http://e/a":[{"@value":"plain"}],"#,
            r#""http://e/b":[{"@value":"English","@language":"en"}],"#,
            r#""http://e/c":[{"@value":"2012-05-12","@type":"http://www.w3.org/2001/XMLSchema#date"}],"#,
            r#""http://e/d":[{"@value":"str"}],"#,
            r#""http://e/e":[{"@value":"x","@type":"https://www.w3.org/ns/i18n#en_ltr"}],"#,
            r#""http://e/f":[{"@value":"1","@type":"http://www.w3.org/2001/XMLSchema#integer"}]}]"#,
        )
    );
}

// ── useNativeTypes ──────────────────────────────────────────────────────────────

#[test]
fn native_types_cover_the_full_xsd_lexical_edge_set() {
    let mut opts = FromRdfOptions::default();
    opts.use_native_types = true;
    let s = RdfTerm::iri("http://e/s");
    let quads = [
        // Booleans: the xsd lexical space is {true, false, 1, 0} (fromRdf/0027);
        // "true" and "1" convert to the SAME native value, so they deduplicate.
        q(s.clone(), RdfTerm::iri("http://e/b1"), RdfTerm::typed_literal("true", xsd("boolean"))),
        q(s.clone(), RdfTerm::iri("http://e/b1"), RdfTerm::typed_literal("1", xsd("boolean"))),
        q(s.clone(), RdfTerm::iri("http://e/b2"), RdfTerm::typed_literal("0", xsd("boolean"))),
        q(s.clone(), RdfTerm::iri("http://e/b3"), RdfTerm::typed_literal("True", xsd("boolean"))),
        // Integers canonicalize (sign/leading zeros); invalid forms stay typed.
        q(s.clone(), RdfTerm::iri("http://e/i1"), RdfTerm::typed_literal("+007", xsd("integer"))),
        q(s.clone(), RdfTerm::iri("http://e/i2"), RdfTerm::typed_literal("-0", xsd("integer"))),
        q(s.clone(), RdfTerm::iri("http://e/i3"), RdfTerm::typed_literal("notnative", xsd("integer"))),
        // Doubles: valid finite forms convert; INF/NaN/overflow stay typed strings.
        q(s.clone(), RdfTerm::iri("http://e/d1"), RdfTerm::typed_literal("1.1E-1", xsd("double"))),
        q(s.clone(), RdfTerm::iri("http://e/d2"), RdfTerm::typed_literal("+INF", xsd("double"))),
        q(s.clone(), RdfTerm::iri("http://e/d3"), RdfTerm::typed_literal("NaN", xsd("double"))),
        q(
            s.clone(),
            RdfTerm::iri("http://e/d4"),
            RdfTerm::typed_literal("0.1e999999999999999", xsd("double")),
        ),
        // Non-native numeric datatypes (xsd:decimal) always stay typed strings.
        q(s.clone(), RdfTerm::iri("http://e/n1"), RdfTerm::typed_literal("1.1", xsd("decimal"))),
        q(s.clone(), RdfTerm::iri("http://e/s1"), RdfTerm::typed_literal("str", xsd("string"))),
    ];
    assert_eq!(
        render(&quads, &opts),
        concat!(
            r#"[{"@id":"http://e/s","#,
            r#""http://e/b1":[{"@value":true}],"#,
            r#""http://e/b2":[{"@value":false}],"#,
            r#""http://e/b3":[{"@value":"True","@type":"http://www.w3.org/2001/XMLSchema#boolean"}],"#,
            r#""http://e/i1":[{"@value":7}],"#,
            r#""http://e/i2":[{"@value":0}],"#,
            r#""http://e/i3":[{"@value":"notnative","@type":"http://www.w3.org/2001/XMLSchema#integer"}],"#,
            r#""http://e/d1":[{"@value":0.11}],"#,
            r#""http://e/d2":[{"@value":"+INF","@type":"http://www.w3.org/2001/XMLSchema#double"}],"#,
            r#""http://e/d3":[{"@value":"NaN","@type":"http://www.w3.org/2001/XMLSchema#double"}],"#,
            r#""http://e/d4":[{"@value":"0.1e999999999999999","@type":"http://www.w3.org/2001/XMLSchema#double"}],"#,
            r#""http://e/n1":[{"@value":"1.1","@type":"http://www.w3.org/2001/XMLSchema#decimal"}],"#,
            r#""http://e/s1":[{"@value":"str"}]}]"#,
        )
    );
}

// ── @json literals ──────────────────────────────────────────────────────────────

#[test]
fn json_literals_parse_into_json_value_objects() {
    let s = RdfTerm::iri("http://e/s");
    let quads = [
        q(
            s.clone(),
            RdfTerm::iri("http://e/obj"),
            RdfTerm::typed_literal(r#"{"foo":"bar"}"#, format!("{}JSON", RDF_NS)),
        ),
        q(
            s.clone(),
            RdfTerm::iri("http://e/null"),
            RdfTerm::typed_literal("null", format!("{}JSON", RDF_NS)),
        ),
        q(
            s.clone(),
            RdfTerm::iri("http://e/num"),
            RdfTerm::typed_literal("1.23", format!("{}JSON", RDF_NS)),
        ),
    ];
    assert_eq!(
        render_default(&quads),
        concat!(
            r#"[{"@id":"http://e/s","#,
            r#""http://e/obj":[{"@value":{"foo":"bar"},"@type":"@json"}],"#,
            r#""http://e/null":[{"@value":null,"@type":"@json"}],"#,
            r#""http://e/num":[{"@value":1.23,"@type":"@json"}]}]"#,
        )
    );
}

#[test]
fn malformed_json_literal_raises_invalid_json_literal() {
    let quads = [q(
        RdfTerm::iri("http://e/s"),
        RdfTerm::iri("http://e/p"),
        RdfTerm::typed_literal("bareword", format!("{}JSON", RDF_NS)),
    )];
    let err = from_rdf(&quads, &FromRdfOptions::default()).expect_err("must reject");
    assert_eq!(err.code(), JsonLdErrorCode::InvalidJsonLiteral);
    assert_eq!(err.code().as_str(), "invalid JSON literal");
}

#[test]
fn json_literal_stays_typed_in_json_ld_10_mode() {
    let quads = [q(
        RdfTerm::iri("http://e/s"),
        RdfTerm::iri("http://e/p"),
        RdfTerm::typed_literal("bareword", format!("{}JSON", RDF_NS)),
    )];
    let mut opts = FromRdfOptions::default();
    opts.processing_mode = ProcessingMode::JsonLd10;
    // 1.0 mode: no @json decoding, so no parse and no error — a plain typed literal.
    assert_eq!(
        render(&quads, &opts),
        format!(
            r#"[{{"@id":"http://e/s","http://e/p":[{{"@value":"bareword","@type":"{}JSON"}}]}}]"#,
            RDF_NS
        )
    );
}

// ── rdfDirection: i18n-datatype ─────────────────────────────────────────────────

#[test]
fn i18n_datatype_decodes_language_and_direction() {
    let mut opts = FromRdfOptions::default();
    opts.rdf_direction = RdfDirection::I18nDatatype;
    let s = RdfTerm::iri("http://e/s");
    let quads = [
        q(
            s.clone(),
            RdfTerm::iri("http://e/a"),
            RdfTerm::typed_literal("en-US", "https://www.w3.org/ns/i18n#en-us_rtl"),
        ),
        // Empty language part: @direction only (fromRdf/di05).
        q(
            s.clone(),
            RdfTerm::iri("http://e/b"),
            RdfTerm::typed_literal("no language", "https://www.w3.org/ns/i18n#_rtl"),
        ),
        // No underscore in the fragment: not direction-encoded — stays typed.
        q(
            s.clone(),
            RdfTerm::iri("http://e/c"),
            RdfTerm::typed_literal("x", "https://www.w3.org/ns/i18n#en"),
        ),
    ];
    assert_eq!(
        render(&quads, &opts),
        concat!(
            r#"[{"@id":"http://e/s","#,
            r#""http://e/a":[{"@value":"en-US","@language":"en-us","@direction":"rtl"}],"#,
            r#""http://e/b":[{"@value":"no language","@direction":"rtl"}],"#,
            r#""http://e/c":[{"@value":"x","@type":"https://www.w3.org/ns/i18n#en"}]}]"#,
        )
    );
}

#[test]
fn i18n_datatype_is_not_special_in_compound_literal_mode() {
    // fromRdf/di09-di10: in compound-literal mode an i18n datatype stays typed.
    let mut opts = FromRdfOptions::default();
    opts.rdf_direction = RdfDirection::CompoundLiteral;
    let quads = [q(
        RdfTerm::iri("http://e/s"),
        RdfTerm::iri("http://e/p"),
        RdfTerm::typed_literal("en-US", "https://www.w3.org/ns/i18n#en-us_rtl"),
    )];
    assert_eq!(
        render(&quads, &opts),
        r#"[{"@id":"http://e/s","http://e/p":[{"@value":"en-US","@type":"https://www.w3.org/ns/i18n#en-us_rtl"}]}]"#
    );
}

// ── rdfDirection: compound-literal ──────────────────────────────────────────────

fn compound_literal_quads(language: Option<&str>) -> Vec<RdfQuad> {
    let cl = RdfTerm::blank("cl1");
    let mut quads = vec![
        q(RdfTerm::iri("http://e/a"), RdfTerm::iri("http://e/label"), cl.clone()),
        q(cl.clone(), rdf("value"), RdfTerm::literal("en-US")),
    ];
    if let Some(lang) = language {
        quads.push(q(cl.clone(), rdf("language"), RdfTerm::literal(lang)));
    }
    quads.push(q(cl, rdf("direction"), RdfTerm::literal("rtl")));
    quads
}

#[test]
fn compound_literal_collapses_into_a_value_object() {
    let mut opts = FromRdfOptions::default();
    opts.rdf_direction = RdfDirection::CompoundLiteral;
    // fromRdf/di12 shape: value + language + direction.
    assert_eq!(
        render(&compound_literal_quads(Some("en-us")), &opts),
        r#"[{"@id":"http://e/a","http://e/label":[{"@value":"en-US","@language":"en-us","@direction":"rtl"}]}]"#
    );
    // fromRdf/di11 shape: no language.
    assert_eq!(
        render(&compound_literal_quads(None), &opts),
        r#"[{"@id":"http://e/a","http://e/label":[{"@value":"en-US","@direction":"rtl"}]}]"#
    );
}

#[test]
fn compound_literal_with_malformed_language_raises() {
    let mut opts = FromRdfOptions::default();
    opts.rdf_direction = RdfDirection::CompoundLiteral;
    let err = from_rdf(&compound_literal_quads(Some("12-x")), &opts).expect_err("must reject");
    assert_eq!(err.code(), JsonLdErrorCode::InvalidLanguageTaggedString);
    assert_eq!(err.code().as_str(), "invalid language-tagged string");
}

#[test]
fn compound_literal_shape_is_untouched_outside_compound_mode() {
    // fromRdf/di07 shape: in i18n-datatype mode the blank node stays a node object.
    let mut opts = FromRdfOptions::default();
    opts.rdf_direction = RdfDirection::I18nDatatype;
    assert_eq!(
        render(&compound_literal_quads(None), &opts),
        concat!(
            r#"[{"@id":"_:cl1","#,
            r#""http://www.w3.org/1999/02/22-rdf-syntax-ns#value":[{"@value":"en-US"}],"#,
            r#""http://www.w3.org/1999/02/22-rdf-syntax-ns#direction":[{"@value":"rtl"}]},"#,
            r#"{"@id":"http://e/a","http://e/label":[{"@id":"_:cl1"}]}]"#,
        )
    );
}

#[test]
fn shared_compound_literal_is_not_converted() {
    // Referenced twice → not "referenced once" → stays a node object even in
    // compound-literal mode.
    let mut opts = FromRdfOptions::default();
    opts.rdf_direction = RdfDirection::CompoundLiteral;
    let mut quads = compound_literal_quads(None);
    quads.push(q(
        RdfTerm::iri("http://e/b"),
        RdfTerm::iri("http://e/label"),
        RdfTerm::blank("cl1"),
    ));
    let out = render(&quads, &opts);
    assert!(out.contains(r#"{"@id":"_:cl1","#), "cl node must stay: {}", out);
    assert!(!out.contains("@direction\":"), "no @direction value object: {}", out);
}

// ── rdf:List reconstruction ─────────────────────────────────────────────────────

fn cell(label: &str, first: RdfTerm, rest: RdfTerm) -> Vec<RdfQuad> {
    vec![
        q(RdfTerm::blank(label), rdf("first"), first),
        q(RdfTerm::blank(label), rdf("rest"), rest),
    ]
}

#[test]
fn well_formed_chain_collapses_into_a_list() {
    let mut quads = vec![q(
        RdfTerm::iri("http://e/s"),
        RdfTerm::iri("http://e/p"),
        RdfTerm::blank("l0"),
    )];
    quads.extend(cell("l0", RdfTerm::literal("a"), RdfTerm::blank("l1")));
    quads.extend(cell("l1", RdfTerm::literal("b"), rdf("nil")));
    // Cells are consumed; the rdf:nil stub carries only @id and is dropped.
    assert_eq!(
        render_default(&quads),
        r#"[{"@id":"http://e/s","http://e/p":[{"@list":[{"@value":"a"},{"@value":"b"}]}]}]"#
    );
}

#[test]
fn typed_list_cells_still_collapse() {
    // fromRdf/0016: cells may carry @type = [rdf:List] (deduplicated) and stay
    // well-formed; the redundant type is consumed with the cell.
    let mut quads = vec![q(
        RdfTerm::iri("http://e/s"),
        RdfTerm::iri("http://e/p"),
        RdfTerm::blank("l0"),
    )];
    quads.extend(cell("l0", RdfTerm::literal("a"), rdf("nil")));
    quads.push(q(RdfTerm::blank("l0"), rdf("type"), rdf("List")));
    quads.push(q(RdfTerm::blank("l0"), rdf("type"), rdf("List"))); // duplicate
    assert_eq!(
        render_default(&quads),
        r#"[{"@id":"http://e/s","http://e/p":[{"@list":[{"@value":"a"}]}]}]"#
    );
}

#[test]
fn direct_nil_references_become_empty_lists() {
    // fromRdf/0026 + li01: EVERY rdf:nil reference converts, whatever the property.
    let quads = [
        q(RdfTerm::iri("http://e/s"), rdf("first"), rdf("nil")),
        q(RdfTerm::iri("http://e/s"), RdfTerm::iri("http://e/p"), rdf("nil")),
    ];
    assert_eq!(
        render_default(&quads),
        format!(
            r#"[{{"@id":"http://e/s","{}first":[{{"@list":[]}}],"http://e/p":[{{"@list":[]}}]}}]"#,
            RDF_NS
        )
    );
}

#[test]
fn nested_lists_reconstruct_recursively() {
    // fromRdf/li01 exact shape: a one-cell list whose single item is the empty list.
    let mut quads = vec![q(
        RdfTerm::iri("http://e/a"),
        RdfTerm::iri("http://e/p"),
        RdfTerm::blank("l1"),
    )];
    quads.extend(cell("l1", rdf("nil"), rdf("nil")));
    assert_eq!(
        render_default(&quads),
        r#"[{"@id":"http://e/a","http://e/p":[{"@list":[{"@list":[]}]}]}]"#
    );

    // fromRdf/li02 shape: an outer list of two inner single-item lists — the inner
    // chains convert AFTER the outer consumed their referencing cells, which is
    // exactly the aliasing case the deferred slot renderer must reproduce.
    let mut quads = vec![q(
        RdfTerm::iri("http://e/a"),
        RdfTerm::iri("http://e/p"),
        RdfTerm::blank("l1"),
    )];
    quads.extend(cell("ia", RdfTerm::literal("a"), rdf("nil")));
    quads.extend(cell("ib", RdfTerm::literal("b"), rdf("nil")));
    quads.extend(cell("l1", RdfTerm::blank("ia"), RdfTerm::blank("l2")));
    quads.extend(cell("l2", RdfTerm::blank("ib"), rdf("nil")));
    assert_eq!(
        render_default(&quads),
        r#"[{"@id":"http://e/a","http://e/p":[{"@list":[{"@list":[{"@value":"a"}]},{"@list":[{"@value":"b"}]}]}]}]"#
    );
}

#[test]
fn cell_with_extra_property_stays_a_plain_node() {
    let mut quads = vec![q(
        RdfTerm::iri("http://e/s"),
        RdfTerm::iri("http://e/p"),
        RdfTerm::blank("l0"),
    )];
    quads.extend(cell("l0", RdfTerm::literal("a"), rdf("nil")));
    quads.push(q(
        RdfTerm::blank("l0"),
        RdfTerm::iri("http://e/extra"),
        RdfTerm::literal("x"),
    ));
    let out = render_default(&quads);
    // Not well-formed → no chain, but the nil reference itself still converts.
    assert!(out.contains(r#""@id":"_:l0""#), "cell must stay: {}", out);
    assert!(
        out.contains(&format!(r#""{}rest":[{{"@list":[]}}]"#, RDF_NS)),
        "nil rest still becomes the empty list: {}",
        out
    );
    assert!(!out.contains(r#""@list":[{"@value":"a"}]"#), "must NOT collapse: {}", out);
}

#[test]
fn doubly_referenced_cell_breaks_the_chain() {
    let mut quads = vec![
        q(RdfTerm::iri("http://e/s"), RdfTerm::iri("http://e/p"), RdfTerm::blank("l0")),
        // A second reference to l0 → referenced twice → chain not convertible.
        q(RdfTerm::iri("http://e/t"), RdfTerm::iri("http://e/q"), RdfTerm::blank("l0")),
    ];
    quads.extend(cell("l0", RdfTerm::literal("a"), rdf("nil")));
    let out = render_default(&quads);
    assert!(out.contains(r#""@id":"_:l0""#), "shared cell must stay: {}", out);
}

#[test]
fn cross_graph_shared_cell_stays_plain() {
    // fromRdf/0020 (the case the previous engine-writer lane failed): _:z1 is
    // referenced from graph G (z0's rest) AND graph G1 (x's p) — referenced-once is
    // GLOBAL, so the chain must NOT collapse; z1's nil rest still becomes [].
    let g = RdfTerm::iri("http://e/G");
    let g1 = RdfTerm::iri("http://e/G1");
    let mut quads = vec![qg(
        RdfTerm::iri("http://e/z"),
        RdfTerm::iri("http://e/q"),
        RdfTerm::blank("z0"),
        g.clone(),
    )];
    quads.push(qg(RdfTerm::blank("z0"), rdf("first"), RdfTerm::literal("cell-A"), g.clone()));
    quads.push(qg(RdfTerm::blank("z0"), rdf("rest"), RdfTerm::blank("z1"), g.clone()));
    quads.push(qg(RdfTerm::blank("z1"), rdf("first"), RdfTerm::literal("cell-B"), g.clone()));
    quads.push(qg(RdfTerm::blank("z1"), rdf("rest"), rdf("nil"), g.clone()));
    quads.push(qg(RdfTerm::iri("http://e/x"), RdfTerm::iri("http://e/p"), RdfTerm::blank("z1"), g1));
    assert_eq!(
        render_default(&quads),
        concat!(
            r#"[{"@id":"http://e/G","@graph":["#,
            r#"{"@id":"_:z0","http://www.w3.org/1999/02/22-rdf-syntax-ns#first":[{"@value":"cell-A"}],"http://www.w3.org/1999/02/22-rdf-syntax-ns#rest":[{"@id":"_:z1"}]},"#,
            r#"{"@id":"_:z1","http://www.w3.org/1999/02/22-rdf-syntax-ns#first":[{"@value":"cell-B"}],"http://www.w3.org/1999/02/22-rdf-syntax-ns#rest":[{"@list":[]}]},"#,
            r#"{"@id":"http://e/z","http://e/q":[{"@id":"_:z0"}]}]},"#,
            r#"{"@id":"http://e/G1","@graph":[{"@id":"http://e/x","http://e/p":[{"@id":"_:z1"}]}]}]"#,
        )
    );
}

#[test]
fn same_graph_duplicate_reference_still_collapses() {
    // fromRdf/0022: the duplicate quad is deduplicated (a dataset is a SET), so the
    // cell is still referenced exactly once and the list collapses.
    let g = RdfTerm::iri("http://e/G");
    let mut quads = vec![
        qg(RdfTerm::iri("http://e/z"), RdfTerm::iri("http://e/q"), RdfTerm::blank("z0"), g.clone()),
        qg(RdfTerm::iri("http://e/z"), RdfTerm::iri("http://e/q"), RdfTerm::blank("z0"), g.clone()),
    ];
    quads.push(qg(RdfTerm::blank("z0"), rdf("first"), RdfTerm::literal("cell-A"), g.clone()));
    quads.push(qg(RdfTerm::blank("z0"), rdf("rest"), rdf("nil"), g));
    assert_eq!(
        render_default(&quads),
        r#"[{"@id":"http://e/G","@graph":[{"@id":"http://e/z","http://e/q":[{"@list":[{"@value":"cell-A"}]}]}]}]"#
    );
}

// ── named graphs and emission rules ─────────────────────────────────────────────

#[test]
fn named_graphs_nest_under_graph_name_nodes() {
    let quads = [
        // A named graph whose name node also has default-graph properties.
        qg(
            RdfTerm::iri("http://e/s"),
            RdfTerm::iri("http://e/p"),
            RdfTerm::literal("in-g"),
            RdfTerm::iri("http://e/g"),
        ),
        q(
            RdfTerm::iri("http://e/g"),
            RdfTerm::iri("http://e/label"),
            RdfTerm::literal("the graph"),
        ),
        // A blank-node graph name gets a default-graph stub with @graph.
        qg(
            RdfTerm::iri("http://e/t"),
            RdfTerm::iri("http://e/p"),
            RdfTerm::literal("in-b"),
            RdfTerm::blank("gb"),
        ),
    ];
    assert_eq!(
        render_default(&quads),
        concat!(
            r#"[{"@id":"_:gb","@graph":[{"@id":"http://e/t","http://e/p":[{"@value":"in-b"}]}]},"#,
            r#"{"@id":"http://e/g","http://e/label":[{"@value":"the graph"}],"#,
            r#""@graph":[{"@id":"http://e/s","http://e/p":[{"@value":"in-g"}]}]}]"#,
        )
    );
}

#[test]
fn rdf_nil_as_a_subject_is_emitted() {
    // fromRdf/0023: rdf:nil with its own properties is an ordinary node.
    let quads = [q(rdf("nil"), RdfTerm::iri("http://e/foo"), RdfTerm::iri("http://e/bar"))];
    assert_eq!(
        render_default(&quads),
        format!(r#"[{{"@id":"{}nil","http://e/foo":[{{"@id":"http://e/bar"}}]}}]"#, RDF_NS)
    );
}

#[test]
fn non_rdf_quads_are_ignored_and_generalized_predicates_accepted() {
    let quads = [
        // Literal subject / predicate / graph name: not RDF — ignored.
        q(RdfTerm::literal("s"), RdfTerm::iri("http://e/p"), RdfTerm::literal("o")),
        q(RdfTerm::iri("http://e/s"), RdfTerm::literal("p"), RdfTerm::literal("o")),
        qg(
            RdfTerm::iri("http://e/s"),
            RdfTerm::iri("http://e/p"),
            RdfTerm::literal("o"),
            RdfTerm::literal("g"),
        ),
    ];
    assert_eq!(render_default(&quads), "[]");

    // A blank-node predicate is generalized RDF: accepted, keyed as _:label.
    let quads = [q(
        RdfTerm::iri("http://e/s"),
        RdfTerm::blank("p"),
        RdfTerm::literal("o"),
    )];
    assert_eq!(
        render_default(&quads),
        r#"[{"@id":"http://e/s","_:p":[{"@value":"o"}]}]"#
    );
}

#[test]
fn ordered_flag_does_not_change_the_always_sorted_output() {
    let quads = [
        q(RdfTerm::iri("http://e/b"), RdfTerm::iri("http://e/p"), RdfTerm::literal("2")),
        q(RdfTerm::iri("http://e/a"), RdfTerm::iri("http://e/p"), RdfTerm::literal("1")),
    ];
    let mut ordered = FromRdfOptions::default();
    ordered.ordered = true;
    assert_eq!(render_default(&quads), render(&quads, &ordered));
    // Subjects come out code-point sorted either way.
    assert_eq!(
        render_default(&quads),
        r#"[{"@id":"http://e/a","http://e/p":[{"@value":"1"}]},{"@id":"http://e/b","http://e/p":[{"@value":"2"}]}]"#
    );
}

#[test]
fn output_is_the_json_arr_variant() {
    let doc = from_rdf(&[], &FromRdfOptions::default()).expect("empty dataset");
    assert!(matches!(doc, Json::Arr(items) if items.is_empty()));
}

/// Lists nested in one another in a cycle are reachable from no emitted node; one of
/// them stays an ordinary node, so every triple survives.
#[test]
fn lists_nested_in_a_cycle_keep_their_triples() {
    let (a, b) = (RdfTerm::blank("a"), RdfTerm::blank("b"));
    let quads = [
        q(a.clone(), rdf("first"), b.clone()),
        q(a.clone(), rdf("rest"), rdf("nil")),
        q(b.clone(), rdf("first"), a.clone()),
        q(b.clone(), rdf("rest"), rdf("nil")),
    ];
    let out = render_default(&quads);
    assert!(out.contains(r#""@id":"_:a""#) || out.contains(r#""@id":"_:b""#), "{out}");
    assert!(out.contains(r#""@list""#), "{out}");
}

/// Deeply nested lists are collapsed only so far: the rest stay ordinary nodes, so the
/// document nests within the walks' bound and every triple survives.
#[test]
fn deeply_nested_lists_stay_shallow() {
    let n = 1000;
    let cell = |i: usize| RdfTerm::blank(format!("l{i}"));
    let mut quads = vec![q(RdfTerm::iri("http://ex/s"), RdfTerm::iri("http://ex/p"), cell(0))];
    for i in 0..n {
        let first = if i + 1 < n { cell(i + 1) } else { RdfTerm::lang_literal("x", "en") };
        quads.push(q(cell(i), rdf("first"), first));
        quads.push(q(cell(i), rdf("rest"), rdf("nil")));
    }
    let doc = from_rdf(&quads, &FromRdfOptions::default()).unwrap();
    fn depth(j: &Json) -> usize {
        match j {
            Json::Arr(a) => 1 + a.iter().map(depth).max().unwrap_or(0),
            Json::Obj(m) => 1 + m.iter().map(|(_, v)| depth(v)).max().unwrap_or(0),
            _ => 0,
        }
    }
    assert!(depth(&doc) < 100, "depth {}", depth(&doc));
    // Every cell is either collapsed into a list or emitted as a node: count rdf:first.
    let out = render_default(&quads);
    let firsts = out.matches(&format!("{RDF_NS}first")).count();
    let lists = out.matches(r#""@list""#).count();
    assert_eq!(firsts + lists, n, "{out}");
}
