# Native data-directory boot

`SOLID_SERVER_DATA_DIR` (unset by default) selects a data root with the RDF index
at `<data>/index` and resource bytes at `<data>/blobs`. It implies
`PSS_SPARQ_BACKEND=embedded`; explicitly selecting any other backend refuses boot.
Setting it together with a non-empty `SOLID_SERVER_SPARQ_DIR` also refuses boot.
The legacy `SOLID_SERVER_SPARQ_DIR` setting still opens an existing snapshot and
retains the durable-index/in-memory-blob refusal. A directory-backed index opened
through this legacy path also compacts on graceful shutdown.

The default native build opens `FsBlobStore` at `<data>/blobs`. `embedded-sparq` is required; `blob-fs` is required when no object URL is set.
A missing feature refuses boot with a message naming it.
The default-off `blob-object` feature lets `SOLID_SERVER_BLOB_URL` replace only
the byte plane; its URL and provider options are described in
[object storage](blob-object.md). The offline directory-copy backup and restore
procedure covers only the filesystem blob backend. Without a data
directory, the native binary uses in-memory blobs.

The default provider `none` checks `<data>/format.json` before opening either plane:

```json
{"schema":1,"index":{"major":1},"blobs":{"major":1}}
```

An absent marker is published atomically (temporary file, file sync, hard link,
data-directory sync) only if both plane directories are absent or empty. Publication
never replaces a concurrently created marker; that marker is validated instead. Existing
plane content without the marker is refused as a foreign directory. An unknown
schema, incompatible index/blob major, or malformed marker refuses boot. A marker
without an index resumes first boot. An absent `blobs.backend` denotes
`"filesystem"` for compatibility. Object boot writes
`"blobs":{"major":1,"backend":"object","url":"s3://bucket/prefix"}`.
The backend and exact URL must match on every reopen, including a marker-only
first boot. Backend/location changes require migration; credentials and provider
options are not recorded and may rotate without changing the namespace. Do not remove the marker from an existing store.

With `SOLID_SERVER_CRYPT_PROVIDER=local`, a build with `crypt` upgrades blobs to
`{"major":2,"encryption":"envelope-v1"}` under the index lock after validating
existing keys. This is one-way; `none` and older binaries refuse the upgraded
directory. See [blob encryption](blob-encryption.md) for KEK backup and format details.

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

## Backup and restore

This procedure uses the durable `SOLID_SERVER_DATA_DIR` contract with the default
native build and filesystem blob backend described above. It applies to both Solid
and LWS protocol modes. It does not back up ephemeral in-memory storage or a
remote SPARQ service. An index-only snapshot does not include resource bytes.

When `SOLID_SERVER_BLOB_URL` selects object storage, including `file://`, the bytes
live in the separately configured object namespace. Stop the server
and coordinate the index backup with a separate backup of that entire namespace;
preserve object keys, including the pool layout. Restore both to the same stopped
state before restarting. `format.json` must still record the same backend and
exact URL, while provider options and credentials may rotate. A changed backend
or URL requires migration; editing or removing the marker does not migrate bytes.

With encryption enabled, archive the directory exactly as stored: preserve the
major-2 marker (`blobs.major=2`, `encryption=envelope-v1`), all `e1-` bodies
(including `$pool$e1-`), any legacy `b1-` bodies, and the index's wrapped-key
records. Copying ciphertext does not require decryption. Back up the local KEK
separately, retain its restricted permissions, and restore that same KEK before
starting a build with `crypt` and `SOLID_SERVER_CRYPT_PROVIDER=local`. Preserve
the protocol and configured pod scope used to wrap keys. Provider `none` refuses
the restored major-2 directory; a missing or different KEK prevents decryption.
The data-directory archive does not include a KEK stored outside that directory.
The format upgrade preserves an object backend's recorded backend and URL.

### Make an offline backup

