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

/// The suffix the LWS surface stores a resource's linkset record under (`crate::lws::META_SUFFIX`,
/// which is not built for every target this module is).
const LINKSET_RECORD_SUFFIX: &str = ".meta";

/// Whether `key` (a request path without query or fragment, or a store key) names a linkset
/// record of the LWS surface: a non-container whose last segment ends in `.meta` (ASCII
/// case-insensitive), as sent or %-decoded. A container `X.meta/` and what lies under it are
/// ordinary resources. Used to refuse such a target, and to keep a stored record from deciding
/// an LDP outcome (the slash rule, listings, group documents).
pub(crate) fn is_linkset_record_path(key: &str) -> bool {
    let names_record = |key: &str| {
        key.len() >= LINKSET_RECORD_SUFFIX.len()
            && key.as_bytes()[key.len() - LINKSET_RECORD_SUFFIX.len()..]
                .eq_ignore_ascii_case(LINKSET_RECORD_SUFFIX.as_bytes())
    };
    names_record(key) || names_record(&crate::identity::percent_decode_lossy(key))
}

/// The segment under which the LWS surface keeps its service records (grants, access requests,
/// subscriptions) in the store, below its storage root (`crate::lws::GRANTS_PATH` and its
/// siblings).
const LWS_SERVICE_SEGMENT: &str = ".lws";

/// Whether `path` lies in the LWS surface's service namespace: a segment is `.lws` (ASCII
/// case-insensitive), as sent or %-decoded. `path` is the path alone. Used only to refuse.
pub(crate) fn is_lws_service_path(path: &str) -> bool {
    let names_service = |path: &str| {
        path.split('/')
            .any(|seg| seg.eq_ignore_ascii_case(LWS_SERVICE_SEGMENT))
    };
    names_service(path) || names_service(&crate::identity::percent_decode_lossy(path))
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

    // The LWS surface keeps each resource's linkset record at `{resource}.meta` in the same
    // store (`crate::lws::META_SUFFIX`). Where both surfaces share a store, such a name is that
    // record and never an LDP resource, so it is refused OUTRIGHT here as well: 404, in any case,
    // %-decoded too, whichever surfaces are enabled.
    if is_linkset_record_path(path_only) {
        return Err(ServerError::NotFound);
    }

    // Nor is anything in its service namespace (`.lws/`), where it keeps grants, access requests
    // and subscriptions in the same store.
    if is_lws_service_path(path_only) {
        return Err(ServerError::NotFound);
    }

    // A trailing-slash path names a container — INCLUDING the storage root "/" itself (the root is a
    // `ldp:BasicContainer`, so a `GET /` must render its `ldp:contains` listing, not be treated as a
    // plain resource).
    let is_container = path_only.ends_with('/');

    // Split into interior segments: drop the leading empty segment (before the first '/') and a
    // single trailing empty segment (the container slash). Any remaining empty segment is a
    // "//" (leading, interior or trailing), which is rejected. "." / ".." in any segment is path
    // traversal — rejected.
    if path_only.contains("//") {
        return Err(ServerError::BadRequest(
            "empty path segment rejected".into(),
        ));
    }
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #[test]
    fn the_reserved_names_are_the_lws_surfaces() {
        assert_eq!(super::LINKSET_RECORD_SUFFIX, crate::lws::META_SUFFIX);
        for path in [
            crate::lws::GRANTS_PATH,
            crate::lws::REQUESTS_PATH,
            crate::lws::SUBSCRIPTIONS_PATH,
        ] {
            assert!(path.starts_with(&format!("/{}/", super::LWS_SERVICE_SEGMENT)));
        }
    }
}
