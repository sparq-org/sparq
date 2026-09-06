// AUTHORED-BY Claude Fable 5
//! Provider-issued WebIDs hosted OUTSIDE the pod — the identity host.
//!
//! See `research/lws-design-records.md` §4 — the reconstruction of RSS
//! `docs/design/webid-outside-pod.md`, itself the RSS adaptation of prod-solid-server's PSS
//! `decisions/0020-webid-outside-pod.md`. The WebID document is the Solid-OIDC identity trust root:
//! every resource server on the web dereferences it to learn which issuers may mint tokens for that
//! WebID (`solid:oidcIssuer`). Hosting it INSIDE the pod — a WAC-governed, owner-writable resource
//! — leaves it one over-broad `acl:default` grant away from ecosystem-wide identity takeover. This
//! module bakes the separation in from the start:
//!
//! - **WebID form:** `https://<identity-host>/<handle>#me` (document at
//!   `https://<identity-host>/<handle>`); default host `id.<base authority>`.
//! - **Storage:** id-docs live under the **reserved internal namespace**
//!   `<base>/.identity/<handle>` — a key space that is OUTSIDE the LDP-resource→storage mapping:
//!   no containment edge is ever recorded for it (it appears in no `ldp:contains` listing), the
//!   LDP surface refuses the whole namespace outright (`is_reserved_identity_path` — 404, every
//!   method, every origin, %-decoded too, **regardless of the identity feature flag**), and so no
//!   `.acl` exists or can ever be created for it — **no WAC grant can ever apply to an id-doc**.
//!   That impossibility, not a policy check, is the security property. When the SPARQ-authoritative
//!   WAC design (`sparq#992`) lands, the id-docs move to a dedicated SPARQ **named graph** excluded
//!   from the WAC evaluation scope by construction (the design doc records the mapping).
//! - **Serving:** the Host-keyed `identity_gate_middleware` (the outermost application layer)
//!   serves `GET`/`HEAD /{handle}` with Turtle/JSON-LD conneg, an ETag/`If-None-Match` 304, public
//!   cache headers and an explicit `Access-Control-Allow-Origin: *`; **no WAC evaluation, no
//!   `.acl` Link, no `WWW-Authenticate`, no auth processing at all**; every other method → `405`;
//!   anything not exactly one valid non-reserved handle → `404` fail-closed.
//! - **Writes:** id-docs are written ONLY through the `Store` seam by boot seeding today
//!   ([`crate::seed::seed_conformance_with_identity`]) and by the future admin provisioning seam —
//!   never through the LDP path (which refuses the namespace).

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::ldp::conditional;
use crate::ldp::content::{
    classify, negotiate_accept_with_profile, parse_to_triples, serialize_triples_negotiated,
};
use crate::ldp::handler::LdpState;
use crate::store::Store;

/// The reserved LDP path segment under which id-docs are stored (`<base>/.identity/<handle>`).
///
/// The DOTTED prefix is load-bearing: the handle grammar ([`is_valid_handle`]) admits no `.`, so no
/// handle can ever collide with the namespace, and the LDP surface refuses the whole subtree
/// unconditionally ([`is_reserved_identity_path`]).
pub const RESERVED_SEGMENT: &str = ".identity";

/// Handles that can never be minted/served even though they match the handle grammar.
///
/// - `identity` — reserves the namespace's own (undotted) name.
/// - `livez` / `readyz` — the health probes are mounted OUTSIDE the identity gate (deliberately
///   overload-exempt), so a request for `/<probe>` on the id host is answered by the probe route,
///   not this module; reserving the names makes that shadowing explicit and keeps a future
///   provisioner from ever minting them.
pub const RESERVED_HANDLES: [&str; 3] = ["identity", "livez", "readyz"];

/// The identity-host configuration (the analogue of prod-solid-server's `PSS_IDENTITY_HOST`).
///
/// Built once at boot ([`IdentityConfig::new`]) and carried into the router assembly
/// (`crate::app::AppState::with_identity`). `None` (the default) keeps the serving half OFF —
/// but the LDP-surface refusal of `/.identity/**` holds regardless (flag-independent), so
/// pre-seeded documents can never become LDP-addressable when the flag later turns on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityConfig {
    /// The id-host authority (`host` or `host:port`), lowercased. The gate matches the request's
    /// `Host` header (HTTP/1.1) / `:authority` (HTTP/2) against this EXACTLY (after lowercasing) —
    /// exact-Host routing, fail-closed.
    host: String,
    /// The origin (`scheme://host`) minted into id-doc IRIs / WebIDs.
    origin: String,
}

