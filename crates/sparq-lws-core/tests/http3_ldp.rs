// [GPT-5.6] sq-oprna.2, sq-oprna.5
//! HTTP/3 integration coverage for the Solid/LDP server's default-off `http3` feature.
//!
//! The test serves one production `build_router_with_overload` router over both the existing TLS
//! TCP path and the QUIC helper. A throwaway test CA anchors the checked-in localhost certificate.
//! It compares a representative response matrix over HTTP/1.1 and HTTP/3, proves WebSocket-over-h3
//! is refused while the TCP WebSocket fallback works, and separately exhausts a tight per-IP bucket.
//! If the QUIC peer is not injected as `ConnectInfo<SocketAddr>`, that limiter test fails open to the
//! public LDP handler and returns 200 instead of 429.

#![cfg(feature = "http3")]

mod common;

use std::{error::Error, future::poll_fn, sync::Arc, time::Duration};

use axum::body::{Body, Bytes};
use axum::http::{Method, Request, StatusCode, Version};
use bytes::Buf as _;
use common::{BASE_URL, KeyKit, jwks_provider};
use h3::quic::OpenStreams;
use quinn::crypto::rustls::QuicClientConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::CertificateDer;
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_http3::{quic_server_config, serve_h3};
use sparq_lws_core::app::{AppState, OverloadConfig, build_router_with_overload};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::notifications::NotificationHub;
use sparq_lws_core::overload::AdmissionControl;
use sparq_lws_core::rate_limit::RateLimiter;
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient, Store};
use sparq_lws_core::tls::{TlsMode, build_rustls_config};
use tower::ServiceExt as _;

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

const CERT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-cert.pem");
const KEY_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-key.pem");
const CA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-ca.pem");
const RESOURCE_BODY: &str =
    "<https://pod.example/pub#me> <http://xmlns.com/foaf/0.1/name> \"Alice\" .";

// [GPT-5.6] sq-oprna.5: the invariant deliberately excludes transport-only headers such as Alt-Svc.
#[derive(Debug, PartialEq, Eq)]
struct ResponseSnapshot {
    status: StatusCode,
    content_type: Option<String>,
    body: Bytes,
}

struct RequestCase {
    name: &'static str,
    method: Method,
    path: &'static str,
    accept: Option<&'static str>,
    expected_status: StatusCode,
    body_witness: &'static [u8],
}

async fn production_router(rate: f64, burst: f64) -> (axum::Router, NotificationHub) {
    let issuer_key = KeyKit::generate();
    let config = VerifierConfig::new(vec![common::ISSUER.to_string()], BASE_URL);
    let replay = InMemoryReplayStore::with_window(config.replay_ttl());
    let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay).unwrap();
    let auth = AuthContext::new(verifier, BASE_URL);
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());

    store
        .write(
            "https://pod.example/pub",
            Bytes::from_static(RESOURCE_BODY.as_bytes()),
            "text/turtle",
        )
        .await
        .expect("seed public LDP resource");
    store
        .write(
            "https://pod.example/pub.acl",
            Bytes::from_static(
                br#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
@prefix foaf: <http://xmlns.com/foaf/0.1/>.
<#public> a acl:Authorization;
  acl:agentClass foaf:Agent;
  acl:accessTo <https://pod.example/pub>;
  acl:mode acl:Read."#,
            ),
            "text/turtle",
        )
        .await
        .expect("seed public resource ACL");

    let overload = OverloadConfig {
        admission: AdmissionControl::new(64),
        request_timeout: None,
        rate_limiter: Some(RateLimiter::new(
            rate, burst, /* trusted_proxy_hops */ 0, /* exempt_loopback */ false,
            /* exempt_internal */ false,
        )),
        body_limit_bytes: sparq_lws_core::body_limit::DEFAULT_MAX_BODY_BYTES,
    };
    let ldp = LdpState::new(store, BASE_URL);
    let notifications = ldp.notifications.clone();
    (
        build_router_with_overload(AppState::new(auth, ldp), overload),
        notifications,
    )
}

