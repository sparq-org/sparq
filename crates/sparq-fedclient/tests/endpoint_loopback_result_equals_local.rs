//! END-TO-END loopback test (bead sq-my8wd.2, epic sq-my8wd): drive the `sparq-fedclient`
//! `Endpoint` SRJ adapter + `discovery` against a **REAL** `sparq_server::serve` endpoint on
//! an ephemeral `127.0.0.1:0` loopback port — over the crate's own native `ureq` HTTP
//! transport — and assert the federated result **EQUALS** local `sparq-engine` evaluation of
//! the same query over the same fixture graph.
//!
//! ## Why this test exists (the honest gap it closes)
//!
//! Every other result-equals-local test in this crate
//! (`planner_result_equals_local_eval.rs`, `multi_source_union_result_equals_local.rs`, …)
//! answers each leaf sub-query through an **in-memory** [`Transport`] double
//! (`EngineTransport`) that runs the sub-query on an in-process `Graph` — so the SRJ
//! request/response, the ureq HTTP round-trip, and the SSRF egress guard on the wire are
//! never exercised. This test stands up a real server and drives the WHOLE native path:
//! the [`Endpoint`] adapter's [`HttpTransport`] POSTs each leaf sub-query to the loopback
//! `/sparql` endpoint, the server evaluates it and serialises SPARQL-Results-JSON, and the
//! Phase-3 interpreter ([`materialize_single_source`]) parses it back and natural-joins the
//! leaves — mirroring the conformance keystone (`sparq-conformance/src/service_loopback.rs`)
//! but for the fedclient CLIENT path (its own transport + guard), not the engine SERVICE path.
//!
//! ## The load-bearing SSRF egress invariant (this is why the bead is Opus-tier)
//!
//! `127.0.0.0/8` is a private/internal range the default-deny SSRF guard REFUSES. The
//! fedclient egress guard is **not** a process-global thread-local that a call widens and
//! restores; it is an **owned** [`EgressGuard`](sparq_fedclient::EgressGuard) confined to the
//! [`Endpoint`] (and, for discovery, the `HttpFetcher`) instance that holds it — so the scope
//! IS the guard's ownership (RAII), and it is impossible to *globally* widen the allowlist.
//! The bare loopback host is opted back in ONLY on the guard of the one endpoint that must
//! reach the server; nothing else is re-opened, and when that endpoint drops the allowlist
//! goes with it (there is no residual state to restore).
//!
//! We make that non-vacuous: against the SAME live server, a sibling **default-deny**
//! `Endpoint` (empty allowlist) is REFUSED with [`FedError::EgressRefused`], while the
//! allowlisted one reaches the server and returns the correct data. The refusal therefore
//! cannot be a "server is down" artifact — the allowlisted sibling proves the server is up —
//! it is specifically the guard refusing. If the guard's protection were removed (the bare
//! host allowlisted), the refusal assertion would flip to a success and this test would go
//! RED; that is the non-vacuity proof. See the PR body for the mechanical flip-to-red run.
//!
//! **Honest boundary (what THIS test keys on).** The fedclient `EgressGuard` ALREADY supports
//! port-scoped `host:port` allowlist entries — `allow_host("127.0.0.1:PORT")` is honoured by the
//! port-aware `is_allowed_port` / `check_endpoint`, which share the engine SERVICE guard's
//! per-entry rule (`sparq_engine::allowlist_entry_permits`); this landed in bead sq-vbnyc
//! (PR #1311). So port-scoping is present in the guard, not missing. This test simply does NOT
//! exercise it: it keys its allowlist by **bare host** (`127.0.0.1`) — a host-level entry that
//! re-opens that host on every port — because a bare-host entry is sufficient here and mirrors the
//! conformance keystone. The still-open bead sq-xemcd is a SEPARATE surface (the `sparq-engine`
//! SERVICE-egress allowlist at the engine layer), NOT this fedclient guard's port scoping. Each
//! loopback endpoint binds a fresh ephemeral port and the assertions target exactly that port.
//!
//! Gated on the `fedclient` feature; with the feature off the whole file compiles to nothing,
//! so the default build (and the `--workspace` byte/wasm ratchets) are unaffected.
//!
//! sq-my8wd.2 — flagged for Fable re-review when available.

