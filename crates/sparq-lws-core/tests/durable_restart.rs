//! Durable native HTTP restart and crash invariants. Each case owns its data directory and origin.
#![cfg(all(unix, feature = "embedded-sparq", feature = "blob-fs"))]

mod common;
#[path = "common/durable.rs"]
mod durable;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use durable::{
    inspect_blobs, payload, split_member, success, Corpus, Fixture, Mode, Snapshot, LWS_JSON,
    OWNER, READER,
};
use reqwest::Method;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TEST_DEADLINE: Duration = Duration::from_secs(240);
const BINARY: &str = "application/octet-stream";

async fn bounded(work: impl std::future::Future<Output = ()>) {
    tokio::time::timeout(TEST_DEADLINE, work)
        .await
        .expect("durability test exceeded deadline");
}

async fn clean_restart(mode: Mode) {
    let f = Fixture::new(mode).await;
    let server = f.boot().await;
    let corpus = Corpus::write(&f).await;
    server.stop();
    inspect_blobs(&f, false).await;
    let server = f.boot().await;
    corpus.verify(&f).await;
    server.stop();
    inspect_blobs(&f, false).await;
}

#[tokio::test]
async fn clean_restart_solid() {
    bounded(clean_restart(Mode::Solid)).await;
}
#[tokio::test]
async fn clean_restart_lws() {
    bounded(clean_restart(Mode::Lws)).await;
}

#[derive(Clone, Debug)]
struct Write {
    path: String,
    body: Vec<u8>,
    delete: bool,
    create: bool,
    partial: bool,
    prior: Option<Snapshot>,
}

#[derive(Debug)]
struct Ack {
    hash: [u8; 32],
    etag: Option<String>,
}

#[derive(Debug)]
enum Progress {
    Partial(usize),
    Complete(usize, Result<Ack, String>),
}

