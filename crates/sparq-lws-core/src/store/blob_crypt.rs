//! Native envelope encryption and index-backed wrapped keys.
//!
//! This library stage has no environment or data-directory format wiring. A single
//! decorator owns reservations for one exclusive index. See the server skill for
//! the integration boundary and the crypt crate README for the envelope format.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use sparq_lws_crypt::{
    is_envelope, open, parse_header, seal, Dek, KeyContext, KeyProvider, WrappedKey,
};
use tokio::sync::Mutex;

use super::{
    blob::BlobScope, limits::DATA_KEY_CONTAINER, BlobEntry, BlobError, BlobScopePolicy, BlobStore,
    CompositeStore, ResourceMeta, SparqClient, SparqError,
};

const BLOCK: u64 = 1 << 16;
const ROTATE_AT: u64 = 1 << 31;
const RECORD_TYPE: &str = "application/x-sparq-lws-data-key+json";

/// Default lifetime of an unwrapped key, including the current write key.
pub const DEFAULT_DEK_TTL: Duration = Duration::from_secs(600);

/// Protocol label authenticated by key wrapping and stored with every DEK.
#[derive(Debug, Clone, Copy)]
pub enum CryptScheme {
    /// Linked Web Storage: the storage root is the pod identity.
    Lws,
    /// Solid: the first path segment is the pod identity.
    Solid,
}

impl CryptScheme {
    fn as_str(self) -> &'static str {
        match self {
            Self::Lws => "lws",
            Self::Solid => "solid",
        }
    }
}

/// Monotonic clock used for key expiration. Implementations must not go backwards.
pub trait CryptClock: Send + Sync {
    /// Elapsed time from an arbitrary, stable epoch.
    fn now(&self) -> Duration;
}

struct Clock(Instant);
impl CryptClock for Clock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// Key cache configuration. The seal reservation and rotation limits are fixed.
pub struct CryptOptions {
    /// Lifetime of unwrapped DEKs. Expiry re-unwraps, never regenerates a current key.
    pub ttl: Duration,
    /// Monotonic time source; tests can advance it without sleeping.
    pub clock: Arc<dyn CryptClock>,
    block: u64,
    rotate_at: u64,
}

impl Default for CryptOptions {
    fn default() -> Self {
        Self {
            ttl: DEFAULT_DEK_TTL,
            clock: Arc::new(Clock(Instant::now())),
            block: BLOCK,
            rotate_at: ROTATE_AT,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyRecord {
    dek_id: String,
    wrapped: Vec<u8>,
    pod: String,
    scheme: String,
    reserved_seals: u64,
    created: u64,
    current: bool,
}

impl KeyRecord {
    fn context(&self) -> KeyContext {
        KeyContext {
            pod: self.pod.clone(),
            scheme: self.scheme.clone(),
        }
    }

    fn id(&self) -> Result<[u8; 16], BlobError> {
        let mut id = [0; 16];
        if self.dek_id.len() != 32
            || !self
                .dek_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid_record());
        }
        for (i, byte) in id.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&self.dek_id[i * 2..i * 2 + 2], 16)
                .map_err(|_| invalid_record())?;
        }
        Ok(id)
    }

    fn meta(&self) -> Result<ResourceMeta, BlobError> {
        Ok(ResourceMeta {
            content_type: RECORD_TYPE.into(),
            blob_key: String::new(),
            etag: serde_json::to_string(self).map_err(|_| invalid_record())?,
            last_modified: None,
        })
    }
}

fn invalid_record() -> BlobError {
    BlobError::Backend("invalid data-key record".into())
}
fn crypt_error(error: sparq_lws_crypt::Error) -> BlobError {
    BlobError::Backend(error.to_string())
}
fn index_error(_: SparqError) -> BlobError {
    BlobError::Backend("data-key index operation failed".into())
}
fn hex(id: &[u8; 16]) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}
fn record_iri(id: &[u8; 16]) -> String {
    format!("{DATA_KEY_CONTAINER}{}", hex(id))
}

