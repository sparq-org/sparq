//! sq-ushvx (epic sq-my8wd) — in-process SERVICE-federation test harness.
//!
//! This is the **federation keystone** for the SERVICE / HTTP-protocol conformance
//! program (design record `research/service-federation-conformance.md`, Option C): a
//! reusable fixture that stands up a **real** [`sparq_server::serve`] endpoint over a
//! fixture graph on an **ephemeral** `127.0.0.1:0` loopback port, returns the bound URL,
//! and tears the server down cleanly on drop. A federated query is then driven through
//! the engine's **real** `ureq` HTTP transport ([`sparq_engine::query`] under the
//! `service` feature), so the *whole* federated path — HTTP request/response, `Accept`
//! content negotiation, SPARQL-Results-JSON/XML parsing, the bind-join over the wire — is
//! exercised end-to-end, not a `pub(crate)` canned-mock seam.
//!
//! Later beads under sq-my8wd wire the W3C `sparql11/service` evaluation suite and the
//! HTTP-protocol suites onto this harness; this bead delivers only the reusable fixture
//! plus a smoke test, and deliberately does **not** graduate a conformance floor yet.
//!
//! ## SSRF egress scoping (load-bearing invariant)
//!
//! A loopback address (`127.0.0.0/8`) is refused by the engine's default-deny SSRF egress
//! filter ([`sparq_engine::with_service_egress_allow`]). The harness opts the loopback
//! host back in for the **duration of the single federated query only** (a scoped guard,
//! restored on return/unwind) — it does **NOT** disable the egress filter globally. The
//! DNS-rebinding-safe resolver stays fully installed and re-vets every resolved IP, so the
//! invariant the engine guarantees (vetted-IP == dialled-IP, no resolve/re-resolve TOCTOU)
//! is preserved. See [`LoopbackEndpoint::run_federated`].
//!
//! **Honest boundary:** the engine's egress allowlist is keyed by *bare host* (it has no
//! port granularity — `sparq_engine::service::egress_policy` matches the SERVICE IRI
//! authority host, not `host:port`). The harness therefore allowlists exactly the bound
//! loopback **host** (`127.0.0.1`), which under the default-deny-*private* policy re-opens
//! only that one private host while every other private/internal destination stays
//! refused. Within a single test process this is the finest scoping the engine offers; a
//! genuinely port-scoped engine allowlist (so two concurrent loopback endpoints on
//! different ports are mutually unreachable) is a separately-reviewable engine change to
//! the SSRF primitive and is tracked as a follow-up (see the crate README / PR body). The
//! harness binds each endpoint to a fresh ephemeral port and the smoke test asserts the
//! SERVICE IRI targets exactly that bound port, so the *test* scope is the bound port even
//! though the *engine guard* is host-wide.

use std::net::SocketAddr;
use std::sync::mpsc;
use std::thread::JoinHandle;

use sparq_core::Graph;
use sparq_server::{router, serve, AppState};

/// A real in-process [`sparq_server`] SPARQL endpoint bound to an ephemeral loopback port,
/// serving a fixture [`Graph`]. Construct with [`LoopbackEndpoint::serve`]; the bound base
/// URL is [`url`](Self::url) and the SPARQL Protocol endpoint is [`sparql_url`](Self::sparql_url).
///
/// The server runs on its own background thread with a dedicated multi-thread tokio runtime.
/// Dropping the `LoopbackEndpoint` signals a graceful shutdown and joins the thread, so the
/// OS port is released and no task is leaked between tests (RAII teardown).
pub struct LoopbackEndpoint {
    addr: SocketAddr,
    /// Dropping this `Sender` resolves the server's shutdown future (a graceful drain).
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    /// The background runtime thread; joined on drop after shutdown is signalled.
    handle: Option<JoinHandle<()>>,
}

impl LoopbackEndpoint {
    /// Stand up a real `sparq-server` over `graph` on a fresh `127.0.0.1:0` ephemeral port.
    ///
    /// Returns once the listener is bound (the bound address is known), with the accept
    /// loop running on a background thread. Panics only if the loopback bind itself fails
    /// (which on a healthy host it does not — `127.0.0.1:0` always has a free port), so the
    /// fixture stays ergonomic in `#[test]` code.
    pub fn serve(graph: Graph) -> Self {
        // The default loopback server uses the stock `AppState` (no descriptor / opt-in
        // surfaces). sq-1uuxz: the SD/GSP lane needs the `federation-descriptors`
        // runtime flag turned on, so it builds its own `AppState` via `serve_with`.
        Self::serve_with(move || AppState::new(graph))
    }