#![cfg(feature = "fedclient")]

use std::net::SocketAddr;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use sparq_core::Graph;
use sparq_engine::query;
// `discover` / `Provenance` / `HttpFetcher` are the `discovery` module's surface (not
// re-exported at the crate root); everything else is at the crate root.
use sparq_fedclient::discovery::{discover, HttpFetcher, Provenance};
use sparq_fedclient::{
    materialize_single_source, solutions_equal, EgressGuard, Endpoint, FedError, FederatedSource,
    Relation, SourceResolver, SubQuery,
};
use sparq_fedplan::{
    plan_bgp, select_sources, Bgp, PlanOptions, PredPartition, SourceDescriptor, SourceId, Term,
    TriplePattern, Var,
};
use sparq_server::{router, serve, AppState};

// ─── A real in-process sparq-server loopback endpoint (RAII teardown) ────────────────
//
// Mirrors `sparq-conformance/src/service_loopback.rs` but is self-contained in this test
// file (the bead scopes this work to ONE new tests/ integration file, no src changes). The
// server runs on its own background thread with a dedicated multi-thread tokio runtime;
// dropping the handle signals a graceful shutdown and joins the thread so the OS port is
// released between tests. sq-my8wd.2.

/// A real `sparq-server` SPARQL endpoint bound to an ephemeral loopback port, serving a
/// fixture [`Graph`].
struct Loopback {
    addr: SocketAddr,
    /// Dropping this `Sender` resolves the server's shutdown future (a graceful drain).
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    /// The background runtime thread; joined on drop after shutdown is signalled.
    handle: Option<JoinHandle<()>>,
}

