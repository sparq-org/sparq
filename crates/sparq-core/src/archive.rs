//! Store native datasets in one immutable file with shared mapped views.
//!
//! [GPT-6] This opt-in container preserves the directory format's dictionary,
//! permutation, statistics, numeric and temporal codecs. The writer streams one
//! dataset at a time, without per-resource temporary files. Opening maps one file;
//! loading a dataset validates its tables and constructs ordinary [`Graph`] values.
//! Every derived graph and snapshot owns a reference to that same mapping.
//!
//! This is immutable base storage, not a transaction manager. Updates use the
//! existing in-memory overlays; they survive only when the application journals
//! them or writes a new archive. There are no per-graph file handles or WAL files.
//! Graph descriptors, dictionary metadata, compressed block directories and overlays
//! still consume heap memory. Applications must load and prepare **all** required
//! datasets before announcing readiness if preparation must be outside timing.
//!
//! # Examples
//! ```no_run
//! use sparq_core::{Graph, archive::{NativeArchive, NativeArchiveWriter}};
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let graph = Graph::load_dataset("", "nquads")?;
//! let mut writer = NativeArchiveWriter::create("population.spqa")?;
//! let id = writer.append(&graph, true)?;
//! writer.finish()?;
//! // SAFETY: this application never changes the file while any mapped view lives.
//! let archive = unsafe { NativeArchive::open("population.spqa")? };
//! let prepared = archive.load_dataset(id)?;
//! // Apply authorization preparation here, before serving any request.
//! assert_eq!(prepared.len(), 0);
//! # Ok(()) }
//! ```

use crate::mapped::MappedBytes;
use crate::native_io::NativeSink;
use crate::{dict::Id, Graph, NumData, TempData};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

// Container v1: 40-byte header, aligned native components, per-dataset component
// tables, then fixed 24-byte dataset records (payload start, table start, table size).
const MAGIC: &[u8; 8] = b"SPQARCH1";
const VERSION: u32 = 1;
const HEADER: usize = 40;
const INDEX_RECORD: usize = 24;
// Bound recursive metadata traversal on corrupt/unreasonably nested input.
const MAX_DEPTH: usize = 128;

pub(crate) fn invalid(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("native archive: {message}"),
    )
}

/// Committed archive counts and logical length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveStats {
    /// Number of independently addressable datasets.
    pub datasets: u64,
    /// Entire archive length, including metadata and alignment padding.
    pub bytes: u64,
}

#[derive(Debug)]
pub(crate) struct CountingWriter {
    inner: BufWriter<File>,
    position: u64,
}

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        self.position += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Stream native datasets into a new archive.
///
/// One dataset's component metadata and one fixed-size record per completed dataset
/// are retained during construction. Native codecs retain their normal per-graph
/// scratch requirements. A failed append poisons the writer; finish cannot publish it.
#[derive(Debug)]
pub struct NativeArchiveWriter {
    writer: CountingWriter,
    path: PathBuf,
    datasets: Vec<[u64; 3]>,
    components: Vec<(String, u64, u64)>,
    prefix: String,
    failed: bool,
}

