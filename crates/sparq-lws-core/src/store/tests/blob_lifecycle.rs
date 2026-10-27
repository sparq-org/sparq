//! Blob/index equality after requests through both protocol routers.

use super::{Shared, *};
use std::collections::HashSet;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use tower::ServiceExt;

use super::common;

const BASE: &str = common::BASE_URL;
const RDF: &str = "<urn:subject> <urn:predicate> <urn:object> .";
const CONTAINER: &str = "<http://www.w3.org/ns/ldp#BasicContainer>; rel=\"type\"";
const LWS_CONTAINER: &str = "<https://www.w3.org/ns/lws#Container>; rel=\"type\"";

async fn equal_keys<S: SparqClient, B: BlobStore>(index: &S, blob: &B, label: &str) -> bool {
    let referenced: HashSet<_> = index
        .referenced_blob_keys()
        .await
        .unwrap()
        .into_iter()
        .filter(|key| !key.is_empty())
        .collect();
    let actual: HashSet<_> = blob
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.key)
        .collect();
    let equal = actual == referenced;
    eprintln!(
        "{label}: equal={equal}, extra={:?}, missing={:?}",
        actual.difference(&referenced).collect::<Vec<_>>(),
        referenced.difference(&actual).collect::<Vec<_>>()
    );
    equal
}

