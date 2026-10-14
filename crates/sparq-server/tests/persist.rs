//! (sq-7cxr, gh-44) Integration tests for DURABLE PERSISTENCE — the QLever
//! `--persist-updates` equivalent wired onto the engine's directory-backed `Graph` + WAL.
//!
//! The acceptance criterion (gh-44): **a process RESTART preserves ALL updates with NO
//! rebuild.** We prove it the way an operator would: start a server with `--persist <dir>`
//! (modelled by `ServerConfig::persist_dir`), apply several UPDATEs — default graph AND a
//! named graph — over HTTP, SHUT THE SERVER DOWN (drop its state so the writer thread joins
//! and every WAL handle is released), start a BRAND-NEW server on the SAME `<dir>`, and assert
//! every triple is present via query — with no explicit rebuild step. We also assert the
//! back-compat contract: with NO persist dir the server is in-memory, so a "restart" (a fresh
//! in-memory server) does NOT see the prior updates.
//!
//! Durability point: each `post_update` 204 is the group-commit ack, which the writer only
//! sends AFTER `ServerApplier::seal` has WAL-fsync'd the batch to the durable graph (see
//! `src/http.rs`). So by the time a 204 returns, the update is already on disk — the restart
//! merely re-opens it.
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

/// A unique scratch directory under the system temp dir, removed on drop so the test leaves
/// no persist-dir cruft behind (repo hygiene + the `df` watchdog). No `tempfile` dep in the
/// workspace, so this mirrors the existing `std::env::temp_dir()` + manual-cleanup pattern.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "sparq-persist-test-{}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        ScratchDir(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A running server we can SHUT DOWN: holds the graceful-shutdown trigger + the serve task's
/// join handle, so a test can drop the whole thing and be sure the writer thread joined (which
/// happens when the last `AppState`/`Writer` Arc drops) before reopening the same persist dir.
struct Server {
    base: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Server {
    /// Boots a server on an ephemeral port with the given config, returning a handle whose
    /// `base` is the URL. The serve future resolves on the shutdown signal, so `stop()` truly
    /// tears the server (and its durable writer) down.
    async fn start(config: ServerConfig) -> Self {
        // Empty seed; an existing persist dir is opened (seed ignored), an empty one is created.
        let graph = Graph::load_str("", "turtle").unwrap();
        let state = AppState::try_with_config(graph, config).expect("durable open");
        let app = router(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await
                .unwrap();
        });
        Server { base: format!("http://{addr}"), shutdown: Some(tx), task: Some(task) }
    }

    /// Signals graceful shutdown and waits for the serve task to finish, so the `AppState`
    /// (and the writer thread it owns) is dropped — modelling a clean process restart.
    ///
    /// (Copilot PR#80) We `expect()` the join result rather than discarding it: if
    /// the serve task PANICKED (e.g. the durable writer thread fail-closed and the panic
    /// propagated, or `axum::serve(...).await.unwrap()` hit a serve error), `task.await` returns
    /// `Err(JoinError)`, and swallowing it would let the test pass while masking a real failure.
    async fn stop(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            task.await.expect("server serve task panicked / failed");
        }
    }
}

async fn post_update(cl: &reqwest::Client, base: &str, update: &str) -> reqwest::StatusCode {
    cl.post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body(update.to_string())
        .send()
        .await
        .unwrap()
        .status()
}

