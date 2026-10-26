# Native data-directory boot

`SOLID_SERVER_DATA_DIR` (unset by default) selects a data root with the RDF index
at `<data>/index` and resource bytes at `<data>/blobs`. It implies
`PSS_SPARQ_BACKEND=embedded`; explicitly selecting any other backend refuses boot.
Setting it together with a non-empty `SOLID_SERVER_SPARQ_DIR` also refuses boot.
The legacy `SOLID_SERVER_SPARQ_DIR` setting still opens an existing snapshot and
retains the durable-index/in-memory-blob refusal.

**Adapter availability:** this build refuses `SOLID_SERVER_DATA_DIR` with
“the filesystem blob store is not built in”. It never substitutes ephemeral
bytes for the requested durable store. The native binary otherwise uses in-memory
blobs, so it is unsuitable for durable production data. Filesystem adapter
integration is tracked by [#7188](https://github.com/sparq-org/sparq/issues/7188).

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
The index opens before the blob constructor runs. `SOLID_SERVER_WAIT_FOR_LOCK_SECS`
(default `0`) is a non-negative integer: data-directory boot retries only
`WouldBlock` with bounded backoff until that duration, then reports the lock path.
Other errors fail immediately.

Durability is checked in both directions. A durable/shared index with in-memory
blobs refuses boot. Durable blobs with an ephemeral index also refuse unless
`SOLID_SERVER_ALLOW_INCONSISTENT_DURABILITY=1` (default unset) explicitly allows
that inconsistent combination; this override never permits a durable index with
ephemeral bytes.

SIGTERM or Ctrl-C stops accepting and drains requests. The periodic reconciler is
stopped and awaited before `EmbeddedSparqClient::compact()` runs on the embedded
actor for a directory-backed index. Successful compaction empties its journals;
a failure is logged and returns a non-zero exit while retaining recovery data.
Dropping the final client joins its actor and releases the index lock.
