# Native blob encryption integration

The default-off `sparq-lws-core/crypt` library layer uses the `e1` format from
[sparq-lws-crypt](../../../crates/sparq-lws-crypt/README.md). Runtime activation is
the final P1-2 stage of [#7189](https://github.com/sparq-org/sparq/issues/7189),
following the storage-runtime integration in
[#7232](https://github.com/sparq-org/sparq/pull/7232). These settings are reserved
for that integration; the current binary does not read them.

| Setting | Default | Effect at runtime integration |
| --- | --- | --- |
| `SOLID_SERVER_CRYPT_PROVIDER` | `none` | `none` preserves plaintext writes; `local` enables envelopes; `aws-kms` and `azure-kv` report “not built” until P1-3. |
| `SOLID_SERVER_CRYPT_LOCAL_KEK_FILE` | unset | Required by `local`: exactly 32 raw bytes in a regular file with no group/other permissions, such as `0600`. |
| `SOLID_SERVER_CRYPT_DEK_TTL_SECS` | `600` | TTL for all unwrapped keys, including the current write key; expiry unwraps the existing key and skips its unused reservation. |
| `SOLID_SERVER_CRYPT_POD_SCOPE` | unset | Fixed Solid pod identity; otherwise Solid uses the first path segment. LWS uses the storage root. |

The library assembler is `CompositeStore::encrypt_blobs(provider, scheme, options)`.
Assembly starts with an empty body cache, preserving its capacity, so existing
bodies pass through the selected decoder. `CryptOptions::default()` uses the
default TTL and a monotonic clock; callers can
set `ttl` and inject a `CryptClock`. The protocol scheme is the fixed string `lws`
or `solid` and forms part of the wrapping context. Subscription-pool records
use their storage root even with a fixed Solid scope override. Changing a record's context
prevents unwrapping.

Resource reads compare the DEK record's pod with the requested resource's pod
after opening the envelope. A mismatch returns an error without returning bytes.
The body cache stores the pod authenticated for each entry; a lookup from another
pod returns an error and evicts the entry. Inserting a DEK sweeps expired cached
keys; active key flights keep their shared slots.

Reinstate restores a held record only to its original resource. This integrity
rule applies with or without `crypt`: the held directory's metadata stores the
original IRI in its entity-tag field, while each child hold keeps its holder in
its own entity-tag field. An older held directory with no recorded original IRI
cannot be reinstated and returns a clear storage error; its holds can still be
released. Same-resource reinstatement preserves the held bytes and validators.

Wrapped keys live in index records below `urn:x-sparq-lws:data-keys/`, with empty
blob pointers, following the internal hold-record pattern. Each record's metadata
holds a JSON value containing `dek_id` (lowercase hex), `wrapped` (the byte array
from `WrappedKey::to_bytes()`), `pod`, `scheme`, `reserved_seals` (integer),
`created` (Unix seconds), and `current` (boolean). These are internal persistence
fields, not HTTP representations. HTTP projections of the reserved key namespace
return 404 for authenticated and anonymous resource requests. The native library requires an exclusive index
and one shared decorator per index. A retired key stays available for reads.
Rotation retires the old record before publishing a new current record; a stop in
between can leave no current key, and the next writer creates one.

The runtime stage must atomically change `format.json` to
`"blobs":{"major":2,"encryption":"envelope-v1"}` on first encrypted open. This
is a one-way major transition: older binaries accept only blob major 1, preventing
them from serving ciphertext as plaintext. Opening a major-2 directory with
provider `none` must refuse startup. Additive migration leaves older plaintext
bodies readable and encrypts all subsequent body writes.

Back up the KEK independently and keep it available with the index and blobs.
Losing it makes the encrypted data unrecoverable. The index stays plaintext on
disk and requires a CMK-encrypted block volume (EBS, Azure Disk, or dm-crypt).
The protection does not cover a root admin reading process memory or the running
host's mmap'd index. See the server skill for the complete P1 threat model.

The `SQLB` prefix is reserved even in truncated or unsupported envelopes. Matching
bytes always take the envelope path, and authentication/framing errors propagate;
there is no plaintext retry. An unknown DEK id returns `UnknownDataKey`, logged
with the opaque blob key only. A legacy plaintext body that begins with `SQLB`
therefore fails closed; P1-4 re-encryption will report those blobs. A truncated or
unsupported header is a framing error; a valid header naming an absent key returns
`UnknownDataKey`.
