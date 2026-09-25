// [GPT-6] Optional local diagnostics, never part of a proof presentation.
use serde::Serialize;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

// Bound retention independently of how long an opted-in driver remains alive.
const MAX_EVENTS: usize = 4096;

/// An observed driver operation, excluding surrounding API work.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DriverStage {
    /// Acquire the workspace lock, including lock-file preparation and contention.
    NargoCacheLock,
    /// Run `nargo compile`; compiler-internal cache hits are not observable.
    NargoCompile,
    /// Read, verify or publish an immutable content-addressed ACIR snapshot.
    AcirSnapshot,
    /// Write the private per-job ACIR copy, excluding the preceding ACIR read.
    AcirPrivateCopy,
    /// Run `nargo execute` and check that it produced a witness.
    NargoExecute,
    /// Run `bb prove --write_vk`, including both proof and key generation.
    BbProveAndWriteVk,
    /// Run independent `bb write_vk` during canonical key reconstruction.
    BbWriteVk,
    /// Run `bb verify`, excluding publication of its input files.
    BbVerify,
}

/// The observed stage outcome, without input data or subprocess diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DriverStageOutcome {
    /// The operation completed according to the driver's existing contract.
    Success,
    /// The verification subprocess returned a nonzero status.
    ///
    /// This alone does not distinguish invalid proofs from backend failures.
    Rejected,
    /// A spawn, I/O or operation error prevented completion.
    Failed,
}

/// An observed immutable-cache operation, distinct from compiler-internal caching.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AcirCacheObservation {
    /// Existing content was read and verified byte-for-byte before reuse.
    ReusedVerifiedSnapshot,
    /// A new immutable snapshot was published.
    PublishedSnapshot,
}

/// One local wall-clock measurement, carrying no witness data or file paths.
#[derive(Clone, Debug, Serialize)]
pub struct DriverStageEvent {
    pub stage: DriverStage,
    pub outcome: DriverStageOutcome,
    pub elapsed_seconds: f64,
    /// None means no immutable-cache decision was observed in this stage.
    pub acir_cache: Option<AcirCacheObservation>,
}

/// Bounded local stage diagnostics, separate from every cryptographic statement.
///
/// Events appear in completion order; concurrent callers may interleave them.
/// Use a separate driver per experiment if attribution to one call is required.
/// Timing can itself disclose workload information. Retain these diagnostics
/// privately unless publishing synthetic experiments or explicitly permitted data.
/// `complete` is false after retention overflow or internal collector poisoning.
#[derive(Debug, Default, Serialize)]
pub struct DriverStageMetrics {
    pub events: Vec<DriverStageEvent>,
    pub dropped_events: u64,
    pub complete: bool,
}

#[derive(Default)]
pub(super) struct Collector(Mutex<DriverStageMetrics>);
impl Collector {
    fn lock(&self) -> MutexGuard<'_, DriverStageMetrics> {
        match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub(super) fn take(&self) -> DriverStageMetrics {
        let mut metrics = self.lock();
        let mut taken = std::mem::take(&mut *metrics);
        taken.complete = taken.dropped_events == 0 && !self.0.is_poisoned();
        taken
    }

    fn record(&self, event: DriverStageEvent) {
        let mut metrics = self.lock();
        if metrics.events.len() < MAX_EVENTS {
            metrics.events.push(event);
        } else {
            metrics.dropped_events = metrics.dropped_events.saturating_add(1);
        }
    }
}

pub(super) struct StageTimer<'a> {
    running: Option<(&'a Collector, Instant, DriverStage)>,
}
impl<'a> StageTimer<'a> {
    pub(super) fn start(collector: Option<&'a Collector>, stage: DriverStage) -> Self {
        Self {
            running: collector.map(|collector| (collector, Instant::now(), stage)),
        }
    }

    pub(super) fn finish(
        mut self,
        outcome: DriverStageOutcome,
        acir_cache: Option<AcirCacheObservation>,
    ) {
        self.record(outcome, acir_cache);
    }

    fn record(&mut self, outcome: DriverStageOutcome, acir_cache: Option<AcirCacheObservation>) {
        if let Some((collector, start, stage)) = self.running.take() {
            let elapsed_seconds = start.elapsed().as_secs_f64();
            collector.record(DriverStageEvent {
                stage,
                outcome,
                elapsed_seconds,
                acir_cache,
            });
        }
    }
}
impl Drop for StageTimer<'_> {
    fn drop(&mut self) {
        // Early `?` returns and unwinds remain failed events, never successes.
        self.record(DriverStageOutcome::Failed, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_collection_is_bounded_and_drain_is_explicit() {
        let collector = Collector::default();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let collector = &collector;
                scope.spawn(move || {
                    for _ in 0..MAX_EVENTS / 8 + 1 {
                        StageTimer::start(Some(collector), DriverStage::NargoCompile)
                            .finish(DriverStageOutcome::Success, None);
                    }
                });
            }
        });
        let metrics = collector.take();
        assert_eq!(metrics.events.len(), MAX_EVENTS);
        assert_eq!(metrics.dropped_events, 8);
        assert!(!metrics.complete);
        assert!(metrics.events.iter().all(|e| e.elapsed_seconds.is_finite()
            && e.outcome == DriverStageOutcome::Success
            && e.acir_cache.is_none()));
        let empty = collector.take();
        assert!(empty.complete && empty.events.is_empty());
    }

    #[test]
    fn early_error_is_failed_and_collectors_are_independent() {
        let first = Collector::default();
        let second = Collector::default();
        drop(StageTimer::start(Some(&first), DriverStage::NargoExecute));
        assert_eq!(first.take().events[0].outcome, DriverStageOutcome::Failed);
        assert!(second.take().events.is_empty());
        StageTimer::start(None, DriverStage::BbVerify).finish(DriverStageOutcome::Rejected, None);
        assert!(first.take().events.is_empty());
    }
}
