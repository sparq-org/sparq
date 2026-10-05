// AUTHORED-BY Claude Opus 4.8
//! Store-trait tests against the in-memory composite (SPARQ-authoritative metadata + blob bytes),
//! plus the RDF content-type classification + validation.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Bytes;
use sparq_lws_core::error::ServerError;
use sparq_lws_core::ldp::content::{classify, validate_rdf, RdfFormat};
use sparq_lws_core::store::{
    BlobError, BlobStore, CompositeStore, DeleteOutcome, InMemoryBlobStore, InMemorySparqClient,
    InMemoryStoreLimits, ResourceMeta, SparqClient, SparqError, Store, StoreUsage,
};

const IRI: &str = "https://pod.example/alice/data";
const TURTLE: &str =
    "<https://pod.example/alice/data#me> <http://xmlns.com/foaf/0.1/name> \"Alice\" .";

fn store() -> impl Store {
    CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new())
}

#[tokio::test]
async fn read_of_a_missing_resource_is_not_found() {
    let s = store();
    let err = s.read(IRI).await.unwrap_err();
    assert!(matches!(err, ServerError::NotFound));
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
async fn bounded_store_reports_usage_and_refuses_bytes_beyond_quota() {
    // [GPT-5.6] Mutation witness: removing the aggregate-byte admission check makes the second write
    // succeed, so both the error and unchanged usage assertions fail.
    let limits = InMemoryStoreLimits::new(5, 4);
    let s = CompositeStore::in_memory_with_limits(limits);

    s.write("urn:a", Bytes::from_static(b"123"), "text/plain")
        .await
        .unwrap();
    assert_eq!(
        s.usage().unwrap(),
        StoreUsage {
            total_bytes: 3,
            resource_count: 1,
        }
    );

    let error = s
        .write("urn:b", Bytes::from_static(b"456"), "text/plain")
        .await
        .unwrap_err();
    assert!(matches!(error, ServerError::InsufficientStorage));
    assert_eq!(error.status(), axum::http::StatusCode::INSUFFICIENT_STORAGE);
    assert_eq!(
        s.usage().unwrap(),
        StoreUsage {
            total_bytes: 3,
            resource_count: 1,
        }
    );
    assert_eq!(s.quota(), limits);
    assert!(!s.exists("urn:b").await.unwrap());
}

#[tokio::test]
async fn rewrite_replaces_the_bytes() {
    let s = store();
    s.write(IRI, Bytes::from_static(b"<a> <b> <c> ."), "text/turtle")
        .await
        .unwrap();
    let new_body = Bytes::from_static(b"<a> <b> <d> .");
    s.write(IRI, new_body.clone(), "text/turtle").await.unwrap();
    let resource = s.read(IRI).await.unwrap();
    assert_eq!(resource.body, new_body);
}

#[tokio::test]
async fn each_write_to_the_same_iri_mints_a_distinct_blob_key() {
    // The ROOT fix, observed at the Store seam: two successive writes to the SAME IRI must land their
    // bytes under DISTINCT blob keys (no deterministic key reuse). The metadata pointer moves to the
    // newest key, a read resolves the latest committed blob through it, and the first write's key is
    // reclaimed once the index has moved off it (its bytes were never overwritten in place).
    let inner = Arc::new(InMemoryBlobStore::new());
    let blob = SharedBlob {
        inner: inner.clone(),
    };
    let s = CompositeStore::new(InMemorySparqClient::new(), blob);

    let m1 = s
        .write(IRI, Bytes::from_static(b"<a> <b> <v1> ."), "text/turtle")
        .await
        .unwrap();
    let m2 = s
        .write(IRI, Bytes::from_static(b"<a> <b> <v2> ."), "text/turtle")
        .await
        .unwrap();

    assert_ne!(
        m1.blob_key, m2.blob_key,
        "two writes to the same IRI must mint DISTINCT blob keys (no deterministic reuse)"
    );
    // The superseded key is reclaimed after the index commit; it is never reused for the new bytes.
    assert!(
        matches!(inner.get(&m1.blob_key).await, Err(BlobError::NotFound)),
        "the first write's key is reclaimed once the index no longer references it"
    );
    assert_eq!(
        inner.get(&m2.blob_key).await.unwrap(),
        Bytes::from_static(b"<a> <b> <v2> ."),
    );
    // A read returns the LATEST committed blob (resolved through the metadata pointer, which names m2).
    let resource = s.read(IRI).await.unwrap();
    assert_eq!(resource.body, Bytes::from_static(b"<a> <b> <v2> ."));
    assert_eq!(resource.meta.blob_key, m2.blob_key);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_writes_to_the_same_iri_do_not_collide_on_a_blob_key() {
    // THE CONCURRENCY PROOF (the race the unique-key fix retires): N concurrent writes to the SAME IRI
    // must each land their bytes under a DIFFERENT key, so no two writes can collide / interleave on a
    // shared object. We assert:
    //   (1) every concurrent write minted a UNIQUE blob key (no two share one),
    //   (2) every key still in the blob store holds exactly the bytes ITS write put there (no write
    //       clobbered another's bytes on a shared key — the old deterministic-key collision), and every
    //       superseded key has been reclaimed (only the winner's bytes remain), and
    //   (3) a read after the burst returns the LATEST COMMITTED blob — the bytes whose metadata pointer
    //       won the index `put_meta` race — resolved through that pointer.
    //
    // Under the OLD deterministic key, all N writes shared ONE key: their bytes raced on a single
    // object (only the last `put` survived; the others' bytes were lost), so (1) and (2) would FAIL.
    let inner = Arc::new(InMemoryBlobStore::new());
    let blob = SharedBlob {
        inner: inner.clone(),
    };
    let s = Arc::new(CompositeStore::new(InMemorySparqClient::new(), blob));
    let n: usize = 64;

    let mut handles = Vec::new();
    for i in 0..n {
        let s = Arc::clone(&s);
        handles.push(tokio::spawn(async move {
            // Each task writes a DISTINCT body so its key→body mapping is identifiable.
            let body = Bytes::from(format!("<a> <b> <v{i}> ."));
            let meta = s.write(IRI, body.clone(), "text/turtle").await.unwrap();
            (meta.blob_key, body)
        }));
    }
    let mut keys_to_bodies = Vec::new();
    for h in handles {
        keys_to_bodies.push(h.await.unwrap());
    }

    // (1) Every concurrent write minted a UNIQUE key.
    let mut keys: Vec<String> = keys_to_bodies.iter().map(|(k, _)| k.clone()).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(
        keys.len(),
        n,
        "every concurrent write to the same IRI must mint a UNIQUE blob key (no collision)"
    );

    // (2) Whatever is still stored under a key is that write's own bytes (no clobber), and the
    // in-memory index's atomic replace lets every superseded key be reclaimed.
    for (key, body) in &keys_to_bodies {
        if let Ok(stored) = inner.get(key).await {
            assert_eq!(
                stored, *body,
                "each concurrent write's bytes must be intact under its own key — never clobbered"
            );
        }
    }
    assert_eq!(
        inner.usage().unwrap().resource_count,
        1,
        "every superseded key is reclaimed; only the winning write's bytes remain"
    );

    // (3) A read returns the latest committed blob, resolved through the surviving metadata pointer.
    let resource = s.read(IRI).await.unwrap();
    let winning_key = s.meta(IRI).await.unwrap().unwrap().blob_key;
    assert_eq!(resource.meta.blob_key, winning_key);
    let expected_body = keys_to_bodies
        .iter()
        .find(|(k, _)| *k == winning_key)
        .map(|(_, b)| b.clone())
        .expect("the winning metadata pointer must name one of the writes' keys");
    assert_eq!(
        resource.body, expected_body,
        "a read after concurrent writes must return the bytes of whichever write committed its index last"
    );
}

#[tokio::test]
async fn different_bytes_yield_a_different_etag() {
    let s = store();
    let m1 = s
        .write(IRI, Bytes::from_static(b"<a> <b> <c> ."), "text/turtle")
        .await
        .unwrap();
    let m2 = s
        .write(
            IRI,
            Bytes::from_static(b"<a> <b> <different> ."),
            "text/turtle",
        )
        .await
        .unwrap();
    assert_ne!(
        m1.etag, m2.etag,
        "different content must yield a different ETag"
    );
}

// --- M2: delete + containment ---

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
    // Deleting a never-written resource is not an error at the store layer.
    s.delete(IRI, None).await.unwrap();
}

#[tokio::test]
async fn create_in_container_records_membership() {
    let s = store();
    let container = "https://pod.example/alice/";
    let child = "https://pod.example/alice/note1";
    // The container must exist before a child can be attached (atomic parent-exists invariant).
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

    let children = s.list_children(container).await.unwrap();
    assert_eq!(
        children.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
        vec![child]
    );
    assert!(s.exists(child).await.unwrap());
}

#[tokio::test]
async fn create_in_a_missing_container_is_not_found() {
    let s = store();
    // The container was never created — create_in_container must refuse + write nothing.
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
    // Nothing was written under the missing container.
    assert!(!s.exists("https://pod.example/missing/child").await.unwrap());
}

#[tokio::test]
async fn create_in_container_twice_keeps_a_single_membership() {
    // Re-creating the same child IRI in a container must not duplicate the membership edge
    // (add_child is idempotent).
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
        vec![child]
    );
}

