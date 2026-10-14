// Bounded in-memory storage configuration and observability.

/// Default aggregate body-byte ceiling for the in-memory Solid store (64 MiB).
pub const DEFAULT_IN_MEMORY_MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;

/// Default stored-entry ceiling for each in-memory storage map.
pub const DEFAULT_IN_MEMORY_MAX_RESOURCE_COUNT: usize = 4_096;

/// Hard admission limits for the in-memory metadata and blob stores.
///
/// The byte ceiling applies to bytes physically retained by [`super::InMemoryBlobStore`], including
/// unreferenced versions awaiting reconciliation. The count ceiling is enforced independently by
/// both in-memory maps, so zero-byte writes and metadata-only growth are bounded too. A zero value
/// deliberately admits no new entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InMemoryStoreLimits {
    /// Maximum aggregate bytes physically retained by the blob store.
    pub max_total_bytes: usize,
    /// Maximum entries retained by either the blob or metadata store.
    pub max_resource_count: usize,
}

impl InMemoryStoreLimits {
    /// Construct explicit byte and entry ceilings.
    pub const fn new(max_total_bytes: usize, max_resource_count: usize) -> Self {
        Self {
            max_total_bytes,
            max_resource_count,
        }
    }
}

impl Default for InMemoryStoreLimits {
    fn default() -> Self {
        Self::new(
            DEFAULT_IN_MEMORY_MAX_TOTAL_BYTES,
            DEFAULT_IN_MEMORY_MAX_RESOURCE_COUNT,
        )
    }
}

/// A point-in-time in-memory store usage view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreUsage {
    /// Aggregate bytes physically retained by the blob store.
    pub total_bytes: usize,
    /// Occupied entry slots at the fuller of the blob and metadata maps.
    ///
    /// This includes unreferenced blob versions awaiting reconciliation, because they consume the
    /// same bounded capacity as live resources.
    pub resource_count: usize,
}
