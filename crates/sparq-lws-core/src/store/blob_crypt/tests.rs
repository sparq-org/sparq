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

use crate::store::tests::common;

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
    unwraps: Arc<AtomicU64>,
    generations: Arc<AtomicU64>,
    refuse: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
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
        unwraps: Arc::new(AtomicU64::new(0)),
        generations: Arc::new(AtomicU64::new(0)),
        refuse: Arc::new(AtomicBool::new(false)),
        pause: Arc::new(AtomicBool::new(false)),
        entered: Arc::default(),
        release: Arc::default(),
    }
}

impl Provider {
    async fn before_call(&self) -> sparq_lws_crypt::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.pause.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        tokio::task::yield_now().await;
        if self.refuse.load(Ordering::SeqCst) {
            return Err(sparq_lws_crypt::Error::ProviderUnavailable);
        }
        Ok(())
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
        self.generations.fetch_add(1, Ordering::SeqCst);
        self.before_call().await?;
        self.inner.generate_data_key(ctx).await
    }
    async fn unwrap(
        &self,
        wrapped: &WrappedKey,
        ctx: &KeyContext,
    ) -> sparq_lws_crypt::Result<SecretKey> {
        self.unwraps.fetch_add(1, Ordering::SeqCst);
        self.before_call().await?;
        self.inner.unwrap(wrapped, ctx).await
    }
}

#[derive(Clone, Default)]
struct Backend {
    bytes: Arc<InMemoryBlobStore>,
    refuse: Arc<AtomicBool>,
    required_scope: Option<BlobScope>,
    unscoped_reads: Arc<AtomicU64>,
}

