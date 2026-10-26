//! Browser CORS contract through the native protocol routers and production overload stack.

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use sparq_lws_core::app::{with_overload_layers, OverloadConfig};
use sparq_lws_core::lws::{self, LwsConfig, TYPE_SEARCH_PATH};
use sparq_lws_core::rate_limit::RateLimiter;
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient};
use tower::ServiceExt;

const BASE: &str = "https://lws.example";
const ORIGIN: &str = "https://suite.jeswr.org";
const REQUEST_HEADERS: &str = "authorization, dpop, content-type, if-match, last-event-id";

async fn app(open: bool, overload: OverloadConfig) -> Router {
    let mut cfg = LwsConfig::new(BASE);
    cfg.open = open;
    cfg.max_body = overload.body_limit_bytes;
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    let router = lws::router(store, cfg).await.unwrap();
    // A controlled pending route isolates the production timeout without timing storage work.
    let router = router.route("/pending", get(std::future::pending::<StatusCode>));
    with_overload_layers(router, overload)
}

fn request(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(format!("{BASE}{path}"))
        .header(header::ORIGIN, ORIGIN)
        .extension(ConnectInfo(
            "203.0.113.10:5000".parse::<SocketAddr>().unwrap(),
        ));
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    builder.body(Body::from(body.to_owned())).unwrap()
}

fn tokens(headers: &HeaderMap, name: &str) -> Vec<String> {
    headers
        .get_all(name)
        .iter()
        .flat_map(|value| value.to_str().unwrap().split(','))
        .map(|value| value.trim().to_ascii_lowercase())
        .collect()
}

fn assert_cors(response: &Response) {
    let headers = response.headers();
    let origins: Vec<_> = headers
        .get_all(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .iter()
        .collect();
    assert_eq!(origins.len(), 1);
    assert_eq!(origins[0], ORIGIN);
    assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_CREDENTIALS], "true");
    let exposed = tokens(headers, "access-control-expose-headers");
    assert!(!exposed.iter().any(|value| value == "*"));
    for required in [
        "link",
        "etag",
        "location",
        "www-authenticate",
        "accept-patch",
        "allow",
        "retry-after",
        "preference-applied",
    ] {
        assert!(
            exposed.iter().any(|value| value == required),
            "missing {required}"
        );
    }
    let vary = tokens(headers, "vary");
    assert_eq!(vary.iter().filter(|value| *value == "origin").count(), 1);
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Response {
    let response = app
        .clone()
        .oneshot(request(method, path, headers, body))
        .await
        .unwrap();
    assert_cors(&response);
    response
}

fn preflight() -> Request<Body> {
    request(
        "OPTIONS",
        "/",
        &[
            ("access-control-request-method", "PUT"),
            ("access-control-request-headers", REQUEST_HEADERS),
        ],
        "",
    )
}

#[tokio::test]
async fn preflight_reflects_origin_credentials_methods_and_requested_headers() {
    let app = app(false, OverloadConfig::new(8, None)).await;
    let response = app.clone().oneshot(preflight()).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_cors(&response);
    let methods = tokens(response.headers(), "access-control-allow-methods");
    for required in ["put", "patch", "delete", "query"] {
        assert!(
            methods.iter().any(|value| value == required),
            "missing {required}"
        );
    }
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS],
        REQUEST_HEADERS
    );
    assert!(to_bytes(response.into_body(), 1024)
        .await
        .unwrap()
        .is_empty());

    let response = call(
        &app,
        "OPTIONS",
        "/",
        &[("access-control-request-method", "DELETE")],
        "",
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(tokens(response.headers(), "access-control-allow-headers")
        .iter()
        .any(|value| value == "depth"));
}