impl Loopback {
    /// Stand up a real `sparq-server` over `graph` on a fresh `127.0.0.1:0` ephemeral port.
    /// Returns once the listener is bound (the bound address is known).
    fn start(graph: Graph) -> Loopback {
        let (addr_tx, addr_rx) = mpsc::channel::<SocketAddr>();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let handle = std::thread::Builder::new()
            .name("sparq-fedclient-loopback".into())
            .spawn(move || {
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
                    // Hand the bound address back BEFORE entering the accept loop, so `start`
                    // returns only once the URL is known.
                    addr_tx.send(addr).expect("report bound loopback addr");
                    let app = router(AppState::new(graph));
                    let shutdown = async move {
                        // Either an explicit signal or the Sender being dropped resolves this.
                        let _ = shutdown_rx.await;
                    };
                    // Tiny, trusted fixtures ⇒ no slow-client deadlines (None/None). A serve
                    // failure panics the thread (a test-infra failure, never a silent skip).
                    serve(listener, app, None, None, shutdown)
                        .await
                        .expect("loopback serve loop");
                });
            })
            .expect("spawn loopback server thread");

        // Bound with a hard deadline so a DEAD server thread fails the test FAST instead of
        // hanging the whole binary (and CI): if the thread panics or exits before it reports its
        // bound `127.0.0.1:0` address, a plain `recv()` would block forever. A few seconds is ample
        // for an in-process ephemeral-port bind; a timeout or a dropped sender both mean the server
        // thread died before binding, so we panic with a clear message (the thread's own panic is
        // already on stderr) rather than deadlocking. sq-my8wd.2.
        let addr = match addr_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(addr) => addr,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!(
                "loopback server thread did not report its bound address within 10s — it likely \
                 panicked or hung before binding 127.0.0.1:0 (see its panic on stderr)"
            ),
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!(
                "loopback server thread exited before reporting its bound address — its bind/serve \
                 setup panicked before send (see its panic on stderr)"
            ),
        };
        Loopback {
            addr,
            shutdown: Some(shutdown_tx),
            handle: Some(handle),
        }
    }

    /// The bound loopback *host* (`127.0.0.1`) — the bare-host allowlist key.
    fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    /// The SPARQL Protocol query endpoint URL, e.g. `http://127.0.0.1:54321/sparql`.
    fn sparql_url(&self) -> String {
        format!("http://{}/sparql", self.addr)
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// ─── Fixture + fedplan helpers (mirror planner_result_equals_local_eval.rs) ──────────

const DATA: &str = r#"
@prefix ex: <http://ex/> .
ex:alice ex:knows ex:bob .
ex:alice ex:knows ex:carol .
ex:bob   ex:knows ex:dave .
ex:carol ex:knows ex:dave .
ex:alice ex:name "Alice" .
ex:bob   ex:name "Bob" .
ex:carol ex:name "Carol" .
ex:dave  ex:name "Dave" .
ex:alice ex:age "30"^^<http://www.w3.org/2001/XMLSchema#integer> .
ex:bob   ex:age "25"^^<http://www.w3.org/2001/XMLSchema#integer> .
"#;

fn fixture() -> Graph {
    Graph::load_str(DATA, "turtle").unwrap()
}

fn iri(s: &str) -> Term {
    Term::Iri(s.to_string())
}
fn var(s: &str) -> Term {
    Term::Var(Var::new(s))
}

/// A one-source descriptor declaring each predicate in `preds` so `select_sources` retains
/// the single source for every pattern. The stats only steer join *order*; result-equivalence
/// holds for any order. sq-my8wd.2.
fn descriptor(preds: &[&str]) -> SourceDescriptor {
    let mut b = SourceDescriptor::builder(SourceId::new("loopback")).total_triples(1000);
    for p in preds {
        b = b.predicate(PredPartition {
            predicate: (*p).into(),
            triples: 100,
            distinct_subjects: 50,
            distinct_objects: 50,
        });
    }
    b.build()
}

/// Plan `bgp` over one source, drive it through the REAL loopback `Endpoint` (native ureq
/// transport, allowlisting exactly the bound loopback host), and assert the materialised
/// federated result's solution multiset equals `sparq_engine::query(&graph, whole_query)`.
///
/// The server holds the SAME fixture `graph`, so each leaf sub-query the adapter POSTs is
/// answered exactly as local evaluation would — the canonical answer IS the engine. The
/// `Endpoint` is built with an [`EgressGuard`] that allowlists ONLY the bare loopback host,
/// so its native `HttpTransport` resolver re-opens that one private host (and nothing else).
fn assert_federated_over_http_equals_local(
    server: &Loopback,
    graph: &Graph,
    bgp: &Bgp,
    descriptor: &SourceDescriptor,
    whole_query: &str,
) {
    let descriptors = [descriptor.clone()];
    let sel = select_sources(bgp, &descriptors);
    let tree = plan_bgp(bgp, &sel, &descriptors, &PlanOptions::default())
        .expect("non-empty BGP yields a plan");

    // The allowlisted endpoint: scoped to exactly the bound loopback host, over the crate's
    // real native IP-pinning HTTP transport (guard-vetted IP == dialled IP, no re-resolve).
    let guard = EgressGuard::deny_private().allow_host(server.host());
    let ep = Endpoint::with_guard_native(server.sparql_url(), guard);
    let adapters: Vec<&dyn FederatedSource> = vec![&ep];
    let resolver = SourceResolver::new(bgp, &adapters);

    let fed: Relation =
        materialize_single_source(&resolver, &sel, &ep, &tree).expect("interpreter succeeds");

    let local = query(graph, whole_query).expect("local eval succeeds");
    let local_vars: Vec<String> = local.vars.iter().map(|v| v.as_str().to_string()).collect();

    assert!(
        solutions_equal(&fed.vars, &fed.rows, &local_vars, &local.rows),
        "federated (over real HTTP) result must equal local eval.\n  federated = {fed:?}\n  \
         local vars = {local_vars:?}, rows = {:?}",
        local.rows,
    );
}

// ─── Result-equivalence over the REAL HTTP path ──────────────────────────────────────

#[test]
fn single_pattern_over_http_equals_local() {
    let graph = fixture();
    let server = Loopback::start(fixture());
    // ?s ex:knows ?o — one leaf, no join.
    let bgp = Bgp::new(vec![TriplePattern::new(
        var("s"),
        iri("http://ex/knows"),
        var("o"),
    )]);
    assert_federated_over_http_equals_local(
        &server,
        &graph,
        &bgp,
        &descriptor(&["http://ex/knows"]),
        "SELECT ?s ?o WHERE { ?s <http://ex/knows> ?o }",
    );
}

#[test]
fn star_join_over_http_equals_local() {
    let graph = fixture();
    let server = Loopback::start(fixture());
    // ?s ex:knows ?o . ?s ex:name ?n — a star on ?s (a real over-the-wire bind/hash join).
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/knows"), var("o")),
        TriplePattern::new(var("s"), iri("http://ex/name"), var("n")),
    ]);
    assert_federated_over_http_equals_local(
        &server,
        &graph,
        &bgp,
        &descriptor(&["http://ex/knows", "http://ex/name"]),
        "SELECT ?s ?o ?n WHERE { ?s <http://ex/knows> ?o . ?s <http://ex/name> ?n }",
    );
}