impl NativeArchiveWriter {
    /// Create an archive without replacing an existing file.
    ///
    /// # Errors
    /// Returns an I/O error if the parent is absent or the destination exists.
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        let mut writer = CountingWriter {
            inner: BufWriter::new(file),
            position: 0,
        };
        // No valid magic before finish: interruption cannot masquerade as a ready archive.
        writer.write_all(&[0; HEADER])?;
        Ok(Self {
            writer,
            path,
            datasets: Vec::new(),
            components: Vec::new(),
            prefix: String::new(),
            failed: false,
        })
    }

    /// Append a dataset using raw or native block-compressed permutations.
    ///
    /// Returns its zero-based dataset ID. Pending overlays are folded into the
    /// serialized base without changing the input. Named-graph ordering is preserved.
    ///
    /// # Errors
    /// Returns an error on write failure, invalid graph names, excessive nesting,
    /// or after an earlier error.
    pub fn append(&mut self, graph: &Graph, compressed: bool) -> io::Result<u64> {
        if self.failed {
            return Err(invalid("writer is poisoned by an earlier append failure"));
        }
        self.failed = true;
        self.align()?;
        let start = self.writer.position;
        self.write_graph(graph, compressed, "", 0)?;
        self.align()?;
        let table = self.writer.position;
        self.writer
            .write_all(&(self.components.len() as u64).to_le_bytes())?;
        for (name, offset, length) in self.components.drain(..) {
            let name_len =
                u32::try_from(name.len()).map_err(|_| invalid("component name too long"))?;
            self.writer.write_all(&name_len.to_le_bytes())?;
            self.writer.write_all(name.as_bytes())?;
            self.writer.write_all(&offset.to_le_bytes())?;
            self.writer.write_all(&length.to_le_bytes())?;
        }
        let id = self.datasets.len() as u64;
        self.datasets
            .push([start, table, self.writer.position - table]);
        self.failed = false;
        Ok(id)
    }

    fn write_graph(
        &mut self,
        graph: &Graph,
        compressed: bool,
        prefix: &str,
        depth: usize,
    ) -> io::Result<()> {
        if depth > MAX_DEPTH {
            return Err(invalid("named graph nesting exceeds archive limit"));
        }
        let mut names = std::collections::HashSet::new();
        for (name, _) in &graph.named {
            if !matches!(name, oxrdf::Term::NamedNode(_) | oxrdf::Term::BlankNode(_))
                || !names.insert(name)
            {
                return Err(invalid("invalid or duplicate named graph"));
            }
        }
        self.prefix = prefix.to_owned();
        graph.store.write_native(self, compressed)?;
        graph.dict.write_native(self)?;
        let n = graph.dict.len();
        self.component("numerics.bin", |w| {
            crate::stream_write_numerics_to(w, n, &graph.numerics)
        })?;
        self.component("temporals.bin", |w| {
            crate::stream_write_temporals_to(w, n, &graph.temporals)
        })?;
        for (i, (_, sub)) in graph.named.iter().enumerate() {
            self.write_graph(sub, compressed, &format!("{prefix}named/{i}/"), depth + 1)?;
        }
        if !graph.named.is_empty() {
            self.prefix = prefix.to_owned();
            self.component("named.bin", |w| {
                let mut bytes = Vec::new();
                bytes.extend_from_slice(&crate::NAMED_MANIFEST_MAGIC.to_le_bytes());
                bytes.extend_from_slice(&crate::NAMED_FORMAT_VERSION.to_le_bytes());
                let count = u32::try_from(graph.named.len())
                    .map_err(|_| invalid("too many named graphs"))?;
                bytes.extend_from_slice(&count.to_le_bytes());
                for (name, _) in &graph.named {
                    crate::encode_graph_name(&mut bytes, name);
                }
                w.write_all(&bytes)
            })?;
        }
        Ok(())
    }

    fn align(&mut self) -> io::Result<()> {
        let padding = (8 - self.writer.position % 8) % 8;
        self.writer.write_all(&[0; 8][..padding as usize])
    }

    /// Commit metadata and synchronize the completed archive.
    ///
    /// A successful finish is the durable preparation receipt. On failure the
    /// destination is retained for inspection and must not be treated as ready.
    ///
    /// # Errors
    /// Returns an error after a failed append or if writing/synchronization fails.
    pub fn finish(mut self) -> io::Result<ArchiveStats> {
        if self.failed {
            return Err(invalid("cannot finish a poisoned writer"));
        }
        self.align()?;
        let index = self.writer.position;
        for entry in &self.datasets {
            for value in entry {
                self.writer.write_all(&value.to_le_bytes())?;
            }
        }
        let stats = ArchiveStats {
            datasets: self.datasets.len() as u64,
            bytes: self.writer.position,
        };
        self.writer.flush()?;
        // First make all payload/index bytes durable, then publish the complete header.
        self.writer.inner.get_ref().sync_all()?;
        self.writer.inner.seek(SeekFrom::Start(0))?;
        self.writer.inner.write_all(MAGIC)?;
        self.writer.inner.write_all(&VERSION.to_le_bytes())?;
        self.writer.inner.write_all(&0_u32.to_le_bytes())?;
        self.writer.inner.write_all(&index.to_le_bytes())?;
        self.writer.inner.write_all(&stats.datasets.to_le_bytes())?;
        self.writer.inner.write_all(&stats.bytes.to_le_bytes())?;
        self.writer.inner.flush()?;
        self.writer.inner.get_ref().sync_all()?;
        #[cfg(unix)]
        File::open(
            self.path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?
        .sync_all()?;
        Ok(stats)
    }
}

impl NativeSink for NativeArchiveWriter {
    type Writer = CountingWriter;
    fn component<T>(
        &mut self,
        name: &str,
        write: impl FnOnce(&mut Self::Writer) -> io::Result<T>,
    ) -> io::Result<T> {
        self.align()?;
        let start = self.writer.position;
        let result = write(&mut self.writer)?;
        self.components.push((
            format!("{}{name}", self.prefix),
            start,
            self.writer.position - start,
        ));
        Ok(result)
    }
}

