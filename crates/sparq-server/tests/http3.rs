#![cfg(feature = "http3")]
//! [GPT-5.6] sq-oprna.3/.5: production HTTP/3 wiring and cross-protocol conformance.

use std::{
    error::Error,
    future::poll_fn,
    io::Read as _,
    net::{SocketAddr, TcpListener, UdpSocket},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};

use bytes::{Buf as _, Bytes};
use futures_util::{SinkExt as _, StreamExt as _};
use h3::quic::OpenStreams;
use quinn::crypto::rustls::QuicClientConfig;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::CertificateDer;

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

const RESULTS_JSON: &str = "application/sparql-results+json";

// [GPT-5.6] sq-oprna.5: compare the transport-neutral response contract, not incidental headers.
#[derive(Debug, PartialEq, Eq)]
struct ResponseSnapshot {
    status: u16,
    content_type: Option<String>,
    body: Bytes,
}

struct RequestCase {
    name: &'static str,
    method: axum::http::Method,
    path: &'static str,
    content_type: Option<&'static str>,
    accept: Option<&'static str>,
    body: &'static [u8],
    expected_status: axum::http::StatusCode,
    body_witness: &'static [u8],
}

struct H3Request<'a> {
    method: axum::http::Method,
    path: &'a str,
    content_type: Option<&'a str>,
    accept: Option<&'a str>,
    headers: &'a [(&'a str, &'a str)],
    body: &'a [u8],
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn free_tcp_addr() -> TestResult<SocketAddr> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?)
}

fn free_udp_addr() -> TestResult<SocketAddr> {
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    Ok(socket.local_addr()?)
}

fn h3_client(ca_path: &std::path::Path) -> TestResult<quinn::Endpoint> {
    let ca_file = std::fs::File::open(ca_path)?;
    let mut roots = rustls::RootCertStore::empty();
    for cert in CertificateDer::pem_reader_iter(ca_file) {
        roots.add(cert?)?;
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let config = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
    let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse()?)?;
    endpoint.set_default_client_config(config);
    Ok(endpoint)
}

async fn h3_request<O>(
    sender: &mut h3::client::SendRequest<O, Bytes>,
    addr: SocketAddr,
    request: H3Request<'_>,
) -> TestResult<ResponseSnapshot>
where
    O: OpenStreams<Bytes>,
{
    let mut builder = axum::http::Request::builder()
        .method(request.method)
        .uri(format!("https://localhost:{}{}", addr.port(), request.path));
    if let Some(value) = request.content_type {
        builder = builder.header(axum::http::header::CONTENT_TYPE, value);
    }
    if let Some(value) = request.accept {
        builder = builder.header(axum::http::header::ACCEPT, value);
    }
    for &(name, value) in request.headers {
        builder = builder.header(name, value);
    }
    let mut stream = sender.send_request(builder.body(())?).await?;
    if !request.body.is_empty() {
        stream
            .send_data(Bytes::copy_from_slice(request.body))
            .await?;
    }
    stream.finish().await?;

    let response = stream.recv_response().await?;
    let status = response.status().as_u16();
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
    base_url: &str,
    case: &RequestCase,
) -> TestResult<ResponseSnapshot> {
    let mut request = client.request(
        reqwest::Method::from_bytes(case.method.as_str().as_bytes())?,
        format!("{base_url}{}", case.path),
    );
    if let Some(value) = case.content_type {
        request = request.header(reqwest::header::CONTENT_TYPE, value);
    }
    if let Some(value) = case.accept {
        request = request.header(reqwest::header::ACCEPT, value);
    }
    if !case.body.is_empty() {
        request = request.body(case.body.to_vec());
    }
    let response = request.send().await?;
    assert_eq!(response.version(), reqwest::Version::HTTP_11);
    let status = response.status().as_u16();
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

// [GPT-5.6] sq-oprna.6: the combined http2+http3 build reuses the PEM pair for TLS TCP;
// http3 alone and runtime-off listeners stay cleartext. Force h1 so this remains the h1↔h3
// parity oracle in both configurations.
fn tcp_client(addr: SocketAddr, tls: bool) -> TestResult<(reqwest::Client, String)> {
    let mut builder = reqwest::Client::builder().http1_only();
    let base_url = if tls {
        let ca = reqwest::Certificate::from_pem(&std::fs::read(fixture("http3-ca.pem"))?)?;
        builder = builder
            .use_rustls_tls()
            .tls_built_in_root_certs(false)
            .add_root_certificate(ca)
            .resolve("localhost", addr);
        format!("https://localhost:{}", addr.port())
    } else {
        format!("http://{addr}")
    };
    Ok((builder.build()?, base_url))
}

#[test]
fn http3_requires_both_certificate_and_key_flags() -> TestResult {
    let output = Command::new(env!("CARGO_BIN_EXE_sparq-server"))
        .arg("--http3")
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--http3 requires both --tls-cert FILE and --tls-key FILE"),
        "unexpected stderr: {stderr}"
    );
    Ok(())
}

