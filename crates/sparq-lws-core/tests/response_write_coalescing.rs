// AUTHORED-BY Claude Fable 5
//! P1.4 (`research/lws-design-records.md` §7; RSS `docs/design/beyond-50k-throughput.md` §4 item
//! P1.4 — the vectored-write / response-coalescing audit): a DETERMINISTIC guard that pins the
//! number of write-family syscalls the HTTP/1.1 response path emits per response, measured at the
//! hyper→transport seam, **driving the REAL assembled router** (`build_router` — CORS → public-read
//! skip → auth → WAC → LDP handler → `serve_read`/`negotiate_body`) over the SAME `axum::serve`
//! path production uses. Both read paths are covered: the anonymous cases traverse CORS →
//! public-read skip → `serve_read` (the skip short-circuits before the auth middleware), and the
//! authenticated case (`real_authenticated_get_…`) carries a DPoP-bound token so it traverses the
//! FULL auth middleware → WAC → LDP handler → `serve_read` (the P0.1 `authed-doc` class).
//!
//! ## Why this test exists — the P1.4 finding
//!
//! The P0.1 Linux syscall baseline (`bench/syscalls-results/2026-07-04-linux.md`) shows a
//! **`writev` 1.04/req + `write` 1.04/req** pair on every read class (anon-doc, listing,
//! authed-doc). The RSS design doc's §2.1/§4 flagged, as UNVERIFIED, whether the response
//! "header+body coalesce into one vectored write" — the P1.4 lever was to collapse a presumed
//! two-writes-per-response into one.
//!
//! This test resolves that flag with a deterministic, cross-platform measurement. A `CountListener`
//! (an `axum::serve::Listener`) wraps every accepted loopback `TcpStream` in a counting adapter
//! that tallies each `poll_write` (a `write(2)`) vs `poll_write_vectored` (a `writev(2)`) hyper
//! issues — == the response's real connection-socket write-family syscalls, modulo partial writes,
//! which do not occur for these small loopback responses. `axum::serve` then serves the real router
//! over that listener while the test drives K sequential keep-alive requests.
//!
//! The measured result (asserted below): **exactly 1 `writev` and 0 plain `write` per response**
//! for a real anonymous public-document GET, a real `206 Partial Content` Range GET, a real
//! container-listing GET, and a real authenticated (DPoP-bound) private-document GET. hyper's h1
//! encoder ALREADY buffers the response head + the length-delimited
//! `Bytes` body the handler produces into ONE `WriteBuf` and flushes it as a single vectored write
//! (Queue strategy, since a loopback `TcpStream` advertises `is_write_vectored() == true`).
//!
//! **Therefore the P0.1 `write` 1.04/req is NOT the HTTP response** — the response is already at the
//! P1.4 target of one write per response. The second per-request `write` is a process-level
//! (non-connection-socket) syscall: the tokio multi-threaded runtime's mio reactor waker (`write()`
//! to an `eventfd`), which fires ~once per request under a single-connection work-stealing ping-pong.
//! It is NOT reducible by any response-serialization change; reducing it is a runtime-level lever
//! (`SO_REUSEPORT` sharded single-thread accept — P1.7 — or the Phase-3 thread-per-core direction),
//! out of P1.4's scope. Confirming the strace `write` is specifically the reactor waker fd (and the
//! TLS-transport write shape) needs an EC2 re-run of `bench/syscalls.sh` with
//! `strace -yy -e trace=write,writev` (the `-yy` shows the fd kind: `<eventfd:...>` vs the socket).
//!
//! ## nl48 reconciliation — same conclusion, now with a turnkey EC2 confirm
//! Bead `suite-tracker-nl48` re-raised "coalesce the anon-doc head+body write" from the SAME P0.1
//! `write`+`writev` pair. It is this exact, already-merged finding: the response is already one
//! vectored socket write, so there is no serialization change to make (forcing hyper's Flatten
//! strategy via `http1::Builder::writev(false)` would only swap the `writev` for a `write` **plus** a
//! body-into-header memcpy — same write-class count, strictly worse). The residual aggregate `write`
//! is the reactor-waker eventfd above. `bench/syscalls-fd-kind.sh` now makes the `strace -yy` fd-kind
//! confirm turnkey on the EC2, and `bench/SYSCALLS.md` §"nl48" records the reconciliation + the
//! secondary PUT `getrandom` (blob-key mint, security-adjacent — unchanged) / `futex` (in-memory
//! test-double lock — a harness artifact) findings.
//!
//! ## What this guards
//!
//! A REGRESSION guard, not an optimization. Because it drives the REAL handler, it catches a change
//! that splits a response into a header write + a separate body write — e.g. a handler returning the
//! body as a multi-frame / unknown-length streaming body instead of a single length-delimited `Bytes`
//! frame, or a body wrapper that breaks `is_end_stream`/`size_hint`. It also asserts the response
//! BYTES are exactly what the handler produced (a coalesced write must not truncate/reorder).
//!
//! Scope note: this exercises the PLAIN-HTTP serve path (the P0.1 baseline transport). Over TLS the
//! same bytes leave as a single flattened `write` (rustls advertises `is_write_vectored() == false`
//! → hyper's Flatten strategy) — an INFERENCE from the strategy selection, not measured here; the
//! EC2 follow-up covers it.

