// Bounded in-memory storage configuration and observability.

/// Default aggregate body-byte ceiling for the in-memory Solid store (64 MiB).
pub const DEFAULT_IN_MEMORY_MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;

/// Default stored-entry ceiling for each in-memory storage map.
pub const DEFAULT_IN_MEMORY_MAX_RESOURCE_COUNT: usize = 4_096;

/// The path, under a storage's root, of the service container whose records the pool holds: the
/// LWS notification subscriptions.
///
/// Spelled here rather than taken from `lws::SUBSCRIPTIONS_PATH`, because the `lws` module is
/// not built for `wasm32` while this one is. `lws::notify`'s
/// `the_store_pools_what_the_notification_service_bounds` holds the two to each other wherever
/// both are built.
pub const SERVICE_POOL_PATH: &str = "/.lws/subscriptions/";

/// Default stored-entry ceiling for the service pool ([`in_service_pool`]).
///
/// The pool is capacity of its own, apart from the content ceilings: records in it are admitted
/// against this and take nothing from [`DEFAULT_IN_MEMORY_MAX_RESOURCE_COUNT`], so however many
/// stand, a write of content is admitted exactly as it would be with none of them. It holds what
/// the notification service admits (`lws::notify::MAX_SUBSCRIPTIONS` from subscribers other than
/// the storage owner and `MAX_OWNER_SUBSCRIPTIONS` from the owner, 512 each), with room for the
/// service container and for records a start holds back or quarantines.
pub const DEFAULT_IN_MEMORY_MAX_POOL_RESOURCE_COUNT: usize = 512 + 512 + 256;

/// Default aggregate body-byte ceiling for the service pool: every slot in it holding a record
/// as large as a subscription request may be (`lws::notify::MAX_SUBSCRIPTION_BYTES`, 16 KiB).
pub const DEFAULT_IN_MEMORY_MAX_POOL_BYTES: usize =
    DEFAULT_IN_MEMORY_MAX_POOL_RESOURCE_COUNT * 16 * 1024;

/// Whether the record named by `iri` belongs to the service pool: a notification subscription,
/// or the service container that holds them.
///
/// These are bounded by the notification service itself, so the store bounds them apart from
/// stored content rather than letting them take its room. `/.lws/` is reserved, so nothing a
/// client may name as content falls in here.
pub fn in_service_pool(iri: &str) -> bool {
    iri.contains(SERVICE_POOL_PATH)
}

/// The mark [`crate::store::CompositeStore`] puts at the front of the blob key it mints for a
/// service-pool record, so the blob store can tell one without being told what an IRI means. A
/// key minted from an absolute IRI never begins with it.
pub const POOLED_BLOB_KEY_MARK: &str = "$pool$";

/// Whether `key` is the blob key of a service-pool record ([`POOLED_BLOB_KEY_MARK`]).
pub fn is_pooled_blob_key(key: &str) -> bool {
    key.starts_with(POOLED_BLOB_KEY_MARK)
}

/// Hard admission limits for the in-memory metadata and blob stores.
///
/// The byte ceiling applies to bytes physically retained by [`super::InMemoryBlobStore`], including
/// unreferenced versions awaiting reconciliation. The count ceiling is enforced independently by
/// both in-memory maps, so zero-byte writes and metadata-only growth are bounded too. A zero value
/// deliberately admits no new entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InMemoryStoreLimits {
    /// Maximum aggregate bytes of stored content physically retained by the blob store.
    pub max_total_bytes: usize,
    /// Maximum stored-content entries retained by either the blob or metadata store.
    pub max_resource_count: usize,
    /// Maximum entries in the service pool ([`in_service_pool`]), counted apart from
    /// [`InMemoryStoreLimits::max_resource_count`].
    pub max_pool_resource_count: usize,
    /// Maximum aggregate bytes of service-pool entries, counted apart from
    /// [`InMemoryStoreLimits::max_total_bytes`].
    pub max_pool_bytes: usize,
}

impl InMemoryStoreLimits {
    /// Construct explicit content byte and entry ceilings, with the default service pool.
    pub const fn new(max_total_bytes: usize, max_resource_count: usize) -> Self {
        Self {
            max_total_bytes,
            max_resource_count,
            max_pool_resource_count: DEFAULT_IN_MEMORY_MAX_POOL_RESOURCE_COUNT,
            max_pool_bytes: DEFAULT_IN_MEMORY_MAX_POOL_BYTES,
        }
    }

    /// The same content ceilings with an explicit service pool.
    pub const fn with_pool(self, max_pool_resource_count: usize, max_pool_bytes: usize) -> Self {
        Self {
            max_pool_resource_count,
            max_pool_bytes,
            ..self
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Naming a pool leaves the content ceilings alone, and the other way round.
    #[test]
    fn a_named_pool_leaves_the_content_ceilings_alone() {
        let narrowed = InMemoryStoreLimits::new(64, 8).with_pool(3, 99);
        assert_eq!(
            (narrowed.max_total_bytes, narrowed.max_resource_count),
            (64, 8)
        );
        assert_eq!(
            (narrowed.max_pool_resource_count, narrowed.max_pool_bytes),
            (3, 99)
        );
        let default = InMemoryStoreLimits::default();
        assert_eq!(
            default.max_pool_resource_count,
            DEFAULT_IN_MEMORY_MAX_POOL_RESOURCE_COUNT
        );
    }

    /// Subscription records are in the pool, and so is their service container; stored content
    /// is not, and a blob key is told by the mark its minting puts on it.
    #[test]
    fn the_pool_holds_the_subscription_records_and_nothing_else() {
        let root = "http://localhost:3000/";
        assert!(in_service_pool(&format!("{}.lws/subscriptions/", root)));
        assert!(in_service_pool(&format!("{}.lws/subscriptions/abc", root)));
        assert!(!in_service_pool(&format!("{}.lws/grants/abc", root)));
        assert!(!in_service_pool(&format!("{}doc.txt", root)));
        assert!(is_pooled_blob_key(&format!(
            "{}http___localhost_3000_.lws_subscriptions_abc-00",
            POOLED_BLOB_KEY_MARK
        )));
        assert!(!is_pooled_blob_key("http___localhost_3000_doc.txt-00"));
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
