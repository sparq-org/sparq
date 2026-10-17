//! The storage lease: one process at a time serves a storage. A start takes its storage's lease
//! before it does anything else (recovery included) and refuses to start when another process's
//! stands; while it serves, it renews the lease. A process that cannot show it still holds the
//! lease stops serving: every request is answered `503`, for good, until it is started again,
//! since another process may have served the storage meanwhile and what this one keeps in memory
//! (grants, subscriptions, what it has set aside) may no longer be what is stored.
//!
//! The lease is a record in the index ([`Store::take_lease`]), taken and renewed in one atomic
//! step; a store whose index lives in one process grants it, as there is nobody to exclude, and
//! an index opened from a directory is locked to its process when it is opened.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use super::LwsState;
use crate::store::Store;

/// How long a lease stands once taken or renewed.
pub(crate) const TERM: Duration = Duration::from_secs(30);

/// How often a lease is renewed while its holder serves.
pub(crate) const RENEW_EVERY: Duration = Duration::from_secs(5);

/// How long before its lease would run out a holder gives up: it stops serving once it has not
/// renewed for `TERM - MARGIN`, so it has stopped before another process can take the lease even
/// with their clocks this far apart.
pub(crate) const MARGIN: Duration = Duration::from_secs(10);

/// This process's hold on its storage's lease.
pub(crate) struct Lease {
    holder: String,
    renewed: Mutex<tokio::time::Instant>,
    lost: AtomicBool,
}

impl Lease {
    /// Take the lease of the storage `key` as a new holder: `Err` when another process's stands,
    /// or the store cannot say.
    pub(crate) async fn take<S: Store>(store: &S, key: &str) -> Result<Self, String> {
        let holder = super::jose::random_id();
        let at = tokio::time::Instant::now();
        let now = super::jose::now_secs();
        match store.take_lease(key, &holder, until(now), now).await {
            Ok(true) => Ok(Self {
                holder,
                renewed: Mutex::new(at),
                lost: AtomicBool::new(false),
            }),
            Ok(false) => Err(format!(
                "the storage {key} is served by another process (its lease stands)"
            )),
            Err(e) => Err(format!(
                "the lease of the storage {key} cannot be taken: {e}"
            )),
        }
    }

    /// Whether this process may no longer serve its storage.
    pub(crate) fn lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    fn lose(&self, key: &str, why: &str) {
        if !self.lost.swap(true, Ordering::AcqRel) {
            eprintln!("lws: no longer serving {key}: {why}");
        }
    }

    /// Renew the lease once: lost, for good, when another holder's stands, or when it has not
    /// been renewed for `TERM - MARGIN` (whether or not the store answers now).
    async fn renew<S: Store>(&self, store: &S, key: &str) {
        if self.lost() {
            return;
        }
        let since = self
            .renewed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed();
        if since >= TERM - MARGIN {
            return self.lose(key, "its lease was not renewed in time");
        }
        let at = tokio::time::Instant::now();
        let now = super::jose::now_secs();
        match store.take_lease(key, &self.holder, until(now), now).await {
            Ok(true) => *self.renewed.lock().unwrap_or_else(|e| e.into_inner()) = at,
            Ok(false) => self.lose(key, "another process holds its lease"),
            Err(e) => {
                let since = self
                    .renewed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .elapsed();
                if since >= TERM - MARGIN {
                    self.lose(key, &format!("its lease could not be renewed: {e}"));
                }
            }
        }
    }
}

fn until(now: i64) -> i64 {
    now.saturating_add(TERM.as_secs() as i64)
}

/// Renew `state`'s lease every [`RENEW_EVERY`] while it serves; the task goes with the state's
/// last owner's handle (as the expiry sweeper does).
pub(crate) fn spawn_keeper<S: Store + 'static>(
    state: &LwsState<S>,
    mut closed: tokio::sync::watch::Receiver<()>,
) {
    let weak = state.downgrade();
    tokio::spawn(async move {
        let start = tokio::time::Instant::now() + RENEW_EVERY;
        let mut ticks = tokio::time::interval_at(start, RENEW_EVERY);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = closed.changed() => return,
                _ = ticks.tick() => {}
            }
            let Some(state) = weak.upgrade() else {
                return;
            };
            let key = state.cfg.storage();
            // A store that does not answer is not waited on past the next renewal: by then it
            // counts as not renewed.
            let _ = tokio::time::timeout(RENEW_EVERY, state.lease.renew(&state.store, &key)).await;
            if state.lease.lost() {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::super::test_store::{request, state, FlakyStore};
    use super::super::{route, LwsConfig, LwsState};
    use super::*;
    use crate::store::Store;
    use axum::http::{Method, StatusCode};

    /// A start while another process's lease stands changes nothing: no root, no recovery.
    #[tokio::test]
    async fn a_start_without_the_lease_changes_nothing() {
        let store = FlakyStore::new();
        store.refuse_lease.store(true, Ordering::SeqCst);
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        let refused = LwsState::new(store.clone(), cfg.clone())
            .await
            .err()
            .expect("refused");
        assert!(refused.contains("another process"), "{refused}");
        assert!(
            !store.exists(&cfg.storage()).await.unwrap(),
            "the root was written"
        );
        // Nor does a store that cannot say who holds it.
        store.refuse_lease.store(false, Ordering::SeqCst);
        store.fail_lease.store(true, Ordering::SeqCst);
        assert!(LwsState::new(store.clone(), cfg.clone()).await.is_err());
        assert!(!store.exists(&cfg.storage()).await.unwrap());
        store.fail_lease.store(false, Ordering::SeqCst);
        LwsState::new(store, cfg).await.expect("starts");
    }

    async fn status(state: &LwsState<FlakyStore>) -> StatusCode {
        route(state, request(Method::GET, "/", &[], ""))
            .await
            .status()
    }

    /// A lease another holder takes is lost for good: every request is answered `503`, even once
    /// the lease could be taken again.
    #[tokio::test(start_paused = true)]
    async fn a_lease_taken_by_another_stops_serving_for_good() {
        let (st, store) = state(4).await;
        assert_eq!(status(&st).await, StatusCode::OK);
        store.refuse_lease.store(true, Ordering::SeqCst);
        tokio::time::sleep(RENEW_EVERY + Duration::from_secs(1)).await;
        assert!(st.lease.lost());
        let r = route(&st, request(Method::GET, "/", &[], "")).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(r.headers().contains_key(axum::http::header::RETRY_AFTER));
        store.refuse_lease.store(false, Ordering::SeqCst);
        tokio::time::sleep(RENEW_EVERY * 3).await;
        assert_eq!(status(&st).await, StatusCode::SERVICE_UNAVAILABLE);
    }

    /// A store that fails to renew the lease is waited on until the lease is near its end, not
    /// past it: before then the server goes on serving, and renews once the store answers.
    #[tokio::test(start_paused = true)]
    async fn a_lease_not_renewed_in_time_stops_serving() {
        let (st, store) = state(4).await;
        store.fail_lease.store(true, Ordering::SeqCst);
        tokio::time::sleep(RENEW_EVERY + Duration::from_secs(1)).await;
        assert_eq!(status(&st).await, StatusCode::OK, "one renewal failed");
        store.fail_lease.store(false, Ordering::SeqCst);
        tokio::time::sleep(RENEW_EVERY).await;
        assert_eq!(status(&st).await, StatusCode::OK, "renewed");
        store.fail_lease.store(true, Ordering::SeqCst);
        tokio::time::sleep(TERM).await;
        assert_eq!(status(&st).await, StatusCode::SERVICE_UNAVAILABLE);
    }
}
