<!-- [OPUS-4.8] E2EE-queryable NextGraph-style variant (bead sq-tag1q.3 follow-up; maintainer directive 2026-07-11): a SECOND, complementary E2EE-queryable design modelled on NextGraph — broker/relay-mediated, capability-based, commit/CRDT-encrypted — with an honest privacy-implications analysis contrasting it against the client-only Profile CS of research/e2ee-queryable-options.md. Design-for-review; NO implementation; makes NO unaudited cryptographic-soundness claim (sq-qhy4). -->

# E2EE-queryable, NextGraph-style — a second, broker-mediated variant (design record)

**Status:** Deep-research design record (design-for-review; **no implementation**,
doc-only). Author: Claude Opus 4.8 `[OPUS-4.8]`. Date: 2026-07-11. Bead **sq-tag1q.3**
follow-up (epic **sq-tag1q**, the Solid + SPARQL spec-proposal program), acting on the
maintainer's 2026-07-11 review of the E2EE-queryable survey (PR #1948):

> *"whilst it makes sense for there to be a client-only spec and implementation; this
> should NOT be the only spec and implementation. Could you do another which is closer
> to what nextgraph has in design and explain the privacy implications."*

This record answers that directly. It designs a **second** sparq E2EE-queryable variant
modelled on **NextGraph** (broker/relay-mediated, capability-based, commit/CRDT-level
encryption, local materialized query), **complementary to** — not a replacement for —
the client-only **Profile CS** that the survey [`e2ee-queryable-options.md`](./e2ee-queryable-options.md)
recommends as the core profile. Its **centerpiece is the threat-model + privacy-
implications table** (§6) that contrasts the two models honestly: the NextGraph-style
model buys sharing, multi-writer collaboration, and server-side availability at the price
of **more metadata leakage** (a DAG/sync-topology signal, overlay/topic membership as
seen by a serving broker) than the pure client-only model.

> **Reconciled 2026-07-26 (sq-1vbdy).** This record is the **privacy authority** of the
> canonical Profile-BR set: its §5 threat model, §6 leakage table, and §8 BR-1…BR-9 clauses
> stand. Its *mechanism* choices in §4.1–§4.2 (convergent object keys + dedup,
> content-addressed block IDs, Merkle child keys, block-scoped read-caps, two-level
> capabilities) and its "OR-Set of **triples**" are **superseded** by the v0 binding —
> see the contradiction ledger in
> [`e2ee-program-reconciliation-2026-07.md`](./e2ee-program-reconciliation-2026-07.md),
> which also closes open questions §7 Q3(mechanism)/Q4/Q5/Q6. Companion records:
> [`e2ee-nextgraph-variant-gpt56-2026-07.md`](./e2ee-nextgraph-variant-gpt56-2026-07.md)
> (v0 wire binding) and [`sparql-crdt-gpt56-2026-07.md`](./sparql-crdt-gpt56-2026-07.md)
> (the one shared CRDT).
>
> **Honesty banner.** Nothing here is a cryptographic-soundness claim. Every privacy
> property is stated as **designed / intended**, never *proven* — the ZK/crypto
> production gate **sq-qhy4** (P0, open) is live and no external accredited-cryptographer
> sign-off exists for any sparq crypto. sparq today has **zero E2EE / key-management
> code** (survey §2); this is a design, not a report on a built system. The NextGraph
> facts below are as documented by the NextGraph project (alpha software, itself
> unaudited); where a NextGraph property was not confirmable from primary sources it is
> flagged **UNCERTAIN**. The survey's impossibility statement still holds: **general
> server-side SPARQL over end-to-end-encrypted data without leakage does not exist**;
> this variant does *not* claim to change that — it evaluates query locally after
> decrypt, exactly like Profile CS, and differs in the *sync / sharing / storage* layer,
> which is precisely where the extra metadata leaks.

---

## 0. TL;DR / verdict

- **The two models are complementary, and the maintainer is right that CS should not be
  the only one.** Profile CS (client-only) is the strongest-confidentiality point on the
  curve but is **single-writer-centric, sharing-hostile, and availability-poor**: sharing
  means re-wrapping keys and re-uploading, there is no built-in multi-writer merge, and a
  purely-passive blob server offers no collaboration primitives. The NextGraph-style model
  is the point on the curve that makes **collaboration, offline multi-writer merge, and
  always-on relayed availability** first-class — at a stated metadata cost.
- **Proposed second profile: Profile BR ("broker-relayed").** An opt-in
  `sparq-e2ee-sync` crate layering, on top of the same `sparq-e2ee` envelope Profile CS
  needs: (1) **content-addressed encrypted blocks** (per-object AEAD, key carried in the
  object *reference*, not on the server — NextGraph's convergent-encryption-scoped-to-a-
  secret idea); (2) a **commit DAG** per branch whose commits are encrypted objects;
  (3) a **CRDT merge layer** so multiple writers converge (for RDF, an add-wins OR-Set of
  triples — NextGraph's "graph CRDT", which is the *same shape* as the SPARQL-CRDT track
  sq-tag1q.4, so the two designs must share one CRDT); (4) **capability keys**
  (read-cap = decryption key in a reference; write-cap = signing/topic secret); (5) a
  **broker/relay** that stores and pub/sub-routes encrypted blocks **without decrypting**;
  (6) a **local verifier** that syncs encrypted commits, decrypts, materializes the CRDT
  into a **sparq store**, and runs **full SPARQL 1.1 locally** — identical query topology
  to CS.
