//! The `.spqv` on-disk vector store: one f32 embedding per dictionary term id, in a
//! single flat memory-mapped file.
//!
//! # File format (version 2, little-endian)
//!
//! ```text
//! offset 0   magic        b"SPQV"                          4 bytes
//! offset 4   version      u32 = 2                          4 bytes
//! offset 8   dim          u32                              4 bytes
//! offset 12  count        u64                              8 bytes
//! offset 20  reserved     zero padding                     12 bytes
//! offset 32  fingerprint  graph fingerprint                24 bytes
//!            (dict_len: u64, triple_count: u64, content_hash: u64)
//! offset 56  data         [count × dim] f32, dense         count·dim·4 bytes
//! offset 56+count·dim·4   id→slot index                    count·8 bytes
//!            (id: u32, slot: u32) pairs sorted by id ascending
//! ```
//!
//! # [FABLE-5] (sq-lhcot.1) File format version 3 — embedding provenance
//!
//! Version 3 extends the v2 header with a length-prefixed **embedding-provenance block** after the
//! fingerprint, then the data section begins:
//!
//! ```text
//! offset 0..56  (as v2, but version = 3)
//! offset 56  prov_len     u32 (length of the provenance block)   4 bytes
//! offset 60  provenance   [prov_len] bytes                       prov_len bytes
//!            (see crate::spqv_provenance::EmbeddingProvenance::to_bytes)
//! offset 60+prov_len  data ... (as v2)
//! ```
//!
//! The provenance block records the embedding pipeline's identity (model id, model/content version,
//! metric, normalization, verbalization regime) and a RESERVED opaque extension area (KERN boundary:
//! **extension fields reserved pending the cross-implementation profile #1746** — no fields defined).
//! [`VectorStore::check_provenance`] rejects a query whose embedder is incompatible with the store's,
//! closing the reproducibility gap the v2 container left open (a query in a different embedding space
//! returns arithmetically-defined but semantically-WRONG neighbours). See [`crate::spqv_provenance`].
//!
//! **Opt-in write discipline (mirrors how v2 was introduced).** v3 is WRITTEN only when the opt-in
//! `spqv-provenance` feature is on AND a provenance was bound (`VectorStore::with_provenance`); with
//! the feature off (or no provenance) the writer emits v2 exactly as before — the default on-disk
//! format is unchanged. The v3 READ path is ALWAYS compiled: a v3 store opens on a feature-off build
//! (its provenance is exposed via [`VectorStore::provenance`]; only the demanding query check is
//! feature-gated), exactly as v1 and v2 both open regardless of features. A v2/v1 store carries no
//! provenance, so `check_provenance` fails closed against it unless the caller opts into
//! [`LegacyMode::Allow`].
//!
//! # Version 4 — opt-in per-vector metadata
//!
//! With the default-OFF `metadata-sidecar` feature, calling `put_with_meta` selects v4. It adds a
//! deterministic, length-delimited metadata block between the optional provenance block and the
//! aligned vector data. Each entry is `(id, opaque UTF-8 bytes)`, sorted by id. Ordinary `put` calls
//! write v2 (or v3 when provenance is bound) exactly as before, even when the feature is enabled.
//! Metadata is accessed with `meta` and decorates `nearest_exact_with_meta`; it never participates
//! in scoring or ordering.
//!
//! [OPUS-4.8] (sq-32i5) The **fingerprint** binds the store to the graph it was built against
//! (see [`crate::fingerprint`]): a store keyed by dictionary id is meaningless against a graph
//! whose ids have shifted, so [`VectorStore::check_graph`] errors on a mismatch instead of letting
//! a query silently mis-resolve. Version-1 files (no fingerprint block, 32-byte header) still open,
//! but `check_graph` reports them as unverifiable — see [`VectorStore::open`] and the back-compat
//! note there.
//!
//! [OPUS-4.8] (sq-wlzi) **ID-KEYED STALENESS CONTRACT.** Because every embedding is keyed by the
//! build-time dictionary id, a store is valid ONLY against the **exact graph generation it was built
//! against**. To serve it, persist that graph (`Graph::save`) and reopen THAT graph (`Graph::open`,
//! which mmaps the **frozen** dict id order — both gated by `sparq-core`'s `mmap` feature) to resolve
//! query terms — **never re-parse the source RDF** (`Graph::load_str` et al.): sparq-core's parallel
//! sharded dict merge assigns thread-count-dependent ids, so a re-parse produces a *different*
//! `id → term` binding and `get`/`nearest_term` mis-resolve. `check_graph` is a
//! backstop, **not** a sufficient guard for this case — the sq-xhiv fingerprint folds the term *set*
//! and is thread-count-stable, so it PASSES a re-parse of the same RDF even though the ids permuted.
//! See [`crate::fingerprint`] for the full rationale and `tests/staleness_contract.rs` for the
//! round-trip-vs-trap demonstration.
//!
//! Coverage is **sparse by design**: in an RDF graph only entities get embeddings, not
//! every literal, so vectors are stored densely in *insertion* slots and the trailing
//! sorted `(id, slot)` index maps dictionary ids to slots — `get` on an mmap'd store is
//! one binary search plus a contiguous slice, no per-vector allocation. The data and
//! index sections both start at multiples of 4 from the page-aligned map, so the f32
//! casts are always aligned.
//!
//! Build phase ([`VectorStore::create`] + [`VectorStore::put`]) accumulates vectors in
//! RAM; [`VectorStore::finalize`] writes the file once and re-opens it memory-mapped,
//! so the same handle serves reads before *and* after finalization. For datasets whose
//! dense data does not fit in RAM, [`StreamingWriter`] appends each vector to the data
//! section as it is `put` and spills the id→slot index to a sidecar file, so build-phase
//! memory is O(1) (finalize transiently holds the 8-byte-per-vector index to sort it —
//! a `dim × 4 / 8` reduction, e.g. 192× for 384-d f32). Both writers produce
//! byte-identical version-1 files.
//!
//! `.spqv` bytes are always little-endian. Readers on big-endian hosts validate the complete
//! little-endian document first, then copy it into aligned owned storage and byte-swap only the
//! dense f32 data region for native reads. Headers, provenance, metadata, and the trailing index
//! remain in their canonical little-endian representation. Writers still reject big-endian hosts.

use crate::fingerprint::{self, Fingerprint, FINGERPRINT_LEN};
use crate::spqv_provenance::EmbeddingProvenance;
// [FABLE-5] (sq-98c) memmap2 is a native-only dependency (target-gated out of wasm32 builds in
// Cargo.toml); on wasm the read paths use the owned-bytes backing below instead of a map.
#[cfg(not(target_arch = "wasm32"))]
use memmap2::Mmap;
use rustc_hash::FxHashMap;
use sparq_core::dict::Id;
use sparq_core::Graph;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// First four bytes of every `.spqv` file.
pub const SPQV_MAGIC: [u8; 4] = *b"SPQV";
/// Current DEFAULT format version. [OPUS-4.8] (sq-32i5) v2 adds the 24-byte graph fingerprint block
/// at offset 32; v1 files (32-byte header, no fingerprint) still open but cannot be verified. v2
/// remains the default WRITE version — the v3 embedding-provenance format ([`SPQV_VERSION_V3`]) is
/// written only under the opt-in `spqv-provenance` feature with a bound provenance. [FABLE-5]
pub const SPQV_VERSION: u32 = 2;
/// [FABLE-5] (sq-lhcot.1) The embedding-provenance format version — a v2 header plus a
/// length-prefixed [`EmbeddingProvenance`] block after the fingerprint. WRITTEN only when the opt-in
/// `spqv-provenance` feature is on and a provenance was bound (`VectorStore::with_provenance`); READ
/// always (a v3 file opens regardless of features).
pub const SPQV_VERSION_V3: u32 = 3;
/// Metadata-bearing `.spqv` format, written only by the opt-in `metadata-sidecar` feature when at
/// least one vector carries a tag.
#[cfg(feature = "metadata-sidecar")]
pub const SPQV_VERSION_V4: u32 = 4;
/// Header length of a version-1 file (no fingerprint block).
const HEADER_LEN_V1: usize = 32;
/// Header length of the version-2 format: the v1 header + the fingerprint block. Also the length of
/// the v3 header PREFIX (the v3 provenance block + its `u32` length prefix follow this offset).
const HEADER_LEN: usize = HEADER_LEN_V1 + FINGERPRINT_LEN;

/// [OPUS-4.8] A 4-byte-aligned owned byte buffer. A plain `Vec<u8>` has alignment 1, so its base
/// pointer may land on an odd address; `slot_vector` casts `&[u8]` slices of the buffer to
/// `&[f32]` via `from_raw_parts`, which is UNDEFINED BEHAVIOR on an unaligned pointer. Backing the
/// owned bytes with a `Vec<u32>` (alignment 4) guarantees the base — and therefore every
/// `HEADER_LEN + slot·dim·4` offset (all multiples of 4) — is f32-aligned. See review 1874.
pub(crate) struct AlignedBytes {
    /// Backing storage; only `len` bytes are logically valid (the last word may be padding).
    words: Vec<u32>,
    len: usize,
}

#[cfg(test)]
mod big_endian_read_tests {
    use super::{Backing, VectorStore, HEADER_LEN};

    fn opposite_native(value: f32) -> [u8; 4] {
        let mut bytes = value.to_ne_bytes();
        bytes.reverse();
        bytes
    }

    // (sq-i7w) Mutation witness: the fixture's dense words deliberately use the byte
    // order opposite to this test host, while every structural field stays canonical LE. Removing
    // or breaking the forced production swap makes both `get` and `iter` return the wrong floats.
    #[test]
    fn forced_big_endian_read_swaps_only_dense_f32_words() {
        let mut fixture = Vec::new();
        fixture.extend_from_slice(b"SPQV");
        fixture.extend_from_slice(&2u32.to_le_bytes());
        fixture.extend_from_slice(&2u32.to_le_bytes());
        fixture.extend_from_slice(&2u64.to_le_bytes());
        fixture.extend_from_slice(&[0; 12]);
        fixture.extend_from_slice(&[0; 24]);
        for value in [1.25f32, -2.5, 3.75, 0.125] {
            fixture.extend_from_slice(&opposite_native(value));
        }
        // The index remains LE and sorted by id; insertion slot order is 42, then 7.
        fixture.extend_from_slice(&7u32.to_le_bytes());
        fixture.extend_from_slice(&1u32.to_le_bytes());
        fixture.extend_from_slice(&42u32.to_le_bytes());
        fixture.extend_from_slice(&0u32.to_le_bytes());

        let data_end = HEADER_LEN + 4 * 4;
        assert_ne!(
            f32::from_ne_bytes(fixture[HEADER_LEN..HEADER_LEN + 4].try_into().unwrap()),
            1.25,
            "fixture must require a byte swap on the current host"
        );
        let header = fixture[..HEADER_LEN].to_vec();
        let index = fixture[data_end..].to_vec();

        let store = VectorStore::open_from_bytes_force_swap(fixture).unwrap();
        assert_eq!(store.get(42), Some(&[1.25f32, -2.5][..]));
        assert_eq!(store.get(7), Some(&[3.75f32, 0.125][..]));
        assert_eq!(store.get(8), None);
        let pairs: Vec<(u32, Vec<f32>)> = store
            .iter()
            .map(|(id, vector)| (id, vector.to_vec()))
            .collect();
        assert_eq!(pairs, [(42, vec![1.25, -2.5]), (7, vec![3.75, 0.125])]);

        let Backing::Map(backing) = &store.backing else {
            panic!("an opened store must use a read backing");
        };
        assert_eq!(&backing[..HEADER_LEN], header);
        assert_eq!(&backing[data_end..], index);
    }
}

impl AlignedBytes {
    fn from_vec(bytes: Vec<u8>) -> AlignedBytes {
        AlignedBytes::from_slice(&bytes)
    }

    fn from_slice(bytes: &[u8]) -> AlignedBytes {
        let len = bytes.len();
        // ceil(len / 4) words; the final word is zero-padded.
        let words = vec![0u32; len.div_ceil(4)];
        let mut ab = AlignedBytes { words, len };
        // SAFETY: `words` holds at least `len` bytes (rounded up to a word) and is u32-aligned
        // (≥ align(u8)); the destination region is exclusively owned here.
        let dst =
            unsafe { std::slice::from_raw_parts_mut(ab.words.as_mut_ptr() as *mut u8, len) };
        dst.copy_from_slice(bytes);
        ab
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: `words` is u32-aligned and holds ≥ `len` initialized bytes (the copy above);
        // f32/u8 reads of this region are in bounds. The base is 4-byte aligned by construction.
        unsafe { std::slice::from_raw_parts(self.words.as_ptr() as *const u8, self.len) }
    }

    /// Reverses each complete 4-byte word in `start..end`. Both offsets are word-aligned; callers
    /// use this only for a fully validated dense f32 region.
    fn swap_words(&mut self, start: usize, end: usize) {
        debug_assert_eq!(start % 4, 0);
        debug_assert_eq!(end % 4, 0);
        debug_assert!(end <= self.len);
        for word in &mut self.words[start / 4..end / 4] {
            *word = word.swap_bytes();
        }
    }
}

/// Read-phase backing bytes: a memory map ([`VectorStore::open`]) or an owned
/// buffer ([`VectorStore::open_from_bytes`] — environments without a
/// filesystem). Both deref to `[u8]`; every read path is shared.
/// `pub(crate)` so [`crate::diskann`] shares the same backing for `.spqg` files. [FABLE-5]
pub(crate) enum Bytes {
    /// [FABLE-5] (sq-98c) Native-only: memmap2 is target-gated out of wasm32 builds, where
    /// the owned-bytes backing serves every read instead.
    #[cfg(not(target_arch = "wasm32"))]
    Map(Mmap),
    /// [OPUS-4.8] f32-aligned owned bytes (see `AlignedBytes`) so the `slot_vector` f32 cast is
    /// always aligned — a plain `Vec<u8>` is alignment 1 and would risk UB. Review 1874.
    Owned(AlignedBytes),
}

