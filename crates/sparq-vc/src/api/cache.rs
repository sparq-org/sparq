//! Startup-loaded context documents. Neither loader performs I/O during requests.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use sparq_jsonld::{DocumentLoader, JsonLdError, JsonLdErrorCode, RemoteDocument};

use super::ApiError;

/// Required URL → filename mappings for an offline VC-API cache.
pub const CONTEXT_FILES: &[(&str, &str)] = &[
    (
        "https://www.w3.org/ns/credentials/v2",
        "credentials-v2.jsonld",
    ),
    (
        "https://www.w3.org/2018/credentials/v1",
        "credentials-v1.jsonld",
    ),
    (
        "https://www.w3.org/ns/credentials/examples/v2",
        "examples-v2.jsonld",
    ),
    (
        "https://w3id.org/security/data-integrity/v2",
        "data-integrity-v2.jsonld",
    ),
    (
        "https://w3id.org/security/data-integrity/v1",
        "data-integrity-v1.jsonld",
    ),
    (
        "https://w3id.org/security/multikey/v1",
        "multikey-v1.jsonld",
    ),
    ("https://www.w3.org/ns/did/v1", "did-v1.jsonld"),
];

/// Immutable cache shared by all requests; unknown URLs are errors.
#[derive(Clone)]
pub struct OfflineContexts(Arc<BTreeMap<String, String>>);

impl OfflineContexts {
    /// Load all [`CONTEXT_FILES`] from a trusted directory at service startup.
    /// Context bytes are retained exactly as staged.
    pub fn from_directory(path: &Path) -> Result<Self, ApiError> {
        let documents = CONTEXT_FILES
            .iter()
            .map(|(url, file)| {
                let text = std::fs::read_to_string(path.join(file))
                    .map_err(|_| ApiError::new(format!("missing offline context: {file}")))?;
                Ok(((*url).to_owned(), text))
            })
            .collect::<Result<BTreeMap<_, _>, ApiError>>()?;
        Self::from_documents(documents)
    }

    /// Construct a cache from trusted local documents (also useful in socket-free hosts).
    /// Every required URL must have a JSON object containing `@context`.
    pub fn from_documents(documents: BTreeMap<String, String>) -> Result<Self, ApiError> {
        for (url, _) in CONTEXT_FILES {
            let text = documents
                .get(*url)
                .ok_or_else(|| ApiError::new("incomplete offline context cache"))?;
            let value: serde_json::Value = serde_json::from_str(text)
                .map_err(|_| ApiError::new("invalid JSON in offline context cache"))?;
            if !value.is_object() || value.get("@context").is_none() {
                return Err(ApiError::new("offline context must contain @context"));
            }
        }
        Ok(Self(Arc::new(documents)))
    }

    pub(super) fn bytes(&self, url: &str) -> Result<&str, ApiError> {
        self.0
            .get(url)
            .map(String::as_str)
            .ok_or_else(|| ApiError::new("URL is absent from the offline cache"))
    }
}

impl DocumentLoader for OfflineContexts {
    fn load_document(&self, url: &str) -> Result<RemoteDocument, JsonLdError> {
        self.bytes(url)
            .map(|text| RemoteDocument::new(text, url))
            .map_err(|_| {
                JsonLdError::with_detail(
                    JsonLdErrorCode::LoadingDocumentFailed,
                    "URL is absent from the offline cache",
                )
            })
    }
}
