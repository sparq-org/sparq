//! Query generations with a fair barrier after optimistic attempts.

use crate::error::ServerError;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

pub(super) const OPTIMISTIC_ATTEMPTS: usize = 2;

pub(super) fn validate_barrier_budget(budget: Duration) -> Result<(), String> {
    if budget.is_zero() || budget >= super::records::LOCK_WAIT {
        return Err(
            "SOLID_SERVER_LWS_SPARQL_BARRIER_MS must be positive and less than 2000 ms".to_string(),
        );
    }
    Ok(())
}

pub(super) fn parse_barrier_budget(value: &str) -> Result<Duration, String> {
    let ms = value
        .parse::<u64>()
        .map_err(|_| "SOLID_SERVER_LWS_SPARQL_BARRIER_MS must be a positive integer".to_string())?;
    let budget = Duration::from_millis(ms);
    validate_barrier_budget(budget)?;
    Ok(budget)
}

tokio::task_local! {
    static CHANGES: Vec<Arc<Gate>>;
}

#[derive(Default)]
pub(super) struct Gate {
    admission: Arc<RwLock<()>>,
    activity: Mutex<(u64, usize)>,
}

struct Changing {
    gate: Arc<Gate>,
    _lease: Option<OwnedRwLockReadGuard<()>>,
}

impl Drop for Changing {
    fn drop(&mut self) {
        let mut activity = self.gate.activity.lock().unwrap_or_else(|e| e.into_inner());
        activity.0 = activity.0.wrapping_add(1);
        activity.1 -= 1;
    }
}

pub(crate) struct Snapshot {
    gate: Arc<Gate>,
    epoch: u64,
    _barrier: Option<OwnedRwLockWriteGuard<()>>,
}

impl Snapshot {
    pub(crate) fn unchanged(&self) -> bool {
        *self.gate.activity.lock().unwrap_or_else(|e| e.into_inner()) == (self.epoch, 0)
    }
}

impl Gate {
    pub(super) async fn change<T>(
        self: &Arc<Self>,
        work: impl Future<Output = T>,
    ) -> Result<T, ServerError> {
        let mut parents = CHANGES.try_with(Clone::clone).unwrap_or_default();
        let nested = parents.iter().any(|gate| Arc::ptr_eq(gate, self));
        // Nested calls use the task's existing lease. New top-level work uses the bounded
        // lock wait; the fair queue admits a pending query before subsequent changes.
        let lease = if nested {
            None
        } else {
            Some(
                tokio::time::timeout(
                    super::records::LOCK_WAIT,
                    self.admission.clone().read_owned(),
                )
                .await
                .map_err(|_| ServerError::ResourceChanged)?,
            )
        };
        {
            let mut activity = self.activity.lock().unwrap_or_else(|e| e.into_inner());
            activity.0 = activity.0.wrapping_add(1);
            activity.1 += 1;
        }
        let _changing = Changing {
            gate: self.clone(),
            _lease: lease,
        };
        parents.push(self.clone());
        Ok(CHANGES.scope(parents, work).await)
    }

    #[cfg(test)]
    pub(super) fn barrier_pending(&self) -> bool {
        self.admission.try_read().is_err()
    }

    pub(super) async fn snapshot(self: &Arc<Self>, barrier: bool) -> Option<Snapshot> {
        let lease = if barrier {
            Some(self.admission.clone().write_owned().await)
        } else {
            None
        };
        let (epoch, active) = *self.activity.lock().unwrap_or_else(|e| e.into_inner());
        (active == 0).then(|| Snapshot {
            gate: self.clone(),
            epoch,
            _barrier: lease,
        })
    }
}
