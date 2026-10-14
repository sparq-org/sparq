//! (sq-oy1f.24) End-to-end tests for the JSON-LD 1.1 Context Processing +
//! Create Term Definition + IRI Expansion foundation, exercised through the crate's public
//! API. Fixtures are drawn from the worked examples of the W3C JSON-LD 1.1 API spec
//! (<https://www.w3.org/TR/json-ld11-api/>). No network: remote contexts use the
//! deny-by-default `NoopLoader` (fail-closed) or the local-fixture `FsLoader`.

use std::collections::BTreeMap;

use sparq_jsonld::context::ActiveContext;
use sparq_jsonld::{
    DocumentLoader, Direction, FsLoader, Json, JsonLdError, JsonLdErrorCode, JsonLdOptions,
    NoopLoader, Override, ProcessingMode, RemoteDocument,
};

const BASE: &str = "http://example.org/";

/// Parses a JSON context fragment.
fn json(s: &str) -> Json {
    Json::parse(s).expect("valid JSON fixture")
}

/// Processes a context fragment against a fresh active context based at [`BASE`], using the
/// deny-by-default loader.
fn process(ctx: &str) -> Result<ActiveContext, JsonLdError> {
    ActiveContext::new(Some(BASE)).process(&json(ctx), Some(BASE), &NoopLoader, &JsonLdOptions::default())
}

/// Like [`process`] but asserts success.
fn ok(ctx: &str) -> ActiveContext {
    process(ctx).expect("context should process")
}

/// The error code raised by processing `ctx`.
fn err_code(ctx: &str) -> JsonLdErrorCode {
    process(ctx).expect_err("context should fail").code()
}

// -------------------------------------------------------------------------------------
// Create Term Definition — the shapes.
// -------------------------------------------------------------------------------------

