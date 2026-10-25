// AUTHORED-BY Claude Opus 4.8
//! Store-trait integration tests against the **embedded** SPARQ backend — re-proving the data-path
//! behaviors on the in-process engine (`EmbeddedSparqClient` over a `CompositeStore`), NOT assuming
//! them from the in-memory double.
//!
//! These mirror the core data-path cases in `tests/store.rs` (meta read, exists, put/rewrite,
//! create-child membership + missing-container guard, delete, delete-if-empty's three outcomes,
//! containment, parent-detach, referenced-blob-keys) but run them against the REAL sparq engine via
//! the embedded client. A green run proves the engine executes the named-graph builders in
//! [`sparq_lws_core::store::sparql`] with the same semantics the HTTP/in-mem impls give — the whole
//! point of the embed (same queries, different transport).
//!
//! The whole file is gated on the `embedded-sparq` feature — ON BY DEFAULT since sq-gg0qq.3 (the
//! embedded engine is the first-class in-workspace backend); it is a no-op test binary only under
//! `--no-default-features` (the engine-free profile).
#![cfg(feature = "embedded-sparq")]

use axum::body::Bytes;
use sparq_lws_core::error::ServerError;
use sparq_lws_core::store::embedded::EmbeddedSparqClient;
use sparq_lws_core::store::{
    CompositeStore, DeleteOutcome, InMemoryBlobStore, ResourceMeta, SparqClient, Store,
};

const IRI: &str = "https://pod.example/alice/data";
const TURTLE: &str =
    "<https://pod.example/alice/data#me> <http://xmlns.com/foaf/0.1/name> \"Alice\" .";

/// A composite store over the EMBEDDED SPARQ engine (fresh in-memory graph) + an in-memory blob
/// store — the exact production wiring `PSS_SPARQ_BACKEND=embedded` selects, minus the durable blob
/// backend (out of scope for the data-path proof).
fn store() -> impl Store {
    CompositeStore::new(
        EmbeddedSparqClient::in_memory().expect("empty in-memory embedded graph"),
        InMemoryBlobStore::new(),
    )
}

struct TestDir(std::path::PathBuf);

impl TestDir {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!("sparq-lws-restart-{}-{nonce}", std::process::id())))
    }

    fn reopen(&self, client: EmbeddedSparqClient) -> EmbeddedSparqClient {
        // Deliberately no save or graceful compaction: only acknowledged commits can survive.
        drop(client);
        EmbeddedSparqClient::open(&self.0).unwrap()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn durable_meta(key: &str) -> ResourceMeta {
    ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: key.into(),
        etag: format!("\"{key}\""),
        last_modified: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_783_254_896)),
    }
}

#[tokio::test]
async fn durable_create_child_keeps_metadata_and_parent_containment_together() {
    let dir = TestDir::new();
    let client = EmbeddedSparqClient::open_or_create(&dir.0).unwrap();
    let parent = "https://pod.example/alice/";
    let child = "https://pod.example/alice/note";
    let parent_meta = durable_meta("parent");
    client.put_meta(parent, parent_meta.clone()).await.unwrap();
    let mut client = dir.reopen(client);
    for key in ["first", "replacement"] {
        let expected = durable_meta(key);
        client
            .create_child(parent, child, expected.clone())
            .await
            .unwrap();
        // Both the initial create and the replacement must retain directory backing.
        client = dir.reopen(client);
        assert_eq!(client.get_meta(parent).await.unwrap(), parent_meta);
        assert_eq!(client.get_meta(child).await.unwrap(), expected);
        assert_eq!(client.list_children(parent).await.unwrap(), vec![child]);
    }
}

