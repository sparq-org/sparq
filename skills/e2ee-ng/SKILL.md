---
name: e2ee-ng
description: Build the low-level primitives of the NextGraph-style E2EE-queryable profile for sparq with the opt-in sparq-e2ee-ng crate — deterministic capability encoding with strict read/publish/admin separation, recipient-wrapped secrets (X25519 sealed-box), randomized padded block/commit envelopes (XChaCha20-Poly1305) under a domain-separated HKDF-SHA-256 key schedule, an object chunker that splits a large object into Merkle-linked multi-block sets, Ed25519 author/publisher/admin signatures, signed epoch transitions (the revocation mechanism), a fail-closed deterministic-CBOR codec with explicit parser limits, and golden test vectors. Use when constructing/parsing capabilities, sealing/opening encrypted blocks or chunked multi-block objects, minting commit ids, or authoring epoch transitions for the E2EE-NG profile. RESEARCH-GRADE, NOT externally audited — every confidentiality/integrity/authorization/revocation property is designed/intended, not proven; production use is gated by sq-qhy4. This crate is the capability/envelope/epoch layer ONLY — sync, broker protocol, CRDT, and materialization are NOT implemented here, and it does NOT make SPARQL run over ciphertext (querying stays local over decrypted, materialized state).
---

# sparq-e2ee-ng — E2EE-NG profile primitives (capability / envelope / epoch)

`sparq-e2ee-ng` implements the low-level cryptographic **primitives** of the
NextGraph-style **E2EE-queryable profile** designed in
[`research/e2ee-nextgraph-variant-gpt56-2026-07.md`](../../research/e2ee-nextgraph-variant-gpt56-2026-07.md)
(program `sq-tag1q`). In that profile an RDF dataset is a local-first repository:
encrypted content-addressed blocks carry a causal DAG of encrypted commits; an
always-on broker stores and routes *opaque* blocks but never sees a SPARQL
request. This crate is the **capability / envelope / epoch** layer that everything
else is built on.

<!-- privacy-claims-allow: The whole "Maturity" block is a NEGATIVE/scoped honesty caveat
     (explicitly denies any proven soundness/privacy claim, states designed/intended only,
     and names the sq-qhy4 gate) — not an achieved-property claim. -->
> **Maturity (read first).** RESEARCH-GRADE and **externally UNAUDITED**. Every
> confidentiality, integrity, authorization, revocation, and convergence property
> is **designed/intended, NOT proven** — the construction has had no external
> cryptographic review. The v0 suite name is a placeholder; production suite
> selection + the soundness review are gated by **`sq-qhy4`**. This crate does
> **not** claim cryptographic soundness or zero-knowledge, and it does **not**
> turn SPARQL into ciphertext-side evaluation: the profile keeps querying *local*
> over decrypted, materialized state. It is the **primitives** layer only — the
> sync, broker-protocol, CRDT, and materialization layers (design §6, §8.4–8.5)
> are **not** in this crate. Encryption + key material live **only** behind this
> crate: `sparq-core` / `sparq-engine` / `sparq-substrate` never link a cipher.

## Quickstart

The crate is in-workspace, `publish = false`, so depend by path:

```toml
[dependencies]
sparq-e2ee-ng = { path = "crates/sparq-e2ee-ng" }
```

Seal a block and open it (the round-trip fails closed on any wrong scope):

```rust
use sparq_e2ee_ng::envelope::{seal_block_random, open_block, BlockContext, ObjectKind};
use sparq_e2ee_ng::ids::{RepoId, BranchId, Epoch, ObjectId, Secret32};

let k_read = Secret32::random();                 // branch read secret K_read
let ctx = BlockContext {                         // repo/branch/epoch/kind are AD-bound,
    repo: RepoId::random(),                       // NOT serialized into the envelope
    branch: BranchId::random(),
    epoch: Epoch(0),
    kind: ObjectKind::Operation,
};
let object = ObjectId::random();
let env = seal_block_random(&k_read, &ctx, &object, 0, 1, b"CRDT op bytes")?;
let pt = open_block(&k_read, &ctx, &env)?;        // wrong repo/branch/epoch/chunk => Err
assert_eq!(pt, b"CRDT op bytes");
# Ok::<(), sparq_e2ee_ng::Error>(())
```

