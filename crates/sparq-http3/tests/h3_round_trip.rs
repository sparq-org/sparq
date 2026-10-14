#![cfg(feature = "server")]

use std::{error::Error, future::poll_fn, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    body::{Body, Bytes},
    extract::ConnectInfo,
    http::{Method, Request, StatusCode},
    routing::{get, post},
    Router,
};
use bytes::Buf as _;
use h3::quic::OpenStreams;
use quinn::crypto::rustls::QuicClientConfig;
use rcgen::{generate_simple_self_signed, CertifiedKey};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sparq_http3::{
    alt_svc_layer, quic_server_config, serve_h3, serve_h3_with_limits, H3ConnectionLimits,
    Http3ConfigError,
};
use tower::ServiceExt as _;

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

fn certificate() -> TestResult<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["localhost".to_owned()])?;
    Ok((
        cert.der().clone(),
        PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
    ))
}

fn rustls_server_config(
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
) -> TestResult<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    Ok(rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?)
}

fn client_endpoint(cert: CertificateDer<'static>) -> TestResult<quinn::Endpoint> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert)?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"h3".to_vec()];

    let client_config = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
    let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse()?)?;
    endpoint.set_default_client_config(client_config);
    Ok(endpoint)
}

async fn request<O>(
    sender: &mut h3::client::SendRequest<O, Bytes>,
    method: Method,
    uri: String,
    body: Option<Bytes>,
) -> TestResult<(StatusCode, Bytes)>
where
    O: OpenStreams<Bytes>,
{
    let request = Request::builder().method(method).uri(uri).body(())?;
    let mut stream = sender.send_request(request).await?;
    if let Some(body) = body {
        stream.send_data(body).await?;
    }
    stream.finish().await?;

    let response = stream.recv_response().await?;
    let mut body = Vec::new();
    while let Some(mut chunk) = stream.recv_data().await? {
        let remaining = chunk.remaining();
        body.extend_from_slice(&chunk.copy_to_bytes(remaining));
    }
    Ok((response.status(), Bytes::from(body)))
}

#[test]
fn config_rejects_a_server_without_h3_alpn() -> TestResult {
    let (cert, key) = certificate()?;
    let config = rustls_server_config(cert, key)?;

    let error = quic_server_config(config).expect_err("missing h3 ALPN must fail closed");
    assert!(matches!(error, Http3ConfigError::MissingH3Alpn));
    Ok(())
}

#[test]
fn config_replaces_quinns_unbounded_connection_receive_window() -> TestResult {
    let (cert, key) = certificate()?;
    let mut tls = rustls_server_config(cert, key)?;
    tls.alpn_protocols = vec![b"h3".to_vec()];

    let config = quic_server_config(tls)?;
    let transport = format!("{:?}", config.transport);
    assert!(
        transport.contains("receive_window:"),
        "Quinn's transport debug output must expose the checked receive window: {transport}"
    );
    assert!(
        !transport.contains(&format!(
            "receive_window: {}",
            quinn::VarInt::MAX.into_inner()
        )),
        "the connection receive window must not retain Quinn's VarInt::MAX default: {transport}"
    );
    Ok(())
}

#[tokio::test]
async fn alt_svc_layer_emits_the_exact_advertisement_and_replaces_stale_values() -> TestResult {
    let app = Router::new()
        .route("/", get(|| async { [("alt-svc", "h2=\":8443\"; ma=60")] }))
        .layer(alt_svc_layer(443));
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty())?)
        .await?;

    assert_eq!(
        response
            .headers()
            .get("alt-svc")
            .and_then(|value| value.to_str().ok()),
        Some("h3=\":443\"; ma=86400")
    );
    Ok(())
}