#[test]
fn path_join_over_http_equals_local() {
    let graph = fixture();
    let server = Loopback::start(fixture());
    // ?s ex:knows ?o . ?o ex:knows ?z . ?z ex:name ?n — a 2-hop path + name lookup.
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/knows"), var("o")),
        TriplePattern::new(var("o"), iri("http://ex/knows"), var("z")),
        TriplePattern::new(var("z"), iri("http://ex/name"), var("n")),
    ]);
    assert_federated_over_http_equals_local(
        &server,
        &graph,
        &bgp,
        &descriptor(&["http://ex/knows", "http://ex/name"]),
        "SELECT ?s ?o ?z ?n WHERE { \
            ?s <http://ex/knows> ?o . \
            ?o <http://ex/knows> ?z . \
            ?z <http://ex/name> ?n }",
    );
}

#[test]
fn typed_literal_over_http_equals_local() {
    let graph = fixture();
    let server = Loopback::start(fixture());
    // ?s ex:age ?a . ?s ex:name ?n — a typed (xsd:integer) literal must SRJ-round-trip over
    // the wire and reproduce the engine's term exactly.
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/age"), var("a")),
        TriplePattern::new(var("s"), iri("http://ex/name"), var("n")),
    ]);
    assert_federated_over_http_equals_local(
        &server,
        &graph,
        &bgp,
        &descriptor(&["http://ex/age", "http://ex/name"]),
        "SELECT ?s ?a ?n WHERE { ?s <http://ex/age> ?a . ?s <http://ex/name> ?n }",
    );
}

#[test]
fn empty_result_over_http_equals_local() {
    let graph = fixture();
    let server = Loopback::start(fixture());
    // A bound object that never occurs ⇒ empty result; the federated path must also be empty
    // (not error, not a spurious row) even over the real transport.
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/knows"), iri("http://ex/nobody")),
        TriplePattern::new(var("s"), iri("http://ex/name"), var("n")),
    ]);
    assert_federated_over_http_equals_local(
        &server,
        &graph,
        &bgp,
        &descriptor(&["http://ex/knows", "http://ex/name"]),
        "SELECT ?s ?n WHERE { ?s <http://ex/knows> <http://ex/nobody> . ?s <http://ex/name> ?n }",
    );
}

// ─── discovery.rs over the REAL HTTP path ────────────────────────────────────────────

#[test]
fn discovery_over_http_ask_probe_succeeds_when_allowlisted() {
    let server = Loopback::start(fixture());
    // The loopback server publishes no Service Description / VoID (the `server` feature only,
    // no `federation-descriptors`), so `discover` falls through to the FedX-style ASK probe:
    // `GET /sparql?query=ASK{?s ?p ?o}` returns an SRJ boolean ⇒ a live, reachable endpoint.
    // The fetcher allowlists exactly the bare loopback host so its SSRF resolver re-opens it.
    let fetcher = HttpFetcher::new().allow_private_host(server.host());
    let disc = discover(&server.sparql_url(), &fetcher)
        .expect("discovery reaches the allowlisted loopback endpoint");
    assert_eq!(
        disc.provenance,
        Provenance::AskProbe,
        "no SD/VoID published ⇒ ASK-probe provenance"
    );
    assert!(
        disc.capability.returns_sparql_results_json(),
        "an ASK-reachable SPARQL endpoint advertises the SRJ result format"
    );
}