Mint the three separated capabilities and sign the public grant with an admin key:

```rust
use sparq_e2ee_ng::capability::{base_grant, Capability, Validity};
use sparq_e2ee_ng::ids::{RepoId, BranchId, Epoch, TopicId, Secret32};
use sparq_e2ee_ng::sign::SecretSigningKey;

let admin = SecretSigningKey::generate();
let publisher = SecretSigningKey::generate();
let g = base_grant(RepoId::random(), BranchId::random(), Epoch(0), TopicId::random(),
                   Validity { not_before: 0, not_after: u64::MAX }, vec!["wss://b".into()]);

let read  = Capability::new_read(g.clone(), Secret32::random())?;          // {read}
let write = Capability::new_write(g.clone(), Secret32::random(), &publisher)?; // {read,publish}
let adm   = Capability::new_admin(g, &admin)?;                             // {admin}, distinct sk_admin
read.validate()?; write.validate()?; adm.validate()?;                     // separation invariants
# Ok::<(), sparq_e2ee_ng::Error>(())
```

## Features / public API surface

- **`ids`** — 32-byte public identifiers (`RepoId`/`BranchId`/`TopicId`/`BlockId`/
  `ObjectId`/`CommitId`/`CapId`/`AuthorKeyId`), `Epoch`, and the zeroizing
  `Secret32`. Every "random" id is drawn from a CSPRNG and is **not** derived from
  plaintext/ciphertext (no convergent-encryption leakage).
- **`cbor`** — a minimal **deterministic** CBOR codec (RFC 8949 core deterministic:
  shortest ints, definite lengths, strictly-ascending map keys) with a fail-closed
  `Reader` enforcing explicit `Limits` (max string/array/map/depth), rejecting
  non-canonical encodings + trailing bytes, and skipping negative-int extension keys
  while rejecting unknown mandatory ones.
- **`suite`** — algorithm agility with one bound v0 suite (`SUITE_V0`) and
  `check_suite`; `aead_seal`/`aead_open` (XChaCha20-Poly1305).
- **`keyschedule`** — domain-separated HKDF-SHA-256: `object_key` (binds K_read to
  repo/branch/epoch/object), `block_key`, `wrap_key`.
- **`sign`** — `SecretSigningKey` / `PublicVerifyingKey` (Ed25519) for the three
  separated authorities.
- **`wrap`** — recipient-wrapped secrets: `wrap`/`unwrap` + `WrappedSecret`
  (X25519 ECDH → wrap key → AEAD; fresh ephemeral key + nonce per wrap). This is
  ephemeral-*static* ECDH: **not forward-secret** — compromise of the recipient's
  long-term private key exposes previously recorded wraps.
- **`capability`** — `PublicGrant` (deterministic encode/decode, `cap_id`,
  admin `sign`/`verify`, and a **branch set**: `ScopedBranch` +
  `branch_scope`/`set_branch_scope`/`covers_branch`), `Capability`
  (`new_read`/`new_write`/`new_admin`, `validate`,
  `encode_secret`/`decode_secret`), `wrap_capability`/`unwrap_capability` (the
  typed §4.2 secret-transfer path: encode-secret + recipient-wrap under the fixed
  `CAPABILITY_WRAP_AAD`, plaintext never handed to the caller), and `delegate`
  (narrow-only).
- **`envelope`** — `BlockEnvelope` (`seal_block`/`seal_block_random`/`open_block`,
  padded to `PAD_CLASSES`, `digest`/`commit_id`) and `Commit` (author `sign`/`verify`,
  `encode`/`decode`).
- **`object`** — the object chunker for objects too large for one block:
  `seal_object`/`seal_object_with` split a plaintext into randomized leaf blocks
  and link them with internal **Merkle node** blocks whose child links
  (`{block id, chunk index, SHA-256(child envelope)}`) are authenticated *inside*
  the encrypted parent; `open_object` / `SealedObject::open` verify the whole tree
  and reassemble. `ObjectLayout` shapes it (`chunk_size`/`arity`), `ObjectLimits`
  bounds an untrusted block set, and `SealedObject::merkle_root`/`commit_id` is
  the root block's digest.