impl Bytes {
    /// Copies `bytes` into the f32-aligned owned backing. Shared by the `open_from_bytes`
    /// entry points here and in [`crate::diskann`]. [FABLE-5]
    pub(crate) fn owned(bytes: Vec<u8>) -> Bytes {
        Bytes::Owned(AlignedBytes::from_vec(bytes))
    }

    /// Converts this backing to aligned owned bytes and swaps only the dense f32 word range.
    /// The caller must first validate the entire container, including its trailing index.
    fn into_owned_swapped(self, start: usize, end: usize) -> Bytes {
        let mut owned = match self {
            #[cfg(not(target_arch = "wasm32"))]
            Bytes::Map(map) => AlignedBytes::from_slice(&map),
            Bytes::Owned(owned) => owned,
        };
        owned.swap_words(start, end);
        Bytes::Owned(owned)
    }
}

/// [FABLE-5] (sq-98c) Opens the read backing for a store/index file: a read-only memory map on
/// native targets, a buffered `std::fs::read` into the f32-aligned owned backing on wasm32
/// (memmap2 is target-gated out of wasm builds; a wasm target WITH a filesystem, e.g. WASI,
/// reads the whole file — on `wasm32-unknown-unknown` the read fails with a clean I/O error,
/// and [`VectorStore::open_from_bytes`] / [`DiskAnnIndex::open_from_bytes`] are the
/// filesystem-less paths). Validation downstream is identical for both backings.
///
/// [`DiskAnnIndex::open_from_bytes`]: crate::diskann::DiskAnnIndex::open_from_bytes
pub(crate) fn open_backing(path: &Path) -> Result<Bytes, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let file = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
        // SAFETY: read-only map of a regular file; we treat concurrent external
        // modification of the file as out of contract (same stance as sparq-core's
        // mmap'd dictionary/indexes).
        let map = unsafe { Mmap::map(&file) }.map_err(|e| format!("mmap {}: {e}", path.display()))?;
        Ok(Bytes::Map(map))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        Ok(Bytes::owned(bytes))
    }
}

impl std::ops::Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Bytes::Map(m) => m,
            Bytes::Owned(v) => v.as_bytes(),
        }
    }
}

enum Backing {
    /// Build phase: vectors accumulate in RAM; `finalize` writes the file.
    Build { data: Vec<f32>, slots: FxHashMap<Id, u32>, ids: Vec<Id> },
    /// Read phase: the whole file is memory-mapped or held as owned bytes.
    Map(Bytes),
}

/// [FABLE-5] (sq-lhcot.1) How [`VectorStore::check_provenance`] treats a LEGACY (v1/v2) store that
/// carries no embedding provenance. The default is [`Reject`](Self::Reject) — fail-closed, since a
/// legacy store's embedding pipeline is unverifiable. A v3 store is always checked against its
/// recorded provenance regardless of this mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyMode {
    /// Fail-closed (the default): a legacy store with no provenance REJECTS a provenance-demanding
    /// query — its embedding pipeline cannot be verified compatible.
    Reject,
    /// Bypass the check for a LEGACY (no-provenance) store only — for a caller that KNOWS the store
    /// was built with a compatible pipeline. A v3 store is still checked against its recorded
    /// provenance.
    Allow,
}

/// A flat per-term-id f32 vector store backed by one `.spqv` file. See the module docs
/// for the format and the build/read lifecycle.
pub struct VectorStore {
    dim: usize,
    path: PathBuf,
    backing: Backing,
    /// [OPUS-4.8] (sq-32i5) The graph fingerprint bound to this store: `Some` for the current
    /// (version-2) format, `None` for a legacy version-1 file that predates fingerprinting (it
    /// opens but cannot be verified — see [`open`](Self::open)) and for a freshly `create`d store
    /// until [`with_fingerprint`](Self::with_fingerprint) is called.
    /// [`check_graph`](Self::check_graph) uses it to reject a stale store before a query
    /// can mis-resolve.
    fingerprint: Option<Fingerprint>,
    /// [FABLE-5] (sq-lhcot.1) The embedding provenance bound to this store: `Some` for a v3 file (or
    /// a build-phase store after `with_provenance` (feature-gated)), `None` for a v1/v2 file
    /// (which predate embedding provenance) and a freshly `create`d store until a provenance is bound.
    /// [`check_provenance`](Self::check_provenance) uses it to reject an incompatible query embedder.
    provenance: Option<EmbeddingProvenance>,
    /// Byte offset where the dense vector data begins: [`HEADER_LEN`] for a version-2 file (the
    /// v2 build path), [`HEADER_LEN_V1`] for an opened legacy version-1 file, and
    /// `HEADER_LEN + 4 + prov_len` for a v3 file (the fingerprint block + the length-prefixed
    /// provenance block). Every read path (`get`, `iter`, `slot_vector`, the trailing index) keys off
    /// this so all versions are read correctly by the same code. [FABLE-5]
    data_offset: usize,
    /// slot→id for mmap mode (the on-disk index is sorted by id, the data by slot);
    /// built lazily on the first [`iter`](Self::iter), O(count) once.
    inverse: std::sync::OnceLock<Vec<Id>>,
    /// [OPUS-4.8] (sq-pi44) The in-RAM delta sidecar: vectors appended/updated and ids tombstoned
    /// since the immutable base was built. `None` until the first delta mutation
    /// ([`add`](Self::add)/[`remove`](Self::remove)/[`update`](Self::update)) allocates it, bound to
    /// the base fingerprint. Every read path
    /// ([`get`](Self::get)/[`iter`](Self::iter)/[`len`](Self::len)) consults it transparently;
    /// [`compact`](Self::compact) folds it back into a fresh base. Gated behind the opt-in `delta`
    /// feature, so the default build carries no delta state. See [`crate::delta`].
    #[cfg(feature = "delta")]
    delta: Option<crate::delta::VectorDelta>,
    /// Opaque per-vector tags. Present only in `metadata-sidecar` builds, so the default store
    /// carries neither the map nor the v4 codec.
    #[cfg(feature = "metadata-sidecar")]
    metadata: FxHashMap<Id, String>,
}

impl VectorStore {
    /// Starts a new store that will be written to `path` (build phase). `dim` is fixed
    /// for the store's lifetime; every [`put`](Self::put) must match it.
    pub fn create<P: Into<PathBuf>>(path: P, dim: usize) -> Result<VectorStore, String> {
        if dim == 0 || dim > u32::MAX as usize {
            return Err(format!("invalid vector dimension {dim}"));
        }
        if cfg!(target_endian = "big") {
            return Err(".spqv is a little-endian format; big-endian targets are unsupported".into());
        }
        Ok(VectorStore {
            dim,
            path: path.into(),
            backing: Backing::Build { data: Vec::new(), slots: FxHashMap::default(), ids: Vec::new() },
            fingerprint: None,
            provenance: None,
            data_offset: HEADER_LEN,
            inverse: std::sync::OnceLock::new(),
            #[cfg(feature = "delta")]
            delta: None,
            #[cfg(feature = "metadata-sidecar")]
            metadata: FxHashMap::default(),
        })
    }

    /// [OPUS-4.8] (sq-32i5) Binds this build-phase store to `graph`'s fingerprint, which
    /// [`finalize`](Self::finalize) then embeds in the `.spqv` header. Call it before `finalize`
    /// (most naturally right after `create`, against the SAME graph whose term ids the vectors are
    /// keyed by). A store finalized WITHOUT a fingerprint is written in the version-2 format with a
    /// zeroed fingerprint, which [`check_graph`](Self::check_graph) treats as "unverified"; prefer
    /// setting it so a stale store is caught. Chains: `VectorStore::create(p, d)?.with_fingerprint(g)`.
    #[must_use]
    pub fn with_fingerprint(mut self, graph: &Graph) -> VectorStore {
        self.fingerprint = Some(Fingerprint::of(graph));
        self
    }

    /// [FABLE-5] (sq-lhcot.1) Binds this build-phase store to the embedding `provenance` its vectors
    /// were produced with — the model id, model/content version, metric, normalization, and
    /// verbalization regime. [`finalize`](Self::finalize) then writes the store in the **v3** format
    /// with the provenance embedded in the header. Call it before `finalize` (most naturally right
    /// after `create`, with the provenance of the embedder used to `put` the vectors).
    ///
    /// Binding a provenance is what selects the v3 write path: a store finalized WITHOUT one is
    /// written in the v2 format (no provenance), exactly as before. This method is gated behind the
    /// opt-in `spqv-provenance` feature — mirroring how the v2 fingerprint format was introduced, the
    /// v3 WRITE surface is opt-in so the default build's on-disk format is unchanged. Chains:
    /// `VectorStore::create(p, d)?.with_provenance(prov)`.
    #[cfg(feature = "spqv-provenance")]
    #[must_use]
    pub fn with_provenance(mut self, provenance: EmbeddingProvenance) -> VectorStore {
        self.provenance = Some(provenance);
        self
    }

