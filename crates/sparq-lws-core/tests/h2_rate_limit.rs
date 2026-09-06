// AUTHORED-BY Claude Opus 4.8
//! SECURITY regression: the PRE-CRYPTO per-IP rate limiter (`src/rate_limit.rs`) MUST still throttle
//! many MULTIPLEXED requests sent over ONE HTTP/2 connection from one IP.
//!
//! ## Why this test exists (the h2-multiplexing concern)
//! The per-IP rate limiter keys its token bucket on the DIRECT PEER IP, read from
//! `ConnectInfo<SocketAddr>` in each request's extensions. Over HTTP/1.1 that is obviously per-request
//! (one request per connection at a time). Over HTTP/2 a single connection multiplexes MANY concurrent
//! request streams — all sharing the SAME peer IP. The concern an attacker would probe: does opening N
//! streams over one h2 connection DODGE the per-IP limit (e.g. if the limiter saw the connection once,
//! not each stream)? It must NOT — every multiplexed stream is a separate `http::Request` that flows
//! through the same middleware stack and carries the same `ConnectInfo`, so the bucket drains per stream.
//!
//! This boots the FULL production app (auth + LDP + the rate-limit/admission layers) over the EXACT TLS
//! serve construction the binary uses (`from_tcp_rustls(..).map(connection-cap acceptor).http_builder()`
//! with `into_make_service_with_connect_info::<SocketAddr>()`, so `ConnectInfo` is populated just as in
//! production), negotiates h2 over ALPN, fires MANY streams over ONE connection from one source IP, and
//! asserts the limiter trips (429s appear). A bypass (the limiter not seeing each multiplexed stream)
//! would show ZERO 429s — so the test fails vacuously-closed only if the guard genuinely holds.
//!
//! `#[ignore]`d by default (fixture cert + real socket I/O), like `transport_dos.rs` / `tls_handshake.rs`.
//! Run with `cargo test --test h2_rate_limit -- --ignored`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{jwks_provider, KeyKit, BASE_URL};
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router_with_overload, AppState, OverloadConfig};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::overload::AdmissionControl;
use sparq_lws_core::rate_limit::RateLimiter;
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient};
use sparq_lws_core::tls::{build_rustls_config, TlsMode};
use sparq_lws_core::transport::{ConnectionLimiter, TransportConfig};
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

const CERT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-cert.pem");
const KEY_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-key.pem");
const CA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-ca.pem");

/// A rustls client trusting ONLY the fixture CA, offering `h2` first via ALPN (so the handshake
/// negotiates HTTP/2 against the server's `[h2, http/1.1]`).
fn h2_client_config() -> ClientConfig {
    let pem = std::fs::read(CA_PATH).expect("read fixture CA");
    let mut roots = RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(&pem) {
        let cert: CertificateDer<'_> = cert.expect("parse fixture CA");
        roots.add(cert).expect("add fixture CA");
    }
    let mut cfg = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    cfg
}

/// Build the FULL production app (auth + LDP + the overload/rate-limit layers) with the rate limiter
/// sized to `(rate, burst)`, and loopback/internal exemptions OFF so the loopback peer the test connects
/// from IS throttled. Mirrors `tests/rate_limit_http.rs::app_with_rate_limit` but is served over a real
/// TLS+h2 socket here (not a `oneshot`), so the WHOLE serve path — incl. `ConnectInfo` injection — is
/// exercised end-to-end over multiplexed h2 streams.
fn app_with_rate_limit(rate: f64, burst: f64) -> axum::Router {
    let issuer_key = KeyKit::generate();
    let config = VerifierConfig::new(vec![common::ISSUER.to_string()], BASE_URL);
    let replay = InMemoryReplayStore::with_window(config.replay_ttl());
    let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay).unwrap();
    let ctx = AuthContext::new(verifier, BASE_URL);
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    let ldp = LdpState::new(store, BASE_URL);

    let overload = OverloadConfig {
        admission: AdmissionControl::new(10_000), // generous — only the rate limiter is under test
        request_timeout: None,
        rate_limiter: Some(RateLimiter::new(
            rate, burst, /* trusted_proxy_hops */ 0, /* exempt_loopback */ false,
            /* exempt_internal */ false,
        )),
        body_limit_bytes: sparq_lws_core::body_limit::DEFAULT_MAX_BODY_BYTES,
    };
    build_router_with_overload(AppState::new(ctx, ldp), overload)
}