#[tokio::test]
async fn create_child_commits_metadata_and_membership_atomically() {
    // Directly exercise the atomic create on the SparqClient: both the child metadata and the edge
    // appear together, and a missing container is refused with nothing written.
    let sparq = InMemorySparqClient::new();
    let container = "https://pod.example/alice/";
    let child = "https://pod.example/alice/note1";
    let meta = ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: "k".into(),
        etag: "\"e\"".into(),
        last_modified: None,
    };

    // Missing container ⇒ NotFound, no metadata + no edge written.
    let err = sparq
        .create_child(container, child, meta.clone())
        .await
        .unwrap_err();
    assert!(matches!(err, SparqError::NotFound));
    assert!(!sparq.exists(child).await.unwrap());
    assert!(sparq.list_children(container).await.unwrap().is_empty());

    // With the container indexed, create_child commits the child meta AND the edge together.
    sparq.put_meta(container, meta.clone()).await.unwrap();
    sparq.create_child(container, child, meta).await.unwrap();
    assert!(sparq.exists(child).await.unwrap());
    assert_eq!(
        sparq.list_children(container).await.unwrap(),
        vec![child.to_string()]
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
    assert_eq!(s.list_children(container).await.unwrap().len(), 1);

    s.delete(child, Some(container)).await.unwrap();
    assert!(s.list_children(container).await.unwrap().is_empty());
    assert!(!s.exists(child).await.unwrap());
}