#[test]
fn simple_term_maps_to_its_iri() {
    let ac = ok(r#"{"name": "http://schema.org/name"}"#);
    assert_eq!(
        ac.term_definition("name").unwrap().iri(),
        Some("http://schema.org/name")
    );
    assert!(ac.has_term("name"));
}

#[test]
fn keyword_alias_definitions() {
    // Aliasing keywords (§ term definitions): `id` → `@id`, `type` → `@type`.
    let ac = ok(r#"{"id": "@id", "type": "@type"}"#);
    assert_eq!(ac.term_definition("id").unwrap().iri(), Some("@id"));
    assert_eq!(ac.term_definition("type").unwrap().iri(), Some("@type"));
    // Expansion of the alias yields the keyword (IRI Expansion step 4).
    assert_eq!(ac.expand_iri("id", false, true).as_deref(), Some("@id"));
    assert_eq!(ac.expand_iri("type", false, true).as_deref(), Some("@type"));
}

#[test]
fn type_container_and_id_type_mapping() {
    let ac = ok(
        r#"{"members": {"@id": "http://example.org/members", "@type": "@id", "@container": "@set"}}"#,
    );
    let def = ac.term_definition("members").unwrap();
    assert_eq!(def.iri(), Some("http://example.org/members"));
    assert_eq!(def.type_mapping(), Some("@id"));
    assert_eq!(def.container(), &["@set".to_string()]);
}

#[test]
fn language_and_index_containers() {
    let ac = ok(
        r#"{
            "label": {"@id": "http://example.org/label", "@container": "@language"},
            "byIdx": {"@id": "http://example.org/idx", "@container": "@index", "@index": "http://example.org/key"}
        }"#,
    );
    assert_eq!(
        ac.term_definition("label").unwrap().container(),
        &["@language".to_string()]
    );
    let idx = ac.term_definition("byIdx").unwrap();
    assert_eq!(idx.container(), &["@index".to_string()]);
    assert_eq!(idx.index(), Some("http://example.org/key"));
}

#[test]
fn graph_container_combinations_are_valid() {
    // `@container: [@graph, @id]` is one of the allowed 1.1 combinations.
    let ac = ok(
        r#"{"g": {"@id": "http://example.org/g", "@container": ["@graph", "@id"]}}"#,
    );
    let c = ac.term_definition("g").unwrap().container();
    assert!(c.contains(&"@graph".to_string()) && c.contains(&"@id".to_string()));
}

#[test]
fn redefining_type_with_set_container_is_allowed() {
    // JSON-LD 1.1 permits redefining @type with @container: @set (and/or @protected).
    let ac = ok(r#"{"@type": {"@container": "@set"}}"#);
    let def = ac.term_definition("@type").unwrap();
    assert_eq!(def.iri(), Some("@type"));
    assert_eq!(def.container(), &["@set".to_string()]);
}

#[test]
fn reverse_property_definition() {
    let ac = ok(r#"{"parent": {"@reverse": "http://example.org/child"}}"#);
    let def = ac.term_definition("parent").unwrap();
    assert!(def.is_reverse());
    assert_eq!(def.iri(), Some("http://example.org/child"));
}

#[test]
fn scoped_context_is_stored_on_the_term() {
    let ac = ok(
        r#"{"Person": {"@id": "http://schema.org/Person", "@context": {"name": "http://schema.org/name"}}}"#,
    );
    assert!(ac.term_definition("Person").unwrap().context().is_some());
}

#[test]
fn term_scoped_language_and_direction_are_tri_state() {
    let ac = ok(
        r#"{
            "@language": "en",
            "plain": {"@id": "http://example.org/plain", "@language": null},
            "fr": {"@id": "http://example.org/fr", "@language": "fr", "@direction": "rtl"}
        }"#,
    );
    // Explicit null suppresses the language; a value binds it.
    assert_eq!(ac.term_definition("plain").unwrap().language(), &Override::Null);
    assert_eq!(
        ac.term_definition("fr").unwrap().language(),
        &Override::Set("fr".to_string())
    );
    assert_eq!(
        ac.term_definition("fr").unwrap().direction(),
        &Override::Set(Direction::Rtl)
    );
    // An unset term keeps the fall-through-to-default marker.
    assert!(!ac
        .term_definition("plain")
        .unwrap()
        .direction()
        .is_set());
}

#[test]
fn null_mapped_term_is_retained_but_drops_on_expansion() {
    let ac = ok(r#"{"@vocab": "http://example.org/", "drop": null}"#);
    assert!(ac.has_term("drop"));
    assert_eq!(ac.term_definition("drop").unwrap().iri(), None);
    // A vocab reference to a null-mapped term expands to null (the term is dropped).
    assert_eq!(ac.expand_iri("drop", false, true), None);
    // An undefined term still expands via @vocab.
    assert_eq!(
        ac.expand_iri("kept", false, true).as_deref(),
        Some("http://example.org/kept")
    );
}

// -------------------------------------------------------------------------------------
// Context-level defaults.
// -------------------------------------------------------------------------------------

#[test]
fn vocab_language_direction_defaults() {
    let ac = ok(r#"{"@vocab": "http://schema.org/", "@language": "en", "@direction": "ltr"}"#);
    assert_eq!(ac.vocabulary_mapping(), Some("http://schema.org/"));
    assert_eq!(ac.default_language(), Some("en"));
    assert_eq!(ac.default_base_direction(), Some(Direction::Ltr));
}

#[test]
fn base_is_set_reset_and_resolved_relatively() {
    // Absolute @base replaces the base IRI.
    let ac = ok(r#"{"@base": "http://other.example/"}"#);
    assert_eq!(ac.base_iri(), Some("http://other.example/"));

    // Relative @base resolves against the current base.
    let ac = ActiveContext::new(Some("http://example.org/a/b"))
        .process(
            &json(r#"{"@base": "sub/"}"#),
            Some("http://example.org/a/b"),
            &NoopLoader,
            &JsonLdOptions::default(),
        )
        .unwrap();
    assert_eq!(ac.base_iri(), Some("http://example.org/a/sub/"));

    // Null @base clears the base IRI.
    let ac = ok(r#"{"@base": null}"#);
    assert_eq!(ac.base_iri(), None);
}

#[test]
fn null_context_resets_terms() {
    let base = ok(r#"{"@vocab": "http://schema.org/", "name": "http://schema.org/name"}"#);
    let reset = base
        .process(&json("null"), Some(BASE), &NoopLoader, &JsonLdOptions::default())
        .unwrap();
    assert_eq!(reset.term_count(), 0);
    assert_eq!(reset.vocabulary_mapping(), None);
}

// -------------------------------------------------------------------------------------
// IRI Expansion — vocab / compact IRIs / relative refs / keywords.
// -------------------------------------------------------------------------------------

#[test]
fn expand_prefers_a_term_then_vocab() {
    let ac = ok(r#"{"@vocab": "http://schema.org/", "name": "http://xmlns.com/foaf/0.1/name"}"#);
    // A defined term wins.
    assert_eq!(
        ac.expand_iri("name", false, true).as_deref(),
        Some("http://xmlns.com/foaf/0.1/name")
    );
    // An undefined vocab reference uses @vocab.
    assert_eq!(
        ac.expand_iri("age", false, true).as_deref(),
        Some("http://schema.org/age")
    );
}

#[test]
fn expand_simple_term_prefix_produces_compact_iri() {
    // A simple-term prefix whose IRI ends with a gen-delim gets the @prefix flag.
    let ac = ok(r#"{"foaf": "http://xmlns.com/foaf/0.1/"}"#);
    assert!(ac.term_definition("foaf").unwrap().is_prefix());
    assert_eq!(
        ac.expand_iri("foaf:name", false, true).as_deref(),
        Some("http://xmlns.com/foaf/0.1/name")
    );
}

#[test]
fn expand_non_prefix_term_leaves_compact_iri_unexpanded() {
    // A map-form term without @prefix (and not ending in a gen-delim) is not a prefix, so a
    // compact IRI using it is treated as an already-absolute IRI (returned unchanged).
    let ac = ok(r#"{"ex": {"@id": "http://example.org/vocab"}}"#);
    assert!(!ac.term_definition("ex").unwrap().is_prefix());
    assert_eq!(ac.expand_iri("ex:foo", false, true).as_deref(), Some("ex:foo"));
}

#[test]
fn expand_explicit_prefix_flag() {
    let ac = ok(r#"{"ex": {"@id": "http://example.org/vocab#", "@prefix": true}}"#);
    assert!(ac.term_definition("ex").unwrap().is_prefix());
    assert_eq!(
        ac.expand_iri("ex:foo", false, true).as_deref(),
        Some("http://example.org/vocab#foo")
    );
}

#[test]
fn expand_keywords_and_document_relative() {
    let ac = ok(r#"{}"#);
    // Keywords expand to themselves; keyword-shaped non-keywords expand to null.
    assert_eq!(ac.expand_iri("@type", false, true).as_deref(), Some("@type"));
    assert_eq!(ac.expand_iri("@bogus", false, true), None);
    // A document-relative reference resolves against the base IRI.
    assert_eq!(
        ac.expand_iri("thing", true, false).as_deref(),
        Some("http://example.org/thing")
    );
    // An absolute IRI is returned unchanged.
    assert_eq!(
        ac.expand_iri("http://x/y", true, false).as_deref(),
        Some("http://x/y")
    );
}

// -------------------------------------------------------------------------------------
// @protected.
// -------------------------------------------------------------------------------------

#[test]
fn protected_flag_propagates_from_context() {
    let ac = ok(r#"{"@protected": true, "name": "http://schema.org/name"}"#);
    assert!(ac.term_definition("name").unwrap().is_protected());
}

#[test]
fn protected_term_redefinition_is_rejected_but_identical_is_allowed() {
    let base = ok(r#"{"@protected": true, "name": "http://schema.org/name"}"#);
    // Redefining a protected term with a different IRI fails.
    let e = base
        .process(
            &json(r#"{"name": "http://other.example/name"}"#),
            Some(BASE),
            &NoopLoader,
            &JsonLdOptions::default(),
        )
        .unwrap_err();
    assert_eq!(e.code(), JsonLdErrorCode::ProtectedTermRedefinition);
    // Redefining it identically is a no-op (allowed).
    let ok2 = base.process(
        &json(r#"{"name": "http://schema.org/name"}"#),
        Some(BASE),
        &NoopLoader,
        &JsonLdOptions::default(),
    );
    assert!(ok2.is_ok());
}

#[test]
fn nullifying_a_context_with_protected_terms_fails() {
    let base = ok(r#"{"@protected": true, "name": "http://schema.org/name"}"#);
    let e = base
        .process(&json("null"), Some(BASE), &NoopLoader, &JsonLdOptions::default())
        .unwrap_err();
    assert_eq!(e.code(), JsonLdErrorCode::InvalidContextNullification);
}

// -------------------------------------------------------------------------------------
// Negative tests — exact spec error codes.
// -------------------------------------------------------------------------------------

#[test]
fn keyword_redefinition_error() {
    assert_eq!(err_code(r#"{"@id": "http://example.org/"}"#), JsonLdErrorCode::KeywordRedefinition);
}

#[test]
fn cyclic_iri_mapping_error() {
    assert_eq!(err_code(r#"{"a": "b:x", "b": "a:y"}"#), JsonLdErrorCode::CyclicIriMapping);
}

#[test]
fn invalid_reverse_property_with_id() {
    assert_eq!(
        err_code(r#"{"x": {"@reverse": "http://example.org/r", "@id": "http://example.org/x"}}"#),
        JsonLdErrorCode::InvalidReverseProperty
    );
}

#[test]
fn invalid_container_mapping_error() {
    // @list cannot combine with @set.
    assert_eq!(
        err_code(r#"{"x": {"@id": "http://example.org/x", "@container": ["@list", "@set"]}}"#),
        JsonLdErrorCode::InvalidContainerMapping
    );
}

#[test]
fn invalid_type_mapping_error() {
    assert_eq!(
        err_code(r#"{"x": {"@id": "http://example.org/x", "@type": "@bogus"}}"#),
        JsonLdErrorCode::InvalidTypeMapping
    );
}

#[test]
fn invalid_keyword_alias_to_context() {
    assert_eq!(
        err_code(r#"{"ctx": "@context"}"#),
        JsonLdErrorCode::InvalidKeywordAlias
    );
}

#[test]
fn invalid_vocab_mapping_error() {
    assert_eq!(err_code(r#"{"@vocab": ["not-a-string"]}"#), JsonLdErrorCode::InvalidVocabMapping);
}

#[test]
fn invalid_propagate_value_error() {
    assert_eq!(err_code(r#"{"@propagate": "yes"}"#), JsonLdErrorCode::InvalidPropagateValue);
}

#[test]
fn invalid_import_value_error() {
    assert_eq!(err_code(r#"{"@import": true}"#), JsonLdErrorCode::InvalidImportValue);
}

#[test]
fn invalid_nest_value_rejects_keyword_form() {
    // A keyword-form value other than `@nest` (even an unrecognised one) is invalid.
    assert_eq!(
        err_code(r#"{"x": {"@id": "http://example.org/x", "@nest": "@bogus"}}"#),
        JsonLdErrorCode::InvalidNestValue
    );
    assert_eq!(
        err_code(r#"{"x": {"@id": "http://example.org/x", "@nest": "@id"}}"#),
        JsonLdErrorCode::InvalidNestValue
    );
    // `@nest` itself and a plain nest property name are accepted.
    ok(r#"{"x": {"@id": "http://example.org/x", "@nest": "@nest"}}"#);
    ok(r#"{"x": {"@id": "http://example.org/x", "@nest": "nestProp"}}"#);
}

#[test]
fn processing_mode_conflict_error() {
    let mut opts = JsonLdOptions::default();
    opts.processing_mode = ProcessingMode::JsonLd10;
    let e = ActiveContext::new(Some(BASE))
        .process(&json(r#"{"@version": 1.1}"#), Some(BASE), &NoopLoader, &opts)
        .unwrap_err();
    assert_eq!(e.code(), JsonLdErrorCode::ProcessingModeConflict);
}

// -------------------------------------------------------------------------------------
// Remote contexts — deny-by-default, and the explicit fixture loader.
// -------------------------------------------------------------------------------------

#[test]
fn remote_context_is_denied_by_default() {
    // The default NoopLoader refuses; a string (remote) context fails closed with the
    // `loading remote context failed` code — no ambient network.
    let e = ActiveContext::new(Some(BASE))
        .process(
            &Json::Str("http://remote.example/ctx.jsonld".into()),
            Some(BASE),
            &NoopLoader,
            &JsonLdOptions::default(),
        )
        .unwrap_err();
    assert_eq!(e.code(), JsonLdErrorCode::LoadingRemoteContextFailed);
}

/// An in-memory loader serving a single fixed URL — a hermetic stand-in for a fetch.
struct MapLoader(BTreeMap<String, String>);

impl DocumentLoader for MapLoader {
    fn load_document(&self, url: &str) -> Result<RemoteDocument, JsonLdError> {
        match self.0.get(url) {
            Some(doc) => Ok(RemoteDocument::new(doc.clone(), url)),
            None => Err(JsonLdError::new(JsonLdErrorCode::LoadingDocumentFailed)),
        }
    }
}

#[test]
fn remote_context_loads_through_an_explicit_loader() {
    let mut m = BTreeMap::new();
    m.insert(
        "http://vocab.example/ctx.jsonld".to_string(),
        r#"{"@context": {"name": "http://schema.org/name"}}"#.to_string(),
    );
    let loader = MapLoader(m);
    let ac = ActiveContext::new(Some(BASE))
        .process(
            &Json::Str("http://vocab.example/ctx.jsonld".into()),
            Some(BASE),
            &loader,
            &JsonLdOptions::default(),
        )
        .unwrap();
    assert_eq!(
        ac.term_definition("name").unwrap().iri(),
        Some("http://schema.org/name")
    );
}

#[test]
fn import_merges_a_remote_context_under_the_local_one() {
    let mut m = BTreeMap::new();
    m.insert(
        "http://vocab.example/base.jsonld".to_string(),
        r#"{"@context": {"name": "http://schema.org/name"}}"#.to_string(),
    );
    let loader = MapLoader(m);
    let ctx = json(
        r#"{"@version": 1.1, "@import": "http://vocab.example/base.jsonld", "age": "http://schema.org/age"}"#,
    );
    let ac = ActiveContext::new(Some(BASE))
        .process(&ctx, Some(BASE), &loader, &JsonLdOptions::default())
        .unwrap();
    // The imported term and the local term both resolve.
    assert_eq!(
        ac.term_definition("name").unwrap().iri(),
        Some("http://schema.org/name")
    );
    assert_eq!(
        ac.term_definition("age").unwrap().iri(),
        Some("http://schema.org/age")
    );
}

#[test]
fn base_after_a_remote_context_in_the_same_array_still_applies() {
    // Regression: `remote contexts` is per-invocation, so a remote context earlier in the
    // array must not suppress a sibling local `@base` (the remote-context stack is popped).
    let mut m = BTreeMap::new();
    m.insert(
        "http://vocab.example/c.jsonld".to_string(),
        r#"{"@context": {"name": "http://schema.org/name"}}"#.to_string(),
    );
    let loader = MapLoader(m);
    let ctx = json(r#"["http://vocab.example/c.jsonld", {"@base": "http://after.example/"}]"#);
    let ac = ActiveContext::new(Some(BASE))
        .process(&ctx, Some(BASE), &loader, &JsonLdOptions::default())
        .unwrap();
    assert_eq!(
        ac.term_definition("name").unwrap().iri(),
        Some("http://schema.org/name")
    );
    assert_eq!(ac.base_iri(), Some("http://after.example/"));
}

#[test]
fn fs_loader_serves_a_remote_context_from_a_fixture_file() {
    // Exercises the crate's own FsLoader on a temp fixture (still no network).
    let dir = std::env::temp_dir().join(format!("sparq-jsonld-ctx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = dir.join("ctx.jsonld");
    std::fs::write(&fixture, r#"{"@context": {"title": "http://purl.org/dc/terms/title"}}"#).unwrap();

    let loader = FsLoader::new().map_prefix("http://vocab.example/", &dir);
    let ac = ActiveContext::new(Some(BASE))
        .process(
            &Json::Str("http://vocab.example/ctx.jsonld".into()),
            Some(BASE),
            &loader,
            &JsonLdOptions::default(),
        )
        .unwrap();
    assert_eq!(
        ac.term_definition("title").unwrap().iri(),
        Some("http://purl.org/dc/terms/title")
    );

    std::fs::remove_dir_all(&dir).ok();
}