    /// Opens an existing `.spqv` file read-only. On little-endian native hosts this is cheap: only
    /// the header and trailing `count·8`-byte id→slot index are read eagerly (the index is validated
    /// so no later read can panic on a corrupt file); the vector data — the overwhelming bulk of the
    /// file — stays memory-mapped and is paged in by the OS on access.
    ///
    /// [OPUS-4.8] (sq-32i5) Back-compat: a current (version-2) file's graph fingerprint is read into
    /// [`fingerprint`](Self::fingerprint) for [`check_graph`](Self::check_graph). A legacy version-1
    /// file (32-byte header, no fingerprint) still opens — its `fingerprint` is `None`, so
    /// `check_graph` reports it as unverifiable rather than silently passing. Rebuild such a store to
    /// enable the staleness check.
    ///
    /// [FABLE-5] (sq-98c) On wasm32 (memmap2 target-gated out) this reads the whole file into
    /// the same f32-aligned owned backing [`open_from_bytes`](Self::open_from_bytes) uses —
    /// identical validation, no map. `wasm32-unknown-unknown` has no filesystem, so there the
    /// read fails with a clean I/O error and `open_from_bytes` is the supported path.
    ///
    /// On a big-endian host, complete little-endian validation happens first; the file is then
    /// copied into aligned owned storage and only the dense f32 words are byte-swapped for native
    /// reads. The header, provenance/metadata blocks, and index remain canonical little-endian.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<VectorStore, String> {
        let path = path.as_ref();
        let backing = open_backing(path)?;
        Self::open_validated(backing, path.to_path_buf(), &path.display().to_string())
    }

    /// Opens a `.spqv` document held entirely in memory — for environments
    /// without a filesystem (the bytes were fetched, embedded, or decompressed
    /// by the caller). Validation is identical to [`open`](Self::open); reads borrow the aligned
    /// owned buffer instead of a memory map. On big-endian hosts, only the validated dense f32 words
    /// are byte-swapped for native reads. The handle is read-only (`put`/`finalize` behave as on any
    /// opened store).
    pub fn open_from_bytes(bytes: Vec<u8>) -> Result<VectorStore, String> {
        // [OPUS-4.8] Copy into a 4-byte-aligned backing so the read-phase f32 casts are aligned
        // (a plain Vec<u8> is alignment 1 — casting its slices to &[f32] is UB). Review 1874.
        Self::open_validated(Bytes::owned(bytes), PathBuf::new(), "<bytes>")
    }

    /// Test-only seam that exercises the big-endian swap-on-read branch on any host. Its input's
    /// dense f32 words must use the byte order opposite to the current target; all structural fields
    /// remain canonical little-endian.
    #[cfg(test)]
    fn open_from_bytes_force_swap(bytes: Vec<u8>) -> Result<VectorStore, String> {
        Self::open_validated_inner(Bytes::owned(bytes), PathBuf::new(), "<bytes-swapped>", true)
    }

    /// Shared header/index validation behind [`open`](Self::open) and
    /// [`open_from_bytes`](Self::open_from_bytes).
    fn open_validated(map: Bytes, path: PathBuf, origin: &str) -> Result<VectorStore, String> {
        Self::open_validated_inner(map, path, origin, cfg!(target_endian = "big"))
    }

    fn open_validated_inner(
        map: Bytes,
        path: PathBuf,
        origin: &str,
        swap_dense_for_native: bool,
    ) -> Result<VectorStore, String> {
        // The version-1 header is the smallest valid header; version 2 adds a fixed-size block.
        if map.len() < HEADER_LEN_V1 {
            return Err(format!("{origin}: truncated header"));
        }
        if map[0..4] != SPQV_MAGIC {
            return Err(format!("{origin}: not a .spqv file (bad magic)"));
        }
        let version = u32::from_le_bytes(map[4..8].try_into().unwrap());
        let dim = u32::from_le_bytes(map[8..12].try_into().unwrap()) as usize;
        let count64 = u64::from_le_bytes(map[12..20].try_into().unwrap());
        if dim == 0 {
            return Err(format!("{origin}: zero dimension"));
        }
        let count: usize = count64
            .try_into()
            .map_err(|_| format!("{origin}: count {count64} exceeds the address space"))?;
        #[cfg(feature = "metadata-sidecar")]
        let mut metadata = FxHashMap::default();
        // [OPUS-4.8] (sq-32i5) v1 (no fingerprint, 32-byte header), v2 (fingerprint, 56-byte header),
        // and [FABLE-5] (sq-lhcot.1) v3 (v2 header + a length-prefixed embedding-provenance block)
        // all open; the header length and the offset where the vector data begins depend on the
        // version, so every downstream read keys off `data_offset` below. The v3 READ path is always
        // compiled (a v3 file opens even on a build without the `spqv-provenance` feature).
        let (data_offset, fingerprint, provenance): (usize, Option<Fingerprint>, Option<EmbeddingProvenance>) =
            match version {
                1 => (HEADER_LEN_V1, None, None),
                2 => {
                    if map.len() < HEADER_LEN {
                        return Err(format!(
                            "{origin}: truncated version-2 header (fingerprint block)"
                        ));
                    }
                    // [OPUS-4.8] (sq-32i5) An all-zero block (a v2 store finalized without
                    // `with_fingerprint`) decodes to `None` ("unverifiable"), not a zero fingerprint
                    // that would surface as a spurious "DIFFERENT graph" mismatch.
                    let fp = Fingerprint::from_bytes_opt(&map[HEADER_LEN_V1..HEADER_LEN]);
                    (HEADER_LEN, fp, None)
                }
                3 => {
                    // [FABLE-5] (sq-lhcot.1) v3 header = the v2 header (magic/version/dim/count/
                    // reserved/fingerprint) + a `u32` provenance-block length at HEADER_LEN + that
                    // many provenance bytes; the data section follows. Validate each step before
                    // slicing so a corrupt header is a descriptive error, never an out-of-bounds read.
                    if map.len() < HEADER_LEN + 4 {
                        return Err(format!(
                            "{origin}: truncated version-3 header (provenance length prefix)"
                        ));
                    }
                    let fp = Fingerprint::from_bytes_opt(&map[HEADER_LEN_V1..HEADER_LEN]);
                    let prov_len =
                        u32::from_le_bytes(map[HEADER_LEN..HEADER_LEN + 4].try_into().unwrap())
                            as usize;
                    if prov_len > crate::spqv_provenance::MAX_PROVENANCE_BLOCK_LEN {
                        return Err(format!(
                            "{origin}: version-3 provenance block length {prov_len} exceeds the cap"
                        ));
                    }
                    let prov_start = HEADER_LEN + 4;
                    let prov_end = prov_start.checked_add(prov_len).ok_or_else(|| {
                        format!("{origin}: version-3 provenance length {prov_len} overflows")
                    })?;
                    if map.len() < prov_end {
                        return Err(format!(
                            "{origin}: truncated version-3 provenance block (need {prov_len} bytes)"
                        ));
                    }
                    let prov = EmbeddingProvenance::from_bytes(&map[prov_start..prov_end])
                        .map_err(|e| format!("{origin}: {e}"))?;
                    // [FABLE-5] The data section is zero-padded to the next 4-byte boundary after the
                    // provenance block (see `build_header`) so the f32 casts stay aligned. Recompute
                    // the same padded offset here; the pad bytes (if any) are not part of the block.
                    let padded = prov_end.div_ceil(4) * 4;
                    if map.len() < padded {
                        return Err(format!(
                            "{origin}: truncated version-3 header (provenance padding)"
                        ));
                    }
                    (padded, fp, Some(prov))
                }
                #[cfg(feature = "metadata-sidecar")]
                4 => {
                    if map.len() < HEADER_LEN + 4 {
                        return Err(format!(
                            "{origin}: truncated version-4 header (provenance length prefix)"
                        ));
                    }
                    let fp = Fingerprint::from_bytes_opt(&map[HEADER_LEN_V1..HEADER_LEN]);
                    let prov_len =
                        u32::from_le_bytes(map[HEADER_LEN..HEADER_LEN + 4].try_into().unwrap())
                            as usize;
                    if prov_len > crate::spqv_provenance::MAX_PROVENANCE_BLOCK_LEN {
                        return Err(format!(
                            "{origin}: version-4 provenance block length {prov_len} exceeds the cap"
                        ));
                    }
                    let prov_start = HEADER_LEN + 4;
                    let prov_end = prov_start.checked_add(prov_len).ok_or_else(|| {
                        format!("{origin}: version-4 provenance length {prov_len} overflows")
                    })?;
                    let meta_len_end = prov_end.checked_add(8).ok_or_else(|| {
                        format!("{origin}: version-4 metadata length offset overflows")
                    })?;
                    if map.len() < meta_len_end {
                        return Err(format!(
                            "{origin}: truncated version-4 header (metadata length prefix)"
                        ));
                    }
                    let provenance = if prov_len == 0 {
                        None
                    } else {
                        Some(
                            EmbeddingProvenance::from_bytes(&map[prov_start..prov_end])
                                .map_err(|e| format!("{origin}: {e}"))?,
                        )
                    };
                    let meta_len64 =
                        u64::from_le_bytes(map[prov_end..meta_len_end].try_into().unwrap());
                    let meta_len: usize = meta_len64.try_into().map_err(|_| {
                        format!("{origin}: metadata block length exceeds the address space")
                    })?;
                    let meta_end = meta_len_end.checked_add(meta_len).ok_or_else(|| {
                        format!("{origin}: version-4 metadata length {meta_len} overflows")
                    })?;
                    if map.len() < meta_end {
                        return Err(format!(
                            "{origin}: truncated version-4 metadata block (need {meta_len} bytes)"
                        ));
                    }
                    metadata = crate::metadata::decode(
                        &map[meta_len_end..meta_end],
                        count,
                        origin,
                    )?;
                    let padded = meta_end.div_ceil(4) * 4;
                    if map.len() < padded {
                        return Err(format!(
                            "{origin}: truncated version-4 header (metadata padding)"
                        ));
                    }
                    (padded, fp, provenance)
                }
                v => return Err(format!("{origin}: unsupported .spqv version {v}")),
            };
        // Checked size arithmetic: a malformed header must be rejected here, not wrap
        // around and pass the size check (or panic later in `get`/`iter`).
        let expect = count
            .checked_mul(dim)
            .and_then(|n| n.checked_mul(4))
            .and_then(|data| count.checked_mul(8).and_then(|idx| data.checked_add(idx)))
            .and_then(|body| body.checked_add(data_offset))
            .ok_or_else(|| format!("{origin}: dim={dim} count={count} overflows the file size"))?;
        if map.len() != expect {
            return Err(format!(
                "{origin}: file is {} bytes, expected {expect} for dim={dim} count={count}",
                map.len()
            ));
        }
        // Validate the trailing id→slot index up front (one sequential pass over
        // count·8 bytes — a small fraction of the file; the vector data itself stays
        // untouched/unpaged): ids strictly ascending (sorted + unique, what `get`'s
        // binary search assumes) and every slot in range and used exactly once (what
        // `iter`'s inverse map assumes). After this, no read path can panic on a
        // corrupt index.
        {
            let index_start = data_offset + count * dim * 4;
            let index = &map[index_start..index_start + count * 8];
            let mut slot_seen = vec![false; count];
            let mut prev_id: Option<Id> = None;
            #[cfg(feature = "metadata-sidecar")]
            let mut metadata_matches = 0usize;
            for i in 0..count {
                let id = u32::from_le_bytes(index[i * 8..i * 8 + 4].try_into().unwrap());
                let slot =
                    u32::from_le_bytes(index[i * 8 + 4..i * 8 + 8].try_into().unwrap()) as usize;
                if prev_id.is_some_and(|p| p >= id) {
                    return Err(format!(
                        "{origin}: index entry {i} (id {id}) is not strictly ascending"
                    ));
                }
                prev_id = Some(id);
                if slot >= count || slot_seen[slot] {
                    return Err(format!(
                        "{origin}: index entry {i} has invalid or duplicate slot {slot} (count {count})"
                    ));
                }
                slot_seen[slot] = true;
                #[cfg(feature = "metadata-sidecar")]
                if metadata.contains_key(&id) {
                    metadata_matches += 1;
                }
            }
            #[cfg(feature = "metadata-sidecar")]
            if metadata_matches != metadata.len() {
                return Err(format!(
                    "{origin}: metadata references an id that has no vector"
                ));
            }
        }
        // (sq-i7w) Validation above deliberately reads the canonical LE header,
        // provenance/metadata, and index before this conversion. A BE host cannot cast the LE f32
        // payload directly, so copy the complete backing to aligned owned storage and reverse only
        // the dense words. Structural bytes remain LE and all later explicit from_le_bytes reads are
        // unchanged. The test-only forced branch provides a mutation witness on LE CI hosts.
        let map = if swap_dense_for_native {
            let data_end = data_offset + count * dim * 4;
            map.into_owned_swapped(data_offset, data_end)
        } else {
            map
        };
        Ok(VectorStore {
            dim,
            path,
            backing: Backing::Map(map),
            fingerprint,
            provenance,
            data_offset,
            inverse: std::sync::OnceLock::new(),
            #[cfg(feature = "delta")]
            delta: None,
            #[cfg(feature = "metadata-sidecar")]
            metadata,
        })
    }

    /// Appends `vector` for term `id` (build phase only). Errors after
    /// [`finalize`](Self::finalize)/[`open`](Self::open), on a dimension mismatch, on a
    /// non-finite component, on an all-zero vector (no direction — cosine is undefined
    /// on it, so the exact and HNSW searchers could not even agree on its rank), or if
    /// `id` already has a vector.
    pub fn put(&mut self, id: Id, vector: &[f32]) -> Result<(), String> {
        validate_vector(self.dim, id, vector)?;
        let Backing::Build { data, slots, ids } = &mut self.backing else {
            return Err("put on a finalized/opened store (the .spqv file is immutable)".into());
        };
        if slots.contains_key(&id) {
            return Err(format!("id {id} already has a vector"));
        }
        slots.insert(id, ids.len() as u32);
        ids.push(id);
        data.extend_from_slice(vector);
        Ok(())
    }

    /// Appends `vector` for `id` and associates an opaque UTF-8 metadata tag with it.
    ///
    /// Validation and build-phase restrictions are identical to [`put`](Self::put). The tag is
    /// persisted byte-for-byte by [`finalize`](Self::finalize), including an empty string. Writing
    /// the first tag selects the opt-in v4 format; stores built exclusively with `put` retain the
    /// existing v2/v3 format exactly.
    #[cfg(feature = "metadata-sidecar")]
    pub fn put_with_meta(&mut self, id: Id, vector: &[f32], meta: &str) -> Result<(), String> {
        if meta.len() > u32::MAX as usize {
            return Err(format!("metadata for id {id} exceeds the .spqv length limit"));
        }
        self.put(id, vector)?;
        self.metadata.insert(id, meta.to_owned());
        Ok(())
    }

    /// Writes the `.spqv` file and re-opens it memory-mapped; the handle then serves
    /// reads from the map and rejects further [`put`](Self::put)s. Idempotent on an
    /// already-finalized/opened store.
    pub fn finalize(&mut self) -> Result<(), String> {
        let Backing::Build { data, slots, ids } = &mut self.backing else {
            return Ok(());
        };
        let count = ids.len();
        // [FABLE-5] (sq-lhcot.1) Build the header: v3 (with the embedding-provenance block) if a
        // provenance is bound, else v2 exactly as before. `build_header` is the single seam that both
        // this in-RAM writer and the `StreamingWriter` share, so the two writers stay byte-identical.
        let header = build_header(
            self.dim,
            count,
            self.fingerprint,
            self.provenance.as_ref(),
            #[cfg(feature = "metadata-sidecar")]
            (!self.metadata.is_empty()).then_some(&self.metadata),
        );

        let mut index: Vec<(Id, u32)> = slots.iter().map(|(&id, &slot)| (id, slot)).collect();
        index.sort_unstable();
        let mut index_bytes = Vec::with_capacity(count * 8);
        for (id, slot) in index {
            index_bytes.extend_from_slice(&id.to_le_bytes());
            index_bytes.extend_from_slice(&slot.to_le_bytes());
        }

        // f32 → LE bytes: a plain cast on little-endian targets (`create` rejects big-endian
        // above; the read path supports big-endian via swap-on-read).
        // SAFETY: f32 has no invalid bit patterns and align(f32) ≥ align(u8).
        let data_bytes =
            unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 4) };

        let mut file = std::fs::File::create(&self.path)
            .map_err(|e| format!("create {}: {e}", self.path.display()))?;
        use std::io::Write;
        file.write_all(&header)
            .and_then(|()| file.write_all(data_bytes))
            .and_then(|()| file.write_all(&index_bytes))
            .and_then(|()| file.flush())
            .map_err(|e| format!("write {}: {e}", self.path.display()))?;
        // [FABLE-5] (sq-98c) Explicit close before the reopen below. On wasm32 the `File` stub
        // has no `Drop` impl, which trips clippy's `drop_non_drop` there — the close is still
        // intentional on every target that can reach this path (native + WASI).
        #[allow(clippy::drop_non_drop)]
        drop(file);

        let reopened = VectorStore::open(&self.path)?;
        self.backing = reopened.backing;
        self.fingerprint = reopened.fingerprint;
        self.provenance = reopened.provenance;
        self.data_offset = reopened.data_offset;
        self.inverse = std::sync::OnceLock::new();
        #[cfg(feature = "metadata-sidecar")]
        {
            self.metadata = reopened.metadata;
        }
        Ok(())
    }

    /// The vector for term `id`, or `None` if it has none. Works in both phases:
    /// a hash lookup during build, a binary search over the mmap'd index after.
    ///
    /// [OPUS-4.8] (sq-pi44) With the `delta` feature, the in-RAM delta is consulted FIRST: a
    /// tombstoned id reads as absent, an appended/updated id reads its delta vector (shadowing the
    /// base), and any other id reads the immutable base — so search transparently sees the delta.
    pub fn get(&self, id: Id) -> Option<&[f32]> {
        #[cfg(feature = "delta")]
        if let Some(delta) = &self.delta {
            if delta.tombstones.contains(&id) {
                return None;
            }
            if let Some(v) = delta.appended.get(&id) {
                return Some(v.as_slice());
            }
        }
        self.base_get(id)
    }

    /// Returns `id`'s opaque metadata tag, or `None` when the id has no effective vector or was
    /// written with [`put`](Self::put) rather than [`put_with_meta`](Self::put_with_meta).
    #[cfg(feature = "metadata-sidecar")]
    pub fn meta(&self, id: Id) -> Option<&str> {
        self.get(id)?;
        self.metadata.get(&id).map(String::as_str)
    }

    /// The vector for `id` from the immutable base only (no delta) — the original `get` body. The
    /// delta-aware [`get`](Self::get) calls this after consulting the delta.
    fn base_get(&self, id: Id) -> Option<&[f32]> {
        match &self.backing {
            Backing::Build { data, slots, .. } => {
                let slot = *slots.get(&id)? as usize;
                Some(&data[slot * self.dim..(slot + 1) * self.dim])
            }
            Backing::Map(map) => {
                // [OPUS-4.8] (sq-pi44) The BASE count (not the delta-aware `len`): this reads the
                // on-disk index, and the delta-aware `len` calls back into `base_get`, so using it
                // here would recurse infinitely.
                let count = self.base_len();
                let index = &map[self.data_offset + count * self.dim * 4..];
                let id_at = |i: usize| -> Id {
                    u32::from_le_bytes(index[i * 8..i * 8 + 4].try_into().unwrap())
                };
                let (mut lo, mut hi) = (0usize, count);
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    if id_at(mid) < id {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                if lo == count || id_at(lo) != id {
                    return None;
                }
                let slot =
                    u32::from_le_bytes(index[lo * 8 + 4..lo * 8 + 8].try_into().unwrap()) as usize;
                Some(self.slot_vector(map, slot))
            }
        }
    }

    /// Number of stored vectors — the EFFECTIVE count once the in-RAM delta (appended/updated
    /// vectors and tombstones) is applied. With the `delta` feature off, or no delta yet, this is
    /// the base count.
    pub fn len(&self) -> usize {
        #[cfg(feature = "delta")]
        if let Some(delta) = &self.delta {
            // effective = base − (base ids that are tombstoned) − (base ids shadowed by an append)
            //             + (appended ids). Tombstones and appends are disjoint by construction, so
            // a base id is double-counted with the appended set only via the `append` branch.
            let base = self.base_len();
            let removed = delta.tombstones.iter().filter(|&&id| self.base_get(id).is_some()).count();
            let shadowed =
                delta.appended.keys().filter(|&&id| self.base_get(id).is_some()).count();
            return base - removed - shadowed + delta.appended.len();
        }
        self.base_len()
    }

    /// The base (on-disk / build-phase) vector count, ignoring any in-RAM delta — the original
    /// `len` body. The delta-aware [`len`](Self::len) builds on this.
    fn base_len(&self) -> usize {
        match &self.backing {
            Backing::Build { ids, .. } => ids.len(),
            Backing::Map(map) => u64::from_le_bytes(map[12..20].try_into().unwrap()) as usize,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The store's vector dimension.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// [OPUS-4.8] (sq-32i5) The graph fingerprint this store was built against, or `None` for a
    /// legacy version-1 file (predates fingerprinting) or a store finalized without
    /// [`with_fingerprint`](Self::with_fingerprint). See [`check_graph`](Self::check_graph).
    pub fn fingerprint(&self) -> Option<Fingerprint> {
        self.fingerprint
    }

    /// [OPUS-4.8] (sq-32i5) **Checked open guard**: verifies this store was built against `graph`
    /// by recomputing `graph`'s fingerprint and comparing it to the stored one. Returns a
    /// descriptive `Err` if they differ — the store is keyed by `graph`'s dictionary ids, so a
    /// mismatch means a query would silently resolve to the WRONG vectors. A legacy version-1 file
    /// (no stored fingerprint) also errors, as "unverifiable" rather than silently passing.
    ///
    /// Call this once after [`open`](Self::open) (it is O(dict_len), not per-query) before issuing
    /// `get`/`nearest_term` against `graph`. The term-by-query entry points
    /// ([`crate::ann::nearest_term_exact_checked`], `DiskAnnIndex::nearest_term_checked`) run it.
    pub fn check_graph(&self, graph: &Graph) -> fingerprint::CheckResult {
        let origin = if self.path.as_os_str().is_empty() {
            "<bytes>".to_string()
        } else {
            self.path.display().to_string()
        };
        fingerprint::check_against(self.fingerprint, graph, fingerprint::Artifact::Store, &origin)
    }

    /// [FABLE-5] (sq-lhcot.1) The embedding provenance this store was built with, or `None` for a
    /// legacy v1/v2 file (which predate embedding provenance) or a store finalized without
    /// `with_provenance` (feature-gated). See [`check_provenance`](Self::check_provenance).
    /// Always available (the v3 read path is always compiled, even without the `spqv-provenance`
    /// feature).
    pub fn provenance(&self) -> Option<&EmbeddingProvenance> {
        self.provenance.as_ref()
    }

    /// [FABLE-5] (sq-lhcot.1) **Mandatory compatibility guard.** Verifies a query issued under
    /// `query` embedding provenance is compatible with this store's — the SAME model id, model/content
    /// version, metric, normalization, and verbalization regime, AND the same `dim` — so the query
    /// vector lands in the store's embedding space. Returns a descriptive `Err` naming every mismatched
    /// axis; the query would otherwise return arithmetically-defined but semantically WRONG neighbours.
    ///
    /// **Fail-closed for legacy stores.** A v1/v2 store carries no provenance ([`provenance`](Self::provenance)
    /// is `None`). By default (`legacy` = [`LegacyMode::Reject`]) this REJECTS such a store — its
    /// embedding pipeline is unverifiable, so we cannot certify the query is compatible. A caller that
    /// KNOWS the legacy store is compatible can pass [`LegacyMode::Allow`] to bypass the check for a
    /// legacy store only (a v3 store is ALWAYS checked against its recorded provenance regardless of
    /// `legacy`). See [`LegacyMode`].
    ///
    /// The reserved (KERN) extension area of the provenance does NOT participate in the check
    /// (extension fields reserved pending the #1746 profile), so a v3 store written by a future
    /// implementation that populated it stays queryable by this build.
    pub fn check_provenance(
        &self,
        query: &EmbeddingProvenance,
        legacy: LegacyMode,
    ) -> Result<(), String> {
        let origin = if self.path.as_os_str().is_empty() {
            "<bytes>".to_string()
        } else {
            self.path.display().to_string()
        };
        match &self.provenance {
            // Dimension is not an EmbeddingProvenance field: a query vector of the wrong width is
            // already a hard error on the search/`get` path (it compares `dim`-width slices), so the
            // provenance check covers the model/metric/normalization/verbalization axes, and the
            // width axis is enforced structurally by the vector length.
            Some(stored) => stored.compatible_with(query).map_err(|e| format!("{origin}: {e}")),
            None => match legacy {
                LegacyMode::Reject => Err(format!(
                    "{origin}: this store carries no embedding provenance (a legacy v1/v2 .spqv, or \
                     one finalized without binding a provenance) and so cannot be verified compatible \
                     with the query embedder; rebuild it with an embedding provenance (v3), or pass \
                     LegacyMode::Allow if you KNOW the store was built with a compatible pipeline",
                )),
                LegacyMode::Allow => Ok(()),
            },
        }
    }

    /// Iterates all `(id, vector)` pairs — what ANN index construction consumes. Base entries come
    /// first in insertion-slot order, then any delta-appended entries.
    ///
    /// [OPUS-4.8] (sq-pi44) With the `delta` feature, the EFFECTIVE set is yielded: a tombstoned
    /// base id is skipped, a base id shadowed by a delta append is skipped (the appended vector is
    /// yielded instead, from the delta tail), and delta-only appends are appended after the base —
    /// so search over `iter` transparently unions base+delta and honours tombstones.
    pub fn iter(&self) -> impl Iterator<Item = (Id, &[f32])> + '_ {
        let base_count = self.base_len();
        let base = (0..base_count)
            .map(move |slot| match &self.backing {
                Backing::Build { data, ids, .. } => {
                    (ids[slot], &data[slot * self.dim..(slot + 1) * self.dim])
                }
                Backing::Map(map) => (self.slot_id(map, slot), self.slot_vector(map, slot)),
            })
            .filter(move |&(id, _)| self.base_id_is_live(id));
        base.chain(self.delta_iter())
    }

    /// Whether base id `id` survives the delta into the effective view: not tombstoned and not
    /// shadowed by a delta append (the append is yielded from the delta tail instead, to avoid a
    /// duplicate). Always `true` with the `delta` feature off / no delta.
    #[allow(unused_variables)]
    fn base_id_is_live(&self, id: Id) -> bool {
        #[cfg(feature = "delta")]
        if let Some(delta) = &self.delta {
            return !delta.tombstones.contains(&id) && !delta.appended.contains_key(&id);
        }
        true
    }

    /// The delta-appended `(id, vector)` pairs (empty with the `delta` feature off / no delta), to
    /// chain after the base in [`iter`](Self::iter).
    fn delta_iter(&self) -> impl Iterator<Item = (Id, &[f32])> + '_ {
        #[cfg(feature = "delta")]
        {
            self.delta
                .iter()
                .flat_map(|d| d.appended.iter().map(|(&id, v)| (id, v.as_slice())))
        }
        #[cfg(not(feature = "delta"))]
        {
            std::iter::empty()
        }
    }

    /// The id stored at insertion `slot` (mmap mode). The on-disk index only maps
    /// id→slot (sorted by id, for `get`'s binary search); the slot→id direction is
    /// rebuilt lazily in RAM rather than duplicated in the file.
    fn slot_id(&self, map: &Bytes, slot: usize) -> Id {
        self.inverse_ids(map)[slot]
    }

    /// slot→id for mmap mode, built once (O(count)) and cached.
    fn inverse_ids(&self, map: &Bytes) -> &[Id] {
        self.inverse.get_or_init(|| {
            let count = u64::from_le_bytes(map[12..20].try_into().unwrap()) as usize;
            let index = &map[self.data_offset + count * self.dim * 4..];
            let mut inv = vec![0 as Id; count];
            for i in 0..count {
                let id = u32::from_le_bytes(index[i * 8..i * 8 + 4].try_into().unwrap());
                let slot =
                    u32::from_le_bytes(index[i * 8 + 4..i * 8 + 8].try_into().unwrap()) as usize;
                inv[slot] = id;
            }
            inv
        })
    }

    fn slot_vector<'m>(&self, map: &'m Bytes, slot: usize) -> &'m [f32] {
        let start = self.data_offset + slot * self.dim * 4;
        let bytes = &map[start..start + self.dim * 4];
        debug_assert_eq!(bytes.as_ptr() as usize % std::mem::align_of::<f32>(), 0);
        // SAFETY: the backing base is 4-byte aligned — a memory map is page-aligned, and the
        // owned path uses `AlignedBytes` (u32-backed, alignment 4) precisely so this cast is
        // aligned ([OPUS-4.8] review 1874; a raw Vec<u8> is alignment 1 and would be UB here).
        // `start` is a multiple of 4 (data_offset [32 or 56] + slot·dim·4), so the pointer stays
        // f32-aligned; the range is in bounds (validated in `open`); f32 accepts any bit pattern;
        // the slice borrows the backing, owned by `self`.
        unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const f32, self.dim) }
    }
}