/// Wrapped-key records live only in the authoritative index. Their blob pointer
/// is empty; no key object is ever published to, listed by, or GC'd from the byte plane.
struct KeyRecords {
    index: Arc<dyn SparqClient>,
}

impl KeyRecords {
    async fn load(&self, id: &[u8; 16]) -> Result<KeyRecord, BlobError> {
        let meta = self
            .index
            .get_meta(&record_iri(id))
            .await
            .map_err(|e| match e {
                SparqError::NotFound => BlobError::UnknownDataKey,
                other => index_error(other),
            })?;
        if meta.content_type != RECORD_TYPE || !meta.blob_key.is_empty() {
            return Err(invalid_record());
        }
        let record: KeyRecord = serde_json::from_str(&meta.etag).map_err(|_| invalid_record())?;
        if record.id()? != *id
            || record.reserved_seals > ROTATE_AT
            || !matches!(record.scheme.as_str(), "lws" | "solid")
            || record.pod.is_empty()
        {
            return Err(invalid_record());
        }
        WrappedKey::from_bytes(&record.wrapped).map_err(crypt_error)?;
        Ok(record)
    }

    async fn all(&self) -> Result<Vec<KeyRecord>, BlobError> {
        let children = match self.index.list_children(DATA_KEY_CONTAINER).await {
            Ok(children) => children,
            Err(SparqError::NotFound) => Vec::new(),
            Err(e) => return Err(index_error(e)),
        };
        let mut records = Vec::new();
        for iri in children {
            let suffix = iri
                .strip_prefix(DATA_KEY_CONTAINER)
                .ok_or_else(invalid_record)?;
            let dummy = KeyRecord {
                dek_id: suffix.into(),
                wrapped: Vec::new(),
                pod: String::new(),
                scheme: String::new(),
                reserved_seals: 0,
                created: 0,
                current: false,
            };
            records.push(self.load(&dummy.id()?).await?);
        }
        Ok(records)
    }

    async fn save(&self, record: &KeyRecord) -> Result<(), BlobError> {
        self.index
            .create_child(
                DATA_KEY_CONTAINER,
                &record_iri(&record.id()?),
                record.meta()?,
            )
            .await
            .map_err(index_error)
    }
}

struct CachedDek {
    dek: Dek,
    record: KeyRecord,
    expires: Duration,
}

struct SlotState<T> {
    value: Option<T>,
    failure: Option<BlobError>,
}

struct Slot<T> {
    state: Mutex<SlotState<T>>,
    failure_epoch: AtomicU64,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(SlotState {
                value: None,
                failure: None,
            }),
            failure_epoch: AtomicU64::new(0),
        }
    }
}

impl<T> Slot<T> {
    fn observed(&self) -> u64 {
        self.failure_epoch.load(Ordering::Acquire)
    }

    async fn lock(&self) -> tokio::sync::MutexGuard<'_, SlotState<T>> {
        self.state.lock().await
    }

    fn joined_failure(&self, state: &SlotState<T>, observed: u64) -> Option<BlobError> {
        (self.observed() != observed)
            .then(|| state.failure.clone())
            .flatten()
    }

    fn failed(&self, state: &mut SlotState<T>, error: &BlobError) {
        state.failure = Some(error.clone());
        self.failure_epoch.fetch_add(1, Ordering::Release);
    }
}

type KeySlot = Arc<Slot<CachedDek>>;
type PodSlot = Arc<Slot<[u8; 16]>>;

/// Encrypt resource bodies under one current DEK per pod. Unscoped writes refuse.
///
/// Construct once per exclusive index and share the decorator across tasks. All
/// reservations and provider calls are serialized per pod/key, across reads and writes.
/// Runtime activation is a separate integration stage of #7189.
pub struct EncryptingBlobStore<B, K: KeyProvider> {
    inner: B,
    provider: K,
    records: KeyRecords,
    scheme: CryptScheme,
    options: CryptOptions,
    keys: Mutex<HashMap<[u8; 16], KeySlot>>,
    pods: Mutex<HashMap<String, PodSlot>>,
}

