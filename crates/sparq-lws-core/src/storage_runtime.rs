//! Native storage selection and durable-directory boot. The index owns the data-directory lock.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

#[cfg(all(feature = "embedded-sparq", feature = "blob-fs"))]
use sparq_lws_core::store::blob_fs::FsBlobStore;
#[cfg(feature = "embedded-sparq")]
use sparq_lws_core::store::{BlobEntry, BlobError, BlobStore, InMemoryBlobStore};

const ENV_DATA_DIR: &str = "SOLID_SERVER_DATA_DIR";
const ENV_SPARQ_DIR: &str = "SOLID_SERVER_SPARQ_DIR";
const ENV_BACKEND: &str = "PSS_SPARQ_BACKEND";
const ENV_WAIT: &str = "SOLID_SERVER_WAIT_FOR_LOCK_SECS";
const ENV_ALLOW_INCONSISTENT: &str = "SOLID_SERVER_ALLOW_INCONSISTENT_DURABILITY";
const ENV_CRYPT_PROVIDER: &str = "SOLID_SERVER_CRYPT_PROVIDER";
#[cfg(feature = "crypt")]
const ENV_CRYPT_KEK: &str = "SOLID_SERVER_CRYPT_LOCAL_KEK_FILE";
#[cfg(feature = "crypt")]
const ENV_CRYPT_TTL: &str = "SOLID_SERVER_CRYPT_DEK_TTL_SECS";
#[cfg(feature = "crypt")]
const ENV_CRYPT_SCOPE: &str = "SOLID_SERVER_CRYPT_POD_SCOPE";

#[cfg(feature = "crypt")]
#[derive(Debug)]
struct CryptConfig {
    kek_file: PathBuf,
    ttl: Duration,
    lws: bool,
    pod: Option<String>,
}

#[derive(Debug)]
pub(super) struct StorageConfig {
    pub backend: String,
    data_dir: Option<PathBuf>,
    sparq_dir: Option<PathBuf>,
    #[cfg(feature = "embedded-sparq")]
    wait: Duration,
    allow_inconsistent: bool,
    #[cfg(feature = "crypt")]
    crypt: Option<CryptConfig>,
}

impl StorageConfig {
    pub fn from_env() -> io::Result<Self> {
        Self::read_environment(|key| std::env::var(key))
    }

