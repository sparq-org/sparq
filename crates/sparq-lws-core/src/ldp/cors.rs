// AUTHORED-BY Claude Opus 4.8
//! The Solid Protocol CORS middleware.
//!
//! Solid is a browser-first protocol: an app at one origin reads/writes a pod at another, so the
//! server MUST implement CORS (Solid Protocol §CORS, deferring to the Fetch standard). This is a
//! hand-rolled axum middleware (rather than `tower-http`'s `CorsLayer`) because the Solid Conformance
//! Test Harness makes two assertions `tower-http` does not satisfy out of the box:
//!
//! 1. **`Vary: Origin`** must contain the literal `Origin` (capital O) — the harness's
//!    `match header Vary contains 'Origin'` is case-SENSITIVE, and `tower-http` emits a lowercased
//!    `vary: origin, …`.
//! 2. **`Access-Control-Expose-Headers`** must be present on the PREFLIGHT (OPTIONS) response too —
//!    `tower-http` adds expose-headers only to non-preflight responses, but the harness asserts it on
//!    every CORS response including the preflight.
//!
//! The posture:
//! - **`Access-Control-Allow-Origin` REFLECTS the request `Origin`** verbatim (never a bare `*` — a
//!   credentialed request requires an exact origin, and the harness asserts ACAO == its origin).
//! - **`Access-Control-Allow-Credentials: true`** (the browser may send the DPoP/Authorization
//!   credentials cross-origin) — which is WHY the origin is reflected, not `*`.
//! - **`Vary: Origin`** (capital), so a cache never serves one origin's CORS response to another.
//! - A **preflight** (`OPTIONS` with `Access-Control-Request-Method`) is answered with an EMPTY 204 +
//!   `Access-Control-Allow-Methods` (the LDP verb set) + `Access-Control-Allow-Headers` REFLECTING the
//!   requested `Access-Control-Request-Headers` (so `X-CUSTOM, Content-Type, Accept` are echoed) +
//!   `Access-Control-Expose-Headers`.
//! - A **simple/actual** request's response carries ACAO + ACAC + expose-headers + `Vary: Origin`.
//!
//! The middleware is the OUTERMOST layer, so the CORS headers ride on EVERY response (incl. the
//! anonymous 401 the `cors-simple-requests` scenario asserts ACAO on) and a preflight is answered
//! before auth runs (a browser preflight carries no credentials).

use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// The methods advertised in a preflight `Access-Control-Allow-Methods` — the LDP verb set, plus
/// `QUERY` for LWS type search.
const ALLOW_METHODS: &str = "OPTIONS, HEAD, GET, PUT, POST, DELETE, PATCH, QUERY";

/// The response headers a cross-origin script may read. A CONCRETE enumeration (the harness rejects
/// `*`, which is also invalid alongside `Allow-Credentials: true`). Covers the LDP/Solid surface a
/// client reads cross-origin: entity validators, content metadata, the discovery `Link`s, the
/// `Location`/`Allow`/`Accept-*` advertisements, the `WWW-Authenticate` challenge, and `WAC-Allow`.
const EXPOSE_HEADERS: &str = "Accept-Ranges, Content-Length, Content-Range, Content-Type, ETag, \
     Last-Modified, Link, Location, Vary, Allow, WWW-Authenticate, Accept-Post, Accept-Patch, \
     Updates-Via, WAC-Allow, Accept-Query, Content-Location";

/// The default `Access-Control-Allow-Headers` for a preflight that did NOT send
/// `Access-Control-Request-Headers` (a safe baseline covering the Solid auth + content headers).
const DEFAULT_ALLOW_HEADERS: &str = "Authorization, DPoP, Content-Type, Accept, Slug, Link, \
     If-Match, If-None-Match, Range";

/// The axum CORS middleware. Reflects the `Origin` + the preflight's requested headers; answers a
/// preflight itself; otherwise runs the inner service and decorates the response.
pub async fn cors_middleware(req: Request, next: Next) -> Response {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let is_options = req.method() == Method::OPTIONS;
    let acrm_present = req
        .headers()
        .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);
    // Reflect the preflight's requested headers (echo them in Allow-Headers), or the default set.
    let requested_headers = req
        .headers()
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    // A PREFLIGHT (OPTIONS + Access-Control-Request-Method) is answered HERE with an empty 204 — it
    // does not reach the inner service (a browser preflight carries no credentials).
    if is_options && acrm_present {
        let mut resp = StatusCode::NO_CONTENT.into_response();
        let h = resp.headers_mut();
        decorate_common(h, origin.as_deref());
        set(h, header::ACCESS_CONTROL_ALLOW_METHODS, ALLOW_METHODS);
        let allow_headers = requested_headers
            .as_deref()
            .unwrap_or(DEFAULT_ALLOW_HEADERS);
        set(h, header::ACCESS_CONTROL_ALLOW_HEADERS, allow_headers);
        return resp;
    }

    // Otherwise run the inner service and decorate the response with the CORS headers (on EVERY
    // response, including an error/401, so a cross-origin script can read the outcome + challenge).
    let mut resp = next.run(req).await;
    decorate_common(resp.headers_mut(), origin.as_deref());
    resp
}

