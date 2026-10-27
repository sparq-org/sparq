use super::*;

#[test]
fn local_configuration_requires_a_kek_and_valid_ttl_and_scope() {
    for value in [None, Some(""), Some(" ")] {
        let mut values = vec![(ENV_CRYPT_PROVIDER, "local")];
        if let Some(value) = value {
            values.push((ENV_CRYPT_KEK, value));
        }
        assert!(config(&values)
            .unwrap_err()
            .to_string()
            .contains(ENV_CRYPT_KEK));
    }
    for ttl in ["-1", "bad", "0.5", "", "18446744073709551616"] {
        let error = config(&[
            (ENV_CRYPT_PROVIDER, "local"),
            (ENV_CRYPT_KEK, "key"),
            (ENV_CRYPT_TTL, ttl),
        ])
        .unwrap_err();
        assert!(error.to_string().contains(ENV_CRYPT_TTL));
    }
    for scope in ["", " "] {
        assert!(config(&[
            (ENV_CRYPT_PROVIDER, "local"),
            (ENV_CRYPT_KEK, "key"),
            (ENV_CRYPT_SCOPE, scope)
        ])
        .unwrap_err()
        .to_string()
        .contains(ENV_CRYPT_SCOPE));
    }
    let cfg = config(&[(ENV_CRYPT_PROVIDER, "local"), (ENV_CRYPT_KEK, "key")]).unwrap();
    assert!(cfg.crypt_enabled());
    assert_eq!(cfg.crypt.as_ref().unwrap().ttl, Duration::from_secs(600));
    // Dormant encryption settings cannot alter the default plaintext configuration.
    let cfg = config(&[
        (ENV_CRYPT_KEK, "missing"),
        (ENV_CRYPT_TTL, "invalid"),
        (ENV_CRYPT_SCOPE, ""),
    ])
    .unwrap();
    assert!(!cfg.crypt_enabled());
}

#[cfg(all(feature = "embedded-sparq", feature = "blob-fs"))]
mod durable {
    use super::*;
    use std::path::Path;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use bytes::Bytes;
    use sparq_lws_core::store::{
        BodyCache, CompositeStore, EmbeddedSparqClient, SparqClient, Store,
    };
    use tower::ServiceExt;

    const BASE: &str = "https://pod.example";
    const KEYS: &str = "urn:x-sparq-lws:data-keys/";
    const BODY: &[u8] = b"private resource bytes\0\xff never on disk as plaintext";