/// A shared mapping of an immutable native archive.
///
/// Container metadata is checked by [`open`](Self::open). Each
/// [`load_dataset`](Self::load_dataset) additionally validates every native component
/// of that dataset. Keep the returned graphs instead of loading them again per query.
pub struct NativeArchive {
    mapping: Arc<memmap2::Mmap>,
    index: usize,
    count: u64,
}

impl std::fmt::Debug for NativeArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeArchive")
            .field("datasets", &self.count)
            .field("bytes", &self.mapping.len())
            .finish()
    }
}

impl NativeArchive {
    /// Map an archive and validate its dataset index.
    ///
    /// # Safety
    /// The archive bytes and file length must remain unchanged until this object
    /// **and every returned graph or snapshot** have been dropped. A read-only file
    /// handle does not prevent another handle/process from modifying the same file.
    ///
    /// # Errors
    /// Returns an error for I/O failures, incomplete headers, or invalid index bounds.
    pub unsafe fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path)?;
        if file.metadata()?.len() < HEADER as u64 {
            return Err(invalid("incomplete header"));
        }
        // SAFETY: the caller guarantees immutable bytes/length for every derived view.
        let mapping = Arc::new(unsafe { memmap2::Mmap::map(&file)? });
        let mut input = Input::new(&mapping);
        if input.take(8)? != MAGIC || input.u32()? != VERSION || input.u32()? != 0 {
            return Err(invalid("unsupported or unfinished header"));
        }
        let index = input.usize()?;
        let count = input.u64()?;
        let length = input.usize()?;
        let index_bytes = usize::try_from(count)
            .ok()
            .and_then(|n| n.checked_mul(INDEX_RECORD))
            .ok_or_else(|| invalid("dataset index length overflows"))?;
        if index < HEADER
            || index % 8 != 0
            || length != mapping.len()
            || index.checked_add(index_bytes) != Some(length)
        {
            return Err(invalid("dataset index is outside the committed file"));
        }
        let archive = Self {
            mapping,
            index,
            count,
        };
        let mut previous_end = HEADER;
        for id in 0..count {
            let (start, table, table_len) = archive.entry(id)?;
            let end = table
                .checked_add(table_len)
                .ok_or_else(|| invalid("dataset extent overflows"))?;
            if start < previous_end
                || start % 8 != 0
                || table % 8 != 0
                || table < start
                || table_len < 8
                || end > index
            {
                return Err(invalid("dataset extents overlap or escape payload"));
            }
            previous_end = end;
        }
        Ok(archive)
    }

    /// Return the number of datasets in the committed archive.
    pub fn len(&self) -> u64 {
        self.count
    }

    /// Return whether the archive contains no datasets.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Validate and load a dataset as shared native mapped views.
    ///
    /// No RDF parsing, index construction, legacy-table fallback or WAL opening
    /// occurs. Returned graphs may use in-memory update overlays and snapshots.
    /// Loading validates all native table bytes; do this before readiness, not per query.
    ///
    /// # Errors
    /// Returns an error for an absent ID, invalid native data, missing required
    /// components, unsupported graph nesting, or malformed component metadata.
    pub fn load_dataset(&self, id: u64) -> io::Result<Graph> {
        let (start, table, length) = self.entry(id)?;
        let mut input = Input::new(&self.mapping[table..table + length]);
        let count = input.usize()?;
        if count > input.remaining() / 21 {
            return Err(invalid("component count exceeds table length"));
        }
        let mut files = BTreeMap::new();
        let mut extents = Vec::new();
        for _ in 0..count {
            let n = input.u32()? as usize;
            let name = std::str::from_utf8(input.take(n)?)
                .map_err(|_| invalid("non-UTF8 component name"))?;
            if name.is_empty()
                || name
                    .split('/')
                    .any(|s| s.is_empty() || s == "." || s == "..")
                || name.contains(['\\', '\0'])
            {
                return Err(invalid("noncanonical component name"));
            }
            let offset = input.usize()?;
            let length = input.usize()?;
            let end = offset
                .checked_add(length)
                .ok_or_else(|| invalid("component extent overflows"))?;
            if offset < start || end > table {
                return Err(invalid("component escapes dataset payload"));
            }
            let bytes = MappedBytes::region(Arc::clone(&self.mapping), offset, length)?;
            if files.insert(name.to_owned(), bytes).is_some() {
                return Err(invalid("duplicate component name"));
            }
            if length > 0 {
                extents.push((offset, end));
            }
        }
        if input.remaining() != 0 {
            return Err(invalid("trailing component metadata"));
        }
        extents.sort_unstable();
        if extents.windows(2).any(|pair| pair[1].0 < pair[0].1) {
            return Err(invalid("native components overlap"));
        }
        let graph = load_graph(&mut files, "", 0)?;
        if !files.is_empty() {
            return Err(invalid("unreferenced or unsupported native component"));
        }
        Ok(graph)
    }

    fn entry(&self, id: u64) -> io::Result<(usize, usize, usize)> {
        if id >= self.count {
            return Err(invalid("dataset ID outside archive"));
        }
        // open checked the entire index extent and count fits usize.
        let offset = self.index + id as usize * INDEX_RECORD;
        let mut input = Input::new(&self.mapping[offset..offset + INDEX_RECORD]);
        Ok((input.usize()?, input.usize()?, input.usize()?))
    }
}