fn fixture_roots() -> TestResult<rustls::RootCertStore> {
    let pem = std::fs::read(CA_PATH)?;
    let mut roots = rustls::RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(&pem) {
        roots.add(cert?)?;
    }
    Ok(roots)
}

fn client_tls_config(alpn: &[u8]) -> TestResult<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_root_certificates(fixture_roots()?)
        .with_no_client_auth();
    tls.alpn_protocols = vec![alpn.to_vec()];
    Ok(tls)
}

fn h3_client_endpoint() -> TestResult<quinn::Endpoint> {
    let tls = client_tls_config(b"h3")?;
    let client_config = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
    let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse()?)?;
    endpoint.set_default_client_config(client_config);
    Ok(endpoint)
}

async fn h3_request<O>(
    sender: &mut h3::client::SendRequest<O, Bytes>,
    method: Method,
    uri: &str,
    accept: Option<&str>,
    headers: &[(&str, &str)],
) -> TestResult<ResponseSnapshot>
where
    O: OpenStreams<Bytes>,
{
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(value) = accept {
        request = request.header(axum::http::header::ACCEPT, value);
    }
    for &(name, value) in headers {
        request = request.header(name, value);
    }
    let request = request.body(())?;
    let mut stream = sender.send_request(request).await?;
    stream.finish().await?;

    let response = stream.recv_response().await?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut body = Vec::new();
    while let Some(mut chunk) = stream.recv_data().await? {
        let remaining = chunk.remaining();
        body.extend_from_slice(&chunk.copy_to_bytes(remaining));
    }
    Ok(ResponseSnapshot {
        status,
        content_type,
        body: Bytes::from(body),
    })
}

async fn h1_request(
    client: &reqwest::Client,
    server_addr: std::net::SocketAddr,
    case: &RequestCase,
) -> TestResult<ResponseSnapshot> {
    let mut request = client.request(
        reqwest::Method::from_bytes(case.method.as_str().as_bytes())?,
        format!("https://localhost:{}{}", server_addr.port(), case.path),
    );
    if let Some(value) = case.accept {
        request = request.header(reqwest::header::ACCEPT, value);
    }
    let response = request.send().await?;
    assert_eq!(response.version(), reqwest::Version::HTTP_11);
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response.bytes().await?;
    Ok(ResponseSnapshot {
        status,
        content_type,
        body,
    })
}