#[tokio::test]
async fn deleting_a_container_clears_its_own_containment_set() {
    // Deleting a container removes its OWN record AND its `ldp:contains` set (parity with the live
    // `DROP SILENT GRAPH`): a container re-created at the same IRI must not inherit a stale member.
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
    // Create the sub-container as a child of the parent, then a child inside it.
    s.create_in_container(
        parent,
        container,
        Bytes::from_static(b"<#c> <#p> \"S\" ."),
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
    // Empty the sub-container (the handler's empty-container precondition), then delete the container.
    s.delete(child, Some(container)).await.unwrap();
    assert!(s.list_children(container).await.unwrap().is_empty());
    s.delete(container, Some(parent)).await.unwrap();

    // The container is gone, detached from its parent, and its own containment set is cleared.
    assert!(!s.exists(container).await.unwrap());
    assert!(s.list_children(parent).await.unwrap().is_empty());
    assert!(
        s.list_children(container).await.unwrap().is_empty(),
        "a deleted container must leave no stale containment set"
    );
}

// --- M2: ATOMIC empty-container delete (the TOCTOU fix) ---

#[tokio::test]
async fn delete_container_if_empty_refuses_a_populated_container() {
    // The atomic op must report NotEmpty for a populated container AND delete NOTHING — the container
    // and its child both survive (no orphaning). This is the safety invariant: a non-empty container
    // is never deleted, so a concurrently-added child can't be orphaned under a deleted parent.
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
    // NOTHING was deleted: the container + its child + the membership edge all survive.
    assert!(
        s.exists(container).await.unwrap(),
        "a NotEmpty result must leave the container present"
    );
    assert!(
        s.exists(child).await.unwrap(),
        "a NotEmpty result must leave the child present (not orphaned)"
    );
    assert_eq!(
        s.list_children(container)
            .await
            .unwrap()
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>(),
        vec![child],
        "a NotEmpty result must leave the membership edge intact"
    );
}

#[tokio::test]
async fn delete_container_if_empty_deletes_an_empty_container() {
    // The atomic op returns Deleted for an empty container, and it is gone afterwards.
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
    // An absent container ⇒ NotFound (the handler maps this to 404), nothing touched.
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
    // container re-created at the same IRI inherits no stale membership — the delete-then-recreate
    // no-stale-membership case routed through the NEW atomic op.
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
    s.create_in_container(
        parent,
        container,
        Bytes::from_static(b"<#c> <#p> \"S\" ."),
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

    // While the sub-container has a member, the atomic delete refuses it (409 ⇒ NotEmpty).
    assert_eq!(
        s.delete_container_if_empty(container, Some(parent))
            .await
            .unwrap(),
        DeleteOutcome::NotEmpty
    );

    // Empty it, then the atomic delete succeeds + detaches from the parent.
    s.delete(child, Some(container)).await.unwrap();
    assert_eq!(
        s.delete_container_if_empty(container, Some(parent))
            .await
            .unwrap(),
        DeleteOutcome::Deleted
    );
    assert!(!s.exists(container).await.unwrap());
    assert!(
        s.list_children(parent).await.unwrap().is_empty(),
        "the deleted container must be detached from its parent"
    );

    // Re-create a container at the SAME IRI — it must inherit NO stale member from the deleted one.
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
        "a re-created container must not inherit a stale containment set"
    );
}

#[tokio::test]
async fn delete_meta_if_empty_on_the_sparq_client_is_atomic() {
    // Directly exercise the atomic op on the SparqClient: NotEmpty leaves both the container record and
    // the membership edge intact; Deleted removes the record + clears the containment set.
    let sparq = InMemorySparqClient::new();
    let container = "https://pod.example/alice/";
    let child = "https://pod.example/alice/note1";
    let meta = ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: "k".into(),
        etag: "\"e\"".into(),
        last_modified: None,
    };

    // Absent ⇒ NotFound.
    assert_eq!(
        sparq.delete_meta_if_empty(container, None).await.unwrap(),
        DeleteOutcome::NotFound
    );

    // Populated ⇒ NotEmpty, nothing removed.
    sparq.put_meta(container, meta.clone()).await.unwrap();
    sparq
        .create_child(container, child, meta.clone())
        .await
        .unwrap();
    assert_eq!(
        sparq.delete_meta_if_empty(container, None).await.unwrap(),
        DeleteOutcome::NotEmpty
    );
    assert!(sparq.exists(container).await.unwrap());
    assert_eq!(
        sparq.list_children(container).await.unwrap(),
        vec![child.to_string()]
    );

    // Empty it, then Deleted ⇒ record gone + containment set cleared.
    sparq.remove_child(container, child).await.unwrap();
    sparq.delete_meta(child).await.unwrap();
    assert_eq!(
        sparq.delete_meta_if_empty(container, None).await.unwrap(),
        DeleteOutcome::Deleted
    );
    assert!(!sparq.exists(container).await.unwrap());
    assert!(sparq.list_children(container).await.unwrap().is_empty());
}

