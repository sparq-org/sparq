//! [OPUS-4.8] sq-2999l (gh-906): e2e integration tests for the OPT-IN CDC change-stream poll
//! endpoint (`GET /streams`) in the Amazon-Neptune-Streams `GetRecords` shape, over the merged
//! durable change-stream (sq-b4fns / #1223).
//!
//! These tests run ONLY with the `change-stream` cargo feature (the whole file is gated). They
//! spin the real axum server pointed at a durable log directory and drive the REAL path over HTTP:
//!   * the OPT-IN posture — `/streams` is `404` unless a log directory is configured;
//!   * the load-bearing invariant — write commits over the HTTP UPDATE path, GET `/streams` from
//!     offset N, and assert the EXACT ordered change sequence (op + quad + seq + generation) PLUS
//!     the continuation token (`nextSequenceNumber` / `hasMoreRecords`);
//!   * the iterator types (`TRIM_HORIZON`, `AT_SEQUENCE_NUMBER`, `AFTER_SEQUENCE_NUMBER`, `LATEST`);
//!   * `limit` paging + resume via the returned continuation token;
//!   * fail-closed `400` on a sequence-anchored iterator type with no anchor;
//!   * [OPUS-5] (sq-iz7ag) retention-awareness — a log whose oldest segments retention already
//!     dropped still replays under `TRIM_HORIZON` (from the horizon, never a `500`), and an
//!     `at`/`after` offset below the horizon is a `410` naming where to resume;
//!   * the read-auth gate (a poll is a READ);
//!   * durability — the records survive a fresh `ChangeLog::open` on the same directory (a restart).
#![cfg(feature = "change-stream")]

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

