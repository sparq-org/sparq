//! Integration tests for the SPARQL Update path over the sparq-serve generation ring +
//! sequenced writer (Wave A4 rewiring — see `src/http.rs::AppState` and
//! research/concurrent-serving.md §6).
//!
//! Covers the properties the design promises:
//!   * sequential consistency over HTTP — every group-commit ack means the generation
//!     containing the update is published, so it is visible to the next query;
//!   * atomicity of failures — a rejected update is skipped (writer re-forks and
//!     replays), the published chain never contains a partial effect, and the server
//!     keeps accepting updates;
//!   * reader isolation — queries pin a generation lock-free and never wait on the
//!     writer, even while a batch commit is doing its O(graph) fork;
//!   * ring pinning — a generation pinned before a commit keeps serving its old state
//!     after the writer publishes past it (snapshot consistency for long responses).
//!
//! The `#[ignore]`d benchmark at the bottom is the honest update-cost measurement on a
//! 1M-triple graph under the ring (per-batch O(graph) fork — the recorded A2 trade):
//!   cargo test -p sparq-server --release --test updates -- --ignored --nocapture
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use std::time::{Duration, Instant};

use sparq_core::dict::Dict;
use sparq_core::Graph;
use sparq_server::{router, AppState, PodId, ServerConfig, GLOBAL_POD};
use tokio::net::TcpListener;

/// A synthetic graph of `n` distinct triples (10 predicates, n/10 subjects), built
/// directly from interned ids — cheap enough for debug-mode tests.
fn synthetic_graph(n: usize) -> Graph {
    let nsubj = (n / 10).max(1);
    let mut dict = Dict::new();
    let preds: Vec<_> = (0..10).map(|p| dict.intern_iri(&format!("http://ex/p{p}"))).collect();
    let subjs: Vec<_> = (0..nsubj).map(|s| dict.intern_iri(&format!("http://ex/s{s}"))).collect();
    // Murmur-style mix so object indexes don't collapse under set dedup.
    let mix = |i: usize| -> usize {
        let mut h = i as u64;
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        (h % nsubj as u64) as usize
    };
    let triples: Vec<_> = (0..n).map(|i| [subjs[i % nsubj], preds[i % 10], subjs[mix(i)]]).collect();
    Graph::from_parts(dict, triples)
}

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

/// Counts the solutions of `query` by counting binding objects in the JSON result.
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

// ---------------------------------------------------------------------------
// Sequential consistency
// ---------------------------------------------------------------------------

/// Every update in a sequence must be visible to the immediately following query: the
/// 204 is the writer's group-commit ack, which only returns once the generation
/// containing the update is published — so a fresh pin (the next query) must see it.
/// Interleaving queries after every update would expose an ack-before-publish bug or a
/// batch that dropped an earlier update.
#[tokio::test]
async fn sequential_updates_are_all_visible() {
    let base = spawn_with(synthetic_graph(1_000), ServerConfig::default()).await;
    let cl = reqwest::Client::new();
    for i in 0..12 {
        let ins = format!("INSERT DATA {{ <http://ex/u{i}> <http://ex/seen> <http://ex/yes> }}");
        assert_eq!(post_update(&cl, &base, &ins).await, 204);
        let n = count_rows(&cl, &base, "SELECT ?s WHERE { ?s <http://ex/seen> ?o }").await;
        assert_eq!(n, i + 1, "update {i} (or an earlier one lost in lag replay) is not visible");
    }
    // Deletes flow through the same path.
    assert_eq!(
        post_update(&cl, &base, "DELETE DATA { <http://ex/u0> <http://ex/seen> <http://ex/yes> }").await,
        204
    );
    assert_eq!(count_rows(&cl, &base, "SELECT ?s WHERE { ?s <http://ex/seen> ?o }").await, 11);
}

// ---------------------------------------------------------------------------
// Failed updates: atomic + recoverable
// ---------------------------------------------------------------------------

