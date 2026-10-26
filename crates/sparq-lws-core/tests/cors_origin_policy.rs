//! Origin policy through both protocol routers and the production overload stack.

mod common;

use std::net::SocketAddr;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use common::{jwks_provider, KeyKit, BASE_URL, ISSUER};
use solid_oidc_verifier::{
    config::VerifierConfig, replay::InMemoryReplayStore, verifier::Verifier,
};
use sparq_lws_core::app::{
    build_router, build_router_with_overload, with_overload_layers_and_cors, AppState,
    OverloadConfig,
};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::identity::IdentityConfig;
use sparq_lws_core::ldp::{cors::CorsPolicy, handler::LdpState};
use sparq_lws_core::lws::{self, LwsConfig};
use sparq_lws_core::rate_limit::RateLimiter;
use sparq_lws_core::seed::seed_conformance_with_identity;
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient};
use tower::ServiceExt;

const LIST: &str = "HTTPS://App.Example:8443, http://Other.Example, https://[2001:DB8::1]:9443";
const ALLOWED: &str = "https://aPP.eXAMPLE:8443";
const UNLISTED: &str = "https://elsewhere.example";

async fn app(
    lws: bool,
    policy: CorsPolicy,
    overload: Option<OverloadConfig>,
    identity: bool,
) -> Router {
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    if lws {
        let mut cfg = LwsConfig::new(BASE_URL);
        cfg.open = true;
        cfg.cors = policy.clone();
        let router = lws::router(store, cfg).await.unwrap();
        return match overload {
            Some(overload) => with_overload_layers_and_cors(router, overload, policy),
            None => router,
        };
    }
    let key = KeyKit::generate();
    let cfg = VerifierConfig::new(vec![ISSUER.to_owned()], BASE_URL);
    let replay = InMemoryReplayStore::with_window(cfg.replay_ttl());
    let verifier = Verifier::new(cfg, jwks_provider(&key), replay).unwrap();
    let identity = identity.then(|| IdentityConfig::new(BASE_URL, None).unwrap());
    if let Some(identity) = &identity {
        seed_conformance_with_identity(&store, BASE_URL, ISSUER, Some(identity))
            .await
            .unwrap();
    }
    let mut state = AppState::new(
        AuthContext::new(verifier, BASE_URL),
        LdpState::new(store, BASE_URL),
    );
    state.cors = policy;
    state.identity = identity;
    match overload {
        Some(overload) => build_router_with_overload(state, overload),
        None => build_router(state),
    }
}

fn request(method: &str, path: &str, origin: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "pod.example")
        .extension(ConnectInfo(
            "203.0.113.10:5000".parse::<SocketAddr>().unwrap(),
        ));
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    if method == "OPTIONS" {
        builder = builder
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "PUT")
            .header(
                header::ACCESS_CONTROL_REQUEST_HEADERS,
                "Authorization, DPoP",
            );
    }
    builder.body(Body::empty()).unwrap()
}

fn assert_vary(headers: &HeaderMap) {
    let origins = headers
        .get_all(header::VARY)
        .iter()
        .flat_map(|value| value.to_str().unwrap().split(','))
        .filter(|value| value.trim() == "Origin")
        .count();
    assert_eq!(origins, 1, "Vary must contain Origin exactly once");
}

fn assert_allowed(response: &Response, origin: &str) {
    assert_eq!(
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .map(|value| value.to_str().unwrap()),
        Some(origin),
        "listed Origin must be reflected",
    );
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_CREDENTIALS],
        "true"
    );
    assert_vary(response.headers());
}

fn assert_unlisted(response: &Response) {
    for name in response.headers().keys() {
        assert!(
            !name.as_str().starts_with("access-control-allow-"),
            "unlisted origin received {name}"
        );
    }
    assert_vary(response.headers());
}

#[tokio::test]
async fn unset_policy_preserves_origin_reflection_in_both_modes() {
    for lws in [false, true] {
        let app = app(
            lws,
            CorsPolicy::default(),
            Some(OverloadConfig::new(8, None)),
            false,
        )
        .await;
        for origin in [ALLOWED, UNLISTED, "http://arbitrary.example:9217", "null"] {
            for method in ["GET", "OPTIONS"] {
                let response = app
                    .clone()
                    .oneshot(request(method, "/", Some(origin)))
                    .await
                    .unwrap();
                assert_allowed(&response, origin);
                if method == "OPTIONS" {
                    assert_eq!(response.status(), StatusCode::NO_CONTENT);
                }
            }
        }
    }
}