/// A unique scratch directory for one test's durable change-stream log.
fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sparq-streams-{}-{}-{:?}",
        tag,
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Boots the real server with the change-stream log at `dir` (or no log when `dir` is `None`, the
/// OFF case), optionally behind a read-auth token. Returns its base URL.
async fn spawn(dir: Option<std::path::PathBuf>, auth: Option<&str>) -> String {
    let graph = Graph::load_str("", "turtle").unwrap();
    let config = ServerConfig {
        change_stream_dir: dir,
        auth_token: auth.map(str::to_string),
        auth_token_read: auth.is_some(),
        ..ServerConfig::default()
    };
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// POSTs a SPARQL Update over the HTTP protocol; asserts a 2xx ack (the commit is published, and —
/// with the change-stream on — recorded).
async fn post_update(cl: &reqwest::Client, base: &str, update: &str) {
    let status = cl
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body(update.to_string())
        .send()
        .await
        .unwrap()
        .status();
    assert!(status.is_success(), "update failed: {status}: {update}");
}

/// GETs `/streams` with the given query parameters and returns the parsed JSON body + HTTP status.
async fn poll(
    cl: &reqwest::Client,
    base: &str,
    params: &[(&str, &str)],
) -> (reqwest::StatusCode, serde_json::Value) {
    let resp = cl
        .get(format!("{base}/streams"))
        .query(params)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp
        .json::<serde_json::Value>()
        .await
        .unwrap_or(serde_json::Value::Null);
    (status, body)
}

// ---------------------------------------------------------------------------
// OPT-IN posture.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn streams_404_when_not_configured() {
    let base = spawn(None, None).await;
    let resp = client()
        .get(format!("{base}/streams"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

// ---------------------------------------------------------------------------
// THE load-bearing invariant: write commits, GET from offset N, assert the EXACT change
// sequence + the continuation token.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn poll_from_offset_returns_exact_change_sequence_and_continuation() {
    let dir = scratch("exact");
    let base = spawn(Some(dir.clone()), None).await;
    let cl = client();

    // Three commits with real inserts/deletes across the default + a named graph.
    // C1 (gen 1, seq 0): +s1
    post_update(
        &cl,
        &base,
        "INSERT DATA { <http://ex/s1> <http://ex/p> <http://ex/o1> }",
    )
    .await;
    // C2 (gen 2, seq 1): +s2 (named graph), -s1
    post_update(
        &cl,
        &base,
        "DELETE DATA { <http://ex/s1> <http://ex/p> <http://ex/o1> };\n\
         INSERT DATA { GRAPH <http://ex/g> { <http://ex/s2> <http://ex/p> <http://ex/o2> } }",
    )
    .await;
    // C3 (gen 3, seq 2): +s3
    post_update(
        &cl,
        &base,
        "INSERT DATA { <http://ex/s3> <http://ex/p> <http://ex/o3> }",
    )
    .await;

    // Poll from offset 1 (AFTER seq 0): must return EXACTLY commits seq 1 and seq 2, in order.
    let (status, body) = poll(&cl, &base, &[("at", "1")]).await;
    assert_eq!(status, 200);
    assert_eq!(body["totalRecords"], 2, "two commits from seq 1: {body}");
    assert_eq!(body["lastSequenceNumber"], 2);
    assert_eq!(
        body["nextSequenceNumber"], 3,
        "resume token is one past the last"
    );
    assert_eq!(body["hasMoreRecords"], false);

    let records = body["records"].as_array().unwrap();
    // C2 flattened: +s2 (ADD, commitNum 1) and -s1 (REMOVE, commitNum 1); then C3: +s3 (commitNum 2).
    // The ADD precedes the REMOVE within a commit (ChangeRecord order: inserts then deletes).
    assert_eq!(records.len(), 3, "2 changes in C2 + 1 in C3: {body}");

    assert_eq!(records[0]["eventId"]["commitNum"], 1);
    assert_eq!(records[0]["op"], "ADD");
    assert_eq!(records[0]["generation"], 2);
    assert_eq!(
        records[0]["data"]["stmt"],
        "<http://ex/s2> <http://ex/p> <http://ex/o2> <http://ex/g> ."
    );

    assert_eq!(records[1]["eventId"]["commitNum"], 1);
    assert_eq!(records[1]["op"], "REMOVE");
    assert_eq!(
        records[1]["data"]["stmt"],
        "<http://ex/s1> <http://ex/p> <http://ex/o1> ."
    );

    assert_eq!(records[2]["eventId"]["commitNum"], 2);
    assert_eq!(records[2]["op"], "ADD");
    assert_eq!(records[2]["generation"], 3);
    assert_eq!(
        records[2]["data"]["stmt"],
        "<http://ex/s3> <http://ex/p> <http://ex/o3> ."
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn trim_horizon_replays_the_whole_stream() {
    let dir = scratch("horizon");
    let base = spawn(Some(dir.clone()), None).await;
    let cl = client();
    post_update(
        &cl,
        &base,
        "INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/b> }",
    )
    .await;
    post_update(
        &cl,
        &base,
        "INSERT DATA { <http://ex/c> <http://ex/p> <http://ex/d> }",
    )
    .await;

    // No params == TRIM_HORIZON (replay from seq 0).
    let (status, body) = poll(&cl, &base, &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body["totalRecords"], 2);
    assert_eq!(
        body["records"].as_array().unwrap()[0]["eventId"]["commitNum"],
        0
    );
    assert_eq!(body["nextSequenceNumber"], 2);

    // Explicit TRIM_HORIZON is identical.
    let (_, body2) = poll(&cl, &base, &[("iteratorType", "TRIM_HORIZON")]).await;
    assert_eq!(body2["totalRecords"], 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn after_sequence_number_resumes_past_the_anchor() {
    let dir = scratch("after");
    let base = spawn(Some(dir.clone()), None).await;
    let cl = client();
    for i in 0..3 {
        post_update(
            &cl,
            &base,
            &format!("INSERT DATA {{ <http://ex/s{i}> <http://ex/p> <http://ex/o{i}> }}"),
        )
        .await;
    }
    // AFTER seq 0 → start at seq 1 → commits 1 and 2.
    let (status, body) = poll(
        &cl,
        &base,
        &[("iteratorType", "AFTER_SEQUENCE_NUMBER"), ("after", "0")],
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["totalRecords"], 2);
    assert_eq!(
        body["records"].as_array().unwrap()[0]["eventId"]["commitNum"],
        1
    );

    // A bare ?after=1 infers AFTER_SEQUENCE_NUMBER → just commit 2.
    let (_, body2) = poll(&cl, &base, &[("after", "1")]).await;
    assert_eq!(body2["totalRecords"], 1);
    assert_eq!(
        body2["records"].as_array().unwrap()[0]["eventId"]["commitNum"],
        2
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn limit_pages_and_continuation_token_resumes() {
    let dir = scratch("paging");
    let base = spawn(Some(dir.clone()), None).await;
    let cl = client();
    for i in 0..5 {
        post_update(
            &cl,
            &base,
            &format!("INSERT DATA {{ <http://ex/s{i}> <http://ex/p> <http://ex/o{i}> }}"),
        )
        .await;
    }
    // Page 1: limit 2 from the start → commits 0,1; more available; resume at 2.
    let (_, page1) = poll(&cl, &base, &[("limit", "2")]).await;
    assert_eq!(page1["totalRecords"], 2);
    assert_eq!(page1["hasMoreRecords"], true);
    let next = page1["nextSequenceNumber"].as_u64().unwrap();
    assert_eq!(next, 2);

    // Page 2: resume via the continuation token (?at=next) → commits 2,3; more available.
    let (_, page2) = poll(&cl, &base, &[("at", &next.to_string()), ("limit", "2")]).await;
    assert_eq!(page2["totalRecords"], 2);
    assert_eq!(page2["hasMoreRecords"], true);
    assert_eq!(
        page2["records"].as_array().unwrap()[0]["eventId"]["commitNum"],
        2
    );
    let next2 = page2["nextSequenceNumber"].as_u64().unwrap();

    // Page 3: the tail → commit 4 only; no more.
    let (_, page3) = poll(&cl, &base, &[("at", &next2.to_string()), ("limit", "2")]).await;
    assert_eq!(page3["totalRecords"], 1);
    assert_eq!(page3["hasMoreRecords"], false);
    assert_eq!(page3["nextSequenceNumber"], 5);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn latest_tails_only_new_records() {
    let dir = scratch("latest");
    let base = spawn(Some(dir.clone()), None).await;
    let cl = client();
    post_update(
        &cl,
        &base,
        "INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/b> }",
    )
    .await;

    // LATEST starts at the current tail (next_seq == 1): nothing yet.
    let (status, body) = poll(&cl, &base, &[("iteratorType", "LATEST")]).await;
    assert_eq!(status, 200);
    assert_eq!(body["totalRecords"], 0);
    assert_eq!(
        body["nextSequenceNumber"], 1,
        "tail resumes at the same offset"
    );

    // A new commit then shows up from that offset.
    post_update(
        &cl,
        &base,
        "INSERT DATA { <http://ex/c> <http://ex/p> <http://ex/d> }",
    )
    .await;
    let (_, body2) = poll(&cl, &base, &[("at", "1")]).await;
    assert_eq!(body2["totalRecords"], 1);
    assert_eq!(
        body2["records"].as_array().unwrap()[0]["eventId"]["commitNum"],
        1
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Fail-closed + auth.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn anchored_iterator_without_anchor_is_400() {
    let dir = scratch("noanchor");
    let base = spawn(Some(dir.clone()), None).await;
    let resp = client()
        .get(format!("{base}/streams"))
        .query(&[("iteratorType", "AT_SEQUENCE_NUMBER")])
        .send()
        .await
        .unwrap();
    // Fail-closed: never a silent replay-all when the anchor is missing.
    assert_eq!(resp.status(), 400);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn poll_is_gated_by_read_auth() {
    let dir = scratch("auth");
    let base = spawn(Some(dir.clone()), Some("s3cret")).await;
    let cl = client();
    // No token → 401.
    let resp = cl.get(format!("{base}/streams")).send().await.unwrap();
    assert_eq!(resp.status(), 401);
    // Correct token → 200.
    let resp = cl
        .get(format!("{base}/streams"))
        .header("authorization", "Bearer s3cret")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// [FABLE-5] (gh-2436) POST /admin/change-stream/rebase — the operator resync of the RUNNING
// server's tracked log (an honest gap record; the recorder never stops).
// ---------------------------------------------------------------------------

/// POSTs `/admin/change-stream/rebase` with the given JSON body (`None` = empty body) and
/// returns the HTTP status + parsed JSON body.
async fn rebase(
    cl: &reqwest::Client,
    base: &str,
    body: Option<&str>,
) -> (reqwest::StatusCode, serde_json::Value) {
    let resp = cl
        .post(format!("{base}/admin/change-stream/rebase"))
        .body(body.unwrap_or("").to_string())
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp
        .json::<serde_json::Value>()
        .await
        .unwrap_or(serde_json::Value::Null);
    (status, body)
}

#[tokio::test]
async fn admin_rebase_404_when_not_configured() {
    let base = spawn(None, None).await;
    let (status, _) = rebase(&client(), &base, None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn admin_rebase_is_gated_by_write_auth() {
    let dir = scratch("rebase-auth");
    let base = spawn(Some(dir.clone()), Some("s3cret")).await;
    let cl = client();
    // No token → 401 (a rebase APPENDS to the durable log: the write/admin gate).
    let resp = cl
        .post(format!("{base}/admin/change-stream/rebase"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    // Correct token → 200.
    let resp = cl
        .post(format!("{base}/admin/change-stream/rebase"))
        .header("authorization", "Bearer s3cret")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let _ = std::fs::remove_dir_all(&dir);
}

/// THE e2e invariant (gh-2436): an empty-body rebase on the RUNNING server appends the gap
/// record at the writer's current generation, `GET /streams` renders it as an explicit
/// `"op": "REBASE"` marker (never silently flattened away), and recording chains forward —
/// all without restarting the server or the recorder.
#[tokio::test]
async fn admin_rebase_appends_a_visible_gap_and_the_stream_resumes() {
    let dir = scratch("rebase-live");
    let base = spawn(Some(dir.clone()), None).await;
    let cl = client();

    // Explicit starting baseline at the current generation (0, nothing committed yet).
    let (status, body) = rebase(&cl, &base, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["seq"], 0);
    assert_eq!(body["generation"], 0);
    assert_eq!(body["rebase"], true);
    assert_eq!(body["nextSequenceNumber"], 1);

    // The recorder never stopped: the next commit chains from the declared baseline.
    post_update(
        &cl,
        &base,
        "INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/b> }",
    )
    .await;

    let (status, body) = poll(&cl, &base, &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body["totalRecords"], 2, "{body}");
    let records = body["records"].as_array().unwrap();
    assert_eq!(records.len(), 2, "one REBASE marker + one ADD: {body}");
    assert_eq!(records[0]["op"], "REBASE");
    assert_eq!(records[0]["eventId"]["commitNum"], 0);
    assert_eq!(records[0]["generation"], 0);
    assert!(
        records[0].get("data").is_none(),
        "a gap marker carries no quad: {body}"
    );
    assert_eq!(records[1]["op"], "ADD");
    assert_eq!(records[1]["generation"], 1);
    assert_eq!(body["nextSequenceNumber"], 2);

    let _ = std::fs::remove_dir_all(&dir);
}

/// The fail-closed posture over HTTP: a same-lineage rebase that does not move the stream
/// forward is a 409 (never a silent baseline rewrite); the explicit `newLineage` assertion
/// (the post-restore case, where the ring's numbering restarts) is what overrides it; an
/// unknown body field is a 400 (never a silently-defaulted typo).
#[tokio::test]
async fn admin_rebase_non_forward_is_409_unless_new_lineage() {
    let dir = scratch("rebase-409");
    let base = spawn(Some(dir.clone()), None).await;
    let cl = client();
    post_update(
        &cl,
        &base,
        "INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/b> }",
    )
    .await;

    // Generation 1 is already the recorded baseline: not forward → 409, nothing appended.
    let (status, _) = rebase(&cl, &base, Some(r#"{"generation": 1}"#)).await;
    assert_eq!(status, 409);
    // An unrecognised field never silently defaults → 400.
    let (status, _) = rebase(&cl, &base, Some(r#"{"lineage": true}"#)).await;
    assert_eq!(status, 400);

    // The operator asserts a replaced lineage → the same target is accepted as a gap.
    let (status, body) = rebase(&cl, &base, Some(r#"{"generation": 1, "newLineage": true}"#)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["seq"], 1);
    assert_eq!(body["rebase"], true);

    // Recording chains on: commit 2 lands after the gap.
    post_update(
        &cl,
        &base,
        "INSERT DATA { <http://ex/c> <http://ex/p> <http://ex/d> }",
    )
    .await;
    let (_, body) = poll(&cl, &base, &[]).await;
    assert_eq!(body["totalRecords"], 3, "{body}");
    let records = body["records"].as_array().unwrap();
    assert_eq!(records[1]["op"], "REBASE");
    assert_eq!(records[2]["op"], "ADD");

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Durability: the records persist on disk (a fresh ChangeLog::open on the same dir sees them).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn recorded_changes_are_durable_on_disk() {
    let dir = scratch("durable");
    let base = spawn(Some(dir.clone()), None).await;
    let cl = client();
    post_update(
        &cl,
        &base,
        "INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/b> }",
    )
    .await;
    post_update(
        &cl,
        &base,
        "INSERT DATA { <http://ex/c> <http://ex/p> <http://ex/d> }",
    )
    .await;

    // A FRESH log handle opened on the same directory (a process restart) re-reads the segments
    // and replays the same two records — the on-disk log is the durable source of truth.
    let reopened = sparq_serve::ChangeLog::open(&dir).expect("reopen durable log");
    assert_eq!(reopened.next_seq(), 2, "resumes at the recorded seq");
    let all = reopened.poll(0).expect("poll all");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].seq, 0);
    assert_eq!(all[1].seq, 1);

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// [GPT-5.6] (sq-kqofk) Concurrency: concurrent updates may share a group-committed generation;
// the writer-thread hook records one gapless, monotonic stream record per published generation.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_updates_record_a_gapless_monotonic_stream() {
    let dir = scratch("concurrent");
    let base = spawn(Some(dir.clone()), None).await;

    // Fire 12 distinct updates concurrently from 4 tasks. Group commit may place several updates
    // in one generation, but every inserted quad must appear exactly once in the stream.
    const N: usize = 12;
    let mut handles = Vec::new();
    for i in 0..N {
        let base = base.clone();
        handles.push(tokio::spawn(async move {
            let cl = client();
            post_update(
                &cl,
                &base,
                &format!("INSERT DATA {{ <http://ex/s{i}> <http://ex/p> <http://ex/o{i}> }}"),
            )
            .await;
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    // Poll the whole stream: one record per PUBLISHED generation (not per submitter), gapless seqs,
    // strictly increasing generations, and exactly N flattened quad changes.
    let cl = client();
    let (status, body) = poll(&cl, &base, &[("limit", "1000")]).await;
    assert_eq!(status, 200);
    let commit_count = body["totalRecords"].as_u64().unwrap();
    assert!(
        (1..=N as u64).contains(&commit_count),
        "every published generation is recorded: {body}"
    );
    let records = body["records"].as_array().unwrap();
    assert_eq!(records.len(), N, "every concurrent insert is recorded once");
    let mut commit_generations = std::collections::BTreeMap::new();
    for record in records {
        let commit = record["eventId"]["commitNum"].as_u64().unwrap();
        let generation = record["generation"].as_u64().unwrap();
        assert!(
            commit < commit_count,
            "commit seq stays inside the gapless range: {body}"
        );
        if let Some(previous) = commit_generations.insert(commit, generation) {
            assert_eq!(
                previous, generation,
                "one commit seq identifies one generation: {body}"
            );
        }
    }
    assert_eq!(commit_generations.len() as u64, commit_count);
    let mut previous_generation = 0;
    for (expected_seq, (&seq, &generation)) in commit_generations.iter().enumerate() {
        assert_eq!(seq, expected_seq as u64, "commit seqs are gapless: {body}");
        assert!(
            generation > previous_generation,
            "published generations are strictly increasing: {body}"
        );
        previous_generation = generation;
    }
    assert_eq!(body["nextSequenceNumber"], commit_count);

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// [OPUS-5] (sq-iz7ag) Retention-aware TRIM_HORIZON — a log whose oldest segments retention
// already dropped (sq-n9s4d) still replays, and a dropped offset is a caller-recoverable 410.
// ---------------------------------------------------------------------------

/// Builds a durable change log at `dir` holding three recorded commits ONE RECORD PER SEGMENT
/// (so retention can drop whole segments), then trims every non-active segment away. Returns the
/// resulting trim horizon — the oldest seq still retained — and asserts the trim really happened,
/// so a test built on it cannot pass vacuously against an untrimmed log.
fn seed_and_trim(dir: &std::path::Path) -> u64 {
    use sparq_serve::{ChangeLog, ChangeLogConfig, GenerationRing, PodId, RetentionPolicy};

    let load = |nq: &str| Graph::load_dataset(nq, "nquads").expect("seed parses");
    let pod = PodId::new("http://ex/g");
    let ring: GenerationRing<Graph> =
        GenerationRing::new(load("<http://ex/s0> <http://ex/p> <http://ex/o0> .\n"));
    let g0 = ring.current();
    let g1 = ring.publish(load("<http://ex/s1> <http://ex/p> <http://ex/o1> .\n"), [pod.clone()]);
    let g2 = ring.publish(load("<http://ex/s2> <http://ex/p> <http://ex/o2> .\n"), [pod.clone()]);
    let g3 = ring.publish(load("<http://ex/s3> <http://ex/p> <http://ex/o3> .\n"), [pod]);
    {
        let mut log = ChangeLog::open_with_config(
            dir,
            ChangeLogConfig {
                segment_target_bytes: 1,
                fsync: false,
            },
        )
        .expect("open log");
        log.record_commit(&g0, &g1).expect("record 0->1");
        log.record_commit(&g1, &g2).expect("record 1->2");
        log.record_commit(&g2, &g3).expect("record 2->3");
    }
    let mut log = ChangeLog::open(dir).expect("reopen log");
    let report = log
        .apply_retention(&RetentionPolicy {
            max_total_bytes: Some(0),
            ..RetentionPolicy::default()
        })
        .expect("retention");
    assert!(report.segments_dropped > 0, "the test needs a REAL trim");
    assert!(report.first_retained_seq > 0, "seq 0 must be gone");
    report.first_retained_seq
}

/// THE e2e invariant (sq-iz7ag): pointed at an ALREADY-trimmed log, `GET /streams` must serve
/// `TRIM_HORIZON` from the horizon — the whole point of the iterator type is "replay everything
/// RETAINED", and resolving it to seq 0 polls below the log's earliest surviving record, which
/// the durable log rejects fail-closed, i.e. a `500` on every trimmed log. An `at`/`after` anchor
/// below the horizon stays fail-closed but surfaces as a `410` naming the horizon (the consumer
/// re-bootstraps) rather than as that opaque `500` — and never as a silent restart at the horizon.
#[tokio::test]
async fn trim_horizon_replays_a_trimmed_log_and_a_dropped_offset_is_410() {
    let dir = scratch("trimmed");
    let horizon = seed_and_trim(&dir);
    // The server opens the already-trimmed directory (the restart-after-retention case).
    let base = spawn(Some(dir.clone()), None).await;
    let cl = client();

    // TRIM_HORIZON: a 200 replaying from the horizon, not a 500 from polling below it.
    let (status, body) = poll(&cl, &base, &[("iteratorType", "TRIM_HORIZON")]).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["totalRecords"], 3 - horizon, "{body}");
    assert_eq!(body["records"][0]["eventId"]["commitNum"], horizon, "{body}");
    assert_eq!(body["lastSequenceNumber"], 2, "{body}");
    assert_eq!(body["nextSequenceNumber"], 3, "{body}");
    assert_eq!(body["hasMoreRecords"], false, "{body}");

    // No `iteratorType` at all defaults to TRIM_HORIZON — same answer, still not a 500.
    let (status, body) = poll(&cl, &base, &[]).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["records"][0]["eventId"]["commitNum"], horizon, "{body}");

    // An anchor BELOW the horizon: 410 Gone naming the horizon to resume from.
    let (status, body) = poll(&cl, &base, &[("at", "0")]).await;
    assert_eq!(status, 410, "{body}");
    let message = body["error"].as_str().unwrap_or_default();
    assert!(message.contains("trim horizon"), "{message}");
    assert!(message.contains(&horizon.to_string()), "{message}");
    // ...and it does NOT silently restart at the horizon: no records were served.
    assert!(body.get("records").is_none(), "{body}");

    // The boundary is the RESOLVED start: `after=horizon-1` starts exactly AT the horizon, so
    // nothing was dropped from under that consumer and the poll succeeds.
    let anchor = (horizon - 1).to_string();
    let (status, body) = poll(&cl, &base, &[("after", anchor.as_str())]).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["records"][0]["eventId"]["commitNum"], horizon, "{body}");

    // LATEST still tails the log rather than replaying the retained records.
    let (status, body) = poll(&cl, &base, &[("iteratorType", "LATEST")]).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["totalRecords"], 0, "{body}");
    assert_eq!(body["nextSequenceNumber"], 3, "{body}");

    let _ = std::fs::remove_dir_all(&dir);
}
