// Bounded in-memory storage configuration and observability.

/// Default aggregate body-byte ceiling for the in-memory Solid store (64 MiB).
pub const DEFAULT_IN_MEMORY_MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;

/// Default stored-entry ceiling for each in-memory storage map.
pub const DEFAULT_IN_MEMORY_MAX_RESOURCE_COUNT: usize = 4_096;

/// How long one call to a remote store may take before it is abandoned: the SPARQL-over-HTTP
/// client's per-request timeout (`http::HttpSparqClient`), and the longest a background
/// settlement waits for one attempt at a fence before trying again.
pub const STORE_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The path, under a storage's root, of the service container whose records the pool holds: the
/// LWS notification subscriptions.
///
/// Spelled here rather than taken from `lws::SUBSCRIPTIONS_PATH`, because the `lws` module is
/// not built for `wasm32` while this one is. `lws::notify`'s
/// `the_store_pools_what_the_notification_service_bounds` holds the two to each other wherever
/// both are built.
pub const SERVICE_POOL_PATH: &str = "/.lws/subscriptions/";

/// The fragment, on the service pool's container ([`SERVICE_POOL_PATH`]), of the one marker the
/// notification service keeps there ([`is_service_marker`]).
pub const SERVICE_MARKER_FRAGMENT: &str = "#high-water";

/// Whether `iri` has the shape of the notification service's marker: an `http` or `https`
/// origin (a host that is a registered name or a bracketed IPv6 address, no user information,
/// and a port, if given, of digits alone) followed by exactly the service pool's container and
/// [`SERVICE_MARKER_FRAGMENT`]. A storage has one marker, and its index entry has a slot of its
/// own, admitted apart from every ceiling, so storing it never waits on room (no request can name
/// it: it is a fragment). The in-memory index keeps that slot for one IRI alone, the first new
/// marker stored while it is free; any other of this shape is admitted as an ordinary pool
/// record, and stays one.
pub fn is_service_marker(iri: &str) -> bool {
    let Some(rest) = iri
        .strip_prefix("https://")
        .or_else(|| iri.strip_prefix("http://"))
    else {
        return false;
    };
    let Some(at) = rest.find('/') else {
        return false;
    };
    let (authority, path) = rest.split_at(at);
    // An IPv6 literal is bracketed, and its colons are not a port's; any other host is a
    // registered name (RFC 3986 `reg-name`, which an IPv4 address also is).
    let (host_ok, port) = match authority.strip_prefix('[') {
        Some(inner) => {
            let Some((literal, after)) = inner.split_once(']') else {
                return false;
            };
            let port = match after {
                "" => None,
                after => match after.strip_prefix(':') {
                    Some(port) => Some(port),
                    None => return false,
                },
            };
            (literal.parse::<std::net::Ipv6Addr>().is_ok(), port)
        }
        None => {
            let (host, port) = match authority.split_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            };
            (is_reg_name(host), port)
        }
    };
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    host_ok && port_ok && path.strip_prefix(SERVICE_POOL_PATH) == Some(SERVICE_MARKER_FRAGMENT)
}

/// Whether `host` is a non-empty RFC 3986 `reg-name`: unreserved characters, percent-encoded
/// octets and sub-delimiters only.
fn is_reg_name(host: &str) -> bool {
    let bytes = host.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'%' => {
                let hex = bytes.get(at + 1..at + 3);
                if !hex.is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit)) {
                    return false;
                }
                at += 3;
                continue;
            }
            b if b.is_ascii_alphanumeric() => {}
            b'-' | b'.' | b'_' | b'~' => {}
            b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' => {}
            _ => return false,
        }
        at += 1;
    }
    !host.is_empty()
}

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
/// stored content rather than letting them take its room. Only an IRI whose path, right after
/// its origin, begins with the service's own path is in it: `/.lws/` directly below the storage
/// root is reserved, so nothing a client may name as content falls in here (the same segment
/// anywhere deeper is an ordinary name, and stays content).
pub fn in_service_pool(iri: &str) -> bool {
    iri.split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|at| &rest[at..]))
        .is_some_and(|path| path.starts_with(SERVICE_POOL_PATH))
}

/// Whether the record named by `iri` is a hold's ([`crate::store::Store::hold`]): kept apart from
/// every ceiling, so how many a change holds (a fixed number per resource it changes, whatever
/// their content) never takes room from content or refuses a write. Holds are bounded by what
/// each change may stage and by how many changes run at once.
pub fn is_hold_record(iri: &str) -> bool {
    iri.starts_with(crate::store::HELD_CONTAINER)
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
        // Content that only names the same segments is content.
        for content in [
            format!("{}alice/.lws/subscriptions/x", root),
            format!("{}alice/x/.lws/subscriptions/", root),
            format!("{}%2Elws/subscriptions/x", root),
            format!("{}.lws/subscriptionsx", root),
        ] {
            assert!(!in_service_pool(&content), "{content}");
        }
        assert!(!in_service_pool(&format!("{}.lws/grants/abc", root)));
        assert!(!in_service_pool(&format!("{}doc.txt", root)));
        assert!(is_pooled_blob_key(&format!(
            "{}b1-00000000000000000000000000000000",
            POOLED_BLOB_KEY_MARK
        )));
        assert!(!is_pooled_blob_key("b1-00000000000000000000000000000000"));
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