#[tokio::test]
async fn allowlist_matches_only_exact_origins_with_ascii_case_folding_in_both_modes() {
    for lws in [false, true] {
        // Exercise both the inner router alone and the complete production stack.
        for overload in [None, Some(OverloadConfig::new(8, None))] {
            let app = app(
                lws,
                CorsPolicy::from_origins(LIST).unwrap(),
                overload,
                false,
            )
            .await;
            for origin in [
                ALLOWED,
                "HTTP://OTHER.EXAMPLE",
                "https://[2001:db8::1]:9443",
            ] {
                for method in ["GET", "OPTIONS"] {
                    let response = app
                        .clone()
                        .oneshot(request(method, "/", Some(origin)))
                        .await
                        .unwrap();
                    assert_allowed(&response, origin);
                    if method == "OPTIONS" {
                        assert_eq!(response.status(), StatusCode::NO_CONTENT);
                        assert!(response
                            .headers()
                            .contains_key(header::ACCESS_CONTROL_ALLOW_METHODS));
                        assert_eq!(
                            response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS],
                            "Authorization, DPoP"
                        );
                    }
                }
            }
            for origin in [
                UNLISTED,
                "https://app.example",
                "http://app.example:8443",
                "https://app.example:9443",
                "https://app.example:8443.evil.example",
                "https://sub.app.example:8443",
                "https://app.example:8443/",
                "null",
                "https://user@app.example:8443",
                "https://app.example:8443 https://other.example",
            ] {
                for method in ["GET", "OPTIONS"] {
                    let response = app
                        .clone()
                        .oneshot(request(method, "/", Some(origin)))
                        .await
                        .unwrap();
                    assert_unlisted(&response);
                    assert_eq!(
                        response.status(),
                        if method == "OPTIONS" {
                            StatusCode::NO_CONTENT
                        } else if lws {
                            StatusCode::OK
                        } else {
                            StatusCode::UNAUTHORIZED
                        }
                    );
                    if method == "OPTIONS" {
                        assert!(to_bytes(response.into_body(), 1024)
                            .await
                            .unwrap()
                            .is_empty());
                    }
                }
            }
            let mut malformed = request("GET", "/", Some(ALLOWED));
            malformed.headers_mut().insert(
                header::ORIGIN,
                axum::http::HeaderValue::from_bytes(b"https://\xff").unwrap(),
            );
            assert_unlisted(&app.clone().oneshot(malformed).await.unwrap());
            // CORS does not change the actual response to the same anonymous request.
            let without = app
                .clone()
                .oneshot(request("GET", "/", None))
                .await
                .unwrap();
            let unlisted = app
                .clone()
                .oneshot(request("GET", "/", Some(UNLISTED)))
                .await
                .unwrap();
            assert_unlisted(&without);
            assert_eq!(unlisted.status(), without.status());
            assert_eq!(
                to_bytes(unlisted.into_body(), 1024 * 1024).await.unwrap(),
                to_bytes(without.into_body(), 1024 * 1024).await.unwrap(),
            );
        }
    }
}

#[tokio::test]
async fn explicit_ports_are_not_inferred_or_normalized() {
    let app = app(
        true,
        CorsPolicy::from_origins(" https://app.example , https://app.example:0443 ").unwrap(),
        None,
        false,
    )
    .await;
    for origin in ["HTTPS://APP.EXAMPLE", "https://app.example:0443"] {
        let response = app
            .clone()
            .oneshot(request("GET", "/", Some(origin)))
            .await
            .unwrap();
        assert_allowed(&response, origin);
    }
    let response = app
        .oneshot(request("GET", "/", Some("https://app.example:443")))
        .await
        .unwrap();
    assert_unlisted(&response);
}

#[tokio::test]
async fn overload_rejections_apply_the_allowlist_in_both_modes() {
    for lws in [false, true] {
        for method in ["GET", "OPTIONS"] {
            for origin in [ALLOWED, UNLISTED] {
                let mut overload = OverloadConfig::new(8, None);
                // One token per day keeps the second request below the refill threshold.
                overload.rate_limiter = Some(RateLimiter::new(1.0 / 86400.0, 1.0, 0, false, false));
                let app = app(
                    lws,
                    CorsPolicy::from_origins(LIST).unwrap(),
                    Some(overload),
                    false,
                )
                .await;
                let admitted = app
                    .clone()
                    .oneshot(request(method, "/", Some(origin)))
                    .await
                    .unwrap();
                assert_ne!(admitted.status(), StatusCode::TOO_MANY_REQUESTS);
                let rejected = app
                    .oneshot(request(method, "/", Some(origin)))
                    .await
                    .unwrap();
                assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
                assert!(rejected.headers().contains_key(header::RETRY_AFTER));
                if origin == ALLOWED {
                    assert_allowed(&rejected, origin);
                } else {
                    assert_unlisted(&rejected);
                }
                assert!(!rejected
                    .headers()
                    .contains_key(header::ACCESS_CONTROL_ALLOW_METHODS));
            }
        }
    }
}