    fn read_environment(
        mut var: impl FnMut(&str) -> Result<String, std::env::VarError>,
    ) -> io::Result<Self> {
        let mut invalid_key = None;
        let result = Self::read(|key| match var(key) {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotUnicode(_)) if key.starts_with("SOLID_SERVER_CRYPT_") => {
                // Invalid provider bytes must never silently select the plaintext default.
                invalid_key = Some(key.to_owned());
                None
            }
            Err(_) => None,
        });
        if let Some(key) = invalid_key {
            return Err(invalid(format!("{key} must contain valid Unicode")));
        }
        result
    }

    fn read(mut var: impl FnMut(&str) -> Option<String>) -> io::Result<Self> {
        let path = |raw: Option<String>| {
            raw.filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        };
        let data_dir = path(var(ENV_DATA_DIR));
        let sparq_dir = path(var(ENV_SPARQ_DIR));
        let backend = var(ENV_BACKEND).map(|value| value.trim().to_ascii_lowercase());
        if data_dir.is_some() && sparq_dir.is_some() {
            return Err(invalid(format!(
                "{ENV_DATA_DIR} and {ENV_SPARQ_DIR} cannot be set together"
            )));
        }
        if data_dir.is_some() && backend.as_ref().is_some_and(|value| value != "embedded") {
            return Err(invalid(format!(
                "{ENV_DATA_DIR} requires {ENV_BACKEND}=embedded; unset {ENV_BACKEND} or set it to embedded"
            )));
        }
        let wait = match var(ENV_WAIT) {
            None => Duration::ZERO,
            Some(raw) => Duration::from_secs(raw.trim().parse::<u64>().map_err(|_| {
                invalid(format!(
                    "{ENV_WAIT} must be a non-negative integer in seconds"
                ))
            })?),
        };
        #[cfg(not(feature = "embedded-sparq"))]
        let _ = wait;
        let allow_inconsistent = var(ENV_ALLOW_INCONSISTENT).as_deref() == Some("1");
        let provider = var(ENV_CRYPT_PROVIDER).unwrap_or_else(|| "none".into());
        let encrypted = match provider.trim().to_ascii_lowercase().as_str() {
            "none" => false,
            "local" => {
                #[cfg(not(feature = "crypt"))]
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("{ENV_CRYPT_PROVIDER}=local is not built; enable the `crypt` feature"),
                ));
                #[cfg(feature = "crypt")]
                true
            }
            "aws-kms" | "azure-kv" => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("{ENV_CRYPT_PROVIDER}={provider} is not built"),
                ))
            }
            _ => {
                return Err(invalid(format!(
                    "{ENV_CRYPT_PROVIDER} must be none, local, aws-kms or azure-kv"
                )))
            }
        };
        #[cfg(not(feature = "crypt"))]
        let _ = encrypted;
        #[cfg(feature = "crypt")]
        let crypt = if encrypted {
            let kek_file = path(var(ENV_CRYPT_KEK)).ok_or_else(|| {
                invalid(format!("{ENV_CRYPT_KEK} is required for provider=local"))
            })?;
            let ttl = match var(ENV_CRYPT_TTL) {
                None => sparq_lws_core::store::blob_crypt::DEFAULT_DEK_TTL,
                Some(raw) => Duration::from_secs(raw.trim().parse::<u64>().map_err(|_| {
                    invalid(format!(
                        "{ENV_CRYPT_TTL} must be a non-negative integer in seconds"
                    ))
                })?),
            };
            let pod = var(ENV_CRYPT_SCOPE).map(|value| value.trim().to_owned());
            if pod.as_deref() == Some("") {
                return Err(invalid(format!(
                    "{ENV_CRYPT_SCOPE} must be a non-empty pod identity"
                )));
            }
            Some(CryptConfig {
                kek_file,
                ttl,
                pod,
                lws: var("SOLID_SERVER_PROTOCOL")
                    .is_some_and(|value| value.trim().eq_ignore_ascii_case("lws")),
            })
        } else {
            None
        };
        Ok(Self {
            backend: backend.unwrap_or_else(|| {
                if data_dir.is_some() {
                    "embedded"
                } else {
                    "memory"
                }
                .into()
            }),
            data_dir,
            sparq_dir,
            #[cfg(feature = "embedded-sparq")]
            wait,
            allow_inconsistent,
            #[cfg(feature = "crypt")]
            crypt,
        })
    }

    #[cfg(any(feature = "embedded-sparq", feature = "crypt"))]
    fn encryption_enabled(&self) -> bool {
        #[cfg(feature = "crypt")]
        return self.crypt.is_some();
        #[cfg(not(feature = "crypt"))]
        false
    }

    pub fn blob_scope(&self) -> sparq_lws_core::store::BlobScopePolicy {
        use sparq_lws_core::store::BlobScopePolicy;
        #[cfg(feature = "crypt")]
        if let Some(crypt) = &self.crypt {
            if crypt.lws {
                return BlobScopePolicy::Lws;
            }
            if let Some(pod) = &crypt.pod {
                return BlobScopePolicy::Fixed(sparq_lws_core::store::BlobScope(pod.clone()));
            }
        }
        BlobScopePolicy::Solid
    }

    #[cfg(feature = "crypt")]
    pub fn crypt_enabled(&self) -> bool {
        self.encryption_enabled()
    }

    /// Called once per index, before sharing handles with the router and reconciler.
    /// The embedded index retains its exclusive data-directory lock through this call.
    #[cfg(feature = "crypt")]
    pub async fn encrypt<B: sparq_lws_core::store::BlobStore>(
        &self,
        index: std::sync::Arc<dyn sparq_lws_core::store::SparqClient>,
        blob: B,
    ) -> io::Result<
        sparq_lws_core::store::blob_crypt::EncryptingBlobStore<
            B,
            sparq_lws_crypt::LocalKeyProvider,
        >,
    > {
        use sparq_lws_core::store::blob_crypt::{CryptOptions, CryptScheme, EncryptingBlobStore};
        let crypt = self
            .crypt
            .as_ref()
            .ok_or_else(|| invalid("encryption is not configured"))?;
        let provider = sparq_lws_crypt::LocalKeyProvider::from_file(&crypt.kek_file)
            .map_err(|error| invalid(format!("{ENV_CRYPT_KEK}: {error}")))?;
        let mut options = CryptOptions::default();
        options.ttl = crypt.ttl;
        let scheme = if crypt.lws {
            CryptScheme::Lws
        } else {
            CryptScheme::Solid
        };
        // Opening validates every wrapped key, including retired keys, before any request is served.
        let encrypted = EncryptingBlobStore::new(blob, provider, index, scheme, options)
            .await
            .map_err(io::Error::other)?;
        #[cfg(feature = "embedded-sparq")]
        if let Some(data) = &self.data_dir {
            upgrade_blob_format(data)?;
        }
        Ok(encrypted)
    }

    pub fn has_data_dir(&self) -> bool {
        self.data_dir.is_some()
    }

    pub fn index_is_durable(&self) -> bool {
        sparq_backend_is_durable(
            &self.backend,
            self.data_dir.is_some() || self.sparq_dir.is_some(),
        )
    }

    pub fn check_durability(&self, blob_is_durable: bool) -> io::Result<()> {
        let index_is_durable = self.index_is_durable();
        if reject_durable_sparq_with_inmem_blob(
            &self.backend,
            self.data_dir.is_some() || self.sparq_dir.is_some(),
            !blob_is_durable,
        ) {
            return Err(invalid(
                "a durable/shared SPARQ index requires a durable blob store; the in-memory blob store is ephemeral",
            ));
        }
        if blob_is_durable && !index_is_durable && !self.allow_inconsistent {
            return Err(invalid(format!(
                "a durable blob store requires a durable SPARQ index; set {ENV_ALLOW_INCONSISTENT}=1 only to allow inconsistent durability explicitly"
            )));
        }
        Ok(())
    }

    /// The constructor is injected so tests can exercise the boot order with durable test bytes.
    /// It runs only after the index lock is held; a failure drops and joins the index actor.
    #[cfg(feature = "embedded-sparq")]
    pub async fn open_embedded_with<B, F>(
        &self,
        blob_is_durable: bool,
        make_blob: F,
    ) -> io::Result<(sparq_lws_core::store::EmbeddedSparqClient, B)>
    where
        B: sparq_lws_core::store::BlobStore,
        F: FnOnce(Option<&std::path::Path>) -> io::Result<B>,
    {
        self.check_durability(blob_is_durable)?;
        let data_dir = self.data_dir.clone();
        let sparq_dir = self.sparq_dir.clone();
        let wait = self.wait;
        let encrypted = self.encryption_enabled();
        let client = tokio::task::spawn_blocking(move || {
            use sparq_lws_core::store::EmbeddedSparqClient;
            if let Some(data) = data_dir {
                prepare_format(&data, encrypted)?;
                let client = open_with_retry(
                    &data.join("index"),
                    wait,
                    EmbeddedSparqClient::open_or_create,
                )?;
                // A prior holder may have upgraded the marker while this boot waited.
                prepare_format(&data, encrypted)?;
                Ok(client)
            } else if let Some(dir) = sparq_dir {
                // Keep the legacy path's existing-snapshot semantics and backend error text.
                EmbeddedSparqClient::open(&dir).map_err(io::Error::other)
            } else {
                EmbeddedSparqClient::in_memory().map_err(io::Error::other)
            }
        })
        .await
        .map_err(io::Error::other)??;
        let blobs = self.data_dir.as_ref().map(|data| data.join("blobs"));
        let blob = make_blob(blobs.as_deref())?;
        Ok((client, blob))
    }
}

