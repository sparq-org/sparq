# sparq-lws-blob-fs

Standalone, std-only filesystem storage for immutable opaque LWS blob keys.
This workspace crate is not published and does not depend on `sparq-lws-core`.

## 🚀 Quickstart

The caller must acquire the exclusive data-directory lock **before** opening and
hold it until all store handles and pending operations are dropped. Use a directory
controlled by the caller and a filesystem supporting hard links and directory sync.
Opening creates the layout and sweeps abandoned writes in `tmp/`; do not open while
another handle is writing.

```rust,ignore
use sparq_lws_blob_fs::FsBlobStore;

// With the data-directory lock held:
let store = FsBlobStore::open(blob_root)?;
let key = "b1-0123456789abcdef0123456789abcdef"; // use a fresh random key per write
store.put(key, b"body")?;
assert_eq!(store.get(key)?, b"body");
store.delete(key)?;
```

## ✨ Features

- Synchronous `open`, `put`, `get`, `exists`, `delete`, `stat` and `list`, using
  `std::io::Result`. Only `($pool$)?b1-[0-9a-f]{32}` keys are accepted.
- Ordinary layout: `objects/<first hex pair>/<second hex pair>/<key>`; pooled
  layout: `pool/<first hex pair>/<key>`. Hex pairs come from the random key part.
- An exclusive temporary file is written and synced before hard-link publication
  without overwrite. Existing keys fail even when their bytes match. Fan-out
  directories and their parent entries are synced on first use per open store,
  including pre-existing directories. A bitmap shared across clones records only
  successful initialization; a per-store mutex serializes first use. Reopening
  clears the bitmap. The final parent is synced after every publication.
- Idempotent deletion syncs the containing directory after unlink. Keys must never
  be rewritten or reused, including after deletion. A failure after publication
  can leave an orphan; retry with a fresh key.
- Listing returns `Listing { blobs, foreign }` and excludes `tmp/`. `blobs` holds
  keys with filesystem modification times. Invalid names, misplaced valid keys
  and non-files are skipped and reported in `foreign`; unexpected fan-out
  directories are reported without traversing them. Filesystem I/O errors still
  propagate. The async adapter returns the blobs as `Some(mtime)` with no
  generation witness and emits a warning for foreign entries once per listing.

## 📚 Learn more

- [`sparq-lws-core`](../sparq-lws-core) supplies the async `BlobStore` adapter via
  its default-on native `blob-fs` feature. Filesystem work uses the blocking pool.
- [Solid/LWS usage skill](../../skills/solid-lws-server/SKILL.md).

## License

MIT — see the workspace license.
