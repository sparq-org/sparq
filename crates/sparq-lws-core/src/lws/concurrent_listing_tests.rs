//! Router-level container listing regressions.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use test_store::{body_json, response_bytes, FlakyStore};
use tokio::sync::{Barrier, Semaphore};
use tower::ServiceExt;

fn app(state: &LwsState<FlakyStore>) -> Router {
    Router::new()
        .fallback(dispatch::<FlakyStore>)
        .with_state(state.clone())
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Response {
    let mut request = axum::http::Request::builder().method(method).uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app.clone()
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap()
}

async fn listing(app: &Router) -> serde_json::Value {
    let response = call(app, "GET", "/", &[("accept", LWS_JSON)], "").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], LWS_JSON);
    assert!(test_store::links(&response, "type").contains(&format!("{LWS_NS}Container")));
    let doc = body_json(response).await;
    assert_eq!(doc["type"], "Container");
    let items = doc["items"].as_array().expect("items array");
    assert_eq!(doc["totalItems"].as_u64().unwrap() as usize, items.len());
    let mut ids = std::collections::HashSet::new();
    for item in items {
        assert!(ids.insert(item["id"].as_str().expect("member URI")));
        assert_eq!(item["type"], "DataResource");
        assert!(item["format"].as_str().is_some());
    }
    doc
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn listings_finish_during_continuous_creates_updates_and_deletes() {
    let (state, _) = test_store::state(1000).await;
    let app = app(&state);
    let stop = Arc::new(AtomicBool::new(false));
    let counts = Arc::new([
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    ]);
    let start = Arc::new(Barrier::new(36));
    let mut writers = tokio::task::JoinSet::new();
    for writer in 0..32 {
        let (app, stop, counts, start) = (app.clone(), stop.clone(), counts.clone(), start.clone());
        writers.spawn(async move {
            start.wait().await;
            let mut n = 0;
            while !stop.load(Ordering::SeqCst) {
                let slug = format!("w{writer}-{n}");
                let path = format!("/{slug}");
                assert_eq!(
                    call(
                        &app,
                        "POST",
                        "/",
                        &[("slug", &slug), ("content-type", "text/plain")],
                        "old"
                    )
                    .await
                    .status(),
                    StatusCode::CREATED
                );
                counts[0].fetch_add(1, Ordering::SeqCst);
                assert_eq!(
                    call(
                        &app,
                        "PUT",
                        &path,
                        &[("content-type", "application/json")],
                        "{}"
                    )
                    .await
                    .status(),
                    StatusCode::NO_CONTENT
                );
                counts[1].fetch_add(1, Ordering::SeqCst);
                assert_eq!(
                    call(&app, "DELETE", &path, &[], "").await.status(),
                    StatusCode::NO_CONTENT
                );
                counts[2].fetch_add(1, Ordering::SeqCst);
                n += 1;
            }
        });
    }
    let mut readers = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let (app, start) = (app.clone(), start.clone());
        readers.spawn(async move {
            start.wait().await;
            for _ in 0..100 {
                let doc = listing(&app).await;
                for item in doc["items"].as_array().unwrap() {
                    // The fields must describe one committed version.
                    assert!(matches!(
                        (item["format"].as_str(), item["size"].as_u64()),
                        (Some("text/plain"), Some(3)) | (Some("application/json"), Some(2))
                    ));
                }
            }
        });
    }
    // Writers stop only AFTER all reads finish, so a quiet period cannot let this pass.
    tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(result) = readers.join_next().await {
            result.unwrap();
        }
    })
    .await
    .expect("listings make progress while writes continue");
    stop.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(result) = writers.join_next().await {
            result.unwrap();
        }
    })
    .await
    .expect("writers finish too");
    assert!(counts.iter().all(|n| n.load(Ordering::SeqCst) > 0));
    assert_eq!(listing(&app).await["totalItems"], 0);
}

#[tokio::test]
async fn listing_does_not_wait_for_a_member_waiting_for_its_parent() {
    let (state, _) = test_store::state(100).await;
    let app = app(&state);
    assert_eq!(
        call(&app, "POST", "/", &[("slug", "x")], "old")
            .await
            .status(),
        StatusCode::CREATED
    );
    // A writer takes the member before the parent. It has not changed stored state yet.
    let _member = state.locks.lock(&state.cfg.absolute("/x")).await.unwrap();
    let _parent = state.locks.lock(&state.cfg.storage()).await.unwrap();
    let read = tokio::spawn({
        let app = app.clone();
        async move { listing(&app).await }
    });
    tokio::task::yield_now().await;
    // Queue a writer for the parent after the listing, while it still owns the member.
    let writer = tokio::spawn({
        let state = state.clone();
        async move { state.locks.read(&state.cfg.storage()).await.unwrap() }
    });
    drop(_parent);
    let doc = tokio::time::timeout(Duration::from_secs(2), read)
        .await
        .expect("no member lock acquired under container")
        .unwrap();
    assert_eq!(doc["items"][0]["id"], state.cfg.absolute("/x"));
    drop(writer.await.unwrap());
}