pub(super) fn sparq_backend_is_durable(backend: &str, sparq_dir_set: bool) -> bool {
    match backend {
        "http" => true,
        "embedded" => sparq_dir_set,
        _ => false,
    }
}

pub(super) fn reject_durable_sparq_with_inmem_blob(
    backend: &str,
    sparq_dir_set: bool,
    blob_is_in_memory: bool,
) -> bool {
    blob_is_in_memory && sparq_backend_is_durable(backend, sparq_dir_set)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(feature = "embedded-sparq")]
pub(super) enum NativeBlobs {
    InMemory(InMemoryBlobStore),
    #[cfg(feature = "blob-fs")]
    Filesystem(FsBlobStore),
}

#[cfg(feature = "embedded-sparq")]
#[async_trait::async_trait]
impl BlobStore for NativeBlobs {
    async fn get(&self, key: &str) -> Result<bytes::Bytes, BlobError> {
        match self {
            Self::InMemory(store) => BlobStore::get(store, key).await,
            #[cfg(feature = "blob-fs")]
            Self::Filesystem(store) => BlobStore::get(store, key).await,
        }
    }

    async fn put(&self, key: &str, body: bytes::Bytes) -> Result<(), BlobError> {
        match self {
            Self::InMemory(store) => BlobStore::put(store, key, body).await,
            #[cfg(feature = "blob-fs")]
            Self::Filesystem(store) => BlobStore::put(store, key, body).await,
        }
    }

    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        match self {
            Self::InMemory(store) => BlobStore::exists(store, key).await,
            #[cfg(feature = "blob-fs")]
            Self::Filesystem(store) => BlobStore::exists(store, key).await,
        }
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        match self {
            Self::InMemory(store) => BlobStore::delete(store, key).await,
            #[cfg(feature = "blob-fs")]
            Self::Filesystem(store) => BlobStore::delete(store, key).await,
        }
    }

    async fn list(&self) -> Result<Vec<BlobEntry>, BlobError> {
        match self {
            Self::InMemory(store) => BlobStore::list(store).await,
            #[cfg(feature = "blob-fs")]
            Self::Filesystem(store) => BlobStore::list(store).await,
        }
    }

    async fn stat(&self, key: &str) -> Result<Option<BlobEntry>, BlobError> {
        match self {
            Self::InMemory(store) => BlobStore::stat(store, key).await,
            #[cfg(feature = "blob-fs")]
            Self::Filesystem(store) => BlobStore::stat(store, key).await,
        }
    }

    async fn delete_if_unchanged(&self, key: &str, generation: u64) -> Result<bool, BlobError> {
        match self {
            Self::InMemory(store) => BlobStore::delete_if_unchanged(store, key, generation).await,
            #[cfg(feature = "blob-fs")]
            Self::Filesystem(store) => BlobStore::delete_if_unchanged(store, key, generation).await,
        }
    }
}