#[tokio::test]
async fn durable_delete_paths_do_not_resurrect_records_or_containment() {
    let dir = TestDir::new();
    let client = EmbeddedSparqClient::open_or_create(&dir.0).unwrap();
    let parent = "https://pod.example/alice/";
    let detached = "https://pod.example/alice/detached";
    let deleted = "https://pod.example/alice/deleted";
    let empty = "https://pod.example/alice/empty/";
    client
        .put_meta(parent, durable_meta("parent"))
        .await
        .unwrap();
    for iri in [detached, deleted, empty] {
        client
            .create_child(parent, iri, durable_meta(iri))
            .await
            .unwrap();
    }
    let client = dir.reopen(client);
    assert_eq!(
        client.delete_meta_if_empty(parent, None).await.unwrap(),
        DeleteOutcome::NotEmpty
    );
    client.remove_child(parent, detached).await.unwrap();
    let client = dir.reopen(client);
    assert_eq!(
        client.get_meta(detached).await.unwrap(),
        durable_meta(detached)
    );
    let mut listed = client.list_children(parent).await.unwrap();
    listed.sort();
    assert_eq!(listed, vec![deleted, empty]);

    client.delete_meta(deleted).await.unwrap();
    let client = dir.reopen(client);
    assert!(!client.exists(deleted).await.unwrap());
    assert_eq!(
        client
            .delete_meta_if_empty(empty, Some(parent))
            .await
            .unwrap(),
        DeleteOutcome::Deleted
    );
    let client = dir.reopen(client);
    assert!(!client.exists(empty).await.unwrap());
    // delete_meta drops only the resource; the explicit detach and guarded delete remove edges.
    assert_eq!(client.list_children(parent).await.unwrap(), vec![deleted]);
    assert_eq!(
        client.get_meta(parent).await.unwrap(),
        durable_meta("parent")
    );
    assert_eq!(
        client.get_meta(detached).await.unwrap(),
        durable_meta(detached)
    );
    assert_eq!(
        client
            .delete_meta_if_empty(empty, Some(parent))
            .await
            .unwrap(),
        DeleteOutcome::NotFound
    );
    client.delete_meta(deleted).await.unwrap();
    let client = dir.reopen(client);
    assert!(!client.exists(deleted).await.unwrap());
}

#[tokio::test]
async fn durable_reclaim_records_and_marker_raises_survive_reopen() {
    let dir = TestDir::new();
    let client = EmbeddedSparqClient::open_or_create(&dir.0).unwrap();
    let marker = "https://pod.example/.lws/subscriptions/#high-water";
    client.put_meta(IRI, durable_meta("live")).await.unwrap();
    assert!(!client.begin_reclaim("live").await.unwrap());
    assert!(client.begin_reclaim("orphan").await.unwrap());
    assert_eq!(client.raise_marker(marker, 300).await.unwrap(), 300);
    let client = dir.reopen(client);
    assert_eq!(client.reclaiming().await.unwrap(), vec!["orphan"]);
    assert_eq!(client.get_meta(marker).await.unwrap().etag, "300");
    assert_eq!(client.raise_marker(marker, 200).await.unwrap(), 300);
    client.end_reclaim("orphan").await.unwrap();
    let client = dir.reopen(client);
    assert!(client.reclaiming().await.unwrap().is_empty());
    assert_eq!(client.get_meta(marker).await.unwrap().etag, "300");
    assert_eq!(client.raise_marker(marker, 400).await.unwrap(), 400);
    let client = dir.reopen(client);
    assert_eq!(client.get_meta(marker).await.unwrap().etag, "400");
    assert_eq!(client.get_meta(IRI).await.unwrap(), durable_meta("live"));
}