impl IdentityConfig {
    /// Build the identity config from the server's public base URL plus an optional host override
    /// (the `SOLID_SERVER_IDENTITY_HOST` value). The default host is `id.<base authority>` —
    /// DERIVED rather than hard-coded so the server stays deployment-agnostic (the ADR-0020
    /// convention: `id.solid-test.jeswr.org` on a live deploy, `id.localhost:3000` in dev).
    ///
    /// Fails (fail-closed at boot) when:
    /// - the base URL has no parseable authority,
    /// - the override is empty / contains `/` or whitespace (not an authority),
    /// - the id host EQUALS the base authority — that misconfiguration would swallow ALL LDP
    ///   traffic into the identity gate.
    pub fn new(base_url: &str, host_override: Option<&str>) -> Result<Self, String> {
        let (scheme, base_authority) = split_origin(base_url)
            .ok_or_else(|| format!("identity: base URL {base_url:?} has no authority"))?;
        let host = match host_override.map(str::trim).filter(|h| !h.is_empty()) {
            Some(h) => {
                if h.contains('/') || h.chars().any(char::is_whitespace) {
                    return Err(format!(
                        "identity: host {h:?} is not a bare authority (host[:port])"
                    ));
                }
                h.to_ascii_lowercase()
            }
            None => format!("id.{base_authority}"),
        };
        if host == base_authority {
            return Err(format!(
                "identity: the identity host {host:?} must differ from the base authority \
                 {base_authority:?} (an identical host would route ALL LDP traffic to the id-doc \
                 surface)"
            ));
        }
        let origin = format!("{scheme}://{host}");
        Ok(Self { host, origin })
    }

    /// The id-host authority the gate matches the request `Host` against (lowercased).
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The identity origin (`scheme://host`) — the prefix of every minted WebID.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// The PUBLIC id-doc IRI for a handle: `<origin>/<handle>` (the WebID is `<doc>#me`).
    pub fn doc_iri(&self, handle: &str) -> String {
        format!("{}/{handle}", self.origin)
    }

    /// The WebID for a handle: `<origin>/<handle>#me`.
    pub fn webid(&self, handle: &str) -> String {
        format!("{}/{handle}#me", self.origin)
    }
}

/// Split `scheme://authority[/...]` into `(scheme, lowercased authority)`.
fn split_origin(url: &str) -> Option<(&str, String)> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }
    Some((scheme, authority.to_ascii_lowercase()))
}

/// The RESERVED store key of a handle's id-doc: `<base>/.identity/<handle>`.
///
/// This key is deliberately NOT the served (id-host) IRI: the document's own subject IRIs are on
/// the identity origin ([`IdentityConfig::doc_iri`]); the reserved key is only where its bytes
/// live in the store — a namespace the LDP surface refuses outright.
pub fn reserved_doc_iri(base_url: &str, handle: &str) -> String {
    let base = base_url.trim_end_matches('/');
    format!("{base}/{RESERVED_SEGMENT}/{handle}")
}

/// Whether a request PATH targets the reserved identity namespace — `/.identity` or
/// `/.identity/**` as its FIRST segment, matched on both the RAW and the PERCENT-DECODED form,
/// case-insensitively (defence-in-depth: the raw store key can only ever be the lowercase dotted
/// form, but a future case- or percent-normalising backend must not re-open the namespace).
///
/// This predicate is the single chokepoint both refusal sites use: the identity gate (outermost,
/// every route) and [`crate::ldp::target::parse_target`] (belt-and-braces — covers every handler
/// plus any internally-constructed target that re-validates through it).
pub fn is_reserved_identity_path(path: &str) -> bool {
    first_segment_is_reserved(path) || first_segment_is_reserved(&percent_decode_lossy(path))
}

/// Whether the first path segment equals [`RESERVED_SEGMENT`] (ASCII case-insensitive).
fn first_segment_is_reserved(path: &str) -> bool {
    let trimmed = path.trim_start_matches('/');
    let first = trimmed.split('/').next().unwrap_or("");
    first.eq_ignore_ascii_case(RESERVED_SEGMENT)
}