// [OPUS-4.8] (sq-pi44) The incremental delta layer: add / remove / update against a finalized store
// plus `compact`. Gated behind the opt-in `delta` feature so the default build carries no delta
// surface. See [`crate::delta`] for the design and the honest in-RAM-only scope.
#[cfg(feature = "delta")]
impl VectorStore {
    /// The in-RAM delta sidecar, allocated on first mutation and bound to the base store's
    /// fingerprint (the generation the appended/tombstoned ids are keyed against).
    fn delta_mut(&mut self) -> &mut crate::delta::VectorDelta {
        let generation = self.fingerprint;
        self.delta.get_or_insert_with(|| crate::delta::VectorDelta::new(generation))
    }

    /// [OPUS-4.8] (sq-pi44) **Add a NEW vector** for term `id` against an already-finalized store,
    /// writing it to the in-RAM delta (no file rebuild). Errors on the same vector validation as
    /// [`put`](Self::put) (dimension, finiteness, non-zero direction), or if `id` ALREADY has an
    /// effective vector (present in the base and not tombstoned, or already in the delta) — use
    /// [`update`](Self::update) to replace an existing one. May be called before or after
    /// `finalize`; on a build-phase store it is equivalent to `put`.
    pub fn add(&mut self, id: Id, vector: &[f32]) -> Result<(), String> {
        validate_vector(self.dim, id, vector)?;
        // On a build-phase store, route to the in-RAM data exactly like `put` (no delta needed).
        if matches!(self.backing, Backing::Build { .. }) {
            return self.put(id, vector);
        }
        if self.get(id).is_some() {
            return Err(format!(
                "id {id} already has a vector; use `update` to replace it"
            ));
        }
        let delta = self.delta_mut();
        delta.tombstones.remove(&id);
        delta.appended.insert(id, vector.to_vec());
        Ok(())
    }

    /// [OPUS-4.8] (sq-pi44) **Remove** term `id`'s vector from the effective view by tombstoning it
    /// in the in-RAM delta (no file rebuild). A subsequent [`get`](Self::get)/[`iter`](Self::iter)
    /// excludes it. Works whether the id lives in the base or only in the delta. Returns `true` if
    /// the id had an effective vector that this removed, `false` if it had none (a no-op). On a
    /// build-phase store the vector is dropped directly from the in-RAM build set.
    pub fn remove(&mut self, id: Id) -> bool {
        let dim = self.dim;
        // Build phase: drop directly from the accumulating data (re-pack the dense slots).
        if let Backing::Build { data, slots, ids } = &mut self.backing {
            let Some(slot) = slots.remove(&id) else { return false };
            let slot = slot as usize;
            // Remove the dense vector and the id at `slot`, then renumber the slots after it.
            data.drain(slot * dim..(slot + 1) * dim);
            ids.remove(slot);
            for s in slots.values_mut() {
                if (*s as usize) > slot {
                    *s -= 1;
                }
            }
            #[cfg(feature = "metadata-sidecar")]
            self.metadata.remove(&id);
            return true;
        }
        let had = self.get(id).is_some();
        if had {
            let delta = self.delta_mut();
            delta.appended.remove(&id);
            delta.tombstones.insert(id);
            #[cfg(feature = "metadata-sidecar")]
            self.metadata.remove(&id);
        }
        had
    }