#[tokio::test]
async fn delete_meta_if_empty_folds_the_parent_detach_into_the_one_op() {
    // FINDING 2 (in-memory parity): the parent-edge detach must happen IN the same atomic op as the
    // record delete — a single `delete_meta_if_empty(container, Some(parent))` both removes the
    // container record AND detaches `<parent> ldp:contains <container>`, with no separate `remove_child`
    // step. After a Deleted outcome the parent's containment must no longer list the container.
    let sparq = InMemorySparqClient::new();
    let parent = "https://pod.example/alice/";
    let container = "https://pod.example/alice/sub/";
    let meta = ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: "k".into(),
        etag: "\"e\"".into(),
        last_modified: None,
    };
    sparq.put_meta(parent, meta.clone()).await.unwrap();
    // Index the empty sub-container AND its edge in the parent (create_child commits both).
    sparq
        .create_child(parent, container, meta.clone())
        .await
        .unwrap();
    assert_eq!(
        sparq.list_children(parent).await.unwrap(),
        vec![container.to_string()],
        "the parent lists the sub-container before the delete"
    );

    // ONE op deletes the (empty) sub-container AND detaches it from the parent — no follow-up call.
    assert_eq!(
        sparq
            .delete_meta_if_empty(container, Some(parent))
            .await
            .unwrap(),
        DeleteOutcome::Deleted
    );
    assert!(!sparq.exists(container).await.unwrap());
    assert!(
        sparq.list_children(parent).await.unwrap().is_empty(),
        "the parent edge is detached in the SAME atomic op (no separate remove_child)"
    );
}