// Independent check of the composite store's documented content/type ETag encoding. Do not
// derive expected bytes or tags from an index lookup: an index with a wrong key must fail here.
fn body_etag(body: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for b in BINARY.bytes().chain([0xff]).chain(body.iter().copied()) {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    format!("\"{}-{hash}\"", body.len())
}

async fn members(f: &Fixture, path: &str) -> BTreeSet<String> {
    let snapshot = f.snapshot(path).await;
    assert_eq!(snapshot.status, 200);
    if f.mode == Mode::Lws {
        let doc: Value = serde_json::from_slice(&snapshot.body).unwrap();
        assert_eq!(
            doc["totalItems"].as_u64().unwrap() as usize,
            doc["items"].as_array().unwrap().len(),
            "test must inspect the entire listing"
        );
        doc["items"]
            .as_array()
            .expect("LWS listing items")
            .iter()
            .map(|m| m["id"].as_str().unwrap().to_string())
            .collect()
    } else {
        use sparq_lws_core::ldp::content::{parse_to_triples, RdfFormat};
        parse_to_triples(
            RdfFormat::Turtle,
            &snapshot.body,
            &format!("{}{path}", f.base),
        )
        .unwrap()
        .into_iter()
        .filter(|t| t.predicate.as_str() == "http://www.w3.org/ns/ldp#contains")
        .map(|t| match t.object {
            oxrdf::Term::NamedNode(n) => n.into_string(),
            other => panic!("containment member is not an IRI: {other}"),
        })
        .collect()
    }
}

async fn crash(mode: Mode) {
    let f = Fixture::new(mode).await;
    let server = f.boot().await;
    for i in 0..4 {
        f.container(&format!("/c{i}/")).await;
    }
    let mut writes = Vec::new();
    for id in 0..200 {
        // 160 normal requests plus 40 real HTTP requests held mid-body. This makes the kill
        // point deterministic even on a fast filesystem; no timer guesses when work is active.
        let partial = id >= 160;
        let delete = !partial && id % 5 == 0;
        let create = partial || id % 5 >= 2;
        let path = format!("/c{}/item-{id}.bin", id % 4);
        let prior = if !create {
            f.put(&path, payload(id + 300, 257), BINARY).await;
            Some(f.snapshot(&path).await)
        } else {
            None
        };
        let length = if partial {
            4 * 1024 * 1024
        } else {
            [0, 1, 8193, 131_073, 3 * 1024 * 1024][(id / 5) % 5]
        };
        writes.push(Write {
            path,
            body: if delete { vec![] } else { payload(id, length) },
            delete,
            create,
            partial,
            prior,
        });
    }
    assert_eq!(
        writes
            .iter()
            .map(|w| &w.path)
            .collect::<BTreeSet<_>>()
            .len(),
        200,
        "each IRI must have exactly one writer"
    );
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(201));
    let (send, mut progress) = tokio::sync::mpsc::unbounded_channel();
    let mut tasks = tokio::task::JoinSet::new();
    let active_partial = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let killed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    for (id, w) in writes.iter().cloned().enumerate() {
        let send = send.clone();
        let active_partial = active_partial.clone();
        let killed = killed.clone();
        let barrier = barrier.clone();
        let request = if w.delete {
            f.request(Method::DELETE, &w.path)
        } else {
            f.write_request(&w.path, w.create)
                .header("Content-Type", BINARY)
                .body(w.body.clone())
        };
        let address = f.base.strip_prefix("http://").unwrap().to_string();
        let (target, slug) = if mode == Mode::Lws {
            split_member(&w.path)
        } else {
            (w.path.clone(), String::new())
        };
        let token = f.token(OWNER);
        tasks.spawn(async move {
            barrier.wait().await;
            if w.partial {
                let mut stream = tokio::net::TcpStream::connect(&address).await.unwrap();
                let method = if mode == Mode::Lws { "POST" } else { "PUT" };
                let auth = if mode == Mode::Lws { format!("Authorization: Bearer {token}\r\n") } else { String::new() };
                let header = format!("{method} {target} HTTP/1.1\r\nHost: {address}\r\n{auth}Slug: {slug}\r\nContent-Type: {BINARY}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", w.body.len());
                stream.write_all(header.as_bytes()).await.unwrap();
                stream.write_all(&w.body[..4096]).await.unwrap();
                stream.flush().await.unwrap();
                active_partial.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                send.send(Progress::Partial(id)).unwrap();
                // Retain the open request until SIGKILL closes it; receiving a successful response
                // to an incomplete body would itself be a failure. The outer test bounds this wait.
                let mut response = Vec::new();
                let result = stream.read_to_end(&mut response).await;
                active_partial.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                assert!(killed.load(std::sync::atomic::Ordering::SeqCst), "request ended before SIGKILL: {response:?}");
                assert!(!response.starts_with(b"HTTP/1.1 2"), "partial body acknowledged: {response:?}");
                let _ = result;
                return;
            }
            let result = async {
                let response = request.send().await.map_err(|e| e.to_string())?;
                if !response.status().is_success() {
                    return Err(format!("{}: {}", response.status(), response.text().await.unwrap()));
                }
                let etag = response.headers().get("etag").map(|h| h.to_str().unwrap().to_string());
                if !w.delete {
                    assert_eq!(etag.as_deref(), Some(body_etag(&w.body).as_str()), "acknowledged ETag: {}", w.path);
                    if mode == Mode::Lws && w.create {
                        assert_eq!(response.headers()["location"].to_str().unwrap(),
                            format!("http://{address}{}", w.path), "Slug must select the intended IRI");
                    }
                }
                // Complete the exchange before declaring it acknowledged to the controller.
                response.bytes().await.map_err(|e| e.to_string())?;
                Ok(Ack { hash: Sha256::digest(&w.body).into(), etag })
            }.await;
            let _ = send.send(Progress::Complete(id, result));
        });
    }
    drop(send);
    barrier.wait().await;
    let mut acknowledgements = BTreeMap::new();
    let mut partials = BTreeSet::new();
    let mut errors = BTreeMap::new();
    while acknowledgements.len() < 100 || partials.len() < 40 {
        match progress
            .recv()
            .await
            .expect("writers ended before kill threshold")
        {
            Progress::Partial(id) => {
                assert!(partials.insert(id));
            }
            Progress::Complete(id, Ok(ack)) => {
                assert!(acknowledgements.insert(id, ack).is_none());
            }
            Progress::Complete(id, Err(error)) => {
                errors.insert(id, error);
            }
        }
    }
    assert_eq!(
        partials.len(),
        40,
        "must kill with HTTP requests still in flight"
    );
    assert!(acknowledgements.len() >= 100 && acknowledgements.len() < 200);
    assert!(
        acknowledgements.keys().any(|id| writes[*id].delete),
        "acknowledge a delete before killing"
    );
    assert!(
        acknowledgements
            .keys()
            .any(|id| !writes[*id].create && !writes[*id].delete),
        "acknowledge an overwrite before killing"
    );
    assert!(
        errors.is_empty(),
        "requests failed before the kill: {errors:?}"
    );
    assert_eq!(
        active_partial.load(std::sync::atomic::Ordering::SeqCst),
        40,
        "all partial requests must still be active at kill"
    );
    killed.store(true, std::sync::atomic::Ordering::SeqCst);
    server.kill();
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    // Include every 2xx racing the kill, not only events consumed before the signal.
    while let Ok(event) = progress.try_recv() {
        if let Progress::Complete(id, Ok(ack)) = event {
            assert!(acknowledgements.insert(id, ack).is_none());
        }
    }
    eprintln!(
        "{mode:?}: SIGKILL with {} acknowledged writes, {} incomplete HTTP bodies",
        acknowledgements.len(),
        partials.len()
    );
    let server = f.boot().await;
    let mut readable = BTreeMap::<String, BTreeSet<String>>::new();
    for (id, w) in writes.iter().enumerate() {
        let snapshot = f.snapshot(&w.path).await;
        let ack = acknowledgements.get(&id);
        if let Some(ack) = ack {
            if w.delete {
                assert_eq!(snapshot.status, 404, "acknowledged delete {}", w.path);
            } else {
                assert_eq!(
                    snapshot.status, 200,
                    "acknowledged write missing: {}",
                    w.path
                );
                assert_eq!(
                    <[u8; 32]>::from(Sha256::digest(&snapshot.body)),
                    ack.hash,
                    "acknowledged body {}",
                    w.path
                );
                assert_eq!(snapshot.body, w.body, "acknowledged bytes {}", w.path);
                assert_eq!(snapshot.etag, ack.etag, "acknowledged ETag {}", w.path);
            }
        } else if snapshot.status != 404 {
            assert_eq!(snapshot.status, 200, "unacknowledged {}", w.path);
            let old = w
                .prior
                .as_ref()
                .is_some_and(|old| old.body == snapshot.body && old.etag == snapshot.etag);
            let new = !w.delete
                && snapshot.body == w.body
                && snapshot.etag.as_deref() == Some(body_etag(&w.body).as_str());
            assert!(
                old || new,
                "unacknowledged write is neither complete old nor complete new: {}",
                w.path
            );
        } else if !w.create && !w.delete {
            panic!(
                "unacknowledged overwrite lost both old and new state: {}",
                w.path
            );
        }
        if snapshot.status == 200 {
            let (parent, _) = split_member(&w.path);
            readable
                .entry(parent)
                .or_default()
                .insert(format!("{}{}", f.base, w.path));
        }
    }
    for i in 0..4 {
        let parent = format!("/c{i}/");
        let listing = members(&f, &parent).await;
        assert_eq!(
            listing,
            readable.remove(&parent).unwrap_or_default(),
            "containment both directions: {parent}"
        );
        for iri in listing {
            assert_eq!(
                f.snapshot(iri.strip_prefix(&f.base).unwrap()).await.status,
                200,
                "listed member unreadable: {iri}"
            );
        }
    }
    server.stop();
    inspect_blobs(&f, true).await;
    let server = f.boot().await;
    // HTTP reads after GC catch removal hidden by a warm body cache or by comparing only key names.
    for (id, ack) in &acknowledgements {
        let w = &writes[*id];
        let s = f.snapshot(&w.path).await;
        if w.delete {
            assert_eq!(s.status, 404);
        } else {
            assert_eq!(s.status, 200);
            assert_eq!(s.body, w.body);
            assert_eq!(s.etag, ack.etag);
        }
    }
    server.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigkill_200_writes_solid() {
    bounded(crash(Mode::Solid)).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigkill_200_writes_lws() {
    bounded(crash(Mode::Lws)).await;
}

async fn grant_and_webhook(kill: bool) {
    use axum::{extract::State, http::StatusCode, routing::post, Router};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let inbox = format!("http://{}/inbox", listener.local_addr().unwrap());
    let (delivered, mut deliveries) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let app = Router::new()
        .route(
            "/inbox",
            post(
                |State(delivered): State<tokio::sync::mpsc::UnboundedSender<Value>>,
                 headers: axum::http::HeaderMap,
                 body: bytes::Bytes| async move {
                    assert!(
                        headers.contains_key("signature") && headers.contains_key("content-digest")
                    );
                    delivered
                        .send(serde_json::from_slice(&body).unwrap())
                        .unwrap();
                    StatusCode::NO_CONTENT
                },
            ),
        )
        .with_state(delivered);
    let receiver = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    // Aborting on unwind also releases the receiver's socket.
    struct Abort(tokio::task::JoinHandle<()>);
    impl Drop for Abort {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _receiver = Abort(receiver);
    let f = Fixture::new(Mode::Lws).await;
    let server = f.boot().await;
    f.put("/topic.bin", b"before restart".to_vec(), BINARY)
        .await;
    assert_eq!(
        f.request_as(Method::GET, "/topic.bin", Some(READER))
            .send()
            .await
            .unwrap()
            .status(),
        403,
        "reader must depend on the persisted grant"
    );
    let grant = f.grant_read("/topic.bin").await;
    let subscription = success(f.request_as(Method::POST, "/.lws/subscriptions/", Some(READER))
        .header("Content-Type", LWS_JSON)
        .json(&json!({"type": "WebhookSubscription", "topic": [format!("{}/topic.bin", f.base)], "inbox": inbox}))
        .send().await.unwrap(), "subscription").await;
    assert_eq!(subscription.status(), 201);
    let subscription = subscription.headers()["location"]
        .to_str()
        .unwrap()
        .to_string();
    if kill {
        server.kill();
    } else {
        server.stop();
    }
    let server = f.boot().await;
    assert_eq!(
        f.snapshot(grant.strip_prefix(&f.base).unwrap())
            .await
            .status,
        200
    );
    assert_eq!(
        f.request_as(
            Method::GET,
            subscription.strip_prefix(&f.base).unwrap(),
            Some(READER)
        )
        .send()
        .await
        .unwrap()
        .status(),
        200
    );
    assert_eq!(
        f.request_as(Method::GET, "/topic.bin", Some(READER))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let body = if kill {
        b"after SIGKILL".to_vec()
    } else {
        b"after SIGTERM".to_vec()
    };
    f.put("/topic.bin", body.clone(), BINARY).await;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let delivery = deliveries.recv().await.expect("webhook receiver stopped");
            if delivery["activity"]["type"]
                .as_array()
                .is_some_and(|types| types.contains(&json!("Update")))
                && delivery["activity"]["object"]["id"] == format!("{}/topic.bin", f.base)
            {
                break;
            }
        }
    })
    .await
    .expect("persisted subscription did not deliver the post-restart Update");
    let result = Snapshot::read(
        f.request_as(Method::GET, "/topic.bin", Some(READER))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(result.status, 200);
    assert_eq!(result.body, body);
    // Revocation is a negative control: authentication/ownership/open mode cannot explain the read.
    success(
        f.request(Method::DELETE, grant.strip_prefix(&f.base).unwrap())
            .send()
            .await
            .unwrap(),
        "revoke grant",
    )
    .await;
    assert_eq!(
        f.request_as(Method::GET, "/topic.bin", Some(READER))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    server.stop();
}

#[tokio::test]
async fn grants_and_webhooks_survive_sigterm() {
    bounded(grant_and_webhook(false)).await;
}
#[tokio::test]
async fn grants_and_webhooks_survive_sigkill() {
    bounded(grant_and_webhook(true)).await;
}