    fn kek(dir: &Directory, name: &str, byte: u8) -> PathBuf {
        use std::io::Write;
        let path = dir.0.join(name);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path).unwrap().write_all(&[byte; 32]).unwrap();
        path
    }

    fn local(dir: &Directory, key: &Path, extra: &[(&str, &str)]) -> StorageConfig {
        let mut values = vec![
            (ENV_DATA_DIR, dir.0.to_str().unwrap()),
            (ENV_CRYPT_PROVIDER, "local"),
            (ENV_CRYPT_KEK, key.to_str().unwrap()),
        ];
        values.extend_from_slice(extra);
        config(&values).unwrap()
    }

    async fn raw_boot(cfg: &StorageConfig) -> io::Result<(EmbeddedSparqClient, NativeBlobs)> {
        cfg.open_embedded_with(true, make_blobs).await
    }

    async fn encrypted_store(
        cfg: &StorageConfig,
    ) -> (
        EmbeddedSparqClient,
        CompositeStore<
            EmbeddedSparqClient,
            sparq_lws_core::store::blob_crypt::EncryptingBlobStore<
                NativeBlobs,
                sparq_lws_crypt::LocalKeyProvider,
            >,
        >,
    ) {
        let (index, raw) = raw_boot(cfg).await.unwrap();
        let blobs = cfg.encrypt(Arc::new(index.clone()), raw).await.unwrap();
        let store = CompositeStore::with_body_cache(index.clone(), blobs, BodyCache::disabled())
            .with_blob_scope(cfg.blob_scope(BASE));
        (index, store)
    }

    async fn records(index: &impl SparqClient) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for iri in index.list_children(KEYS).await.unwrap() {
            out.push(serde_json::from_str(&index.get_meta(&iri).await.unwrap().etag).unwrap());
        }
        out
    }

    fn marker(dir: &Directory) -> serde_json::Value {
        serde_json::from_slice(&fs::read(dir.0.join("format.json")).unwrap()).unwrap()
    }

    fn raw(dir: &Directory, key: &str) -> Bytes {
        FsBlobStore::open(dir.0.join("blobs"))
            .unwrap()
            .get(key)
            .unwrap()
            .into()
    }

    // This calls the production assembly entry point, including the provider branch,
    // the decorator, and sharing with the optional reconciler. Only authentication is a fixture.
    async fn app(
        cfg: &StorageConfig,
        issuer: &auth_fixture::KeyKit,
        seed: bool,
        reconcile: bool,
    ) -> Result<
        (
            EmbeddedSparqClient,
            axum::Router,
            Option<tokio::task::JoinHandle<()>>,
        ),
        Box<dyn std::error::Error>,
    > {
        let (index, mut blobs) = raw_boot(cfg).await?;
        if seed {
            let store = CompositeStore::new(index.clone(), blobs);
            for path in ["/", "/alice/", "/bob/"] {
                store
                    .write(&format!("{BASE}{path}"), Bytes::new(), "text/turtle")
                    .await?;
            }
            let acl = format!(
                "@prefix acl: <http://www.w3.org/ns/auth/acl#>. <#owner> a acl:Authorization; acl:agent <{}>; acl:accessTo <{BASE}/>; acl:default <{BASE}/>; acl:mode acl:Read, acl:Write, acl:Control.",
                auth_fixture::WEBID,
            );
            store
                .write(&format!("{BASE}/.acl"), Bytes::from(acl), "text/turtle")
                .await?;
            drop(store);
            blobs = make_blobs(Some(&cfg.data_dir.as_ref().unwrap().join("blobs")))?;
        }
        let verifier_cfg = solid_oidc_verifier::config::VerifierConfig::new(
            vec![auth_fixture::ISSUER.into()],
            BASE,
        );
        let replay = solid_oidc_verifier::replay::InMemoryReplayStore::with_window(
            verifier_cfg.replay_ttl(),
        );
        let verifier = solid_oidc_verifier::verifier::Verifier::new(
            verifier_cfg,
            auth_fixture::jwks_provider(issuer),
            replay,
        )?;
        let auth = sparq_lws_core::auth::AuthContext::new(verifier, BASE);
        let (app, task) = crate::build_app_for_backend(
            index.clone(),
            blobs,
            reconcile.then_some(Duration::from_secs(3600)),
            cfg,
            BASE,
            auth_fixture::ISSUER,
            Duration::from_secs(600),
            auth,
            crate::OverloadConfig::new(32, None),
            None,
        )
        .await?;
        Ok((index, app, task))
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

    async fn put_get(
        app: &axum::Router,
        path: &str,
        issuer: &auth_fixture::KeyKit,
        key: &auth_fixture::KeyKit,
    ) -> http::HeaderValue {
        let response = app
            .clone()
            .oneshot(request("PUT", path, BODY, issuer, key))
            .await
            .unwrap();
        let status = response.status();
        let tag = response.headers().get(header::ETAG).cloned();
        let detail = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        assert!(
            status.is_success(),
            "PUT {status}: {}",
            String::from_utf8_lossy(&detail)
        );
        let response = app
            .clone()
            .oneshot(request("GET", path, b"", issuer, key))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let etag = response.headers()[header::ETAG].clone();
        if let Some(tag) = tag {
            assert_eq!(tag, etag);
        }
        assert_eq!(to_bytes(response.into_body(), 1 << 20).await.unwrap(), BODY);
        etag
    }

    #[tokio::test]
    async fn native_http_boot_encrypts_and_restarts_with_plaintext_etags_and_pod_checks() {
        let dir = Directory::new();
        let key_file = kek(&dir, "kek", 19);
        let cfg = local(&dir, &key_file, &[]);
        let issuer = auth_fixture::KeyKit::generate();
        let key = auth_fixture::KeyKit::generate();
        let (index, plain_app, task) = app(&dir.config(), &issuer, true, false).await.unwrap();
        assert!(task.is_none());
        let plain_tag = put_get(&plain_app, "/alice/legacy", &issuer, &key).await;
        let legacy = index
            .get_meta(&format!("{BASE}/alice/legacy"))
            .await
            .unwrap();
        assert_eq!(raw(&dir, &legacy.blob_key), BODY);
        assert_eq!(marker(&dir)["blobs"]["major"], 1);
        drop((plain_app, index));

        let (index, encrypted_app, task) = app(&cfg, &issuer, false, true).await.unwrap();
        assert_eq!(
            marker(&dir)["blobs"],
            serde_json::json!({"major":2,"encryption":"envelope-v1"})
        );
        assert_eq!(
            put_get(&encrypted_app, "/alice/data", &issuer, &key).await,
            plain_tag
        );
        assert_eq!(
            put_get(&encrypted_app, "/bob/data", &issuer, &key).await,
            plain_tag
        );
        let alice = index.get_meta(&format!("{BASE}/alice/data")).await.unwrap();
        let bob = index.get_meta(&format!("{BASE}/bob/data")).await.unwrap();
        let alice_bytes = raw(&dir, &alice.blob_key);
        let bob_bytes = raw(&dir, &bob.blob_key);
        for bytes in [&alice_bytes, &bob_bytes] {
            assert!(bytes.starts_with(b"SQLB"));
            assert!(!bytes.windows(BODY.len()).any(|window| window == BODY));
        }
        assert_ne!(
            sparq_lws_crypt::parse_header(&alice_bytes).unwrap().dek_id,
            sparq_lws_crypt::parse_header(&bob_bytes).unwrap().dek_id
        );
        assert_eq!(records(&index).await.len(), 2);
        let task = task.expect("reconciler must share the configured decorator");
        task.abort();
        let _ = task.await;
        drop((encrypted_app, index));

        let (index, reopened, task) = app(&cfg, &issuer, false, false).await.unwrap();
        assert!(task.is_none());
        for path in ["/alice/legacy", "/alice/data", "/bob/data"] {
            let response = reopened
                .clone()
                .oneshot(request("GET", path, b"", &issuer, &key))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::ETAG], plain_tag);
            assert_eq!(to_bytes(response.into_body(), 1 << 20).await.unwrap(), BODY);
        }
        // Repoint the very same authenticated blob key, bypassing HTTP mutation rules.
        // The already-warm cache must enforce the pod check as well as the cold path.
        let mut substituted = bob.clone();
        substituted.blob_key.clone_from(&alice.blob_key);
        index
            .put_meta(&format!("{BASE}/bob/data"), substituted)
            .await
            .unwrap();
        let response = reopened
            .clone()
            .oneshot(request("GET", "/bob/data", b"", &issuer, &key))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        drop((reopened, index));
        let (index, reopened, _) = app(&cfg, &issuer, false, false).await.unwrap();
        let response = reopened
            .clone()
            .oneshot(request("GET", "/bob/data", b"", &issuer, &key))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "a cold cache must also reject another pod"
        );
        index
            .put_meta(&format!("{BASE}/bob/data"), bob)
            .await
            .unwrap();
        assert_eq!(
            put_get(&reopened, "/alice/after-restart", &issuer, &key).await,
            plain_tag
        );
        let after = index
            .get_meta(&format!("{BASE}/alice/after-restart"))
            .await
            .unwrap();
        let after_bytes = raw(&dir, &after.blob_key);
        assert_ne!(alice_bytes, after_bytes);
        assert_eq!(
            sparq_lws_crypt::parse_header(&alice_bytes).unwrap().dek_id,
            sparq_lws_crypt::parse_header(&after_bytes).unwrap().dek_id
        );
        assert!(records(&index)
            .await
            .iter()
            .any(|r| r["reserved_seals"].as_u64().unwrap() >= 2 * (1 << 16)));
        drop((reopened, index));

        let failure = raw_boot(&dir.config())
            .await
            .err()
            .expect("major 2 must refuse none");
        assert!(failure.to_string().contains("blobs.major=2"));
        assert!(failure.to_string().contains("none"));
        let wrong_key = kek(&dir, "wrong-kek", 20);
        let failure = app(&local(&dir, &wrong_key, &[]), &issuer, false, false)
            .await
            .err()
            .expect("wrong KEK must refuse at boot");
        assert!(failure.to_string().contains("wrong KEK"), "{failure}");
        // Both failed startups release the index lock and preserve the upgrade.
        drop(app(&cfg, &issuer, false, false).await.unwrap());
        assert_eq!(marker(&dir)["blobs"]["major"], 2);
    }

    #[tokio::test]
    async fn native_memory_boot_encrypts_without_a_data_directory() {
        use crate::reconcile_runtime::SharedStore;
        use sparq_lws_core::store::{InMemoryBlobStore, InMemorySparqClient};

        let dir = Directory::new();
        let key_file = kek(&dir, "kek", 49);
        let cfg = config(&[
            (ENV_CRYPT_PROVIDER, "local"),
            (ENV_CRYPT_KEK, key_file.to_str().unwrap()),
        ])
        .unwrap();
        assert_eq!(cfg.backend, "memory");
        let issuer = auth_fixture::KeyKit::generate();
        let key = auth_fixture::KeyKit::generate();
        let index = SharedStore::new(InMemorySparqClient::new());
        let blobs = SharedStore::new(InMemoryBlobStore::new());
        let store = CompositeStore::new(index.clone(), blobs.clone());
        store
            .write(&format!("{BASE}/"), Bytes::new(), "text/turtle")
            .await
            .unwrap();
        let acl = format!("@prefix acl: <http://www.w3.org/ns/auth/acl#>. <#owner> a acl:Authorization; acl:agent <{}>; acl:accessTo <{BASE}/>; acl:default <{BASE}/>; acl:mode acl:Read, acl:Write, acl:Control.", auth_fixture::WEBID);
        store
            .write(&format!("{BASE}/.acl"), Bytes::from(acl), "text/turtle")
            .await
            .unwrap();
        drop(store);
        let verifier_cfg = solid_oidc_verifier::config::VerifierConfig::new(
            vec![auth_fixture::ISSUER.into()],
            BASE,
        );
        let replay = solid_oidc_verifier::replay::InMemoryReplayStore::with_window(
            verifier_cfg.replay_ttl(),
        );
        let verifier = solid_oidc_verifier::verifier::Verifier::new(
            verifier_cfg,
            auth_fixture::jwks_provider(&issuer),
            replay,
        )
        .unwrap();
        let (app, task) = crate::build_app_for_backend(
            index.clone(),
            blobs.clone(),
            None,
            &cfg,
            BASE,
            auth_fixture::ISSUER,
            Duration::from_secs(600),
            sparq_lws_core::auth::AuthContext::new(verifier, BASE),
            crate::OverloadConfig::new(32, None),
            None,
        )
        .await
        .unwrap();
        assert!(task.is_none());
        put_get(&app, "/item", &issuer, &key).await;
        let meta = index.get_meta(&format!("{BASE}/item")).await.unwrap();
        let bytes = blobs.get(&meta.blob_key).await.unwrap();
        assert!(bytes.starts_with(b"SQLB"));
        assert!(!bytes.windows(BODY.len()).any(|window| window == BODY));
        assert_eq!(records(&index).await.len(), 1);
        assert!(!dir.0.join("format.json").exists());
    }

    #[tokio::test]
    async fn native_fixed_scope_applies_with_and_without_reconciliation() {
        for reconcile in [false, true] {
            let dir = Directory::new();
            let key_file = kek(&dir, "kek", 51);
            let cfg = local(&dir, &key_file, &[(ENV_CRYPT_SCOPE, "fixed-pod")]);
            let issuer = auth_fixture::KeyKit::generate();
            let key = auth_fixture::KeyKit::generate();
            let (index, router, task) = app(&cfg, &issuer, true, reconcile).await.unwrap();
            put_get(&router, "/alice/data", &issuer, &key).await;
            put_get(&router, "/bob/data", &issuer, &key).await;
            let keys = records(&index).await;
            assert_eq!(keys.len(), 1);
            assert_eq!(keys[0]["pod"], "fixed-pod");
            assert_eq!(keys[0]["scheme"], "solid");
            if let Some(task) = task {
                task.abort();
                let _ = task.await;
            }
        }
    }

    #[tokio::test]
    async fn fresh_encrypted_boot_upgrades_before_writes_and_reopen_preserves_the_marker() {
        let dir = Directory::new();
        let key = kek(&dir, "key", 53);
        let cfg = local(&dir, &key, &[]);
        let issuer = auth_fixture::KeyKit::generate();
        drop(app(&cfg, &issuer, false, false).await.unwrap());
        assert_eq!(
            marker(&dir)["blobs"],
            serde_json::json!({"major":2,"encryption":"envelope-v1"})
        );
        let mut format = marker(&dir);
        format["extra"] = "preserve extension".into();
        let bytes = serde_json::to_vec_pretty(&format).unwrap();
        fs::write(dir.0.join("format.json"), &bytes).unwrap();
        drop(app(&cfg, &issuer, false, false).await.unwrap());
        assert_eq!(fs::read(dir.0.join("format.json")).unwrap(), bytes);
    }

    #[tokio::test]
    async fn runtime_tampering_and_unknown_envelopes_fail_closed() {
        let dir = Directory::new();
        let key_file = kek(&dir, "key", 61);
        let cfg = local(&dir, &key_file, &[]);
        let issuer = auth_fixture::KeyKit::generate();
        let key = auth_fixture::KeyKit::generate();
        let (index, router, _) = app(&cfg, &issuer, true, false).await.unwrap();
        put_get(&router, "/alice/data", &issuer, &key).await;
        put_get(&router, "/bob/data", &issuer, &key).await;
        let alice = index.get_meta(&format!("{BASE}/alice/data")).await.unwrap();
        let original = raw(&dir, &alice.blob_key);
        let path = dir
            .0
            .join("blobs/objects")
            .join(&alice.blob_key[3..5])
            .join(&alice.blob_key[5..7])
            .join(&alice.blob_key);
        drop((router, index));
        let mut flipped = original.to_vec();
        *flipped.last_mut().unwrap() ^= 1;
        let mut unknown = original.to_vec();
        unknown[5..21].fill(0);
        for damaged in [flipped, unknown, b"SQLB".to_vec()] {
            fs::write(&path, damaged).unwrap();
            let (_index, router, _) = app(&cfg, &issuer, false, false).await.unwrap();
            let response = router
                .oneshot(request("GET", "/alice/data", b"", &issuer, &key))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        }
        fs::write(&path, original).unwrap();
        let (index, router, _) = app(&cfg, &issuer, false, false).await.unwrap();
        let mut keys = index.list_children(KEYS).await.unwrap();
        keys.sort();
        let mut record = index.get_meta(&keys[0]).await.unwrap();
        let saved = record.clone();
        let mut fields: serde_json::Value = serde_json::from_str(&record.etag).unwrap();
        let other: serde_json::Value =
            serde_json::from_str(&index.get_meta(&keys[1]).await.unwrap().etag).unwrap();
        fields["wrapped"] = other["wrapped"].clone();
        record.etag = serde_json::to_string(&fields).unwrap();
        index.put_meta(&keys[0], record).await.unwrap();
        drop((router, index));
        let failure = app(&cfg, &issuer, false, false)
            .await
            .err()
            .expect("swapped key must refuse startup");
        assert!(
            failure.to_string().contains("authentication failed"),
            "{failure}"
        );
        let (index, blobs) = raw_boot(&cfg).await.unwrap();
        index.put_meta(&keys[0], saved).await.unwrap();
        drop((index, blobs));
        drop(app(&cfg, &issuer, false, false).await.unwrap());
    }

    #[tokio::test]
    async fn runtime_scope_scheme_and_zero_ttl_reach_the_decorator() {
        for (lws, fixed, expected_pods) in [
            (false, None, 2),
            (false, Some("deployment-pod"), 1),
            (true, Some("ignored-in-lws"), 1),
        ] {
            let dir = Directory::new();
            let key = kek(&dir, "key", 32);
            let mut extra = vec![(ENV_CRYPT_TTL, "0")];
            if let Some(fixed) = fixed {
                extra.push((ENV_CRYPT_SCOPE, fixed));
            }
            if lws {
                extra.push(("SOLID_SERVER_PROTOCOL", "lws"));
            }
            let cfg = local(&dir, &key, &extra);
            let (index, store) = encrypted_store(&cfg).await;
            for path in ["/alice/a", "/alice/b", "/bob/a"] {
                store
                    .write(
                        &format!("{BASE}{path}"),
                        Bytes::from_static(BODY),
                        "application/octet-stream",
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    store.read(&format!("{BASE}{path}")).await.unwrap().body,
                    BODY
                );
            }
            let keys = records(&index).await;
            assert_eq!(keys.len(), expected_pods);
            for record in &keys {
                assert_eq!(record["scheme"], if lws { "lws" } else { "solid" });
            }
            let pod = if lws {
                format!("{BASE}/")
            } else {
                fixed
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{BASE}/alice/"))
            };
            let record = keys.iter().find(|record| record["pod"] == pod).unwrap();
            assert!(
                record["reserved_seals"].as_u64().unwrap() >= 2 * (1 << 16),
                "zero TTL must restore above the reservation"
            );
            if let Some(fixed) = fixed.filter(|_| !lws) {
                assert_eq!(record["pod"], fixed);
                store
                    .write(
                        &format!("{BASE}/.lws/subscriptions/pool"),
                        Bytes::from_static(BODY),
                        "text/plain",
                    )
                    .await
                    .unwrap();
                assert!(records(&index)
                    .await
                    .iter()
                    .any(|record| record["pod"] == format!("{BASE}/")));
            }
        }
    }

    #[tokio::test]
    async fn lws_path_prefix_is_part_of_the_authenticated_pod() {
        for storage in ["https://pod.example/", "https://pod.example/storage/"] {
            let dir = Directory::new();
            let key = kek(&dir, "key", 37);
            let cfg = local(
                &dir,
                &key,
                &[
                    ("SOLID_SERVER_PROTOCOL", "lws"),
                    (ENV_CRYPT_SCOPE, "ignored-in-lws"),
                ],
            );
            let (index, raw) = raw_boot(&cfg).await.unwrap();
            let encrypted = cfg.encrypt(Arc::new(index.clone()), raw).await.unwrap();
            let store =
                CompositeStore::with_body_cache(index.clone(), encrypted, BodyCache::disabled())
                    .with_blob_scope(cfg.blob_scope(storage));
            for path in ["alice/file", "bob/file", ".lws/subscriptions/item"] {
                let iri = format!("{storage}{path}");
                store
                    .write(&iri, Bytes::from_static(BODY), "application/octet-stream")
                    .await
                    .unwrap();
                assert_eq!(store.read(&iri).await.unwrap().body, BODY);
            }
            let keys = records(&index).await;
            assert_eq!(keys.len(), 1);
            assert_eq!(keys[0]["pod"], storage);
            assert_eq!(keys[0]["scheme"], "lws");
        }
    }

    #[tokio::test]
    async fn invalid_kek_and_format_refuse_without_upgrading_or_publishing_bytes() {
        let dir = Directory::new();
        let key = kek(&dir, "key", 33);
        let issuer = auth_fixture::KeyKit::generate();
        fs::write(&key, b"too short").unwrap();
        let cfg = local(&dir, &key, &[]);
        let failure = app(&cfg, &issuer, false, false).await.err().unwrap();
        assert!(failure.to_string().contains(ENV_CRYPT_KEK));
        assert_eq!(marker(&dir)["blobs"]["major"], 1);
        assert!(FsBlobStore::open(dir.0.join("blobs"))
            .unwrap()
            .list()
            .unwrap()
            .blobs
            .is_empty());
        fs::write(&key, [33; 32]).unwrap();
        for blobs in [
            serde_json::json!({"major":2}),
            serde_json::json!({"major":2,"encryption":"unknown"}),
            serde_json::json!({"major":3,"encryption":"envelope-v1"}),
            serde_json::json!({"major":1,"encryption":"envelope-v1"}),
        ] {
            let format = serde_json::json!({"schema":1,"index":{"major":1},"blobs":blobs});
            let bytes = serde_json::to_vec(&format).unwrap();
            fs::write(dir.0.join("format.json"), &bytes).unwrap();
            let failure = raw_boot(&cfg).await.err().unwrap();
            assert!(failure.to_string().contains("format.json"));
            assert_eq!(fs::read(dir.0.join("format.json")).unwrap(), bytes);
        }
    }

    #[tokio::test]
    async fn marker_creation_never_downgrades_an_existing_encrypted_format() {
        let dir = Directory::new();
        let format = br#"{"schema":1,"index":{"major":1},"blobs":{"major":2,"encryption":"envelope-v1"},"extra":"preserved"}"#;
        fs::write(dir.0.join("format.json"), format).unwrap();
        write_format(
            &dir.0,
            br#"{"schema":1,"index":{"major":1},"blobs":{"major":1}}"#,
            false,
        )
        .unwrap();
        assert_eq!(fs::read(dir.0.join("format.json")).unwrap(), format);
        assert!(fs::read_dir(&dir.0).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    }

    #[tokio::test]
    async fn waiting_plain_boot_revalidates_the_format_after_acquiring_the_lock() {
        let dir = Directory::new();
        let key = kek(&dir, "key", 45);
        let (index, blobs) = raw_boot(&dir.config()).await.unwrap();
        let cfg = config(&[(ENV_DATA_DIR, dir.0.to_str().unwrap()), (ENV_WAIT, "3")]).unwrap();
        let mut waiting = tokio::spawn(async move { raw_boot(&cfg).await });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut waiting)
                .await
                .is_err()
        );
        let encrypted = local(&dir, &key, &[])
            .encrypt(Arc::new(index.clone()), blobs)
            .await
            .unwrap();
        drop((encrypted, index));
        let failure = waiting
            .await
            .unwrap()
            .err()
            .expect("waiter must reject the upgraded directory");
        assert!(failure.to_string().contains("blobs.major=2"));
    }
}