/// Number of solutions to `query` (counts JSON result bindings).
async fn count_rows(cl: &reqwest::Client, base: &str, query: &str) -> usize {
    let body: serde_json::Value = cl
        .get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        .query(&[("query", query)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["results"]["bindings"].as_array().unwrap().len()
}

/// (sq-ycle) The boolean answer to an `ASK` query (the SPARQL JSON `boolean` field).
async fn ask(cl: &reqwest::Client, base: &str, query: &str) -> bool {
    let body: serde_json::Value = cl
        .get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        .query(&[("query", query)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["boolean"].as_bool().unwrap()
}

/// The `value` field of the single binding of variable `var` for a one-row `query` (asserts
/// exactly one row). Used to read back a non-deterministically-generated literal.
async fn single_value(cl: &reqwest::Client, base: &str, query: &str, var: &str) -> String {
    let body: serde_json::Value = cl
        .get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        .query(&[("query", query)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let bindings = body["results"]["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), 1, "expected exactly one binding for ?{var}");
    bindings[0][var]["value"].as_str().unwrap().to_string()
}

fn persist_config(dir: &Path) -> ServerConfig {
    ServerConfig { persist_dir: Some(dir.to_path_buf()), ..ServerConfig::default() }
}

// ---------------------------------------------------------------------------
// The acceptance test: restart preserves ALL updates (default + named) with NO rebuild.
// ---------------------------------------------------------------------------

/// gh-44 ACCEPTANCE. Apply default-graph and named-graph updates to a `--persist` server,
/// shut it down, start a NEW server on the SAME dir, and assert every triple is present —
/// no explicit rebuild. This is the showstopper the PSS migration was blocked on.
#[tokio::test]
async fn restart_preserves_all_updates_no_rebuild() {
    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();

    // --- Process 1: write into the default graph AND a named graph, then DELETE one. ---
    let s1 = Server::start(persist_config(scratch.path())).await;
    for i in 0..5 {
        let ins = format!("INSERT DATA {{ <http://ex/d{i}> <http://ex/p> <http://ex/v> }}");
        assert_eq!(post_update(&cl, &s1.base, &ins).await, 204, "default insert {i}");
    }
    // A GRAPH-scoped INSERT that first creates a brand-new named graph (the durability gap
    // this work closed: the new named graph must be born directory-backed / WAL'd).
    for i in 0..3 {
        let ins = format!(
            "INSERT DATA {{ GRAPH <http://ex/g1> {{ <http://ex/n{i}> <http://ex/q> <http://ex/w> }} }}"
        );
        assert_eq!(post_update(&cl, &s1.base, &ins).await, 204, "named insert {i}");
    }
    // Delete one default-graph triple — the retraction must persist too (not resurrect on reopen).
    assert_eq!(
        post_update(&cl, &s1.base, "DELETE DATA { <http://ex/d0> <http://ex/p> <http://ex/v> }").await,
        204
    );

    // Sanity in-process before restart: 4 default + 3 named.
    assert_eq!(count_rows(&cl, &s1.base, "SELECT * WHERE { ?s <http://ex/p> ?o }").await, 4);
    assert_eq!(
        count_rows(&cl, &s1.base, "SELECT * WHERE { GRAPH <http://ex/g1> { ?s ?p ?o } }").await,
        3
    );

    // --- RESTART: tear the first server fully down, boot a fresh one on the SAME dir. ---
    s1.stop().await;
    let s2 = Server::start(persist_config(scratch.path())).await;

    // No rebuild step ran — `Graph::open` replayed the WAL. Everything must be present.
    assert_eq!(
        count_rows(&cl, &s2.base, "SELECT * WHERE { ?s <http://ex/p> ?o }").await,
        4,
        "default-graph inserts (minus the one delete) must survive the restart"
    );
    // The deleted triple must STAY deleted across the restart (the retraction was WAL'd).
    assert_eq!(
        count_rows(&cl, &s2.base, "SELECT * WHERE { <http://ex/d0> <http://ex/p> ?o }").await,
        0,
        "the deletion must persist (not resurrect on reopen)"
    );
    // Named-graph triples must survive too — the showstopper-within-the-showstopper.
    assert_eq!(
        count_rows(&cl, &s2.base, "SELECT * WHERE { GRAPH <http://ex/g1> { ?s ?p ?o } }").await,
        3,
        "named-graph inserts must survive the restart"
    );

    // And the reopened server is still WRITABLE and durable: one more update, restart again.
    assert_eq!(
        post_update(&cl, &s2.base, "INSERT DATA { <http://ex/d9> <http://ex/p> <http://ex/v> }").await,
        204
    );
    s2.stop().await;
    let s3 = Server::start(persist_config(scratch.path())).await;
    assert_eq!(
        count_rows(&cl, &s3.base, "SELECT * WHERE { ?s <http://ex/p> ?o }").await,
        5,
        "an update applied after the first restart must itself survive a second restart"
    );
    s3.stop().await;
}

// ---------------------------------------------------------------------------
// (Copilot PR#80) Non-deterministic update: the durable graph must persist the
// EXACT value the in-memory side committed and acked — NOT a re-rolled one.
// ---------------------------------------------------------------------------

/// REGRESSION for the durability-divergence finding (Copilot PR#80). A `DELETE/INSERT … WHERE`
/// that binds a non-deterministic value (`STRUUID()`) commits one resolved literal in memory and
/// 204-acks it. The durable mirror must persist *that* literal, not a second, independently-rolled
/// one — otherwise a restart would surface a value the client never saw, breaking "204 ⇒ durable".
///
/// With the old "re-execute the update string against the durable graph" mirror this FAILED: the
/// second execution re-rolled `STRUUID()`, so after a restart the persisted value differed from the
/// value the live server returned. We assert byte-equality of the value before and after the restart.
#[tokio::test]
async fn nondeterministic_update_persists_committed_value_not_a_reroll() {
    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();

    let s1 = Server::start(persist_config(scratch.path())).await;
    // Seed a subject, then tag it with a freshly-generated STRUUID via DELETE/INSERT … WHERE.
    assert_eq!(
        post_update(&cl, &s1.base, "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }").await,
        204
    );
    assert_eq!(
        post_update(
            &cl,
            &s1.base,
            "INSERT { <http://ex/s> <http://ex/tag> ?u } WHERE { <http://ex/s> <http://ex/p> ?o . BIND(STRUUID() AS ?u) }",
        )
        .await,
        204
    );

    // The value the LIVE (in-memory, already-acked) server holds.
    let live = single_value(&cl, &s1.base, "SELECT ?u WHERE { <http://ex/s> <http://ex/tag> ?u }", "u").await;
    assert!(!live.is_empty(), "STRUUID() must have produced a value");

    // RESTART and read the persisted value: it MUST be the identical literal (not a re-roll).
    s1.stop().await;
    let s2 = Server::start(persist_config(scratch.path())).await;
    let persisted =
        single_value(&cl, &s2.base, "SELECT ?u WHERE { <http://ex/s> <http://ex/tag> ?u }", "u").await;
    assert_eq!(
        persisted, live,
        "the persisted STRUUID() value must equal the value the live server acked — \
         the durable mirror must NOT re-execute (and thus re-roll) the update"
    );
    s2.stop().await;
}

// ---------------------------------------------------------------------------
// (sq-ycle, gh-48) PSS combined multi-op update body: accepted as ONE request
// AND committed ATOMICALLY (all-or-nothing) on the durable `--persist` path.
// ---------------------------------------------------------------------------

/// gh-48 (a)+(b). PSS's `putDocument`/`deleteResource` send ONE request body of `;`-separated
/// operations — `DROP SILENT GRAPH <r> ; INSERT DATA { GRAPH <r> … ; GRAPH <parent> ldp:contains
/// <r> }` — that rewrites the resource graph AND the parent's `ldp:contains` containment in a
/// single shot. PSS's reconciler does NOT repair index-internal containment desync, so this body
/// MUST be (a) accepted as one update request and (b) applied atomically: a single 204, with the
/// child graph AND the parent containment triple BOTH present afterwards (never one without the
/// other). This test sends PSS's exact shape, asserts the single 204 + the fully-applied post-state,
/// then RESTARTS the `--persist` server to prove the WHOLE body is durable as one atomic commit
/// (the [`sparq_engine`] txn-journal commit point, sq-ycle) — a crash mid-body can never leave the
/// parent containment pointing at a child graph that did not survive (or vice versa).
#[tokio::test]
async fn pss_combined_multiop_body_accepted_and_atomic() {
    const LDP_CONTAINS: &str = "http://www.w3.org/ns/ldp#contains";
    const R: &str = "http://ex/pod/resource1";
    const PARENT: &str = "http://ex/pod/";

    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();
    let s1 = Server::start(persist_config(scratch.path())).await;

    // The EXACT PSS combined body: DROP the resource graph, then in ONE INSERT DATA write the new
    // resource content into GRAPH <r> AND the parent containment triple into GRAPH <parent>.
    let body = format!(
        "DROP SILENT GRAPH <{R}> ; \
         INSERT DATA {{ \
           GRAPH <{R}> {{ <{R}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/ldp#Resource> . \
                          <{R}> <http://ex/title> \"hello\" }} \
           GRAPH <{PARENT}> {{ <{PARENT}> <{LDP_CONTAINS}> <{R}> }} \
         }}"
    );

    // (a) ACCEPTED: the `;`-separated multi-op body is one update request → a SINGLE 204.
    assert_eq!(post_update(&cl, &s1.base, &body).await, 204, "the combined multi-op body must be accepted as ONE update (single 204)");

    // (b) FULLY APPLIED: both halves of the body are present — the child graph content AND the
    // parent containment triple. (No partial write: containment without the resource, or vice versa.)
    let child = format!("SELECT * WHERE {{ GRAPH <{R}> {{ ?s ?p ?o }} }}");
    let containment = format!("ASK WHERE {{ GRAPH <{PARENT}> {{ <{PARENT}> <{LDP_CONTAINS}> <{R}> }} }}");
    assert_eq!(count_rows(&cl, &s1.base, &child).await, 2, "the child resource graph must be fully written");
    assert!(ask(&cl, &s1.base, &containment).await, "the parent ldp:contains triple must be present");

    // The whole body is ONE atomic durable commit: a restart re-opens both halves together.
    s1.stop().await;
    let s2 = Server::start(persist_config(scratch.path())).await;
    assert_eq!(count_rows(&cl, &s2.base, &child).await, 2, "the child graph must survive the restart (durable)");
    assert!(ask(&cl, &s2.base, &containment).await, "the parent containment must survive the restart, in lockstep with the child graph");

    // A second putDocument-shaped body REPLACES the resource graph atomically and is durable too.
    let body2 = format!(
        "DROP SILENT GRAPH <{R}> ; \
         INSERT DATA {{ \
           GRAPH <{R}> {{ <{R}> <http://ex/title> \"updated\" }} \
           GRAPH <{PARENT}> {{ <{PARENT}> <{LDP_CONTAINS}> <{R}> }} \
         }}"
    );
    assert_eq!(post_update(&cl, &s2.base, &body2).await, 204);
    assert_eq!(count_rows(&cl, &s2.base, &child).await, 1, "the DROP+re-INSERT replaced the resource graph atomically (old content gone)");
    s2.stop().await;
    let s3 = Server::start(persist_config(scratch.path())).await;
    assert_eq!(count_rows(&cl, &s3.base, &child).await, 1, "the replacement survives a second restart");
    assert!(ask(&cl, &s3.base, &containment).await);
    s3.stop().await;
}

/// gh-48 all-or-nothing. A multi-op request whose LATER operation is invalid must leave NO
/// partial write — the request fails (non-2xx) and the valid prefix is NOT committed (not in
/// memory, and — on `--persist` — not on disk after a restart). This is the engine's
/// request-level atomicity: the server writer applies the body to a private fork and seals
/// (and durably commits) ONLY on full success, so a rejected body never publishes or persists
/// a prefix. (A non-SILENT `LOAD` of an unfetchable source is the reliable mid-body failure.)
#[tokio::test]
async fn invalid_second_op_leaves_no_partial_write() {
    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();
    let s1 = Server::start(persist_config(scratch.path())).await;

    // op 1 (valid INSERT DATA) ; op 2 (a non-SILENT LOAD that the engine refuses → request error).
    let failing = "INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/b> } ; LOAD <http://ex/nope.ttl>";
    let status = post_update(&cl, &s1.base, failing).await;
    assert!(!status.is_success(), "a multi-op body with an invalid op must be rejected (non-2xx), got {status}");

    // The valid prefix must NOT have committed in memory (all-or-nothing).
    assert_eq!(
        count_rows(&cl, &s1.base, "SELECT * WHERE { <http://ex/a> ?p ?o }").await,
        0,
        "the valid INSERT DATA prefix must NOT be visible after a rejected multi-op request"
    );

    // …and nothing of it reached the durable store either: a restart shows an empty store.
    s1.stop().await;
    let s2 = Server::start(persist_config(scratch.path())).await;
    assert_eq!(
        count_rows(&cl, &s2.base, "SELECT * WHERE { ?s ?p ?o }").await,
        0,
        "a rejected multi-op request must persist NOTHING (no partial durable write)"
    );
    s2.stop().await;
}

// ---------------------------------------------------------------------------
// Back-compat: no persist dir => in-memory => a fresh server does NOT see prior updates.
// ---------------------------------------------------------------------------

/// Without `--persist`, the server is purely in-memory: a brand-new server (a "restart")
/// starts empty. This pins the historical behaviour so the new flag is the ONLY thing that
/// changes durability — `persist_dir == None` is byte-for-byte the old in-memory server.
#[tokio::test]
async fn no_persist_dir_is_in_memory_and_lost_on_restart() {
    let cl = reqwest::Client::new();

    let s1 = Server::start(ServerConfig::default()).await;
    assert_eq!(
        post_update(&cl, &s1.base, "INSERT DATA { <http://ex/x> <http://ex/p> <http://ex/y> }").await,
        204
    );
    assert_eq!(count_rows(&cl, &s1.base, "SELECT * WHERE { ?s ?p ?o }").await, 1);
    s1.stop().await;

    // A new in-memory server shares nothing with the old one — it starts empty.
    let s2 = Server::start(ServerConfig::default()).await;
    assert_eq!(
        count_rows(&cl, &s2.base, "SELECT * WHERE { ?s ?p ?o }").await,
        0,
        "an in-memory server (no --persist) must NOT see a previous server's updates"
    );
    s2.stop().await;
}

// ---------------------------------------------------------------------------
// A pre-existing on-disk store is the source of truth: the DATA_FILE seed is ignored.
// ---------------------------------------------------------------------------

/// When the persist dir already holds a store, opening it must take that store as-is and
/// ignore the seed graph (QLever's persisted-index-wins semantics). We prove it by seeding a
/// dir via one server's updates, then booting a second server on the same dir with a NON-empty
/// seed graph — the seed must NOT appear; only the persisted data does.
#[tokio::test]
async fn existing_store_wins_over_seed() {
    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();

    // Establish a persisted store with exactly one triple.
    let s1 = Server::start(persist_config(scratch.path())).await;
    assert_eq!(
        post_update(&cl, &s1.base, "INSERT DATA { <http://ex/keep> <http://ex/p> <http://ex/v> }").await,
        204
    );
    s1.stop().await;

    // Reopen with a NON-empty seed graph; the existing store must win (seed ignored).
    let seed = Graph::load_str("<http://ex/seed> <http://ex/p> <http://ex/v> .", "ntriples").unwrap();
    let state = AppState::try_with_config(seed, persist_config(scratch.path())).expect("reopen");
    let app = router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).with_graceful_shutdown(async { let _ = rx.await; }).await.unwrap();
    });
    let base = format!("http://{addr}");

    assert_eq!(
        count_rows(&cl, &base, "SELECT * WHERE { <http://ex/keep> ?p ?o }").await,
        1,
        "the persisted triple must be present"
    );
    assert_eq!(
        count_rows(&cl, &base, "SELECT * WHERE { <http://ex/seed> ?p ?o }").await,
        0,
        "the seed graph must be IGNORED when the persist dir already holds a store"
    );

    let _ = tx.send(());
    // (Copilot PR#80) Fail the test on a panicked/failed serve task rather than
    // discarding the join result (which would silently mask a server-side failure).
    task.await.expect("server serve task panicked / failed");
}

// ---------------------------------------------------------------------------
// (sq-vpx4) GRACEFUL DEGRADATION on a TRANSIENT durable-write I/O error.
//
// Before sq-vpx4 a durable-write failure in `ServerApplier::seal` PANICKED the writer
// thread (fail-closed but server-killing). These tests prove the new contract end-to-end
// over HTTP: a durable-write error REFUSES the in-flight write with HTTP 503 (retryable)
// WITHOUT tearing down the writer / the server, the refused write is NEVER published or
// acked (fail-closed preserved — byte-identical store), reads keep being served from the
// last published snapshot, and a SUBSEQUENT write after recovery succeeds (204) + is
// durable across a restart.
//
// The failure is injected via the `test-seams`-only seam
// (`AppState::with_config_inject_durable_failure`) — production code is unchanged.
// ---------------------------------------------------------------------------

/// Boots a `--persist` server whose seal path fails its first `n_fail` durable commits
/// (then succeeds), modelling a transient `ENOSPC` that clears. `test-seams` only.
#[cfg(feature = "test-seams")]
async fn start_with_transient_failures(config: ServerConfig, n_fail: usize) -> Server {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    let graph = Graph::load_str("", "turtle").unwrap();
    let remaining = Arc::new(AtomicUsize::new(n_fail));
    let hook: Box<dyn FnMut() -> Option<String> + Send> = Box::new(move || {
        if remaining.load(Ordering::SeqCst) > 0 {
            remaining.fetch_sub(1, Ordering::SeqCst);
            Some("injected transient durable-write error (ENOSPC)".to_string())
        } else {
            None
        }
    });
    let state = AppState::with_config_inject_durable_failure(graph, config, hook)
        .expect("durable open with injected failure seam");
    let app = router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await
            .unwrap();
    });
    Server { base: format!("http://{addr}"), shutdown: Some(tx), task: Some(task) }
}

/// A TRANSIENT durable-write error → the in-flight write gets HTTP 503, the writer thread
/// SURVIVES (no panic, server still serving), reads keep working, and a SUBSEQUENT write
/// after recovery succeeds (204) + publishes + is durable across a restart.
#[cfg(feature = "test-seams")]
#[tokio::test]
async fn transient_durable_write_error_503_then_recovers() {
    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();

    // First seal fails once, then durability is healthy.
    let s = start_with_transient_failures(persist_config(scratch.path()), 1).await;

    // The FIRST update hits the injected failure → 503 (the write is refused).
    let refused = post_update(&cl, &s.base, "INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/v> }").await;
    assert_eq!(
        refused, 503,
        "a transient durable-write error must refuse the in-flight write with 503 (retryable)",
    );

    // FAIL-CLOSED: the refused write was NOT published — it must be absent.
    assert_eq!(
        count_rows(&cl, &s.base, "SELECT * WHERE { ?s ?p ?o }").await,
        0,
        "a refused (non-durable) write must NOT be published / queryable",
    );

    // The writer SURVIVED: reads still work (server alive), and the SAME write retried now
    // succeeds (durability recovered) — 204 + queryable.
    let retried = post_update(&cl, &s.base, "INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/v> }").await;
    assert_eq!(retried, 204, "after recovery the retried write must succeed (writer survived)");
    assert_eq!(count_rows(&cl, &s.base, "SELECT * WHERE { ?s ?p ?o }").await, 1);

    // The recovered write is genuinely DURABLE: restart on the same dir and it's still there.
    s.stop().await;
    let s2 = Server::start(persist_config(scratch.path())).await;
    assert_eq!(
        count_rows(&cl, &s2.base, "SELECT * WHERE { ?s ?p ?o }").await,
        1,
        "the post-recovery write must survive a restart (it really was made durable)",
    );
    s2.stop().await;
}

/// A PERSISTENT durable-write error → every write is refused 503 (no panic), the writer
/// stays alive, and reads keep being served from the last published snapshot the whole time
/// (degraded read-only mode).
#[cfg(feature = "test-seams")]
#[tokio::test]
async fn persistent_durable_write_error_repeated_503_reads_survive() {
    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();

    // Jam durability from the start (a large failure budget) and assert the (empty) last
    // published snapshot keeps serving reads under repeated refusals.
    let s = start_with_transient_failures(persist_config(scratch.path()), 1_000).await;

    for n in 0..4 {
        let status = post_update(
            &cl,
            &s.base,
            &format!("INSERT DATA {{ <http://ex/x{n}> <http://ex/p> <http://ex/v> }}"),
        )
        .await;
        assert_eq!(status, 503, "write #{n} under persistent durability failure must be 503");
        // Reads keep being served (the server is alive, degraded read-only).
        assert_eq!(
            count_rows(&cl, &s.base, "SELECT * WHERE { ?s ?p ?o }").await,
            0,
            "reads must keep being served from the (empty) last published snapshot during the outage",
        );
    }
    s.stop().await;
}

// ---------------------------------------------------------------------------
// (sq-bif.14) MULTI-STEP / SEQUENCED durable-write failure injection.
//
// The two tests above fail a CONTIGUOUS PREFIX of seals (a transient burst that clears, or a
// permanent jam). They do not exercise the INTERLEAVED path: a writer that recovers, takes more
// durable writes, then fails AGAIN, then recovers again — the realistic flapping-disk / quota-
// bouncing failure mode. This test drives that sequence over HTTP through the SAME `test-seams`
// seal seam and asserts the graceful-degradation contract holds at EVERY step:
//   * each seal that the injected hook fails → the in-flight write is refused 503 and is NEVER
//     published (fail-closed), so the live row count does NOT advance on a failed step;
//   * each seal the hook lets through → 204 and the row count advances by exactly that write;
//   * the writer SURVIVES every failure (the server keeps serving across the whole sequence);
//   * the cumulative live set after the sequence is EXACTLY the successful writes (no failed
//     write leaked in, no successful write lost), and that set is durable across a restart.
// The seam's hook is a stateful `FnMut`, so the per-seal failure decision is driven by a seal
// counter — failures land at chosen, NON-prefix positions.
// ---------------------------------------------------------------------------

/// Boots a `--persist` server whose Nth durable seal fails iff `fail_on` contains N (0-based over
/// the seals that actually reach the durable commit). `test-seams` only. Unlike
/// `start_with_transient_failures` (which fails the first `n_fail` seals), this fails an ARBITRARY
/// SET of seal indices, so a recover→fail→recover sequence can be modelled exactly.
#[cfg(feature = "test-seams")]
async fn start_with_failures_at(config: ServerConfig, fail_on: &'static [usize]) -> Server {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    let graph = Graph::load_str("", "turtle").unwrap();
    let seal_idx = Arc::new(AtomicUsize::new(0));
    let hook: Box<dyn FnMut() -> Option<String> + Send> = Box::new(move || {
        let n = seal_idx.fetch_add(1, Ordering::SeqCst);
        if fail_on.contains(&n) {
            Some(format!("injected durable-write error at seal #{}", n))
        } else {
            None
        }
    });
    let state = AppState::with_config_inject_durable_failure(graph, config, hook)
        .expect("durable open with injected failure seam");
    let app = router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await
            .unwrap();
    });
    Server { base: format!("http://{}", addr), shutdown: Some(tx), task: Some(task) }
}

