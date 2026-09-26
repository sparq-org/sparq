//! [OPUS-4.8] sq-toze.36 (cert gap GX-13) — in-binary container HEALTHCHECK probe.
//!
//! CIS Docker Benchmark §4.6 wants a `HEALTHCHECK` baked into the image so the container
//! runtime can tell an *unhealthy-but-still-running* process apart from a healthy one. The
//! server already serves `GET /health` (returns the literal body `ok`), but the runtime
//! stage of the image is **distroless** (`gcr.io/distroless/cc-debian12`): there is NO
//! shell and NO `curl`/`wget`, so a classic `HEALTHCHECK CMD curl …` cannot run there.
//!
//! The portable answer is to make the server binary probe *itself*: the Dockerfile's
//! `HEALTHCHECK` invokes `sparq-server --health-probe`, which opens a plain TCP connection
//! to the loopback `/health` endpoint, sends a minimal HTTP/1.0 `GET`, and exits `0` iff the
//! response status line is `200`. No external tool, no extra dependency (it reuses tokio's
//! `net` + `io-util`, already in the `server` feature stack), and it works inside distroless
//! because the probe *is* the shipped binary.
//!
//! The HTTP-response classification (`probe_healthy`) is pure and unit-tested; the async
//! `run_probe` is the thin TCP wrapper around it.

use std::time::Duration;

/// Default address the probe targets — the loopback the server binds *inside* the container.
///
/// The container `ENTRYPOINT` binds `0.0.0.0:3030`, but a probe running in the same network
/// namespace reaches it on loopback, so the probe defaults to `127.0.0.1:3030`. An operator
/// who remaps the port can override it with `--health-probe-addr HOST:PORT` (or the
/// `SPARQ_HEALTH_PROBE_ADDR` env var).
pub const DEFAULT_PROBE_ADDR: &str = "127.0.0.1:3030";

/// The total time the probe waits (connect + write + read) before declaring the server
/// unhealthy. Kept well under the Dockerfile `HEALTHCHECK --timeout` so the probe returns a
/// clean non-zero exit rather than being killed mid-flight.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// The raw request the probe sends. HTTP/1.0 with `Connection: close` so the server closes
/// the socket after responding and the probe's read loop terminates without parsing
/// `Content-Length`. `Host` is required by some routers even on 1.0; loopback is fine here.
///
/// Only built when the async probe (`server` feature) is, or under `cfg(test)` for the
/// request-shape unit test; the pure `probe_healthy` classifier needs no request builder.
#[cfg(any(feature = "server", test))]
fn probe_request(host: &str) -> String {
    format!("GET /health HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n")
}

/// Classify an HTTP response (the raw bytes the server wrote) as healthy or not.
///
/// Healthy iff the status line is a `200`. We deliberately key on the status code, not the
/// `ok` body: the body is an implementation detail of the `/health` route, while a `200`
/// status line is the stable health contract (and a body check would force us to read past
/// the headers). A non-200 (e.g. a `503` a future readiness gate might emit) is unhealthy.
pub fn probe_healthy(response: &[u8]) -> bool {
    // The status line is the first CRLF-delimited line: `HTTP/1.1 200 OK`.
    let line_end = response
        .windows(2)
        .position(|w| w == b"\r\n")
        .unwrap_or(response.len());
    let status_line = &response[..line_end];
    let Ok(status_line) = std::str::from_utf8(status_line) else {
        return false;
    };
    // Split off the protocol token; the next whitespace-delimited token is the status code.
    let mut parts = status_line.split_whitespace();
    let proto = parts.next().unwrap_or("");
    if !proto.starts_with("HTTP/") {
        return false;
    }
    matches!(parts.next(), Some("200"))
}