/// A pass-through [`BlobStore`] over an `Arc`-shared [`InMemoryBlobStore`], so a test can build a
/// [`CompositeStore`] AND retain a handle to the underlying blob store to inspect which physical keys
/// the writes landed under (the unique-per-write-key proof). Pure delegation — no behaviour change.
struct SharedBlob {
    inner: Arc<InMemoryBlobStore>,
}

#[async_trait]
impl BlobStore for SharedBlob {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        self.inner.get(key).await
    }
    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        self.inner.put(key, body).await
    }
    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        self.inner.exists(key).await
    }
    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        self.inner.delete(key).await
    }
    async fn list(&self) -> Result<Vec<sparq_lws_core::store::BlobEntry>, BlobError> {
        self.inner.list().await
    }
    async fn delete_if_unchanged(
        &self,
        key: &str,
        expected_generation: u64,
    ) -> Result<bool, BlobError> {
        self.inner
            .delete_if_unchanged(key, expected_generation)
            .await
    }
}

/// A [`BlobStore`] wrapper whose `delete` is unconditional (exactly like `InMemoryBlobStore`'s and
/// the real `object_store` delete the HIGH finding is about), but which COUNTS its `delete` calls so
/// the test can prove the store no longer issues an inline blob delete. The inner store is shared
/// (`Arc`) so the test can stage the in-window recreate and inspect the surviving bytes directly.
struct CountingBlob {
    inner: Arc<InMemoryBlobStore>,
    deletes_seen: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl BlobStore for CountingBlob {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        self.inner.get(key).await
    }

    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        self.inner.put(key, body).await
    }

    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        self.inner.exists(key).await
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        // Unconditional, idempotent — same contract as the real object_store delete. The point is only
        // to count: an inline delete here is exactly the clobber the finding describes.
        self.deletes_seen
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.delete(key).await
    }

    async fn list(&self) -> Result<Vec<sparq_lws_core::store::BlobEntry>, BlobError> {
        self.inner.list().await
    }

    async fn delete_if_unchanged(
        &self,
        key: &str,
        expected_generation: u64,
    ) -> Result<bool, BlobError> {
        // Delegate to the inner atomic CAS. This is the reconciler's delete path, not the inline
        // post-index-delete `delete` this test counts — so it deliberately does NOT bump `deletes_seen`.
        self.inner
            .delete_if_unchanged(key, expected_generation)
            .await
    }
}

