# Object-store blob plane

Status: **wired**. The native binary selects `ObjectBlobStore::from_url` when
`SOLID_SERVER_BLOB_URL` is set, behind the default-off `blob-object` feature.
Without that feature a set URL is a startup error. With the URL unset, storage
selection is unchanged. `SOLID_SERVER_DATA_DIR` is required: the index stays at
`<data>/index`; object storage replaces `<data>/blobs`.

`SOLID_SERVER_BLOB_URL` defaults to unset and accepts `file://`, `s3://`, `az://`,
and `gs://`. It must contain no user information, query string or fragment.
`SOLID_SERVER_BLOB_OPTIONS` defaults to unset (an empty map); when supplied it
must be a JSON object mapping provider option names to string values, with a URL
also set. Unknown option names fail startup. For example, S3-compatible storage
can use `{"aws_endpoint":"https://objects.example","aws_region":"us-east-1"}`;
credentials may be supplied as `aws_access_key_id`, `aws_secret_access_key` and
`aws_session_token`, or through provider-default discovery. No other server
settings are inferred as provider options. Neither URLs nor option values are
logged; options are never written to `format.json`.

The marker records `blobs.backend = "object"` and the exact configured URL in
`blobs.url`. A different backend or URL refuses reopening before either plane is
opened. Legacy markers with no backend denote filesystem blobs. Credential and
endpoint options are not part of this identity and may rotate; keep them pointed
at the same namespace. Migrations require moving both planes coherently; do not
edit or remove the marker to bypass a mismatch.

The crate pins `object_store` and enables only LocalFileSystem by default.
Enable crate feature `aws`, `azure`, or `gcp` for `s3://`, `az://`, or `gs://`.
`file://` is available without a provider feature. The object crate has no server
dependency, and the default server dependency graph excludes `object_store`.

`file://` is for tests and development only. Its LocalFileSystem configuration
does not fsync file data or directories, so it is not crash-durable. For durable
local storage in library integrations, use `store::blob_fs::FsBlobStore` with
the native `blob-fs` feature and hold the exclusive data-directory lock.
Without an object URL, native server boot uses filesystem blobs with
`SOLID_SERVER_DATA_DIR` and in-memory blobs otherwise; see
[native storage](native-storage.md).

For a local development boot, create the object directory first:

```sh
mkdir -p /tmp/lws-object-bytes
SOLID_SERVER_DATA_DIR=/tmp/lws-index \
SOLID_SERVER_BLOB_URL=file:///tmp/lws-object-bytes/ \
cargo run -p sparq-lws-core --features blob-object
```

For a cloud provider, enable its feature too, for example
`--features blob-object,sparq-lws-blob-object/aws` (or `/azure`, `/gcp`).

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
and nested directories are excluded from listings. If a later page fails, a sweep
returns an error immediately and earlier batches may already have completed.

For backend authors, enable `sparq-lws-core/test-support` and call
`store::blob_contract::run(&backend)` on an empty isolated backend. Every backend
runs the same validated, create-only contract, including concurrent creates of
one key. `InMemoryBlobStore::with_immutable_keys()` supplies this mode locally;
its legacy constructors retain arbitrary-key replacement for existing fixtures.
The `blob-fs` tests also run this suite over an isolated `FsBlobStore` directory.

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