#[tokio::test]
async fn durable_commit_failure_requires_reopen_and_keeps_the_frame_for_recovery() {
    let dir = TestDir::new();
    let client = EmbeddedSparqClient::open_or_create(&dir.0).unwrap();
    // A real filesystem obstruction: the first named graph cannot materialize after the
    // transaction frame is fsync'd. No fault hook or disabled fsync stands in for the write.
    std::fs::create_dir_all(dir.0.join("named")).unwrap();
    let obstruction = dir.0.join("named/0");
    std::fs::write(&obstruction, b"not a directory").unwrap();
    let expected = durable_meta("committed");
    let error = client.put_meta(IRI, expected.clone()).await.unwrap_err();
    assert!(
        error.to_string().contains("index must be reopened"),
        "{error}"
    );
    let frame = std::fs::read(dir.0.join("txn.log")).unwrap();
    assert!(!frame.is_empty(), "keep the committed frame for reopen");

    let refused = "https://pod.example/refused";
    let error = client
        .put_meta(refused, durable_meta("refused"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("index must be reopened"), "{error}");
    let read_error = client.exists(refused).await.unwrap_err();
    assert_eq!(read_error.to_string(), error.to_string());
    assert_eq!(std::fs::read(dir.0.join("txn.log")).unwrap(), frame);

    // Repair the external obstruction only after dropping the poisoned client. Recovery is
    // Graph::open's job; the client must never try to repair a poisoned journal in-process.
    drop(client);
    std::fs::remove_file(obstruction).unwrap();
    let client = EmbeddedSparqClient::open(&dir.0).unwrap();
    assert_eq!(client.get_meta(IRI).await.unwrap(), expected);
    assert!(!client.exists(refused).await.unwrap());
    assert_eq!(std::fs::metadata(dir.0.join("txn.log")).unwrap().len(), 0);
}

#[tokio::test]
async fn durable_store_create_in_container_keeps_lws_record_pointers_on_reopen() {
    let dir = TestDir::new();
    let client = EmbeddedSparqClient::open_or_create(&dir.0).unwrap();
    let store = CompositeStore::new(client.clone(), InMemoryBlobStore::new());
    let mut records = Vec::new();
    for kind in ["grants", "requests", "subscriptions"] {
        let container = format!("https://pod.example/.lws/{kind}/");
        let record = format!("{container}record");
        store
            .write(&container, Bytes::from_static(b"{}"), "application/json")
            .await
            .unwrap();
        let expected = store
            .create_in_container(
                &container,
                &record,
                Bytes::from_static(b"{\"id\":\"record\"}"),
                "application/json",
            )
            .await
            .unwrap();
        records.push((container, record, expected));
    }
    // Only the index is under test; these blob bytes are intentionally ephemeral.
    drop(store);
    let client = dir.reopen(client);
    for (container, record, expected) in records {
        let actual = client.get_meta(&record).await.unwrap();
        assert_eq!(actual.content_type, expected.content_type);
        assert_eq!(actual.blob_key, expected.blob_key);
        assert_eq!(actual.etag, expected.etag);
        assert!(actual.last_modified.is_some());
        assert_eq!(
            client.list_children(&container).await.unwrap(),
            vec![record]
        );
        assert!(client.exists(&container).await.unwrap());
    }
}

#[tokio::test]
async fn read_of_a_missing_resource_is_not_found() {
    let s = store();
    assert!(matches!(
        s.read(IRI).await.unwrap_err(),
        ServerError::NotFound
    ));
}

#[tokio::test]
async fn exists_is_false_before_a_write() {
    let s = store();
    assert!(!s.exists(IRI).await.unwrap());
}

#[tokio::test]
async fn write_then_read_round_trips_bytes_and_content_type() {
    let s = store();
    let body = Bytes::from_static(TURTLE.as_bytes());
    let meta = s.write(IRI, body.clone(), "text/turtle").await.unwrap();
    assert_eq!(meta.content_type, "text/turtle");
    assert!(!meta.etag.is_empty());

    assert!(s.exists(IRI).await.unwrap());
    let resource = s.read(IRI).await.unwrap();
    assert_eq!(resource.body, body);
    assert_eq!(resource.meta.content_type, "text/turtle");
    assert_eq!(resource.meta.etag, meta.etag);
}

#[tokio::test]
async fn rewrite_replaces_the_bytes_and_metadata() {
    let s = store();
    s.write(IRI, Bytes::from_static(b"<a> <b> <c> ."), "text/turtle")
        .await
        .unwrap();
    let new_body = Bytes::from_static(b"<a> <b> <d> .");
    s.write(IRI, new_body.clone(), "application/ld+json")
        .await
        .unwrap();
    let resource = s.read(IRI).await.unwrap();
    assert_eq!(resource.body, new_body);
    // The metadata record is REPLACED (single-valued), not accumulated, so a read is deterministic.
    assert_eq!(resource.meta.content_type, "application/ld+json");
}

#[tokio::test]
async fn delete_removes_metadata_and_bytes() {
    let s = store();
    s.write(IRI, Bytes::from_static(TURTLE.as_bytes()), "text/turtle")
        .await
        .unwrap();
    assert!(s.exists(IRI).await.unwrap());

    s.delete(IRI, None).await.unwrap();
    assert!(!s.exists(IRI).await.unwrap());
    assert!(matches!(
        s.read(IRI).await.unwrap_err(),
        ServerError::NotFound
    ));
}

#[tokio::test]
async fn delete_is_idempotent_on_absent() {
    let s = store();
    s.delete(IRI, None).await.unwrap();
}

#[tokio::test]
async fn create_in_container_records_membership() {
    let s = store();
    let container = "https://pod.example/alice/";
    let child = "https://pod.example/alice/note1";
    s.write(
        container,
        Bytes::from_static(b"<#c> <#p> \"C\" ."),
        "text/turtle",
    )
    .await
    .unwrap();
    s.create_in_container(
        container,
        child,
        Bytes::from_static(b"<#it> <#p> \"x\" ."),
        "text/turtle",
    )
    .await
    .unwrap();

    assert_eq!(
        s.list_children(container)
            .await
            .unwrap()
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>(),
        vec![child]
    );
    assert!(s.exists(child).await.unwrap());
}

#[tokio::test]
async fn create_in_a_missing_container_is_not_found() {
    let s = store();
    let err = s
        .create_in_container(
            "https://pod.example/missing/",
            "https://pod.example/missing/child",
            Bytes::from_static(b"<#it> <#p> \"x\" ."),
            "text/turtle",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::NotFound));
    assert!(!s.exists("https://pod.example/missing/child").await.unwrap());
}

