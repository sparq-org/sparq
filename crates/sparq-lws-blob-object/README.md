# sparq-lws-blob-object

Experimental native object-store blob plane, `publish = false`. The crate is
available and wired into the native server behind its default-off `blob-object` feature.

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
native `blob-fs` feature and hold the exclusive data-directory lock. Native server
boot selects this backend with `SOLID_SERVER_BLOB_URL` plus
`SOLID_SERVER_DATA_DIR` (the index remains local). Unset URL retains filesystem
blobs with a data directory and in-memory blobs otherwise. A set URL without
`blob-object` fails startup. `SOLID_SERVER_BLOB_OPTIONS` is an optional JSON map
of provider keys to string values (default empty); unknown keys fail startup.
The URL must have no user information, query or fragment. Credentials belong in
options or provider-default discovery. Configuration values are never logged.
`format.json` records the backend and exact URL, excluding options; reopening
with a different backend or URL requires migration. Legacy markers denote
filesystem blobs. Cloud provider features must also be enabled.
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