1. Send SIGTERM to the server and **wait for the process to exit successfully**.
   Exit status zero means requests have drained, the reconciler has stopped and
   shutdown compaction has completed. Sending the signal alone is insufficient.
   Run `wait` in the shell that owns the server process, or use the service
   manager's stop-and-wait operation and inspect the exit status.
2. With the server stopped, archive the **whole data directory**, including
   `format.json`, `index/` and `blobs/`. Include all blob subdirectories and any
   remaining journal/recovery files. The lock files are advisory and harmless to
   copy: a copied file does not carry the original process's OS lock.

For example, in the shell that launched the server in the background (and saved
its PID in `server_pid`):

```sh
set -eu
kill -TERM "$server_pid"
if ! wait "$server_pid"; then
  echo 'Server did not stop cleanly; do not treat this as a clean backup' >&2
  exit 1
fi
data_dir=/srv/solid/data
archive=/srv/backups/solid-data.tar
tar -cpf "$archive" -C "$data_dir" .
```

Keep the archive outside the data directory, check `tar`'s exit status, and keep
the source stopped until archiving finishes. Back up deployment configuration
and external secrets separately, including any AS and notification signing-key
files configured outside the data directory. The directory archive contains
only files under that directory. Protect both the archive and the key backups
as private data.
Credentials are not stored in `format.json`.

### Restore an offline backup

1. Unpack a trusted archive into a new, **empty** directory. Do not overlay an
   existing store or unrelated files. Use a filesystem that supports the
   documented locking, hard-link and sync requirements. `mktemp -d` creates an
   empty destination:

   ```sh
   set -eu
   archive=/srv/backups/solid-data.tar
   restored_dir=$(mktemp -d /srv/solid/restored.XXXXXXXX)
   tar -xpf "$archive" -C "$restored_dir"
   ```

2. Check extraction succeeded and the root contains `format.json`, `index/`
   and `blobs/`. Restore external configuration and signing keys, use a
   compatible server build, and retain `SOLID_SERVER_BASE_URL`: stored resource
   and grant IRIs are not rewritten to a new origin. Ensure the server's OS
   user can read and write the restored directory.
   Do not combine an index from one backup with blobs from another.
3. Start with `SOLID_SERVER_DATA_DIR` pointing at that directory. For example,
   with the other settings restored in the environment:

   ```sh
   SOLID_SERVER_DATA_DIR="$restored_dir" /usr/local/bin/sparq-lws-core
   ```

Boot refuses populated `index/` or `blobs/` directories without `format.json`
as foreign data, and refuses an unsupported marker schema or index/blob major.
Do not remove or edit the marker to bypass a refusal. These are boot guards;
they cannot stop `tar` overwriting files in a non-empty destination, so always
extract into an empty directory. A copy in its own directory has independent
locks and can boot while the original runs, using a separate listener. Preserve
the public origin when checking it through that listener.

### Consistency limits and volume snapshots

Online or hot backup is unsupported. Copying or snapshotting a running data
directory is unsafe: the index and blob planes can come from different writes.
A backup is only as consistent as the stop that preceded it. A SIGKILL, forced
container termination, failed compaction or non-zero exit is not a clean stop;
retain the original files for recovery rather than labelling their copy clean.

A crash can leave unreferenced blob files until the reconciler runs. The
offline archive preserves them with the rest of the directory; do not omit
blobs from the backup or delete files by guessing which keys are live.

For a container, stop it and wait for clean exit before detaching the data
volume. On Kubernetes, scale the workload to zero, wait for its pod to finish
cleanly and for the volume to detach, then snapshot the detached volume. Ensure
the orchestrator allows shutdown to finish rather than escalating to SIGKILL.
Snapshot the volume holding the **whole** data directory, not just one plane.
Restore its snapshot into a fresh volume before starting the workload.
See the [native container configuration](../../../crates/sparq-lws-core/README.md)
and [Helm persistence settings](../../helm-deploy/SKILL.md#use-persistence-and-private-images)
for the deployment configuration to restore alongside the volume.