impl<B: BlobStore, K: KeyProvider> EncryptingBlobStore<B, K> {
    /// Open over an exclusive index, validating existing wrapped keys before use.
    /// Errors include a wrong local KEK. Wrapped-key records never contain plaintext keys.
    pub async fn new(
        inner: B,
        provider: K,
        index: Arc<dyn SparqClient>,
        scheme: CryptScheme,
        options: CryptOptions,
    ) -> Result<Self, BlobError> {
        index.exclusive().map_err(index_error)?;
        let this = Self {
            inner,
            provider,
            records: KeyRecords { index },
            scheme,
            options,
            keys: Mutex::new(HashMap::new()),
            pods: Mutex::new(HashMap::new()),
        };
        for record in this.records.all().await? {
            if record.scheme != scheme.as_str() {
                return Err(invalid_record());
            }
            let id = record.id()?;
            if record.current {
                let slot = this.pod_slot(&record.pod).await;
                if slot.lock().await.value.replace(id).is_some() {
                    return Err(invalid_record());
                }
            }
            let key = this
                .provider
                .unwrap(
                    &WrappedKey::from_bytes(&record.wrapped).map_err(crypt_error)?,
                    &record.context(),
                )
                .await
                .map_err(crypt_error)?;
            let cached = CachedDek {
                dek: Dek::new(key, id, record.reserved_seals).map_err(crypt_error)?,
                record,
                expires: this.expires(),
            };
            this.key_slot(id).await.lock().await.value = Some(cached);
            this.sweep_expired_keys().await;
        }
        // A directory has no byte-plane object. Reopening preserves its child edges.
        this.records
            .index
            .put_meta(
                DATA_KEY_CONTAINER,
                ResourceMeta {
                    content_type: RECORD_TYPE.into(),
                    blob_key: String::new(),
                    etag: String::new(),
                    last_modified: None,
                },
            )
            .await
            .map_err(index_error)?;
        Ok(this)
    }

    fn expires(&self) -> Duration {
        self.options.clock.now().saturating_add(self.options.ttl)
    }

    async fn sweep_expired_keys(&self) {
        let now = self.options.clock.now();
        self.keys.lock().await.retain(|_, slot| {
            let Ok(mut state) = slot.state.try_lock() else {
                return true;
            };
            if state.value.as_ref().is_some_and(|c| now >= c.expires) {
                state.value = None;
                // Keep slots that have active or queued callers on the same flight.
                return Arc::strong_count(slot) > 1;
            }
            true
        });
    }

    async fn key_slot(&self, id: [u8; 16]) -> KeySlot {
        self.keys
            .lock()
            .await
            .entry(id)
            .or_insert_with(|| Arc::new(Slot::default()))
            .clone()
    }

    async fn pod_slot(&self, pod: &str) -> PodSlot {
        self.pods
            .lock()
            .await
            .entry(pod.to_owned())
            .or_insert_with(|| Arc::new(Slot::default()))
            .clone()
    }

    async fn refresh(
        &self,
        id: [u8; 16],
        slot: &Slot<CachedDek>,
        cached: &mut SlotState<CachedDek>,
        observed: u64,
    ) -> Result<(), BlobError> {
        if cached
            .value
            .as_ref()
            .is_some_and(|c| self.options.clock.now() < c.expires)
        {
            return Ok(());
        }
        if let Some(error) = slot.joined_failure(cached, observed) {
            return Err(error);
        }
        // Drop the old secret even when the provider refuses the refresh.
        cached.value = None;
        cached.failure = None;
        let refreshed = async {
            let record = self.records.load(&id).await?;
            let key = self
                .provider
                .unwrap(
                    &WrappedKey::from_bytes(&record.wrapped).map_err(crypt_error)?,
                    &record.context(),
                )
                .await
                .map_err(crypt_error)?;
            Ok(CachedDek {
                dek: Dek::new(key, id, record.reserved_seals).map_err(crypt_error)?,
                record,
                expires: self.expires(),
            })
        }
        .await;
        match refreshed {
            Ok(value) => {
                cached.value = Some(value);
                self.sweep_expired_keys().await;
                Ok(())
            }
            Err(error) => {
                slot.failed(cached, &error);
                Err(error)
            }
        }
    }

