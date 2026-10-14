#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

use sparq_jsonld::{DocumentLoader, JsonLdError, NoopLoader, RemoteDocument};

/// A document loader backed exclusively by context documents embedded in this crate.
///
/// `RegistryLoader` performs exact IRI matching and never accesses the network. Unknown
/// IRIs are delegated to [`NoopLoader`], preserving its fail-closed error code and detail.
#[derive(Clone, Copy, Debug, Default)]
pub struct RegistryLoader;

// sq-ypu93 — byte-for-byte embedded contexts keep lookup deterministic and
// independent of the filesystem and network at runtime.
const CONTEXTS: &[(&str, &str)] = &[
    (
        "https://schema.org/docs/jsonldcontext.jsonld",
        include_str!("../contexts/schema-org.jsonld"),
    ),
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

impl RegistryLoader {
    /// Creates the stateless bundled registry loader.
    pub const fn new() -> Self {
        Self
    }

    /// Returns the exact vendored bytes for a registered context IRI.
    pub fn get(url: &str) -> Option<&'static str> {
        CONTEXTS
            .iter()
            .find_map(|(iri, document)| (*iri == url).then_some(*document))
    }
}

impl DocumentLoader for RegistryLoader {
    fn load_document(&self, url: &str) -> Result<RemoteDocument, JsonLdError> {
        match Self::get(url) {
            Some(document) => {
                let mut remote = RemoteDocument::new(document, url);
                remote.content_type = Some("application/ld+json".to_string());
                Ok(remote)
            }
            None => NoopLoader.load_document(url),
        }
    }
}
