// [GPT-6] Native mapped tables may share one archive mapping.
use std::ops::Deref;
#[cfg(feature = "native-archive")]
use std::sync::Arc;

/// Immutable bytes with an aligned base, owned for every derived view's lifetime.
pub(crate) enum MappedBytes {
    Whole(memmap2::Mmap),
    #[cfg(feature = "native-archive")]
    Region {
        mapping: Arc<memmap2::Mmap>,
        start: usize,
        end: usize,
    },
}

impl From<memmap2::Mmap> for MappedBytes {
    fn from(mapping: memmap2::Mmap) -> Self {
        Self::Whole(mapping)
    }
}

impl MappedBytes {
    #[cfg(feature = "native-archive")]
    pub(crate) fn region(
        mapping: Arc<memmap2::Mmap>,
        start: usize,
        len: usize,
    ) -> std::io::Result<Self> {
        let end = start.checked_add(len).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "mapped region overflows")
        })?;
        // Native dictionaries/numeric tables cast to u64/f64, so EVERY region starts
        // at an eight-byte boundary even when its particular codec uses byte reads.
        if start % 8 != 0 || end > mapping.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "mapped region is unaligned or outside archive",
            ));
        }
        Ok(Self::Region {
            mapping,
            start,
            end,
        })
    }
}

impl Deref for MappedBytes {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        match self {
            Self::Whole(mapping) => mapping,
            #[cfg(feature = "native-archive")]
            Self::Region {
                mapping,
                start,
                end,
            } => &mapping[*start..*end],
        }
    }
}