    /// [OPUS-4.8] (sq-pi44) **Update** (replace) the vector for an EXISTING term `id`, writing the
    /// new vector to the in-RAM delta (no file rebuild). Errors on the same vector validation as
    /// [`put`](Self::put), or if `id` has no effective vector to replace (use [`add`](Self::add)
    /// for a new id). On a build-phase store it rewrites the in-RAM vector in place.
    pub fn update(&mut self, id: Id, vector: &[f32]) -> Result<(), String> {
        validate_vector(self.dim, id, vector)?;
        let dim = self.dim;
        if self.get(id).is_none() {
            return Err(format!("id {id} has no vector to update; use `add` for a new id"));
        }
        // Build phase: rewrite the dense slot in place.
        if let Backing::Build { data, slots, .. } = &mut self.backing {
            if let Some(&slot) = slots.get(&id) {
                let slot = slot as usize;
                data[slot * dim..(slot + 1) * dim].copy_from_slice(vector);
                return Ok(());
            }
        }
        let delta = self.delta_mut();
        delta.tombstones.remove(&id);
        delta.appended.insert(id, vector.to_vec());
        Ok(())
    }

    /// [OPUS-4.8] (sq-pi44) Whether this store has any pending in-RAM delta (appends or tombstones)
    /// not yet folded into the base by [`compact`](Self::compact).
    pub fn has_delta(&self) -> bool {
        self.delta.as_ref().is_some_and(|d| !d.is_empty())
    }

    /// [OPUS-4.8] (sq-pi44) The pending delta (appended/updated vectors + tombstones + the
    /// generation it is bound to), or `None` if none has been started. Read-only; for the
    /// generation-tie staleness check ([`apply_delta`](Self::apply_delta)) and introspection.
    pub fn delta(&self) -> Option<&crate::delta::VectorDelta> {
        self.delta.as_ref()
    }

    /// [OPUS-4.8] (sq-pi44) Removes and returns the pending in-RAM delta, leaving the store reading
    /// the bare base again. Pair with [`apply_delta`](Self::apply_delta) to move a delta from one
    /// store handle to another (the generation tie guards a mismatch).
    pub fn take_delta(&mut self) -> Option<crate::delta::VectorDelta> {
        self.delta.take()
    }

    /// [OPUS-4.8] (sq-pi44) Installs `delta` onto this store, REJECTING it if its bound graph
    /// generation does not match this base store's fingerprint — a delta built against generation N
    /// must not be applied to a base of generation M ≠ N (its ids would mis-key). Both a present
    /// mismatch and a delta-without-generation against a fingerprinted base (or vice-versa) are
    /// errors; only an exact generation match (or both `None`, the unverified-by-design case) is
    /// accepted. Replaces any delta already present.
    pub fn apply_delta(&mut self, delta: crate::delta::VectorDelta) -> Result<(), String> {
        if delta.generation() != self.fingerprint {
            return Err(format!(
                "delta generation mismatch: the delta was built against {}, this store is {} — \
                 applying it would mis-key the appended/tombstoned ids",
                describe_generation(delta.generation()),
                describe_generation(self.fingerprint),
            ));
        }
        self.delta = Some(delta);
        Ok(())
    }

    /// [OPUS-4.8] (sq-pi44) **Compaction**: folds the in-RAM delta into a FRESH base `.spqv` at
    /// `out_path`, returning the opened, fully validated store. The result is **equivalent to a
    /// from-scratch rebuild over the same final vector set**: its `get`/`iter`/`len` agree exactly
    /// with a store built by `create` + `put` over `self.iter()`'s effective `(id, vector)` pairs.
    /// The new base is bound to `graph`'s fingerprint (pass the SAME graph the effective ids are
    /// keyed by); the compacted store carries no delta.
    ///
    /// `out_path` may equal the base path — the new file is written first, then opened, so a
    /// same-path compaction replaces the old base atomically enough for the single-writer contract
    /// (concurrent external mutation is out of contract, as for [`open`](Self::open)).
    pub fn compact<P: Into<PathBuf>>(
        &self,
        out_path: P,
        graph: &Graph,
    ) -> Result<VectorStore, String> {
        let out_path = out_path.into();
        let mut fresh = VectorStore::create(&out_path, self.dim)?.with_fingerprint(graph);
        // [FABLE-5] (sq-lhcot.1) Carry the source store's embedding provenance forward, so a v3 store
        // stays v3 (and mandatory-compatibility-checkable) after a compaction. Set the field directly
        // (the public `with_provenance` builder is `spqv-provenance`-gated; `compact` is `delta`-gated,
        // and the provenance value is always present on the read-path struct). A v2/v1 source has
        // `None` here, so its compaction stays v2 — the format is preserved across compaction.
        fresh.provenance = self.provenance.clone();
        // Collect first so the new file is independent of `self`'s mmap (which may be the same
        // path): `iter()` yields the effective base+delta view in a deterministic order, and `put`
        // re-sorts the id→slot index on `finalize`, so the output is order-independent of the
        // collection order — i.e. byte-for-byte a from-scratch rebuild for the same id→vector map.
        let pairs: Vec<(Id, Vec<f32>)> =
            self.iter().map(|(id, v)| (id, v.to_vec())).collect();
        for (id, v) in &pairs {
            #[cfg(feature = "metadata-sidecar")]
            if let Some(meta) = self.meta(*id) {
                fresh.put_with_meta(*id, v, meta)?;
                continue;
            }
            fresh.put(*id, v)?;
        }
        fresh.finalize()?;
        Ok(fresh)
    }

    /// [OPUS-4.8] (sq-7e50) The conventional persisted-delta sidecar path for a `.spqv` base at
    /// `base`: the base path with `d` substituted for the trailing `v` (`foo.spqv` → `foo.spqd`),
    /// or `base` + `.spqd` if it does not end in `.spqv`. Used by [`save_delta`](Self::save_delta)
    /// and [`open_with_delta`](Self::open_with_delta) so a delta lives next to its base by default.
    pub fn sibling_delta_path(base: &Path) -> PathBuf {
        let s = base.as_os_str().to_string_lossy();
        if let Some(stem) = s.strip_suffix(".spqv") {
            PathBuf::from(format!("{}.spqd", stem))
        } else {
            let mut p = base.as_os_str().to_os_string();
            p.push(".spqd");
            PathBuf::from(p)
        }
    }

    /// [OPUS-4.8] (sq-7e50) **Persists the in-RAM delta** to a `.spqd` sidecar at
    /// [`sibling_delta_path`](Self::sibling_delta_path) of this store's base path, so incremental
    /// add/remove/update survive a process restart without a [`compact`](Self::compact). Returns the
    /// sidecar path written. Crash-durable: the bytes are written to a sibling `.spqd-tmp`, `fsync`ed
    /// (`sync_all`), then atomically `rename`d over the final path — a crash mid-write leaves the
    /// previous sidecar (or none) intact, never a half-written file at the live path (the same
    /// discipline as [`StreamingWriter`]). The header carries this store's bound graph fingerprint
    /// (the generation tie), so [`open_with_delta`](Self::open_with_delta) rejects the sidecar
    /// against a mismatched base.
    ///
    /// A store with NO pending delta writes an empty (header-only) sidecar — replaying it is a no-op,
    /// and it still carries the generation so a stale base is still caught. See [`save_delta_to`](Self::save_delta_to) to
    /// choose the path explicitly.
    pub fn save_delta(&self) -> Result<PathBuf, String> {
        let path = Self::sibling_delta_path(&self.path);
        self.save_delta_to(&path)?;
        Ok(path)
    }

    /// [OPUS-4.8] (sq-7e50) Like [`save_delta`](Self::save_delta) but persists the `.spqd` sidecar to
    /// an explicit `path`. Serializes the in-RAM delta (or an empty delta bound to this store's
    /// fingerprint if none has been started — so the generation tie is still recorded), writing it
    /// crash-durably (tmp + `sync_all` + atomic rename).
    pub fn save_delta_to<P: AsRef<Path>>(&self, path: P) -> Result<(), String> {
        let path = path.as_ref();
        // An unstarted delta persists as an empty delta bound to THIS store's generation, so a
        // reopened base still validates the (empty) sidecar against its fingerprint.
        let bytes = match &self.delta {
            Some(d) => d.to_bytes(self.dim),
            None => crate::delta::VectorDelta::new(self.fingerprint).to_bytes(self.dim),
        };
        let tmp = {
            let mut p = path.as_os_str().to_os_string();
            p.push("-tmp");
            PathBuf::from(p)
        };
        // Write the full sidecar to the tmp path, fsync it, then atomically rename it over the
        // final path. On any error the partial tmp is removed so a live `.spqd` is never replaced
        // by a half-written one.
        let write_tmp = || -> std::io::Result<()> {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, path)
        };
        write_tmp().map_err(|e| {
            std::fs::remove_file(&tmp).ok();
            format!("save_delta {}: {e}", path.display())
        })
    }

    /// [OPUS-4.8] (sq-7e50) Whether a persisted `.spqd` delta sidecar exists at
    /// [`sibling_delta_path`](Self::sibling_delta_path) of `base`.
    pub fn has_persisted_delta(base: &Path) -> bool {
        Self::sibling_delta_path(base).exists()
    }

    /// [OPUS-4.8] (sq-7e50) **Opens a `.spqv` base AND replays its persisted `.spqd` delta**, so a
    /// store reopened after a restart reads the SAME effective (base + delta) view it had before the
    /// handle dropped — no [`compact`](Self::compact) required. Equivalent to
    /// [`open`](Self::open)`(base)` followed by reading the sidecar at
    /// [`sibling_delta_path`](Self::sibling_delta_path) and [`apply_delta`](Self::apply_delta)ing it,
    /// so all of the existing guards apply:
    ///
    ///   * the base is fully validated exactly as [`open`](Self::open),
    ///   * the persisted delta's `dim` must match the base (else a descriptive `Err`),
    ///   * the persisted delta's base-generation fingerprint must match the base store's bound
    ///     fingerprint — `apply_delta` REJECTS a sidecar written against a different graph generation
    ///     (the sq-32i5 generation tie), so a stale `.spqd` against a rebuilt base can never mis-key,
    ///   * a truncated / corrupt sidecar is rejected by [`VectorDelta::from_bytes`](crate::delta::VectorDelta)
    ///     (the exact-length partial-write guard), never read out of bounds.
    ///
    /// If NO sidecar exists this is exactly [`open`](Self::open) (the store reads the bare base).
    pub fn open_with_delta<P: AsRef<Path>>(base: P) -> Result<VectorStore, String> {
        let base = base.as_ref();
        let delta_path = Self::sibling_delta_path(base);
        Self::open_with_delta_at(base, &delta_path)
    }

    /// [OPUS-4.8] (sq-7e50) Like [`open_with_delta`](Self::open_with_delta) but reads the persisted
    /// delta from an explicit `delta_path`. If `delta_path` does not exist, returns the bare opened
    /// base (no error — a base with no sidecar is the no-delta case).
    pub fn open_with_delta_at<P: AsRef<Path>, Q: AsRef<Path>>(
        base: P,
        delta_path: Q,
    ) -> Result<VectorStore, String> {
        let base = base.as_ref();
        let delta_path = delta_path.as_ref();
        let mut store = VectorStore::open(base)?;
        if !delta_path.exists() {
            return Ok(store);
        }
        let bytes = std::fs::read(delta_path)
            .map_err(|e| format!("read {}: {e}", delta_path.display()))?;
        let (delta, delta_dim) = crate::delta::VectorDelta::from_bytes(&bytes)
            .map_err(|e| format!("{}: {e}", delta_path.display()))?;
        if delta_dim != store.dim {
            return Err(format!(
                "{}: delta dimension {} does not match the base store dimension {}",
                delta_path.display(),
                delta_dim,
                store.dim
            ));
        }
        // `apply_delta` enforces the generation tie (delta fingerprint == base fingerprint).
        store
            .apply_delta(delta)
            .map_err(|e| format!("{}: {e}", delta_path.display()))?;
        Ok(store)
    }
}

/// [OPUS-4.8] (sq-pi44) A one-line description of a delta/store generation for the mismatch error.
#[cfg(feature = "delta")]
fn describe_generation(fp: Option<Fingerprint>) -> String {
    match fp {
        Some(f) => format!(
            "generation(dict_len={}, triples={}, hash={:#018x})",
            f.dict_len, f.triple_count, f.content_hash
        ),
        None => "an unbound/unverified generation".to_string(),
    }
}

