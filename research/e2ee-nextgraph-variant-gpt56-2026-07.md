<!-- [GPT-5.6] Design/spec record for the NextGraph-style E2EE-queryable variant. -->

# E2EE-queryable RDF, variant NG: encrypted local-first repositories

**Status:** design and embedded specification draft; no implementation. **Date:** 2026-07-11.
**Program:** `sq-tag1q`. This is a complementary design to the client-only survey in
[`e2ee-queryable-options.md`](./e2ee-queryable-options.md), not a claim about shipped
sparq or NextGraph compatibility.

> **Reconciled 2026-07-26 (sq-1vbdy).** The "NG profile" below is **not a rival profile**:
> it is the **v0 wire binding** of the canonical **Profile BR** framed in
> [`e2ee-queryable-nextgraph-variant-2026-07.md`](./e2ee-queryable-nextgraph-variant-2026-07.md).
> §4, §5, §6 and §8 are canonical as that binding; §8.3's `"or-set-quads-v0"` CRDT token is
> **superseded** by the frozen `sparq-crdt-delta/1` format of
> [`sparql-crdt-gpt56-2026-07.md`](./sparql-crdt-gpt56-2026-07.md) / `site/specs/sparql-crdt.typ`
> — the E2EE layer defines no CRDT of its own. See the contradiction ledger and the
> one-CRDT binding contract in
> [`e2ee-program-reconciliation-2026-07.md`](./e2ee-program-reconciliation-2026-07.md).
>
> **Honesty and audit boundary.** Every confidentiality, integrity, authorization,
> revocation, and convergence property below is **designed/intended**, not proven.
> The construction has not received an external cryptographic review. Production use
> remains gated by **sq-qhy4**. In particular, this record does not call an encrypted
> broker “zero knowledge,” does not claim cryptographic soundness, and does not turn
> local SPARQL into ciphertext-side query evaluation. The rigorous comparison belongs
> in [`e2ee-queryable-nextgraph-variant-2026-07.md`](./e2ee-queryable-nextgraph-variant-2026-07.md),
> owned by the independent privacy reviewer; this record only identifies the protocol's
> disclosure boundary. (That record is the privacy/threat-model authority this section
> defers to; the placeholder filename originally written here was never authored —
> corrected 2026-07-26, sq-1vbdy.)

## 1. Decision

Define an optional **NG profile** in which an RDF dataset is a local-first repository:
branches contain a causally ordered DAG of encrypted commits; commits carry RDF CRDT
operations and are chunked into encrypted, content-addressed blocks; an always-on broker
stores, synchronizes, and routes those opaque blocks by overlay and topic. A holder of a
branch read capability synchronizes blocks, decrypts and validates them locally,
materializes an RDF snapshot into a local sparq store, and runs ordinary SPARQL there.

The decisive property is placement: the broker participates in synchronization but
never receives a SPARQL request. **Querying remains local.** The profile is therefore a
repository-and-sync protocol, not searchable encryption and not server-side SPARQL over
ciphertext.

