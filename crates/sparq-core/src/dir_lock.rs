//! The lock on a persisted graph directory: one writable holder at a time may open, create,
//! save into, or recover a directory, so two processes (or two opens in one process) never
//! replay, truncate, compact or rewrite the same files at once. Read-only opens
//! (`Graph::open_read_only`) share the lock with each other, never with a writable holder.
//!
//! The lock is an advisory lock ([`fs4::FileExt::try_lock`] / `try_lock_shared`: `flock` on
//! Unix, `LockFileEx` on Windows) on a lock file. It is held as long as its [`DirLock`] lives
//! (for a graph: while the graph, or any mapped view of it, lives). The owning process
//! explicitly unlocks both files before closing them. On Unix, `flock` belongs to an open file
//! description: forked or duplicated descriptors share its lock, so closing the parent's file
//! alone would leave it held until every duplicate closes. Explicit unlock releases that shared
//! lock immediately, even while a child is between fork and exec. An inherited guard skips
//! unlocking when dropped in a forked child, so it cannot release the parent's live lock.
//! The operating system also releases the lock when its final descriptor closes.
//! A second attempt fails at once ([`std::io::ErrorKind::WouldBlock`]) rather than wait.
//! The lock never waits or bypasses a live holder. Advisory locks over network file systems
//! (NFS and the like) are not relied on: such directories are not supported.
//!
//! A store's lock is two lock files ([`DirLock`]): `<dir>.lock` beside the directory, taken
//! when it exists or can be created, then `sparq.lock` inside it, always. A store whose parent
//! directory is not writable (a mount root, say) is held by its inside lock alone: it opens,
//! is written and read as any other, but a compaction or restore, which renames the directory,
//! needs the outside lock and is refused, naming the parent. A filesystem or drive root has no
//! sibling namespace and uses only its inside lock; keep a store in a subdirectory when it
//! needs operations that rename its directory. Both files are empty and harmless to keep:
//! never delete either as "stale" while the directory may be in use, since a holder
//! keeps the lock on the file it opened, and a new file at that path would let a second holder
//! in. Anyone who can open a lock file can take a shared lock on it and so keep a writer out.
//!
//! The flat layout, whose directory compaction renames, locks a sibling file named for the
//! directory's resolved path ([`LockLocation::sibling_of`]), so one directory
//! reached through a symbolic link, a `..` detour or a trailing slash is one lock. The graph
//! opened from it keeps that one resolved path for its log, its recovery and its swaps too. Two
//! bind mounts of one directory have different parents, and so different sibling locks: that
//! alias is not excluded on the flat layout.
//!
//! The lock guards a store's files only because they are reachable through nothing but the
//! `Graph` that holds it: the openers of a store's dictionary and indexes are crate-internal, and
//! every memory-mapped part of a graph (its indexes, its dictionary, and every fork, snapshot or
//! clone of them) keeps the lock while it lives.
//!
//! Ownership is per root store and checked when a directory is claimed (opened, saved into,
//! built or restored): a path inside a store is refused, and a whole-store write refuses a
//! directory holding another store, or one it cannot tell is its own. These checks see the disk
//! as it is when they run and reserve nothing; separate processes creating nested stores (one
//! inside the other's directory) at the same time are not supported.
//!
//! The whole design is advisory, over a namespace the processes using it trust: it keeps
//! well-behaved holders apart and catches mistakes, not someone racing the filesystem. Paths
//! are checked, then opened by name (no `O_NOFOLLOW` or `openat`), so a change between the
//! check and the open is not caught; and each of a store's files is published on its own, so a
//! save or build is not atomic across files (the directory swaps replace a store as a whole).

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// `dir` resolved: its canonical path (symbolic links and `.`/`..` resolved, any trailing slash
/// dropped). A directory that does not exist yet resolves through its nearest existing
/// ancestor, creating nothing; a dangling symbolic link is an error, not a directory to create.
pub(crate) fn resolve_dir(dir: &Path) -> io::Result<PathBuf> {
    // The nearest ancestor that exists (as anything, a link included), and the names below it.
    let mut missing = Vec::new();
    let mut existing = dir.to_path_buf();
    loop {
        match existing.symlink_metadata() {
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                missing.push(
                    existing
                        .file_name()
                        .ok_or_else(|| unnamed(dir))?
                        .to_os_string(),
                );
                existing = match existing.parent() {
                    Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
                    _ => std::env::current_dir()?,
                };
            }
            Err(e) => return Err(e),
        }
    }
    let mut resolved = existing.canonicalize().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} is a dangling symbolic link: open the directory by its real path",
                    existing.display()
                ),
            )
        } else {
            e
        }
    })?;
    for name in missing.iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

