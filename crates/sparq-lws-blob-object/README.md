# sparq-lws-blob-object

Experimental native object-store blob plane, `publish = false`. The crate is
available through a default-off library adapter; native server wiring is pending.

## 🚀 Quickstart

```rust,no_run
use bytes::Bytes;
use sparq_lws_blob_object::ObjectBlobStore;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let store = ObjectBlobStore::from_url(
    "file:///existing/blob-directory/",
    std::iter::empty::<(&str, &str)>(),
)?;
let key = "b1-0123456789abcdef0123456789abcdef";
store.put(key, Bytes::from_static(b"body")).await?;
assert_eq!(store.get(key).await?, "body");
# Ok(())
# }
```

## ✨ Features

LocalFileSystem is always available on native targets. Features `aws`, `azure`,
and `gcp` enable `s3://`, `az://`, and `gs://` respectively; all are off by default.
`file://` is for tests and development only: its LocalFileSystem configuration
does not fsync file data or directories and is not crash-durable. For durable
local storage in library integrations, use `store::blob_fs::FsBlobStore` with the
native `blob-fs` feature and hold the exclusive data-directory lock.
`ObjectBlobStore::new(Arc<dyn ObjectStore>, prefix)` wraps an existing backend.
`from_url(url, options)` uses the pinned provider's configuration options and
rejects unknown options. Provider credential discovery can consult local files,
environment variables and instance metadata; supply explicit credentials to avoid
ambient discovery. Errors and Debug output omit backend details.

Keys match `($pool$)?b1-[0-9a-f]{32}`. Normal objects live under `<prefix>/b1-…`;
pooled objects under `<prefix>/pool/b1-…`. Writes use atomic create mode. Callers
must mint a fresh key for every write, including after deletion. Delete is
unconditional and idempotent. Listings stream backend pages, preserve modification
times, and skip foreign objects. No conditional-delete generation is advertised.
The server adapter rejects `delete_if_unchanged` with a backend error; the
reconciler calls `delete` for entries with no generation.

## 📚 Learn more

The optional `sparq-lws-core/blob-object` adapter implements the server's
`BlobStore` trait. See [the server usage guide](../../skills/solid-lws-server/SKILL.md)
and its [object-store reference](../../skills/solid-lws-server/references/blob-object.md).

## License

MIT OR Apache-2.0.
