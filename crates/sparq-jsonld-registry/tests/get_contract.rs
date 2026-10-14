use sparq_jsonld::{DocumentLoader, Json};
use sparq_jsonld_registry::RegistryLoader;

// sq-bif.27 — enumerate the public registry contract so additions do not
// accidentally replace coverage for one of the four currently vendored contexts.
const REGISTERED_IRIS: &[&str] = &[
    "https://schema.org/docs/jsonldcontext.jsonld",
    "https://www.w3.org/ns/activitystreams",
    "https://www.w3.org/2018/credentials/v1",
    "https://www.w3.org/ns/did/v1",
];

#[test]
fn get_returns_parseable_json_for_every_registered_iri() {
    assert_eq!(REGISTERED_IRIS.len(), 4);

    for &iri in REGISTERED_IRIS {
        let document = RegistryLoader::get(iri)
            .unwrap_or_else(|| panic!("registered context was not found: {iri}"));
        assert!(!document.is_empty(), "registered context is empty: {iri}");
        Json::parse(document).unwrap_or_else(|error| {
            panic!("registered context is not valid JSON ({iri}): {error}")
        });
    }
}

#[test]
fn get_requires_exact_iri_equality() {
    // sq-bif.27 — near matches must remain unresolved so the offline
    // registry cannot silently widen its allowlist.
    for iri in [
        "https://schema.org/docs/jsonldcontext.jsonld/",
        "",
        "HTTPS://www.w3.org/ns/activitystreams",
    ] {
        assert!(
            RegistryLoader::get(iri).is_none(),
            "near-match IRI unexpectedly resolved: {iri}"
        );
    }
}

#[test]
fn registered_load_reports_json_ld_content_type() {
    let document = RegistryLoader::new()
        .load_document(REGISTERED_IRIS[0])
        .expect("registered context should load");

    assert_eq!(
        document.content_type.as_deref(),
        Some("application/ld+json")
    );
}