#[tokio::test]
async fn delete_container_if_empty_does_not_clobber_a_concurrent_same_iri_recreate() {
    // Regression: deleting a container must NOT remove a concurrent same-IRI recreate's bytes. Blob
    // keys are UNIQUE PER WRITE, so a recreate mints a DIFFERENT key from the original, and the inline
    // reclaim after the index delete removes exactly the key the index held — the original's — never
    // the recreate's.
    //
    // We model the concurrent recreate writing its bytes (under its own unique key) before the index
    // delete returns. The recreate's bytes must survive; the original's are reclaimed (one delete).
    let deletes_seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let inner = Arc::new(InMemoryBlobStore::new());
    let blob = CountingBlob {
        inner: inner.clone(),
        deletes_seen: deletes_seen.clone(),
    };
    let s = CompositeStore::new(InMemorySparqClient::new(), blob);

    let container = "https://pod.example/alice/sub/";
    // Create the (empty) container — bytes land at a freshly-MINTED unique key for this write.
    s.write(
        container,
        Bytes::from_static(b"<#c> <#p> \"v1\" ."),
        "text/turtle",
    )
    .await
    .unwrap();
    // The original write's unique key (captured from the authoritative metadata pointer before delete).
    let original_key = s
        .meta(container)
        .await
        .unwrap()
        .expect("the container's metadata is present after the write")
        .blob_key;

    // Stage a concurrent same-IRI recreate. With unique-per-write keys it lands its bytes under its OWN
    // distinct key — NOT the original's. (We assert the keys differ below.) These bytes must survive.
    let recreate_bytes = Bytes::from_static(b"<#recreated> <#p> \"v2\" .");
    let recreate_key = format!("{original_key}-recreate-distinct");
    assert_ne!(
        original_key, recreate_key,
        "a unique-per-write recreate must NOT share the original's blob key (the root fix)"
    );
    inner
        .put(&recreate_key, recreate_bytes.clone())
        .await
        .unwrap();

    // Atomically delete the (now-empty, as far as the index is concerned) container's index row.
    let outcome = s.delete_container_if_empty(container, None).await.unwrap();
    assert_eq!(outcome, DeleteOutcome::Deleted);

    // Exactly one inline reclaim, of the key the deleted index row referenced.
    assert_eq!(
        deletes_seen.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "delete_container_if_empty reclaims the deleted container's own bytes inline"
    );
    assert!(
        matches!(inner.get(&original_key).await, Err(BlobError::NotFound)),
        "the deleted container's bytes are reclaimed"
    );

    // The concurrent recreate's bytes (under their own unique key) are intact — never clobbered, because
    // a recreate no longer shares a key with the deleted container.
    let surviving = inner
        .get(&recreate_key)
        .await
        .expect("a concurrent same-IRI recreate's bytes must survive the container delete");
    assert_eq!(
        surviving, recreate_bytes,
        "the recreated resource's bytes must be intact (never clobbered)"
    );
}

#[tokio::test]
async fn meta_returns_etag_without_bytes() {
    let s = store();
    let written = s
        .write(IRI, Bytes::from_static(TURTLE.as_bytes()), "text/turtle")
        .await
        .unwrap();
    let meta = s
        .meta(IRI)
        .await
        .unwrap()
        .expect("meta present after write");
    assert_eq!(meta.etag, written.etag);
    // Absent resource ⇒ None.
    assert!(s.meta("https://pod.example/nope").await.unwrap().is_none());
}

// --- content-type classification + RDF validation ---

#[test]
fn classifies_turtle_and_jsonld_ignoring_params() {
    assert_eq!(classify(Some("text/turtle")).unwrap(), RdfFormat::Turtle);
    assert_eq!(
        classify(Some("text/turtle; charset=utf-8")).unwrap(),
        RdfFormat::Turtle
    );
    assert_eq!(
        classify(Some("application/ld+json")).unwrap(),
        RdfFormat::JsonLd
    );
    // Case-insensitive.
    assert_eq!(classify(Some("TEXT/Turtle")).unwrap(), RdfFormat::Turtle);
}

#[test]
fn rejects_an_unsupported_or_absent_content_type() {
    assert!(matches!(
        classify(Some("application/json")).unwrap_err(),
        ServerError::UnsupportedMediaType(_)
    ));
    assert!(matches!(
        classify(None).unwrap_err(),
        ServerError::UnsupportedMediaType(_)
    ));
}

#[test]
fn validates_well_formed_turtle() {
    let n = validate_rdf(RdfFormat::Turtle, TURTLE.as_bytes(), IRI).unwrap();
    assert_eq!(n, 1);
}

#[test]
fn relative_iris_resolve_against_the_resource_base() {
    // A document using relative IRIs is valid — they resolve against the resource's own IRI.
    let body = b"<#me> <http://xmlns.com/foaf/0.1/name> \"Alice\" .";
    let n = validate_rdf(RdfFormat::Turtle, body, IRI).unwrap();
    assert_eq!(n, 1);
}

#[test]
fn rejects_malformed_turtle() {
    let bad = b"<a> <b> ."; // missing object
    let err = validate_rdf(RdfFormat::Turtle, bad, IRI).unwrap_err();
    assert!(matches!(err, ServerError::BadRequest(_)));
}