/// [FABLE-5] (sq-lhcot.1) Builds the `.spqv` header bytes for a store with `count` vectors of the
/// given `dim`, optional graph `fingerprint`, optional embedding `provenance`, and (when enabled)
/// optional metadata. Emits v4 when metadata is present, v3 when only provenance is present, and v2
/// otherwise. The single header seam both the in-RAM [`VectorStore::finalize`] and the
/// [`StreamingWriter`] share, so untagged stores agree byte-for-byte.
fn build_header(
    dim: usize,
    count: usize,
    fingerprint: Option<Fingerprint>,
    provenance: Option<&EmbeddingProvenance>,
    #[cfg(feature = "metadata-sidecar")] metadata: Option<&FxHashMap<Id, String>>,
) -> Vec<u8> {
    // The provenance-block bytes (empty ⇒ v2). The `EmbeddingProvenance` codec is always compiled
    // (v3 read path), so this seam works in both feature states — v3 is written only when a
    // provenance was bound, which itself requires the feature-gated `with_provenance`.
    let prov_bytes = provenance.map(|p| p.to_bytes());
    let mut header = vec![0u8; HEADER_LEN];
    header[0..4].copy_from_slice(&SPQV_MAGIC);
    #[cfg(feature = "metadata-sidecar")]
    let metadata_bytes = metadata.map(crate::metadata::encode);
    // Version tag: v4 iff metadata is present, otherwise v3 iff provenance is present, else v2.
    #[cfg(feature = "metadata-sidecar")]
    let version = if metadata_bytes.is_some() {
        SPQV_VERSION_V4
    } else if prov_bytes.is_some() {
        SPQV_VERSION_V3
    } else {
        SPQV_VERSION
    };
    #[cfg(not(feature = "metadata-sidecar"))]
    let version = if prov_bytes.is_some() { SPQV_VERSION_V3 } else { SPQV_VERSION };
    header[4..8].copy_from_slice(&version.to_le_bytes());
    header[8..12].copy_from_slice(&(dim as u32).to_le_bytes());
    header[12..20].copy_from_slice(&(count as u64).to_le_bytes());
    // [OPUS-4.8] (sq-32i5) Embed the graph fingerprint (offset 32..56). A store with none writes a
    // zeroed block, which `open` decodes back to `None` ("unverifiable"), NOT a spurious mismatch.
    if let Some(fp) = fingerprint {
        header[HEADER_LEN_V1..HEADER_LEN].copy_from_slice(&fp.to_bytes());
    }
    // [FABLE-5] v3: append the length-prefixed provenance block after the fingerprint block, then
    // ZERO-PAD to the next 4-byte boundary so the dense f32 data section that follows stays
    // 4-byte-aligned (the `slot_vector` f32 cast requires it — a misaligned cast is UB). The stored
    // `prov_len` is the REAL block length; the reader recomputes the same padded data offset.
    #[cfg(feature = "metadata-sidecar")]
    if let Some(metadata_bytes) = metadata_bytes {
        let provenance_bytes = prov_bytes.as_deref().unwrap_or_default();
        header.extend_from_slice(&(provenance_bytes.len() as u32).to_le_bytes());
        header.extend_from_slice(provenance_bytes);
        header.extend_from_slice(&(metadata_bytes.len() as u64).to_le_bytes());
        header.extend_from_slice(&metadata_bytes);
        let pad = (4 - header.len() % 4) % 4;
        header.extend(std::iter::repeat_n(0u8, pad));
        debug_assert_eq!(header.len() % 4, 0, "v4 data section must start 4-byte aligned");
        return header;
    }
    if let Some(bytes) = prov_bytes {
        header.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        header.extend_from_slice(&bytes);
        let pad = (4 - header.len() % 4) % 4;
        header.extend(std::iter::repeat_n(0u8, pad));
        debug_assert_eq!(header.len() % 4, 0, "v3 data section must start 4-byte aligned");
    }
    header
}

/// Shared `put` validation: dimension, finiteness, non-zero direction.
fn validate_vector(dim: usize, id: Id, vector: &[f32]) -> Result<(), String> {
    if vector.len() != dim {
        return Err(format!("vector has dim {}, store has dim {dim}", vector.len()));
    }
    if vector.iter().any(|v| !v.is_finite()) {
        return Err(format!("vector for id {id} has a non-finite component"));
    }
    if vector.iter().all(|&v| v == 0.0) {
        return Err(format!("vector for id {id} is all-zero (cosine is undefined on it)"));
    }
    Ok(())
}

/// A streaming `.spqv` builder for stores whose dense data would not fit in
/// RAM: every [`put`](Self::put) appends the vector straight to the file's
/// data section and spills the id to a sidecar file, so build-phase memory is
/// O(1) regardless of store size. [`finalize`](Self::finalize) sorts the
/// spilled id→slot index (transiently 8 bytes per vector in RAM — a
/// `dim·4 / 8` reduction over the in-RAM builder, 192× at 384-d), appends it,
/// patches the header count, and opens the finished store.
///
/// The output is byte-identical to [`VectorStore::create`]`+put+finalize` for
/// the same `put` sequence and the same fingerprint (same version-2 format). One
/// contract difference, documented: a DUPLICATE id is detected at `finalize`
/// (the sort reveals it) rather than at `put`, since detecting it eagerly
/// would need the in-RAM id set this writer exists to avoid.
///
/// [OPUS-4.8] (sq-32i5) To bind the streaming store to its graph (so a stale store is caught —
/// see [`crate::fingerprint`]), build it with [`create_with_fingerprint`](Self::create_with_fingerprint);
/// a plain [`create`](Self::create) writes an unverifiable store (all-zero fingerprint).
///
/// ```no_run
/// use sparq_vectors::StreamingWriter;
/// let mut w = StreamingWriter::create("/tmp/big.spqv", 384).unwrap();
/// w.put(7, &[0.1; 384]).unwrap();
/// let store = w.finalize().unwrap(); // a normal, validated VectorStore
/// assert!(store.get(7).is_some());
/// ```
pub struct StreamingWriter {
    dim: usize,
    path: PathBuf,
    data: BufWriter<std::fs::File>,
    ids: BufWriter<std::fs::File>,
    ids_path: PathBuf,
    count: u64,
}

impl StreamingWriter {
    /// Starts a streaming store build at `path` (the sidecar id spill lives at
    /// `path` + `.ids-tmp` until [`finalize`](Self::finalize) removes it). The store is written
    /// WITHOUT a graph fingerprint (unverifiable — [`check_graph`](VectorStore::check_graph) errors);
    /// use [`create_with_fingerprint`](Self::create_with_fingerprint) to bind it to a graph.
    pub fn create<P: Into<PathBuf>>(path: P, dim: usize) -> Result<StreamingWriter, String> {
        Self::create_inner(path, dim, None, None)
    }

    /// [OPUS-4.8] (sq-32i5) Like [`create`](Self::create) but embeds `graph`'s fingerprint in the
    /// header, so [`check_graph`](VectorStore::check_graph) can reject the finished store if it is
    /// later queried against a different graph generation. Pass the SAME graph whose term ids the
    /// vectors are keyed by.
    pub fn create_with_fingerprint<P: Into<PathBuf>>(
        path: P,
        dim: usize,
        graph: &Graph,
    ) -> Result<StreamingWriter, String> {
        Self::create_inner(path, dim, Some(Fingerprint::of(graph)), None)
    }

    /// [FABLE-5] (sq-lhcot.1) Like [`create_with_fingerprint`](Self::create_with_fingerprint) but
    /// ALSO binds an embedding `provenance`, so the streamed store is written in the **v3** format
    /// with the provenance in the header (mandatory-compatibility checkable). Gated behind the opt-in
    /// `spqv-provenance` feature — the v3 WRITE surface is opt-in (mirroring how the v2 fingerprint
    /// format was introduced). Pass the SAME graph whose term ids the vectors are keyed by.
    #[cfg(feature = "spqv-provenance")]
    pub fn create_with_provenance<P: Into<PathBuf>>(
        path: P,
        dim: usize,
        graph: &Graph,
        provenance: EmbeddingProvenance,
    ) -> Result<StreamingWriter, String> {
        Self::create_inner(path, dim, Some(Fingerprint::of(graph)), Some(provenance))
    }

    fn create_inner<P: Into<PathBuf>>(
        path: P,
        dim: usize,
        fingerprint: Option<Fingerprint>,
        provenance: Option<EmbeddingProvenance>,
    ) -> Result<StreamingWriter, String> {
        if dim == 0 || dim > u32::MAX as usize {
            return Err(format!("invalid vector dimension {dim}"));
        }
        if cfg!(target_endian = "big") {
            return Err(".spqv is a little-endian format; big-endian targets are unsupported".into());
        }
        let path = path.into();
        let mut file = std::fs::File::create(&path)
            .map_err(|e| format!("create {}: {e}", path.display()))?;
        // [FABLE-5] Header with a zero count placeholder; finalize patches offset 12. The fingerprint
        // (offset 32..56) and — for v3 — the length-prefixed provenance block are known up front, so
        // they are written here (via the shared `build_header` seam), not patched. `build_header`
        // emits v3 iff a provenance is present, else v2 (byte-identical to the pre-v3 streaming header).
        let header = build_header(
            dim,
            0,
            fingerprint,
            provenance.as_ref(),
            #[cfg(feature = "metadata-sidecar")]
            None,
        );
        file.write_all(&header).map_err(|e| format!("write {}: {e}", path.display()))?;
        let ids_path = {
            let mut p = path.clone().into_os_string();
            p.push(".ids-tmp");
            PathBuf::from(p)
        };
        let ids_file = std::fs::File::create(&ids_path)
            .map_err(|e| format!("create {}: {e}", ids_path.display()))?;
        Ok(StreamingWriter {
            dim,
            path,
            data: BufWriter::new(file),
            ids: BufWriter::new(ids_file),
            ids_path,
            count: 0,
        })
    }

    /// Appends `vector` for term `id` to the data section. Validation matches
    /// [`VectorStore::put`] except that a duplicate `id` is reported at
    /// [`finalize`](Self::finalize), not here (see the type docs).
    pub fn put(&mut self, id: Id, vector: &[f32]) -> Result<(), String> {
        validate_vector(self.dim, id, vector)?;
        // f32 → LE bytes: a plain cast on little-endian targets (`create` rejects
        // big-endian).
        // SAFETY: f32 has no invalid bit patterns and align(f32) ≥ align(u8).
        let bytes =
            unsafe { std::slice::from_raw_parts(vector.as_ptr() as *const u8, vector.len() * 4) };
        self.data.write_all(bytes).map_err(|e| format!("write {}: {e}", self.path.display()))?;
        self.ids
            .write_all(&id.to_le_bytes())
            .map_err(|e| format!("write {}: {e}", self.ids_path.display()))?;
        self.count += 1;
        Ok(())
    }

    /// Number of vectors written so far.
    pub fn len(&self) -> usize {
        self.count as usize
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Sorts and appends the id→slot index, patches the header count, fsyncs,
    /// removes the sidecar, and opens the finished store (memory-mapped,
    /// fully validated — exactly as [`VectorStore::open`] would). On error the
    /// partial `.spqv` and sidecar are removed.
    pub fn finalize(self) -> Result<VectorStore, String> {
        let StreamingWriter { dim: _, path, data, ids, ids_path, count } = self;
        let cleanup = |msg: String| -> String {
            std::fs::remove_file(&path).ok();
            std::fs::remove_file(&ids_path).ok();
            msg
        };
        let result = (|| -> Result<VectorStore, String> {
            drop(ids); // flush the spill before reading it back
            let id_bytes = std::fs::read(&ids_path)
                .map_err(|e| format!("read {}: {e}", ids_path.display()))?;
            debug_assert_eq!(id_bytes.len() as u64, count * 4);
            // The transient 8-bytes-per-vector index: (id, slot), sorted by id.
            let mut index: Vec<(Id, u32)> = id_bytes
                .chunks_exact(4)
                .enumerate()
                .map(|(slot, b)| (u32::from_le_bytes(b.try_into().unwrap()), slot as u32))
                .collect();
            index.sort_unstable();
            if let Some(w) = index.windows(2).find(|w| w[0].0 == w[1].0) {
                return Err(format!("id {} has two vectors (duplicate put)", w[0].0));
            }
            let mut file = data
                .into_inner()
                .map_err(|e| format!("flush {}: {e}", path.display()))?;
            let mut index_bytes = Vec::with_capacity(index.len() * 8);
            for (id, slot) in index {
                index_bytes.extend_from_slice(&id.to_le_bytes());
                index_bytes.extend_from_slice(&slot.to_le_bytes());
            }
            file.write_all(&index_bytes)
                .and_then(|()| file.seek(SeekFrom::Start(12)).map(|_| ()))
                .and_then(|()| file.write_all(&count.to_le_bytes()))
                .and_then(|()| file.sync_all())
                .map_err(|e| format!("write {}: {e}", path.display()))?;
            // [FABLE-5] (sq-98c) See `finalize`: intentional close-before-reopen; on wasm32 the
            // `File` stub has no `Drop` impl and clippy's `drop_non_drop` fires there.
            #[allow(clippy::drop_non_drop)]
            drop(file);
            std::fs::remove_file(&ids_path).ok();
            VectorStore::open(&path)
        })();
        result.map_err(cleanup)
    }
}

// [OPUS-4.8] sq-hkud (epic sq-toze, gap MS-G4) — Kani bounded model-checking proof of the
// `.spqv` on-disk-format validator. This is the formal-methods complement to the Miri (UB),
// fuzz (libFuzzer corpus), oracle (deterministic corruption sweep) and ASan (sanitised
// corpus) coverage already in place. Those four EXECUTE the validator on SPECIFIC inputs
// (the corpus, plus whatever libFuzzer reaches); Kani PROVES the safety property over EVERY
// input up to a bounded buffer size — closing the residual "did the corpus miss a hostile
// header/index?" gap that motivated MS-G4. It is a complement, not a replacement: Kani's
// bound is small, so fuzz/oracle/ASan still carry the unbounded + UB coverage.
//
// WHY THIS VALIDATOR IS A GOOD KANI TARGET (the MS-G4 feasibility verdict):
//   * `VectorStore::open_from_bytes` is a PURE function of an owned `Vec<u8>` — no file I/O,
//     no `mmap`, no FFI, no syscalls (the constructs Kani cannot model). It runs the SAME
//     `open_validated` header/length/bounds logic as the mmap'd `open`, so proving it proves
//     the validator that the B5 hostile-on-disk-file boundary depends on.
//   * The state space is BOUNDABLE: the checked size arithmetic ties `count`/`dim` to
//     `map.len()`, so a small symbolic buffer bounds every loop and every allocation. A
//     buffer just past `HEADER_LEN` exercises every branch (magic, version 1/2/other,
//     `dim == 0`, the `checked_mul`/`checked_add` overflow rejections, the truncated-header
//     and exact-size checks, and at least one trip of the ascending-id / in-range-slot
//     index loop).
//
// UNTESTED-HERE: Kani is almost certainly NOT installed in the authoring environment, so
// this harness has NOT been run; it is written against Kani's documented API
// (`kani::any()`, `kani::assume()`, `#[kani::proof]`, `#[kani::unwind]`) and is VALIDATED ON
// THE FIRST CI RUN of the `kani` lane (.github/workflows/kani.yml). It is `#[cfg(kani)]`, so
// the normal `cargo build`/`clippy`/`test` never compile it and the crate is unaffected.
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// The header is the smallest valid `.spqv` prefix; pick a symbolic buffer a little
    /// larger so the harness can also build a well-formed one-entry store (header + one
    /// `dim`-wide f32 vector + one 8-byte index entry) and reach the index-validation loop —
    /// not only the early-return header checks. Kept small so model checking terminates.
    const MAX_LEN: usize = HEADER_LEN + 32;