- **The privacy trade-off, stated once, plainly:** Profile CS leaks (to a dumb blob
  server) resource-level access patterns + ciphertext sizes/timing only (survey tier T3,
  → T4 with full-replica sync). Profile BR additionally leaks, **to a serving broker**,
  **pub/sub topic membership** (which encrypted branches a client subscribes to /
  publishes to), the **set of a user's devices** (they attach to one broker),
  **per-commit sync timing and block sizes** (a live editing-activity + causal-rate
  signal), and **overlay membership** for that broker's own clients. It does **not** leak
  plaintext, and — if the commit header is itself an encrypted object — the plaintext DAG
  causal links stay hidden, though a broker still observes the *rate and volume* of
  encrypted commit traffic per topic. Net: **BR trades a bounded, well-scoped metadata
  channel for collaboration + availability.** Both belong in the spec; a deployment picks
  per data-sensitivity.
- **No soundness claim, and two honest hard limits carried from NextGraph's own design:**
  (a) **No forward secrecy** — NextGraph explicitly calls PFS an "anti-feature" (a joining
  collaborator must read history), so revocation is **re-key-going-forward only**: a
  removed member who cached old keys can still decrypt all pre-revocation history. Profile
  BR inherits this unless we deliberately diverge (§4.4). (b) Everything cryptographic is
  **designed, unaudited** (sq-qhy4).

---

## 1. Why a second model — the honest gap in client-only CS

The survey's Profile CS is deliberately the *maximally-confidential* corner: server is a
dumb authenticated blob store, all crypto and all query run client-side, and the only
server-visible signal is resource-level access patterns + sizes/timing. That is the right
**core**. But "client-only" carries structural costs that a broker-mediated model exists
to solve, and the maintainer's instinct that it "should not be the only" one is correct on
the merits:

1. **Sharing is a manual re-wrap, not a live grant.** In CS, to share resource *R* with a
   delegate you re-wrap *R*'s DEK to their key and they must fetch the ciphertext. There is
   no notion of a *living shared document* that both parties keep editing; every change is
   a new upload the other side must poll for. NextGraph's **capability + pub/sub** model
   makes "give Bob read access" = hand Bob a read-cap (a reference containing a decryption
   key) and "Bob sees updates" = Bob subscribes to the branch topic on a broker.
2. **No multi-writer merge.** CS as specified is single-writer-at-a-time; two clients
   editing the same resource offline is a last-writer-wins clobber (or a manual conflict).
   Real collaboration needs a **CRDT** so concurrent edits converge deterministically.
   NextGraph builds the whole system on a commit-DAG + CRDT; this is also *exactly* the
   sparq **SPARQL-CRDT** track (sq-tag1q.4) — which is the strongest argument for designing
   BR now: the CRDT is shared work, not duplicated work.
3. **Availability depends on the client being online.** CS's server is passive; if the
   authoring device is offline, a collaborator gets nothing until it returns. A **broker**
   is an always-on relay that stores-and-forwards encrypted blocks so a collaborator syncs
   even when the author is dark — **without the broker ever decrypting**.
4. **No efficient partial sync / dedup.** CS syncs whole resources. NextGraph's
   content-addressed encrypted blocks give **structural dedup** (identical plaintext under
   the same store-secret → identical ciphertext block, referenced by both) and **partial
   fetch** (a client fetches only the blocks its read-cap references).

The cost of all four is **metadata**: to relay, dedup, and route encrypted blocks, the
broker must see *which* encrypted things exist, *who* wants *which* topics, and *when*.
Profile CS's whole confidentiality story is that no such intermediary exists. That is the
trade-off this record makes explicit (§6). Neither profile dominates; they are different
points, and a spec that offers both lets a deployment choose.

---

## 2. What NextGraph actually is (documented design; unaudited alpha)

Grounded in a primary-source research pass over `docs.nextgraph.org` (the Encryption,
Repo-format spec, Network, Sync-protocol, Verifier, CRDTs, Documents, Wallet, DID, and
Social-network pages), the `nextgraph-rs` repository, and the `ng-oxigraph` crate.
NextGraph is **alpha**, dual-licensed **Apache-2.0 OR MIT**, EU-NGI/NLnet-funded, and
**not externally audited** — so it is a *design reference*, not a proof of security.

### 2.1 Data model — repos, branches, commits, blocks, objects

- **Repo** = the E2EE group for one *Document* — its own encryption group with a set of
  member users and one or more branches. A repo is created with a root branch (members +
  permissions) and a main branch (transactional content).
- **Branch** = a transactional structure carrying a branch public-key ID, a **CRDT type**
  chosen per-branch, a pub/sub **TopicID**, and an *encrypted* topic private key. A branch
  is a **DAG of commits**.
- **Commit** = content + author signature + metadata. The commit *header* encodes causal
  links (`acks`/`deps`/`nacks`/`ndeps`), i.e. the DAG edges; the author is a *hashed*
  UserId; write permission is referenced by pointers to root-branch commits that granted
  it. Each commit is signed by an authorized editor.