#[tokio::test]
async fn resource_methods_and_anonymous_challenge_are_browser_readable() {
    let app = app(true, OverloadConfig::new(8, None)).await;
    let created = call(
        &app,
        "POST",
        "/",
        &[("content-type", "application/json"), ("slug", "data.json")],
        r#"{"value":1}"#,
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let location = created.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string();
    let path = location.strip_prefix(BASE).unwrap();
    let read = call(&app, "GET", path, &[], "").await;
    assert_eq!(read.status(), StatusCode::OK);
    assert_eq!(read.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(
        &to_bytes(read.into_body(), 1024).await.unwrap()[..],
        br#"{"value":1}"#
    );

    let written = call(
        &app,
        "PUT",
        path,
        &[("content-type", "application/json"), ("if-match", "*")],
        r#"{"value":2}"#,
    )
    .await;
    assert_eq!(written.status(), StatusCode::NO_CONTENT);
    let patched = call(
        &app,
        "PATCH",
        path,
        &[
            ("content-type", "application/json-patch+json"),
            ("if-match", "*"),
        ],
        r#"[{"op":"replace","path":"/value","value":3}]"#,
    )
    .await;
    assert_eq!(patched.status(), StatusCode::NO_CONTENT);
    let queried = call(
        &app,
        "QUERY",
        TYPE_SEARCH_PATH,
        &[("content-type", "application/lws-query+json")],
        "{}",
    )
    .await;
    assert_eq!(queried.status(), StatusCode::OK);
    let deleted = call(&app, "DELETE", path, &[("if-match", "*")], "").await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let closed = self::app(false, OverloadConfig::new(8, None)).await;
    let refused = call(
        &closed,
        "GET",
        "/",
        &[("accept", "application/lws+json")],
        "",
    )
    .await;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert!(refused.headers().contains_key(header::WWW_AUTHENTICATE));
}

fn limited() -> OverloadConfig {
    let mut config = OverloadConfig::new(8, None);
    config.rate_limiter = Some(RateLimiter::new(1.0, 1.0, 0, false, false));
    config
}

#[tokio::test]
async fn rate_limit_rejection_is_browser_readable() {
    let app = app(false, limited()).await;
    assert_eq!(
        call(&app, "GET", "/", &[], "").await.status(),
        StatusCode::UNAUTHORIZED
    );
    let rejected = call(&app, "GET", "/", &[], "").await;
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(rejected.headers().contains_key(header::RETRY_AFTER));
}

#[tokio::test]
async fn preflight_flood_is_rate_limited() {
    let app = app(false, limited()).await;
    let admitted = app.clone().oneshot(preflight()).await.unwrap();
    assert_eq!(admitted.status(), StatusCode::NO_CONTENT);
    assert_cors(&admitted);
    let rejected = app.oneshot(preflight()).await.unwrap();
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_cors(&rejected);
    assert!(rejected.headers().contains_key(header::RETRY_AFTER));
    assert!(!rejected
        .headers()
        .contains_key(header::ACCESS_CONTROL_ALLOW_METHODS));
}

#[tokio::test]
async fn admission_rejects_actual_requests_and_preflights_with_readable_cors() {
    let config = OverloadConfig::new(1, None);
    let held = config.admission.try_admit_for_test().unwrap();
    let app = app(false, config).await;
    let rejected = call(&app, "GET", "/", &[], "").await;
    assert_eq!(rejected.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(rejected.headers().contains_key(header::RETRY_AFTER));
    let rejected = app.oneshot(preflight()).await.unwrap();
    assert_eq!(rejected.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_cors(&rejected);
    drop(held);
}

#[tokio::test]
async fn oversize_body_and_timeout_are_browser_readable() {
    let mut config = OverloadConfig::new(8, Some(Duration::from_millis(20)));
    config.body_limit_bytes = 4;
    let app = app(true, config).await;
    let rejected = call(
        &app,
        "POST",
        "/",
        &[("content-type", "text/plain")],
        "too large",
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let timed_out = call(&app, "GET", "/pending", &[], "").await;
    assert_eq!(timed_out.status(), StatusCode::GATEWAY_TIMEOUT);
}

#[tokio::test]
async fn requests_without_origin_vary_without_allow_origin_even_when_shed() {
    let config = OverloadConfig::new(1, None);
    let admission = config.admission.clone();
    let app = app(false, config).await;
    for shed in [false, true] {
        let _held = shed.then(|| admission.try_admit_for_test().unwrap());
        let mut req = request("GET", "/", &[("accept", "application/lws+cid")], "");
        req.headers_mut().remove(header::ORIGIN);
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            if shed {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::OK
            }
        );
        assert!(!response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
        assert!(tokens(response.headers(), "vary")
            .iter()
            .any(|value| value == "origin"));
    }
}

#[tokio::test]
async fn solid_overload_rejections_share_the_browser_contract() {
    use common::{jwks_provider, KeyKit, BASE_URL, ISSUER};
    use solid_oidc_verifier::{
        config::VerifierConfig, replay::InMemoryReplayStore, verifier::Verifier,
    };
    use sparq_lws_core::{
        app::{build_router_with_overload, AppState},
        auth::AuthContext,
        ldp::handler::LdpState,
    };

    let key = KeyKit::generate();
    let config = VerifierConfig::new(vec![ISSUER.to_string()], BASE_URL);
    let replay = InMemoryReplayStore::with_window(config.replay_ttl());
    let verifier = Verifier::new(config, jwks_provider(&key), replay).unwrap();
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    let app = build_router_with_overload(
        AppState::new(
            AuthContext::new(verifier, BASE_URL),
            LdpState::new(store, BASE_URL),
        ),
        limited(),
    );
    let response = call(&app, "GET", "/alice/private", &[], "").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let rejected = call(&app, "GET", "/alice/private", &[], "").await;
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(rejected.headers().contains_key(header::RETRY_AFTER));
}
