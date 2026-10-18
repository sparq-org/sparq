//! The lock on a persisted graph directory: one writable holder at a time may open, create,
//! save into, or recover a directory, so two processes (or two opens in one process) never
//! replay, truncate, compact or rewrite the same files at once. Read-only opens
//! (`Graph::open_read_only`) share the lock with each other, never with a writable holder.
//!
//! The lock is an advisory lock ([`fs4::FileExt::try_lock`] / `try_lock_shared`: `flock` on
//! Unix, `LockFileEx` on Windows) on a lock file. It is held as long as its [`DirLock`] lives
//! (for a graph: while the graph, or any named graph taken out of it, lives) and released by
//! the operating system when the process ends, however it ends, so it never outlives its
//! process. A second attempt fails at once ([`std::io::ErrorKind::WouldBlock`]) rather than
//! wait. Advisory locks over network file systems (NFS and the like) are not relied on: such
//! directories are not supported.
//!
//! The lock file lives beside the directory, so the parent directory must be writable (at least
//! once, to create it). It is empty and harmless to keep: never delete it as "stale" while the
//! directory may be in use, since a holder keeps the lock on the file it opened, and a new file
//! at that path would let a second holder in.
//!
//! The flat layout, whose directory compaction renames, locks a sibling file named for the
//! directory's resolved path ([`LockLocation::sibling_of`]), so one directory
//! reached through a symbolic link, a `..` detour or a trailing slash is one lock. The graph
//! opened from it keeps that one resolved path for its log, its recovery and its swaps too. Two
//! bind mounts of one directory have different parents, and so different sibling locks: that
//! alias is not excluded on the flat layout.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// `dir` resolved: its canonical path (symbolic links and `.`/`..` resolved, any trailing slash
/// dropped). A directory that does not exist yet resolves through its parent, which must; a
/// dangling symbolic link is an error, not a directory to create.
pub(crate) fn resolve_dir(dir: &Path) -> io::Result<PathBuf> {
    match dir.canonicalize() {
        Ok(c) => Ok(c),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if dir.symlink_metadata().is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "{} is a dangling symbolic link: open the directory by its real path",
                        dir.display()
                    ),
                ));
            }
            let name = dir.file_name().ok_or_else(|| unnamed(dir))?;
            let parent = match dir.parent() {
                Some(p) if !p.as_os_str().is_empty() => p.canonicalize()?,
                _ => std::env::current_dir()?.canonicalize()?,
            };
            Ok(parent.join(name))
        }
        Err(e) => Err(e),
    }
}

/// Whether `a` and `b` name one existing file or directory (device and inode on Unix, volume
/// and file index on Windows), however they are spelled. `false` when either is missing.
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    same_file::is_same_file(a, b).unwrap_or(false)
}

/// Where a directory's lock file is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockLocation(PathBuf);

impl LockLocation {
    /// The lock file at `path`, for the directory `dir`: refused when `path` is inside `dir`
    /// (the directory's own files are replaced wholesale by a swap, so a lock among them would
    /// not exclude anything).
    pub(crate) fn at(dir: &Path, path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let dir = resolve_dir(dir)?;
        let resolved = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(parent) => resolve_dir(parent)?,
            None => std::env::current_dir()?.canonicalize()?,
        };
        if resolved.starts_with(&dir) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "the lock file {} is inside the directory {} it locks",
                    path.display(),
                    dir.display()
                ),
            ));
        }
        Ok(Self(path))
    }

    /// The flat layout's lock for the directory `dir`: `<dir>.lock` beside it, named for its
    /// resolved path (symbolic links and `.`/`..` resolved, any trailing slash dropped; a
    /// dangling symbolic link is an error). The directory need not exist yet; its parent must.
    pub fn sibling_of(dir: &Path) -> io::Result<Self> {
        let resolved = resolve_dir(dir)?;
        let mut name = resolved
            .file_name()
            .ok_or_else(|| unnamed(dir))?
            .to_os_string();
        name.push(".lock");
        Self::at(&resolved, resolved.with_file_name(name))
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
    shared: bool,
}

impl DirLock {
    /// Take the lock at `location` exclusively, creating its file if need be: `WouldBlock` when
    /// another holder (another process, or another open in this one, shared or exclusive) has
    /// it.
    pub fn acquire(location: LockLocation) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(location.path())?;
        Self::lock(file, location, false)
    }

    /// Take the lock at `location` shared, for a read-only open: several shared holders
    /// coexist, but none while an exclusive holder has it (`WouldBlock`), and an exclusive
    /// holder cannot take it while any shared one does. An existing lock file is opened for
    /// reading only; a missing one is created, which needs the parent directory writable.
    pub fn acquire_shared(location: LockLocation) -> io::Result<Self> {
        let file = match File::open(location.path()) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(location.path())?,
            Err(e) => return Err(e),
        };
        Self::lock(file, location, true)
    }

    fn lock(file: File, location: LockLocation, shared: bool) -> io::Result<Self> {
        let locked = if shared {
            fs4::FileExt::try_lock_shared(&file)
        } else {
            fs4::FileExt::try_lock(&file)
        };
        match locked {
            Ok(()) => Ok(Self {
                _file: file,
                location,
                shared,
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

    /// Whether this is a shared (read-only) hold.
    pub fn is_shared(&self) -> bool {
        self.shared
    }

    /// Where this lock is.
    pub fn location(&self) -> &LockLocation {
        &self.location
    }

    /// Whether this is the lock at `location`: the same lock file by identity, not by spelling
    /// (so the `C:\` and `\\?\C:\` spellings of one Windows path agree).
    pub(crate) fn is_at(&self, location: &LockLocation) -> bool {
        same_file(self.location.path(), location.path())
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
            assert!(held.is_at(&location), "{}", alias.display());
        }
        drop(held);
        std::fs::remove_dir_all(&root).ok();
    }

    /// A symbolic link to nothing is an error, not a directory still to be created.
    #[cfg(unix)]
    #[test]
    fn a_dangling_symbolic_link_is_an_error() {
        let root = scratch("dangling");
        let link = root.join("link");
        std::os::unix::fs::symlink(root.join("gone"), &link).unwrap();
        let err = LockLocation::sibling_of(&link).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
        assert!(resolve_dir(&link).is_err());
        // A missing directory under a real parent still resolves.
        assert_eq!(
            resolve_dir(&root.join("new")).unwrap(),
            root.canonicalize().unwrap().join("new")
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// A lock file inside the directory it locks is refused; one beside it is not.
    #[test]
    fn a_lock_inside_its_directory_is_refused() {
        let root = scratch("inside");
        let dir = root.join("g");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for inside in [dir.join("x.lock"), dir.join("sub").join("x.lock")] {
            let err = LockLocation::at(&dir, &inside).unwrap_err();
            assert_eq!(
                err.kind(),
                io::ErrorKind::InvalidInput,
                "{}",
                inside.display()
            );
        }
        LockLocation::at(&dir, root.join("g.other-lock")).expect("beside the directory");
        std::fs::remove_dir_all(&root).ok();
    }

    /// A bare relative name resolves against the canonical working directory.
    #[test]
    fn a_bare_name_resolves_against_the_canonical_working_directory() {
        let name = format!("sparq-dirlock-bare-{}", std::process::id());
        let resolved = resolve_dir(Path::new(&name)).unwrap();
        assert_eq!(
            resolved,
            std::env::current_dir()
                .unwrap()
                .canonicalize()
                .unwrap()
                .join(&name)
        );
    }
}