/// Resolves a store directory, rejecting any existing non-directory before taking its lock.
pub(crate) fn resolve_store_dir(dir: &Path) -> io::Result<PathBuf> {
    #[cfg(windows)]
    refuse_windows_device_path(dir)?;
    let check = |path: &Path| match path.metadata() {
        Ok(meta) if !meta.is_dir() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a graph store directory", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    };
    // Inspect the supplied path first: a descriptor path can resolve to a missing
    // pathname. It still names an existing file, not a directory to create.
    check(dir)?;
    let resolved = resolve_dir(dir)?;
    check(&resolved)?;
    Ok(resolved)
}

/// Whether `a` and `b` name one existing file or directory (device and inode on Unix, volume
/// and file index on Windows), however they are spelled. `false` when either is missing.
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    same_file::is_same_file(a, b).unwrap_or(false)
}

/// Whether the resolved directory has no parent or sibling namespace (including drive roots).
pub(crate) fn is_filesystem_root(dir: &Path) -> bool {
    dir.is_absolute() && dir.parent().is_none()
}

/// What a memory-mapped store backing holds while it lives: the lock of the store directory
/// its files are in (`None` for a backing that maps nothing from a store). Every copy of a
/// backing (a fork, a snapshot, a graph rebuilt around a dictionary) carries it, so a mapping
/// never outlives its lock.
pub(crate) type Pin = Option<std::sync::Arc<DirLock>>;

/// Whether `path` (a file or a directory, existing or not) is a graph store directory or inside
/// one: its resolved path, or an ancestor of it, holds a store marker or a legacy store's
/// files. A tool writing an output file refuses such a path, since a store's files belong to the
/// graph that holds its lock. Checked when called: nothing stops a store being created there
/// afterwards.
///
/// Fails closed: a path that cannot be resolved (a dangling symbolic link, a magic link such as
/// `/dev/stdout` on a pipe, a component that cannot be read) is an error, never checked as
/// spelled. A path that does not exist yet is resolved through its nearest existing ancestor.
/// This checks where the path leads, not which file it names: a hard link elsewhere to a store's
/// file is not detected, so a tool replaces its output by renaming a fresh file over the path
/// (which replaces the link) rather than writing through it.
pub fn is_inside_store(path: &Path) -> io::Result<bool> {
    Ok(store_root(path)?.is_some())
}

/// The outermost graph store directory that `path` (a file or a directory, existing or not) is,
/// or is inside, after resolving it as [`is_inside_store`] does (an error when it cannot be
/// resolved); `None` when it is in no store. For a store's own file that is the store's root:
/// the directory to open (or open read-only) to hold its lock.
pub fn store_root(path: &Path) -> io::Result<Option<PathBuf>> {
    let resolved = resolve_dir(path)?;
    Ok(resolved
        .ancestors()
        .filter(|a| crate::is_store(a))
        .last()
        .map(Path::to_path_buf))
}

/// A file name as the filesystem compares it, for checking it against the names a store
/// reserves: uppercased then lowercased, also matching Unicode aliases of reserved ASCII
/// letters (macOS and Windows filesystems ignore case by default), and on
/// Windows without trailing dots and spaces (which Windows drops from a name).
pub(crate) fn comparable_name(name: &str) -> String {
    #[cfg(windows)]
    let name = name.trim_end_matches(['.', ' ']);
    name.to_uppercase().to_lowercase()
}

