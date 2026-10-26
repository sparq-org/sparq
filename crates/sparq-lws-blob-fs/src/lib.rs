//! Synchronous filesystem storage for immutable opaque blob keys.
//!
//! The caller must hold the exclusive data-directory lock before [`FsBlobStore::open`]
//! and until every handle is dropped. Opening sweeps temporary writes. The directory
//! must be controlled by the caller; this API does not protect against another actor
//! changing its files or symlinks. The filesystem must support hard links and directory sync.

#![forbid(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// The marker for service-pool blob keys.
pub const POOLED_BLOB_KEY_MARK: &str = sparq_lws_blob_key::POOL_MARK;

/// A stored blob and its filesystem modification time.
#[derive(Debug, Clone)]
pub struct BlobEntry {
    /// The complete opaque key, including the pool marker when present.
    pub key: String,
    /// Always `Some(file mtime)`; listing fails if it cannot be read.
    pub last_modified: Option<SystemTime>,
    /// Always `None`: immutable filesystem keys have no generation witness.
    pub generation: Option<u64>,
}

/// Published blobs and foreign filesystem entries skipped during listing.
#[derive(Debug, Default)]
pub struct Listing {
    /// Blobs in their expected layout, sorted by key.
    pub blobs: Vec<BlobEntry>,
    /// Invalid names, misplaced keys and non-files, sorted by path.
    /// Unexpected fan-out directories are reported without traversing them.
    pub foreign: Vec<PathBuf>,
}

const OBJECT_LEAF_OFFSET: usize = 256;
const POOL_DIRECTORY_OFFSET: usize = OBJECT_LEAF_OFFSET + 256 * 256;
const FANOUT_DIRECTORIES: usize = POOL_DIRECTORY_OFFSET + 256;

#[derive(Debug)]
struct SyncedDirectories {
    bits: Box<[AtomicU64]>,
    initializing: Mutex<()>,
}

impl SyncedDirectories {
    fn new() -> Self {
        Self {
            bits: (0..FANOUT_DIRECTORIES / 64)
                .map(|_| AtomicU64::new(0))
                .collect(),
            initializing: Mutex::new(()),
        }
    }

    fn ensure(&self, path: &Path, index: usize) -> io::Result<()> {
        let word = &self.bits[index / 64];
        let mask = 1 << (index % 64);
        if word.load(Ordering::Acquire) & mask == 0 {
            // Serialize only first use, then recheck for a concurrent initializer.
            let _guard = self
                .initializing
                .lock()
                .map_err(|_| io::Error::other("fan-out initialization lock poisoned"))?;
            if word.load(Ordering::Acquire) & mask == 0 {
                // AlreadyExists must still sync: another writer may have created
                // the directory without making its parent entry durable.
                ensure_dir(path)?;
                word.fetch_or(mask, Ordering::Release);
            }
        }
        Ok(())
    }
}

/// A std-only blob store, independent of the server's async storage traits.
#[derive(Debug, Clone)]
pub struct FsBlobStore {
    root: PathBuf,
    synced: Arc<SyncedDirectories>,
}