#[tokio::test]
async fn create_in_container_twice_keeps_a_single_membership() {
    let s = store();
    let container = "https://pod.example/alice/";
    let child = "https://pod.example/alice/note1";
    s.write(
        container,
        Bytes::from_static(b"<#c> <#p> \"C\" ."),
        "text/turtle",
    )
    .await
    .unwrap();
    for _ in 0..2 {
        s.create_in_container(
            container,
            child,
            Bytes::from_static(b"<#it> <#p> \"x\" ."),
            "text/turtle",
        )
        .await
        .unwrap();
    }
    assert_eq!(
        s.list_children(container)
            .await
            .unwrap()
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>(),
        vec![child],
        "a re-create of the same child IRI must not duplicate the membership edge"
    );
}

#[tokio::test]
async fn create_child_commits_metadata_and_membership_atomically_on_the_engine() {
    // Directly on the EMBEDDED SparqClient: a missing container is refused with nothing written; with
    // the container indexed, the child's metadata AND its edge land together.
    let sparq = EmbeddedSparqClient::in_memory().unwrap();
    let container = "https://pod.example/alice/";
    let child = "https://pod.example/alice/note1";
    let meta = ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: "k".into(),
        etag: "\"e\"".into(),
        last_modified: None,
    };

    let err = sparq
        .create_child(container, child, meta.clone())
        .await
        .unwrap_err();
    assert!(matches!(err, sparq_lws_core::store::SparqError::NotFound));
    assert!(!sparq.exists(child).await.unwrap());
    assert!(sparq.list_children(container).await.unwrap().is_empty());

    sparq.put_meta(container, meta.clone()).await.unwrap();
    sparq.create_child(container, child, meta).await.unwrap();
    assert!(sparq.exists(child).await.unwrap());
    assert_eq!(
        sparq.list_children(container).await.unwrap(),
        vec![child.to_string()]
    );
}

