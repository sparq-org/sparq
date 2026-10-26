use std::{
    fs::OpenOptions,
    io::Write,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use super::*;
use crate::store::{
    reconcile_orphans, BackendCounters, BodyCache, CountingBlobStore, InMemoryBlobStore,
    InMemorySparqClient, ReconcileOptions, Store,
};
use sparq_lws_crypt::{LocalKeyProvider, SecretKey};

#[path = "../../../tests/common/mod.rs"]
mod common;

#[derive(Default)]
struct ManualClock(AtomicU64);
impl CryptClock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_secs(self.0.load(Ordering::SeqCst))
    }
}

#[derive(Clone)]
struct Provider {
    inner: Arc<LocalKeyProvider>,
    calls: Arc<AtomicU64>,
}

fn provider(byte: u8) -> Provider {
    let mut suffix = [0; 16];
    getrandom::getrandom(&mut suffix).unwrap();
    let path = std::env::temp_dir().join(format!("lane-3-kek-{}", hex(&suffix)));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).unwrap();
    file.write_all(&[byte; 32]).unwrap();
    drop(file);
    let inner = LocalKeyProvider::from_file(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    Provider {
        inner: Arc::new(inner),
        calls: Arc::new(AtomicU64::new(0)),
    }
}

#[async_trait]
impl KeyProvider for Provider {
    fn id(&self) -> &str {
        self.inner.id()
    }
    async fn generate_data_key(
        &self,
        ctx: &KeyContext,
    ) -> sparq_lws_crypt::Result<(SecretKey, WrappedKey)> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        self.inner.generate_data_key(ctx).await
    }
    async fn unwrap(
        &self,
        wrapped: &WrappedKey,
        ctx: &KeyContext,
    ) -> sparq_lws_crypt::Result<SecretKey> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        self.inner.unwrap(wrapped, ctx).await
    }
}

#[derive(Clone, Default)]
struct Backend {
    bytes: Arc<InMemoryBlobStore>,
    refuse: Arc<AtomicBool>,
}

#[async_trait]
impl BlobStore for Backend {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        self.bytes.get(key).await
    }
    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        if self.refuse.swap(false, Ordering::SeqCst) {
            return Err(BlobError::Backend("interrupted publication".into()));
        }
        self.bytes.put(key, body).await
    }
    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        self.bytes.exists(key).await
    }
    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        self.bytes.delete(key).await
    }
    async fn list(&self) -> Result<Vec<BlobEntry>, BlobError> {
        self.bytes.list().await
    }
    async fn stat(&self, key: &str) -> Result<Option<BlobEntry>, BlobError> {
        self.bytes.stat(key).await
    }
    async fn delete_if_unchanged(&self, key: &str, generation: u64) -> Result<bool, BlobError> {
        self.bytes.delete_if_unchanged(key, generation).await
    }
}

type Encrypted = CompositeStore<InMemorySparqClient, EncryptingBlobStore<Backend, Provider>>;

struct Fixture {
    index: Arc<InMemorySparqClient>,
    backend: Backend,
    provider: Provider,
    clock: Arc<ManualClock>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            index: Arc::new(InMemorySparqClient::new()),
            backend: Backend::default(),
            provider: provider(0x31),
            clock: Arc::new(ManualClock::default()),
        }
    }
    fn options(&self) -> CryptOptions {
        CryptOptions {
            clock: self.clock.clone(),
            ..Default::default()
        }
    }
    fn plain(&self) -> CompositeStore<InMemorySparqClient, Backend> {
        CompositeStore {
            sparq: self.index.clone(),
            blob: self.backend.clone(),
            blob_scope: BlobScopePolicy::Solid,
            body_cache: BodyCache::disabled(),
            holding: Default::default(),
        }
    }
    async fn encrypted(&self) -> Encrypted {
        self.plain()
            .encrypt_blobs(self.provider.clone(), CryptScheme::Solid, self.options())
            .await
            .unwrap()
    }
}

const ALICE: &str = "https://pod.example/alice/file";
const BOB: &str = "https://pod.example/bob/file";
const BODY: &[u8] = b"the original resource bytes";

async fn current(store: &Encrypted, pod: &str) -> KeyRecord {
    store
        .blob
        .records
        .all()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.pod == pod && r.current)
        .unwrap()
}
async fn seal_count(store: &Encrypted, id: [u8; 16]) -> u64 {
    store
        .blob
        .key_slot(id)
        .await
        .lock()
        .await
        .as_ref()
        .unwrap()
        .dek
        .seal_count()
}