/// Lossy percent-decoding for the reserved-namespace check: valid `%XX` pairs are decoded, an
/// invalid escape is kept verbatim (it cannot address a store key anyway — LDP store keys are the
/// RAW path), and non-UTF-8 decodes are replaced lossily. Used ONLY for the fail-closed refusal
/// predicate, never to construct a key.
fn percent_decode_lossy(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| -> Option<u8> {
                match b {
                    b'0'..=b'9' => Some(b - b'0'),
                    b'a'..=b'f' => Some(b - b'a' + 10),
                    b'A'..=b'F' => Some(b - b'A' + 10),
                    _ => None,
                }
            };
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The fail-closed handle grammar: 1–64 chars, ASCII lowercase alphanumeric plus interior `-`,
/// starting with an alphanumeric, and not a [`RESERVED_HANDLES`] name. No `.` is admitted, so no
/// handle can ever name the dotted reserved segment; no `%` is admitted, so only the canonical
/// (undecoded) form of a handle addresses its id-doc — anything else is a 404, never a decode.
pub fn is_valid_handle(handle: &str) -> bool {
    if handle.is_empty() || handle.len() > 64 {
        return false;
    }
    let mut chars = handle.chars();
    let first = chars.next().expect("non-empty checked above");
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return false;
    }
    !RESERVED_HANDLES.contains(&handle)
}

/// The identity gate's shared state: the (optional) serving config + the LDP state whose store the
/// id-docs are read from. The GATE is mounted unconditionally; only the SERVING half is
/// config-gated — see [`identity_gate_middleware`].
pub struct IdentityGate<S: Store> {
    /// `Some` ⇒ id-host serving is enabled; `None` ⇒ only the unconditional namespace refusal runs.
    pub serving: Option<IdentityConfig>,
    /// The shared LDP state (store + base URL). The gate reads id-docs through the SAME store the
    /// LDP surface uses — at the reserved keys the LDP surface itself can never address.
    pub ldp: Arc<LdpState<S>>,
}

