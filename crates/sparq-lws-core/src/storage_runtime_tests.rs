use super::*;

#[cfg(feature = "embedded-sparq")]
#[path = "../tests/common/mod.rs"]
mod auth_fixture;

fn config(values: &[(&str, &str)]) -> io::Result<StorageConfig> {
    StorageConfig::read(|key| {
        values
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| (*value).into())
    })
}

#[test]
fn storage_selection_and_invalid_configuration() {
    let default = config(&[]).unwrap();
    assert_eq!(default.backend, "memory");
    assert!(default.check_durability(false).is_ok());
    assert!(default.check_durability(true).is_err());
    assert_eq!(
        config(&[(ENV_DATA_DIR, "data")]).unwrap().backend,
        "embedded"
    );
    for backend in ["memory", "http", "", "unknown"] {
        assert!(config(&[(ENV_DATA_DIR, "data"), (ENV_BACKEND, backend)]).is_err());
    }
    assert!(config(&[(ENV_DATA_DIR, "data"), (ENV_SPARQ_DIR, "old")]).is_err());
    for wait in ["-1", "bad", "0.5", ""] {
        assert!(config(&[(ENV_WAIT, wait)]).is_err());
    }
    assert!(config(&[(ENV_WAIT, "0")]).is_ok());
    assert!(config(&[(ENV_WAIT, "1")]).is_ok());
}

#[cfg(feature = "embedded-sparq")]
use std::fs;
#[cfg(feature = "embedded-sparq")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "embedded-sparq")]
struct Directory(PathBuf);

