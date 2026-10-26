//! Native, create-only object storage for opaque LWS blob keys.
//!
//! This crate is independent of the server. The optional server adapter implements its
//! `BlobStore` trait and maps these errors to the server's opaque error vocabulary.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;
use std::time::SystemTime;

use bytes::Bytes;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use object_store::path::Path;
use object_store::{ObjectMeta, ObjectStore, ObjectStoreExt, PutMode};

/// An opaque error: backend diagnostic strings can contain configuration and URLs.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// The key does not match the blob-key grammar.
    #[error("invalid blob key")]
    InvalidKey,
    /// The constructor configuration is invalid or its provider feature is disabled.
    #[error("invalid blob-store configuration or disabled provider")]
    Configuration,
    /// No object exists at this key.
    #[error("blob not found")]
    NotFound,
    /// A create would replace an existing object.
    #[error("blob already exists")]
    AlreadyExists,
    /// An object-store operation failed.
    #[error("object-store operation failed")]
    Backend,
}

fn opaque_error(error: object_store::Error) -> Error {
    match error {
        object_store::Error::NotFound { .. } => Error::NotFound,
        object_store::Error::AlreadyExists { .. } | object_store::Error::Precondition { .. } => {
            Error::AlreadyExists
        }
        _ => Error::Backend,
    }
}

// Consolidate with the filesystem backend when its independent lane lands (review follow-up).
fn validate_key(key: &str) -> Result<(&str, bool), Error> {
    let (bare, pooled) = key
        .strip_prefix("$pool$")
        .map_or((key, false), |k| (k, true));
    let suffix = bare.strip_prefix("b1-").ok_or(Error::InvalidKey)?;
    if suffix.len() != 32
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::InvalidKey);
    }
    Ok((bare, pooled))
}

/// A listed key and its backend-recorded modification time. No delete generation is advertised.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The original logical key, including `$pool$` for a pooled object.
    pub key: String,
    /// Backend modification time, used by the reconciler's grace window.
    pub last_modified: SystemTime,
}

/// A create-only byte plane under a dedicated object prefix.
///
/// Keys must never be reused, including after deletion. `PutMode::Create` prevents replacing
/// extant bytes; callers retain responsibility for minting fresh keys for every write.
#[derive(Clone)]
pub struct ObjectBlobStore {
    store: Arc<dyn ObjectStore>,
    prefix: Path,
}

// Do not expose provider Display/Debug implementations or configured URLs.
impl std::fmt::Debug for ObjectBlobStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectBlobStore").finish_non_exhaustive()
    }
}

impl ObjectBlobStore {
    /// Wrap an existing backend, isolating this store under `prefix`.
    pub fn new(store: Arc<dyn ObjectStore>, prefix: impl AsRef<str>) -> Result<Self, Error> {
        let prefix = Path::parse(prefix.as_ref()).map_err(|_| Error::Configuration)?;
        Ok(Self { store, prefix })
    }

    /// Open `file://`, `s3://` (`aws`), `az://` (`azure`), or `gs://` (`gcp`).
    ///
    /// Options are the pinned object_store provider's configuration key/value pairs. This does
    /// not read environment variables. Local directories must already exist. Query strings,
    /// fragments, and URL user information are refused; pass credentials through options.
    pub fn from_url<I, K, V>(url: &str, options: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        let url = url::Url::parse(url).map_err(|_| Error::Configuration)?;
        if !matches!(url.scheme(), "file" | "s3" | "az" | "gs")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::Configuration);
        }
        let (store, prefix) =
            object_store::parse_url_opts(&url, options).map_err(|_| Error::Configuration)?;
        Ok(Self {
            store: store.into(),
            prefix,
        })
    }

    fn path(&self, key: &str) -> Result<Path, Error> {
        let (bare, pooled) = validate_key(key)?;
        Ok(if pooled {
            self.prefix.clone().join("pool").join(bare)
        } else {
            self.prefix.clone().join(bare)
        })
    }

    /// Create fresh bytes, refusing an existing key atomically at the backend.
    pub async fn put(&self, key: &str, body: Bytes) -> Result<(), Error> {
        let path = self.path(key)?;
        self.store
            .put_opts(&path, body.into(), PutMode::Create.into())
            .await
            .map_err(opaque_error)?;
        Ok(())
    }

    /// Read a blob's bytes.
    pub async fn get(&self, key: &str) -> Result<Bytes, Error> {
        let path = self.path(key)?;
        self.store
            .get(&path)
            .await
            .map_err(opaque_error)?
            .bytes()
            .await
            .map_err(opaque_error)
    }

    /// Test existence through HEAD, mapping only NotFound to false.
    pub async fn exists(&self, key: &str) -> Result<bool, Error> {
        Ok(self.stat(key).await?.is_some())
    }

    /// Unconditionally remove a key, succeeding when it is already absent.
    pub async fn delete(&self, key: &str) -> Result<(), Error> {
        let path = self.path(key)?;
        match self.store.delete(&path).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => Err(opaque_error(e)),
        }
    }

    /// Return one object's current metadata through HEAD.
    pub async fn stat(&self, key: &str) -> Result<Option<Entry>, Error> {
        let path = self.path(key)?;
        match self.store.head(&path).await {
            Ok(meta) => Ok(Some(Entry {
                key: key.into(),
                last_modified: meta.last_modified.into(),
            })),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(opaque_error(e)),
        }
    }

    /// Stream backend pages lazily, yielding only valid keys in this store's namespace.
    ///
    /// Objects outside the key grammar or in nested foreign directories are left alone.
    pub fn list(&self) -> BoxStream<'_, Result<Entry, Error>> {
        self.store
            .list(Some(&self.prefix))
            .filter_map(move |meta| async move {
                match meta {
                    Ok(meta) => self.entry(meta).map(Ok),
                    Err(e) => Some(Err(opaque_error(e))),
                }
            })
            .boxed()
    }

    fn entry(&self, meta: ObjectMeta) -> Option<Entry> {
        let mut parts = meta.location.prefix_match(&self.prefix)?;
        let first = parts.next()?;
        let key = if first.as_ref() == "pool" {
            format!("$pool${}", parts.next()?.as_ref())
        } else {
            first.as_ref().to_string()
        };
        if parts.next().is_some() {
            return None;
        }
        validate_key(&key).ok()?;
        Some(Entry {
            key,
            last_modified: meta.last_modified.into(),
        })
    }
}
