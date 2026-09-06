// AUTHORED-BY GPT-5.6
//! End-to-end witnesses for the HTTP/1 request-head transport bounds.

use std::sync::Arc;
use std::time::Duration;

use axum::routing::get;
use axum::Router;
use sparq_lws_core::tls::{build_rustls_config, TlsMode};
use sparq_lws_core::transport::{ConnectionLimiter, TransportConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

const CERT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-cert.pem");
const KEY_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-key.pem");
const CA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-ca.pem");
// Deliberately independent of the production constants: relaxing/removing a production bound makes
// these fixed attack fixtures reach the handler and turns the tests red (the mutation witness).
const FLOOD_HEADER_COUNT: usize = 101;
const FLOOD_HEADER_VALUE_BYTES: usize = 64 * 1024;

type ClientStream = tokio_rustls::client::TlsStream<tokio::net::TcpStream>;

fn client_config() -> ClientConfig {
    let pem = std::fs::read(CA_PATH).expect("read fixture CA");
    let mut roots = RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(&pem) {
        let cert: CertificateDer<'_> = cert.expect("parse fixture CA");
        roots.add(cert).expect("add fixture CA");
    }
    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}

async fn boot_server(
    header_read_timeout: Duration,
) -> (
    std::net::SocketAddr,
    axum_server::Handle<std::net::SocketAddr>,
) {
    let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();
    let rustls = build_rustls_config(
        &TlsMode::Tls {
            cert_path: CERT_PATH.into(),
            key_path: KEY_PATH.into(),
        },
        false,
    )
    .await
    .expect("build fixture TLS config")
    .expect("TLS mode produces config");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind server");
    let addr = listener.local_addr().expect("server address");
    let listener = listener.into_std().expect("convert listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");

    let mut transport = TransportConfig::from_env();
    transport.header_read_timeout = Some(header_read_timeout);
    let limiter = ConnectionLimiter::new(transport.max_connections);
    let handle = axum_server::Handle::new();
    let serve_handle = handle.clone();
    tokio::spawn(async move {
        let mut server = axum_server::from_tcp_rustls(listener, rustls)
            .expect("create TLS server")
            .handle(serve_handle)
            .map(move |acceptor| limiter.wrap_acceptor(acceptor));
        transport.apply_to_builder(server.http_builder());
        let app = Router::new().route("/healthz", get(|| async { "ok" }));
        let _ = server.serve(app.into_make_service()).await;
    });
    (addr, handle)
}

async fn connect(addr: std::net::SocketAddr) -> ClientStream {
    let connector = TlsConnector::from(Arc::new(client_config()));
    let name = ServerName::try_from("localhost").expect("valid DNS name");
    let tcp = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to bound listener");
    connector.connect(name, tcp).await.expect("TLS handshake")
}

async fn response(stream: &mut ClientStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut bytes))
        .await
        .expect("server did not close HTTP/1 response");
    // A fail-closed parser/timeout path may drop the TLS socket without a close_notify. The transport
    // closure is the expected witness; retain any HTTP error response bytes hyper emitted first.
    if let Err(error) = result {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::UnexpectedEof,
            "unexpected response read error"
        );
    }
    bytes
}

#[tokio::test]
async fn rejects_header_count_above_explicit_bound() {
    let (addr, handle) = boot_server(Duration::from_secs(2)).await;
    let mut stream = connect(addr).await;
    let mut request =
        String::from("GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    for index in 0..FLOOD_HEADER_COUNT {
        request.push_str(&format!("X-Flood-{index}: x\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("send request");
    let bytes = response(&mut stream).await;
    assert!(
        bytes.starts_with(b"HTTP/1.1 431"),
        "over-count request must be rejected with 431, got: {}",
        String::from_utf8_lossy(&bytes)
    );
    handle.graceful_shutdown(Some(Duration::from_secs(1)));
}

#[tokio::test]
async fn rejects_aggregate_header_bytes_above_explicit_bound() {
    let (addr, handle) = boot_server(Duration::from_secs(2)).await;
    let mut stream = connect(addr).await;
    let value = "x".repeat(FLOOD_HEADER_VALUE_BYTES);
    let request = format!(
        "GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nX-Flood: {value}\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("send request");
    let bytes = response(&mut stream).await;
    assert!(
        bytes.starts_with(b"HTTP/1.1 431") || bytes.is_empty(),
        "over-byte request must be rejected before dispatch, got: {}",
        String::from_utf8_lossy(&bytes)
    );
    handle.graceful_shutdown(Some(Duration::from_secs(1)));
}

#[tokio::test]
async fn closes_stalled_partial_header_at_configured_timeout() {
    let (addr, handle) = boot_server(Duration::from_millis(100)).await;
    let mut stream = connect(addr).await;
    stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: local")
        .await
        .expect("send partial request head");
    let bytes = response(&mut stream).await;
    assert!(
        bytes.is_empty(),
        "stalled partial head must be closed without dispatch"
    );
    handle.graceful_shutdown(Some(Duration::from_secs(1)));
}

#[tokio::test]
async fn accepts_normal_request_within_all_bounds() {
    let (addr, handle) = boot_server(Duration::from_secs(2)).await;
    let mut stream = connect(addr).await;
    stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("send normal request");
    let bytes = response(&mut stream).await;
    assert!(
        bytes.starts_with(b"HTTP/1.1 200") && bytes.ends_with(b"ok"),
        "normal request must reach the service, got: {}",
        String::from_utf8_lossy(&bytes)
    );
    handle.graceful_shutdown(Some(Duration::from_secs(1)));
}