#[cfg(feature = "embedded-sparq")]
impl Directory {
    fn new() -> Self {
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let dir = Self(std::env::temp_dir().join(format!(
            "lws-datadir-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        )));
        fs::create_dir(&dir.0).unwrap();
        dir
    }

    fn config(&self) -> StorageConfig {
        config(&[(ENV_DATA_DIR, self.0.to_str().unwrap())]).unwrap()
    }
}

#[cfg(feature = "embedded-sparq")]
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(feature = "embedded-sparq")]
#[tokio::test]
async fn boot_without_data_dir_keeps_ephemeral_blobs_and_delegates_their_lifecycle() {
    use bytes::Bytes;
    let cfg = config(&[(ENV_BACKEND, "embedded")]).unwrap();
    let (client, blobs) = cfg.open_embedded_with(false, make_blobs).await.unwrap();
    let key = "b1-0123456789abcdef0123456789abcdef";
    assert!(blobs.list().await.unwrap().is_empty());
    blobs
        .put(key, Bytes::from_static(b"ephemeral"))
        .await
        .unwrap();
    assert_eq!(blobs.get(key).await.unwrap(), b"ephemeral"[..]);
    assert!(blobs.exists(key).await.unwrap());
    let entry = blobs.stat(key).await.unwrap().unwrap();
    assert_eq!(entry.key, key);
    assert_eq!(blobs.list().await.unwrap().len(), 1);
    assert!(!blobs
        .delete_if_unchanged(key, entry.generation.unwrap() + 1)
        .await
        .unwrap());
    assert_eq!(blobs.get(key).await.unwrap(), b"ephemeral"[..]);
    assert!(blobs
        .delete_if_unchanged(key, entry.generation.unwrap())
        .await
        .unwrap());
    assert!(!blobs.exists(key).await.unwrap());
    let key = "b1-1123456789abcdef0123456789abcdef";
    blobs.put(key, Bytes::new()).await.unwrap();
    blobs.delete(key).await.unwrap();
    assert!(blobs.stat(key).await.unwrap().is_none());
    let key = "b1-2123456789abcdef0123456789abcdef";
    blobs
        .put(key, Bytes::from_static(b"discard on drop"))
        .await
        .unwrap();
    drop((client, blobs));
    let (_client, blobs) = cfg.open_embedded_with(false, make_blobs).await.unwrap();
    assert!(blobs.list().await.unwrap().is_empty());
}

#[cfg(all(feature = "embedded-sparq", not(feature = "blob-fs")))]
#[tokio::test]
async fn unavailable_filesystem_adapter_refuses_and_releases_the_index_lock() {
    let dir = Directory::new();
    let failure = dir
        .config()
        .open_embedded_with(true, make_blobs)
        .await
        .err()
        .unwrap();
    assert_eq!(failure.kind(), io::ErrorKind::Unsupported);
    assert!(failure
        .to_string()
        .contains("filesystem blob store is not built in"));
    assert!(failure.to_string().contains("blob-fs"));
    assert!(!dir.0.join("blobs").exists());
    drop(sparq_lws_core::store::EmbeddedSparqClient::open_or_create(&dir.0.join("index")).unwrap());
}

#[cfg(all(feature = "embedded-sparq", feature = "blob-fs"))]
mod embedded {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    use async_trait::async_trait;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use bytes::Bytes;
    use sparq_lws_core::store::{
        BlobEntry, BlobError, BlobStore, CompositeStore, EmbeddedSparqClient, Store,
    };
    use tower::ServiceExt;

    const BASE: &str = "https://pod.example";
    const CHILD: &str = "https://pod.example/item";
    const FORMAT: &str = r#"{"schema":1,"index":{"major":1},"blobs":{"major":1}}"#;

    // Test-only durable bytes. Keys are hex-encoded to keep opaque store keys inside this fixture.
    // No generation or timestamp is advertised, so the reconciler never collects these test files.
    struct DirectoryBlobs(PathBuf);

    impl DirectoryBlobs {
        fn open(path: Option<&Path>) -> io::Result<Self> {
            let path = path.expect("durable blob path");
            assert!(path.parent().unwrap().join("format.json").is_file());
            fs::create_dir_all(path)?;
            Ok(Self(path.into()))
        }

        fn path(&self, key: &str) -> PathBuf {
            self.0.join(
                key.as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
            )
        }
    }

    fn blob_error(error: io::Error) -> BlobError {
        if error.kind() == io::ErrorKind::NotFound {
            BlobError::NotFound
        } else {
            BlobError::Backend(error.to_string())
        }
    }

    #[async_trait]
    impl BlobStore for DirectoryBlobs {
        async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
            fs::read(self.path(key))
                .map(Bytes::from)
                .map_err(blob_error)
        }
        async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
            use std::io::Write;
            let mut file = fs::File::create(self.path(key)).map_err(blob_error)?;
            file.write_all(&body).map_err(blob_error)?;
            file.sync_all().map_err(blob_error)?;
            fs::File::open(&self.0)
                .and_then(|dir| dir.sync_all())
                .map_err(blob_error)
        }
        async fn exists(&self, key: &str) -> Result<bool, BlobError> {
            match fs::metadata(self.path(key)) {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(blob_error(error)),
            }
        }
        async fn delete(&self, key: &str) -> Result<(), BlobError> {
            match fs::remove_file(self.path(key)) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(blob_error(error)),
            }
        }
        async fn list(&self) -> Result<Vec<BlobEntry>, BlobError> {
            fs::read_dir(&self.0)
                .map_err(blob_error)?
                .map(|entry| {
                    let name = entry.map_err(blob_error)?.file_name();
                    let name = name.to_str().unwrap();
                    let bytes = name
                        .as_bytes()
                        .chunks_exact(2)
                        .map(|pair| {
                            u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()
                        })
                        .collect();
                    Ok(BlobEntry {
                        key: String::from_utf8(bytes).unwrap(),
                        last_modified: None,
                        generation: None,
                    })
                })
                .collect()
        }
        async fn delete_if_unchanged(
            &self,
            _key: &str,
            _generation: u64,
        ) -> Result<bool, BlobError> {
            Ok(false)
        }
    }

    type Boot = (EmbeddedSparqClient, NativeBlobs);

    async fn boot(config: &StorageConfig) -> io::Result<Boot> {
        config.open_embedded().await
    }

    fn error<B>(result: io::Result<(EmbeddedSparqClient, B)>) -> io::Error {
        match result {
            Err(error) => error,
            Ok(_) => panic!("boot unexpectedly succeeded"),
        }
    }

    #[tokio::test]
    async fn fresh_data_dir_boots_with_the_filesystem_layout() {
        let dir = Directory::new();
        let held = boot(&dir.config()).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                &fs::read(dir.0.join("format.json")).unwrap()
            )
            .unwrap(),
            serde_json::json!({"schema":1,"index":{"major":1},"blobs":{"major":1}})
        );
        assert!(dir.0.join("index").is_dir());
        for path in ["blobs/objects", "blobs/pool", "blobs/tmp"] {
            assert!(dir.0.join(path).is_dir(), "missing {path}");
        }
        drop(held);
    }

    #[tokio::test]
    async fn filesystem_boot_reclaims_replaced_and_deleted_resource_bytes() {
        use sparq_lws_core::store::BodyCache;

        let dir = Directory::new();
        let (client, blobs) = boot(&dir.config()).await.unwrap();
        let blobs = crate::reconcile_runtime::SharedStore::new(blobs);
        let store = CompositeStore::with_body_cache(client, blobs.clone(), BodyCache::disabled());
        let first = store
            .write(CHILD, Bytes::from_static(b"first"), "text/plain")
            .await
            .unwrap();
        let second = store
            .write(CHILD, Bytes::from_static(b"second"), "text/plain")
            .await
            .unwrap();
        assert_eq!(store.read(CHILD).await.unwrap().body, b"second"[..]);
        assert!(!blobs.exists(&first.blob_key).await.unwrap());
        assert!(matches!(
            blobs.get(&first.blob_key).await,
            Err(BlobError::NotFound)
        ));
        assert!(blobs.exists(&second.blob_key).await.unwrap());
        assert!(matches!(
            blobs
                .put(&second.blob_key, Bytes::from_static(b"overwrite"))
                .await,
            Err(BlobError::Backend(_))
        ));
        assert_eq!(blobs.get(&second.blob_key).await.unwrap(), b"second"[..]);
        let entries = blobs.list().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, second.blob_key);
        let entry = blobs.stat(&second.blob_key).await.unwrap().unwrap();
        assert_eq!(entry.key, second.blob_key);
        assert!(entry.last_modified.is_some());
        assert_eq!(entry.generation, None);
        assert!(blobs
            .delete_if_unchanged(&second.blob_key, 0)
            .await
            .is_err());
        assert_eq!(store.read(CHILD).await.unwrap().body, b"second"[..]);
        store.delete(CHILD, None).await.unwrap();
        assert!(blobs.list().await.unwrap().is_empty());
        assert!(blobs.stat(&second.blob_key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn filesystem_open_failure_releases_the_lock_and_preserves_recoverable_data() {
        let dir = Directory::new();
        let cfg = dir.config();
        let (client, blobs) = boot(&cfg).await.unwrap();
        let store = CompositeStore::new(client, blobs);
        store
            .write(
                CHILD,
                Bytes::from_static(b"keep after failed boot"),
                "text/plain",
            )
            .await
            .unwrap();
        let expected = store.meta(CHILD).await.unwrap();
        drop(store);
        let marker = fs::read(dir.0.join("format.json")).unwrap();
        let journals: Vec<_> = logs(&dir.0.join("index"))
            .into_iter()
            .map(|path| {
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        assert!(journals.iter().any(|(_, bytes)| !bytes.is_empty()));
        // A real filesystem failure, after opening the index, must abort boot.
        let temporary = dir.0.join("blobs/tmp");
        fs::remove_dir(&temporary).unwrap();
        fs::write(&temporary, b"not a directory").unwrap();
        let failure = error(boot(&cfg).await);
        assert_eq!(failure.kind(), io::ErrorKind::NotADirectory);
        assert_eq!(fs::read(&temporary).unwrap(), b"not a directory");
        assert_eq!(fs::read(dir.0.join("format.json")).unwrap(), marker);
        for (path, bytes) in journals {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        drop(EmbeddedSparqClient::open_or_create(&dir.0.join("index")).unwrap());
        fs::remove_file(temporary).unwrap();
        let (client, blobs) = boot(&cfg).await.unwrap();
        let store = CompositeStore::new(client, blobs);
        assert_eq!(store.meta(CHILD).await.unwrap(), expected);
        assert_eq!(
            store.read(CHILD).await.unwrap().body,
            b"keep after failed boot"[..]
        );
    }

    #[tokio::test]
    async fn fresh_boot_holds_the_index_lock_before_constructing_blobs() {
        let dir = Directory::new();
        let cfg = dir.config();
        let (client, blobs) = cfg
            .open_embedded_with(true, |path| {
                let marker: serde_json::Value =
                    serde_json::from_slice(&fs::read(dir.0.join("format.json"))?)?;
                assert_eq!(
                    marker,
                    serde_json::json!({"schema":1,"index":{"major":1},"blobs":{"major":1}})
                );
                let lock = EmbeddedSparqClient::open_or_create(&dir.0.join("index"))
                    .err()
                    .unwrap();
                assert_eq!(lock.kind(), io::ErrorKind::WouldBlock);
                make_blobs(path)
            })
            .await
            .unwrap();
        assert!(dir.0.join("format.json").is_file());
        assert!(dir.0.join("index").is_dir());
        for path in ["blobs/objects", "blobs/pool", "blobs/tmp"] {
            assert!(dir.0.join(path).is_dir(), "missing {path}");
        }
        assert!(dir.0.join("index.lock").is_file());
        assert!(dir.0.join("index/sparq.lock").is_file());
        assert!(!dir.0.join("LOCK").exists());
        drop(blobs);
        drop(client);
        drop(boot(&cfg).await.unwrap());
    }

    #[tokio::test]
    async fn second_boot_fails_fast_without_constructing_or_changing_blobs() {
        let dir = Directory::new();
        let cfg = dir.config();
        let held = boot(&cfg).await.unwrap();
        let temporary = dir.0.join("blobs/tmp/in-flight");
        fs::write(&temporary, b"pending publication").unwrap();
        let failure = error(
            tokio::time::timeout(Duration::from_secs(2), boot(&cfg))
                .await
                .expect("contending boot must fail promptly"),
        );
        assert_eq!(failure.kind(), io::ErrorKind::WouldBlock);
        assert!(failure
            .to_string()
            .contains(&dir.0.join("index.lock").display().to_string()));
        assert_eq!(fs::read(&temporary).unwrap(), b"pending publication");
        let called = AtomicBool::new(false);
        let result = cfg
            .open_embedded_with(true, |path| {
                called.store(true, Ordering::SeqCst);
                make_blobs(path)
            })
            .await;
        let error = error(result);
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(error
            .to_string()
            .contains(&dir.0.join("index.lock").display().to_string()));
        assert!(!called.load(Ordering::SeqCst));
        assert_eq!(fs::read(&temporary).unwrap(), b"pending publication");
        drop(held);
        let reopened = boot(&cfg).await.unwrap();
        assert!(
            !temporary.exists(),
            "the next lock holder must sweep abandoned writes"
        );
        drop(reopened);
    }

    #[tokio::test]
    async fn waiting_boot_succeeds_after_the_first_holder_drops() {
        let dir = Directory::new();
        let held = boot(&dir.config()).await.unwrap();
        let cfg = config(&[(ENV_DATA_DIR, dir.0.to_str().unwrap()), (ENV_WAIT, "2")]).unwrap();
        let mut waiter = tokio::spawn(async move { boot(&cfg).await });
        assert!(tokio::time::timeout(Duration::from_millis(50), &mut waiter)
            .await
            .is_err());
        drop(held);
        let reopened = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        drop(reopened);
    }

    #[tokio::test]
    async fn waiting_boot_expires_with_would_block_and_the_lock_path() {
        let dir = Directory::new();
        let held = boot(&dir.config()).await.unwrap();
        let cfg = config(&[(ENV_DATA_DIR, dir.0.to_str().unwrap()), (ENV_WAIT, "1")]).unwrap();
        let failure = error(boot(&cfg).await);
        assert_eq!(failure.kind(), io::ErrorKind::WouldBlock);
        assert!(failure.to_string().contains("index.lock"));
        drop(held);
    }

    #[tokio::test]
    async fn format_mismatches_and_foreign_planes_refuse_before_opening_either_plane() {
        for marker in [
            r#"{"schema":2,"index":{"major":1},"blobs":{"major":1}}"#,
            r#"{"schema":1,"index":{"major":2},"blobs":{"major":1}}"#,
            r#"{"schema":1,"index":{"major":1},"blobs":{"major":2}}"#,
            "{}",
            "not json",
        ] {
            let dir = Directory::new();
            fs::write(dir.0.join("format.json"), marker).unwrap();
            assert!(error(boot(&dir.config()).await)
                .to_string()
                .contains("format.json"));
            assert!(!dir.0.join("index").exists());
            assert!(!dir.0.join("blobs").exists());
            assert_eq!(
                fs::read_to_string(dir.0.join("format.json")).unwrap(),
                marker
            );
        }
        for plane in ["index", "blobs"] {
            let dir = Directory::new();
            fs::create_dir(dir.0.join(plane)).unwrap();
            let content = dir.0.join(plane).join("foreign");
            fs::write(&content, b"untouched").unwrap();
            assert!(error(boot(&dir.config()).await)
                .to_string()
                .contains("foreign"));
            assert_eq!(fs::read(&content).unwrap(), b"untouched");
            assert!(!dir.0.join("format.json").exists());
        }
    }

    #[tokio::test]
    async fn marker_only_and_empty_planes_resume_first_boot() {
        for empty_planes in [false, true] {
            let dir = Directory::new();
            if empty_planes {
                fs::create_dir(dir.0.join("index")).unwrap();
                fs::create_dir(dir.0.join("blobs")).unwrap();
            } else {
                fs::write(dir.0.join("format.json"), FORMAT).unwrap();
            }
            drop(boot(&dir.config()).await.unwrap());
            assert!(
                dir.0.join("index/dict-meta.bin").exists() || dir.0.join("index/dict.bin").exists()
            );
        }
    }

    #[tokio::test]
    async fn durability_guards_apply_at_the_injected_boot_entry() {
        let dir = Directory::new();
        let cfg = dir.config();
        let constructed = AtomicBool::new(false);
        let failure = cfg
            .open_embedded_with(false, |_| {
                constructed.store(true, Ordering::SeqCst);
                Ok(sparq_lws_core::store::InMemoryBlobStore::new())
            })
            .await
            .err()
            .unwrap();
        assert!(failure.to_string().contains("durable blob store"));
        assert!(!constructed.load(Ordering::SeqCst));
        assert!(!dir.0.join("format.json").exists());
        fs::write(dir.0.join("format.json"), FORMAT).unwrap();
        for override_value in [None, Some("true"), Some("0"), Some("1")] {
            let mut values = vec![(ENV_BACKEND, "embedded")];
            if let Some(value) = override_value {
                values.push((ENV_ALLOW_INCONSISTENT, value));
            }
            let cfg = config(&values).unwrap();
            let result = cfg
                .open_embedded_with(true, |_| DirectoryBlobs::open(Some(&dir.0.join("blobs"))))
                .await;
            if override_value == Some("1") {
                assert!(result.is_ok());
            } else {
                assert!(error(result).to_string().contains("durable SPARQ index"));
            }
        }
    }

    fn logs(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.extend(logs(&path));
            } else if matches!(
                path.file_name().unwrap().to_str(),
                Some("wal.log" | "txn.log")
            ) {
                out.push(path);
            }
        }
        out
    }

    #[tokio::test]
    async fn shutdown_compaction_empties_journals_and_releases_the_actor_lock() {
        let dir = Directory::new();
        let (client, blobs) = boot(&dir.config()).await.unwrap();
        let store = CompositeStore::new(client.clone(), blobs);
        store
            .write(&format!("{BASE}/"), Bytes::new(), "text/turtle")
            .await
            .unwrap();
        let _written = store
            .create_in_container(
                &format!("{BASE}/"),
                CHILD,
                Bytes::from_static(b"durable body"),
                "text/plain",
            )
            .await
            .unwrap();
        let expected = store.meta(CHILD).await.unwrap();
        let before = logs(&dir.0.join("index"));
        assert!(!before.is_empty());
        assert!(before
            .iter()
            .any(|path| fs::metadata(path).unwrap().len() > 0));
        drop(store);
        let mut shutdown = StorageShutdown::default();
        shutdown.retain(&dir.config(), &client);
        drop(client);
        shutdown.run(async { Ok(()) }, None).await.unwrap();
        let after = logs(&dir.0.join("index"));
        assert!(!after.is_empty());
        assert!(after
            .iter()
            .all(|path| fs::metadata(path).unwrap().len() == 0));
        let (client, blobs) = boot(&dir.config()).await.unwrap();
        let store = CompositeStore::new(client, blobs);
        assert_eq!(store.meta(CHILD).await.unwrap(), expected);
        assert_eq!(store.read(CHILD).await.unwrap().body, b"durable body"[..]);
    }

    #[tokio::test]
    async fn shutdown_compaction_failure_propagates_and_keeps_recoverable_data() {
        let dir = Directory::new();
        let (client, blobs) = boot(&dir.config()).await.unwrap();
        let store = CompositeStore::new(client.clone(), blobs);
        store
            .write(CHILD, Bytes::from_static(b"keep me"), "text/plain")
            .await
            .unwrap();
        drop(store);
        let journals: Vec<_> = logs(&dir.0.join("index"))
            .into_iter()
            .map(|path| {
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        assert!(journals.iter().any(|(_, bytes)| !bytes.is_empty()));
        // Compaction swaps the complete index tree at its root, even when the pending
        // writes live in named graphs. Block that swap, independent of journal order.
        let staging = dir.0.join("index.compact-new");
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("foreign"), b"preserve").unwrap();
        let mut shutdown = StorageShutdown::default();
        shutdown.retain(&dir.config(), &client);
        drop(client);
        let failure = shutdown
            .run(async { Ok(()) }, None)
            .await
            .expect_err("foreign staging must refuse shutdown compaction");
        let message = failure.to_string();
        assert!(message.contains("embedded engine compact"), "{message}");
        assert!(message.contains(staging.to_str().unwrap()), "{message}");
        for (path, bytes) in journals {
            assert_eq!(
                fs::read(&path).unwrap(),
                bytes,
                "changed {}",
                path.display()
            );
        }
        assert_eq!(fs::read(staging.join("foreign")).unwrap(), b"preserve");
        fs::remove_dir_all(staging).unwrap();
        let (client, blobs) = boot(&dir.config()).await.unwrap();
        assert_eq!(
            CompositeStore::new(client, blobs)
                .read(CHILD)
                .await
                .unwrap()
                .body,
            b"keep me"[..]
        );
    }

    async fn protocol_router(
        client: EmbeddedSparqClient,
        blobs: NativeBlobs,
        lws: bool,
        seed: bool,
        issuer: &auth_fixture::KeyKit,
    ) -> axum::Router {
        let store = CompositeStore::new(client, blobs);
        if lws {
            let mut cfg = sparq_lws_core::lws::LwsConfig::new(BASE);
            cfg.open = true;
            return sparq_lws_core::lws::router(store, cfg).await.unwrap();
        }
        if seed {
            store
                .write(&format!("{BASE}/"), Bytes::new(), "text/turtle")
                .await
                .unwrap();
            let acl = format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#owner> a acl:Authorization; acl:agent <{}>; acl:accessTo <{BASE}/>;
                acl:default <{BASE}/>; acl:mode acl:Read, acl:Write, acl:Control."#,
                auth_fixture::WEBID
            );
            store
                .write(&format!("{BASE}/.acl"), Bytes::from(acl), "text/turtle")
                .await
                .unwrap();
        }
        let cfg = solid_oidc_verifier::config::VerifierConfig::new(
            vec![auth_fixture::ISSUER.into()],
            BASE,
        );
        let replay =
            solid_oidc_verifier::replay::InMemoryReplayStore::with_window(cfg.replay_ttl());
        let verifier = solid_oidc_verifier::verifier::Verifier::new(
            cfg,
            auth_fixture::jwks_provider(issuer),
            replay,
        )
        .unwrap();
        sparq_lws_core::build_router(sparq_lws_core::AppState::new(
            sparq_lws_core::auth::AuthContext::new(verifier, BASE),
            sparq_lws_core::ldp::handler::LdpState::new(store, BASE),
        ))
    }

    fn request(
        method: &str,
        path: &str,
        body: &'static [u8],
        issuer: &auth_fixture::KeyKit,
        key: &auth_fixture::KeyKit,
    ) -> Request<Body> {
        let token = auth_fixture::mint_access_token(issuer, &key.thumbprint);
        let proof = auth_fixture::mint_dpop_proof(key, method, &format!("{BASE}{path}"), &token);
        Request::builder()
            .method(method)
            .uri(format!("{BASE}{path}"))
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::AUTHORIZATION, format!("DPoP {token}"))
            .header("DPoP", proof)
            .body(Body::from(body))
            .unwrap()
    }

    pub(super) async fn restart_round_trip(lws: bool, object: bool) {
        const BODY: &[u8] = b"persisted bytes\0\xff\n";
        let dir = Directory::new();
        let object_dir = Directory::new();
        let object_url = url::Url::from_directory_path(&object_dir.0).unwrap();
        let cfg = if object {
            config(&[
                (ENV_DATA_DIR, dir.0.to_str().unwrap()),
                (ENV_BLOB_URL, object_url.as_str()),
            ])
            .unwrap()
        } else {
            dir.config()
        };
        let issuer = auth_fixture::KeyKit::generate();
        let key = auth_fixture::KeyKit::generate();
        let (client, blobs) = boot(&cfg).await.unwrap();
        let app = protocol_router(client, blobs, lws, true, &issuer).await;
        let mut put = request("PUT", "/item", BODY, &issuer, &key);
        if lws {
            // LWS creates members with POST; then PUT replaces that member's bytes.
            let mut create = request("POST", "/", b"initial", &issuer, &key);
            create.headers_mut().remove(header::AUTHORIZATION);
            create.headers_mut().remove("DPoP");
            create.headers_mut().insert("slug", "item".parse().unwrap());
            let created = app.clone().oneshot(create).await.unwrap();
            assert_eq!(created.status(), StatusCode::CREATED);
            assert_eq!(created.headers()[header::LOCATION], CHILD);
            put.headers_mut().remove(header::AUTHORIZATION);
            put.headers_mut().remove("DPoP");
            put.headers_mut()
                .insert(header::IF_MATCH, "*".parse().unwrap());
        }
        let response = app.clone().oneshot(put).await.unwrap();
        let status = response.status();
        let put_tag = response.headers().get(header::ETAG).cloned();
        let detail = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        assert!(
            status.is_success(),
            "PUT failed: {status}: {}",
            String::from_utf8_lossy(&detail)
        );
        let read = |method: &str, path: &str| {
            let mut req = request(method, path, b"", &issuer, &key);
            if lws {
                req.headers_mut().remove(header::AUTHORIZATION);
                req.headers_mut().remove("DPoP");
                if path == "/" {
                    req.headers_mut()
                        .insert(header::ACCEPT, "application/lws+json".parse().unwrap());
                }
            } else if path == "/" {
                req.headers_mut()
                    .insert(header::ACCEPT, "text/turtle".parse().unwrap());
            }
            req
        };
        let response = app.clone().oneshot(read("GET", "/item")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let etag = response.headers()[header::ETAG].clone();
        if let Some(put_tag) = put_tag {
            assert_eq!(etag, put_tag);
        }
        assert_eq!(to_bytes(response.into_body(), 1 << 20).await.unwrap(), BODY);
        let listing = app.clone().oneshot(read("GET", "/")).await.unwrap();
        assert_eq!(listing.status(), StatusCode::OK);
        let listing = to_bytes(listing.into_body(), 1 << 20).await.unwrap();
        assert!(String::from_utf8_lossy(&listing).contains("item"));
        assert!(logs(&dir.0.join("index"))
            .iter()
            .any(|path| fs::metadata(path).unwrap().len() > 0));
        let marker: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.0.join("format.json")).unwrap()).unwrap();
        if object {
            assert_eq!(marker["blobs"]["backend"], "object");
            assert_eq!(marker["blobs"]["url"], object_url.as_str());
            assert!(!dir.0.join("blobs").exists());
            assert!(fs::read_dir(&object_dir.0).unwrap().next().is_some());
        } else {
            assert_eq!(
                marker,
                serde_json::from_str::<serde_json::Value>(FORMAT).unwrap()
            );
            assert!(dir.0.join("blobs").is_dir());
            assert!(fs::read_dir(&object_dir.0).unwrap().next().is_none());
        }
        // Drop without compaction: writes must recover from the actual journals.
        drop(app);
        // The LWS maintenance task holds only a transient lease; let its close notification run.
        tokio::task::yield_now().await;
        let (client, blobs) = boot(&cfg).await.unwrap();
        let app = protocol_router(client, blobs, lws, false, &issuer).await;
        let response = app.clone().oneshot(read("GET", "/item")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::ETAG], etag);
        assert_eq!(to_bytes(response.into_body(), 1 << 20).await.unwrap(), BODY);
        let response = app.clone().oneshot(read("GET", "/")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), 1 << 20).await.unwrap(),
            listing
        );
    }

    #[tokio::test]
    async fn solid_put_survives_restart_with_identical_bytes_etag_and_listing() {
        restart_round_trip(false, false).await;
    }

    #[tokio::test]
    async fn lws_put_survives_restart_with_identical_bytes_etag_and_listing() {
        restart_round_trip(true, false).await;
    }

    #[tokio::test]
    async fn listener_drains_an_inflight_write_before_compacting_and_releasing_the_lock() {
        use std::sync::Arc;
        use tokio::sync::{oneshot, Notify};
        let dir = Directory::new();
        let (client, blobs) = boot(&dir.config()).await.unwrap();
        let mut shutdown = StorageShutdown::default();
        shutdown.retain(&dir.config(), &client);
        let store = Arc::new(CompositeStore::new(client, blobs));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let handler_started = started.clone();
        let handler_release = release.clone();
        let app = axum::Router::new().route(
            "/write",
            axum::routing::put(move || {
                let store = store.clone();
                let started = handler_started.clone();
                let release = handler_release.clone();
                async move {
                    started.notify_one();
                    release.notified().await;
                    store
                        .write(CHILD, Bytes::from_static(b"drained request"), "text/plain")
                        .await
                        .unwrap();
                    StatusCode::NO_CONTENT
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel();
        let serving = async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await?;
            Ok(())
        };
        let traffic = async {
            let request = tokio::spawn(async move {
                reqwest::Client::new()
                    .put(format!("http://{address}/write"))
                    .send()
                    .await
                    .unwrap()
            });
            started.notified().await;
            stop.send(()).unwrap();
            tokio::task::yield_now().await;
            assert!(
                !request.is_finished(),
                "shutdown must wait for the admitted write"
            );
            release.notify_one();
            assert_eq!(request.await.unwrap().status(), StatusCode::NO_CONTENT);
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(shutdown.run(serving, None), traffic)
        })
        .await
        .unwrap();
        result.unwrap();
        assert!(logs(&dir.0.join("index"))
            .iter()
            .all(|path| fs::metadata(path).unwrap().len() == 0));
        let (client, blobs) = boot(&dir.config()).await.unwrap();
        assert_eq!(
            CompositeStore::new(client, blobs)
                .read(CHILD)
                .await
                .unwrap()
                .body,
            b"drained request"[..]
        );
    }
}

#[cfg(not(feature = "blob-object"))]
#[test]
fn object_url_without_feature_is_a_startup_error() {
    let failure = config(&[
        (ENV_DATA_DIR, "unused-data"),
        (ENV_BLOB_URL, "file:///unused-blobs"),
    ])
    .unwrap_err();
    assert_eq!(failure.kind(), io::ErrorKind::InvalidInput);
    assert!(failure.to_string().contains(ENV_BLOB_URL));
    assert!(failure.to_string().contains("blob-object"));
    assert!(!failure.to_string().contains("unused-blobs"));
}

#[test]
fn object_options_without_url_are_refused() {
    let failure = config(&[(ENV_BLOB_OPTIONS, "{}")]).unwrap_err();
    assert!(failure.to_string().contains(ENV_BLOB_URL));
}

#[cfg(all(feature = "embedded-sparq", feature = "blob-object"))]
#[test]
fn object_configuration_is_explicit_and_redacted() {
    let failure = config(&[(ENV_BLOB_URL, "file:///unused")]).unwrap_err();
    assert!(failure.to_string().contains(ENV_DATA_DIR));
    for url in [
        "",
        "not-a-url",
        "https://bucket/path",
        "s3://user:secret@bucket/path",
        "s3://bucket/path?secret=value",
        "gs://bucket/path#secret",
    ] {
        let failure = config(&[(ENV_DATA_DIR, "unused"), (ENV_BLOB_URL, url)]).unwrap_err();
        assert!(failure.to_string().contains(ENV_BLOB_URL));
        assert!(!failure.to_string().contains("secret"));
    }
    for options in ["secret", "[]", r#"{"token":123}"#] {
        let failure = config(&[
            (ENV_DATA_DIR, "unused"),
            (ENV_BLOB_URL, "file:///unused"),
            (ENV_BLOB_OPTIONS, options),
        ])
        .unwrap_err();
        assert!(failure.to_string().contains(ENV_BLOB_OPTIONS));
        assert!(!failure.to_string().contains("secret"));
    }
    let cfg = config(&[
        (ENV_DATA_DIR, "unused"),
        (ENV_BLOB_URL, "s3://private-bucket/path"),
        (ENV_BLOB_OPTIONS, r#"{"aws_secret_access_key":"secret"}"#),
    ])
    .unwrap();
    let diagnostic = format!("{cfg:?}");
    assert!(!diagnostic.contains("secret"));
    assert!(!diagnostic.contains("private-bucket"));
}

#[cfg(all(
    feature = "embedded-sparq",
    feature = "blob-object",
    feature = "blob-fs"
))]
mod object_boot {
    use super::*;
    use futures_util::TryStreamExt;

