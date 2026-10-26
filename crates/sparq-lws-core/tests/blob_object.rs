#![cfg(all(
    feature = "blob-object",
    feature = "test-support",
    not(target_arch = "wasm32")
))]

mod common;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use futures_util::TryStreamExt;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::store::blob_contract;
use sparq_lws_core::store::blob_object::{ObjectBlobAdapter, ObjectBlobStore};
use sparq_lws_core::store::reconcile::{reconcile_orphans, ReconcileOptions};
use sparq_lws_core::store::{BlobStore, CompositeStore, InMemorySparqClient, SparqClient, Store};
use std::time::Duration;
use tower::ServiceExt;

fn local(dir: &tempfile::TempDir) -> ObjectBlobAdapter {
    let url = url::Url::from_directory_path(dir.path()).unwrap();
    ObjectBlobAdapter(
        ObjectBlobStore::from_url(url.as_str(), std::iter::empty::<(&str, &str)>()).unwrap(),
    )
}

#[tokio::test]
async fn local_filesystem_contract() {
    let dir = tempfile::tempdir().unwrap();
    let blob = local(&dir);
    blob_contract::run(&blob).await;
    let key = "b1-000000000000000000000000000000ff";
    blob.put(key, Bytes::new()).await.unwrap();
    let entry = blob.stat(key).await.unwrap().unwrap();
    assert!(entry.generation.is_none());
    assert!(entry.last_modified.is_some());
    assert!(blob
        .list()
        .await
        .unwrap()
        .iter()
        .all(|e| e.generation.is_none()));
}

async fn http_round_trip(blob: ObjectBlobAdapter) {
    use solid_oidc_verifier::{
        config::VerifierConfig, replay::InMemoryReplayStore, verifier::Verifier,
    };
    let store = CompositeStore::new(InMemorySparqClient::new(), blob.clone());
    let acl = format!(
        r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
        <#owner> a acl:Authorization; acl:agent <{}>; acl:accessTo <{}/>;
        acl:default <{}/>; acl:mode acl:Read, acl:Write, acl:Control."#,
        common::WEBID,
        common::BASE_URL,
        common::BASE_URL
    );
    store
        .write(
            &format!("{}/.acl", common::BASE_URL),
            Bytes::from(acl),
            "text/turtle",
        )
        .await
        .unwrap();
    let issuer = common::KeyKit::generate();
    let client = common::KeyKit::generate();
    let config = VerifierConfig::new(vec![common::ISSUER.into()], common::BASE_URL);
    let replay = InMemoryReplayStore::with_window(config.replay_ttl());
    let verifier = Verifier::new(config, common::jwks_provider(&issuer), replay).unwrap();
    let app = build_router(AppState::new(
        AuthContext::new(verifier, common::BASE_URL),
        LdpState::new(store, common::BASE_URL),
    ));
    let iri = format!("{}/object.bin", common::BASE_URL);
    let call = |method: &'static str, body: &'static str| {
        let access = common::mint_access_token(&issuer, &client.thumbprint);
        let proof = common::mint_dpop_proof(&client, method, &iri, &access);
        app.clone().oneshot(
            Request::builder()
                .method(method)
                .uri(&iri)
                .header("Authorization", format!("DPoP {access}"))
                .header("DPoP", proof)
                .header("Content-Type", "application/octet-stream")
                .body(Body::from(body))
                .unwrap(),
        )
    };
    assert_eq!(
        call("PUT", "first").await.unwrap().status(),
        StatusCode::CREATED
    );
    let keys: std::collections::BTreeSet<_> = blob
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.key)
        .collect();
    let response = call("GET", "").await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        "first"
    );
    assert_eq!(
        call("PUT", "second").await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
    let current: std::collections::BTreeSet<_> = blob
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.key)
        .collect();
    // Overwrite commits a fresh pointer and reclaims the superseded body through reclaim_blob.
    assert_eq!(keys.difference(&current).count(), 1);
    assert_eq!(current.difference(&keys).count(), 1);
    let response = call("GET", "").await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        "second"
    );
    assert_eq!(
        call("DELETE", "").await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call("GET", "").await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    let remaining = blob.list().await.unwrap();
    assert_eq!(remaining.len(), current.len() - 1);
    assert!(remaining
        .iter()
        .all(|e| keys.contains(&e.key) && current.contains(&e.key)));
}

#[tokio::test]
async fn router_write_overwrite_delete_reclaims_local_objects() {
    let dir = tempfile::tempdir().unwrap();
    http_round_trip(local(&dir)).await;
}

