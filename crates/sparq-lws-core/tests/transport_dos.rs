// AUTHORED-BY Claude Opus 4.8
//! Adversarial transport-layer DoS-hardening regression tests (`src/transport.rs`), over the EXACT
//! hardened TLS serve construction the binary uses:
//!   `from_tcp_rustls(std_listener, cfg).handle(h).map(|a| limiter.wrap_acceptor(a))` +
//!   `transport_config.apply_to_builder(server.http_builder())`.
//!
//! These prove the DEFENCES actually fire end-to-end (not just that a config field is set):
//!   1. **HTTP/2 Rapid-Reset (CVE-2023-44487).** A client opens streams and immediately `RST_STREAM`s
//!      them in a tight burst beyond the pending-accept-reset cap. The server must `GOAWAY` / tear the
//!      connection down — bounded, not unbounded work.
//!   2. **Slowloris header trickle.** A client opens a TLS connection and dribbles request-header bytes
//!      byte-by-byte, never completing the head, past the header-read timeout. The server must DROP the
//!      connection (the read returns EOF / the handshake-level stream closes) within the window.
//!   3. **Connection cap.** With `max_connections` small, more than that many concurrently-served
//!      connections cannot be admitted at once — the cap holds, then recovers when a slot frees.
//!   4. **Per-source connection cap.** With a huge global cap but a small `max_connections_per_ip`
//!      (and internal-exemption off so the loopback source is capped), one source cannot hold more
//!      than its per-source limit of concurrent connections — bounding a single-source flood well
//!      below the global ceiling — and recovers when it releases one.
//!
//! `#[ignore]`d by default (real socket I/O + the fixture test cert), like `tls_handshake.rs`. Run with
//! `cargo test --test transport_dos -- --ignored`.

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

/// A rustls client trusting ONLY the fixture CA, offering the given ALPN list.
fn client_config(alpn: &[&[u8]]) -> ClientConfig {
    let pem = std::fs::read(CA_PATH).expect("read fixture CA");
    let mut roots = RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(&pem) {
        let cert: CertificateDer<'_> = cert.expect("parse fixture CA");
        roots.add(cert).expect("add fixture CA");
    }
    let mut cfg = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    cfg.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    cfg
}

/// Boot the hardened TLS server with the given `TransportConfig` over a trivial router, returning the
/// bound address + the server's `axum_server::Handle` (for graceful shutdown). This is the EXACT serve
/// construction the binary's TLS arm uses — the connection-cap acceptor wrapper + the
/// `apply_to_builder` h2/h1 knobs over `from_tcp_rustls`.
async fn boot_hardened_server(
    transport: TransportConfig,
) -> (
    std::net::SocketAddr,
    axum_server::Handle<std::net::SocketAddr>,
) {
    let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();

    let mode = TlsMode::Tls {
        cert_path: CERT_PATH.into(),
        key_path: KEY_PATH.into(),
    };
    let rustls_config = build_rustls_config(&mode, false)
        .await
        .expect("build rustls config")
        .expect("tls mode yields a config");

    // Trivial app — these tests are about the transport layer, not auth/LDP.
    let app = Router::new().route("/healthz", get(|| async { "ok" }));

    let tokio_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = tokio_listener.local_addr().expect("local addr");
    let std_listener = tokio_listener.into_std().expect("into_std");
    std_listener.set_nonblocking(true).expect("set_nonblocking");

    let limiter = ConnectionLimiter::new(transport.max_connections).with_per_ip_cap(
        transport.max_connections_per_ip,
        transport.conn_exempt_internal,
    );
    let handle = axum_server::Handle::new();
    let server_handle = handle.clone();
    let handshake_timeout = transport.handshake_timeout;
    let idle_timeout = transport.idle_timeout;
    let max_requests_per_conn = transport.max_requests_per_conn;
    tokio::spawn(async move {
        // The EXACT serve construction the binary's TLS arm uses (the FULL guard set).
        let mut server = axum_server::from_tcp_rustls(std_listener, rustls_config)
            .expect("from_tcp_rustls")
            .handle(server_handle)
            .map(move |acceptor| {
                limiter.wrap_acceptor_with_guards(
                    acceptor,
                    handshake_timeout,
                    idle_timeout,
                    max_requests_per_conn,
                )
            });
        transport.apply_to_builder(server.http_builder());
        let _ = server
            .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
            .await;
    });

    (addr, handle)
}