- **Object** = a content-addressed serialized item (a commit, a commit body, a signature,
  a file, a snapshot), stored as a **Merkle tree of encrypted Blocks** (each block ≤ 1 MB;
  internal nodes carry child block keys, leaves carry encrypted data chunks).
- **CRDT layer — three models, per-branch:** their own **graph CRDT for RDF**, described
  as an add-wins **OR-Set** they formalize as "SU-Set" (SPARQL-Update Set) — chosen because
  "RDF has no unique keys, it cannot conflict"; plus **Automerge** and **Yjs** for
  ordered/text data. RDF triples are stored/synced via the graph CRDT, implemented in their
  **`ng-oxigraph`** fork ("mostly adds CRDTs to RDF/SPARQL").

### 2.2 Encryption — key in the reference, not on the broker

- **Granularity:** per-object, realized per-block (≤ 1 MB blocks).
- **Cipher:** **ChaCha20** (256-bit key), confirmed in the format spec.
- **Key derivation:** **convergent, scoped to a store secret** — the object key is a
  BLAKE3-keyed hash derived from the store's `ReadCapSecret` (so identical plaintext under
  the *same* store secret → identical ciphertext, enabling content-addressing + dedup,
  but *not* global convergent encryption). Nonce = 0 for stored objects; nonce =
  `commit_seq` for pub/sub events.
- **Object reference = `(ObjectId, ObjectKey)`** where `ObjectId` = BLAKE3 digest of the
  encrypted Merkle root and `ObjectKey` = the 32-byte ChaCha20 key. **The decryption key
  is part of the reference**, not stored on the broker — so possession of a reference *is*
  read access, and "brokers know nothing about Documents/Commits/Branches/Stores/Wallets."

### 2.3 Capabilities — read-cap vs write-cap

- **ReadCap** = an object reference (ID + key) pointing at the latest branch/root-branch
  commit; sharing a read-cap grants read of that branch's DAG. Read can be scoped by block
  or by branch.
