# Native data-directory boot

`SOLID_SERVER_DATA_DIR` (unset by default) selects a data root with the RDF index
at `<data>/index` and resource bytes at `<data>/blobs`. It implies
`PSS_SPARQ_BACKEND=embedded`; explicitly selecting any other backend refuses boot.
Setting it together with a non-empty `SOLID_SERVER_SPARQ_DIR` also refuses boot.
The legacy `SOLID_SERVER_SPARQ_DIR` setting still opens an existing snapshot and
retains the durable-index/in-memory-blob refusal. A directory-backed index opened
through this legacy path also compacts on graceful shutdown.

The default native build opens `FsBlobStore` at `<data>/blobs`. Both
`embedded-sparq` and `blob-fs` are required; a build without either feature refuses
data-directory boot with a message naming the missing feature. Without a data
directory, the native binary uses in-memory blobs.

The boot contract checks `<data>/format.json` before opening either plane:

```json
{"schema":1,"index":{"major":1},"blobs":{"major":1}}
```

An absent marker is published atomically (temporary file, file sync, rename,
data-directory sync) only if both plane directories are absent or empty. Existing
plane content without the marker is refused as a foreign directory. An unknown
schema, incompatible index/blob major, or malformed marker refuses boot. A marker
without an index resumes first boot. Do not remove the marker from an existing store.

`EmbeddedSparqClient::open_or_create(dir)` delegates empty-store creation,
interrupted-swap recovery and exclusive locking to the core. Its I/O result preserves
`WouldBlock` when another holder owns the directory. The index's `<data>/index.lock`
and `<data>/index/sparq.lock` guard the data root; there is no separate `LOCK` file.
The index lock is held before the blob constructor opens the store and sweeps
abandoned writes from `blobs/tmp/`. A second boot that cannot acquire the index
lock never opens or sweeps blobs. `SOLID_SERVER_WAIT_FOR_LOCK_SECS`
(default `0`) is a non-negative integer: data-directory boot retries only
`WouldBlock` with bounded backoff until that duration, then reports the lock path.
Other errors fail immediately.

## Embedded snapshot reopen and lock handles

With `embedded-sparq`, `EmbeddedSparqClient::open(dir)` opens a saved graph and
keeps its directory locks on the graph's actor thread. Dropping the last client
drains queued jobs and joins that thread before returning. A save into the open
directory compacts it; a save elsewhere writes an independent snapshot.

The graph holds `<dir>.lock` beside its directory when available, and `sparq.lock`
inside it. An inside-only holder can serve a store whose parent is not writable,
but cannot compact by renaming the root directory. Never remove these lock files
while the directory may be in use.

On Unix, forked or duplicated descriptors share one open file description and its
advisory lock. Closing the parent's descriptor alone can leave that lock held by a
child between fork and exec. The owning process now explicitly unlocks both files
before closing them, releasing the shared lock even while inherited descriptors
remain open. A guard records its owning process and skips explicit unlock if it is
dropped in a forked child, preserving the parent's live lock. Staging locks and
inside locks moved during a swap use the same release rule. Independent shared
readers and mapped views retain their locks until their final owners drop.

The embedded snapshot opener attempts the graph open once and immediately refuses
contention from a live holder. It needs no retry for a just-dropped graph's inherited
handles. Data-directory boot keeps its separate configured wait, including the default
zero wait, and exponential backoff starting at 10 ms and capped at 250 ms.
`EmbeddedSparqClient::open_or_create`, `Graph::open`, and `DirLock` also keep immediate
refusal on contention. No directory-lock protocol or stored-file format changes.

The descriptor lifetime follows the documented
[Linux `flock` semantics](https://man7.org/linux/man-pages/man2/flock.2.html) and
[macOS `flock` semantics](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/flock.2.html).

## Durability and shutdown

Durability is checked in both directions. A durable/shared index with in-memory
blobs refuses boot. Durable blobs with an ephemeral index also refuse unless
`SOLID_SERVER_ALLOW_INCONSISTENT_DURABILITY=1` (default unset) explicitly allows
that inconsistent combination; this override never permits a durable index with
ephemeral bytes.

SIGTERM or Ctrl-C stops accepting and drains requests. The periodic reconciler is
stopped and awaited before `EmbeddedSparqClient::compact()` runs on the embedded
actor for a directory-backed index. Successful compaction empties its journals;
a failure is logged and returns a non-zero exit while retaining recovery data.
Dropping the final client joins its actor and explicitly releases its index locks.
