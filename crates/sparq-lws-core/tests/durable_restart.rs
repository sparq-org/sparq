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
    let mut listings = BTreeMap::new();
    for path in ["/corpus/", "/corpus/nested/", "/corpus/nested/deep/"] {
        let listing = members(&f, path).await;
        assert!(
            !listing.is_empty(),
            "clean corpus must witness containment: {path}"
        );
        listings.insert(path, listing);
    }
    server.stop();
    inspect_blobs(&f, false).await;
    let server = f.boot().await;
    corpus.verify(&f).await;
    for (path, expected) in listings {
        assert_eq!(
            members(&f, path).await,
            expected,
            "restart changed membership: {path}"
        );
    }
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
    Complete(usize, Result<Ack, Failure>),
}

#[derive(Debug)]
struct Failure {
    message: String,
    before_kill: bool,
}

// Record the status/headers as soon as they arrive. A truncated response body cannot turn a
// received 2xx into an unacknowledged mutation. The return value describes body drainage only.
async fn send_write(
    id: usize,
    request: reqwest::RequestBuilder,
    write: &Write,
    mode: Mode,
    base: &str,
    killed: &std::sync::atomic::AtomicBool,
    progress: &tokio::sync::mpsc::UnboundedSender<Progress>,
) -> bool {
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            progress
                .send(Progress::Complete(
                    id,
                    Err(Failure {
                        message: format!("{error:?}"),
                        before_kill: !killed.load(std::sync::atomic::Ordering::SeqCst),
                    }),
                ))
                .unwrap();
            return false;
        }
    };
    if !response.status().is_success() {
        progress
            .send(Progress::Complete(
                id,
                Err(Failure {
                    message: format!("{}: {}", write.path, response.status()),
                    before_kill: !killed.load(std::sync::atomic::Ordering::SeqCst),
                }),
            ))
            .unwrap();
        return false;
    }
    let etag = response
        .headers()
        .get("etag")
        .map(|h| h.to_str().unwrap().to_string());
    if !write.delete {
        assert_eq!(
            etag.as_deref(),
            Some(body_etag(&write.body).as_str()),
            "acknowledged ETag: {}",
            write.path
        );
        if mode == Mode::Lws && write.create {
            assert_eq!(
                response.headers()["location"].to_str().unwrap(),
                format!("{base}{}", write.path),
                "Slug must select the intended IRI"
            );
        }
    }
    progress
        .send(Progress::Complete(
            id,
            Ok(Ack {
                hash: Sha256::digest(&write.body).into(),
                etag,
            }),
        ))
        .unwrap();
    response.bytes().await.is_ok()
}

fn record_completion(
    id: usize,
    result: Result<Ack, Failure>,
    acknowledgements: &mut BTreeMap<usize, Ack>,
) {
    match result {
        Ok(ack) => assert!(acknowledgements.insert(id, ack).is_none()),
        Err(failure) => assert!(
            !failure.before_kill,
            "request failed before kill: writer {id}: {}",
            failure.message
        ),
    }
}

fn drain_completions(
    progress: &mut tokio::sync::mpsc::UnboundedReceiver<Progress>,
    acknowledgements: &mut BTreeMap<usize, Ack>,
) {
    while let Ok(event) = progress.try_recv() {
        if let Progress::Complete(id, result) = event {
            record_completion(id, result, acknowledgements);
        }
    }
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
    listing_members(f.mode, &format!("{}{path}", f.base), &snapshot)
}