- **`epoch`** — `EpochTransition` (`sign`/`verify`, monotonic-epoch check,
  `HistoryPolicy::{ForwardOnly, HistoryRekeyed}`) — the revocation mechanism.

## Gotchas

- **Secrets never go on the wire in the clear.** `K_read` and private keys live in
  the secret-bearing `Capability::encode_secret` output (a bearer secret) — wrap it
  with `wrap` before it leaves protected local storage. Prefer
  `wrap_capability`/`unwrap_capability`: same two steps, but the AAD is the fixed
  `CAPABILITY_WRAP_AAD` (so a capability wrapping cannot be confused with a DEK or
  bare-`K_read` wrapping made under a caller-chosen label), the capability is
  `validate`d before it is wrapped, and the plaintext is zeroized rather than
  handed to you. `PublicGrant::encode` (what
  a broker sees) carries **no** secret field and `PublicGrant::decode` **rejects** a
  secret field if one is present.
- **The opaque header is AD-bound, not serialized.** `open_block` needs the same
  `BlockContext` (repo/branch/epoch/kind) used to seal; those fields are
  authenticated but never appear in `BlockEnvelope` (design §5). A wrong context
  fails closed as `Error::Decrypt`.
- **A chunked object's block set is exact, and chunk 0 is the root.** `open_object`
  wants *exactly* the object's blocks (root included) — an extra, missing, or
  duplicated block is rejected, not ignored, so filter a larger store by
  `object_id` first. Only the Merkle root may occupy `ROOT_CHUNK_INDEX` (0), and a
  child that disagrees with the digest/position its encrypted parent recorded fails
  with `Error::Integrity` *before* its plaintext is used. Chunking is deliberately
  **not** content-defined: boundaries and block ids never derive from plaintext, so
  cross-object deduplication is intentionally sacrificed (design §2).
- **Revocation is not retroactive.** An `EpochTransition` mints a fresh topic/key
  set and re-keys *future* writes; it cannot erase plaintext or keys a former member
  already obtained. `HistoryPolicy` is an honest disclosure of that, not a secrecy
  guarantee.
- **Delegation only narrows.** `delegate` rejects any child grant that widens the
  branch set, authority set, validity window, or epoch bound relative to its parent.
  The epoch bound is the child's own signed `PublicGrant::max_epoch` ceiling — the
  child keeps the parent's exact `epoch`, so a narrowed grant never claims an epoch
  that disagrees with the topics it carries.
- **A branch set is authorization, not key distribution.** A grant is scoped to one
  or more branches (`Delegation::branches` selects a subset out of the parent's set;
  `None` inherits it), and each branch in scope carries its own epoch-specific topic,
  so delegation never invents routing material and re-delegation can never recover a
  branch an ancestor dropped. But a `Capability` still bears exactly **one** branch
  read secret (design §8.2 field 10), so a multi-branch grant needs the other
  branches' key material delivered separately — per §4.2, "cryptographic key
  possession remains necessary; the signed grant alone is not a decryption key". The
  set is canonical (ascending, primary = lowest, topics distinct) and is admin-signed;
  a single-branch grant omits the field and encodes exactly as it always did.
- **Determinism is load-bearing.** `cap_id` and `CommitId` are hashes of canonical
  bytes; the golden vectors in `tests/vectors.rs` pin the exact wire format. A
  change to those bytes is a wire-format break, not a refactor.

## Learn more

- Design record: [`research/e2ee-nextgraph-variant-gpt56-2026-07.md`](../../research/e2ee-nextgraph-variant-gpt56-2026-07.md)
- Complementary simpler baseline (Profile CS) + threat-model handoff: design §3, §9.
- The vetted-crypto, standards-interop sibling for *signing* RDF (not confidentiality):
  [`verifiable-credentials`](../verifiable-credentials/SKILL.md).
