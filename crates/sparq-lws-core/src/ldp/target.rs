// AUTHORED-BY Claude Opus 4.8
//! LDP target / request-URL parsing.
//!
//! The verifier needs the reconstructed request URL (scheme/host/port/path; query+fragment
//! stripped) for the DPoP `htu` check, and the store keys resources by their absolute IRI. This
//! module derives both from the request parts against a configured public base URL.
//!
//! It is pure value logic (no I/O), so it ports directly from the production server's `target`
//! helper and is exhaustively unit-testable.

use crate::error::ServerError;

/// A parsed LDP request target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LdpTarget {
    /// The absolute resource IRI (base + path), used as the store key.
    pub iri: String,
    /// The reconstructed request URL for the DPoP `htu` check: scheme/host/port/path, with the
    /// query and fragment stripped. For LDP resources this equals [`LdpTarget::iri`].
    pub htu: String,
    /// Whether the target names a container (a trailing-slash path).
    pub is_container: bool,
}

/// Parse a request path against the public base URL into an [`LdpTarget`].
///
/// `base` is the server's public origin (e.g. `https://pod.example`), WITHOUT a trailing slash.
/// `path` is the request path (e.g. `/alice/data`), which MUST start with `/`.
///
/// The query string and fragment are stripped (the DPoP `htu` is path-only per RFC 9449 §4.3, and
/// LDP targets are query-independent). Path traversal (`..`, `.`) is rejected (a 400), as are
/// non-absolute paths.
pub fn parse_target(base: &str, path: &str) -> Result<LdpTarget, ServerError> {
    let base = base.trim_end_matches('/');
    if base.is_empty() {
        return Err(ServerError::BadRequest("empty base url".into()));
    }

    // Strip query + fragment — htu is path-only and LDP targets are query-independent.
    let path_only = path.split(['?', '#']).next().unwrap_or("");
    if !path_only.starts_with('/') {
        return Err(ServerError::BadRequest("path must be absolute".into()));
    }

    // The reserved provider-identity namespace (`/.identity/**` — see `crate::identity` and
    // `research/lws-design-records.md` §4) is refused OUTRIGHT on the LDP surface: 404 for every
    // method, %-decoded too, REGARDLESS of the identity feature flag — so no `.acl` can ever exist
    // for it and no WAC grant can ever apply to an id-doc. The identity gate middleware refuses it
    // first (outermost); this check is the belt-and-braces chokepoint covering every handler and
    // any internally-constructed target re-validated through `parse_target`.
    if crate::identity::is_reserved_identity_path(path_only) {
        return Err(ServerError::NotFound);
    }

    // A trailing-slash path names a container — INCLUDING the storage root "/" itself (the root is a
    // `ldp:BasicContainer`, so a `GET /` must render its `ldp:contains` listing, not be treated as a
    // plain resource).
    let is_container = path_only.ends_with('/');

    // Split into interior segments: drop the leading empty segment (before the first '/') and a
    // single trailing empty segment (the container slash). Any remaining empty segment is an
    // interior "//", which is rejected. "." / ".." in any segment is path traversal — rejected.
    let trimmed = path_only.trim_start_matches('/').trim_end_matches('/');
    if !trimmed.is_empty() {
        for seg in trimmed.split('/') {
            if seg.is_empty() {
                return Err(ServerError::BadRequest(
                    "empty path segment rejected".into(),
                ));
            }
            if seg == ".." || seg == "." {
                return Err(ServerError::BadRequest("path traversal rejected".into()));
            }
        }
    }

    let iri = format!("{base}{path_only}");
    Ok(LdpTarget {
        htu: iri.clone(),
        iri,
        is_container,
    })
}