/// The identity gate — the OUTERMOST application middleware (mounted in
/// [`crate::app::build_router`] around every app route). Two halves:
///
/// 1. **The unconditional namespace refusal.** A request whose path targets `/.identity/**`
///    (raw or %-decoded) is answered `404` immediately — every method, every origin, every Host,
///    BEFORE auth/WAC/storage, regardless of the identity flag. This is what makes "no `.acl`
///    can ever exist for the namespace" a construction-level property.
/// 2. **Exact-Host id-doc serving** (only when [`IdentityGate::serving`] is `Some`). A request
///    whose `Host` matches the configured id host is handled ENTIRELY here and never reaches the
///    LDP/auth stack: `GET`/`HEAD /{handle}` serves the id-doc (conneg, ETag/304, public cache,
///    `ACAO: *` — and NO `WWW-Authenticate`, NO `.acl` Link, NO WAC); any other method is `405`
///    (`Allow: GET, HEAD`); any other path shape is a fail-closed `404`.
///
/// A request matching neither falls through unchanged (`next.run`) — the gate can only ever
/// ANSWER EARLIER with less, never grant more, so it is not an authorization bypass surface: the
/// id-docs it serves are world-readable by design (a WebID must dereference anonymously).
pub async fn identity_gate_middleware<S: Store>(
    State(state): State<Arc<IdentityGate<S>>>,
    req: Request,
    next: Next,
) -> Response {
    // Half 1 — the unconditional refusal (flag-independent, every method/origin/Host).
    if is_reserved_identity_path(req.uri().path()) {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    // Half 2 — exact-Host id-doc serving (config-gated).
    let id_serving = state
        .serving
        .as_ref()
        .filter(|config| request_host_matches(&req, config.host()));
    if let Some(config) = id_serving {
        // Extract the OWNED request facts up front and drop the request BEFORE the (awaiting)
        // serve: the request body is `!Sync`, so borrowing the request across an await would make
        // this middleware's future `!Send`. The id-doc surface never reads a body anyway.
        let method = req.method().clone();
        let path = req.uri().path().to_owned();
        let accept = header_string(&req, header::ACCEPT);
        let if_none_match = header_string(&req, header::IF_NONE_MATCH);
        drop(req);
        return serve_identity_request(
            &state,
            config,
            &method,
            &path,
            accept.as_deref(),
            if_none_match.as_deref(),
        )
        .await;
    }

    next.run(req).await
}

/// An owned copy of a request header's string value (`None` when absent / non-UTF-8).
fn header_string(req: &Request, name: header::HeaderName) -> Option<String> {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Whether the request's authority — the `Host` header (HTTP/1.1) or the URI `:authority`
/// (HTTP/2) — equals the configured id host, ASCII-case-insensitively. A missing/unreadable
/// authority never matches (fail-closed: the request falls through to the normal stack).
fn request_host_matches(req: &Request, id_host: &str) -> bool {
    let header_host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok());
    let uri_host = req.uri().authority().map(|a| a.as_str());
    header_host
        .or(uri_host)
        .map(|h| h.trim().eq_ignore_ascii_case(id_host))
        .unwrap_or(false)
}

/// Handle a request addressed to the id host. GET/HEAD of exactly one valid non-reserved handle
/// serves the id-doc; everything else is 405 (non-read method) or 404 (any other path shape).
/// Takes the OWNED request facts (method/path/headers) — never the request itself — so the
/// awaiting future stays `Send` (see the caller).
async fn serve_identity_request<S: Store>(
    state: &IdentityGate<S>,
    config: &IdentityConfig,
    method: &Method,
    path: &str,
    accept: Option<&str>,
    if_none_match: Option<&str>,
) -> Response {
    // Method gate FIRST: the surface is GET/HEAD-only. Every other method — including OPTIONS and
    // every write — is 405 with the honest Allow set. (A browser GET of an id-doc is a "simple"
    // CORS request — `Accept` is safelisted — so no preflight OPTIONS is ever needed.)
    let is_head = *method == Method::HEAD;
    if *method != Method::GET && !is_head {
        return method_not_allowed();
    }

    // Path shape: exactly `/{handle}`, one segment, no trailing slash, valid non-reserved handle.
    // Anything else — the root, a nested path, a percent-encoded or malformed handle — is a
    // fail-closed 404 (never an existence probe, never a decode).
    let handle = match path.strip_prefix('/') {
        Some(h) if is_valid_handle(h) => h,
        _ => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };

    // Read the id-doc from the RESERVED store key. The store is the same seam the LDP surface
    // uses; the key is one the LDP surface refuses to address. No WAC evaluation runs — public by
    // construction: no ACL exists (or can exist) for the reserved namespace, so evaluating WAC
    // here would fail-closed deny and wrongly break every id-host dereference.
    let doc_key = reserved_doc_iri(state.ldp.base_url(), handle);
    let resource = match state.ldp.store.read(&doc_key).await {
        Ok(r) => r,
        Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };

    // Content negotiation between the two RDF formats (Turtle stored; JSON-LD re-serialised).
    let stored = match classify(Some(&resource.meta.content_type)) {
        Ok(f) => f,
        // A non-RDF stored id-doc is a seeding/provisioning bug — fail closed as absent rather
        // than serve unnegotiable bytes as an identity document.
        Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    let negotiated = match negotiate_accept_with_profile(accept, stored) {
        Some(n) => n,
        None => return (StatusCode::NOT_ACCEPTABLE, "not acceptable").into_response(),
    };
    // The stored bytes are the representation ONLY in the stored format with no honoured JSON-LD
    // `profile` — an honoured profile (expanded / compacted) always re-serialises into the
    // requested document form, mirroring the LDP read path's `negotiate_body`.
    let verbatim = negotiated.serves_stored_verbatim(stored);

    // The response validator is REPRESENTATION-SPECIFIC (RFC 9110 §8.8.3 — an entity-tag identifies
    // a representation, not a resource), mirroring the LDP read path's `negotiated_validator`: the
    // STORED format (Turtle) keeps the stored strong ETag; a re-serialised representation gets a
    // DISTINCT variant tag (`"<state>+<variant>"`, via `conditional::variant_etag`, the shared
    // profile-aware `NegotiatedFormat::variant_suffix` token). Computed BEFORE the If-None-Match
    // check so a 304 short-circuit uses the tag THIS request's representation would carry — a
    // client holding the Turtle tag but asking for JSON-LD therefore gets a fresh 200, never a 304
    // for bytes it never received (the cross-representation-304 bug).
    let etag = if verbatim {
        resource.meta.etag.clone()
    } else {
        conditional::variant_etag(&resource.meta.etag, negotiated.variant_suffix())
    };

    // Conditional GET: an If-None-Match hit (against the REPRESENTATION-SPECIFIC validator above)
    // answers 304 with the same ETag + cache headers.
    if if_none_match_hits(if_none_match, &etag) {
        let mut resp = StatusCode::NOT_MODIFIED.into_response();
        add_identity_headers(&mut resp, &etag);
        return resp;
    }

    // Re-serialise when the negotiated representation differs from the stored bytes (the other
    // format, or an honoured JSON-LD profile's document form). The parse base is the PUBLIC id-doc
    // IRI (relative IRIs in the doc resolve against the served identity, never the internal
    // reserved key).
    let body_bytes = if verbatim {
        resource.body.to_vec()
    } else {
        let doc_iri = config.doc_iri(handle);
        let triples = match parse_to_triples(stored, &resource.body, &doc_iri) {
            Ok(t) => t,
            Err(_) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
            }
        };
        match serialize_triples_negotiated(negotiated, &triples) {
            Ok(b) => b,
            Err(_) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
            }
        }
    };

    let mut resp = Response::new(if is_head {
        Body::empty()
    } else {
        Body::from(body_bytes)
    });
    // The `Content-Type` echoes any honoured JSON-LD `profile` back to the client (the JSON-LD 1.1
    // IANA registration), matching the LDP read path.
    if let Ok(ct) = HeaderValue::from_str(&negotiated.content_type()) {
        resp.headers_mut().insert(header::CONTENT_TYPE, ct);
    }
    add_identity_headers(&mut resp, &etag);
    resp
}