- **Write** = repo-level (`RepoWriteCapSecret`, a ChaCha20 key stored encrypted in each
  owner's cryptobox); a per-branch `BranchWriteCapSecret` is BLAKE3-derived from it plus the
  TopicID + BranchID. Commits are signed by authorized editors; the pub/sub publisher signs
  events with the topic private key and proves forwarding authorization to the broker.
  Higher-level integrity uses threshold signatures + a certificate chain rooted at the repo
  key.
- **Revocation = epoch renewal (re-key going forward); NO forward secrecy.** NextGraph
  renews the epoch to refresh keys (analogized to a double ratchet); the **inner-overlay ID
  changes on epoch renewal** (e.g., after kicking a member) and the TopicID can be renewed.
  They **explicitly reject Perfect Forward Secrecy** as "an anti-feature… we obviously want
  a joining collaborator to have access to all the historical data of a DAG." **Implication
  (honest):** a removed member who cached old keys keeps read access to all pre-revocation
  history; revocation only prevents reading *new* post-epoch content.

### 2.4 Broker / overlay / sync — what it can see

- **Broker** = an always-on relay (Core / Edge / Local) that stores and pub/sub-routes
  encrypted blocks. Clients connect to *one* broker; brokers relay among themselves.
  Transport is additionally wrapped in the **Noise Protocol**.
- **Overlay = one per Store, split Outer/Inner.** Outer overlay = read-only/anonymous,
  `OuterOverlayId = BLAKE3 hash(StoreId)` (never changes). Inner overlay = editors-only,
  `InnerOverlayId = BLAKE3 keyed-hash(StoreId)` keyed from the store overlay-branch
  ReadCapSecret (**changes on epoch renewal**).
- **Pub/sub:** topics identified by public **TopicID**; subscribe-for-read by presenting
  only the TopicID; publish by signing with the topic private key. Notably "the broker
  doesn't even see or know the branchID" — only the TopicID.
- **A broker CAN see:** the encrypted events/blocks it stores + routes; **TopicIDs and
  pub/sub subscribe/publish relationships** (who subscribes/publishes to which topic);
  overlays it participates in; the users/devices registered to it (a user's device set,
  since "a specific user, and all their devices, have to be connected to the same
  broker"); and the IP/connection metadata of peers connected to it. Block **sizes** and
  sync **timing** are observable.
- **A broker CANNOT see:** plaintext content; the notions of Documents/Commits/Branches/
  Stores/Wallets; the branchID (only the topicID). **UNCERTAIN — DAG visibility:** the
  commit header carries the causal links but there is an optional commit-header *key*, so
  the *plaintext* DAG is hidden if that key is not disclosed; a broker still observes
  per-topic event ordering/timing and the encrypted-block reference graph it routes, which
  is a *structural* signal NextGraph does not quantify. There is **no onion/mixing**, so a
  serving broker learns a partial "who-subscribes-to-what" for *its own* clients; NextGraph
  frames privacy as *no single party sees the whole social graph*, not *no party sees any*.

### 2.5 Verifier / querying / identity

- **Verifier** = the component that decrypts commits and materializes CRDT state; it runs
  **client-side in the app** (native Tauri with persistent storage; browser in-memory;
  Node/Deno; Rust crate), and can *optionally* run in the user's own `ngd` daemon (still
  holding the user's keys — not a zero-trust broker). Query flow: subscribe to branch
  topics → receive encrypted commits → read-cap holders decrypt → apply CRDT patches in
  causal order → materialized RDF → **query with SPARQL** via their `ng-oxigraph` fork.
  **All query is local-after-decrypt; the broker never queries plaintext.**
- **Wallet / identity:** an on-device **Wallet** holds the user's private keys (unlocked by
  a "Pazzle" or mnemonic). Identity is their **own DID method `did:ng`** (NURIs like
  `did:ng:o:<id>`), *not* WebID — a divergence from Solid that matters for a sparq/Solid
  binding (§4.1).

---

## 3. How the two sparq models differ — a precise divergence map

Profile CS is defined in the survey §3.a/§5. Profile BR is this record. They **share** the
AEAD envelope + DEK/KEK vocabulary and the local-decrypt-then-query topology; they
**diverge** in the sync/storage/collaboration substrate. Where they diverge, cite:

| Dimension | Profile CS (survey §3.a) — client-only | Profile BR (this record) — NextGraph-style |
|---|---|---|
| Server role | Dumb authenticated **blob store**; passive | **Broker/relay**: stores + pub/sub-routes encrypted blocks; still never decrypts |
| Unit of storage | Whole encrypted **resource** blob | Content-addressed encrypted **object → Merkle tree of ≤1 MB blocks** (dedup, partial fetch) |
| Change model | Overwrite a resource ciphertext | Append a signed **commit** to a per-branch **DAG** |
| Concurrency | Single-writer / last-writer-wins / manual | **CRDT merge** (RDF = add-wins OR-Set) — multi-writer converges; shared with SPARQL-CRDT (sq-tag1q.4) |
| Sharing | Re-wrap DEK to delegate; delegate polls | Hand a **read-cap** (ref with key); delegate **subscribes** to the topic |
| Availability of updates | Only while author online | Always-on relay stores-and-forwards |
| Key model | Per-resource DEK under per-recipient KEK | **Read-cap = decryption key in the reference**; **write-cap = repo/branch signing secret**; store-scoped convergent object keys |
| What server sees | Resource access pattern + sizes/timing (T3) | + **topic membership**, **device set**, per-commit **sync timing/sizes**, overlay membership for its clients |
| Query | Sync ciphertext → decrypt → in-mem index → **full SPARQL** | Sync encrypted commits → decrypt → **materialize CRDT into a sparq store** → **full SPARQL** (same engine, same locality) |
| Revocation | Re-encrypt on membership change (lazy = weakening) | **Epoch renew / re-key going forward**; **no forward secrecy** (NextGraph's stated stance) |
| sparq gap to ship | Envelope + keys crate only | + block/DAG/CRDT sync layer + capability model + a broker (or reuse a Solid Notifications/pod as relay) |

The **crucial non-obvious divergence** is concurrency + the CRDT: BR is not "CS with a
relay bolted on" — the relay only makes sense once changes are *append-only signed commits
over a CRDT*, because a passive relay cannot merge overwrites. That is why BR must be
co-designed with the SPARQL-CRDT track, and why its metadata surface (a live commit stream
per topic) is fundamentally larger than CS's (occasional whole-resource fetches).

---

## 4. Profile BR — the second design, concretely

Opt-in throughout; **sparq-core / sparq-engine stay lean** (survey §5 rule; MEMORY:
opt-in-feature-architecture). No engine changes: BR reuses the query engine exactly as CS
does — the net-new surface is the *sync + capability + CRDT* layer and an optional broker.

### 4.1 Identity + capability model (Solid-anchored, NextGraph-shaped)

- **Read capability** = a reference `(objectId, objectKey)` — the AEAD key travels *in* the
  capability, per NextGraph §2.2. Possession of a read-cap for a branch head lets a client
  walk + decrypt that branch's commit DAG. Read-caps are the sharing primitive: "share
  branch B with WebID W" = wrap B's read-cap to W's public key (WebID-anchored keypair —
  the Solid binding CS already needs, survey §3.a "Key management").
- **Write capability** = a per-repo signing secret (a per-branch secret derived from it),
  held by editors; every commit is signed, and the broker verifies the *publisher* is
  authorized to forward on the topic (a signature over the broker's identity), without
  learning content. This is NextGraph's split (§2.3) mapped onto sparq.
- **Identity divergence from NextGraph:** NextGraph uses its own `did:ng` DID method; sparq
  should **anchor on WebID** (the Solid binding, `crates/sparq-solid`) and treat the
  capability keypairs as WebID-associated keys, NOT adopt `did:ng`. This keeps BR
  interoperable with the Solid access-control layer and the rest of sq-tag1q. (Open
  question §7: exact key-discovery location in the WebID profile.)

### 4.2 Storage: encrypted objects, blocks, and the commit DAG

- An RDF change becomes a **commit** whose body is an encrypted object (Merkle tree of AEAD
  blocks). Object keys are **store-scoped convergent** (NextGraph §2.2) — identical
  plaintext blocks under one store secret dedup, but two different stores never converge, so
  cross-tenant plaintext-equality leakage is bounded to a single store's own members.
- Commits form a **DAG per branch**; the commit *header* (causal links) is itself an
  encrypted object so the **plaintext DAG stays hidden from the broker** (§2.4 UNCERTAIN
  caveat: a *rate/volume* signal remains — honestly recorded in §6).
- **Padding discipline is normative** (as in CS): block sizes are a fingerprint; the spec
  MUST require size bucketing.

### 4.3 CRDT merge → materialize into a sparq store → SPARQL

- The RDF CRDT is an **add-wins OR-Set of triples** (NextGraph's "graph CRDT" / SU-Set),
  which is the **same CRDT the SPARQL-CRDT track (sq-tag1q.4) specifies** — the two tracks
  MUST share one CRDT definition and one implementation crate (`sparq-crdt`, bead
  sq-tag1q.7), with BR adding the *encryption + sync + capability* envelope on top. Designing
  BR on a divergent CRDT would be a duplication bug.
- **Query flow (identical locality to CS):** a client subscribes to the branch topics its
  read-caps cover → the broker forwards encrypted commits → the client decrypts, applies
  CRDT patches in causal order, and **materializes the resulting triples into an in-memory
  sparq `Graph`/`Store`** (via `load_reader` / `Store.load`, survey §2) → runs **full SPARQL
  1.1 locally**. No fragment carve-out; the broker never evaluates a query. The materialized
  store is exactly what the existing engine expects, so **BR reuses `sparq-engine`
  unchanged** — the same lean-core outcome CS achieves.
- Incremental materialization: because commits are deltas, steady-state re-query applies only
  new commits to the existing materialized store rather than re-parsing everything — a
  genuine efficiency BR has over CS's whole-resource re-sync (recorded as a *shape*, not a
  number; box measurements are non-canonical).

### 4.4 Revocation + forward-secrecy — an honest fork in the road

NextGraph chooses **no PFS**: re-key going forward, removed members keep historical read via
cached keys (§2.3). Profile BR has two honest options, and the spec should present the
trade-off rather than silently inherit one:

- **BR-history (NextGraph-parity):** epoch-renew forward; removed members retain historical
  read. Simplest, matches collaboration-wants-history, but **weakest revocation** — the spec
  MUST disclose "revocation does not retroactively protect history from a member who cached
  keys."
- **BR-reencrypt (stronger, costlier):** on member removal, **re-encrypt (rotate object
  keys for) the sensitive branch history** so a cached-key holder loses access going forward
  *and* to re-fetched history. This costs a re-encryption sweep and breaks dedup across the
  epoch boundary; it does NOT retroactively erase data the removed member *already
  downloaded* (nothing can), and it is **not forward secrecy in the cryptographic sense** —
  it is proactive re-keying. The spec MUST NOT call either option "forward secrecy" or claim
  a post-compromise guarantee (sq-qhy4; and the property is genuinely weaker than the term
  implies).

**No soundness language anywhere:** whichever option, the spec states the *intended*
behaviour and the *disclosed residual*, never a proven guarantee.

### 4.5 The broker — build vs reuse

BR needs an always-on relay that stores + pub/sub-routes encrypted blocks without decrypting.
Two honest paths, to be decided at the spec/impl phase (§7 open question):

- **Reuse Solid as the relay:** a Solid pod already does authenticated resource CRUD +
  Notifications (WebSockets / webhooks). BR could store encrypted blocks as opaque pod
  resources and use Solid Notifications as the pub/sub layer — **maximal reuse, zero new
  server**, at the cost of the pod operator seeing the same block/topic metadata a broker
  would. This is the most Solid-native option and keeps BR inside the sq-tag1q Solid
  program. It does mean the "broker" and the "blob store" are the same Solid server, so the
  metadata observer is the pod host.
- **A dedicated `sparq-broker` relay:** a minimal Rust relay (opt-in crate) implementing the
  block-store + pub/sub + publisher-authorization checks, closer to NextGraph's broker.
  More faithful to the NextGraph model (overlays, inner/outer split) but more surface to
  build + secure.

Recommendation: **specify the block/pub-sub contract abstractly** so *either* a Solid pod or
a dedicated relay can implement it; ship the **Solid-pod-as-relay** binding first (reuse,
lean), leave the dedicated broker as a later opt-in. Either way the broker/relay is
**untrusted for confidentiality** and trusted only for availability + correct routing.

---

## 5. Threat model — actors, assumptions, what each party holds

Two adversary strengths, per the maintainer's ask:

- **Honest-but-curious (HbC) broker/relay/pod host:** follows the protocol, logs and
  analyzes everything it legitimately sees. This is the realistic operator threat.
- **Malicious broker/relay/pod host:** may drop, reorder, withhold, replay, or selectively
  deliver encrypted blocks, and may lie about what it stores — but **cannot forge content**
  (commits are signed) and **cannot decrypt** (no keys). It CANNOT fabricate plaintext, but
  it CAN mount availability attacks and can *equivocate* (show different clients different
  DAG views) unless clients cross-check — an honest limit recorded in §6.

Parties and what they hold:

| Party | Holds | Can decrypt? | Can write? |
|---|---|---|---|
| Data owner (author) | Wallet: read-caps + write-caps for its repos | Yes | Yes |
| Read delegate | A read-cap (reference with key) for a branch | Yes (that branch) | No |
| Broker / relay / pod host | Encrypted blocks; TopicIDs; subscriber map; device sets; timing | **No** | No (routes only) |
| Network observer | Ciphertext on the wire (Noise-wrapped) + traffic timing/volume | No | No |
| Removed member (post-revocation) | Whatever keys it cached before removal | **Historical only** (BR-history) | No |

Assumptions the whole design rests on (all *designed*, none *audited* — sq-qhy4): the AEAD
is not broken; keys are generated + kept client-side and never reach the broker; the CRDT
merge is deterministic + convergent; signatures bind authorship; and the client device +
Wallet are not compromised (a compromised client is game-over for *its* data in **either**
model — neither CS nor BR defends a malicious endpoint).

---

## 6. Privacy-implications table — the centerpiece (CS vs BR)

**How to read this.** Each row is a distinct thing an adversary might learn; each pair of
cells is *what leaks, to whom* under **Profile CS (client-only)** vs **Profile BR
(NextGraph-style)**, using the survey's leakage tiers where they apply (T0 worst … T4 best;
§4 of the survey). "To whom" is the server-side observer: for CS a **dumb blob store**, for
BR a **broker/relay/pod host**. All BR crypto is **designed, unaudited** (sq-qhy4); no cell
is a proven guarantee.

| Leakage / property | Profile CS (client-only) | Profile BR (NextGraph-style) |
|---|---|---|
| **Plaintext triple content** | Hidden. Server sees ciphertext only. | Hidden. Broker sees encrypted blocks only; only read-cap holders decrypt. |
| **Query text / query shape** | Hidden — query runs client-side; server never sees it. | Hidden — same; broker never evaluates or sees a query. |
| **Query answers / result sizes** | Hidden — computed client-side. | Hidden — computed client-side after materialization. |
| **Graph / DAG structure (causal links)** | N/A — no DAG; server sees only whole-resource blobs. | Plaintext DAG **hidden** IF commit headers are encrypted objects (§4.2); **UNCERTAIN residual**: broker observes per-topic commit *rate/volume + ordering* + the encrypted-block reference graph it routes — a coarse structural/activity signal NextGraph does not quantify. **Higher than CS.** |
| **Resource / object access pattern** | **T3** — which resource blobs fetched together, when, how big (traffic analysis; collapses to **T4** under full-replica sync + padding). | **Higher** — broker sees which *encrypted blocks/objects* a client fetches per topic (partial-fetch selectivity is a signal); collapses toward CS-level only under fetch-all-blocks + padding. |
| **Access/search pattern (SSE-style)** | None — no server-side query. | None — no server-side query (broker routes, never searches). Same as CS. |
| **Membership (who is in the group)** | Minimal — server sees who fetches which blobs (a weak co-access signal); no explicit group notion. | **Leaks more** — broker sees **pub/sub topic subscribe/publish membership** for *its own* clients (a partial who-collaborates-with-whom); overlay membership; per NextGraph, no single party sees the *whole* social graph, but a serving broker sees its slice. **Higher than CS.** |
| **User's device set** | Server sees devices only insofar as they fetch blobs (weak). | **Leaks** — a user's devices attach to *one* broker, so that broker learns the device set (NextGraph §2.4). **Higher than CS.** |
| **Timing / size side-channels** | Ciphertext sizes + update timing (**T3/T4** with padding + batching). | **Live commit-stream timing + block sizes per topic** — a real-time *editing-activity + causal-rate* signal (who is actively editing what, when). Padding + size-bucketing required; still a richer temporal channel than CS's occasional fetches. **Higher than CS.** |
| **Network / IP metadata** | Server sees connecting client IPs. | Broker sees connecting client IPs; **no onion/mixing** in the NextGraph model, so the serving broker learns IP↔topic linkage for its clients. (Both mitigated only by Tor/VPN, out of scope.) |
| **HbC broker — net** | Learns: resource co-access + sizes/timing. Learns nothing about content, structure, membership. | Learns: **topic membership, device sets, commit timing/sizes, overlay membership, partial social slice** — but no plaintext, no query, and no plaintext DAG (if headers encrypted). |
| **Malicious broker — net** | Can withhold/reorder/replay *blobs* (availability + staleness attacks); cannot forge content, cannot decrypt. Client detects tampering via AEAD auth + can verify it has the latest via its own records. | Can withhold/reorder/replay/selectively-deliver *commits* and **equivocate** (show different clients different DAG heads) unless clients cross-check heads out-of-band; cannot forge commits (signed) or decrypt. Equivocation-resistance is a **design requirement, not a proven property** (sq-qhy4). |
| **Forward secrecy (FS)** | Not provided (no session-key ratchet in a store-at-rest model); a compromised current key exposes current data. Re-encrypt-on-revocation is the only lever. | **Not provided by design** — NextGraph rejects PFS so joiners can read history; revocation is **re-key going forward** (BR-history) or a proactive re-encrypt sweep (BR-reencrypt, §4.4). Spec MUST disclose; MUST NOT claim FS. |
| **Post-compromise security (PCS)** | None claimed — a compromised client/key exposes its data; re-keying limits *future* exposure only. | None claimed — same; epoch renewal limits *future* exposure but a cached-key removed member keeps historical read (BR-history). No PCS guarantee (sq-qhy4). |
| **Collaboration / multi-writer** | **Weak** — single-writer / manual merge; sharing = re-wrap + poll. | **Strong** — CRDT multi-writer convergence; live read-cap sharing + pub/sub updates. **BR's whole point.** |
| **Availability of updates when author offline** | **Poor** — passive server; collaborators wait for the author to come online. | **Good** — always-on relay stores-and-forwards encrypted blocks. **BR's whole point.** |
| **Server-side selectivity / cost** | None — client downloads all authorized-relevant ciphertext; cost scales with corpus (survey §3.a). | Better — content-addressed partial fetch + dedup + incremental commit sync; **but partial-fetch selectivity is itself an access-pattern signal** (the efficiency *is* the leak). |

**The one-sentence honest verdict:** Profile BR **leaks strictly more metadata than CS** —
a bounded, well-characterized channel of *topic/overlay membership + device sets + live
commit timing/sizes + a coarse DAG-activity signal, to a serving broker* — and in exchange
delivers *multi-writer collaboration, live sharing, and server-side availability* that CS
structurally cannot. It leaks **no more plaintext, query, or answer** than CS. Choose CS for
maximal confidentiality of a mostly-private corpus; choose BR when collaboration/availability
is worth exposing the sync-topology metadata to the relay.

---

## 7. Recommendation + open questions for the maintainer

**Recommendation.** Add Profile BR to the E2EE-queryable spec (sq-tag1q.5) as a **second,
optional profile alongside** Profile CS and Profile SE — CS remains the mandatory-to-
implement core; BR is the *collaboration/availability* profile with a **mandatory leakage
statement** (the §6 verdict, in-spec). Make **leakage a normative vocabulary** (the survey's
T0–T4 + BR's added membership/timing rows) so BR must declare its tier. Keep the ZK/MPC
composition an **informative, non-normative annex** with the sq-qhy4 caveat verbatim (BR does
not change that either). Design BR's CRDT as the **same** artifact as SPARQL-CRDT
(sq-tag1q.4/.7) — one `sparq-crdt` crate, BR adds the encryption/sync/capability envelope.

**Open questions that genuinely need the maintainer:**

1. **Broker path (§4.5):** Solid-pod-as-relay (reuse, lean, pod host is the observer) vs a
   dedicated `sparq-broker` (NextGraph-faithful, more surface)? Recommend Solid-pod-first;
   confirm.
2. **Revocation semantics (§4.4):** BR-history (NextGraph-parity, weakest revocation, honest
   disclosure) vs BR-reencrypt (costlier, breaks dedup, stronger-but-not-FS)? Or make it a
   per-deployment knob the spec exposes?
3. **Identity (§4.1):** confirm WebID-anchored capability keys (recommended) over adopting
   NextGraph's `did:ng`. Where do recipient public keys live in the WebID profile?
4. **CRDT sharing (§4.3):** confirm BR and SPARQL-CRDT (sq-tag1q.4) share one CRDT + one
   crate; who owns the definition?
5. **Named-graph / dataset shape:** NextGraph maps one Document → one repo/branch DAG. How
   do sparq **named graphs / quads** map onto branches (one branch per named graph? one repo
   per dataset)? Affects both the CRDT and the metadata surface.
6. **Interaction with Profile SE (survey §3.c):** could BR carry *structure-exposed*
   commits (predicates cleartext, values AEAD) to let the broker do coarse structural
   routing? This *increases* leakage to full topology — record as a possible-but-discouraged
   composition, not a default.

**Uncertainties (carry forward, do not resolve here):** (a) the exact broker-visible DAG
signal in NextGraph is **UNCERTAIN** (§2.4) — BR's §6 "graph/DAG structure" row is
conservative pending a firmer read of the `ng-net`/`ng-broker` sources; (b) NextGraph is
itself **alpha + unaudited**, so it is a design reference, not a proof; (c) sparq has **no
crypto code**, so all of BR is design; (d) no cryptographic property here is proven — sq-qhy4
gates any soundness claim.

---

## 8. Spec-text draft for Profile BR (target: jeswr / w3id namespace — NOT published here)

The following is **draft normative-flavored spec text** for the second profile, to be folded
into `site/specs/e2ee-sparql.typ` (house Typst format, sq-rvgr2) and later contributed to the
**jeswr / w3id-controlled** spec namespace (bead below). **It is NOT published to that
namespace by this record** — it is drafted here for review. Status language MUST never claim
W3C standing (sq-tag1q house rule). RFC-2119 keywords are UPPERCASE.

> ### Profile BR — Broker-Relayed E2EE-Queryable (optional, leakage-disclosed)
>
> **BR-1 (Overview, informative).** A conforming Profile-BR deployment stores RDF as a
> DAG of end-to-end-encrypted commits over a per-branch CRDT, relayed by an untrusted
> broker that stores and routes encrypted blocks without decrypting them. Clients holding
> the relevant read capability synchronize the encrypted commits, decrypt and merge them
> locally into a materialized RDF store, and evaluate SPARQL 1.1 over that local store.
> Query evaluation is ALWAYS local; the broker MUST NOT evaluate queries.
>
> **BR-2 (Encryption granularity).** Content MUST be encrypted per object as a tree of
> AEAD-encrypted blocks. An implementation MUST use an AEAD from the spec's algorithm
> registry. Block plaintext sizes MUST be padded to size buckets from the registry;
> commit *headers* SHOULD be encrypted objects so the plaintext commit DAG is not exposed
> to the broker.
>
> **BR-3 (Object reference).** A read capability MUST be an object reference carrying both
> the object identifier and the decryption key; the decryption key MUST NOT be transmitted
> to or stored by the broker. Possession of a read capability grants read access to the
> referenced branch. Object keys MAY be derived convergently scoped to a per-store secret
> to enable content-addressing and deduplication; an implementation using convergent keys
> MUST disclose the resulting equal-plaintext-within-a-store linkability.
>
> **BR-4 (Write capability + authorship).** Every commit MUST be signed by an editor
> holding the repository (or per-branch) write capability. A broker MUST verify that a
> publisher is authorized to forward on a topic before relaying, WITHOUT access to
> plaintext. A broker MUST NOT be able to forge or alter commit content.
>
> **BR-5 (CRDT convergence).** RDF content MUST be represented as a convergent replicated
> data type such that concurrent commits from multiple writers merge deterministically to
> the same materialized graph. The RDF CRDT MUST be the one defined by the SPARQL-CRDT
> profile of this specification; Profile BR adds only the encryption, capability, and
> relay envelope.
>
> **BR-6 (Sync + query flow).** A client MUST subscribe only to the branch topics its read
> capabilities cover, decrypt received commits, apply CRDT merge in causal order, and
> materialize the result into a local RDF store over which it evaluates SPARQL 1.1
> locally. Re-evaluation SHOULD apply only new commits incrementally.
>
> **BR-7 (Revocation).** An implementation MUST document its revocation semantics as
> exactly one of: (a) *forward re-key* (a removed member retains read access to history it
> already holds or can re-fetch cached-key-decryptable ciphertext), or (b) *proactive
> re-encryption* (sensitive history is re-encrypted so cached-key holders lose access to
> re-fetched history; data already downloaded by a removed member cannot be revoked). An
> implementation MUST NOT describe either as "forward secrecy" or as a post-compromise
> guarantee.
>
> **BR-8 (Mandatory leakage statement, normative).** A Profile-BR implementation MUST
> surface, in its conformance documentation, a leakage statement to the effect that a
> serving broker observes: pub/sub topic and overlay membership, the set of a user's
> devices, per-commit synchronization timing and block sizes, and client network metadata
> — while it does NOT observe plaintext content, query text, query answers, or (if commit
> headers are encrypted) the plaintext commit DAG. The statement MUST note that this
> metadata surface is strictly larger than the client-only Profile CS.
>
> **BR-9 (No soundness claim).** No conformance claim under this profile asserts a proven
> cryptographic-soundness, forward-secrecy, or post-compromise-security property. Any
> composition with zero-knowledge or multi-party-computation mechanisms is governed by the
> specification's non-normative verifiability annex and its pending-external-audit caveat.

---

## References

NextGraph facts are from a primary-source pass over the NextGraph documentation and
`nextgraph-rs` (2026-07-11); NextGraph is **alpha, unaudited** — a design reference, not a
security proof. sparq-estate + cryptographic-literature citations are inherited from the
survey [`e2ee-queryable-options.md`](./e2ee-queryable-options.md); see its reference list
for [IKK12], [CGPR15], [NKW15], [GSBNR17], [ZKP16], [NS09], [FKPS17], [FKPS20], [TV23],
[Wri25], [BWK26] and the rest.

- NextGraph — Encryption. <https://docs.nextgraph.org/en/encryption/>
- NextGraph — Repository format specification. <https://docs.nextgraph.org/en/specs/format-repo/>
- NextGraph — Network / brokers / overlays. <https://docs.nextgraph.org/en/network/>
- NextGraph — Sync protocol. <https://docs.nextgraph.org/en/protocol/>
- NextGraph — Verifier. <https://docs.nextgraph.org/en/verifier/>
- NextGraph — CRDTs. <https://docs.nextgraph.org/en/framework/crdts/>
- NextGraph — Documents / stores / blocks. <https://docs.nextgraph.org/en/documents/>
- NextGraph — Wallet. <https://docs.nextgraph.org/en/wallet/>
- NextGraph — DID / NURI. <https://docs.nextgraph.org/en/framework/nuri/>
- NextGraph — Social network / anonymity. <https://docs.nextgraph.org/en/social-network/>
- `nextgraph-rs` (Apache-2.0 OR MIT, alpha). <https://git.nextgraph.org/NextGraph/nextgraph-rs>
- `ng-oxigraph` (Oxigraph fork adding CRDTs to RDF/SPARQL). <https://docs.rs/crate/ng-oxigraph/latest>

In-estate records cited: [`e2ee-queryable-options.md`](./e2ee-queryable-options.md) (the
client-only survey this record complements), and — via that survey —
[`crypto-erase-at-rest.md`](./crypto-erase-at-rest.md),
[`mpc-zkp-federated-sparql-design.md`](./mpc-zkp-federated-sparql-design.md),
[`zk-audit-readiness-dossier.md`](./zk-audit-readiness-dossier.md),
[`sparq-solid-scope.md`](./sparq-solid-scope.md).
