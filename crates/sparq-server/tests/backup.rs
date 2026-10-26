//! (sq-o5bi) Integration tests for ONLINE consistent-snapshot backup + restore —
//! the `backup` feature's `POST /admin/backup` + `POST /admin/restore` admin routes and the
//! restore-on-start path (`ServerConfig::restore`).
//!
//! These exercise the REAL HTTP path (not a mock): boot the server, apply updates so the
//! generation has moved past 0, take a backup, then restore it into a fresh (or the same)
//! server and assert the restored store is queryable and IDENTICAL. We also assert the
//! load-bearing fail-closed invariant: a corrupt / non-artifact body is rejected and the live
//! store is left untouched; an unauthenticated request is gated; a `--persist` server refuses.
//!
//! Run: `cargo test -p sparq-server --features backup --test backup`

#![cfg(feature = "backup")]

use std::sync::atomic::{AtomicU64, Ordering};

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

/// Boots the real server over `graph` on an ephemeral port; returns its base URL.
async fn spawn_with(graph: Graph, config: ServerConfig) -> String {
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
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

/// Counts the solutions of `query` over the named/default dataset (`FROM`-free, so the engine
/// queries the whole dataset).
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

/// Downloads a backup artifact from `POST /admin/backup`, asserting 200.
async fn backup(cl: &reqwest::Client, base: &str) -> Vec<u8> {
    let resp = cl
        .post(format!("{base}/admin/backup"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "backup must be 200");
    resp.bytes().await.unwrap().to_vec()
}

/// Counts all quads (default + named) across the dataset via a wildcard query that also walks
/// named graphs through a UNION — so a missing named graph would be caught.
async fn count_all(cl: &reqwest::Client, base: &str) -> usize {
    // `GRAPH ?g { ?s ?p ?o }` covers named graphs; the bare BGP covers the default graph.
    count_rows(
        cl,
        base,
        "SELECT * WHERE { { ?s ?p ?o } UNION { GRAPH ?g { ?s ?p ?o } } }",
    )
    .await
}

/// ROUND TRIP: backup of a moved-on store, restored into a FRESH server, is queryable and
/// equal — and the restored server keeps accepting updates afterward.
#[tokio::test]
async fn backup_then_restore_into_fresh_server_is_identical() {
    let cl = reqwest::Client::new();
    let src = spawn_with(
        Graph::load_str("<http://ex/s0> <http://ex/p> <http://ex/o0> .", "turtle").unwrap(),
        ServerConfig::default(),
    )
    .await;
    // Move the generation past 0 with a default-graph insert and a NAMED-graph insert.
    assert_eq!(
        post_update(
            &cl,
            &src,
            "INSERT DATA { <http://ex/s1> <http://ex/p> <http://ex/o1> }"
        )
        .await,
        reqwest::StatusCode::NO_CONTENT
    );
    assert_eq!(
        post_update(
            &cl,
            &src,
            "INSERT DATA { GRAPH <http://ex/g> { <http://ex/s2> <http://ex/p> <http://ex/o2> } }"
        )
        .await,
        reqwest::StatusCode::NO_CONTENT
    );
    let src_total = count_all(&cl, &src).await;
    assert_eq!(src_total, 3, "1 seed + 1 default insert + 1 named insert");

    let artifact = backup(&cl, &src).await;
    assert!(
        artifact.starts_with(b"SPARQ-BACKUP "),
        "artifact is the self-describing format"
    );

    // Restore into a brand-new empty server.
    let dst = spawn_with(
        Graph::load_str("", "turtle").unwrap(),
        ServerConfig::default(),
    )
    .await;
    assert_eq!(count_all(&cl, &dst).await, 0, "fresh server starts empty");
    let resp = cl
        .post(format!("{dst}/admin/restore"))
        .body(artifact)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::OK,
        "restore must be 200"
    );

    // The restored store is queryable and identical.
    assert_eq!(
        count_all(&cl, &dst).await,
        src_total,
        "restored quad count equals source"
    );
    assert_eq!(
        count_rows(
            &cl,
            &dst,
            "SELECT * WHERE { GRAPH <http://ex/g> { ?s ?p ?o } }"
        )
        .await,
        1,
        "the named graph survived the round trip"
    );
    // The restored server keeps serving WRITES (fresh ring+writer wired in).
    assert_eq!(
        post_update(
            &cl,
            &dst,
            "INSERT DATA { <http://ex/s3> <http://ex/p> <http://ex/o3> }"
        )
        .await,
        reqwest::StatusCode::NO_CONTENT
    );
    assert_eq!(
        count_all(&cl, &dst).await,
        src_total + 1,
        "post-restore update is visible"
    );
}

/// RESTORE-ON-START: a server constructed from a graph rehydrated by the restore-on-start
/// import path serves the restored data immediately. We drive the same `backup_import` the
/// `--restore` flag uses, then boot a server on the result — modelling exactly the binary's
/// startup wiring without spawning a subprocess.
#[tokio::test]
async fn restore_on_start_seeds_the_store() {
    let cl = reqwest::Client::new();
    // Produce an artifact from a populated server.
    let src = spawn_with(
        Graph::load_str("<http://ex/a> <http://ex/p> <http://ex/b> .", "turtle").unwrap(),
        ServerConfig::default(),
    )
    .await;
    post_update(
        &cl,
        &src,
        "INSERT DATA { <http://ex/c> <http://ex/p> <http://ex/d> }",
    )
    .await;
    let artifact = backup(&cl, &src).await;

    // The restore-on-start import path: import the artifact into a seed graph, boot on it.
    let (graph, meta) = sparq_serve::backup_import(&artifact[..]).expect("artifact imports");
    assert_eq!(
        meta.triples, 2,
        "metadata records the captured triple count"
    );
    let restored = spawn_with(graph, ServerConfig::default()).await;
    assert_eq!(
        count_all(&cl, &restored).await,
        2,
        "a server booted from the restored graph serves the data on start"
    );
}

/// FAIL-CLOSED: a non-artifact body is rejected (400) and the live store is untouched.
#[tokio::test]
async fn restore_rejects_non_artifact_and_leaves_store_intact() {
    let cl = reqwest::Client::new();
    let base = spawn_with(
        Graph::load_str("<http://ex/keep> <http://ex/p> <http://ex/o> .", "turtle").unwrap(),
        ServerConfig::default(),
    )
    .await;
    assert_eq!(count_all(&cl, &base).await, 1);

    // A bare N-Quads body looks like a body but lacks the artifact magic → fail closed.
    let resp = cl
        .post(format!("{base}/admin/restore"))
        .body("<http://ex/x> <http://ex/y> <http://ex/z> .\n")
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "a non-artifact restore is rejected"
    );
    // The live store is unchanged — the rejected restore swapped nothing in.
    assert_eq!(
        count_all(&cl, &base).await,
        1,
        "the store is intact after a rejected restore"
    );
}