#[test]
fn validates_well_formed_jsonld() {
    let json = br#"{
        "@id": "https://pod.example/alice/data#me",
        "http://xmlns.com/foaf/0.1/name": "Alice"
    }"#;
    let n = validate_rdf(RdfFormat::JsonLd, json, IRI).unwrap();
    assert_eq!(n, 1);
}

#[test]
fn rejects_malformed_jsonld() {
    let bad = b"{ not valid json";
    let err = validate_rdf(RdfFormat::JsonLd, bad, IRI).unwrap_err();
    assert!(matches!(err, ServerError::BadRequest(_)));
}

// ---- Superseded-blob reclamation (the rewrite/delete storage leak) ----
//
// Every write mints a fresh blob key. Before the fix, the superseded key was left for the reconciler
// (off by default, one-hour grace), so sustained rewrites filled the in-memory blob quota with dead
// bodies and writes started failing with 507. These pin that the store reclaims the old key inline
// once the index no longer references it.

/// A deliberately tight quota: 50 stored entries. Without inline reclamation the 51st rewrite fails.
fn tight_store() -> CompositeStore<InMemorySparqClient, InMemoryBlobStore> {
    CompositeStore::in_memory_with_limits(InMemoryStoreLimits::new(1024 * 1024, 50))
}

#[tokio::test]
async fn sustained_rewrites_of_one_resource_never_exhaust_the_blob_quota() {
    let s = tight_store();
    for i in 0..1000 {
        let body = Bytes::from(format!("<a> <b> \"v{i}\" ."));
        s.write(IRI, body, "text/turtle")
            .await
            .unwrap_or_else(|e| panic!("rewrite {i} failed: {e:?}"));
        let usage = s.usage().unwrap();
        assert!(
            usage.resource_count <= 1,
            "rewrite {i}: superseded blobs must be reclaimed, got {} stored entries",
            usage.resource_count
        );
    }
    let latest = s.read(IRI).await.unwrap();
    assert_eq!(latest.body, Bytes::from_static(b"<a> <b> \"v999\" ."));
}

#[tokio::test]
async fn create_delete_cycles_never_exhaust_the_blob_quota() {
    let s = tight_store();
    let container = "https://pod.example/alice/c/";
    s.write(container, Bytes::new(), "text/turtle")
        .await
        .unwrap();
    for i in 0..1000 {
        // A document created into the container, then deleted.
        let child = format!("{container}doc-{i}");
        s.create_in_container(container, &child, Bytes::from_static(b"x"), "text/plain")
            .await
            .unwrap_or_else(|e| panic!("create {i} failed: {e:?}"));
        s.delete(&child, Some(container))
            .await
            .unwrap_or_else(|e| panic!("delete {i} failed: {e:?}"));
        // A root-level resource written then deleted.
        s.write(IRI, Bytes::from_static(b"y"), "text/plain")
            .await
            .unwrap_or_else(|e| panic!("write {i} failed: {e:?}"));
        s.delete(IRI, None).await.unwrap();
        // A child container created then deleted through the atomic empty-container path.
        let sub = format!("{container}sub-{i}/");
        s.create_in_container(container, &sub, Bytes::new(), "text/turtle")
            .await
            .unwrap_or_else(|e| panic!("create container {i} failed: {e:?}"));
        let outcome = s
            .delete_container_if_empty(&sub, Some(container))
            .await
            .unwrap();
        assert_eq!(outcome, DeleteOutcome::Deleted);
        let usage = s.usage().unwrap();
        assert!(
            usage.resource_count <= 1,
            "cycle {i}: deleted blobs must be reclaimed, got {} stored entries",
            usage.resource_count
        );
    }
}

/// A [`SparqClient`] whose `get_meta` answers ONCE with a staged stale record, then forwards. It
/// replays deterministically the reader that read the metadata just before a rewrite's index commit
/// and reached the blob only after that rewrite reclaimed it.
struct StaleOnceSparq {
    inner: InMemorySparqClient,
    stale: Arc<std::sync::Mutex<Option<ResourceMeta>>>,
}