This follows NextGraph's architectural shape, not its wire compatibility. NextGraph
describes repositories containing branches and commit DAGs, encrypted blocks, one
pub/sub topic per branch, brokers, and edge verifiers that decrypt and materialize
state ([architecture](https://docs.nextgraph.org/en/architecture/),
[sync protocol](https://docs.nextgraph.org/en/protocol/),
[verifier](https://docs.nextgraph.org/en/verifier/)). Its published specifications are
explicitly work in progress ([specification status](https://docs.nextgraph.org/en/specs/));
the normative draft in §8 is consequently sparq's own versioned profile.

## 2. What is borrowed, and what is not

NextGraph is the design precedent for five choices:

1. A local-first repository has branches whose commits form a causal DAG; CRDT updates
   are replayed to a materialized state ([CRDT framework](https://docs.nextgraph.org/en/framework/crdts/),
   [repository format](https://docs.nextgraph.org/en/specs/format-repo/)).
2. Immutable encrypted blocks are the storage/transport unit, and objects are chunked
   into Merkle trees ([sync protocol](https://docs.nextgraph.org/en/protocol/),
   [repository format](https://docs.nextgraph.org/en/specs/format-repo/)).
3. Branch topics carry encrypted commit events through a two-tier broker/pub-sub network;
   trusted edge verifiers decrypt and apply them ([network](https://docs.nextgraph.org/en/network/)).
4. Possession of key material acts as a capability: a read capability unlocks branch
   objects, while publishing requires distinct authority ([encryption](https://docs.nextgraph.org/en/encryption/)).
5. Inner/outer overlays scope collaboration and sharing ([network](https://docs.nextgraph.org/en/network/)).

The reference implementation is
[`nextgraph-rs`](https://github.com/nextgraph-org/nextgraph-rs) (canonical upstream:
[`git.nextgraph.org`](https://git.nextgraph.org/NextGraph/nextgraph-rs)). It exposes
concrete client operations including topic subscription/sync, block existence/get/put,
commit retrieval, repository pinning, and event publication; the documentation lists
the same operations ([client protocol](https://docs.nextgraph.org/en/specs/protocol-client/)).

This profile deliberately does **not** copy NextGraph's BARE encoding, DID method,
transport, exact key derivation, convergent-encryption construction, signing/quorum
model, or wire enums. NextGraph documents keyed convergent block encryption and visible
Merkle child identifiers ([repository format](https://docs.nextgraph.org/en/specs/format-repo/));
copying those details without a profile-specific analysis could introduce confirmation,
equality, and graph-shape leakage. NG v0 instead uses randomized AEAD and stable random
block identifiers. Deduplication is intentionally sacrificed pending `sq-qhy4` review.

## 3. Complement to PR #1948, not a rename

The existing Profile CS in PR #1948 stores an AEAD-encrypted RDF resource or bundle on
an ordinary authenticated blob server. A client fetches, decrypts, parses, indexes, and
runs full SPARQL. It proposes generic per-resource DEKs wrapped to recipients; WAC/ACP
may authorize ciphertext CRUD; the server is otherwise unchanged. That remains the
smallest interoperable baseline and is preferable when collaboration/sync history is
unnecessary.

NG diverges in concrete ways:

| Concern | PR #1948 Profile CS | This NG variant |
|---|---|---|
| Stored unit | resource/bundle ciphertext | encrypted commit/block DAG |
| State | encrypted snapshot | local CRDT materialization at a causal frontier |
| Sharing authority | recipient-wrapped DEK, naturally WebID-associated | key-bearing repo/branch read, write, and admin capabilities |
| Server | unchanged blob holder | protocol broker: pin, have/want sync, pub/sub, head announcements |
| Collaboration | manifests/deltas/CRDT deferred | branch/commit/CRDT semantics are the profile |
| Isolation | server resource/container boundary | overlay plus branch topic |
| Freshness | fetched resource versions | declared materialized frontier/heads |
| Revocation | rewrap/re-encrypt policy | epoch change, new topic and future-write/read keys |
| Leakage | blob names/access/size/timing | additional overlay/topic, subscription, DAG/sync and publisher metadata |

Both profiles preserve the same honest limit: no general SPARQL evaluation occurs over
ciphertext at the server. NG improves offline collaboration, incremental replication,
and causal freshness; it expands metadata visible to the broker and adds substantial
key, merge, and protocol complexity.

## 4. Repository and capability model

### 4.1 Repository objects

- `RepoId`: random 256-bit public identifier for one repository.
- `BranchId`: random 256-bit stable identifier; a repository has a root branch and one
  or more data branches. Identifiers are not RDF IRIs unless explicitly mapped locally.
- `Epoch`: monotonically increasing unsigned integer for membership/key rotation.
- `TopicId`: epoch-specific random public routing identifier for one branch.
- `CommitId`: hash of the canonical encrypted commit-envelope bytes.
- `BlockId`: random 256-bit identifier authenticated inside its encrypted parent; it is
  not derived from plaintext or ciphertext in v0.
- `Frontier`: a set of causally maximal commit IDs.

Each commit names parents, branch, epoch, author key ID, logical clock, operation object,
and optional snapshot object. Its plaintext RDF payload is a canonical sequence of
CRDT operations. The initial profile uses an observed-remove quad set: add operations
carry unique dots; removes identify observed dots; merge is set union; deterministic
materialization retains live dots. A later CRDT kind requires a new media/profile value.

### 4.2 Keys and capabilities

The following is a **sparq variant**, not a claim that NextGraph flattens its authority
this way:

- A branch **read capability** contains `RepoId`, `BranchId`, `Epoch`, `TopicId`, a
  branch read secret `K_read`, broker locators, and constraints. It permits intended
  decryption of blocks for that branch/epoch and subscription to its topic.
- A branch **write capability** contains the read capability plus a private publishing
  key `sk_publish`. The broker registers the corresponding `pk_publish` for the topic;
  commits additionally carry an author signature. Possession is intended to permit
  publication, not administration.
- A repository **admin capability** contains a distinct `sk_admin` plus repository
  metadata authority. It permits intended creation/rotation of branches, publisher-set
  changes, and signed epoch-transition commits. It does not reuse `K_read` or
  `sk_publish`.

Capabilities are bearer secrets. They MUST be transferred inside a separately protected
channel or recipient wrapping envelope; they MUST NOT appear in RDF, broker requests,
logs, URLs, or topic messages. A capability has a canonical CBOR representation and a
`cap_id = SHA-256(canonical-public-fields || random_cap_nonce)` for local lookup. The
secret-bearing serialization is never used as an identifier.

Delegation creates a new capability wrapper with constraints no broader than its parent:
branch set, operations (`read`, `publish`, `admin`), `not_before`, `not_after`, and an
optional maximum epoch. The admin signature authenticates the public grant. Cryptographic
key possession remains necessary; the signed grant alone is not a decryption key.

Revocation cannot erase plaintext or keys already obtained. An epoch transition creates
a fresh topic, `K_read`, and publisher key set, distributes new capabilities only to
remaining members, and encrypts all future commits under the new epoch. Historical
access is explicit: `forward-only` leaves old epochs readable to former members;
`history-rekeyed` requires a new encrypted snapshot and garbage-collection policy. The
broker can stop accepting the removed publisher after it observes an admin-authorized
transition, but that is an intended online enforcement layer, not retrospective secrecy.

## 5. Broker boundary: exact disclosure ledger

The broker is not trusted with RDF or query confidentiality, but it is trusted for
availability only to the degree configured. It can omit, delay, replay, reorder, or
equivocate about blocks. Clients therefore validate envelopes, signatures, parent
closure, epochs, and CRDT rules locally.

### The broker sees

- transport facts: client IP/network endpoint, connection and account identity or
  pseudonymous peer public key, broker authentication outcome, session duration;
- overlay identifiers and membership/contact patterns; topic IDs, subscriptions,
  publisher registrations, and which peer publishes to or fetches from which topic;
- message types, timing, ordering, sequence/cursor values, retry behavior, and sizes;
- ciphertext envelope bytes, opaque block IDs, requested/present/missing block-ID sets,
  pin/unpin state, retention state, and storage volume;
- commit/event IDs and declared topic/epoch; **if clear routing headers are enabled**, the
  parent commit IDs, block counts, and root block IDs, revealing a commit DAG. v0's
  default is `opaque-header`, where parents are inside ciphertext and only event order
  and opaque IDs remain visible;
- registered publisher public keys and signatures needed for broker admission; and
- broker-to-broker routing topology in an overlay.

These observations permit traffic correlation and inference about activity, membership,
co-access, update rate, approximate object size, and (with clear headers) branch history
shape. Padding/batching can reduce but does not eliminate this metadata.

### Hidden from a conforming broker by design

- RDF terms, quads, graph names, CRDT operations, plaintext commits/snapshots, MIME and
  semantic document types;
- `RepoId` and stable `BranchId` when only epoch-specific opaque topics are sent;
- `K_read`, private publisher/admin keys, capability secret fields, block AEAD keys and
  recipient wrapping keys;
- SPARQL query text/algebra, local indexes/plans, intermediate bindings, answers, and
  which locally materialized graphs contributed to an answer;
- the plaintext materialized repository and local sparq store; and
- encrypted parent/child relationships in `opaque-header` mode.

The broker necessarily learns that a peer fetched some blocks. It does not learn whether
those blocks were fetched for a SPARQL query, background replication, or UI rendering.
NG MUST NOT be described as hiding access patterns, membership, volume, or timing.

## 6. Sync, materialize, query

1. **Open.** The local verifier imports a read/write/admin capability from protected
   client storage, connects to a listed broker, authenticates transport, and sends
   `OpenRepo` using only overlay/topic routing fields.
2. **Subscribe.** It sends `TopicSub(topic_id, epoch, cursor?)`. For an initial clone it
   sends `TopicSyncReq` with an empty frontier; for an incremental clone it sends known
   heads and a compact known-ID summary. Bloom filters are permitted only as a bandwidth
   hint; false positives are repaired by parent-closure fetching.
3. **Have/want.** The broker returns advertised heads and missing opaque IDs. The client
   uses `BlocksExist`, `BlocksGet`, and, for publication, `BlocksPut`. Every received
   block is size-checked, AEAD-opened locally, and linked to the expected object.
4. **Validate/replay.** The verifier decrypts commit envelopes, checks branch/epoch,
   author/admin signatures, causal parents and CRDT validity, topologically replays new
   commits, and records the accepted frontier. Failure is fail-closed for that commit;
   it does not poison already accepted state.
5. **Materialize.** Live OR-set quads are streamed into a fresh local `sparq_core::Graph`
   or wasm `Store`. An implementation may incrementally update a private local store,
   but the normative result is the deterministic dataset at the accepted frontier.
6. **Query.** The application passes SPARQL directly to local `sparq-engine` or
   `@sparq-org/sparq`. No query protocol message is defined. Results remain local unless
   the application explicitly exports them.
7. **Edit/publish.** Local edits become CRDT operations, then a signed commit. The client
   encrypts/chunks it, uploads absent blocks, and sends `PublishEvent`. The broker checks
   topic and publisher admission without decrypting the commit and fans out the event.

A query result MUST be labeled with `(repo_id, branch_id, epoch, frontier)`. “Current”
means current at that locally accepted frontier, never globally latest. Offline queries
are valid at the last accepted frontier.

## 7. sparq architecture

Implement this only as an opt-in capability crate, provisionally `sparq-e2ee-ng`, with
default features off. It depends on `sparq-core` for graph ingestion and may offer
adapters for native `sparq-engine` and `sparq-wasm`; neither core crate depends on it.

Suggested internal modules are `capability`, `envelope`, `repo`, `crdt`, `sync`,
`broker_protocol`, and `materialize`. Crypto dependencies stay behind the capability
crate. A separate opt-in broker binary/crate stores opaque blocks and topics; it MUST
not link the query engine. The lean wasm bundle remains unchanged unless its own
`e2ee-ng` feature is selected. Public APIs will require same-change updates to the
matching usage skills.

The crate boundary matters: encryption and networking must not enter `sparq-core`,
`sparq-engine`, or `sparq-substrate`; local materialization uses their existing public
ingestion surfaces. This is a design target, not evidence that such a crate exists.

## 8. Embedded draft specification (not published)

**Namespace target:** the eventual vocabulary is intended for the **jeswr / w3id**
namespace. This record does not allocate or publish any term there. Examples use the
non-dereferenceable placeholder `urn:jeswr:w3id:e2ee-ng:draft:2026-07#` (`eng:`).

### 8.1 Conformance and encoding

The profile identifier is `urn:jeswr:w3id:e2ee-ng:draft:2026-07`. Protocol structures
MUST use deterministic CBOR (RFC 8949 core deterministic encoding). Integers are
unsigned unless stated; byte strings have the exact lengths below. Unknown mandatory
fields cause rejection; extension keys are negative integers and are ignored unless a
negotiated extension says otherwise. Maximum message/block sizes are deployment limits
advertised by `Hello`, never assumed from NextGraph's limits.

Implementations MUST provide algorithm agility. v0 suite names are placeholders pending
external review: `AEAD-256`, `KDF-256`, `SIG-256`, and `WRAP-256`. A deployment MUST bind
one reviewed concrete suite identifier into every capability, commit, and session. It
MUST NOT silently substitute algorithms.

### 8.2 Capability wire object

```cbor-diag
{
  1: 0,                         / version /
  2: h'<32-byte repo id>',
  3: h'<32-byte branch id>',
  4: uint,                      / epoch /
  5: h'<32-byte topic id>',
  6: ["read", "publish"],       / authority; admin distinct /
  7: {1: uint, 2: uint},        / not-before, not-after /
  8: ["wss://broker.example"],
  9: "suite-id",
  10: h'<32-byte read secret>', / secret: omit from public grant /
  11: h'<publisher private key>', / optional; never sent to broker /
  12: h'<admin private key>',   / optional; never combined with 11 /
  13: h'<32-byte random capability nonce>',
  14: h'<admin signature over public grant>'
}
```

Public grants include keys 1–9, 13, the publisher public key when applicable, parent
grant ID, and signature. Secret keys 10–12 MUST be recipient-wrapped or transferred by
an out-of-band protected channel. A broker receives `topic_id`, public publisher/admin
verification keys, grant bounds necessary for admission, and signatures—never 10–12.

### 8.3 Encrypted block and commit envelope

```cbor-diag
/ BlockEnvelopeV0; canonical bytes are authenticated as a whole /
{
  1: 0,
  2: h'<32-byte random block id>',
  3: h'<32-byte random object id>',
  4: uint, 5: uint,             / chunk index, chunk count /
  6: "suite-id",
  7: h'<nonce>',
  8: h'<ciphertext-and-tag>',
  9: uint                       / padded plaintext length class /
}
```

The client derives a per-object key from `K_read`, repo, branch, epoch, and random
object ID using the selected KDF; derives a domain-separated per-block key; chooses a
fresh nonce as required by the suite; and AEAD-seals padded block plaintext. Associated
data binds version, suite, repo/branch/epoch (inside the opaque header), object ID,
block ID, chunk position, and object kind. A block ID MUST be random in v0 and MUST NOT
be a plaintext hash. Brokers MAY integrity-check transport hashes outside the envelope,
but those hashes are not semantic identities.

The decrypted root object is:

```cbor-diag
{
  1: 0, 2: h'<repo id>', 3: h'<branch id>', 4: uint,
  5: [h'<parent commit id>', ...],
  6: h'<author public-key id>', 7: uint, / logical clock /
  8: "or-set-quads-v0",
  9: [h'<operation-object root>', ...],
  10: h'<snapshot root>' / optional /,
  11: h'<author signature over fields 1..10>'
}
```

`CommitId = SHA-256(canonical encrypted root envelope)`. Signatures bind the plaintext
commit fields and the root block ID. An epoch-transition commit additionally binds the
old/new epoch, old/new topics, new verification-key set, history policy, and an admin
signature. Implementations reject nonce reuse, duplicate `(object, chunk)` positions,
wrong associated data, invalid signatures, unknown epochs, missing causal parents after
repair, or invalid CRDT operations.

### 8.4 Client/broker messages

Every request carries `request_id`; every response echoes it and returns either `ok` or
a typed error. Routing fields are outside commit ciphertext and are broker-visible.

- `Hello{versions, suites, max_block_size, padding_classes}` / `HelloAck{chosen,...}`.
- `OpenRepo{overlay_id, topic_id, epoch, peer_id, auth}` establishes routing context;
  despite its name it MUST NOT send stable repo/branch IDs.
- `PinRepo{topic_id, retention}` and `RepoPinStatus{topic_id}` manage opaque retention.
- `TopicSub{topic_id, epoch, after_cursor?}` subscribes; `TopicUnsub{topic_id}` stops it.
- `TopicSyncReq{topic_id, epoch, known_heads[], target_heads[]?, known_commits?}` asks for
  reconciliation. `known_commits` may be a negotiated Bloom filter and is non-authoritative.
- `TopicSyncResp{advertised_heads[], missing_block_ids[], cursor, more}` pages the reply.
- `BlocksExist{ids[]}` returns a bit vector; `BlocksGet{ids[]}` returns envelopes or
  `missing`; `BlocksPut{envelopes[]}` stores opaque blocks idempotently.
- `CommitGet{commit_ids[]}` resolves root block envelopes when the mapping is retained.
- `PublishEvent{topic_id, epoch, commit_id, root_block_id, publisher_key_id, signature}`
  announces an uploaded commit; `Event{..., cursor}` is its fan-out form.
- `EpochAdvance{old_topic, new_topic, transition_commit_id, admin_signature}` replaces
  routing/admission state. New capabilities travel outside this API.

The broker MUST authenticate publisher admission, enforce negotiated sizes/rate limits,
store exact envelope bytes, make idempotent operations stable, and never request a read
secret. It MAY garbage-collect unpinned/unreachable opaque blocks under an advertised
retention policy. It MUST NOT claim completeness: clients detect missing causal closure.

### 8.5 Sync API exposed to applications

The client library surface is:

```text
import_capability(secret_bytes) -> CapabilityHandle
open(handle, BrokerPolicy) -> RepositorySession
sync(session, FrontierPolicy) -> SyncReport
materialize(session, branch, frontier) -> SparqDataset
query(dataset, sparql, QueryOptions) -> (Results, FrontierLabel)
apply(session, QuadOps) -> PendingCommit
publish(session, PendingCommit) -> CommitId
rotate(session, Membership, HistoryPolicy) -> EpochTransition
```

`sync` verifies and persists opaque blocks before replay, repairs parent closure, and
returns accepted/rejected/quarantined commit IDs plus the accepted frontier.
`materialize` MUST be deterministic for the same valid commit closure. `query` MUST be
purely local and MUST NOT contact a broker implicitly. Applications choose whether to
call `sync` first; the result label makes that choice observable.

### 8.6 RDF vocabulary sketch

The future jeswr/w3id vocabulary needs terms corresponding to `eng:Repository`,
`eng:Branch`, `eng:epoch`, `eng:frontier`, `eng:profile`, `eng:crdtKind`, and
`eng:historyPolicy`. Secret capability or key bytes MUST NOT have RDF serialization
terms. RDF metadata is emitted only into the local materialized administrative graph,
which is excluded from application queries unless explicitly included.

## 9. Threat-model handoff and open design gates

The separate privacy/threat-model record should compare Profile CS and NG under broker
compromise, malicious clients, collusion, traffic observation, rollback/equivocation,
capability theft, revocation, device compromise, and convergent-versus-randomized block
encryption. This record's input to that review is the exact ledger in §5.

Before implementation can be called production-ready, `sq-qhy4` must cover at least:
algorithm suite selection; domain separation and nonce lifecycle; capability wrapping
and delegation; publisher/admin separation; epoch transition and rollback resistance;
padding policy; malicious block/commit parsing; broker equivocation; metadata claims;
and test vectors. Until then, all properties remain intended design properties.

## 10. Consequences

The variant gives sparq a concrete local-first collaboration profile: offline edits,
incremental encrypted synchronization, causal result labels, and local full-SPARQL
evaluation using the existing engine. It also adds a broker protocol, CRDT semantics,
capability lifecycle, replay validation, and more observable metadata than Profile CS.
Operators and specs must present that trade honestly. Profile CS remains mandatory as
the simpler baseline; NG is opt-in and complementary.
