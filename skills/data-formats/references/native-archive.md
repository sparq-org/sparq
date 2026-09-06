# Native archive paging

[GPT-6] `sparq-core`'s opt-in `native-archive` feature includes `mmap` and adds
`sparq_core::archive`. It stores independently addressable datasets, including their
named graphs, in a single immutable file. The existing dictionary, raw or compressed
permutation, predicate-statistics, numeric and temporal codecs are unchanged.

```toml
sparq-core = { version = "0.1", features = ["native-archive"] }
```

```rust
use sparq_core::{Graph, archive::{NativeArchive, NativeArchiveWriter}};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let data = Graph::load_dataset(
    "<https://e/s> <https://e/p> <https://e/o> <https://e/resource> .",
    "nquads",
)?;
let mut writer = NativeArchiveWriter::create("datasets.spqa")?;
let dataset_id = writer.append(&data, true)?; // native block-compressed indexes
let stats = writer.finish()?;
assert_eq!(stats.datasets, 1);

// SAFETY: this application prevents file modification/truncation until every
// archive handle, returned Graph and fork/snapshot derived from it is dropped.
let archive = unsafe { NativeArchive::open("datasets.spqa")? };
let graph = archive.load_dataset(dataset_id)?;
assert_eq!(graph.named.len(), 1);
# Ok(()) }
```

`create` exclusively creates a new file; it never overwrites an existing archive.
`append(&Graph, compressed)` returns a sequential dataset ID and incorporates pending
overlays in the serialized base without modifying the input graph. It streams native
components directly, without temporary files for each resource. Construction retains
the normal per-graph codec scratch space, one dataset's component metadata and an index
record per appended dataset. An append failure poisons the writer. `finish` writes the
dataset index, publishes the versioned header and synchronizes the file and parent
directory. Its `ArchiveStats` reports dataset count and whole-file logical bytes.
Keep an incomplete destination for diagnosis or discard it explicitly after failure;
only a successful finish is a durable preparation receipt.

`NativeArchive::open` maps the file and checks the container header and dataset index.
`len` returns its dataset count; `is_empty` tests for none. `load_dataset(id)` checks
component bounds/alignment, dictionary records and lookup tables, permutation row IDs
and ordering, compressed blocks, required table lengths and named-graph metadata before
returning an ordinary `Graph`. Missing tables are errors, including tables for which
legacy directory opening can rebuild a fallback. It performs no RDF parsing, index
construction, per-graph file opening or WAL replay. It does traverse the persisted
bytes for validation, so this work belongs in preparation, before readiness.

Container and native-codec validation checks framing and memory safety. The format has
no content checksum or authentication tag: a well-formed change to content can still
pass. Use a trusted immutable preparation artifact and an independently verified
digest/manifest when content integrity is required. Archives must use compatible native
codec and index-build settings; a build requiring absent permutations rejects them.

The archive and every returned graph are `Send + Sync`. Mapped components retain a
reference to the same mapping, so graphs may outlive the `NativeArchive` handle. The
unsafe lifetime requirement still applies to those graphs and their forks. Opening a
read-only handle alone does not stop another process from changing or truncating the
underlying file; the application must enforce immutability. Do not modify a mapped
archive in place. Publish a newly prepared immutable file for a new base generation.

Index and dictionary payload pages remain file backed. Graph descriptors, dictionary
prefix/datatype metadata, predicate statistics, compressed block directories and update
overlays remain on the heap. This API does not make authorization state file backed.
For a server that excludes preparation from measured requests, load every required
dataset once, instantiate and materialize its real authorization state, replay trusted
application journals, and retain all stores before readiness. Record admission failure
if that metadata cannot fit the chosen memory limit. Calling `load_dataset` on a timed
miss repeats validation; this API deliberately does not hide that cost behind a cache.

Updates use the existing mutable overlays; archive graphs have no directory association,
transaction journal or WAL. `Graph::fork` shares archive-backed payloads, including
numeric and temporal columns, while copying extensions and overlays. The application
owns durable update acknowledgments, journal replay and authorization invalidation;
alternatively append the updated graph into a new archive and finish it. Archive
loading restores data only and grants no access rights by itself.
