//! Concurrent HTTP mutations preserve the acknowledged members and contents of one container.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::routing::get;
use axum::{Extension, Router};
use bytes::Bytes;
use sparq_lws_core::auth::VerifiedToken;
use sparq_lws_core::ldp::{content, handler};
use sparq_lws_core::lws::{self, LwsConfig};
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient, Store};
use tower::ServiceExt;

const BASE: &str = "https://pod.example";
const PARENT: &str = "https://pod.example/c/";
const BINARY: &str = "application/octet-stream";
const CORRECTNESS_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug)]
enum Mode {
    Solid,
    Lws,
}

async fn router<S: Store + 'static>(mode: Mode, store: S) -> Router {
    match mode {
        Mode::Lws => {
            let mut cfg = LwsConfig::new(BASE);
            cfg.open = true;
            cfg.page_size = 1000;
            lws::router(store, cfg).await.unwrap()
        }
        Mode::Solid => {
            store
                .write(&format!("{BASE}/"), Bytes::new(), "text/turtle")
                .await
                .unwrap();
            let acl = format!(
                "@prefix acl: <http://www.w3.org/ns/auth/acl#>. \
                 <#public> a acl:Authorization; acl:agentClass <http://xmlns.com/foaf/0.1/Agent>; \
                 acl:accessTo <{BASE}/>; acl:default <{BASE}/>; acl:mode acl:Read, acl:Write, acl:Control."
            );
            store
                .write(&format!("{BASE}/.acl"), Bytes::from(acl), "text/turtle")
                .await
                .unwrap();
            Router::new()
                .route(
                    "/",
                    get(handler::get_handler::<S>).post(handler::post_handler::<S>),
                )
                .route(
                    "/{*path}",
                    get(handler::get_handler::<S>)
                        .put(handler::put_handler::<S>)
                        .post(handler::post_handler::<S>)
                        .delete(handler::delete_handler::<S>),
                )
                .layer(Extension(VerifiedToken::public()))
                .with_state(Arc::new(handler::LdpState::new(store, BASE)))
        }
    }
}

