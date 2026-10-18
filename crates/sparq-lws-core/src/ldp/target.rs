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

/// Whether the store key `key` is a linkset record of the LWS surface: a non-container whose last
/// segment ends in `.meta` (ASCII case-insensitive). Store keys are raw, so nothing is decoded. A
/// container `X.meta/` and what lies under it are ordinary resources. Used to keep a stored record
/// from deciding an LDP outcome (the slash rule, listings, group documents).
pub(crate) fn names_linkset_record(key: &str) -> bool {
    key.len() >= LINKSET_RECORD_SUFFIX.len()
        && key.as_bytes()[key.len() - LINKSET_RECORD_SUFFIX.len()..]
            .eq_ignore_ascii_case(LINKSET_RECORD_SUFFIX.as_bytes())
}

/// Whether the request path `path` (without query or fragment) names a linkset record of the LWS
/// surface ([`names_linkset_record`]), as sent or %-decoded. Used only to refuse a target.
pub(crate) fn is_linkset_record_path(path: &str) -> bool {
    names_linkset_record(path) || names_linkset_record(&crate::identity::percent_decode_lossy(path))
}

/// The segment under which the LWS surface keeps its service records (grants, access requests,
/// subscriptions) in the store, directly below its storage root (`crate::lws::GRANTS_PATH` and its
/// siblings).
const LWS_SERVICE_SEGMENT: &str = ".lws";

/// Whether `path` (a path from the storage root, starting with `/`) lies in the LWS surface's
/// service namespace: its first non-empty segment is `.lws` (ASCII case-insensitive), so a
/// repeated leading slash does not take a path out of it.
fn names_lws_service(path: &str) -> bool {
    path.starts_with('/')
        && path
            .trim_start_matches('/')
            .split('/')
            .next()
            .is_some_and(|seg| seg.eq_ignore_ascii_case(LWS_SERVICE_SEGMENT))
}

/// Whether the request path `path` (without query or fragment) lies in the LWS surface's service
/// namespace ([`names_lws_service`]), as sent or %-decoded. A `.lws` segment deeper down is an
/// ordinary name. Used only to refuse a target.
pub(crate) fn is_lws_service_path(path: &str) -> bool {
    names_lws_service(path) || names_lws_service(&crate::identity::percent_decode_lossy(path))
}

/// Whether the store key `key` is a service record of the LWS surface kept under `base` (the
/// public base URL): its path from `base` lies in the service namespace. Raw, like every store key.
pub(crate) fn names_lws_service_record(key: &str, base: &str) -> bool {
    key.strip_prefix(base.trim_end_matches('/'))
        .is_some_and(names_lws_service)
}

/// Whether the path segment `seg` is a dot segment (`.` or `..`), as sent or %-decoded (`%2E`,
/// `%2e%2E`, `.%2E`): a name that a client or intermediary normalising the URL would resolve away
/// is never a resource name.
fn is_dot_segment(seg: &str) -> bool {
    let is_dots = |s: &str| s == "." || s == "..";
    is_dots(seg) || (seg.contains('%') && is_dots(&crate::identity::percent_decode_lossy(seg)))
}

/// Parse a request path against the public base URL into an [`LdpTarget`].
///
/// `base` is the server's public origin (e.g. `https://pod.example`), WITHOUT a trailing slash.
/// `path` is the request path (e.g. `/alice/data`), which MUST start with `/`.
///
/// The query string and fragment are stripped (the DPoP `htu` is path-only per RFC 9449 §4.3, and
/// LDP targets are query-independent). Path traversal (`..`, `.`, as sent or %-decoded) is
/// rejected (a 400), as are non-absolute paths.
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

    // Split into interior segments: leading and trailing slashes are dropped for this check (the
    // store key keeps them). Any remaining empty segment is an interior "//", which is rejected.
    // "." / ".." in any segment is path traversal — rejected.
    let trimmed = path_only.trim_start_matches('/').trim_end_matches('/');
    if !trimmed.is_empty() {
        for seg in trimmed.split('/') {
            if seg.is_empty() {
                return Err(ServerError::BadRequest(
                    "empty path segment rejected".into(),
                ));
            }
            if is_dot_segment(seg) {
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

    /// A dot segment is refused as sent and %-decoded, in any case; a name merely containing
    /// dots or an encoded dot is not one.
    #[test]
    fn an_encoded_dot_segment_is_refused() {
        let base = "https://pod.example";
        for path in [
            "/a/%2E/b",
            "/a/%2e%2E/b",
            "/a/.%2E/b",
            "/a/%2E.",
            "/%2E%2E/",
            "/a/..",
            "/a/./b",
        ] {
            assert!(
                matches!(
                    super::parse_target(base, path),
                    Err(crate::error::ServerError::BadRequest(_))
                ),
                "{path}"
            );
        }
        for path in ["/a/%2E%2E%2E", "/a/x%2E", "/a/...", "/a/%2Ex"] {
            assert!(super::parse_target(base, path).is_ok(), "{path}");
        }
    }

    /// Store keys are raw: an encoded name is not the record's key, though a request naming it
    /// is refused.
    #[test]
    fn a_store_key_is_matched_as_stored() {
        let base = "https://pod.example";
        assert!(super::names_linkset_record(
            "https://pod.example/a/doc.META"
        ));
        assert!(!super::names_linkset_record(
            "https://pod.example/a/doc%2Emeta"
        ));
        assert!(!super::names_linkset_record(
            "https://pod.example/a/doc.meta/"
        ));
        assert!(super::is_linkset_record_path("/a/doc%2Emeta"));
        assert!(super::names_lws_service_record(
            "https://pod.example/.lws/requests/r",
            base
        ));
        assert!(!super::names_lws_service_record(
            "https://pod.example/%2Elws/requests/r",
            base
        ));
        assert!(!super::names_lws_service_record(
            "https://pod.example/a/.lws/r",
            base
        ));
        assert!(!super::names_lws_service_record(
            "https://other.example/.lws/r",
            base
        ));
    }
}