    /// PROPERTY: `open_from_bytes` (hence the shared `open_validated` validator) never
    /// panics, never reads out of bounds, and never triggers UB for ANY byte buffer up to
    /// `MAX_LEN` bytes — it always returns cleanly (`Ok` for a well-formed buffer, `Err` for
    /// every malformed one). Kani explores all such buffers symbolically. The success path
    /// of `open_from_bytes` does an aligned-copy + the full validation; the failure path is a
    /// plain `Err`. Neither may abort.
    #[kani::proof]
    // (sq-kycq5) [SONNET-4.6] Loop census for this harness: the setup loop `for b in
    // bytes.iter_mut()` runs up to `len` times where `len <= MAX_LEN`; the
    // `AlignedBytes::from_vec` aligned copy covers the same `len` bytes; the fingerprint
    // scan in `Fingerprint::from_bytes_opt` covers FINGERPRINT_LEN = 24 bytes; and the
    // index-validation loop `for i in 0..count` runs at most 2 times within MAX_LEN = 88
    // (a dim=1 count=2 store needs 56 + 8 + 16 = 80 bytes; count=3 needs 92 > 88).
    // All loops are bounded by MAX_LEN = HEADER_LEN + 32 = 56 + 32 = 88.
    // Therefore the required unwind bound is MAX_LEN + 1 = 89. The previous bound of 40
    // was LESS than MAX_LEN (40 < 88) and its comment "// > MAX_LEN" was factually wrong;
    // that bound fired an unwinding assertion in the nightly Kani lane (an INCOMPLETE proof,
    // not a counterexample). See the sq-gnvfc fix for the sibling harness for the same
    // pattern. NOTE: raising the bound may make this harness slower (more unwinding steps);
    // that is acceptable — the lane's per-harness timeout keeps it visible, and an honest
    // timeout beats a false incomplete proof.
    #[kani::unwind(89)] // MAX_LEN + 1 = 88 + 1; bounds every loop in the harness cone
    fn open_from_bytes_never_panics() {
        // A symbolic length in `0..=MAX_LEN`, then a symbolic buffer of that length.
        let len: usize = kani::any();
        kani::assume(len <= MAX_LEN);
        let mut bytes = vec![0u8; len];
        for b in bytes.iter_mut() {
            *b = kani::any();
        }
        // The property is simply: this call returns (it must not panic / OOB / UB). We do
        // not constrain the result — both `Ok` and `Err` are correct outcomes; the harness
        // proves the validator is TOTAL over the bounded input domain.
        let _ = VectorStore::open_from_bytes(bytes);
    }

    /// PROPERTY (focused): a buffer that already carries the magic + version-2 tag still
    /// never panics in the size-arithmetic + index-validation tail. Fixing the prefix
    /// shrinks the symbolic surface so Kani spends its budget on the arithmetic/loop logic
    /// (the part the corpus is least likely to have exhausted) rather than on the magic byte.
    #[kani::proof]
    // [OPUS-4.8] (sq-gnvfc) The harness buffer is HEADER_LEN + 16 = 72 concrete bytes. The
    // setup loop `bytes[8..].iter_mut()` runs 64 iterations, and every other per-element pass
    // over the full 72-byte buffer (aligned copy, the fingerprint scan, the index loop) needs
    // at most 72 steps; 73 > 72 bounds them all. The old bound of 40 < 64 fired an unwinding
    // assertion (an INCOMPLETE proof, not a counterexample) on the setup loop.
    #[kani::unwind(73)]
    fn open_validated_v2_tail_never_panics() {
        let mut bytes = vec![0u8; HEADER_LEN + 16];
        bytes[0..4].copy_from_slice(&SPQV_MAGIC);
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes()); // version 2
        // dim and count are symbolic so the checked-overflow + size-mismatch + index loop
        // are all exercised; the fingerprint block and the trailing bytes stay symbolic too.
        for b in bytes[8..].iter_mut() {
            *b = kani::any();
        }
        let _ = VectorStore::open_from_bytes(bytes);
    }

    // DOMAIN-COVERAGE SELF-CHECK, part 1 of 2 (the sq-og8u8 anti-vacuity pattern). Lesson of
    // sq-sqtk2.1 (2026-07-04): a bound can SILENTLY prune the very inputs a harness means to
    // cover, so it passes VACUOUSLY while reporting nothing wrong. The two totality harnesses
    // above prove `open_from_bytes` NEVER panics over a bounded symbolic byte domain — but
    // that is worthless if the ACCEPT path (the aligned copy + the full header / version /
    // size / fingerprint validation) is unreachable within `MAX_LEN` and every in-domain
    // buffer is rejected at the first size check.
    //
    // The self-check is split in two because a `#[kani::proof]` form of the accept-path pin
    // does NOT terminate practically under Kani 0.67 (measured locally: > 20 min, dominated by
    // CBMC's memory-model churn in the `AlignedBytes` raw-pointer copy + the error-arm
    // `format!`/alloc machinery — the same cost class that keeps the two totality harnesses
    // above over the nightly lane's per-harness budget). Splitting loses nothing: the input is
    // fully CONCRETE, so native execution checks the identical property, and the buffer lies
    // INSIDE the symbolic harnesses' domain, so their (eventual) no-panic/no-UB verdict covers
    // it symbolically too.
    //
    //   part 1 (here, compile-time, evaluated whenever Kani builds this crate): the bounded
    //   domain is big enough to CONTAIN a well-formed store — `MAX_LEN >= HEADER_LEN`. A
    //   re-scope that shrank the fuzzed domain below the minimal valid store goes red at
    //   kani-build time.
    //
    //   part 2 (`domain_bounded_buffer_contains_an_accepted_store` in `fingerprint_tests`,
    //   every `cargo test` run): the minimal well-formed `HEADER_LEN`-byte version-2 buffer —
    //   which part 1 proves in-domain — actually validates `Ok`, so the totality harnesses'
    //   domain genuinely contains an ACCEPTED store and their proof is not vacuously
    //   reject-only. [OPUS-4.8] sq-og8u8
    const _DOMAIN_ADMITS_A_WELL_FORMED_STORE: () =
        assert!(MAX_LEN >= HEADER_LEN, "the fuzzed domain must contain a minimal valid store");

    // DOMAIN-COVERAGE SELF-CHECK for the v2-tail harness (sq-og8u8 anti-vacuity pattern): the
    // focused `open_validated_v2_tail_never_panics` harness fixes a 72-byte (HEADER_LEN + 16)
    // buffer with the magic + version-2 prefix. Its interesting tail is the index-validation
    // loop (`for i in 0..count`), which is only reached when count >= 1 — a bound or budget that
    // silently forced count = 0 would make the harness pass VACUOUSLY over the reject/skip path.
    // With version 2 (data_offset = HEADER_LEN) the body budget is HEADER_LEN + 16 - HEADER_LEN =
    // 16 bytes; a minimal 1-entry dim=1 store needs count*dim*4 + count*8 = 1*4 + 8 = 12 <= 16. ✓
    // Its runtime companion `v2_tail_domain_admits_an_indexed_store` (in `fingerprint_tests`)
    // proves a concrete such buffer validates `Ok`. [OPUS-4.8] sq-gnvfc
    const _V2_TAIL_DOMAIN_ADMITS_AN_INDEXED_STORE: () = assert!(
        HEADER_LEN + 16 >= HEADER_LEN + 4 + 8,
        "v2-tail harness domain must fit >= 1 index entry (dim=1 count=1: 4-byte vector + 8-byte slot)"
    );
}