mod common;

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use common::{jwks_provider, mint_access_token, mint_dpop_proof, KeyKit, BASE_URL, ISSUER};
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient, Store};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};

const WEBID: &str = common::WEBID;

/// Counts hyper's write-family calls at the transport seam.
struct Counters {
    writes: AtomicUsize,
    writevs: AtomicUsize,
}

/// Wraps a real accepted `TcpStream`, forwarding all I/O to it while counting how many times hyper
/// calls `poll_write` (a plain `write(2)`) vs `poll_write_vectored` (a `writev(2)`). Because hyper
/// writes exclusively through the IO handed to it, these counts equal the response's real
/// write-family syscalls on the connection socket (partial writes, which would inflate the count, do
/// not occur for these small loopback responses).
struct CountIo {
    inner: TcpStream,
    c: Arc<Counters>,
}

impl AsyncRead for CountIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for CountIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.c.writes.fetch_add(1, Ordering::SeqCst);
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.c.writevs.fetch_add(1, Ordering::SeqCst);
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// An `axum::serve::Listener` that wraps every accepted `TcpStream` in a `CountIo`, so the entire
/// real serve path (`axum::serve` → `hyper_util` auto builder → hyper h1) writes through the
/// counter. Sets `TCP_NODELAY` to match the production accept path (`src/nodelay.rs`).
struct CountListener {
    inner: TcpListener,
    c: Arc<Counters>,
}

impl axum::serve::Listener for CountListener {
    type Io = CountIo;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.inner.accept().await {
                Ok((sock, addr)) => {
                    sock.set_nodelay(true).ok();
                    return (
                        CountIo {
                            inner: sock,
                            c: self.c.clone(),
                        },
                        addr,
                    );
                }
                Err(_) => continue,
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

type MemStore = CompositeStore<InMemorySparqClient, InMemoryBlobStore>;

async fn seed_acl(store: &MemStore, iri: &str, body: &str) {
    store
        .write(
            iri,
            axum::body::Bytes::from(body.to_string()),
            "text/turtle",
        )
        .await
        .expect("seed acl");
}

/// Build the real router around a caller-seeded in-memory store, exactly as `main`/the integration
/// tests do (`build_router(AppState::new(ctx, ldp))`). `issuer_key` is the key whose JWK the verifier
/// trusts — the caller supplies it so an authenticated request can mint a token the server accepts
/// (for the anonymous cases it is irrelevant; a throwaway key is fine).
fn build_real_router(store: MemStore, issuer_key: &KeyKit) -> axum::Router {
    let config = VerifierConfig::new(vec![ISSUER.to_string()], BASE_URL);
    let replay = InMemoryReplayStore::with_window(config.replay_ttl());
    let verifier = Verifier::new(config, jwks_provider(issuer_key), replay).unwrap();
    let ctx = AuthContext::new(verifier, BASE_URL);
    let ldp = LdpState::new(store, BASE_URL);
    build_router(AppState::new(ctx, ldp))
}

/// Serve `router` over a fresh loopback `CountListener`; returns `(addr, counters, join_handle)`.
async fn serve(router: axum::Router) -> (SocketAddr, Arc<Counters>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let c = Arc::new(Counters {
        writes: AtomicUsize::new(0),
        writevs: AtomicUsize::new(0),
    });
    let count_listener = CountListener {
        inner: listener,
        c: c.clone(),
    };
    let handle = tokio::spawn(async move {
        let _ = axum::serve(count_listener, router.into_make_service()).await;
    });
    (addr, c, handle)
}

/// One HTTP/1.1 response read off `stream`: `(status_code, body_bytes)`.
async fn read_one_response(
    stream: &mut TcpStream,
    rbuf: &mut [u8],
    acc: &mut Vec<u8>,
) -> (u16, Vec<u8>) {
    let (hdr_end, status, body_len) = loop {
        if let Some(pos) = acc.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&acc[..pos]);
            let status: u16 = head
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse().ok())
                .expect("status line");
            let cl: usize = head
                .to_ascii_lowercase()
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length:")
                        .map(|v| v.trim().to_string())
                })
                .and_then(|v| v.parse().ok())
                .expect("content-length header");
            break (pos + 4, status, cl);
        }
        let n = stream.read(rbuf).await.unwrap();
        assert!(n > 0, "unexpected EOF reading headers");
        acc.extend_from_slice(&rbuf[..n]);
    };
    while acc.len() < hdr_end + body_len {
        let n = stream.read(rbuf).await.unwrap();
        assert!(n > 0, "unexpected EOF reading body");
        acc.extend_from_slice(&rbuf[..n]);
    }
    let body = acc[hdr_end..hdr_end + body_len].to_vec();
    acc.drain(..hdr_end + body_len);
    (status, body)
}