/// The invariant id-doc response headers: ETag, public cache, `ACAO: *`, `Vary: Accept` — and by
/// omission the contract's negatives: NO `WWW-Authenticate`, NO `.acl`/`describedby` Link, NO
/// `WAC-Allow` (no WAC ran).
fn add_identity_headers(resp: &mut Response, etag: &str) {
    if let Ok(v) = HeaderValue::from_str(etag) {
        resp.headers_mut().insert(header::ETAG, v);
    }
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    resp.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    resp.headers_mut()
        .insert(header::VARY, HeaderValue::from_static("accept"));
}

/// `405 Method Not Allowed` with the honest `Allow: GET, HEAD` + `ACAO: *`.
fn method_not_allowed() -> Response {
    let mut resp = (StatusCode::METHOD_NOT_ALLOWED, "method not allowed").into_response();
    resp.headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
    resp.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    resp
}

/// Whether an `If-None-Match` header value matches `etag` (`*`, or any listed entity-tag, weak
/// prefixes tolerated) — the 304 branch of the id-doc read.
fn if_none_match_hits(raw: Option<&str>, etag: &str) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    raw.split(',').map(str::trim).any(|candidate| {
        candidate == "*" || candidate == etag || candidate.strip_prefix("W/") == Some(etag)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_derives_default_host_and_origin() {
        let c = IdentityConfig::new("https://pod.example", None).unwrap();
        assert_eq!(c.host(), "id.pod.example");
        assert_eq!(c.origin(), "https://id.pod.example");
        assert_eq!(c.webid("alice"), "https://id.pod.example/alice#me");
        assert_eq!(c.doc_iri("alice"), "https://id.pod.example/alice");

        // A dev base keeps its port in the derived authority.
        let dev = IdentityConfig::new("http://localhost:3000", None).unwrap();
        assert_eq!(dev.host(), "id.localhost:3000");
        assert_eq!(dev.origin(), "http://id.localhost:3000");
    }

    #[test]
    fn config_honours_override_and_lowercases() {
        let c = IdentityConfig::new("https://pod.example", Some("ID.Example.ORG")).unwrap();
        assert_eq!(c.host(), "id.example.org");
        assert_eq!(c.origin(), "https://id.example.org");
    }

    #[test]
    fn config_rejects_bad_hosts_fail_closed() {
        // No authority in the base URL.
        assert!(IdentityConfig::new("not-a-url", None).is_err());
        assert!(IdentityConfig::new("https://", None).is_err());
        // An override that is not a bare authority.
        assert!(IdentityConfig::new("https://pod.example", Some("id.example/path")).is_err());
        assert!(IdentityConfig::new("https://pod.example", Some("id host")).is_err());
        // The id host must differ from the base authority (would swallow all LDP traffic).
        assert!(IdentityConfig::new("https://pod.example", Some("pod.example")).is_err());
        assert!(IdentityConfig::new("https://pod.example", Some("POD.EXAMPLE")).is_err());
    }

    #[test]
    fn reserved_path_predicate_matches_raw_and_percent_forms() {
        // The namespace itself + everything under it, raw…
        assert!(is_reserved_identity_path("/.identity"));
        assert!(is_reserved_identity_path("/.identity/"));
        assert!(is_reserved_identity_path("/.identity/alice"));
        assert!(is_reserved_identity_path("/.identity/alice.acl"));
        assert!(is_reserved_identity_path("/.identity/a/b/c"));
        // …percent-encoded (%2E = '.', %69 = 'i', %2F = '/')…
        assert!(is_reserved_identity_path("/%2Eidentity/alice"));
        assert!(is_reserved_identity_path("/%2e%69dentity/alice"));
        assert!(is_reserved_identity_path("/.identity%2Falice"));
        // …and case variants (defence-in-depth).
        assert!(is_reserved_identity_path("/.IDENTITY/alice"));
        assert!(is_reserved_identity_path("/.Identity"));
    }

    #[test]
    fn reserved_path_predicate_leaves_normal_paths_alone() {
        assert!(!is_reserved_identity_path("/"));
        assert!(!is_reserved_identity_path("/alice/profile/card"));
        // Only the ROOT-level segment is reserved.
        assert!(!is_reserved_identity_path("/alice/.identity/x"));
        // The undotted name is a normal path (the HANDLE 'identity' is reserved separately).
        assert!(!is_reserved_identity_path("/identity"));
        // A near-miss segment is not the namespace.
        assert!(!is_reserved_identity_path("/.identity-x/alice"));
        // An invalid percent escape stays verbatim and does not match.
        assert!(!is_reserved_identity_path("/%2identity/alice"));
    }

    #[test]
    fn handle_grammar_is_fail_closed() {
        for ok in ["alice", "bob-2", "a", "0x", "user-name-42"] {
            assert!(is_valid_handle(ok), "{ok:?} must be a valid handle");
        }
        for bad in [
            "",
            "-alice",        // must start alphanumeric
            "Alice",         // lowercase only
            "alice.card",    // no dots — the reserved segment can never be a handle
            ".identity",     // the namespace itself
            "identity",      // reserved name
            "livez",         // shadowed by the health route — reserved
            "readyz",        // shadowed by the health route — reserved
            "al ice",        // whitespace
            "al/ice",        // path separator
            "al%2Fice",      // percent form
            "café",          // non-ASCII
            &"a".repeat(65), // too long
        ] {
            assert!(!is_valid_handle(bad), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn variant_validators_are_representation_specific() {
        // Finding 1: the STORED format (Turtle) keeps the stored strong tag; a re-serialised
        // representation (JSON-LD) gets a DISTINCT variant tag — so an If-None-Match carrying the
        // Turtle tag can never match the JSON-LD representation (no cross-representation 304).
        let stored = "\"42-abc\"";
        let turtle = stored.to_string();
        let negotiated = negotiate_accept_with_profile(
            Some("application/ld+json"),
            crate::ldp::content::RdfFormat::Turtle,
        )
        .expect("acceptable");
        let jsonld = conditional::variant_etag(stored, negotiated.variant_suffix());
        assert_eq!(turtle, "\"42-abc\"");
        assert_eq!(jsonld, "\"42-abc+jsonld\"");
        assert_ne!(
            turtle, jsonld,
            "the two representations must have distinct tags"
        );
        // The If-None-Match matcher agrees: the Turtle tag hits Turtle but NOT the JSON-LD tag.
        assert!(if_none_match_hits(Some(&turtle), &turtle));
        assert!(!if_none_match_hits(Some(&turtle), &jsonld));
        assert!(if_none_match_hits(Some(&jsonld), &jsonld));
    }

    #[test]
    fn reserved_doc_key_is_under_the_refused_namespace() {
        let key = reserved_doc_iri("https://pod.example/", "alice");
        assert_eq!(key, "https://pod.example/.identity/alice");
        // The key's path IS refused by the LDP predicate — the invariant that makes the id-doc
        // unreachable (and un-ACL-able) through the LDP surface.
        assert!(is_reserved_identity_path("/.identity/alice"));
    }
}