fn request(method: Method, path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(format!("{BASE}{path}"))
        .header(header::CONTENT_TYPE, BINARY)
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn response(
    app: &Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Bytes) {
    let (parts, body) = app.clone().oneshot(request).await.unwrap().into_parts();
    (
        parts.status,
        parts.headers,
        to_bytes(body, 1 << 20).await.unwrap(),
    )
}

fn create(mode: Mode, name: &str, body: &str) -> Request<Body> {
    match mode {
        Mode::Lws => {
            let mut req = request(Method::POST, "/c/", body);
            req.headers_mut().insert("slug", name.parse().unwrap());
            req
        }
        Mode::Solid => request(Method::PUT, &format!("/c/{name}"), body),
    }
}

async fn workload<S: Store + 'static>(mode: Mode, store: S) {
    let app = router(mode, store).await;
    let mut container = match mode {
        Mode::Lws => request(Method::POST, "/", ""),
        Mode::Solid => request(Method::PUT, "/c/", ""),
    };
    let kind = match mode {
        Mode::Lws => "<https://www.w3.org/ns/lws#Container>; rel=\"type\"",
        Mode::Solid => "<http://www.w3.org/ns/ldp#BasicContainer>; rel=\"type\"",
    };
    container
        .headers_mut()
        .insert(header::LINK, kind.parse().unwrap());
    container.headers_mut().insert("slug", "c".parse().unwrap());
    assert_eq!(response(&app, container).await.0, StatusCode::CREATED);
    let mut expected = BTreeMap::new();
    for id in 0..64 {
        let name = format!("old-{id}.bin");
        let body = format!("old-{id}");
        let (status, _, _) = response(&app, create(mode, &name, &body)).await;
        assert_eq!(status, StatusCode::CREATED, "seed {mode:?}: {name}");
        expected.insert(name, body);
    }
    // Verify the old state before it can become an overwrite or delete's acknowledged state.
    for (name, body) in &expected {
        let (status, _, bytes) =
            response(&app, request(Method::GET, &format!("/c/{name}"), "")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes.as_ref(), body.as_bytes());
    }

    let barrier = Arc::new(tokio::sync::Barrier::new(161));
    let mut tasks = tokio::task::JoinSet::new();
    for id in 0..160 {
        let (name, body, req, status) = match id % 5 {
            0 => {
                let name = format!("old-{}.bin", id / 5 * 2);
                let req = request(Method::DELETE, &format!("/c/{name}"), "");
                (name, None, req, StatusCode::NO_CONTENT)
            }
            1 => {
                let name = format!("old-{}.bin", id / 5 * 2 + 1);
                let body = format!("updated-{id}");
                let req = request(Method::PUT, &format!("/c/{name}"), &body);
                (name, Some(body), req, StatusCode::NO_CONTENT)
            }
            _ => {
                let name = format!("new-{id}.bin");
                let body = format!("new-{id}");
                let req = create(mode, &name, &body);
                (name, Some(body), req, StatusCode::CREATED)
            }
        };
        let (app, barrier) = (app.clone(), barrier.clone());
        tasks.spawn(async move {
            barrier.wait().await;
            let (actual, headers, _) = response(&app, req).await;
            assert_eq!(actual, status, "{mode:?}: {name}");
            if actual == StatusCode::CREATED {
                assert_eq!(headers[header::LOCATION], format!("{PARENT}{name}"));
            }
            (name, body)
        });
    }
    barrier.wait().await;
    let mut acknowledged = BTreeSet::new();
    while let Some(result) = tasks.join_next().await {
        let (name, body) = result.unwrap();
        assert!(acknowledged.insert(name.clone()));
        match body {
            Some(body) => {
                expected.insert(name, body);
            }
            None => {
                assert!(expected.remove(&name).is_some());
            }
        }
    }
    assert_eq!(acknowledged.len(), 160);
    let mut listing = request(Method::GET, "/c/", "");
    listing.headers_mut().insert(
        header::ACCEPT,
        match mode {
            Mode::Lws => "application/lws+json",
            Mode::Solid => "text/turtle",
        }
        .parse()
        .unwrap(),
    );
    let (status, _, bytes) = response(&app, listing).await;
    assert_eq!(status, StatusCode::OK);
    let members: BTreeSet<String> = match mode {
        Mode::Lws => {
            let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(doc["id"], PARENT);
            let items = doc["items"].as_array().unwrap();
            assert_eq!(doc["totalItems"].as_u64().unwrap() as usize, items.len());
            assert_eq!(items.len(), expected.len(), "one item per surviving member");
            items
                .iter()
                .map(|item| item["id"].as_str().unwrap().to_string())
                .collect()
        }
        Mode::Solid => content::parse_to_triples(content::RdfFormat::Turtle, &bytes, PARENT)
            .unwrap()
            .into_iter()
            .filter(|triple| triple.predicate.as_str() == "http://www.w3.org/ns/ldp#contains")
            .map(|triple| {
                assert_eq!(triple.subject.to_string(), format!("<{PARENT}>"));
                match triple.object {
                    oxrdf::Term::NamedNode(iri) => iri.into_string(),
                    other => panic!("unexpected member: {other}"),
                }
            })
            .collect(),
    };
    assert_eq!(
        members,
        expected
            .keys()
            .map(|name| format!("{PARENT}{name}"))
            .collect()
    );
    for (name, body) in expected {
        let (status, _, bytes) =
            response(&app, request(Method::GET, &format!("/c/{name}"), "")).await;
        assert_eq!(status, StatusCode::OK, "{name}");
        assert_eq!(bytes.as_ref(), body.as_bytes(), "{name}");
    }
    for name in acknowledged.iter().filter(|name| name.starts_with("old-")) {
        let id: usize = name
            .strip_prefix("old-")
            .unwrap()
            .strip_suffix(".bin")
            .unwrap()
            .parse()
            .unwrap();
        if id.is_multiple_of(2) {
            assert_eq!(
                response(&app, request(Method::GET, &format!("/c/{name}"), ""))
                    .await
                    .0,
                StatusCode::NOT_FOUND,
                "deleted {name}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mixed_mutations_finish_with_acknowledged_listing() {
    for mode in [Mode::Solid, Mode::Lws] {
        eprintln!("checking {mode:?} with in-memory storage");
        tokio::time::timeout(
            CORRECTNESS_DEADLINE,
            workload(
                mode,
                CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new()),
            ),
        )
        .await
        .expect("in-memory mutations exceeded the correctness deadline");
        #[cfg(all(feature = "embedded-sparq", feature = "blob-fs"))]
        {
            use sparq_lws_core::store::{blob_fs::FsBlobStore, EmbeddedSparqClient};
            struct Directory(std::path::PathBuf);
            impl Drop for Directory {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
            let mut nonce = [0u8; 16];
            getrandom::getrandom(&mut nonce).unwrap();
            let dir = Directory(std::env::temp_dir().join(format!("lws-mutations-{:x?}", nonce)));
            std::fs::create_dir(&dir.0).unwrap();
            eprintln!("checking {mode:?} with durable storage");
            let store = CompositeStore::new(
                EmbeddedSparqClient::open_or_create(&dir.0.join("index")).unwrap(),
                FsBlobStore::open(dir.0.join("blobs")).unwrap(),
            );
            tokio::time::timeout(CORRECTNESS_DEADLINE, workload(mode, store))
                .await
                .expect("durable mutations exceeded the correctness deadline");
        }
    }
}