#[async_trait]
impl BlobStore for Backend {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        self.unscoped_reads.fetch_add(1, Ordering::SeqCst);
        if self.required_scope.is_some() {
            return Err(BlobError::Backend("scoped read required".into()));
        }
        self.bytes.get(key).await
    }
    async fn get_scoped(&self, scope: &BlobScope, key: &str) -> Result<Bytes, BlobError> {
        if self.required_scope.as_ref().is_some_and(|s| s != scope) {
            return Err(BlobError::Backend("unexpected read scope".into()));
        }
        self.bytes.get(key).await
    }
    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        if self.required_scope.is_some() {
            return Err(BlobError::Backend("scoped write required".into()));
        }
        if self.refuse.swap(false, Ordering::SeqCst) {
            return Err(BlobError::Backend("interrupted publication".into()));
        }
        self.bytes.put(key, body).await
    }
    async fn put_scoped(&self, scope: &BlobScope, key: &str, body: Bytes) -> Result<(), BlobError> {
        if self.required_scope.as_ref().is_some_and(|s| s != scope) {
            return Err(BlobError::Backend("unexpected write scope".into()));
        }
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

#[derive(Clone, Default)]
struct ControlledIndex {
    inner: Arc<InMemorySparqClient>,
    // One-shot faults: refuse before commit, lose the reply, or pause after commit.
    fault: Arc<AtomicU64>,
    committed: Arc<tokio::sync::Notify>,
    key_lists: Arc<AtomicU64>,
}

#[async_trait]
impl SparqClient for ControlledIndex {
    async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError> {
        self.inner.get_meta(iri).await
    }
    async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
        self.inner.put_meta(iri, meta).await
    }
    async fn exists(&self, iri: &str) -> Result<bool, SparqError> {
        self.inner.exists(iri).await
    }
    fn exclusive(&self) -> Result<(), SparqError> {
        self.inner.exclusive()
    }
    fn lifecycle(&self) -> Arc<super::super::Lifecycle> {
        self.inner.lifecycle()
    }
    async fn delete_meta(&self, iri: &str) -> Result<(), SparqError> {
        self.inner.delete_meta(iri).await
    }
    async fn delete_meta_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<crate::store::DeleteOutcome, SparqError> {
        self.inner.delete_meta_if_empty(iri, parent).await
    }
    async fn create_child(
        &self,
        container: &str,
        child: &str,
        meta: ResourceMeta,
    ) -> Result<(), SparqError> {
        let fault = if container == DATA_KEY_CONTAINER {
            self.fault.swap(0, Ordering::SeqCst)
        } else {
            0
        };
        let failure = || SparqError::Backend("private index detail".into());
        match fault {
            1 => return Err(failure()),
            4 => return Err(SparqError::QuotaExceeded),
            5 => return Err(SparqError::ServicePoolFull),
            _ => {}
        }
        self.inner.create_child(container, child, meta).await?;
        if fault == 2 {
            return Err(failure());
        }
        if fault == 3 {
            self.committed.notify_one();
            std::future::pending::<()>().await;
        }
        Ok(())
    }
    async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError> {
        self.inner.remove_child(container, child).await
    }
    async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError> {
        if container == DATA_KEY_CONTAINER {
            self.key_lists.fetch_add(1, Ordering::SeqCst);
        }
        self.inner.list_children(container).await
    }
    async fn referenced_blob_keys(&self) -> Result<std::collections::HashSet<String>, SparqError> {
        self.inner.referenced_blob_keys().await
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
            blob: Arc::new(self.backend.clone()),
            blob_scope: BlobScopePolicy::Solid,
            body_cache: Arc::new(BodyCache::disabled()),
            lifecycle: Default::default(),
            history: Default::default(),
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

#[tokio::test]
async fn encrypted_pool_capacity_is_typed_only_for_resource_creates() {
    use crate::{error::ServerError, store::InMemoryStoreLimits};

    for blob_limit in [true, false] {
        let mut f = Fixture::new();
        let limits = InMemoryStoreLimits {
            max_pool_resource_count: 2,
            ..Default::default()
        };
        if blob_limit {
            f.backend.bytes = Arc::new(InMemoryBlobStore::with_limits(limits));
        } else {
            f.index = Arc::new(InMemorySparqClient::with_limits(limits));
        }
        let store = f.encrypted().await;
        let container = "https://pod.example/.lws/subscriptions/";
        let first = format!("{container}first");
        let next = format!("{container}next");
        store
            .write(container, Bytes::new(), "text/turtle")
            .await
            .unwrap();
        let meta = store
            .create_in_container(container, &first, Bytes::from_static(BODY), "text/plain")
            .await
            .unwrap();
        assert!(meta.blob_key.starts_with("$pool$e1-"));
        assert!(is_envelope(
            &f.backend.bytes.get(&meta.blob_key).await.unwrap()
        ));
        assert_eq!(store.read(&first).await.unwrap().body, BODY);
        assert!(matches!(
            store
                .create_in_container(container, &next, Bytes::from_static(BODY), "text/plain")
                .await,
            Err(ServerError::ServicePoolFull)
        ));
        assert!(matches!(
            store
                .write(&next, Bytes::from_static(BODY), "text/plain")
                .await,
            Err(ServerError::InsufficientStorage)
        ));
        assert!(matches!(
            store
                .restore(&next, Some(container), Bytes::from_static(BODY), &meta)
                .await,
            Err(ServerError::InsufficientStorage)
        ));
        store
            .write(ALICE, Bytes::from_static(BODY), "text/plain")
            .await
            .unwrap();
        assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
    }
}

#[tokio::test]
async fn lws_subscriptions_encrypt_after_a_plaintext_boot_and_keep_the_pool_signal() {
    use crate::{
        error::ServerError,
        lws::{LwsConfig, LWS_JSON, SUBSCRIPTIONS_PATH},
        store::InMemoryStoreLimits,
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let mut f = Fixture::new();
    f.backend.bytes = Arc::new(InMemoryBlobStore::with_limits(
        InMemoryStoreLimits::default().with_pool(2, 1024 * 1024),
    ));
    let plain = CompositeStore::with_body_cache(
        (*f.index).clone(),
        f.backend.clone(),
        BodyCache::disabled(),
    );
    let mut cfg = LwsConfig::new("https://pod.example");
    let owner = "https://owner.example/";
    cfg.owner = Some(owner.into());
    let container = cfg.absolute(SUBSCRIPTIONS_PATH);
    let legacy = plain
        .write(&container, Bytes::new(), LWS_JSON)
        .await
        .unwrap();
    assert!(legacy.blob_key.starts_with("$pool$b1-"));
    let store = plain
        .encrypt_blobs(f.provider.clone(), CryptScheme::Lws, f.options())
        .await
        .unwrap();
    let token = crate::lws::tokens::mint(&cfg, owner, "https://app.example/", &cfg.realm());
    let body = serde_json::json!({"type": "SseSubscription", "topic": [cfg.storage()]}).to_string();
    let app = crate::lws::router(store.clone(), cfg).await.unwrap();
    // An encrypted reopen alone leaves the original pool container in legacy form.
    assert!(!f
        .backend
        .bytes
        .list()
        .await
        .unwrap()
        .iter()
        .any(|entry| entry.key.starts_with("$pool$e1-")));
    let request = || {
        Request::builder()
            .method("POST")
            .uri(SUBSCRIPTIONS_PATH)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", LWS_JSON)
            .body(Body::from(body.clone()))
            .unwrap()
    };
    let created = app.clone().oneshot(request()).await.unwrap();
    let (parts, body) = created.into_parts();
    let body = axum::body::to_bytes(body, 65536).await.unwrap();
    assert_eq!(
        parts.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let iri = parts.headers["location"].to_str().unwrap();
    let meta = f.index.get_meta(iri).await.unwrap();
    assert!(meta.blob_key.starts_with("$pool$e1-"));
    assert!(is_envelope(
        &f.backend.bytes.get(&meta.blob_key).await.unwrap()
    ));
    assert!(
        serde_json::from_slice::<serde_json::Value>(&store.read(iri).await.unwrap().body).is_ok()
    );
    assert!(matches!(
        store
            .create_in_container(
                &container,
                &format!("{container}next"),
                Bytes::from_static(b"{}"),
                LWS_JSON
            )
            .await,
        Err(ServerError::ServicePoolFull)
    ));
    assert_eq!(
        app.oneshot(request()).await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn encrypted_key_record_capacity_is_general_storage_capacity() {
    use crate::error::ServerError;

    let f = Fixture::new();
    let index = ControlledIndex::default();
    let store = CompositeStore::new(index.clone(), f.backend.clone())
        .encrypt_blobs(f.provider.clone(), CryptScheme::Solid, f.options())
        .await
        .unwrap();
    for fault in [4, 5] {
        index.fault.store(fault, Ordering::SeqCst);
        assert!(matches!(
            store
                .create_in_container(
                    "https://pod.example/.lws/subscriptions/",
                    "https://pod.example/.lws/subscriptions/next",
                    Bytes::from_static(BODY),
                    "text/plain"
                )
                .await,
            Err(ServerError::InsufficientStorage)
        ));
        assert!(f.backend.bytes.list().await.unwrap().is_empty());
    }
}

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
async fn seal_count<S: SparqClient>(
    store: &CompositeStore<S, EncryptingBlobStore<Backend, Provider>>,
    id: [u8; 16],
) -> u64 {
    store
        .blob
        .key_slot(id)
        .await
        .lock()
        .await
        .value
        .as_ref()
        .unwrap()
        .dek
        .seal_count()
}

#[tokio::test]
async fn boot_unwraps_only_a_probe_and_current_keys_and_reads_retired_keys_lazily() {
    let f = Fixture::new();
    let mut options = f.options();
    options.block = 1;
    options.rotate_at = 2;
    let store = f
        .plain()
        .encrypt_blobs(f.provider.clone(), CryptScheme::Solid, options)
        .await
        .unwrap();
    let mut bodies = Vec::new();
    for i in 0..40 {
        let iri = format!("{ALICE}/{i}");
        let meta = store
            .write(&iri, Bytes::from_static(BODY), "text/plain")
            .await
            .unwrap();
        let id = parse_header(&f.backend.bytes.get(&meta.blob_key).await.unwrap())
            .unwrap()
            .dek_id;
        bodies.push((iri, id));
    }
    store
        .write(BOB, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let records = store.blob.records.all().await.unwrap();
    assert!(records.len() > 10);
    let currents = records.iter().filter(|r| r.current).count() as u64;
    let generations = f.provider.generations.load(Ordering::SeqCst);
    drop(store);
    f.provider.unwraps.store(0, Ordering::SeqCst);
    let store = Arc::new(f.encrypted().await);
    let boot_unwraps = f.provider.unwraps.load(Ordering::SeqCst);
    assert!((currents..=currents + 1).contains(&boot_unwraps));
    assert_eq!(f.provider.generations.load(Ordering::SeqCst), generations);
    for record in records.iter().filter(|r| r.current) {
        assert!(store
            .blob
            .keys
            .lock()
            .await
            .contains_key(&record.id().unwrap()));
        let pod = store.blob.pod_slot(&record.pod).await;
        assert!(
            matches!(pod.lock().await.value, Some(PodKey::Current(id)) if id == record.id().unwrap())
        );
    }
    // Choose a retired key other than the optional boot probe.
    let (iri, id) = bodies
        .into_iter()
        .find(|(_, id)| {
            records.iter().any(|r| !r.current && r.id().unwrap() == *id)
                && !store.blob.keys.try_lock().unwrap().contains_key(id)
        })
        .unwrap();
    assert!(!store.blob.keys.lock().await.contains_key(&id));
    let mut readers = Vec::new();
    for _ in 0..12 {
        let store = store.clone();
        let iri = iri.clone();
        readers.push(tokio::spawn(async move {
            assert_eq!(store.read(&iri).await.unwrap().body, BODY);
        }));
    }
    for reader in readers {
        reader.await.unwrap();
    }
    assert_eq!(f.provider.unwraps.load(Ordering::SeqCst), boot_unwraps + 1);
    assert_eq!(store.read(&iri).await.unwrap().body, BODY);
    assert_eq!(f.provider.unwraps.load(Ordering::SeqCst), boot_unwraps + 1);
}

#[tokio::test]
async fn boot_with_only_retired_keys_still_refuses_a_wrong_kek() {
    let f = Fixture::new();
    let store = f.encrypted().await;
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let mut record = current(&store, "https://pod.example/alice/").await;
    record.current = false;
    store.blob.records.save(&record).await.unwrap();
    drop(store);
    let wrong = provider(0x41);
    let result = f
        .plain()
        .encrypt_blobs(wrong.clone(), CryptScheme::Solid, f.options())
        .await;
    assert!(matches!(result, Err(BlobError::Backend(ref e)) if e == "wrong KEK"));
    assert_eq!(wrong.unwraps.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn boot_checks_the_local_kek_identity_of_every_retired_record() {
    let f = Fixture::new();
    let store = f.encrypted().await;
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let record = current(&store, "https://pod.example/alice/").await;
    for id in [0xa7, 0xa8] {
        let mut retired = record.clone();
        retired.dek_id = hex(&[id; 16]);
        retired.current = false;
        store.blob.records.save(&retired).await.unwrap();
    }
    // Choose a retired entry after the boot probe in the stable child listing.
    let mut retired = store
        .blob
        .records
        .all()
        .await
        .unwrap()
        .into_iter()
        .skip(1)
        .find(|r| !r.current)
        .unwrap();
    let other = provider(0x41);
    let (_, wrapped) = other.generate_data_key(&retired.context()).await.unwrap();
    retired.wrapped = wrapped.to_bytes().unwrap();
    store.blob.records.save(&retired).await.unwrap();
    drop(store);
    let result = f
        .plain()
        .encrypt_blobs(f.provider.clone(), CryptScheme::Solid, f.options())
        .await;
    assert!(matches!(result, Err(BlobError::Backend(ref e)) if e == "wrong KEK"));
}

#[tokio::test]
async fn boot_checks_the_provider_of_every_retired_record() {
    let f = Fixture::new();
    let store = f.encrypted().await;
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let record = current(&store, "https://pod.example/alice/").await;
    for id in [0xa7, 0xa8] {
        let mut retired = record.clone();
        retired.dek_id = hex(&[id; 16]);
        retired.current = false;
        store.blob.records.save(&retired).await.unwrap();
    }
    let mut retired = store
        .blob
        .records
        .all()
        .await
        .unwrap()
        .into_iter()
        .skip(1)
        .find(|r| !r.current)
        .unwrap();
    let mut wrapped = WrappedKey::from_bytes(&retired.wrapped).unwrap();
    wrapped.provider_id = "another-provider".into();
    retired.wrapped = wrapped.to_bytes().unwrap();
    store.blob.records.save(&retired).await.unwrap();
    drop(store);
    let result = f
        .plain()
        .encrypt_blobs(f.provider.clone(), CryptScheme::Solid, f.options())
        .await;
    assert!(matches!(result, Err(BlobError::Backend(ref e)) if e == "wrong key provider"));
}

#[tokio::test]
async fn completed_unknown_key_reads_release_their_cache_slots() {
    let f = Fixture::new();
    let store = f.encrypted().await;
    let meta = f
        .plain()
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    for id in 0..12u8 {
        let mut envelope = b"SQLB\x01".to_vec();
        envelope.extend_from_slice(&[id; 16]);
        envelope.extend_from_slice(&[0; 28]);
        f.backend
            .bytes
            .put(&meta.blob_key, Bytes::from(envelope))
            .await
            .unwrap();
        assert!(matches!(
            store.read(ALICE).await,
            Err(crate::error::ServerError::Storage(ref e)) if e == "UnknownDataKey"
        ));
        assert!(store.blob.keys.lock().await.is_empty());
    }
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn completed_generation_failures_release_empty_pod_slots() {
    let f = Fixture::new();
    let store = f.encrypted().await;
    f.provider.refuse.store(true, Ordering::SeqCst);
    for id in 0..12 {
        let iri = format!("https://pod.example/p{id}/file");
        assert!(store
            .write(&iri, Bytes::from_static(BODY), "text/plain")
            .await
            .is_err());
        assert!(store.blob.pods.lock().await.is_empty());
        assert!(store.blob.keys.lock().await.is_empty());
    }
    assert!(store.blob.records.all().await.unwrap().is_empty());
    assert!(f.backend.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn unknown_key_reads_emit_only_the_blob_key() {
    use tracing::instrument::WithSubscriber;

    struct Capture(Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let f = Fixture::new();
    let store = f.encrypted().await;
    let meta = f
        .plain()
        .write(
            ALICE,
            Bytes::from_static(b"SQLB\x01xxxxxxxxxxxxxxxxyyyyyyyyyyyyprivate legacy body"),
            "text/plain",
        )
        .await
        .unwrap();
    let output = Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_level(false)
        .with_target(false)
        .with_writer(move || Capture(writer.clone()))
        .finish();
    let result = store.read(ALICE).with_subscriber(subscriber).await;
    assert!(result.is_err());
    let diagnostic = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert_eq!(
        diagnostic,
        format!("UnknownDataKey blob_key={:?}\n", meta.blob_key)
    );
}

#[tokio::test]
async fn first_writes_for_new_pods_do_not_enumerate_key_records() {
    let f = Fixture::new();
    let index = ControlledIndex::default();
    let store =
        CompositeStore::with_body_cache(index.clone(), f.backend.clone(), BodyCache::disabled())
            .encrypt_blobs(f.provider.clone(), CryptScheme::Solid, f.options())
            .await
            .unwrap();
    let lists = index.key_lists.load(Ordering::SeqCst);
    for i in 0..12 {
        let iri = format!("https://pod.example/pod{i}/body");
        store
            .write(&iri, Bytes::from_static(BODY), "text/plain")
            .await
            .unwrap();
        assert_eq!(store.read(&iri).await.unwrap().body, BODY);
        assert_eq!(index.key_lists.load(Ordering::SeqCst), lists);
    }
    assert_eq!(store.blob.records.all().await.unwrap().len(), 12);
    assert_eq!(f.provider.generations.load(Ordering::SeqCst), 12);
}

#[tokio::test]
async fn encrypted_writes_require_envelopes_on_read_read_at_and_http_get() {
    use crate::error::ServerError;
    use axum::{body::to_bytes, http::StatusCode};

    for pooled in [false, true] {
        let f = Fixture::new();
        let store = f.encrypted().await;
        let iri = if pooled {
            "https://pod.example/.lws/subscriptions/body"
        } else {
            ALICE
        };
        let meta = store
            .write(iri, Bytes::from_static(BODY), "text/plain")
            .await
            .unwrap();
        assert!(envelope_key(&meta.blob_key));
        assert_eq!(meta.blob_key.starts_with("$pool$"), pooled);
        assert_eq!(store.read(iri).await.unwrap().body, BODY);
        assert!(store
            .blob
            .put_scoped(
                &BlobScope("https://pod.example/alice/".into()),
                "b1-00000000000000000000000000000000",
                Bytes::from_static(BODY)
            )
            .await
            .is_err());
        f.backend
            .bytes
            .put(&meta.blob_key, Bytes::from_static(b"non-envelope body"))
            .await
            .unwrap();
        assert!(
            matches!(store.read(iri).await, Err(ServerError::Storage(ref e)) if e.contains("requires an envelope"))
        );
        assert!(
            matches!(store.read_at(iri, &meta).await, Err(ServerError::Storage(ref e)) if e.contains("requires an envelope"))
        );
        drop(store);
        let reopened = f.encrypted().await;
        assert!(
            matches!(reopened.read(iri).await, Err(ServerError::Storage(ref e)) if e.contains("requires an envelope"))
        );
        if !pooled {
            let http = Http::new(reopened).await;
            let response = http.call("GET", "/alice/file", "text/plain", "").await;
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let body = to_bytes(response.into_body(), 10000).await.unwrap();
            assert!(!body
                .windows(b"non-envelope body".len())
                .any(|w| w == b"non-envelope body"));
        }
    }
}

#[tokio::test]
async fn resource_reads_and_http_get_use_scoped_blob_reads() {
    use axum::{body::to_bytes, http::StatusCode};
    let f = Fixture::new();
    let store = f.encrypted().await;
    let meta = store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
    assert_eq!(store.read_at(ALICE, &meta).await.unwrap(), BODY);
    let http = Http::new(store).await;
    let response = http.call("GET", "/alice/file", "text/plain", "").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_bytes(response.into_body(), 10000).await.unwrap(), BODY);
    assert_eq!(f.backend.unscoped_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unscoped_reads_authenticate_by_key_and_scoped_reads_use_the_resource_pod() {
    let f = Fixture::new();
    let store = f.encrypted().await;
    let meta = store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    assert_eq!(store.blob.get(&meta.blob_key).await.unwrap(), BODY);
    assert!(store
        .blob
        .get_scoped(&BlobScope("https://pod.example/bob/".into()), &meta.blob_key)
        .await
        .is_err());
    let mut bytes = f.backend.bytes.get(&meta.blob_key).await.unwrap().to_vec();
    *bytes.last_mut().unwrap() ^= 1;
    f.backend.bytes.put(&meta.blob_key, bytes.into()).await.unwrap();
    assert!(store.blob.get(&meta.blob_key).await.is_err());
}

#[tokio::test]
async fn pod_reads_require_the_original_pod_with_cold_and_warm_body_cache() {
    use crate::{
        acl_cache::AclCache,
        authz::{AccessMode, WacAuthorizer},
        error::ServerError,
    };

    for path in ["read", "read_at", "acl"] {
        for warm in [false, true] {
            let f = Fixture::new();
            let mut store = f.encrypted().await;
            store.body_cache = Arc::new(BodyCache::new(65536));
            let source = if path == "acl" {
                "https://pod.example/alice/file.acl"
            } else {
                ALICE
            };
            let target = if path == "acl" {
                "https://pod.example/bob/file.acl"
            } else {
                BOB
            };
            let original = store
                .write(source, Bytes::from_static(BODY), "text/plain")
                .await
                .unwrap();
            if warm {
                assert_eq!(store.read(source).await.unwrap().body, BODY);
                assert!(store
                    .body_cache
                    .get_for_pod(
                        "https://pod.example/alice/",
                        &original.blob_key,
                        &original.etag
                    )
                    .unwrap()
                    .is_some());
            }
            // The index edit is confined to this test.
            f.index
                .replace_meta(target, original.clone())
                .await
                .unwrap();
            let result = match path {
                "read" => store.read(target).await.map(|_| ()),
                "read_at" => {
                    let plan = store.read_plan(target, &[]).await.unwrap();
                    store
                        .read_at(target, &plan.target.unwrap())
                        .await
                        .map(|_| ())
                }
                "acl" => {
                    let acl_cache = AclCache::new(16);
                    let auth =
                        WacAuthorizer::with_cache(&store, "https://pod.example/", &acl_cache);
                    let candidates = auth.read_plan_candidates(BOB);
                    let iris = candidates.iter().map(|c| c.acl.clone()).collect::<Vec<_>>();
                    let plan = store.read_plan(BOB, &iris).await.unwrap();
                    auth.authorize_read_planned(
                        AccessMode::Read,
                        None,
                        None,
                        &candidates,
                        &plan.acls,
                    )
                    .await
                    .map(|_| ())
                }
                _ => unreachable!(),
            };
            let expected = if warm {
                "cached body pod"
            } else {
                "data-key pod"
            };
            assert!(
                matches!(result, Err(ServerError::Storage(ref e)) if e.contains(expected)),
                "{path} warm={warm}: {result:?}"
            );
            assert!(store
                .body_cache
                .get_for_pod(
                    "https://pod.example/alice/",
                    &original.blob_key,
                    &original.etag
                )
                .unwrap()
                .is_none());
            assert_eq!(store.read(source).await.unwrap().body, BODY);
        }
    }
}

#[tokio::test]
async fn parsed_acl_cache_rechecks_changed_blob_pointers_with_the_same_etag() {
    use crate::{
        acl_cache::AclCache,
        authz::{AccessMode, ReadDecision, WacAuthorizer},
        error::ServerError,
    };

    for planned in [false, true] {
        let f = Fixture::new();
        let store = f.encrypted().await;
        let alice_acl = format!("{ALICE}.acl");
        let bob_acl = format!("{BOB}.acl");
        let rules = format!(
            "@prefix acl: <http://www.w3.org/ns/auth/acl#>. \
             <urn:rule> a acl:Authorization; acl:agent <{}>; \
             acl:accessTo <{ALICE}>, <{BOB}>; acl:mode acl:Read.",
            common::WEBID
        );
        let alice = store
            .write(&alice_acl, Bytes::from(rules.clone()), "text/turtle")
            .await
            .unwrap();
        let bob = store
            .write(&bob_acl, rules.into(), "text/turtle")
            .await
            .unwrap();
        assert_eq!(alice.etag, bob.etag);
        assert_ne!(alice.blob_key, bob.blob_key);
        let cache = AclCache::new(16);
        let auth = WacAuthorizer::with_cache(&store, common::BASE_URL, &cache);
        for _ in 0..2 {
            assert!(matches!(
                auth.authorize_read(BOB, AccessMode::Read, Some(common::WEBID), None)
                    .await
                    .unwrap(),
                ReadDecision::Allow(_)
            ));
        }
        assert!(!cache.is_empty());
        f.index.replace_meta(&bob_acl, alice).await.unwrap();
        let result = if planned {
            let candidates = auth.read_plan_candidates(BOB);
            let iris = candidates.iter().map(|c| c.acl.clone()).collect::<Vec<_>>();
            let plan = store.read_plan(BOB, &iris).await.unwrap();
            auth.authorize_read_planned(
                AccessMode::Read,
                Some(common::WEBID),
                None,
                &candidates,
                &plan.acls,
            )
            .await
        } else {
            auth.authorize_read(BOB, AccessMode::Read, Some(common::WEBID), None)
                .await
        };
        assert!(
            matches!(result, Err(ServerError::Storage(ref e)) if e.contains("data-key pod")),
            "planned={planned}: {result:?}"
        );
    }
}

#[tokio::test]
async fn inserting_a_dek_sweeps_expired_keys_without_regenerating_them() {
    let f = Fixture::new();
    let store = f.encrypted().await;
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let alice = current(&store, "https://pod.example/alice/")
        .await
        .id()
        .unwrap();
    f.clock.0.store(DEFAULT_DEK_TTL.as_secs(), Ordering::SeqCst);
    store
        .write(BOB, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    assert!(!store.blob.keys.lock().await.contains_key(&alice));
    let bob = current(&store, "https://pod.example/bob/")
        .await
        .id()
        .unwrap();
    assert!(store.blob.keys.lock().await.contains_key(&bob));
    assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
    assert_eq!(
        current(&store, "https://pod.example/alice/")
            .await
            .id()
            .unwrap(),
        alice
    );
    assert_eq!(store.blob.records.all().await.unwrap().len(), 2);
}

#[tokio::test]
async fn encryption_forwards_scoped_operations_to_its_backend() {
    let mut f = Fixture::new();
    f.backend.required_scope = Some(BlobScope("https://pod.example/alice/".into()));
    let store = f.encrypted().await;
    let meta = store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    assert!(f
        .backend
        .bytes
        .get(&meta.blob_key)
        .await
        .unwrap()
        .starts_with(b"SQLB"));
    assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
    assert_eq!(store.read_at(ALICE, &meta).await.unwrap(), BODY);
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
    assert!(store.reinstate(BOB, None, &held).await.is_err());
    assert!(!store.exists(BOB).await.unwrap());
    assert!(matches!(
        store.read(BOB).await,
        Err(crate::error::ServerError::NotFound)
    ));
    store.delete(ALICE, None).await.unwrap();
    store.reinstate(ALICE, None, &held).await.unwrap();
    assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
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
        blob: Arc::new(CountingBlobStore::new(
            Arc::try_unwrap(store.blob).ok().unwrap(),
            counters.clone(),
        )),
        blob_scope: BlobScopePolicy::Solid,
        body_cache: Arc::new(BodyCache::disabled()),
        lifecycle: store.lifecycle,
        history: store.history,
        holding: Default::default(),
    };
    counted
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    assert_eq!(counters.snapshot().blob_puts, 1);
    let meta = counted.meta(ALICE).await.unwrap().unwrap();
    counted.sparq.replace_meta(BOB, meta).await.unwrap();
    assert!(
        matches!(counted.read(BOB).await, Err(crate::error::ServerError::Storage(e)) if e.contains("data-key pod"))
    );
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
            CryptScheme::Lws,
            BlobScopePolicy::LwsStorage(BlobScope("https://pod.example/storage/".into())),
            "https://pod.example/storage/",
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
            current(
                &store,
                if matches!(store.blob_scope, BlobScopePolicy::LwsStorage(_)) {
                    expected
                } else {
                    "https://pod.example/"
                },
            )
            .await
            .scheme,
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
    let id = store.blob.generate(&scope, &mut None).await.unwrap();
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
async fn index_commit_errors_never_publish_unreserved_envelopes() {
    // Fail initial reservation, extension, and retirement, both before and after
    // the index actually commits. Every case runs through the resource write path.
    for fault in [1, 2] {
        for prior_writes in 0..3 {
            let f = Fixture::new();
            let index = ControlledIndex::default();
            let mut options = f.options();
            options.block = 1;
            options.rotate_at = 2;
            let store = CompositeStore::with_body_cache(
                index.clone(),
                f.backend.clone(),
                BodyCache::disabled(),
            )
            .encrypt_blobs(f.provider.clone(), CryptScheme::Solid, options)
            .await
            .unwrap();
            for i in 0..prior_writes {
                store
                    .write(
                        &format!("{ALICE}/{i}"),
                        Bytes::from_static(BODY),
                        "text/plain",
                    )
                    .await
                    .unwrap();
            }
            let before = f.backend.list().await.unwrap();
            index.fault.store(fault, Ordering::SeqCst);
            let error = store
                .write(ALICE, Bytes::from_static(BODY), "text/plain")
                .await
                .expect_err("a failed key-record commit must fail the resource write");
            assert!(!error.to_string().contains("private index detail"));
            assert!(matches!(
                index.get_meta(ALICE).await,
                Err(SparqError::NotFound)
            ));
            let after = f.backend.list().await.unwrap();
            assert_eq!(after.len(), before.len());
            for entry in before {
                assert_eq!(store.blob.get(&entry.key).await.unwrap(), BODY);
            }
            store
                .write(ALICE, Bytes::from_static(BODY), "text/plain")
                .await
                .unwrap();
            assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
            let records = store.blob.records.all().await.unwrap();
            assert_eq!(records.iter().filter(|r| r.current).count(), 1);
            assert!(records.iter().all(|r| r.reserved_seals > 0));
        }
    }
}

#[tokio::test]
async fn cancelled_first_writer_recovers_its_committed_key_without_regenerating() {
    let f = Fixture::new();
    let index = ControlledIndex::default();
    let store = Arc::new(
        CompositeStore::with_body_cache(index.clone(), f.backend.clone(), BodyCache::disabled())
            .encrypt_blobs(f.provider.clone(), CryptScheme::Solid, f.options())
            .await
            .unwrap(),
    );
    index.fault.store(3, Ordering::SeqCst);
    let writer = store.clone();
    let task = tokio::spawn(async move {
        writer
            .write(ALICE, Bytes::from_static(BODY), "text/plain")
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), index.committed.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(f.backend.list().await.unwrap().is_empty());
    let records = store.blob.records.all().await.unwrap();
    assert_eq!(records.len(), 1);
    let id = records[0].id().unwrap();
    assert_eq!(records[0].reserved_seals, BLOCK);
    let lists = index.key_lists.load(Ordering::SeqCst);
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    assert_eq!(index.key_lists.load(Ordering::SeqCst), lists);
    let records = store.blob.records.all().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reserved_seals, 2 * BLOCK);
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 2);
    assert_eq!(f.provider.generations.load(Ordering::SeqCst), 1);
    let slot = store.blob.key_slot(id).await;
    assert_eq!(
        slot.lock().await.value.as_ref().unwrap().dek.seal_count(),
        BLOCK + 1
    );
    assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
}

#[tokio::test]
async fn reopen_after_retirement_before_generation_preserves_bodies_and_seal_counts() {
    let f = Fixture::new();
    let index = ControlledIndex::default();
    let options = || CryptOptions {
        block: 2,
        rotate_at: 4,
        ..f.options()
    };
    let open = || {
        CompositeStore::with_body_cache(index.clone(), f.backend.clone(), BodyCache::disabled())
            .encrypt_blobs(f.provider.clone(), CryptScheme::Solid, options())
    };
    let store = Arc::new(open().await.unwrap());
    let mut old_bodies = Vec::new();
    for i in 0..4 {
        let iri = format!("{ALICE}/{i}");
        let meta = store
            .write(&iri, Bytes::from_static(BODY), "text/plain")
            .await
            .unwrap();
        old_bodies.push((iri, meta));
    }
    let old_id = parse_header(
        &f.backend
            .bytes
            .get(&old_bodies[0].1.blob_key)
            .await
            .unwrap(),
    )
    .unwrap()
    .dek_id;
    assert_eq!(seal_count(&store, old_id).await, 4);
    let generations = f.provider.generations.load(Ordering::SeqCst);
    // Stop after the retirement commit, before its reply or any generation call.
    index.fault.store(3, Ordering::SeqCst);
    let writer = store.clone();
    let task = tokio::spawn(async move {
        writer
            .write(ALICE, Bytes::from_static(BODY), "text/plain")
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), index.committed.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let records = store.blob.records.all().await.unwrap();
    assert_eq!(records.len(), 1);
    assert!(!records[0].current);
    assert_eq!(records[0].reserved_seals, 4);
    assert_eq!(f.provider.generations.load(Ordering::SeqCst), generations);
    drop(store);

    let store = open().await.unwrap();
    assert_eq!(f.provider.generations.load(Ordering::SeqCst), generations);
    let meta = store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let new_id = parse_header(&f.backend.bytes.get(&meta.blob_key).await.unwrap())
        .unwrap()
        .dek_id;
    assert_ne!(old_id, new_id);
    assert_eq!(seal_count(&store, new_id).await, 1);
    assert_eq!(
        f.provider.generations.load(Ordering::SeqCst),
        generations + 1
    );
    let records = store.blob.records.all().await.unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records.iter().filter(|r| r.current).count(), 1);
    for (iri, _) in &old_bodies {
        assert_eq!(store.read(iri).await.unwrap().body, BODY);
    }
    assert_eq!(seal_count(&store, old_id).await, 4);
    drop(store);

    let store = open().await.unwrap();
    assert_eq!(seal_count(&store, new_id).await, 2);
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    assert_eq!(seal_count(&store, new_id).await, 3);
    assert_eq!(
        f.provider.generations.load(Ordering::SeqCst),
        generations + 1
    );
    assert_eq!(
        store
            .blob
            .records
            .load(&old_id)
            .await
            .unwrap()
            .reserved_seals,
        4
    );
    assert_eq!(
        store
            .blob
            .records
            .load(&new_id)
            .await
            .unwrap()
            .reserved_seals,
        4
    );
    for (iri, _) in &old_bodies {
        assert_eq!(store.read(iri).await.unwrap().body, BODY);
    }
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
async fn failed_unwrap_is_shared_by_queued_reads_and_writes_then_retried() {
    async fn access(store: &Encrypted, write: bool) -> crate::error::ServerResult<()> {
        if write {
            store
                .write(ALICE, Bytes::from_static(BODY), "text/plain")
                .await
                .map(|_| ())
        } else {
            store.read(ALICE).await.map(|_| ())
        }
    }
    for first_writes in [false, true] {
        let f = Fixture::new();
        let store = f.encrypted().await;
        let meta = store
            .write(ALICE, Bytes::from_static(BODY), "text/plain")
            .await
            .unwrap();
        f.clock.0.store(DEFAULT_DEK_TTL.as_secs(), Ordering::SeqCst);
        f.provider.refuse.store(true, Ordering::SeqCst);
        f.provider.pause.store(true, Ordering::SeqCst);
        let mut first = Box::pin(access(&store, first_writes));
        assert!(futures_util::poll!(first.as_mut()).is_pending());
        f.provider.entered.notified().await;
        // Poll every request into the held key/pod slots before ending the flight.
        let mut joined = Vec::new();
        for write in [false, true, false, true] {
            let mut request = Box::pin(access(&store, write));
            assert!(futures_util::poll!(request.as_mut()).is_pending());
            joined.push(request);
        }
        f.provider.release.notify_one();
        assert!(first.await.is_err());
        for request in joined {
            assert!(request.await.is_err());
        }
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 2);
        assert_eq!(f.index.get_meta(ALICE).await.unwrap(), meta);
        assert_eq!(f.backend.list().await.unwrap().len(), 1);
        f.provider.refuse.store(false, Ordering::SeqCst);
        assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
        store
            .write(ALICE, Bytes::from_static(BODY), "text/plain")
            .await
            .unwrap();
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 3);
        assert_eq!(store.blob.records.all().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn failed_generation_is_shared_by_queued_writers_then_retried() {
    let f = Fixture::new();
    let store = f.encrypted().await;
    f.provider.refuse.store(true, Ordering::SeqCst);
    f.provider.pause.store(true, Ordering::SeqCst);
    let mut first = store.write(ALICE, Bytes::from_static(BODY), "text/plain");
    assert!(futures_util::poll!(first.as_mut()).is_pending());
    f.provider.entered.notified().await;
    let mut joined = Vec::new();
    for _ in 0..4 {
        let mut request = store.write(ALICE, Bytes::from_static(BODY), "text/plain");
        assert!(futures_util::poll!(request.as_mut()).is_pending());
        joined.push(request);
    }
    f.provider.release.notify_one();
    assert!(first.await.is_err());
    for request in joined {
        assert!(request.await.is_err());
    }
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    assert!(store.blob.records.all().await.unwrap().is_empty());
    assert!(f.backend.list().await.unwrap().is_empty());
    assert!(store.meta(ALICE).await.unwrap().is_none());
    f.provider.refuse.store(false, Ordering::SeqCst);
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 2);
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
    assert!(store
        .read_plan(ALICE, std::slice::from_ref(&iri))
        .await
        .is_err());
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
        store.blob.as_ref(),
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
    async fn new<S: Store + 'static>(store: S) -> Self {
        store
            .write("https://pod.example/", Bytes::new(), "text/turtle")
            .await
            .unwrap();
        let acl = format!("@prefix acl: <http://www.w3.org/ns/auth/acl#>. <#owner> a acl:Authorization; acl:agent <{}>; acl:accessTo <https://pod.example/>; acl:default <https://pod.example/>; acl:mode acl:Read, acl:Write, acl:Control.", common::WEBID);
        store
            .write("https://pod.example/.acl", acl.into(), "text/turtle")
            .await
            .unwrap();
        Self::from_store(store)
    }
    fn from_store<S: Store + 'static>(store: S) -> Self {
        use solid_oidc_verifier::{
            config::VerifierConfig, replay::InMemoryReplayStore, verifier::Verifier,
        };
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

#[cfg(all(feature = "blob-fs", feature = "embedded-sparq"))]
#[tokio::test]
async fn filesystem_http_round_trip_encrypts_disk_bytes_and_reopens_every_resource() {
    use crate::store::{blob_fs::FsBlobStore, EmbeddedSparqClient};
    use axum::{body::to_bytes, http::StatusCode};

    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let mut suffix = [0; 16];
    getrandom::getrandom(&mut suffix).unwrap();
    // A private directory gives this test exclusive ownership through both opens.
    let directory =
        Directory(std::env::temp_dir().join(format!("lws-encrypted-filesystem-{}", hex(&suffix))));
    let index_dir = directory.0.join("index");
    let blob_dir = directory.0.join("blobs");
    sparq_core::Graph::load_str("", "ntriples")
        .unwrap()
        .save(&index_dir)
        .unwrap();
    let store = CompositeStore::with_body_cache(
        EmbeddedSparqClient::open(&index_dir).unwrap(),
        FsBlobStore::open(&blob_dir).unwrap(),
        BodyCache::disabled(),
    )
    .encrypt_blobs(provider(0x41), CryptScheme::Solid, CryptOptions::default())
    .await
    .unwrap();
    let pooled_iri = "https://pod.example/.lws/subscriptions/private";
    let pooled_body = b"private subscription bytes";
    store
        .write(pooled_iri, Bytes::from_static(pooled_body), "text/plain")
        .await
        .unwrap();
    let pooled_meta = store.meta(pooled_iri).await.unwrap().unwrap();
    let blobs = store.blob.inner.clone();
    let index = store.sparq.clone();
    let http = Http::new(store).await;
    let bodies = [
        ("/alice", BODY),
        ("/bob", b"a different pod's resource\0bytes".as_slice()),
    ];
    for (path, body) in bodies {
        let response = http
            .call(
                "PUT",
                path,
                "application/octet-stream",
                std::str::from_utf8(body).unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let etag = response.headers()["etag"].clone();
        let meta = index
            .get_meta(&format!("{}{path}", common::BASE_URL))
            .await
            .unwrap();
        let plain = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let expected = plain
            .write(
                &format!("{}{path}", common::BASE_URL),
                Bytes::copy_from_slice(body),
                "application/octet-stream",
            )
            .await
            .unwrap();
        assert_eq!(meta.etag, expected.etag);
        assert_eq!(etag.to_str().unwrap(), expected.etag);
        let response = http.call("GET", path, "application/octet-stream", "").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["etag"], etag);
        assert_eq!(to_bytes(response.into_body(), 100000).await.unwrap(), body);
    }
    let mut snapshots = Vec::new();
    for path in ["/", "/.acl", "/alice", "/bob"] {
        let response = http.call("GET", path, "text/turtle", "").await;
        assert_eq!(response.status(), StatusCode::OK);
        let etag = response.headers()["etag"].clone();
        let body = to_bytes(response.into_body(), 100000).await.unwrap();
        snapshots.push((path, etag, body));
    }
    let listing = blobs.list().unwrap();
    assert!(listing.foreign.is_empty());
    assert_eq!(listing.blobs.len(), snapshots.len() + 1);
    for entry in &listing.blobs {
        let (random, path) = if let Some(key) = entry.key.strip_prefix("$pool$e1-") {
            (key, blob_dir.join("pool"))
        } else {
            (
                entry.key.strip_prefix("e1-").unwrap(),
                blob_dir.join("objects"),
            )
        };
        let path = path.join(&random[..2]);
        let path = if entry.key.starts_with("$pool$") {
            path
        } else {
            path.join(&random[2..4])
        };
        let raw = std::fs::read(path.join(&entry.key)).unwrap();
        assert!(raw.starts_with(b"SQLB"), "{} must be encrypted", entry.key);
        for plaintext in [
            BODY,
            pooled_body.as_slice(),
            common::WEBID.as_bytes(),
            bodies[1].1,
        ] {
            assert!(!raw
                .windows(plaintext.len())
                .any(|window| window == plaintext));
        }
    }
    // Drop the router, all index/blob handles, provider and DEK caches before reopening.
    drop(http);
    drop(index);
    drop(blobs);
    let store = CompositeStore::with_body_cache(
        EmbeddedSparqClient::open(&index_dir).unwrap(),
        FsBlobStore::open(&blob_dir).unwrap(),
        BodyCache::disabled(),
    )
    .encrypt_blobs(provider(0x41), CryptScheme::Solid, CryptOptions::default())
    .await
    .unwrap();
    let pooled = store.read(pooled_iri).await.unwrap();
    assert_eq!(pooled.body, pooled_body[..]);
    assert_eq!(pooled.meta, pooled_meta);
    let http = Http::from_store(store);
    for (path, etag, body) in snapshots {
        let response = http.call("GET", path, "text/turtle", "").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["etag"], etag);
        assert_eq!(to_bytes(response.into_body(), 100000).await.unwrap(), body);
    }
    drop(http);
}

#[tokio::test]
async fn http_reads_recheck_acl_blob_pointers_after_warming_the_parse_cache() {
    use axum::{body::to_bytes, http::StatusCode};

    let f = Fixture::new();
    let store = f.encrypted().await;
    store
        .write(BOB, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    let rules = format!(
        "@prefix acl: <http://www.w3.org/ns/auth/acl#>. \
         <urn:rule> a acl:Authorization; acl:agent <{}>; \
         acl:accessTo <{ALICE}>, <{BOB}>; acl:mode acl:Read.",
        common::WEBID
    );
    let alice = store
        .write(
            &format!("{ALICE}.acl"),
            Bytes::from(rules.clone()),
            "text/turtle",
        )
        .await
        .unwrap();
    let bob = store
        .write(&format!("{BOB}.acl"), rules.into(), "text/turtle")
        .await
        .unwrap();
    assert_eq!(alice.etag, bob.etag);
    let http = Http::new(store).await;
    for _ in 0..2 {
        let response = http.call("GET", "/bob/file", "text/plain", "").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(to_bytes(response.into_body(), 10000).await.unwrap(), BODY);
    }
    f.index
        .replace_meta(&format!("{BOB}.acl"), alice)
        .await
        .unwrap();
    for method in ["GET", "HEAD"] {
        let response = http.call(method, "/bob/file", "text/plain", "").await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), 10000).await.unwrap();
        assert!(!body.windows(BODY.len()).any(|window| window == BODY));
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
            .value
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
async fn retained_bodies_keep_their_original_pod_after_encryption_and_cloning() {
    let f = Fixture::new();
    let mut plain = f
        .plain()
        .with_history(crate::store::history::HistoryConfig {
            versions: true,
            trash: true,
            ..Default::default()
        })
        .unwrap();
    plain.body_cache = Arc::new(BodyCache::new(65536));
    let lifecycle = plain.lifecycle();
    let store = plain
        .encrypt_blobs(f.provider.clone(), CryptScheme::Solid, f.options())
        .await
        .unwrap();
    let cloned = store.clone();
    assert!(Arc::ptr_eq(&lifecycle, &cloned.lifecycle()));
    assert!(cloned.history_config().versions);
    assert!(cloned.history_config().trash);
    store
        .write(ALICE, Bytes::from_static(BODY), "text/plain")
        .await
        .unwrap();
    assert_eq!(store.read(ALICE).await.unwrap().body, BODY);
    cloned
        .write(ALICE, Bytes::from_static(b"second"), "text/plain")
        .await
        .unwrap();
    let version = store.versions(Some(ALICE)).await.unwrap().pop().unwrap();
    assert!(envelope_key(&version.meta.blob_key));
    assert_eq!(
        cloned.read_at(&version.member, &version.meta).await.unwrap(),
        BODY
    );
    assert!(cloned.read_at(BOB, &version.meta).await.is_err());
    assert_eq!(
        cloned.read_at(&version.member, &version.meta).await.unwrap(),
        BODY
    );
    assert_eq!(cloned.read(ALICE).await.unwrap().body, "second");
    let member = store
        .trash_resource(
            ALICE,
            None,
            crate::store::history::TrashFields {
                original: ALICE.into(),
                trashed_at: crate::clock::now(),
                deleted_by: None,
                deleter_could_read: true,
            },
        )
        .await
        .unwrap();
    let trash = store.trash_entry(&member).await.unwrap().unwrap();
    assert!(envelope_key(&trash.meta.blob_key));
    assert_eq!(
        cloned.read_at(&member, &trash.meta).await.unwrap(),
        "second"
    );
    assert!(cloned.read_at(BOB, &trash.meta).await.is_err());
    assert_eq!(
        cloned.read_at(&member, &trash.meta).await.unwrap(),
        "second"
    );
}

#[tokio::test]
async fn assembly_rechecks_legacy_bodies_with_the_new_decoder() {
    let f = Fixture::new();
    let mut plain = f.plain();
    plain.body_cache = Arc::new(BodyCache::with_max_entry_bytes(64000, 32000));
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