#[tokio::test]
async fn modified_time_round_trips_through_the_real_engine() {
    // The `pss:modified` `xsd:dateTime` written by `put_meta` must survive storage + retrieval
    // through the ACTUAL sparq-engine (not just the query-string builders): a whole-second instant
    // in ⇒ the same instant back out of `get_meta`. This is the end-to-end proof the OPTIONAL SELECT
    // + the typed-literal storage work on a real engine — the basis of a correct `If-Modified-Since`.
    use std::time::{Duration, UNIX_EPOCH};

    let sparq = EmbeddedSparqClient::in_memory().unwrap();
    let iri = "https://pod.example/alice/data";
    // Whole seconds (the write truncates sub-second precision).
    let t = UNIX_EPOCH + Duration::from_secs(1_783_254_896);
    let meta = ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: "k".into(),
        etag: "\"e\"".into(),
        last_modified: Some(t),
    };
    sparq.put_meta(iri, meta).await.unwrap();
    let read_back = sparq.get_meta(iri).await.unwrap();
    assert_eq!(
        read_back.last_modified,
        Some(t),
        "the modification time must round-trip exactly through the engine"
    );

    // A re-write with a NEW time REPLACES it (single-valued), never accumulates.
    let t2 = UNIX_EPOCH + Duration::from_secs(1_783_254_900);
    let meta2 = ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: "k2".into(),
        etag: "\"e2\"".into(),
        last_modified: Some(t2),
    };
    sparq.put_meta(iri, meta2).await.unwrap();
    let after = sparq.get_meta(iri).await.unwrap();
    assert_eq!(
        after.last_modified,
        Some(t2),
        "a re-write bumps the modification time (single-valued replace)"
    );

    // A record written with NO modification time reads back `None` (the untracked case → 200).
    let none_iri = "https://pod.example/alice/untimed";
    let meta_none = ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: "k3".into(),
        etag: "\"e3\"".into(),
        last_modified: None,
    };
    sparq.put_meta(none_iri, meta_none).await.unwrap();
    assert_eq!(
        sparq.get_meta(none_iri).await.unwrap().last_modified,
        None,
        "no recorded modification time reads back as None"
    );
}

#[tokio::test]
async fn delete_detaches_from_parent_container() {
    let s = store();
    let container = "https://pod.example/alice/";
    let child = "https://pod.example/alice/note1";
    s.write(
        container,
        Bytes::from_static(b"<#c> <#p> \"C\" ."),
        "text/turtle",
    )
    .await
    .unwrap();
    s.create_in_container(
        container,
        child,
        Bytes::from_static(b"<#it> <#p> \"x\" ."),
        "text/turtle",
    )
    .await
    .unwrap();
    assert_eq!(
        s.list_children(container)
            .await
            .unwrap()
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>(),
        vec![child]
    );

    // Deleting the child WITH its parent detaches the membership edge atomically.
    s.delete(child, Some(container)).await.unwrap();
    assert!(!s.exists(child).await.unwrap());
    assert!(
        s.list_children(container).await.unwrap().is_empty(),
        "the child's membership edge must be detached from the parent on delete"
    );
}

#[tokio::test]
async fn delete_container_if_empty_refuses_a_populated_container() {
    // Safety invariant: a non-empty container is NEVER deleted — NotEmpty, nothing touched.
    let s = store();
    let container = "https://pod.example/alice/";
    let child = "https://pod.example/alice/note1";
    s.write(
        container,
        Bytes::from_static(b"<#c> <#p> \"C\" ."),
        "text/turtle",
    )
    .await
    .unwrap();
    s.create_in_container(
        container,
        child,
        Bytes::from_static(b"<#it> <#p> \"x\" ."),
        "text/turtle",
    )
    .await
    .unwrap();

    let outcome = s.delete_container_if_empty(container, None).await.unwrap();
    assert_eq!(outcome, DeleteOutcome::NotEmpty);
    assert!(
        s.exists(container).await.unwrap(),
        "NotEmpty must leave the container present"
    );
    assert!(
        s.exists(child).await.unwrap(),
        "NotEmpty must leave the child present (not orphaned)"
    );
    assert_eq!(
        s.list_children(container)
            .await
            .unwrap()
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>(),
        vec![child],
        "NotEmpty must leave the membership edge intact"
    );
}

#[tokio::test]
async fn delete_container_if_empty_deletes_an_empty_container() {
    let s = store();
    let container = "https://pod.example/alice/empty/";
    s.write(
        container,
        Bytes::from_static(b"<#c> <#p> \"C\" ."),
        "text/turtle",
    )
    .await
    .unwrap();
    assert!(s.exists(container).await.unwrap());

    let outcome = s.delete_container_if_empty(container, None).await.unwrap();
    assert_eq!(outcome, DeleteOutcome::Deleted);
    assert!(!s.exists(container).await.unwrap());
    assert!(matches!(
        s.read(container).await.unwrap_err(),
        ServerError::NotFound
    ));
}

#[tokio::test]
async fn delete_container_if_empty_reports_not_found_for_an_absent_container() {
    let s = store();
    let outcome = s
        .delete_container_if_empty("https://pod.example/alice/nope/", None)
        .await
        .unwrap();
    assert_eq!(outcome, DeleteOutcome::NotFound);
}