/// FAIL-CLOSED: a CORRUPT artifact (a flipped body byte) is rejected (400), store intact.
#[tokio::test]
async fn restore_rejects_corrupt_artifact() {
    let cl = reqwest::Client::new();
    let base = spawn_with(
        Graph::load_str("<http://ex/keep> <http://ex/p> <http://ex/o> .", "turtle").unwrap(),
        ServerConfig::default(),
    )
    .await;
    let mut artifact = backup(&cl, &base).await;
    // Flip the very last byte of the body — the digest check must catch it.
    let last = artifact.len() - 1;
    artifact[last] ^= 0xff;
    let resp = cl
        .post(format!("{base}/admin/restore"))
        .body(artifact)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        count_all(&cl, &base).await,
        1,
        "store intact after a corrupt restore"
    );
}

/// AUTH: with a write token set, both admin routes require the Bearer token.
#[tokio::test]
async fn admin_routes_are_write_gated() {
    let cl = reqwest::Client::new();
    let config = ServerConfig {
        auth_token: Some("s3cret".into()),
        ..ServerConfig::default()
    };
    let base = spawn_with(
        Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap(),
        config,
    )
    .await;

    // Unauthenticated backup → 401.
    let resp = cl
        .post(format!("{base}/admin/backup"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    // Unauthenticated restore → 401.
    let resp = cl
        .post(format!("{base}/admin/restore"))
        .body("anything")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Authenticated backup → 200.
    let resp = cl
        .post(format!("{base}/admin/backup"))
        .header("authorization", "Bearer s3cret")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
}

/// Downloads a DELTA artifact from `POST /admin/backup/delta?from=N`, returning (status, bytes).
async fn backup_delta(
    cl: &reqwest::Client,
    base: &str,
    from: u64,
) -> (reqwest::StatusCode, Vec<u8>) {
    let resp = cl
        .post(format!("{base}/admin/backup/delta"))
        .query(&[("from", from.to_string())])
        .send()
        .await
        .unwrap();
    let status = resp.status();
    (status, resp.bytes().await.unwrap().to_vec())
}

/// (sq-bu1a) PITR ROUND TRIP via the real HTTP delta route: take a BASE backup at
/// generation 0, move the store forward, export a DELTA(0→current) over HTTP, then restore the
/// base + replay the delta — the recovered store equals the moved-on source.
#[tokio::test]
async fn pitr_base_plus_http_delta_recovers_the_moved_on_store() {
    let cl = reqwest::Client::new();
    let src = spawn_with(
        Graph::load_str("<http://ex/s0> <http://ex/p> <http://ex/o0> .", "turtle").unwrap(),
        ServerConfig::default(),
    )
    .await;
    // BASE @ generation 0 (before any update).
    let base_artifact = backup(&cl, &src).await;

    // Move forward: a default insert + a named-graph insert + a delete of the seed.
    post_update(
        &cl,
        &src,
        "INSERT DATA { <http://ex/s1> <http://ex/p> <http://ex/o1> }",
    )
    .await;
    post_update(
        &cl,
        &src,
        "INSERT DATA { GRAPH <http://ex/g> { <http://ex/s2> <http://ex/p> <http://ex/o2> } }",
    )
    .await;
    post_update(
        &cl,
        &src,
        "DELETE DATA { <http://ex/s0> <http://ex/p> <http://ex/o0> }",
    )
    .await;
    let src_total = count_all(&cl, &src).await;
    assert_eq!(src_total, 2, "seed deleted, 2 inserts remain");

    // DELTA(0 -> current) over HTTP.
    let (status, delta_artifact) = backup_delta(&cl, &src, 0).await;
    assert_eq!(status, reqwest::StatusCode::OK, "delta export must be 200");
    assert!(
        delta_artifact.starts_with(b"SPARQ-BACKUP-DELTA "),
        "delta is the distinct self-describing kind"
    );

    // RESTORE base + replay delta into a fresh server (via the AppState PITR primitive the
    // binary's --restore + --restore-delta uses).
    let state = AppState::with_config(Graph::load_str("", "turtle").unwrap(), ServerConfig::default());
    let recovered_gen = state
        .restore_from_with_deltas(&base_artifact, &[delta_artifact])
        .expect("base + delta restore");
    assert_eq!(recovered_gen, 3, "recovered to the source's writer-seq (3)");
    let app = sparq_server::router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let dst = format!("http://{addr}");

    assert_eq!(
        count_all(&cl, &dst).await,
        src_total,
        "PITR-recovered quad count equals the moved-on source"
    );
    assert_eq!(
        count_rows(&cl, &dst, "SELECT * WHERE { GRAPH <http://ex/g> { ?s ?p ?o } }").await,
        1,
        "the named-graph insert was replayed"
    );
    // The deleted seed triple is absent after replay (counted as a SELECT, not an ASK).
    assert_eq!(
        count_rows(
            &cl,
            &dst,
            "SELECT * WHERE { <http://ex/s0> <http://ex/p> <http://ex/o0> }"
        )
        .await,
        0,
        "the deleted seed triple is absent after replay"
    );
}

/// (sq-bu1a) FAIL-CLOSED: `restore_from_with_deltas` rejects a corrupt delta and the
/// base install never happens (the whole op is atomic — import + replay before any swap).
#[tokio::test]
async fn pitr_rejects_corrupt_delta_in_chain() {
    let cl = reqwest::Client::new();
    let src = spawn_with(
        Graph::load_str("<http://ex/s0> <http://ex/p> <http://ex/o0> .", "turtle").unwrap(),
        ServerConfig::default(),
    )
    .await;
    let base_artifact = backup(&cl, &src).await;
    post_update(
        &cl,
        &src,
        "INSERT DATA { <http://ex/s1> <http://ex/p> <http://ex/o1> }",
    )
    .await;
    let (status, mut delta_artifact) = backup_delta(&cl, &src, 0).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    // Corrupt the delta body (flip the last byte).
    let last = delta_artifact.len() - 1;
    delta_artifact[last] ^= 0xff;

    let state = AppState::with_config(Graph::load_str("", "turtle").unwrap(), ServerConfig::default());
    let res = state.restore_from_with_deltas(&base_artifact, &[delta_artifact]);
    assert!(res.is_err(), "a corrupt delta must fail the whole restore closed");
}

/// (sq-bu1a) The delta route is WRITE-gated, POST-only, and 400s a missing/invalid
/// `from`; an aged-out `from` is 410 Gone (mirroring time-travel).
#[tokio::test]
async fn delta_route_is_gated_and_validates_from() {
    let cl = reqwest::Client::new();
    // Auth-gated server: unauthenticated delta -> 401.
    let gated = spawn_with(
        Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap(),
        ServerConfig {
            auth_token: Some("s3cret".into()),
            ..ServerConfig::default()
        },
    )
    .await;
    let resp = cl
        .post(format!("{gated}/admin/backup/delta"))
        .query(&[("from", "0")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Open server: missing `from` -> 400.
    let open = spawn_with(
        Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap(),
        ServerConfig::default(),
    )
    .await;
    let resp = cl
        .post(format!("{open}/admin/backup/delta"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "a missing `from` is a 400"
    );
    // Non-numeric `from` -> 400.
    let resp = cl
        .post(format!("{open}/admin/backup/delta"))
        .query(&[("from", "abc")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // `from` >= current (no movement yet, current is 0): from=0 is not earlier than current=0,
    // so the range check rejects with 400.
    let (status, _b) = backup_delta(&cl, &open, 0).await;
    assert_eq!(
        status,
        reqwest::StatusCode::BAD_REQUEST,
        "from must be earlier than the current generation"
    );

    // An aged-out `from` (never retained without time-travel; far beyond current) -> 410 Gone.
    post_update(
        &cl,
        &open,
        "INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/b> }",
    )
    .await;
    let (status, _b) = backup_delta(&cl, &open, 999).await;
    assert_eq!(
        status,
        reqwest::StatusCode::GONE,
        "a `from` generation that is not retained is 410 Gone"
    );
}

/// A unique scratch persist directory removed on drop (the persist.rs hygiene pattern).
struct ScratchDir(std::path::PathBuf);
impl ScratchDir {
    fn new() -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        ScratchDir(std::env::temp_dir().join(format!(
            "sparq-backup-persist-{}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        )))
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn persist_config(dir: &std::path::Path) -> ServerConfig {
    ServerConfig { persist_dir: Some(dir.to_path_buf()), ..ServerConfig::default() }
}

/// POSTs a restore artifact to `/admin/restore`, optionally with `?persist=true`, returning the
/// status. `persist == true` is the write-through opt-in (sq-ft7u).
async fn post_restore(cl: &reqwest::Client, base: &str, artifact: Vec<u8>, persist: bool) -> reqwest::StatusCode {
    let url = if persist {
        format!("{base}/admin/restore?persist=true")
    } else {
        format!("{base}/admin/restore")
    };
    cl.post(url).body(artifact).send().await.unwrap().status()
}

/// PERSIST CONTRACT (sq-ft7u): a `--persist` durable server refuses an in-memory-only restore
/// (no `?persist=true`) with 409, but ACCEPTS a restore that opts into write-through
/// (`?persist=true`) with 200. (The 200 path's durability is proven by
/// `restore_persist_survives_restart` below; this test pins the status contract.)
#[tokio::test]
async fn restore_refuses_on_persist_server_without_opt_in_accepts_with_it() {
    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();
    let base = spawn_with(Graph::load_str("", "turtle").unwrap(), persist_config(scratch.path())).await;

    // WITHOUT the persist opt-in → 409 (an in-memory-only restore would be lost on restart).
    let resp = cl
        .post(format!("{base}/admin/restore"))
        .body("anything")
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::CONFLICT,
        "an in-memory-only restore on a --persist server must be refused (409)"
    );

    // WITH ?persist=true, a VALID artifact → 200 (write-through accepted).
    let artifact = make_artifact(&cl).await;
    assert_eq!(
        post_restore(&cl, &base, artifact, true).await,
        reqwest::StatusCode::OK,
        "a write-through restore (?persist=true) on a --persist server must be accepted (200)"
    );
}

/// Produces a valid backup artifact from a small populated in-memory source server.
async fn make_artifact(cl: &reqwest::Client) -> Vec<u8> {
    let src = spawn_with(
        Graph::load_str("<http://ex/a> <http://ex/p> <http://ex/b> .", "turtle").unwrap(),
        ServerConfig::default(),
    )
    .await;
    post_update(cl, &src, "INSERT DATA { <http://ex/c> <http://ex/p> <http://ex/d> }").await;
    post_update(
        cl,
        &src,
        "INSERT DATA { GRAPH <http://ex/g> { <http://ex/n> <http://ex/q> <http://ex/w> } }",
    )
    .await;
    backup(cl, &src).await
}

// ---------------------------------------------------------------------------
// (sq-ft7u) RESTORE-INTO-LIVE-DURABLE-STORE (--persist write-through).
//
// The load-bearing proof: a restore opted into write-through (?persist=true) on a --persist
// server is written THROUGH to the durable directory, so it SURVIVES A RESTART (reopen the same
// dir → restored triples present). We reuse the persist.rs restart pattern: a Server we can DROP
// (so its writer thread joins + every WAL handle releases), then REOPEN the same persist_dir.
// We also pin: corrupt-artifact still fail-closes WITHOUT touching the durable store; the route
// stays write-gated.
// ---------------------------------------------------------------------------

/// A running server we can SHUT DOWN cleanly (mirrors persist.rs): on `stop()` the serve task
/// resolves, dropping the `AppState` so the writer thread joins and the durable dir's WAL handle
/// releases — modelling a clean process restart.
struct Server {
    base: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Server {
    async fn start(config: ServerConfig) -> Self {
        // Empty seed; an existing persist dir is opened (seed ignored), an empty one created.
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

    async fn stop(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            task.await.expect("server serve task panicked / failed");
        }
    }
}

/// THE CORE PROOF (sq-ft7u). On a `--persist` server, a `?persist=true` restore is written through
/// to the durable directory and SURVIVES A RESTART: after restoring an artifact captured from a
/// SEPARATE in-memory source, the restored triples are present; after dropping the server and
/// REOPENING the same persist_dir, they are STILL present (the on-disk base IS the restored image).
#[tokio::test]
async fn restore_persist_survives_restart() {
    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();

    // An artifact captured from a SEPARATE source server (default-graph + a named graph).
    let artifact = make_artifact(&cl).await;

    // A fresh --persist server, initially with its OWN distinct data (proves the restore REPLACES).
    let s1 = Server::start(persist_config(scratch.path())).await;
    assert_eq!(
        post_update(&cl, &s1.base, "INSERT DATA { <http://ex/preexisting> <http://ex/p> <http://ex/o> }").await,
        reqwest::StatusCode::NO_CONTENT
    );
    assert_eq!(count_all(&cl, &s1.base).await, 1, "the durable server starts with its own one triple");

    // Restore WITH write-through → 200, and the restored content replaces the pre-existing data.
    assert_eq!(
        post_restore(&cl, &s1.base, artifact.clone(), true).await,
        reqwest::StatusCode::OK,
        "write-through restore must be accepted on a --persist server"
    );
    // make_artifact's source = 1 seed + 1 default insert + 1 named insert = 3 quads.
    assert_eq!(count_all(&cl, &s1.base).await, 3, "the restored quad set is live after restore");
    assert_eq!(
        count_rows(&cl, &s1.base, "SELECT * WHERE { <http://ex/preexisting> ?p ?o }").await,
        0,
        "the restore REPLACED the pre-existing durable content"
    );
    assert_eq!(
        count_rows(&cl, &s1.base, "SELECT * WHERE { GRAPH <http://ex/g> { ?s ?p ?o } }").await,
        1,
        "the named graph from the artifact is live after the restore"
    );

    // The restored durable server is still WRITABLE (the writer survived; new WAL on the new base).
    assert_eq!(
        post_update(&cl, &s1.base, "INSERT DATA { <http://ex/postrestore> <http://ex/p> <http://ex/o> }").await,
        reqwest::StatusCode::NO_CONTENT
    );
    assert_eq!(count_all(&cl, &s1.base).await, 4, "a post-restore update is visible");

    // RESTART: drop the server (writer joins, WAL handle released), reopen the SAME dir.
    s1.stop().await;
    let s2 = Server::start(persist_config(scratch.path())).await;

    // The restored triples — AND the post-restore update — survived the restart (durable).
    assert_eq!(
        count_all(&cl, &s2.base).await,
        4,
        "the restored dataset + the post-restore update must survive the restart (written through)"
    );
    assert_eq!(
        count_rows(&cl, &s2.base, "SELECT * WHERE { GRAPH <http://ex/g> { ?s ?p ?o } }").await,
        1,
        "the restored named graph survives the restart"
    );
    assert_eq!(
        count_rows(&cl, &s2.base, "SELECT * WHERE { <http://ex/preexisting> ?p ?o }").await,
        0,
        "the replaced pre-existing content must NOT resurrect after a restart"
    );
    s2.stop().await;
}

/// FAIL-CLOSED on a `--persist` server: a CORRUPT artifact restore (?persist=true) is rejected
/// (400) and the durable store + dir are UNTOUCHED — reopening the dir shows the original data,
/// with no `.compact-new`/`.compact-old` swap leftovers that would mis-heal on the next open.
#[tokio::test]
async fn restore_persist_corrupt_artifact_leaves_durable_store_untouched() {
    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();

    let s1 = Server::start(persist_config(scratch.path())).await;
    assert_eq!(
        post_update(&cl, &s1.base, "INSERT DATA { <http://ex/keep> <http://ex/p> \"ORIGINAL\" }").await,
        reqwest::StatusCode::NO_CONTENT
    );
    assert_eq!(count_all(&cl, &s1.base).await, 1);

    // Corrupt a valid artifact (flip the last body byte) and restore it WITH write-through.
    let mut artifact = make_artifact(&cl).await;
    let last = artifact.len() - 1;
    artifact[last] ^= 0xff;
    assert_eq!(
        post_restore(&cl, &s1.base, artifact, true).await,
        reqwest::StatusCode::BAD_REQUEST,
        "a corrupt write-through restore must fail closed (400)"
    );
    // The live durable store is unchanged (the import failed before any swap began).
    assert_eq!(count_all(&cl, &s1.base).await, 1, "live durable store intact after a corrupt restore");

    // No swap-sibling leftovers next to the persist dir (a stray .compact-new/-old could mis-heal).
    // (Staging names append to the whole directory name: `<dir>.compact-new`.)
    let sibling = |kind: &str| {
        let mut name = scratch.path().file_name().unwrap().to_os_string();
        name.push(format!(".{kind}"));
        scratch.path().with_file_name(name)
    };
    let (new_sib, old_sib) = (sibling("compact-new"), sibling("compact-old"));
    assert!(!new_sib.exists(), "no .compact-new leftover after a failed restore");
    assert!(!old_sib.exists(), "no .compact-old leftover after a failed restore");

    // And the original survives a restart (the corrupt restore truly touched nothing on disk).
    s1.stop().await;
    let s2 = Server::start(persist_config(scratch.path())).await;
    assert_eq!(count_all(&cl, &s2.base).await, 1, "original durable data survives a restart");
    assert_eq!(
        count_rows(&cl, &s2.base, "SELECT * WHERE { <http://ex/keep> ?p ?o }").await,
        1,
        "the original (un-restored-over) triple is still present after a restart"
    );
    s2.stop().await;
}

/// AUTH: the write-through restore route stays WRITE-gated — an unauthenticated ?persist=true
/// restore is 401 (the auth gate runs before the persist-posture checks).
#[tokio::test]
async fn restore_persist_route_is_write_gated() {
    let scratch = ScratchDir::new();
    let cl = reqwest::Client::new();
    let config = ServerConfig { auth_token: Some("s3cret".into()), ..persist_config(scratch.path()) };
    let s = Server::start(config).await;

    let resp = cl
        .post(format!("{}/admin/restore?persist=true", s.base))
        .body("anything")
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "an unauthenticated write-through restore must be 401"
    );
    s.stop().await;
}

/// An IN-MEMORY server (no --persist) refuses ?persist=true with 409 — there is no durable dir to
/// write the restore through to. (The in-memory restore path stays available WITHOUT ?persist.)
#[tokio::test]
async fn restore_persist_on_in_memory_server_is_409() {
    let cl = reqwest::Client::new();
    let base = spawn_with(Graph::load_str("", "turtle").unwrap(), ServerConfig::default()).await;
    let artifact = make_artifact(&cl).await;
    assert_eq!(
        post_restore(&cl, &base, artifact, true).await,
        reqwest::StatusCode::CONFLICT,
        "?persist=true on an in-memory server must be 409 (no durable dir to write through)"
    );
}

// ---------------------------------------------------------------------------
// (sq-fy8ci) SINGLE-FLIGHT restore guard.
//
// Two concurrent restores used to be silently serialized by the single writer thread
// (individually crash-safe, last-writer-wins, no signal to the operator). The guard makes the
// collision EXPLICIT: while one restore is in flight, a second `POST /admin/restore` is 409.
// The in-flight restore is simulated DETERMINISTICALLY by holding the public permit
// (`AppState::try_begin_restore`) — the exact object the route claims — rather than racing two
// real restores (which would be flaky-by-construction).
// ---------------------------------------------------------------------------

/// Like [`spawn_with`], but also returns the (Clone) `AppState` behind the router so a test can
/// drive state-level APIs (e.g. hold the restore permit) against the SAME server identity.
async fn spawn_with_state(graph: Graph, config: ServerConfig) -> (String, AppState) {
    let state = AppState::with_config(graph, config);
    let app = router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), state)
}

/// DIRECT unit test for the public permit API: `try_begin_restore` grants exactly ONE live
/// permit per server state — a second call (even through a state CLONE, as every handler holds
/// one) is refused while the first permit is alive, and dropping the permit releases it.
#[tokio::test]
async fn try_begin_restore_permit_is_single_flight() {
    let state = AppState::new(Graph::load_str("", "turtle").unwrap());
    let first = state.try_begin_restore();
    assert!(first.is_some(), "an idle server must grant the restore permit");
    assert!(
        state.clone().try_begin_restore().is_none(),
        "a second permit must be refused while the first is alive — including through a clone \
         (handlers hold clones; the permit is per-server identity)"
    );
    drop(first);
    assert!(
        state.try_begin_restore().is_some(),
        "dropping the permit must release the single-flight guard (RAII, no wedge)"
    );
}

/// ROUTE CONTRACT: while a restore is in flight, a second `POST /admin/restore` is 409 and the
/// live store is left UNTOUCHED; once the in-flight restore completes (the permit drops), the
/// SAME artifact restores 200 — the guard is a permit, not a poison.
#[tokio::test]
async fn concurrent_restore_is_rejected_409_then_succeeds_after_release() {
    let cl = reqwest::Client::new();
    let artifact = make_artifact(&cl).await; // 3 triples (default + named)
    let (base, state) =
        spawn_with_state(Graph::load_str("", "turtle").unwrap(), ServerConfig::default()).await;
    assert_eq!(count_all(&cl, &base).await, 0, "destination starts empty");

    // Deterministically simulate an in-flight restore: hold the route's own permit.
    let permit = state.try_begin_restore().expect("idle server grants the permit");
    let resp = cl
        .post(format!("{base}/admin/restore"))
        .body(artifact.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::CONFLICT,
        "a restore posted while one is in flight must be rejected 409, not silently serialized"
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("already in progress"),
        "the 409 must say WHY (restore already in progress); got: {body}"
    );
    assert_eq!(
        count_all(&cl, &base).await,
        0,
        "the refused restore must leave the live store untouched"
    );

    // Release the permit (the in-flight restore "completed"): the same artifact now lands.
    drop(permit);
    assert_eq!(
        post_restore(&cl, &base, artifact, false).await,
        reqwest::StatusCode::OK,
        "once the permit is released the restore must succeed"
    );
    assert_eq!(
        count_all(&cl, &base).await,
        3,
        "the post-release restore must actually install the artifact"
    );
}