/// A refused update (unsupported operation) must leave the published graph untouched —
/// including when the refusal happens after a *prefix* of the request already applied
/// to the writer's working copy (the writer discards it, re-forks and replays — see
/// sparq-serve's failed-update policy) — and the server must keep accepting updates.
#[tokio::test]
async fn failed_update_is_atomic_and_server_recovers() {
    let base = spawn_with(synthetic_graph(1_000), ServerConfig::default()).await;
    let cl = reqwest::Client::new();

    // Two good updates first, so the failure lands on a non-trivial published chain.
    assert_eq!(post_update(&cl, &base, "INSERT DATA { <http://ex/a> <http://ex/q> <http://ex/b> }").await, 204);
    assert_eq!(post_update(&cl, &base, "INSERT DATA { <http://ex/c> <http://ex/q> <http://ex/d> }").await, 204);

    // A request whose FIRST operation succeeds and SECOND fails at execution (LOAD from a
    // nonexistent file — named-graph ops themselves are supported since conformance round 3):
    // must be a 400 with no partial effect visible.
    let partial = "INSERT DATA { <http://ex/leak> <http://ex/q> <http://ex/leak> } ; \
                   LOAD <file:///nonexistent/sparq-test-no-such-file.nt>";
    assert_eq!(post_update(&cl, &base, partial).await, 400);
    assert_eq!(
        count_rows(&cl, &base, "SELECT ?s WHERE { <http://ex/leak> <http://ex/q> ?s }").await,
        0,
        "a refused update leaked a partial application into the published graph"
    );
    assert_eq!(count_rows(&cl, &base, "SELECT ?s WHERE { ?s <http://ex/q> ?o }").await, 2);

    // The discarded working copy is re-forked transparently: further updates still work.
    assert_eq!(post_update(&cl, &base, "INSERT DATA { <http://ex/e> <http://ex/q> <http://ex/f> }").await, 204);
    assert_eq!(post_update(&cl, &base, "INSERT DATA { <http://ex/g> <http://ex/q> <http://ex/h> }").await, 204);
    assert_eq!(count_rows(&cl, &base, "SELECT ?s WHERE { ?s <http://ex/q> ?o }").await, 4);
}

// ---------------------------------------------------------------------------
// Concurrent reads proceed while updates commit (no reader stall, over HTTP)
// ---------------------------------------------------------------------------

/// The end-to-end no-reader-stall property the ring exists for: GET queries keep
/// answering (with bounded latency) while a stream of updates commits, each update
/// paying the writer's O(graph) batch fork on a 200k-triple graph. Under the removed
/// double buffer this was where the §4.3/§4.4 reclaim stall could bite; under the ring
/// readers pin generations lock-free and never interact with the writer. Prints the
/// observed read latencies (this doubles as the Wave A4 perf smoke).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_queries_proceed_while_updates_commit() {
    let n = 200_000;
    let base = spawn_with(synthetic_graph(n), ServerConfig::default()).await;
    let probe = "SELECT ?s WHERE { ?s <http://ex/cp> ?o }";
    let updates = 5usize;

    let upd_base = base.clone();
    let updater = tokio::spawn(async move {
        let cl = reqwest::Client::new();
        let t = Instant::now();
        for i in 0..updates {
            let ins = format!("INSERT DATA {{ <http://ex/c{i}> <http://ex/cp> <http://ex/cv> }}");
            assert_eq!(post_update(&cl, &upd_base, &ins).await, 204);
        }
        t.elapsed()
    });

    let cl = reqwest::Client::new();
    let mut max_read = Duration::ZERO;
    let mut total_read = Duration::ZERO;
    let mut samples = 0u32;
    while !updater.is_finished() {
        let t = Instant::now();
        let rows = count_rows(&cl, &base, probe).await;
        let dt = t.elapsed();
        max_read = max_read.max(dt);
        total_read += dt;
        samples += 1;
        assert!(rows <= updates, "reader saw an uncommitted state: {rows} rows");
    }
    let updates_took = updater.await.unwrap();
    assert_eq!(count_rows(&cl, &base, probe).await, updates, "an acked update is not visible");
    assert!(samples > 3, "sampler barely ran ({samples} samples) — cannot judge stalling");
    eprintln!(
        "reads during {updates} committing updates ({updates_took:?} total): \
         {samples} samples, mean {:?}, max {max_read:?}",
        total_read / samples
    );
    // Reads are point scans answered from a pinned generation; 500 ms is an enormous
    // margin over the milliseconds they take, while each update batch pays an O(graph)
    // fork that is far slower — a reader waiting on the writer would blow through this.
    assert!(
        max_read < Duration::from_millis(500),
        "a reader stalled for {max_read:?} while updates were committing"
    );
}

// ---------------------------------------------------------------------------
// Readers never block on the writer
// ---------------------------------------------------------------------------

