use sparq_jsonld::{expand, DocumentLoader, Json, JsonLdErrorCode, JsonLdOptions};
use sparq_jsonld_registry::RegistryLoader;

struct ContextCase {
    iri: &'static str,
    compact: &'static str,
    expanded: &'static str,
}

// sq-lpj2j — exercise term/type expansion through every bundled context,
// rather than treating successfully parsed vendored bytes as proof of their semantics.
const CONTEXT_CASES: &[ContextCase] = &[
    ContextCase {
        iri: "https://schema.org/docs/jsonldcontext.jsonld",
        compact: r#"{"@context":"https://schema.org/docs/jsonldcontext.jsonld","name":"Ada"}"#,
        expanded: r#"[{"https://schema.org/name":[{"@value":"Ada"}]}]"#,
    },
    ContextCase {
        iri: "https://www.w3.org/ns/activitystreams",
        compact: r#"{"@context":"https://www.w3.org/ns/activitystreams","@type":"Note"}"#,
        expanded: r#"[{"@type":["https://www.w3.org/ns/activitystreams#Note"]}]"#,
    },
    ContextCase {
        iri: "https://www.w3.org/2018/credentials/v1",
        compact: r#"{"@context":"https://www.w3.org/2018/credentials/v1","@type":"VerifiableCredential"}"#,
        expanded: r#"[{"@type":["https://www.w3.org/2018/credentials#VerifiableCredential"]}]"#,
    },
    ContextCase {
        iri: "https://www.w3.org/ns/did/v1",
        compact: r#"{"@context":"https://www.w3.org/ns/did/v1","controller":"did:example:123"}"#,
        expanded: r#"[{"https://www.w3.org/ns/did#controller":[{"@id":"did:example:123"}]}]"#,
    },
];

#[test]
fn every_bundled_context_has_expected_semantics_and_metadata() {
    assert_eq!(CONTEXT_CASES.len(), 4);

    let loader = RegistryLoader::new();
    for case in CONTEXT_CASES {
        let document = loader
            .load_document(case.iri)
            .unwrap_or_else(|error| panic!("failed to load bundled context {}: {error}", case.iri));
        assert_eq!(document.document_url, case.iri);
        assert_eq!(
            document.content_type.as_deref(),
            Some("application/ld+json"),
            "unexpected content type for {}",
            case.iri
        );

        let compact = Json::parse(case.compact)
            .unwrap_or_else(|error| panic!("invalid compact fixture for {}: {error}", case.iri));
        let actual = expand(&compact, &JsonLdOptions::default(), &loader)
            .unwrap_or_else(|error| panic!("failed to expand fixture for {}: {error}", case.iri));
        let expected = Json::parse(case.expanded)
            .unwrap_or_else(|error| panic!("invalid expanded fixture for {}: {error}", case.iri));
        assert_eq!(
            actual, expected,
            "semantic expansion differs for {}",
            case.iri
        );
    }
}

#[test]
fn unknown_context_fails_closed() {
    let error = RegistryLoader::new()
        .load_document("https://example.invalid/not-bundled")
        .unwrap_err();
    assert_eq!(error.code(), JsonLdErrorCode::LoadingDocumentFailed);
}