/// Drive K sequential keep-alive requests (raw request line + Host) on ONE connection; returns each
/// response's `(status, body)`.
async fn drive(addr: SocketAddr, request: &[u8], k: usize) -> Vec<(u16, Vec<u8>)> {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.set_nodelay(true).ok();
    let mut rbuf = vec![0u8; 16384];
    let mut acc: Vec<u8> = Vec::new();
    let mut out = Vec::with_capacity(k);
    for _ in 0..k {
        stream.write_all(request).await.unwrap();
        stream.flush().await.unwrap();
        out.push(read_one_response(&mut stream, &mut rbuf, &mut acc).await);
    }
    drop(stream);
    out
}

/// Drive K sequential keep-alive requests carrying a valid DPoP-bound `(Authorization, DPoP)` pair
/// (the P0.1 `authed-doc` class). A FRESH access token + a FRESH DPoP proof (unique `jti`) is minted
/// per request, so the anti-replay store admits every one and the FULL auth middleware → WAC → LDP
/// handler runs each time — the public-read skip does NOT fire when an `Authorization` header is
/// present. Returns each response's `(status, body)`.
async fn drive_authenticated(
    addr: SocketAddr,
    method: &str,
    path: &str,
    extra_header: Option<&str>,
    issuer_key: &KeyKit,
    client_key: &KeyKit,
    k: usize,
) -> Vec<(u16, Vec<u8>)> {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.set_nodelay(true).ok();
    let mut rbuf = vec![0u8; 16384];
    let mut acc: Vec<u8> = Vec::new();
    let mut out = Vec::with_capacity(k);
    let htu = format!("{BASE_URL}{path}");
    for _ in 0..k {
        // Fresh token + proof per request — the proof `jti` is unique, so replay never rejects.
        let access = mint_access_token(issuer_key, &client_key.thumbprint);
        let proof = mint_dpop_proof(client_key, method, &htu, &access);
        let extra = extra_header.map(|h| format!("{h}\r\n")).unwrap_or_default();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: pod.example\r\nAuthorization: DPoP {access}\r\nDPoP: {proof}\r\n{extra}\r\n"
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        out.push(read_one_response(&mut stream, &mut rbuf, &mut acc).await);
    }
    drop(stream);
    out
}

const PUB_DOC: &str =
    "<https://pod.example/pub#it> <http://xmlns.com/foaf/0.1/name> \"Public\" .\n";

