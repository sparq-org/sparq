//! Document loading — the [`DocumentLoader`] trait, **deny-by-default**.
//!
//! (sq-oy1f.23) The pipeline dereferences remote `@context`, `@import`, and
//! remote-document references **only** through a [`DocumentLoader`]. The default loader is
//! [`NoopLoader`], which refuses every load and raises
//! [`JsonLdErrorCode::LoadingDocumentFailed`]: enabling JSON-LD gives a surface **no ambient
//! network** (design record §5). The network
//! `HttpLoader` with its SSRF allowlist, and the conformance `MockLoader`, arrive in bead
//! `sq-oy1f.32` behind an opt-in feature; this scaffold ships the trait, the
//! [`RemoteDocument`] carrier, the fail-closed [`NoopLoader`], and the local-fixture
//! [`FsLoader`].

use std::collections::BTreeMap;
use std::path::{Component, PathBuf};

use crate::error::{JsonLdError, JsonLdErrorCode};

/// A retrieved document (the spec's `RemoteDocument`).
///
/// `document` is the raw retrieved text — this scaffold does not yet parse it (the JSON
/// parser lands with the algorithms in `sq-oy1f.24`+).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDocument {
    /// The retrieved document, as unparsed text.
    pub document: String,
    /// The final URL after any redirects (the base for relative references in the document).
    pub document_url: String,
    /// The `Content-Type` of the retrieved document, if known.
    pub content_type: Option<String>,
    /// A context URL supplied out-of-band (e.g. an HTTP `Link rel="…json-ld#context"`).
    pub context_url: Option<String>,
    /// The `profile` parameter of the content type, if any.
    pub profile: Option<String>,
}

impl RemoteDocument {
    /// A minimal remote document carrying only its text and final URL.
    pub fn new(document: impl Into<String>, document_url: impl Into<String>) -> Self {
        RemoteDocument {
            document: document.into(),
            document_url: document_url.into(),
            content_type: None,
            context_url: None,
            profile: None,
        }
    }
}

/// Retrieves a document for a URL (the spec's `LoadDocumentCallback`). Implementations that
/// dereference the network **must** enforce an explicit allowlist; there is no ambient
/// network access by default.
pub trait DocumentLoader {
    /// Loads the document at `url`, or fails with a JSON-LD error code. A loader that cannot
    /// (or will not) retrieve `url` returns [`JsonLdErrorCode::LoadingDocumentFailed`].
    fn load_document(&self, url: &str) -> Result<RemoteDocument, JsonLdError>;
}

/// The default, **fail-closed** loader: every load is refused. This is what a surface gets
/// unless it explicitly opts into a network- or fixture-backed loader, so merely enabling
/// JSON-LD never grants ambient network access.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopLoader;

impl DocumentLoader for NoopLoader {
    fn load_document(&self, url: &str) -> Result<RemoteDocument, JsonLdError> {
        Err(JsonLdError::with_detail(
            JsonLdErrorCode::LoadingDocumentFailed,
            format!("remote document loading is disabled (NoopLoader): {}", url),
        ))
    }
}

/// A local-filesystem loader that maps URL **prefixes** to fixture directories. Intended for
/// **trusted local fixtures** (e.g. the offline conformance suite) — never for
/// attacker-supplied URLs. A URL whose prefix is not registered, or that would escape its
/// fixture directory, is refused with `loading document failed`.
#[derive(Clone, Debug, Default)]
pub struct FsLoader {
    // Prefix → directory. A BTreeMap keeps prefix iteration deterministic; longest-prefix
    // wins so nested mappings resolve unambiguously.
    prefixes: BTreeMap<String, PathBuf>,
}

impl FsLoader {
    /// A loader with no mappings (refuses every URL until [`FsLoader::map_prefix`] is called).
    pub fn new() -> Self {
        FsLoader::default()
    }

    /// Registers a URL-prefix → fixture-directory mapping. Returns `self` for chaining.
    pub fn map_prefix(mut self, url_prefix: impl Into<String>, dir: impl Into<PathBuf>) -> Self {
        self.prefixes.insert(url_prefix.into(), dir.into());
        self
    }

