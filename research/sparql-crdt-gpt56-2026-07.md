<!-- [GPT-5.6] SPARQL-CRDT design record for sq-tag1q. This is a design, not an implementation or proof. -->

# SPARQL-CRDT: replicated RDF datasets with SPARQL Update at origin

**Status:** proposed design; no implementation and no formal proof. **Date:** 2026-07-11.
**Program:** `sq-tag1q`; this record informs the normative proposal `sq-tag1q.4` and the
opt-in implementation `sq-tag1q.7`. It extends those beads and the E2EE survey in
[`e2ee-queryable-options.md`](./e2ee-queryable-options.md); it does not replace the
proposal draft or duplicate E2EE envelope/key design.

> **Reconciled 2026-07-26 (sq-1vbdy).** This is the **single** RDF CRDT of the sparq
> estate: the E2EE Profile BR
> ([`e2ee-queryable-nextgraph-variant-2026-07.md`](./e2ee-queryable-nextgraph-variant-2026-07.md)
> + its v0 binding
> [`e2ee-nextgraph-variant-gpt56-2026-07.md`](./e2ee-nextgraph-variant-gpt56-2026-07.md))
> carries **these** deltas and defines no CRDT of its own — see the binding contract (one
> replication domain per branch, one shared membership epoch, opaque-payload encoding
> boundary) in
> [`e2ee-program-reconciliation-2026-07.md`](./e2ee-program-reconciliation-2026-07.md).
> Where this record and the frozen proposal `site/specs/sparql-crdt.typ` (sq-tag1q.4)
> differ, **the proposal wins**: notably its `CRDT-SCOPE` puts `COPY`/`MOVE`/`ADD` outside
> the profile, superseding those three rows of §5.2.
>
> **Honesty boundary.** The algorithms below are *designed* to make replicas that have
> accepted the same valid deltas expose the same RDF dataset. The algebra is based on
> established dotted observed-remove and delta-state CRDT constructions, but this exact
> dataset/update/interchange design has not been formally proved, model-checked, or
> implemented in sparq. Property testing and a formal convergence proof are separate
> implementation beads. “Converges” below means this intended, narrowly scoped property;
> it does not mean preservation of every user's update intent, serializability, causal
> consistency under an arbitrary relay, RDF entailment convergence, or Byzantine safety.

## 1. Decision in one page

The replicated value is an **RDF dataset**, represented as a set of RDF 1.1 quads. The
default graph has a distinguished graph key; every named graph IRI is another key.
Each quad is an element of an **add-wins observed-remove set** implemented as a dotted
delta-state CRDT:

- an add creates a globally unique dot `(replica_id, counter)` and associates it with a
  canonical quad;
- a remove records all dots for that quad visible in the evaluator's causal context;
- a concurrent add has a dot the remover did not observe, so it remains visible;
- state and delta joins use set union plus the causal-context/dot-store rules, not wall
  clocks; replay and duplicate delivery are intended to be harmless;
- tombstones are represented by the causal context, compacted as a version vector plus
  a sparse dot cloud, rather than retained as one unbounded quad tombstone each.

The **authoring log** is SPARQL Update. The **replication log** is not the original query:
an origin evaluates the update once against its local visible snapshot, expands it to a
finite ordered transaction containing concrete quad additions/removals, allocates dots,
and publishes one atomic CRDT delta. A receiving replica applies that delta; it never
re-evaluates the `WHERE` clause. This distinction is essential: re-evaluation would make
delivery order and the receiver's snapshot affect the result.

The wire unit is a deterministic binary/CBOR-style sidecar envelope containing protocol
version, dataset ID, origin ID, transaction ID, origin causal context, removals, dotted
adds, and optional cleartext provenance. RDF reification is not used for causal metadata:
metadata is replication state, not asserted domain data, and putting it in the dataset
would alter query answers and recurse into replication of its own bookkeeping.

The implementation belongs in a new, opt-in `sparq-crdt` crate. Nothing in
`sparq-core`, `sparq-engine`, the native binaries, Python, or lean wasm depends on it by
default. The crate wraps a sparq graph/dataset projection and uses `sparq-engine` only at
the origin for pattern-update evaluation.