#[tokio::test]
async fn delete_container_if_empty_detaches_from_parent_and_routes_recreate_clean() {
    // The atomic empty-delete detaches the (deleted) container from its parent's containment, and a
    // container re-created at the same IRI inherits NO stale membership.
    let s = store();
    let parent = "https://pod.example/alice/";
    let container = "https://pod.example/alice/sub/";
    let child = "https://pod.example/alice/sub/note1";
    s.write(
        parent,
        Bytes::from_static(b"<#c> <#p> \"P\" ."),
        "text/turtle",
    )
    .await
    .unwrap();
    // The sub-container is a child of the parent.
    s.create_in_container(
        parent,
        container,
        Bytes::from_static(b"<#c> <#p> \"S\" ."),
        "text/turtle",
    )
    .await
    .unwrap();
    // Give the sub-container a child of its own, then remove it so the sub-container is empty.
    s.create_in_container(
        container,
        child,
        Bytes::from_static(b"<#it> <#p> \"x\" ."),
        "text/turtle",
    )
    .await
    .unwrap();
    s.delete(child, Some(container)).await.unwrap();

    // Now the empty sub-container deletes AND detaches from the parent.
    let outcome = s
        .delete_container_if_empty(container, Some(parent))
        .await
        .unwrap();
    assert_eq!(outcome, DeleteOutcome::Deleted);
    assert!(!s.exists(container).await.unwrap());
    assert!(
        s.list_children(parent).await.unwrap().is_empty(),
        "the deleted sub-container must be detached from the parent's containment"
    );

    // Re-create a container at the SAME IRI under the parent: it must inherit no stale membership.
    s.create_in_container(
        parent,
        container,
        Bytes::from_static(b"<#c> <#p> \"S2\" ."),
        "text/turtle",
    )
    .await
    .unwrap();
    assert!(
        s.list_children(container).await.unwrap().is_empty(),
        "a container re-created at the same IRI must inherit no stale containment"
    );
    assert_eq!(
        s.list_children(parent)
            .await
            .unwrap()
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>(),
        vec![container],
        "the parent re-contains the recreated sub-container exactly once"
    );
}

#[tokio::test]
async fn referenced_blob_keys_reflects_live_pointers() {
    // The reconciler's referenced-set: every live metadata record's blob key is referenced; a deleted
    // resource's key drops out.
    let s = store();
    let m1 = s
        .write(
            "https://pod.example/a",
            Bytes::from_static(b"<a> <b> <c> ."),
            "text/turtle",
        )
        .await
        .unwrap();
    let m2 = s
        .write(
            "https://pod.example/b",
            Bytes::from_static(b"<a> <b> <d> ."),
            "text/turtle",
        )
        .await
        .unwrap();
    // Reach the SparqClient through a second composite over the same engine is not possible (the engine
    // is moved into the store), so assert via list/containment-independent behaviour: deleting `a`
    // must drop its key from the live set. We re-prove referenced_blob_keys directly on a standalone
    // embedded client below; here we assert the store-level delete leaves `b` readable.
    s.delete("https://pod.example/a", None).await.unwrap();
    assert!(!s.exists("https://pod.example/a").await.unwrap());
    assert!(s.exists("https://pod.example/b").await.unwrap());
    assert_ne!(
        m1.blob_key, m2.blob_key,
        "distinct resources get distinct blob keys"
    );
}

#[tokio::test]
async fn referenced_blob_keys_on_the_embedded_client_collects_all_pointers() {
    // Directly on the embedded SparqClient: the referenced set is exactly the live records' keys.
    let sparq = EmbeddedSparqClient::in_memory().unwrap();
    let m = |bk: &str| ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: bk.into(),
        etag: "\"e\"".into(),
        last_modified: None,
    };
    sparq
        .put_meta("https://pod.example/a", m("k1"))
        .await
        .unwrap();
    sparq
        .put_meta("https://pod.example/b", m("k2"))
        .await
        .unwrap();
    let keys = sparq.referenced_blob_keys().await.unwrap();
    assert!(keys.contains("k1") && keys.contains("k2"), "got {keys:?}");
    assert_eq!(keys.len(), 2);

    // After deleting `a`, its key is no longer referenced.
    sparq.delete_meta("https://pod.example/a").await.unwrap();
    let keys = sparq.referenced_blob_keys().await.unwrap();
    assert!(
        !keys.contains("k1") && keys.contains("k2"),
        "k1 should drop out: {keys:?}"
    );
}

