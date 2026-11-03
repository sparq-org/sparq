use super::*;
use std::sync::Arc;

const DOC: &str = "https://pod.example/doc";
const INSERT: &[u8] = b"<https://e/s> <https://e/p> \"v\" .\n";

#[derive(Default)]
struct PausingBlobs {
    inner: InMemoryBlobStore,
    pause: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl BlobStore for PausingBlobs {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        self.inner.get(key).await
    }
    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        if self.pause.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.entered.notify_one();
            self.resume.notified().await;
        }
        self.inner.put(key, body).await
    }
    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        self.inner.exists(key).await
    }
    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        self.inner.delete(key).await
    }
    async fn list(&self) -> Result<Vec<blob::BlobEntry>, BlobError> {
        self.inner.list().await
    }
    async fn delete_if_unchanged(&self, key: &str, generation: u64) -> Result<bool, BlobError> {
        self.inner.delete_if_unchanged(key, generation).await
    }
}

async fn publication_snapshot<S: SparqClient + 'static>(index: S) {
    let store = CompositeStore::new(index, PausingBlobs::default());
    store
        .write(DOC, Bytes::from_static(b"old\n"), "text/turtle")
        .await
        .unwrap();
    store
        .blob
        .pause
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let appending = store.clone();
    let task =
        tokio::spawn(async move { appending.append(DOC, Bytes::from_static(b"first\n")).await });
    store.blob.entered.notified().await;
    let reserved = store.read(DOC).await.unwrap();
    assert_eq!(reserved.body, "old\n");
    store
        .append(DOC, Bytes::from_static(b"second\n"))
        .await
        .unwrap();
    let second = store.read(DOC).await.unwrap();
    assert_eq!(second.body, "old\nsecond\n");
    store.blob.resume.notify_one();
    task.await.unwrap().unwrap();
    let final_read = store.read(DOC).await.unwrap();
    assert_ne!(
        final_read.meta.etag, second.meta.etag,
        "publication refreshes validators"
    );
    assert_eq!(
        store.read_at(DOC, &reserved.meta).await.unwrap(),
        reserved.body
    );
    assert_eq!(store.read_at(DOC, &second.meta).await.unwrap(), second.body);
    assert_eq!(final_read.body, "old\nsecond\nfirst\n");
}

#[tokio::test]
async fn append_publication_preserves_snapshots_and_validators() {
    publication_snapshot(InMemorySparqClient::new()).await;
    #[cfg(feature = "embedded-sparq")]
    publication_snapshot(EmbeddedSparqClient::in_memory().unwrap()).await;
}