## 2. Requirements and non-goals

### Requirements

1. Offline, multi-writer mutation of an RDF dataset and retry-safe delta exchange.
2. Per-quad semantics across the default graph and named graphs.
3. SPARQL 1.1 Update as the application authoring interface.
4. Deterministic materialization from CRDT state into sparq's queryable indexes.
5. Incremental peer sync without requiring exactly-once transport.
6. A clean composition point with client-side E2EE: merge plaintext locally, transport
   authenticated ciphertext; do not put keys or encryption into the CRDT algebra.
7. Explicit conflict and blank-node semantics rather than hidden last-writer-wins rules.

### Non-goals

- ACID isolation or a single global order across replicas.
- Replaying a pattern update at every receiver.
- Preserving the result of a hypothetical serial execution of concurrent updates.
- Replicating inferred triples as authoritative facts. Entailment is a local derived view.
- Sequence editing. RDF Collections remain ordinary triples and receive set semantics.
- Byzantine agreement, authorization, signatures, E2EE, or key distribution. Those may
  wrap admission and transport but are not properties supplied by the CRDT.
- Making `LOAD`, `SERVICE`, or remote dereferencing deterministic across origins.

## 3. Prior art and fit

### SU-Set and RDF-native work

Ibáñez et al.'s SU-Set directly targets RDF graphs updated through SPARQL Update. It is
the closest semantic ancestor: update is evaluated at a replica and represented with
commutative replicated-set operations. Later RDF GraphStore work and decentralized RDF
evolution research reinforce the need to separate graph reconciliation from user-level
change intent. This design adopts SU-Set's RDF/SPARQL framing but makes the unit a
**quad**, gives named graphs first-class identity, and uses modern dotted delta-state
metadata and an explicit sync envelope.

The m-ld system represents live RDF-like information with clone journals and uses a
sequence CRDT for ordered data. Its useful lesson is architectural: a journal/change
protocol and a queryable graph projection are distinct layers. We do not adopt a
sequence CRDT for the base RDF dataset because RDF graphs are sets; RGA/LSEQ-style
ordering is useful only for an application abstraction such as a collaboratively edited
list. Encoding an RDF Collection as `rdf:first`/`rdf:rest` triples does not magically
give it sequence conflict semantics.

### NextGraph

NextGraph exposes an RDF Graph CRDT based on observed-remove/SU-Set logic alongside
Automerge and Yjs document components. Its document model cleanly separates graph,
discrete structured content, and binary attachments; commits can contain concurrent
CRDT operations. That validates the composition rather than suggesting one universal
CRDT. sparq should likewise use a quad-set CRDT for RDF, and let an application embed an
Automerge/Yjs-style value when it truly needs maps, text, or sequences. NextGraph also
combines CRDTs with encrypted repositories, but this record does not copy or claim
compatibility with its commit, signature, quorum, or encryption protocols.

### Delta-state CRDTs, OR-Sets, and RGA

Almeida, Shoker, and Baquero show how delta-mutators can emit small joinable states and
how causal delta intervals support causally consistent anti-entropy. A dotted causal
container splits live payload dots from a causal context; this is a good fit for RDF
because a dataset can be large while a SPARQL transaction often touches few quads.
Observed-remove semantics give deletion a defensible meaning: remove what the user saw,
not unseen concurrent work. Add-wins is preferable to remove-wins for collaborative RDF
because it does not silently erase a concurrently asserted fact. This is a policy
choice, not a universal semantic truth, and must be visible in the API/profile name.

RGA is an ordered-sequence CRDT. It is not the base representation: triples/quads have
no standard order, and imposing RGA identifiers would add irrelevant order metadata to
every fact. A later RDF-list abstraction could translate list edits into an RGA side
object and project it to RDF, but mixing that into v1 would blur two data types.

### Automerge and Matrix-style event DAGs

