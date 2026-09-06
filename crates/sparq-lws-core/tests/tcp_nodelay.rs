// AUTHORED-BY Claude Fable 5
//! P1.5 (beyond-50k — `research/lws-design-records.md` §7; RSS
//! `docs/design/beyond-50k-throughput.md` §4): `TCP_NODELAY` is set on the accepted sockets of BOTH
//! serve paths. The DETERMINISTIC metric (per the perf-gate rule) is the socket-option STATE —
//! `TcpStream::nodelay()` reads back `true` after the path's nodelay mechanism runs — not any
//! wall-clock latency (which is advisory). Each test drives the EXACT primitive `src/main.rs`
//! wires:
//!
//! - the plain `axum::serve` path taps every accepted stream with [`sparq_lws_core::nodelay::tap_nodelay`]
//!   (the `ListenerExt` tap), and
//! - the in-process TLS path composes [`sparq_lws_core::nodelay::NoDelayAcceptor`] (axum-server's
//!   acceptor) as the inner acceptor of the `RustlsAcceptor`, which sets the option on the RAW
//!   `TcpStream` before the handshake.
//!
//! A regression that drops either wiring makes the corresponding `nodelay()` read `false` and fails.

use axum::serve::Listener;
use axum_server::accept::Accept;
use tokio::net::{TcpListener, TcpStream};

/// Plain path: the `tap_nodelay`-wrapped listener sets `TCP_NODELAY` on each accepted stream. This is
/// the exact wrapper `axum::serve(tap_nodelay(listener), ..)` uses in `main`; we drive one accept
/// through it and read the option back off the accepted `TcpStream`.
#[tokio::test]
async fn plain_path_tap_sets_nodelay_on_accepted_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let mut tapped = sparq_lws_core::nodelay::tap_nodelay(listener);

    // Connect a client so there is a pending connection to accept.
    let client = tokio::spawn(async move { TcpStream::connect(addr).await });

    // Accept through the tapped listener (the `axum::serve::Listener` seam `axum::serve` drives).
    let (io, _peer) = tapped.accept().await;
    assert!(
        io.nodelay().expect("read nodelay on the accepted stream"),
        "the plain serve path must set TCP_NODELAY on accepted sockets"
    );

    // The client half succeeded (sanity: a real accepted connection, not a spurious wakeup).
    let _client_stream = client.await.expect("client task").expect("client connect");
}

/// TLS path: the `NoDelayAcceptor` composed as the inner acceptor of the `RustlsAcceptor` sets
/// `TCP_NODELAY` on the RAW accepted `TcpStream` before the handshake. `NoDelayAcceptor::Stream ==
/// TcpStream`, so we can read the option straight off the acceptor's output — no handshake needed to
/// prove the socket-option effect (the DETERMINISTIC metric).
#[tokio::test]
async fn tls_path_nodelay_acceptor_sets_nodelay_on_raw_stream() {
    let acceptor = sparq_lws_core::nodelay::NoDelayAcceptor::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");

    let client = tokio::spawn(async move { TcpStream::connect(addr).await });

    let (server_stream, _peer) = listener.accept().await.expect("accept");
    // Run the acceptor exactly as composed inside the RustlsAcceptor: it sets nodelay on the raw
    // stream, then yields it unchanged. `()` stands in for the per-connection service.
    let (stream, _svc): (TcpStream, ()) =
        Accept::<TcpStream, ()>::accept(&acceptor, server_stream, ())
            .await
            .expect("nodelay acceptor");
    assert!(
        stream
            .nodelay()
            .expect("read nodelay on the raw accepted stream"),
        "the TLS serve path's NoDelayAcceptor must set TCP_NODELAY on the raw accepted socket"
    );

    let _client_stream = client.await.expect("client task").expect("client connect");
}

/// Baseline contrast (documents WHY the wiring is needed, not just that it works): a freshly accepted
/// `TcpStream` WITHOUT any nodelay mechanism has Nagle ON (`nodelay() == false`) on this platform's
/// default. This is the "before" state both serve paths would ship with absent P1.5 — so the two
/// assertions above are a real change, not a no-op.
#[tokio::test]
async fn untuned_accept_leaves_nagle_on_by_default() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let client = tokio::spawn(async move { TcpStream::connect(addr).await });
    let (server_stream, _peer) = listener.accept().await.expect("accept");
    assert!(
        !server_stream.nodelay().expect("read nodelay"),
        "a raw accepted stream defaults to Nagle ON — this is the baseline P1.5 changes"
    );
    let _client_stream = client.await.expect("client task").expect("client connect");
}
