//! The exclusive lock on a persisted graph directory: one holder at a time may open, create,
//! save into, or recover a directory, so two processes (or two opens in one process) never
//! replay, truncate, compact or rewrite the same files at once.
//!
//! The lock is an advisory lock ([`fs4::FileExt::try_lock`]: `flock` on Unix, `LockFileEx` on
//! Windows) on a lock file. It is held as long as its [`DirLock`] lives and released by the
//! operating system when the process ends, however it ends, so it is never lost while its
//! holder runs and never outlives it. A second attempt fails at once
//! ([`std::io::ErrorKind::WouldBlock`]) rather than wait. Advisory locks over network file
//! systems (NFS and the like) are not relied on: such directories are not supported.
//!
//! Where the lock file is is the caller's choice ([`LockLocation`]). The flat layout, whose
//! directory compaction renames, locks a sibling file named for the directory's canonical
//! path ([`LockLocation::sibling_of`]), so one directory reached through a symbolic link or
//! a trailing slash is one lock. Two bind mounts of one directory have different parents,
//! and so different sibling locks: that alias is not excluded on the flat layout.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// Where a directory's lock file is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockLocation(PathBuf);

impl LockLocation {
    /// The lock file at `path`, exactly.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    /// The flat layout's lock for the directory `dir`: `<dir>.lock` beside it, named for its
    /// canonical path (symbolic links and `.`/`..` resolved, any trailing slash dropped). The
    /// directory need not exist yet; its parent must.
    pub fn sibling_of(dir: &Path) -> io::Result<Self> {
        let canonical = match dir.canonicalize() {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let name = dir.file_name().ok_or_else(|| unnamed(dir))?;
                let parent = match dir.parent() {
                    Some(p) if !p.as_os_str().is_empty() => p.canonicalize()?,
                    _ => std::env::current_dir()?,
                };
                parent.join(name)
            }
            Err(e) => return Err(e),
        };
        let mut name = canonical
            .file_name()
            .ok_or_else(|| unnamed(dir))?
            .to_os_string();
        name.push(".lock");
        Ok(Self(canonical.with_file_name(name)))
    }

    /// The lock file's path.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

fn unnamed(dir: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{} names no directory to lock", dir.display()),
    )
}

/// An exclusive lock on a graph directory, held while this lives.
#[derive(Debug)]
pub struct DirLock {
    _file: File,
    location: LockLocation,
}

impl DirLock {
    /// Take the lock at `location`, creating its file if need be: `WouldBlock` when another
    /// holder (another process, or another open in this one) has it.
    pub fn acquire(location: LockLocation) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(location.path())?;
        match fs4::FileExt::try_lock(&file) {
            Ok(()) => Ok(Self {
                _file: file,
                location,
            }),
            Err(fs4::TryLockError::WouldBlock) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "{} is locked: the directory is in use by another holder",
                    location.path().display()
                ),
            )),
            Err(fs4::TryLockError::Error(e)) => Err(e),
        }
    }

    /// Where this lock is.
    pub fn location(&self) -> &LockLocation {
        &self.location
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sparq-dirlock-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// One holder at a time, released when dropped.
    #[test]
    fn a_directory_has_one_holder_at_a_time() {
        let root = scratch("one");
        let dir = root.join("g");
        let first = DirLock::acquire(LockLocation::sibling_of(&dir).unwrap()).unwrap();
        let second = DirLock::acquire(LockLocation::sibling_of(&dir).unwrap());
        assert_eq!(second.unwrap_err().kind(), io::ErrorKind::WouldBlock);
        drop(first);
        DirLock::acquire(LockLocation::sibling_of(&dir).unwrap()).expect("released");
        std::fs::remove_dir_all(&root).ok();
    }

    /// A trailing slash, a `..` detour and a symbolic link name one lock.
    #[test]
    fn aliases_of_a_directory_share_its_lock() {
        let root = scratch("alias");
        let dir = root.join("g");
        std::fs::create_dir_all(&dir).unwrap();
        let held = DirLock::acquire(LockLocation::sibling_of(&dir).unwrap()).unwrap();
        let mut aliases = vec![
            PathBuf::from(format!("{}/", dir.display())),
            root.join("g").join("..").join("g"),
        ];
        #[cfg(unix)]
        {
            let link = root.join("link");
            std::os::unix::fs::symlink(&dir, &link).unwrap();
            aliases.push(link);
        }
        for alias in aliases {
            let location = LockLocation::sibling_of(&alias).unwrap();
            assert_eq!(&location, held.location(), "{}", alias.display());
        }
        drop(held);
        std::fs::remove_dir_all(&root).ok();
    }
}