/// The names a store reserves beside or inside itself: its marker, staging record and inside
/// lock, its scratch (`.sparq-*`), the staging siblings of a swap (`<dir>.compact-new`,
/// `.compact-old`, `.drop-new`, `.drop-old`) and the scratch a staging directory is made under
/// (`*.sparq-part`). `name` is a [`comparable_name`].
fn is_reserved_name(name: &str) -> bool {
    name == crate::STORE_MARKER
        || name == crate::STAGING_MARKER
        || name == INSIDE_LOCK
        || name.starts_with(".sparq-")
        || [
            ".compact-new",
            ".compact-old",
            ".drop-new",
            ".drop-old",
            crate::STAGING_SCRATCH_SUFFIX,
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

/// Whether the entry at `entry` (resolved parent, then its own name) is a store's, inside one,
/// or a name a store reserves: a store, or anything in one; a reserved name; a lock file
/// (`<name>.lock`, in any case) of something at `<name>` or of a reserved name; or a regular
/// file some holder has locked now.
fn is_reserved_entry(entry: &Path) -> io::Result<bool> {
    if entry.ancestors().any(crate::is_store) {
        return Ok(true);
    }
    let Some(name) = entry.file_name() else {
        return Ok(false);
    };
    let name = comparable_name(&name.to_string_lossy());
    if is_reserved_name(&name) {
        return Ok(true);
    }
    if let Some(stem) = name.strip_suffix(".lock").filter(|s| !s.is_empty()) {
        // The stem as given (before case folding), so the check of what exists at it uses the
        // spelling the filesystem compares.
        let given = entry.file_name().map(|n| n.to_string_lossy().into_owned());
        let stem = given
            .as_deref()
            .and_then(|given| {
                #[cfg(windows)]
                let given = given.trim_end_matches(['.', ' ']);
                given.rsplit_once('.').map(|(stem, _)| stem)
            })
            .unwrap_or(stem);
        let locked = entry.with_file_name(stem);
        if locked.symlink_metadata().is_ok() || is_reserved_entry(&locked)? {
            return Ok(true);
        }
    }
    if entry.symlink_metadata().is_ok_and(|m| m.is_file()) && DirLock::is_held(entry)? {
        return Ok(true);
    }
    Ok(false)
}

/// Whether a tool must refuse to replace the file at `path` with its output: the path, or
/// where it leads, is a store, inside one, or a name a store reserves beside itself (its
/// `<dir>.lock` lock file, the staging siblings of a swap, its marker, staging record or
/// `.sparq-*` scratch; a `<name>.lock` of anything that exists at `<name>`, or that some holder
/// has locked now). Checked twice: the entry itself (its parent resolved, its own name as
/// given), so a symbolic link inside a store is refused even when it points outside, and, when
/// the entry exists, the file it resolves to.
///
/// Fails closed: a path whose parent or target cannot be resolved (a dangling symbolic link
/// included) is an error. Checked when called, like [`is_inside_store`]; a hard link elsewhere
/// to a store's file is not detected, which is why a tool replaces its output by renaming a
/// fresh file over the path rather than writing through it.
pub fn is_reserved_store_path(path: &Path) -> io::Result<bool> {
    let name = path.file_name().ok_or_else(|| unnamed(path))?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let entry = resolve_dir(parent)?.join(name);
    if is_reserved_entry(&entry)? {
        return Ok(true);
    }
    match entry.symlink_metadata() {
        Ok(meta) if meta.file_type().is_symlink() => is_reserved_entry(&resolve_dir(&entry)?),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
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
        let dir = resolve_store_dir(dir)?;
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

    /// The lock namespace for a resolved store: its sibling, or its inside lock at a
    /// filesystem root, where no sibling exists. Touches nothing on disk.
    pub(crate) fn beside(resolved: &Path) -> io::Result<Self> {
        if is_filesystem_root(resolved) {
            return Ok(Self(resolved.join(INSIDE_LOCK)));
        }
        let mut name = resolved
            .file_name()
            .ok_or_else(|| unnamed(resolved))?
            .to_os_string();
        name.push(".lock");
        Ok(Self(resolved.with_file_name(name)))
    }

    /// The flat layout's lock for the directory `dir`: `<dir>.lock` beside it, named for its
    /// resolved path (symbolic links and `.`/`..` resolved, any trailing slash dropped; a
    /// dangling symbolic link is an error). The directory need not exist yet; its parent must.
    pub fn sibling_of(dir: &Path) -> io::Result<Self> {
        let resolved = resolve_store_dir(dir)?;
        let mut name = resolved
            .file_name()
            .ok_or_else(|| unnamed(dir))?
            .to_os_string();
        name.push(".lock");
        Self::at(&resolved, resolved.with_file_name(name))
    }

    /// The lock beside the entry `path` (`<path>.lock`), named for its resolved parent and its
    /// own name as given: a symbolic link at `path` itself is not followed (a staging path's
    /// lock, taken before anything at that path is checked).
    pub(crate) fn beside_entry(path: &Path) -> io::Result<Self> {
        let name = path.file_name().ok_or_else(|| unnamed(path))?;
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        Self::beside(&resolve_dir(parent)?.join(name))
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

/// The lock file a store keeps inside its own directory ([`DirLock`]), reserved
/// like the store's other internal names.
pub const INSIDE_LOCK: &str = "sparq.lock";

/// The lock on a graph directory, held while this lives: exclusive or shared.
///
/// A store's lock is two lock files, taken in this
/// order, outside then inside, by every holder:
///
/// - the OUTSIDE lock, `<dir>.lock` beside the directory, when it exists or can be created (an
///   existing one is opened read-only when it cannot be opened for writing: a lock needs no
///   write access). It stays put when the directory is renamed, so it is what excludes holders
///   across a swap's renames, and what a missing directory's recovery or creation needs;
/// - the INSIDE lock, [`INSIDE_LOCK`] in the directory, always (a holder that cannot open or
///   create it is refused). A holder whose parent directory is not writable (a store at a mount
///   root, say) holds the inside lock alone; it can open, write and read the store, but not rename it.
///
/// A holder renaming the directory (a compaction or restore swap) must hold the outside lock.
/// It keeps the old directory's inside lock until the old directory is gone, and takes the new
/// directory's inside lock before the new directory is put in place (on Windows, where a file
/// held open inside a directory blocks its rename, it lets go of the inside lock for the
/// renames and takes the new one after them, refusing writes if someone took it in between).
/// After locking the inside file, a holder checks it is still the file at `<dir>/sparq.lock`,
/// so a file a swap renamed away is never taken for the directory's lock.
#[derive(Debug)]
pub struct DirLock {
    outside: Option<File>,
    location: LockLocation,
    shared: bool,
    process: u32,
    /// The store directory and the lock held on its inside lock file (`None` for a staging
    /// path's lock, which has no inside lock; the file is `None` while a swap has let go of it).
    inside: Option<(PathBuf, std::sync::Mutex<Option<InsideLock>>)>,
}

impl Drop for DirLock {
    fn drop(&mut self) {
        if self.process != std::process::id() {
            return;
        }
        // Unlock before closing: fork/dup copies share the same Unix lock.
        if let Some((_, held)) = &mut self.inside {
            if let Some(file) = held
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_mut()
            {
                file.unlock();
            }
        }
        if let Some(file) = &self.outside {
            let _ = fs4::FileExt::unlock(file);
        }
    }
}

/// An inside lock may move between a staged base and its directory guard during a swap.
/// Keep the same owner-aware release on every exit, including failed swaps and recovery.
#[derive(Debug)]
pub(crate) struct InsideLock {
    file: File,
    process: u32,
    unlocked: bool,
}

impl InsideLock {
    pub(crate) fn new(file: File) -> Self {
        Self {
            file,
            process: std::process::id(),
            unlocked: false,
        }
    }

    fn unlock(&mut self) {
        if !self.unlocked && self.process == std::process::id() {
            // Drop cannot report errors. Closing the file remains the fallback.
            self.unlocked = fs4::FileExt::unlock(&self.file).is_ok();
        }
    }
}

impl Drop for InsideLock {
    fn drop(&mut self) {
        self.unlock();
    }
}

/// Whether `e` says a file cannot be created or written here (as opposed to some other error).
fn unwritable(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
    )
}

/// Opens the lock file at `path` to lock it: read-write, created when `create` and missing;
/// else, or when it cannot be created or written, an existing file read-only. `None` when it
/// neither exists nor can be created.
fn open_lock_file(path: &Path, create: bool) -> io::Result<Option<File>> {
    check_regular_lock_file(path)?;
    if create {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
        {
            Ok(f) => return Ok(Some(f)),
            Err(e) if unwritable(&e) => {}
            Err(e) => return Err(e),
        }
    }
    match File::open(path) {
        Ok(f) => Ok(Some(f)),
        Err(e) if e.kind() == io::ErrorKind::NotFound || unwritable(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// DOS device basenames remain device names with extensions or trailing spaces and dots.
#[cfg(any(windows, test))]
fn windows_device_name(name: &str) -> bool {
    let stem = name
        .split(['.', ':'])
        .next()
        .unwrap_or(name)
        .trim_end_matches(' ');
    let stem = comparable_name(stem);
    if matches!(
        stem.as_str(),
        "con" | "prn" | "aux" | "nul" | "conin$" | "conout$"
    ) {
        return true;
    }
    ["com", "lpt"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|port| {
            matches!(
                port,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

/// Refuses device endpoints before metadata lookup, including missing DOS aliases.
#[cfg(windows)]
fn refuse_windows_device_path(path: &Path) -> io::Result<()> {
    use std::path::{Component, Prefix};
    // Verbatim filesystem paths address literal filenames rather than DOS aliases.
    let verbatim = matches!(path.components().next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(..) | Prefix::Verbatim(_)));
    let special = path.components().any(|component| match component {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::DeviceNS(_) => true,
            Prefix::Verbatim(name) => {
                let name = name.to_string_lossy();
                name.eq_ignore_ascii_case("GLOBALROOT") || windows_device_name(&name)
            }
            _ => false,
        },
        Component::Normal(name) => !verbatim && windows_device_name(&name.to_string_lossy()),
        _ => false,
    });
    if special {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} is a device path, not a store directory or regular lock file",
                path.display()
            ),
        ));
    }
    Ok(())
}

/// Refuses special files before opening them: opening a FIFO could otherwise wait forever.
fn check_regular_lock_file(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    refuse_windows_device_path(path)?;
    match path.metadata() {
        Ok(meta) if !meta.is_file() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular lock file", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Locks `file` (the lock file at `path`), exclusively or shared, without waiting.
fn try_lock(file: &File, path: &Path, shared: bool) -> io::Result<()> {
    let locked = if shared {
        fs4::FileExt::try_lock_shared(file)
    } else {
        fs4::FileExt::try_lock(file)
    };
    match locked {
        Ok(()) => Ok(()),
        Err(fs4::TryLockError::WouldBlock) => Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            format!(
                "{} is locked: the directory is in use by another holder",
                path.display()
            ),
        )),
        Err(fs4::TryLockError::Error(e)) => Err(e),
    }
}

/// Takes the inside lock of the store directory `dir`: an error when it cannot be opened or
/// created, when it is held, or when the
/// file locked is no longer the one at `<dir>/sparq.lock` (a swap renamed it away meanwhile:
/// `WouldBlock`, retry).
pub(crate) fn lock_inside(dir: &Path, shared: bool) -> io::Result<Option<InsideLock>> {
    let path = dir.join(INSIDE_LOCK);
    let Some(file) = open_lock_file(&path, true)? else {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "the lock file {} cannot be opened or created: the store directory {} must \
                 allow access to its inside lock",
                path.display(),
                dir.display()
            ),
        ));
    };
    try_lock(&file, &path, shared)?;
    let file = InsideLock::new(file);
    let same = same_file::Handle::from_file(file.file.try_clone()?)
        .and_then(|held| Ok(held == same_file::Handle::from_path(&path)?))
        .unwrap_or(false);
    if !same {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            format!(
                "{} was replaced while it was being locked (the store is being swapped): retry",
                path.display()
            ),
        ));
    }
    Ok(Some(file))
}