    /// Resolves `url` to a fixture path under the longest matching prefix, rejecting any
    /// remainder that is absolute or contains a `..` component (fixture-escape guard).
    fn resolve(&self, url: &str) -> Option<PathBuf> {
        let (prefix, dir) = self
            .prefixes
            .iter()
            .filter(|(p, _)| url.starts_with(p.as_str()))
            .max_by_key(|(p, _)| p.len())?;
        let rel = &url[prefix.len()..];
        let rel = rel.trim_start_matches('/');
        let rel_path = PathBuf::from(rel);
        // Fixture-escape guard: reject absolute paths and any parent-dir traversal.
        if rel_path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
        {
            return None;
        }
        Some(dir.join(rel_path))
    }

    /// Guesses a content type from a fixture path's extension.
    fn content_type_for(path: &std::path::Path) -> Option<String> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("jsonld") => Some("application/ld+json".to_string()),
            Some("json") => Some("application/json".to_string()),
            Some("html") | Some("htm") => Some("text/html".to_string()),
            _ => None,
        }
    }
}

impl DocumentLoader for FsLoader {
    fn load_document(&self, url: &str) -> Result<RemoteDocument, JsonLdError> {
        let deny = |detail: String| JsonLdError::with_detail(JsonLdErrorCode::LoadingDocumentFailed, detail);
        let path = self
            .resolve(url)
            .ok_or_else(|| deny(format!("no fixture mapping for {}", url)))?;
        let text = std::fs::read_to_string(&path)
            .map_err(|e| deny(format!("{}: {}", path.display(), e)))?;
        let content_type = FsLoader::content_type_for(&path);
        Ok(RemoteDocument {
            document: text,
            document_url: url.to_string(),
            content_type,
            context_url: None,
            profile: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_loader_denies_every_load_fail_closed() {
        let loader = NoopLoader;
        let err = loader
            .load_document("https://example.org/ctx.jsonld")
            .unwrap_err();
        // The load-bearing invariant: the default loader refuses, fail-closed, with the
        // spec's `loading document failed` code — no ambient network.
        assert_eq!(err.code(), JsonLdErrorCode::LoadingDocumentFailed);
    }

    #[test]
    fn remote_document_new_sets_text_and_url() {
        let d = RemoteDocument::new("{}", "https://example.org/x");
        assert_eq!(d.document, "{}");
        assert_eq!(d.document_url, "https://example.org/x");
        assert_eq!(d.content_type, None);
        assert_eq!(d.context_url, None);
        assert_eq!(d.profile, None);
    }

    #[test]
    fn fs_loader_new_is_empty_and_denies_unmapped() {
        let loader = FsLoader::new();
        let err = loader.load_document("https://example.org/a.jsonld").unwrap_err();
        assert_eq!(err.code(), JsonLdErrorCode::LoadingDocumentFailed);
    }

    #[test]
    fn fs_loader_map_prefix_reads_a_local_fixture() {
        let dir = std::env::temp_dir().join(format!("sparq-jsonld-fs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fixture = dir.join("ctx.jsonld");
        std::fs::write(&fixture, r#"{"@context":{}}"#).unwrap();

        let loader = FsLoader::new().map_prefix("https://vocab.example/", &dir);
        let doc = loader
            .load_document("https://vocab.example/ctx.jsonld")
            .unwrap();
        assert_eq!(doc.document, r#"{"@context":{}}"#);
        assert_eq!(doc.document_url, "https://vocab.example/ctx.jsonld");
        assert_eq!(doc.content_type.as_deref(), Some("application/ld+json"));

        std::fs::remove_file(&fixture).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn fs_loader_rejects_parent_dir_escape() {
        let dir = std::env::temp_dir();
        let loader = FsLoader::new().map_prefix("https://vocab.example/", &dir);
        // A `..` traversal must be refused, not resolved outside the fixture dir.
        let err = loader
            .load_document("https://vocab.example/../etc/passwd")
            .unwrap_err();
        assert_eq!(err.code(), JsonLdErrorCode::LoadingDocumentFailed);
    }

    #[test]
    fn fs_loader_longest_prefix_wins() {
        let outer = std::env::temp_dir().join(format!("sparq-jsonld-outer-{}", std::process::id()));
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(outer.join("a.json"), "OUTER").unwrap();
        std::fs::write(inner.join("a.json"), "INNER").unwrap();

        let loader = FsLoader::new()
            .map_prefix("https://ex/", &outer)
            .map_prefix("https://ex/inner/", &inner);
        let doc = loader.load_document("https://ex/inner/a.json").unwrap();
        assert_eq!(doc.document, "INNER");

        std::fs::remove_dir_all(&outer).ok();
    }
}