/// Single blob construction point, called with the index lock held for a data directory.
#[cfg(feature = "embedded-sparq")]
pub(super) fn make_blobs(path: Option<&std::path::Path>) -> io::Result<NativeBlobs> {
    if let Some(path) = path {
        #[cfg(feature = "blob-fs")]
        return FsBlobStore::open(path).map(NativeBlobs::Filesystem);
        #[cfg(not(feature = "blob-fs"))]
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "{ENV_DATA_DIR} cannot boot: the filesystem blob store is not built in; enable the `blob-fs` feature (requested {})",
                path.display()
            ),
        ));
    }
    Ok(NativeBlobs::InMemory(InMemoryBlobStore::new()))
}

#[cfg(feature = "embedded-sparq")]
fn open_with_retry<T>(
    dir: &std::path::Path,
    wait: Duration,
    mut open: impl FnMut(&std::path::Path) -> io::Result<T>,
) -> io::Result<T> {
    let started = std::time::Instant::now();
    let mut backoff = Duration::from_millis(10);
    loop {
        match open(dir) {
            Ok(value) => return Ok(value),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let remaining = wait.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        format!(
                            "cannot acquire data-directory index lock {} (inside {}/sparq.lock): {error}",
                            dir.with_extension("lock").display(),
                            dir.display()
                        ),
                    ));
                }
                std::thread::sleep(backoff.min(remaining));
                backoff = (backoff * 2).min(Duration::from_millis(250));
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(feature = "embedded-sparq")]
fn prepare_format(data: &std::path::Path, encrypted: bool) -> io::Result<()> {
    use std::fs;

    const FORMAT: &[u8] = b"{\"schema\":1,\"index\":{\"major\":1},\"blobs\":{\"major\":1}}\n";
    fs::create_dir_all(data)?;
    let marker = data.join("format.json");
    match fs::read(&marker) {
        Ok(bytes) => return validate_format(&bytes, encrypted).map(|_| ()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    // Inspect only. Neither plane is created until the marker has been published.
    for name in ["index", "blobs"] {
        match fs::read_dir(data.join(name)) {
            Ok(mut entries) => {
                if entries.next().transpose()?.is_some() {
                    // A concurrent fresh boot may have published the marker after our first read.
                    if let Ok(bytes) = fs::read(&marker) {
                        return validate_format(&bytes, encrypted).map(|_| ());
                    }
                    return Err(invalid(format!(
                        "refusing foreign data directory {}: format.json is absent but {name}/ has content",
                        data.display()
                    )));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    write_format(data, FORMAT, false)?;
    validate_format(&fs::read(marker)?, encrypted).map(|_| ())
}

#[cfg(all(feature = "crypt", feature = "embedded-sparq"))]
fn upgrade_blob_format(data: &std::path::Path) -> io::Result<()> {
    let bytes = std::fs::read(data.join("format.json"))?;
    if validate_format(&bytes, true)? == 2 {
        return Ok(());
    }
    let mut format: serde_json::Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    format["blobs"]["major"] = 2.into();
    format["blobs"]["encryption"] = "envelope-v1".into();
    write_format(
        data,
        &serde_json::to_vec(&format).map_err(io::Error::other)?,
        true,
    )
}

#[cfg(feature = "embedded-sparq")]
fn write_format(data: &std::path::Path, bytes: &[u8], replace: bool) -> io::Result<()> {
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;

    let marker = data.join("format.json");
    // Unique temporary names allow concurrent fresh boots without introducing another lock.
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
    let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let tmp = data.join(format!(".format-{suffix}.tmp"));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        if replace {
            // Upgrades run with the exclusive index lock held.
            fs::rename(&tmp, &marker)?;
        } else {
            // A delayed fresh boot must never replace a marker another boot upgraded.
            match fs::hard_link(&tmp, &marker) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            fs::remove_file(&tmp)?;
        }
        File::open(data)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

#[cfg(feature = "embedded-sparq")]
fn validate_format(bytes: &[u8], encrypted: bool) -> io::Result<u64> {
    #[derive(serde::Deserialize)]
    struct Plane {
        major: u64,
    }
    #[derive(serde::Deserialize)]
    struct BlobFormat {
        major: u64,
        encryption: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct Format {
        schema: u64,
        index: Plane,
        blobs: BlobFormat,
    }
    let format: Format = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("invalid format.json: {error}")))?;
    if format.schema != 1 || format.index.major != 1 {
        return Err(invalid(
            "unsupported format.json: expected schema=1, index.major=1",
        ));
    }
    match (format.blobs.major, format.blobs.encryption.as_deref()) {
        (1, None) => Ok(1),
        (2, Some("envelope-v1")) if encrypted => Ok(2),
        (2, Some("envelope-v1")) => Err(invalid(format!(
            "format.json blobs.major=2 requires encryption; {ENV_CRYPT_PROVIDER}=none cannot open this directory"
        ))),
        _ => Err(invalid("unsupported format.json blob format: expected major=1 or major=2 with encryption=envelope-v1")),
    }
}

/// Own the final embedded handle across listener drain, background-task stop and compaction.
#[derive(Default)]
pub(super) struct StorageShutdown {
    #[cfg(feature = "embedded-sparq")]
    client: Option<sparq_lws_core::store::EmbeddedSparqClient>,
}

impl StorageShutdown {
    #[cfg(feature = "embedded-sparq")]
    pub fn retain(
        &mut self,
        config: &StorageConfig,
        client: &sparq_lws_core::store::EmbeddedSparqClient,
    ) {
        if config.index_is_durable() {
            self.client = Some(client.clone());
        }
    }

    pub async fn run<F>(
        self,
        serving: F,
        reconcile_task: Option<tokio::task::JoinHandle<()>>,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        F: std::future::Future<Output = Result<(), Box<dyn std::error::Error>>>,
    {
        let served = serving.await;
        if let Some(task) = reconcile_task {
            task.abort();
            let _ = task.await;
        }
        #[cfg(feature = "embedded-sparq")]
        if let Some(client) = self.client {
            // No timeout: the actor must finish its job and join before the lock is released.
            let result = client.compact().await;
            if let Err(error) = &result {
                eprintln!("  SHUTDOWN: embedded index compaction failed: {error}");
            }
            drop(client);
            result?;
        }
        served
    }
}

#[cfg(test)]
#[path = "storage_runtime_tests.rs"]
mod tests;