// --- sq-gg0qq.3 promotion invariants -----------------------------------------------------------
// The two properties the first-class promotion must re-prove on the REAL engine: (1) per-resource
// NAMED-GRAPH ISOLATION (graph IRI == resource IRI — the WAC-design model), and (2) the held-lock
// check-then-act ATOMICITY of `create_child` / `delete_meta_if_empty` under actual concurrency
// (many tokio tasks racing one engine), not just sequentially.

#[tokio::test]
async fn named_graph_isolation_mutating_one_resource_never_bleeds_into_another() {
    // Every resource lives in its OWN named graph (graph IRI == resource IRI), so a rewrite of `a`'s
    // record and then a full delete of `a` must leave `b`'s record — and a third CONTAINER's
    // membership set — byte-identical. A bleed (shared-graph storage, a DELETE WHERE that matches
    // across graphs) flips one of these assertions.
    let sparq = EmbeddedSparqClient::in_memory().unwrap();
    let m = |bk: &str, etag: &str| ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: bk.into(),
        etag: format!("\"{etag}\""),
        last_modified: None,
    };
    let a = "https://pod.example/alice/a";
    let b = "https://pod.example/alice/b";
    let container = "https://pod.example/alice/";
    let child = "https://pod.example/alice/c1";
    sparq.put_meta(a, m("ka", "ea")).await.unwrap();
    sparq.put_meta(b, m("kb", "eb")).await.unwrap();
    sparq.put_meta(container, m("kc", "ec")).await.unwrap();
    sparq
        .create_child(container, child, m("kchild", "echild"))
        .await
        .unwrap();

    // Rewrite `a` (same graph, new record) — `b` must be untouched.
    sparq.put_meta(a, m("ka2", "ea2")).await.unwrap();
    let got_b = sparq.get_meta(b).await.unwrap();
    assert_eq!(got_b.blob_key, "kb");
    assert_eq!(got_b.etag, "\"eb\"");

    // Delete `a` entirely — `b`, the container record, AND the containment edge all survive.
    sparq.delete_meta(a).await.unwrap();
    assert!(matches!(
        sparq.get_meta(a).await.unwrap_err(),
        sparq_lws_core::store::SparqError::NotFound
    ));
    assert_eq!(sparq.get_meta(b).await.unwrap().blob_key, "kb");
    assert_eq!(
        sparq.list_children(container).await.unwrap(),
        vec![child.to_string()],
        "deleting an unrelated resource must not disturb a container's membership graph"
    );
    // And the referenced-key set reflects exactly the survivors (a's key gone, nothing else lost).
    let keys = sparq.referenced_blob_keys().await.unwrap();
    assert!(!keys.contains("ka") && !keys.contains("ka2"), "{keys:?}");
    for live in ["kb", "kc", "kchild"] {
        assert!(keys.contains(live), "missing {live}: {keys:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_same_child_creates_commit_exactly_one_untorn_record() {
    // 16 tasks race `create_child(container, SAME child)` with CORRELATED metadata (blob_key `k<i>`
    // ⇄ etag `"e<i>"`). The held-lock guarded insert must leave exactly ONE membership edge and a
    // record whose (blob_key, etag) pair comes from ONE writer — a torn mix (k3 with "e7") or a
    // duplicated edge means the check-then-act interleaved.
    let sparq = EmbeddedSparqClient::in_memory().unwrap();
    let container = "https://pod.example/alice/";
    let child = "https://pod.example/alice/note";
    sparq
        .put_meta(
            container,
            ResourceMeta {
                content_type: "text/turtle".into(),
                blob_key: "kc".into(),
                etag: "\"ec\"".into(),
                last_modified: None,
            },
        )
        .await
        .unwrap();

    let mut handles = Vec::new();
    for i in 0..16u32 {
        let sparq = sparq.clone();
        handles.push(tokio::spawn(async move {
            sparq
                .create_child(
                    container,
                    child,
                    ResourceMeta {
                        content_type: "text/turtle".into(),
                        blob_key: format!("k{i}"),
                        etag: format!("\"e{i}\""),
                        last_modified: None,
                    },
                )
                .await
        }));
    }
    for h in handles {
        h.await
            .unwrap()
            .expect("create_child on a present container succeeds");
    }

    assert_eq!(
        sparq.list_children(container).await.unwrap(),
        vec![child.to_string()],
        "16 racing creators must leave exactly ONE membership edge"
    );
    let got = sparq.get_meta(child).await.unwrap();
    let i: u32 = got
        .blob_key
        .strip_prefix('k')
        .and_then(|s| s.parse().ok())
        .expect("blob_key is one of the writers' keys");
    assert_eq!(
        got.etag,
        format!("\"e{i}\""),
        "record is TORN: blob_key {} paired with etag {}",
        got.blob_key,
        got.etag
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_delete_if_empty_vs_create_child_never_leaves_a_torn_state() {
    // The TOCTOU pair the trait doc promises away: `delete_meta_if_empty(C)` racing
    // `create_child(C, child)`. Under the held engine lock exactly one of two CONSISTENT worlds may
    // remain per round — (1) delete won: C gone, child refused NotFound, parent edge detached; or
    // (2) create won: delete reports NotEmpty, C present with the child. `Deleted` AND a successful
    // create in the same round is the torn interleaving this test exists to catch.
    for round in 0..24u32 {
        let sparq = EmbeddedSparqClient::in_memory().unwrap();
        let m = |bk: &str| ResourceMeta {
            content_type: "text/turtle".into(),
            blob_key: bk.into(),
            etag: "\"e\"".into(),
            last_modified: None,
        };
        let parent = "https://pod.example/";
        let container = "https://pod.example/dir/";
        let child = "https://pod.example/dir/item";
        sparq.put_meta(parent, m("kp")).await.unwrap();
        sparq
            .create_child(parent, container, m("kc"))
            .await
            .unwrap();

        let deleter = {
            let sparq = sparq.clone();
            tokio::spawn(async move { sparq.delete_meta_if_empty(container, Some(parent)).await })
        };
        let creator = {
            let sparq = sparq.clone();
            tokio::spawn(async move { sparq.create_child(container, child, m("kx")).await })
        };
        let deleted = deleter.await.unwrap().unwrap();
        let created = creator.await.unwrap();

        let container_exists = sparq.exists(container).await.unwrap();
        let child_exists = sparq.exists(child).await.unwrap();
        let parent_children = sparq.list_children(parent).await.unwrap();
        match (deleted, created.is_ok()) {
            (DeleteOutcome::Deleted, false) => {
                assert!(
                    !container_exists && !child_exists && parent_children.is_empty(),
                    "round {round}: delete won but state is torn \
                     (container={container_exists}, child={child_exists}, parent={parent_children:?})"
                );
            }
            (DeleteOutcome::NotEmpty, true) => {
                assert!(
                    container_exists && child_exists,
                    "round {round}: create won but state is torn \
                     (container={container_exists}, child={child_exists})"
                );
                assert_eq!(
                    sparq.list_children(container).await.unwrap(),
                    vec![child.to_string()],
                    "round {round}"
                );
                assert_eq!(
                    parent_children,
                    vec![container.to_string()],
                    "round {round}"
                );
            }
            (outcome, created_ok) => panic!(
                "round {round}: TORN interleaving — delete={outcome:?}, create_ok={created_ok} \
                 (the held-lock check-then-act admitted both, or refused both)"
            ),
        }
    }
}

/// A marker raise is one step on the engine: a later raise to an earlier time is a no-op, and
/// the raise needs no blob.
#[tokio::test]
async fn a_marker_raise_never_lowers_what_is_stored() {
    let store = store();
    let iri = "https://pod.example/.lws/subscriptions/#high-water";
    assert!(store.raises_markers());
    assert_eq!(store.read_marker(iri).await.unwrap(), None);
    assert_eq!(store.raise_marker(iri, 300).await.unwrap(), 300);
    assert_eq!(store.raise_marker(iri, 200).await.unwrap(), 300);
    assert_eq!(
        store.read_marker(iri).await.unwrap().as_deref(),
        Some("300")
    );
    assert_eq!(store.raise_marker(iri, 400).await.unwrap(), 400);
}