/// The CORS headers common to a preflight and a simple/actual response: reflected ACAO, credentials,
/// the exposed-headers enumeration, and `Vary: Origin`. When there is no `Origin`, only `Vary:
/// Origin` is added (so a cache keys correctly) — no ACAO is emitted for a non-CORS request.
fn decorate_common(headers: &mut HeaderMap, origin: Option<&str>) {
    // `Vary: Origin` (capital O — the harness's `contains 'Origin'` is case-sensitive). Appended so
    // it coexists with any Vary the handler already set (e.g. content negotiation).
    if let Ok(v) = HeaderValue::from_static("Origin")
        .to_str()
        .map(str::to_string)
    {
        // Merge with an existing Vary rather than clobber it.
        merge_vary(headers, &v);
    }
    if let Some(o) = origin {
        if let Ok(value) = HeaderValue::from_str(o) {
            headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        }
        set(headers, header::ACCESS_CONTROL_ALLOW_CREDENTIALS, "true");
        set(
            headers,
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            EXPOSE_HEADERS,
        );
    }
}

/// Append `value` to the `Vary` header, preserving any existing Vary tokens (de-duplicated,
/// case-insensitive on the token).
fn merge_vary(headers: &mut HeaderMap, value: &str) {
    let existing = headers
        .get(header::VARY)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let already = existing
        .split(',')
        .any(|t| t.trim().eq_ignore_ascii_case(value));
    let merged = if existing.trim().is_empty() {
        value.to_string()
    } else if already {
        existing
    } else {
        format!("{existing}, {value}")
    };
    if let Ok(v) = HeaderValue::from_str(&merged) {
        headers.insert(header::VARY, v);
    }
}

/// Insert a static-ish header value, silently skipping a value that cannot be encoded.
fn set(headers: &mut HeaderMap, name: HeaderName, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        headers.insert(name, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    fn app() -> Router {
        Router::new()
            .route("/r", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(cors_middleware))
    }

    #[tokio::test]
    async fn preflight_returns_empty_204_with_reflected_origin_and_headers() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/r")
                    .header("origin", "https://tester")
                    .header("access-control-request-method", "POST")
                    .header(
                        "access-control-request-headers",
                        "X-CUSTOM, Content-Type, Accept",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let h = resp.headers();
        assert_eq!(
            h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
            "https://tester"
        );
        assert!(h
            .get(header::ACCESS_CONTROL_ALLOW_METHODS)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("POST"));
        let allow_headers = h
            .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(allow_headers.contains("X-CUSTOM"));
        assert!(allow_headers.contains("Content-Type"));
        assert!(allow_headers.contains("Accept"));
        // Expose-headers present on the preflight (the harness asserts this).
        let expose = h
            .get(header::ACCESS_CONTROL_EXPOSE_HEADERS)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(!expose.is_empty() && expose != "*");
        // Vary: Origin with a CAPITAL O.
        assert!(h
            .get(header::VARY)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Origin"));
    }

    #[tokio::test]
    async fn simple_request_with_origin_gets_acao_expose_and_vary() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/r")
                    .header("origin", "https://tester")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let h = resp.headers();
        assert_eq!(
            h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
            "https://tester"
        );
        let expose = h
            .get(header::ACCESS_CONTROL_EXPOSE_HEADERS)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(!expose.is_empty() && expose != "*");
        assert!(h
            .get(header::VARY)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Origin"));
    }

    #[tokio::test]
    async fn no_origin_request_gets_no_acao_but_vary_origin() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/r")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let h = resp.headers();
        assert!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
        // Vary: Origin is still set so a cache keys correctly.
        assert!(h
            .get(header::VARY)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Origin"));
    }

    #[tokio::test]
    async fn merges_a_handler_set_vary_accept_with_vary_origin() {
        // The LDP read handler sets `Vary: Accept` (content negotiation — bead vltw); this CORS
        // middleware runs OUTERMOST and must MERGE its own `Vary: Origin` onto that rather than
        // clobber it, so a cache correctly keys on BOTH headers.
        let app = Router::new()
            .route(
                "/r",
                get(|| async {
                    let mut resp = "ok".into_response();
                    resp.headers_mut()
                        .insert(header::VARY, HeaderValue::from_static("Accept"));
                    resp
                }),
            )
            .layer(axum::middleware::from_fn(cors_middleware));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/r")
                    .header("origin", "https://tester")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let vary = resp
            .headers()
            .get(header::VARY)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            vary.contains("Accept"),
            "must keep the handler's Vary: Accept, got {vary:?}"
        );
        assert!(
            vary.contains("Origin"),
            "must ALSO add Vary: Origin, got {vary:?}"
        );
    }

    #[tokio::test]
    async fn plain_options_without_acrm_passes_through_to_the_handler() {
        // A non-preflight OPTIONS (no Access-Control-Request-Method) is NOT short-circuited — it
        // reaches the inner service (here the route has no OPTIONS handler → 405, but the point is the
        // middleware does not intercept it as a preflight).
        let resp = app()
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/r")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // The inner router returns 405 for OPTIONS on a GET-only route; the middleware did not
        // short-circuit it as a preflight (which would have been 204).
        assert_ne!(resp.status(), StatusCode::NO_CONTENT);
    }
}
