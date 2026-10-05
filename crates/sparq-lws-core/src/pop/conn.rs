// AUTHORED-BY Claude Opus 4.8
//! PoP Tier-1b — the per-connection client-certificate binding (`ConnPop`) + its acceptor wiring.
//!
//! Design: `research/lws-design-records.md` §7 (RSS `docs/design/high-throughput-pop-auth.md` §7,
//! bead 2 — not in this tree). This is the TRANSPORT half of RFC 8705 mTLS-bound tokens: read the
//! client certificate the peer presented on the TLS connection **once per connection**, compute its
//! `cnf.x5t#S256` thumbprint, and make it available to every request served over that connection so
//! the auth layer can match a cert-bound token against it ([`crate::auth`] +
//! [`crate::pop::dispatch`]).
//!
//! ## Why once-per-connection (not once-per-request)
//! A TLS connection's client certificate is fixed for the connection's lifetime (RFC 8705 §3 + the
//! rustls `peer_certificates()` contract — it is the ORIGINAL handshake identity, returned identically
//! for full AND resumed handshakes). Hashing it once at accept time and stamping the result onto every
//! request via a request extension turns the RFC 8705 §3 per-request obligation into a 32-byte memcmp
//! (in [`crate::pop::cert_bound`]) rather than a per-request SHA-256 over the DER.
//!
//! ## TLS-resumption re-binding (a fail-closed property, not a special case)
//! A resumed TLS session is a NEW connection object with its OWN `ServerConnection`; the acceptor runs
//! [`ConnPopAcceptor::accept`] afresh for it and reads `peer_certificates()` from THAT connection. So a
//! resumed connection cannot inherit a stale/foreign cert binding from an earlier connection — its
//! `ConnPop` is (re)computed from its own session state. If a resumed connection presents no client
//! certificate, its `ConnPop.thumbprint` is `None` and a cert-bound token over it is rejected
//! fail-closed (never a downgrade to bearer). This is the connection-scoped-injection design, not a
//! bespoke resumption branch.
//!
//! ## Gating
//! This wiring is added to the serve path ONLY when the mTLS flag is on
//! ([`crate::tls::mtls_bound_tokens_from_env`]). With the flag off, no `ConnPop` is injected and the
//! auth layer runs no confirmation dispatch, so the DPoP/plain serve paths are byte-identical.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use axum_server::accept::Accept;

use super::cert_bound::CertThumbprint;
use super::sk::derive::{Ekm, EXPORTER_CONTEXT, EXPORTER_LABEL};
use super::sk::{ConnSk, SESSION_ENDPOINT_PATH};

/// The per-connection proof-of-possession context: the SHA-256 thumbprint of the client certificate
/// the peer presented on THIS TLS connection, or `None` when the connection presented no client
/// certificate. Injected into every request's extensions by [`ConnPopService`]; read by the auth layer
/// to satisfy (or fail-closed reject) a cert-bound token.
///
/// `Clone` (cheap — a 32-byte thumbprint) so the connection-constant value can be stamped onto each
/// request; `Send + Sync + 'static` so it is a valid axum/http request extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnPop {
    /// The presented client certificate's `cnf.x5t#S256` thumbprint, or `None` if no client cert was
    /// presented on this connection.
    pub thumbprint: Option<CertThumbprint>,
}

impl ConnPop {
    /// Build the connection PoP from the peer certificate's DER bytes (the FIRST/leaf cert), computing
    /// the thumbprint once. `None` DER ⇒ no client certificate on this connection ⇒ `thumbprint: None`.
    pub fn from_peer_cert_der(der: Option<Vec<u8>>) -> Self {
        Self {
            thumbprint: der.map(|d| CertThumbprint::from_cert_der(&d)),
        }
    }

    /// The presented thumbprint, if any (borrowing, for the constant-time compare in
    /// [`crate::pop::cert_bound::verify_cert_binding`]).
    pub fn thumbprint(&self) -> Option<&CertThumbprint> {
        self.thumbprint.as_ref()
    }
}

/// A connection IO stream that can yield the peer (client) certificate DER it negotiated.
///
/// Implemented for the concrete TLS stream the serve path produces ([`tokio_rustls::server::TlsStream`])
/// and forwarded through the connection-cap wrapper ([`crate::transport::PermittedStream`], impl in
/// `transport.rs`). Reading it is only meaningful AFTER the TLS handshake has completed — the acceptor
/// calls it exactly there, once.
pub trait PeerCertDer {
    /// The DER bytes of the peer's leaf certificate, or `None` if the peer presented no client
    /// certificate (the common plain-DPoP case).
    fn peer_cert_der(&self) -> Option<Vec<u8>>;
}