#[async_trait]
impl SparqClient for StaleOnceSparq {
    async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError> {
        if let Some(stale) = self.stale.lock().unwrap().take() {
            return Ok(stale);
        }
        self.inner.get_meta(iri).await
    }
    async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
        self.inner.put_meta(iri, meta).await
    }
    async fn replace_meta(
        &self,
        iri: &str,
        meta: ResourceMeta,
    ) -> Result<Option<ResourceMeta>, SparqError> {
        self.inner.replace_meta(iri, meta).await
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
        self.inner.create_child(container, child, meta).await
    }
    async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError> {
        self.inner.remove_child(container, child).await
    }
    async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError> {
        self.inner.list_children(container).await
    }
    async fn referenced_blob_keys(&self) -> Result<std::collections::HashSet<String>, SparqError> {
        self.inner.referenced_blob_keys().await
    }
}

type StaleSlot = Arc<std::sync::Mutex<Option<ResourceMeta>>>;

fn stale_once_store() -> (CompositeStore<StaleOnceSparq, InMemoryBlobStore>, StaleSlot) {
    let slot = StaleSlot::default();
    let store = CompositeStore::with_body_cache(
        StaleOnceSparq {
            inner: InMemorySparqClient::new(),
            stale: slot.clone(),
        },
        InMemoryBlobStore::new(),
        sparq_lws_core::store::BodyCache::disabled(),
    );
    (store, slot)
}

#[tokio::test]
async fn a_reclaimed_blob_is_retried_by_read_but_never_substituted_by_read_at() {
    let (s, stale) = stale_once_store();
    let v1 = s
        .write(IRI, Bytes::from_static(b"<a> <b> \"v1\" ."), "text/turtle")
        .await
        .unwrap();
    let v2 = s
        .write(IRI, Bytes::from_static(b"<a> <b> \"v22\" ."), "text/turtle")
        .await
        .unwrap();
    assert_ne!(v1.etag, v2.etag);

    // `read`: its first metadata read sees v1, whose blob the second write already reclaimed.
    *stale.lock().unwrap() = Some(v1.clone());
    let r = s
        .read(IRI)
        .await
        .expect("a reclaimed blob is retried, not an error");
    assert_eq!(r.body, Bytes::from_static(b"<a> <b> \"v22\" ."));
    assert_eq!(
        r.meta.etag, v2.etag,
        "validators must match the bytes returned"
    );
    assert_eq!(r.meta.blob_key, v2.blob_key);

    // `read_at`: the caller holds (and authorized) v1 from its read plan. Its blob is reclaimed; the
    // store must NOT hand back v2's bytes in its place — v2 was never authorized (a rewrite can
    // change the ACL with it). It reports `ResourceChanged` so the caller restarts, authz included.
    let err = s.read_at(IRI, &v1).await.unwrap_err();
    assert!(
        matches!(err, ServerError::ResourceChanged),
        "read_at must never substitute a newer version's bytes, got {err:?}"
    );
    // Through the CURRENT pointer it serves exactly that version.
    assert_eq!(
        s.read_at(IRI, &v2).await.unwrap(),
        Bytes::from_static(b"<a> <b> \"v22\" .")
    );

    // Deleted in the window ⇒ NotFound, not a storage error.
    s.delete(IRI, None).await.unwrap();
    let err = s.read_at(IRI, &v2).await.unwrap_err();
    assert!(matches!(err, ServerError::NotFound), "got {err:?}");
}

#[tokio::test]
async fn read_reports_inconsistency_when_the_index_still_points_at_a_missing_blob() {
    // Not a race: the current pointer itself names a missing blob. The retry must not loop or mask it.
    let inner = Arc::new(InMemoryBlobStore::new());
    let s = CompositeStore::new(
        InMemorySparqClient::new(),
        SharedBlob {
            inner: inner.clone(),
        },
    );
    let m = s
        .write(IRI, Bytes::from_static(b"x"), "text/plain")
        .await
        .unwrap();
    inner.delete(&m.blob_key).await.unwrap();
    let err = s.read(IRI).await.unwrap_err();
    assert!(matches!(err, ServerError::Storage(_)), "got {err:?}");
}