    fn object_config(data: &Directory, objects: &Directory) -> StorageConfig {
        let url = url::Url::from_directory_path(&objects.0).unwrap();
        config(&[
            (ENV_DATA_DIR, data.0.to_str().unwrap()),
            (ENV_BLOB_URL, url.as_str()),
        ])
        .unwrap()
    }

    #[tokio::test]
    async fn file_object_solid_put_get_survives_restart() {
        embedded::restart_round_trip(false, true).await;
    }

    #[tokio::test]
    async fn file_object_lws_put_get_survives_restart() {
        embedded::restart_round_trip(true, true).await;
    }

    #[tokio::test]
    async fn reopening_with_different_blob_backend_or_url_refuses_before_open() {
        let objects = Directory::new();
        let other_objects = Directory::new();
        for original_object in [false, true] {
            let data = Directory::new();
            let original = if original_object {
                object_config(&data, &objects)
            } else {
                data.config()
            };
            drop(original.open_embedded().await.unwrap());
            let marker = fs::read(data.0.join("format.json")).unwrap();
            let changed = if original_object {
                data.config()
            } else {
                object_config(&data, &objects)
            };
            let failure = changed.open_embedded().await.err().unwrap();
            assert!(failure.to_string().contains("blob backend mismatch"));
            assert_eq!(fs::read(data.0.join("format.json")).unwrap(), marker);
            if original_object {
                let failure = object_config(&data, &other_objects)
                    .open_embedded()
                    .await
                    .err()
                    .unwrap();
                assert!(failure.to_string().contains("blob URL mismatch"));
                assert!(!failure
                    .to_string()
                    .contains(other_objects.0.to_str().unwrap()));
                assert_eq!(fs::read(data.0.join("format.json")).unwrap(), marker);
                assert!(fs::read_dir(&other_objects.0).unwrap().next().is_none());
            }
            drop(original.open_embedded().await.unwrap());
        }
        // Legacy markers have no backend field and denote filesystem blobs.
        let data = Directory::new();
        fs::write(
            data.0.join("format.json"),
            br#"{"schema":1,"index":{"major":1},"blobs":{"major":1}}"#,
        )
        .unwrap();
        let failure = object_config(&data, &objects)
            .open_embedded()
            .await
            .err()
            .unwrap();
        assert!(failure.to_string().contains("blob backend mismatch"));
        assert!(!data.0.join("index").exists());
    }

