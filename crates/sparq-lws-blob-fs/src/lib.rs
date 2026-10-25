//! Filesystem blob storage with immutable keys and durable atomic publication.
//!
//! All filesystem operations run in Tokio's blocking pool. Open a root before using it,
//! with no writes in flight: opening removes all temporary files left by interrupted writes.
//! The root and its directories must be managed exclusively by trusted storage code.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use bytes::Bytes;

const POOL_MARK: &str = "$pool$";
const ENCODED_POOL_MARK: &str = "%24pool%24";

/// An error reading or mutating filesystem blob storage.
#[derive(Debug)]
pub enum Error {
    /// The requested blob does not exist.
    NotFound,
    /// A put attempted to reuse an existing key.
    AlreadyExists,
    /// The key does not match the b1 key scheme.
    InvalidKey,
    /// A filesystem, entropy, or blocking-task operation failed.
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("blob not found"),
            Self::AlreadyExists => f.write_str("blob key already exists"),
            Self::InvalidKey => f.write_str("invalid b1 blob key"),
            Self::Io(error) => write!(f, "filesystem blob error: {error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::NotFound {
            Self::NotFound
        } else {
            Self::Io(error)
        }
    }
}

/// A stored blob's filesystem metadata.
#[derive(Debug, Clone)]
pub struct BlobEntry {
    /// The decoded opaque b1 key, including the pool mark when present.
    pub key: String,
    /// The file length in bytes.
    pub size: u64,
    /// The filesystem modification time, used as an age witness.
    pub last_modified: Option<SystemTime>,
    /// Always `None`: this backend has no native write version.
    pub generation: Option<u64>,
}

#[derive(Debug)]
struct Inner {
    root: PathBuf,
    // Serialize the existence check and rename across clones of this handle.
    // Independently opened handles must still honor the unique-per-write key contract.
    put_lock: Mutex<()>,
}

/// Filesystem bytes stored under unique-per-write b1 keys.
///
/// Accepted keys match `^(\$pool\$)?b1-[0-9a-f]{32}$`. Ordinary blobs live in
/// `objects/<first two hex>/<next two hex>/<key>`; pooled blobs live in
/// `pool/<first two hex>/<key>`, with `$` encoded as `%24` in filenames.
/// Callers must never reuse a key, including after deletion.
#[derive(Debug, Clone)]
pub struct FsBlobStore {
    inner: Arc<Inner>,
}

impl FsBlobStore {
    /// Create the layout and remove interrupted writes from `tmp/`.
    ///
    /// Open only while there are no active writes to this root. Clones share the
    /// same root and write lock; opening again performs startup recovery again.
    ///
    /// # Errors
    /// Returns an error if creating, syncing, or cleaning the layout fails.
    pub async fn open(root: &Path) -> Result<Self, Error> {
        let root = root.to_path_buf();
        blocking(move || {
            create_durable_dir(&root)?;
            let root = root.canonicalize()?;
            for name in ["objects", "pool", "tmp"] {
                create_durable_dir(&root.join(name))?;
            }
            let tmp = root.join("tmp");
            for item in fs::read_dir(&tmp)? {
                let item = item?;
                if item.file_type()?.is_dir() {
                    fs::remove_dir_all(item.path())?;
                } else {
                    fs::remove_file(item.path())?;
                }
            }
            sync_dir(&tmp)?;
            Ok(Self {
                inner: Arc::new(Inner {
                    root,
                    put_lock: Mutex::new(()),
                }),
            })
        })
        .await
    }

    /// Read the complete bytes for a key.
    ///
    /// # Errors
    /// Returns `InvalidKey`, `NotFound`, or an I/O error.
    pub async fn get(&self, key: &str) -> Result<Bytes, Error> {
        let path = blob_path(&self.inner.root, key)?;
        blocking(move || Ok(Bytes::from(fs::read(path)?))).await
    }