Automerge offers nested maps, lists, and text with deterministic handling of concurrent
changes, exposes same-property conflicts, and has a transport-independent sync protocol.
It is a useful model for change hashes, heads, resumable peer sync, and keeping conflict
information inspectable. Mapping RDF into a generic JSON map is less suitable: RDF
predicates are naturally multi-valued, blank nodes and named graphs do not map cleanly,
and map-register “winner” rules would discard legitimate concurrent objects.

Matrix rooms use an authenticated event DAG and deterministic state resolution. That is
a useful dissemination/admission comparison, especially for versioned event formats,
but it is not an RDF set CRDT. A total resolution algorithm is more machinery than the
quad set needs and introduces authorization/state-event semantics outside SPARQL. v1
therefore uses joinable CRDT deltas; a signed event-DAG transport can carry them later.

### Why not LWW

Last-writer-wins needs a total timestamp/tie-break order, makes clock or actor ordering
part of RDF truth, and discards one concurrent assertion without representing the
conflict. RDF predicates are multi-valued by default, so two concurrent objects are not
intrinsically a conflict. Shape validation may diagnose a `sh:maxCount 1` violation,
but the replicated dataset should retain both facts until an explicit update resolves
it. CRDT convergence and SHACL validity are separate properties.

## 4. Replicated data model

### 4.1 Identifiers and canonical elements

```text
ReplicaId    = 128 or 256 bits, random and persistent per replica incarnation
Counter      = monotonically increasing u64, never reused by that ReplicaId
Dot          = (ReplicaId, Counter)
DatasetId    = stable opaque identifier for one replication domain
GraphKey     = Default | Named(Iri)
QuadKey      = (subject, predicate, object, GraphKey)
VersionVector[ReplicaId] = greatest contiguous observed counter
DotCloud     = observed counters above gaps not summarized by the vector
CausalContext = (VersionVector, DotCloud)
DotStore     = map QuadKey -> non-empty set<Dot>
State        = (DatasetId, DotStore, CausalContext)
```

IRIs and literals use their RDF-term identity, not display prefixes. Literal identity
includes lexical form, datatype IRI, and language tag as RDF defines it. RDF-star triple
terms require an explicitly versioned extension and are out of v1 unless the normative
spec chooses RDF 1.2 dataset terms.

`ReplicaId` rotation creates a new identity; counters must survive process restarts.
Exhaustion or lost durable counter state must fail closed by rotating the ID, never
reusing dots. `DatasetId` prevents applying a valid delta to the wrong dataset.

### 4.2 Visibility, add, remove, and join

A quad is visible iff `DotStore[quad]` is non-empty.

**Add(q):** increment the local counter, create fresh dot `d`, insert `d` under `q`, and
observe `d` in the causal context. Emit the corresponding delta.

**Remove(q):** read `R = DotStore[q]`. Remove those dots from the live store while
retaining them as observed by the causal context. Emit a delta that carries `R` as
removed/observed dots. Removing an absent quad is a no-op. A dot created concurrently
with this evaluation is not in `R`, so it survives: add-wins concurrency.

**Join:** merge causal contexts, retain a live `(q,d)` if it is live on both sides, or if
it is live on one side and the other side has not observed-and-removed `d`. Compact the
resulting context into contiguous version-vector prefixes plus sparse gaps. This is the
standard dotted observed-remove shape; the normative spec and implementation must give
the full equations and executable test vectors rather than relying on this prose.

The intended merge laws are commutativity, associativity, and idempotence. They are an
acceptance target to test and prove, not a claim that this prose constitutes a proof.

### 4.3 Tombstones and garbage collection

The causal context is the logical tombstone. Version-vector compaction prevents one
record per removed quad/dot in the common contiguous case. A sparse dot cloud remains
when delivery has gaps.

Garbage collection is unsafe merely because a wall-clock retention period elapsed. A
replica may drop context only below a **causal stability frontier** acknowledged by every
replica that can still write in the current membership epoch. Membership changes require
an epoch protocol: revoke a replica, establish a new membership set/frontier, and prevent
the old replica's unauthenticated re-entry. v1 may conservatively retain context forever
if no trusted stability service is configured. Snapshot compaction must preserve the
frontier and rejected-old-epoch rule.