    #[tokio::test]
    async fn native_blobs_forwards_stream_beyond_object_snapshot_limit() {
        let data = Directory::new();
        let objects = Directory::new();
        let cfg = object_config(&data, &objects);
        let (_client, blobs) = cfg.open_embedded().await.unwrap();
        let keys: std::collections::BTreeSet<_> =
            (0..4097).map(|i| format!("b1-{i:032x}")).collect();
        for key in &keys {
            fs::write(objects.0.join(key), b"body").unwrap();
        }
        assert!(
            blobs.list().await.is_err(),
            "fixture must exceed snapshot limit"
        );
        let entries: Vec<_> = blobs
            .list_stream()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(entries
            .iter()
            .all(|entry| entry.last_modified.is_some() && entry.generation.is_none()));
        assert_eq!(
            entries
                .into_iter()
                .map(|entry| entry.key)
                .collect::<std::collections::BTreeSet<_>>(),
            keys
        );
    }

    #[tokio::test]
    async fn native_filesystem_stream_reports_stored_entries() {
        let data = Directory::new();
        let (_client, blobs) = data.config().open_embedded().await.unwrap();
        let key = "b1-0123456789abcdef0123456789abcdef";
        blobs
            .put(key, bytes::Bytes::from_static(b"body"))
            .await
            .unwrap();
        let entries: Vec<_> = blobs
            .list_stream()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, key);
        assert!(entries[0].last_modified.is_some());
    }

    #[tokio::test]
    async fn invalid_provider_options_fail_without_exposing_credentials() {
        let data = Directory::new();
        let objects = Directory::new();
        let url = url::Url::from_directory_path(&objects.0).unwrap();
        let cfg = config(&[
            (ENV_DATA_DIR, data.0.to_str().unwrap()),
            (ENV_BLOB_URL, url.as_str()),
            (ENV_BLOB_OPTIONS, r#"{"private-option":"private-secret"}"#),
        ])
        .unwrap();
        let failure = cfg.open_embedded().await.err().unwrap();
        assert!(failure.to_string().contains(ENV_BLOB_OPTIONS));
        assert!(!failure.to_string().contains("private-secret"));
        assert!(!failure.to_string().contains(url.as_str()));
        assert!(fs::read_dir(&objects.0).unwrap().next().is_none());
        // Blob construction failed after opening the index; its lock was released.
        drop(
            object_config(&data, &objects)
                .open_embedded()
                .await
                .unwrap(),
        );
    }
}