/// While an update is doing substantial writer-side work (parse + apply of a bulk
/// INSERT DATA batch — the original single-triple update became too fast to sample
/// once `Graph::fork` went O(delta)), readers must keep pinning generations and
/// answering queries — `current()` is a lock-free load that never touches the writer.
/// Each observed count must also be one of the two committed states (snapshot
/// consistency: old or new, never partial).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn readers_are_not_blocked_during_a_slow_update() {
    let n = 200_000;
    let state = AppState::with_config(synthetic_graph(n), ServerConfig::default());
    let probe = "SELECT ?o WHERE { <http://ex/w> <http://ex/w> ?o }";
    let before = sparq_engine::query(state.current().snapshot(), probe).unwrap().rows.len();
    assert_eq!(before, 0);

    // One probe triple plus a 50k-triple bulk payload: the batch commits atomically,
    // so the probe still observes exactly "old" (0 rows) or "new" (1 row), while the
    // parse + apply gives the sampler a real window to observe during.
    let mut update = String::from("INSERT DATA { <http://ex/w> <http://ex/w> <http://ex/w> .\n");
    for i in 0..50_000 {
        update.push_str(&format!("<http://ex/bulk/{i}> <http://ex/bulkp> <http://ex/bulko> .\n"));
    }
    update.push('}');

    let writer_state = state.clone();
    let writer = tokio::task::spawn_blocking(move || {
        let t = Instant::now();
        writer_state.apply_update(&update).unwrap();
        t.elapsed()
    });

    // Sample reader latency until the update commits (and a little after).
    let mut max_read = Duration::ZERO;
    let mut samples = 0u32;
    let mut committed = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    while !committed && Instant::now() < deadline {
        let t = Instant::now();
        let gen = state.current();
        let rows = sparq_engine::query(gen.snapshot(), probe).unwrap().rows.len();
        let dt = t.elapsed();
        max_read = max_read.max(dt);
        samples += 1;
        assert!(rows == 0 || rows == 1, "reader saw a state that was never committed: {rows} rows");
        committed = rows == 1;
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let update_took = writer.await.unwrap();
    assert!(committed, "update did not become visible within 60s");
    assert!(samples > 3, "sampler barely ran ({samples} samples) — cannot judge blocking");
    // Pinning is a lock-free arc-swap load (~ns) and the publish is one pointer store;
    // 250ms is an enormous margin, while the batch fork itself takes much longer.
    assert!(
        max_read < Duration::from_millis(250),
        "a reader stalled for {max_read:?} during an update that took {update_took:?}"
    );
}

// ---------------------------------------------------------------------------
// Ring pinning: old generations stay readable after the writer publishes past them
// ---------------------------------------------------------------------------

/// The server-level pinning contract that makes long/streamed responses snapshot-
/// consistent: a generation pinned BEFORE updates commit keeps serving exactly its
/// old state afterwards (the writer never reclaims), while a fresh pin sees every
/// acked update. Sequential acked updates advance the generation number one batch
/// each (group commit can only coalesce concurrent submissions).
#[test]
fn pinned_generations_stay_readable_after_commits() {
    let state = AppState::with_config(synthetic_graph(1_000), ServerConfig::default());
    let probe = "SELECT ?s WHERE { ?s <http://ex/kp> ?o }";
    let pinned = state.current();
    let n0 = pinned.number();
    for i in 0..6 {
        let upd = format!("INSERT DATA {{ <http://ex/k{i}> <http://ex/kp> <http://ex/kv> }}");
        state.apply_update(&upd).unwrap_or_else(|e| panic!("{e}"));
    }
    let fresh = state.current();
    assert_eq!(fresh.number(), n0 + 6, "each sequentially acked update publishes one generation");
    let new_rows = sparq_engine::query(fresh.snapshot(), probe).unwrap();
    assert_eq!(new_rows.rows.len(), 6, "a fresh pin must see every acked update");
    // The old pin still answers from its own immutable snapshot — zero of the updates.
    let old_rows = sparq_engine::query(pinned.snapshot(), probe).unwrap();
    assert_eq!(old_rows.rows.len(), 0, "a pinned generation changed under a reader");
    assert_eq!(pinned.number(), n0);
}

// ---------------------------------------------------------------------------
// Honest update-cost benchmark (1M triples) — run explicitly in release mode
// ---------------------------------------------------------------------------