fn component(
    files: &mut BTreeMap<String, MappedBytes>,
    prefix: &str,
    name: &str,
) -> io::Result<MappedBytes> {
    files
        .remove(&format!("{prefix}{name}"))
        .ok_or_else(|| invalid(&format!("missing {prefix}{name}")))
}

fn load_graph(
    files: &mut BTreeMap<String, MappedBytes>,
    prefix: &str,
    depth: usize,
) -> io::Result<Graph> {
    if depth > MAX_DEPTH {
        return Err(invalid("named graph nesting exceeds archive limit"));
    }
    let meta = component(files, prefix, "dict-meta.bin")?;
    let dict = crate::dict::Dict::open_mapped_with(&*meta, meta.len(), |name| {
        component(files, prefix, name)
    })?;
    let store =
        crate::store::TripleStore::open_archive(|name| component(files, prefix, name), dict.len())?;
    let numeric = component(files, prefix, "numerics.bin")?;
    let temporal = component(files, prefix, "temporals.bin")?;
    if dict.len().checked_mul(8) != Some(numeric.len())
        || dict.len().checked_mul(9) != Some(temporal.len())
    {
        return Err(invalid(
            "numeric or temporal table length differs from dictionary",
        ));
    }
    let mut named = Vec::new();
    if let Some(manifest) = files.remove(&format!("{prefix}named.bin")) {
        let mut input = Input::new(&manifest);
        if input.u32()? != crate::NAMED_MANIFEST_MAGIC
            || input.u32()? != crate::NAMED_FORMAT_VERSION
        {
            return Err(invalid("invalid named graph manifest"));
        }
        let count = input.u32()? as usize;
        if count > input.remaining() / crate::MIN_MANIFEST_ENTRY_BYTES {
            return Err(invalid("named graph count exceeds manifest length"));
        }
        let mut position = 12;
        let mut names = std::collections::HashSet::new();
        for i in 0..count {
            let (name, next) =
                crate::decode_graph_name(&manifest, position).map_err(|e| invalid(&e))?;
            if !names.insert(name.clone()) {
                return Err(invalid("duplicate named graph"));
            }
            position = next;
            named.push((
                name,
                load_graph(files, &format!("{prefix}named/{i}/"), depth + 1)?,
            ));
        }
        if position != manifest.len() {
            return Err(invalid("trailing named graph metadata"));
        }
    }
    Ok(Graph {
        dict,
        store,
        numerics: NumData::Mapped(numeric, Default::default()),
        temporals: TempData::Mapped(temporal, Default::default()),
        high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
        named,
        graph_prefix_index: std::sync::Mutex::new(None),
        wal: None,
        txn: None,
    })
}

pub(crate) fn validate_row(
    row: [Id; 3],
    previous: &mut Option<[Id; 3]>,
    dict_len: usize,
) -> io::Result<()> {
    if previous.is_some_and(|last| last >= row)
        || row
            .iter()
            .any(|&id| id == 0 || (!crate::dict::is_inline(id) && id as usize > dict_len))
    {
        return Err(invalid(
            "permutation contains invalid IDs, duplicate rows or unsorted rows",
        ));
    }
    *previous = Some(row);
    Ok(())
}

pub(crate) struct Input<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Input<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }
    pub(crate) fn take(&mut self, length: usize) -> io::Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| invalid("metadata length overflows"))?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| invalid("truncated metadata"))?;
        self.position = end;
        Ok(bytes)
    }
    pub(crate) fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("four bytes"),
        ))
    }
    pub(crate) fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("eight bytes"),
        ))
    }
    pub(crate) fn usize(&mut self) -> io::Result<usize> {
        usize::try_from(self.u64()?).map_err(|_| invalid("metadata integer exceeds platform size"))
    }
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<NativeArchive>();
    assert_send_sync::<Graph>();
};

#[cfg(test)]
#[path = "archive_tests.rs"]
mod tests;