#[tokio::test]
async fn h3_dispatches_body_status_and_peer_connect_info() -> TestResult {
    async fn echo(body: Bytes) -> (StatusCode, Bytes) {
        (StatusCode::CREATED, body)
    }

    async fn peer(ConnectInfo(peer): ConnectInfo<SocketAddr>) -> String {
        peer.to_string()
    }

    let (cert, key) = certificate()?;
    let mut tls = rustls_server_config(cert.clone(), key)?;
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let endpoint = quinn::Endpoint::server(quic_server_config(tls)?, "127.0.0.1:0".parse()?)?;
    let server_addr = endpoint.local_addr()?;
    let router = Router::new()
        .route("/echo", post(echo))
        .route("/peer", get(peer));

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_h3(endpoint, router, async move {
        let _ = shutdown_rx.await;
    }));

    let client_endpoint = client_endpoint(cert)?;
    let client_addr = client_endpoint.local_addr()?;
    let connection = client_endpoint.connect(server_addr, "localhost")?.await?;
    let (mut driver, mut sender) = h3::client::new(h3_quinn::Connection::new(connection)).await?;
    let driver = tokio::spawn(async move {
        let _ = poll_fn(|context| driver.poll_close(context)).await;
    });

    let (status, body) = request(
        &mut sender,
        Method::POST,
        format!("https://localhost:{}/echo", server_addr.port()),
        Some(Bytes::from_static(b"request body crosses h3 and axum")),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body, "request body crosses h3 and axum");

    let (status, body) = request(
        &mut sender,
        Method::GET,
        format!("https://localhost:{}/peer", server_addr.port()),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, client_addr.to_string());

    drop(sender);
    client_endpoint.close(quinn::VarInt::from_u32(0), b"test complete");
    tokio::time::timeout(Duration::from_secs(5), driver).await??;
    shutdown_tx
        .send(())
        .map_err(|()| "server stopped before shutdown")?;
    tokio::time::timeout(Duration::from_secs(5), server).await???;
    Ok(())
}

async fn limited_server(
    limits: H3ConnectionLimits,
    router: Router,
) -> TestResult<(
    SocketAddr,
    CertificateDer<'static>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<std::io::Result<()>>,
)> {
    let (cert, key) = certificate()?;
    let mut tls = rustls_server_config(cert.clone(), key)?;
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let endpoint = quinn::Endpoint::server(quic_server_config(tls)?, "127.0.0.1:0".parse()?)?;
    let server_addr = endpoint.local_addr()?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_h3_with_limits(endpoint, router, limits, async move {
        let _ = shutdown_rx.await;
    }));
    Ok((server_addr, cert, shutdown_tx, server))
}

async fn stop_limited_server(
    endpoint: quinn::Endpoint,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
) -> TestResult {
    endpoint.close(quinn::VarInt::from_u32(0), b"test complete");
    shutdown_tx
        .send(())
        .map_err(|()| "server stopped before shutdown")?;
    tokio::time::timeout(Duration::from_secs(5), server).await???;
    Ok(())
}

#[tokio::test]
async fn global_connection_cap_queues_excess_connections_until_release() -> TestResult {
    let limits = H3ConnectionLimits {
        max_connections: 1,
        max_connections_per_ip: None,
        exempt_internal_ips: true,
        ..H3ConnectionLimits::default()
    };
    let (server_addr, cert, shutdown_tx, server) = limited_server(limits, Router::new()).await?;
    let endpoint = client_endpoint(cert)?;

    let first = endpoint.connect(server_addr, "localhost")?.await?;
    let second = endpoint.connect(server_addr, "localhost")?;
    tokio::pin!(second);
    assert!(
        tokio::time::timeout(Duration::from_millis(250), &mut second)
            .await
            .is_err(),
        "a second QUIC connection must remain queued while the sole global slot is held"
    );

    first.close(quinn::VarInt::from_u32(0), b"release global slot");
    let second = tokio::time::timeout(Duration::from_secs(5), second).await??;
    second.close(quinn::VarInt::from_u32(0), b"test complete");
    stop_limited_server(endpoint, shutdown_tx, server).await
}