#[cfg(feature = "embedded-sparq")]
#[test]
fn format_publication_preserves_a_concurrent_winners_backend() {
    let dir = Directory::new();
    let marker = dir.0.join("format.json");
    let tmp = dir.0.join("staged-format");
    let existing = br#"{"schema":1,"index":{"major":1},"blobs":{"major":1}}"#;
    fs::write(&marker, existing).unwrap();
    fs::write(&tmp, br#"{"schema":1,"index":{"major":1},"blobs":{"major":1,"backend":"object","url":"file:///other"}}"#).unwrap();
    assert!(publish_format(&tmp, &marker, Some("file:///other")).is_err());
    assert_eq!(fs::read(&marker).unwrap(), existing);
    fs::write(&tmp, existing).unwrap();
    publish_format(&tmp, &marker, None).unwrap();
    assert_eq!(fs::read(&marker).unwrap(), existing);
    fs::remove_file(&marker).unwrap();
    publish_format(&tmp, &marker, None).unwrap();
    assert_eq!(fs::read(&marker).unwrap(), existing);
}

#[cfg(feature = "embedded-sparq")]
#[test]
fn format_backend_identity_is_checked_independently_of_url() {
    let marker = br#"{"schema":1,"index":{"major":1},"blobs":{"major":1,"backend":"unknown"}}"#;
    assert!(validate_format(marker, None).is_err());
    let marker = br#"{"schema":1,"index":{"major":1},"blobs":{"major":1,"backend":"object","url":"s3://private/location"}}"#;
    let failure = validate_format(marker, Some("s3://other/location")).unwrap_err();
    assert!(failure.to_string().contains("blob URL mismatch"));
    assert!(!failure.to_string().contains("private"));
}
