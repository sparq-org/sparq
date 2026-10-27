# sparq-lws-crypt

Native, unpublished workspace crate for key providers and the `e1` encrypted blob
format (P1-1 of [#7189](https://github.com/sparq-org/sparq/issues/7189)), consumed by the default-off
native `sparq-lws-core/crypt` decorator. Server and environment wiring follow separately.
Its API and dependencies are absent on wasm32. Runtime dependencies are the existing
`aws-lc-rs`, `zeroize` and `async-trait` crates; no cloud SDK is included.

## 🚀 Quickstart

Use a path dependency from a native consumer. Load a raw, exactly 32-byte KEK
with `LocalKeyProvider::from_file(path)`; on Unix the opened regular file must have
no group/other permissions (for example `0600` or `0400`). File errors omit the path.

```rust,ignore
use sparq_lws_crypt::{Dek, KeyContext, KeyProvider, LocalKeyProvider, open, seal};

# async fn example() -> sparq_lws_crypt::Result<()> {
let provider = LocalKeyProvider::from_file("/run/secrets/blob-kek")?;
let context = KeyContext { pod: "https://pods.example/alice/".into(), scheme: "b1".into() };
let (secret, wrapped) = provider.generate_data_key(&context).await?;
let stored_key = wrapped.to_bytes()?;
// The caller allocates a unique random id per DEK and persists it with the wrapped key.
let dek = Dek::new(secret, [0x71; 16], 0)?; // illustrative id only
let envelope = seal(&dek, "b1/blob-key", b"body")?;
assert_eq!(open(&dek, "b1/blob-key", &envelope)?, b"body");
# Ok(())
# }
```

`seal` returns `Result<Vec<u8>>`: randomness, length limits and exhausted budgets can fail.
`SecretKey` owns zeroizing storage, has no `Clone`, and exposes bytes only via explicit provider borrows.
Key diagnostics are redacted. Returned plaintext buffers belong to the caller.

## ✨ Features

### e1 byte format

All offsets and lengths below count bytes. AES-256-GCM uses a 16-byte tag.

| Offset | Length | Field |
| --- | --- | --- |
| 0 | 4 | ASCII `SQLB` |
| 4 | 1 | version `01` |
| 5 | 16 | `dek_id` |
| 21 | 12 | fresh random nonce |
| 33 | plaintext length | ciphertext |
| end minus 16 | 16 | authentication tag |

AAD is exactly `"e1" || dek_id || u32BE(blob_key byte length) || blob_key UTF-8`.
No URI or Unicode normalization occurs. Each DEK belongs to exactly one pod and
unwraps only under that pod's `KeyContext`. Including its unique `dek_id` in AAD
therefore binds the envelope to that pod's key identity as well as the blob key.
The caller must preserve this ownership and never reuse DEK material across pods.

`parse_header` reads a complete header without authenticating it. `open` also
requires a tag, checks the expected id and authenticates before returning plaintext.
`is_envelope` reserves every `SQLB` prefix, even truncated or unknown versions:
a matching blob must go through `open`, with errors propagated. Other prefixes can
remain legacy plaintext `b1` blobs. A new version changes the magic/version prefix's
version byte, retaining reserved `SQLB`; old readers refuse unknown versions.

### Context and wrapped-key formats

`KeyContext::to_bytes()` has exactly one canonical encoding:
`SQLC || 01 || u32BE(pod length) || pod || u32BE(scheme length) || scheme`.
Both strings are exact UTF-8. This is the wrapping AAD and the canonical context
representation for provider integrations; length prefixes prevent field ambiguity.

`WrappedKey::to_bytes()` is
`SQLK || 01 || LP(provider_id) || LP(kek_id) || LP(payload)`, where `LP` is
`u32BE(byte length) || bytes`. Identifiers are UTF-8; payload is opaque. Parsing
rejects invalid UTF-8, truncated/oversized fields, unknown versions and trailing bytes.
Local `provider_id` is `local`; its payload is `nonce(12) || encrypted DEK(32) || tag(16)`.
Wrapping uses AES-256-GCM, a fresh random nonce and canonical context AAD.
Local `kek_id` is lowercase hex of the first 16 bytes of
`SHA-256("sparq-lws-crypt/local-kek-id/v1\0" || KEK)` (`\0` is one zero byte).
The provider rejects a different KEK id before any unwrap attempt. A matching id
with an invalid tag/context returns `AuthenticationFailed`.

### Key lifecycle

The hard limit is **2^32 seal attempts per DEK**, using random 96-bit nonces; rotate early.
`Dek::seal_count()` includes failed attempts after reservation. Its atomic counter cannot wrap
past the limit across callers sharing a `Dek`. Import a conservative lifetime count with `Dek::new`;
persist reservations before use or rotate on restart. Independently imported copies/processes
have separate counters. Exhausted keys remain readable. KEK wrapping uses a separate random
nonce space; operators must also rotate a heavily used KEK.

### How P1-2 uses this

The native `sparq-lws-core/crypt` decorator stores wrapped DEKs in reserved index records
outside HTTP and blob GC: `WrappedKey::to_bytes()`, random `dek_id`, pod, scheme (`lws`/`solid`),
creation time, current flag and `reserved_seals`. It durably reserves `2^16` attempts before use;
restart/expiry skips unused attempts, and rotation starts a fresh key at `2^31`.
Per-pod reservations and per-id unwraps share one TTL policy. Back up index, blobs and KEK together.
The native server activates this with `--features crypt`, `SOLID_SERVER_CRYPT_PROVIDER=local` and `SOLID_SERVER_CRYPT_LOCAL_KEK_FILE`; boot validates keys and upgrades blobs to major 2, which provider `none` cannot reopen.

### Threat model

This layer protects blob bytes on lost/stolen volumes and snapshots,
from cloud/backup operators with byte-plane access and no KMS grant, and from storage
admins without the KEK. It does **not** protect against a root admin dumping process
memory or reading the mmap'd index on the running host (P3 scope). **The sparq index
directory holds the RDF graph in plaintext on disk; P1 requires it to sit on a
CMK-encrypted block volume (EBS/Azure Disk/dm-crypt).** Application-level index
encryption is research tracked in [#2567](https://github.com/sparq-org/sparq/issues/2567).
ETags derive from plaintext, so equal bodies get equal ETags across pods.

## 📚 Learn more

[Solid/LWS server skill](../../skills/solid-lws-server/SKILL.md): server integration status.
Run `cargo test -p sparq-lws-crypt` for vectors, tampering, parser properties,
nonce uniqueness and key-file permissions. `tests/vectors/e1.json` holds public test-only keys
and deterministic vectors checked through private nonce hooks and cross-checked with an
independent AES-GCM implementation. Deterministic nonces are absent from the public API.

## License

MIT OR Apache-2.0, like the workspace.
