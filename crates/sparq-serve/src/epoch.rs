//! Per-pod epoch vectors (research/concurrent-serving.md §6.3, §6.5).
//!
//! Each [`Generation`](crate::Generation) carries a map pod-id → epoch. Publishing
//! a generation that touched pods P bumps exactly those pods' epochs. This is the
//! cache-invalidation hook for Wave B: a cached entry records the epochs of the
//! pods its query touched, and stays valid while those epochs are unchanged — so
//! unrelated writes (other pods) never churn cache keys. Pods are named graphs
//! (§6.5: "pod/named-graph granularity is the natural and honest conflict unit"),
//! and per-pod epochs are also load-bearing for horizontal scaling (§6.8: pods are
//! the shard key; the epoch vector partitions cleanly).
//!
//! Only the data structure and bump logic live here — no cache (Wave B).

use std::fmt;
use std::sync::Arc;

use rustc_hash::FxHashMap;

/// A pod's epoch: a monotonically increasing version counter, bumped each time a
/// published generation touched the pod. `0` means "never touched since the ring
/// was created". Only equality/ordering against a recorded value is meaningful.
pub type Epoch = u64;

/// Identifies a pod — in sparq-serve's world, a named graph (the graph IRI).
///
/// A cheap-to-clone interned string (`Arc<str>`): epoch maps are cloned on every
/// publish, and at Solid-fixture scale (~1.1 K graphs, §7.2) cloning a map of
/// `Arc<str>` keys on a 2–5 ms group-commit cadence (§6.5) is noise.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct PodId(Arc<str>);

impl PodId {
    /// Creates a pod id from the pod's named-graph IRI (or any stable identifier).
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        PodId(id.into())
    }

    /// The identifier this pod id was created from.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PodId {
    fn from(s: &str) -> Self {
        PodId::new(s)
    }
}

impl From<String> for PodId {
    fn from(s: String) -> Self {
        PodId::new(s)
    }
}

impl fmt::Display for PodId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The pod-id → epoch vector carried by every generation.
///
/// Immutable once published (it is reachable only through an `Arc<Generation>`);
/// the ring derives generation N+1's vector by cloning N's and bumping the touched
/// pods. Pods absent from the map implicitly have epoch `0`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PodEpochs {
    map: FxHashMap<PodId, Epoch>,
}

impl PodEpochs {
    /// The pod's current epoch (`0` if the pod has never been touched).
    pub fn epoch(&self, pod: &PodId) -> Epoch {
        self.map.get(pod).copied().unwrap_or(0)
    }

    /// Number of pods that have been touched at least once.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True if no pod has ever been touched.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Iterates over every touched pod and its epoch (arbitrary order).
    pub fn iter(&self) -> impl Iterator<Item = (&PodId, Epoch)> {
        self.map.iter().map(|(k, v)| (k, *v))
    }

    /// Bumps the pod's epoch by one, returning the new value. Crate-private: the
    /// only legal mutation site is [`GenerationRing::publish`](crate::GenerationRing::publish),
    /// which bumps the *next* generation's freshly cloned vector — published
    /// vectors are never mutated.
    pub(crate) fn bump(&mut self, pod: PodId) -> Epoch {
        let e = self.map.entry(pod).or_insert(0);
        *e += 1;
        *e
    }

    /// (sq-o5bi) Sets a pod's epoch to an exact value while RECONSTRUCTING the
    /// vector from a backup artifact (`crate::backup`). Crate-private: the only legal caller is
    /// the importer rebuilding the recorded epoch vector before it seeds the restored ring —
    /// published vectors are still never mutated in normal operation. Compiled when EITHER the
    /// `backup` or the `change-stream` feature is on (sq-b4fns: the change stream pulls the
    /// `backup` module privately for its shared `BackupError`/digest).
    #[cfg(any(feature = "backup", feature = "change-stream"))]
    #[allow(dead_code)] // unused in a change-stream-only build (the importer is backup-only)
    pub(crate) fn set(&mut self, pod: PodId, epoch: Epoch) {
        self.map.insert(pod, epoch);
    }
}