/// A SEQUENCED outage: durable writes succeed, then a seal fails, then writes recover, then a seal
/// fails AGAIN, then recovers — interleaved with reads. At every step fail-closed holds (a refused
/// write never lands, a recovered write does), the writer survives the whole sequence, and the
/// final live set is EXACTLY the successful writes, durable across a restart. `test-seams` only.
#[cfg(feature = "test-seams")]
#[tokio::test]
async fn sequenced_durable_failures_recover_and_refail_fail_closed() {
    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();

    // Fail the 2nd and 4th durable seals (0-based seal indices 1 and 3). So the planned outcome
    // for writes w0..w4 is: w0 ok, w1 REFUSED, w2 ok, w3 REFUSED, w4 ok → 3 live rows.
    let s = start_with_failures_at(persist_config(scratch.path()), &[1, 3]).await;

    // `(should_fail, subject)` for five writes; we track the live count as we go so each step's
    // assertion depends on the PREVIOUS steps having behaved (a regression that mis-publishes a
    // refused write, or drops a good one, shifts the running count and trips a later assert).
    let plan = [false, true, false, true, false];
    let mut expected_live: usize = 0;

    for (i, should_fail) in plan.iter().enumerate() {
        let status = post_update(
            &cl,
            &s.base,
            &format!("INSERT DATA {{ <http://ex/w{}> <http://ex/p> <http://ex/v> }}", i),
        )
        .await;

        if *should_fail {
            assert_eq!(
                status, 503,
                "write w{} hits an injected durable failure → must be refused 503",
                i
            );
            // Fail-closed: the refused write is NOT in the live view.
            assert!(
                !ask(&cl, &s.base, &format!("ASK {{ <http://ex/w{}> ?p ?o }}", i)).await,
                "a refused (non-durable) write w{} must NOT be published",
                i
            );
        } else {
            assert_eq!(
                status, 204,
                "write w{} seals durably → must succeed 204 (writer survived prior failures)",
                i
            );
            expected_live += 1;
            // The successful write IS in the live view.
            assert!(
                ask(&cl, &s.base, &format!("ASK {{ <http://ex/w{}> ?p ?o }}", i)).await,
                "a successful write w{} must be published",
                i
            );
        }

        // The running cumulative count matches exactly the successful writes so far — reads keep
        // being served from the last published snapshot the whole way through the outage.
        assert_eq!(
            count_rows(&cl, &s.base, "SELECT * WHERE { ?s ?p ?o }").await,
            expected_live,
            "after step w{} the live set must be exactly the successful writes so far",
            i
        );
    }

    assert_eq!(expected_live, 3, "plan sanity: w0, w2, w4 should have committed");

    // The surviving live set is durable across a clean restart: exactly the successful writes,
    // and NONE of the refused ones ever resurrect.
    s.stop().await;
    let s2 = Server::start(persist_config(scratch.path())).await;
    assert_eq!(
        count_rows(&cl, &s2.base, "SELECT * WHERE { ?s ?p ?o }").await,
        3,
        "the successful writes must survive a restart (they really were made durable)",
    );
    for i in [0usize, 2, 4] {
        assert!(
            ask(&cl, &s2.base, &format!("ASK {{ <http://ex/w{}> ?p ?o }}", i)).await,
            "committed write w{} must survive the restart",
            i
        );
    }
    for i in [1usize, 3] {
        assert!(
            !ask(&cl, &s2.base, &format!("ASK {{ <http://ex/w{}> ?p ?o }}", i)).await,
            "refused write w{} must NEVER resurrect after a restart",
            i
        );
    }
    s2.stop().await;
}

