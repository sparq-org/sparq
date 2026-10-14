use sparq_jsonld::{expand, DocumentLoader, Json, JsonLdErrorCode, JsonLdOptions, NoopLoader};
use sparq_jsonld_registry::RegistryLoader;

const SCHEMA_CONTEXT: &str = include_str!("../contexts/schema-org.jsonld");
const SCHEMA_IRI: &str = "https://schema.org/docs/jsonldcontext.jsonld";
const REGISTERED_CONTEXTS: &[(&str, &str)] = &[
    (SCHEMA_IRI, SCHEMA_CONTEXT),
    (
        "https://www.w3.org/ns/activitystreams",
        include_str!("../contexts/activitystreams.jsonld"),
    ),
    (
        "https://www.w3.org/2018/credentials/v1",
        include_str!("../contexts/credentials-v1.jsonld"),
    ),
    (
        "https://www.w3.org/ns/did/v1",
        include_str!("../contexts/did-v1.jsonld"),
    ),
];

#[test]
fn registered_context_round_trips_vendored_bytes() {
    let document = RegistryLoader::new().load_document(SCHEMA_IRI).unwrap();
    assert_eq!(document.document.as_bytes(), SCHEMA_CONTEXT.as_bytes());
    assert_eq!(document.document_url, SCHEMA_IRI);
    assert_eq!(
        document.content_type.as_deref(),
        Some("application/ld+json")
    );
}

#[test]
fn every_registered_context_load_is_complete_and_idempotent() {
    // sq-sw1yr — pin the complete registry so deleting an expected entry
    // cannot silently reduce the coverage of the per-context assertions below.
    assert_eq!(REGISTERED_CONTEXTS.len(), 4);

    let loader = RegistryLoader::new();
    for &(iri, embedded_document) in REGISTERED_CONTEXTS {
        let first = loader
            .load_document(iri)
            .unwrap_or_else(|error| panic!("failed to load registered context {iri}: {error}"));
        let second = loader.load_document(iri).unwrap_or_else(|error| {
            panic!("failed to load registered context {iri} a second time: {error}")
        });

        assert_eq!(first, second, "successive loads differ for {iri}");
        assert_eq!(first.document.as_bytes(), embedded_document.as_bytes());
        assert_eq!(first.document_url, iri);
        assert_eq!(
            first.content_type.as_deref(),
            Some("application/ld+json"),
            "registered context {iri} has no JSON-LD content type"
        );
        Json::parse(&first.document)
            .unwrap_or_else(|error| panic!("registered context {iri} is not JSON: {error}"));
    }
}

#[test]
fn unregistered_context_has_exact_noop_denial() {
    let iri = "https://example.invalid/unregistered-context";
    let registry_error = RegistryLoader::new().load_document(iri).unwrap_err();
    let noop_error = NoopLoader.load_document(iri).unwrap_err();
    assert_eq!(
        registry_error.code(),
        JsonLdErrorCode::LoadingDocumentFailed
    );
    assert_eq!(registry_error.code(), noop_error.code());
    assert_eq!(registry_error.detail(), noop_error.detail());
}

#[test]
fn schema_context_expands_fully_offline() {
    let input = Json::parse(
        r#"{"@context":"https://schema.org/docs/jsonldcontext.jsonld","@type":"Person","name":"Ada"}"#,
    )
    .unwrap();
    let noop_error = expand(&input, &JsonLdOptions::default(), &NoopLoader).unwrap_err();
    assert_eq!(
        noop_error.code(),
        JsonLdErrorCode::LoadingRemoteContextFailed
    );

    let expanded = expand(&input, &JsonLdOptions::default(), &RegistryLoader::new()).unwrap();
    let expected = Json::parse(
        r#"[{"@type":["https://schema.org/Person"],"https://schema.org/name":[{"@value":"Ada"}]}]"#,
    )
    .unwrap();
    assert_eq!(expanded, expected);
}