#[tokio::test]
async fn composite_write_paths_are_scoped_and_etags_remain_plaintext_derived() {
    let f = Fixture::new();
    let plain = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    let expected = plain
        .write(ALICE, Bytes::from_static(BODY), "application/octet-stream")
        .await
        .unwrap();
    let store = f.encrypted().await;
    assert!(store
        .blob
        .put("unscoped", Bytes::from_static(BODY))
        .await
        .is_err());
    assert!(!f.backend.exists("unscoped").await.unwrap());
    let meta = store
        .write(ALICE, Bytes::from_static(BODY), "application/octet-stream")
        .await
        .unwrap();
    assert_eq!(meta.etag, expected.etag);
    assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
    let raw = f.backend.get(&meta.blob_key).await.unwrap();
    assert!(raw.starts_with(b"SQLB"));
    assert!(!raw.windows(BODY.len()).any(|w| w == BODY));
    store
        .write("https://pod.example/alice/", Bytes::new(), "text/turtle")
        .await
        .unwrap();
    let child = "https://pod.example/alice/child";
    store
        .create_in_container(
            "https://pod.example/alice/",
            child,
            Bytes::from_static(BODY),
            "text/plain",
        )
        .await
        .unwrap();
    store
        .restore(ALICE, None, Bytes::from_static(BODY), &meta)
        .await
        .unwrap();
    store
        .restore(
            child,
            Some("https://pod.example/alice/"),
            Bytes::from_static(BODY),
            &meta,
        )
        .await
        .unwrap();
    let held = store.hold(ALICE, "urn:holder").await.unwrap().unwrap();
    store.reinstate(ALICE, None, &held).await.unwrap();
    store.release(&held, "urn:holder").await.unwrap();
    store
        .write(
            "https://pod.example/.lws/subscriptions/s",
            Bytes::from_static(BODY),
            "text/plain",
        )
        .await
        .unwrap();
    for entry in f.backend.list().await.unwrap() {
        assert!(
            f.backend
                .get(&entry.key)
                .await
                .unwrap()
                .starts_with(b"SQLB"),
            "{}",
            entry.key
        );
    }
    // A transparent counter decorator must preserve the scoped call as well.
    let counters = BackendCounters::new();
    let counted = CompositeStore {
        sparq: store.sparq,
        blob: CountingBlobStore::new(store.blob, counters.clone()),
        blob_scope: BlobScopePolicy::Solid,
        body_cache: BodyCache::disabled(),
        holding: Default::default(),
    };
    counted
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    assert_eq!(counters.snapshot().blob_puts, 1);
}

#[tokio::test]
async fn protocols_and_fixed_scope_select_the_expected_context() {
    for (scheme, policy, expected) in [
        (
            CryptScheme::Lws,
            BlobScopePolicy::Solid,
            "https://pod.example/",
        ),
        (
            CryptScheme::Solid,
            BlobScopePolicy::Solid,
            "https://pod.example/alice/",
        ),
        (
            CryptScheme::Solid,
            BlobScopePolicy::Fixed(BlobScope("https://single.example/".into())),
            "https://single.example/",
        ),
    ] {
        let f = Fixture::new();
        let store = f
            .plain()
            .with_blob_scope(policy)
            .encrypt_blobs(f.provider.clone(), scheme, f.options())
            .await
            .unwrap();
        store
            .write(ALICE, Bytes::from_static(BODY), "text/plain")
            .await
            .unwrap();
        let record = current(&store, expected).await;
        assert_eq!(record.scheme, scheme.as_str());
        store
            .write(
                "https://pod.example/.lws/subscriptions/subscription",
                Bytes::from_static(BODY),
                "text/plain",
            )
            .await
            .unwrap();
        assert_eq!(
            current(&store, "https://pod.example/").await.scheme,
            scheme.as_str()
        );
    }
}

