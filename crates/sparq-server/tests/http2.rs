#![cfg(feature = "http2")]
//! sq-oprna.6: real TLS/ALPN HTTP/2 negotiation and HTTP/1.1 fallback.

use std::{
    error::Error,
    io::Read as _,
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

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

fn tls_client(addr: SocketAddr, http1_only: bool) -> TestResult<reqwest::Client> {
    let ca = reqwest::Certificate::from_pem(&std::fs::read(fixture("http3-ca.pem"))?)?;
    let mut builder = reqwest::Client::builder()
        .use_rustls_tls()
        .tls_built_in_root_certs(false)
        .add_root_certificate(ca)
        .resolve("localhost", addr);
    if http1_only {
        builder = builder.http1_only();
    }
    Ok(builder.build()?)
}

#[test]
fn tcp_tls_rejects_an_incomplete_credential_pair() -> TestResult {
    let output = Command::new(env!("CARGO_BIN_EXE_sparq-server"))
        .arg("--tls-cert")
        .arg(fixture("http3-cert.pem"))
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--tls-cert and --tls-key must be provided together"),
        "unexpected stderr: {stderr}"
    );
    Ok(())
}

#[tokio::test]
async fn tls_alpn_negotiates_h2_and_preserves_h1() -> TestResult {
    let addr = free_tcp_addr()?;
    let child = Command::new(env!("CARGO_BIN_EXE_sparq-server"))
        .arg("--addr")
        .arg(addr.to_string())
        .arg("--tls-cert")
        .arg(fixture("http3-cert.pem"))
        .arg("--tls-key")
        .arg(fixture("http3-key.pem"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut server = ChildGuard(child);
    let h2 = tls_client(addr, false)?;
    let url = format!("https://localhost:{}/health", addr.port());

    let mut h2_response = None;
    for _ in 0..100 {
        match h2.get(&url).send().await {
            Ok(response) => {
                h2_response = Some(response);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(25)).await,
        }
    }
    let Some(h2_response) = h2_response else {
        let _ = server.0.kill();
        let mut stderr = String::new();
        if let Some(mut pipe) = server.0.stderr.take() {
            pipe.read_to_string(&mut stderr)?;
        }
        let _ = server.0.wait();
        return Err(format!("TLS server did not become ready: {stderr}").into());
    };
    assert_eq!(h2_response.version(), reqwest::Version::HTTP_2);
    assert_eq!(h2_response.status(), reqwest::StatusCode::OK);
    assert_eq!(h2_response.text().await?, "ok");

    let h1 = tls_client(addr, true)?;
    let h1_response = h1.get(url).send().await?;
    assert_eq!(h1_response.version(), reqwest::Version::HTTP_11);
    assert_eq!(h1_response.status(), reqwest::StatusCode::OK);
    assert_eq!(h1_response.text().await?, "ok");
    Ok(())
}