async fn operations<B: BlobStore + 'static>(blob: B, lws: bool) -> bool {
    let index = Arc::new(InMemorySparqClient::new());
    let blob = Arc::new(blob);
    let counters = BackendCounters::new();
    let store = CompositeStore::new(
        Shared(index.clone()),
        CountingBlobStore::new(Shared(blob.clone()), counters.clone()),
    );
    store
        .write(
            &format!("{BASE}/"),
            Bytes::new(),
            if lws {
                crate::lws::LWS_JSON
            } else {
                "text/turtle"
            },
        )
        .await
        .unwrap();
    let acl = format!("@prefix acl: <http://www.w3.org/ns/auth/acl#> . <#owner> a acl:Authorization; acl:agent <{}>; acl:accessTo <{BASE}/>; acl:default <{BASE}/>; acl:mode acl:Read, acl:Write, acl:Control .", common::WEBID);
    store
        .write(
            &format!("{BASE}/.acl"),
            Bytes::from(acl.clone()),
            "text/turtle",
        )
        .await
        .unwrap();
    let updated_acl = format!("{acl}\n<urn:acl-version> <urn:predicate> <urn:updated> .");
    let issuer = common::KeyKit::generate();
    let client = common::KeyKit::generate();
    let app = if lws {
        let mut config = crate::lws::LwsConfig::new(BASE);
        config.open = true;
        crate::lws::router(store, config).await.unwrap()
    } else {
        let config = VerifierConfig::new(vec![common::ISSUER.to_string()], BASE);
        let replay = InMemoryReplayStore::with_window(config.replay_ttl());
        let verifier = Verifier::new(config, common::jwks_provider(&issuer), replay).unwrap();
        crate::app::build_router(crate::app::AppState::new(
            crate::auth::AuthContext::new(verifier, BASE),
            crate::ldp::handler::LdpState::new(store, BASE),
        ))
    };
    let container = if lws { LWS_CONTAINER } else { CONTAINER };
    let patch_type = if lws {
        "application/json-patch+json"
    } else {
        "application/sparql-update"
    };
    let patch = if lws {
        r#"[{"op":"replace","path":"/value","value":2}]"#
    } else {
        "INSERT DATA { <urn:patched> <urn:predicate> <urn:object> . }"
    };
    let patch_target = if lws { "/c/data.json" } else { "/c/data.ttl" };
    let cases = [
        (
            "container PUT",
            "PUT",
            "/c/",
            "text/turtle",
            "",
            container,
            if lws {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::CREATED
            },
        ),
        (
            "container POST",
            "POST",
            "/",
            "text/turtle",
            "",
            container,
            StatusCode::CREATED,
        ),
        (
            "RDF create",
            "PUT",
            "/c/data.ttl",
            "text/turtle",
            RDF,
            "",
            StatusCode::CREATED,
        ),
        (
            "RDF overwrite",
            "PUT",
            "/c/data.ttl",
            "text/turtle",
            "<urn:new> <urn:predicate> <urn:object> .",
            "",
            StatusCode::NO_CONTENT,
        ),
        (
            "binary create",
            "PUT",
            "/c/data.bin",
            "application/octet-stream",
            "first",
            "",
            StatusCode::CREATED,
        ),
        (
            "binary overwrite",
            "PUT",
            "/c/data.bin",
            "application/octet-stream",
            "second",
            "",
            StatusCode::NO_CONTENT,
        ),
        (
            "JSON create",
            "PUT",
            "/c/data.json",
            "application/json",
            r#"{"value":1}"#,
            "",
            StatusCode::CREATED,
        ),
        (
            "PATCH",
            "PATCH",
            patch_target,
            patch_type,
            patch,
            "",
            StatusCode::NO_CONTENT,
        ),
        (
            "ACL write",
            "PUT",
            "/.acl",
            "text/turtle",
            &updated_acl,
            "",
            StatusCode::NO_CONTENT,
        ),
        (
            "resource DELETE",
            "DELETE",
            "/c/data.bin",
            "",
            "",
            "",
            StatusCode::NO_CONTENT,
        ),
        (
            "RDF DELETE",
            "DELETE",
            "/c/data.ttl",
            "",
            "",
            "",
            StatusCode::NO_CONTENT,
        ),
        (
            "JSON DELETE",
            "DELETE",
            "/c/data.json",
            "",
            "",
            "",
            StatusCode::NO_CONTENT,
        ),
        (
            "empty container DELETE",
            "DELETE",
            "/c/",
            "",
            "",
            "",
            StatusCode::NO_CONTENT,
        ),
    ];
    let mut all_equal = true;
    for (label, method, path, content_type, body, link, expected) in cases {
        let create_member = lws && expected == StatusCode::CREATED && method == "PUT";
        let method = if create_member { "POST" } else { method };
        let request_path = if create_member { "/c/" } else { path };
        let mut request = Request::builder()
            .method(method)
            .uri(format!("{BASE}{request_path}"));
        if !lws {
            let token = common::mint_access_token(&issuer, &client.thumbprint);
            let proof = common::mint_dpop_proof(&client, method, &format!("{BASE}{path}"), &token);
            request = request
                .header("authorization", format!("DPoP {token}"))
                .header("dpop", proof);
        }
        if lws && method == "POST" {
            request = request.header(
                "slug",
                if label == "container POST" {
                    "c"
                } else {
                    path.rsplit('/').next().unwrap()
                },
            );
        }
        if !content_type.is_empty() {
            request = request.header("content-type", content_type);
        }
        if !link.is_empty() {
            request = request.header("link", link);
        }
        let measured = counters.measure();
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let location = response
            .headers()
            .get("location")
            .map(|v| v.to_str().unwrap().to_string());
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            status,
            expected,
            "lws={lws} {label}: {}",
            String::from_utf8_lossy(&bytes)
        );
        if !lws && label == "container PUT" {
            let once = measured.delta().blob_puts == 1;
            eprintln!("container PUT writes target once: {once}");
            all_equal &= once;
        }
        let iri = format!("{BASE}{path}");
        if expected == StatusCode::NOT_FOUND {
            assert!(!index.exists(&iri).await.unwrap());
            all_equal &= equal_keys(index.as_ref(), blob.as_ref(), "LWS absent PUT").await;
            continue;
        }
        match method {
            "POST" => {
                let created = location.unwrap();
                if !link.is_empty() {
                    assert!(created.ends_with('/'), "POST must create a container");
                }
                if lws && create_member {
                    assert_eq!(created, iri);
                }
                let meta = index.get_meta(&created).await.unwrap();
                assert_eq!(blob.get(&meta.blob_key).await.unwrap(), body.as_bytes());
                assert!(index
                    .list_children(&format!("{BASE}{request_path}"))
                    .await
                    .unwrap()
                    .contains(&created));
            }
            "PUT" => {
                let meta = index.get_meta(&iri).await.unwrap();
                assert_eq!(blob.get(&meta.blob_key).await.unwrap(), body.as_bytes());
                if path != "/.acl" {
                    assert!(index
                        .list_children(&format!(
                            "{BASE}{}",
                            if path == "/c/" { "/" } else { "/c/" }
                        ))
                        .await
                        .unwrap()
                        .contains(&iri));
                }
            }
            "PATCH" => {
                let meta = index.get_meta(&iri).await.unwrap();
                let body = blob.get(&meta.blob_key).await.unwrap();
                if lws {
                    assert_eq!(
                        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                        serde_json::json!({"value": 2})
                    );
                } else {
                    let triples = crate::ldp::content::parse_to_triples(
                        crate::ldp::content::RdfFormat::Turtle,
                        &body,
                        &iri,
                    )
                    .unwrap();
                    assert_eq!(triples.len(), 2);
                    assert!(triples
                        .iter()
                        .any(|triple| triple.subject.to_string() == "<urn:patched>"));
                }
            }
            "DELETE" => {
                assert!(!index.exists(&iri).await.unwrap());
                let parent = if path == "/c/" { "/" } else { "/c/" };
                assert!(!index
                    .list_children(&format!("{BASE}{parent}"))
                    .await
                    .unwrap()
                    .contains(&iri));
            }
            _ => unreachable!(),
        }
        all_equal &= equal_keys(index.as_ref(), blob.as_ref(), &format!("lws={lws} {label}")).await;
    }
    all_equal
}