#[tokio::test]
async fn h3_response_matrix_matches_h1_and_websocket_falls_back() -> TestResult {
    let tcp_addr = free_tcp_addr()?;
    let udp_addr = free_udp_addr()?;
    let child = Command::new(env!("CARGO_BIN_EXE_sparq-server"))
        .arg("--addr")
        .arg(tcp_addr.to_string())
        .arg("--http3")
        .arg("--http3-addr")
        .arg(udp_addr.to_string())
        .arg("--tls-cert")
        .arg(fixture("http3-cert.pem"))
        .arg("--tls-key")
        .arg(fixture("http3-key.pem"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut server = ChildGuard(child);

    let (http, tcp_base) = tcp_client(tcp_addr, cfg!(feature = "http2"))?;
    let readiness_url = format!("{tcp_base}/health");
    let expected_alt_svc = format!("h3=\":{}\"; ma=86400", udp_addr.port());
    // [SONNET-4.6] The Alt-Svc header is installed only after the QUIC endpoint
    // is bound, so it is the readiness condition needed by this test.
    let mut health = None;
    let mut last_probe_failure = None;
    for _ in 0..100 {
        match http.get(&readiness_url).send().await {
            Ok(response) => {
                let status = response.status();
                let alt_svc = response
                    .headers()
                    .get("alt-svc")
                    .and_then(|value| value.to_str().ok());
                if status.is_success() && alt_svc == Some(expected_alt_svc.as_str()) {
                    health = Some(response);
                    break;
                } else {
                    last_probe_failure =
                        Some(format!("status {}, alt-svc {:?}", status, alt_svc));
                }
            }
            Err(error) => last_probe_failure = Some(error.to_string()),
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let Some(health) = health else {
        let _ = server.0.kill();
        let mut stderr = String::new();
        if let Some(mut pipe) = server.0.stderr.take() {
            pipe.read_to_string(&mut stderr)?;
        }
        let _ = server.0.wait();
        return Err(format!(
            "server did not become ready (last probe failure: {}): {}",
            last_probe_failure.as_deref().unwrap_or("no probe completed"),
            stderr
        )
        .into());
    };
    assert_eq!(health.version(), reqwest::Version::HTTP_11);
    assert_eq!(
        health
            .headers()
            .get("alt-svc")
            .and_then(|value| value.to_str().ok()),
        Some(expected_alt_svc.as_str()),
        "the h1 GET response must advertise the successfully bound QUIC port"
    );
    let endpoint = h3_client(&fixture("http3-ca.pem"))?;
    let connection = endpoint.connect(udp_addr, "localhost")?.await?;
    let (mut driver, mut sender) = h3::client::new(h3_quinn::Connection::new(connection)).await?;
    let driver = tokio::spawn(async move {
        let _ = poll_fn(|context| driver.poll_close(context)).await;
    });
    let cases = [
        RequestCase {
            name: "health",
            method: axum::http::Method::GET,
            path: "/health",
            content_type: None,
            accept: None,
            body: b"",
            expected_status: axum::http::StatusCode::OK,
            body_witness: b"ok",
        },
        RequestCase {
            name: "GET SELECT as CSV",
            method: axum::http::Method::GET,
            path: "/sparql?query=SELECT%20%3Fx%20WHERE%20%7B%20VALUES%20%3Fx%20%7B%20%3Chttp%3A%2F%2Fexample.com%2Fhttp3-get%3E%20%7D%20%7D",
            content_type: None,
            accept: Some("text/csv"),
            body: b"",
            expected_status: axum::http::StatusCode::OK,
            body_witness: b"http://example.com/http3-get",
        },
        RequestCase {
            name: "POST ASK as SPARQL JSON",
            method: axum::http::Method::POST,
            path: "/sparql",
            content_type: Some("application/sparql-query"),
            accept: Some(RESULTS_JSON),
            body: b"ASK { }",
            expected_status: axum::http::StatusCode::OK,
            body_witness: b"\"boolean\":true",
        },
        RequestCase {
            name: "not-found error",
            method: axum::http::Method::GET,
            path: "/not-an-endpoint",
            content_type: None,
            accept: None,
            body: b"",
            expected_status: axum::http::StatusCode::NOT_FOUND,
            body_witness: b"not found",
        },
    ];

    for case in &cases {
        let h1 = h1_request(&http, &tcp_base, case).await?;
        let h3 = h3_request(
            &mut sender,
            udp_addr,
            H3Request {
                method: case.method.clone(),
                path: case.path,
                content_type: case.content_type,
                accept: case.accept,
                headers: &[],
                body: case.body,
            },
        )
        .await?;
        assert_eq!(h1.status, case.expected_status.as_u16(), "{}", case.name);
        assert!(
            h1.body
                .windows(case.body_witness.len())
                .any(|window| window == case.body_witness),
            "{} must exercise a non-vacuous response: {:?}",
            case.name,
            h1.body
        );
        assert_eq!(
            h3, h1,
            "{} must be byte-equivalent over h3 and h1",
            case.name
        );
    }

    // HTTP/1.1 Upgrade headers are illegal in h3. Axum consequently requires RFC 9220 CONNECT for
    // this HTTP version, but the server deliberately exposes no extended-CONNECT route: the h3 GET
    // is refused with 405 without closing the connection, then the client falls back to TCP.
    let refused = h3_request(
        &mut sender,
        udp_addr,
        H3Request {
            method: axum::http::Method::GET,
            path: "/subscriptions",
            content_type: None,
            accept: None,
            headers: &[
                ("sec-websocket-version", "13"),
                ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="),
            ],
            body: b"",
        },
    )
    .await?;
    assert_eq!(
        refused.status,
        axum::http::StatusCode::METHOD_NOT_ALLOWED.as_u16(),
        "WebSocket over h3 must be cleanly refused until extended CONNECT is implemented"
    );
    let after_refusal = h3_request(
        &mut sender,
        udp_addr,
        H3Request {
            method: axum::http::Method::GET,
            path: "/health",
            content_type: None,
            accept: None,
            headers: &[],
            body: b"",
        },
    )
    .await?;
    assert_eq!(after_refusal.status, axum::http::StatusCode::OK.as_u16());

    #[cfg(not(feature = "http2"))]
    let (mut websocket, upgrade) =
        tokio_tungstenite::connect_async(format!("ws://{tcp_addr}/subscriptions")).await?;
    #[cfg(feature = "http2")]
    let (mut websocket, upgrade) = {
        let ca_file = std::fs::File::open(fixture("http3-ca.pem"))?;
        let mut roots = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_reader_iter(ca_file) {
            roots.add(cert?)?;
        }
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_root_certificates(roots)
            .with_no_client_auth();
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        let connector = tokio_rustls::TlsConnector::from(Arc::new(tls));
        let stream = tokio::net::TcpStream::connect(tcp_addr).await?;
        let stream = connector
            .connect(
                rustls::pki_types::ServerName::try_from("localhost")?,
                stream,
            )
            .await?;
        tokio_tungstenite::client_async(
            format!("wss://localhost:{}/subscriptions", tcp_addr.port()),
            stream,
        )
        .await?
    };
    assert_eq!(
        upgrade.status(),
        axum::http::StatusCode::SWITCHING_PROTOCOLS
    );
    assert_eq!(upgrade.version(), axum::http::Version::HTTP_11);
    websocket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({ "subscribe": { "query": "SELECT ?x WHERE { VALUES ?x { <http://example.com/ws-fallback> } }" } })
                .to_string()
                .into(),
        ))
        .await?;
    let subscribed = tokio::time::timeout(Duration::from_secs(5), websocket.next())
        .await?
        .ok_or_else(|| std::io::Error::other("WebSocket closed before subscribed response"))??;
    let subscribed: serde_json::Value = serde_json::from_str(subscribed.to_text()?)?;
    assert!(subscribed.get("subscribed").is_some(), "{subscribed}");
    let initial = tokio::time::timeout(Duration::from_secs(5), websocket.next())
        .await?
        .ok_or_else(|| std::io::Error::other("WebSocket closed before initial notification"))??;
    assert!(
        initial
            .to_text()?
            .contains("http://example.com/ws-fallback"),
        "the h1 fallback must carry a real subscription result"
    );
    websocket.close(None).await?;

    drop(sender);
    endpoint.close(quinn::VarInt::from_u32(0), b"test complete");
    tokio::time::timeout(Duration::from_secs(5), driver).await??;
    Ok(())
}

#[tokio::test]
async fn http3_runtime_off_does_not_advertise_alt_svc() -> TestResult {
    let tcp_addr = free_tcp_addr()?;
    let child = Command::new(env!("CARGO_BIN_EXE_sparq-server"))
        .arg("--addr")
        .arg(tcp_addr.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut server = ChildGuard(child);

    let (http, tcp_base) = tcp_client(tcp_addr, false)?;
    let url = format!("{tcp_base}/health");
    for _ in 0..100 {
        match http.get(&url).send().await {
            Ok(response) => {
                assert!(
                    response.headers().get("alt-svc").is_none(),
                    "a feature-on binary without --http3 must not advertise an unbound listener"
                );
                return Ok(());
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(25)).await,
        }
    }

    let _ = server.0.kill();
    let mut stderr = String::new();
    if let Some(mut pipe) = server.0.stderr.take() {
        pipe.read_to_string(&mut stderr)?;
    }
    let _ = server.0.wait();
    Err(format!("server did not become ready: {stderr}").into())
}