#[tokio::test]
async fn identity_document_wildcard_never_has_credentials_and_respects_the_allowlist() {
    for policy in [
        CorsPolicy::default(),
        CorsPolicy::from_origins(LIST).unwrap(),
    ] {
        let app = app(false, policy, Some(OverloadConfig::new(8, None)), true).await;
        for method in ["GET", "HEAD", "POST"] {
            let mut req = request(method, "/alice", Some(ALLOWED));
            req.headers_mut()
                .insert(header::HOST, "id.pod.example".parse().unwrap());
            let response = app.clone().oneshot(req).await.unwrap();
            assert_eq!(
                response.status(),
                if method == "POST" {
                    StatusCode::METHOD_NOT_ALLOWED
                } else {
                    StatusCode::OK
                }
            );
            assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
            assert!(
                !response
                    .headers()
                    .contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS),
                "wildcard response must not enable credentials"
            );
            assert_vary(response.headers());
        }
    }
    for overload in [None, Some(OverloadConfig::new(8, None))] {
        let app = app(
            false,
            CorsPolicy::from_origins(LIST).unwrap(),
            overload,
            true,
        )
        .await;
        for origin in [UNLISTED, ALLOWED] {
            let mut req = request("GET", "/alice", Some(origin));
            req.headers_mut()
                .insert(header::HOST, "id.pod.example".parse().unwrap());
            let response = app.clone().oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            if origin == UNLISTED {
                assert_unlisted(&response);
            } else {
                assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
                assert!(!response
                    .headers()
                    .contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS));
                assert_vary(response.headers());
            }
        }
    }
}

#[test]
fn malformed_allowlist_refuses_boot_in_both_modes() {
    for mode in ["solid", "lws"] {
        for entry in [
            "*",
            "",
            "https://app.example,",
            "not-an-origin",
            "null",
            "https://app.example/",
            "https://app.example/path",
            "https://user@app.example",
            "https://app.example?query",
            "https://app.example#fragment",
            "https://*.example",
            "https://app.example:65536",
            "https://app.example:",
            "https://[bad-ipv6]",
            "https://app.example\\other",
            "https://app.example https://other.example",
            "https://app.example, *",
        ] {
            let mut child = Command::new(env!("CARGO_BIN_EXE_sparq-lws-core"))
                .env_clear()
                .env("SOLID_SERVER_PROTOCOL", mode)
                .env("SOLID_SERVER_CORS_ORIGINS", entry)
                .env("SOLID_SERVER_BIND", "127.0.0.1:0")
                .env("PSS_SPARQ_BACKEND", "memory")
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    let output = child.wait_with_output().unwrap();
                    panic!(
                        "{mode} boot did not refuse {entry:?}: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            let output = child.wait_with_output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!status.success(), "{mode} accepted {entry:?}");
            assert!(
                stderr.contains("SOLID_SERVER_CORS_ORIGINS") && stderr.contains("invalid origin"),
                "{mode} {entry:?}: {stderr}"
            );
        }
    }
}

struct RunningServer {
    child: std::process::Child,
    log: std::path::PathBuf,
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.log);
    }
}

#[tokio::test]
async fn binary_threads_the_environment_policy_through_both_modes() {
    for mode in ["solid", "lws"] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let base = format!("http://{address}");
        drop(listener);
        let log =
            std::env::temp_dir().join(format!("sparq-cors-{}-{mode}.log", std::process::id()));
        let log_file = std::fs::File::create_new(&log).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_sparq-lws-core"))
            .env_clear()
            .env("SOLID_SERVER_PROTOCOL", mode)
            .env("SOLID_SERVER_CORS_ORIGINS", LIST)
            .env("SOLID_SERVER_BIND", address.to_string())
            .env("SOLID_SERVER_BASE_URL", &base)
            .env("SOLID_SERVER_RATE_LIMIT_PER_IP", "off")
            .env("PSS_SPARQ_BACKEND", "memory")
            .stdout(Stdio::null())
            .stderr(log_file)
            .spawn()
            .unwrap();
        let mut server = RunningServer { child, log };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if client.get(format!("{base}/livez")).send().await.is_ok() {
                break;
            }
            assert!(
                server.child.try_wait().unwrap().is_none() && Instant::now() < deadline,
                "{mode} failed to start: {}",
                std::fs::read_to_string(&server.log).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        for origin in [ALLOWED, UNLISTED] {
            for method in [reqwest::Method::GET, reqwest::Method::OPTIONS] {
                let preflight = method == reqwest::Method::OPTIONS;
                let mut request = client
                    .request(method, format!("{base}/"))
                    .header(header::ORIGIN, origin);
                if preflight {
                    request = request.header(header::ACCESS_CONTROL_REQUEST_METHOD, "PUT");
                }
                let response = request.send().await.unwrap();
                if preflight {
                    assert_eq!(response.status(), StatusCode::NO_CONTENT);
                }
                assert_vary(response.headers());
                if origin == ALLOWED {
                    assert_eq!(
                        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
                        origin
                    );
                    assert_eq!(
                        response.headers()[header::ACCESS_CONTROL_ALLOW_CREDENTIALS],
                        "true"
                    );
                } else {
                    assert!(
                        !response
                            .headers()
                            .keys()
                            .any(|name| name.as_str().starts_with("access-control-allow-")),
                        "{mode} binary granted CORS to an unlisted origin"
                    );
                }
            }
        }
    }
}