#[tokio::test]
async fn request_operations_keep_blob_and_index_keys_equal() {
    let mut equal = true;
    for lws in [false, true] {
        equal &= operations(InMemoryBlobStore::new(), lws).await;
        #[cfg(all(feature = "blob-fs", not(target_arch = "wasm32")))]
        {
            let directory = Directory::new();
            equal &= operations(
                super::super::blob_fs::FsBlobStore::open(&directory.0).unwrap(),
                lws,
            )
            .await;
        }
    }
    assert!(equal, "request left unequal blob and index key sets");
}

#[cfg(all(feature = "blob-fs", not(target_arch = "wasm32")))]
struct Directory(std::path::PathBuf);

#[cfg(all(feature = "blob-fs", not(target_arch = "wasm32")))]
impl Directory {
    fn new() -> Self {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).unwrap();
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = std::env::temp_dir().join(format!("lws-blob-lifecycle-{suffix}"));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

#[cfg(all(feature = "blob-fs", not(target_arch = "wasm32")))]
impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[tokio::test]
async fn child_replacements_reclaim_versions_and_respect_holds() {
    let store = S::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    let parent = "https://pod.example/c/";
    let child = "https://pod.example/c/item";
    store
        .write(parent, Bytes::new(), "text/turtle")
        .await
        .unwrap();
    let original = store
        .create_in_container(parent, child, Bytes::from_static(b"original"), "text/plain")
        .await
        .unwrap();
    let held = store.hold(child, "urn:test:change").await.unwrap().unwrap();
    store
        .create_in_container(parent, child, Bytes::from_static(b"created"), "text/plain")
        .await
        .unwrap();
    assert_eq!(
        store.blob.get(&held.blob_key).await.unwrap(),
        b"original"[..]
    );
    let replacement = store.meta(child).await.unwrap().unwrap();
    store
        .restore(
            child,
            Some(parent),
            Bytes::from_static(b"original"),
            &original,
        )
        .await
        .unwrap();
    assert!(
        !store.blob.exists(&replacement.blob_key).await.unwrap(),
        "restore must reclaim its predecessor"
    );
    let replacement = store.meta(child).await.unwrap().unwrap();
    store.reinstate(child, Some(parent), &held).await.unwrap();
    assert!(
        !store.blob.exists(&replacement.blob_key).await.unwrap(),
        "reinstate must reclaim its predecessor"
    );
    store.release(&held, "urn:test:change").await.unwrap();
    assert_eq!(store.read(child).await.unwrap().body, b"original"[..]);
    store
        .create_in_container(parent, child, Bytes::from_static(b"final"), "text/plain")
        .await
        .unwrap();
    assert!(
        !store.blob.exists(&held.blob_key).await.unwrap(),
        "unheld predecessor must be reclaimed"
    );
    assert!(equal_keys(store.sparq.as_ref(), &store.blob, "child replacements").await);
}

#[derive(Clone, Copy, Debug)]
enum Failure {
    Missing,
    Full,
    Backend,
    Committed,
    Unsettled,
}

struct FailingIndex {
    inner: InMemorySparqClient,
    failure: std::sync::Mutex<Option<Failure>>,
}

impl FailingIndex {
    fn error(failure: Failure) -> SparqError {
        match failure {
            Failure::Missing => SparqError::NotFound,
            Failure::Full => SparqError::QuotaExceeded,
            Failure::Backend | Failure::Committed => {
                SparqError::Backend("test commit result".into())
            }
            Failure::Unsettled => SparqError::Unsettled {
                detail: "test pending commit".into(),
                fence: Fence::new("test-pending", vec![]),
            },
        }
    }
    fn take(&self) -> Option<Failure> {
        self.failure.lock().unwrap().take()
    }
}

#[async_trait]
impl SparqClient for FailingIndex {
    async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError> {
        self.inner.get_meta(iri).await
    }
    async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
        if let Some(failure) = self.take() {
            if matches!(failure, Failure::Committed) {
                self.inner.put_meta(iri, meta).await?;
            }
            return Err(Self::error(failure));
        }
        self.inner.put_meta(iri, meta).await
    }
    async fn exists(&self, iri: &str) -> Result<bool, SparqError> {
        self.inner.exists(iri).await
    }
    async fn delete_meta(&self, iri: &str) -> Result<(), SparqError> {
        self.inner.delete_meta(iri).await
    }
    async fn delete_meta_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<DeleteOutcome, SparqError> {
        self.inner.delete_meta_if_empty(iri, parent).await
    }
    async fn create_child(
        &self,
        container: &str,
        child: &str,
        meta: ResourceMeta,
    ) -> Result<(), SparqError> {
        if let Some(failure) = self.take() {
            if matches!(failure, Failure::Committed) {
                self.inner.create_child(container, child, meta).await?;
            }
            return Err(Self::error(failure));
        }
        self.inner.create_child(container, child, meta).await
    }
    async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError> {
        self.inner.remove_child(container, child).await
    }
    async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError> {
        self.inner.list_children(container).await
    }
    async fn referenced_blob_keys(&self) -> Result<HashSet<String>, SparqError> {
        self.inner.referenced_blob_keys().await
    }
    async fn begin_reclaim(&self, key: &str) -> Result<bool, SparqError> {
        self.inner.begin_reclaim(key).await
    }
    async fn end_reclaim(&self, key: &str) -> Result<(), SparqError> {
        self.inner.end_reclaim(key).await
    }
}

