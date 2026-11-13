//! Read backing for immutable store files. The directory owner pins mapped files.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

// The smallest page-sized power of two covering the resource-metadata file tail,
// including materialized bases. Keep this internal: it selects a read strategy,
// never a persistence format or a write policy.
pub(crate) const SMALL_FILE_BYTES: usize = 4096;

pub(crate) enum Backing {
    Owned(Vec<u8>),
    Mapped(memmap2::Mmap),
}

/// Typed owned arrays never reinterpret the alignment of a byte allocation.
pub(crate) enum Array<T> {
    Owned(Vec<T>),
    Mapped(memmap2::Mmap),
}

impl<T> Array<T> {
    pub(crate) fn heap_bytes(&self) -> usize {
        match self {
            Self::Owned(v) => v.capacity() * std::mem::size_of::<T>(),
            Self::Mapped(_) => 0,
        }
    }
}

impl std::ops::Deref for Backing {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Owned(v) => v,
            Self::Mapped(m) => m,
        }
    }
}

impl Backing {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Self::open_file(file, len)
    }

    pub(crate) fn open_file(file: File, len: u64) -> io::Result<Self> {
        Self::from_file(file, len, threshold())
    }

    fn from_file(file: File, len: u64, threshold: usize) -> io::Result<Self> {
        if len <= threshold as u64 {
            // Bound both the read and its allocation independently of the stat. A file
            // changed between stat and read is refused, including same-strategy growth.
            let mut bytes = Vec::new();
            let read_limit = (threshold as u64).saturating_add(1);
            file.take(read_limit).read_to_end(&mut bytes)?;
            if bytes.len() as u64 != len {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "store file size changed during open",
                ));
            }
            return Ok(Self::Owned(bytes));
        }
        let len = usize::try_from(len).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "store file exceeds address space",
            )
        })?;
        // Reuse the length already checked above; asking mmap to stat it again adds
        // work to every large-file open without changing the chosen backing.
        // SAFETY: read-only mapping of a file owned by this store for its lifetime.
        // The caller retains its existing directory pin and never mutates the file.
        let map = unsafe { memmap2::MmapOptions::new().len(len).map(&file) };
        Ok(Self::Mapped(map.map_err(mapping_error)?))
    }

    pub(crate) fn decode<const N: usize, T>(self, decode: impl FnMut([u8; N]) -> T) -> Array<T> {
        match self {
            Self::Owned(bytes) => Array::Owned(
                bytes
                    .chunks_exact(N)
                    .map(|b| b.try_into().unwrap())
                    .map(decode)
                    .collect(),
            ),
            Self::Mapped(map) => Array::Mapped(map),
        }
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        match self {
            Self::Owned(v) => v.capacity(),
            Self::Mapped(_) => 0,
        }
    }
}

#[derive(Debug)]
struct MappingLimit(io::Error);

impl std::fmt::Display for MappingLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "store memory mapping limit reached (on Linux check vm.max_map_count or address-space limits): {}",
            self.0
        )
    }
}

impl std::error::Error for MappingLimit {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

fn mapping_error(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::OutOfMemory {
        io::Error::other(MappingLimit(error))
    } else {
        error
    }
}

fn threshold() -> usize {
    #[cfg(test)]
    if let Some(value) = OVERRIDE.with(|v| v.get()) {
        return value;
    }
    // Build-time coverage for the complete crash/fault suite, including integration
    // tests that link the non-test library. Local test overrides still take priority.
    if cfg!(sparq_backing = "mapped") {
        0
    } else if cfg!(sparq_backing = "owned") {
        usize::MAX
    } else {
        SMALL_FILE_BYTES
    }
}

#[cfg(test)]
thread_local! {
    static OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_threshold<T>(value: usize, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OVERRIDE.with(|v| v.set(self.0));
        }
    }
    let _restore = Restore(OVERRIDE.with(|v| v.replace(Some(value))));
    run()
}

#[cfg(test)]
#[path = "backing_tests.rs"]
mod tests;