impl<S> PeerCertDer for tokio_rustls::server::TlsStream<S> {
    fn peer_cert_der(&self) -> Option<Vec<u8>> {
        // `get_ref().1` is the rustls `ServerConnection`; `peer_certificates()` returns the client
        // chain (leaf first) for BOTH full and resumed handshakes, or `None` when no client cert was
        // presented. We copy the leaf DER (a public value) so nothing borrows the connection.
        let (_io, conn) = self.get_ref();
        conn.peer_certificates()
            .and_then(|certs| certs.first())
            .map(|cert| cert.as_ref().to_vec())
    }
}

/// A connection IO stream that can export TLS keying material for the DPoP-SK `cb=tls-exporter`
/// flavour (PoP Tier 2). Like [`PeerCertDer`], meaningful only AFTER the handshake completed —
/// the acceptor calls it exactly there, once per connection.
pub trait TlsExporter {
    /// 32 bytes of RFC 8446 §7.5 Exported Keying Material under `label` with an explicit
    /// zero-length context, or `None` when unavailable. Implementations MUST return `None` for
    /// anything below TLS 1.3 (the DPoP-SK spec REQUIRES TLS 1.3 for the flavour — exporter
    /// soundness on older versions carries extended-master-secret preconditions this profile
    /// refuses to depend on) — fail-closed: no EKM ⇒ `cb=tls-exporter` establishment is a 400.
    fn export_ekm_32(&self, label: &[u8]) -> Option<[u8; 32]>;
}

impl<S> TlsExporter for tokio_rustls::server::TlsStream<S> {
    fn export_ekm_32(&self, label: &[u8]) -> Option<[u8; 32]> {
        let (_io, conn) = self.get_ref();
        // TLS 1.3 REQUIRED (spec §"Key derivation"): rustls would happily export on TLS 1.2, so
        // gate on the negotiated protocol version explicitly.
        if conn.protocol_version() != Some(rustls::ProtocolVersion::TLSv1_3) {
            return None;
        }
        // The exporter uses the `exporter_master_secret` (rustls' post-handshake exporter; the
        // early exporter is never exposed here — 0-RTT stays disabled, `max_early_data_size = 0`).
        conn.export_keying_material([0u8; 32], label, Some(EXPORTER_CONTEXT))
            .ok()
    }
}

/// Monotonic per-process TLS-connection ids for the DPoP-SK connection pin. Never reused within a
/// process lifetime; sessions are in-process soft state, so process-scoped uniqueness suffices.
static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

fn next_conn_id() -> u64 {
    NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed)
}

/// The per-connection DPoP-SK context the acceptor computed (PoP Tier 2): the connection id (for
/// the `cb=tls-exporter` session pin) + the exporter output (present only on a TLS 1.3
/// connection). Held by the per-connection service; the EKM secret is stamped ONLY onto the
/// establishment-path request (no other request needs it).
#[derive(Debug, Clone)]
pub struct SkConnContext {
    conn_id: u64,
    ekm: Option<Ekm>,
}

/// A tower [`Service`](tower::Service) wrapper that stamps a fixed [`ConnPop`] (and, when the
/// DPoP-SK exporter path is active, a [`ConnSk`]) onto every request's extensions before
/// delegating to the inner service. One instance per connection (built by [`ConnPopAcceptor`]),
/// so the connection-constant PoP reaches every request — the same pattern axum's `ConnectInfo`
/// uses to surface per-connection data.
#[derive(Clone)]
pub struct ConnPopService<S> {
    inner: S,
    pop: ConnPop,
    sk: Option<SkConnContext>,
}

impl<S> ConnPopService<S> {
    /// Wrap `inner`, injecting `pop` into each request (no DPoP-SK context — the Tier-1b-only path).
    pub fn new(inner: S, pop: ConnPop) -> Self {
        Self {
            inner,
            pop,
            sk: None,
        }
    }

    /// Additionally inject the DPoP-SK per-connection context (PoP Tier 2).
    pub fn with_sk(mut self, sk: Option<SkConnContext>) -> Self {
        self.sk = sk;
        self
    }
}