### 4.4 Blank nodes

Parser-local blank-node labels cannot identify the same node across replicas. At origin
ingest and before delta creation, every blank node introduced by an update is replaced
with a stable, non-dereferenceable skolem IRI under a dataset-scoped namespace, derived
from `(DatasetId, transaction_id, origin-local blank label/random nonce)`. Existing
skolem IDs remain unchanged. The delta protocol never carries raw blank-node labels.

Query/result APIs may render those IRIs as implementation-managed blank nodes when safe,
but replicated identity is the skolem IRI. This chooses stable identity and replay over
preserving raw blank-node syntax. It is intentionally distinct from RDFC canonical
blank-node labels, which describe a graph snapshot and can change as the graph changes.

## 5. SPARQL Update compilation

### 5.1 Transaction boundary

One accepted SPARQL Update request becomes one `CrdtTransaction`:

```text
tx_id, dataset_id, origin_id, origin_context,
source_update_hash?, ordered concrete removes, ordered dotted adds
```

The origin evaluates all operations using the SPARQL Update request's normal sequential
semantics against one local transaction view: effects of earlier operations are visible
to later operations. It then commits the local state and emits the concrete aggregate
delta atomically. Relays and replicas either admit the whole transaction or none of it.
Atomic visibility is a protocol/store property; the merge algebra itself sees a delta.

The optional source hash/provenance supports audit and UI history. The raw update text is
not needed for convergence and may be omitted or encrypted because it can leak data.

### 5.2 Concrete mapping

| SPARQL Update form | Origin behavior | Emitted CRDT effect |
|---|---|---|
| `INSERT DATA` | Resolve graph targets; skolemize new blank nodes | Fresh dotted add per concrete quad not already represented by the transaction policy |
| `DELETE DATA` | Resolve exact quads; blank nodes are prohibited by SPARQL grammar | Remove all locally visible dots for each concrete quad |
| `DELETE/INSERT ... WHERE` | Evaluate `WHERE` once on origin snapshot; instantiate templates per solution; apply SPARQL ordering (`DELETE` then `INSERT`) | Concrete observed removes and fresh adds; receivers never evaluate `WHERE` |
| `DELETE WHERE` | Evaluate and instantiate at origin | Concrete observed removes |
| `CLEAR` / `DROP` | Enumerate locally visible quads in selected graph(s) | Remove their visible dots; concurrent/unseen adds survive |
| `COPY` | Snapshot source at origin, clear destination locally, add instantiated destination quads | Concrete removes + adds; not a persistent alias |
| `MOVE` | `COPY` plus locally observed source removal | Concrete source/destination effects |
| `ADD` | Snapshot source and add destination quads | Concrete adds |
| `CREATE` | Record graph-existence metadata only if the profile models empty graph existence | See §5.4 |
| `LOAD` | Dereference and parse once at origin, recording content digest/media type | Concrete adds; receiver never dereferences |
| `WITH` / `USING` | Affect origin evaluation dataset exactly as SPARQL defines | No special wire operation |

For `INSERT DATA` of a currently visible quad, the recommended v1 policy is a semantic
no-op rather than creating an extra support dot. For a delete-then-insert in the same
transaction, deletion observes the old dots and insertion creates a fresh dot. The
normative test vectors must pin this down because redundant support dots affect later
delete storage, though not the visible RDF set.

### 5.3 Worked concurrency examples

**Concurrent add and delete.** Both replicas start with quad `q` at dot `a:1`. A removes
`q`, observing `a:1`; B concurrently inserts `q`, allocating `b:1`. After exchange,
`a:1` is removed and `b:1` remains, so `q` is visible.

**Two values under a functional-looking predicate.** A inserts `:x :status "draft"` and
B inserts `:x :status "final"`. Both remain. RDF permits this; a SHACL `maxCount` rule may
flag it, and a user can resolve it with a later observed delete. The CRDT does not invent
an LWW winner.