/// End-to-end HTTP update cost on a 1M-triple graph under the generation ring.
///
/// Honest numbers, stated plainly: under the ring every BATCH commit pays the
/// writer's O(graph) fork (the A2 `GraphApplier` decision — the double buffer's
/// in-place reclaim is impossible by design and was the measured §4.3/§4.4 stall),
/// so an ISOLATED update's ack latency is fork-priced. Group commit amortises the
/// fork across every update that arrives in the same window — so the second
/// measurement drives the server with concurrent submitters and reports updates/s.
/// Run: cargo test -p sparq-server --release --test updates -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "benchmark — run explicitly in --release"]
async fn bench_update_latency_1m() {
    let n: usize = std::env::var("SPARQ_BENCH_TRIPLES").ok().and_then(|v| v.parse().ok()).unwrap_or(1_000_000);
    let t = Instant::now();
    let graph = synthetic_graph(n);
    eprintln!("graph: {} triples, built in {:.2?}", graph.len(), t.elapsed());

    // Reference: one engine-level rebuild — the same O(graph) cost the writer's
    // per-batch fork pays.
    let ins = "INSERT DATA { <http://ex/b0> <http://ex/q> <http://ex/b0> }";
    let t = Instant::now();
    let rebuilt = sparq_engine::update(&graph, ins).unwrap();
    let fork_engine = t.elapsed();
    eprintln!("reference  O(graph) rebuild (engine call):    {fork_engine:.2?}");
    drop(rebuilt);

    let base = spawn_with(graph, ServerConfig::default()).await;
    let cl = reqwest::Client::new();

    // Isolated updates: each is its own batch → ack latency ≈ window + fork.
    let reps = 10;
    let mut lat = Vec::with_capacity(reps);
    for i in 0..reps {
        let upd = format!("INSERT DATA {{ <http://ex/a{i}> <http://ex/q> <http://ex/a{i}> }}");
        let t = Instant::now();
        assert_eq!(post_update(&cl, &base, &upd).await, 204);
        lat.push(t.elapsed());
    }
    lat.sort();
    eprintln!(
        "isolated   update ack (HTTP, {reps} reps):        median {:.2?}  max {:.2?}   (fork-priced — the recorded A2 trade)",
        lat[reps / 2],
        lat[reps - 1]
    );

    // Concurrent updates: submitters share group-commit windows → the fork amortises.
    let conc = 32usize;
    let per_task = 8usize;
    let t = Instant::now();
    let mut tasks = Vec::new();
    for c in 0..conc {
        let base = base.clone();
        tasks.push(tokio::spawn(async move {
            let cl = reqwest::Client::new();
            for i in 0..per_task {
                let upd = format!("INSERT DATA {{ <http://ex/g{c}x{i}> <http://ex/q> <http://ex/g{c}x{i}> }}");
                assert_eq!(post_update(&cl, &base, &upd).await, 204);
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let took = t.elapsed();
    let total = conc * per_task;
    eprintln!(
        "concurrent {total} updates, {conc} submitters:        {took:.2?} total → {:.1} updates/s (group commit amortises the fork)",
        total as f64 / took.as_secs_f64()
    );
    assert_eq!(count_rows(&cl, &base, "SELECT ?s WHERE { ?s <http://ex/q> ?o }").await, reps + total);
}

// ---------------------------------------------------------------------------
// Per-named-graph cache invalidation scope (sq-uqh, Wave B)
// ---------------------------------------------------------------------------
//
// These exercise the END-TO-END epoch behaviour the cache hook depends on:
// `apply_update` extracts the per-named-graph visibility scope of the parsed update and
// publishes a generation that bumps ONLY those pods' epochs. A cached read records the
// epochs of the pods it touched and stays valid while they are unchanged — so the
// load-bearing properties are:
//   * a write to graph A does NOT bump graph B's epoch (a B-scoped cache entry survives);
//   * a cross-graph / default-graph / dynamically-scoped write DOES bump the global pod
//     (the catch-all every entry records — so it invalidates everything, never under).

/// The epoch of a pod in the current generation (0 = never touched).
fn epoch_of(state: &AppState, pod: &str) -> u64 {
    state.current().epochs().epoch(&PodId::new(pod))
}

/// A write scoped to graph A bumps ONLY graph A's epoch: graph B's epoch (and any cache
/// entry recording it) is untouched, while a B-scoped write later bumps B but still not A.
/// This is the whole point of finer-than-global tagging — no cross-graph invalidation churn.
#[test]
fn write_to_graph_a_does_not_invalidate_graph_b() {
    let state = AppState::with_config(synthetic_graph(100), ServerConfig::default());
    let pod_a = "http://ex/g/A";
    let pod_b = "http://ex/g/B";

    // Baseline: nothing touched yet.
    let (a0, b0, g0) = (epoch_of(&state, pod_a), epoch_of(&state, pod_b), epoch_of(&state, GLOBAL_POD));

    // A write that names ONLY graph A.
    state
        .apply_update("INSERT DATA { GRAPH <http://ex/g/A> { <http://ex/s> <http://ex/p> <http://ex/o> } }")
        .unwrap_or_else(|e| panic!("{e}"));

    let (a1, b1, g1) = (epoch_of(&state, pod_a), epoch_of(&state, pod_b), epoch_of(&state, GLOBAL_POD));
    assert_eq!(a1, a0 + 1, "the write to A must bump A's epoch");
    assert_eq!(b1, b0, "a write to A must NOT bump B's epoch — a B-scoped cache entry survives");
    assert_eq!(g1, g0, "a single-named-graph write must NOT bump the global pod");

    // A subsequent write naming ONLY graph B bumps B, still leaving A where it was.
    state
        .apply_update("INSERT DATA { GRAPH <http://ex/g/B> { <http://ex/s> <http://ex/p> <http://ex/o> } }")
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(epoch_of(&state, pod_b), b1 + 1, "the write to B must bump B's epoch");
    assert_eq!(epoch_of(&state, pod_a), a1, "the write to B must NOT re-bump A");
}

/// A cross-graph / unscopable write (here CLEAR ALL, and a default-graph INSERT) MUST bump
/// the global pod — the catch-all every cache entry records — so it invalidates everything.
/// Under-invalidating here would be a stale read; over-invalidating (the global bump) is the
/// honest conservative choice.
#[test]
fn cross_graph_and_default_writes_bump_global() {
    let state = AppState::with_config(synthetic_graph(100), ServerConfig::default());

    // (1) A default-graph INSERT DATA cannot be scoped finer than the catch-all → global.
    let g0 = epoch_of(&state, GLOBAL_POD);
    state
        .apply_update("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }")
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(epoch_of(&state, GLOBAL_POD), g0 + 1, "a default-graph write must bump global");

    // (2) A cross-graph CLEAR ALL is unbounded → global again.
    let g1 = epoch_of(&state, GLOBAL_POD);
    state.apply_update("CLEAR ALL").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(epoch_of(&state, GLOBAL_POD), g1 + 1, "CLEAR ALL must bump global");

    // (3) A DELETE/INSERT WHERE with a VARIABLE graph target is not statically scopable → global.
    let g2 = epoch_of(&state, GLOBAL_POD);
    state
        .apply_update("INSERT { GRAPH ?g { ?s ?p ?o } } WHERE { GRAPH ?g { ?s ?p ?o } }")
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(epoch_of(&state, GLOBAL_POD), g2 + 1, "a variable-graph-target write must bump global");
}

/// A write mixing a named-graph effect with a default-graph effect bumps BOTH the named pod
/// AND global in ONE publish — finer scoping is additive over the catch-all, so a cache
/// entry recording either (or both) is correctly invalidated.
#[test]
fn mixed_write_bumps_named_and_global_in_one_generation() {
    let state = AppState::with_config(synthetic_graph(100), ServerConfig::default());
    let pod_a = "http://ex/g/A";
    let n0 = state.current().number();
    let (a0, g0) = (epoch_of(&state, pod_a), epoch_of(&state, GLOBAL_POD));

    state
        .apply_update(
            "INSERT DATA { GRAPH <http://ex/g/A> { <http://ex/s> <http://ex/p> <http://ex/o> } \
                          <http://ex/d> <http://ex/p> <http://ex/o> }",
        )
        .unwrap_or_else(|e| panic!("{e}"));

    // One update => one published generation, but BOTH pods bumped within it.
    assert_eq!(state.current().number(), n0 + 1, "one update publishes exactly one generation");
    assert_eq!(epoch_of(&state, pod_a), a0 + 1, "the named-graph half must bump A");
    assert_eq!(epoch_of(&state, GLOBAL_POD), g0 + 1, "the default-graph half must bump global");
}