async fn publication_replacement<S: SparqClient + 'static>(index: S) {
    let store = CompositeStore::new(index, PausingBlobs::default());
    let old = store
        .write(DOC, Bytes::from_static(b"old"), "text/turtle")
        .await
        .unwrap();
    store
        .blob
        .pause
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let appending = store.clone();
    let task = tokio::spawn(async move { appending.append(DOC, Bytes::from_static(INSERT)).await });
    store.blob.entered.notified().await;
    store
        .write(DOC, Bytes::from_static(b"new"), "text/turtle")
        .await
        .unwrap();
    let replacement = store.meta(DOC).await.unwrap().unwrap();
    store.blob.resume.notify_one();
    assert!(matches!(
        task.await.unwrap(),
        Err(ServerError::ResourceChanged)
    ));
    let current = store.read(DOC).await.unwrap();
    assert_eq!(current.meta, replacement);
    assert_eq!(current.body, "new");
    assert!(!store
        .sparq
        .exists(&append_dir(&old.blob_key))
        .await
        .unwrap());
    assert_eq!(store.blob.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn an_insert_racing_a_replacement_does_not_publish_into_the_retired_key() {
    publication_replacement(InMemorySparqClient::new()).await;
    #[cfg(feature = "embedded-sparq")]
    publication_replacement(EmbeddedSparqClient::in_memory().unwrap()).await;
}

async fn concurrent_inserts<S: SparqClient + 'static>(index: S) {
    let store = CompositeStore::new(index, InMemoryBlobStore::new());
    store
        .write(DOC, Bytes::new(), "text/turtle; charset=utf-8")
        .await
        .unwrap();
    let start = Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = Vec::new();
    for n in 0..16 {
        let store = store.clone();
        let start = start.clone();
        tasks.push(tokio::spawn(async move {
            start.wait().await;
            store
                .append(DOC, Bytes::from(format!("<urn:s{n}> <urn:p> \"{n}\" .\n")))
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let read = store.read(DOC).await.unwrap();
    assert_eq!(read.meta.append_seq, 16);
    for name in store
        .sparq
        .list_children(&append_dir(&read.meta.blob_key))
        .await
        .unwrap()
    {
        let suffix = name.rsplit('-').next().unwrap();
        assert_eq!(suffix.len(), 32, "each record has a fresh random suffix");
        assert!(suffix.bytes().all(|b| b.is_ascii_hexdigit()));
    }
    let text = std::str::from_utf8(&read.body).unwrap();
    let mut lines: Vec<_> = text.lines().map(str::to_owned).collect();
    let mut expected: Vec<_> = (0..16)
        .map(|n| format!("<urn:s{n}> <urn:p> \"{n}\" ."))
        .collect();
    lines.sort();
    expected.sort();
    assert_eq!(
        lines, expected,
        "every concurrent insert is represented exactly once"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_stored_inserts_are_all_visible_once() {
    concurrent_inserts(InMemorySparqClient::new()).await;
    #[cfg(feature = "embedded-sparq")]
    concurrent_inserts(EmbeddedSparqClient::in_memory().unwrap()).await;
}

#[tokio::test]
async fn a_reclaimed_append_snapshot_never_returns_only_the_cached_base() {
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    store
        .write(DOC, Bytes::from_static(b"old"), "text/turtle")
        .await
        .unwrap();
    store.append(DOC, Bytes::from_static(INSERT)).await.unwrap();
    let snapshot = store.read(DOC).await.unwrap();
    assert_eq!(snapshot.body, [b"old\n".as_slice(), INSERT].concat());
    store
        .write(DOC, Bytes::from_static(b"new"), "text/turtle")
        .await
        .unwrap();
    match store.read_at(DOC, &snapshot.meta).await {
        Ok(bytes) => assert_eq!(
            bytes, snapshot.body,
            "a cached snapshot must include all inserts"
        ),
        Err(ServerError::ResourceChanged) => {}
        result => panic!("expected the full snapshot or a read-plan retry: {result:?}"),
    }
}

async fn put_back_fork<S: SparqClient>(index: S) {
    let store = CompositeStore::new(index, InMemoryBlobStore::new());
    for prior in [false, true] {
        store
            .write(DOC, Bytes::from_static(b"old\n"), "text/turtle")
            .await
            .unwrap();
        let expected = if prior {
            store
                .append(DOC, Bytes::from_static(b"kept\n"))
                .await
                .unwrap();
            "old\nkept\n"
        } else {
            "old\n"
        };
        let held = store.hold(DOC, "undo").await.unwrap().unwrap();
        store
            .append(DOC, Bytes::from_static(b"undone\n"))
            .await
            .unwrap();
        store.reinstate(DOC, None, &held).await.unwrap();
        store.release(&held, "undo").await.unwrap();
        assert_eq!(store.read(DOC).await.unwrap().body, expected);
        store
            .append(DOC, Bytes::from_static(b"later\n"))
            .await
            .unwrap();
        assert_eq!(
            store.read(DOC).await.unwrap().body,
            format!("{expected}later\n")
        );
    }
}

#[tokio::test]
async fn a_later_insert_does_not_resurrect_an_undone_insert() {
    put_back_fork(InMemorySparqClient::new()).await;
    #[cfg(feature = "embedded-sparq")]
    put_back_fork(EmbeddedSparqClient::in_memory().unwrap()).await;
}

async fn replace_hides<S: SparqClient>(index: S) {
    let store = CompositeStore::new(index, InMemoryBlobStore::new());
    store
        .write(DOC, Bytes::from_static(b"old"), "text/turtle")
        .await
        .unwrap();
    store.append(DOC, Bytes::from_static(INSERT)).await.unwrap();
    assert_eq!(
        store.read(DOC).await.unwrap().body,
        [b"old\n".as_slice(), INSERT].concat()
    );
    let old = store.meta(DOC).await.unwrap().unwrap();
    assert!(old.append_seq > 0);
    store
        .write(DOC, Bytes::from_static(b"new"), "text/turtle")
        .await
        .unwrap();
    let current = store.read(DOC).await.unwrap();
    assert_eq!(current.body, "new");
    assert_eq!(current.meta.append_seq, 0);
    assert_ne!(current.meta.blob_key, old.blob_key);
}

#[tokio::test]
async fn a_replace_hides_earlier_inserts() {
    replace_hides(InMemorySparqClient::new()).await;
    #[cfg(feature = "embedded-sparq")]
    replace_hides(EmbeddedSparqClient::in_memory().unwrap()).await;
}

async fn put_back<S: SparqClient>(index: S) {
    let store = CompositeStore::new(index, InMemoryBlobStore::new());
    store
        .write(DOC, Bytes::from_static(b"old"), "text/turtle")
        .await
        .unwrap();
    store.append(DOC, Bytes::from_static(INSERT)).await.unwrap();
    let held = store.hold(DOC, "change").await.unwrap().unwrap();
    let body = store.read_at(DOC, &held).await.unwrap();
    assert_eq!(body, [b"old\n".as_slice(), INSERT].concat());
    store
        .write(DOC, Bytes::from_static(b"new"), "text/turtle")
        .await
        .unwrap();
    store.reinstate(DOC, None, &held).await.unwrap();
    store.release(&held, "change").await.unwrap();
    let restored = store.read(DOC).await.unwrap();
    assert_eq!(restored.body, body);
    assert_eq!(restored.meta, held);
    // Copied compensation writes the merged bytes to a new key: its sequence
    // resets, so later appends cannot duplicate anything in that copy.
    let copied = store.restore(DOC, None, body.clone(), &held).await.unwrap();
    assert_eq!(copied.append_seq, 0);
    store.append(DOC, Bytes::from_static(INSERT)).await.unwrap();
    assert_eq!(
        store.read(DOC).await.unwrap().body,
        [body.as_ref(), INSERT].concat()
    );
}

#[tokio::test]
async fn a_put_back_restores_the_inserts_it_hid() {
    put_back(InMemorySparqClient::new()).await;
    #[cfg(feature = "embedded-sparq")]
    put_back(EmbeddedSparqClient::in_memory().unwrap()).await;
}

async fn reclaim<S: SparqClient>(index: S) {
    let store = CompositeStore::new(index, InMemoryBlobStore::new());
    let before = store.blob.list().await.unwrap().len();
    let meta = store
        .write(DOC, Bytes::from_static(b"old"), "text/turtle")
        .await
        .unwrap();
    for _ in 0..3 {
        store.append(DOC, Bytes::from_static(INSERT)).await.unwrap();
    }
    let dir = append_dir(&meta.blob_key);
    let records = store.sparq.list_children(&dir).await.unwrap();
    assert_eq!(records.len(), 3);
    let referenced = store.sparq.referenced_blob_keys().await.unwrap();
    for record in &records {
        let meta = store.sparq.get_meta(record).await.unwrap();
        assert!(
            referenced.contains(&meta.blob_key),
            "the orphan sweep must retain each insert"
        );
    }
    assert_eq!(store.blob.list().await.unwrap().len(), before + 4);
    store.delete(DOC, None).await.unwrap();
    store.finish_reclaims().await.unwrap();
    assert!(store.sparq.list_children(&dir).await.unwrap().is_empty());
    assert!(!store.sparq.exists(&dir).await.unwrap());
    for record in records {
        assert!(!store.sparq.exists(&record).await.unwrap());
    }
    assert_eq!(store.blob.list().await.unwrap().len(), before);
}

#[tokio::test]
async fn reclaiming_a_document_removes_its_inserts() {
    reclaim(InMemorySparqClient::new()).await;
    #[cfg(feature = "embedded-sparq")]
    reclaim(EmbeddedSparqClient::in_memory().unwrap()).await;
}

async fn bump_atomic<S: SparqClient + 'static>(index: S) {
    let index = Arc::new(index);
    let meta = ResourceMeta {
        content_type: " \tTEXT/TURTLE \t; charset=utf-8".into(),
        blob_key: "key".into(),
        etag: "\"old\"".into(),
        last_modified: None,
        append_seq: 0,
    };
    index.put_meta(DOC, meta).await.unwrap();
    let mut tasks = Vec::new();
    let start = Arc::new(tokio::sync::Barrier::new(32));
    for _ in 0..32 {
        let index = index.clone();
        let start = start.clone();
        tasks.push(tokio::spawn(async move {
            start.wait().await;
            index.bump_append(DOC).await.unwrap()
        }));
    }
    let mut sequences = Vec::new();
    for task in tasks {
        let (key, seq) = task.await.unwrap();
        assert_eq!(key, "key");
        sequences.push(seq);
    }
    sequences.sort();
    assert_eq!(sequences, (1..=32).collect::<Vec<_>>());
    let meta = index.get_meta(DOC).await.unwrap();
    assert_eq!(meta.append_seq, 32);
    assert_ne!(meta.etag, "\"old\"");
    assert!(meta.etag.starts_with('"') && !meta.etag.starts_with("W/"));
    assert!(meta.last_modified.is_some());
    assert_eq!(index.read_plan(DOC, &[]).await.unwrap().target, Some(meta));
    for ct in ["application/ld+json", "image/png"] {
        let mut denied = index.get_meta(DOC).await.unwrap();
        denied.content_type = ct.into();
        index.put_meta(DOC, denied.clone()).await.unwrap();
        assert!(matches!(
            index.bump_append(DOC).await,
            Err(SparqError::Unsupported)
        ));
        assert_eq!(index.get_meta(DOC).await.unwrap(), denied);
    }
    assert!(matches!(
        index.bump_append("https://pod.example/missing").await,
        Err(SparqError::NotFound)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bump_append_is_atomic() {
    bump_atomic(InMemorySparqClient::new()).await;
    #[cfg(feature = "embedded-sparq")]
    bump_atomic(EmbeddedSparqClient::in_memory().unwrap()).await;
}

#[cfg(all(feature = "http-sparq", feature = "embedded-sparq"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bump_append_is_atomic_over_http() {
    use axum::{extract::State, http::HeaderMap, response::IntoResponse, routing::post, Router};
    use std::sync::Mutex;
    async fn endpoint(
        State(graph): State<Arc<Mutex<sparq_core::Graph>>>,
        headers: HeaderMap,
        body: String,
    ) -> axum::response::Response {
        let mut graph = graph.lock().unwrap();
        if headers["content-type"] == "application/sparql-update" {
            *graph = sparq_engine::update(&graph, &body).expect("valid index update");
            axum::http::StatusCode::NO_CONTENT.into_response()
        } else {
            (
                [("content-type", "application/sparql-results+json")],
                sparq_engine::query_json(&graph, &body).expect("valid index query"),
            )
                .into_response()
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/sparql", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/sparql", post(endpoint))
        .with_state(Arc::new(Mutex::new(sparq_core::Graph::new())));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    bump_atomic(HttpSparqClient::new(url.clone())).await;
    publication_snapshot(HttpSparqClient::new(url.clone())).await;
    publication_replacement(HttpSparqClient::new(url.clone())).await;
    concurrent_inserts(HttpSparqClient::new(url.clone())).await;
    put_back_fork(HttpSparqClient::new(url.clone())).await;
    reclaim(HttpSparqClient::new(url)).await;
    server.abort();
}

#[tokio::test]
async fn a_read_snapshot_merges_only_its_own_sequence() {
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    store
        .write(DOC, Bytes::from_static(b"old\n"), "text/turtle")
        .await
        .unwrap();
    let mut expected = b"old\n".to_vec();
    for n in 1..=12 {
        let insert = format!("<urn:s{n}> <urn:p> \"{n}\" .\n");
        expected.extend_from_slice(insert.as_bytes());
        store.append(DOC, Bytes::from(insert)).await.unwrap();
    }
    let snapshot = store.meta(DOC).await.unwrap().unwrap();
    store
        .append(DOC, Bytes::from_static(b"last\n"))
        .await
        .unwrap();
    assert_eq!(store.read_at(DOC, &snapshot).await.unwrap(), expected);
    assert_eq!(
        store.read(DOC).await.unwrap().body,
        [expected, b"last\n".to_vec()].concat()
    );
}

#[tokio::test]
async fn unfinished_reclaims_remove_stored_inserts_on_start() {
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    let meta = store
        .write(DOC, Bytes::from_static(b"old"), "text/turtle")
        .await
        .unwrap();
    store.append(DOC, Bytes::from_static(INSERT)).await.unwrap();
    store.sparq.delete_resource(DOC, None).await.unwrap();
    assert!(store.sparq.begin_reclaim(&meta.blob_key).await.unwrap());
    // Simulate the process stopping after removing the document bytes, before
    // it removed the append directory or completed the reclaim marker.
    store.blob.delete(&meta.blob_key).await.unwrap();
    assert_eq!(store.blob.list().await.unwrap().len(), 1);
    store.finish_reclaims().await.unwrap();
    assert!(store.blob.list().await.unwrap().is_empty());
    assert!(store
        .sparq
        .list_children(&append_dir(&meta.blob_key))
        .await
        .unwrap()
        .is_empty());
    assert!(store.sparq.reclaiming().await.unwrap().is_empty());
}

#[tokio::test]
async fn append_admission_counts_its_records_and_reclaims_failed_bytes() {
    let store = CompositeStore::new(
        InMemorySparqClient::with_limits(InMemoryStoreLimits::new(1024, 1)),
        InMemoryBlobStore::new(),
    );
    store
        .write(DOC, Bytes::from_static(b"old"), "text/turtle")
        .await
        .unwrap();
    let before = store.usage().unwrap();
    let meta = store.meta(DOC).await.unwrap();
    assert!(matches!(
        store.append(DOC, Bytes::from_static(INSERT)).await,
        Err(ServerError::InsufficientStorage)
    ));
    assert_eq!(store.usage().unwrap(), before);
    assert_eq!(store.meta(DOC).await.unwrap(), meta);
    assert_eq!(store.read(DOC).await.unwrap().body, "old");
}

#[tokio::test]
async fn each_insert_uses_one_content_slot_with_or_without_an_append_directory() {
    for prior in [false, true] {
        let store = CompositeStore::in_memory_with_limits(InMemoryStoreLimits::new(1024, 3));
        store
            .write(DOC, Bytes::from_static(b"old"), "text/turtle")
            .await
            .unwrap();
        if prior {
            store.append(DOC, Bytes::from_static(INSERT)).await.unwrap();
        } else {
            store
                .write("https://pod.example/other", Bytes::new(), "text/turtle")
                .await
                .unwrap();
        }
        store
            .append(DOC, Bytes::from_static(INSERT))
            .await
            .expect("one spare content slot admits one insert");
        assert!(matches!(
            store.append(DOC, Bytes::from_static(INSERT)).await,
            Err(ServerError::InsufficientStorage)
        ));
    }
}

#[tokio::test]
async fn orphan_reconciliation_removes_a_retired_documents_inserts() {
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    let meta = store
        .write(DOC, Bytes::from_static(b"old"), "text/turtle")
        .await
        .unwrap();
    store.append(DOC, Bytes::from_static(INSERT)).await.unwrap();
    // Stop after removing the document's metadata, before starting its reclaim.
    store.sparq.delete_resource(DOC, None).await.unwrap();
    reconcile_orphans(
        &*store.sparq,
        &*store.blob,
        &ReconcileOptions {
            grace: std::time::Duration::ZERO,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    assert!(store.blob.list().await.unwrap().is_empty());
    assert!(!store
        .sparq
        .exists(&append_dir(&meta.blob_key))
        .await
        .unwrap());
    assert!(store.sparq.reclaiming().await.unwrap().is_empty());
}

#[cfg(feature = "embedded-sparq")]
#[tokio::test]
async fn an_append_sequence_survives_embedded_reopen() {
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Scratch(std::env::temp_dir().join(format!(
        "lws-append-{}",
        CompositeStore::<InMemorySparqClient, InMemoryBlobStore>::mint_blob_key(DOC).unwrap()
    )));
    std::fs::create_dir(&dir.0).unwrap();
    let path = dir.0.join("index");
    let index = EmbeddedSparqClient::open_or_create(&path).unwrap();
    index
        .put_meta(
            DOC,
            ResourceMeta {
                content_type: "text/turtle".into(),
                blob_key: "key".into(),
                etag: "\"old\"".into(),
                last_modified: None,
                append_seq: 0,
            },
        )
        .await
        .unwrap();
    index.bump_append(DOC).await.unwrap();
    let before = index.get_meta(DOC).await.unwrap();
    assert_eq!(before.append_seq, 1);
    drop(index);
    let reopened = EmbeddedSparqClient::open(&path).unwrap();
    assert_eq!(reopened.get_meta(DOC).await.unwrap(), before);
    #[cfg(all(feature = "blob-fs", not(target_arch = "wasm32")))]
    {
        let blobs = dir.0.join("blobs");
        let store = CompositeStore::new(reopened, blob_fs::FsBlobStore::open(&blobs).unwrap());
        store
            .write(DOC, Bytes::from_static(b"old"), "text/turtle")
            .await
            .unwrap();
        store.append(DOC, Bytes::from_static(INSERT)).await.unwrap();
        let before = store.read(DOC).await.unwrap();
        drop(store);
        let reopened = CompositeStore::new(
            EmbeddedSparqClient::open(&path).unwrap(),
            blob_fs::FsBlobStore::open(&blobs).unwrap(),
        );
        let after = reopened.read(DOC).await.unwrap();
        assert_eq!(after.meta, before.meta);
        assert_eq!(after.body, before.body);
        assert_eq!(after.body, [b"old\n".as_slice(), INSERT].concat());
    }
}