**Pattern update.** A runs `DELETE { ?s :status "draft" } INSERT { ?s :status "final" }
WHERE { ?s :status "draft" }`. Only matches visible at A are concretized. A concurrent
new `:status "draft"` at B survives. Replicas converge after receiving the same delta,
but the final dataset need not equal executing the textual update after B's insertion.

### 5.4 Named graphs and empty graph existence

Quads make graph membership part of element identity: deleting a quad from graph `g1`
does not affect the same triple in `g2`. `GRAPH ?g` pattern results are evaluated at the
origin and concretized normally.

RDF datasets and store APIs disagree on whether an empty named graph has observable
existence. v1 should choose one of two explicit profiles:

1. **Quad-only baseline (recommended):** graph existence is derived from visible quads;
   `CREATE`, empty graphs, and the distinction between `CLEAR` and `DROP` have no durable
   replicated observation beyond their quad effects.
2. **Graph-catalog extension:** a second add-wins observed-remove set replicates graph
   names. `CREATE` adds a name, `DROP` removes observed name dots plus observed quads,
   and `CLEAR` removes only quads.

Keeping the catalog as a separate typed CRDT avoids fake bookkeeping triples appearing
in SPARQL results. The implementation bead should start quad-only unless the normative
UPD selects the extension as required.

### 5.5 Errors, nondeterminism, and authorization

If parsing, evaluation, skolemization, external fetch, quota, or durable-dot allocation
fails, no local mutation or delta is committed. Functions such as `NOW`, `UUID`, and
`RAND` are evaluated once at the origin and their concrete terms are replicated.
`SERVICE` and `LOAD` execute only at origin and should be disabled by default in offline
or high-assurance profiles.

Authorization is checked when an origin creates a transaction and again when a receiver
admits it. Rejection policy must be deterministic within a sharing/membership epoch or
authorized replicas may intentionally expose different admitted datasets. The CRDT does
not make malicious or differently authorized replicas converge.

## 6. Delta interchange and synchronization

### 6.1 Envelope

Use a versioned, canonical binary envelope (canonical CBOR is a candidate; the spec must
freeze its profile before implementation), not RDF reification:

```text
protocol_version, dataset_id, membership_epoch,
origin_replica_id, tx_id, origin_context,
removed: [(quad_key, [dot...])],
added:   [(quad_key, dot)],
optional source_update_hash, optional previous_change_hashes
```

Canonical term encoding must define UTF-8 normalization policy, datatype/language-tag
encoding, default/named graph tags, length limits, sorting, duplicate rejection, and a
hash domain separator. Unknown versions fail closed. Resource limits are checked before
allocation. A signature or AEAD authenticates the exact canonical bytes at a surrounding
security layer; neither should sign an ambiguous alternate encoding.

RDF-star reifiers are unsuitable for v1 metadata because causal dots are not domain
assertions, applications could edit them through SPARQL, and querying would expose
replication internals. A debugging export may render metadata as RDF, explicitly outside
the replicated dataset and never accepted as the canonical wire form.

### 6.2 Peer protocol

1. Peers exchange `DatasetId`, protocol/profile versions, membership epoch, and causal
   summaries.
2. Each side requests missing delta intervals when its peer can serve them.
3. Deltas may be duplicated and reordered; admission validates identity, epoch, bounds,
   and optional authentication before join.
4. If history was compacted or gaps cannot be served, send a snapshot consisting of the
   dot store, causal context, stability frontier, and a content hash.
5. A peer acknowledges the new causal summary; acknowledgements feed stability tracking
   but do not alone authorize garbage collection outside the membership rule.

Delta intervals should be causally closed when the profile promises causal visibility.
If transport delivers arbitrary independent deltas, the quad set is still intended to
eventually converge after all valid deltas arrive, but intermediate snapshots may not
reflect causal transaction order. Multi-delta transactions must not become partially
visible.

Back-pressure, maximum transaction size, snapshot chunking, and malicious sparse-dot
cloud growth are mandatory implementation concerns. Merkle summaries may optimize
reconciliation later; they do not replace causal context.