async fn sweep(blob: ObjectBlobAdapter) {
    let index = InMemorySparqClient::new();
    let store = CompositeStore::new(index, blob.clone());
    store
        .write(
            "https://pod.example/live",
            Bytes::from_static(b"live"),
            "application/octet-stream",
        )
        .await
        .unwrap();
    let live = store.read("https://pod.example/live").await.unwrap();
    let orphan = "b1-ffffffffffffffffffffffffffffffff";
    blob.put(orphan, Bytes::from_static(b"orphan"))
        .await
        .unwrap();
    assert!(blob
        .stat(orphan)
        .await
        .unwrap()
        .unwrap()
        .last_modified
        .is_some());
    // The same objects are first inside, then outside a deliberate grace window. S3 stamps
    // whole seconds, so the nonzero grace is much larger than a tick. No sleeps are required.
    let index = InMemorySparqClient::new();
    index
        .replace_meta("https://pod.example/live", live.meta.clone())
        .await
        .unwrap();
    let report = reconcile_orphans(
        &index,
        &blob,
        &ReconcileOptions {
            grace: Duration::from_secs(3600),
            dry_run: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(report.referenced, 1);
    assert_eq!(report.too_young, 1);
    assert_eq!(report.deleted, 0);
    assert!(blob.exists(orphan).await.unwrap());
    let report = reconcile_orphans(
        &index,
        &blob,
        &ReconcileOptions {
            grace: Duration::ZERO,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(report.deleted, 1);
    assert_eq!(report.referenced, 1);
    assert!(!blob.exists(orphan).await.unwrap());
    assert_eq!(blob.get(&live.meta.blob_key).await.unwrap(), "live");
    store
        .delete("https://pod.example/live", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn local_orphan_sweep_respects_references_and_grace() {
    let dir = tempfile::tempdir().unwrap();
    sweep(local(&dir)).await;
}

#[tokio::test]
async fn reconcile_streams_beyond_snapshot_limit() {
    let dir = tempfile::tempdir().unwrap();
    let blob = local(&dir);
    for n in 0..4100 {
        blob.put(&format!("b1-{n:032x}"), Bytes::new())
            .await
            .unwrap();
    }
    assert!(blob.list().await.is_err());
    let count = blob
        .list_stream()
        .await
        .unwrap()
        .try_fold(0, |n, _| async move { Ok(n + 1) })
        .await
        .unwrap();
    assert_eq!(count, 4100);
    let index = InMemorySparqClient::new();
    let report = reconcile_orphans(
        &index,
        &blob,
        &ReconcileOptions {
            grace: Duration::ZERO,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(report.scanned, 4100);
    assert_eq!(report.deleted, 4100);
    assert_eq!(blob.list().await.unwrap().len(), 0);
}

fn minio() -> ObjectBlobAdapter {
    let url = std::env::var("LWS_BLOB_TEST_URL")
        .expect("set LWS_BLOB_TEST_URL to a dedicated s3://bucket/unique-prefix");
    let endpoint = std::env::var("LWS_BLOB_TEST_ENDPOINT").expect("set LWS_BLOB_TEST_ENDPOINT");
    let key = std::env::var("LWS_BLOB_TEST_ACCESS_KEY").expect("set LWS_BLOB_TEST_ACCESS_KEY");
    let secret = std::env::var("LWS_BLOB_TEST_SECRET_KEY").expect("set LWS_BLOB_TEST_SECRET_KEY");
    ObjectBlobAdapter(
        ObjectBlobStore::from_url(
            &url,
            [
                ("aws_endpoint", endpoint),
                ("aws_access_key_id", key),
                ("aws_secret_access_key", secret),
                ("aws_region", "us-east-1".into()),
                ("aws_allow_http", "true".into()),
                ("aws_virtual_hosted_style_request", "false".into()),
                ("aws_conditional_put", "etag".into()),
            ],
        )
        .unwrap(),
    )
}

#[tokio::test]
#[ignore = "requires a dedicated MinIO bucket and the dependency's aws feature"]
async fn minio_contract_router_and_orphan_sweep() {
    let blob = minio();
    blob_contract::run(&blob).await;
    http_round_trip(blob.clone()).await;
    // The HTTP fixture leaves its root ACL. Remove it before the isolated sweep fixture.
    for entry in blob.list().await.unwrap() {
        blob.delete(&entry.key).await.unwrap();
    }
    sweep(blob).await;
}
