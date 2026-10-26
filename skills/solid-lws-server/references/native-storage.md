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

## Backup and restore

This procedure uses the durable `SOLID_SERVER_DATA_DIR` contract with the default
native build described above. It applies to both Solid
and LWS protocol modes. It does not back up ephemeral in-memory storage or a
remote SPARQ service.

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

### Restore an offline backup

1. Unpack a trusted archive into a new, **empty** directory. Do not overlay an
   existing store or unrelated files. `mktemp -d` creates an empty destination:

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