## 7. Conflict semantics and user experience

“Conflict-free” describes deterministic merge, not absence of semantic disagreement.

- concurrent additions of distinct quads are both retained;
- concurrent add versus observed remove is add-wins;
- concurrent removes are idempotent;
- shape/cardinality/ontology contradictions remain visible data;
- a pattern update affects only its origin snapshot;
- an explicit later update can resolve a semantic conflict by observing and removing
  all unwanted live dots.

The public API should expose transaction provenance and, optionally, per-quad live-dot
origins for conflict-aware tooling. Ordinary SPARQL query sees only the RDF dataset, not
dots. SHACL validation can run after merge and report violations without rejecting
otherwise valid deltas by default; rejection based on local validation would risk
divergence unless every replica shares the identical deterministic policy and shapes.

Inference should be recomputed as a local derived layer from converged asserted quads.
Replicating materialized RDFS/OWL/N3 consequences as ordinary additions would confuse
asserted and derived support and makes retraction non-local.

## 8. sparq architecture

Create `crates/sparq-crdt`, default features empty, with no reverse dependency from
`sparq-core` or `sparq-engine`:

```text
sparq-core Graph/dataset indexes  <--- materialized visible quad projection
             ^
             | depends on
sparq-crdt: state, dots, deltas, codec, journal, sync summaries
             |
             +-- optional/origin-update feature --> sparq-engine evaluator
```

Suggested modules:

- `id`: dataset/replica/dot/transaction identifiers and durable counter contract;
- `state`: dotted quad set, graph catalog extension, merge, materialization iterator;
- `delta`: local mutators and atomic transaction delta;
- `update`: evaluate-at-origin adapter to sparq-engine, feature-gated;
- `codec`: bounded canonical interchange and test vectors;
- `journal`: append, snapshot, crash recovery, causal interval serving;
- `sync`: summaries and reconciliation state machine;
- `provenance`: optional non-query-visible audit metadata.

The first implementation may rebuild a `Graph` from visible quads after merge for
correctness. An incremental adapter should later translate visibility edges (absent→
present, present→absent) into index changes without exposing dots to the engine. The
store must durably append a transaction and advance its dot counter before acknowledging
success; crash tests must cover every boundary.

As a new public crate it requires `skills/sparql-crdt/SKILL.md`, a short README, direct
unit tests for every public function, property tests over reordered/duplicated delivery,
codec vectors, fuzzing of untrusted envelopes, both-feature-state clippy/tests/rustdoc,
and dependency attestations for any new package. The workspace and wasm size ratchets
must show that default consumers remain unchanged.

## 9. Composition with E2EE

This is the interface with PR #1948's client-side E2EE direction:

```text
SPARQL Update
  -> local plaintext evaluation
  -> concrete canonical CRDT delta
  -> optional signature / authorization evidence
  -> AEAD envelope under the E2EE profile
  -> untrusted relay/blob store
  -> recipient authentication + decryption
  -> CRDT admission and merge
  -> local plaintext sparq indexes and query
```

CRDT metadata can leak actors, counters, graph/term identity, transaction size, and
change timing. Therefore the client-side E2EE profile should encrypt the entire delta
payload including causal context and terms; only routing envelope fields that the relay
strictly needs stay clear. Padding/batching may reduce size/timing leakage but is an E2EE
spec decision. The relay must not need plaintext SPARQL or plaintext quads.

Sign-then-encrypt versus encrypt-then-sign, dataset key hierarchy, recipient membership,
key rotation, revocation, replay windows, and forward secrecy belong to the E2EE design.
The CRDT requires only an authenticated admission result plus stable `DatasetId`,
`ReplicaId`, and membership epoch. Dot identity must not be derived from encryption keys:
key rotation must not rewrite the CRDT history.

Profile SE from the E2EE survey (structure exposed, literals encrypted) composes
differently: the CRDT may merge cleartext topology and opaque literal ciphertext terms,
but equality and duplicate suppression then operate on ciphertext term identity. Key
rotation can create a semantically duplicate plaintext with a different ciphertext.
That profile therefore needs a stable keyed equality/term identifier or a decrypt-and-
rewrite migration; it must not be declared compatible until specified and tested.