impl FsBlobStore {
    /// Create the layout and remove abandoned writes from `tmp/`.
    ///
    /// The caller must already hold the exclusive data-directory lock and retain
    /// it for this store's lifetime. Never open while another handle is writing.
    pub fn open(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = std::path::absolute(root)?;
        ensure_dir(&root)?;
        for name in ["objects", "pool", "tmp"] {
            ensure_dir(&root.join(name))?;
        }
        for entry in fs::read_dir(root.join("tmp"))? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        }
        sync_dir(&root.join("tmp"))?;
        Ok(Self {
            root,
            synced: Arc::new(SyncedDirectories::new()),
        })
    }

    /// Create a blob. An existing key is an error, even when its bytes match.
    ///
    /// Keys must be unique per write and must never be reused, even after deletion.
    /// The bytes are synced before a no-overwrite hard-link publication, and the
    /// final parent is synced before success. An error after publication can leave
    /// an orphan for reconciliation; callers must not retry with the same key.
    /// Fan-out directories and their parent entries are synced on first use per
    /// open store, shared across clones. Failed directory syncs are retried.
    pub fn put(&self, key: &str, body: &[u8]) -> io::Result<()> {
        let path = self.path(key)?;
        let parent = path.parent().expect("blob path has a parent");
        let (random, pooled) = parse_key(key)?;
        let first = usize::from_str_radix(&random[..2], 16).expect("validated hex pair");
        if pooled {
            self.synced.ensure(parent, POOL_DIRECTORY_OFFSET + first)?;
        } else {
            self.synced
                .ensure(parent.parent().expect("fan-out path has a parent"), first)?;
            let second = usize::from_str_radix(&random[2..4], 16).expect("validated hex pair");
            self.synced
                .ensure(parent, OBJECT_LEAF_OFFSET + first * 256 + second)?;
        }
        let (temporary, mut file) = self.create_temporary()?;
        file.write_all(body)?;
        sync_file(&file, &temporary.0)?;
        fs::hard_link(&temporary.0, &path)?;
        fs::remove_file(&temporary.0)?;
        sync_dir(parent)?;
        Ok(())
    }

    /// Read a blob. An absent key returns [`io::ErrorKind::NotFound`].
    pub fn get(&self, key: &str) -> io::Result<Vec<u8>> {
        fs::read(self.path(key)?)
    }

    /// Whether a blob exists. Invalid keys and filesystem errors propagate.
    pub fn exists(&self, key: &str) -> io::Result<bool> {
        Ok(self.stat(key)?.is_some())
    }

    /// Unlink a blob and sync its parent. Deleting an absent key succeeds.
    pub fn delete(&self, key: &str) -> io::Result<()> {
        let path = self.path(key)?;
        match fs::remove_file(&path) {
            Ok(()) => sync_dir(path.parent().expect("blob path has a parent")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Read one blob's modification time, or `None` if absent.
    pub fn stat(&self, key: &str) -> io::Result<Option<BlobEntry>> {
        let path = self.path(key)?;
        match fs::metadata(path) {
            Ok(meta) => Ok(Some(BlobEntry {
                key: key.to_string(),
                last_modified: Some(meta.modified()?),
                generation: None,
            })),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// List published keys with modification times. Temporary writes are excluded.
    /// Foreign entries are reported separately; blobs concurrently removed are skipped.
    /// Filesystem I/O errors still propagate.
    pub fn list(&self) -> io::Result<Listing> {
        let mut listing = Listing::default();
        for (directory, depth) in [("objects", 2), ("pool", 1)] {
            self.list_directory(&self.root.join(directory), depth, &mut listing)?;
        }
        listing.blobs.sort_by(|a, b| a.key.cmp(&b.key));
        listing.foreign.sort();
        Ok(listing)
    }

    fn path(&self, key: &str) -> io::Result<PathBuf> {
        // Validate the entire key before constructing any path from it.
        let (random, pooled) = parse_key(key)?;
        Ok(if pooled {
            self.root.join("pool").join(&random[..2]).join(key)
        } else {
            self.root
                .join("objects")
                .join(&random[..2])
                .join(&random[2..4])
                .join(key)
        })
    }

    fn create_temporary(&self) -> io::Result<(Temporary, File)> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let name = format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let path = self.root.join("tmp").join(name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((Temporary(path), file)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }

    fn list_directory(
        &self,
        directory: &Path,
        depth: usize,
        listing: &mut Listing,
    ) -> io::Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            let name = entry.file_name();
            if depth > 0
                && kind.is_dir()
                && name.to_str().is_some_and(|name| {
                    name.len() == 2 && name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                })
            {
                self.list_directory(&path, depth - 1, listing)?;
            } else if depth == 0 && kind.is_file() {
                let Some(key) = name
                    .to_str()
                    .filter(|key| self.path(key).is_ok_and(|expected| expected == path))
                else {
                    listing.foreign.push(path);
                    continue;
                };
                if let Some(blob) = self.stat(key)? {
                    listing.blobs.push(blob);
                }
            } else {
                listing.foreign.push(path);
            }
        }
        Ok(())
    }
}

fn parse_key(key: &str) -> io::Result<(&str, bool)> {
    let (bare, pooled) = sparq_lws_blob_key::parse(key).ok_or_else(invalid_key)?;
    Ok((&bare[sparq_lws_blob_key::PREFIX.len()..], pooled))
}

fn invalid_key() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid blob key")
}

fn sync_dir(path: &Path) -> io::Result<()> {
    sync_file(&File::open(path)?, path)
}

fn sync_file(file: &File, _path: &Path) -> io::Result<()> {
    #[cfg(test)]
    tests::before_sync(_path)?;
    file.sync_all()
}

fn ensure_dir(path: &Path) -> io::Result<()> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            ensure_dir(path.parent().ok_or(e)?)?;
            return ensure_dir(path);
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    // Sync even on AlreadyExists: another writer may have just created the
    // directory and not yet synced its entry in the parent.
    sync_dir(path)?;
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

struct Temporary(PathBuf);

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests;
