# Object-store blob plane

Status: **crate available; server wiring pending**. The default-off native
`blob-object` feature exposes a library adapter; enabling it does not change
server boot, configuration or the native storage marker.

The crate pins `object_store` and enables only LocalFileSystem by default.
Enable crate feature `aws`, `azure`, or `gcp` for `s3://`, `az://`, or `gs://`.
`file://` is available without a provider feature. The object crate has no server
dependency, and the default server dependency graph excludes `object_store`.

`file://` is for tests and development only. Its LocalFileSystem configuration
does not fsync file data or directories, so it is not crash-durable. For durable
local storage in library integrations, use `store::blob_fs::FsBlobStore` with
the native `blob-fs` feature and hold the exclusive data-directory lock.

```rust,no_run
use sparq_lws_core::store::blob_object::{ObjectBlobAdapter, ObjectBlobStore};
use sparq_lws_core::store::{CompositeStore, InMemorySparqClient};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let blobs = ObjectBlobStore::from_url(
    "file:///existing/blob-directory/",
    std::iter::empty::<(&str, &str)>(),
)?;
let store = CompositeStore::new(InMemorySparqClient::new(), ObjectBlobAdapter(blobs));
# let _ = store;
# Ok(())
# }
```

`ObjectBlobStore::new(Arc<dyn ObjectStore>, prefix)` accepts an existing provider.
`from_url(url, options)` accepts the pinned provider's key/value configuration,
including credentials passed separately from the URL. Unknown options are rejected.
Provider credential discovery can consult local files, environment variables and
instance metadata; supply explicit credentials to avoid ambient discovery. URL user information, query strings and fragments are rejected.
Use a dedicated local directory or a provisioned bucket/container and an isolated
prefix. Errors contain fixed descriptions, without backend diagnostics or URLs.

Keys use `($pool$)?b1-[0-9a-f]{32}`. Normal objects are stored as
`<prefix>/b1-…`; pooled keys as `<prefix>/pool/b1-…`. Every write uses create mode
and must use a fresh key. Never reuse a key even after deletion. HEAD supplies
existence and metadata; a missing key is absent, while other backend errors
propagate. Delete is idempotent and unconditional.

`BlobStore::list_stream` exposes modification times and `generation = None`.
The reconciler consumes bounded batches, rechecks references and age, then calls
`BlobStore::delete` for entries with no generation. `delete_if_unchanged` returns
`BlobError::Backend("conditional delete unsupported".into())`: calling it on this
backend is a caller bug. The compatibility `list` snapshot refuses more
than 4096 entries; use the stream for an unbounded namespace. Foreign object names
and nested directories are excluded from listings. If a later page fails, the current
batch aborts immediately; earlier batches may already have completed.

For backend authors, enable `sparq-lws-core/test-support` and call
`store::blob_contract::run(&backend)` on an empty isolated backend. The in-memory fixture and object
backends run the same validated, create-only contract, including concurrent creates of
one key. `InMemoryBlobStore::with_immutable_keys()` supplies this mode locally;
its legacy constructors retain arbitrary-key replacement for existing fixtures.
Filesystem backend integration with this suite is deferred to its follow-up.

Run the LocalFileSystem contract and router tests with:

```sh
cargo test -p sparq-lws-core --features blob-object,test-support --test blob_object
```

The opt-in MinIO test requires the dependency's `aws` feature and an empty,
dedicated bucket/prefix. Test variables have no defaults: `LWS_BLOB_TEST_URL`
is its `s3://bucket/unique-prefix` URL; `LWS_BLOB_TEST_ENDPOINT` is the HTTP
MinIO endpoint; `LWS_BLOB_TEST_ACCESS_KEY` and `LWS_BLOB_TEST_SECRET_KEY` supply
its credentials. The test deletes its own fixture objects. These variables do
not configure the server binary.

```sh
cargo test -p sparq-lws-core \
  --features blob-object,test-support,sparq-lws-blob-object/aws \
  --test blob_object minio_contract_router_and_orphan_sweep -- --ignored
```
