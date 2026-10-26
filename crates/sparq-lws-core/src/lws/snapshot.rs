//! A storage-wide read generation, shared by resource locks and tracked store writes.
//!
//! Mutations may nest (a transaction takes several IRI locks and calls the store). A queued
//! reader must therefore let an existing mutation acquire another lease: writer-preferring
//! recursive RwLock acquisition would deadlock. Queries wait for all mutations to finish;
//! while any query holds a read generation, no mutation may start. The request deadline bounds
//! waiting under continuous writes. Owned leases follow detached writes and recovery tasks.

use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Default)]
pub(crate) struct Snapshot {
    counts: Mutex<(usize, usize)>, // readers, mutations
    changed: Notify,
}

pub(crate) struct Lease {
    snapshot: Arc<Snapshot>,
    reading: bool,
}

impl Snapshot {
    fn try_enter(self: &Arc<Self>, reading: bool) -> Option<Lease> {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        if (reading && counts.1 != 0) || (!reading && counts.0 != 0) {
            return None;
        }
        if reading {
            counts.0 += 1;
        } else {
            counts.1 += 1;
        }
        Some(Lease {
            snapshot: self.clone(),
            reading,
        })
    }

    async fn enter(self: &Arc<Self>, reading: bool) -> Lease {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(lease) = self.try_enter(reading) {
                return lease;
            }
            changed.await;
        }
    }

    pub(crate) async fn read(self: &Arc<Self>) -> Lease {
        self.enter(true).await
    }
    pub(crate) async fn mutate(self: &Arc<Self>) -> Lease {
        self.enter(false).await
    }
    pub(crate) fn try_mutate(self: &Arc<Self>) -> Option<Lease> {
        self.try_enter(false)
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut counts = self
            .snapshot
            .counts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.reading {
            counts.0 -= 1;
        } else {
            counts.1 -= 1;
        }
        drop(counts);
        self.snapshot.changed.notify_waiters();
    }
}