#[tokio::test]
async fn h3_response_matrix_matches_h1_and_websocket_falls_back() -> TestResult {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let (app, notifications) = production_router(100.0, 100.0).await;
    let mode = TlsMode::Tls {
        cert_path: CERT_PATH.into(),
        key_path: KEY_PATH.into(),
    };
    let tcp_tls = build_rustls_config(&mode, false)
        .await?
        .expect("TLS mode yields a config");
    assert_eq!(
        tcp_tls.get_inner().alpn_protocols,
        vec![b"h2".to_vec(), b"http/1.1".to_vec()],
        "the TCP listener's ALPN contract must remain unchanged"
    );

    let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let server_addr = tcp_listener.local_addr()?;
    let std_listener = tcp_listener.into_std()?;
    std_listener.set_nonblocking(true)?;

    // Clone before adding h3: QUIC and TCP reuse cert/provider/policy, but never share ALPN mutation.
    let mut quic_tls = (*tcp_tls.get_inner()).clone();
    quic_tls.alpn_protocols = vec![b"h3".to_vec()];
    let h3_endpoint = quinn::Endpoint::server(quic_server_config(quic_tls)?, server_addr)?;
    let h3_addr = h3_endpoint.local_addr()?;

    let tcp_handle = axum_server::Handle::new();
    let tcp_server_handle = tcp_handle.clone();
    let tcp_app = app
        .clone()
        .layer(sparq_http3::alt_svc_layer(h3_addr.port()));
    let tcp_server = tokio::spawn(async move {
        axum_server::from_tcp_rustls(std_listener, tcp_tls)
            .expect("construct TLS server")
            .handle(tcp_server_handle)
            .serve(tcp_app.into_make_service_with_connect_info::<std::net::SocketAddr>())
            .await
    });

    let (h3_shutdown_tx, h3_shutdown_rx) = tokio::sync::oneshot::channel();
    let h3_server = tokio::spawn(serve_h3(h3_endpoint, app, async move {
        let _ = h3_shutdown_rx.await;
    }));

    let ca_pem = std::fs::read(CA_PATH)?;
    let ca = reqwest::Certificate::from_pem(&ca_pem)?;
    let h1_client = reqwest::Client::builder()
        .add_root_certificate(ca)
        .resolve("localhost", server_addr)
        .use_rustls_tls()
        .http1_only()
        .build()?;
    let resource_uri = format!("https://localhost:{}/pub", server_addr.port());
    let advertised = h1_client.get(&resource_uri).send().await?;
    let expected_alt_svc = format!("h3=\":{}\"; ma=86400", h3_addr.port());
    assert_eq!(
        advertised
            .headers()
            .get("alt-svc")
            .and_then(|value| value.to_str().ok()),
        Some(expected_alt_svc.as_str()),
        "the h1 GET response must advertise the live h3 listener port"
    );

    let client_endpoint = h3_client_endpoint()?;
    let connection = client_endpoint.connect(server_addr, "localhost")?.await?;
    let (mut driver, mut sender) = h3::client::new(h3_quinn::Connection::new(connection)).await?;
    let driver = tokio::spawn(async move {
        let _ = poll_fn(|context| driver.poll_close(context)).await;
    });

    let cases = [
        RequestCase {
            name: "LDP GET",
            method: Method::GET,
            path: "/pub",
            accept: Some("text/turtle"),
            expected_status: StatusCode::OK,
            body_witness: b"Alice",
        },
        RequestCase {
            name: "LDP HEAD",
            method: Method::HEAD,
            path: "/pub",
            accept: Some("text/turtle"),
            expected_status: StatusCode::OK,
            body_witness: b"",
        },
        RequestCase {
            name: "notification discovery",
            method: Method::GET,
            path: "/.well-known/solid",
            accept: Some("text/turtle"),
            expected_status: StatusCode::OK,
            body_witness: b"notificationChannel",
        },
        RequestCase {
            name: "liveness",
            method: Method::GET,
            path: "/livez",
            accept: None,
            expected_status: StatusCode::OK,
            body_witness: b"live\n",
        },
        RequestCase {
            name: "malformed notification receive",
            method: Method::GET,
            path: "/.notifications/WebSocketChannel2023/receive",
            accept: None,
            expected_status: StatusCode::BAD_REQUEST,
            body_witness: b"missing topic",
        },
    ];

    for case in &cases {
        let h1 = h1_request(&h1_client, server_addr, case).await?;
        let h3 = h3_request(
            &mut sender,
            case.method.clone(),
            &format!("https://localhost:{}{}", h3_addr.port(), case.path),
            case.accept,
            &[],
        )
        .await?;
        assert_eq!(h1.status, case.expected_status, "{}", case.name);
        if case.body_witness.is_empty() {
            assert!(h1.body.is_empty(), "{} must have an empty body", case.name);
        } else {
            assert!(
                h1.body
                    .windows(case.body_witness.len())
                    .any(|window| window == case.body_witness),
                "{} must exercise a non-vacuous response: {:?}",
                case.name,
                h1.body
            );
        }
        assert_eq!(
            h3, h1,
            "{} must be byte-equivalent over h3 and h1",
            case.name
        );
    }

    let topic = "https://pod.example/pub";
    let token = notifications
        .mint_receive_token("https://alice.example/#me", topic)
        .await;
    let mut websocket_url = url::Url::parse(&format!(
        "wss://localhost:{}/.notifications/WebSocketChannel2023/receive",
        server_addr.port()
    ))?;
    websocket_url
        .query_pairs_mut()
        .append_pair("topic", topic)
        .append_pair("token", &token);
    let h3_websocket_uri = websocket_url.as_str().replacen("wss://", "https://", 1);
    let refused = h3_request(
        &mut sender,
        Method::GET,
        &h3_websocket_uri,
        None,
        &[
            ("sec-websocket-version", "13"),
            ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="),
        ],
    )
    .await?;
    assert_eq!(
        refused.status,
        StatusCode::METHOD_NOT_ALLOWED,
        "WebSocket over h3 must be cleanly refused until extended CONNECT is implemented"
    );
    let after_refusal = h3_request(
        &mut sender,
        Method::GET,
        &format!("https://localhost:{}/livez", h3_addr.port()),
        None,
        &[],
    )
    .await?;
    assert_eq!(after_refusal.status, StatusCode::OK);

    let tcp = tokio::net::TcpStream::connect(server_addr).await?;
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_tls_config(b"http/1.1")?));
    let server_name = rustls::pki_types::ServerName::try_from("localhost")?.to_owned();
    let tls = connector.connect(server_name, tcp).await?;
    let (mut websocket, upgrade) =
        tokio_tungstenite::client_async(websocket_url.as_str(), tls).await?;
    assert_eq!(upgrade.status(), StatusCode::SWITCHING_PROTOCOLS);
    assert_eq!(upgrade.version(), Version::HTTP_11);
    tokio::time::timeout(Duration::from_secs(5), async {
        while notifications.subscriber_count(topic).await != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    websocket.close(None).await?;

    drop(sender);
    client_endpoint.close(quinn::VarInt::from_u32(0), b"test complete");
    tokio::time::timeout(Duration::from_secs(5), driver).await??;
    h3_shutdown_tx
        .send(())
        .map_err(|()| "h3 server stopped before shutdown")?;
    tokio::time::timeout(Duration::from_secs(5), h3_server).await???;

    drop(h1_client);
    tcp_handle.graceful_shutdown(Some(Duration::from_secs(1)));
    tokio::time::timeout(Duration::from_secs(5), tcp_server).await???;
    Ok(())
}

#[tokio::test]
async fn h3_requests_carry_peer_connect_info() -> TestResult {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let (app, _) = production_router(0.0001, 1.0).await;
    let mode = TlsMode::Tls {
        cert_path: CERT_PATH.into(),
        key_path: KEY_PATH.into(),
    };
    let tcp_tls = build_rustls_config(&mode, false)
        .await?
        .expect("TLS mode yields a config");
    let mut quic_tls = (*tcp_tls.get_inner()).clone();
    quic_tls.alpn_protocols = vec![b"h3".to_vec()];
    let h3_endpoint =
        quinn::Endpoint::server(quic_server_config(quic_tls)?, "127.0.0.1:0".parse()?)?;
    let h3_addr = h3_endpoint.local_addr()?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_h3(h3_endpoint, app, async move {
        let _ = shutdown_rx.await;
    }));

    let client_endpoint = h3_client_endpoint()?;
    let connection = client_endpoint.connect(h3_addr, "localhost")?.await?;
    let (mut driver, mut sender) = h3::client::new(h3_quinn::Connection::new(connection)).await?;
    let driver = tokio::spawn(async move {
        let _ = poll_fn(|context| driver.poll_close(context)).await;
    });
    let uri = format!("https://localhost:{}/pub", h3_addr.port());
    let first = h3_request(&mut sender, Method::GET, &uri, None, &[]).await?;
    assert_eq!(first.status, StatusCode::OK);
    let limited = h3_request(&mut sender, Method::GET, &uri, None, &[]).await?;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "the second same-IP request must be rate-limited; missing QUIC ConnectInfo fails open"
    );

    drop(sender);
    client_endpoint.close(quinn::VarInt::from_u32(0), b"test complete");
    tokio::time::timeout(Duration::from_secs(5), driver).await??;
    shutdown_tx
        .send(())
        .map_err(|()| "h3 server stopped before shutdown")?;
    tokio::time::timeout(Duration::from_secs(5), server).await???;
    Ok(())
}

#[tokio::test]
async fn unconfigured_http3_router_does_not_advertise_alt_svc() -> TestResult {
    let (app, _) = production_router(100.0, 100.0).await;
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("{BASE_URL}/pub"))
                .body(Body::empty())?,
        )
        .await?;

    assert!(
        response.headers().get("alt-svc").is_none(),
        "the original TCP router must not advertise HTTP/3 before a QUIC endpoint binds"
    );
    Ok(())
}