#[tokio::test]
async fn per_ip_connection_cap_refuses_excess_connections() -> TestResult {
    let limits = H3ConnectionLimits {
        max_connections: 4,
        max_connections_per_ip: Some(1),
        exempt_internal_ips: false,
        ..H3ConnectionLimits::default()
    };
    let (server_addr, cert, shutdown_tx, server) = limited_server(limits, Router::new()).await?;
    let endpoint = client_endpoint(cert)?;

    let first = endpoint.connect(server_addr, "localhost")?.await?;
    let second = endpoint.connect(server_addr, "localhost")?;
    let refusal = tokio::time::timeout(Duration::from_secs(5), second).await?;
    assert!(
        refusal.is_err(),
        "a second connection from the same IP must be refused at the per-IP cap"
    );

    first.close(quinn::VarInt::from_u32(0), b"test complete");
    stop_limited_server(endpoint, shutdown_tx, server).await
}

// sq-4rkcc: the load-bearing DoS-hardening invariant — with the per-connection request
// limit at 1, a second request on the SAME connection must not reach the router while the first
// request task still holds the sole permit; it may start only after that task completes. A raised
// or removed limit, a permit dropped at the end of the accept-loop iteration, or a permit dropped
// inside the task before dispatch all let the second request enter concurrently and turn the
// negative-window assertion red.
#[tokio::test]
async fn per_connection_request_cap_backpressures_the_excess_request() -> TestResult {
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let handler_release = release.clone();
    let gated = move || {
        let entered_tx = entered_tx.clone();
        let release = handler_release.clone();
        async move {
            let _ = entered_tx.send(());
            release
                .acquire()
                .await
                .expect("the test release semaphore is never closed")
                .forget();
            StatusCode::OK
        }
    };
    let limits = H3ConnectionLimits {
        max_requests_per_connection: 1,
        ..H3ConnectionLimits::default()
    };
    let router = Router::new().route("/gated", get(gated));
    let (server_addr, cert, shutdown_tx, server) = limited_server(limits, router).await?;

    let client_endpoint = client_endpoint(cert)?;
    let connection = client_endpoint.connect(server_addr, "localhost")?.await?;
    let (mut driver, sender) = h3::client::new(h3_quinn::Connection::new(connection)).await?;
    let driver = tokio::spawn(async move {
        let _ = poll_fn(|context| driver.poll_close(context)).await;
    });

    let url = format!("https://localhost:{}/gated", server_addr.port());
    let mut first_sender = sender.clone();
    let first_url = url.clone();
    let first =
        tokio::spawn(async move { request(&mut first_sender, Method::GET, first_url, None).await });
    tokio::time::timeout(Duration::from_secs(5), entered_rx.recv())
        .await?
        .ok_or("the first request never entered the handler")?;

    let mut second_sender = sender.clone();
    let second = tokio::spawn(async move { request(&mut second_sender, Method::GET, url, None).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(250), entered_rx.recv())
            .await
            .is_err(),
        "the second request must not enter the handler while the sole request permit is held"
    );

    // Completing the first request task releases its permit and unblocks the accept loop.
    release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), entered_rx.recv())
        .await?
        .ok_or("the second request never entered the handler after the permit was released")?;
    release.add_permits(1);

    let (status, _) = tokio::time::timeout(Duration::from_secs(5), first).await???;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = tokio::time::timeout(Duration::from_secs(5), second).await???;
    assert_eq!(status, StatusCode::OK);

    drop(sender);
    client_endpoint.close(quinn::VarInt::from_u32(0), b"test complete");
    tokio::time::timeout(Duration::from_secs(5), driver).await??;
    stop_limited_server(client_endpoint, shutdown_tx, server).await
}

#[tokio::test]
async fn internal_ip_exemption_leaves_only_the_global_cap() -> TestResult {
    let limits = H3ConnectionLimits {
        max_connections: 2,
        max_connections_per_ip: Some(1),
        exempt_internal_ips: true,
        ..H3ConnectionLimits::default()
    };
    let (server_addr, cert, shutdown_tx, server) = limited_server(limits, Router::new()).await?;
    let endpoint = client_endpoint(cert)?;

    let first = endpoint.connect(server_addr, "localhost")?.await?;
    let second = endpoint.connect(server_addr, "localhost")?.await?;

    first.close(quinn::VarInt::from_u32(0), b"test complete");
    second.close(quinn::VarInt::from_u32(0), b"test complete");
    stop_limited_server(endpoint, shutdown_tx, server).await
}