    async fn generate(&self, scope: &BlobScope) -> Result<[u8; 16], BlobError> {
        let context = KeyContext {
            pod: scope.0.clone(),
            scheme: self.scheme.as_str().into(),
        };
        let (key, wrapped) = self
            .provider
            .generate_data_key(&context)
            .await
            .map_err(crypt_error)?;
        let mut id = [0; 16];
        getrandom::getrandom(&mut id)
            .map_err(|_| crypt_error(sparq_lws_crypt::Error::ProviderUnavailable))?;
        if self
            .records
            .index
            .exists(&record_iri(&id))
            .await
            .map_err(index_error)?
        {
            return Err(invalid_record());
        }
        let record = KeyRecord {
            dek_id: hex(&id),
            wrapped: wrapped.to_bytes().map_err(crypt_error)?,
            pod: scope.0.clone(),
            scheme: context.scheme,
            reserved_seals: self.options.block.min(self.options.rotate_at),
            created: crate::clock::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            current: true,
        };
        // The reservation commits BEFORE a caller can use this key for its first seal.
        self.records.save(&record).await?;
        self.key_slot(id).await.lock().await.value = Some(CachedDek {
            dek: Dek::new(key, id, 0).map_err(crypt_error)?,
            record,
            expires: self.expires(),
        });
        self.sweep_expired_keys().await;
        Ok(id)
    }

    async fn encrypt(&self, scope: &BlobScope, key: &str, body: Bytes) -> Result<Bytes, BlobError> {
        if scope.0.is_empty() {
            return Err(invalid_record());
        }
        let pod = self.pod_slot(&scope.0).await;
        // Observe before waiting: only callers already queued on this flight share
        // its failure. A later independent request can retry immediately.
        let pod_observed = pod.observed();
        let mut current = pod.lock().await;
        if let Some(error) = pod.joined_failure(&current, pod_observed) {
            return Err(error);
        }
        let result = self
            .encrypt_current(scope, key, body, &mut current.value)
            .await;
        match &result {
            Ok(_) => current.failure = None,
            Err(error) => pod.failed(&mut current, error),
        }
        result
    }

    async fn encrypt_current(
        &self,
        scope: &BlobScope,
        key: &str,
        body: Bytes,
        current: &mut Option<[u8; 16]>,
    ) -> Result<Bytes, BlobError> {
        loop {
            let id = match *current {
                Some(id) => id,
                None => {
                    // A cancelled first writer may have committed before publishing
                    // its in-memory pointer. Discover it before generating another.
                    let mut found = None;
                    for record in self.records.all().await? {
                        if record.current
                            && record.pod == scope.0
                            && found.replace(record.id()?).is_some()
                        {
                            return Err(invalid_record());
                        }
                    }
                    let id = match found {
                        Some(id) => id,
                        None => self.generate(scope).await?,
                    };
                    *current = Some(id);
                    id
                }
            };
            let slot = self.key_slot(id).await;
            let observed = slot.observed();
            let mut cached = slot.lock().await;
            self.refresh(id, &slot, &mut cached, observed).await?;
            let c = cached.value.as_mut().ok_or_else(invalid_record)?;
            if c.record.pod != scope.0 || c.record.scheme != self.scheme.as_str() {
                return Err(invalid_record());
            }
            if c.dek.seal_count() >= self.options.rotate_at {
                let mut retired = c.record.clone();
                retired.current = false;
                self.records.save(&retired).await?;
                c.record = retired;
                *current = None;
                continue;
            }
            if c.dek.seal_count() >= c.record.reserved_seals {
                let mut raised = c.record.clone();
                raised.reserved_seals = raised
                    .reserved_seals
                    .checked_add(self.options.block)
                    .ok_or_else(invalid_record)?
                    .min(self.options.rotate_at);
                if let Err(e) = self.records.save(&raised).await {
                    cached.value = None;
                    return Err(e);
                }
                c.record = raised;
            }
            return seal(&c.dek, key, &body)
                .map(Bytes::from)
                .map_err(crypt_error);
        }
    }
    async fn decrypt(&self, scope: Option<&BlobScope>, key: &str) -> Result<Bytes, BlobError> {
        let body = self.inner.get(key).await?;
        if !is_envelope(&body) {
            return Ok(body);
        }
        let id = parse_header(&body)
            .ok_or_else(|| crypt_error(sparq_lws_crypt::Error::MalformedInput))?
            .dek_id;
        let slot = self.key_slot(id).await;
        let observed = slot.observed();
        let mut cached = slot.lock().await;
        if let Err(e) = self.refresh(id, &slot, &mut cached, observed).await {
            if matches!(e, BlobError::UnknownDataKey) {
                eprintln!("UnknownDataKey blob={key}");
            }
            return Err(e);
        }
        let c = cached.value.as_ref().ok_or_else(invalid_record)?;
        let plaintext = open(&c.dek, key, &body).map_err(crypt_error)?;
        if scope.is_some_and(|scope| c.record.pod != scope.0) {
            return Err(BlobError::Backend(
                "data-key pod does not match resource pod".into(),
            ));
        }
        Ok(Bytes::from(plaintext))
    }
}