    /// sq-1uuxz: stand up the loopback server over a CALLER-BUILT [`AppState`], so a
    /// lane can opt into a server config the stock [`serve`](Self::serve) does not (e.g. the
    /// `federation-descriptors` runtime flag the Service-Description lane requires). `make_app`
    /// is run ON the server thread (it constructs the `AppState` — which holds the `Graph` — there,
    /// so the closure need only be `Send`). All bind / teardown / SSRF scoping is identical to
    /// [`serve`](Self::serve); this is the shared seam both constructors route through.
    pub fn serve_with(make_app: impl FnOnce() -> AppState + Send + 'static) -> Self {
        let (addr_tx, addr_rx) = mpsc::channel::<SocketAddr>();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let handle = std::thread::Builder::new()
            .name("sparq-conformance-loopback".into())
            .spawn(move || {
                // A small dedicated runtime: the harness serves test-sized fixtures, so a
                // couple of worker threads are ample and keep the footprint low.
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("build loopback tokio runtime");
                rt.block_on(async move {
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                        .await
                        .expect("bind 127.0.0.1:0 loopback");
                    let addr = listener.local_addr().expect("listener local_addr");
                    // Hand the bound address back to the constructor BEFORE entering the
                    // accept loop, so `serve` returns only once the URL is known.
                    addr_tx.send(addr).expect("report bound loopback addr");
                    let app = router(make_app());
                    // The shutdown future resolves when the `LoopbackEndpoint` is dropped
                    // (the paired `Sender` is dropped, which makes the receiver resolve).
                    // `serve` then stops accepting and drains in-flight connections.
                    let shutdown = async move {
                        // Either an explicit signal or the Sender being dropped resolves
                        // this; both mean "tear down". The Result is intentionally ignored.
                        let _ = shutdown_rx.await;
                    };
                    // No slow-client deadlines on the loopback test endpoint (None/None):
                    // the fixtures are tiny and trusted, so the slow-loris / slow-body
                    // guards add nothing here. Errors are surfaced via the thread panicking
                    // (a serve failure is a test-infra failure, not a silent skip).
                    serve(listener, app, None, None, shutdown)
                        .await
                        .expect("loopback serve loop");
                });
            })
            .expect("spawn loopback server thread");

        let addr = addr_rx
            .recv()
            .expect("loopback server reported its bound address");
        LoopbackEndpoint {
            addr,
            shutdown: Some(shutdown_tx),
            handle: Some(handle),
        }
    }

    /// The bound `SocketAddr` (always `127.0.0.1:<ephemeral-port>`).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The bound loopback *host* (`127.0.0.1`) — the bare-host allowlist key for the
    /// engine's SSRF egress policy. See the module-level SSRF note for why this is
    /// host-level (not port-level) and what that means.
    pub fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    /// The base URL the server is bound to, e.g. `http://127.0.0.1:54321`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The SPARQL Protocol query endpoint URL, e.g. `http://127.0.0.1:54321/sparql` — the
    /// IRI to put inside a `SERVICE <...>` clause.
    pub fn sparql_url(&self) -> String {
        format!("{}/sparql", self.url())
    }

    /// Run `query` over the `local` graph through the engine's **real** `ureq` HTTP
    /// transport, with the egress allowlist scoped (for this call only) to this endpoint's
    /// loopback host so a `SERVICE <{sparql_url}>` clause can reach it.
    ///
    /// The egress filter is **not** globally disabled: only this endpoint's loopback host
    /// is added to the default-deny-private allowlist for the lifetime of the call (the
    /// scope is restored on return and on unwind). See the module-level SSRF note.
    pub fn run_federated(
        &self,
        local: &Graph,
        query: &str,
    ) -> Result<sparq_engine::QueryResult, String> {
        sparq_engine::with_service_egress_allow([self.host()], || sparq_engine::query(local, query))
    }
}

impl std::fmt::Debug for LoopbackEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopbackEndpoint")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl Drop for LoopbackEndpoint {
    fn drop(&mut self) {
        // Signal a graceful shutdown (or just drop the Sender — either resolves the
        // server's shutdown future), then join the runtime thread so the port is released
        // and the runtime is fully wound down before the next test binds an endpoint.
        if let Some(tx) = self.shutdown.take() {
            // Ignore the error: if the receiver was already dropped (the serve loop ended),
            // there is nothing left to signal.
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            // A panic inside the server thread would surface here; in Drop we cannot
            // propagate it, so we ignore the join result (the panic already aborted that
            // thread and any in-flight test assertion will have failed independently).
            let _ = handle.join();
        }
    }
}