#[tokio::test]
async fn pods_have_distinct_deks_and_backend_tampering_never_returns_bytes() {
    let f = Fixture::new();
    let store = f.encrypted().await;
    let a = store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let b = store
        .write(BOB, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let a_bytes = f.backend.get(&a.blob_key).await.unwrap();
    let b_bytes = f.backend.get(&b.blob_key).await.unwrap();
    assert_ne!(
        parse_header(&a_bytes).unwrap().dek_id,
        parse_header(&b_bytes).unwrap().dek_id
    );
    let mut tampered = a_bytes.to_vec();
    *tampered.last_mut().unwrap() ^= 1;
    f.backend.put(&a.blob_key, tampered.into()).await.unwrap();
    assert!(store.read(ALICE).await.is_err());
    // A valid envelope moved to a different opaque key also fails authentication.
    f.backend.put(&b.blob_key, a_bytes).await.unwrap();
    assert!(store.read(BOB).await.is_err());
}

#[tokio::test]
async fn restart_and_interrupted_publication_skip_every_prior_reservation() {
    let f = Fixture::new();
    let store = f.encrypted().await;
    f.backend.refuse.store(true, Ordering::SeqCst);
    assert!(store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .is_err());
    assert!(f.backend.list().await.unwrap().is_empty());
    let record = current(&store, "https://pod.example/alice/").await;
    assert_eq!(record.reserved_seals, BLOCK);
    let id = record.id().unwrap();
    let before = seal_count(&store, id).await;
    drop(store);
    let store = f.encrypted().await;
    assert_eq!(seal_count(&store, id).await, BLOCK);
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    assert_eq!(seal_count(&store, id).await, BLOCK + 1);
    assert!(seal_count(&store, id).await > before);
    assert_eq!(
        current(&store, "https://pod.example/alice/")
            .await
            .reserved_seals,
        2 * BLOCK
    );
    drop(store);
    let store = f.encrypted().await;
    assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    assert_eq!(seal_count(&store, id).await, 2 * BLOCK + 1);
}

#[tokio::test]
async fn reservation_commits_before_any_seal_and_parallel_writers_share_it() {
    let f = Fixture::new();
    let store = f.encrypted().await;
    // Persist a first reservation without publishing or sealing any body.
    let scope = BlobScope("https://pod.example/alice/".into());
    let id = store.blob.generate(&scope).await.unwrap();
    assert_eq!(seal_count(&store, id).await, 0);
    assert_eq!(
        store.blob.records.load(&id).await.unwrap().reserved_seals,
        BLOCK
    );
    drop(store);
    let store = Arc::new(f.encrypted().await);
    let mut tasks = Vec::new();
    for i in 0..20 {
        let store = store.clone();
        tasks.push(tokio::spawn(async move {
            store
                .write(
                    &format!("https://pod.example/alice/{i}"),
                    Bytes::from_static(BODY),
                    "text/plain",
                )
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(seal_count(&store, id).await, BLOCK + 20);
    assert_eq!(
        store.blob.records.load(&id).await.unwrap().reserved_seals,
        2 * BLOCK
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn rotation_uses_a_new_current_key_and_old_bytes_stay_readable() {
    let f = Fixture::new();
    let mut options = f.options();
    options.block = 2;
    options.rotate_at = 4;
    let store = f
        .plain()
        .encrypt_blobs(f.provider.clone(), CryptScheme::Solid, options)
        .await
        .unwrap();
    let mut metas = Vec::new();
    for i in 0..5 {
        metas.push(
            store
                .write(
                    &format!("https://pod.example/alice/{i}"),
                    Bytes::from_static(BODY),
                    "text/plain",
                )
                .await
                .unwrap(),
        );
    }
    let first = parse_header(&f.backend.get(&metas[0].blob_key).await.unwrap())
        .unwrap()
        .dek_id;
    let last = parse_header(&f.backend.get(&metas[4].blob_key).await.unwrap())
        .unwrap()
        .dek_id;
    assert_ne!(first, last);
    let records = store.blob.records.all().await.unwrap();
    assert_eq!(records.iter().filter(|r| r.current).count(), 1);
    assert_eq!(
        store
            .blob
            .records
            .load(&first)
            .await
            .unwrap()
            .reserved_seals,
        4
    );
    assert_eq!(
        current(&store, "https://pod.example/alice/")
            .await
            .id()
            .unwrap(),
        last
    );
    for i in 0..5 {
        assert_eq!(
            store
                .read(&format!("https://pod.example/alice/{i}"))
                .await
                .unwrap()
                .body,
            BODY
        );
    }
}

#[tokio::test]
async fn ttl_and_single_flight_bound_provider_calls_for_mixed_reads_and_writes() {
    let f = Fixture::new();
    let store = Arc::new(f.encrypted().await);
    let pods = 3;
    for pod in 0..pods {
        store
            .write(
                &format!("https://pod.example/p{pod}/read"),
                Bytes::from_static(BODY),
                "text/plain",
            )
            .await
            .unwrap();
    }
    for window in 0..3 {
        f.clock
            .0
            .store(window * DEFAULT_DEK_TTL.as_secs(), Ordering::SeqCst);
        let mut tasks = Vec::new();
        for i in 0..30 {
            let store = store.clone();
            tasks.push(tokio::spawn(async move {
                let pod = i % pods;
                assert_eq!(
                    store
                        .read(&format!("https://pod.example/p{pod}/read"))
                        .await
                        .unwrap()
                        .body,
                    BODY
                );
                store
                    .write(
                        &format!("https://pod.example/p{pod}/write{i}"),
                        Bytes::from_static(BODY),
                        "text/plain",
                    )
                    .await
                    .unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), pods * (window + 1));
    }
    assert_eq!(
        store.blob.records.all().await.unwrap().len() as u64,
        pods,
        "expiry must not generate replacement keys"
    );
}

#[tokio::test]
async fn mixed_legacy_and_unknown_or_malformed_envelopes_never_fall_back() {
    let f = Fixture::new();
    f.plain()
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let store = f.encrypted().await;
    assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
    let legacy = Bytes::from_static(b"SQLB\x01xxxxxxxxxxxxxxxxyyyyyyyyyyyyold legacy body");
    let meta = f.plain().write(BOB, legacy, "text/plain").await.unwrap();
    assert!(matches!(
        store.blob.get(&meta.blob_key).await,
        Err(BlobError::UnknownDataKey)
    ));
    for malformed in [b"SQLB".as_slice(), b"SQLB\x02unknown version"] {
        f.backend
            .put(&meta.blob_key, Bytes::copy_from_slice(malformed))
            .await
            .unwrap();
        assert!(store.read(BOB).await.is_err());
    }
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let meta = f.index.get_meta(ALICE).await.unwrap();
    assert!(f
        .backend
        .get(&meta.blob_key)
        .await
        .unwrap()
        .starts_with(b"SQLB"));
}

#[tokio::test]
async fn wrong_kek_at_open_and_swapped_wrapped_records_are_rejected() {
    let f = Fixture::new();
    let store = f.encrypted().await;
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    store
        .write(BOB, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let a = current(&store, "https://pod.example/alice/").await;
    let b = current(&store, "https://pod.example/bob/").await;
    drop(store);
    let result = f
        .plain()
        .encrypt_blobs(provider(0x41), CryptScheme::Solid, f.options())
        .await;
    assert!(matches!(result, Err(BlobError::Backend(ref s)) if s == "wrong KEK"));
    let store = f.encrypted().await;
    let mut swapped = a.clone();
    swapped.wrapped = b.wrapped;
    store.blob.records.save(&swapped).await.unwrap();
    f.clock.0.store(DEFAULT_DEK_TTL.as_secs(), Ordering::SeqCst);
    assert!(store.read(ALICE).await.is_err());
    assert!(store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .is_err());
}

#[tokio::test]
async fn key_records_stay_outside_resource_operations_and_the_reconciler() {
    let f = Fixture::new();
    let store = f.encrypted().await;
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let record = current(&store, "https://pod.example/alice/").await;
    let iri = record_iri(&record.id().unwrap());
    assert!(store.read(&iri).await.is_err());
    assert!(store.meta(&iri).await.unwrap().is_none());
    assert!(!store.exists(&iri).await.unwrap());
    assert!(store.read_plan(&iri, &[]).await.is_err());
    assert!(store.list_children(DATA_KEY_CONTAINER).await.is_err());
    assert!(store
        .list_children_within(DATA_KEY_CONTAINER, 10000)
        .await
        .is_err());
    assert!(store.write(&iri, Bytes::new(), "text/plain").await.is_err());
    assert!(store.delete(&iri, None).await.is_err());
    assert!(store
        .delete_container_if_empty(DATA_KEY_CONTAINER, None)
        .await
        .is_err());
    let raw_before = f.index.get_meta(&iri).await.unwrap();
    let report = reconcile_orphans(
        f.index.as_ref(),
        &store.blob,
        &ReconcileOptions {
            grace: Duration::ZERO,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(report.orphaned, 0);
    assert_eq!(f.index.get_meta(&iri).await.unwrap(), raw_before);
    assert!(!f.index.referenced_blob_keys().await.unwrap().contains(""));
}

struct Http {
    app: axum::Router,
    issuer: common::KeyKit,
    client: common::KeyKit,
}
impl Http {
    async fn new(store: Encrypted) -> Self {
        use solid_oidc_verifier::{
            config::VerifierConfig, replay::InMemoryReplayStore, verifier::Verifier,
        };
        store
            .write("https://pod.example/", Bytes::new(), "text/turtle")
            .await
            .unwrap();
        let acl = format!("@prefix acl: <http://www.w3.org/ns/auth/acl#>. <#owner> a acl:Authorization; acl:agent <{}>; acl:accessTo <https://pod.example/>; acl:default <https://pod.example/>; acl:mode acl:Read, acl:Write, acl:Control.", common::WEBID);
        store
            .write("https://pod.example/.acl", acl.into(), "text/turtle")
            .await
            .unwrap();
        let issuer = common::KeyKit::generate();
        let client = common::KeyKit::generate();
        let config = VerifierConfig::new(vec![common::ISSUER.into()], common::BASE_URL);
        let replay = InMemoryReplayStore::with_window(config.replay_ttl());
        let verifier = Verifier::new(config, common::jwks_provider(&issuer), replay).unwrap();
        let auth = crate::auth::AuthContext::new(verifier, common::BASE_URL);
        let ldp = crate::ldp::handler::LdpState::new(store, common::BASE_URL);
        Self {
            app: crate::app::build_router(crate::app::AppState::new(auth, ldp)),
            issuer,
            client,
        }
    }
    async fn call(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        body: &str,
    ) -> axum::response::Response {
        use tower::ServiceExt;
        let access = common::mint_access_token(&self.issuer, &self.client.thumbprint);
        let proof = common::mint_dpop_proof(
            &self.client,
            method,
            &format!("{}{path}", common::BASE_URL),
            &access,
        );
        let request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("DPoP {access}"))
            .header("dpop", proof)
            .header("content-type", content_type)
            .body(axum::body::Body::from(body.to_owned()))
            .unwrap();
        self.app.clone().oneshot(request).await.unwrap()
    }
}

#[tokio::test]
async fn http_put_get_etags_tampering_and_private_record_names() {
    use axum::{body::to_bytes, http::StatusCode};
    let f = Fixture::new();
    let http = Http::new(f.encrypted().await).await;
    let response = http
        .call(
            "PUT",
            "/alice",
            "application/octet-stream",
            std::str::from_utf8(BODY).unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let etag = response.headers()["etag"].clone();
    let meta = f.index.get_meta("https://pod.example/alice").await.unwrap();
    assert!(f
        .backend
        .get(&meta.blob_key)
        .await
        .unwrap()
        .starts_with(b"SQLB"));
    let response = http
        .call("GET", "/alice", "application/octet-stream", "")
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["etag"], etag);
    assert_eq!(to_bytes(response.into_body(), 10000).await.unwrap(), BODY);
    let response = http.call("GET", "/", "text/turtle", "").await;
    assert_eq!(response.status(), StatusCode::OK);
    let listing = to_bytes(response.into_body(), 100000).await.unwrap();
    assert!(!String::from_utf8_lossy(&listing).contains(DATA_KEY_CONTAINER));
    let mut raw = f.backend.get(&meta.blob_key).await.unwrap().to_vec();
    *raw.last_mut().unwrap() ^= 1;
    f.backend.put(&meta.blob_key, raw.into()).await.unwrap();
    // The fixture disables the body cache, so this drives envelope authentication again.
    assert_eq!(
        http.call("GET", "/alice", "text/plain", "").await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let iri = f
        .index
        .list_children(DATA_KEY_CONTAINER)
        .await
        .unwrap()
        .pop()
        .unwrap();
    for path in [
        format!("/{iri}"),
        format!(
            "/urn%3Ax-sparq-lws%3Adata-keys/{}",
            iri.rsplit('/').next().unwrap()
        ),
    ] {
        for method in ["GET", "PUT", "DELETE"] {
            assert_eq!(
                http.call(method, &path, "text/plain", "").await.status(),
                StatusCode::NOT_FOUND,
                "{method} {path}"
            );
            use tower::ServiceExt;
            let request = axum::http::Request::builder()
                .method(method)
                .uri(&path)
                .body(axum::body::Body::empty())
                .unwrap();
            assert_eq!(
                http.app.clone().oneshot(request).await.unwrap().status(),
                StatusCode::NOT_FOUND
            );
        }
    }
    assert!(f.index.get_meta(&iri).await.is_ok());
}

#[cfg(feature = "sparql-endpoint")]
#[tokio::test]
async fn sparql_cannot_select_key_records_even_with_an_explicit_dataset() {
    use axum::{body::to_bytes, http::StatusCode};
    let f = Fixture::new();
    let http = Http::new(f.encrypted().await).await;
    let iri = f
        .index
        .list_children(DATA_KEY_CONTAINER)
        .await
        .unwrap()
        .pop()
        .unwrap();
    for query in [
        "SELECT ?g ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } }".to_owned(),
        format!("SELECT ?s ?p ?o FROM <{iri}> WHERE {{ ?s ?p ?o }}"),
    ] {
        let response = http
            .call("POST", "/sparql", "application/sparql-query", &query)
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 100000).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(!text.contains(DATA_KEY_CONTAINER));
        assert!(!text.contains("reserved_seals"));
        assert!(!text.contains("SQLK"));
    }
}

#[cfg(feature = "embedded-sparq")]
#[tokio::test]
async fn directory_index_reopen_restores_wrapped_keys_and_reservations() {
    use crate::store::EmbeddedSparqClient;
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let mut suffix = [0; 16];
    getrandom::getrandom(&mut suffix).unwrap();
    let dir = Directory(std::env::temp_dir().join(format!("lane-3-index-{}", hex(&suffix))));
    sparq_core::Graph::load_str("", "ntriples")
        .unwrap()
        .save(&dir.0)
        .unwrap();
    let f = Fixture::new();
    let store = CompositeStore::with_body_cache(
        EmbeddedSparqClient::open(&dir.0).unwrap(),
        f.backend.clone(),
        BodyCache::disabled(),
    )
    .encrypt_blobs(f.provider.clone(), CryptScheme::Solid, f.options())
    .await
    .unwrap();
    let meta = store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let id = parse_header(&f.backend.get(&meta.blob_key).await.unwrap())
        .unwrap()
        .dek_id;
    assert_eq!(
        store.blob.records.load(&id).await.unwrap().reserved_seals,
        BLOCK
    );
    drop(store);
    let store = CompositeStore::with_body_cache(
        EmbeddedSparqClient::open(&dir.0).unwrap(),
        f.backend.clone(),
        BodyCache::disabled(),
    )
    .encrypt_blobs(f.provider.clone(), CryptScheme::Solid, f.options())
    .await
    .unwrap();
    assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    assert_eq!(
        store.blob.records.load(&id).await.unwrap().reserved_seals,
        2 * BLOCK
    );
    assert_eq!(
        store
            .blob
            .key_slot(id)
            .await
            .lock()
            .await
            .as_ref()
            .unwrap()
            .dek
            .seal_count(),
        BLOCK + 1
    );
    assert!(!store
        .sparq
        .referenced_blob_keys()
        .await
        .unwrap()
        .contains(""));
    drop(store);
}

#[tokio::test]
async fn internal_key_records_do_not_consume_content_slots_or_escape_through_edges() {
    let mut f = Fixture::new();
    f.index = Arc::new(InMemorySparqClient::with_limits(
        crate::store::InMemoryStoreLimits::new(100000, 1),
    ));
    let store = f.encrypted().await;
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let record = current(&store, "https://pod.example/alice/").await;
    let iri = record_iri(&record.id().unwrap());
    let meta = f.index.get_meta(&iri).await.unwrap();
    f.index.create_child(ALICE, &iri, meta).await.unwrap();
    assert!(store.list_children(ALICE).await.unwrap().is_empty());
    assert!(store
        .list_children_within(ALICE, 100000)
        .await
        .unwrap()
        .unwrap()
        .is_empty());
    assert!(store.write(BOB, Bytes::new(), "text/plain").await.is_err());
    assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
}

#[tokio::test]
async fn assembly_rechecks_legacy_bodies_with_the_new_decoder() {
    let f = Fixture::new();
    let mut plain = f.plain();
    plain.body_cache = BodyCache::with_max_entry_bytes(64000, 32000);
    let legacy = Bytes::from_static(b"SQLB\x01xxxxxxxxxxxxxxxxyyyyyyyyyyyyold legacy body");
    plain
        .write(ALICE, legacy.clone(), "text/plain")
        .await
        .unwrap();
    assert_eq!(plain.read(ALICE).await.unwrap().body, legacy);
    let encrypted = plain
        .encrypt_blobs(f.provider.clone(), CryptScheme::Solid, f.options())
        .await
        .unwrap();
    assert_eq!(encrypted.body_cache.max_bytes(), 64000);
    assert_eq!(encrypted.body_cache.max_entry_bytes(), 32000);
    assert!(encrypted.read(ALICE).await.is_err());
}