E2EE does not make a malicious writer honest, and CRDT convergence does not establish
ciphertext confidentiality or authenticity. No cryptographic correctness or soundness
claim is made here.

## 10. Intended properties and verification plan

Subject to valid dots, one dataset/profile/epoch, identical admission policy, atomic
transactions, and eventual delivery of the same delta set, the design intends:

1. deterministic visible-dataset convergence independent of valid delta order;
2. duplicate-delivery tolerance;
3. add-wins resolution for concurrent add/remove of the same quad;
4. preservation of named-graph membership as part of quad identity;
5. no receiver-side dependence on SPARQL evaluation or external resources.

It does **not** intend serializability, global intention preservation, automatic SHACL
validity, entailment equivalence, or convergence between replicas admitting different
transactions.

Verification should be staged:

- executable algebra unit vectors, including gaps in causal contexts;
- generated schedules permuting, duplicating, batching, snapshotting, and replaying
  deltas across at least three logical replicas;
- differential materialized-dataset comparisons after common delivery;
- crash-recovery tests for counter allocation/journal/snapshot boundaries;
- model checking of a bounded state machine;
- a separate mechanized or peer-reviewed proof of the merge laws and convergence theorem;
- adversarial codec fuzzing and quota tests;
- E2EE composition tests only after its envelope/admission contract is specified.

Property tests provide evidence and regression protection; they are not a formal proof.

## 11. Decisions the normative proposal must freeze

1. Exact dotted-set join equations and canonical executable vectors.
2. Whether empty named-graph existence is baseline or a graph-catalog extension.
3. Canonical binary format and limits.
4. Transaction atomicity and redundant-add behavior.
5. Replica membership epochs and causal-stability acknowledgements.
6. Which SPARQL Update operations are baseline, origin-only, or rejected.
7. Blank-node skolem namespace and API presentation.
8. Conformance classes: origin evaluator, replica, delta relay, and optional stable-
   frontier coordinator.

## References

- Almeida, Shoker, Baquero. “Delta State Replicated Data Types.” *Journal of Parallel
  and Distributed Computing* 111 (2018), 162–173.
  <https://doi.org/10.1016/j.jpdc.2017.08.003>
- Enes, Almeida, Baquero, Leitão. “Efficient Synchronization of State-based CRDTs.”
  *ICDCS* 2019. <https://arxiv.org/abs/1803.02750>
- Preguiça, Baquero, Shapiro. “Conflict-free Replicated Data Types (CRDTs).” 2018.
  <https://arxiv.org/abs/1805.06358>
- Ibáñez, Skaf-Molli, Molli, Corby. “Live Linked Data: Synchronising Semantic Stores
  with Commutative Replicated Data Types.” WWW 2012 companion (SU-Set), pp. 1091–1096.
  <https://archives.iw3c2.org/www2012/proceedings/companion/p1091.pdf>
- m-ld. “Realtime Information Sharing with RDF.” SEMANTiCS/CEUR 2021.
  <https://ceur-ws.org/Vol-2941/paper1.pdf>
- NextGraph documentation, “Conflict-Free Replicated Data Types” and “Architecture.”
  <https://docs.nextgraph.org/en/framework/crdts/> and
  <https://docs.nextgraph.org/en/architecture/>
- Automerge documentation, “Merge Rules,” “Conflicts,” and “Concepts.”
  <https://automerge.org/docs/reference/under-the-hood/merge-rules/>
- Matrix Specification, “Room Versions” and state resolution.
  <https://spec.matrix.org/latest/rooms/>
- Roh et al. “Replicated Abstract Data Types: Building Blocks for Collaborative
  Applications.” *Journal of Parallel and Distributed Computing* 71(3), 2011, 354–368.
  <https://doi.org/10.1016/j.jpdc.2010.12.006>
- W3C. “SPARQL 1.1 Update.” Recommendation, 2013.
  <https://www.w3.org/TR/sparql11-update/>