impl<S, B> tower::Service<http::Request<B>> for ConnPopService<S>
where
    S: tower::Service<http::Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: http::Request<B>) -> Self::Future {
        // Insert the connection PoP so downstream (the auth middleware) can read it. `insert` REPLACES
        // any pre-existing `ConnPop` extension — a request cannot smuggle its own cert binding (the
        // value is set HERE from the TLS connection, never from client-controlled request data).
        req.extensions_mut().insert(self.pop.clone());
        if let Some(sk) = &self.sk {
            // Same anti-smuggle property for the DPoP-SK context. The per-connection EKM SECRET is
            // exposed only to the establishment endpoint's request; every other request carries just
            // the connection id (the session pin input).
            let ekm = if req.uri().path() == SESSION_ENDPOINT_PATH {
                sk.ekm.clone()
            } else {
                None
            };
            req.extensions_mut().insert(ConnSk::new(sk.conn_id, ekm));
        }
        self.inner.call(req)
    }
}

/// An [`Accept`] wrapper that reads the peer certificate ONCE (right after the inner accept completes
/// the TLS handshake), computes its [`ConnPop`], and wraps the produced service in a [`ConnPopService`]
/// so every request on the connection carries the binding. The IO stream is passed through UNCHANGED
/// (`type Stream = A::Stream`) — this wrapper only reads from it and wraps the SERVICE.
///
/// Layered OUTERMOST on the TLS serve path, above the connection-cap acceptor, so the handshake is
/// already complete when [`accept`](Self::accept) reads the cert. Added only when the mTLS flag is on.
#[derive(Clone)]
pub struct ConnPopAcceptor<A> {
    inner: A,
    /// PoP Tier 2: when `true`, also export DPoP-SK keying material once per connection and inject
    /// a [`ConnSk`] (connection id + establishment-path EKM) alongside the [`ConnPop`]. Off by
    /// default so the Tier-1b-only path is unchanged.
    sk_exporter: bool,
}

impl<A> ConnPopAcceptor<A> {
    /// Wrap `inner` (the connection-cap acceptor over the rustls acceptor).
    pub fn new(inner: A) -> Self {
        Self {
            inner,
            sk_exporter: false,
        }
    }

    /// Enable the DPoP-SK exporter path (`SOLID_SERVER_DPOP_SK` + in-process TLS).
    pub fn with_sk_exporter(mut self, enabled: bool) -> Self {
        self.sk_exporter = enabled;
        self
    }
}

