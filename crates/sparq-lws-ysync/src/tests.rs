use super::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use yrs::sync::{Message, SyncMessage};
use yrs::updates::{decoder::Decode, encoder::Encode};
use yrs::{Doc, ReadTxn, StateVector, Text, Transact};

#[derive(Clone, Default)]
struct Memory {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    fail: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    changes: Changes,
    lists: Arc<AtomicUsize>,
    accesses: Arc<AtomicUsize>,
    append_only: Arc<AtomicBool>,
    pause_list: Arc<AtomicBool>,
    list_entered: Arc<tokio::sync::Notify>,
    list_released: Arc<tokio::sync::Notify>,
    committed: Arc<tokio::sync::Notify>,
    quiet_create: Arc<AtomicBool>,
    pause_create: Arc<AtomicBool>,
    fail_after_create: Arc<AtomicBool>,
    entered: Arc<tokio::sync::Notify>,
    released: Arc<tokio::sync::Notify>,
}
#[async_trait]
impl Authorizer for Memory {
    async fn access(&self, _: &str, agent: &Agent) -> Result<Access, Fault> {
        self.accesses.fetch_add(1, Ordering::SeqCst);
        Ok(Access {
            read: true,
            write: true,
            delete: !self.append_only.load(Ordering::SeqCst) || agent.subject == "owner",
        })
    }
}
#[async_trait]
impl RoomStore for Memory {
    async fn list(&self, _: &str) -> Result<Vec<String>, Fault> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        let files = self.files.lock().unwrap().keys().cloned().collect();
        if self.pause_list.swap(false, Ordering::SeqCst) {
            self.list_entered.notify_one();
            self.list_released.notified().await;
        }
        Ok(files)
    }
    async fn read(&self, file: &str) -> Result<Vec<u8>, Fault> {
        self.files
            .lock()
            .unwrap()
            .get(file)
            .cloned()
            .ok_or(Fault::Store)
    }
    async fn create(&self, _: &str, agent: &Agent, file: &LogFile) -> Result<String, Fault> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) || agent.subject == "blocked" {
            return Err(Fault::Store);
        }
        if file.kind == LogKind::Update && self.pause_create.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.released.notified().await;
        }
        let name = format!("{}-{}-{call}", file.millis, file.nonce);
        self.files
            .lock()
            .unwrap()
            .insert(name.clone(), file.encode());
        if !self.quiet_create.load(Ordering::SeqCst) {
            self.changes.created("room", &name);
        }
        self.committed.notify_one();
        if self.fail_after_create.load(Ordering::SeqCst) {
            return Err(Fault::Store);
        }
        Ok(name)
    }
    async fn delete(&self, _: &str, _: &Agent, file: &str) -> Result<(), Fault> {
        self.files.lock().unwrap().remove(file);
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn failed_flush_retries_and_keeps_pending_data() {
    let store = Memory::default();
    store.fail.store(true, Ordering::SeqCst);
    let relay = Relay::new(store.clone(), Limits::default(), store.changes.clone());
    let agent = Agent {
        subject: "writer".into(),
        client: None,
        origin: None,
    };
    relay.ticket([1; 32], "room", agent).await.unwrap();
    let session = relay.join([1; 32], "room").await.unwrap();
    let doc = Doc::with_client_id(1);
    doc.get_or_insert_text("text")
        .push(&mut doc.transact_mut(), "retained");
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    relay
        .handle(
            &session,
            &Message::Sync(SyncMessage::Update(update)).encode_v1(),
        )
        .await
        .unwrap();
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert!(store.calls.load(Ordering::SeqCst) >= 1);
    assert!(store.files.lock().unwrap().is_empty());
    store.fail.store(false, Ordering::SeqCst);
    tokio::time::advance(std::time::Duration::from_secs(3)).await;
    tokio::task::yield_now().await;
    assert_eq!(store.files.lock().unwrap().len(), 1);
    relay.leave(&session).await;
}

#[tokio::test(start_paused = true)]
async fn failed_snapshot_confirmation_preserves_folded_files() {
    for kind in [LogKind::Update, LogKind::Snapshot] {
        let store = Memory::default();
        let doc = Doc::with_client_id(8);
        doc.get_or_insert_text("text")
            .push(&mut doc.transact_mut(), "durable");
        let file = LogFile {
            kind,
            millis: 10,
            nonce: 1,
            update: doc
                .transact()
                .encode_state_as_update_v1(&StateVector::default()),
        };
        store
            .files
            .lock()
            .unwrap()
            .insert("original".into(), file.encode());
        let relay = Relay::new(
            store.clone(),
            Limits {
                compact_after: 1,
                ..Default::default()
            },
            store.changes.clone(),
        );
        relay
            .ticket(
                [2; 32],
                "room",
                Agent {
                    subject: "writer".into(),
                    client: None,
                    origin: None,
                },
            )
            .await
            .unwrap();
        let session = relay.join([2; 32], "room").await.unwrap();
        relay.leave(&session).await;
        store.fail.store(true, Ordering::SeqCst);
        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            store.calls.load(Ordering::SeqCst) > 0,
            kind == LogKind::Update,
            "only update resources count toward the compaction threshold"
        );
        let files = store.files.lock().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            files["original"],
            file.encode(),
            "unconfirmed snapshot cannot delete the source"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn one_authors_retry_does_not_delay_other_authors() {
    let store = Memory::default();
    let relay = Relay::new(store.clone(), Limits::default(), store.changes.clone());
    for (id, subject) in [(1, "blocked"), (2, "writer")] {
        relay
            .ticket(
                [id; 32],
                "room",
                Agent {
                    subject: subject.into(),
                    client: None,
                    origin: None,
                },
            )
            .await
            .unwrap();
        let session = relay.join([id; 32], "room").await.unwrap();
        let doc = Doc::with_client_id(id as u64);
        doc.get_or_insert_text("text")
            .push(&mut doc.transact_mut(), subject);
        let update = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        relay
            .handle(
                &session,
                &Message::Sync(SyncMessage::Update(update)).encode_v1(),
            )
            .await
            .unwrap();
        if subject == "blocked" {
            tokio::task::yield_now().await;
            tokio::time::advance(std::time::Duration::from_secs(1)).await;
            tokio::task::yield_now().await;
        }
    }
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_millis(500)).await;
    tokio::task::yield_now().await;
    assert_eq!(store.files.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn flush_repersists_after_snapshot_during_create() {
    use std::time::Duration;
    let store = Memory::default();
    store.pause_create.store(true, Ordering::SeqCst);
    let relay = Relay::new(store.clone(), Limits::default(), store.changes.clone());
    let agent = Agent {
        subject: "writer".into(),
        client: None,
        origin: None,
    };
    relay.ticket([31; 32], "room", agent.clone()).await.unwrap();
    let mut session = relay.join([31; 32], "room").await.unwrap();
    let doc = Doc::with_client_id(9100);
    doc.get_or_insert_text("text")
        .push(&mut doc.transact_mut(), "batch");
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    relay
        .handle(
            &session,
            &Message::Sync(SyncMessage::Update(update)).encode_v1(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), store.entered.notified())
        .await
        .unwrap();
    let snapshot_doc = Doc::with_client_id(9101);
    snapshot_doc
        .get_or_insert_text("text")
        .push(&mut snapshot_doc.transact_mut(), "snapshot");
    let snapshot = LogFile {
        kind: LogKind::Snapshot,
        millis: u64::MAX - 1,
        nonce: 7,
        update: snapshot_doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default()),
    };
    store
        .files
        .lock()
        .unwrap()
        .insert("external-snapshot".into(), snapshot.encode());
    store.changes.created("room", "external-snapshot");
    // Observe the snapshot through the live relay before releasing the in-flight create.
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let frame = session.incoming.recv().await.unwrap().unwrap();
            if let Message::Sync(SyncMessage::Update(bytes)) = Message::decode_v1(&frame).unwrap() {
                if bytes == snapshot.update {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    store.pause_create.store(false, Ordering::SeqCst);
    store.released.notify_one();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let files: Vec<_> = store
                .files
                .lock()
                .unwrap()
                .values()
                .map(|b| LogFile::decode(b).unwrap())
                .collect();
            if files
                .iter()
                .any(|f| f.kind == LogKind::Update && f.millis >= snapshot.millis)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("batch must be persisted after the new cutoff");
    relay.leave(&session).await;
    drop(session);
    drop(relay);
    let restart = Relay::new(store.clone(), Limits::default(), store.changes.clone());
    restart.ticket([32; 32], "room", agent).await.unwrap();
    let mut peer = restart.join([32; 32], "room").await.unwrap();
    restart
        .handle(
            &peer,
            &Message::Sync(SyncMessage::SyncStep1(StateVector::default())).encode_v1(),
        )
        .await
        .unwrap();
    let restored = Doc::new();
    loop {
        let frame = peer.incoming.recv().await.unwrap().unwrap();
        if let Message::Sync(SyncMessage::SyncStep2(bytes)) = Message::decode_v1(&frame).unwrap() {
            restored
                .transact_mut()
                .apply_update(yrs::Update::decode_v1(&bytes).unwrap())
                .unwrap();
            break;
        }
    }
    let text =
        yrs::GetString::get_string(&restored.get_or_insert_text("text"), &restored.transact());
    assert!(text.contains("batch") && text.contains("snapshot"));
}

#[tokio::test(start_paused = true)]
async fn retry_reserves_retained_log_bytes() {
    let store = Memory::default();
    store.fail_after_create.store(true, Ordering::SeqCst);
    let limits = Limits {
        document_bytes: 256,
        compact_after: 100,
        ..Default::default()
    };
    let relay = Relay::new(store.clone(), limits.clone(), store.changes.clone());
    let agent = Agent {
        subject: "writer".into(),
        client: None,
        origin: None,
    };
    relay.ticket([41; 32], "room", agent.clone()).await.unwrap();
    let session = relay.join([41; 32], "room").await.unwrap();
    let doc = Doc::with_client_id(9200);
    doc.get_or_insert_text("text")
        .push(&mut doc.transact_mut(), &"x".repeat(160));
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    relay
        .handle(
            &session,
            &Message::Sync(SyncMessage::Update(update)).encode_v1(),
        )
        .await
        .unwrap();
    for _ in 0..10 {
        tokio::time::advance(std::time::Duration::from_secs(31)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }
    assert!(store.calls.load(Ordering::SeqCst) >= 3);
    let bytes: usize = store.files.lock().unwrap().values().map(Vec::len).sum();
    assert!(
        bytes <= limits.document_bytes * 4 - (limits.document_bytes + 16),
        "retries must leave room for one maximum-size snapshot"
    );
    let restart = Relay::new(store.clone(), limits, store.changes.clone());
    restart.ticket([42; 32], "room", agent).await.unwrap();
    let _peer = restart
        .join([42; 32], "room")
        .await
        .expect("retained retries must reload");
}

#[tokio::test(start_paused = true)]
async fn compaction_checks_authority_and_backs_off_until_writer_set_changes() {
    use std::time::Duration;
    let store = Memory::default();
    store.append_only.store(true, Ordering::SeqCst);
    let doc = Doc::with_client_id(9300);
    doc.get_or_insert_text("text")
        .push(&mut doc.transact_mut(), "retained");
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    store.files.lock().unwrap().insert(
        "original".into(),
        LogFile {
            kind: LogKind::Update,
            millis: 10,
            nonce: 0,
            update: update.clone(),
        }
        .encode(),
    );
    let relay = Relay::new(
        store.clone(),
        Limits {
            compact_after: 1,
            ..Default::default()
        },
        store.changes.clone(),
    );
    let writer = Agent {
        subject: "writer".into(),
        client: None,
        origin: None,
    };
    relay.ticket([51; 32], "room", writer).await.unwrap();
    let _writer = relay.join([51; 32], "room").await.unwrap();
    for _ in 0..64 {
        tokio::time::advance(Duration::from_millis(500)).await;
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }
    assert_eq!(
        store.lists.load(Ordering::SeqCst),
        1,
        "ineligible writers must not reload the log"
    );
    assert!(
        store.accesses.load(Ordering::SeqCst) <= 10,
        "unsuccessful attempts must back off"
    );
    assert!(store.files.lock().unwrap().contains_key("original"));
    relay
        .ticket(
            [52; 32],
            "room",
            Agent {
                subject: "owner".into(),
                client: None,
                origin: None,
            },
        )
        .await
        .unwrap();
    let _owner = relay.join([52; 32], "room").await.unwrap();
    for _ in 0..2 {
        tokio::time::advance(Duration::from_millis(500)).await;
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }
    let files = store.files.lock().unwrap();
    assert_eq!(files.len(), 1);
    let snapshot = LogFile::decode(files.values().next().unwrap()).unwrap();
    assert_eq!(
        snapshot.kind,
        LogKind::Snapshot,
        "a newly remembered owner resets the retry deadline"
    );
    assert_eq!(snapshot.update, update);
}

async fn concurrent_reload_reservation(by_bytes: bool) {
    use std::time::Duration;
    let store = Memory::default();
    let limits = Limits {
        compact_after: 1,
        max_log_files: if by_bytes { 100 } else { 3 },
        document_bytes: if by_bytes { 256 } else { 1024 },
        ..Default::default()
    };
    let base = Doc::with_client_id(9400);
    base.get_or_insert_text("text")
        .push(&mut base.transact_mut(), &"x".repeat(175));
    let initial = base
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    for nonce in 0..1 {
        store.files.lock().unwrap().insert(
            format!("seed-{nonce}"),
            LogFile {
                kind: LogKind::Update,
                millis: 10,
                nonce,
                update: initial.clone(),
            }
            .encode(),
        );
    }
    let relay = Relay::new(store.clone(), limits, store.changes.clone());
    relay
        .ticket(
            [61; 32],
            "room",
            Agent {
                subject: "owner".into(),
                client: None,
                origin: None,
            },
        )
        .await
        .unwrap();
    let mut owner = relay.join([61; 32], "room").await.unwrap();
    relay
        .ticket(
            [62; 32],
            "room",
            Agent {
                subject: "writer".into(),
                client: None,
                origin: None,
            },
        )
        .await
        .unwrap();
    let writer = relay.join([62; 32], "room").await.unwrap();
    store.pause_list.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(2), store.list_entered.notified())
        .await
        .unwrap();
    let editing = Doc::with_client_id(9401);
    editing
        .transact_mut()
        .apply_update(yrs::Update::decode_v1(&initial).unwrap())
        .unwrap();
    editing
        .get_or_insert_text("text")
        .push(&mut editing.transact_mut(), "a");
    let update = editing
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    relay
        .handle(
            &owner,
            &Message::Sync(SyncMessage::Update(update)).encode_v1(),
        )
        .await
        .unwrap();
    base.get_or_insert_text("text")
        .push(&mut base.transact_mut(), "b");
    let outside = base
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    store.files.lock().unwrap().insert(
        "concurrent".into(),
        LogFile {
            kind: LogKind::Update,
            millis: 20,
            nonce: 0,
            update: outside.clone(),
        }
        .encode(),
    );
    store.changes.created("room", "concurrent");
    // The live update confirms ingest has recorded the committed append during reload.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let frame = owner.incoming.recv().await.unwrap().unwrap();
            if matches!(Message::decode_v1(&frame).unwrap(), Message::Sync(SyncMessage::Update(bytes)) if bytes == outside) { break; }
        }
    }).await.unwrap();
    store.pause_create.store(true, Ordering::SeqCst);
    store.list_released.notify_one();
    // The next flush proves reload has finished; hold it while testing the reservation.
    tokio::time::timeout(Duration::from_secs(2), store.entered.notified())
        .await
        .unwrap();
    let probe = Doc::with_client_id(9402);
    probe
        .get_or_insert_text("text")
        .push(&mut probe.transact_mut(), "p");
    let update = probe
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    assert_eq!(
        relay
            .handle(
                &writer,
                &Message::Sync(SyncMessage::Update(update)).encode_v1()
            )
            .await,
        Err(Fault::Capacity),
        "reload must retain the concurrent append's reservation"
    );
    store.pause_create.store(false, Ordering::SeqCst);
    store.released.notify_one();
}

#[tokio::test(start_paused = true)]
async fn reload_retains_concurrent_append_file_count() {
    concurrent_reload_reservation(false).await;
}
#[tokio::test(start_paused = true)]
async fn reload_retains_concurrent_append_bytes() {
    concurrent_reload_reservation(true).await;
}

#[tokio::test(start_paused = true)]
async fn reload_retains_concurrent_flush_compaction_count() {
    use std::time::Duration;
    let store = Memory::default();
    // Flush must retain its confirmed file even without a self-create notification.
    store.quiet_create.store(true, Ordering::SeqCst);
    store.append_only.store(true, Ordering::SeqCst);
    let base = Doc::with_client_id(9450);
    base.get_or_insert_text("text")
        .push(&mut base.transact_mut(), "base");
    let initial = base
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    for nonce in 0..2 {
        store.files.lock().unwrap().insert(
            format!("seed-{nonce}"),
            LogFile {
                kind: LogKind::Update,
                millis: 10,
                nonce,
                update: initial.clone(),
            }
            .encode(),
        );
    }
    let relay = Relay::new(
        store.clone(),
        Limits {
            compact_after: 3,
            ..Default::default()
        },
        store.changes.clone(),
    );
    relay
        .ticket(
            [71; 32],
            "room",
            Agent {
                subject: "writer".into(),
                client: None,
                origin: None,
            },
        )
        .await
        .unwrap();
    let writer = relay.join([71; 32], "room").await.unwrap();
    store.pause_list.store(true, Ordering::SeqCst);
    // Overflow the bounded change feed to request reconciliation below the compaction threshold.
    for _ in 0..129 {
        store.changes.created("room", "seed-0");
    }
    tokio::time::timeout(Duration::from_secs(2), store.list_entered.notified())
        .await
        .unwrap();
    let local = Doc::with_client_id(9451);
    local
        .get_or_insert_text("text")
        .push(&mut local.transact_mut(), "live");
    let update = local
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    relay
        .handle(
            &writer,
            &Message::Sync(SyncMessage::Update(update)).encode_v1(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), store.committed.notified())
        .await
        .unwrap();
    assert_eq!(store.files.lock().unwrap().len(), 3);
    store.list_released.notify_one();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    relay
        .ticket(
            [72; 32],
            "room",
            Agent {
                subject: "owner".into(),
                client: None,
                origin: None,
            },
        )
        .await
        .unwrap();
    let _owner = relay.join([72; 32], "room").await.unwrap();
    for _ in 0..2 {
        tokio::time::advance(Duration::from_millis(500)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }
    let files = store.files.lock().unwrap();
    assert_eq!(
        files.len(),
        1,
        "the concurrent flush must count toward live compaction"
    );
    let snapshot = LogFile::decode(files.values().next().unwrap()).unwrap();
    assert_eq!(snapshot.kind, LogKind::Snapshot);
    let restored = Doc::new();
    restored
        .transact_mut()
        .apply_update(yrs::Update::decode_v1(&snapshot.update).unwrap())
        .unwrap();
    let text =
        yrs::GetString::get_string(&restored.get_or_insert_text("text"), &restored.transact());
    assert!(text.contains("base") && text.contains("live"));
}

#[tokio::test(start_paused = true)]
async fn compaction_rechecks_authority_after_reload() {
    use std::time::Duration;
    let store = Memory::default();
    store.files.lock().unwrap().insert(
        "original".into(),
        LogFile {
            kind: LogKind::Update,
            millis: 10,
            nonce: 0,
            update: vec![0, 0],
        }
        .encode(),
    );
    let relay = Relay::new(
        store.clone(),
        Limits {
            compact_after: 1,
            ..Default::default()
        },
        store.changes.clone(),
    );
    relay
        .ticket(
            [81; 32],
            "room",
            Agent {
                subject: "writer".into(),
                client: None,
                origin: None,
            },
        )
        .await
        .unwrap();
    let writer = relay.join([81; 32], "room").await.unwrap();
    store.pause_list.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(2), store.list_entered.notified())
        .await
        .unwrap();
    store.append_only.store(true, Ordering::SeqCst);
    store.list_released.notify_one();
    for _ in 0..2 {
        tokio::time::advance(Duration::from_millis(500)).await;
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }
    assert_eq!(
        store.calls.load(Ordering::SeqCst),
        0,
        "the refreshed authority must precede snapshot creation"
    );
    assert!(store.files.lock().unwrap().contains_key("original"));
    let doc = Doc::with_client_id(9600);
    doc.get_or_insert_text("text")
        .push(&mut doc.transact_mut(), "still writable");
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    relay
        .handle(
            &writer,
            &Message::Sync(SyncMessage::Update(update)).encode_v1(),
        )
        .await
        .unwrap();
}