#[tokio::test]
async fn failed_commits_reclaim_only_settled_unreferenced_bytes() {
    let mut all_equal = true;
    for operation in ["write", "create", "restore"] {
        for failure in [
            Failure::Missing,
            Failure::Full,
            Failure::Backend,
            Failure::Committed,
            Failure::Unsettled,
        ] {
            let index = FailingIndex {
                inner: InMemorySparqClient::new(),
                failure: Default::default(),
            };
            let store = CompositeStore::new(index, InMemoryBlobStore::new());
            let parent = "https://pod.example/c/";
            let child = "https://pod.example/c/item";
            let meta = store
                .write(parent, Bytes::new(), "text/turtle")
                .await
                .unwrap();
            *store.sparq.failure.lock().unwrap() = Some(failure);
            let body = Bytes::from_static(b"candidate");
            let result = match operation {
                "write" => store.write(child, body, "text/plain").await,
                "create" => {
                    store
                        .create_in_container(parent, child, body, "text/plain")
                        .await
                }
                "restore" => store.restore(child, Some(parent), body, &meta).await,
                _ => unreachable!(),
            };
            if matches!(failure, Failure::Committed) {
                assert_eq!(store.read(child).await.unwrap().body, b"candidate"[..]);
                if operation == "restore" {
                    assert!(result.is_ok());
                } else {
                    assert!(result.is_err());
                }
            } else {
                assert!(result.is_err(), "{operation} {failure:?}");
                assert!(store.meta(child).await.unwrap().is_none());
            }
            if matches!(failure, Failure::Unsettled) {
                assert!(
                    matches!(result, Err(ServerError::Unsettled { .. })),
                    "{operation}: {result:?}"
                );
                assert_eq!(
                    store.blob.list().await.unwrap().len(),
                    2,
                    "pending commit must retain its candidate bytes"
                );
            } else {
                all_equal &= equal_keys(
                    store.sparq.as_ref(),
                    &store.blob,
                    &format!("{operation} {failure:?}"),
                )
                .await;
            }
        }
    }
    assert!(
        all_equal,
        "settled failure left unequal blob and index key sets"
    );
}

async fn concurrent_replacements<S: SparqClient + 'static>(index: S) {
    let store = Arc::new(CompositeStore::new(
        CountingSparqClient::new(index, BackendCounters::new()),
        InMemoryBlobStore::new(),
    ));
    let parent = "https://pod.example/c/";
    let child = "https://pod.example/c/item";
    store
        .write(parent, Bytes::new(), "text/turtle")
        .await
        .unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    for byte in 0..16 {
        let store = store.clone();
        tasks.spawn(async move {
            store
                .create_in_container(
                    parent,
                    child,
                    Bytes::from(vec![byte]),
                    "application/octet-stream",
                )
                .await
                .unwrap();
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    assert_eq!(
        store
            .list_children(parent)
            .await
            .unwrap()
            .iter()
            .map(|iri| iri.as_str())
            .collect::<Vec<_>>(),
        [child]
    );
    let current = store.read(child).await.unwrap();
    assert_eq!(current.body.len(), 1);
    assert!(current.body[0] < 16);
    assert!(equal_keys(store.sparq.as_ref(), &store.blob, "concurrent replacements").await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_child_replacements_account_for_every_version() {
    concurrent_replacements(InMemorySparqClient::new()).await;
    #[cfg(feature = "embedded-sparq")]
    concurrent_replacements(super::super::embedded::EmbeddedSparqClient::in_memory().unwrap())
        .await;
}