impl<A, I, S> Accept<I, S> for ConnPopAcceptor<A>
where
    A: Accept<I, S> + Clone + Send + Sync + 'static,
    A::Stream: PeerCertDer + TlsExporter + Send,
    A::Service: Send + 'static,
    A::Future: Send,
    I: Send + 'static,
    S: Send + 'static,
{
    type Stream = A::Stream;
    type Service = ConnPopService<A::Service>;
    type Future = Pin<Box<dyn Future<Output = io::Result<(Self::Stream, Self::Service)>> + Send>>;

    fn accept(&self, stream: I, service: S) -> Self::Future {
        let inner = self.inner.clone();
        let sk_exporter = self.sk_exporter;
        Box::pin(async move {
            // The inner accept resolves AFTER the TLS handshake completes, so the peer certificate (if
            // any) is available on the produced stream. Read + hash it exactly once here.
            let (io_stream, svc) = inner.accept(stream, service).await?;
            let pop = ConnPop::from_peer_cert_der(io_stream.peer_cert_der());
            // PoP Tier 2: export the DPoP-SK keying material exactly once per connection. `None`
            // (not TLS 1.3 / export failure) is carried as-is — establishment then fails CLOSED
            // with a 400, never a derivation from weaker material.
            let sk = sk_exporter.then(|| SkConnContext {
                conn_id: next_conn_id(),
                ekm: io_stream.export_ekm_32(EXPORTER_LABEL).map(Ekm::new),
            });
            Ok((io_stream, ConnPopService::new(svc, pop).with_sk(sk)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::future::Ready;

    const CERT_A: &[u8] = b"-----fake-der-cert-A-----";

    #[test]
    fn conn_pop_hashes_present_cert_and_none_stays_none() {
        // A presented cert ⇒ its thumbprint; no cert ⇒ None (a cert-bound token over it is denied
        // fail-closed downstream).
        let with = ConnPop::from_peer_cert_der(Some(CERT_A.to_vec()));
        assert_eq!(
            with.thumbprint(),
            Some(&CertThumbprint::from_cert_der(CERT_A))
        );
        let without = ConnPop::from_peer_cert_der(None);
        assert_eq!(without.thumbprint(), None);
    }

    /// A stream stand-in exposing a fixed peer cert DER (models a handshaked TLS stream without needing
    /// a real handshake — the trait is the seam the acceptor consumes). The exporter half models a
    /// TLS 1.3 connection whose EKM depends on the LABEL (as a real exporter's does), so the
    /// wrong-label negative is exercisable at this seam.
    struct FakeStream(Option<Vec<u8>>);
    impl PeerCertDer for FakeStream {
        fn peer_cert_der(&self) -> Option<Vec<u8>> {
            self.0.clone()
        }
    }
    impl TlsExporter for FakeStream {
        fn export_ekm_32(&self, label: &[u8]) -> Option<[u8; 32]> {
            use sha2::{Digest as _, Sha256};
            let mut h = Sha256::new();
            h.update(b"fake-conn-secret");
            h.update(label);
            Some(h.finalize().into())
        }
    }

    #[test]
    fn peer_cert_der_trait_yields_the_presented_der() {
        assert_eq!(
            FakeStream(Some(CERT_A.to_vec())).peer_cert_der(),
            Some(CERT_A.to_vec())
        );
        assert_eq!(FakeStream(None).peer_cert_der(), None);
    }

    /// A trivial inner tower service that records the `ConnPop` (if any) it saw on the request, so the
    /// service-wrapper test can assert the extension was injected.
    #[derive(Clone)]
    struct RecordingService;
    impl tower::Service<http::Request<()>> for RecordingService {
        type Response = Option<ConnPop>;
        type Error = Infallible;
        type Future = Ready<Result<Option<ConnPop>, Infallible>>;
        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, req: http::Request<()>) -> Self::Future {
            std::future::ready(Ok(req.extensions().get::<ConnPop>().cloned()))
        }
    }

    #[tokio::test]
    async fn service_injects_conn_pop_into_request_extensions() {
        use tower::ServiceExt as _;
        let pop = ConnPop::from_peer_cert_der(Some(CERT_A.to_vec()));
        let svc = ConnPopService::new(RecordingService, pop.clone());
        let seen = svc
            .oneshot(http::Request::new(()))
            .await
            .expect("infallible");
        assert_eq!(
            seen,
            Some(pop),
            "the request must carry the connection's ConnPop"
        );
    }

    #[tokio::test]
    async fn service_replaces_a_smuggled_conn_pop_extension() {
        // Defence in depth: a request arriving with its OWN ConnPop extension (a hypothetical smuggle)
        // must be OVERWRITTEN by the connection's real value — never trusted.
        use tower::ServiceExt as _;
        let real = ConnPop::from_peer_cert_der(Some(CERT_A.to_vec()));
        let forged = ConnPop::from_peer_cert_der(Some(b"forged".to_vec()));
        let svc = ConnPopService::new(RecordingService, real.clone());
        let mut req = http::Request::new(());
        req.extensions_mut().insert(forged);
        let seen = svc.oneshot(req).await.expect("infallible");
        assert_eq!(
            seen,
            Some(real),
            "the connection's real ConnPop must win over a smuggled one"
        );
    }

    /// A mock acceptor producing a `FakeStream` + a unit service — exercises `ConnPopAcceptor::accept`
    /// end to end (reads the cert from the stream, wraps the service) without a TLS handshake.
    #[derive(Clone)]
    struct FakeAcceptor {
        der: Option<Vec<u8>>,
    }
    impl<S> Accept<(), S> for FakeAcceptor {
        type Stream = FakeStream;
        type Service = S;
        type Future = Ready<io::Result<(FakeStream, S)>>;
        fn accept(&self, _stream: (), service: S) -> Self::Future {
            std::future::ready(Ok((FakeStream(self.der.clone()), service)))
        }
    }

    #[tokio::test]
    async fn acceptor_reads_cert_once_and_wraps_service() {
        use tower::ServiceExt as _;
        let acceptor = ConnPopAcceptor::new(FakeAcceptor {
            der: Some(CERT_A.to_vec()),
        });
        let (_stream, wrapped) = acceptor
            .accept((), RecordingService)
            .await
            .expect("accept ok");
        // The wrapped service must inject the ConnPop derived from the stream's cert.
        let seen = wrapped
            .oneshot(http::Request::new(()))
            .await
            .expect("infallible");
        assert_eq!(
            seen,
            Some(ConnPop::from_peer_cert_der(Some(CERT_A.to_vec())))
        );
    }

    /// A recording service capturing the `ConnSk` (if any) a request carried.
    #[derive(Clone)]
    struct SkRecordingService;
    impl tower::Service<http::Request<()>> for SkRecordingService {
        type Response = Option<ConnSk>;
        type Error = Infallible;
        type Future = Ready<Result<Option<ConnSk>, Infallible>>;
        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, req: http::Request<()>) -> Self::Future {
            std::future::ready(Ok(req.extensions().get::<ConnSk>().cloned()))
        }
    }

    #[tokio::test]
    async fn sk_context_injects_conn_id_everywhere_but_ekm_only_on_establishment_path() {
        use tower::ServiceExt as _;
        let ekm = Ekm::new([9u8; 32]);
        let svc = ConnPopService::new(SkRecordingService, ConnPop { thumbprint: None }).with_sk(
            Some(SkConnContext {
                conn_id: 42,
                ekm: Some(ekm.clone()),
            }),
        );
        // Establishment path: conn_id + the EKM secret.
        let req = http::Request::builder()
            .uri(SESSION_ENDPOINT_PATH)
            .body(())
            .unwrap();
        let seen = svc.clone().oneshot(req).await.expect("infallible").unwrap();
        assert_eq!(seen.conn_id, 42);
        assert_eq!(seen.ekm, Some(ekm));
        // Any other path: conn_id only — the secret is not spread across ordinary requests.
        let req = http::Request::builder()
            .uri("/alice/data")
            .body(())
            .unwrap();
        let seen = svc.clone().oneshot(req).await.expect("infallible").unwrap();
        assert_eq!(seen.conn_id, 42);
        assert_eq!(seen.ekm, None);
    }

    #[tokio::test]
    async fn sk_context_replaces_a_smuggled_conn_sk_extension() {
        use tower::ServiceExt as _;
        let svc = ConnPopService::new(SkRecordingService, ConnPop { thumbprint: None }).with_sk(
            Some(SkConnContext {
                conn_id: 1,
                ekm: None,
            }),
        );
        let mut req = http::Request::builder()
            .uri("/alice/data")
            .body(())
            .unwrap();
        // A forged ConnSk claiming a different pinned connection (and carrying a forged EKM) must
        // be OVERWRITTEN by the connection's real context — never trusted.
        req.extensions_mut()
            .insert(ConnSk::new(777, Some(Ekm::new([1u8; 32]))));
        let seen = svc.oneshot(req).await.expect("infallible").unwrap();
        assert_eq!(seen.conn_id, 1);
        assert_eq!(seen.ekm, None);
    }

    #[tokio::test]
    async fn acceptor_without_sk_flag_injects_no_conn_sk() {
        use tower::ServiceExt as _;
        let acceptor = ConnPopAcceptor::new(FakeAcceptor { der: None });
        let (_stream, wrapped) = acceptor
            .accept((), SkRecordingService)
            .await
            .expect("accept ok");
        let seen = wrapped
            .oneshot(
                http::Request::builder()
                    .uri(SESSION_ENDPOINT_PATH)
                    .body(())
                    .unwrap(),
            )
            .await
            .expect("infallible");
        assert_eq!(seen, None, "Tier-1b-only path must carry no ConnSk");
    }

    #[tokio::test]
    async fn acceptor_with_sk_flag_exports_ekm_under_the_dpop_sk_label() {
        use tower::ServiceExt as _;
        let acceptor = ConnPopAcceptor::new(FakeAcceptor { der: None }).with_sk_exporter(true);
        let (stream, wrapped) = acceptor
            .accept((), SkRecordingService)
            .await
            .expect("accept ok");
        let seen = wrapped
            .oneshot(
                http::Request::builder()
                    .uri(SESSION_ENDPOINT_PATH)
                    .body(())
                    .unwrap(),
            )
            .await
            .expect("infallible")
            .expect("ConnSk must be injected");
        // The EKM must be the stream's export under EXACTLY the dedicated DPoP-SK label…
        assert_eq!(
            seen.ekm,
            Some(Ekm::new(stream.export_ekm_32(EXPORTER_LABEL).unwrap()))
        );
        // …and NOT under any other label (e.g. the RFC 9266 channel-binding value).
        assert_ne!(
            seen.ekm,
            Some(Ekm::new(
                stream.export_ekm_32(b"EXPORTER-Channel-Binding").unwrap()
            ))
        );
    }

    #[tokio::test]
    async fn acceptor_no_client_cert_yields_none_binding() {
        use tower::ServiceExt as _;
        let acceptor = ConnPopAcceptor::new(FakeAcceptor { der: None });
        let (_stream, wrapped) = acceptor
            .accept((), RecordingService)
            .await
            .expect("accept ok");
        let seen = wrapped
            .oneshot(http::Request::new(()))
            .await
            .expect("infallible");
        assert_eq!(
            seen,
            Some(ConnPop { thumbprint: None }),
            "no client cert ⇒ a None-thumbprint ConnPop (cert-bound tokens denied fail-closed)"
        );
    }
}