#[async_trait]
impl<B: BlobStore, K: KeyProvider> BlobStore for EncryptingBlobStore<B, K> {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        self.decrypt(None, key).await
    }

    async fn get_scoped(&self, scope: &BlobScope, key: &str) -> Result<Bytes, BlobError> {
        self.decrypt(Some(scope), key).await
    }

    async fn put(&self, _: &str, _: Bytes) -> Result<(), BlobError> {
        Err(BlobError::Backend(
            "encrypted blob writes require a pod scope".into(),
        ))
    }

    async fn put_scoped(&self, scope: &BlobScope, key: &str, body: Bytes) -> Result<(), BlobError> {
        let envelope = self.encrypt(scope, key, body).await?;
        self.inner.put(key, envelope).await
    }

    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        self.inner.exists(key).await
    }
    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        self.inner.delete(key).await
    }
    async fn list(&self) -> Result<Vec<BlobEntry>, BlobError> {
        self.inner.list().await
    }
    async fn stat(&self, key: &str) -> Result<Option<BlobEntry>, BlobError> {
        self.inner.stat(key).await
    }
    async fn delete_if_unchanged(&self, key: &str, generation: u64) -> Result<bool, BlobError> {
        self.inner.delete_if_unchanged(key, generation).await
    }
}

impl<S: SparqClient + 'static, B: BlobStore> CompositeStore<S, B> {
    /// Assemble the library decorator over this store's index and byte plane.
    /// Runtime activation and directory-format validation are separate integration work.
    pub async fn encrypt_blobs<K: KeyProvider>(
        self,
        provider: K,
        scheme: CryptScheme,
        options: CryptOptions,
    ) -> Result<CompositeStore<S, EncryptingBlobStore<B, K>>, BlobError> {
        let policy = match scheme {
            CryptScheme::Lws => BlobScopePolicy::Lws,
            CryptScheme::Solid => self.blob_scope,
        };
        let blob =
            EncryptingBlobStore::new(self.blob, provider, self.sparq.clone(), scheme, options)
                .await?;
        // Re-read existing bodies through the envelope sniff after changing decoders.
        let body_cache = super::BodyCache::with_max_entry_bytes(
            self.body_cache.max_bytes(),
            self.body_cache.max_entry_bytes(),
        );
        Ok(CompositeStore {
            sparq: self.sparq,
            blob,
            blob_scope: policy,
            body_cache,
            holding: self.holding,
        })
    }
}

#[cfg(test)]
mod tests;