#[tokio::test]
async fn listings_never_show_a_create_or_update_that_is_put_back() {
    let (state, store) = test_store::state(100).await;
    let app = app(&state);
    let uri = state.cfg.absolute("/x");
    let gate = Arc::new(Semaphore::new(0));
    *store.hold_next_create_reply_of.lock().unwrap() = Some((uri.clone(), gate.clone()));
    store.fail_after_create.store(true, Ordering::SeqCst);
    let create = tokio::spawn({
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                "/",
                &[("slug", "x"), ("content-type", "text/plain")],
                "old",
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !store.exists(&uri).await.unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let read = tokio::spawn({
        let app = app.clone();
        async move { listing(&app).await }
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!read.is_finished(), "uncommitted create must not be listed");
    gate.add_permits(1);
    assert!(create.await.unwrap().status().is_server_error());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), read)
            .await
            .unwrap()
            .unwrap()["totalItems"],
        0
    );
    assert!(!store.exists(&uri).await.unwrap());
    store.fail_after_create.store(false, Ordering::SeqCst);
    assert_eq!(
        call(
            &app,
            "POST",
            "/",
            &[("slug", "x"), ("content-type", "text/plain")],
            "old"
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    let before = listing(&app).await;
    let metadata = meta_key(&uri);
    let entered = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    *store.write_cycles.lock().unwrap() = Some(test_store::WriteCycles {
        uris: vec![metadata.clone()],
        entered: entered.clone(),
        release: release.clone(),
    });
    *store.lose_nth_write_of.lock().unwrap() = Some((metadata, 2));
    let update = tokio::spawn({
        let app = app.clone();
        async move {
            call(
                &app,
                "PUT",
                "/x",
                &[
                    ("content-type", "application/json"),
                    ("link", "<https://example.org/New>; rel=\"type\""),
                ],
                "{}",
            )
            .await
        }
    });
    entered.acquire().await.unwrap().forget();
    release.add_permits(1); // pending metadata
    entered.acquire().await.unwrap().forget(); // content changed, final metadata not yet
    assert_eq!(
        store.meta(&uri).await.unwrap().unwrap().content_type,
        "application/json"
    );
    let read = tokio::spawn({
        let app = app.clone();
        async move { listing(&app).await }
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!read.is_finished(), "uncommitted update must not be listed");
    release.add_permits(1); // final metadata write loses its reply; journal restores both records
    assert!(update.await.unwrap().status().is_server_error());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), read)
            .await
            .unwrap()
            .unwrap(),
        before
    );
    assert_eq!(
        response_bytes(call(&app, "GET", "/x", &[], "").await)
            .await
            .2,
        "old"
    );
}

#[tokio::test]
async fn listing_never_shows_a_removal_that_is_put_back() {
    let (state, store) = test_store::state(100).await;
    let app = app(&state);
    assert_eq!(
        call(
            &app,
            "POST",
            "/",
            &[
                ("slug", "d"),
                (
                    "link",
                    "<https://www.w3.org/ns/lws#Container>; rel=\"type\""
                )
            ],
            ""
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    for slug in ["x", "y"] {
        assert_eq!(
            call(
                &app,
                "POST",
                "/d/",
                &[("slug", slug), ("content-type", "text/plain")],
                "old"
            )
            .await
            .status(),
            StatusCode::CREATED
        );
    }
    let before = body_json(call(&app, "GET", "/d/", &[("accept", LWS_JSON)], "").await).await;
    let x = state.cfg.absolute("/d/x");
    let container = state.cfg.absolute("/d/");
    let gate = Arc::new(Semaphore::new(0));
    *store.hold_next_delete_of.lock().unwrap() = Some((container.clone(), gate.clone()));
    *store.fail_delete_of.lock().unwrap() = Some(meta_key(&container));
    let delete = tokio::spawn({
        let app = app.clone();
        async move { call(&app, "DELETE", "/d/", &[("depth", "infinity")], "").await }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while store.exists(&x).await.unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let read = tokio::spawn({
        let app = app.clone();
        async move {
            let response = call(&app, "GET", "/d/", &[("accept", LWS_JSON)], "").await;
            assert_eq!(response.status(), StatusCode::OK);
            body_json(response).await
        }
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        !read.is_finished(),
        "partly removed container must not be listed"
    );
    gate.add_permits(1);
    assert!(delete.await.unwrap().status().is_server_error());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), read)
            .await
            .unwrap()
            .unwrap(),
        before
    );
}