/// Run the container health probe against `addr`: connect, send `GET /health`, read the
/// response, and return `Ok(())` iff it is a `200`. Any connect/IO/timeout failure, or a
/// non-200 response, returns `Err`. The whole exchange is bounded by [`PROBE_TIMEOUT`].
///
/// `main` maps `Ok` -> exit 0 (healthy) and `Err` -> a non-zero exit (unhealthy), which is
/// exactly the contract a Docker `HEALTHCHECK` consumes.
#[cfg(feature = "server")]
pub async fn run_probe(addr: &str) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let host = addr.to_string();
    let fut = async move {
        let mut stream = TcpStream::connect(&host)
            .await
            .map_err(|e| format!("connect {host}: {e}"))?;
        stream
            .write_all(probe_request(&host).as_bytes())
            .await
            .map_err(|e| format!("write {host}: {e}"))?;
        // Cap the read: the status line + headers are tiny, and `Connection: close` means the
        // server closes after the small `ok` body. 4 KiB is generous and bounds a hostile peer.
        let mut buf = Vec::with_capacity(256);
        let mut chunk = [0u8; 1024];
        loop {
            let n = stream
                .read(&mut chunk)
                .await
                .map_err(|e| format!("read {host}: {e}"))?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.len() >= 4096 {
                break;
            }
        }
        if probe_healthy(&buf) {
            Ok(())
        } else {
            let first = buf
                .iter()
                .position(|&b| b == b'\r' || b == b'\n')
                .unwrap_or(buf.len());
            Err(format!(
                "unhealthy /health response: {:?}",
                String::from_utf8_lossy(&buf[..first])
            ))
        }
    };
    tokio::time::timeout(PROBE_TIMEOUT, fut)
        .await
        .map_err(|_| format!("/health probe timed out after {PROBE_TIMEOUT:?}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_200_is_healthy() {
        assert!(probe_healthy(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"
        ));
        assert!(probe_healthy(b"HTTP/1.0 200 OK\r\n\r\nok"));
    }

    #[test]
    fn non_200_is_unhealthy() {
        assert!(!probe_healthy(b"HTTP/1.1 503 Service Unavailable\r\n\r\n"));
        assert!(!probe_healthy(
            b"HTTP/1.1 500 Internal Server Error\r\n\r\n"
        ));
        assert!(!probe_healthy(b"HTTP/1.1 404 Not Found\r\n\r\n"));
    }

    #[test]
    fn garbage_is_unhealthy() {
        assert!(!probe_healthy(b""));
        assert!(!probe_healthy(b"not an http response"));
        // A bare body with no status line must NOT pass.
        assert!(!probe_healthy(b"ok"));
        // A `200` that is not in the status-code position must NOT pass.
        assert!(!probe_healthy(b"HTTP/1.1 418 200 teapot\r\n\r\n"));
    }

    #[test]
    fn non_utf8_status_line_is_unhealthy() {
        // [OPUS-4.8] sq-4vao: a status line whose bytes are not valid UTF-8 (a hostile or
        // corrupt peer) must fail closed at the `from_utf8` guard rather than panic — the
        // `0xFF` byte is an invalid UTF-8 start byte, so the whole line fails to decode.
        assert!(!probe_healthy(b"\xFF\xFE 200 OK\r\n\r\n"));
        // Invalid bytes after the protocol token are equally rejected.
        assert!(!probe_healthy(b"HTTP/1.1 \xFF\xFF\r\n\r\n"));
    }

    #[test]
    fn status_line_without_trailing_crlf_still_parses() {
        // Defensive: a response truncated to just the status line is still classified.
        assert!(probe_healthy(b"HTTP/1.1 200 OK"));
        assert!(!probe_healthy(b"HTTP/1.1 503"));
    }

    #[test]
    fn request_is_well_formed_http10() {
        let req = probe_request("127.0.0.1:3030");
        assert!(req.starts_with("GET /health HTTP/1.0\r\n"));
        assert!(req.contains("Host: 127.0.0.1:3030\r\n"));
        assert!(req.contains("Connection: close\r\n"));
        assert!(req.ends_with("\r\n\r\n"));
    }

    // [OPUS-4.8] sq-qcnn.37: tokio tests for the async `run_probe` TCP path — the unhealthy
    // response branch (lines 110–117) and the 4 KiB buffer-cap break (line 104). Both require a
    // real TcpListener mock in the same process. The pure `probe_healthy` classifier is already
    // covered by the sync tests above; these pin the I/O path that surrounds it.

    #[cfg(feature = "server")]
    async fn read_mock_request(conn: &mut tokio::net::TcpStream) {
        use tokio::io::AsyncReadExt;
        // [GPT-6] Closing with unread request bytes can reset TCP on macOS. Read
        // only the bounded GET headers, not EOF: the probe waits for our reply.
        tokio::time::timeout(PROBE_TIMEOUT, async {
            let mut request = [0u8; 1024];
            let mut used = 0;
            loop {
                assert!(used < request.len(), "mock request exceeds header limit");
                let read = conn.read(&mut request[used..]).await.unwrap();
                assert_ne!(read, 0, "probe closed before sending complete headers");
                used += read;
                if request[..used].windows(4).any(|w| w == b"\r\n\r\n") {
                    assert!(request[..used].starts_with(b"GET /health HTTP/1.0\r\n"));
                    break;
                }
            }
        })
        .await
        .expect("mock request timed out");
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn run_probe_unhealthy_response_returns_descriptive_err() {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        // Spawn a mock server that returns one non-200 response then shuts down cleanly.
        let server = tokio::spawn(async move {
            if let Ok((mut conn, _)) = listener.accept().await {
                read_mock_request(&mut conn).await;
                let _ = conn
                    .write_all(b"HTTP/1.1 503 Service Unavailable\r\n\r\n")
                    .await;
                // Explicit shutdown so the probe's read loop sees EOF (not a connection reset).
                let _ = conn.shutdown().await;
            }
        });
        let result = run_probe(&addr).await;
        tokio::time::timeout(PROBE_TIMEOUT, server)
            .await
            .expect("mock response task timed out")
            .unwrap();
        assert!(result.is_err(), "non-200 response must be an Err");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("unhealthy"),
            "error message must mention 'unhealthy': {msg}",
        );
        // The status line text should be in the error for diagnostics.
        assert!(
            msg.contains("503") || msg.contains("Service Unavailable"),
            "error message should carry the status line: {msg}",
        );
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn run_probe_caps_read_buffer_at_4096_bytes() {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        // Send a healthy status line followed by >4 KiB of padding so the buffer-cap
        // `break` at line 104 triggers before the stream reaches EOF.
        let server = tokio::spawn(async move {
            if let Ok((mut conn, _)) = listener.accept().await {
                read_mock_request(&mut conn).await;
                let mut response = b"HTTP/1.1 200 OK\r\n\r\nok".to_vec();
                response.extend(std::iter::repeat_n(b'x', 5000));
                let _ = conn.write_all(&response).await;
                // Explicit shutdown so the probe can read the buffer-cap break and then EOF.
                let _ = conn.shutdown().await;
            }
        });
        let result = run_probe(&addr).await;
        tokio::time::timeout(PROBE_TIMEOUT, server)
            .await
            .expect("mock response task timed out")
            .unwrap();
        // The 200 status line is in the first chunk, so the probe should succeed.
        assert!(result.is_ok(), "healthy large response must succeed: {:?}", result);
    }
}
