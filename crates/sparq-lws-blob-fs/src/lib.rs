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
use std::time::SystemTime;

/// The marker for service-pool blob keys.
pub const POOLED_BLOB_KEY_MARK: &str = "$pool$";

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

/// A std-only blob store, independent of the server's async storage traits.
#[derive(Debug, Clone)]
pub struct FsBlobStore {
    root: PathBuf,
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
        Ok(Self { root })
    }

    /// Create a blob. An existing key is an error, even when its bytes match.
    ///
    /// Keys must be unique per write and must never be reused, even after deletion.
    /// The bytes are synced before a no-overwrite hard-link publication, and the
    /// final parent is synced before success. An error after publication can leave
    /// an orphan for reconciliation; callers must not retry with the same key.
    pub fn put(&self, key: &str, body: &[u8]) -> io::Result<()> {
        let path = self.path(key)?;
        let parent = path.parent().expect("blob path has a parent");
        // A concurrent writer may have created the first fan-out directory but
        // not synced its entry yet. Establish that entry before the next level.
        ensure_dir(parent.parent().expect("fan-out path has a parent"))?;
        ensure_dir(parent)?;
        let (temporary, mut file) = self.create_temporary()?;
        file.write_all(body)?;
        file.sync_all()?;
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
    /// Blobs concurrently removed while listing are skipped.
    pub fn list(&self) -> io::Result<Vec<BlobEntry>> {
        let mut entries = Vec::new();
        for (directory, depth) in [("objects", 2), ("pool", 1)] {
            self.list_directory(&self.root.join(directory), depth, &mut entries)?;
        }
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(entries)
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
        entries: &mut Vec<BlobEntry>,
    ) -> io::Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if depth > 0 && kind.is_dir() {
                self.list_directory(&entry.path(), depth - 1, entries)?;
            } else if depth == 0 && kind.is_file() {
                let key = entry.file_name();
                let key = key.to_str().ok_or_else(invalid_key)?;
                // Refuse a valid key stored in the wrong layout too.
                if self.path(key)? != entry.path() {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "misplaced blob"));
                }
                if let Some(blob) = self.stat(key)? {
                    entries.push(blob);
                }
            }
        }
        Ok(())
    }
}

fn parse_key(key: &str) -> io::Result<(&str, bool)> {
    let unmarked = key.strip_prefix(POOLED_BLOB_KEY_MARK);
    let random = unmarked
        .unwrap_or(key)
        .strip_prefix("b1-")
        .ok_or_else(invalid_key)?;
    if random.len() != 32
        || !random
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(invalid_key());
    }
    Ok((random, unmarked.is_some()))
}

fn invalid_key() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid blob key")
}

fn sync_dir(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
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