// ---------------------------------------------------------------------------
// (sq-x32t, epic sq-toze.33) ADMIN WAL COMPACTION / VACUUM — erasure-completeness.
// ---------------------------------------------------------------------------

/// POSTs to `/admin/compact` (optionally with a Bearer token) and returns the status.
async fn post_compact(cl: &reqwest::Client, base: &str, token: Option<&str>) -> reqwest::StatusCode {
    let mut req = cl.post(format!("{base}/admin/compact"));
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {}", t));
    }
    req.send().await.unwrap().status()
}

/// Recursively true iff `needle`'s bytes appear in any regular file under `dir` — proves a
/// deleted datum's bytes are PHYSICALLY present/absent on disk.
fn dir_contains_bytes(dir: &Path, needle: &[u8]) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if dir_contains_bytes(&p, needle) {
                return true;
            }
        } else if let Ok(bytes) = std::fs::read(&p) {
            if bytes.windows(needle.len()).any(|w| w == needle) {
                return true;
            }
        }
    }
    false
}

/// The end-to-end ADMIN COMPACTION flow over HTTP: INSERT some data (one to keep, one secret),
/// DELETE the secret, then `POST /admin/compact`. After compaction the LIVE data is intact AND the
/// deleted secret's bytes are PHYSICALLY GONE from the `--persist` directory — and a restart
/// confirms the erasure is durable. This is the operator-invokable erasure of the runbook §7a.
#[tokio::test]
async fn admin_compact_physically_erases_deleted_data() {
    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();

    {
        let s = Server::start(persist_config(scratch.path())).await;
        assert_eq!(
            post_update(&cl, &s.base, "INSERT DATA { <http://ex/keep> <http://ex/p> \"KEEP-LIVE-9f3a\" }").await,
            204,
        );
        assert_eq!(
            post_update(&cl, &s.base, "INSERT DATA { <http://ex/alice> <http://ex/ssn> \"SECRET-SSN-7b21\" }").await,
            204,
        );
        // Logical erasure: the secret is gone from the live view…
        assert_eq!(
            post_update(&cl, &s.base, "DELETE DATA { <http://ex/alice> <http://ex/ssn> \"SECRET-SSN-7b21\" }").await,
            204,
        );
        assert_eq!(count_rows(&cl, &s.base, "SELECT * WHERE { ?s ?p ?o }").await, 1, "only KEEP remains live");
        // …but its bytes still linger in the on-disk WAL/dict history until compaction.
        assert!(dir_contains_bytes(scratch.path(), b"SECRET-SSN-7b21"), "pre-compaction: secret still on disk");

        // Operator-invoked compaction (no auth token configured here → ungated).
        assert_eq!(post_compact(&cl, &s.base, None).await, 200, "compaction must succeed");

        // Live data preserved; deleted secret physically erased; server still serving.
        assert_eq!(count_rows(&cl, &s.base, "SELECT * WHERE { ?s ?p ?o }").await, 1, "live data survives compaction");
        assert!(dir_contains_bytes(scratch.path(), b"KEEP-LIVE-9f3a"), "live data must remain on disk");
        assert!(!dir_contains_bytes(scratch.path(), b"SECRET-SSN-7b21"), "deleted secret bytes must be physically erased");

        // Writes still work after compaction (fresh WAL).
        assert_eq!(
            post_update(&cl, &s.base, "INSERT DATA { <http://ex/after> <http://ex/p> <http://ex/v> }").await,
            204,
        );
        s.stop().await;
    }
    // Durability: restart sees the post-compaction state; the secret never resurrects.
    let s2 = Server::start(persist_config(scratch.path())).await;
    assert_eq!(count_rows(&cl, &s2.base, "SELECT * WHERE { ?s ?p ?o }").await, 2, "KEEP + after survive restart");
    assert!(!dir_contains_bytes(scratch.path(), b"SECRET-SSN-7b21"), "erasure must survive the restart");
    s2.stop().await;
}