impl DirLock {
    /// Take the lock at `location` exclusively, creating its file if need be: `WouldBlock` when
    /// another holder (another process, or another open in this one, shared or exclusive) has
    /// it. A lock of this kind has no inside lock (a staging path's lock).
    pub fn acquire(location: LockLocation) -> io::Result<Self> {
        check_regular_lock_file(location.path())?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(location.path())?;
        try_lock(&file, location.path(), false)?;
        Ok(Self {
            outside: Some(file),
            location,
            shared: false,
            process: std::process::id(),
            inside: None,
        })
    }

    /// Take a store's outside lock at `location` (the lock file beside the directory), shared
    /// or exclusive, when it exists or can be created; with `create` false, an existing lock
    /// file only. Its inside lock is taken next ([`lock_inside`](Self::lock_inside)), once the
    /// directory `dir` exists. `WouldBlock` when another holder has the outside lock.
    pub(crate) fn acquire_outside(
        location: LockLocation,
        dir: &Path,
        shared: bool,
        create: bool,
    ) -> io::Result<Self> {
        // A filesystem/drive root has no sibling. Its location identifies the inside
        // lock, which is acquired once by lock_inside after the directory is checked.
        let outside = if is_filesystem_root(dir) {
            None
        } else {
            match open_lock_file(location.path(), create)? {
                Some(file) => {
                    try_lock(&file, location.path(), shared)?;
                    Some(file)
                }
                None => None,
            }
        };
        Ok(Self {
            outside,
            location,
            shared,
            process: std::process::id(),
            inside: Some((dir.to_path_buf(), std::sync::Mutex::new(None))),
        })
    }