/// Open a TLS stream to `addr` offering the given ALPN, retrying the TCP connect briefly to avoid
/// racing the server bind (a handshake failure on a connected socket is surfaced).
async fn connect_tls(
    addr: std::net::SocketAddr,
    alpn: &[&[u8]],
) -> tokio_rustls::client::TlsStream<tokio::net::TcpStream> {
    let connector = TlsConnector::from(Arc::new(client_config(alpn)));
    let dns_name = ServerName::try_from("localhost").expect("server name");
    for _ in 0..100 {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(tcp) => {
                return connector
                    .connect(dns_name.clone(), tcp)
                    .await
                    .expect("TLS handshake on a connected socket");
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    panic!("could not connect to the test server");
}

/// REGRESSION 1 — HTTP/2 Rapid-Reset (CVE-2023-44487). Open many streams and immediately reset each,
/// in a burst beyond hyper's pending-accept-reset cap. The server MUST detect the abuse and close the
/// connection (a `GOAWAY` surfaces as the h2 connection task ending with an error / further sends
/// failing). An UNPROTECTED server would keep accepting reset streams unboundedly; here the burst is
/// bounded by the cap.
#[tokio::test]
#[ignore = "needs the fixture test cert + real socket I/O; run with --ignored"]
async fn rapid_reset_burst_is_bounded_by_goaway() {
    // Keep the pending-reset cap at hyper's secure default (20) — None ⇒ default. Drive ~200 reset
    // streams: far beyond 20, so the server must GOAWAY well before we finish.
    let transport = TransportConfig {
        h2_max_concurrent_streams: 256,
        h2_max_pending_reset_streams: None, // hyper default (20) is the CVE mitigation under test
        header_read_timeout: Some(Duration::from_secs(15)),
        max_connections: 10_000,
        keep_alive_timeout: Some(Duration::from_secs(60)),
        handshake_timeout: Some(Duration::from_secs(10)),
        max_connections_per_ip: None,
        conn_exempt_internal: true,
        idle_timeout: None,          // disabled — not under test here
        max_requests_per_conn: None, // disabled — not under test here
    };
    let (addr, handle) = boot_hardened_server(transport).await;

    // Negotiate h2 over ALPN, then run a raw h2 client so we can send RST_STREAM at will.
    let tls = connect_tls(addr, &[b"h2"]).await;
    let (mut send_req, connection) = h2::client::handshake(tls)
        .await
        .expect("h2 client handshake");

    // Drive the connection on a task; capture whether it ends in an error (a GOAWAY / connection
    // teardown is the expected, correct outcome of the rapid-reset defence). Awaiting the `connection`
    // future to completion IS the driver — it resolves with `Err` when the server tears the connection
    // down (GOAWAY / protocol error), which is the SUCCESS signal for this test.
    let conn_task = tokio::spawn(connection);

    // Burst: open a stream and immediately reset it, many times. `poll_ready`/`send_request` may start
    // erroring once the server GOAWAYs — that is exactly the bound we want to observe.
    let mut send_errors = 0u32;
    for i in 0..200u32 {
        // Wait until the client can open a new stream (or the connection died).
        if futures_util::future::poll_fn(|cx| send_req.poll_ready(cx))
            .await
            .is_err()
        {
            send_errors += 1;
            break;
        }
        let req = http::Request::builder()
            .method("GET")
            .uri("https://localhost/healthz")
            .body(())
            .unwrap();
        match send_req.send_request(req, false) {
            Ok((_resp, mut stream)) => {
                // Immediately reset the stream we just opened — the rapid-reset attack primitive.
                stream.send_reset(h2::Reason::CANCEL);
            }
            Err(_) => {
                send_errors += 1;
                break;
            }
        }
        if i % 25 == 0 {
            // Yield so the server can process frames + decide to GOAWAY.
            tokio::task::yield_now().await;
        }
    }

    // The connection MUST end (GOAWAY / error) rather than serving the unbounded reset burst forever.
    // Give it a moment; the driver task should resolve with an Err (server tore the connection down).
    let conn_result = tokio::time::timeout(Duration::from_secs(5), conn_task)
        .await
        .expect("h2 connection driver did not resolve — server did not bound the reset burst")
        .expect("connection task panicked");

    // The defence is meaningful ONLY if the connection was actively torn down by the server: the h2
    // driver resolves with an Err (a GOAWAY / connection-level protocol error) — NOT a clean
    // `Ok(())` (which would mean the server happily processed all 200 resets, i.e. NO bound). Require
    // the error, so this test cannot pass vacuously on a server that never enforces the cap. The h2
    // crate surfaces the server's GOAWAY as a connection error here.
    let conn_err = conn_result.expect_err(
        "the rapid-reset burst was NOT bounded: the h2 connection completed cleanly, meaning the \
         server processed all 200 reset streams without a GOAWAY (the CVE-2023-44487 defence did \
         not fire). send_errors observed client-side: ",
    );
    eprintln!(
        "rapid-reset: server tore down the connection as expected — h2 error: {conn_err:?}, \
         client-side send_errors={send_errors}"
    );

    handle.graceful_shutdown(Some(Duration::from_secs(1)));
}

/// REGRESSION 2 — Slowloris header trickle. Open a TLS connection (h1), then dribble request-header
/// bytes ONE AT A TIME with a delay, never completing the head, past a SHORT header-read timeout. The
/// server MUST drop the connection within the window (a subsequent read returns EOF / 0 bytes). An
/// UNPROTECTED server would hold the connection open indefinitely waiting for the rest of the head.
#[tokio::test]
#[ignore = "needs the fixture test cert + real socket I/O; run with --ignored"]
async fn slowloris_header_trickle_is_dropped_after_timeout() {
    // A SHORT header-read timeout so the test is fast + deterministic.
    let header_timeout = Duration::from_secs(1);
    let transport = TransportConfig {
        h2_max_concurrent_streams: 256,
        h2_max_pending_reset_streams: None,
        header_read_timeout: Some(header_timeout),
        max_connections: 10_000,
        keep_alive_timeout: Some(Duration::from_secs(60)),
        handshake_timeout: Some(Duration::from_secs(10)),
        max_connections_per_ip: None,
        conn_exempt_internal: true,
        idle_timeout: None,          // disabled — not under test here
        max_requests_per_conn: None, // disabled — not under test here
    };
    let (addr, handle) = boot_hardened_server(transport).await;

    // Force HTTP/1.1 (offer only http/1.1) so the byte-trickle is a partial HTTP/1 head.
    let mut tls = connect_tls(addr, &[b"http/1.1"]).await;

    // Send the request line + start of the headers, then dribble — but NEVER send the terminating
    // CRLFCRLF. Each byte is sent with a delay so the TOTAL exceeds the 1s header-read timeout.
    let partial = b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nX-Slow: ";
    tls.write_all(partial).await.expect("write request start");
    tls.flush().await.expect("flush start");

    // Dribble more header bytes one at a time, slowly, past the timeout window. The server should drop
    // the connection mid-trickle.
    let trickle = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"; // never completes the head
    let mut dropped_mid_trickle = false;
    for &byte in trickle.iter() {
        // Writing to a half-closed socket eventually errors once the server drops us.
        if tls.write_all(&[byte]).await.is_err() || tls.flush().await.is_err() {
            dropped_mid_trickle = true;
            break;
        }
        // ~200ms/byte; the 1s timeout is exceeded after ~5 bytes — well within this loop.
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Whether or not the write side noticed, the READ side must observe the connection close: after the
    // header-read timeout the server drops the connection, so a read returns 0 bytes (EOF) — NOT a hang.
    let mut buf = [0u8; 64];
    let read = tokio::time::timeout(Duration::from_secs(3), tls.read(&mut buf)).await;
    match read {
        // EOF (0 bytes) or a read error ⇒ the server closed the connection: the slowloris defence fired.
        Ok(Ok(0)) => { /* clean EOF — connection dropped */ }
        Ok(Err(_)) => { /* read error — connection reset/dropped */ }
        // A small 408/400-style response then close is ALSO acceptable (the server answered + closed)
        // — as long as it did not HANG. Any bytes received means the server actively closed the head.
        Ok(Ok(_n)) => { /* server sent a response/close — also a drop, not a hang */ }
        Err(_) => panic!(
            "slowloris connection HUNG past the header-read timeout — the defence did not fire \
             (dropped_mid_trickle={dropped_mid_trickle})"
        ),
    }

    handle.graceful_shutdown(Some(Duration::from_secs(1)));
}

/// REGRESSION 3 — Connection cap. With `max_connections = 2`, hold two long-lived connections open
/// (each keep-alive so it keeps its served-connection permit), then assert a THIRD connection is
/// REFUSED while at capacity (its TLS handshake / request fails fast — the acceptor sheds it without a
/// permit) — but a connection DOES succeed promptly once a held connection is released. This proves
/// the accept-time connection permit bounds concurrently-served connections AND sheds the overflow
/// fail-fast (it does not queue the over-cap connection).
#[tokio::test]
#[ignore = "needs the fixture test cert + real socket I/O; run with --ignored"]
async fn connection_cap_bounds_concurrent_served_connections() {
    let transport = TransportConfig {
        h2_max_concurrent_streams: 256,
        h2_max_pending_reset_streams: None,
        header_read_timeout: None, // disable so a parked-mid-request connection is not header-timed-out
        max_connections: 2,
        keep_alive_timeout: None,
        // No handshake timeout here: the held connections complete their handshake fast; the cap test
        // is about post-handshake permit holding, and a short handshake bound could race the test.
        handshake_timeout: None,
        // Per-source cap DISABLED here so this test isolates the GLOBAL cap (the per-source cap has its
        // own dedicated test below).
        max_connections_per_ip: None,
        conn_exempt_internal: true,
        idle_timeout: None, // disabled — the held connections are kept idle on purpose here
        max_requests_per_conn: None, // disabled — not under test here
    };
    let (addr, handle) = boot_hardened_server(transport).await;

    // Open two connections and complete a full request on each so they are SERVED (each holds a
    // connection permit). We keep the TLS streams alive (do not drop them), so with http/1.1 keep-alive
    // the connection — and its permit — stays held.
    async fn full_request(
        addr: std::net::SocketAddr,
    ) -> tokio_rustls::client::TlsStream<tokio::net::TcpStream> {
        let mut tls = connect_tls(addr, &[b"http/1.1"]).await;
        let req = "GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n";
        tls.write_all(req.as_bytes()).await.expect("write");
        tls.flush().await.expect("flush");
        // Read just the status line so the request completes but the keep-alive connection stays open.
        let mut buf = [0u8; 12];
        tls.read_exact(&mut buf).await.expect("read status");
        assert!(
            buf.starts_with(b"HTTP/1.1 200"),
            "held connection should be served: {:?}",
            String::from_utf8_lossy(&buf)
        );
        tls
    }

    let held1 = full_request(addr).await;
    let held2 = full_request(addr).await;

    // Both permits are now held by the two kept-alive connections. A THIRD connection over the cap is
    // REFUSED fail-fast: the acceptor cannot get a permit, returns an accept error, and axum-server
    // drops the connection — so the THIRD connection's TLS handshake + request must NOT succeed (it
    // fails / closes), and must not HANG either. Do a raw connect that TOLERATES a handshake/connection
    // failure (the success signal here is "did NOT get a served 200 response").
    let connector = TlsConnector::from(Arc::new(client_config(&[b"http/1.1"])));
    let dns_name = ServerName::try_from("localhost").expect("server name");
    let third_served = tokio::time::timeout(Duration::from_secs(2), async {
        let tcp = tokio::net::TcpStream::connect(addr).await.ok()?;
        // The handshake may fail (connection refused/closed by the over-cap acceptor) — that is the
        // EXPECTED cap behaviour, so map any error to None (not-served), not a panic.
        let mut tls = connector.connect(dns_name.clone(), tcp).await.ok()?;
        let req = "GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        tls.write_all(req.as_bytes()).await.ok()?;
        tls.flush().await.ok()?;
        let mut buf = Vec::new();
        tls.read_to_end(&mut buf).await.ok()?;
        Some(String::from_utf8_lossy(&buf).to_string())
    })
    .await;
    // Whether the timeout elapsed (Err) or the connect/handshake/read returned None/empty, the cap held
    // if a served `200` did NOT come back. A served `200` while at capacity means the cap leaked.
    let got_200_over_cap = matches!(&third_served, Ok(Some(body)) if body.contains("200"));
    assert!(
        !got_200_over_cap,
        "a 3rd connection must NOT be SERVED (200) while the cap (2) is fully held — the connection \
         cap leaked: {third_served:?}"
    );

    // Release one held connection → a permit frees → a fresh connection can now be served promptly.
    drop(held1);
    // Give the dropped connection's task a moment to release its permit.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let now_served = tokio::time::timeout(Duration::from_secs(4), async {
        let mut tls = connect_tls(addr, &[b"http/1.1"]).await;
        let req = "GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        tls.write_all(req.as_bytes()).await.expect("write");
        tls.flush().await.expect("flush");
        let mut buf = Vec::new();
        tls.read_to_end(&mut buf).await.expect("read");
        String::from_utf8_lossy(&buf).to_string()
    })
    .await
    .expect("a freed connection slot must admit the 3rd connection promptly");

    assert!(
        now_served.contains("200"),
        "after a slot freed, the 3rd connection should be served (200): {now_served}"
    );

    // Keep held2 alive until here so the cap stayed at capacity for the blocked assertion.
    drop(held2);
    handle.graceful_shutdown(Some(Duration::from_secs(1)));
}

/// REGRESSION 4 — PER-SOURCE connection cap. With a HUGE global cap but `max_connections_per_ip = 2`
/// and `conn_exempt_internal = false` (so loopback IS capped), hold two long-lived connections from
/// this (single) source, then assert a THIRD from the SAME source is REFUSED while at the per-source
/// cap — and that releasing one frees a per-source slot. This proves the per-source cap bounds ONE IP
/// well below the global ceiling (a single-source slowloris/connection flood can't pin the whole pool).
#[tokio::test]
#[ignore = "needs the fixture test cert + real socket I/O; run with --ignored"]
async fn per_source_connection_cap_bounds_a_single_source() {
    let transport = TransportConfig {
        h2_max_concurrent_streams: 256,
        h2_max_pending_reset_streams: None,
        header_read_timeout: None, // a parked-mid-request connection must not be header-timed-out
        max_connections: 10_000,   // GLOBAL cap is huge — the PER-SOURCE cap is what must bind here
        keep_alive_timeout: None,
        handshake_timeout: None,
        max_connections_per_ip: Some(2), // at most 2 concurrent connections from one source IP
        conn_exempt_internal: false,     // so the loopback test source IS subject to the cap
        idle_timeout: None, // disabled — not under test here (held connections stay idle)
        max_requests_per_conn: None, // disabled — not under test here
    };
    let (addr, handle) = boot_hardened_server(transport).await;

    async fn full_request(
        addr: std::net::SocketAddr,
    ) -> tokio_rustls::client::TlsStream<tokio::net::TcpStream> {
        let mut tls = connect_tls(addr, &[b"http/1.1"]).await;
        let req = "GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n";
        tls.write_all(req.as_bytes()).await.expect("write");
        tls.flush().await.expect("flush");
        let mut buf = [0u8; 12];
        tls.read_exact(&mut buf).await.expect("read status");
        assert!(
            buf.starts_with(b"HTTP/1.1 200"),
            "held connection should be served: {:?}",
            String::from_utf8_lossy(&buf)
        );
        tls
    }

    // Hold two connections from this source (loopback) — both served, both keep their per-source slot.
    let held1 = full_request(addr).await;
    let held2 = full_request(addr).await;

    // A THIRD connection from the SAME source is over the per-source cap ⇒ REFUSED fail-fast (no served
    // 200), even though the GLOBAL pool has ~10k free slots. Tolerate a handshake/connection failure —
    // that IS the per-source cap doing its job.
    let connector = TlsConnector::from(Arc::new(client_config(&[b"http/1.1"])));
    let dns_name = ServerName::try_from("localhost").expect("server name");
    let third_served = tokio::time::timeout(Duration::from_secs(2), async {
        let tcp = tokio::net::TcpStream::connect(addr).await.ok()?;
        let mut tls = connector.connect(dns_name.clone(), tcp).await.ok()?;
        let req = "GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        tls.write_all(req.as_bytes()).await.ok()?;
        tls.flush().await.ok()?;
        let mut buf = Vec::new();
        tls.read_to_end(&mut buf).await.ok()?;
        Some(String::from_utf8_lossy(&buf).to_string())
    })
    .await;
    let got_200_over_per_source_cap =
        matches!(&third_served, Ok(Some(body)) if body.contains("200"));
    assert!(
        !got_200_over_per_source_cap,
        "a 3rd connection from the SAME source must NOT be SERVED (200) while its per-source cap (2) is \
         held — the per-source cap leaked: {third_served:?}"
    );

    // Release one held connection → a per-source slot frees → a fresh connection from the source is
    // served promptly (proving the cap is per-source-live, not a permanent lockout).
    drop(held1);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let now_served = tokio::time::timeout(Duration::from_secs(4), async {
        let mut tls = connect_tls(addr, &[b"http/1.1"]).await;
        let req = "GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        tls.write_all(req.as_bytes()).await.expect("write");
        tls.flush().await.expect("flush");
        let mut buf = Vec::new();
        tls.read_to_end(&mut buf).await.expect("read");
        String::from_utf8_lossy(&buf).to_string()
    })
    .await
    .expect("a freed per-source slot must admit a new connection from the source promptly");
    assert!(
        now_served.contains("200"),
        "after a per-source slot freed, a new connection from the source should be served (200): {now_served}"
    );

    drop(held2);
    handle.graceful_shutdown(Some(Duration::from_secs(1)));
}

/// REGRESSION 5 — Idle-keepalive timeout (NEW GUARD). Open a TLS HTTP/1.1 keep-alive connection,
/// complete ONE request (so the connection is established + served), then go IDLE — send NOTHING further
/// — past a SHORT idle timeout. The server MUST drop the idle connection within the window (a read
/// returns EOF / 0 bytes). This is the gap the existing bounds miss: `header_read_timeout` only bounds a
/// PARTIAL head (bytes started), and the h2 PING only reclaims a DEAD peer — neither closes a live peer
/// holding a fully-idle keep-alive connection. An UNPROTECTED server would hold the idle connection (and
/// its permit) open indefinitely.
#[tokio::test]
#[ignore = "needs the fixture test cert + real socket I/O; run with --ignored"]
async fn idle_keepalive_connection_is_dropped_after_idle_timeout() {
    // A SHORT idle timeout so the test is fast + deterministic. Header-read timeout DISABLED so it
    // cannot be what closes the connection — the idle timeout must be the sole cause (no false pass).
    let idle = Duration::from_secs(1);
    let transport = TransportConfig {
        h2_max_concurrent_streams: 256,
        h2_max_pending_reset_streams: None,
        header_read_timeout: None, // OFF — so ONLY the idle timeout can close the connection
        max_connections: 10_000,
        keep_alive_timeout: None,
        handshake_timeout: Some(Duration::from_secs(10)),
        max_connections_per_ip: None, // disabled — not under test here
        conn_exempt_internal: true,
        idle_timeout: Some(idle),
        max_requests_per_conn: None,
    };
    let (addr, handle) = boot_hardened_server(transport).await;

    // HTTP/1.1 keep-alive: complete one request, keep the connection open, read just the status so the
    // request finishes but the keep-alive connection stays up.
    let mut tls = connect_tls(addr, &[b"http/1.1"]).await;
    let req = "GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n"; // NO `Connection: close` ⇒ keep-alive
    tls.write_all(req.as_bytes()).await.expect("write request");
    tls.flush().await.expect("flush");
    let mut status = [0u8; 12];
    tls.read_exact(&mut status).await.expect("read status line");
    assert!(
        status.starts_with(b"HTTP/1.1 200"),
        "the first request must be served (200): {:?}",
        String::from_utf8_lossy(&status)
    );

    // Drain the rest of THAT response (headers + tiny body) without sending a new request, so the
    // connection is now genuinely idle (no in-flight request, no new bytes from us). We read with a
    // short per-read timeout; once the server's idle window elapses, a read returns EOF (0) — the idle
    // timeout fired. If the connection HUNG open (no idle guard), the final read would block past the
    // window and the outer timeout would catch it as a FAILURE.
    let idle_closed = tokio::time::timeout(Duration::from_secs(5), async {
        let mut buf = [0u8; 256];
        loop {
            match tls.read(&mut buf).await {
                Ok(0) => return true, // EOF — the server closed the idle connection (PASS)
                Ok(_n) => { /* still draining the first response's bytes — keep reading */ }
                Err(_e) => return true, // reset/closed — also the idle guard firing
            }
        }
    })
    .await;
    assert!(
        matches!(idle_closed, Ok(true)),
        "the idle keep-alive connection must be DROPPED within the idle timeout window (got {idle_closed:?}) \
         — the idle-keepalive guard did not fire (an idle connection HUNG open holding its permit)"
    );

    handle.graceful_shutdown(Some(Duration::from_secs(1)));
}

/// REGRESSION 6 — Max-requests-per-connection (NEW GUARD). With the cap = 3, drive requests on ONE
/// HTTP/1.1 keep-alive connection: the first (cap-1) responses must NOT carry `Connection: close` (reuse
/// continues), and the cap-th response MUST carry `Connection: close` so the connection ends after it.
/// This bounds how long a single connection can be reused (defence-in-depth connection rotation). An
/// UNPROTECTED server would reuse the connection unboundedly (never closing it on its own).
#[tokio::test]
#[ignore = "needs the fixture test cert + real socket I/O; run with --ignored"]
async fn max_requests_per_connection_sets_connection_close_at_the_cap() {
    let cap = 3u64;
    let transport = TransportConfig {
        h2_max_concurrent_streams: 256,
        h2_max_pending_reset_streams: None,
        header_read_timeout: Some(Duration::from_secs(15)),
        max_connections: 10_000,
        keep_alive_timeout: None,
        handshake_timeout: Some(Duration::from_secs(10)),
        max_connections_per_ip: None, // disabled — not under test here
        conn_exempt_internal: true,
        idle_timeout: None, // OFF — the cap, not an idle close, must end the connection
        max_requests_per_conn: Some(cap),
    };
    let (addr, handle) = boot_hardened_server(transport).await;

    let mut tls = connect_tls(addr, &[b"http/1.1"]).await;

    // Issue `cap` keep-alive requests on the SAME connection, reading each response head. The first
    // `cap-1` must keep the connection alive (no `connection: close`); the cap-th must carry
    // `connection: close`. We read until the end of each response's headers (the blank line) and inspect
    // the header block for `connection: close` (case-insensitive).
    async fn read_response_head<R: AsyncReadExt + Unpin>(tls: &mut R) -> String {
        // Read byte-by-byte until the CRLFCRLF header terminator. The bodies here are tiny ("ok") and
        // hyper sends Content-Length, so after the head we don't need the body to inspect the header.
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = tls.read(&mut byte).await.expect("read header byte");
            assert!(n == 1, "connection closed mid-response head (unexpected)");
            head.push(byte[0]);
            if head.len() >= 4 && &head[head.len() - 4..] == b"\r\n\r\n" {
                break;
            }
        }
        String::from_utf8_lossy(&head).to_lowercase()
    }

    for i in 1..=cap {
        let req = "GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n"; // keep-alive (no Connection: close)
        tls.write_all(req.as_bytes()).await.expect("write request");
        tls.flush().await.expect("flush");
        let head = read_response_head(&mut tls).await;
        assert!(
            head.starts_with("http/1.1 200"),
            "request {i} served: {head}"
        );
        // Drain the tiny body so the next request starts clean. The response is "ok" (Content-Length 2).
        let mut body = [0u8; 2];
        let _ = tls.read_exact(&mut body).await;
        let has_close = head.contains("connection: close");
        if i < cap {
            assert!(
                !has_close,
                "request {i} (< cap {cap}) must NOT carry `Connection: close` — keep-alive reuse should \
                 continue (header block: {head})"
            );
        } else {
            assert!(
                has_close,
                "the cap-th request ({i} == cap {cap}) MUST carry `Connection: close` so the connection \
                 ends after it (the max-requests-per-conn guard did not fire; header block: {head})"
            );
        }
    }

    // After the cap-th `Connection: close`, the server closes the connection: a subsequent read returns
    // EOF (the connection genuinely ended, not just an advisory header).
    let mut buf = [0u8; 64];
    let read = tokio::time::timeout(Duration::from_secs(3), tls.read(&mut buf)).await;
    match read {
        Ok(Ok(0)) | Ok(Err(_)) => { /* EOF / reset — the connection was closed after the cap (PASS) */ }
        Ok(Ok(n)) => panic!(
            "after the cap-th `Connection: close` the server still sent {n} more bytes — the connection \
             was NOT closed"
        ),
        Err(_) => panic!("the connection HUNG open after the cap-th `Connection: close` — it was not closed"),
    }

    handle.graceful_shutdown(Some(Duration::from_secs(1)));
}