    /// Publish a new blob after syncing its bytes and directory entries.
    ///
    /// # Errors
    /// Returns `InvalidKey`, `AlreadyExists` on key reuse, or an I/O error.
    /// An I/O error after rename may leave a complete blob; retry with a fresh key.
    pub async fn put(&self, key: &str, body: Bytes) -> Result<(), Error> {
        let destination = blob_path(&self.inner.root, key)?;
        let filename = encoded_key(key);
        let inner = self.inner.clone();
        blocking(move || {
            let _guard = inner
                .put_lock
                .lock()
                .map_err(|_| Error::Io(io::Error::other("blob write lock poisoned")))?;
            // The caller's unique-key contract plus this handle's lock prevents
            // a destination from appearing between the pre-check and rename.
            match fs::symlink_metadata(&destination) {
                Ok(_) => return Err(Error::AlreadyExists),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            let parent = destination.parent().expect("blob path has a parent");
            if let Some(grandparent) = parent.parent() {
                // Sync the first shard's entry even if a concurrent creator made it.
                create_durable_dir(grandparent)?;
            }
            create_durable_dir(parent)?;
            let tmp = inner
                .root
                .join("tmp")
                .join(format!("{filename}.{}", nonce()?));
            let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
            let result = (|| {
                file.write_all(&body)?;
                file.sync_all()?;
                drop(file);
                fs::rename(&tmp, &destination)?;
                sync_dir(parent)?;
                Ok(())
            })();
            if result.is_err() {
                // Startup recovery also removes a temporary file if cleanup fails.
                let _ = fs::remove_file(&tmp);
            }
            result
        })
        .await
    }

    /// Check whether a stored key exists.
    ///
    /// # Errors
    /// Returns `InvalidKey` or an I/O error other than a missing key.
    pub async fn exists(&self, key: &str) -> Result<bool, Error> {
        match self.stat(key).await {
            Ok(_) => Ok(true),
            Err(Error::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Remove a blob and sync its parent; missing keys succeed.
    ///
    /// # Errors
    /// Returns `InvalidKey` or an I/O error other than a missing key.
    pub async fn delete(&self, key: &str) -> Result<(), Error> {
        let path = blob_path(&self.inner.root, key)?;
        blocking(move || match fs::remove_file(&path) {
            Ok(()) => Ok(sync_dir(path.parent().expect("blob path has a parent"))?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        })
        .await
    }

    /// Read one blob's size and modification time without enumerating storage.
    ///
    /// # Errors
    /// Returns `InvalidKey`, `NotFound`, or an I/O error.
    pub async fn stat(&self, key: &str) -> Result<BlobEntry, Error> {
        let path = blob_path(&self.inner.root, key)?;
        let key = key.to_owned();
        blocking(move || entry(&path, key)).await
    }

    /// List valid blobs in their canonical shards, excluding temporary and unexpected files.
    ///
    /// # Errors
    /// Returns an error if walking storage or reading metadata fails. A blob removed
    /// during enumeration is skipped.
    pub async fn list(&self) -> Result<Vec<BlobEntry>, Error> {
        let inner = self.inner.clone();
        blocking(move || {
            let mut entries = Vec::new();
            for (base, pooled) in [("objects", false), ("pool", true)] {
                for first in fs::read_dir(inner.root.join(base))? {
                    let first = first?;
                    if !hex_directory(&first)? {
                        continue;
                    }
                    if pooled {
                        list_leaf(&inner.root, &first.path(), true, &mut entries)?;
                    } else {
                        for second in fs::read_dir(first.path())? {
                            let second = second?;
                            if hex_directory(&second)? {
                                list_leaf(&inner.root, &second.path(), false, &mut entries)?;
                            }
                        }
                    }
                }
            }
            Ok(entries)
        })
        .await
    }
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| Error::Io(io::Error::other(error)))?
}

fn blob_path(root: &Path, key: &str) -> Result<PathBuf, Error> {
    let (bare, pooled) = key
        .strip_prefix(POOL_MARK)
        .map_or((key, false), |bare| (bare, true));
    let hex = bare.strip_prefix("b1-").ok_or(Error::InvalidKey)?;
    if hex.len() != 32 || !hex.bytes().all(lower_hex) {
        return Err(Error::InvalidKey);
    }
    if pooled {
        Ok(root.join("pool").join(&hex[..2]).join(encoded_key(key)))
    } else {
        Ok(root
            .join("objects")
            .join(&hex[..2])
            .join(&hex[2..4])
            .join(key))
    }
}

fn lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn encoded_key(key: &str) -> String {
    key.replace('$', "%24")
}

fn nonce() -> Result<String, Error> {
    let mut bytes = [0; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| Error::Io(io::Error::other(error.to_string())))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn sync_dir(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn create_durable_dir(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        if !parent.is_dir() {
            create_durable_dir(parent)?;
        }
    }
    fs::create_dir_all(path)?;
    if let Some(parent) = parent {
        sync_dir(parent)?;
    }
    Ok(())
}

fn entry(path: &Path, key: String) -> Result<BlobEntry, Error> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(Error::Io(io::Error::other("blob is not a regular file")));
    }
    Ok(BlobEntry {
        key,
        size: metadata.len(),
        last_modified: Some(metadata.modified()?),
        generation: None,
    })
}

fn hex_directory(item: &fs::DirEntry) -> io::Result<bool> {
    Ok(item.file_type()?.is_dir()
        && item
            .file_name()
            .to_str()
            .is_some_and(|name| name.len() == 2 && name.bytes().all(lower_hex)))
}

fn list_leaf(
    root: &Path,
    directory: &Path,
    pooled: bool,
    entries: &mut Vec<BlobEntry>,
) -> Result<(), Error> {
    for item in fs::read_dir(directory)? {
        let item = item?;
        if !item.file_type()?.is_file() {
            continue;
        }
        let name = item.file_name();
        let Some(name) = name.to_str() else { continue };
        let key = if pooled {
            let Some(bare) = name.strip_prefix(ENCODED_POOL_MARK) else {
                continue;
            };
            format!("{POOL_MARK}{bare}")
        } else {
            name.to_owned()
        };
        if blob_path(root, &key).ok().as_ref() != Some(&item.path()) {
            continue;
        }
        match entry(&item.path(), key) {
            Ok(entry) => entries.push(entry),
            Err(Error::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
