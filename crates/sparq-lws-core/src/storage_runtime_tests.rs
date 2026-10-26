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
mod embedded {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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

    struct Directory(PathBuf);

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

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

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

    type Boot = (EmbeddedSparqClient, DirectoryBlobs);

    async fn boot(config: &StorageConfig) -> io::Result<Boot> {
        config.open_embedded_with(true, DirectoryBlobs::open).await
    }

    fn error(result: io::Result<Boot>) -> io::Error {
        match result {
            Err(error) => error,
            Ok(_) => panic!("boot unexpectedly succeeded"),
        }
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
                DirectoryBlobs::open(path)
            })
            .await
            .unwrap();
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
        let called = AtomicBool::new(false);
        let result = cfg
            .open_embedded_with(true, |path| {
                called.store(true, Ordering::SeqCst);
                DirectoryBlobs::open(path)
            })
            .await;
        let error = error(result);
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(error
            .to_string()
            .contains(&dir.0.join("index.lock").display().to_string()));
        assert!(!called.load(Ordering::SeqCst));
        drop(held);
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
    async fn unavailable_filesystem_adapter_refuses_and_releases_the_index_lock() {
        let dir = Directory::new();
        let failure = dir
            .config()
            .open_embedded_with(true, make_blobs)
            .await
            .err()
            .unwrap();
        assert!(failure
            .to_string()
            .contains("filesystem blob store is not built in"));
        assert!(!dir.0.join("blobs").exists());
        drop(boot(&dir.config()).await.unwrap());
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
        let journal = logs(&dir.0.join("index"))
            .into_iter()
            .find(|path| fs::metadata(path).unwrap().len() > 0)
            .unwrap();
        let bytes = fs::read(&journal).unwrap();
        // The core refuses an unexpected swap staging path without deleting its contents.
        let staging = journal.parent().unwrap().with_extension("compact-new");
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("foreign"), b"preserve").unwrap();
        let mut shutdown = StorageShutdown::default();
        shutdown.retain(&dir.config(), &client);
        drop(client);
        assert!(shutdown.run(async { Ok(()) }, None).await.is_err());
        assert_eq!(fs::read(&journal).unwrap(), bytes);
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
        blobs: DirectoryBlobs,
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

    async fn restart_round_trip(lws: bool) {
        const BODY: &[u8] = b"persisted bytes\0\xff\n";
        let dir = Directory::new();
        let cfg = dir.config();
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
        restart_round_trip(false).await;
    }

    #[tokio::test]
    async fn lws_put_survives_restart_with_identical_bytes_etag_and_listing() {
        restart_round_trip(true).await;
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