fn listing_members(mode: Mode, parent: &str, snapshot: &Snapshot) -> BTreeSet<String> {
    assert_eq!(snapshot.status, 200);
    if mode == Mode::Lws {
        let doc: Value = serde_json::from_slice(&snapshot.body).unwrap();
        assert_eq!(doc["id"].as_str(), Some(parent), "container identity");
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
        parse_to_triples(RdfFormat::Turtle, &snapshot.body, parent)
            .unwrap()
            .into_iter()
            .filter(|t| t.predicate.as_str() == "http://www.w3.org/ns/ldp#contains")
            .map(|t| {
                assert_eq!(
                    t.subject.to_string(),
                    format!("<{parent}>"),
                    "containment subject"
                );
                match t.object {
                    oxrdf::Term::NamedNode(n) => n.into_string(),
                    other => panic!("containment member is not an IRI: {other}"),
                }
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
            let body = payload(id + 300, 257);
            f.put(&path, body.clone(), BINARY).await;
            let prior = f.snapshot(&path).await;
            prior.assert_resource(&path, mode);
            assert_eq!(prior.body, body, "pre-existing bytes: {path}");
            assert_eq!(
                prior.etag.as_deref(),
                Some(body_etag(&body).as_str()),
                "pre-existing ETag: {path}"
            );
            Some(prior)
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
                let header = format!("{method} {target} HTTP/1.1\r\nHost: {address}\r\n{auth}Slug: {slug}\r\nContent-Type: {BINARY}\r\nContent-Length: {}\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n", w.body.len());
                stream.write_all(header.as_bytes()).await.unwrap();
                // Hyper sends 100 only once the handler polls this request body. A socket write
                // alone could sit in the accept queue and would not prove server-side activity.
                let mut interim = Vec::new();
                while !interim.ends_with(b"\r\n\r\n") {
                    interim.push(stream.read_u8().await.unwrap());
                    assert!(interim.len() < 4096, "oversized interim response");
                }
                assert_eq!(interim, b"HTTP/1.1 100 Continue\r\n\r\n");
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
            send_write(id, request, &w, mode, &format!("http://{address}"), &killed, &send).await;
        });
    }
    drop(send);
    barrier.wait().await;
    let mut acknowledgements = BTreeMap::new();
    let mut partials = BTreeSet::new();
    while acknowledgements.len() < 100 || partials.len() < 40 {
        match progress
            .recv()
            .await
            .expect("writers ended before kill threshold")
        {
            Progress::Partial(id) => {
                assert!(partials.insert(id));
            }
            Progress::Complete(id, result) => {
                record_completion(id, result, &mut acknowledgements);
                if acknowledgements.len() % 25 == 0 {
                    eprintln!(
                        "{mode:?}: {} writes acknowledged, {} bodies in flight",
                        acknowledgements.len(),
                        partials.len()
                    );
                }
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
    drain_completions(&mut progress, &mut acknowledgements);
    eprintln!(
        "{mode:?}: SIGKILL with {} acknowledged writes, {} incomplete HTTP bodies",
        acknowledgements.len(),
        partials.len()
    );
    let server = f.boot().await;
    let mut readable = BTreeMap::<String, BTreeSet<String>>::new();
    let mut recovered = BTreeMap::new();
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
        recovered.insert(w.path.clone(), snapshot);
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
    for (path, expected) in recovered {
        assert_eq!(
            f.snapshot(&path).await,
            expected,
            "GC changed recovered {path}"
        );
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

#[cfg(test)]
mod guards {
    use super::*;

    #[tokio::test]
    async fn received_2xx_survives_a_truncated_response_body() {
        bounded(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let write = Write {
                path: "/resource.bin".into(),
                body: b"new".to_vec(),
                delete: false,
                create: false,
                partial: false,
                prior: None,
            };
            let etag = body_etag(&write.body);
            let reply = format!("HTTP/1.1 201 Created\r\nETag: {etag}\r\nContent-Length: 10\r\nConnection: close\r\n\r\nx");
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut headers = Vec::new();
                while !headers.ends_with(b"\r\n\r\n") {
                    headers.push(stream.read_u8().await.unwrap());
                    assert!(headers.len() < 8192);
                }
                let mut body = [0; 3];
                stream.read_exact(&mut body).await.unwrap();
                assert_eq!(&body, b"new");
                stream.write_all(reply.as_bytes()).await.unwrap();
                stream.shutdown().await.unwrap();
            });
            let client = reqwest::Client::builder().no_proxy()
                .timeout(Duration::from_secs(5)).build().unwrap();
            let request = client.put(format!("{base}{}", write.path))
                .header("Content-Type", BINARY).body(write.body.clone());
            let (send, mut received) = tokio::sync::mpsc::unbounded_channel();
            let killed = std::sync::atomic::AtomicBool::new(false);
            assert!(!send_write(7, request, &write, Mode::Solid, &base, &killed, &send).await,
                "control must actually truncate the response body");
            match received.try_recv().unwrap() {
                Progress::Complete(7, Ok(ack)) => {
                    assert_eq!(ack.hash, <[u8; 32]>::from(Sha256::digest(b"new")));
                    assert_eq!(ack.etag, Some(etag));
                }
                event => panic!("received 2xx was not recorded: {event:?}"),
            }
            assert!(received.try_recv().is_err(), "exactly one completion");
            tokio::time::timeout(Duration::from_secs(5), server).await.unwrap().unwrap();
        }).await;
    }

    #[test]
    #[should_panic(expected = "request failed before kill")]
    fn queued_pre_kill_failure_is_rejected() {
        // The controller already sent SIGKILL before draining this earlier event.
        let (send, mut pending) = tokio::sync::mpsc::unbounded_channel();
        send.send(Progress::Complete(
            9,
            Err(Failure {
                message: "500 Internal Server Error".into(),
                before_kill: true,
            }),
        ))
        .unwrap();
        drain_completions(&mut pending, &mut BTreeMap::new());
    }

    #[test]
    fn post_kill_transport_error_keeps_prior_acknowledgements() {
        let (send, mut pending) = tokio::sync::mpsc::unbounded_channel();
        send.send(Progress::Complete(
            1,
            Ok(Ack {
                hash: [1; 32],
                etag: Some("witness".into()),
            }),
        ))
        .unwrap();
        send.send(Progress::Complete(
            2,
            Err(Failure {
                message: "connection reset by SIGKILL".into(),
                before_kill: false,
            }),
        ))
        .unwrap();
        let mut acknowledged = BTreeMap::new();
        drain_completions(&mut pending, &mut acknowledged);
        assert_eq!(acknowledged.len(), 1);
        assert_eq!(acknowledged[&1].hash, [1; 32]);
        assert_eq!(acknowledged[&1].etag.as_deref(), Some("witness"));
    }

    fn listing(body: &str) -> Snapshot {
        Snapshot {
            status: 200,
            body: body.as_bytes().to_vec(),
            etag: None,
            content_type: None,
            last_modified: None,
        }
    }

    #[test]
    #[should_panic(expected = "containment subject")]
    fn solid_listing_rejects_another_subject() {
        listing_members(Mode::Solid, "http://pod.example/c/", &listing(
            "<urn:another-container> <http://www.w3.org/ns/ldp#contains> <http://pod.example/c/item> ."));
    }

    #[test]
    #[should_panic(expected = "container identity")]
    fn lws_listing_rejects_another_container() {
        listing_members(
            Mode::Lws,
            "http://pod.example/c/",
            &listing(
                r#"{"id":"urn:another-container","totalItems":1,"items":[{"id":"http://pod.example/c/item"}]}"#,
            ),
        );
    }

    #[test]
    fn listings_accept_the_expected_parent() {
        let expected = BTreeSet::from(["http://pod.example/c/item".to_string()]);
        for (mode, body) in [
            (Mode::Solid, "<http://pod.example/c/> <http://www.w3.org/ns/ldp#contains> <http://pod.example/c/item> ."),
            (Mode::Lws, r#"{"id":"http://pod.example/c/","totalItems":1,"items":[{"id":"http://pod.example/c/item"}]}"#),
        ] {
            assert_eq!(listing_members(mode, "http://pod.example/c/", &listing(body)), expected);
        }
    }
}