#[cfg(test)]
mod fingerprint_tests {
    // [OPUS-4.8] (sq-32i5) End-to-end checked-open tests for the `.spqv` store: build against
    // graph A then (1) open/query against A → OK + correct neighbours, (2) open/query against a
    // DIFFERENT graph B → descriptive Err (NOT wrong results), (3) fingerprint round-trips through
    // serialize→deserialize, (4) a legacy version-1 file opens but reports as unverifiable.
    use super::*;
    use crate::ann::{nearest_term_exact, nearest_term_exact_checked};
    use oxrdf::{NamedNode, Term};
    use sparq_core::Graph;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn tmp(tag: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("sparq_fp_{tag}_{}_{n}.spqv", std::process::id()))
    }

    fn graph(ttl: &str) -> Graph {
        Graph::load_str(ttl, "turtle").expect("load test turtle")
    }

    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new(s).unwrap())
    }

    // A and B have DIFFERENT dictionaries and triple counts.
    const A: &str = r#"
        @prefix ex: <http://example.org/> .
        ex:alice ex:knows ex:bob .
        ex:bob ex:knows ex:carol .
    "#;
    const B: &str = r#"
        @prefix ex: <http://example.org/> .
        ex:dave ex:likes ex:eve .
        ex:eve ex:likes ex:frank .
        ex:frank ex:likes ex:gina .
    "#;

    /// Builds a 4-d store over graph `g`, giving `alice` a vector nearest to `bob` and far from
    /// `carol`, so a correct `nearest_term(alice)` ranks `bob` first.
    fn build_store(g: &Graph, path: &std::path::Path) -> VectorStore {
        let alice = g.id_of(&iri("http://example.org/alice")).unwrap();
        let bob = g.id_of(&iri("http://example.org/bob")).unwrap();
        let carol = g.id_of(&iri("http://example.org/carol")).unwrap();
        let mut s = VectorStore::create(path, 4).unwrap().with_fingerprint(g);
        s.put(alice, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        s.put(bob, &[0.9, 0.1, 0.0, 0.0]).unwrap(); // close to alice
        s.put(carol, &[0.0, 0.0, 0.0, 1.0]).unwrap(); // far from alice
        s.finalize().unwrap();
        s
    }

    /// DOMAIN-COVERAGE SELF-CHECK, part 2 of 2 (the sq-og8u8 anti-vacuity pattern — part 1,
    /// the compile-time `MAX_LEN >= HEADER_LEN` binding, lives in `kani_proofs`; see the
    /// rationale there): the minimal well-formed version-2 buffer — exactly `HEADER_LEN`
    /// bytes: magic + version 2 + `dim = 1` + `count = 0` (no data, no index) + an all-zero
    /// fingerprint (decodes to `None`/unverifiable, still valid) — validates `Ok`. Because
    /// part 1 proves this buffer lies INSIDE the Kani totality harnesses' `MAX_LEN` domain,
    /// this test going green means those harnesses' domain genuinely CONTAINS an accepted
    /// store: their no-panic proof is not vacuously covering only the reject branches. If a
    /// format change makes every in-domain buffer rejectable, this goes red on every
    /// `cargo test` run. [OPUS-4.8] sq-og8u8
    #[test]
    fn domain_bounded_buffer_contains_an_accepted_store() {
        let mut bytes = vec![0u8; 56]; // HEADER_LEN — pinned numerically on purpose:
        assert_eq!(bytes.len(), HEADER_LEN, "minimal v2 store is exactly the header");
        bytes[0..4].copy_from_slice(&SPQV_MAGIC);
        bytes[4..8].copy_from_slice(&SPQV_VERSION.to_le_bytes());
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes()); // dim = 1 (dim == 0 is rejected)
        // count = 0 (bytes 12..20 stay zero) ⇒ no data + no index; reserved + fingerprint
        // stay all-zero (a valid v2 store finalized without `with_fingerprint`).
        let store = VectorStore::open_from_bytes(bytes)
            .expect("a minimal well-formed v2 store must validate — else the accept path is vacuous");
        assert_eq!(store.dim(), 1);
        assert!(store.fingerprint().is_none(), "all-zero fingerprint decodes to None");
    }

    /// DOMAIN-COVERAGE SELF-CHECK for the v2-tail harness (sq-og8u8 anti-vacuity pattern,
    /// sq-gnvfc — companion to the compile-time `_V2_TAIL_DOMAIN_ADMITS_AN_INDEXED_STORE`
    /// const in `kani_proofs`). Proves a concrete 72-byte buffer in the
    /// `open_validated_v2_tail_never_panics` harness's domain (the magic + version-2 prefix,
    /// count = 1, dim = 2) validates `Ok` — so the index-validation loop `for i in 0..count`
    /// runs exactly once. This confirms the focused harness genuinely covers the indexed-store
    /// path, not only the size-mismatch / bad-index reject branches. [OPUS-4.8] sq-gnvfc
    #[test]
    fn v2_tail_domain_admits_an_indexed_store() {
        // Buffer layout for a valid v2 store with dim = 2, count = 1:
        //   [0..4]   SPQV_MAGIC
        //   [4..8]   version = 2 (LE u32)
        //   [8..12]  dim = 2 (LE u32)
        //   [12..20] count = 1 (LE u64)
        //   [20..32] reserved (zeros) — fills the rest of HEADER_LEN_V1 (32 bytes)
        //   [32..56] fingerprint block (all-zero = no fingerprint, still a valid v2) — HEADER_LEN
        //   [56..64] vector data: one dim=2 f32 vector (2 * 4 = 8 bytes)
        //   [64..72] index: one entry — id = 1 (u32 LE) + slot = 0 (u32 LE) = 8 bytes
        //   total = 72 bytes = HEADER_LEN + 16
        let mut bytes = vec![0u8; HEADER_LEN + 16];
        assert_eq!(bytes.len(), 72, "buffer must be HEADER_LEN + 16 = 72 bytes");
        bytes[0..4].copy_from_slice(&SPQV_MAGIC);
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes()); // version 2
        bytes[8..12].copy_from_slice(&2u32.to_le_bytes()); // dim = 2
        bytes[12..20].copy_from_slice(&1u64.to_le_bytes()); // count = 1
        // fingerprint block [32..56] stays all-zero (valid: no fingerprint).
        // vector data [56..64]: f32 values [1.0, 0.0] in LE.
        bytes[56..60].copy_from_slice(&1.0f32.to_le_bytes());
        bytes[60..64].copy_from_slice(&0.0f32.to_le_bytes());
        // index [64..72]: entry 0 — id = 1 (u32 LE), slot = 0 (u32 LE).
        bytes[64..68].copy_from_slice(&1u32.to_le_bytes()); // id = 1
        bytes[68..72].copy_from_slice(&0u32.to_le_bytes()); // slot = 0
        let store = VectorStore::open_from_bytes(bytes).expect(
            "72-byte v2 store with dim=2 count=1 must validate — the index loop must be reachable \
             in the focused harness domain",
        );
        assert_eq!(store.dim(), 2);
        assert!(store.fingerprint().is_none(), "all-zero fingerprint block decodes to None");
    }

    #[test]
    fn open_against_build_graph_ok_and_correct() {
        let ga = graph(A);
        let path = tmp("ok");
        build_store(&ga, &path);
        let store = VectorStore::open(&path).unwrap();
        // (1) Checked open against the SAME graph → OK.
        assert!(store.check_graph(&ga).is_ok());
        // ...and the neighbours are correct: bob is alice's nearest.
        let got = nearest_term_exact_checked(&store, &ga, &iri("http://example.org/alice"), 1)
            .expect("checked query against the build graph must succeed");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, iri("http://example.org/bob"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn query_against_different_graph_errs_not_wrong() {
        let ga = graph(A);
        let path = tmp("mismatch");
        build_store(&ga, &path);
        let store = VectorStore::open(&path).unwrap();
        let gb = graph(B); // different dict ids AND triple count
        // (2) Checked open/query against a DIFFERENT graph → descriptive Err, not silent wrong data.
        let err = store.check_graph(&gb).expect_err("a mismatched graph must be rejected");
        assert!(err.contains("fingerprint mismatch"), "err was: {err}");
        let qerr = nearest_term_exact_checked(&store, &gb, &iri("http://example.org/dave"), 1)
            .expect_err("a checked query against a mismatched graph must error");
        assert!(qerr.contains("wrong results"), "err was: {qerr}");
        // The UNCHECKED path is the foot-gun this guards: it does NOT error. We only assert the
        // checked path refuses; the unchecked one is documented as the unsafe form.
        let _silent = nearest_term_exact(&store, &gb, &iri("http://example.org/dave"), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn fingerprint_survives_round_trip() {
        let ga = graph(A);
        let path = tmp("rt");
        let built = build_store(&ga, &path);
        let want = built.fingerprint().expect("built store has a fingerprint");
        // (3) Reopen from disk: the stored fingerprint must equal the one set at build, and the
        // live graph's recomputed fingerprint.
        let reopened = VectorStore::open(&path).unwrap();
        assert_eq!(reopened.fingerprint(), Some(want));
        assert_eq!(reopened.fingerprint(), Some(Fingerprint::of(&ga)));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn store_without_fingerprint_is_unverifiable() {
        // A store finalized WITHOUT `with_fingerprint` writes an all-zero block; check_graph must
        // not silently certify it against any non-empty graph.
        let ga = graph(A);
        let alice = ga.id_of(&iri("http://example.org/alice")).unwrap();
        let path = tmp("nofp");
        let mut s = VectorStore::create(&path, 4).unwrap();
        s.put(alice, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        s.finalize().unwrap();
        let store = VectorStore::open(&path).unwrap();
        // [OPUS-4.8] (sq-32i5) An all-zero fingerprint block decodes to `None`, so check_graph
        // reports it as unverifiable ("carries no graph fingerprint") rather than a spurious
        // "DIFFERENT graph" mismatch — and never silently certifies it.
        assert_eq!(store.fingerprint(), None);
        let err = store
            .check_graph(&ga)
            .expect_err("unbound store must not be certified");
        assert!(err.contains("carries no graph fingerprint"), "err: {err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn legacy_v1_file_opens_but_is_unverifiable() {
        // (4) Back-compat: hand-craft a version-1 `.spqv` (32-byte header, no fingerprint block)
        // and confirm it still OPENS and READS correctly, but `check_graph` reports it as
        // unverifiable (the documented legacy path) rather than silently passing.
        let path = tmp("legacy");
        let dim = 4usize;
        let vectors: [(Id, [f32; 4]); 2] = [(7, [1.0, 0.0, 0.0, 0.0]), (3, [0.0, 1.0, 0.0, 0.0])];
        let count = vectors.len();
        // ids must be ascending in the index; slots follow insertion order here.
        let mut header = [0u8; HEADER_LEN_V1];
        header[0..4].copy_from_slice(&SPQV_MAGIC);
        header[4..8].copy_from_slice(&1u32.to_le_bytes()); // version 1
        header[8..12].copy_from_slice(&(dim as u32).to_le_bytes());
        header[12..20].copy_from_slice(&(count as u64).to_le_bytes());
        let mut bytes = header.to_vec();
        for (_, v) in &vectors {
            for x in v {
                bytes.extend_from_slice(&x.to_le_bytes());
            }
        }
        // index: (id, slot) sorted by id ascending → (3, slot1), (7, slot0)
        let mut idx: Vec<(Id, u32)> =
            vectors.iter().enumerate().map(|(slot, (id, _))| (*id, slot as u32)).collect();
        idx.sort_unstable();
        for (id, slot) in idx {
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend_from_slice(&slot.to_le_bytes());
        }
        std::fs::write(&path, &bytes).unwrap();

        let store = VectorStore::open(&path).expect("a legacy v1 file must still open");
        assert!(store.fingerprint().is_none(), "v1 file carries no fingerprint");
        // Reads work against the v1 (offset-32) layout.
        assert_eq!(store.get(7), Some(&[1.0, 0.0, 0.0, 0.0][..]));
        assert_eq!(store.get(3), Some(&[0.0, 1.0, 0.0, 0.0][..]));
        // ...but it cannot be verified against a graph.
        let err = store.check_graph(&graph(A)).expect_err("a v1 file must not certify a graph");
        assert!(err.contains("predates graph-fingerprinting") || err.contains("legacy"), "err: {err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn streaming_writer_with_fingerprint_round_trips() {
        let ga = graph(A);
        let alice = ga.id_of(&iri("http://example.org/alice")).unwrap();
        let path = tmp("stream");
        let mut w = StreamingWriter::create_with_fingerprint(&path, 4, &ga).unwrap();
        w.put(alice, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        let store = w.finalize().unwrap();
        assert_eq!(store.fingerprint(), Some(Fingerprint::of(&ga)));
        assert!(store.check_graph(&ga).is_ok());
        assert!(store.check_graph(&graph(B)).is_err());
        std::fs::remove_file(&path).ok();
    }
}

// [FABLE-5] (sq-lhcot.1) ALWAYS-COMPILED tests for the `.spqv` v3 embedding-provenance READ path and
// the `check_provenance` guard. Compiled in BOTH feature states: the v3 READ path (and `build_header`
// / the `EmbeddingProvenance` codec) is always present, so a v3 file written by a feature-on build
// must OPEN on a feature-off build — this module synthesizes a v3 file with the always-compiled
// `build_header` (no `with_provenance` needed) and proves it opens and checks correctly regardless of
// the `spqv-provenance` feature. Direct unit tests for every new public read-path fn (coverage floor).
#[cfg(test)]
mod provenance_read_tests {
    use super::*;
    use crate::spqv_provenance::{EmbeddingProvenance, Metric, Normalization};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn tmp(tag: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("sparq_v3read_{tag}_{}_{n}.spqv", std::process::id()))
    }

    fn prov() -> EmbeddingProvenance {
        let mut p = EmbeddingProvenance::new("model-A", Metric::Cosine, Normalization::L2);
        p.model_version = "v1".into();
        p.verbalization = "entity-verbalized".into();
        p
    }

    /// Synthesize a real v3 `.spqv` file for `(dim, ids→vectors, provenance)` using ONLY the
    /// always-compiled `build_header` seam (no `with_provenance`, so this works with the feature off).
    fn write_v3(path: &std::path::Path, dim: usize, rows: &[(Id, Vec<f32>)], p: &EmbeddingProvenance) {
        let count = rows.len();
        let header = build_header(
            dim,
            count,
            None,
            Some(p),
            #[cfg(feature = "metadata-sidecar")]
            None,
        );
        let mut bytes = header;
        // Dense data in insertion-slot order.
        for (_, v) in rows {
            for &x in v {
                bytes.extend_from_slice(&x.to_le_bytes());
            }
        }
        // id→slot index sorted by id.
        let mut index: Vec<(Id, u32)> =
            rows.iter().enumerate().map(|(slot, (id, _))| (*id, slot as u32)).collect();
        index.sort_unstable();
        for (id, slot) in index {
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend_from_slice(&slot.to_le_bytes());
        }
        std::fs::write(path, &bytes).unwrap();
    }

    #[test]
    fn v3_file_opens_and_exposes_provenance_in_both_feature_states() {
        let path = tmp("open");
        let rows = vec![(1u32, vec![1.0, 0.0, 0.0, 0.0]), (2u32, vec![0.0, 1.0, 0.0, 0.0])];
        write_v3(&path, 4, &rows, &prov());
        let store = VectorStore::open(&path).expect("a v3 file must open regardless of the feature");
        assert_eq!(store.provenance(), Some(&prov()), "provenance read from the v3 header");
        // Data reads correctly past the variable-length v3 header (data_offset from the header).
        assert_eq!(store.get(1), Some(&[1.0, 0.0, 0.0, 0.0][..]));
        assert_eq!(store.get(2), Some(&[0.0, 1.0, 0.0, 0.0][..]));
        assert_eq!(store.len(), 2);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn check_provenance_accepts_compatible_and_rejects_incompatible() {
        let path = tmp("check");
        write_v3(&path, 4, &[(1u32, vec![1.0, 0.0, 0.0, 0.0])], &prov());
        let store = VectorStore::open(&path).unwrap();
        // Compatible query (same provenance) → Ok.
        assert!(store.check_provenance(&prov(), LegacyMode::Reject).is_ok());
        // Incompatible (wrong metric) → Err naming the axis.
        let mut q = prov();
        q.metric = Metric::Euclidean;
        let err = store.check_provenance(&q, LegacyMode::Reject).unwrap_err();
        assert!(err.contains("metric"), "err names the axis: {err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn legacy_v2_store_rejects_by_default_and_allows_on_opt_in() {
        // A plain v2 store (no provenance) fails closed by default, passes under LegacyMode::Allow.
        let path = tmp("legacy");
        let mut store = VectorStore::create(&path, 4).unwrap();
        store.put(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        store.finalize().unwrap();
        let store = VectorStore::open(&path).unwrap();
        assert!(store.provenance().is_none());
        assert!(store.check_provenance(&prov(), LegacyMode::Reject).is_err());
        assert!(store.check_provenance(&prov(), LegacyMode::Allow).is_ok());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn corrupt_v3_provenance_block_is_rejected_not_paniced() {
        // A v3 header whose provenance block is malformed (unknown metric tag) must be a descriptive
        // open error, never a panic or an out-of-bounds read (fail-closed).
        let path = tmp("corrupt");
        write_v3(&path, 4, &[(1u32, vec![1.0, 0.0, 0.0, 0.0])], &prov());
        let mut bytes = std::fs::read(&path).unwrap();
        // The provenance block starts at HEADER_LEN + 4; its metric tag is 2 bytes in (after the
        // u16 block version). Corrupt it to an unknown metric tag.
        let metric_tag_off = HEADER_LEN + 4 + 2;
        bytes[metric_tag_off] = 200;
        std::fs::write(&path, &bytes).unwrap();
        let err = VectorStore::open(&path)
            .err()
            .expect("a corrupt v3 provenance block must be rejected");
        assert!(err.contains("metric tag") || err.contains("provenance"), "err: {err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn v3_header_declares_version_3_and_data_offset_shifts() {
        // The header is v3 and the data offset is HEADER_LEN + 4 + prov_len (not the v2 HEADER_LEN).
        let path = tmp("hdr");
        write_v3(&path, 4, &[(1u32, vec![1.0, 0.0, 0.0, 0.0])], &prov());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), SPQV_VERSION_V3);
        let store = VectorStore::open(&path).unwrap();
        // data_offset is past the provenance block; the single vector still reads.
        assert_eq!(store.get(1), Some(&[1.0, 0.0, 0.0, 0.0][..]));
        std::fs::remove_file(&path).ok();
    }
}