    /// Takes the inside lock of this store's directory, which must exist now (see
    /// [`acquire_outside`](Self::acquire_outside)). Every holder must take this lock;
    /// a holder that cannot open or create it is refused.
    pub(crate) fn lock_inside(&self) -> io::Result<()> {
        let Some((dir, held)) = &self.inside else {
            return Ok(());
        };
        let file = lock_inside(dir, self.shared)?;
        *held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = file;
        Ok(())
    }

    /// Makes sure the inside lock held is the one at `<dir>/sparq.lock` now (after a failed swap
    /// was recovered, which may have put another directory in place): kept when it is, taken
    /// afresh when not.
    pub(crate) fn ensure_inside(&self) -> io::Result<()> {
        let Some((dir, held)) = &self.inside else {
            return Ok(());
        };
        let mut held = held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(file) = held.as_ref() {
            let current = same_file::Handle::from_file(file.file.try_clone()?)
                .and_then(|h| Ok(h == same_file::Handle::from_path(dir.join(INSIDE_LOCK))?))
                .unwrap_or(false);
            if current {
                return Ok(());
            }
        }
        *held = None;
        *held = lock_inside(dir, self.shared)?;
        Ok(())
    }

    /// Lets go of the inside lock (a swap about to rename the directory, on Windows).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn release_inside(&self) {
        if let Some((_, held)) = &self.inside {
            held.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
        }
    }

    /// Puts `file`, a lock already held on the new directory's inside lock file, in place of
    /// the inside lock held now, which is returned (for the caller to keep until the old
    /// directory is gone). Unix only: Windows takes the new lock after the renames.
    #[cfg(not(windows))]
    pub(crate) fn replace_inside(&self, file: Option<InsideLock>) -> Option<InsideLock> {
        match &self.inside {
            Some((_, held)) => std::mem::replace(
                &mut *held
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                file,
            ),
            None => file,
        }
    }

    /// Whether this holder has the outside lock (beside the directory): what renaming the
    /// directory needs.
    pub fn holds_outside(&self) -> bool {
        self.outside.is_some()
    }

    /// Whether some holder (exclusive or shared) has the lock file at `path` now: tried
    /// exclusively, and released at once if that succeeds.
    pub(crate) fn is_held(path: &Path) -> io::Result<bool> {
        check_regular_lock_file(path)?;
        let file = File::open(path)?;
        match fs4::FileExt::try_lock(&file) {
            Ok(()) => {
                fs4::FileExt::unlock(&file)?;
                Ok(false)
            }
            Err(fs4::TryLockError::WouldBlock) => Ok(true),
            Err(fs4::TryLockError::Error(e)) => Err(e),
        }
    }

    /// Whether this is a shared (read-only) hold.
    pub fn is_shared(&self) -> bool {
        self.shared
    }

    /// Its lock namespace: the outside file, or the inside file for a filesystem root.
    pub fn location(&self) -> &LockLocation {
        &self.location
    }

    /// Whether this is the lock of the store directory `dir` (by identity, not by spelling).
    pub(crate) fn guards(&self, dir: &Path) -> bool {
        match &self.inside {
            Some((own, _)) => same_file(own, dir),
            None => false,
        }
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

    #[cfg(unix)]
    fn duplicate_inside(held: &DirLock) -> File {
        held.inside
            .as_ref()
            .unwrap()
            .1
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .file
            .try_clone()
            .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn dropping_the_guard_releases_locks_with_duplicate_descriptors() {
        let root = scratch("duplicate-descriptors");
        let dir = root.join("g");
        std::fs::create_dir(&dir).unwrap();
        let location = LockLocation::sibling_of(&dir).unwrap();
        for shared in [false, true] {
            let held = DirLock::acquire_outside(location.clone(), &dir, shared, true).unwrap();
            held.lock_inside().unwrap();
            let outside = held.outside.as_ref().unwrap().try_clone().unwrap();
            let inside = duplicate_inside(&held);
            drop(held);
            let next = DirLock::acquire_outside(location.clone(), &dir, false, true)
                .expect("outside lock released with its guard");
            next.lock_inside()
                .expect("inside lock released with its guard");
            drop(next);
            drop((inside, outside));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_inherited_guard_does_not_unlock_either_owners_file() {
        let root = scratch("process-owner");
        let dir = root.join("g");
        std::fs::create_dir(&dir).unwrap();
        let location = LockLocation::sibling_of(&dir).unwrap();
        let mut held = DirLock::acquire_outside(location.clone(), &dir, false, true).unwrap();
        held.lock_inside().unwrap();
        let outside = held.outside.as_ref().unwrap().try_clone().unwrap();
        let inside = duplicate_inside(&held);
        // Model a forked child's PID without forking a multithreaded test harness.
        let other_process = std::process::id().wrapping_add(1);
        held.process = other_process;
        held.inside
            .as_mut()
            .unwrap()
            .1
            .get_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .process = other_process;
        drop(held);
        assert_eq!(
            DirLock::acquire(location).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            lock_inside(&dir, false).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        fs4::FileExt::unlock(&inside).unwrap();
        fs4::FileExt::unlock(&outside).unwrap();
        drop((inside, outside));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn staging_and_inside_only_guards_release_duplicate_descriptors() {
        let root = scratch("partial-guards");
        let dir = root.join("g");
        std::fs::create_dir(&dir).unwrap();
        let location = LockLocation::sibling_of(&dir).unwrap();
        // No sibling exists and creation is disabled: exercise inside-only storage.
        let held = DirLock::acquire_outside(location.clone(), &dir, false, false).unwrap();
        assert!(!held.holds_outside());
        held.lock_inside().unwrap();
        let inside = duplicate_inside(&held);
        drop(held);
        drop(lock_inside(&dir, false).expect("inside-only lock released"));
        drop(inside);
        let held = DirLock::acquire(location.clone()).unwrap();
        let outside = held.outside.as_ref().unwrap().try_clone().unwrap();
        drop(held);
        drop(DirLock::acquire(location).expect("staging lock released"));
        drop(outside);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pins_and_independent_readers_keep_live_locks() {
        let root = scratch("pins-readers");
        let dir = root.join("g");
        std::fs::create_dir(&dir).unwrap();
        let location = LockLocation::sibling_of(&dir).unwrap();
        let reader = || {
            let held = DirLock::acquire_outside(location.clone(), &dir, true, true).unwrap();
            held.lock_inside().unwrap();
            held
        };
        let first = std::sync::Arc::new(reader());
        let pin: Pin = Some(first.clone());
        let second = reader();
        let copies = (
            first.outside.as_ref().unwrap().try_clone().unwrap(),
            duplicate_inside(&first),
        );
        drop(first);
        assert_eq!(
            DirLock::acquire(location.clone()).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(pin);
        assert_eq!(
            DirLock::acquire(location.clone()).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            lock_inside(&dir, false).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(second);
        drop(DirLock::acquire(location).expect("final reader released outside lock"));
        drop(lock_inside(&dir, false).expect("final reader released inside lock"));
        drop(copies);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn detached_swap_locks_and_release_inside_unlock_duplicate_descriptors() {
        let root = scratch("swap-duplicates");
        let dir = root.join("g");
        std::fs::create_dir(&dir).unwrap();
        let held =
            DirLock::acquire_outside(LockLocation::sibling_of(&dir).unwrap(), &dir, false, true)
                .unwrap();
        held.lock_inside().unwrap();
        let released_copy = duplicate_inside(&held);
        held.release_inside();
        drop(lock_inside(&dir, false).expect("released inside lock unlocked"));
        drop(released_copy);
        held.lock_inside().unwrap();
        let old_copy = duplicate_inside(&held);
        let old = root.join("old");
        std::fs::rename(&dir, &old).unwrap();
        std::fs::create_dir(&dir).unwrap();
        let detached = held.replace_inside(lock_inside(&dir, false).unwrap());
        assert_eq!(
            lock_inside(&old, false).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(detached);
        drop(lock_inside(&old, false).expect("detached old inside lock unlocked"));
        let new_copy = duplicate_inside(&held);
        drop(held);
        drop(lock_inside(&dir, false).expect("replacement inside lock unlocked"));
        drop((old_copy, new_copy));
        std::fs::remove_dir_all(root).unwrap();
    }

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
        #[cfg_attr(not(unix), allow(unused_mut))] // a symbolic link is added on Unix only
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

    /// Root paths select an inside-only namespace without opening files in the host root.
    #[test]
    fn filesystem_roots_use_an_inside_only_lock_namespace() {
        let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
        let root = cwd.ancestors().last().unwrap();
        let location = LockLocation::beside(root).expect("a root has an inside lock namespace");
        assert_eq!(location.path(), root.join(INSIDE_LOCK));
        let lock = DirLock::acquire_outside(location, root, false, true).unwrap();
        assert!(!lock.holds_outside());
        assert_eq!(lock.location().path(), root.join(INSIDE_LOCK));
        let error = crate::refuse_swap_without_outside_lock(root, Some(&lock)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("has no parent directory"));
        assert!(error.to_string().contains(&root.display().to_string()));
    }
    #[test]
    fn ensure_inside_locks_the_replacement_directory() {
        let root = scratch("ensure-inside");
        let dir = root.join("g");
        std::fs::create_dir(&dir).unwrap();
        let lock =
            DirLock::acquire_outside(LockLocation::sibling_of(&dir).unwrap(), &dir, false, true)
                .unwrap();
        lock.lock_inside().unwrap();
        #[cfg(windows)]
        lock.release_inside();
        #[cfg(unix)]
        let old_copy = duplicate_inside(&lock);
        let old = root.join("old");
        std::fs::rename(&dir, &old).unwrap();
        std::fs::create_dir(&dir).unwrap();
        lock.ensure_inside().unwrap();
        lock.ensure_inside()
            .expect("already holding the replacement lock");
        assert_eq!(
            lock_inside(&dir, false).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(
            lock_inside(&old, false).unwrap().is_some(),
            "old lock was kept"
        );
        drop(lock);
        #[cfg(unix)]
        drop(old_copy);
        assert!(lock_inside(&dir, false).unwrap().is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn regular_lock_files_exclude_devices_and_accept_missing_files() {
        let root = scratch("regular-lock");
        let missing = root.join("missing");
        check_regular_lock_file(&missing).unwrap();
        std::fs::write(&missing, b"").unwrap();
        check_regular_lock_file(&missing).unwrap();
        assert_eq!(
            check_regular_lock_file(Path::new("/dev/null"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_device_names_are_matched_before_extensions_and_in_any_case() {
        for name in [
            "NUL",
            "nul.lock",
            "Con",
            "PRN.log",
            "AUX .lock",
            "COM1",
            "lpt9.lock",
            "COM¹.tmp",
            "LPT²",
            "CONOUT$",
            "CONIN$",
            "CONıN$",
        ] {
            assert!(windows_device_name(name), "{name}");
        }
        for name in ["null", "console", "COM10", "LPT0", "port.COM1", "data.lock"] {
            assert!(!windows_device_name(name), "{name}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_device_paths_are_refused_before_any_files_are_created() {
        for name in ["NUL", "nul.lock", "CONOUT$", r"\\.\NUL", r"\\?\GLOBALROOT"] {
            let path = Path::new(name);
            assert!(refuse_windows_device_path(path).is_err(), "{name}");
            assert!(resolve_store_dir(path).is_err(), "{name}");
            assert!(check_regular_lock_file(path).is_err(), "{name}");
        }
        assert!(refuse_windows_device_path(Path::new(r"\\?\C:\data\nul.lock")).is_ok());
    }
}