// ─── THE load-bearing SSRF egress-refusal assertions (non-vacuous) ───────────────────

#[test]
fn egress_refused_outside_guard_scope_source_adapter() {
    let server = Loopback::start(fixture());
    let url = server.sparql_url();
    let sub = SubQuery::new("SELECT ?s ?o WHERE { ?s <http://ex/knows> ?o }");

    // (A) CONTROL — the server IS up and reachable: an `Endpoint` whose guard allowlists the
    // bare loopback host reaches it and returns a non-empty SRJ body. This makes (B) below a
    // guard refusal, NOT a "server is down" artifact.
    let allowed = Endpoint::with_guard_native(
        url.clone(),
        EgressGuard::deny_private().allow_host(server.host()),
    );
    let body = allowed
        .execute(&sub)
        .expect("allowlisted endpoint reaches the live loopback server");
    assert!(
        body.contains("\"bindings\""),
        "the live server returned a SPARQL-Results-JSON body: {body}"
    );

    // (B) THE ASSERTION — OUTSIDE the guard scope: a sibling default-deny `Endpoint`
    // (empty allowlist) against the SAME live loopback host is REFUSED before any socket is
    // opened. The fedclient guard is owned per-endpoint (no process-global to widen), so the
    // allowlist granted to `allowed` above does NOT leak to `denied` here.
    let denied = Endpoint::native(url.clone());
    let err = denied
        .execute(&sub)
        .expect_err("default-deny endpoint must REFUSE the loopback (127.0.0.0/8) host");
    assert!(
        matches!(err, FedError::EgressRefused(_)),
        "the refusal must be an egress denial, got: {err:?}"
    );

    // NON-VACUITY: this assertion fails-closed only because the guard refuses. If the guard's
    // protection were removed (i.e. `denied` also allowlisted `server.host()`), `execute`
    // would succeed and `expect_err` above would PANIC — the test would go RED. The PR body
    // records the mechanical flip-to-red run that proves this is not vacuous.
    //
    // NO GLOBAL WIDEN / RESTORED-ON-UNWIND: `allowed` has already run its request (its guard
    // scope entered and left). A freshly-built default-deny endpoint against the same host is
    // STILL refused, so no residual widening survived the allowlisted call.
}

#[test]
fn egress_refused_outside_guard_scope_discovery() {
    let server = Loopback::start(fixture());
    let url = server.sparql_url();

    // CONTROL: an allowlisted fetcher discovers the live endpoint (proves it is up).
    let allowed = HttpFetcher::new().allow_private_host(server.host());
    discover(&url, &allowed).expect("allowlisted discovery reaches the live loopback endpoint");

    // THE ASSERTION: a default-deny `HttpFetcher` (no `allow_private_host`) cannot reach the
    // loopback host — every guarded GET (SD, VoID, ASK probe) is egress-refused, so `discover`
    // returns an error. Same live server; the ONLY difference is the SSRF allowlist.
    let denied = HttpFetcher::new();
    let err = discover(&url, &denied)
        .expect_err("default-deny discovery must REFUSE the loopback (127.0.0.0/8) host");
    // EGRESS-DENIAL-SPECIFIC (not vacuous): `discover`'s failure path ALWAYS wraps the cause in
    // "…did not answer an ASK probe…", so matching "ASK probe" would pass for ANY discovery
    // failure (a connection error, a parse error) — it would NOT prove the SSRF guard refused. The
    // guard's refusal is the ONLY `discover` error carrying the resolver's
    // "discovery egress refused: … resolves only to private/internal addresses …" text, so we
    // require BOTH distinguishing fragments. Non-vacuity is nailed by the CONTROL above: the SAME
    // discovery against the SAME live server SUCCEEDS once the host is allowlisted, so the only
    // thing that turned success into failure here is the egress scope — this is the guard
    // refusing, not a "server is down" / unrelated-failure artifact. sq-my8wd.2.
    assert!(
        err.contains("egress refused") && err.contains("private/internal"),
        "discovery must fail-closed with an EGRESS-DENIAL error specifically (not a generic \
         unreachability that merely mentions the ASK probe), got: {err}"
    );
}
