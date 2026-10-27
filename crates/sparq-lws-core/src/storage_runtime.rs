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

#[derive(Debug)]
pub(super) struct StorageConfig {
    pub backend: String,
    data_dir: Option<PathBuf>,
    sparq_dir: Option<PathBuf>,
    #[cfg(feature = "embedded-sparq")]
    wait: Duration,
    allow_inconsistent: bool,
}

impl StorageConfig {
    pub fn from_env() -> io::Result<Self> {
        Self::read(|key| std::env::var(key).ok())
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
        })
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
        let client = tokio::task::spawn_blocking(move || {
            use sparq_lws_core::store::EmbeddedSparqClient;
            if let Some(data) = data_dir {
                prepare_format(&data)?;
                open_index_with_retry(
                    &data.join("index"),
                    wait,
                    EmbeddedSparqClient::open_or_create,
                )
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
fn open_index_with_retry<T>(
    dir: &std::path::Path,
    wait: Duration,
    mut open: impl FnMut(&std::path::Path) -> io::Result<T>,
) -> io::Result<T> {
    crate::open_retry::open_with_retry(
        wait,
        Duration::from_millis(10),
        Duration::from_millis(250),
        || open(dir),
    )
    .map_err(|error| {
        if error.kind() == io::ErrorKind::WouldBlock {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "cannot acquire data-directory index lock {} (inside {}/sparq.lock): {error}",
                    dir.with_extension("lock").display(),
                    dir.display()
                ),
            )
        } else {
            error
        }
    })
}

#[cfg(feature = "embedded-sparq")]
fn prepare_format(data: &std::path::Path) -> io::Result<()> {
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;

    const FORMAT: &[u8] = b"{\"schema\":1,\"index\":{\"major\":1},\"blobs\":{\"major\":1}}\n";
    fs::create_dir_all(data)?;
    let marker = data.join("format.json");
    match fs::read(&marker) {
        Ok(bytes) => return validate_format(&bytes),
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
                        return validate_format(&bytes);
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
    // Unique temporary names allow concurrent fresh boots without introducing another lock.
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
    let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let tmp = data.join(format!(".format-{suffix}.tmp"));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let result = (|| {
        file.write_all(FORMAT)?;
        file.sync_all()?;
        fs::rename(&tmp, &marker)?;
        File::open(data)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

#[cfg(feature = "embedded-sparq")]
fn validate_format(bytes: &[u8]) -> io::Result<()> {
    #[derive(serde::Deserialize)]
    struct Plane {
        major: u64,
    }
    #[derive(serde::Deserialize)]
    struct Format {
        schema: u64,
        index: Plane,
        blobs: Plane,
    }
    let format: Format = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("invalid format.json: {error}")))?;
    if format.schema != 1 || format.index.major != 1 || format.blobs.major != 1 {
        return Err(invalid(
            "unsupported format.json: expected schema=1, index.major=1, blobs.major=1",
        ));
    }
    Ok(())
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
