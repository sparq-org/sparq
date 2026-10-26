use super::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use yrs::sync::{Message, SyncMessage};
use yrs::updates::encoder::Encode;
use yrs::{Doc, ReadTxn, StateVector, Text, Transact};

#[derive(Clone, Default)]
struct Memory {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    fail: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    changes: Changes,
}
#[async_trait]
impl Authorizer for Memory {
    async fn access(&self, _: &str, _: &Agent) -> Result<Access, Fault> {
        Ok(Access {
            read: true,
            write: true,
        })
    }
}
#[async_trait]
impl RoomStore for Memory {
    async fn list(&self, _: &str) -> Result<Vec<String>, Fault> {
        Ok(self.files.lock().unwrap().keys().cloned().collect())
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) || agent.subject == "blocked" {
            return Err(Fault::Store);
        }
        let name = format!("{}-{}", file.millis, file.nonce);
        self.files
            .lock()
            .unwrap()
            .insert(name.clone(), file.encode());
        self.changes.created("room", &name);
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
