use super::*;
use crate::store::{
    BackendCounters, BlobStore, CompositeStore, CountingSparqClient, InMemoryBlobStore,
    InMemorySparqClient, Store,
};

const PARENT: &str = "https://pod.example/c/";
const CHILD: &str = "https://pod.example/c/item";

fn meta(key: &str) -> ResourceMeta {
    ResourceMeta {
        content_type: "text/plain".into(),
        blob_key: key.into(),
        etag: format!("\"{key}\""),
        last_modified: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(123)),
    }
}

struct Directory(std::path::PathBuf);

impl Directory {
    fn empty() -> Self {
        let dir = Self(std::env::temp_dir().join(format!(
            "lws-journal-{}-{}",
            std::process::id(),
            next_nonce()
        )));
        Graph::load_str("", "ntriples")
            .unwrap()
            .save(&dir.0)
            .unwrap();
        dir
    }

    fn open(&self) -> EmbeddedSparqClient {
        EmbeddedSparqClient::open(&self.0).unwrap()
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn every_embedded_update_uses_the_commit_helper() {
    let source = include_str!("embedded.rs");
    let calls: Vec<_> = source
        .lines()
        .filter(|line| line.contains("sparq_engine::update_in_place"))
        .collect();
    assert_eq!(calls.len(), 2, "only the commit helper may execute updates");
    assert!(calls[0].contains("sparq_engine::update_in_place_atomic("));
    assert!(calls[1].contains("sparq_engine::update_in_place_capturing("));
    let helper = source
        .split("fn commit(")
        .nth(1)
        .unwrap()
        .split("#[path")
        .next()
        .unwrap();
    assert!(calls.iter().all(|call| helper.contains(call.trim())));
}

#[tokio::test]
async fn save_refuses_the_open_directory_and_aliases_without_losing_later_writes() {
    let dir = Directory::empty();
    let c = dir.open();
    c.put_meta(PARENT, meta("parent")).await.unwrap();
    c.create_child(PARENT, CHILD, meta("before")).await.unwrap();
    let mut destinations = vec![dir.0.clone(), dir.0.join("."), dir.0.join("new/..")];
    #[cfg(unix)]
    {
        let alias = dir.0.join("alias");
        std::os::unix::fs::symlink(&dir.0, &alias).unwrap();
        destinations.push(alias);
    }
    for destination in destinations {
        assert!(matches!(
            c.save(&destination).await,
            Err(SparqError::Backend(ref e)) if e.contains("open backing directory")
        ));
        assert_eq!(c.get_meta(CHILD).await.unwrap(), meta("before"));
        assert_eq!(c.list_children(PARENT).await.unwrap(), [CHILD]);
    }
    let snapshot = Directory(dir.0.join("snapshot"));
    c.save(&snapshot.0).await.unwrap();
    assert_eq!(
        snapshot.open().get_meta(CHILD).await.unwrap(),
        meta("before")
    );
    c.put_meta(CHILD, meta("after")).await.unwrap();
    let later = "https://pod.example/c/later";
    c.create_child(PARENT, later, meta("later")).await.unwrap();
    drop(c);
    let reopened = dir.open();
    assert_eq!(reopened.get_meta(CHILD).await.unwrap(), meta("after"));
    assert_eq!(reopened.get_meta(later).await.unwrap(), meta("later"));
    let mut children = reopened.list_children(PARENT).await.unwrap();
    children.sort();
    assert_eq!(children, [CHILD, later]);
}

#[tokio::test]
async fn acknowledged_writes_survive_repeated_recovery() {
    let dir = Directory::empty();
    for batch in 0..3 {
        let c = dir.open();
        for i in batch * 12..(batch + 1) * 12 {
            c.put_meta(
                &format!("https://pod.example/r{i}"),
                meta(&format!("key{i}")),
            )
            .await
            .unwrap();
        }
        drop(c);
        let c = dir.open();
        for i in 0..(batch + 1) * 12 {
            assert_eq!(
                c.get_meta(&format!("https://pod.example/r{i}"))
                    .await
                    .unwrap(),
                meta(&format!("key{i}")),
                "every acknowledged write remains after recovery"
            );
        }
    }
}

#[tokio::test]
async fn every_index_mutator_survives_a_reopen() {
    let dir = Directory::empty();
    let c = dir.open();
    c.put_meta(PARENT, meta("parent")).await.unwrap();
    let replaced = "https://pod.example/replaced";
    c.put_meta(replaced, meta("old")).await.unwrap();
    assert_eq!(
        c.replace_meta(replaced, meta("new")).await.unwrap(),
        Some(meta("old"))
    );
    c.create_child(PARENT, CHILD, meta("child")).await.unwrap();
    c.remove_child(PARENT, CHILD).await.unwrap();
    let ordinary = "https://pod.example/c/ordinary";
    c.create_child(PARENT, ordinary, meta("ordinary"))
        .await
        .unwrap();
    assert_eq!(
        c.delete_resource(ordinary, Some(PARENT)).await.unwrap(),
        Some("ordinary".into())
    );
    let removed = "https://pod.example/removed";
    c.put_meta(removed, meta("removed")).await.unwrap();
    c.delete_meta(removed).await.unwrap();
    let empty = "https://pod.example/c/empty/";
    c.create_child(PARENT, empty, meta("empty")).await.unwrap();
    assert_eq!(
        c.delete_meta_if_empty(empty, Some(PARENT)).await.unwrap(),
        DeleteOutcome::Deleted
    );
    assert!(c.begin_reclaim("garbage").await.unwrap());
    assert!(c.begin_reclaim("finished").await.unwrap());
    c.end_reclaim("finished").await.unwrap();
    assert!(!c.begin_reclaim("child").await.unwrap());
    let marker = "https://pod.example/.lws/subscriptions/#high-water";
    assert_eq!(c.raise_marker(marker, 42).await.unwrap(), 42);
    assert_eq!(c.raise_marker(marker, 10).await.unwrap(), 42);
    drop(c);

    for _ in 0..2 {
        let c = dir.open();
        assert_eq!(c.get_meta(PARENT).await.unwrap(), meta("parent"));
        assert_eq!(c.get_meta(replaced).await.unwrap(), meta("new"));
        assert_eq!(c.get_meta(CHILD).await.unwrap(), meta("child"));
        assert!(!c.exists(removed).await.unwrap());
        assert!(!c.exists(empty).await.unwrap());
        assert!(!c.exists(ordinary).await.unwrap());
        assert!(c.list_children(PARENT).await.unwrap().is_empty());
        assert_eq!(c.reclaiming().await.unwrap(), ["garbage"]);
        assert_eq!(c.get_meta(marker).await.unwrap().etag, "42");
    }
    let c = dir.open();
    c.end_reclaim("garbage").await.unwrap();
    c.put_meta(CHILD, meta("after-recovery")).await.unwrap();
    c.raise_marker(marker, 84).await.unwrap();
    drop(c);
    let c = dir.open();
    assert!(c.reclaiming().await.unwrap().is_empty());
    assert_eq!(c.get_meta(CHILD).await.unwrap(), meta("after-recovery"));
    assert_eq!(c.get_meta(marker).await.unwrap().etag, "84");
}

/// Reproduce the crash boundary through the public journal API: no core fault hook exists.
/// Diffing the captured dataset also expands DROP without depending on private engine helpers.
async fn leave_frame(c: &EmbeddedSparqClient, update: String, partial: bool) {
    c.dispatch(move |g| {
        fn quads(g: &Graph) -> std::collections::HashSet<(Option<oxrdf::Term>, [oxrdf::Term; 3])> {
            let mut quads = std::collections::HashSet::new();
            for (slot, graph) in std::iter::once((None, g))
                .chain(g.named.iter().map(|(name, sub)| (Some(name.clone()), sub)))
            {
                for ids in graph.iter_ids() {
                    quads.insert((slot.clone(), ids.map(|id| graph.dict.term(id))));
                }
            }
            quads
        }
        let before = quads(&g.graph);
        let mut working = g.graph.fork();
        let effects = sparq_engine::update_in_place_capturing(
            &mut working,
            &update,
            &sparq_engine::QueryBudget::unlimited(),
        )
        .unwrap();
        assert!(!effects.is_empty());
        let after = quads(&working);
        drop(working);
        let records: Vec<_> = before
            .difference(&after)
            .map(|(slot, t)| (false, slot.clone(), t.clone()))
            .chain(
                after
                    .difference(&before)
                    .map(|(slot, t)| (true, slot.clone(), t.clone())),
            )
            .collect();
        assert!(
            records
                .iter()
                .map(|r| &r.1)
                .collect::<std::collections::HashSet<_>>()
                .len()
                >= 2
        );
        g.graph.commit_txn(&records).unwrap();
        if partial {
            let (insert, slot, triple) = records
                .iter()
                .find(|r| matches!(&r.1, Some(oxrdf::Term::NamedNode(n)) if n.as_str() == PARENT))
                .unwrap();
            let i = g
                .graph
                .named
                .iter()
                .position(|(name, _)| Some(name) == slot.as_ref())
                .unwrap();
            let (ins, del) = if *insert {
                (std::slice::from_ref(triple), &[][..])
            } else {
                (&[][..], std::slice::from_ref(triple))
            };
            g.graph.named[i].1.apply_delta(ins, del).unwrap();
        }
        g.graph.poison_txn();
        g.stopped = true;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn create_and_delete_replay_as_whole_multi_graph_transactions() {
    for creating in [true, false] {
        for partial in [false, true] {
            let dir = Directory::empty();
            let c = dir.open();
            c.put_meta(PARENT, meta("parent")).await.unwrap();
            let update = if creating {
                sparql::update_create_child(
                    PARENT,
                    CHILD,
                    "text/plain",
                    "created",
                    "tag",
                    None,
                    "nonce",
                )
                .unwrap()
            } else {
                c.create_child(PARENT, CHILD, meta("deleted"))
                    .await
                    .unwrap();
                sparql::update_delete_resource_with_parent(CHILD, Some(PARENT)).unwrap()
            };
            leave_frame(&c, update, partial).await;
            drop(c);
            assert!(std::fs::metadata(dir.0.join("txn.log")).unwrap().len() > 0);
            for _ in 0..2 {
                let c = dir.open();
                assert_eq!(c.exists(CHILD).await.unwrap(), creating);
                assert_eq!(
                    c.list_children(PARENT).await.unwrap(),
                    if creating {
                        vec![CHILD.to_string()]
                    } else {
                        vec![]
                    }
                );
                if creating {
                    assert_eq!(c.get_meta(CHILD).await.unwrap().blob_key, "created");
                }
            }
            let c = dir.open();
            c.create_child(PARENT, CHILD, meta("next")).await.unwrap();
            drop(c);
            let c = dir.open();
            assert_eq!(c.get_meta(CHILD).await.unwrap(), meta("next"));
            assert_eq!(c.list_children(PARENT).await.unwrap(), [CHILD]);
        }
    }
}

#[tokio::test]
async fn an_uncommitted_multi_graph_frame_applies_nothing() {
    for creating in [true, false] {
        let dir = Directory::empty();
        let c = dir.open();
        c.put_meta(PARENT, meta("parent")).await.unwrap();
        let update = if creating {
            sparql::update_create_child(PARENT, CHILD, "text/plain", "new", "tag", None, "nonce")
                .unwrap()
        } else {
            c.create_child(PARENT, CHILD, meta("old")).await.unwrap();
            sparql::update_delete_resource_with_parent(CHILD, Some(PARENT)).unwrap()
        };
        leave_frame(&c, update, false).await;
        drop(c);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(dir.0.join("txn.log"))
            .unwrap();
        file.set_len(file.metadata().unwrap().len() - 1).unwrap();
        file.sync_all().unwrap();
        drop(file);
        for _ in 0..2 {
            let c = dir.open();
            assert_eq!(c.exists(CHILD).await.unwrap(), !creating);
            assert_eq!(
                c.list_children(PARENT).await.unwrap(),
                if creating {
                    vec![]
                } else {
                    vec![CHILD.to_string()]
                }
            );
        }
    }
}

#[tokio::test]
async fn refused_commit_leaves_state_unchanged_and_actor_usable() {
    let dir = Directory::empty();
    let c = dir.open();
    c.put_meta(CHILD, meta("before")).await.unwrap();
    c.dispatch(|g| {
        g.apply_failure = Some(sparq_engine::ApplyError {
            applied: false,
            message: "refused".into(),
        });
        Ok(())
    })
    .await
    .unwrap();
    assert!(matches!(
        c.put_meta(CHILD, meta("refused")).await,
        Err(SparqError::Backend(_))
    ));
    assert_eq!(c.get_meta(CHILD).await.unwrap(), meta("before"));
    c.put_meta(CHILD, meta("after")).await.unwrap();
    drop(c);
    assert_eq!(dir.open().get_meta(CHILD).await.unwrap(), meta("after"));
}

#[tokio::test]
async fn indeterminate_commit_stops_reads_writes_and_snapshots() {
    let dir = Directory::empty();
    let c = dir.open();
    c.dispatch(|g| {
        g.apply_failure = Some(sparq_engine::ApplyError {
            applied: true,
            message: "injected".into(),
        });
        Ok(())
    })
    .await
    .unwrap();
    let error = c.put_meta(CHILD, meta("uncertain")).await.unwrap_err();
    assert!(
        matches!(error, SparqError::Backend(ref e) if e.contains("indeterminate") && e.contains("restart to recover"))
    );
    for _ in 0..3 {
        let errors = [
            c.clone().get_meta(CHILD).await.unwrap_err(),
            c.put_meta(CHILD, meta("later")).await.unwrap_err(),
            c.exists(CHILD).await.unwrap_err(),
            c.save(&dir.0).await.unwrap_err(),
        ];
        for error in errors {
            assert!(
                matches!(error, SparqError::Backend(ref e) if e.contains("restart to recover"))
            );
        }
    }
    assert!(!dir.open().exists(CHILD).await.unwrap());
}

#[tokio::test]
async fn materialization_failure_stops_the_actor_and_reopens_the_committed_frame() {
    let dir = Directory::empty();
    let c = dir.open();
    c.put_meta(PARENT, meta("parent")).await.unwrap();
    // The parent occupies named/0. Refuse creation of the child's named/1 directory
    // after the engine has committed the whole transaction frame.
    let blocker = dir.0.join("named/1");
    std::fs::write(&blocker, b"refuse child materialization").unwrap();
    let error = c.create_child(PARENT, CHILD, meta("committed")).await;
    assert!(matches!(
        error,
        Err(SparqError::Backend(ref e))
            if e.contains("indeterminate") && e.contains("restart to recover")
    ));
    assert!(std::fs::metadata(dir.0.join("txn.log")).unwrap().len() > 0);
    for error in [
        c.exists(CHILD).await.unwrap_err(),
        c.put_meta(CHILD, meta("later")).await.unwrap_err(),
        c.save(&dir.0).await.unwrap_err(),
    ] {
        assert!(matches!(error, SparqError::Backend(ref e) if e.contains("restart to recover")));
    }
    drop(c);
    std::fs::remove_file(blocker).unwrap();
    for _ in 0..2 {
        let c = dir.open();
        assert_eq!(c.get_meta(CHILD).await.unwrap(), meta("committed"));
        assert_eq!(c.list_children(PARENT).await.unwrap(), [CHILD]);
    }
}

#[tokio::test]
async fn journal_cannot_repair_an_incomplete_named_snapshot() {
    let dir = Directory::empty();
    let c = dir.open();
    c.put_meta(PARENT, meta("parent")).await.unwrap();
    c.dispatch(|graph| {
        graph
            .ensure_named(&oxrdf::NamedNode::new(CHILD).unwrap().into())
            .unwrap();
        Ok(())
    })
    .await
    .unwrap();
    let update = sparql::update_create_child(
        PARENT,
        CHILD,
        "text/plain",
        "replayed",
        "tag",
        None,
        "nonce",
    )
    .unwrap();
    leave_frame(&c, update, false).await;
    drop(c);
    // Model a published manifest whose new snapshot was not fully persisted. Core
    // opens every named snapshot before replaying the parent's transaction journal.
    // This is a filesystem-state simulation, not an injected power loss.
    let snapshot = dir.0.join("named/1/dict-meta.bin");
    let saved = std::fs::read(&snapshot).unwrap();
    std::fs::remove_file(&snapshot).unwrap();
    assert!(matches!(
        EmbeddedSparqClient::open(&dir.0),
        Err(SparqError::Backend(_))
    ));
    assert!(std::fs::metadata(dir.0.join("txn.log")).unwrap().len() > 0);
    // Repairing that base snapshot lets the untouched frame recover both graphs.
    std::fs::write(snapshot, saved).unwrap();
    let c = dir.open();
    assert_eq!(c.get_meta(CHILD).await.unwrap().blob_key, "replayed");
    assert_eq!(c.list_children(PARENT).await.unwrap(), [CHILD]);
}

#[tokio::test]
async fn a_whole_job_is_captured_before_committing() {
    let dir = Directory::empty();
    let c = dir.open();
    c.put_meta(CHILD, meta("before")).await.unwrap();
    let valid = sparql::update_put_meta(CHILD, "text/plain", "after", "tag", None).unwrap();
    let invalid = format!("{valid}; INSERT DATA {{ invalid }}");
    assert!(c
        .dispatch(move |g| commit(g, &invalid, "invalid batch"))
        .await
        .is_err());
    assert_eq!(c.get_meta(CHILD).await.unwrap(), meta("before"));
    drop(c);
    assert_eq!(dir.open().get_meta(CHILD).await.unwrap(), meta("before"));
}

#[tokio::test]
async fn holds_intents_and_high_water_survive_recovery_and_further_writes() {
    let dir = Directory::empty();
    let store = CompositeStore::new(dir.open(), InMemoryBlobStore::new());
    store
        .write(PARENT, bytes::Bytes::new(), "text/turtle")
        .await
        .unwrap();
    store
        .create_in_container(
            PARENT,
            CHILD,
            bytes::Bytes::from_static(b"held"),
            "text/plain",
        )
        .await
        .unwrap();
    let held = store.hold(CHILD, "urn:holder").await.unwrap().unwrap();
    let mut current = store
        .write(CHILD, bytes::Bytes::from_static(b"current"), "text/plain")
        .await
        .unwrap();
    let intents = "urn:x-sparq-lws:intents:test/";
    let intent = "urn:x-sparq-lws:intents:test/change";
    store
        .write(intents, bytes::Bytes::new(), "text/turtle")
        .await
        .unwrap();
    let mut intent_meta = store
        .create_in_container(
            intents,
            intent,
            bytes::Bytes::from_static(b"{\"undo\":[]}"),
            "application/json",
        )
        .await
        .unwrap();
    // The RDF timestamp contract intentionally stores whole seconds.
    for expected in [&mut current, &mut intent_meta] {
        expected.last_modified = expected.last_modified.and_then(|time| {
            timestamp::to_xsd_datetime(time).and_then(|value| timestamp::from_xsd_datetime(&value))
        });
    }
    let marker = "https://pod.example/.lws/subscriptions/#high-water";
    store.raise_marker(marker, 42).await.unwrap();
    let blob = store.blob;
    drop(store.sparq);
    let store = CompositeStore::new(dir.open(), blob);
    assert_eq!(store.meta(CHILD).await.unwrap(), Some(current));
    assert_eq!(store.meta(intent).await.unwrap(), Some(intent_meta));
    assert_eq!(store.list_children(intents).await.unwrap().len(), 1);
    let holds = store.holds().await.unwrap();
    assert_eq!(holds.len(), 1);
    assert_eq!(holds[0].0, "urn:holder");
    assert_eq!(holds[0].1.blob_key, held.blob_key);
    assert_eq!(
        store.read_marker(marker).await.unwrap().as_deref(),
        Some("42")
    );
    store.release(&held, "urn:holder").await.unwrap();
    store.delete(intent, Some(intents)).await.unwrap();
    store.raise_marker(marker, 84).await.unwrap();
    let blob = store.blob;
    drop(store.sparq);
    let store = CompositeStore::new(dir.open(), blob);
    assert!(store.holds().await.unwrap().is_empty());
    assert!(!store.exists(intent).await.unwrap());
    assert!(store.list_children(intents).await.unwrap().is_empty());
    assert_eq!(
        store.read_marker(marker).await.unwrap().as_deref(),
        Some("84")
    );
    assert_eq!(
        store.read(CHILD).await.unwrap().body,
        bytes::Bytes::from_static(b"current")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_delete_returns_exactly_the_removed_key_under_concurrent_overwrite() {
    let dir = Directory::empty();
    let embedded = dir.open();
    for client in [
        Arc::new(embedded.clone()) as Arc<dyn SparqClient>,
        Arc::new(InMemorySparqClient::new()) as Arc<dyn SparqClient>,
    ] {
        client.put_meta(PARENT, meta("parent")).await.unwrap();
        for _ in 0..24 {
            client
                .create_child(PARENT, CHILD, meta("old"))
                .await
                .unwrap();
            let writer = client.clone();
            let deleter = client.clone();
            let write = tokio::spawn(async move {
                writer
                    .create_child(PARENT, CHILD, meta("new"))
                    .await
                    .unwrap();
            });
            let delete =
                tokio::spawn(
                    async move { deleter.delete_resource(CHILD, Some(PARENT)).await.unwrap() },
                );
            write.await.unwrap();
            let key = delete.await.unwrap().unwrap();
            match key.as_str() {
                "old" => {
                    assert_eq!(client.get_meta(CHILD).await.unwrap(), meta("new"));
                    assert_eq!(client.list_children(PARENT).await.unwrap(), [CHILD]);
                }
                "new" => {
                    assert!(!client.exists(CHILD).await.unwrap());
                    assert!(client.list_children(PARENT).await.unwrap().is_empty());
                }
                _ => panic!("delete returned a key it did not remove: {key}"),
            }
        }
        client.delete_resource(CHILD, Some(PARENT)).await.unwrap();
        assert_eq!(
            client.delete_resource(CHILD, Some(PARENT)).await.unwrap(),
            None
        );
        client
            .create_child(PARENT, CHILD, meta("parentless"))
            .await
            .unwrap();
        assert_eq!(
            client.delete_resource(CHILD, None).await.unwrap(),
            Some("parentless".into())
        );
        // An absent record still allows a stale edge to be detached.
        assert_eq!(
            client.delete_resource(CHILD, Some(PARENT)).await.unwrap(),
            None
        );
        assert!(client.list_children(PARENT).await.unwrap().is_empty());
    }
    drop(embedded);
    assert!(!dir.open().exists(CHILD).await.unwrap());
    assert!(dir.open().list_children(PARENT).await.unwrap().is_empty());
}

#[tokio::test]
async fn composite_delete_delegates_one_atomic_index_job_and_reclaims_its_key() {
    let c = EmbeddedSparqClient::in_memory().unwrap();
    let counted = CountingSparqClient::new(c.clone(), BackendCounters::new());
    let store = CompositeStore::new(counted, InMemoryBlobStore::new());
    store
        .write(PARENT, bytes::Bytes::new(), "text/turtle")
        .await
        .unwrap();
    let removed = store
        .create_in_container(
            PARENT,
            CHILD,
            bytes::Bytes::from_static(b"removed"),
            "text/plain",
        )
        .await
        .unwrap();
    let unrelated = store
        .write(
            "https://pod.example/other",
            bytes::Bytes::from_static(b"other"),
            "text/plain",
        )
        .await
        .unwrap();
    let before = c.engine_round_trips();
    store.delete(CHILD, Some(PARENT)).await.unwrap();
    // One delete job, one reclaim admission, one reclaim completion. The wrapper must
    // forward the atomic override instead of invoking its default's three separate jobs.
    assert_eq!(c.engine_round_trips() - before, 3);
    assert!(!store.blob.exists(&removed.blob_key).await.unwrap());
    assert!(store.blob.exists(&unrelated.blob_key).await.unwrap());
    assert!(!c.exists(CHILD).await.unwrap());
    assert!(c.list_children(PARENT).await.unwrap().is_empty());
}
