# sparq-lws-blob-fs

Internal filesystem blob backend for the experimental Solid/LWS server.
This crate is `publish = false` and uses the new-crate gate's bench exemption.

## 🚀 Quickstart

```rust,no_run
use bytes::Bytes;
use sparq_lws_blob_fs::FsBlobStore;
use std::path::Path;

# async fn example() -> Result<(), sparq_lws_blob_fs::Error> {
let blobs = FsBlobStore::open(Path::new("blobs")).await?;
let key = "b1-0123456789abcdef0123456789abcdef"; // Mint a fresh key per write.
blobs.put(key, Bytes::from_static(b"body")).await?;
assert_eq!(blobs.get(key).await?, "body");
# Ok(())
# }
```

Run with a Tokio runtime. Open only with no writes in flight: startup removes
everything under `tmp/`. The root must be managed by trusted storage code.

## ✨ Features

- Only `b1-<32 lowercase hex>` and `$pool$b1-<32 lowercase hex>` keys are accepted.
  Callers mint unique-per-write keys and never reuse them after deletion.
- Layout: `objects/<hex[0..2]>/<hex[2..4]>/<key>` or
  `pool/<hex[0..2]>/<key>`, with `$` encoded as `%24` in filenames.
- Writes use `create_new` temporary files, `write_all`, file `sync_all`, rename,
  and parent-directory sync. Creating shards syncs their parent entries too.
  An existing key errors. Clones serialize the existence check and rename;
  independent handles rely on the caller's unique-key contract.
- Deletes sync the parent directory and succeed for missing keys.
- `get` and `stat` return `Error::NotFound` for missing keys; `exists` returns
  false. Invalid keys return `Error::InvalidKey`; other failures use `Error::Io`.
- `list` ignores temporary files, symlinks and unexpected paths. Entries include
  file size, mtime as `Some(last_modified)`, and `generation = None`.
- All filesystem I/O runs through `tokio::task::spawn_blocking`.
  Directory sync errors propagate; the filesystem must support directory fsync.
  An error after rename may leave a complete blob: retry with a fresh key.

## 📚 Learn more

The default-on native `blob-fs` feature of `sparq-lws-core` re-exports this type
as `sparq_lws_core::store::FsBlobStore` and implements its `BlobStore` trait.
The adapter preserves the trait's missing-key `stat` result (`Ok(None)`).
See [the server skill](../../skills/solid-lws-server/SKILL.md).

## License

MIT OR Apache-2.0.