/// The admin compaction endpoint is GATED by the WRITE auth token: no/wrong token → 401, correct
/// token → 200. (Same gate as SPARQL Update — it mutates the durable store.)
#[tokio::test]
async fn admin_compact_requires_write_auth() {
    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();
    let config = ServerConfig { auth_token: Some("s3cr3t".to_string()), ..persist_config(scratch.path()) };
    let s = Server::start(config).await;

    assert_eq!(post_compact(&cl, &s.base, None).await, 401, "no token must be refused");
    assert_eq!(post_compact(&cl, &s.base, Some("wrong")).await, 401, "wrong token must be refused");
    assert_eq!(post_compact(&cl, &s.base, Some("s3cr3t")).await, 200, "correct write token compacts");
    s.stop().await;
}

/// An IN-MEMORY server (no `--persist`) returns 409 on `/admin/compact`: there is no on-disk WAL
/// history to purge (in-memory erasure is already immediate), so a misleading no-op success is
/// avoided.
#[tokio::test]
async fn admin_compact_on_in_memory_server_is_409() {
    let cl = reqwest::Client::new();
    // Default config = no persist dir = in-memory.
    let s = Server::start(ServerConfig::default()).await;
    assert_eq!(post_compact(&cl, &s.base, None).await, 409, "in-memory server has nothing to compact");
    s.stop().await;
}