/// Boot the FULL app over the binary's TLS serve construction, returning the bound addr + handle.
async fn boot_app_over_tls(
    app: axum::Router,
) -> (
    std::net::SocketAddr,
    axum_server::Handle<std::net::SocketAddr>,
) {
    let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();

    let mode = TlsMode::Tls {
        cert_path: CERT_PATH.into(),
        key_path: KEY_PATH.into(),
    };
    let rustls_config = build_rustls_config(&mode, /* mtls_bound_tokens */ false)
        .await
        .expect("build rustls config")
        .expect("tls mode yields a config");

    let tokio_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = tokio_listener.local_addr().expect("local addr");
    let std_listener = tokio_listener.into_std().expect("into_std");
    std_listener.set_nonblocking(true).expect("set_nonblocking");

    // The EXACT serve construction the binary uses: connection-cap acceptor + the transport knobs, and —
    // CRUCIALLY for this test — `into_make_service_with_connect_info::<SocketAddr>()` so each request
    // (each multiplexed h2 stream) carries `ConnectInfo<SocketAddr>` the limiter reads. With the default
    // TransportConfig the new idle/max-requests guards are present but lenient (they never trip here).
    let transport = TransportConfig::from_env();
    let limiter = ConnectionLimiter::new(transport.max_connections);
    let handle = axum_server::Handle::new();
    let server_handle = handle.clone();
    let handshake_timeout = transport.handshake_timeout;
    let idle_timeout = transport.idle_timeout;
    let max_requests_per_conn = transport.max_requests_per_conn;
    tokio::spawn(async move {
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

/// Open ONE h2 connection (negotiating h2 over ALPN), retrying the TCP connect briefly to avoid racing
/// the server bind. Returns the `SendRequest` handle + the driven connection JoinHandle.
async fn connect_h2(
    addr: std::net::SocketAddr,
) -> (
    h2::client::SendRequest<bytes::Bytes>,
    tokio::task::JoinHandle<()>,
) {
    let connector = TlsConnector::from(Arc::new(h2_client_config()));
    let dns_name = ServerName::try_from("localhost").expect("server name");
    for _ in 0..100 {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(tcp) => {
                let tls = connector
                    .connect(dns_name.clone(), tcp)
                    .await
                    .expect("TLS handshake on a connected socket");
                // Confirm h2 was actually negotiated (else the test would silently run over h1 and not
                // prove the MULTIPLEXING property it claims to).
                {
                    let (_io, conn) = tls.get_ref();
                    assert_eq!(
                        conn.alpn_protocol(),
                        Some(&b"h2"[..]),
                        "the server must negotiate h2 for this test to exercise multiplexing"
                    );
                }
                let (send_req, connection) = h2::client::handshake(tls)
                    .await
                    .expect("h2 client handshake");
                let driver = tokio::spawn(async move {
                    let _ = connection.await;
                });
                return (send_req, driver);
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    panic!("could not connect to the test server");
}

/// SECURITY: many MULTIPLEXED h2 requests over ONE connection (one source IP) must STILL trip the
/// per-IP rate limiter — the limiter is NOT bypassed by h2 multiplexing. We size the bucket TIGHT
/// (burst 2, negligible refill) and fire 40 streams over one h2 connection; the first couple are within
/// the burst (reach auth ⇒ 401 anonymous), and the rest MUST be 429'd (rate-limited before auth). The
/// load-bearing assertion: at least one 429 appears AND no request is a 2xx (a 429 is strictly less than
/// auth would grant — never a bypass). A limiter that only counted the CONNECTION (not each stream)
/// would show ZERO 429s here — the bug this guards against.
#[tokio::test]
#[ignore = "needs the fixture test cert + real socket I/O; run with --ignored"]
async fn per_ip_limiter_trips_over_multiplexed_h2_streams() {
    // burst 2, refill ~0 ⇒ the 3rd+ request from the one peer IP is 429'd.
    let app = app_with_rate_limit(0.0001, 2.0);
    let (addr, handle) = boot_app_over_tls(app).await;

    let (mut send_req, driver) = connect_h2(addr).await;

    // Fire many streams over the SINGLE connection. Open them all (multiplexed) and collect each
    // response's status. We target a PROTECTED path so the within-burst requests reach auth (401), which
    // distinguishes "reached auth" (401) from "rate-limited before auth" (429) — a 2xx would be a bypass.
    let url = format!("https://localhost/{}", "alice/private");
    let mut response_futs = Vec::new();
    for _ in 0..40u32 {
        // poll_ready before each send_request (h2 backpressure / stream-credit).
        futures_util::future::poll_fn(|cx| send_req.poll_ready(cx))
            .await
            .expect("h2 connection ready for a new stream");
        let req = http::Request::builder()
            .method("GET")
            .uri(&url)
            .body(())
            .expect("build request");
        let (resp_fut, _send_stream) = send_req
            .send_request(req, true) // end_stream = true (no body)
            .expect("send multiplexed h2 request");
        response_futs.push(resp_fut);
    }

    // Await every multiplexed response and tally the statuses.
    let mut count_429 = 0u32;
    let mut count_401 = 0u32;
    let mut count_2xx = 0u32;
    let mut count_other = 0u32;
    for fut in response_futs {
        match tokio::time::timeout(Duration::from_secs(10), fut).await {
            Ok(Ok(resp)) => {
                let s = resp.status().as_u16();
                match s {
                    429 => count_429 += 1,
                    401 => count_401 += 1,
                    200..=299 => count_2xx += 1,
                    _ => count_other += 1,
                }
            }
            Ok(Err(e)) => panic!("an h2 stream errored unexpectedly: {e:?}"),
            Err(_) => panic!("an h2 response did not arrive in time (server stalled?)"),
        }
    }

    eprintln!(
        "h2-multiplexed over one IP: 401(auth)={count_401} 429(rate-limited)={count_429} \
         2xx={count_2xx} other={count_other}"
    );

    // (1) The limiter TRIPPED over multiplexed streams: at least one 429. A connection-only (not
    // per-stream) limiter would show ZERO 429s here — this is the non-vacuous proof.
    assert!(
        count_429 > 0,
        "the per-IP rate limiter did NOT trip over multiplexed h2 streams from one IP — a 429 must \
         appear (else h2 multiplexing BYPASSES the per-IP limit). 401={count_401} 2xx={count_2xx}"
    );
    // (2) NO request was a 2xx — a rate-limited/anonymous request must never succeed (no auth bypass).
    assert_eq!(
        count_2xx, 0,
        "no multiplexed request may be a 2xx (anonymous + rate-limited ⇒ 401/429 only — a 2xx is an \
         auth bypass). 2xx count was {count_2xx}"
    );
    // (3) Sanity: the within-burst requests DID reach auth (some 401s) — so the limiter is admitting the
    // burst (not denying everything), proving it is the RATE limit tripping, not a blanket deny.
    assert!(
        count_401 >= 1,
        "expected the within-burst multiplexed requests to reach auth (>=1 401); got {count_401}"
    );

    handle.graceful_shutdown(Some(Duration::from_secs(1)));
    driver.abort();
}