/// Seed a private-by-default root plus a foaf:Agent-readable public document at `/pub`.
async fn seed_public_doc(store: &MemStore) {
    seed_acl(
        store,
        "https://pod.example/.acl",
        &format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>;
  acl:accessTo <https://pod.example/>; acl:default <https://pod.example/>;
  acl:mode acl:Read, acl:Write, acl:Control."#
        ),
    )
    .await;
    store
        .write(
            "https://pod.example/pub",
            axum::body::Bytes::from(PUB_DOC),
            "text/turtle",
        )
        .await
        .unwrap();
    seed_acl(
        store,
        "https://pod.example/pub.acl",
        &format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
@prefix foaf: <http://xmlns.com/foaf/0.1/>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>; acl:accessTo <https://pod.example/pub>; acl:mode acl:Read, acl:Write, acl:Control.
<#pub> a acl:Authorization; acl:agentClass foaf:Agent; acl:accessTo <https://pod.example/pub>; acl:mode acl:Read."#
        ),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_public_get_is_single_vectored_write() {
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    seed_public_doc(&store).await;
    let (addr, c, handle) = serve(build_real_router(store, &KeyKit::generate())).await;

    let k = 16;
    let responses = drive(addr, b"GET /pub HTTP/1.1\r\nHost: pod.example\r\n\r\n", k).await;
    handle.abort();

    for (status, body) in &responses {
        assert_eq!(*status, 200, "public GET must be 200");
        assert_eq!(
            body.as_slice(),
            PUB_DOC.as_bytes(),
            "body must be byte-identical"
        );
    }
    let writes = c.writes.load(Ordering::SeqCst);
    let writevs = c.writevs.load(Ordering::SeqCst);
    assert_eq!(
        writes, 0,
        "no plain write() — head+body must not split (got {writes})"
    );
    assert_eq!(
        writevs, k,
        "exactly one writev per response (got {writevs} over {k})"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_range_206_is_single_vectored_write() {
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    seed_public_doc(&store).await;
    let (addr, c, handle) = serve(build_real_router(store, &KeyKit::generate())).await;

    let k = 16;
    let responses = drive(
        addr,
        b"GET /pub HTTP/1.1\r\nHost: pod.example\r\nRange: bytes=0-9\r\n\r\n",
        k,
    )
    .await;
    handle.abort();

    let expected = &PUB_DOC.as_bytes()[0..=9];
    for (status, body) in &responses {
        assert_eq!(*status, 206, "range GET must be 206");
        assert_eq!(
            body.as_slice(),
            expected,
            "partial body must be byte-identical"
        );
    }
    let writes = c.writes.load(Ordering::SeqCst);
    let writevs = c.writevs.load(Ordering::SeqCst);
    assert_eq!(
        writes, 0,
        "no plain write() for a 206 Range body (got {writes})"
    );
    assert_eq!(
        writevs, k,
        "one writev per 206 response (got {writevs} over {k})"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_container_listing_is_single_vectored_write() {
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    // Root owner ACL.
    seed_acl(
        &store,
        "https://pod.example/.acl",
        &format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>;
  acl:accessTo <https://pod.example/>; acl:default <https://pod.example/>;
  acl:mode acl:Read, acl:Write, acl:Control."#
        ),
    )
    .await;
    // A public container with a benchmark-sized child set (matching the P0.1 `listing` class of 100
    // children ≈ 5 KiB — `bench/syscalls-results/2026-07-04-linux.md`), so the rendered `ldp:contains`
    // listing exercises the same larger-body write path the baseline measured. The container record
    // must exist first (an empty-body write, as `seed::ensure_container` does); children are wired via
    // `create_in_container` so the `ldp:contains` edges are real.
    const CHILDREN: usize = 100;
    store
        .write(
            "https://pod.example/list/",
            axum::body::Bytes::new(),
            "text/turtle",
        )
        .await
        .unwrap();
    for i in 0..CHILDREN {
        store
            .create_in_container(
                "https://pod.example/list/",
                &format!("https://pod.example/list/child{i}"),
                axum::body::Bytes::from(PUB_DOC),
                "text/turtle",
            )
            .await
            .unwrap();
    }
    seed_acl(
        &store,
        "https://pod.example/list/.acl",
        &format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
@prefix foaf: <http://xmlns.com/foaf/0.1/>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>; acl:accessTo <https://pod.example/list/>; acl:default <https://pod.example/list/>; acl:mode acl:Read, acl:Write, acl:Control.
<#pub> a acl:Authorization; acl:agentClass foaf:Agent; acl:accessTo <https://pod.example/list/>; acl:default <https://pod.example/list/>; acl:mode acl:Read."#
        ),
    )
    .await;

    let (addr, c, handle) = serve(build_real_router(store, &KeyKit::generate())).await;
    let k = 16;
    let responses = drive(addr, b"GET /list/ HTTP/1.1\r\nHost: pod.example\r\n\r\n", k).await;
    handle.abort();

    for (status, body) in &responses {
        assert_eq!(*status, 200, "public container GET must be 200");
        // Benchmark-sized listing (~5 KiB for 100 children) — the P0.1 `listing` class body size, so
        // this exercises the larger-body write path, not a trivial one.
        assert!(
            body.len() >= 3000,
            "listing body must be benchmark-sized (got {} bytes)",
            body.len()
        );
        // Deterministic content: EVERY seeded child must appear individually in the rendered
        // `ldp:contains` membership (order-independent — the listing may not be sorted). Each child is
        // a Turtle IRI term `<…/list/child{i}>`; the trailing `>` distinguishes `child1` from
        // `child10`, so a body that duplicated one child and omitted others cannot pass.
        let body_str = String::from_utf8_lossy(body);
        for i in 0..CHILDREN {
            let term = format!("/list/child{i}>");
            assert!(
                body_str.contains(&term),
                "listing must render child {i} (`{term}` absent)"
            );
        }
        // The listing render is deterministic per store, so every keep-alive response is identical.
        assert_eq!(
            body, &responses[0].1,
            "each listing response must be identical"
        );
    }
    let writes = c.writes.load(Ordering::SeqCst);
    let writevs = c.writevs.load(Ordering::SeqCst);
    assert_eq!(
        writes, 0,
        "no plain write() for the listing body (got {writes})"
    );
    assert_eq!(
        writevs, k,
        "one writev per listing response (got {writevs} over {k})"
    );
}

const PRIV_DOC: &str =
    "<https://pod.example/priv#it> <http://xmlns.com/foaf/0.1/name> \"Private\" .\n";

/// The P0.1 `authed-doc` class: a CREDENTIALED owner GET of a PRIVATE resource. Because an
/// `Authorization` header is present, the public-read skip does NOT fire — the request traverses the
/// FULL auth middleware (DPoP-bound token verify) → WAC (owner Read) → LDP handler → `serve_read`.
/// This is the coverage the anonymous cases (which short-circuit at the public-read skip) cannot give.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_authenticated_get_is_single_vectored_write() {
    let issuer_key = KeyKit::generate();
    let client_key = KeyKit::generate();
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    // Private-by-default root owned by WEBID (no public default).
    seed_acl(
        &store,
        "https://pod.example/.acl",
        &format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>;
  acl:accessTo <https://pod.example/>; acl:default <https://pod.example/>;
  acl:mode acl:Read, acl:Write, acl:Control."#
        ),
    )
    .await;
    // A PRIVATE document — owner-only, NO public grant. Anonymous would 401; only the authed owner
    // reads it, so the full crypto verify runs.
    store
        .write(
            "https://pod.example/priv",
            axum::body::Bytes::from(PRIV_DOC),
            "text/turtle",
        )
        .await
        .unwrap();
    seed_acl(
        &store,
        "https://pod.example/priv.acl",
        &format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>; acl:accessTo <https://pod.example/priv>; acl:mode acl:Read, acl:Write, acl:Control."#
        ),
    )
    .await;

    let (addr, c, handle) = serve(build_real_router(store, &issuer_key)).await;
    let k = 16;
    let responses =
        drive_authenticated(addr, "GET", "/priv", None, &issuer_key, &client_key, k).await;
    handle.abort();

    for (status, body) in &responses {
        assert_eq!(
            *status, 200,
            "authenticated owner GET of a private doc must be 200 (full auth→WAC→handler)"
        );
        assert_eq!(
            body.as_slice(),
            PRIV_DOC.as_bytes(),
            "authed body must be byte-identical"
        );
    }
    let writes = c.writes.load(Ordering::SeqCst);
    let writevs = c.writevs.load(Ordering::SeqCst);
    assert_eq!(
        writes, 0,
        "no plain write() on the authed read path (got {writes})"
    );
    assert_eq!(
        writevs, k,
        "one writev per authed response (got {writevs} over {k})"
    );
}
